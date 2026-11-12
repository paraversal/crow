use std::collections::HashSet;
use std::hash::Hash;
use std::time::{Duration, Instant};

/// Finder-style multiple selection, keyed by identity (paths in the app) rather
/// than position, so it survives re-sorting and live folder changes.
///
/// - `cursor`: the item keyboard moves start from, and the one the preview shows.
/// - `anchor`: where a Shift-extended range starts.
#[derive(Clone, Debug)]
pub struct Selection<K> {
    selected: HashSet<K>,
    cursor: Option<K>,
    anchor: Option<K>,
}

impl<K> Default for Selection<K> {
    fn default() -> Self {
        Self { selected: HashSet::new(), cursor: None, anchor: None }
    }
}

impl<K: Clone + Eq + Hash> Selection<K> {
    pub fn cursor(&self) -> Option<&K> {
        self.cursor.as_ref()
    }

    pub fn contains(&self, key: &K) -> bool {
        self.selected.contains(key)
    }

    pub fn len(&self) -> usize {
        self.selected.len()
    }

    pub fn is_empty(&self) -> bool {
        self.selected.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &K> {
        self.selected.iter()
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Plain click or arrow key: just this item.
    pub fn select_only(&mut self, key: K) {
        self.selected.clear();
        self.selected.insert(key.clone());
        self.anchor = Some(key.clone());
        self.cursor = Some(key);
    }

    /// Cmd-click: add or remove one item. An added item becomes the anchor; removing
    /// one keeps the anchor unless it was the anchor, then any remaining item takes over.
    pub fn toggle(&mut self, key: K) {
        if self.selected.remove(&key) {
            if self.anchor.as_ref() == Some(&key) {
                self.anchor = self.selected.iter().next().cloned();
            }
            self.cursor = self.anchor.clone();
        } else {
            self.selected.insert(key.clone());
            self.anchor = Some(key.clone());
            self.cursor = Some(key);
        }
    }

    /// Shift-click or Shift-arrow: select from the anchor to `target` in display order,
    /// replacing any previous Shift range. `position` finds a key's row; `key_at` reads one.
    pub fn extend_to(&mut self, target: usize, position: impl Fn(&K) -> Option<usize>, key_at: impl Fn(usize) -> K) {
        let anchor = self.anchor.as_ref().and_then(&position).unwrap_or(target);
        let (from, to) = (anchor.min(target), anchor.max(target));
        self.selected = (from..=to).map(&key_at).collect();
        self.cursor = Some(key_at(target));
        if self.anchor.is_none() {
            self.anchor = self.cursor.clone();
        }
    }

    /// Cmd-A.
    pub fn select_all(&mut self, keys: impl IntoIterator<Item = K>) {
        self.selected = keys.into_iter().collect();
    }

    /// Drop items that no longer exist (after a folder change). The cursor and anchor
    /// fall back to any remaining selected item.
    pub fn retain(&mut self, exists: impl Fn(&K) -> bool) {
        self.selected.retain(|k| exists(k));
        let fallback = || self.selected.iter().next().cloned();
        if self.cursor.as_ref().is_some_and(|k| !exists(k)) {
            self.cursor = fallback();
        }
        if self.anchor.as_ref().is_some_and(|k| !exists(k)) {
            self.anchor = self.cursor.clone();
        }
    }
}

/// Finder's type-to-select: letters typed in quick succession build a prefix,
/// and the first matching name is selected.
#[derive(Clone, Debug, Default)]
pub struct TypeSelect {
    buffer: String,
    last: Option<Instant>,
}

impl TypeSelect {
    /// Pause after which typing starts a new search (Finder's is about one second).
    pub const TIMEOUT: Duration = Duration::from_millis(1000);

    /// Add a typed character; returns the current prefix (lowercased).
    pub fn push(&mut self, c: char, now: Instant) -> &str {
        if self.last.is_none_or(|t| now.duration_since(t) > Self::TIMEOUT) {
            self.buffer.clear();
        }
        self.last = Some(now);
        self.buffer.extend(c.to_lowercase());
        &self.buffer
    }

    /// Whether a typing run is in progress (a space then continues it instead of meaning Quick Look).
    pub fn is_active(&self, now: Instant) -> bool {
        !self.buffer.is_empty() && self.last.is_some_and(|t| now.duration_since(t) <= Self::TIMEOUT)
    }

    /// The first name starting with `prefix`; otherwise the first name that sorts
    /// after it, like Finder, so typing always lands somewhere sensible.
    pub fn find<'a>(prefix: &str, names: impl IntoIterator<Item = &'a str>) -> Option<usize> {
        let mut after = None;
        for (i, name) in names.into_iter().enumerate() {
            let lower = name.to_lowercase();
            if lower.starts_with(prefix) {
                return Some(i);
            }
            if after.is_none() && crate::natural_order(&lower, prefix).is_gt() {
                after = Some(i);
            }
        }
        after
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> Vec<&'static str> {
        vec!["a", "b", "c", "d", "e"]
    }

    fn extend(s: &mut Selection<&'static str>, target: usize) {
        let k = keys();
        s.extend_to(target, |key| k.iter().position(|x| x == key), |i| k[i]);
    }

    fn sorted(s: &Selection<&'static str>) -> Vec<&'static str> {
        let mut v: Vec<_> = s.iter().copied().collect();
        v.sort_unstable();
        v
    }

    #[test]
    fn shift_ranges_replace_each_other_from_the_anchor() {
        let mut s = Selection::default();
        s.select_only("b");
        extend(&mut s, 3);
        assert_eq!(sorted(&s), ["b", "c", "d"]);
        extend(&mut s, 0);
        assert_eq!(sorted(&s), ["a", "b"], "the range pivots on the anchor");
        assert_eq!(s.cursor(), Some(&"a"));
    }

    #[test]
    fn cmd_click_toggles_and_moves_the_anchor() {
        let mut s = Selection::default();
        s.select_only("a");
        s.toggle("c");
        assert_eq!(sorted(&s), ["a", "c"]);
        s.toggle("a");
        assert_eq!(sorted(&s), ["c"]);
        extend(&mut s, 4);
        assert_eq!(sorted(&s), ["a", "c", "d", "e"][1..].to_vec(), "range from the toggled item");
    }

    #[test]
    fn vanished_items_drop_out() {
        let mut s = Selection::default();
        s.select_only("b");
        s.toggle("d");
        s.retain(|k| *k != "d");
        assert_eq!(sorted(&s), ["b"]);
        assert_eq!(s.cursor(), Some(&"b"));
    }

    #[test]
    fn type_select_builds_a_prefix_and_resets_after_a_pause() {
        let t0 = Instant::now();
        let mut t = TypeSelect::default();
        assert_eq!(t.push('R', t0), "r");
        assert_eq!(t.push('e', t0 + Duration::from_millis(300)), "re");
        assert_eq!(t.push('x', t0 + Duration::from_millis(2000)), "x");
    }

    #[test]
    fn type_select_finds_prefix_or_next() {
        let names = ["Applications", "Desktop", "Documents", "Downloads", "Music"];
        assert_eq!(TypeSelect::find("do", names), Some(2));
        assert_eq!(TypeSelect::find("dow", names), Some(3));
        assert_eq!(TypeSelect::find("e", names), Some(4), "nothing starts with e: next name after it");
        assert_eq!(TypeSelect::find("z", names), None);
    }
}
