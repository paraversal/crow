//! The shelf: files set aside to carry to another folder. It holds references to the
//! files, never copies, and lives in the state folder (`shelf.json`), so it survives a
//! restart. Nothing here touches the files themselves.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A file on the shelf, and whether it was a folder when it went there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShelfItem {
    pub path: PathBuf,
    pub is_dir: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shelf {
    items: Vec<ShelfItem>,
}

impl Shelf {
    /// The saved shelf, or an empty one when the file is missing or unreadable.
    pub fn load(path: &Path) -> Shelf {
        std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string(self).map_err(std::io::Error::other)?;
        crate::config::write_atomic(path, &text)
    }

    /// Put `paths` on the shelf after the items already there; a path already on it
    /// keeps its place. Returns how many were new.
    pub fn add(&mut self, entries: impl IntoIterator<Item = (PathBuf, bool)>) -> usize {
        let mut added = 0;
        for (path, is_dir) in entries {
            if !self.items.iter().any(|item| item.path == path) {
                self.items.push(ShelfItem { path, is_dir });
                added += 1;
            }
        }
        added
    }

    pub fn remove(&mut self, path: &Path) -> bool {
        let before = self.items.len();
        self.items.retain(|item| item.path != path);
        self.items.len() != before
    }

    /// Take `paths` off the shelf (they were moved somewhere else).
    pub fn remove_all(&mut self, paths: &[PathBuf]) {
        self.items.retain(|item| !paths.contains(&item.path));
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    pub fn items(&self) -> &[ShelfItem] {
        &self.items
    }

    pub fn paths(&self) -> Vec<PathBuf> {
        self.items.iter().map(|item| item.path.clone()).collect()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str) -> PathBuf {
        PathBuf::from(format!("/files/{name}"))
    }

    #[test]
    fn adding_keeps_order_and_ignores_duplicates() {
        let mut shelf = Shelf::default();
        assert_eq!(shelf.add([(p("a"), false), (p("b"), true)]), 2);
        assert_eq!(shelf.add([(p("b"), true), (p("c"), false)]), 1, "b is already there");
        assert_eq!(shelf.paths(), [p("a"), p("b"), p("c")]);
        assert!(shelf.items()[1].is_dir, "a folder stays a folder");
    }

    #[test]
    fn removing_one_many_or_all() {
        let mut shelf = Shelf::default();
        shelf.add(["a", "b", "c", "d"].map(|n| (p(n), false)));
        assert!(shelf.remove(&p("b")));
        assert!(!shelf.remove(&p("b")));
        shelf.remove_all(&[p("a"), p("d")]);
        assert_eq!(shelf.paths(), [p("c")]);
        shelf.clear();
        assert!(shelf.is_empty());
    }

    #[test]
    fn the_shelf_survives_a_restart_and_a_broken_file_is_an_empty_shelf() {
        let dir = std::env::temp_dir().join(format!("crow-shelf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("shelf.json");
        let mut shelf = Shelf::default();
        shelf.add([(p("a"), false), (p("b"), false)]);
        shelf.save(&file).unwrap();
        assert_eq!(Shelf::load(&file), shelf);
        std::fs::write(&file, "not json").unwrap();
        assert!(Shelf::load(&file).is_empty());
        assert!(Shelf::load(&dir.join("missing.json")).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
