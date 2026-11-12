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

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
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
    fn revisiting_current_is_a_no_op() {
        let mut h = History::new("/a".into());
        h.visit("/a".into());
        assert!(!h.can_go_back());
    }
}
