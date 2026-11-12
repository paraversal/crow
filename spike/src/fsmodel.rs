//! Folder snapshots: read a directory into a sorted, immutable list.

use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime};

#[derive(Clone, Debug)]
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
}

pub struct Snapshot {
    pub dir: PathBuf,
    pub entries: Arc<Vec<Entry>>,
    pub load_ms: f64,
}

/// Read `dir`, folders first, then natural name order (`file2` before `file10`).
/// Unreadable entries are skipped rather than failing the whole folder.
pub fn read_dir(dir: &Path, show_hidden: bool) -> std::io::Result<Snapshot> {
    let start = Instant::now();
    let mut entries: Vec<Entry> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let hidden = name.starts_with('.');
            if hidden && !show_hidden {
                return None;
            }
            // symlink_metadata: never follow links while listing.
            let meta = e.metadata().ok();
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false)
                || (e.file_type().map(|t| t.is_symlink()).unwrap_or(false) && e.path().is_dir());
            Some(Entry {
                path: e.path(),
                name,
                is_dir,
                size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                modified: meta.as_ref().and_then(|m| m.modified().ok()),
                created: meta.as_ref().and_then(|m| m.created().ok()),
            })
        })
        .collect();
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| natural_cmp(&a.name, &b.name)));
    Ok(Snapshot {
        dir: dir.to_path_buf(),
        entries: Arc::new(entries),
        load_ms: start.elapsed().as_secs_f64() * 1000.0,
    })
}

/// Case-insensitive natural order; ties broken by exact bytes so the order is total.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            // Equal by natural order ("file2" vs "File02"): fall back to a plain
            // case-insensitive comparison, then exact bytes, so the order is total.
            (None, None) => return a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b)),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(&c) = it.peek() {
                        if !c.is_ascii_digit() {
                            break;
                        }
                        s.push(c);
                        it.next();
                    }
                    s
                };
                let (n, m) = (take(&mut x), take(&mut y));
                let (nt, mt) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                let ord = nt.len().cmp(&mt.len()).then_with(|| nt.cmp(mt));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(c), Some(d)) => {
                let ord = c.to_lowercase().cmp(d.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                x.next();
                y.next();
            }
        }
    }
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return format!("{bytes} bytes");
    }
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1000.0 && u < UNITS.len() - 1 {
        v /= 1000.0;
        u += 1;
    }
    format!("{:.1} {}", v, UNITS[u])
}

/// Create `count` empty files for the large-folder test.
pub fn make_fixture(dir: &Path, count: usize) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let exts = ["txt", "md", "rs", "png", "json", "pdf", "toml", "swift"];
    for i in 0..count {
        let name = format!("file-{i:06}.{}", exts[i % exts.len()]);
        fs::File::create(dir.join(name))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec!["file10", "File2", "file1", "file02", "a"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["a", "file1", "file02", "File2", "file10"]);
    }

    #[test]
    fn sizes() {
        assert_eq!(human_size(12), "12 bytes");
        assert_eq!(human_size(12_300), "12.3 KB");
    }
}
