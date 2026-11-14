use std::path::{Path, PathBuf};

/// Back/forward history of one tab, like a browser's.
#[derive(Clone, Debug)]
pub struct History {
    back: Vec<PathBuf>,
    current: PathBuf,
    forward: Vec<PathBuf>,
}

impl History {
    pub fn new(start: PathBuf) -> Self {
        Self { back: Vec::new(), current: start, forward: Vec::new() }
    }

    pub fn current(&self) -> &Path {
        &self.current
    }

    /// Go somewhere new: the current folder joins the back stack, forward is cleared.
    /// Navigating to the current folder changes nothing.
    pub fn visit(&mut self, dir: PathBuf) {
        if dir == self.current {
            return;
        }
        self.back.push(std::mem::replace(&mut self.current, dir));
        self.forward.clear();
    }

    /// Show `dir` in place of the current folder, without adding a Back step.
    /// A sidebar walk is one step in history: later cursor moves replace it.
    pub fn replace_current(&mut self, dir: PathBuf) {
        if dir == self.current {
            return;
        }
        self.current = dir;
        self.forward.clear();
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// The folder Back would show, without moving.
    pub fn back_target(&self) -> Option<&Path> {
        self.back.last().map(PathBuf::as_path)
    }

    /// The folder Forward would show, without moving.
    pub fn forward_target(&self) -> Option<&Path> {
        self.forward.last().map(PathBuf::as_path)
    }

    pub fn back(&mut self) -> Option<&Path> {
        let prev = self.back.pop()?;
        self.forward.push(std::mem::replace(&mut self.current, prev));
        Some(&self.current)
    }

    pub fn forward(&mut self) -> Option<&Path> {
        let next = self.forward.pop()?;
        self.back.push(std::mem::replace(&mut self.current, next));
        Some(&self.current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_and_forward() {
        let mut h = History::new("/a".into());
        h.visit("/b".into());
        h.visit("/c".into());
        assert_eq!(h.back(), Some(Path::new("/b")));
        assert_eq!(h.back(), Some(Path::new("/a")));
        assert_eq!(h.back(), None);
        assert_eq!(h.forward(), Some(Path::new("/b")));
        h.visit("/d".into());
        assert!(!h.can_go_forward(), "a new visit clears forward");
        assert_eq!(h.back(), Some(Path::new("/b")));
    }

    #[test]
    fn history_peeks_without_moving() {
        let mut h = History::new("/a".into());
        assert_eq!(h.back_target(), None);
        h.visit("/b".into());
        h.visit("/c".into());
        assert_eq!(h.back_target(), Some(Path::new("/b")));
        assert_eq!(h.current(), Path::new("/c"), "peeking leaves the current folder alone");
        assert_eq!(h.back(), Some(Path::new("/b")));
        assert_eq!(h.forward_target(), Some(Path::new("/c")));
        assert_eq!(h.current(), Path::new("/b"));
    }

    #[test]
    fn replace_current_keeps_the_back_step() {
        let mut h = History::new("/a".into());
        h.visit("/b".into());
        h.replace_current("/c".into());
        assert_eq!(h.current(), Path::new("/c"));
        assert_eq!(h.back(), Some(Path::new("/a")), "the walk did not add a step");
    }

    #[test]
    fn revisiting_current_is_a_no_op() {
        let mut h = History::new("/a".into());
        h.visit("/a".into());
        assert!(!h.can_go_back());
    }
}
