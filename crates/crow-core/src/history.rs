/// Back/forward history of one tab, like a browser's. `L` is a real folder path
/// or a typed destination (`Location`), never a path standing in for a virtual id.
#[derive(Clone, Debug)]
pub struct History<L> {
    back: Vec<L>,
    current: L,
    forward: Vec<L>,
}

impl<L: Clone + PartialEq> History<L> {
    pub fn new(start: L) -> Self {
        Self { back: Vec::new(), current: start, forward: Vec::new() }
    }

    pub fn current(&self) -> &L {
        &self.current
    }

    /// Go somewhere new: the current location joins the back stack, forward is
    /// cleared. Navigating to the current location changes nothing.
    pub fn visit(&mut self, location: L) {
        if location == self.current {
            return;
        }
        self.back.push(std::mem::replace(&mut self.current, location));
        self.forward.clear();
    }

    /// Show `location` in place of the current one, without adding a Back step.
    /// A sidebar walk is one step in history: later cursor moves replace it.
    pub fn replace_current(&mut self, location: L) {
        if location == self.current {
            return;
        }
        self.current = location;
        self.forward.clear();
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// The location Back would show, without moving.
    pub fn back_target(&self) -> Option<&L> {
        self.back.last()
    }

    /// The location Forward would show, without moving.
    pub fn forward_target(&self) -> Option<&L> {
        self.forward.last()
    }

    pub fn back(&mut self) -> Option<&L> {
        let prev = self.back.pop()?;
        self.forward.push(std::mem::replace(&mut self.current, prev));
        Some(&self.current)
    }

    pub fn forward(&mut self) -> Option<&L> {
        let next = self.forward.pop()?;
        self.back.push(std::mem::replace(&mut self.current, next));
        Some(&self.current)
    }

    /// Every retained location, used by teardown to release virtual sessions.
    pub fn locations(&self) -> impl Iterator<Item = &L> {
        self.back.iter().chain(std::iter::once(&self.current)).chain(self.forward.iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::Location;
    use crate::saved_queries::SavedQueryId;
    use std::path::{Path, PathBuf};

    fn path(text: &str) -> PathBuf {
        PathBuf::from(text)
    }

    #[test]
    fn back_and_forward() {
        let mut h: History<PathBuf> = History::new(path("/a"));
        h.visit(path("/b"));
        h.visit(path("/c"));
        assert_eq!(h.back(), Some(&path("/b")));
        assert_eq!(h.back(), Some(&path("/a")));
        assert_eq!(h.back(), None);
        assert_eq!(h.forward(), Some(&path("/b")));
        h.visit(path("/d"));
        assert!(!h.can_go_forward(), "a new visit clears forward");
        assert_eq!(h.back(), Some(&path("/b")));
    }

    #[test]
    fn history_peeks_without_moving() {
        let mut h: History<PathBuf> = History::new(path("/a"));
        assert_eq!(h.back_target(), None);
        h.visit(path("/b"));
        h.visit(path("/c"));
        assert_eq!(h.back_target(), Some(&path("/b")));
        assert_eq!(h.current(), Path::new("/c"), "peeking leaves the current folder alone");
        assert_eq!(h.back(), Some(&path("/b")));
        assert_eq!(h.forward_target(), Some(&path("/c")));
        assert_eq!(h.current(), Path::new("/b"));
    }

    #[test]
    fn replace_current_keeps_the_back_step() {
        let mut h: History<PathBuf> = History::new(path("/a"));
        h.visit(path("/b"));
        h.replace_current(path("/c"));
        assert_eq!(h.current(), Path::new("/c"));
        assert_eq!(h.back(), Some(&path("/a")), "the walk did not add a step");
    }

    #[test]
    fn revisiting_current_is_a_no_op() {
        let mut h: History<PathBuf> = History::new(path("/a"));
        h.visit(path("/a"));
        assert!(!h.can_go_back());
    }

    #[test]
    fn virtual_destinations_keep_ids_across_visits() {
        let id = SavedQueryId::new("recent-pdfs").unwrap();
        let mut h: History<Location> = History::new(Location::Folder(path("/a")));
        h.visit(Location::SavedQuery(id.clone()));
        h.visit(Location::Folder(path("/b")));
        assert_eq!(h.back(), Some(&Location::SavedQuery(id.clone())));
        assert_eq!(h.back(), Some(&Location::Folder(path("/a"))));
        assert_eq!(h.forward(), Some(&Location::SavedQuery(id)));
        assert_eq!(h.locations().count(), 3);
    }
}
