//! Emptying the system Trash: permanent by definition, so no undo record.
//! The app gives the folders (the home Trash and each volume's `.Trashes`); this
//! module removes their contents and reports what it could not remove.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub path: PathBuf,
    pub error: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Top-level items removed; a folder counts once, not once per file.
    pub removed: usize,
    /// Items (and unreadable Trash folders) that stayed.
    pub failed: Vec<Failure>,
}

impl Report {
    /// Nothing was there: no item removed and nothing failed.
    pub fn is_empty(&self) -> bool {
        self.removed == 0 && self.failed.is_empty()
    }
}

/// What the cheap pre-check found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CountReport {
    /// Items in the folders that could be read.
    pub items: usize,
    /// Folders macOS refused to list (`EPERM` from TCC without Full Disk
    /// Access, or `EACCES`). Their contents are unknown, never "empty".
    pub denied: Vec<PathBuf>,
}

/// The number of items the Trash folders hold. A missing folder counts zero; a
/// folder that cannot be listed because of permissions is reported in `denied`
/// instead of being treated as empty. This is the cheap check before asking.
pub fn count(folders: &[PathBuf]) -> CountReport {
    let mut report = CountReport::default();
    for folder in folders {
        match fs::read_dir(folder) {
            Ok(entries) => report.items += entries.filter_map(Result::ok).count(),
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => report.denied.push(folder.clone()),
            Err(_) => {}
        }
    }
    report
}

/// Remove every item in `folders`. A missing folder is skipped; an item that
/// cannot be removed is reported and the rest continue. Symlinks are removed,
/// never followed.
pub fn empty(folders: &[PathBuf]) -> Report {
    let mut report = Report::default();
    for folder in folders {
        let entries = match fs::read_dir(folder) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => {
                report.failed.push(Failure { path: folder.clone(), error: e.to_string() });
                continue;
            }
        };
        for entry in entries {
            let path = match entry {
                Ok(entry) => entry.path(),
                Err(e) => {
                    report.failed.push(Failure { path: folder.clone(), error: e.to_string() });
                    continue;
                }
            };
            match remove_item(&path) {
                Ok(()) => report.removed += 1,
                // Gone between the listing and the removal: it left the Trash.
                Err(e) if e.kind() == io::ErrorKind::NotFound => report.removed += 1,
                Err(e) => report.failed.push(Failure { path, error: e.to_string() }),
            }
        }
    }
    report
}

fn remove_item(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() { fs::remove_dir_all(path) } else { fs::remove_file(path) }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn trash(name: &str) -> crate::test_support::CleanTree {
        crate::test_support::CleanTree::new(&format!("crow-trash-{name}-"))
    }

    #[test]
    fn empties_files_folders_and_symlinks_without_following_them() {
        let dir = trash("all");
        let outside = crate::test_support::TempDir::new("crow-trash-outside-");
        fs::write(outside.join("keep.txt"), "keep").expect("write outside");
        std::os::unix::fs::symlink(&outside, dir.join("link")).expect("create symlink");
        fs::write(dir.join("a.txt"), "a").expect("write file");
        fs::create_dir_all(dir.join("nested/deep")).expect("create folder");
        fs::write(dir.join("nested/deep/b.txt"), "b").expect("write nested");

        assert_eq!(count(std::slice::from_ref(&dir.to_path_buf())), CountReport { items: 3, denied: Vec::new() });
        let report = empty(std::slice::from_ref(&dir.to_path_buf()));
        assert_eq!(report, Report { removed: 3, failed: Vec::new() });
        assert!(read_dir_empty(&dir));
        assert!(outside.join("keep.txt").exists(), "the symlink's target is untouched");
    }

    #[test]
    fn a_missing_folder_is_not_a_failure() {
        let root = crate::test_support::TempDir::new("crow-trash-missing-");
        let missing = root.join("missing");
        let report = empty(std::slice::from_ref(&missing));
        assert!(report.is_empty(), "{report:?}");
        assert_eq!(count(std::slice::from_ref(&missing)), CountReport::default());
    }

    #[test]
    fn an_unremovable_item_leaves_the_rest_and_is_reported() {
        let dir = trash("failure");
        fs::write(dir.join("gone.txt"), "a").expect("write file");
        let locked = dir.join("locked");
        fs::create_dir_all(&locked).expect("create folder");
        fs::write(locked.join("inside.txt"), "b").expect("write nested");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("lock folder");

        let report = empty(std::slice::from_ref(&dir.to_path_buf()));
        assert_eq!(report.removed, 1, "the unlocked item went: {report:?}");
        assert_eq!(report.failed.len(), 1, "{report:?}");
        assert_eq!(report.failed[0].path, locked);
        assert!(!dir.join("gone.txt").exists());
        assert!(locked.exists(), "the locked folder stayed");

        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("unlock");
    }

    #[test]
    fn an_unreadable_trash_is_denied_not_empty() {
        let dir = trash("denied");
        fs::write(dir.join("secret.txt"), "a").expect("write file");
        fs::set_permissions(&*dir, fs::Permissions::from_mode(0o000)).expect("lock trash");
        let report = count(std::slice::from_ref(&dir.to_path_buf()));
        fs::set_permissions(&*dir, fs::Permissions::from_mode(0o755)).expect("unlock trash");
        assert_eq!(report.items, 0);
        assert_eq!(report.denied, vec![dir.to_path_buf()], "an unreadable Trash must not look empty");
    }

    fn read_dir_empty(dir: &Path) -> bool {
        fs::read_dir(dir).expect("read trash").next().is_none()
    }
}
