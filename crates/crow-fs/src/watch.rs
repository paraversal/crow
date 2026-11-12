use std::path::Path;
use std::sync::mpsc::Sender;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

/// Keeps a folder watch alive; dropping it stops the watch.
pub struct FolderWatcher {
    _inner: RecommendedWatcher,
}

/// Watch `dir` (not recursively). Every change sends `()`; the receiver debounces
/// and reloads. M2 replaces the full reload with per-entry diffs.
pub fn watch_folder(dir: &Path, tx: Sender<()>) -> notify::Result<FolderWatcher> {
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            // The receiver is gone when the pane closed; nothing left to notify.
            tx.send(()).ok();
        }
    })?;
    watcher.watch(dir, RecursiveMode::NonRecursive)?;
    Ok(FolderWatcher { _inner: watcher })
}
