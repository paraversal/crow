use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::motion::{MotionMode, SoundSet};

/// The documented default `settings.toml`, written on first launch. Parsing it
/// must give exactly `Settings::default()` (a test checks this).
pub const DEFAULT_SETTINGS: &str = r#"# Crow settings. Changes apply immediately; delete a line to get its default back.

# Theme from ~/.config/themes: "system" follows macOS light/dark through the
# theme index; otherwise a theme id such as "gruvbox-dark-hard".
# View ▸ Theme… (Cmd-Option-T) picks one and writes it here.
theme = "system"

# "finder" (default) or "vim". The Vim layer adds hjkl, gg/G, yy/p, marks
# (m x, 'x) and more on top of the Finder keys; letters then no longer select by name.
keymap = "finder"

# What Return does in the file list: "rename" (default, like Finder) edits the
# name in place; "open" opens the item. F2 renames in either mode.
enter = "rename"

# Key hints in the bottom bar: "auto" (default), "always" or "off".
# "auto" retires each idle hint (/ filter, Cmd-F search, Cmd-K go to) after you
# have used its key 3 times; hints for the focused filter, sidebar or search stay.
# "off" hides every hint and tip.
hints = "auto"

# Contextual tips: "auto" (default) or "off". Tips point out a key the first
# time it matters. "off" mutes every tip; key hints stay as set above.
tips = "auto"

# Folder sizes in the Size column: "off" (default) leaves folders as "--";
# "visible" measures the folders on screen in the background and keeps the
# cache fresh with FSEvents. View ▸ Calculate Folder Sizes measures the current
# folder's folders once even when this is off.
folder_sizes = "off"

# Motion: "auto" (default) follows macOS Reduce Motion (full when it is off, low
# when it is on); "off" changes state in one frame; "low" fades opacity and color
# only; "full" also moves and resizes (trash swallows the row, the gap closes).
motion = "auto"

# Feedback sounds: "crow" (default) plays Finder's trash sounds plus soft ticks
# for move, new folder, undo and copy; "finder" plays only the sounds Finder
# itself makes (trash, empty trash, the alert); "off" is silent. macOS "Play user
# interface sound effects" off silences all but the alert.
sound = "crow"

# The Dock icon: "refined" (default; black crow on bone), "sun" (crow before an
# amber disc), "graphite" (folded grey planes) or "bone" (folded black planes on
# bone). It changes the Dock while Crow runs; scripts/bundle.sh bakes this choice
# into Crow.app, so Finder and Launchpad show it after the next bundle.
app_icon = "refined"

# Icons in the file list, sidebar, palette and previews: "native" (default) uses
# macOS icons. Set a pack id to use a data-only pack from icon-packs/<id>/ or a
# plugin pack named <plugin-id>.<pack-id>. views.toml can override this per folder.
icon_pack = "native"

# The bottom bar's path: "crow" (default) is the Finder-style breadcrumb that
# turns into a field on Cmd-L or a click. "explorer" keeps it a visible path
# field with a Go button; Return, Escape and Tab work the same in both.
path_bar = "crow"

# Let the sidebar show the blurred desktop behind it, like Finder's. Off by default.
# It stays opaque while macOS Reduce Transparency is on. Themes can't turn it on.
translucent_sidebar = false

# Interface font. Empty uses the system font.
font_family = ""
font_size = 13.0

# The app "Open Terminal Here" launches: a name ("Terminal", "iTerm") or a path
# to an app. The folder is passed as the app's document, the way `open -a` does.
terminal = "Terminal"

# List dotfiles and other hidden items. View ▸ Show Hidden Files (Shift-Cmd-.) toggles it.
show_hidden_files = true

# Search (Cmd-F) and Go to (Cmd-K) match dot-named items even while Show Hidden
# Files is off: dimmed, and Cmd-K ranks them after visible matches. false leaves
# them out of search until View ▸ Show Hidden Files is on. A Go to path typed
# with a leading dot still lists them.
search_includes_hidden = true

# What a new Cmd-F search looks for: "all" (names and contents), "names" or
# "contents". The panel's All/Names/Contents control changes the open search;
# this is the mode each new search starts in.
search_match = "all"

# The right column in list view and the last column in column view: a Preview tab
# and an Info tab (Cmd-I). View ▸ Hide Preview (Shift-Cmd-P) toggles the column;
# off is the default only, Cmd-I still opens it. Quick Look still works.
show_preview = true

# Cmd-K: list the commands you ran from the palette under "Recently used"
# (kept in ~/.local/state/crow/palette-commands.json). false hides the list and stops recording.
palette_recent_commands = true

# How folders are shown (list or columns, sort, list columns) is set in views.toml.
# true: each folder opens in its own view, so a window switches between list and
# columns as you move. false: a window keeps the view you give it (Cmd-2, Cmd-3,
# which then save nothing); new windows start in views.toml's [defaults] view and
# new tabs copy their window. Sort and list columns still follow each folder.
view_per_folder = true

[window]
# Size of every new window, in points. Restored sessions keep their own sizes.
width = 1200
height = 760
# Last chosen left sidebar width, in points (140–480).
sidebar_width = 210
"#;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeymapLayer {
    #[default]
    Finder,
    Vim,
}

impl KeymapLayer {
    /// The value in `settings.toml`.
    pub fn key(self) -> &'static str {
        match self {
            KeymapLayer::Finder => "finder",
            KeymapLayer::Vim => "vim",
        }
    }
}

/// What Return does in the file list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnterAction {
    /// Finder's default: edit the name in place.
    #[default]
    Rename,
    /// Open the selected item, as Cmd-Down does.
    Open,
}

impl EnterAction {
    /// The value in `settings.toml`.
    pub fn key(self) -> &'static str {
        match self {
            EnterAction::Rename => "rename",
            EnterAction::Open => "open",
        }
    }
}

/// What a new search matches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchMatch {
    /// Names and contents.
    #[default]
    All,
    Names,
    Contents,
}

impl SearchMatch {
    /// The value in `settings.toml`.
    pub fn key(self) -> &'static str {
        match self {
            SearchMatch::All => "all",
            SearchMatch::Names => "names",
            SearchMatch::Contents => "contents",
        }
    }
}

/// How a pane shows folders.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewMode {
    #[default]
    List,
    Icons,
    Columns,
    Gallery,
}

/// Which key hints the bottom bar shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HintsMode {
    /// Idle hints retire after their key has been used; focus hints stay.
    #[default]
    Auto,
    Always,
    Off,
}

impl HintsMode {
    /// The value in `settings.toml`.
    pub fn key(self) -> &'static str {
        match self {
            HintsMode::Auto => "auto",
            HintsMode::Always => "always",
            HintsMode::Off => "off",
        }
    }
}

/// Whether the contextual tips may appear.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TipsMode {
    #[default]
    Auto,
    Off,
}

impl TipsMode {
    /// The value in `settings.toml`.
    pub fn key(self) -> &'static str {
        match self {
            TipsMode::Auto => "auto",
            TipsMode::Off => "off",
        }
    }
}

/// Whether Crow measures the folder rows on screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FolderSizesMode {
    #[default]
    Off,
    Visible,
}

impl FolderSizesMode {
    /// The value in `settings.toml`.
    pub fn key(self) -> &'static str {
        match self {
            FolderSizesMode::Off => "off",
            FolderSizesMode::Visible => "visible",
        }
    }
}

/// Which of Crow's icons the Dock shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppIcon {
    #[default]
    Refined,
    Sun,
    Graphite,
    Bone,
}

impl AppIcon {
    /// The name in `settings.toml`, which is also the asset's file stem.
    pub fn key(self) -> &'static str {
        match self {
            AppIcon::Refined => "refined",
            AppIcon::Sun => "sun",
            AppIcon::Graphite => "graphite",
            AppIcon::Bone => "bone",
        }
    }
}

/// How the bottom bar shows its path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PathBarMode {
    /// Finder-style breadcrumb; hover or the pencil reveals editing.
    #[default]
    Crow,
    /// A visible path field with a Go button.
    Explorer,
}

impl PathBarMode {
    /// The value in `settings.toml`.
    pub fn key(self) -> &'static str {
        match self {
            PathBarMode::Crow => "crow",
            PathBarMode::Explorer => "explorer",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default)]
pub struct WindowDefaults {
    pub width: u32,
    pub height: u32,
    pub sidebar_width: u32,
}

impl Default for WindowDefaults {
    fn default() -> Self {
        Self { width: 1200, height: 760, sidebar_width: 210 }
    }
}

fn native_icon_pack() -> String {
    crate::icon_pack::NATIVE_ICON_SOURCE.to_owned()
}

fn icon_pack_value<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let value = String::deserialize(deserializer)?;
    crate::icon_pack::validate_source(&value).map_err(serde::de::Error::custom)?;
    Ok(value)
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: String,
    pub keymap: KeymapLayer,
    pub enter: EnterAction,
    pub hints: HintsMode,
    pub tips: TipsMode,
    pub folder_sizes: FolderSizesMode,
    pub motion: MotionMode,
    pub sound: SoundSet,
    pub app_icon: AppIcon,
    #[serde(default = "native_icon_pack", deserialize_with = "icon_pack_value")]
    pub icon_pack: String,
    pub path_bar: PathBarMode,
    pub translucent_sidebar: bool,
    pub font_family: String,
    pub font_size: f32,
    pub terminal: String,
    pub show_hidden_files: bool,
    pub search_includes_hidden: bool,
    pub search_match: SearchMatch,
    pub show_preview: bool,
    pub palette_recent_commands: bool,
    pub view_per_folder: bool,
    pub window: WindowDefaults,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            keymap: KeymapLayer::Finder,
            enter: EnterAction::Rename,
            hints: HintsMode::Auto,
            tips: TipsMode::Auto,
            folder_sizes: FolderSizesMode::Off,
            motion: MotionMode::Auto,
            sound: SoundSet::Crow,
            app_icon: AppIcon::Refined,
            icon_pack: native_icon_pack(),
            path_bar: PathBarMode::Crow,
            translucent_sidebar: false,
            font_family: String::new(),
            font_size: 13.0,
            terminal: "Terminal".into(),
            show_hidden_files: true,
            search_includes_hidden: true,
            search_match: SearchMatch::All,
            show_preview: true,
            palette_recent_commands: true,
            view_per_folder: true,
            window: WindowDefaults::default(),
        }
    }
}

impl Settings {
    /// `None` means "follow the system" in the theme index.
    pub fn theme_id(&self) -> Option<&str> {
        (self.theme != "system" && !self.theme.is_empty()).then_some(self.theme.as_str())
    }

    /// Whether tips are muted: `tips = "off"`, or `hints = "off"` as the older
    /// one-switch fallback.
    pub fn tips_muted(&self) -> bool {
        self.tips == TipsMode::Off || self.hints == HintsMode::Off
    }
}

#[derive(Debug)]
pub enum SettingsError {
    Io(PathBuf, std::io::Error),
    /// Already names the file and, when TOML points at one, the line.
    Parse(String),
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettingsError::Io(path, e) => write!(f, "{}: {e}", path.display()),
            SettingsError::Parse(message) => f.write_str(message),
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

/// Set one top-level key in `settings.toml`, keeping the rest of the file and its comments.
pub fn save_setting(dir: &Path, key: &str, value: &str) -> Result<(), String> {
    save_value(dir, key, toml_edit::Value::from(value))
}

/// Set a boolean top-level key, keeping the rest of the file and its comments.
pub fn save_bool_setting(dir: &Path, key: &str, value: bool) -> Result<(), String> {
    save_value(dir, key, toml_edit::Value::from(value))
}

/// Set a numeric top-level key, keeping the rest of the file and its comments.
/// Writes a float, so `font_size` keeps its `.0`.
pub fn save_number_setting(dir: &Path, key: &str, value: f64) -> Result<(), String> {
    save_value(dir, key, toml_edit::Value::from(value))
}

/// Set a numeric key in a table such as `[window]`, keeping the rest of the file
/// and its comments. Writes an integer, so `width = 900` stays an integer.
pub fn save_table_number(dir: &Path, table: &str, key: &str, value: i64) -> Result<(), String> {
    crate::config::edit_toml(&dir.join("settings.toml"), DEFAULT_SETTINGS, |doc| {
        let mut value = toml_edit::Value::from(value);
        let table = doc
            .entry(table)
            .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()))
            .as_table_mut()
            .ok_or_else(|| format!("settings.toml: `{table}` is not a table"))?;
        if let Some(old) = table.get(key).and_then(toml_edit::Item::as_value) {
            *value.decor_mut() = old.decor().clone();
        }
        table[key] = toml_edit::Item::Value(value);
        Ok(())
    })
}

fn save_value(dir: &Path, key: &str, mut value: toml_edit::Value) -> Result<(), String> {
    crate::config::edit_toml(&dir.join("settings.toml"), DEFAULT_SETTINGS, |doc| {
        if let Some(old) = doc.get(key).and_then(toml_edit::Item::as_value) {
            *value.decor_mut() = old.decor().clone();
        }
        doc[key] = toml_edit::Item::Value(value);
        Ok(())
    })
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
    toml::from_str(&text)
        .map_err(|e| SettingsError::Parse(crate::config::file_message(&path, crate::config::parse_error(&e, &text))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> crate::test_support::TempDir {
        crate::test_support::TempDir::new(&format!("crow-core-{name}-"))
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
    }

    #[test]
    fn partial_file_keeps_other_defaults_and_ignores_unknown_keys() {
        let dir = temp_dir("partial");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("settings.toml"), "keymap = \"vim\"\nfuture_option = 3\n[window]\nwidth = 900\n").unwrap();
        let s = load_settings(&dir).expect("loads");
        assert_eq!(s.keymap, KeymapLayer::Vim);
        assert_eq!(s.window, WindowDefaults { width: 900, ..WindowDefaults::default() });
        assert_eq!(s.theme_id(), None);
    }

    #[test]
    fn icon_pack_defaults_to_native_and_rejects_malformed_ids() {
        let parsed: Settings = toml::from_str("theme = \"system\"").unwrap();
        assert_eq!(parsed.icon_pack, "native");
        let parsed: Settings = toml::from_str("icon_pack = \"example.pack.theme\"").unwrap();
        assert_eq!(parsed.icon_pack, "example.pack.theme");
        assert!(toml::from_str::<Settings>("icon_pack = \"My Pack\"").is_err());
        assert!(toml::from_str::<Settings>("icon_pack = \"\"").is_err());
    }

    #[test]
    fn path_bar_values_parse_and_default_to_crow() {
        let parse = |text: &str| toml::from_str::<Settings>(text).expect("parses");
        assert_eq!(parse("path_bar = \"crow\"").path_bar, PathBarMode::Crow);
        assert_eq!(parse("path_bar = \"explorer\"").path_bar, PathBarMode::Explorer);
        assert_eq!(parse("theme = \"system\"").path_bar, PathBarMode::Crow, "an old config is Crow");

        let dir = temp_dir("path-bar-bad");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("settings.toml"), "path_bar = \"windows\"\n").unwrap();
        let err = load_settings(&dir).expect_err("rejects an unknown mode").to_string();
        assert!(err.contains("settings.toml"), "{err}");
        assert!(err.contains("line 1"), "{err}");
    }

    #[test]
    fn saving_path_bar_keeps_an_adjacent_comment_and_other_keys() {
        let dir = temp_dir("save-path-bar");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("settings.toml"), "# mine\npath_bar = \"crow\" # bottom bar\nkeymap = \"vim\"\n").unwrap();
        let before = fs::read_to_string(dir.join("settings.toml")).unwrap();
        save_setting(&dir, "path_bar", PathBarMode::Explorer.key()).unwrap();
        let after = fs::read_to_string(dir.join("settings.toml")).unwrap();
        assert!(after.contains("\npath_bar = \"explorer\" # bottom bar\n"), "{after}");
        assert!(after.contains("\nkeymap = \"vim\"\n"), "{after}");
        let changed: Vec<_> = before.lines().zip(after.lines()).filter(|(a, b)| a != b).collect();
        assert_eq!(changed.len(), 1, "only the path_bar line changes: {changed:?}");
        assert_eq!(load_settings(&dir).unwrap().path_bar, PathBarMode::Explorer);
    }

    #[test]
    fn saving_a_setting_keeps_comments() {
        let dir = temp_dir("save");
        load_settings(&dir).unwrap();
        save_setting(&dir, "theme", "gruvbox-dark-hard").unwrap();
        let text = fs::read_to_string(dir.join("settings.toml")).unwrap();
        assert!(text.starts_with("# Crow settings."));
        assert!(text.contains("\ntheme = \"gruvbox-dark-hard\"\n"), "{text}");
        assert_eq!(load_settings(&dir).unwrap().theme_id(), Some("gruvbox-dark-hard"));
    }

    #[test]
    fn saving_a_bool_setting_writes_a_bool() {
        let dir = temp_dir("save-bool");
        load_settings(&dir).unwrap();
        save_bool_setting(&dir, "show_preview", false).unwrap();
        let text = fs::read_to_string(dir.join("settings.toml")).unwrap();
        assert!(text.contains("\nshow_preview = false\n"), "{text}");
        assert!(!text.contains("show_preview = \"false\""), "a string would fail to reload: {text}");
        assert!(!load_settings(&dir).unwrap().show_preview, "reloads as off");
        save_bool_setting(&dir, "show_preview", true).unwrap();
        assert!(load_settings(&dir).unwrap().show_preview);
        assert!(!load_settings(&dir).unwrap().translucent_sidebar, "translucency is opt-in");
        save_bool_setting(&dir, "translucent_sidebar", true).unwrap();
        assert!(load_settings(&dir).unwrap().translucent_sidebar);
    }

    #[test]
    fn saving_a_number_setting_keeps_comments_and_writes_a_float() {
        let dir = temp_dir("save-number");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("settings.toml"), "# mine\nfont_size = 13.0 # readable\nkeymap = \"vim\"\n").unwrap();
        save_number_setting(&dir, "font_size", 15.0).unwrap();
        let text = fs::read_to_string(dir.join("settings.toml")).unwrap();
        assert!(text.starts_with("# mine\n"), "{text}");
        assert!(text.contains("\nfont_size = 15.0 # readable\n"), "{text}");
        assert!(text.contains("\nkeymap = \"vim\"\n"), "{text}");
        assert_eq!(load_settings(&dir).unwrap().font_size, 15.0);
    }

    #[test]
    fn saving_a_table_number_keeps_comments_and_writes_an_integer() {
        let dir = temp_dir("save-table-number");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("settings.toml"),
            "# mine\n\n[window]\n# new windows\nwidth = 1200 # points\nheight = 760\n",
        )
        .unwrap();
        save_table_number(&dir, "window", "width", 900).unwrap();
        let text = fs::read_to_string(dir.join("settings.toml")).unwrap();
        assert!(text.starts_with("# mine\n"), "{text}");
        assert!(text.contains("\n[window]\n# new windows\nwidth = 900 # points\n"), "{text}");
        assert!(text.contains("\nheight = 760\n"), "{text}");
        assert!(!text.contains("width = 900.0"), "an integer stays an integer: {text}");
        assert_eq!(load_settings(&dir).unwrap().window.width, 900);
    }

    #[test]
    fn saving_a_table_number_creates_the_table() {
        let dir = temp_dir("save-table-new");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("settings.toml"), "keymap = \"vim\"\n").unwrap();
        save_table_number(&dir, "window", "height", 640).unwrap();
        let text = fs::read_to_string(dir.join("settings.toml")).unwrap();
        assert!(text.contains("\n[window]\nheight = 640\n"), "{text}");
        assert_eq!(load_settings(&dir).unwrap().window.height, 640);
    }

    #[test]
    fn hints_values_parse() {
        let parse = |text: &str| toml::from_str::<Settings>(text).expect("parses");
        assert_eq!(parse("hints = \"auto\"").hints, HintsMode::Auto);
        assert_eq!(parse("hints = \"always\"").hints, HintsMode::Always);
        assert_eq!(parse("hints = \"off\"").hints, HintsMode::Off);
        assert_eq!(parse("theme = \"system\"").hints, HintsMode::Auto, "a missing line is Auto");

        let dir = temp_dir("hints-bad");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("settings.toml"), "hints = \"sometimes\"\n").unwrap();
        let err = load_settings(&dir).expect_err("rejects an unknown mode").to_string();
        assert!(err.contains("settings.toml"), "{err}");
        assert!(err.contains("line 1"), "{err}");
    }

    #[test]
    fn folder_sizes_values_parse() {
        let parse = |text: &str| toml::from_str::<Settings>(text).expect("parses");
        assert_eq!(parse("folder_sizes = \"off\"").folder_sizes, FolderSizesMode::Off);
        assert_eq!(parse("folder_sizes = \"visible\"").folder_sizes, FolderSizesMode::Visible);
        assert_eq!(parse("theme = \"system\"").folder_sizes, FolderSizesMode::Off, "a missing line is off");
    }

    #[test]
    fn tips_values_parse_and_mute() {
        let parse = |text: &str| toml::from_str::<Settings>(text).expect("parses");
        assert_eq!(parse("tips = \"auto\"").tips, TipsMode::Auto);
        assert_eq!(parse("tips = \"off\"").tips, TipsMode::Off);
        assert_eq!(parse("theme = \"system\"").tips, TipsMode::Auto, "a missing line is Auto");

        let dir = temp_dir("tips-bad");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("settings.toml"), "tips = \"sometimes\"\n").unwrap();
        let err = load_settings(&dir).expect_err("rejects an unknown mode").to_string();
        assert!(err.contains("settings.toml"), "{err}");
        assert!(err.contains("line 1"), "{err}");

        assert!(!Settings::default().tips_muted());
        assert!(parse("tips = \"off\"").tips_muted(), "tips = off mutes");
        assert!(parse("hints = \"off\"").tips_muted(), "hints = off still mutes");
        assert_eq!(parse("tips = \"off\"").hints, HintsMode::Auto, "tips = off leaves hints alone");
        assert_eq!(parse("hints = \"off\"").tips, TipsMode::Auto, "hints = off leaves tips alone");
    }

    #[test]
    fn motion_and_sound_values_parse() {
        let parse = |text: &str| toml::from_str::<Settings>(text).expect("parses");
        assert_eq!(parse("motion = \"auto\"").motion, MotionMode::Auto);
        assert_eq!(parse("motion = \"off\"").motion, MotionMode::Off);
        assert_eq!(parse("motion = \"low\"").motion, MotionMode::Low);
        assert_eq!(parse("motion = \"full\"").motion, MotionMode::Full);
        assert_eq!(parse("sound = \"off\"").sound, SoundSet::Off);
        assert_eq!(parse("sound = \"finder\"").sound, SoundSet::Finder);
        assert_eq!(parse("sound = \"crow\"").sound, SoundSet::Crow);
        assert_eq!(parse("theme = \"system\"").motion, MotionMode::Auto, "a missing line is Auto");
        assert_eq!(parse("theme = \"system\"").sound, SoundSet::Crow, "a missing line is Crow");

        let dir = temp_dir("motion-bad");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("settings.toml"), "motion = \"slow\"\n").unwrap();
        let err = load_settings(&dir).expect_err("rejects an unknown mode").to_string();
        assert!(err.contains("settings.toml"), "{err}");
        assert!(err.contains("line 1"), "{err}");
    }

    #[test]
    fn app_icon_values_parse() {
        let parse = |text: &str| toml::from_str::<Settings>(text).expect("parses");
        for icon in [AppIcon::Refined, AppIcon::Sun, AppIcon::Graphite, AppIcon::Bone] {
            assert_eq!(parse(&format!("app_icon = \"{}\"", icon.key())).app_icon, icon);
        }
        assert_eq!(parse("theme = \"system\"").app_icon, AppIcon::Refined, "a missing line is Refined");
        assert!(toml::from_str::<Settings>("app_icon = \"moon\"").is_err());
    }

    #[test]
    fn bad_value_is_an_error_naming_the_file() {
        let dir = temp_dir("bad");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("settings.toml"), "keymap = \"emacs\"\n").unwrap();
        let err = load_settings(&dir).expect_err("rejects unknown layer").to_string();
        assert!(err.contains("settings.toml"), "{err}");
    }
}
