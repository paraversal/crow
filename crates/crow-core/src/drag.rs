//! Spring-loaded folder navigation, independent of GPUI and the clock.
//!
//! One window owns one [`DragMachine`]. Surfaces report the deepest eligible
//! folder under the pointer with its layout generation; the machine arms one
//! stationary timer per stable target. The window turns an expiry into a
//! temporary folder presentation. Nothing here touches history, Recent or disk.

use std::path::{Path, PathBuf};

/// The stationary hover that opens a folder, matching the hand-checked Finder feel.
pub const SPRING_DELAY_MS: u64 = 600;

/// One external drag, from entry to leave, cancel or accepted drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DragId(u64);

/// The folder under the pointer and the layout it was drawn in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpringTarget {
    pub path: PathBuf,
    pub generation: u64,
}

impl SpringTarget {
    pub fn new(path: PathBuf, generation: u64) -> Self {
        Self { path, generation }
    }
}

/// The one armed timer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Armed {
    pub token: u64,
    pub target: SpringTarget,
    pub deadline_ms: u64,
}

/// What the window should do when the drag ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Finish {
    /// Do nothing; the drag ended without a session.
    None,
    /// The drop was accepted. `path` is the final destination; `sprung` says
    /// whether the pane is showing a temporary folder that must become one
    /// history step.
    Accepted { path: PathBuf, sprung: bool },
    /// The drag was cancelled. `restore` is the origin when the pane sprang away.
    Cancelled { restore: Option<PathBuf> },
}

struct Session {
    id: DragId,
    origin: PathBuf,
    current: PathBuf,
    sprung: Vec<PathBuf>,
    armed: Option<Armed>,
    next_token: u64,
    /// A spring just happened: wait for fresh geometry or a pointer move before
    /// arming the next folder, so a reflow cannot open a folder by itself.
    awaiting_hover: bool,
    sprung_generation: u64,
    sprung_pointer: (f32, f32),
}

/// The drag navigation state for one window.
pub struct DragMachine {
    session: Option<Session>,
    next_id: u64,
    delay_ms: u64,
}

impl Default for DragMachine {
    fn default() -> Self {
        Self::new(SPRING_DELAY_MS)
    }
}

impl DragMachine {
    pub fn new(delay_ms: u64) -> Self {
        Self { session: None, next_id: 1, delay_ms }
    }

    /// Start a drag showing `origin`. A previous session is replaced.
    pub fn begin(&mut self, origin: PathBuf) -> DragId {
        let id = DragId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.session = Some(Session {
            id,
            origin: origin.clone(),
            current: origin,
            sprung: Vec::new(),
            armed: None,
            next_token: 1,
            awaiting_hover: false,
            sprung_generation: 0,
            sprung_pointer: (0.0, 0.0),
        });
        id
    }

    pub fn active(&self) -> Option<DragId> {
        self.session.as_ref().map(|session| session.id)
    }

    pub fn armed(&self) -> Option<&Armed> {
        self.session.as_ref()?.armed.as_ref()
    }

    /// The folder the pane should show: the origin, or the deepest sprung folder.
    pub fn current(&self) -> Option<&Path> {
        self.session.as_ref().map(|session| session.current.as_path())
    }

    pub fn sprung(&self) -> &[PathBuf] {
        self.session.as_ref().map_or(&[], |session| session.sprung.as_slice())
    }

    /// Report the deepest eligible folder under `pointer`, or `None` when the
    /// pointer is over something unavailable. Repeating the same target keeps
    /// its original deadline; moving to another target starts a fresh one.
    ///
    /// Returns the armed token when the machine armed or re-armed a timer.
    pub fn observe(
        &mut self,
        id: DragId,
        pointer: (f32, f32),
        target: Option<SpringTarget>,
        now_ms: u64,
    ) -> Option<u64> {
        let session = self.session.as_mut().filter(|session| session.id == id)?;
        let Some(target) = target else {
            session.armed = None;
            return None;
        };
        if session.current == target.path || session.current.starts_with(&target.path) {
            // The shown folder, or one of its ancestors: a spring would loop.
            session.armed = None;
            return None;
        }
        if session.awaiting_hover {
            let fresh_geometry = target.generation != session.sprung_generation;
            let moved = pointer != session.sprung_pointer;
            if !fresh_geometry && !moved {
                session.armed = None;
                return None;
            }
            session.awaiting_hover = false;
        }
        if let Some(armed) = &session.armed
            && armed.target == target
        {
            return Some(armed.token);
        }
        let token = session.next_token;
        session.next_token = session.next_token.wrapping_add(1).max(1);
        session.armed = Some(Armed { token, target, deadline_ms: now_ms.saturating_add(self.delay_ms) });
        Some(token)
    }

    /// The timer fired: open the armed target only if the same session and the
    /// current pointer-over-target survived to now.
    pub fn expire(
        &mut self,
        id: DragId,
        token: u64,
        pointer: (f32, f32),
        target: Option<SpringTarget>,
        now_ms: u64,
    ) -> Option<PathBuf> {
        let session = self.session.as_mut().filter(|session| session.id == id)?;
        let armed = session.armed.as_ref()?;
        if armed.token != token
            || now_ms < armed.deadline_ms
            || target.as_ref() != Some(&armed.target)
            || session.current == armed.target.path
            || session.current.starts_with(&armed.target.path)
        {
            return None;
        }
        let target = session.armed.take()?.target;
        session.current = target.path.clone();
        session.sprung.push(target.path.clone());
        session.awaiting_hover = true;
        session.sprung_generation = target.generation;
        session.sprung_pointer = pointer;
        Some(target.path)
    }

    /// The platform drag left the window: forget the timer. The pane stays in
    /// its sprung folder until the session is finished or restored.
    pub fn leave(&mut self, id: DragId) {
        if let Some(session) = self.session.as_mut().filter(|session| session.id == id) {
            session.armed = None;
        }
    }

    /// End the session. Accepted reports the final destination; cancellation
    /// reports the origin when the pane sprang away from it.
    pub fn finish(&mut self, id: DragId, accepted: bool) -> Finish {
        let Some(session) = self.session.take().filter(|session| session.id == id) else {
            return Finish::None;
        };
        if accepted {
            Finish::Accepted { path: session.current, sprung: !session.sprung.is_empty() }
        } else {
            Finish::Cancelled { restore: (!session.sprung.is_empty()).then_some(session.origin) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(text: &str) -> PathBuf {
        PathBuf::from(text)
    }

    fn target(text: &str, generation: u64) -> SpringTarget {
        SpringTarget::new(path(text), generation)
    }

    fn machine_at_root() -> (DragMachine, DragId) {
        let mut machine = DragMachine::new(SPRING_DELAY_MS);
        let id = machine.begin(path("/root"));
        (machine, id)
    }

    #[test]
    fn arms_at_the_delay_and_keeps_one_deadline() {
        let (mut machine, id) = machine_at_root();
        assert_eq!(machine.observe(id, (10., 10.), Some(target("/root/a", 1)), 100), Some(1));
        assert_eq!(machine.observe(id, (12., 11.), Some(target("/root/a", 1)), 300), Some(1));
        let armed = machine.armed().cloned().expect("armed");
        assert_eq!(armed.deadline_ms, 700, "repeated moves keep the original deadline");
        assert_eq!(machine.expire(id, 1, (12., 11.), Some(target("/root/a", 1)), 699), None);
        assert_eq!(machine.expire(id, 1, (12., 11.), Some(target("/root/a", 1)), 700), Some(path("/root/a")));
        assert_eq!(machine.current(), Some(Path::new("/root/a")));
    }

    #[test]
    fn moving_to_another_target_rearms() {
        let (mut machine, id) = machine_at_root();
        machine.observe(id, (10., 10.), Some(target("/root/a", 1)), 100);
        machine.observe(id, (10., 50.), Some(target("/root/b", 1)), 200);
        assert_eq!(machine.armed().expect("armed").deadline_ms, 800);
        assert_eq!(machine.armed().expect("armed").target.path, path("/root/b"));
        machine.observe(id, (10., 60.), None, 300);
        assert!(machine.armed().is_none(), "an unavailable target clears the timer");
    }

    #[test]
    fn stale_timer_and_changed_generation_do_not_spring() {
        let (mut machine, id) = machine_at_root();
        machine.observe(id, (10., 10.), Some(target("/root/a", 1)), 0);
        assert_eq!(machine.expire(id, 2, (10., 10.), Some(target("/root/a", 1)), 600), None, "stale token");
        assert_eq!(machine.expire(id, 1, (10., 10.), Some(target("/root/a", 2)), 600), None, "reflowed row");
        assert_eq!(machine.expire(id, 1, (10., 10.), Some(target("/root/b", 1)), 600), None, "pointer moved");
        assert_eq!(machine.expire(id, 1, (10., 10.), Some(target("/root/a", 1)), 600), Some(path("/root/a")));
    }

    #[test]
    fn after_a_spring_the_same_coordinate_needs_fresh_geometry() {
        let (mut machine, id) = machine_at_root();
        machine.observe(id, (10., 10.), Some(target("/root/a", 1)), 0);
        assert_eq!(machine.expire(id, 1, (10., 10.), Some(target("/root/a", 1)), 600), Some(path("/root/a")));
        assert_eq!(
            machine.observe(id, (10., 10.), Some(target("/root/a/b", 1)), 700),
            None,
            "the pre-spring geometry at the same point cannot arm by itself"
        );
        assert_eq!(
            machine.observe(id, (10., 10.), Some(target("/root/a/b", 2)), 700),
            Some(2),
            "fresh geometry at the same point can arm"
        );
        assert_eq!(machine.expire(id, 2, (10., 10.), Some(target("/root/a/b", 2)), 1300), Some(path("/root/a/b")));
    }

    #[test]
    fn nested_springs_ignore_the_shown_folder_and_its_ancestors() {
        let (mut machine, id) = machine_at_root();
        machine.observe(id, (10., 10.), Some(target("/root/a", 1)), 0);
        machine.expire(id, 1, (10., 10.), Some(target("/root/a", 1)), 600);
        assert_eq!(machine.observe(id, (11., 10.), Some(target("/root/a", 2)), 700), None, "same folder");
        assert_eq!(machine.observe(id, (12., 10.), Some(target("/root", 2)), 800), None, "ancestor loop");
        assert_eq!(machine.observe(id, (13., 10.), Some(target("/root/a/b", 2)), 900), Some(2), "deeper folder");
        assert_eq!(machine.expire(id, 2, (13., 10.), Some(target("/root/a/b", 2)), 1500), Some(path("/root/a/b")));
        assert_eq!(machine.sprung(), [path("/root/a"), path("/root/a/b")]);
    }

    #[test]
    fn finishing_reports_commit_and_restore_origins() {
        let (mut machine, id) = machine_at_root();
        machine.observe(id, (10., 10.), Some(target("/root/a", 1)), 0);
        machine.expire(id, 1, (10., 10.), Some(target("/root/a", 1)), 600);
        assert_eq!(machine.finish(id, true), Finish::Accepted { path: path("/root/a"), sprung: true });
        let (mut machine, id) = machine_at_root();
        machine.observe(id, (10., 10.), Some(target("/root/a", 1)), 0);
        machine.expire(id, 1, (10., 10.), Some(target("/root/a", 1)), 600);
        assert_eq!(machine.finish(id, false), Finish::Cancelled { restore: Some(path("/root")) });
        assert_eq!(machine.finish(id, false), Finish::None, "one finish per session");
    }

    #[test]
    fn leaving_disarms_but_keeps_the_sprung_presentation() {
        let (mut machine, id) = machine_at_root();
        machine.observe(id, (10., 10.), Some(target("/root/a", 1)), 0);
        machine.expire(id, 1, (10., 10.), Some(target("/root/a", 1)), 600);
        machine.observe(id, (20., 10.), Some(target("/root/a/b", 2)), 700);
        machine.leave(id);
        assert!(machine.armed().is_none());
        assert_eq!(machine.current(), Some(Path::new("/root/a")));
        assert_eq!(machine.finish(id, false), Finish::Cancelled { restore: Some(path("/root")) });
    }

    #[test]
    fn a_new_drag_invalidates_the_old_session() {
        let (mut machine, id) = machine_at_root();
        machine.observe(id, (10., 10.), Some(target("/root/a", 1)), 0);
        let new = machine.begin(path("/other"));
        assert_ne!(id, new);
        assert_eq!(machine.observe(id, (10., 10.), Some(target("/root/a", 1)), 10), None);
        assert_eq!(machine.finish(id, true), Finish::None);
        assert_eq!(machine.expire(new, 1, (10., 10.), None, 600), None);
    }
}
