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
    // FSEvents reports resolved paths (`/private/var/...` for `/var/...`), and
    // when the watched folder is removed and recreated it reports that folder's
    // path instead of the removed children. A diff would then add the new
    // children next to the old rows, so a root event means relist.
    let watched = dir.to_path_buf();
    let resolved = dir.canonicalize().unwrap_or_else(|_| watched.clone());
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let change = match res {
            Ok(event) if event.need_rescan() => FolderChange::Rescan,
            Ok(event) if event.paths.iter().any(|path| path == &watched || path == &resolved) => FolderChange::Rescan,
            Ok(event) => FolderChange::Paths(event.paths),
            Err(_) => FolderChange::Rescan,
        };
        // The receiver is gone when the pane closed; nothing left to notify.
        tx.send(change).ok();
    })?;
    watcher.watch(dir, RecursiveMode::NonRecursive)?;
    Ok(FolderWatcher { _inner: watcher })
}
