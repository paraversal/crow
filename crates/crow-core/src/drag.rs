//! Spring-loaded folder navigation, independent of GPUI and the clock.
//!
//! One window owns one [`DragMachine`]. Surfaces report the deepest eligible
//! folder under the pointer with its layout generation, or the Back button; the
//! machine arms one stationary timer per stable target. The window turns an
//! expiry into a temporary folder presentation. Nothing here touches history,
//! Recent or disk.

use std::path::{Path, PathBuf};

/// The stationary hover that opens a folder, matching the hand-checked Finder feel.
pub const SPRING_DELAY_MS: u64 = 600;
const MAX_SPRING_DEPTH: usize = 32;

/// One drag, from entry to leave, cancel or accepted drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DragId(u64);

/// What the pointer holds still over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpringTarget {
    /// A folder, and the layout it was drawn in.
    Folder { path: PathBuf, generation: u64, allow_ancestor: bool },
    /// The Back button: undo the last spring, then step back through history.
    Back,
}

impl SpringTarget {
    pub fn new(path: PathBuf, generation: u64) -> Self {
        Self::Folder { path, generation, allow_ancestor: false }
    }
}

/// What the pane should show after a spring.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Spring {
    /// A temporary presentation of this folder.
    Enter(PathBuf),
    /// The drag's origin again, restored exactly.
    Origin,
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
    Accepted { path: Option<PathBuf>, sprung: bool },
    /// The drag was cancelled. Restore the presentation if the pane sprang away.
    Cancelled { restore: bool },
}

struct Session {
    id: DragId,
    origin: Option<PathBuf>,
    current: Option<PathBuf>,
    /// Folders opened by holding over them, oldest first.
    sprung: Vec<PathBuf>,
    /// The origin's real Back history, nearest first.
    back_history: Vec<PathBuf>,
    /// How many history steps back the Back button has gone; forward springs
    /// stack on top of that location.
    back_steps: usize,
    armed: Option<Armed>,
    next_token: u64,
    /// A spring just happened: wait for fresh geometry or a pointer move before
    /// arming the next folder, so a reflow cannot open a folder by itself.
    awaiting_hover: bool,
    sprung_generation: u64,
    sprung_pointer: (f32, f32),
}

impl Session {
    /// The location under the forward springs: the origin, or a Back history step.
    fn base(&self) -> Spring {
        match self.back_steps {
            0 => Spring::Origin,
            n => Spring::Enter(self.back_history[n - 1].clone()),
        }
    }

    fn away(&self) -> bool {
        !self.sprung.is_empty() || self.back_steps > 0
    }

    fn can_enter(&self, path: &Path, allow_ancestor: bool) -> bool {
        self.sprung.len() < MAX_SPRING_DEPTH
            && !self.sprung.iter().any(|visited| visited == path)
            && self
                .current
                .as_deref()
                .is_none_or(|current| current != path && (allow_ancestor || !current.starts_with(path)))
    }

    /// Where holding over Back would go, if anywhere.
    fn back_destination(&self) -> Option<Spring> {
        match self.sprung.len() {
            0 => self.back_history.get(self.back_steps).cloned().map(Spring::Enter),
            1 => Some(self.base()),
            n => Some(Spring::Enter(self.sprung[n - 2].clone())),
        }
    }
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

    /// Start a drag showing `origin`, with the origin's real-folder Back history
    /// (nearest first). A previous session is replaced.
    pub fn begin(&mut self, origin: Option<PathBuf>, back_history: Vec<PathBuf>) -> DragId {
        let id = DragId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.session = Some(Session {
            id,
            origin: origin.clone(),
            current: origin,
            sprung: Vec::new(),
            back_history,
            back_steps: 0,
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
        self.session.as_ref()?.current.as_deref()
    }

    pub fn sprung(&self) -> &[PathBuf] {
        self.session.as_ref().map_or(&[], |session| session.sprung.as_slice())
    }

    /// Whether holding over Back would go anywhere right now.
    pub fn can_spring_back(&self) -> bool {
        self.session.as_ref().is_some_and(|session| session.back_destination().is_some())
    }

    /// Report the deepest eligible target under `pointer`, or `None` when the
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
        match &target {
            SpringTarget::Back => {
                // The button does not reflow, so holding still keeps stepping back.
                if session.back_destination().is_none() {
                    session.armed = None;
                    return None;
                }
            }
            SpringTarget::Folder { path, generation, allow_ancestor } => {
                if !session.can_enter(path, *allow_ancestor) {
                    session.armed = None;
                    return None;
                }
                if session.awaiting_hover {
                    let fresh_geometry = *generation != session.sprung_generation;
                    let moved = pointer != session.sprung_pointer;
                    if !fresh_geometry && !moved {
                        session.armed = None;
                        return None;
                    }
                    session.awaiting_hover = false;
                }
            }
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

    /// The timer fired: spring only if the same session and the current
    /// pointer-over-target survived to now.
    pub fn expire(
        &mut self,
        id: DragId,
        token: u64,
        pointer: (f32, f32),
        target: Option<SpringTarget>,
        now_ms: u64,
    ) -> Option<Spring> {
        let session = self.session.as_mut().filter(|session| session.id == id)?;
        let armed = session.armed.as_ref()?;
        if armed.token != token || now_ms < armed.deadline_ms || target.as_ref() != Some(&armed.target) {
            return None;
        }
        let spring = match &armed.target {
            SpringTarget::Back => {
                let destination = session.back_destination()?;
                if session.sprung.pop().is_none() {
                    session.back_steps += 1;
                }
                destination
            }
            SpringTarget::Folder { path, allow_ancestor, .. } => {
                if !session.can_enter(path, *allow_ancestor) {
                    return None;
                }
                session.sprung.push(path.clone());
                Spring::Enter(path.clone())
            }
        };
        let generation = match &armed.target {
            SpringTarget::Folder { generation, .. } => *generation,
            SpringTarget::Back => session.sprung_generation,
        };
        session.armed = None;
        session.current = match &spring {
            Spring::Enter(path) => Some(path.clone()),
            Spring::Origin => session.origin.clone(),
        };
        session.awaiting_hover = true;
        session.sprung_generation = generation;
        session.sprung_pointer = pointer;
        Some(spring)
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
        let away = session.away();
        if accepted {
            Finish::Accepted { path: session.current, sprung: away }
        } else {
            Finish::Cancelled { restore: away }
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
        let id = machine.begin(Some(path("/root")), Vec::new());
        (machine, id)
    }

    #[test]
    fn explicit_ancestor_targets_can_spring_without_revisiting_the_stack() {
        let mut machine = DragMachine::default();
        let id = machine.begin(Some(path("/root/a/b")), Vec::new());
        let ancestor = SpringTarget::Folder { path: path("/root"), generation: 1, allow_ancestor: true };
        assert_eq!(spring(&mut machine, id, ancestor.clone(), 0), Some(Spring::Enter(path("/root"))));
        assert_eq!(spring(&mut machine, id, target("/root/a", 2), 700), Some(Spring::Enter(path("/root/a"))));
        assert_eq!(machine.observe(id, (10., 10.), Some(ancestor), 1400), None);
    }

    #[test]
    fn virtual_origin_can_spring_and_restore_without_a_folder() {
        let mut machine = DragMachine::default();
        let id = machine.begin(None, vec![path("/root")]);
        assert_eq!(machine.current(), None);
        assert_eq!(spring(&mut machine, id, target("/root", 1), 0), Some(Spring::Enter(path("/root"))));
        assert_eq!(spring(&mut machine, id, SpringTarget::Back, 700), Some(Spring::Origin));
        assert_eq!(machine.current(), None);
        assert_eq!(spring(&mut machine, id, target("/root", 2), 1400), Some(Spring::Enter(path("/root"))));
        assert_eq!(machine.finish(id, false), Finish::Cancelled { restore: true });
    }

    #[test]
    fn limits_forward_spring_depth() {
        let (mut machine, id) = machine_at_root();
        for depth in 0..MAX_SPRING_DEPTH {
            let path = format!("/root/{depth}");
            let time = depth as u64 * SPRING_DELAY_MS;
            assert_eq!(
                spring(&mut machine, id, target(&path, depth as u64 + 1), time),
                Some(Spring::Enter(PathBuf::from(path))),
            );
        }
        assert_eq!(machine.observe(id, (10., 10.), Some(target("/root/next", 100)), 30_000), None);
    }

    #[test]
    fn arms_at_the_delay_and_keeps_one_deadline() {
        let (mut machine, id) = machine_at_root();
        assert_eq!(machine.observe(id, (10., 10.), Some(target("/root/a", 1)), 100), Some(1));
        assert_eq!(machine.observe(id, (12., 11.), Some(target("/root/a", 1)), 300), Some(1));
        let armed = machine.armed().cloned().expect("armed");
        assert_eq!(armed.deadline_ms, 700, "repeated moves keep the original deadline");
        assert_eq!(machine.expire(id, 1, (12., 11.), Some(target("/root/a", 1)), 699), None);
        assert_eq!(
            machine.expire(id, 1, (12., 11.), Some(target("/root/a", 1)), 700),
            Some(Spring::Enter(path("/root/a")))
        );
        assert_eq!(machine.current(), Some(Path::new("/root/a")));
    }

    #[test]
    fn moving_to_another_target_rearms() {
        let (mut machine, id) = machine_at_root();
        machine.observe(id, (10., 10.), Some(target("/root/a", 1)), 100);
        machine.observe(id, (10., 50.), Some(target("/root/b", 1)), 200);
        assert_eq!(machine.armed().expect("armed").deadline_ms, 800);
        assert_eq!(machine.armed().expect("armed").target, target("/root/b", 1));
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
        assert_eq!(
            machine.expire(id, 1, (10., 10.), Some(target("/root/a", 1)), 600),
            Some(Spring::Enter(path("/root/a")))
        );
    }

    #[test]
    fn after_a_spring_the_same_coordinate_needs_fresh_geometry() {
        let (mut machine, id) = machine_at_root();
        machine.observe(id, (10., 10.), Some(target("/root/a", 1)), 0);
        assert_eq!(
            machine.expire(id, 1, (10., 10.), Some(target("/root/a", 1)), 600),
            Some(Spring::Enter(path("/root/a")))
        );
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
        assert_eq!(
            machine.expire(id, 2, (10., 10.), Some(target("/root/a/b", 2)), 1300),
            Some(Spring::Enter(path("/root/a/b")))
        );
    }

    #[test]
    fn nested_springs_ignore_the_shown_folder_and_its_ancestors() {
        let (mut machine, id) = machine_at_root();
        machine.observe(id, (10., 10.), Some(target("/root/a", 1)), 0);
        machine.expire(id, 1, (10., 10.), Some(target("/root/a", 1)), 600);
        assert_eq!(machine.observe(id, (11., 10.), Some(target("/root/a", 2)), 700), None, "same folder");
        assert_eq!(machine.observe(id, (12., 10.), Some(target("/root", 2)), 800), None, "ancestor loop");
        assert_eq!(machine.observe(id, (13., 10.), Some(target("/root/a/b", 2)), 900), Some(2), "deeper folder");
        assert_eq!(
            machine.expire(id, 2, (13., 10.), Some(target("/root/a/b", 2)), 1500),
            Some(Spring::Enter(path("/root/a/b")))
        );
        assert_eq!(machine.sprung(), [path("/root/a"), path("/root/a/b")]);
    }

    #[test]
    fn finishing_reports_commit_and_restore_origins() {
        let (mut machine, id) = machine_at_root();
        machine.observe(id, (10., 10.), Some(target("/root/a", 1)), 0);
        machine.expire(id, 1, (10., 10.), Some(target("/root/a", 1)), 600);
        assert_eq!(machine.finish(id, true), Finish::Accepted { path: Some(path("/root/a")), sprung: true });
        let (mut machine, id) = machine_at_root();
        machine.observe(id, (10., 10.), Some(target("/root/a", 1)), 0);
        machine.expire(id, 1, (10., 10.), Some(target("/root/a", 1)), 600);
        assert_eq!(machine.finish(id, false), Finish::Cancelled { restore: true });
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
        assert_eq!(machine.finish(id, false), Finish::Cancelled { restore: true });
    }

    #[test]
    fn a_new_drag_invalidates_the_old_session() {
        let (mut machine, id) = machine_at_root();
        machine.observe(id, (10., 10.), Some(target("/root/a", 1)), 0);
        let new = machine.begin(Some(path("/other")), Vec::new());
        assert_ne!(id, new);
        assert_eq!(machine.observe(id, (10., 10.), Some(target("/root/a", 1)), 10), None);
        assert_eq!(machine.finish(id, true), Finish::None);
        assert_eq!(machine.expire(new, 1, (10., 10.), None, 600), None);
    }

    fn spring(machine: &mut DragMachine, id: DragId, target: SpringTarget, start_ms: u64) -> Option<Spring> {
        let token = machine.observe(id, (10., 10.), Some(target.clone()), start_ms)?;
        machine.expire(id, token, (10., 10.), Some(target), start_ms + SPRING_DELAY_MS)
    }

    #[test]
    fn back_undoes_springs_then_walks_real_history() {
        let mut machine = DragMachine::new(SPRING_DELAY_MS);
        let id = machine.begin(Some(path("/root")), vec![path("/prev"), path("/older")]);
        assert_eq!(spring(&mut machine, id, target("/root/a", 1), 0), Some(Spring::Enter(path("/root/a"))));
        assert_eq!(spring(&mut machine, id, target("/root/a/b", 2), 1000), Some(Spring::Enter(path("/root/a/b"))));

        assert_eq!(spring(&mut machine, id, SpringTarget::Back, 2000), Some(Spring::Enter(path("/root/a"))));
        assert_eq!(spring(&mut machine, id, SpringTarget::Back, 3000), Some(Spring::Origin), "back to the origin");
        assert_eq!(machine.current(), Some(Path::new("/root")));
        assert_eq!(spring(&mut machine, id, SpringTarget::Back, 4000), Some(Spring::Enter(path("/prev"))));
        assert_eq!(spring(&mut machine, id, SpringTarget::Back, 5000), Some(Spring::Enter(path("/older"))));
        assert!(!machine.can_spring_back(), "history is exhausted");
        assert_eq!(machine.observe(id, (10., 10.), Some(SpringTarget::Back), 6000), None, "Back no longer arms");

        // A forward spring from a history step pops back to that step, not the origin.
        assert_eq!(spring(&mut machine, id, target("/older/x", 3), 7000), Some(Spring::Enter(path("/older/x"))));
        assert_eq!(spring(&mut machine, id, SpringTarget::Back, 8000), Some(Spring::Enter(path("/older"))));
        assert_eq!(
            machine.finish(id, true),
            Finish::Accepted { path: Some(path("/older")), sprung: true },
            "a history step is still away from the origin"
        );
    }

    #[test]
    fn holding_still_over_back_keeps_stepping() {
        let mut machine = DragMachine::new(SPRING_DELAY_MS);
        let id = machine.begin(Some(path("/root")), vec![path("/prev"), path("/older")]);
        let token = machine.observe(id, (5., 5.), Some(SpringTarget::Back), 0).expect("armed");
        assert_eq!(
            machine.expire(id, token, (5., 5.), Some(SpringTarget::Back), 600),
            Some(Spring::Enter(path("/prev")))
        );
        let again = machine.observe(id, (5., 5.), Some(SpringTarget::Back), 600).expect("re-armed in place");
        assert_ne!(again, token, "a fresh timer");
        assert_eq!(
            machine.expire(id, again, (5., 5.), Some(SpringTarget::Back), 1200),
            Some(Spring::Enter(path("/older")))
        );
    }

    #[test]
    fn back_to_the_origin_finishes_as_not_sprung() {
        let mut machine = DragMachine::new(SPRING_DELAY_MS);
        let id = machine.begin(Some(path("/root")), Vec::new());
        assert!(!machine.can_spring_back(), "no springs and no history");
        spring(&mut machine, id, target("/root/a", 1), 0);
        assert!(machine.can_spring_back());
        assert_eq!(spring(&mut machine, id, SpringTarget::Back, 1000), Some(Spring::Origin));
        assert_eq!(machine.finish(id, true), Finish::Accepted { path: Some(path("/root")), sprung: false });
    }

    #[test]
    fn a_stale_back_timer_does_not_spring() {
        let mut machine = DragMachine::new(SPRING_DELAY_MS);
        let id = machine.begin(Some(path("/root")), vec![path("/prev")]);
        let token = machine.observe(id, (5., 5.), Some(SpringTarget::Back), 0).expect("armed");
        machine.observe(id, (50., 50.), Some(target("/root/a", 1)), 100);
        assert_eq!(machine.expire(id, token, (5., 5.), Some(SpringTarget::Back), 600), None, "moved away and back");
        assert_eq!(machine.current(), Some(Path::new("/root")));
    }
}
