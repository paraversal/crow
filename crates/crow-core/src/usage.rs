//! How often each action has been used, so hints and tips can step aside once
//! someone knows the key. Counts live in the state dir, not the config: losing
//! them costs a hint, not a setting.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::HintsMode;

/// The file inside the state dir.
pub const USAGE_FILE: &str = "usage.json";

/// An idle hint retires after its key has been used this many times.
pub const RETIRE_AFTER: u32 = 3;

/// Counts stop here; saving is skipped once nothing changes.
pub const USAGE_CAP: u32 = 99;

/// Whether an idle hint with `uses` uses should still show in `mode`.
pub fn idle_hint_visible(mode: HintsMode, uses: u32) -> bool {
    match mode {
        HintsMode::Auto => uses < RETIRE_AFTER,
        HintsMode::Always => true,
        HintsMode::Off => false,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    counts: BTreeMap<String, u32>,
}

impl Usage {
    /// A missing or unreadable file reads as empty: this is state, never a config error.
    pub fn load(path: &Path) -> Usage {
        std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let text = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        crate::write_atomic(path, &text)
    }

    /// Add one use. False once the count is capped, so callers can skip the save.
    pub fn record(&mut self, key: &str) -> bool {
        let count = self.counts.entry(key.to_owned()).or_default();
        if *count >= USAGE_CAP {
            return false;
        }
        *count += 1;
        true
    }

    pub fn count(&self, key: &str) -> u32 {
        self.counts.get(key).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("crow-usage-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn usage_counts_and_caps() {
        let mut usage = Usage::default();
        assert_eq!(usage.count("crow::StartFilter"), 0);
        for _ in 0..100 {
            usage.record("crow::StartFilter");
        }
        assert_eq!(usage.count("crow::StartFilter"), USAGE_CAP);
        assert!(!usage.record("crow::StartFilter"), "the 100th use does not change the file");
        assert!(usage.record("crow::ShowSearch"));
    }

    #[test]
    fn usage_round_trips_through_a_file() {
        let dir = temp_dir("round-trip");
        let path = dir.join(USAGE_FILE);
        let mut usage = Usage::default();
        usage.record("crow::ShowCommandPalette");
        usage.record("crow::ShowCommandPalette");
        usage.save(&path).expect("saves");
        assert_eq!(Usage::load(&path), usage);
        assert_eq!(Usage::load(&path).count("crow::ShowCommandPalette"), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn usage_bad_file_is_empty() {
        let dir = temp_dir("bad");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(USAGE_FILE);
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(Usage::load(&path), Usage::default());
        assert_eq!(Usage::load(&dir.join("missing.json")), Usage::default());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn idle_hint_visibility_per_mode() {
        let cases = [(HintsMode::Auto, 0, true), (HintsMode::Auto, 2, true), (HintsMode::Auto, 3, false)];
        for (mode, uses, visible) in cases {
            assert_eq!(idle_hint_visible(mode, uses), visible, "{mode:?} with {uses} uses");
        }
        assert!(idle_hint_visible(HintsMode::Always, USAGE_CAP));
        assert!(!idle_hint_visible(HintsMode::Off, 0));
    }
}
