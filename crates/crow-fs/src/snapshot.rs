use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::{Entry, Sort, compare, list_dir, sort_entries, stat_entry};

/// An immutable, sorted listing of one folder.
pub struct Snapshot {
    pub dir: PathBuf,
    pub entries: Arc<Vec<Entry>>,
    pub load_ms: f64,
}

/// Kind names come from the system (UTType) and are cached by the caller;
/// sorting by kind asks for them through this.
pub type KindOf<'a> = &'a (dyn Fn(&Entry) -> Arc<str> + Sync);

/// List and sort `dir`.
pub fn read_dir(dir: &Path, show_hidden: bool, sort: &Sort, kind: KindOf) -> std::io::Result<Snapshot> {
    let start = Instant::now();
    let mut entries = Vec::new();
    list_dir(dir, show_hidden, |batch| entries.extend(batch))?;
    sort_entries(&mut entries, sort, kind);
    Ok(Snapshot { dir: dir.to_path_buf(), entries: Arc::new(entries), load_ms: start.elapsed().as_secs_f64() * 1000.0 })
}

/// Stat watcher-reported paths off the main thread. Only paths directly inside
/// `dir` are kept; `None` means the path is gone (or hidden and not shown).
/// A path reported through a symlinked ancestor (`/tmp` for `/private/tmp`) is
/// rewritten into `dir`'s form, so the merge can match it against the listing.
pub fn stat_changes(dir: &Path, changed: &[PathBuf], show_hidden: bool) -> Vec<(PathBuf, Option<Entry>)> {
    // `dir` came from the pane and may name a symlinked ancestor (`/tmp`), while
    // the watcher reports resolved paths (`/private/tmp`); resolve it once so both
    // directions of that mismatch match.
    let canonical_dir = dir.canonicalize().ok();
    let mut seen = HashSet::new();
    changed
        .iter()
        .filter_map(|path| {
            let name = path.file_name()?;
            let inside = match path.parent() {
                Some(parent) => {
                    parent == dir
                        || canonical_dir.as_deref().is_some_and(|canonical_dir| {
                            parent == canonical_dir || parent.canonicalize().is_ok_and(|parent| parent == canonical_dir)
                        })
                }
                None => false,
            };
            if !inside {
                return None;
            }
            let path = dir.join(name);
            // A burst can name the same path more than once; stat it once.
            if !seen.insert(path.clone()) {
                return None;
            }
            let entry = stat_entry(&path).ok().flatten().filter(|entry| show_hidden || !entry.hidden);
            Some((path, entry))
        })
        .collect()
}

/// Merge `stat_changes` results into the folder's current listing. This runs on
/// the main thread against `entries` as they are *now*: two batches that land out
/// of order cannot replay an older listing (a deleted file stays deleted).
pub fn apply_stats(entries: &[Entry], stats: &[(PathBuf, Option<Entry>)], sort: &Sort, kind: KindOf) -> Vec<Entry> {
    // One pass over the listing and one over the sorted changes: this runs on the
    // main thread, so a burst of changes in a 100k folder must stay linear.
    let touched: HashSet<&PathBuf> = stats.iter().map(|(path, _)| path).collect();
    let mut found: Vec<&Entry> = stats.iter().filter_map(|(_, entry)| entry.as_ref()).collect();
    found.sort_by(|a, b| compare(a, b, sort, kind));
    let mut found = found.into_iter().peekable();
    let mut out: Vec<Entry> = Vec::with_capacity(entries.len() + found.len());
    for existing in entries.iter().filter(|entry| !touched.contains(&entry.path)) {
        while let Some(new) = found.next_if(|new| compare(existing, new, sort, kind).is_ge()) {
            out.push(new.clone());
        }
        out.push(existing.clone());
    }
    out.extend(found.cloned());
    out
}

/// Apply watcher-reported changes to a sorted listing without relisting the folder:
/// each changed path directly inside `dir` is re-read and removed, updated or
/// inserted at its sorted position. Paths elsewhere are ignored.
pub fn apply_changes(
    entries: &[Entry],
    dir: &Path,
    changed: &[PathBuf],
    show_hidden: bool,
    sort: &Sort,
    kind: KindOf,
) -> Vec<Entry> {
    let stats = stat_changes(dir, changed, show_hidden);
    apply_stats(entries, &stats, sort, kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SortKey;
    use std::fs;

    fn kind(e: &Entry) -> Arc<str> {
        Arc::from(if e.is_dir { "Folder" } else { e.ext() })
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("crow-fs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir.canonicalize().expect("canonical")
    }

    fn names(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn apply_stats_merges_inserts_updates_and_removals_in_order() {
        let entry = |name: &str, size: u64| Entry {
            name: name.into(),
            path: PathBuf::from("/d").join(name),
            is_dir: false,
            is_symlink: false,
            hidden: false,
            size,
            modified: None,
            created: None,
            added: None,
        };
        let sort = Sort::default();
        let entries = vec![entry("b", 1), entry("d", 1), entry("f", 1)];
        let stats = vec![
            (PathBuf::from("/d/g"), Some(entry("g", 1))),
            (PathBuf::from("/d/a"), Some(entry("a", 1))),
            (PathBuf::from("/d/d"), None),
            (PathBuf::from("/d/f"), Some(entry("f", 9))),
            (PathBuf::from("/d/c"), Some(entry("c", 1))),
        ];
        let out = apply_stats(&entries, &stats, &sort, &kind);
        assert_eq!(names(&out), ["a", "b", "c", "f", "g"]);
        assert_eq!(out[3].size, 9, "an updated entry replaces the old one");
    }

    #[test]
    fn changes_through_a_symlinked_ancestor_still_match() {
        let dir = temp_dir("symlink-change");
        fs::write(dir.join("a.txt"), "a").expect("write");
        let alias = dir.parent().unwrap().join(format!("alias-{}", std::process::id()));
        let _ = fs::remove_file(&alias);
        std::os::unix::fs::symlink(&dir, &alias).expect("symlink");
        let canonical = dir.canonicalize().expect("canonical");

        // The watcher may name the folder through the link (/tmp for /private/tmp).
        let stats = stat_changes(&canonical, &[alias.join("a.txt")], false);
        assert_eq!(stats.len(), 1, "the path is inside the folder");
        assert_eq!(stats[0].0, canonical.join("a.txt"), "rewritten into the listing's form");
        assert!(stats[0].1.is_some(), "and stat'ed there");

        // The reverse holds too: the pane opened through the link, the watcher
        // resolved the path.
        let stats = stat_changes(&alias, &[canonical.join("a.txt")], false);
        assert_eq!(stats.len(), 1, "the resolved path is inside the linked folder");
        assert_eq!(stats[0].0, alias.join("a.txt"), "rewritten into the pane's form");

        // A burst can name the same path twice; it must appear once.
        let twice = stat_changes(&canonical, &[alias.join("a.txt"), alias.join("a.txt")], false);
        assert_eq!(twice.len(), 1, "{twice:?}");

        // A sibling folder's change is ignored.
        let other = dir.parent().unwrap().join(format!("other-{}", std::process::id()));
        fs::create_dir_all(&other).expect("mkdir");
        assert!(stat_changes(&canonical, &[other.join("b.txt")], false).is_empty());
        fs::remove_file(&alias).ok();
        fs::remove_dir_all(&other).ok();
    }

    #[test]
    fn folders_first_then_natural_order_and_hidden_filter() {
        let dir = temp_dir("order");
        for name in ["b10.txt", "b2.txt", ".hidden", "Zeta"] {
            fs::write(dir.join(name), "").expect("write");
        }
        fs::create_dir(dir.join("alpha")).expect("mkdir");
        let sort = Sort::default();
        let listing = |hidden| read_dir(&dir, hidden, &sort, &kind).expect("read").entries;
        assert_eq!(names(&listing(false)), ["alpha", "b2.txt", "b10.txt", "Zeta"]);
        assert!(names(&listing(true)).contains(&".hidden"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn changes_are_applied_in_sorted_position() {
        let dir = temp_dir("diff");
        for name in ["a.txt", "c.txt", "e.txt"] {
            fs::write(dir.join(name), "").unwrap();
        }
        let sort = Sort::default();
        let before = read_dir(&dir, false, &sort, &kind).unwrap().entries;

        fs::write(dir.join("d.txt"), "new").unwrap();
        fs::remove_file(dir.join("a.txt")).unwrap();
        fs::write(dir.join("c.txt"), "grown").unwrap();
        let changed = [dir.join("d.txt"), dir.join("a.txt"), dir.join("c.txt"), PathBuf::from("/elsewhere/x")];
        let after = apply_changes(&before, &dir, &changed, false, &sort, &kind);

        assert_eq!(names(&after), ["c.txt", "d.txt", "e.txt"]);
        assert_eq!(after[0].size, 5, "updated entries are re-read");
        let relisted = read_dir(&dir, false, &sort, &kind).unwrap().entries;
        assert_eq!(after, *relisted, "a diff gives the same result as a full listing");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn diffs_respect_the_current_sort() {
        let dir = temp_dir("diff-sort");
        fs::write(dir.join("small"), "1").unwrap();
        fs::write(dir.join("large"), "1234567").unwrap();
        let sort = Sort { key: SortKey::Size, ascending: false, folders_first: true };
        let before = read_dir(&dir, false, &sort, &kind).unwrap().entries;
        fs::write(dir.join("medium"), "1234").unwrap();
        let after = apply_changes(&before, &dir, &[dir.join("medium")], false, &sort, &kind);
        assert_eq!(names(&after), ["large", "medium", "small"]);
        fs::remove_dir_all(&dir).ok();
    }
}
