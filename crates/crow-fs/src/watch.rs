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
    // children next to the old rows, so a root event means relist. Resolving the
    // folder happens on the watcher's thread (no disk I/O on the main thread)
    // and once per watcher.
    let watched = dir.to_path_buf();
    let resolved = std::sync::OnceLock::new();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let change = match res {
            Ok(event) if event.need_rescan() => FolderChange::Rescan,
            Ok(event) => {
                let root = resolved.get_or_init(|| resolve_root(&watched));
                if event.paths.iter().any(|path| path == &watched || path == root) {
                    FolderChange::Rescan
                } else {
                    FolderChange::Paths(event.paths)
                }
            }
            Err(_) => FolderChange::Rescan,
        };
        // The receiver is gone when the pane closed; nothing left to notify.
        tx.send(change).ok();
    })?;
    watcher.watch(dir, RecursiveMode::NonRecursive)?;
    Ok(FolderWatcher { _inner: watcher })
}

/// `dir`'s resolved form. When `dir` itself is gone (a root event can arrive for
/// the old name after removal), resolve its parent and keep the name.
fn resolve_root(dir: &Path) -> PathBuf {
    dir.canonicalize().unwrap_or_else(|_| match (dir.parent(), dir.file_name()) {
        (Some(parent), Some(name)) => {
            parent.canonicalize().map_or_else(|_| dir.to_path_buf(), |parent| parent.join(name))
        }
        _ => dir.to_path_buf(),
    })
}
