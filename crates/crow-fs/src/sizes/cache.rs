//! The folder-size cache: versioned JSON, keyed by path, with the inode and the
//! FSEvents id it was measured at. Losing it only costs a re-measure.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::Measured;

const VERSION: u32 = 1;

/// Keep the cache bounded; the least recently measured entries go first.
pub const MAX_ENTRIES: usize = 20_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedSize {
    pub bytes: u64,
    pub items: u64,
    /// The folder's inode when measured, so a recreated folder is not mistaken for the old one.
    pub inode: u64,
    pub measured_at: u64,
    /// The FSEvents id when measured (0 until the stream is running).
    pub event_id: u64,
    #[serde(default)]
    pub stale: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cache {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    last_event_id: u64,
    #[serde(default)]
    volume_uuid: Option<String>,
    #[serde(default)]
    entries: BTreeMap<PathBuf, CachedSize>,
}

impl Default for Cache {
    fn default() -> Self {
        Self { version: VERSION, last_event_id: 0, volume_uuid: None, entries: BTreeMap::new() }
    }
}

impl Cache {
    /// Read `path`; a missing, unreadable, malformed or older/newer file is an
    /// empty cache, not an error.
    pub fn load(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        let Ok(cache) = serde_json::from_str::<Self>(&text) else {
            return Self::default();
        };
        if cache.version != VERSION {
            return Self::default();
        }
        cache
    }

    /// Write `path` atomically (temp file, then rename), creating its folder.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string(self).map_err(io::Error::other)?;
        let temp = path.with_extension(format!("tmp-{}", std::process::id()));
        std::fs::write(&temp, text)?;
        std::fs::rename(&temp, path)
    }

    pub fn get(&self, path: &Path) -> Option<&CachedSize> {
        self.entries.get(path)
    }

    pub fn entries(&self) -> impl Iterator<Item = (&Path, &CachedSize)> {
        self.entries.iter().map(|(path, entry)| (path.as_path(), entry))
    }

    /// Store a measurement. A path that is not valid UTF-8 is skipped: the cache
    /// is JSON, and losing one entry is better than losing the file.
    pub fn insert(&mut self, path: &Path, inode: u64, measured: &Measured, event_id: u64, now: u64) -> bool {
        let Some(key) = path.to_str() else {
            return false;
        };
        self.entries.insert(
            PathBuf::from(key),
            CachedSize {
                bytes: measured.bytes,
                items: measured.items,
                inode,
                measured_at: now,
                event_id,
                stale: false,
            },
        );
        true
    }

    /// Stat every entry and mark the ones whose folder was replaced (a different
    /// inode), is no longer a folder, or is gone. Run off the main thread after
    /// `load`: a recreated folder must not show the old folder's size.
    pub fn validate_inodes(&mut self) {
        let mut replaced = Vec::new();
        for (path, entry) in &self.entries {
            let inode =
                std::fs::symlink_metadata(path).ok().filter(|meta| meta.is_dir()).map_or(0, |meta| inode(&meta));
            if inode != entry.inode {
                replaced.push(path.clone());
            }
        }
        for path in replaced {
            if let Some(entry) = self.entries.get_mut(&path) {
                entry.stale = true;
            }
        }
    }

    /// Mark `path` and every ancestor that has an entry stale.
    pub fn mark_stale(&mut self, path: &Path) {
        for ancestor in path.ancestors() {
            if let Some(entry) = self.entries.get_mut(ancestor) {
                entry.stale = true;
            }
        }
    }

    /// `MustScanSubDirs`: mark everything below `path`, then its ancestors.
    pub fn mark_subtree_stale(&mut self, path: &Path) {
        for (key, entry) in &mut self.entries {
            if key.starts_with(path) {
                entry.stale = true;
            }
        }
        self.mark_stale(path);
    }

    /// Drop the least recently measured entries until at most `max` remain.
    pub fn prune(&mut self, max: usize) {
        if self.entries.len() <= max {
            return;
        }
        let mut order: Vec<(u64, PathBuf)> =
            self.entries.iter().map(|(path, entry)| (entry.measured_at, path.clone())).collect();
        order.sort();
        for (_, path) in order.into_iter().take(self.entries.len() - max) {
            self.entries.remove(&path);
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn last_event_id(&self) -> u64 {
        self.last_event_id
    }

    pub fn set_last_event_id(&mut self, id: u64) {
        self.last_event_id = id;
    }

    pub fn volume_uuid(&self) -> Option<&str> {
        self.volume_uuid.as_deref()
    }

    pub fn set_volume_uuid(&mut self, uuid: Option<String>) {
        self.volume_uuid = uuid;
    }

    /// The event store was replaced (a new volume UUID): the history is gone.
    pub fn reset_history(&mut self) {
        self.entries.clear();
        self.last_event_id = 0;
        // The caller stores the current UUID after the reset; keeping the old
        // one would reset the cache again on every launch.
        self.volume_uuid = None;
    }
}

#[cfg(unix)]
fn inode(meta: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.ino()
}

#[cfg(not(unix))]
fn inode(_: &std::fs::Metadata) -> u64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> crate::test_support::TempDir {
        crate::test_support::TempDir::new(&format!("crow-fs-cache-{name}-"))
    }

    fn measured(bytes: u64, items: u64, folders: u64) -> Measured {
        Measured { bytes, items, folders, unreadable: 0, hardlinks: 0 }
    }

    #[test]
    fn round_trip() {
        let dir = temp_dir("round-trip");
        let path = dir.join("folder-sizes.json");
        let mut cache = Cache::default();
        cache.insert(Path::new("/a"), 11, &measured(100, 2, 1), 42, 1000);
        cache.insert(Path::new("/a/b"), 12, &measured(50, 1, 0), 43, 1001);
        cache.set_last_event_id(43);
        cache.set_volume_uuid(Some("uuid".into()));
        cache.save(&path).unwrap();
        let loaded = Cache::load(&path);
        assert_eq!(loaded, cache);
        assert_eq!(loaded.last_event_id(), 43);
        assert_eq!(loaded.volume_uuid(), Some("uuid"));
    }

    #[test]
    fn version_mismatch_is_ignored() {
        let dir = temp_dir("version");
        let path = dir.join("folder-sizes.json");
        std::fs::write(&path, r#"{"version":999,"entries":{"/a":{"bytes":1,"items":1,"folders":0,"unreadable":0,"hardlinks":0,"inode":1,"measured_at":1,"used_at":1,"event_id":0}}}"#).unwrap();
        let cache = Cache::load(&path);
        assert!(cache.is_empty());
        assert_eq!(cache.last_event_id(), 0);
    }

    #[test]
    fn malformed_file_is_ignored() {
        let dir = temp_dir("malformed");
        let path = dir.join("folder-sizes.json");
        std::fs::write(&path, "{not json").unwrap();
        assert!(Cache::load(&path).is_empty());
    }

    #[test]
    fn validation_marks_a_replaced_or_missing_folder_stale() {
        use std::os::unix::fs::MetadataExt;
        let dir = temp_dir("validate");
        let folder = dir.join("folder");
        std::fs::create_dir(&folder).unwrap();
        let actual = std::fs::metadata(&folder).unwrap().ino();
        let mut same = Cache::default();
        same.insert(&folder, actual, &measured(10, 1, 0), 0, 1);
        same.validate_inodes();
        assert!(!same.get(&folder).unwrap().stale, "same inode");
        let mut wrong = Cache::default();
        wrong.insert(&folder, actual + 1, &measured(10, 1, 0), 0, 1);
        wrong.validate_inodes();
        assert!(wrong.get(&folder).unwrap().stale, "a different inode is a recreated folder");
        let mut missing = Cache::default();
        missing.insert(&dir.join("gone"), 1, &measured(10, 1, 0), 0, 1);
        missing.validate_inodes();
        assert!(missing.get(&dir.join("gone")).unwrap().stale, "a missing folder");
    }

    #[test]
    fn bound_drops_the_least_recently_measured() {
        let mut cache = Cache::default();
        for i in 0..5u64 {
            cache.insert(Path::new(&format!("/p{i}")), i, &measured(i, 0, 0), 0, i);
        }
        cache.prune(3);
        assert_eq!(cache.len(), 3);
        assert!(cache.get(Path::new("/p0")).is_none());
        assert!(cache.get(Path::new("/p1")).is_none());
        assert!(cache.get(Path::new("/p4")).is_some());
    }

    #[test]
    fn marking_a_folder_stale_marks_its_ancestors() {
        let mut cache = Cache::default();
        for path in ["/a", "/a/b", "/a/b/c", "/x"] {
            cache.insert(Path::new(path), 1, &measured(1, 1, 0), 0, 1);
        }
        cache.mark_stale(Path::new("/a/b/c"));
        assert!(cache.get(Path::new("/a/b/c")).unwrap().stale);
        assert!(cache.get(Path::new("/a/b")).unwrap().stale);
        assert!(cache.get(Path::new("/a")).unwrap().stale);
        assert!(!cache.get(Path::new("/x")).unwrap().stale);
    }

    #[test]
    fn a_subtree_scan_marks_only_that_subtree() {
        let mut cache = Cache::default();
        for path in ["/a", "/a/b", "/a/b/c", "/a/d", "/x"] {
            cache.insert(Path::new(path), 1, &measured(1, 1, 0), 0, 1);
        }
        cache.mark_subtree_stale(Path::new("/a/b"));
        assert!(cache.get(Path::new("/a/b")).unwrap().stale);
        assert!(cache.get(Path::new("/a/b/c")).unwrap().stale);
        assert!(!cache.get(Path::new("/a/d")).unwrap().stale);
        assert!(cache.get(Path::new("/a")).unwrap().stale, "the parent's size included the subtree");
        assert!(!cache.get(Path::new("/x")).unwrap().stale);
    }

    #[test]
    fn a_non_utf8_path_is_skipped_without_losing_the_file() {
        use std::os::unix::ffi::OsStrExt;
        let dir = temp_dir("non-utf8");
        let path = dir.join("folder-sizes.json");
        let mut cache = Cache::default();
        let odd = Path::new(std::ffi::OsStr::from_bytes(b"/tmp/\xff"));
        assert!(!cache.insert(odd, 1, &measured(1, 1, 0), 0, 1));
        cache.insert(Path::new("/ok"), 2, &measured(2, 1, 0), 0, 1);
        cache.save(&path).unwrap();
        let loaded = Cache::load(&path);
        assert_eq!(loaded.len(), 1);
        assert!(loaded.get(Path::new("/ok")).is_some());
    }

    #[test]
    fn reset_history_forgets_the_uuid_so_the_next_launch_keeps_the_new_one() {
        let mut cache = Cache::default();
        cache.set_volume_uuid(Some("old".into()));
        cache.insert(Path::new("/a"), 1, &measured(10, 1, 0), 42, 1);
        cache.set_last_event_id(42);
        cache.reset_history();
        assert!(cache.is_empty());
        assert_eq!(cache.last_event_id(), 0);
        assert_eq!(cache.volume_uuid(), None, "the caller stores the current UUID");
    }
}
