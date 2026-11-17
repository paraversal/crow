//! The one-time first-launch choice: whether tips show. State, not config, so
//! deleting `settings.toml` does not ask again. Plain state with no UI types;
//! `crow-app` owns the sheet and writes the `tips` setting.

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The file inside the state dir.
pub const ONBOARDING_FILE: &str = "onboarding.json";

/// What `onboarding.json` holds. `tips_choice` is `None` until the user answers;
/// a missing or broken file reads as unanswered and asks again.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OnboardingState {
    #[serde(default)]
    tips_choice: Option<bool>,
}

impl OnboardingState {
    /// A missing or unreadable file reads as unanswered. A broken file is
    /// replaced on the next save; it is state, not config, so no status-line error.
    pub fn load(path: &Path) -> OnboardingState {
        let Ok(text) = std::fs::read_to_string(path) else {
            return OnboardingState::default();
        };
        serde_json::from_str(&text).unwrap_or_else(|e| {
            tracing::warn!("{}: {e}", path.display());
            OnboardingState::default()
        })
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let text = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        crate::write_atomic(path, &text)
    }

    /// Whether the user has answered, either way.
    pub fn is_answered(&self) -> bool {
        self.tips_choice.is_some()
    }

    /// The answer: `true` showed tips, `false` muted them.
    pub fn tips_choice(&self) -> Option<bool> {
        self.tips_choice
    }

    /// Record the answer. True when this changed it, so callers can skip the save.
    pub fn answer(&mut self, show_tips: bool) -> bool {
        if self.tips_choice == Some(show_tips) {
            return false;
        }
        self.tips_choice = Some(show_tips);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("crow-onboarding-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn missing_or_broken_file_is_unanswered() {
        let dir = temp_dir("broken");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(ONBOARDING_FILE);
        assert!(!OnboardingState::load(&path).is_answered(), "missing");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(!OnboardingState::load(&path).is_answered(), "broken");
        std::fs::write(&path, "{}").unwrap();
        assert!(!OnboardingState::load(&path).is_answered(), "no answer yet");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn answers_round_trip() {
        let dir = temp_dir("round-trip");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(ONBOARDING_FILE);

        let mut state = OnboardingState::default();
        assert!(state.answer(false), "the first answer changes it");
        assert!(!state.answer(false), "the same answer is a no-op");
        state.save(&path).expect("saves");
        assert_eq!(OnboardingState::load(&path).tips_choice(), Some(false));

        let mut state = OnboardingState::load(&path);
        assert!(state.answer(true), "a different answer changes it");
        state.save(&path).expect("saves");
        assert_eq!(OnboardingState::load(&path).tips_choice(), Some(true));
        std::fs::remove_dir_all(&dir).ok();
    }
}
