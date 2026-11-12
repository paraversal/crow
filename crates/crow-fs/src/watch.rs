use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

/// What changed in a watched folder.
#[derive(Debug)]
pub enum FolderChange {
    /// These paths changed; apply them with `apply_changes`.
    Paths(Vec<PathBuf>),
    /// The system lost track (event overflow): relist the folder.
    Rescan,
}

/// Keeps a folder watch alive; dropping it stops the watch.
pub struct FolderWatcher {
    _inner: RecommendedWatcher,
}

/// Watch `dir` (not recursively) and send each change to `tx`.
pub fn watch_folder(dir: &Path, tx: Sender<FolderChange>) -> notify::Result<FolderWatcher> {
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let change = match res {
            Ok(event) if event.need_rescan() => FolderChange::Rescan,
            Ok(event) => FolderChange::Paths(event.paths),
            Err(_) => FolderChange::Rescan,
        };
        // The receiver is gone when the pane closed; nothing left to notify.
        tx.send(change).ok();
    })?;
    watcher.watch(dir, RecursiveMode::NonRecursive)?;
    Ok(FolderWatcher { _inner: watcher })
}
