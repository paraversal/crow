use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The documented default `settings.toml`, written on first launch. Parsing it
/// must give exactly `Settings::default()` (a test checks this).
pub const DEFAULT_SETTINGS: &str = r#"# Crow settings. Changes apply immediately; delete a line to get its default back.

# Theme from ~/.config/themes: "system" follows macOS light/dark through the
# theme index; otherwise a theme id such as "gruvbox-dark-hard".
theme = "system"

# "finder" (default) or "vim". The Vim layer adds hjkl, gg/G, marks and more
# on top of the Finder keys.
keymap = "finder"

# Interface font. Empty uses the system font.
font_family = ""
font_size = 13.0

show_hidden_files = false

# How new windows and tabs show folders: "list" or "columns". Cmd-2 and Cmd-3 switch.
view = "list"

[window]
# Size of every new window, in points. Restored sessions keep their own sizes.
width = 1200
height = 760
"#;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeymapLayer {
    #[default]
    Finder,
    Vim,
}

/// How a pane shows folders.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewMode {
    #[default]
    List,
    Columns,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default)]
pub struct WindowDefaults {
    pub width: u32,
    pub height: u32,
}

impl Default for WindowDefaults {
    fn default() -> Self {
        Self { width: 1200, height: 760 }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: String,
    pub keymap: KeymapLayer,
    pub font_family: String,
    pub font_size: f32,
    pub show_hidden_files: bool,
    pub view: ViewMode,
    pub window: WindowDefaults,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            keymap: KeymapLayer::Finder,
            font_family: String::new(),
            font_size: 13.0,
            show_hidden_files: false,
            view: ViewMode::List,
            window: WindowDefaults::default(),
        }
    }
}

impl Settings {
    /// `None` means "follow the system" in the theme index.
    pub fn theme_id(&self) -> Option<&str> {
        (self.theme != "system" && !self.theme.is_empty()).then_some(self.theme.as_str())
    }
}

#[derive(Debug)]
pub enum SettingsError {
    Io(PathBuf, std::io::Error),
    Parse(PathBuf, toml::de::Error),
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettingsError::Io(path, e) => write!(f, "{}: {e}", path.display()),
            SettingsError::Parse(path, e) => write!(f, "{}: {}", path.display(), e.message()),
        }
    }
}

impl std::error::Error for SettingsError {}

/// `$CROW_CONFIG_DIR`, else `$XDG_CONFIG_HOME/crow`, else `~/.config/crow` (also on macOS,
/// matching the theme index and other dotfiles-managed tools).
pub fn config_dir() -> PathBuf {
    if let Some(dir) = env::var_os("CROW_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(dir) = env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(dir).join("crow");
    }
    env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".config").join("crow")
}

/// Load `settings.toml` from `dir`. A missing file is written with the documented
/// defaults first. Unknown keys are ignored so older Crow versions keep working.
/// On error the caller keeps its last good settings and shows the message.
pub fn load_settings(dir: &Path) -> Result<Settings, SettingsError> {
    let path = dir.join("settings.toml");
    if !path.exists() {
        fs::create_dir_all(dir).map_err(|e| SettingsError::Io(dir.to_path_buf(), e))?;
        fs::write(&path, DEFAULT_SETTINGS).map_err(|e| SettingsError::Io(path.clone(), e))?;
    }
    let text = fs::read_to_string(&path).map_err(|e| SettingsError::Io(path.clone(), e))?;
    toml::from_str(&text).map_err(|e| SettingsError::Parse(path, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("crow-core-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn documented_defaults_match_code_defaults() {
        let parsed: Settings = toml::from_str(DEFAULT_SETTINGS).expect("default file parses");
        assert_eq!(parsed, Settings::default());
    }

    #[test]
    fn missing_file_is_created_with_defaults() {
        let dir = temp_dir("create");
        assert_eq!(load_settings(&dir).expect("loads"), Settings::default());
        assert!(dir.join("settings.toml").exists());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn partial_file_keeps_other_defaults_and_ignores_unknown_keys() {
        let dir = temp_dir("partial");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("settings.toml"), "keymap = \"vim\"\nfuture_option = 3\n[window]\nwidth = 900\n").unwrap();
        let s = load_settings(&dir).expect("loads");
        assert_eq!(s.keymap, KeymapLayer::Vim);
        assert_eq!(s.window, WindowDefaults { width: 900, height: 760 });
        assert_eq!(s.theme_id(), None);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bad_value_is_an_error_naming_the_file() {
        let dir = temp_dir("bad");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("settings.toml"), "keymap = \"emacs\"\n").unwrap();
        let err = load_settings(&dir).expect_err("rejects unknown layer").to_string();
        assert!(err.contains("settings.toml"), "{err}");
        fs::remove_dir_all(&dir).ok();
    }
}
