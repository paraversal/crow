//! Measuring a folder's size by walking everything below it.
//!
//! The walk never opens a file: it reads metadata only. Hard links count once,
//! symlinks are not followed, other volumes are not crossed, and a package's
//! contents count toward its parent's bytes while the package itself is one item.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// What a walk found. `items` counts every entry below the root (files, symlinks
/// and folders; a package counts once, not its contents); `folders` is the subset
/// that are directories. `unreadable` counts folders that could not be listed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Measured {
    pub bytes: u64,
    pub items: u64,
    pub folders: u64,
    pub unreadable: u64,
}

/// Stops a walk between entries. Cloning shares the flag, so the scheduler can
/// cancel from another thread.
#[derive(Clone, Debug, Default)]
pub struct Cancel {
    flag: Arc<AtomicBool>,
}

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }
}

/// Measure `root`. `None` when the walk was cancelled or the root itself could not
/// be read; a subfolder that cannot be read is counted in `unreadable` and the
/// rest of the tree is measured.
pub fn measure(root: &Path, cancel: &Cancel) -> Option<Measured> {
    measure_with(root, cancel, &mut |_| {})
}

fn measure_with(root: &Path, cancel: &Cancel, on_dir: &mut dyn FnMut(&Path)) -> Option<Measured> {
    if cancel.is_cancelled() {
        return None;
    }
    let meta = std::fs::metadata(root).ok()?;
    if !meta.is_dir() {
        return None;
    }
    let root_dev = device(&meta);
    let mut walk = Walk::default();
    let done = imp::walk_dir(root, root_dev, true, true, cancel, &mut walk, on_dir);
    done.then_some(walk.measured)
}

#[derive(Default)]
struct Walk {
    measured: Measured,
    /// `(device, file id)` of every hard link already counted.
    seen: HashSet<(u64, u64)>,
}

impl Walk {
    fn file(&mut self, dev: u64, fileid: u64, nlink: u32, size: u64, count: bool) {
        if count {
            self.measured.items += 1;
        }
        if nlink > 1 && !self.seen.insert((dev, fileid)) {
            return;
        }
        self.measured.bytes += size;
    }

    fn symlink(&mut self, size: u64, count: bool) {
        if count {
            self.measured.items += 1;
        }
        self.measured.bytes += size;
    }

    fn dir(&mut self, count: bool) {
        if count {
            self.measured.folders += 1;
            self.measured.items += 1;
        }
    }
}

#[cfg(target_os = "macos")]
fn device(meta: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.dev()
}

#[cfg(not(target_os = "macos"))]
fn device(_: &std::fs::Metadata) -> u64 {
    0
}

#[cfg(target_os = "macos")]
mod imp {
    use super::{Cancel, Walk};
    use crate::listing::{is_package_path, list_raw};
    use std::path::{Path, PathBuf};

    pub(super) fn walk_dir(
        dir: &Path,
        root_dev: u64,
        count: bool,
        root: bool,
        cancel: &Cancel,
        walk: &mut Walk,
        on_dir: &mut dyn FnMut(&Path),
    ) -> bool {
        on_dir(dir);
        if cancel.is_cancelled() {
            return false;
        }
        let mut subdirs: Vec<(PathBuf, bool)> = Vec::new();
        let mut stopped = false;
        let result = list_raw(dir, false, &mut |batch| {
            for entry in batch {
                if cancel.is_cancelled() {
                    stopped = true;
                    return;
                }
                if entry.dev != root_dev {
                    continue;
                }
                if entry.is_symlink() {
                    walk.symlink(entry.size, count);
                } else if entry.is_dir() {
                    let path = entry.path();
                    let package = is_package_path(&path);
                    walk.dir(count);
                    subdirs.push((path, count && !package));
                } else {
                    walk.file(entry.dev, entry.fileid, entry.nlink, entry.size, count);
                }
            }
        });
        if result.is_err() {
            // A subfolder that cannot be listed is partial, not fatal; the root
            // itself has nothing to measure.
            walk.measured.unreadable += 1;
            return !root;
        }
        if stopped {
            return false;
        }
        for (sub, count) in subdirs {
            if !walk_dir(&sub, root_dev, count, false, cancel, walk, on_dir) {
                return false;
            }
        }
        true
    }
}

#[cfg(any(not(target_os = "macos"), test))]
mod portable {
    use super::{Cancel, Walk};
    use crate::listing::is_package_path;
    use std::os::unix::fs::MetadataExt;
    use std::path::{Path, PathBuf};

    pub(super) fn walk_dir(
        dir: &Path,
        root_dev: u64,
        count: bool,
        root: bool,
        cancel: &Cancel,
        walk: &mut Walk,
        on_dir: &mut dyn FnMut(&Path),
    ) -> bool {
        on_dir(dir);
        if cancel.is_cancelled() {
            return false;
        }
        let Ok(read) = std::fs::read_dir(dir) else {
            walk.measured.unreadable += 1;
            return !root;
        };
        let mut subdirs: Vec<(PathBuf, bool)> = Vec::new();
        for entry in read {
            if cancel.is_cancelled() {
                return false;
            }
            let Ok(entry) = entry else {
                walk.measured.unreadable += 1;
                continue;
            };
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
                walk.measured.unreadable += 1;
                continue;
            };
            if meta.dev() != root_dev {
                continue;
            }
            let kind = meta.file_type();
            if kind.is_symlink() {
                walk.symlink(meta.len(), count);
            } else if kind.is_dir() {
                let package = is_package_path(&entry.path());
                walk.dir(count);
                subdirs.push((entry.path(), count && !package));
            } else {
                walk.file(meta.dev(), meta.ino(), meta.nlink() as u32, meta.len(), count);
            }
        }
        for (sub, count) in subdirs {
            if !walk_dir(&sub, root_dev, count, false, cancel, walk, on_dir) {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("crow-fs-sizes-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir.canonicalize().expect("canonical")
    }

    fn write(path: &Path, bytes: usize) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(path, vec![b'x'; bytes]).expect("write file");
    }

    fn measured(root: &Path) -> Measured {
        measure(root, &Cancel::new()).expect("measure")
    }

    #[test]
    fn nested_folders_count_bytes_items_and_folders() {
        let root = temp_dir("nested");
        write(&root.join("a.txt"), 10);
        write(&root.join("sub/b.txt"), 20);
        write(&root.join("sub/deep/c.txt"), 30);
        std::fs::create_dir(root.join("empty")).unwrap();
        let m = measured(&root);
        assert_eq!(m.bytes, 60);
        assert_eq!(m.folders, 3, "sub, deep and empty");
        assert_eq!(m.items, 6, "3 files and 3 folders");
        assert_eq!(m.unreadable, 0);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn hard_links_count_once_but_each_entry_is_an_item() {
        let root = temp_dir("hardlinks");
        write(&root.join("a"), 10);
        std::fs::hard_link(root.join("a"), root.join("b")).unwrap();
        std::fs::hard_link(root.join("a"), root.join("c")).unwrap();
        let m = measured(&root);
        assert_eq!(m.bytes, 10);
        assert_eq!(m.items, 3);
        assert_eq!(m.folders, 0);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn symlinks_are_not_followed_even_in_a_loop() {
        let root = temp_dir("symlinks");
        std::os::unix::fs::symlink("b", root.join("a")).unwrap();
        std::os::unix::fs::symlink("a", root.join("b")).unwrap();
        std::fs::create_dir(root.join("real")).unwrap();
        write(&root.join("real/inside"), 100);
        std::os::unix::fs::symlink("real", root.join("link-to-dir")).unwrap();
        let m = measured(&root);
        assert_eq!(m.bytes, 106, "the links' own sizes plus the real file");
        assert_eq!(m.items, 5, "a, b, link-to-dir, real and inside");
        assert_eq!(m.folders, 1);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_package_counts_as_one_item_but_its_bytes_count() {
        let root = temp_dir("package");
        write(&root.join("App.app/Contents/file"), 10);
        write(&root.join("App.app/Contents/sub/file"), 20);
        write(&root.join("normal/file"), 5);
        let m = measured(&root);
        assert_eq!(m.bytes, 35);
        assert_eq!(m.items, 3, "App.app, normal and normal/file");
        assert_eq!(m.folders, 2, "App.app and normal");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_unreadable_subfolder_is_counted_not_fatal() {
        use std::os::unix::fs::PermissionsExt;
        let root = temp_dir("unreadable");
        write(&root.join("ok.txt"), 5);
        let locked = root.join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let m = measured(&root);
        assert_eq!(m.bytes, 5);
        assert_eq!(m.items, 2, "ok.txt and locked");
        assert_eq!(m.folders, 1);
        assert_eq!(m.unreadable, 1);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_unreadable_root_is_none() {
        use std::os::unix::fs::PermissionsExt;
        let root = temp_dir("unreadable-root");
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o000)).unwrap();
        assert_eq!(measure(&root, &Cancel::new()), None);
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn cancellation_stops_between_entries() {
        let root = temp_dir("cancel");
        write(&root.join("a"), 1);
        std::fs::create_dir(root.join("stop-here")).unwrap();
        write(&root.join("stop-here/b"), 1);
        write(&root.join("z"), 1);
        let cancel = Cancel::new();
        let mut saw_stop = false;
        let result = measure_with(&root, &cancel, &mut |dir| {
            if dir.ends_with("stop-here") {
                saw_stop = true;
                cancel.cancel();
            }
        });
        assert!(saw_stop, "the hook sees every folder");
        assert_eq!(result, None);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_cancelled_walk_does_not_start() {
        let root = temp_dir("cancel-before");
        write(&root.join("a"), 1);
        let cancel = Cancel::new();
        cancel.cancel();
        assert_eq!(measure(&root, &cancel), None);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_fake_device_boundary_is_not_crossed() {
        let root = temp_dir("boundary");
        write(&root.join("a"), 10);
        std::fs::create_dir(root.join("sub")).unwrap();
        write(&root.join("sub/b"), 20);
        let cancel = Cancel::new();
        let mut walk = Walk::default();
        assert!(portable::walk_dir(&root, u64::MAX, true, true, &cancel, &mut walk, &mut |_| {}));
        assert_eq!(walk.measured, Measured::default());
        std::fs::remove_dir_all(&root).ok();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn bulk_and_portable_walks_agree() {
        let root = temp_dir("agree");
        write(&root.join("a.txt"), 10);
        write(&root.join("sub/b.txt"), 20);
        std::fs::hard_link(root.join("a.txt"), root.join("link")).unwrap();
        std::os::unix::fs::symlink("sub", root.join("sym")).unwrap();
        let cancel = Cancel::new();
        let bulk = measure(&root, &cancel).expect("bulk");
        let mut walk = Walk::default();
        let dev = device(&std::fs::metadata(&root).unwrap());
        assert!(portable::walk_dir(&root, dev, true, true, &cancel, &mut walk, &mut |_| {}));
        assert_eq!(bulk, walk.measured);
        std::fs::remove_dir_all(&root).ok();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn bulk_skips_a_fake_device_boundary() {
        let root = temp_dir("bulk-boundary");
        write(&root.join("a"), 10);
        let cancel = Cancel::new();
        let mut walk = Walk::default();
        assert!(imp::walk_dir(&root, u64::MAX, true, true, &cancel, &mut walk, &mut |_| {}));
        assert_eq!(walk.measured, Measured::default());
        std::fs::remove_dir_all(&root).ok();
    }
}
