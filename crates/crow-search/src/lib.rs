//! Search for Crow: directory walking (`ignore`), fuzzy matching (`nucleo`),
//! content search (`grep-searcher`) and exclusion rule sets (`globset` plus
//! sentinel files).

mod engine;
mod predicates;
mod rules;

pub mod corpus;

#[cfg(test)]
mod oracle_tests;

pub use engine::{Control, FuzzyMatcher, Hit, HitKind, MAX_METADATA_RESULTS, Query, Skip, SkipReason, Summary, search};
pub use predicates::{Kind, Predicates};
pub use rules::RuleSet;

/// Owned temporary trees for tests: `CleanTree` also restores mode-000 folders
/// before deleting, including during unwinding.
#[cfg(test)]
pub(crate) mod test_support {
    use std::ops::Deref;
    use std::path::{Path, PathBuf};

    pub(crate) struct TempDir {
        path: PathBuf,
        _guard: tempfile::TempDir,
    }

    impl TempDir {
        pub(crate) fn new(prefix: &str) -> Self {
            let guard = tempfile::Builder::new().prefix(prefix).tempdir().expect("temp dir");
            let path = guard.path().canonicalize().expect("canonical temp dir");
            std::fs::create_dir_all(&path).expect("create temp dir");
            Self { path, _guard: guard }
        }
    }

    impl Deref for TempDir {
        type Target = Path;

        fn deref(&self) -> &Path {
            &self.path
        }
    }

    impl AsRef<Path> for TempDir {
        fn as_ref(&self) -> &Path {
            &self.path
        }
    }

    pub(crate) struct CleanTree(TempDir);

    impl CleanTree {
        pub(crate) fn new(prefix: &str) -> Self {
            Self(TempDir::new(prefix))
        }
    }

    impl Drop for CleanTree {
        fn drop(&mut self) {
            let _ = remove_tree(&self.0.path);
        }
    }

    impl Deref for CleanTree {
        type Target = Path;

        fn deref(&self) -> &Path {
            &self.0.path
        }
    }

    impl AsRef<Path> for CleanTree {
        fn as_ref(&self) -> &Path {
            &self.0.path
        }
    }

    /// Remove a tree that may hold a mode-000 folder: restore directory modes first.
    pub(crate) fn remove_tree(root: &Path) -> std::io::Result<()> {
        if std::fs::symlink_metadata(root).is_err() {
            return Ok(());
        }
        restore_modes(root);
        std::fs::remove_dir_all(root)
    }

    fn restore_modes(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let Ok(metadata) = std::fs::symlink_metadata(path) else {
            return;
        };
        if metadata.file_type().is_symlink() {
            return;
        }
        if metadata.is_dir() {
            if metadata.permissions().mode() & 0o700 != 0o700 {
                let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
            }
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    restore_modes(&entry.path());
                }
            }
        }
    }
}
