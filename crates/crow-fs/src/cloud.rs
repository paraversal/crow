//! Cloud placeholder metadata and the automatic-read policy shared by the app,
//! search and plugins.
//!
//! The cached `CloudStatus` is for display; `ReadDecision` is checked afresh
//! immediately before an automatic content read. The cheap signal is the
//! `SF_DATALESS` file flag, which macOS also returns in the bulk listing Crow
//! already uses. Foundation is only consulted for paths under a known provider
//! root (or a symlink whose target is), never for an ordinary local entry.

use std::fs::Metadata;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// What iCloud reports for one item. `Downloading` wins over every other state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CloudStatus {
    /// A positively ordinary local item, or a cloud item with no current cloud
    /// metadata to report.
    Local,
    /// No local payload is present (the file is a placeholder).
    NotDownloaded,
    /// The provider is actively fetching the payload.
    Downloading,
    /// A current or prior version is present locally.
    Downloaded,
    /// The item is under cloud ambiguity and no trustworthy classification was
    /// obtained. Automatic reads are refused.
    #[default]
    Unknown,
}

impl CloudStatus {
    /// Whether cached metadata is enough to paint a normal local item.
    pub fn is_local(self) -> bool {
        matches!(self, Self::Local | Self::Downloaded)
    }

    /// The Info row's value. `None` for an ordinary local item.
    pub fn info_text(self) -> Option<&'static str> {
        match self {
            Self::Local => None,
            Self::NotDownloaded => Some("Not downloaded"),
            Self::Downloading => Some("Downloading…"),
            Self::Downloaded => Some("Downloaded"),
            Self::Unknown => Some("Status unavailable"),
        }
    }

    /// What VoiceOver says after a row's name.
    pub fn spoken_suffix(self) -> Option<&'static str> {
        match self {
            Self::Local => None,
            Self::NotDownloaded => Some("not downloaded from iCloud"),
            Self::Downloading => Some("downloading from iCloud"),
            Self::Downloaded => Some("downloaded from iCloud"),
            Self::Unknown => Some("iCloud status unavailable"),
        }
    }
}

/// The fresh gate's answer for one automatic read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadDecision {
    Readable,
    /// A verified placeholder: skip the open and keep metadata/name results.
    SkipCloud,
    /// Cloud membership or downloading state cannot be ruled out: fail closed.
    SkipAmbiguous,
}

/// Cheap, no-follow metadata for the read gate. `target_*` is filled only for a
/// symlink, because the link's own `SF_DATALESS` bit describes the link, not the
/// file a reader would open.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReadHint {
    pub dataless: bool,
    pub is_symlink: bool,
    pub target_dataless: Option<bool>,
    pub cloud_candidate: bool,
    pub target_cloud: Option<bool>,
}

impl ReadHint {
    /// Read the hint for `path` now. Runs in a background consumer, never in a
    /// row renderer: a symlink costs one target stat and one canonicalize.
    pub fn for_path(path: &Path) -> Self {
        let meta = std::fs::symlink_metadata(path).ok();
        Self::from_metadata(path, meta.as_ref())
    }

    /// The hint from metadata the walker already has. A symlink's target is
    /// resolved here only when the linkage matters to the gate.
    pub fn from_metadata(path: &Path, meta: Option<&Metadata>) -> Self {
        let is_symlink = meta.is_some_and(|meta| meta.file_type().is_symlink());
        let cloud_candidate = is_cloud_candidate(path);
        let (target_dataless, target_cloud) = if is_symlink {
            match std::fs::metadata(path) {
                Ok(target) => {
                    let target_cloud = std::fs::canonicalize(path).ok().map(|path| is_cloud_candidate(&path));
                    (Some(dataless_metadata(&target)), target_cloud)
                }
                Err(_) => (None, None),
            }
        } else {
            (None, None)
        };
        Self {
            dataless: meta.is_some_and(dataless_metadata),
            is_symlink,
            target_dataless,
            cloud_candidate,
            target_cloud,
        }
    }

    /// The policy that needs no Foundation call.
    pub fn cheap_decision(&self) -> ReadDecision {
        if self.dataless || self.target_dataless == Some(true) {
            return ReadDecision::SkipCloud;
        }
        if self.is_symlink && self.target_cloud.unwrap_or(self.cloud_candidate) {
            return ReadDecision::SkipAmbiguous;
        }
        ReadDecision::Readable
    }
}

/// The app supplies this to every automatic reader (preview, search, plugins).
/// Implementations must be cheap and thread-safe; the gate runs on a background
/// consumer immediately before an open.
pub trait ReadGate: Send + Sync {
    fn check(&self, path: &Path, hint: ReadHint) -> ReadDecision;
}

/// For tests and non-cloud consumers: every ordinary local read is allowed.
pub struct AllowAll;

impl ReadGate for AllowAll {
    fn check(&self, _: &Path, hint: ReadHint) -> ReadDecision {
        hint.cheap_decision()
    }
}

/// iCloud Drive and File Provider roots. These are the paths where a dataless
/// bit alone does not distinguish an active download; Foundation is consulted
/// only there.
pub fn is_cloud_candidate(path: &Path) -> bool {
    static ROOTS: LazyLock<Vec<PathBuf>> = LazyLock::new(|| {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return Vec::new() };
        vec![home.join("Library/Mobile Documents"), home.join("Library/CloudStorage")]
    });
    ROOTS.iter().any(|root| path.starts_with(root))
}

/// `SF_DATALESS` from `<sys/stat.h>`; not exported by libc.
pub const SF_DATALESS: u32 = 0x4000_0000;

pub fn dataless_flags(flags: u32) -> bool {
    flags & SF_DATALESS != 0
}

#[cfg(target_os = "macos")]
pub fn dataless_metadata(meta: &Metadata) -> bool {
    use std::os::macos::fs::MetadataExt;
    dataless_flags(meta.st_flags())
}

#[cfg(not(target_os = "macos"))]
pub fn dataless_metadata(_: &Metadata) -> bool {
    false
}

/// A status cache entry must not authorize a read: this is only for deciding
/// how long to keep painting a glyph.
pub fn status_age_limit(status: CloudStatus) -> std::time::Duration {
    match status {
        CloudStatus::Downloading => std::time::Duration::from_millis(500),
        CloudStatus::NotDownloaded | CloudStatus::Unknown => std::time::Duration::from_secs(2),
        CloudStatus::Downloaded | CloudStatus::Local => std::time::Duration::from_secs(30),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloading_wins_in_the_status_contract() {
        assert_eq!(CloudStatus::Downloading.info_text(), Some("Downloading…"));
        assert_eq!(CloudStatus::Downloaded.info_text(), Some("Downloaded"));
        assert_eq!(CloudStatus::Local.info_text(), None);
        assert_eq!(CloudStatus::Unknown.spoken_suffix(), Some("iCloud status unavailable"));
    }

    #[test]
    fn dataless_flags_and_hints_fail_closed() {
        assert!(dataless_flags(0x4000_0000));
        assert!(dataless_flags(0x4000_0060));
        assert!(!dataless_flags(0x40));

        let cloud = ReadHint { dataless: true, cloud_candidate: true, ..ReadHint::default() };
        assert_eq!(cloud.cheap_decision(), ReadDecision::SkipCloud);

        let link =
            ReadHint { is_symlink: true, cloud_candidate: true, target_dataless: Some(false), ..ReadHint::default() };
        assert_eq!(link.cheap_decision(), ReadDecision::SkipAmbiguous, "a cloud symlink is ambiguous");
        let cloud_link_to_local = ReadHint { target_cloud: Some(false), ..link.clone() };
        assert_eq!(cloud_link_to_local.cheap_decision(), ReadDecision::Readable);
        let local_link_to_cloud = ReadHint { cloud_candidate: false, target_cloud: Some(true), ..link };
        assert_eq!(local_link_to_cloud.cheap_decision(), ReadDecision::SkipAmbiguous);

        let local_link = ReadHint { is_symlink: true, ..ReadHint::default() };
        assert_eq!(local_link.cheap_decision(), ReadDecision::Readable);

        let missing = ReadHint::for_path(Path::new("/definitely/not/here"));
        assert_eq!(missing.cheap_decision(), ReadDecision::Readable, "the open itself will fail");
    }

    #[test]
    fn ordinary_paths_are_not_cloud_candidates() {
        let dir = crate::test_support::TempDir::new("crow-fs-cloud-");
        assert!(!is_cloud_candidate(&dir));
        assert!(!is_cloud_candidate(Path::new("/tmp")));
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
        assert!(is_cloud_candidate(&home.join("Library/Mobile Documents/com~apple~CloudDocs/x")));
        assert!(is_cloud_candidate(&home.join("Library/CloudStorage/Provider/x")));
    }

    #[test]
    fn status_age_limits_are_bounded() {
        assert!(status_age_limit(CloudStatus::Downloading) < status_age_limit(CloudStatus::Downloaded));
        assert_eq!(status_age_limit(CloudStatus::Unknown), std::time::Duration::from_secs(2));
    }
}
