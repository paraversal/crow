//! Commands run from the palette, newest first, remembered across launches in
//! `~/.local/state/crow/palette-commands.json`. Ids are the palette's stable
//! `CommandId::key` strings, so renaming a command never breaks the file.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RecentCommands {
    ids: Vec<String>,
}

impl RecentCommands {
    /// The remembered keys, most recent first.
    pub fn ids(&self) -> &[String] {
        &self.ids
    }

    /// Put `key` at the front, dropping an older mention, and keep at most `max`.
    pub fn push(&mut self, key: &str, max: usize) {
        self.ids.retain(|id| id != key);
        self.ids.insert(0, key.to_owned());
        self.ids.truncate(max);
    }

    /// Drop keys `keep` rejects (commands this build no longer has).
    pub fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        self.ids.retain(|id| keep(id));
    }

    /// A missing or broken file means nothing remembered: state, not config.
    pub fn load(path: &Path) -> RecentCommands {
        std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string(self).map_err(std::io::Error::other)?;
        crate::config::write_atomic(path, &text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_commands_move_to_front_and_cap() {
        let mut recent = RecentCommands::default();
        for key in ["a", "b", "c", "d", "e", "f"] {
            recent.push(key, 5);
        }
        assert_eq!(recent.ids(), ["f", "e", "d", "c", "b"]);
        recent.push("c", 5);
        assert_eq!(recent.ids(), ["c", "f", "e", "d", "b"], "a repeat moves to the front once");
    }

    #[test]
    fn recent_commands_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("state/palette-commands.json");
        let mut recent = RecentCommands::default();
        recent.push("view-as-columns", 5);
        recent.push("view-as-icons", 5);
        recent.save(&file).unwrap();
        assert_eq!(RecentCommands::load(&file), recent);
    }

    #[test]
    fn broken_file_loads_empty() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("palette-commands.json");
        std::fs::write(&file, "{ not json").unwrap();
        assert_eq!(RecentCommands::load(&file), RecentCommands::default());
        assert_eq!(RecentCommands::load(&dir.path().join("missing.json")), RecentCommands::default());
    }

    #[test]
    fn retain_drops_unknown_keys() {
        let mut recent = RecentCommands::default();
        recent.push("gone", 5);
        recent.push("kept", 5);
        recent.retain(|key| key == "kept");
        assert_eq!(recent.ids(), ["kept"]);
    }
}
