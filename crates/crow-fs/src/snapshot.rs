use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use crate::natural_cmp;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub created: Option<SystemTime>,
}

impl Entry {
    pub fn ext(&self) -> &str {
        Path::new(&self.name).extension().and_then(|e| e.to_str()).unwrap_or("")
    }

    /// Application bundles and other packages open rather than navigate.
    pub fn is_package(&self) -> bool {
        self.is_dir && matches!(self.ext(), "app" | "bundle" | "framework" | "photoslibrary")
    }
}

/// An immutable, sorted listing of one folder.
pub struct Snapshot {
    pub dir: PathBuf,
    pub entries: Arc<Vec<Entry>>,
    pub load_ms: f64,
}

/// Read `dir`, folders first, then natural name order.
/// Unreadable entries are skipped rather than failing the whole folder.
pub fn read_dir(dir: &Path, show_hidden: bool) -> std::io::Result<Snapshot> {
    let start = Instant::now();
    let mut entries: Vec<Entry> = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter_map(|dirent| {
            let name = dirent.file_name().into_string().ok()?;
            if name.starts_with('.') && !show_hidden {
                return None;
            }
            let file_type = dirent.file_type().ok()?;
            // Listing never follows links, except to learn whether a link points at a folder.
            let meta = dirent.metadata().ok();
            let is_dir = file_type.is_dir() || (file_type.is_symlink() && dirent.path().is_dir());
            Some(Entry {
                path: dirent.path(),
                name,
                is_dir,
                size: meta.as_ref().map_or(0, |m| m.len()),
                modified: meta.as_ref().and_then(|m| m.modified().ok()),
                created: meta.as_ref().and_then(|m| m.created().ok()),
            })
        })
        .collect();
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| natural_cmp(&a.name, &b.name)));
    Ok(Snapshot { dir: dir.to_path_buf(), entries: Arc::new(entries), load_ms: start.elapsed().as_secs_f64() * 1000.0 })
}

/// Create `count` empty files for large-folder tests and benchmarks.
pub fn make_fixture(dir: &Path, count: usize) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let exts = ["txt", "md", "rs", "png", "json", "pdf", "toml", "swift"];
    for i in 0..count {
        fs::File::create(dir.join(format!("file-{i:06}.{}", exts[i % exts.len()])))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("crow-fs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn folders_first_then_natural_order_and_hidden_filter() {
        let dir = temp_dir("order");
        for name in ["b10.txt", "b2.txt", ".hidden", "Zeta"] {
            fs::write(dir.join(name), "").expect("write");
        }
        fs::create_dir(dir.join("alpha")).expect("mkdir");

        let names = |hidden| -> Vec<String> {
            read_dir(&dir, hidden).expect("read").entries.iter().map(|e| e.name.clone()).collect()
        };
        assert_eq!(names(false), ["alpha", "b2.txt", "b10.txt", "Zeta"]);
        assert!(names(true).contains(&".hidden".to_string()));
        fs::remove_dir_all(&dir).ok();
    }
}
