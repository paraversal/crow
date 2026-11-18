//! iCloud / File Provider status, read from Foundation resource keys. Safe off
//! the main thread; the app calls it from background metadata batches and from
//! the fresh read gate, never from row rendering.

use std::path::{Path, PathBuf};

use crow_fs::{CloudStatus, ReadDecision, ReadGate, ReadHint};
use objc2::rc::autoreleasepool;
use objc2_foundation::NSURLResourceKey;
use objc2_foundation::{
    NSArray, NSNumber, NSString, NSURLIsUbiquitousItemKey, NSURLUbiquitousItemDownloadingStatusCurrent,
    NSURLUbiquitousItemDownloadingStatusDownloaded, NSURLUbiquitousItemDownloadingStatusKey,
    NSURLUbiquitousItemDownloadingStatusNotDownloaded, NSURLUbiquitousItemIsDownloadingKey,
};

/// One item's live classification. `Downloading` takes precedence over every
/// cached status; a lookup error or a ubiquitous item without a status is
/// `Unknown`, never `Local`.
pub fn status(path: &Path) -> CloudStatus {
    autoreleasepool(|_| status_inner(path))
}

/// A batch of classifications in one autorelease pool. Foundation is consulted
/// only for paths under a known provider root.
pub fn status_batch(paths: &[PathBuf]) -> Vec<CloudStatus> {
    autoreleasepool(|_| paths.iter().map(|path| status_inner(path)).collect())
}

fn status_inner(path: &Path) -> CloudStatus {
    let meta = std::fs::symlink_metadata(path).ok();
    if meta.as_ref().is_some_and(|meta| meta.file_type().is_symlink()) {
        if let Ok(target) = std::fs::metadata(path)
            && crow_fs::dataless_metadata(&target)
        {
            return CloudStatus::NotDownloaded;
        }
        // A link inside iCloud describes the link item, not the payload a reader
        // opens. Resolve once so Info and the row report the target's state.
        if let Ok(resolved) = std::fs::canonicalize(path)
            && resolved != path
        {
            return status_inner(&resolved);
        }
    }
    let dataless = meta.as_ref().is_some_and(crow_fs::dataless_metadata);
    if !crow_fs::is_cloud_candidate(path) {
        return if dataless { CloudStatus::NotDownloaded } else { CloudStatus::Local };
    }
    let url = crate::ns_url(path);
    // SAFETY: Foundation's resource-key constants are immutable strings for the
    // lifetime of the process.
    let keys = NSArray::from_slice(&unsafe {
        [NSURLIsUbiquitousItemKey, NSURLUbiquitousItemIsDownloadingKey, NSURLUbiquitousItemDownloadingStatusKey]
    });
    let Ok(values) = url.resourceValuesForKeys_error(&keys) else {
        return if dataless { CloudStatus::NotDownloaded } else { CloudStatus::Unknown };
    };
    let flag = |key: &NSURLResourceKey| {
        values.objectForKey(key).and_then(|value| value.downcast::<NSNumber>().ok()).map(|value| value.boolValue())
    };
    // SAFETY: the constants are immutable strings for the process lifetime.
    let (ubiquitous_key, downloading_key, status_key) = unsafe {
        (NSURLIsUbiquitousItemKey, NSURLUbiquitousItemIsDownloadingKey, NSURLUbiquitousItemDownloadingStatusKey)
    };
    let ubiquitous = flag(ubiquitous_key);
    let downloading = flag(downloading_key) == Some(true);
    let status = values.objectForKey(status_key).and_then(|value| value.downcast::<NSString>().ok());
    classify(ubiquitous, downloading, dataless, status.as_deref())
}

/// The pure status precedence, separated so it can be unit-tested without a
/// real provider.
fn classify(ubiquitous: Option<bool>, downloading: bool, dataless: bool, status: Option<&NSString>) -> CloudStatus {
    if downloading {
        return CloudStatus::Downloading;
    }
    if dataless {
        return CloudStatus::NotDownloaded;
    }
    match ubiquitous {
        Some(true) => {}
        Some(false) => return CloudStatus::Local,
        None => return CloudStatus::Unknown,
    }
    let Some(status) = status else { return CloudStatus::Unknown };
    // SAFETY: the constants are immutable strings for the process lifetime.
    let (current, downloaded, not_downloaded) = unsafe {
        (
            NSURLUbiquitousItemDownloadingStatusCurrent,
            NSURLUbiquitousItemDownloadingStatusDownloaded,
            NSURLUbiquitousItemDownloadingStatusNotDownloaded,
        )
    };
    if status.isEqualToString(current) || status.isEqualToString(downloaded) {
        CloudStatus::Downloaded
    } else if status.isEqualToString(not_downloaded) {
        CloudStatus::NotDownloaded
    } else {
        CloudStatus::Unknown
    }
}

/// The fresh app gate: cheap flags first, Foundation only for a known cloud
/// candidate. A cloud symlink is ambiguous unless its target is positively local.
pub struct SystemReadGate;

impl ReadGate for SystemReadGate {
    fn check(&self, path: &Path, hint: ReadHint) -> ReadDecision {
        let cheap = hint.cheap_decision();
        if cheap != ReadDecision::Readable || !hint.cloud_candidate {
            return cheap;
        }
        match status(path) {
            CloudStatus::Local | CloudStatus::Downloaded => ReadDecision::Readable,
            CloudStatus::NotDownloaded | CloudStatus::Downloading => ReadDecision::SkipCloud,
            CloudStatus::Unknown => ReadDecision::SkipAmbiguous,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ns(value: &str) -> objc2::rc::Retained<NSString> {
        NSString::from_str(value)
    }

    fn current() -> &'static NSString {
        // SAFETY: Foundation's downloading-status constants are immutable strings.
        unsafe { NSURLUbiquitousItemDownloadingStatusCurrent }
    }

    fn downloaded() -> &'static NSString {
        // SAFETY: Foundation's downloading-status constants are immutable strings.
        unsafe { NSURLUbiquitousItemDownloadingStatusDownloaded }
    }

    fn not_downloaded() -> &'static NSString {
        // SAFETY: Foundation's downloading-status constants are immutable strings.
        unsafe { NSURLUbiquitousItemDownloadingStatusNotDownloaded }
    }

    #[test]
    fn status_precedence() {
        assert_eq!(classify(Some(false), false, false, None), CloudStatus::Local);
        assert_eq!(classify(None, false, false, None), CloudStatus::Unknown);
        assert_eq!(classify(Some(true), true, false, None), CloudStatus::Downloading);
        assert_eq!(
            classify(Some(true), true, true, Some(not_downloaded())),
            CloudStatus::Downloading,
            "downloading wins"
        );
        assert_eq!(classify(Some(true), false, true, Some(current())), CloudStatus::NotDownloaded);
        assert_eq!(classify(Some(true), false, false, Some(current())), CloudStatus::Downloaded);
        assert_eq!(classify(Some(true), false, false, Some(downloaded())), CloudStatus::Downloaded);
        assert_eq!(classify(Some(true), false, false, Some(not_downloaded())), CloudStatus::NotDownloaded);
        assert_eq!(classify(Some(true), false, false, None), CloudStatus::Unknown);
        assert_eq!(classify(Some(true), false, false, Some(&ns("something else"))), CloudStatus::Unknown);
    }

    #[test]
    fn local_paths_are_classified_without_a_provider() {
        let dir = tempfile::tempdir().expect("temp");
        assert_eq!(status(dir.path()), CloudStatus::Local);
        let gate = SystemReadGate;
        let hint = ReadHint::for_path(dir.path());
        assert_eq!(gate.check(dir.path(), hint), ReadDecision::Readable);
    }
}
