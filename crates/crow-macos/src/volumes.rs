//! Mounted volumes for the sidebar's Locations: names, free space, eject, and
//! notifications when a disk is mounted, unmounted or renamed.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2_app_kit::{
    NSWorkspace, NSWorkspaceDidMountNotification, NSWorkspaceDidRenameVolumeNotification,
    NSWorkspaceDidUnmountNotification,
};
use objc2_foundation::{
    NSArray, NSFileManager, NSNotification, NSNumber, NSOperationQueue, NSString, NSURLResourceKey,
    NSURLVolumeAvailableCapacityForImportantUsageKey, NSURLVolumeAvailableCapacityKey, NSURLVolumeIsBrowsableKey,
    NSURLVolumeIsEjectableKey, NSURLVolumeIsInternalKey, NSURLVolumeIsRemovableKey, NSURLVolumeLocalizedNameKey,
    NSURLVolumeTotalCapacityKey, NSVolumeEnumerationOptions,
};

use crate::main_thread;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Volume {
    /// Finder's name ("Macintosh HD", "Untitled").
    pub name: String,
    pub path: PathBuf,
    pub total_bytes: u64,
    /// What Finder shows as available: includes space macOS can purge on demand.
    pub free_bytes: u64,
    /// Removable media, disk images and network shares, which get an Eject button.
    pub ejectable: bool,
}

/// The volumes Finder lists, root first. Safe on any thread; network volumes can
/// answer slowly, so call it off the main thread.
pub fn volumes() -> Vec<Volume> {
    autoreleasepool(|_| {
        // SAFETY: the keys are immutable framework constants.
        let keys: Vec<&NSURLResourceKey> = unsafe {
            vec![
                NSURLVolumeLocalizedNameKey,
                NSURLVolumeTotalCapacityKey,
                NSURLVolumeAvailableCapacityForImportantUsageKey,
                NSURLVolumeAvailableCapacityKey,
                NSURLVolumeIsEjectableKey,
                NSURLVolumeIsRemovableKey,
                NSURLVolumeIsInternalKey,
                NSURLVolumeIsBrowsableKey,
            ]
        };
        let key_array = NSArray::from_slice(&keys);
        let Some(urls) = NSFileManager::defaultManager().mountedVolumeURLsIncludingResourceValuesForKeys_options(
            Some(&key_array),
            NSVolumeEnumerationOptions::SkipHiddenVolumes,
        ) else {
            return Vec::new();
        };
        let mut out: Vec<Volume> = urls
            .iter()
            .filter_map(|url| {
                let path = PathBuf::from(url.path()?.to_string());
                let values = url.resourceValuesForKeys_error(&key_array).ok()?;
                let get = |key: &NSURLResourceKey| values.objectForKey(key);
                let number = |key: &NSURLResourceKey| {
                    get(key).and_then(|v| v.downcast::<NSNumber>().ok()).map(|n| n.as_i64().max(0) as u64)
                };
                let flag = |key: &NSURLResourceKey| number(key).is_some_and(|n| n != 0);
                // SAFETY: framework constants, as above.
                unsafe {
                    if !flag(NSURLVolumeIsBrowsableKey) {
                        return None;
                    }
                    let name = get(NSURLVolumeLocalizedNameKey)
                        .and_then(|v| v.downcast::<NSString>().ok())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| path.display().to_string());
                    let free = number(NSURLVolumeAvailableCapacityForImportantUsageKey)
                        .filter(|n| *n > 0)
                        .or_else(|| number(NSURLVolumeAvailableCapacityKey))
                        .unwrap_or(0);
                    let ejectable = flag(NSURLVolumeIsEjectableKey)
                        || flag(NSURLVolumeIsRemovableKey)
                        || !flag(NSURLVolumeIsInternalKey) && path != Path::new("/");
                    Some(Volume {
                        name,
                        total_bytes: number(NSURLVolumeTotalCapacityKey).unwrap_or(0),
                        free_bytes: free,
                        ejectable,
                        path,
                    })
                }
            })
            .collect();
        out.sort_by_key(|v| (v.path != Path::new("/"), v.name.to_lowercase()));
        out
    })
}

/// Unmount and eject the volume at `path`. Blocks until done (seconds for a busy
/// disk), so run it off the main thread. The error is what `diskutil` said.
pub fn eject(path: &Path) -> Result<(), String> {
    let output =
        Command::new("/usr/sbin/diskutil").arg("eject").arg(path).output().map_err(|e| format!("diskutil: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&output.stderr);
    let said = if said.trim().is_empty() { String::from_utf8_lossy(&output.stdout) } else { said };
    Err(said.trim().to_owned())
}

/// Observers of mount notifications; dropping it stops them.
pub struct MountWatcher {
    tokens: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
}

impl Drop for MountWatcher {
    fn drop(&mut self) {
        let center = NSWorkspace::sharedWorkspace().notificationCenter();
        for token in &self.tokens {
            // SAFETY: each token came from this center's addObserver.
            unsafe { center.removeObserver(token.as_ref() as &AnyObject) };
        }
    }
}

/// Call `changed` on the main thread whenever a volume is mounted, unmounted or
/// renamed. `None` off the main thread.
pub fn watch_mounts(changed: impl Fn() + 'static) -> Option<MountWatcher> {
    main_thread()?;
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    let block = RcBlock::new(move |_: NonNull<NSNotification>| changed());
    // SAFETY: framework constants; the block runs on the main queue, where `changed` was made.
    let tokens = unsafe {
        [NSWorkspaceDidMountNotification, NSWorkspaceDidUnmountNotification, NSWorkspaceDidRenameVolumeNotification]
            .into_iter()
            .map(|name| {
                center.addObserverForName_object_queue_usingBlock(
                    Some(name),
                    None,
                    Some(&NSOperationQueue::mainQueue()),
                    &block,
                )
            })
            .collect()
    };
    Some(MountWatcher { tokens })
}

/// The "Reduce motion" accessibility setting.
pub fn reduce_motion() -> bool {
    main_thread().is_some() && NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_startup_disk_is_listed_first() {
        let all = volumes();
        let root = all.first().expect("at least the startup disk");
        assert_eq!(root.path, Path::new("/"));
        assert!(!root.ejectable);
        assert!(root.total_bytes >= root.free_bytes && root.total_bytes > 0, "{root:?}");
    }
}
