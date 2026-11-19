//! Cheap predicate evaluation shared by every search candidate. Kind is decided
//! from the path and the entry type; no content bytes and no LaunchServices call.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Stable kind predicate values, independent of the localized Finder kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Folder,
    File,
    Image,
    Pdf,
    Audio,
    Video,
    Archive,
    Text,
    Extension(String),
}

impl Kind {
    pub fn is_folder(&self) -> bool {
        matches!(self, Kind::Folder)
    }

    /// Whether `path` with `is_dir` is this kind. `File` means any non-folder.
    /// A category is decided by the exact extension; unknown extensions do not
    /// match a category, and a missing or non-UTF-8 extension matches none.
    pub fn matches(&self, path: &Path, is_dir: bool) -> bool {
        if self.is_folder() {
            return is_dir;
        }
        if is_dir {
            return false;
        }
        if matches!(self, Kind::File) {
            return true;
        }
        let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
            return false;
        };
        let extension = extension.to_ascii_lowercase();
        match self {
            Kind::Folder | Kind::File => unreachable!("handled above"),
            Kind::Image => IMAGE.contains(&extension.as_str()),
            Kind::Pdf => extension == "pdf",
            Kind::Audio => AUDIO.contains(&extension.as_str()),
            Kind::Video => VIDEO.contains(&extension.as_str()),
            Kind::Archive => ARCHIVE.contains(&extension.as_str()),
            Kind::Text => TEXT.contains(&extension.as_str()),
            Kind::Extension(expected) => extension == *expected,
        }
    }
}

const IMAGE: &[&str] =
    &["jpg", "jpeg", "png", "gif", "heic", "heif", "tif", "tiff", "bmp", "webp", "svg", "ico", "avif"];
const AUDIO: &[&str] = &["mp3", "m4a", "aac", "wav", "flac", "aiff", "aif", "ogg", "opus", "wma", "mid", "midi"];
const VIDEO: &[&str] = &["mp4", "mov", "m4v", "avi", "mkv", "webm", "wmv", "flv", "mpg", "mpeg", "3gp"];
const ARCHIVE: &[&str] = &["zip", "tar", "gz", "tgz", "bz2", "tbz", "xz", "txz", "7z", "rar", "zst", "dmg", "iso"];
const TEXT: &[&str] = &[
    "txt", "text", "md", "markdown", "rtf", "csv", "tsv", "json", "jsonl", "toml", "yaml", "yml", "xml", "html", "htm",
    "css", "scss", "sass", "js", "jsx", "ts", "tsx", "rs", "py", "rb", "sh", "bash", "zsh", "fish", "c", "h", "cc",
    "cpp", "cxx", "hpp", "m", "mm", "swift", "go", "java", "kt", "kts", "php", "pl", "lua", "sql", "log", "ini", "cfg",
    "conf", "tex", "bib",
];

/// Resolved predicates for one run. The clock is captured once by `search` and
/// passed to every candidate; an age predicate excludes missing and future mtimes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Predicates {
    pub kind: Option<Kind>,
    pub modified_within: Option<Duration>,
}

impl Predicates {
    pub fn is_empty(&self) -> bool {
        self.kind.is_none() && self.modified_within.is_none()
    }

    pub fn matches(&self, path: &Path, is_dir: bool, modified: Option<SystemTime>, run_started: SystemTime) -> bool {
        if let Some(kind) = &self.kind
            && !kind.matches(path, is_dir)
        {
            return false;
        }
        if let Some(window) = self.modified_within {
            let Some(modified) = modified else { return false };
            // A future modification date fails: `duration_since` errors.
            if run_started.duration_since(modified).is_err() {
                return false;
            }
            let cutoff = run_started.checked_sub(window).unwrap_or(SystemTime::UNIX_EPOCH);
            if modified < cutoff {
                return false;
            }
        }
        true
    }
}

/// Deterministic metadata-only ordering: natural name, then the raw path bytes.
/// Used to retain the smallest results of a capped scan regardless of walk order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MetadataKey {
    name: String,
    path: PathBuf,
}

impl MetadataKey {
    pub(crate) fn new(path: &Path) -> Self {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        Self { name, path: path.to_path_buf() }
    }
}

impl PartialOrd for MetadataKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MetadataKey {
    fn cmp(&self, other: &Self) -> Ordering {
        crow_fs::natural_cmp(&self.name, &other.name).then_with(|| self.path.cmp(&other.path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
    }

    #[test]
    fn kind_classification_is_extension_based_and_folder_aware() {
        assert!(Kind::Pdf.matches(Path::new("/x/Report.PDF"), false));
        assert!(!Kind::Pdf.matches(Path::new("/x/report.txt"), false));
        assert!(Kind::Image.matches(Path::new("/x/a.heic"), false));
        assert!(Kind::Audio.matches(Path::new("/x/a.flac"), false));
        assert!(Kind::Video.matches(Path::new("/x/a.mkv"), false));
        assert!(Kind::Archive.matches(Path::new("/x/a.7z"), false));
        assert!(Kind::Text.matches(Path::new("/x/a.rs"), false));
        assert!(Kind::Extension("rs".into()).matches(Path::new("/x/a.RS"), false));
        assert!(!Kind::Extension("rs".into()).matches(Path::new("/x/a.py"), false));
        assert!(Kind::Folder.matches(Path::new("/x/Photos"), true));
        assert!(!Kind::Folder.matches(Path::new("/x/Photos"), false));
        assert!(Kind::File.matches(Path::new("/x/notes"), false));
        assert!(!Kind::File.matches(Path::new("/x/notes"), true));
        // A mislabeled PDF is a PDF by the cheap extension classification.
        assert!(Kind::Pdf.matches(Path::new("/x/mislabeled.pdf"), false));
    }

    #[test]
    fn age_cutoff_is_inclusive_and_rejects_future_or_missing() {
        let run = at(1_000_000);
        let window = Duration::from_secs(100);
        let predicates = Predicates { kind: None, modified_within: Some(window) };
        let cutoff = run - window;
        assert!(predicates.matches(Path::new("/x/a"), false, Some(cutoff), run));
        assert!(!predicates.matches(Path::new("/x/a"), false, Some(cutoff - Duration::from_secs(1)), run));
        assert!(predicates.matches(Path::new("/x/a"), false, Some(run), run));
        assert!(!predicates.matches(Path::new("/x/a"), false, Some(run + Duration::from_secs(1)), run));
        assert!(!predicates.matches(Path::new("/x/a"), false, None, run));
    }

    #[test]
    fn metadata_keys_sort_naturally_then_by_path() {
        let mut keys = [
            MetadataKey::new(Path::new("/b/file-10.txt")),
            MetadataKey::new(Path::new("/a/file-2.txt")),
            MetadataKey::new(Path::new("/a/file-10.txt")),
        ];
        keys.sort();
        let names: Vec<_> = keys.iter().map(|key| key.path.display().to_string()).collect();
        assert_eq!(names, ["/a/file-2.txt", "/a/file-10.txt", "/b/file-10.txt"]);
    }
}
