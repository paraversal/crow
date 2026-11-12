/// Identifies a tab for its whole life, even as tabs around it open and close.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TabId(u64);

/// The tabs of one window, each holding a `T` (the UI keeps a pane there).
/// Never empty: closing the last tab means closing the window.
#[derive(Clone, Debug)]
pub struct Tabs<T> {
    tabs: Vec<(TabId, T)>,
    active: usize,
    next_id: u64,
}

impl<T> Tabs<T> {
    pub fn new(first: T) -> Self {
        Self { tabs: vec![(TabId(0), first)], active: 0, next_id: 1 }
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    /// Always false; present so `len` has its usual companion.
    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (TabId, &T)> {
        self.tabs.iter().map(|(id, t)| (*id, t))
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn active(&self) -> &T {
        &self.tabs[self.active].1
    }

    /// Open a tab right after the active one (Finder's placement) and activate it.
    pub fn open(&mut self, tab: T) -> TabId {
        let id = TabId(self.next_id);
        self.next_id += 1;
        self.tabs.insert(self.active + 1, (id, tab));
        self.active += 1;
        id
    }

    /// Close the active tab and return it. Returns `None` when it is the last
    /// one, which stays open: the caller closes the window instead.
    pub fn close_active(&mut self) -> Option<T> {
        if self.tabs.len() == 1 {
            return None;
        }
        let (_, closed) = self.tabs.remove(self.active);
        self.active = self.active.min(self.tabs.len() - 1);
        Some(closed)
    }

    pub fn activate(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active = index;
        }
    }

    /// Move the active tab by `delta`, wrapping around like Finder.
    pub fn cycle(&mut self, delta: isize) {
        let len = self.tabs.len() as isize;
        self.active = (self.active as isize + delta).rem_euclid(len) as usize;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(t: &Tabs<&'static str>) -> Vec<&'static str> {
        t.iter().map(|(_, name)| *name).collect()
    }

    #[test]
    fn open_inserts_after_active() {
        let mut t = Tabs::new("a");
        t.open("b");
        t.activate(0);
        t.open("c");
        assert_eq!(names(&t), ["a", "c", "b"]);
        assert_eq!(t.active_index(), 1);
    }

    #[test]
    fn last_tab_cannot_close() {
        let mut t = Tabs::new("a");
        assert_eq!(t.close_active(), None);
        t.open("b");
        assert_eq!(t.close_active(), Some("b"));
        assert_eq!(names(&t), ["a"]);
    }

    #[test]
    fn closing_last_position_moves_left() {
        let mut t = Tabs::new("a");
        t.open("b");
        t.open("c");
        t.close_active();
        assert_eq!(*t.active(), "b");
    }

    #[test]
    fn ids_stay_stable() {
        let mut t = Tabs::new("a");
        let b = t.open("b");
        t.activate(0);
        t.open("c");
        assert!(t.iter().any(|(id, name)| id == b && *name == "b"));
    }

    #[test]
    fn cycle_wraps() {
        let mut t = Tabs::new("a");
        t.open("b");
        t.cycle(1);
        assert_eq!(t.active_index(), 0);
        t.cycle(-1);
        assert_eq!(t.active_index(), 1);
    }
}
