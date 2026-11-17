//! The folder-size cache: versioned JSON, keyed by path, with the inode and the
//! FSEvents id it was measured at. Losing it only costs a re-measure.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::Measured;

const VERSION: u32 = 1;

/// Keep the cache bounded; the least recently shown entries go first.
pub const MAX_ENTRIES: usize = 20_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedSize {
    pub bytes: u64,
    pub items: u64,
    pub folders: u64,
    pub unreadable: u64,
    /// Hard-link entries whose bytes were already counted elsewhere. A parent may
    /// combine child sizes only when this is zero everywhere.
    pub hardlinks: u64,
    /// The folder's inode when measured, so a recreated folder is not mistaken for the old one.
    pub inode: u64,
    pub measured_at: u64,
    /// Last time a row showed this entry; the bound drops the smallest.
    pub used_at: u64,
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

    /// A usable entry: not stale, and the folder still has the inode it had when
    /// measured.
    pub fn fresh(&self, path: &Path, inode: u64) -> Option<&CachedSize> {
        self.get(path).filter(|entry| !entry.stale && entry.inode == inode)
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
                folders: measured.folders,
                unreadable: measured.unreadable,
                hardlinks: measured.hardlinks,
                inode,
                measured_at: now,
                used_at: now,
                event_id,
                stale: false,
            },
        );
        true
    }

    /// Mark `path` as shown now, so the bound keeps it.
    pub fn touch(&mut self, path: &Path, now: u64) {
        if let Some(entry) = self.entries.get_mut(path) {
            entry.used_at = now;
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

    /// Drop the least recently shown entries until at most `max` remain.
    pub fn prune(&mut self, max: usize) {
        if self.entries.len() <= max {
            return;
        }
        let mut order: Vec<(u64, PathBuf)> =
            self.entries.iter().map(|(path, entry)| (entry.used_at, path.clone())).collect();
        order.sort();
        for (_, path) in order.into_iter().take(self.entries.len() - max) {
            self.entries.remove(&path);
        }
    }

    /// A parent's size from its own direct files and its cached child folders.
    /// `own` must count only the parent's direct non-folder entries; each child
    /// folder is added as one item and one folder. `None` when a child is missing,
    /// stale, or has hard links (which could cross folders and be counted twice).
    pub fn combined_size(&self, own: &Measured, children: &[PathBuf]) -> Option<Measured> {
        if own.hardlinks > 0 {
            return None;
        }
        let mut total = own.clone();
        for child in children {
            let entry = self.entries.get(child)?;
            if entry.stale || entry.hardlinks > 0 {
                return None;
            }
            total.bytes += entry.bytes;
            total.items += 1 + entry.items;
            total.folders += 1 + entry.folders;
            total.unreadable += entry.unreadable;
        }
        Some(total)
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("crow-fs-cache-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
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
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn version_mismatch_is_ignored() {
        let dir = temp_dir("version");
        let path = dir.join("folder-sizes.json");
        std::fs::write(&path, r#"{"version":999,"entries":{"/a":{"bytes":1,"items":1,"folders":0,"unreadable":0,"hardlinks":0,"inode":1,"measured_at":1,"used_at":1,"event_id":0}}}"#).unwrap();
        let cache = Cache::load(&path);
        assert!(cache.is_empty());
        assert_eq!(cache.last_event_id(), 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn malformed_file_is_ignored() {
        let dir = temp_dir("malformed");
        let path = dir.join("folder-sizes.json");
        std::fs::write(&path, "{not json").unwrap();
        assert!(Cache::load(&path).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn inode_change_is_stale() {
        let mut cache = Cache::default();
        cache.insert(Path::new("/a"), 1, &measured(10, 1, 0), 0, 1);
        assert!(cache.fresh(Path::new("/a"), 1).is_some());
        assert!(cache.fresh(Path::new("/a"), 2).is_none());
        std::fs::remove_dir_all(std::env::temp_dir().join("crow-fs-cache-inode-change")).ok();
    }

    #[test]
    fn bound_drops_the_least_recently_used() {
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
    fn touch_moves_an_entry_out_of_the_bound() {
        let mut cache = Cache::default();
        for i in 0..3u64 {
            cache.insert(Path::new(&format!("/p{i}")), i, &measured(i, 0, 0), 0, i);
        }
        cache.touch(Path::new("/p0"), 10);
        cache.prune(2);
        assert!(cache.get(Path::new("/p0")).is_some());
        assert!(cache.get(Path::new("/p1")).is_none());
        assert!(cache.get(Path::new("/p2")).is_some());
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
    fn a_parent_combines_fresh_children() {
        let mut cache = Cache::default();
        cache.insert(Path::new("/p/a"), 1, &measured(10, 1, 0), 0, 1);
        cache.insert(Path::new("/p/b"), 2, &measured(20, 2, 1), 0, 1);
        let own = measured(5, 1, 0);
        let combined = cache
            .combined_size(&own, &[PathBuf::from("/p/a"), PathBuf::from("/p/b")])
            .expect("both children are fresh");
        assert_eq!(combined.bytes, 35);
        assert_eq!(combined.items, 1 + 2 + 3, "own file, two child folders, their items");
        assert_eq!(combined.folders, 2 + 1, "two child folders and b's child");
    }

    #[test]
    fn a_parent_refuses_stale_or_hardlinked_children() {
        let mut cache = Cache::default();
        cache.insert(Path::new("/p/a"), 1, &measured(10, 1, 0), 0, 1);
        cache.insert(Path::new("/p/b"), 2, &measured(20, 2, 1), 0, 1);
        cache.mark_stale(Path::new("/p/a"));
        assert!(cache.combined_size(&measured(0, 0, 0), &[PathBuf::from("/p/a")]).is_none());
        assert!(cache.combined_size(&measured(0, 0, 0), &[PathBuf::from("/missing")]).is_none());
        let mut own = measured(0, 0, 0);
        own.hardlinks = 1;
        assert!(cache.combined_size(&own, &[PathBuf::from("/p/b")]).is_none());
        let mut hardlinked = measured(1, 1, 0);
        hardlinked.hardlinks = 1;
        cache.insert(Path::new("/p/c"), 3, &hardlinked, 0, 1);
        assert!(cache.combined_size(&measured(0, 0, 0), &[PathBuf::from("/p/c")]).is_none());
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
        std::fs::remove_dir_all(&dir).ok();
    }
}
