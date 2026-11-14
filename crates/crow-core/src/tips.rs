//! One-time contextual tips: which ones have been retired, when one may appear,
//! and the zero-match streak that suggests search. Plain state with no UI types;
//! `crow-app` decides when to ask and how to draw.

use std::collections::BTreeSet;
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// The file inside the state dir.
pub const TIPS_FILE: &str = "tips.json";

/// A folder at least this big can suggest filtering (Q2, 2026-11-14).
pub const TIP_SCROLL_ENTRIES: usize = 200;

/// The first visible row that counts as "scrolled" (Q2): a bit more than one
/// screen of 24 px rows in a 760 px window.
pub const TIP_SCROLL_ROWS: usize = 30;

/// Two tips never land back to back (Q5, 2026-11-14); a relaunch clears it.
pub const TIP_COOLDOWN: Duration = Duration::from_secs(600);

/// The tips U9 knows. New tips in later plans add ids; old builds keep unknown
/// ids in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TipId {
    /// `/` in a long folder.
    Filter,
    /// ⌘F after two zero-match filters.
    Search,
    /// ⌘K after typing a path for a known folder.
    GoTo,
}

impl TipId {
    /// The id in `tips.json`.
    pub fn key(self) -> &'static str {
        match self {
            TipId::Filter => "filter",
            TipId::Search => "search",
            TipId::GoTo => "go-to",
        }
    }

    pub fn all() -> [TipId; 3] {
        [TipId::Filter, TipId::Search, TipId::GoTo]
    }
}

/// What `tips.json` holds: the ids already shown or whose key was used.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TipState {
    #[serde(default)]
    retired: BTreeSet<String>,
}

impl TipState {
    /// A missing or unreadable file reads as empty. A broken file is replaced on
    /// the next save; it is state, not config, so no status-line error.
    pub fn load(path: &Path) -> TipState {
        let Ok(text) = std::fs::read_to_string(path) else {
            return TipState::default();
        };
        serde_json::from_str(&text).unwrap_or_else(|e| {
            tracing::warn!("{}: {e}", path.display());
            TipState::default()
        })
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let text = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        crate::write_atomic(path, &text)
    }

    pub fn is_retired(&self, id: TipId) -> bool {
        self.retired.contains(id.key())
    }

    /// True when this changed the set, so callers can skip the save.
    pub fn retire(&mut self, id: TipId) -> bool {
        self.retired.insert(id.key().to_owned())
    }
}

/// The in-memory rules: retired ids, whether a tip is on screen, and the cooldown.
pub struct TipBook {
    state: TipState,
    on_screen: Option<TipId>,
    last_shown: Option<Instant>,
}

impl TipBook {
    pub fn new(state: TipState) -> TipBook {
        TipBook { state, on_screen: None, last_shown: None }
    }

    pub fn state(&self) -> &TipState {
        &self.state
    }

    pub fn is_on_screen(&self) -> bool {
        self.on_screen.is_some()
    }

    /// The tip this app is showing, if any.
    pub fn on_screen(&self) -> Option<TipId> {
        self.on_screen
    }

    /// Checks 1–5 of the plan; the window checks its own blocking states.
    pub fn may_show(&self, id: TipId, now: Instant, uses: u32, hints_off: bool) -> bool {
        !hints_off
            && !self.state.is_retired(id)
            && uses == 0
            && self.on_screen.is_none()
            && self.last_shown.is_none_or(|last| now.saturating_duration_since(last) >= TIP_COOLDOWN)
    }

    /// Show `id` once: it retires at the same moment (Q1).
    pub fn show(&mut self, id: TipId, now: Instant) {
        self.state.retire(id);
        self.on_screen = Some(id);
        self.last_shown = Some(now);
    }

    pub fn closed(&mut self) {
        self.on_screen = None;
    }

    /// Using the key retires its tip even if it was never shown (Q7).
    pub fn retire(&mut self, id: TipId) -> bool {
        self.state.retire(id)
    }
}

/// Consecutive filter sessions that ended with no matches. Two in a row suggest
/// ⌘F; a session with matches resets it, an empty query leaves it alone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ZeroMatchStreak(u8);

impl ZeroMatchStreak {
    pub fn new() -> ZeroMatchStreak {
        ZeroMatchStreak(0)
    }

    /// End one filter session. True when this was the second zero-match session
    /// in a row; the streak then starts over.
    pub fn end_session(&mut self, query_empty: bool, matches: usize) -> bool {
        if query_empty {
            return false;
        }
        if matches > 0 {
            self.0 = 0;
            return false;
        }
        self.0 += 1;
        if self.0 >= 2 {
            self.0 = 0;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("crow-tips-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn tip_state_round_trips_and_keeps_unknown_ids() {
        let dir = temp_dir("round-trip");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(TIPS_FILE);
        std::fs::write(&path, r#"{"retired":["filter","a-later-tip"]}"#).unwrap();

        let mut state = TipState::load(&path);
        assert!(state.is_retired(TipId::Filter));
        assert!(!state.is_retired(TipId::Search));
        assert!(state.retire(TipId::Search), "a new id changes the set");
        assert!(!state.retire(TipId::Search), "retiring twice is a no-op");
        state.save(&path).expect("saves");

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("a-later-tip"), "a newer Crow's tip survives: {text}");
        assert!(TipState::load(&path).is_retired(TipId::Search));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tip_state_missing_or_broken_file_is_empty() {
        let dir = temp_dir("broken");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(TIPS_FILE);
        assert_eq!(TipState::load(&path), TipState::default(), "missing");
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(TipState::load(&path), TipState::default(), "broken");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tip_book_shows_once_then_retired() {
        let now = Instant::now();
        let mut book = TipBook::new(TipState::default());
        assert!(book.may_show(TipId::Filter, now, 0, false));
        book.show(TipId::Filter, now);
        assert!(book.is_on_screen());
        book.closed();
        assert!(!book.is_on_screen());
        assert!(!book.may_show(TipId::Filter, now + TIP_COOLDOWN, 0, false), "shown once, never again");
        assert!(book.state().is_retired(TipId::Filter));
    }

    #[test]
    fn tip_book_refuses_when_key_used_or_hints_off() {
        let now = Instant::now();
        let book = TipBook::new(TipState::default());
        assert!(!book.may_show(TipId::Filter, now, 1, false), "the key is already known");
        assert!(!book.may_show(TipId::Filter, now, 0, true), "hints = off");
        assert!(book.may_show(TipId::Filter, now, 0, false));
    }

    #[test]
    fn tip_book_one_on_screen_and_cooldown() {
        let now = Instant::now();
        let mut book = TipBook::new(TipState::default());
        book.show(TipId::Filter, now);
        assert!(!book.may_show(TipId::Search, now + TIP_COOLDOWN, 0, false), "one tip app-wide");
        book.closed();
        assert!(!book.may_show(TipId::Search, now + TIP_COOLDOWN - Duration::from_secs(1), 0, false));
        assert!(book.may_show(TipId::Search, now + TIP_COOLDOWN, 0, false), "after the cooldown");
    }

    #[test]
    fn zero_match_streak_needs_two_in_a_row() {
        let mut streak = ZeroMatchStreak::new();
        assert!(!streak.end_session(false, 0));
        assert!(streak.end_session(false, 0), "two in a row");
        assert!(!streak.end_session(false, 0), "the streak starts over");

        let mut streak = ZeroMatchStreak::new();
        assert!(!streak.end_session(false, 0));
        assert!(!streak.end_session(false, 3), "matches reset it");
        assert!(!streak.end_session(false, 0));
        assert!(streak.end_session(false, 0), "two zero sessions after a match fire");
    }

    #[test]
    fn zero_match_streak_ignores_empty_queries() {
        let mut streak = ZeroMatchStreak::new();
        assert!(!streak.end_session(false, 0));
        assert!(!streak.end_session(true, 0), "an empty query changes nothing");
        assert!(streak.end_session(false, 0));
    }
}
