//! The Settings window: one scrolling page of controls bound to `settings.toml`.
//! The file stays the source of truth: every control reads `Ui.settings` on
//! render, and every change writes only its key through `ui::set_setting`.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crow_core::motion::{self, Effective, Event, MotionMode, SoundSet};
use crow_core::{
    DefaultsChange, EnterAction, FolderSizesMode, HintsMode, KeymapLayer, PathBarMode, SearchMatch, TipsMode, ViewMode,
};
use crow_fs::SortKey;
use crow_plugin::{PluginEntry, PluginHandle, PluginState};
use crow_theme::Theme;
use gpui::{
    App, Bounds, Context, Entity, FocusHandle, Focusable, FontWeight, IntoElement, KeyBinding, KeyDownEvent, Render,
    Role, ScrollHandle, SharedString, Subscription, TitlebarOptions, Window, WindowBounds, WindowOptions, actions, div,
    prelude::*, px, size,
};

use crate::actions::OpenSettings;
use crate::controls::{self, ControlMeta};
use crate::icons::{Glyph, glyph};
use crate::plugins::Plugins;
use crate::style::{active_selection, color, cursor_ring, error_soft, inactive_selection, ui_font};
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme_choice::{self, ThemeChoice};
use crate::ui::{self, SettingChange, Ui};

mod catalog;
mod keyboard;
mod preview;
mod saved_queries;
mod search;
use catalog::Category;

actions!(
    settings_window,
    [
        /// Space or Return on the focused control.
        Activate,
        /// Tab.
        FocusNext,
        /// Shift-Tab.
        FocusPrev,
        /// Shift-↑ on a stepper.
        IncrementTen,
        /// Shift-↓ on a stepper.
        DecrementTen,
        /// ↑ on the focused control.
        Up,
        /// ↓ on the focused control.
        Down,
        /// ← on the focused control.
        Left,
        /// → on the focused control.
        Right,
        /// Escape: closes an open drop-down, clears search, otherwise the window.
        Cancel,
        /// ⌘F.
        FocusSearch,
        /// ⌘W.
        CloseSettings,
        /// Delete on a Keyboard row: back to the default shortcut.
        ResetShortcut
    ]
);

/// The settings window's keys. Text fields keep their own keys (Tab and Return
/// arrive as `TextInputEvent`s); ↑ ↓ are free in a field, so a stepper uses them.
pub fn keymap() -> Vec<KeyBinding> {
    let settings = Some("Settings");
    vec![
        KeyBinding::new("space", Activate, Some("Settings && !TextInput")),
        KeyBinding::new("enter", Activate, Some("Settings && !TextInput")),
        KeyBinding::new("left", Left, settings),
        KeyBinding::new("right", Right, settings),
        KeyBinding::new("up", Up, settings),
        KeyBinding::new("down", Down, settings),
        KeyBinding::new("shift-up", IncrementTen, settings),
        KeyBinding::new("shift-down", DecrementTen, settings),
        KeyBinding::new("tab", FocusNext, settings),
        KeyBinding::new("shift-tab", FocusPrev, settings),
        KeyBinding::new("escape", Cancel, settings),
        KeyBinding::new("cmd-w", CloseSettings, settings),
        KeyBinding::new("cmd-f", FocusSearch, settings),
        KeyBinding::new("backspace", ResetShortcut, Some("Settings && !TextInput")),
        KeyBinding::new("delete", ResetShortcut, Some("Settings && !TextInput")),
        KeyBinding::new("cmd-,", OpenSettings, settings),
    ]
    .into_iter()
    .chain(crate::window::saved_query_form::keymap())
    .collect()
}

/// Open the one Settings window, or bring it to the front and focus its first
/// control. Called by ⌘, and Crow ▸ Settings….
pub fn open(cx: &mut App) {
    // The invoking browser's folder is the preview editor's Use Current Folder
    // source. When Settings itself is active, keep the last browser's folder.
    if let Some(handle) = cx.active_window()
        && let Some(browser) = handle.downcast::<crate::window::CrowWindow>()
        && let Ok(dir) = browser.update(cx, |window, _, cx| window.focused_folder(cx))
    {
        cx.global_mut::<Ui>().settings_folder = Some(dir);
    }
    if !cx.windows().iter().any(|handle| handle.downcast::<crate::window::CrowWindow>().is_some()) {
        cx.global_mut::<Ui>().settings_folder = None;
    }
    open_window(cx);
}

/// Open from a browser action, which already has access to the invoking pane.
pub(crate) fn open_for_folder(dir: std::path::PathBuf, cx: &mut App) {
    cx.global_mut::<Ui>().settings_folder = Some(dir);
    open_window(cx);
}

fn open_window(cx: &mut App) {
    if let Some(handle) = cx.windows().into_iter().find_map(|handle| handle.downcast::<SettingsWindow>()) {
        handle
            .update(cx, |this, window, cx| {
                window.activate_window();
                this.focus_first(window, cx);
            })
            .ok();
        cx.activate(true);
        return;
    }
    let bounds = Bounds::centered(None, size(px(760.), px(760.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some("Settings".into()),
            appears_transparent: true,
            traffic_light_position: Some(gpui::point(px(14.), px(19.))),
        }),
        // Like the main window: the band below moves the window.
        app_owns_titlebar_drag: true,
        is_resizable: true,
        window_min_size: Some(size(px(640.), px(420.))),
        ..Default::default()
    };
    match cx.open_window(options, |window, cx| {
        let view = cx.new(|cx| SettingsWindow::new(window, cx));
        let focus = view.read(cx).first_focus(cx);
        window.focus(&focus, cx);
        view
    }) {
        Ok(handle) => {
            handle.update(cx, |_, window, _| window.activate_window()).ok();
            cx.activate(true);
        }
        Err(e) => tracing::error!("open settings window: {e}"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ControlId {
    Theme,
    IconStyle,
    IconPack,
    PathBar,
    SidebarStyle,
    TranslucentSidebar,
    FontFamily,
    FontSize,
    Terminal,
    Keymap,
    Enter,
    Hints,
    Tips,
    FolderSizes,
    ShowHidden,
    SearchHidden,
    SearchMatch,
    SavedQueryList,
    NewSavedQuery,
    ShowPreview,
    ViewPerFolder,
    PaletteRecent,
    Motion,
    Sound,
    PlaySound,
    WindowWidth,
    WindowHeight,
    DefaultView,
    DefaultSort,
    DefaultDirection,
    DefaultFoldersFirst,
    PreviewScope,
    PreviewCurrentFolder,
    PreviewChooseFolder,
    PreviewRuleSelect,
    PreviewRulePattern,
    PreviewAddRule,
    PreviewList,
    PreviewMoveUp,
    PreviewMoveDown,
    PreviewRemove,
    PreviewAdd,
    PreviewClear,
    Plugins,
    KeyboardSearch,
    KeyboardList,
    KeyboardResetAll,
    EditSettings,
    ShowConfig,
    OpenSidebarToml,
    OpenViewsToml,
    OpenExcludeToml,
    OpenPluginsToml,
    OpenSavedQueriesToml,
    OpenKeysToml,
}

impl ControlId {
    /// Every control, used to build focus handles. Reading order comes from
    /// `catalog::units`, so a category can be reordered without touching this.
    const ALL: [ControlId; 55] = [
        ControlId::Theme,
        ControlId::TranslucentSidebar,
        ControlId::IconStyle,
        ControlId::IconPack,
        ControlId::PathBar,
        ControlId::SidebarStyle,
        ControlId::FontFamily,
        ControlId::FontSize,
        ControlId::Keymap,
        ControlId::Enter,
        ControlId::Hints,
        ControlId::Tips,
        ControlId::Terminal,
        ControlId::PaletteRecent,
        ControlId::Motion,
        ControlId::Sound,
        ControlId::PlaySound,
        ControlId::WindowWidth,
        ControlId::WindowHeight,
        ControlId::ShowHidden,
        ControlId::FolderSizes,
        ControlId::DefaultView,
        ControlId::DefaultSort,
        ControlId::DefaultDirection,
        ControlId::DefaultFoldersFirst,
        ControlId::PreviewScope,
        ControlId::PreviewCurrentFolder,
        ControlId::PreviewChooseFolder,
        ControlId::PreviewRuleSelect,
        ControlId::PreviewRulePattern,
        ControlId::PreviewAddRule,
        ControlId::PreviewList,
        ControlId::PreviewMoveUp,
        ControlId::PreviewMoveDown,
        ControlId::PreviewRemove,
        ControlId::PreviewAdd,
        ControlId::PreviewClear,
        ControlId::ShowPreview,
        ControlId::ViewPerFolder,
        ControlId::SearchHidden,
        ControlId::SearchMatch,
        ControlId::SavedQueryList,
        ControlId::NewSavedQuery,
        ControlId::Plugins,
        ControlId::KeyboardSearch,
        ControlId::KeyboardList,
        ControlId::KeyboardResetAll,
        ControlId::EditSettings,
        ControlId::ShowConfig,
        ControlId::OpenSidebarToml,
        ControlId::OpenViewsToml,
        ControlId::OpenExcludeToml,
        ControlId::OpenPluginsToml,
        ControlId::OpenSavedQueriesToml,
        ControlId::OpenKeysToml,
    ];

    fn key(self) -> &'static str {
        match self {
            ControlId::Theme => "settings-theme",
            ControlId::IconStyle => "settings-icon-style",
            ControlId::IconPack => "settings-icon-pack",
            ControlId::PathBar => "settings-path-bar",
            ControlId::SidebarStyle => "settings-sidebar-style",
            ControlId::FontFamily => "settings-font-family",
            ControlId::FontSize => "settings-font-size",
            ControlId::Terminal => "settings-terminal",
            ControlId::Keymap => "settings-keymap",
            ControlId::Enter => "settings-enter",
            ControlId::Hints => "settings-hints",
            ControlId::Tips => "settings-tips",
            ControlId::FolderSizes => "settings-folder-sizes",
            ControlId::ShowHidden => "settings-show-hidden",
            ControlId::SearchHidden => "settings-search-hidden",
            ControlId::SearchMatch => "settings-search-match",
            ControlId::SavedQueryList => "settings-saved-query-list",
            ControlId::NewSavedQuery => "settings-new-saved-query",
            ControlId::ShowPreview => "settings-show-preview",
            ControlId::ViewPerFolder => "settings-view-per-folder",
            ControlId::PaletteRecent => "settings-palette-recent",
            ControlId::TranslucentSidebar => "settings-translucent-sidebar",
            ControlId::Motion => "settings-motion",
            ControlId::Sound => "settings-sound",
            ControlId::PlaySound => "settings-play-sound",
            ControlId::WindowWidth => "settings-window-width",
            ControlId::WindowHeight => "settings-window-height",
            ControlId::DefaultView => "settings-default-view",
            ControlId::DefaultSort => "settings-default-sort",
            ControlId::DefaultDirection => "settings-default-direction",
            ControlId::DefaultFoldersFirst => "settings-default-folders-first",
            ControlId::PreviewScope => "settings-preview-scope",
            ControlId::PreviewCurrentFolder => "settings-preview-current-folder",
            ControlId::PreviewChooseFolder => "settings-preview-choose-folder",
            ControlId::PreviewRuleSelect => "settings-preview-rule",
            ControlId::PreviewRulePattern => "settings-preview-rule-pattern",
            ControlId::PreviewAddRule => "settings-preview-add-rule",
            ControlId::PreviewList => "settings-preview-list",
            ControlId::PreviewMoveUp => "settings-preview-move-up",
            ControlId::PreviewMoveDown => "settings-preview-move-down",
            ControlId::PreviewRemove => "settings-preview-remove",
            ControlId::PreviewAdd => "settings-preview-add",
            ControlId::PreviewClear => "settings-preview-clear",
            ControlId::Plugins => "settings-plugins",
            ControlId::KeyboardSearch => "settings-keyboard-search",
            ControlId::KeyboardList => "settings-keyboard-list",
            ControlId::KeyboardResetAll => "settings-keyboard-reset-all",
            ControlId::EditSettings => "settings-edit-file",
            ControlId::ShowConfig => "settings-show-config",
            ControlId::OpenSidebarToml => "settings-open-sidebar",
            ControlId::OpenViewsToml => "settings-open-views",
            ControlId::OpenExcludeToml => "settings-open-exclude",
            ControlId::OpenPluginsToml => "settings-open-plugins",
            ControlId::OpenSavedQueriesToml => "settings-open-saved-queries",
            ControlId::OpenKeysToml => "settings-open-keys",
        }
    }

    /// The `debug_selector` the control's shared builder gives it: the group
    /// widgets append `-control`; buttons and links use the key itself.
    #[cfg(any(test, feature = "fuzz"))]
    fn selector(self) -> String {
        match self {
            ControlId::Theme
            | ControlId::IconPack
            | ControlId::FontFamily
            | ControlId::FontSize
            | ControlId::Terminal
            | ControlId::Keymap
            | ControlId::Enter
            | ControlId::Hints
            | ControlId::Tips
            | ControlId::FolderSizes
            | ControlId::PathBar
            | ControlId::IconStyle
            | ControlId::SidebarStyle
            | ControlId::ShowHidden
            | ControlId::SearchHidden
            | ControlId::SearchMatch
            | ControlId::ShowPreview
            | ControlId::ViewPerFolder
            | ControlId::PaletteRecent
            | ControlId::TranslucentSidebar
            | ControlId::Motion
            | ControlId::Sound
            | ControlId::WindowWidth
            | ControlId::WindowHeight
            | ControlId::DefaultView
            | ControlId::DefaultSort
            | ControlId::DefaultDirection
            | ControlId::DefaultFoldersFirst
            | ControlId::PreviewScope
            | ControlId::PreviewRuleSelect
            | ControlId::PreviewList
            | ControlId::PreviewAdd => format!("{}-control", self.key()),
            _ => self.key().to_owned(),
        }
    }

    fn label(self) -> &'static str {
        match self {
            ControlId::Theme => "Theme",
            ControlId::IconStyle => "Default icons",
            ControlId::IconPack => "Icon packs",
            ControlId::PathBar => "Path bar",
            ControlId::SidebarStyle => "Sidebar style",
            ControlId::FontFamily => "Font",
            ControlId::FontSize => "Font size",
            ControlId::Terminal => "Terminal app",
            ControlId::Keymap => "Keymap",
            ControlId::Enter => "What Return does",
            ControlId::Hints => "Key hints",
            ControlId::Tips => "Tips",
            ControlId::FolderSizes => "Folder sizes",
            ControlId::ShowHidden => "Show hidden files",
            ControlId::SearchHidden => "Search always includes hidden files",
            ControlId::SearchMatch => "Search matches",
            ControlId::SavedQueryList => "Saved Queries",
            ControlId::NewSavedQuery => "New Saved Query…",
            ControlId::ShowPreview => "Show preview",
            ControlId::ViewPerFolder => "Folders keep their own view",
            ControlId::PaletteRecent => "Remember recent commands in ⌘K",
            ControlId::TranslucentSidebar => "Translucent sidebar",
            ControlId::Motion => "Motion",
            ControlId::Sound => "Sounds",
            ControlId::PlaySound => "Play",
            ControlId::WindowWidth => "Width",
            ControlId::WindowHeight => "Height",
            ControlId::DefaultView => "Default view",
            ControlId::DefaultSort => "Sort by",
            ControlId::DefaultDirection => "Sort direction",
            ControlId::DefaultFoldersFirst => "Folders first",
            ControlId::PreviewScope => "Scope",
            ControlId::PreviewCurrentFolder => "Use Current Folder",
            ControlId::PreviewChooseFolder => "Choose Folder…",
            ControlId::PreviewRuleSelect => "Path rule",
            ControlId::PreviewRulePattern => "New rule pattern",
            ControlId::PreviewAddRule => "Add Preview Rule",
            ControlId::PreviewList => "Providers",
            ControlId::KeyboardSearch => "Search shortcuts",
            ControlId::KeyboardList => "Keyboard shortcuts",
            ControlId::KeyboardResetAll => "Reset all",
            ControlId::PreviewMoveUp => "Move Up",
            ControlId::PreviewMoveDown => "Move Down",
            ControlId::PreviewRemove => "Remove",
            ControlId::PreviewAdd => "Add Provider",
            ControlId::PreviewClear => "Clear preview override",
            ControlId::Plugins => "Plugins",
            ControlId::EditSettings => "Edit settings.toml…",
            ControlId::ShowConfig => "Open config folder",
            ControlId::OpenSidebarToml => "sidebar.toml",
            ControlId::OpenViewsToml => "views.toml",
            ControlId::OpenExcludeToml => "exclude.toml",
            ControlId::OpenPluginsToml => "plugins.toml",
            ControlId::OpenSavedQueriesToml => "saved-queries.toml",
            ControlId::OpenKeysToml => "keys.toml",
        }
    }

    #[allow(dead_code)] // the accessibility test and M7 H7's fuzzer walk it
    fn role(self) -> Role {
        match self {
            ControlId::Theme
            | ControlId::FontFamily
            | ControlId::IconPack
            | ControlId::DefaultSort
            | ControlId::PreviewRuleSelect
            | ControlId::PreviewAdd => Role::ComboBox,
            ControlId::Keymap
            | ControlId::Enter
            | ControlId::Hints
            | ControlId::Tips
            | ControlId::FolderSizes
            | ControlId::PathBar
            | ControlId::IconStyle
            | ControlId::SidebarStyle
            | ControlId::Motion
            | ControlId::Sound
            | ControlId::SearchMatch
            | ControlId::DefaultView
            | ControlId::DefaultDirection
            | ControlId::PreviewScope => Role::RadioGroup,
            ControlId::ShowHidden
            | ControlId::SearchHidden
            | ControlId::ShowPreview
            | ControlId::ViewPerFolder
            | ControlId::PaletteRecent
            | ControlId::TranslucentSidebar
            | ControlId::DefaultFoldersFirst => Role::Switch,
            ControlId::FontSize | ControlId::WindowWidth | ControlId::WindowHeight => Role::SpinButton,
            ControlId::Terminal | ControlId::PreviewRulePattern => Role::TextInput,
            ControlId::KeyboardSearch => Role::SearchInput,
            ControlId::Plugins | ControlId::PreviewList | ControlId::SavedQueryList | ControlId::KeyboardList => {
                Role::ListBox
            }
            ControlId::PlaySound
            | ControlId::KeyboardResetAll
            | ControlId::NewSavedQuery
            | ControlId::EditSettings
            | ControlId::ShowConfig
            | ControlId::OpenSidebarToml
            | ControlId::OpenViewsToml
            | ControlId::OpenExcludeToml
            | ControlId::OpenPluginsToml
            | ControlId::OpenSavedQueriesToml
            | ControlId::OpenKeysToml
            | ControlId::PreviewCurrentFolder
            | ControlId::PreviewChooseFolder
            | ControlId::PreviewAddRule
            | ControlId::PreviewMoveUp
            | ControlId::PreviewMoveDown
            | ControlId::PreviewRemove
            | ControlId::PreviewClear => Role::Button,
        }
    }

    fn description(self, cx: &App) -> Option<SharedString> {
        let text = match self {
            ControlId::Theme => "Themes from ~/.config/themes.",
            ControlId::IconStyle => {
                "Outline draws the sidebar's folders as line glyphs, like Finder; Finder uses its colored icons."
            }
            ControlId::IconPack => {
                "Tick packs to override the default icons; the first pack with an icon wins. A folder's views.toml source wins."
            }
            ControlId::PathBar => "Crow fades in an editor over the breadcrumb; Explorer keeps a visible path field.",
            ControlId::SidebarStyle => "How the sidebar draws its groups. Any group can carry an icon in sidebar.toml.",
            ControlId::FontFamily => "Installed families; System follows macOS.",
            ControlId::FontSize => "10 to 24 points.",
            ControlId::Terminal => {
                "The app Open Terminal Here opens: a name like Terminal or iTerm, or a path to an app."
            }
            ControlId::Keymap => "Vim adds hjkl, gg/G, yy/p and marks on top of the Finder keys.",
            ControlId::Enter => "Rename edits the name in place; Open opens the item. F2 always renames.",
            ControlId::Hints => "Auto retires idle hints after you use their key three times.",
            ControlId::Tips => "Point out a key the first time it matters; off mutes every tip.",
            ControlId::FolderSizes => "Measure the folders on screen in the background; off leaves them as --.",
            ControlId::ShowHidden => "List dotfiles and other hidden items.",
            ControlId::SearchHidden => {
                "Find dot items while Show hidden files is off; Go to ranks them after visible matches."
            }
            ControlId::SearchMatch => "What each new Find looks for; the panel's control changes the open search.",
            ControlId::SavedQueryList | ControlId::NewSavedQuery => return None,
            ControlId::ShowPreview => "The right column's default in list and column view: Preview and Info tabs.",
            ControlId::ViewPerFolder => "Each folder opens in its saved view; off keeps the window's view.",
            ControlId::PaletteRecent => "List the commands you ran from ⌘K under Recently used.",
            ControlId::TranslucentSidebar => {
                "The sidebar shows the blurred desktop behind it. Stays solid while macOS Reduce Transparency is on."
            }
            ControlId::DefaultView => "The view for folders without their own: Icons, List, Columns or Gallery.",
            ControlId::DefaultSort => "The [defaults] order; a rule or folder override still wins.",
            ControlId::DefaultDirection => {
                let ui = cx.global::<Ui>();
                let layer = ui.views.defaults_layer();
                let key = layer.sort.unwrap_or_default();
                let natural = if crow_core::natural_ascending(key) {
                    "A to Z / smallest first"
                } else {
                    "largest or newest first"
                };
                return Some(
                    match layer.ascending {
                        Some(true) => {
                            format!("Saved ascending. Natural for {} is {natural}.", crow_core::sort_key_name(key))
                        }
                        Some(false) => {
                            format!("Saved descending. Natural for {} is {natural}.", crow_core::sort_key_name(key))
                        }
                        None => format!("Natural for {}: {natural}.", crow_core::sort_key_name(key)),
                    }
                    .into(),
                );
            }
            ControlId::DefaultFoldersFirst => "Group folders above files before sorting.",
            ControlId::PreviewScope => "Edit the list for every folder, one folder, or a path rule.",
            ControlId::PreviewCurrentFolder => "Use the folder the browser showed when Settings opened.",
            ControlId::PreviewChooseFolder => "Pick any folder. Cancelling leaves the target and views.toml alone.",
            ControlId::PreviewRuleSelect => "Rules in views.toml order; later rules win for a folder.",
            ControlId::PreviewRulePattern => {
                "A path or glob such as ~/Pictures/**; the new rule goes after the existing ones."
            }
            ControlId::PreviewAddRule => "Append a preview-only rule and keep every other field of the file.",
            ControlId::PreviewList => "Configured providers in execution order; first match wins.",
            ControlId::PreviewMoveUp => "Move the selected provider earlier.",
            ControlId::PreviewMoveDown => "Move the selected provider later.",
            ControlId::PreviewRemove => "Remove the selected provider from this layer.",
            ControlId::PreviewAdd => "Add a registered preview provider that is not in the list yet.",
            ControlId::PreviewClear => "Remove this layer's preview list so it inherits again.",
            ControlId::Motion => {
                let settings = &cx.global::<Ui>().settings;
                return Some(match settings.motion {
                    MotionMode::Auto => {
                        let reduce = crow_macos::reduce_motion();
                        let effective = settings.motion.resolve(reduce);
                        format!(
                            "Auto now: {}; Reduce Motion is {}.",
                            match effective {
                                crow_core::motion::Effective::Off => "Off",
                                crow_core::motion::Effective::Low => "Low",
                                crow_core::motion::Effective::Full => "Full",
                            },
                            if reduce { "on" } else { "off" }
                        )
                        .into()
                    }
                    _ => "Off changes in one frame; Low fades; Full also moves.".into(),
                });
            }
            ControlId::Sound => "Crow adds soft ticks to Finder's sounds; Off is silent.",
            ControlId::PlaySound => "Play the trash sound in the chosen set.",
            ControlId::WindowWidth | ControlId::WindowHeight => "600 to 4000 points.",
            ControlId::Plugins | ControlId::KeyboardSearch | ControlId::KeyboardList => return None,
            ControlId::KeyboardResetAll => "Remove every shortcut change in the chosen layer.",
            ControlId::EditSettings => "Open settings.toml in its default app.",
            ControlId::ShowConfig => "Open ~/.config/crow in Finder.",
            ControlId::OpenSidebarToml
            | ControlId::OpenViewsToml
            | ControlId::OpenExcludeToml
            | ControlId::OpenPluginsToml
            | ControlId::OpenSavedQueriesToml
            | ControlId::OpenKeysToml => return None,
        };
        Some(text.into())
    }

    /// Only the affected backing file's mutations are disabled. Links, the
    /// plugin inventory and Play stay usable.
    fn enabled(self, cx: &App) -> bool {
        let ui = cx.global::<Ui>();
        match self {
            ControlId::EditSettings
            | ControlId::ShowConfig
            | ControlId::Plugins
            | ControlId::PlaySound
            | ControlId::OpenSidebarToml
            | ControlId::OpenViewsToml
            | ControlId::OpenExcludeToml
            | ControlId::OpenPluginsToml
            | ControlId::OpenSavedQueriesToml
            | ControlId::OpenKeysToml => true,
            ControlId::SavedQueryList => !ui.saved_queries.is_empty() && ui.file_error("saved-queries.toml").is_none(),
            ControlId::KeyboardSearch | ControlId::KeyboardList => true,
            ControlId::KeyboardResetAll => ui.file_error("keys.toml").is_none(),
            ControlId::NewSavedQuery => ui.file_error("saved-queries.toml").is_none(),
            ControlId::DefaultView
            | ControlId::DefaultSort
            | ControlId::DefaultDirection
            | ControlId::DefaultFoldersFirst
            | ControlId::PreviewScope
            | ControlId::PreviewCurrentFolder
            | ControlId::PreviewChooseFolder
            | ControlId::PreviewRuleSelect
            | ControlId::PreviewRulePattern
            | ControlId::PreviewAddRule
            | ControlId::PreviewList
            | ControlId::PreviewMoveUp
            | ControlId::PreviewMoveDown
            | ControlId::PreviewRemove
            | ControlId::PreviewAdd
            | ControlId::PreviewClear => ui.views_error().is_none(),
            _ => ui.settings_error().is_none(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PluginAction {
    Approve,
    Enable,
    Disable,
    Reload,
    Retry,
}

impl PluginAction {
    fn label(self) -> &'static str {
        match self {
            Self::Approve => "Approve",
            Self::Enable => "Enable",
            Self::Disable => "Disable",
            Self::Reload => "Reload",
            Self::Retry => "Retry",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Approve => 0,
            Self::Enable => 1,
            Self::Disable => 2,
            Self::Reload => 3,
            Self::Retry => 4,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Approve => "A",
            Self::Enable => "E",
            Self::Disable => "D",
            Self::Reload => "R",
            Self::Retry => "T",
        }
    }

    fn enabled(self, entry: &PluginEntry) -> bool {
        match self {
            Self::Approve => entry.can_approve() && entry.digest.is_some(),
            Self::Enable => entry.can_enable(),
            Self::Disable => entry.can_disable(),
            Self::Reload => entry.can_reload(),
            Self::Retry => entry.can_retry(),
        }
    }

    fn allowed(self, entry: &PluginEntry, grants_error: bool) -> bool {
        plugin_actions(entry, grants_error).contains(&self) && self.enabled(entry)
    }
}

/// One action policy for buttons, Return and action keys. During a grants
/// error only Reload can recover; approve/enable/disable/retry are blocked.
fn plugin_actions(entry: &PluginEntry, grants_error: bool) -> Vec<PluginAction> {
    let mut actions = match entry.state {
        PluginState::PendingApproval => vec![PluginAction::Approve],
        PluginState::Disabled => vec![PluginAction::Enable],
        PluginState::Paused | PluginState::Failed => vec![PluginAction::Retry, PluginAction::Disable],
        PluginState::Running | PluginState::Starting => vec![PluginAction::Disable],
        _ => Vec::new(),
    };
    if entry.can_reload() {
        actions.push(PluginAction::Reload);
    }
    if grants_error {
        actions.retain(|action| *action == PluginAction::Reload);
    }
    actions
}

fn primary_plugin_action(entry: &PluginEntry) -> PluginAction {
    match entry.state {
        PluginState::PendingApproval => PluginAction::Approve,
        PluginState::Disabled => PluginAction::Enable,
        PluginState::Paused | PluginState::Failed => PluginAction::Retry,
        _ => PluginAction::Reload,
    }
}

/// The rendered row an approval may act on. A keyboard approval must match the
/// latest inventory on all three fields before dispatch.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PluginTarget {
    bundle: OsString,
    id: crow_core::PluginId,
    digest: String,
}

impl PluginTarget {
    fn for_entry(entry: &PluginEntry) -> Option<Self> {
        Some(Self { bundle: entry.bundle.clone(), id: entry.id.clone()?, digest: entry.digest.clone()? })
    }
}

#[derive(Default)]
struct TypeAhead {
    buffer: String,
    at: Option<Instant>,
}

/// One stop in the window's own Tab order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusTarget {
    Search,
    ClearSearch,
    Sidebar,
    Control(ControlId),
    /// The Open button for one broken config file.
    Error(&'static str),
}

/// The files the error region covers. `plugins.toml` comes from the plugin
/// inventory and `pack.toml` from the icon pack loader; the rest from the config loader.
const ERROR_FILES: [&str; 8] = [
    "settings.toml",
    "views.toml",
    "sidebar.toml",
    "exclude.toml",
    "plugins.toml",
    "saved-queries.toml",
    "keys.toml",
    "pack.toml",
];

/// The preview editor's layer. The target within the layer is kept while the
/// editor is on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreviewScope {
    Defaults,
    Folder,
    Rule,
}

/// A category or search-result change fades the content in, with a 4 px rise in
/// Full motion. The sidebar and the search field stay put.
#[derive(Clone, Copy)]
struct ContentMotion {
    started: Instant,
    mode: Effective,
}

impl ContentMotion {
    fn progress(self, mode: Effective, now: Instant) -> Option<f32> {
        if self.mode != mode {
            return None;
        }
        motion::navigation_progress(mode, self.started, now, motion::ms::PANEL_TAB)
    }
}

pub struct SettingsWindow {
    focus: FocusHandle,
    /// The sidebar's own keyboard group.
    sidebar: FocusHandle,
    search: Entity<TextInput>,
    query: String,
    clear_search: FocusHandle,
    category: Category,
    /// The row ↑ ↓ moves through while the sidebar has focus.
    sidebar_cursor: Category,
    handles: HashMap<ControlId, FocusHandle>,
    font_size: Entity<TextInput>,
    terminal: Entity<TextInput>,
    window_width: Entity<TextInput>,
    window_height: Entity<TextInput>,
    theme: ThemeChoice,
    theme_scroll: ScrollHandle,
    /// Frames left in which an opening menu re-asks its scroll handle to show the cursor: the
    /// first frame has no viewport bounds yet, so the first request lands nowhere.
    menu_settle: std::cell::Cell<u8>,
    fonts: Vec<String>,
    font_cursor: usize,
    /// Stable row identity; never an inventory index.
    selected_plugin: Option<OsString>,
    /// Last rendered position, used only for Up/Down and disappearance fallback.
    plugin_position: usize,
    /// The approval target captured when the selected row was rendered.
    plugin_target: Option<PluginTarget>,
    /// The bounded plugin rows; Up/Down scroll the selected row into view.
    plugin_scroll: ScrollHandle,
    font_scroll: ScrollHandle,
    /// Icon pack choices for the current render and the id under the menu cursor.
    icon_pack_ids: Vec<String>,
    icon_pack_labels: Vec<String>,
    /// What the closed icon-pack button shows: every chosen pack, in layer order.
    icon_pack_value: String,
    icon_pack_cursor: Option<String>,
    icon_pack_scroll: ScrollHandle,
    /// Default-sort menu cursor (index into the fixed sort-key list).
    sort_cursor: usize,
    sort_scroll: ScrollHandle,
    preview_scope: PreviewScope,
    preview_folder: Option<PathBuf>,
    preview_rule: Option<crow_core::RuleLocator>,
    preview_rule_new: bool,
    preview_pattern: Entity<TextInput>,
    keyboard: keyboard::KeyboardState,
    preview_pattern_text: String,
    preview_new_providers: Vec<String>,
    preview_list_cursor: usize,
    preview_list_scroll: ScrollHandle,
    preview_rule_cursor: usize,
    preview_rule_labels: Vec<String>,
    preview_rule_options_cache: Vec<Option<crow_core::RuleLocator>>,
    preview_add_cursor: usize,
    preview_add_labels: Vec<String>,
    preview_add_ids: Vec<String>,
    preview_add_scroll: ScrollHandle,
    preview_rule_scroll: ScrollHandle,
    preview_write_pending: bool,
    preview_rule_type_ahead: TypeAhead,
    preview_add_type_ahead: TypeAhead,
    preview_message: Option<SharedString>,
    preview_picker_pending: bool,
    saved_query_selection: Option<crow_core::SavedQueryId>,
    saved_query_scroll: ScrollHandle,
    saved_query_form: Option<Box<crate::window::saved_query_form::SavedQueryForm>>,
    content_motion: Option<ContentMotion>,
    /// The motion mode of the last draw, for the shared controls.
    motion_mode: crow_core::motion::Effective,
    /// The drop-down whose menu is open.
    menu: Option<ControlId>,
    /// The menu a mouse-down outside it just closed, so the click that lands on
    /// its own button doesn't open it again. Cleared on every mouse-down.
    dismissed_by_mouse: Option<ControlId>,
    theme_type_ahead: TypeAhead,
    font_type_ahead: TypeAhead,
    page_scroll: ScrollHandle,
    /// Each control's direct-child index in the page, for keyboard scrolling.
    control_items: HashMap<ControlId, usize>,
    /// One Open button per broken config file, and its row in the error region.
    error_handles: HashMap<&'static str, FocusHandle>,
    error_scroll: ScrollHandle,
    error_items: HashMap<&'static str, usize>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = cx.global::<Ui>().settings.clone();
        let font_size = cx.new(|cx| {
            TextInput::new(number_text(f64::from(settings.font_size)), "", cx)
                .with_id("settings-font-size")
                .with_role(Role::SpinButton)
                .with_aria_label("Font size")
                .with_numeric_range(f64::from(settings.font_size), 10.0, 24.0, 1.0)
        });
        let terminal = cx.new(|cx| {
            TextInput::new(settings.terminal.clone(), "", cx)
                .with_id("settings-terminal")
                .with_aria_label("Terminal app")
        });
        let window_width = cx.new(|cx| {
            TextInput::new(settings.window.width.to_string(), "", cx)
                .with_id("settings-window-width")
                .with_role(Role::SpinButton)
                .with_aria_label("Width")
                .with_numeric_range(f64::from(settings.window.width), 600.0, 4000.0, 1.0)
        });
        let window_height = cx.new(|cx| {
            TextInput::new(settings.window.height.to_string(), "", cx)
                .with_id("settings-window-height")
                .with_role(Role::SpinButton)
                .with_aria_label("Height")
                .with_numeric_range(f64::from(settings.window.height), 600.0, 4000.0, 1.0)
        });
        let search = cx.new(|cx| {
            TextInput::new("", "Search Settings", cx)
                .bare()
                .with_id("settings-search")
                .with_role(Role::SearchInput)
                .with_aria_label("Search Settings")
        });
        let preview_pattern = cx.new(|cx| {
            TextInput::new("", "~/Pictures/**", cx)
                .with_id("settings-preview-rule-pattern")
                .with_role(Role::TextInput)
                .with_aria_label("New rule pattern")
        });

        let keyboard = keyboard::KeyboardState::new(cx);
        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe_in(&keyboard.search, window, |this, _, event, window, cx| {
            this.on_keyboard_search_event(event, window, cx);
        }));
        for (id, input) in [
            (ControlId::FontSize, &font_size),
            (ControlId::WindowWidth, &window_width),
            (ControlId::WindowHeight, &window_height),
        ] {
            subscriptions.push(cx.subscribe_in(input, window, move |this, _, event, window, cx| {
                this.on_stepper_event(id, event, window, cx);
            }));
        }
        subscriptions.push(cx.subscribe_in(&terminal, window, |this, _, event, window, cx| {
            this.on_terminal_event(event, window, cx);
        }));
        subscriptions.push(cx.subscribe_in(&search, window, |this, _, event, window, cx| {
            this.on_search_event(event, window, cx);
        }));
        subscriptions.push(cx.subscribe_in(&preview_pattern, window, |this, _, event, window, cx| {
            this.on_preview_pattern_event(event, window, cx);
        }));

        let mut handles = HashMap::new();
        for (index, id) in ControlId::ALL.iter().copied().enumerate() {
            let handle = match id {
                ControlId::FontSize => font_size.read(cx).focus_handle(cx),
                ControlId::Terminal => terminal.read(cx).focus_handle(cx),
                ControlId::WindowWidth => window_width.read(cx).focus_handle(cx),
                ControlId::WindowHeight => window_height.read(cx).focus_handle(cx),
                ControlId::PreviewRulePattern => preview_pattern.read(cx).focus_handle(cx),
                ControlId::KeyboardSearch => keyboard.search.read(cx).focus_handle(cx),
                _ => cx.focus_handle(),
            };
            // The window owns the order (category and search change it). These
            // indexes only keep native focus traversal sane while the view loads.
            handles.insert(id, handle.tab_stop(false).tab_index(index as isize));
        }
        let sidebar = cx.focus_handle().tab_stop(true).tab_index(-1);
        let clear_search = cx.focus_handle().tab_stop(false);
        let mut error_handles = HashMap::new();
        for file in ERROR_FILES {
            error_handles.insert(file, cx.focus_handle().tab_stop(false));
        }
        let category =
            cx.global::<Ui>().settings_category.as_deref().and_then(Category::from_id).unwrap_or_else(Category::first);

        let entity = cx.entity();
        for id in [
            ControlId::Theme,
            ControlId::FontFamily,
            ControlId::IconPack,
            ControlId::DefaultSort,
            ControlId::FontSize,
            ControlId::Terminal,
            ControlId::WindowWidth,
            ControlId::WindowHeight,
        ] {
            let handle = handles[&id].clone();
            let entity = entity.clone();
            subscriptions.push(window.on_focus_out(&handle, cx, move |_, window, cx| {
                entity.update(cx, |this, cx| this.on_focus_lost(id, window, cx));
            }));
        }

        // The title-bar close button skips `close`; drop a pending preview there too.
        let weak = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            weak.update(cx, |this, cx| this.close_menu(window, cx)).ok();
            true
        });

        let mut fonts = vec!["System".to_owned()];
        if !settings.font_family.is_empty() && !fonts.iter().any(|name| name == &settings.font_family) {
            fonts.push(settings.font_family.clone());
        }
        let font_cursor = font_index(&fonts, &settings.font_family);
        // Enumerating the installed families is AppKit work that took ~100 ms on
        // the first Settings open (findings #20). CoreText answers off the main
        // thread; the list fills in a frame later. Tests inject their own list.
        #[cfg(not(test))]
        {
            let task = cx.background_spawn(async move { crow_macos::available_font_families() });
            cx.spawn_in(window, async move |this, cx| {
                let families = task.await;
                this.update_in(cx, |this, _, cx| {
                    let family = cx.global::<Ui>().settings.font_family.clone();
                    let mut fonts = families;
                    if !fonts.iter().any(|name| name == "System") {
                        fonts.insert(0, "System".to_owned());
                    }
                    if !family.is_empty() && !fonts.iter().any(|name| name == &family) {
                        fonts.push(family.clone());
                    }
                    this.font_cursor = font_index(&fonts, &family);
                    this.fonts = fonts;
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        let theme = ThemeChoice::new(Vec::new(), &settings.theme);
        let task = theme_choice::load(cx);
        cx.spawn_in(window, async move |this, cx| {
            let infos = task.await;
            this.update_in(cx, |this, _, cx| {
                let current = cx.global::<Ui>().settings.theme.clone();
                this.theme = ThemeChoice::new(infos, &current);
                cx.notify();
            })
            .ok();
        })
        .detach();

        Self {
            focus: cx.focus_handle(),
            sidebar,
            search,
            query: String::new(),
            clear_search,
            category,
            sidebar_cursor: category,
            handles,
            font_size,
            terminal,
            window_width,
            window_height,
            theme,
            theme_scroll: ScrollHandle::new(),
            menu_settle: std::cell::Cell::new(0),
            fonts,
            font_cursor,
            selected_plugin: None,
            plugin_position: 0,
            plugin_target: None,
            plugin_scroll: ScrollHandle::new(),
            font_scroll: ScrollHandle::new(),
            icon_pack_ids: Vec::new(),
            icon_pack_labels: Vec::new(),
            icon_pack_value: String::new(),
            icon_pack_cursor: None,
            icon_pack_scroll: ScrollHandle::new(),
            sort_cursor: 0,
            sort_scroll: ScrollHandle::new(),
            preview_scope: PreviewScope::Defaults,
            preview_folder: None,
            preview_rule: None,
            preview_rule_new: false,
            preview_pattern,
            keyboard,
            preview_pattern_text: String::new(),
            preview_new_providers: Vec::new(),
            preview_list_cursor: 0,
            preview_list_scroll: ScrollHandle::new(),
            preview_rule_cursor: 0,
            preview_rule_labels: Vec::new(),
            preview_rule_options_cache: Vec::new(),
            preview_add_cursor: 0,
            preview_add_labels: Vec::new(),
            preview_add_ids: Vec::new(),
            preview_add_scroll: ScrollHandle::new(),
            preview_rule_scroll: ScrollHandle::new(),
            preview_write_pending: false,
            preview_rule_type_ahead: TypeAhead::default(),
            preview_add_type_ahead: TypeAhead::default(),
            preview_message: None,
            preview_picker_pending: false,
            saved_query_selection: None,
            saved_query_scroll: ScrollHandle::new(),
            saved_query_form: None,
            content_motion: None,
            motion_mode: crow_core::motion::Effective::Off,
            menu: None,
            dismissed_by_mouse: None,
            theme_type_ahead: TypeAhead::default(),
            font_type_ahead: TypeAhead::default(),
            page_scroll: ScrollHandle::new(),
            control_items: HashMap::new(),
            error_handles,
            error_scroll: ScrollHandle::new(),
            error_items: HashMap::new(),
            _subscriptions: subscriptions,
        }
    }

    fn handle(&self, id: ControlId) -> FocusHandle {
        self.handles[&id].clone()
    }

    fn style<'a>(&'a self, id: ControlId, window: &Window, theme: &'a Theme) -> controls::ControlStyle<'a> {
        controls::ControlStyle {
            focus: self.handles.get(&id).expect("control handle"),
            focused: self.is_focused(id, window),
            theme,
            mode: self.motion_mode,
        }
    }

    fn is_focused(&self, id: ControlId, window: &Window) -> bool {
        self.handles.get(&id).is_some_and(|handle| handle.is_focused(window))
    }

    fn focused_control(&self, window: &Window) -> Option<ControlId> {
        ControlId::ALL.iter().copied().find(|id| self.is_focused(*id, window))
    }

    fn sidebar_focused(&self, window: &Window) -> bool {
        self.sidebar.is_focused(window)
    }

    /// The enabled controls of the page that is on screen, in reading order:
    /// the category's units, or the units matching the active query.
    fn visible_controls(&self, cx: &App) -> Vec<ControlId> {
        let visible = |id: &ControlId| self.preview_control_enabled(*id, cx);
        if self.query.is_empty() {
            return catalog::controls_in(self.category).into_iter().filter(|id| visible(id)).collect();
        }
        search::matches(&self.query)
            .into_iter()
            .flat_map(|hit| catalog::unit_for(hit.unit).into_iter().flat_map(|unit| unit.controls.iter().copied()))
            .filter(|id| visible(id))
            .collect()
    }

    fn search_handle(&self, cx: &App) -> FocusHandle {
        self.search.read(cx).focus_handle(cx)
    }

    /// Every error the window can recover from, in file order.
    fn issues(&self, cx: &App) -> Vec<ui::ConfigIssue> {
        ui::config_issues(cx)
    }

    fn focus_targets(&self, cx: &App) -> Vec<FocusTarget> {
        let body = self.visible_controls(cx);
        let mut targets = vec![FocusTarget::Search];
        if self.query.is_empty() {
            targets.push(FocusTarget::Sidebar);
            targets.extend(body.into_iter().map(FocusTarget::Control));
        } else {
            if body.is_empty() {
                targets.push(FocusTarget::ClearSearch);
            }
            targets.extend(body.into_iter().map(FocusTarget::Control));
            targets.push(FocusTarget::Sidebar);
        }
        targets.extend(self.issues(cx).into_iter().map(|issue| FocusTarget::Error(issue.file)));
        targets
    }

    fn target_handle(&self, target: FocusTarget, cx: &App) -> FocusHandle {
        match target {
            FocusTarget::Search => self.search_handle(cx),
            FocusTarget::ClearSearch => self.clear_search.clone(),
            FocusTarget::Sidebar => self.sidebar.clone(),
            FocusTarget::Control(id) => self.handle(id),
            FocusTarget::Error(file) => self.error_handles[file].clone(),
        }
    }

    /// Tab/Shift-Tab in the window's own order: sidebar, then the visible
    /// category's enabled controls. Wraps at the ends.
    fn advance_focus(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_focused(window, cx);
        let targets = self.focus_targets(cx);
        if targets.is_empty() {
            return;
        }
        let current = targets.iter().position(|target| self.target_handle(*target, cx).is_focused(window));
        let next = match current {
            Some(index) => (index as isize + delta).rem_euclid(targets.len() as isize) as usize,
            None if delta > 0 => 0,
            None => targets.len() - 1,
        };
        let handle = self.target_handle(targets[next], cx);
        window.focus(&handle, cx);
        self.scroll_to_focused(window);
        cx.notify();
    }

    fn move_sidebar(&mut self, delta: isize, cx: &mut Context<Self>) {
        let index = Category::ALL.iter().position(|category| *category == self.sidebar_cursor).unwrap_or(0);
        let next = (index as isize + delta).clamp(0, Category::ALL.len() as isize - 1) as usize;
        if next != index {
            self.sidebar_cursor = Category::ALL[next];
            cx.notify();
        }
    }

    fn start_content_motion(&mut self, cx: &App) {
        let mode = cx.global::<Ui>().feedback.mode;
        self.content_motion = (mode != Effective::Off).then(|| ContentMotion { started: Instant::now(), mode });
    }

    /// Open a category from the sidebar (or a click). Search ends, transient
    /// theme previews revert and focus stays on the sidebar.
    fn select_category(&mut self, category: Category, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_focused(window, cx);
        self.clear_query(false, window, cx);
        if self.category != category {
            self.start_content_motion(cx);
        }
        self.category = category;
        self.sidebar_cursor = category;
        ui::save_settings_category(category.id(), cx);
        self.page_scroll.set_offset(gpui::point(px(0.), px(0.)));
        window.focus(&self.sidebar, cx);
        cx.notify();
    }

    /// The fuzzer's selectors for this window: the search field, the sidebar,
    /// the painted controls of the active category or results, recovery buttons
    /// and an open drop-down's list.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn debug_selectors(&self, cx: &App) -> Vec<String> {
        let mut out = vec!["settings-search-row".to_owned(), "settings-sidebar".to_owned()];
        if !self.query.is_empty() {
            out.push("settings-clear-search".to_owned());
        }
        out.extend(Category::ALL.into_iter().map(|category| format!("settings-category-{}", category.id())));
        out.extend(self.visible_controls(cx).into_iter().map(ControlId::selector));
        for issue in self.issues(cx) {
            out.push(issue.file.to_owned());
        }
        if let Some(menu) = self.menu {
            out.push(format!("{}-menu", menu.key()));
        }
        out
    }

    /// Whether a text field has focus, so the fuzzer types instead of pressing
    /// keys the field would swallow.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn text_focused(&self, window: &Window, cx: &App) -> bool {
        matches!(
            self.focused_control(window),
            Some(
                ControlId::FontSize | ControlId::WindowWidth | ControlId::WindowHeight | ControlId::PreviewRulePattern
            )
        ) || self.search_handle(cx).is_focused(window)
    }

    /// Whether Esc clears the Settings query instead of closing the window.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn search_active(&self) -> bool {
        !self.query.is_empty()
    }

    #[cfg(test)]
    pub fn category_id_for_tests(&self) -> &'static str {
        self.category.id()
    }

    #[cfg(test)]
    pub fn query_for_tests(&self) -> &str {
        &self.query
    }

    #[cfg(test)]
    pub fn search_focused_for_tests(&self, window: &Window, cx: &App) -> bool {
        self.search_handle(cx).is_focused(window)
    }

    #[cfg(test)]
    pub fn content_motion_for_tests(&self, mode: Effective, now: Instant) -> Option<f32> {
        self.content_motion.and_then(|motion| motion.progress(mode, now))
    }

    /// Whether a drop-down is open, so Escape closes it instead of the window.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn menu_open(&self) -> bool {
        self.menu.is_some()
    }

    /// The fuzzer's invariants for this window: focus stays on a control (or in the saved-query editor), and
    /// every painted interactive element has a role and a label. The runner
    /// compares the shown values to `settings.toml` after a settle.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn check_invariants(&self, window: &Window, cx: &App) -> Vec<crate::fuzz::Violation> {
        let mut out = Vec::new();
        let on_error = ERROR_FILES.iter().any(|file| self.error_handles[file].is_focused(window));
        if self.focused_control(window).is_none()
            && !self.search_handle(cx).is_focused(window)
            && !self.sidebar_focused(window)
            && !self.clear_search.is_focused(window)
            && !on_error
            // The saved-query editor is a sheet over the page; its fields are controls too.
            && !self.saved_query_form.as_ref().is_some_and(|form| form.focus.contains_focused(window, cx))
        {
            out.push(crate::fuzz::Violation::new("settings focus", "focus is not on a control"));
        }
        out.extend(crate::fuzz::a11y_violations(window, "settings a11y"));
        out
    }

    fn meta(&self, id: ControlId, cx: &App) -> ControlMeta {
        ControlMeta { id: id.key(), label: id.label(), description: id.description(cx), enabled: id.enabled(cx) }
    }

    fn first_focus(&self, cx: &App) -> FocusHandle {
        self.search_handle(cx)
    }

    /// ⌘, on an open window brings it forward without stealing valid focus.
    fn focus_first(&self, window: &mut Window, cx: &mut App) {
        if self.saved_query_form.is_some()
            || self.sidebar_focused(window)
            || self.search_handle(cx).is_focused(window)
            || self.clear_search.is_focused(window)
            || self.focused_control(window).is_some()
            || ERROR_FILES.iter().any(|file| self.error_handles[file].is_focused(window))
        {
            self.scroll_to_focused(window);
            return;
        }
        let focus = self.first_focus(cx);
        window.focus(&focus, cx);
        self.scroll_to_focused(window);
    }

    fn scroll_to_focused(&self, window: &Window) {
        for file in ERROR_FILES {
            if self.error_handles[file].is_focused(window)
                && let Some(index) = self.error_items.get(file)
            {
                self.error_scroll.scroll_to_item(*index);
            }
        }
        if let Some(id) = self.focused_control(window)
            && let Some(index) = self.control_items.get(&id)
        {
            self.page_scroll.scroll_to_item(*index);
        }
    }

    fn input(&self, id: ControlId) -> Entity<TextInput> {
        match id {
            ControlId::FontSize => self.font_size.clone(),
            ControlId::WindowWidth => self.window_width.clone(),
            ControlId::WindowHeight => self.window_height.clone(),
            _ => unreachable!("not a stepper"),
        }
    }

    fn setting_value(&self, id: ControlId, cx: &App) -> f64 {
        let settings = &cx.global::<Ui>().settings;
        match id {
            ControlId::FontSize => f64::from(settings.font_size),
            ControlId::WindowWidth => f64::from(settings.window.width),
            ControlId::WindowHeight => f64::from(settings.window.height),
            _ => 0.0,
        }
    }

    fn stepper_value(&self, id: ControlId, cx: &App) -> f64 {
        self.input(id)
            .read(cx)
            .text()
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .unwrap_or_else(|| self.setting_value(id, cx))
    }

    /// Put the setting's value into the field. The render-time sync skips a
    /// focused field so external edits never overwrite typing; an arrow or a
    /// commit calls this unconditionally.
    fn write_stepper_text(&self, id: ControlId, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.setting_value(id, cx);
        let text = number_text(value);
        let input = self.input(id);
        if input.read(cx).text() != text {
            input.update(cx, |input, cx| input.set_text(text, window, cx));
        }
        input.update(cx, |input, cx| input.set_numeric_value(value, cx));
    }

    fn sync_stepper(&self, id: ControlId, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_focused(id, window) {
            return;
        }
        self.write_stepper_text(id, window, cx);
    }

    fn set_stepper(&mut self, id: ControlId, value: f64, window: &mut Window, cx: &mut Context<Self>) {
        let (min, max) = stepper_range(id);
        let value = value.clamp(min, max).round();
        match id {
            ControlId::FontSize => ui::set_setting(SettingChange::FontSize(value as f32), cx),
            ControlId::WindowWidth => ui::set_setting(SettingChange::WindowWidth(value as u32), cx),
            ControlId::WindowHeight => ui::set_setting(SettingChange::WindowHeight(value as u32), cx),
            _ => return,
        }
        self.write_stepper_text(id, window, cx);
        cx.notify();
    }

    fn commit_stepper(&mut self, id: ControlId, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.stepper_value(id, cx);
        self.set_stepper(id, value, window, cx);
    }

    fn adjust_stepper(&mut self, id: ControlId, delta: f64, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.stepper_value(id, cx) + delta;
        self.set_stepper(id, value, window, cx);
    }

    fn on_stepper_event(&mut self, id: ControlId, event: &TextInputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            TextInputEvent::Confirmed => self.commit_stepper(id, window, cx),
            // Escape peels one layer, as everywhere else in this window.
            TextInputEvent::Cancelled => self.cancel(window, cx),
            TextInputEvent::CompletionRequested => {
                self.advance_focus(1, window, cx);
            }
            TextInputEvent::Changed | TextInputEvent::BackspaceAtStart => {}
        }
    }

    fn on_focus_lost(&mut self, id: ControlId, window: &mut Window, cx: &mut Context<Self>) {
        match id {
            ControlId::Theme | ControlId::FontFamily | ControlId::IconPack | ControlId::DefaultSort => {
                if self.menu == Some(id) {
                    self.close_menu(window, cx);
                }
            }
            ControlId::FontSize | ControlId::WindowWidth | ControlId::WindowHeight => {
                self.commit_stepper(id, window, cx);
            }
            ControlId::Terminal => self.commit_terminal(window, cx),
            _ => {}
        }
    }

    /// Put the saved terminal in the field. The render-time sync skips a focused
    /// field so an external settings edit never overwrites typing.
    fn sync_terminal(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_focused(ControlId::Terminal, window) {
            return;
        }
        let text = cx.global::<Ui>().settings.terminal.clone();
        if self.terminal.read(cx).text() != text {
            self.terminal.update(cx, |input, cx| input.set_text(text, window, cx));
        }
    }

    fn commit_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.terminal.read(cx).text().trim().to_owned();
        // An empty app would make `open -a` fail; the setting's default stands in.
        let text = if text.is_empty() { "Terminal".to_owned() } else { text };
        ui::set_setting(SettingChange::Terminal(text), cx);
        self.sync_terminal(window, cx);
        cx.notify();
    }

    fn on_terminal_event(&mut self, event: &TextInputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            TextInputEvent::Confirmed => self.commit_terminal(window, cx),
            TextInputEvent::Cancelled => self.cancel(window, cx),
            TextInputEvent::CompletionRequested => {
                self.commit_terminal(window, cx);
                self.advance_focus(1, window, cx);
            }
            TextInputEvent::Changed | TextInputEvent::BackspaceAtStart => {}
        }
    }

    fn on_search_event(&mut self, event: &TextInputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            TextInputEvent::Changed => {
                let text = self.search.read(cx).text().trim().to_owned();
                if text != self.query {
                    let was_searching = !self.query.is_empty();
                    self.close_menu(window, cx);
                    self.query = text;
                    if was_searching != !self.query.is_empty() {
                        self.start_content_motion(cx);
                    }
                    cx.notify();
                }
            }
            TextInputEvent::Confirmed => {
                if let Some(id) = self.visible_controls(cx).first().copied() {
                    window.focus(&self.handle(id), cx);
                    self.scroll_to_focused(window);
                    cx.notify();
                }
            }
            TextInputEvent::Cancelled => self.cancel(window, cx),
            TextInputEvent::CompletionRequested => self.advance_focus(1, window, cx),
            TextInputEvent::BackspaceAtStart => {}
        }
    }

    /// Empty the Settings search and leave the category page of the active
    /// sidebar row in place. `focus_search` also returns focus to the field.
    fn clear_query(&mut self, focus_search: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.query.is_empty() && self.search.read(cx).text().is_empty() {
            return;
        }
        if !self.query.is_empty() {
            self.start_content_motion(cx);
        }
        self.query.clear();
        self.search.update(cx, |input, cx| input.set_text("", window, cx));
        if focus_search {
            let handle = self.search_handle(cx);
            window.focus(&handle, cx);
        }
        cx.notify();
    }

    fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saved_query_form.is_some() {
            return;
        }
        if self.category == Category::Keyboard && self.query.is_empty() {
            self.focus_keyboard_search(window, cx);
            return;
        }
        let input = self.search.clone();
        let len = input.read(cx).text().len();
        input.update(cx, |input, cx| input.select_range(0..len, window, cx));
        let handle = input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sidebar_focused(window) {
            self.select_category(self.sidebar_cursor, window, cx);
            return;
        }
        if self.clear_search.is_focused(window) {
            self.clear_query(true, window, cx);
            return;
        }
        let Some(id) = self.focused_control(window) else { return };
        if !id.enabled(cx) {
            return;
        }
        match id {
            ControlId::Theme
            | ControlId::FontFamily
            | ControlId::IconPack
            | ControlId::DefaultSort
            | ControlId::PreviewRuleSelect
            | ControlId::PreviewAdd => {
                if self.menu == Some(id) {
                    self.keep_menu(window, cx);
                } else {
                    self.open_menu(id, cx);
                }
            }
            ControlId::PreviewScope => {
                self.set_preview_scope(self.preview_scope_index(), window, cx);
            }
            ControlId::PreviewCurrentFolder => self.preview_use_current_folder(cx),
            ControlId::PreviewChooseFolder => self.choose_preview_folder(window, cx),
            ControlId::PreviewAddRule => self.preview_add_rule(window, cx),
            ControlId::PreviewMoveUp => self.preview_move_provider(-1, cx),
            ControlId::PreviewMoveDown => self.preview_move_provider(1, cx),
            ControlId::PreviewRemove => self.preview_remove_provider(cx),
            ControlId::PreviewClear => self.preview_clear(cx),
            ControlId::PreviewList | ControlId::PreviewRulePattern => {}
            ControlId::KeyboardList => self.keyboard_activate(window, cx),
            ControlId::KeyboardResetAll => self.ask_reset_all(cx),
            ControlId::KeyboardSearch => {}
            ControlId::DefaultView => {
                let index = view_index(cx.global::<Ui>().views.defaults_layer().view.unwrap_or_default());
                self.select_default_view(index, cx);
            }
            ControlId::DefaultDirection => {
                let index = direction_index(cx.global::<Ui>().views.defaults_layer().ascending);
                self.select_direction(index, cx);
            }
            ControlId::DefaultFoldersFirst => self.toggle_default_folders_first(cx),
            ControlId::ShowHidden
            | ControlId::SearchHidden
            | ControlId::ShowPreview
            | ControlId::ViewPerFolder
            | ControlId::PaletteRecent
            | ControlId::TranslucentSidebar => self.toggle_bool(id, cx),
            ControlId::PlaySound => crate::feedback::play(Event::Trash, cx),
            ControlId::EditSettings => ui::edit_settings_file(cx),
            ControlId::Plugins => self.activate_plugin(cx),
            ControlId::NewSavedQuery => self.open_saved_query_editor(None, window, cx),
            ControlId::SavedQueryList => {
                if let Some(id) = self.selected_saved_query(cx).cloned() {
                    self.open_saved_query_editor(Some(id), window, cx);
                }
            }
            ControlId::ShowConfig => ui::open_config_folder(cx),
            ControlId::OpenSidebarToml => ui::open_config_file("sidebar.toml", cx),
            ControlId::OpenViewsToml => ui::open_config_file("views.toml", cx),
            ControlId::OpenExcludeToml => ui::open_config_file("exclude.toml", cx),
            ControlId::OpenPluginsToml => ui::open_config_file("plugins.toml", cx),
            ControlId::OpenSavedQueriesToml => ui::open_config_file("saved-queries.toml", cx),
            ControlId::OpenKeysToml => ui::open_config_file("keys.toml", cx),
            ControlId::FontSize | ControlId::WindowWidth | ControlId::WindowHeight => {
                self.commit_stepper(id, window, cx);
            }
            ControlId::Keymap
            | ControlId::Enter
            | ControlId::Hints
            | ControlId::Tips
            | ControlId::FolderSizes
            | ControlId::PathBar
            | ControlId::IconStyle
            | ControlId::SidebarStyle
            | ControlId::Motion
            | ControlId::Sound
            | ControlId::SearchMatch
            | ControlId::Terminal => {}
        }
    }

    fn move_horizontal(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.focused_control(window) else { return };
        match id {
            ControlId::KeyboardList => self.keyboard_horizontal(delta, cx),
            ControlId::Keymap
            | ControlId::Enter
            | ControlId::Hints
            | ControlId::Tips
            | ControlId::FolderSizes
            | ControlId::PathBar
            | ControlId::IconStyle
            | ControlId::SidebarStyle
            | ControlId::Motion
            | ControlId::SearchMatch
            | ControlId::Sound => {
                self.move_segment(id, delta, window, cx);
            }
            ControlId::DefaultView => {
                let current = view_index(cx.global::<Ui>().views.defaults_layer().view.unwrap_or_default());
                let next = (current as isize + delta).clamp(0, 3) as usize;
                if next != current {
                    self.select_default_view(next, cx);
                }
            }
            ControlId::DefaultDirection => {
                let current = direction_index(cx.global::<Ui>().views.defaults_layer().ascending);
                let next = (current as isize + delta).clamp(0, 2) as usize;
                if next != current {
                    self.select_direction(next, cx);
                }
            }
            ControlId::PreviewScope => {
                let next = (self.preview_scope_index() as isize + delta).clamp(0, 2) as usize;
                self.set_preview_scope(next, window, cx);
            }
            _ => {}
        }
    }

    fn move_vertical(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.sidebar_focused(window) {
            self.move_sidebar(delta, cx);
            return;
        }
        let Some(id) = self.focused_control(window) else { return };
        match id {
            ControlId::Theme
            | ControlId::FontFamily
            | ControlId::IconPack
            | ControlId::DefaultSort
            | ControlId::PreviewRuleSelect
            | ControlId::PreviewAdd
                if id.enabled(cx) =>
            {
                if self.menu == Some(id) {
                    self.move_menu(delta, cx);
                } else {
                    self.open_menu(id, cx);
                }
            }
            ControlId::PreviewList => {
                let len = self.preview_list_state_len(cx);
                if len > 0 {
                    let next = (self.preview_list_cursor as isize + delta).clamp(0, len as isize - 1) as usize;
                    if next != self.preview_list_cursor {
                        self.preview_list_cursor = next;
                        self.preview_list_scroll.scroll_to_item(next);
                        cx.notify();
                    }
                }
            }
            ControlId::FontSize | ControlId::WindowWidth | ControlId::WindowHeight => {
                // ↑ is -1 in reading order but +1 for a stepper.
                self.adjust_stepper(id, -(delta as f64), window, cx);
            }
            ControlId::Plugins => self.move_plugin_cursor(delta, cx),
            ControlId::SavedQueryList => self.move_saved_query_cursor(delta, cx),
            ControlId::KeyboardList => self.move_keyboard_cursor(delta, cx),
            _ => {}
        }
    }

    fn move_plugin_cursor(&mut self, delta: isize, cx: &mut Context<Self>) {
        let plugins = cx.global::<Plugins>().inventory.plugins.clone();
        let count = plugins.len();
        if count == 0 {
            self.selected_plugin = None;
            self.plugin_position = 0;
            return;
        }
        if self.selected_plugin.is_none() {
            self.selected_plugin = Some(plugins[0].bundle.clone());
            self.plugin_position = 0;
            self.plugin_scroll.scroll_to_item(0);
            cx.notify();
            return;
        }
        let current = self.selected_plugin_index(&plugins).unwrap_or(self.plugin_position.min(count - 1));
        let next = (current as isize + delta).clamp(0, count as isize - 1) as usize;
        if next != current || self.selected_plugin.is_none() {
            self.selected_plugin = Some(plugins[next].bundle.clone());
            self.plugin_position = next;
            self.plugin_scroll.scroll_to_item(next);
            cx.notify();
        }
    }

    fn selected_plugin_index(&self, plugins: &[PluginEntry]) -> Option<usize> {
        let selected = self.selected_plugin.as_ref()?;
        plugins.iter().position(|entry| &entry.bundle == selected)
    }

    fn selected_plugin(&self, cx: &App) -> Option<PluginEntry> {
        let plugins = &cx.global::<Plugins>().inventory.plugins;
        let index = self.selected_plugin_index(plugins)?;
        plugins.get(index).cloned()
    }

    fn activate_plugin(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self.selected_plugin(cx) else { return };
        self.plugin_action(primary_plugin_action(&entry), cx);
    }

    fn plugin_action(&mut self, action: PluginAction, cx: &mut Context<Self>) {
        let grants_error = cx.global::<Plugins>().inventory.grants_error.is_some();
        let Some(entry) = self.selected_plugin(cx) else { return };
        if !action.allowed(&entry, grants_error) {
            return;
        }
        if action == PluginAction::Approve {
            // The keyboard approval must act on the row that was rendered, not
            // on whatever moved into its place before the refresh was drawn.
            let (Some(target), Some(latest)) = (self.plugin_target.as_ref(), PluginTarget::for_entry(&entry)) else {
                return;
            };
            if *target != latest {
                return;
            }
        }
        let handle = cx.global::<Plugins>().handle.clone();
        send_plugin_action(handle, entry, action, cx);
    }

    fn plugin_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.modified() {
            return;
        }
        match event.keystroke.key.as_str() {
            "a" => self.plugin_action(PluginAction::Approve, cx),
            "e" => self.plugin_action(PluginAction::Enable, cx),
            "d" => self.plugin_action(PluginAction::Disable, cx),
            "r" => self.plugin_action(PluginAction::Reload, cx),
            "t" => self.plugin_action(PluginAction::Retry, cx),
            "o" => ui::open_plugins_folder(cx),
            "g" => ui::open_config_file("plugins.toml", cx),
            "l" => crate::plugins::reload(cx),
            _ => {}
        }
    }

    fn plugin_section(&mut self, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let inventory = cx.global::<Plugins>().inventory.clone();
        let handle = cx.global::<Plugins>().handle.clone();
        let count = inventory.plugins.len();
        let grants_error = inventory.grants_error.is_some();
        let mut scroll_to_selected = false;
        let selected = if count == 0 {
            self.selected_plugin = None;
            self.plugin_position = 0;
            self.plugin_target = None;
            None
        } else {
            let index = match self.selected_plugin_index(&inventory.plugins) {
                Some(index) => index,
                None => {
                    // The selected bundle disappeared: use its previous
                    // position, or the previous row when it was last.
                    let index = self.plugin_position.min(count - 1);
                    self.selected_plugin = Some(inventory.plugins[index].bundle.clone());
                    scroll_to_selected = true;
                    index
                }
            };
            self.plugin_position = index;
            self.plugin_target = PluginTarget::for_entry(&inventory.plugins[index]);
            Some(index)
        };
        let focus = self.handle(ControlId::Plugins);
        let focused = focus.is_focused(window);
        let mut headers: Vec<gpui::AnyElement> = Vec::new();
        if let Some(error) = &inventory.grants_error {
            headers.push(
                div()
                    .id("plugins-grants-error")
                    .role(Role::Alert)
                    .aria_label(error.clone())
                    .text_size(px(11.))
                    .text_color(color(theme.role("error")))
                    .child(error.clone())
                    .into_any_element(),
            );
        }
        if let Some(error) = &inventory.scan_error {
            headers.push(
                div()
                    .id("plugins-scan-error")
                    .role(Role::Alert)
                    .aria_label(error.clone())
                    .text_size(px(11.))
                    .text_color(color(theme.role("error")))
                    .child(error.clone())
                    .into_any_element(),
            );
        }
        let mut plugin_rows: Vec<gpui::AnyElement> = Vec::new();
        for (index, entry) in inventory.plugins.iter().enumerate() {
            plugin_rows.push(
                plugin_row(index, count, entry, Some(index) == selected, focused, grants_error, theme, handle.clone())
                    .into_any_element(),
            );
        }
        let mut list = div()
            .id("settings-plugins")
            .debug_selector(|| "settings-plugins".to_owned())
            .role(Role::ListBox)
            .aria_label(format!("Plugins, {count} installed"))
            .track_focus(&focus)
            .flex()
            .flex_col()
            .gap(px(6.))
            .max_h(px(320.))
            .overflow_y_scroll()
            .track_scroll(&self.plugin_scroll)
            .on_key_down(cx.listener(|this, event, _, cx| this.plugin_key(event, cx)));
        if count == 0 {
            list = list.child(
                div()
                    .id("plugins-empty")
                    .role(Role::Status)
                    .aria_label("No plugins installed")
                    .text_size(px(12.))
                    .text_color(color(theme.role("fg-muted")))
                    .child("No plugins installed."),
            );
        } else {
            list = list.children(plugin_rows);
        }
        let footer = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(open_plugins_button(theme))
            .child(open_grants_button(theme))
            .child(reload_all_button(theme));
        if scroll_to_selected && let Some(index) = selected {
            self.plugin_scroll.scroll_to_item(index);
        }
        div()
            .id("settings-plugins-surface")
            .flex()
            .flex_col()
            .gap(px(6.))
            .p(px(8.))
            .rounded(px(7.))
            .bg(color(theme.role("surface")))
            .when(focused, |surface| surface.shadow(cursor_ring(theme)))
            .children(headers)
            .child(list)
            .child(footer)
            .into_any_element()
    }

    fn adjust_focused(&mut self, amount: f64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.focused_control(window) else { return };
        if matches!(id, ControlId::FontSize | ControlId::WindowWidth | ControlId::WindowHeight) {
            self.adjust_stepper(id, amount, window, cx);
        }
    }

    fn move_segment(&mut self, id: ControlId, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let (current, count) = match id {
            ControlId::Keymap => (keymap_index(cx.global::<Ui>().settings.keymap), 2),
            ControlId::Enter => (enter_index(cx.global::<Ui>().settings.enter), 2),
            ControlId::Hints => (hints_index(cx.global::<Ui>().settings.hints), 3),
            ControlId::Tips => (tips_index(cx.global::<Ui>().settings.tips), 2),
            ControlId::FolderSizes => (folder_sizes_index(cx.global::<Ui>().settings.folder_sizes), 2),
            ControlId::PathBar => (path_bar_index(cx.global::<Ui>().settings.path_bar), 2),
            ControlId::IconStyle => (icon_style_index(cx.global::<Ui>().settings.icon_style), 2),
            ControlId::SidebarStyle => (sidebar_style_index(cx.global::<Ui>().settings.sidebar_style), 5),
            ControlId::SearchMatch => (search_match_index(cx.global::<Ui>().settings.search_match), 3),
            ControlId::Motion => (motion_index(cx.global::<Ui>().settings.motion), 4),
            ControlId::Sound => (sound_index(cx.global::<Ui>().settings.sound), 3),
            _ => return,
        };
        let next = (current as isize + delta).clamp(0, count - 1) as usize;
        if next != current {
            self.select_segment(id, next, window, cx);
        }
    }

    fn select_segment(&mut self, id: ControlId, index: usize, _window: &mut Window, cx: &mut Context<Self>) {
        let change = match id {
            ControlId::Keymap => SettingChange::Keymap(if index == 0 { KeymapLayer::Finder } else { KeymapLayer::Vim }),
            ControlId::Enter => SettingChange::Enter(if index == 0 { EnterAction::Rename } else { EnterAction::Open }),
            ControlId::Hints => SettingChange::Hints(match index {
                0 => HintsMode::Auto,
                1 => HintsMode::Always,
                _ => HintsMode::Off,
            }),
            ControlId::Tips => SettingChange::Tips(if index == 0 { TipsMode::Auto } else { TipsMode::Off }),
            ControlId::FolderSizes => {
                SettingChange::FolderSizes(if index == 0 { FolderSizesMode::Off } else { FolderSizesMode::Visible })
            }
            ControlId::IconStyle => SettingChange::IconStyle(if index == 0 {
                crow_core::IconStyle::Outline
            } else {
                crow_core::IconStyle::Finder
            }),
            ControlId::PathBar => {
                SettingChange::PathBar(if index == 0 { PathBarMode::Crow } else { PathBarMode::Explorer })
            }
            ControlId::SidebarStyle => SettingChange::SidebarStyle(crow_core::SidebarStyle::ALL[index.min(4)]),
            ControlId::SearchMatch => SettingChange::SearchMatch(match index {
                0 => SearchMatch::All,
                1 => SearchMatch::Names,
                _ => SearchMatch::Contents,
            }),
            ControlId::Motion => SettingChange::Motion(match index {
                0 => MotionMode::Auto,
                1 => MotionMode::Off,
                2 => MotionMode::Low,
                _ => MotionMode::Full,
            }),
            ControlId::Sound => SettingChange::Sound(match index {
                0 => SoundSet::Crow,
                1 => SoundSet::Finder,
                _ => SoundSet::Off,
            }),
            _ => return,
        };
        ui::set_setting(change, cx);
        cx.notify();
    }

    fn toggle_bool(&mut self, id: ControlId, cx: &mut Context<Self>) {
        let settings = &cx.global::<Ui>().settings;
        let change = match id {
            ControlId::ShowHidden => SettingChange::ShowHiddenFiles(!settings.show_hidden_files),
            ControlId::SearchHidden => SettingChange::SearchIncludesHidden(!settings.search_includes_hidden),
            ControlId::ShowPreview => SettingChange::ShowPreview(!settings.show_preview),
            ControlId::ViewPerFolder => SettingChange::ViewPerFolder(!settings.view_per_folder),
            ControlId::PaletteRecent => SettingChange::PaletteRecentCommands(!settings.palette_recent_commands),
            ControlId::TranslucentSidebar => SettingChange::TranslucentSidebar(!settings.translucent_sidebar),
            _ => return,
        };
        ui::set_setting(change, cx);
        cx.notify();
    }

    /// Rebuild the icon-pack choices from the current snapshot. The saved id
    /// stays in the list when it is missing, so the menu never drops the value
    /// the file holds. Returns the saved choice's index.
    fn refresh_icon_pack_choices(&mut self, cx: &App) -> usize {
        let saved = cx.global::<Ui>().settings.icon_pack.clone();
        let choices = cx
            .try_global::<crate::icon_packs::IconPacks>()
            .map(crate::icon_packs::IconPacks::snapshot)
            .map(|snapshot| snapshot.choices())
            .unwrap_or_default();
        let mut ids = Vec::new();
        let mut labels = Vec::new();
        for choice in &choices {
            // Duplicate display names are disambiguated with the id; two
            // different ids cannot both be offered as the same label.
            let duplicates = choices.iter().filter(|other| other.name.eq_ignore_ascii_case(&choice.name)).count() > 1;
            let label = if duplicates { format!("{} ({})", choice.name, choice.id) } else { choice.name.clone() };
            ids.push(choice.id.clone());
            labels.push(label);
        }
        // Several packs can be on at once: every chosen pack is ticked, and picking a row adds it
        // to the list or takes it out. A chosen pack that is not installed stays listed.
        let chosen: Vec<&str> =
            crow_core::split_sources(&saved).filter(|part| *part != crow_core::NATIVE_ICON_SOURCE).collect();
        for (id, label) in ids.iter().zip(labels.iter_mut()) {
            if chosen.contains(&id.as_str()) {
                *label = format!("✓ {label}");
            }
        }
        for part in &chosen {
            if !ids.iter().any(|id| id == part) {
                ids.push((*part).to_owned());
                labels.push(format!("✓ {part} (not installed)"));
            }
        }
        let value = cx
            .try_global::<crate::icon_packs::IconPacks>()
            .map_or_else(|| saved.clone(), |packs| packs.snapshot().display_name(&saved));
        let selected = chosen.first().and_then(|first| ids.iter().position(|id| id == first)).unwrap_or(0);
        self.icon_pack_value = if chosen.is_empty() { "None".to_owned() } else { value };
        self.icon_pack_ids = ids;
        self.icon_pack_labels = labels;
        selected
    }

    /// The first chosen pack: where the menu's cursor starts.
    fn first_icon_pack(&self, saved: &str) -> String {
        crow_core::split_sources(saved)
            .find(|part| *part != crow_core::NATIVE_ICON_SOURCE)
            .map(str::to_owned)
            .or_else(|| self.icon_pack_ids.first().cloned())
            .unwrap_or_default()
    }

    fn icon_pack_cursor_index(&self) -> Option<usize> {
        let cursor = self.icon_pack_cursor.as_ref()?;
        self.icon_pack_ids.iter().position(|id| id == cursor)
    }

    /// Open `id`'s menu with the cursor on the saved value.
    fn open_menu(&mut self, id: ControlId, cx: &mut Context<Self>) {
        self.menu_settle.set(3);
        let settings = &cx.global::<Ui>().settings;
        match id {
            ControlId::Theme => {
                self.theme.sync_to(&settings.theme);
                self.theme_scroll.scroll_to_item(self.theme.cursor);
            }
            ControlId::FontFamily => {
                self.font_cursor = font_index(&self.fonts, &settings.font_family);
                self.font_scroll.scroll_to_item(self.font_cursor);
            }
            ControlId::IconPack => {
                let selected = self.refresh_icon_pack_choices(cx);
                self.icon_pack_cursor = Some(self.first_icon_pack(&settings.icon_pack));
                self.icon_pack_scroll.scroll_to_item(selected);
            }
            ControlId::DefaultSort => {
                self.sort_cursor = sort_key_index(cx.global::<Ui>().views.defaults_layer().sort.unwrap_or_default());
                self.sort_scroll.scroll_to_item(self.sort_cursor);
            }
            ControlId::PreviewRuleSelect => self.open_preview_rule_menu(cx),
            ControlId::PreviewAdd => self.open_preview_add_menu(cx),
            _ => return,
        }
        self.menu = Some(id);
        cx.notify();
    }

    /// Close the open menu without keeping its cursor: the theme preview goes
    /// away and every drop-down shows the saved value again.
    fn close_menu(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.menu.take() else { return };
        let settings = &cx.global::<Ui>().settings;
        match id {
            ControlId::Theme => {
                self.theme.sync_to(&settings.theme);
                if cx.global::<Ui>().preview_theme.is_some() {
                    theme_choice::clear_preview(cx);
                }
            }
            ControlId::FontFamily => self.font_cursor = font_index(&self.fonts, &settings.font_family),
            ControlId::IconPack => {
                self.icon_pack_cursor = Some(self.first_icon_pack(&settings.icon_pack));
            }
            ControlId::DefaultSort => {
                self.sort_cursor = sort_key_index(cx.global::<Ui>().views.defaults_layer().sort.unwrap_or_default());
            }
            ControlId::PreviewRuleSelect | ControlId::PreviewAdd => {}
            _ => {}
        }
        cx.notify();
    }

    /// Keep the menu's highlighted entry and close it.
    fn keep_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.menu {
            Some(ControlId::Theme) => self.commit_theme(window, cx),
            Some(ControlId::FontFamily) => self.commit_font(window, cx),
            Some(ControlId::IconPack) => {
                // A multi-select: Return ticks or unticks the highlighted pack and the menu stays
                // open; Escape or a click outside closes it.
                self.commit_icon_pack(cx);
                cx.notify();
                return;
            }
            Some(ControlId::DefaultSort) => self.commit_sort(cx),
            Some(ControlId::PreviewRuleSelect) | Some(ControlId::PreviewAdd) => self.keep_preview_menu(cx),
            _ => {}
        }
        self.menu = None;
        cx.notify();
    }

    fn toggle_menu(&mut self, id: ControlId, window: &mut Window, cx: &mut Context<Self>) {
        if !id.enabled(cx) {
            return;
        }
        if self.dismissed_by_mouse.take() == Some(id) {
            return;
        }
        if self.menu == Some(id) {
            self.close_menu(window, cx);
        } else {
            self.close_menu(window, cx);
            window.focus(&self.handle(id), cx);
            self.open_menu(id, cx);
        }
    }

    fn move_menu(&mut self, delta: isize, cx: &mut Context<Self>) {
        match self.menu {
            Some(ControlId::Theme) => {
                if self.theme.move_cursor(delta) {
                    theme_choice::preview(&self.theme.current_id(), cx);
                    self.theme_scroll.scroll_to_item(self.theme.cursor);
                }
            }
            Some(ControlId::FontFamily) => {
                let last = self.fonts.len().saturating_sub(1);
                self.font_cursor = self.font_cursor.saturating_add_signed(delta).min(last);
                self.font_scroll.scroll_to_item(self.font_cursor);
            }
            Some(ControlId::IconPack) => {
                let Some(current) = self.icon_pack_cursor_index() else { return };
                let last = self.icon_pack_ids.len().saturating_sub(1);
                let next = current.saturating_add_signed(delta).min(last);
                self.icon_pack_cursor = self.icon_pack_ids.get(next).cloned();
                self.icon_pack_scroll.scroll_to_item(next);
            }
            Some(ControlId::DefaultSort) => {
                self.sort_cursor = self.sort_cursor.saturating_add_signed(delta).min(SORT_LABELS.len() - 1);
                self.sort_scroll.scroll_to_item(self.sort_cursor);
            }
            Some(ControlId::PreviewRuleSelect) | Some(ControlId::PreviewAdd) => {
                self.move_preview_menu(delta, cx);
                return;
            }
            _ => return,
        }
        cx.notify();
    }

    /// A click on a menu entry keeps it.
    fn pick(&mut self, id: ControlId, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu != Some(id) {
            return;
        }
        match id {
            ControlId::Theme => {
                self.theme.select(index);
            }
            ControlId::FontFamily if index < self.fonts.len() => self.font_cursor = index,
            ControlId::DefaultSort if index < SORT_LABELS.len() => self.sort_cursor = index,
            _ => return,
        }
        self.keep_menu(window, cx);
    }

    /// A click on an icon-pack row. The ids are captured from the same render
    /// that drew the row, so a catalog reload cannot move the click onto a
    /// different pack.
    fn pick_icon_pack(&mut self, ids: &[String], index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu != Some(ControlId::IconPack) {
            return;
        }
        let Some(id) = ids.get(index) else { return };
        self.icon_pack_cursor = Some(id.clone());
        self.keep_menu(window, cx);
    }

    fn commit_icon_pack(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.icon_pack_cursor.clone() else { return };
        // The menu changed under the cursor: keep the saved choice instead of
        // committing whichever pack now occupies that row.
        if !self.icon_pack_ids.iter().any(|candidate| candidate == &id) {
            return;
        }
        // Picking a pack adds it to (or removes it from) the layered list; Native clears it.
        let next = crow_core::toggle_source(&cx.global::<Ui>().settings.icon_pack, &id);
        if cx.global::<Ui>().settings.icon_pack != next {
            ui::set_setting(SettingChange::IconPack(next), cx);
        }
    }

    fn commit_sort(&mut self, cx: &mut Context<Self>) {
        let key = sort_key_at(self.sort_cursor);
        let current = cx.global::<Ui>().views.defaults_layer().sort.unwrap_or_default();
        if current != key {
            ui::save_defaults_change(DefaultsChange::SortKey(key), None, cx);
        }
    }

    fn select_default_view(&mut self, index: usize, cx: &mut Context<Self>) {
        let mode = match index {
            0 => ViewMode::Icons,
            1 => ViewMode::List,
            2 => ViewMode::Columns,
            _ => ViewMode::Gallery,
        };
        if cx.global::<Ui>().views.defaults_layer().view != Some(mode) {
            ui::save_defaults_change(DefaultsChange::View(mode), None, cx);
        }
    }

    fn select_direction(&mut self, index: usize, cx: &mut Context<Self>) {
        let raw = cx.global::<Ui>().views.defaults_layer().ascending;
        let change = match index {
            0 => raw.is_some().then_some(DefaultsChange::Ascending(None)),
            1 => (raw != Some(true)).then_some(DefaultsChange::Ascending(Some(true))),
            _ => (raw != Some(false)).then_some(DefaultsChange::Ascending(Some(false))),
        };
        if let Some(change) = change {
            ui::save_defaults_change(change, None, cx);
        }
    }

    fn toggle_default_folders_first(&mut self, cx: &mut Context<Self>) {
        let raw = cx.global::<Ui>().views.defaults_layer().folders_first;
        let next = !raw.unwrap_or(true);
        ui::save_defaults_change(DefaultsChange::FoldersFirst(next), None, cx);
    }

    fn commit_theme(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let id = self.theme.current_id();
        if cx.global::<Ui>().settings.theme == id {
            if cx.global::<Ui>().preview_theme.is_some() {
                theme_choice::clear_preview(cx);
            }
        } else {
            ui::choose_theme(&id, cx);
        }
    }

    fn commit_font(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.fonts.get(self.font_cursor).cloned() else { return };
        let family = if name == "System" { String::new() } else { name };
        if cx.global::<Ui>().settings.font_family != family {
            ui::set_setting(SettingChange::FontFamily(family), cx);
        }
    }

    /// Typing on a drop-down opens it at the first entry starting with what was
    /// typed; the theme previews like an arrow key would.
    fn menu_type_ahead(&mut self, id: ControlId, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if !id.enabled(cx) {
            return;
        }
        let found = match id {
            ControlId::Theme => {
                let labels: Vec<String> = (0..self.theme.entries.len()).map(|i| self.theme.label(i)).collect();
                type_ahead(&mut self.theme_type_ahead, &event.keystroke, &labels)
            }
            ControlId::FontFamily => type_ahead(&mut self.font_type_ahead, &event.keystroke, &self.fonts),
            ControlId::IconPack => type_ahead(&mut self.font_type_ahead, &event.keystroke, &self.icon_pack_labels),
            ControlId::DefaultSort => type_ahead(&mut self.font_type_ahead, &event.keystroke, &sort_labels()),
            _ => None,
        };
        let Some(index) = found else { return };
        if self.menu != Some(id) {
            self.open_menu(id, cx);
        }
        match id {
            ControlId::Theme => {
                if self.theme.select(index) {
                    theme_choice::preview(&self.theme.current_id(), cx);
                }
                self.theme_scroll.scroll_to_item(self.theme.cursor);
            }
            ControlId::FontFamily => {
                self.font_cursor = index;
                self.font_scroll.scroll_to_item(index);
            }
            ControlId::IconPack => {
                self.icon_pack_cursor = self.icon_pack_ids.get(index).cloned();
                self.icon_pack_scroll.scroll_to_item(index);
            }
            ControlId::DefaultSort => {
                self.sort_cursor = index;
                self.sort_scroll.scroll_to_item(index);
            }
            _ => {}
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Commit a typed stepper value and close an open menu before focus leaves
    /// a control. Tab and Shift-Tab call this explicitly; the focus-out
    /// listeners cover a click elsewhere.
    fn commit_focused(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_menu(window, cx);
        match self.focused_control(window) {
            Some(ControlId::FontSize) => self.commit_stepper(ControlId::FontSize, window, cx),
            Some(ControlId::Terminal) => self.commit_terminal(window, cx),
            Some(ControlId::WindowWidth) => self.commit_stepper(ControlId::WindowWidth, window, cx),
            Some(ControlId::WindowHeight) => self.commit_stepper(ControlId::WindowHeight, window, cx),
            _ => {}
        }
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saved_query_form.is_some() {
            self.close_saved_query_editor(window, cx);
        } else if self.keyboard_cancel(cx) {
        } else if self.menu.is_some() {
            self.close_menu(window, cx);
        } else if !self.query.is_empty() {
            self.clear_query(true, window, cx);
        } else {
            self.close(window, cx);
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_focused(window, cx);
        window.remove_window();
    }
}

impl SettingsWindow {
    /// A drop-down row. The closed button shows the saved value; the open menu
    /// highlights `cursor`.
    #[allow(clippy::too_many_arguments)]
    fn dropdown(
        &self,
        id: ControlId,
        options: &[String],
        cursor: usize,
        scroll: &ScrollHandle,
        window: &Window,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let settings = &cx.global::<Ui>().settings;
        let saved = match id {
            ControlId::Theme => {
                (0..self.theme.entries.len()).find(|i| self.theme.id(*i) == settings.theme).unwrap_or(0)
            }
            ControlId::FontFamily => font_index(&self.fonts, &settings.font_family),
            ControlId::IconPack => self.icon_pack_ids.iter().position(|id| id == &settings.icon_pack).unwrap_or(0),
            ControlId::DefaultSort => sort_key_index(cx.global::<Ui>().views.defaults_layer().sort.unwrap_or_default()),
            _ => 0,
        };
        let pick = select_callback(cx, move |this, index, window, cx| this.pick(id, index, window, cx));
        self.dropdown_with(id, options, saved, cursor, scroll, window, theme, cx, pick)
    }

    /// The shared pop-up button with a caller-supplied click mapper, so an
    /// icon-pack row can commit the id it was drawn with.
    #[allow(clippy::too_many_arguments)]
    fn dropdown_with(
        &self,
        id: ControlId,
        options: &[String],
        saved: usize,
        cursor: usize,
        scroll: &ScrollHandle,
        window: &Window,
        theme: &Theme,
        cx: &mut Context<Self>,
        pick: controls::Select,
    ) -> gpui::Stateful<gpui::Div> {
        if self.menu == Some(id) && self.menu_settle.get() > 0 {
            self.menu_settle.set(self.menu_settle.get() - 1);
            scroll.scroll_to_item(cursor);
            cx.notify();
        }
        let menu = (self.menu == Some(id)).then(|| controls::Menu {
            cursor,
            scroll,
            dismiss: callback(cx, move |this, window, cx| {
                this.close_menu(window, cx);
                this.dismissed_by_mouse = Some(id);
            }),
        });
        let toggle = callback(cx, move |this, window, cx| this.toggle_menu(id, window, cx));
        let value = if id == ControlId::IconPack {
            self.icon_pack_value.clone()
        } else {
            options.get(saved).cloned().unwrap_or_default()
        };
        controls::dropdown_valued(self.meta(id, cx), options, value, menu, self.style(id, window, theme), toggle, pick)
    }
}

/// The state a control announces, for the accessibility invariant and the
/// keystroke tests. The renderer reads the same `Ui.settings` and cursors.
#[cfg(test)]
#[derive(Debug, PartialEq)]
struct ControlState {
    role: Role,
    label: String,
    enabled: bool,
    selected: Option<usize>,
    toggled: Option<bool>,
    value: Option<f64>,
}

#[cfg(test)]
impl SettingsWindow {
    fn control_state(&self, id: ControlId, cx: &App) -> ControlState {
        let settings = &cx.global::<Ui>().settings;
        ControlState {
            role: id.role(),
            label: id.label().to_owned(),
            enabled: id.enabled(cx),
            selected: match id {
                ControlId::Theme => Some(self.theme.cursor),
                ControlId::FontFamily => Some(self.font_cursor),
                ControlId::Keymap => Some(keymap_index(settings.keymap)),
                ControlId::Enter => Some(enter_index(settings.enter)),
                ControlId::Hints => Some(hints_index(settings.hints)),
                ControlId::Tips => Some(tips_index(settings.tips)),
                ControlId::FolderSizes => Some(folder_sizes_index(settings.folder_sizes)),
                ControlId::PathBar => Some(path_bar_index(settings.path_bar)),
                ControlId::IconStyle => Some(icon_style_index(settings.icon_style)),
                ControlId::SidebarStyle => Some(sidebar_style_index(settings.sidebar_style)),
                ControlId::SearchMatch => Some(search_match_index(settings.search_match)),
                ControlId::Motion => Some(motion_index(settings.motion)),
                ControlId::Sound => Some(sound_index(settings.sound)),
                ControlId::IconPack => {
                    let first = self.first_icon_pack(&settings.icon_pack);
                    self.icon_pack_ids.iter().position(|id| *id == first)
                }
                ControlId::DefaultView => {
                    Some(view_index(cx.global::<Ui>().views.defaults_layer().view.unwrap_or_default()))
                }
                ControlId::DefaultSort => {
                    Some(sort_key_index(cx.global::<Ui>().views.defaults_layer().sort.unwrap_or_default()))
                }
                ControlId::DefaultDirection => {
                    Some(direction_index(cx.global::<Ui>().views.defaults_layer().ascending))
                }
                ControlId::Plugins => self.selected_plugin_index(&cx.global::<Plugins>().inventory.plugins),
                _ => None,
            },
            toggled: match id {
                ControlId::ShowHidden => Some(settings.show_hidden_files),
                ControlId::SearchHidden => Some(settings.search_includes_hidden),
                ControlId::ShowPreview => Some(settings.show_preview),
                ControlId::ViewPerFolder => Some(settings.view_per_folder),
                ControlId::PaletteRecent => Some(settings.palette_recent_commands),
                ControlId::TranslucentSidebar => Some(settings.translucent_sidebar),
                ControlId::DefaultFoldersFirst => {
                    Some(cx.global::<Ui>().views.defaults_layer().folders_first.unwrap_or(true))
                }
                _ => None,
            },
            value: match id {
                ControlId::FontSize => Some(f64::from(settings.font_size)),
                ControlId::WindowWidth => Some(f64::from(settings.window.width)),
                ControlId::WindowHeight => Some(f64::from(settings.window.height)),
                _ => None,
            },
        }
    }

    fn set_theme_entries_for_tests(&mut self, infos: Vec<crow_theme::ThemeInfo>, current: &str) {
        self.theme = ThemeChoice::new(infos, current);
    }

    fn set_fonts_for_tests(&mut self, fonts: Vec<String>, current: &str) {
        self.fonts = fonts;
        self.font_cursor = font_index(&self.fonts, current);
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.motion_mode = cx.global::<Ui>().feedback.mode;
        let (theme, font, font_size, settings) = {
            let ui = cx.global::<Ui>();
            (ui.theme.clone(), ui_font(&ui.settings.font_family), ui.settings.font_size, ui.settings.clone())
        };
        let issues = self.issues(cx);

        if self.menu.is_some() && self.focused_control(window) != self.menu {
            self.close_menu(window, cx);
        }
        if self.menu != Some(ControlId::Theme) {
            self.theme.sync_to(&settings.theme);
        }
        if self.menu != Some(ControlId::FontFamily) {
            self.font_cursor = font_index(&self.fonts, &settings.font_family);
        }
        let icon_selected = self.refresh_icon_pack_choices(cx);
        if self.menu != Some(ControlId::IconPack)
            || self.icon_pack_cursor.as_ref().is_none_or(|id| !self.icon_pack_ids.contains(id))
        {
            self.icon_pack_cursor = Some(self.first_icon_pack(&settings.icon_pack));
        }
        if self.menu != Some(ControlId::DefaultSort) {
            self.sort_cursor = sort_key_index(cx.global::<Ui>().views.defaults_layer().sort.unwrap_or_default());
        }
        for id in [ControlId::FontSize, ControlId::WindowWidth, ControlId::WindowHeight] {
            self.sync_stepper(id, window, cx);
        }
        self.sync_terminal(window, cx);
        self.repair_preview_target(cx);
        let visible = self.visible_controls(cx);
        for id in ControlId::ALL {
            self.handle(id).tab_stop(visible.contains(&id));
        }
        self.sidebar.clone().tab_stop(true);
        for file in ERROR_FILES {
            self.error_handles[file].clone().tab_stop(issues.iter().any(|issue| issue.file == file));
        }
        let search_hits = if self.query.is_empty() { Vec::new() } else { search::matches(&self.query) };
        self.clear_search.clone().tab_stop(!self.query.is_empty() && search_hits.is_empty());
        if let Some(id) = self.focused_control(window)
            && !visible.contains(&id)
        {
            // An external edit disabled or hid the control: keep a usable target.
            let handle = self.search_handle(cx);
            window.focus(&handle, cx);
        }

        let theme_labels: Vec<String> = (0..self.theme.entries.len()).map(|i| self.theme.label(i)).collect();
        let theme_row = self
            .dropdown(ControlId::Theme, &theme_labels, self.theme.cursor, &self.theme_scroll, window, &theme, cx)
            .on_key_down(cx.listener(|this, event, _, cx| this.menu_type_ahead(ControlId::Theme, event, cx)));

        let font_row = self
            .dropdown(ControlId::FontFamily, &self.fonts, self.font_cursor, &self.font_scroll, window, &theme, cx)
            .on_key_down(cx.listener(|this, event, _, cx| this.menu_type_ahead(ControlId::FontFamily, event, cx)));

        let font_size_row = controls::stepper(
            self.meta(ControlId::FontSize, cx),
            &self.font_size,
            self.style(ControlId::FontSize, window, &theme),
        );

        let icon_labels = self.icon_pack_labels.clone();
        let icon_ids = self.icon_pack_ids.clone();
        let icon_cursor = self.icon_pack_cursor_index().unwrap_or(icon_selected);
        let icon_pick =
            select_callback(cx, move |this, index, window, cx| this.pick_icon_pack(&icon_ids, index, window, cx));
        let icon_dropdown = self
            .dropdown_with(
                ControlId::IconPack,
                &icon_labels,
                icon_selected,
                icon_cursor,
                &self.icon_pack_scroll,
                window,
                &theme,
                cx,
                icon_pick,
            )
            .on_key_down(cx.listener(|this, event, _, cx| this.menu_type_ahead(ControlId::IconPack, event, cx)));
        let icon_error = cx.try_global::<crate::icon_packs::IconPacks>().and_then(|packs| packs.error().cloned());
        let icon_row = div().flex().flex_col().gap(px(4.)).child(icon_dropdown).children(icon_error.map(|error| {
            div()
                .id("settings-icon-pack-error")
                .role(Role::Alert)
                .aria_label(error.clone())
                .text_size(px(11.))
                .text_color(color(theme.role("error")))
                .child(error)
        }));

        let icon_style_select =
            select_callback(cx, |this, index, window, cx| this.select_segment(ControlId::IconStyle, index, window, cx));
        let icon_style_row = controls::segmented(
            self.meta(ControlId::IconStyle, cx),
            &["Outline", "Finder"],
            icon_style_index(settings.icon_style),
            self.style(ControlId::IconStyle, window, &theme),
            icon_style_select,
            None,
        );

        let path_bar_select =
            select_callback(cx, |this, index, window, cx| this.select_segment(ControlId::PathBar, index, window, cx));
        let path_bar_row = controls::segmented(
            self.meta(ControlId::PathBar, cx),
            &["Crow", "Explorer"],
            path_bar_index(settings.path_bar),
            self.style(ControlId::PathBar, window, &theme),
            path_bar_select,
            None,
        );

        let sidebar_style_select = select_callback(cx, |this, index, window, cx| {
            this.select_segment(ControlId::SidebarStyle, index, window, cx)
        });
        let sidebar_style_labels = crow_core::SidebarStyle::ALL.map(crow_core::SidebarStyle::label);
        let sidebar_style_row = controls::segmented(
            self.meta(ControlId::SidebarStyle, cx),
            &sidebar_style_labels,
            sidebar_style_index(settings.sidebar_style),
            self.style(ControlId::SidebarStyle, window, &theme),
            sidebar_style_select,
            None,
        );

        let terminal_row = controls::text_field(
            self.meta(ControlId::Terminal, cx),
            &self.terminal,
            self.style(ControlId::Terminal, window, &theme),
            220.0,
        );

        let keymap_select =
            select_callback(cx, |this, index, window, cx| this.select_segment(ControlId::Keymap, index, window, cx));
        let keymap_row = controls::segmented(
            self.meta(ControlId::Keymap, cx),
            &["Finder", "Vim"],
            keymap_index(settings.keymap),
            self.style(ControlId::Keymap, window, &theme),
            keymap_select,
            None,
        );

        let enter_select =
            select_callback(cx, |this, index, window, cx| this.select_segment(ControlId::Enter, index, window, cx));
        let enter_row = controls::segmented(
            self.meta(ControlId::Enter, cx),
            &["Rename", "Open"],
            enter_index(settings.enter),
            self.style(ControlId::Enter, window, &theme),
            enter_select,
            None,
        );

        let hints_select =
            select_callback(cx, |this, index, window, cx| this.select_segment(ControlId::Hints, index, window, cx));
        let hints_row = controls::segmented(
            self.meta(ControlId::Hints, cx),
            &["Auto", "Always", "Off"],
            hints_index(settings.hints),
            self.style(ControlId::Hints, window, &theme),
            hints_select,
            None,
        );

        let tips_select =
            select_callback(cx, |this, index, window, cx| this.select_segment(ControlId::Tips, index, window, cx));
        let tips_row = controls::segmented(
            self.meta(ControlId::Tips, cx),
            &["Auto", "Off"],
            tips_index(settings.tips),
            self.style(ControlId::Tips, window, &theme),
            tips_select,
            None,
        );

        let folder_sizes_select = select_callback(cx, |this, index, window, cx| {
            this.select_segment(ControlId::FolderSizes, index, window, cx)
        });
        let folder_sizes_row = controls::segmented(
            self.meta(ControlId::FolderSizes, cx),
            &["Off", "Visible"],
            folder_sizes_index(settings.folder_sizes),
            self.style(ControlId::FolderSizes, window, &theme),
            folder_sizes_select,
            None,
        );

        let show_hidden = switch_row(self, ControlId::ShowHidden, settings.show_hidden_files, window, &theme, cx);
        let search_hidden =
            switch_row(self, ControlId::SearchHidden, settings.search_includes_hidden, window, &theme, cx);
        let search_match_select = select_callback(cx, |this, index, window, cx| {
            this.select_segment(ControlId::SearchMatch, index, window, cx)
        });
        let search_match_row = controls::segmented(
            self.meta(ControlId::SearchMatch, cx),
            &["All", "Names", "Contents"],
            search_match_index(settings.search_match),
            self.style(ControlId::SearchMatch, window, &theme),
            search_match_select,
            None,
        );
        let show_preview = switch_row(self, ControlId::ShowPreview, settings.show_preview, window, &theme, cx);
        let view_per_folder = switch_row(self, ControlId::ViewPerFolder, settings.view_per_folder, window, &theme, cx);
        let translucent =
            switch_row(self, ControlId::TranslucentSidebar, settings.translucent_sidebar, window, &theme, cx);
        let palette_recent =
            switch_row(self, ControlId::PaletteRecent, settings.palette_recent_commands, window, &theme, cx);

        let motion_select =
            select_callback(cx, |this, index, window, cx| this.select_segment(ControlId::Motion, index, window, cx));
        let motion_row = controls::segmented(
            self.meta(ControlId::Motion, cx),
            &["Auto", "Off", "Low", "Full"],
            motion_index(settings.motion),
            self.style(ControlId::Motion, window, &theme),
            motion_select,
            None,
        );

        let sound_select =
            select_callback(cx, |this, index, window, cx| this.select_segment(ControlId::Sound, index, window, cx));
        let play = controls::button(
            ControlId::PlaySound.key(),
            "Play",
            false,
            ControlId::PlaySound.enabled(cx),
            self.style(ControlId::PlaySound, window, &theme),
            callback(cx, |_, _, cx| crate::feedback::play(Event::Trash, cx)),
        );
        let sound_row = controls::segmented(
            self.meta(ControlId::Sound, cx),
            &["Crow", "Finder", "Off"],
            sound_index(settings.sound),
            self.style(ControlId::Sound, window, &theme),
            sound_select,
            Some(play.into_any_element()),
        );

        let width_row = controls::stepper(
            self.meta(ControlId::WindowWidth, cx),
            &self.window_width,
            self.style(ControlId::WindowWidth, window, &theme),
        );
        let height_row = controls::stepper(
            self.meta(ControlId::WindowHeight, cx),
            &self.window_height,
            self.style(ControlId::WindowHeight, window, &theme),
        );

        let view_select = select_callback(cx, |this, index, _window, cx| this.select_default_view(index, cx));
        let default_view_row = controls::segmented(
            self.meta(ControlId::DefaultView, cx),
            &["Icons", "List", "Columns", "Gallery"],
            view_index(cx.global::<Ui>().views.defaults_layer().view.unwrap_or_default()),
            self.style(ControlId::DefaultView, window, &theme),
            view_select,
            None,
        );

        let sort_labels = sort_labels();
        let sort_row = self
            .dropdown(ControlId::DefaultSort, &sort_labels, self.sort_cursor, &self.sort_scroll, window, &theme, cx)
            .on_key_down(cx.listener(|this, event, _, cx| this.menu_type_ahead(ControlId::DefaultSort, event, cx)));

        let direction_select = select_callback(cx, |this, index, _window, cx| this.select_direction(index, cx));
        let direction_row = controls::segmented(
            self.meta(ControlId::DefaultDirection, cx),
            &["Natural", "Ascending", "Descending"],
            direction_index(cx.global::<Ui>().views.defaults_layer().ascending),
            self.style(ControlId::DefaultDirection, window, &theme),
            direction_select,
            None,
        );

        let folders_first_toggle = callback(cx, |this, _, cx| this.toggle_default_folders_first(cx));
        let folders_first_row = controls::switch(
            self.meta(ControlId::DefaultFoldersFirst, cx),
            cx.global::<Ui>().views.defaults_layer().folders_first.unwrap_or(true),
            self.style(ControlId::DefaultFoldersFirst, window, &theme),
            folders_first_toggle,
        );

        let edit = controls::button(
            ControlId::EditSettings.key(),
            "Edit settings.toml…",
            false,
            true,
            self.style(ControlId::EditSettings, window, &theme),
            callback(cx, |_, _, cx| ui::edit_settings_file(cx)),
        );
        let show_config = controls::button(
            ControlId::ShowConfig.key(),
            "Open config folder",
            false,
            true,
            self.style(ControlId::ShowConfig, window, &theme),
            callback(cx, |_, _, cx| ui::open_config_folder(cx)),
        );
        let config_files = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap(px(8.))
            .child(edit)
            .child(link(self, ControlId::OpenViewsToml, "views.toml", window, &theme, cx))
            .child(link(self, ControlId::OpenSidebarToml, "sidebar.toml", window, &theme, cx))
            .child(link(self, ControlId::OpenExcludeToml, "exclude.toml", window, &theme, cx))
            .child(link(self, ControlId::OpenPluginsToml, "plugins.toml", window, &theme, cx))
            .child(link(self, ControlId::OpenSavedQueriesToml, "saved-queries.toml", window, &theme, cx))
            .child(link(self, ControlId::OpenKeysToml, "keys.toml", window, &theme, cx))
            .child(show_config);
        let config_row = controls::labeled(
            ControlMeta {
                id: "settings-config-files",
                label: "Config files",
                description: Some(
                    "The seven TOML files stay authoritative and editable; Crow writes back only the key you change."
                        .into(),
                ),
                enabled: true,
            },
            &theme,
            config_files,
        );

        self.error_items.clear();
        let error_region = (!issues.is_empty()).then(|| {
            let mut items = Vec::new();
            for (index, issue) in issues.iter().enumerate() {
                self.error_items.insert(issue.file, index);
                let focus = self.error_handles[issue.file].clone();
                let style = controls::ControlStyle {
                    focus: &focus,
                    focused: focus.is_focused(window),
                    theme: &theme,
                    mode: self.motion_mode,
                };
                let file = issue.file;
                let path = issue.path.clone();
                let open = controls::link_button(
                    file,
                    file,
                    true,
                    style,
                    callback(cx, move |_, _, cx| ui::open_config_path(&path, cx)),
                );
                let rest = issue
                    .message
                    .strip_prefix(issue.file)
                    .unwrap_or(&issue.message)
                    .trim_start_matches([' ', ':'])
                    .to_owned();
                items.push(
                    div()
                        .id(("settings-error", index))
                        .role(Role::Alert)
                        .aria_label(issue.message.clone())
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(div().font_weight(FontWeight::SEMIBOLD).child(issue.file))
                                .child(div().text_size(px(11.)).child(rest)),
                        )
                        .child(open)
                        .into_any_element(),
                );
            }
            div()
                .id("settings-errors")
                .role(Role::Group)
                .aria_label("Configuration errors")
                .mx(px(16.))
                .mt(px(10.))
                .p(px(8.))
                .rounded_md()
                .bg(error_soft(&theme))
                .text_color(color(theme.role("error")))
                .max_h(px(132.))
                .overflow_y_scroll()
                .track_scroll(&self.error_scroll)
                .flex()
                .flex_col()
                .gap(px(4.))
                .children(items)
        });

        let plugin_row = self.plugin_section(&theme, window, cx);
        let mut rows: HashMap<ControlId, gpui::AnyElement> = HashMap::new();
        rows.insert(ControlId::Theme, theme_row.into_any_element());
        rows.insert(ControlId::IconStyle, icon_style_row.into_any_element());
        rows.insert(ControlId::IconPack, icon_row.into_any_element());
        rows.insert(ControlId::PathBar, path_bar_row.into_any_element());
        rows.insert(ControlId::SidebarStyle, sidebar_style_row.into_any_element());
        rows.insert(ControlId::FontFamily, font_row.into_any_element());
        rows.insert(ControlId::FontSize, font_size_row.into_any_element());
        rows.insert(ControlId::Keymap, keymap_row.into_any_element());
        rows.insert(ControlId::Enter, enter_row.into_any_element());
        rows.insert(ControlId::Hints, hints_row.into_any_element());
        rows.insert(ControlId::Tips, tips_row.into_any_element());
        rows.insert(ControlId::Terminal, terminal_row.into_any_element());
        rows.insert(ControlId::PaletteRecent, palette_recent.into_any_element());
        rows.insert(ControlId::TranslucentSidebar, translucent.into_any_element());
        rows.insert(ControlId::Motion, motion_row.into_any_element());
        rows.insert(ControlId::Sound, sound_row.into_any_element());
        rows.insert(ControlId::WindowWidth, width_row.into_any_element());
        rows.insert(ControlId::WindowHeight, height_row.into_any_element());
        rows.insert(ControlId::ShowHidden, show_hidden.into_any_element());
        rows.insert(ControlId::FolderSizes, folder_sizes_row.into_any_element());
        rows.insert(ControlId::DefaultView, default_view_row.into_any_element());
        rows.insert(
            ControlId::DefaultSort,
            div().flex().flex_col().gap(px(12.)).child(sort_row).child(direction_row).into_any_element(),
        );
        rows.insert(ControlId::DefaultFoldersFirst, folders_first_row.into_any_element());
        rows.insert(ControlId::ShowPreview, show_preview.into_any_element());
        rows.insert(ControlId::ViewPerFolder, view_per_folder.into_any_element());
        rows.insert(ControlId::SearchHidden, search_hidden.into_any_element());
        rows.insert(ControlId::SearchMatch, search_match_row.into_any_element());
        rows.insert(ControlId::SavedQueryList, self.render_saved_queries(window, &theme, cx));
        rows.insert(ControlId::PreviewScope, self.render_preview_unit(window, &theme, cx));
        rows.insert(ControlId::Plugins, plugin_row);
        rows.insert(
            ControlId::KeyboardList,
            self.render_keyboard(self.query.is_empty() && self.category == Category::Keyboard, window, &theme, cx),
        );
        rows.insert(ControlId::EditSettings, config_row.into_any_element());

        // The titlebar is transparent, so the window's own background shows through
        // it in every theme. A press moves the window; a double-click does what
        // the system setting says.
        let titlebar = div()
            .id("settings-titlebar")
            .debug_selector(|| "settings-titlebar".to_owned())
            .flex_none()
            .h(px(38.))
            .w_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(13.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(color(theme.role("fg-muted")))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|_, event: &gpui::MouseDownEvent, window, cx| {
                    let effects = crate::effects::effects(cx);
                    if event.click_count > 1 {
                        effects.titlebar_double_click(window);
                    } else {
                        effects.start_window_move(window);
                    }
                }),
            )
            .child("Settings");

        let search_focused = self.search_handle(cx).is_focused(window);
        let search_row = div()
            .id("settings-search-row")
            .debug_selector(|| "settings-search-row".to_owned())
            .flex()
            .items_center()
            .gap(px(7.))
            .mx(px(16.))
            .mt(px(8.))
            .h(px(28.))
            .pl(px(9.))
            .pr(px(8.))
            .rounded(px(7.))
            .bg(color(theme.role("surface")))
            .border_1()
            .border_color(if search_focused { color(theme.role("fg-subtle")) } else { color(theme.role("surface")) })
            .child(glyph(Glyph::Search, 13., "fg-muted", &theme))
            .child(div().flex_1().min_w_0().child(self.search.clone()));

        let content_progress =
            self.content_motion.and_then(|motion| motion.progress(cx.global::<Ui>().feedback.mode, Instant::now()));
        if content_progress.is_some() {
            window.request_animation_frame();
        }

        let mut page_items: Vec<gpui::AnyElement> = Vec::new();
        let mut control_items: HashMap<ControlId, usize> = HashMap::new();
        if self.query.is_empty() {
            add_category(&mut page_items, &mut control_items, self.category, &theme, &mut rows);
        } else if search_hits.is_empty() {
            let clear = controls::button(
                "settings-clear-search",
                "Clear Search",
                false,
                true,
                controls::ControlStyle {
                    focus: &self.clear_search,
                    focused: self.clear_search.is_focused(window),
                    theme: &theme,
                    mode: self.motion_mode,
                },
                callback(cx, |this, window, cx| this.clear_query(true, window, cx)),
            );
            page_items.push(
                div()
                    .id("settings-no-match")
                    .debug_selector(|| "settings-no-match".to_owned())
                    .role(Role::Status)
                    .aria_label(format!("No settings match {}", self.query))
                    .w_full()
                    .max_w(px(720.))
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(10.))
                    .py(px(24.))
                    .text_color(color(theme.role("fg-muted")))
                    .child(format!("No settings match “{}”.", self.query))
                    .child(clear)
                    .into_any_element(),
            );
        } else {
            add_search_results(&mut page_items, &mut control_items, &search_hits, &theme, &mut rows);
        }
        self.control_items = control_items;

        let mut scroll = div()
            .id("settings-scroll")
            .debug_selector(|| "settings-scroll".to_owned())
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.page_scroll)
            .px(px(20.))
            .pt(px(8.))
            .pb(px(20.))
            .flex()
            .flex_col()
            .items_center()
            .children(page_items);
        if let Some(t) = content_progress {
            scroll = scroll.relative().top(px(4.0 * (1.0 - t))).opacity(0.25 + 0.75 * t);
        }

        div()
            .id("settings-window")
            .role(Role::Group)
            .aria_label("Settings")
            .relative()
            .key_context("Settings")
            .track_focus(&self.focus)
            .capture_any_mouse_down(cx.listener(|this, _, _, _| this.dismissed_by_mouse = None))
            .on_action(cx.listener(|this, _: &Activate, window, cx| this.activate(window, cx)))
            .on_action(cx.listener(|this, _: &Left, window, cx| this.move_horizontal(-1, window, cx)))
            .on_action(cx.listener(|this, _: &Right, window, cx| this.move_horizontal(1, window, cx)))
            .on_action(cx.listener(|this, _: &Up, window, cx| this.move_vertical(-1, window, cx)))
            .on_action(cx.listener(|this, _: &Down, window, cx| this.move_vertical(1, window, cx)))
            .on_action(cx.listener(|this, _: &IncrementTen, window, cx| this.adjust_focused(10.0, window, cx)))
            .on_action(cx.listener(|this, _: &DecrementTen, window, cx| this.adjust_focused(-10.0, window, cx)))
            .on_action(cx.listener(|this, _: &FocusNext, window, cx| this.advance_focus(1, window, cx)))
            .on_action(cx.listener(|this, _: &FocusPrev, window, cx| this.advance_focus(-1, window, cx)))
            .on_action(cx.listener(|this, _: &Cancel, window, cx| this.cancel(window, cx)))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| this.focus_search(window, cx)))
            .on_action(cx.listener(|this, _: &ResetShortcut, window, cx| {
                if this.is_focused(ControlId::KeyboardList, window) {
                    this.keyboard_reset_cursor_row(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &CloseSettings, window, cx| this.close(window, cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                window.activate_window();
                this.focus_first(window, cx);
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(color(theme.role("bg")))
            .text_color(color(theme.role("fg")))
            .font_family(font)
            .text_size(px(font_size))
            .child(titlebar)
            .child(search_row)
            .children(error_region)
            .child(div().flex_1().min_h_0().flex().child(self.sidebar(&theme, window, cx)).child(scroll))
            .children(self.render_saved_query_editor(window, &theme, cx))
    }
}

fn category_colors(theme: &Theme, focused: bool) -> (gpui::Rgba, gpui::Rgba) {
    (color(theme.role("fg")), if focused { active_selection(theme) } else { inactive_selection(theme) })
}

impl SettingsWindow {
    fn sidebar(&self, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let focused = self.sidebar.is_focused(window);
        let (text, fill) = category_colors(theme, focused);
        let count = Category::ALL.len();
        let mut rows = Vec::new();
        for (index, category) in Category::ALL.into_iter().enumerate() {
            // Search results cut across categories; no sidebar row is the
            // selected page until the query is cleared.
            let selected = self.query.is_empty() && category == self.category;
            let cursor = category == self.sidebar_cursor && focused;
            let row = div()
                .id(("settings-category", index))
                .debug_selector({
                    let id = category.id();
                    move || format!("settings-category-{id}")
                })
                .role(Role::ListBoxOption)
                .aria_label(category.label())
                .aria_selected(selected)
                .aria_position_in_set(index + 1)
                .aria_size_of_set(count)
                .when(selected, |row| row.bg(fill))
                .text_color(text)
                .when(cursor, |row| row.aria_active_descendant())
                .when(cursor && window.last_input_was_keyboard(), |row| row.shadow(cursor_ring(theme)))
                .px(px(9.))
                .py(px(5.))
                .rounded_md()
                .text_size(px(13.))
                .cursor_pointer()
                .when(!selected, |row| row.hover(|style| style.bg(color(theme.role("surface")))))
                .on_click(cx.listener(move |this, _, window, cx| {
                    window.focus(&this.sidebar, cx);
                    this.select_category(category, window, cx);
                }))
                .child(category.label());
            rows.push(row.into_any_element());
        }
        div()
            .id("settings-sidebar")
            .debug_selector(|| "settings-sidebar".to_owned())
            .role(Role::ListBox)
            .aria_label("Settings categories")
            .track_focus(&self.sidebar)
            .w(px(184.))
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .p(px(8.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .border_r_1()
            .border_color(color(theme.role("border")))
            .children(rows)
            .into_any_element()
    }
}

fn section_heading(title: &'static str, theme: &Theme, top: f32) -> gpui::Stateful<gpui::Div> {
    div()
        .id(title)
        .debug_selector(move || title.to_owned())
        .pt(px(top))
        .pb(px(2.))
        .role(Role::Heading)
        .aria_label(title)
        .aria_level(2)
        .w_full()
        .max_w(px(720.))
        .text_size(px(17.))
        .font_weight(FontWeight::BOLD)
        .text_color(color(theme.role("fg")))
        .child(title)
}

/// Add one category's heading and its units to the page's flat child list. The
/// index recorded for each control is what `ScrollHandle::scroll_to_item` takes;
/// a composite unit's controls share its first row.
fn add_category(
    page: &mut Vec<gpui::AnyElement>,
    indices: &mut HashMap<ControlId, usize>,
    category: Category,
    theme: &Theme,
    rows: &mut HashMap<ControlId, gpui::AnyElement>,
) {
    page.push(section_heading(category.label(), theme, 0.0).into_any_element());
    for unit in catalog::units_in(category) {
        push_unit(page, indices, unit, theme, rows, true);
    }
}

/// Search results: category headings in sidebar order, the same unit renderers
/// as the category pages, and no category heading when it has no match.
fn add_search_results(
    page: &mut Vec<gpui::AnyElement>,
    indices: &mut HashMap<ControlId, usize>,
    hits: &[search::Hit],
    theme: &Theme,
    rows: &mut HashMap<ControlId, gpui::AnyElement>,
) {
    for category in Category::ALL {
        let units: Vec<&'static catalog::Unit> =
            hits.iter().filter(|hit| hit.category == category).filter_map(|hit| catalog::unit_for(hit.unit)).collect();
        if units.is_empty() {
            continue;
        }
        page.push(section_heading(category.label(), theme, 0.0).into_any_element());
        for unit in units {
            push_unit(page, indices, unit, theme, rows, false);
        }
    }
}

fn push_unit(
    page: &mut Vec<gpui::AnyElement>,
    indices: &mut HashMap<ControlId, usize>,
    unit: &catalog::Unit,
    theme: &Theme,
    rows: &mut HashMap<ControlId, gpui::AnyElement>,
    category_page: bool,
) {
    let Some(row) = rows.remove(&unit.id) else { return };
    // The Keyboard list takes the page's remaining height instead of a fixed one.
    let fill = category_page && unit.id == ControlId::KeyboardList;
    let index = page.len();
    for id in unit.controls {
        indices.insert(*id, index);
    }
    page.push(
        div()
            .w_full()
            .max_w(px(720.))
            .when(fill, |d| d.flex_1().min_h_0())
            // The page's only unit, so no divider under it.
            .when(!fill, |d| d.pb(px(10.)).mb(px(10.)).border_b_1().border_color(color(theme.role("border"))))
            .child(row)
            .into_any_element(),
    );
}

fn switch_row(
    this: &SettingsWindow,
    id: ControlId,
    on: bool,
    window: &Window,
    theme: &Theme,
    cx: &mut Context<SettingsWindow>,
) -> gpui::Stateful<gpui::Div> {
    let toggle = callback(cx, move |this, _, cx| this.toggle_bool(id, cx));
    controls::switch(this.meta(id, cx), on, this.style(id, window, theme), toggle)
}

fn link(
    this: &SettingsWindow,
    id: ControlId,
    label: &'static str,
    window: &Window,
    theme: &Theme,
    cx: &mut Context<SettingsWindow>,
) -> gpui::Stateful<gpui::Div> {
    let name = match id {
        ControlId::OpenSidebarToml => "sidebar.toml",
        ControlId::OpenViewsToml => "views.toml",
        ControlId::OpenExcludeToml => "exclude.toml",
        ControlId::OpenPluginsToml => "plugins.toml",
        ControlId::OpenSavedQueriesToml => "saved-queries.toml",
        ControlId::OpenKeysToml => "keys.toml",
        _ => unreachable!("{id:?} is not a config-file link"),
    };
    let activate = callback(cx, move |_, _, cx| ui::open_config_file(name, cx));
    controls::link_button(id.key(), label, id.enabled(cx), this.style(id, window, theme), activate)
}

fn callback<T: 'static>(
    cx: &mut Context<T>,
    f: impl Fn(&mut T, &mut Window, &mut Context<T>) + 'static,
) -> controls::Activate {
    let entity = cx.entity();
    Rc::new(move |window, cx| {
        entity.update(cx, |this, cx| f(this, window, cx));
    })
}

fn select_callback<T: 'static>(
    cx: &mut Context<T>,
    f: impl Fn(&mut T, usize, &mut Window, &mut Context<T>) + 'static,
) -> controls::Select {
    let entity = cx.entity();
    Rc::new(move |index, window, cx| {
        entity.update(cx, |this, cx| f(this, index, window, cx));
    })
}

fn type_ahead(state: &mut TypeAhead, keystroke: &gpui::Keystroke, labels: &[String]) -> Option<usize> {
    if keystroke.modifiers.control || keystroke.modifiers.alt || keystroke.modifiers.platform {
        return None;
    }
    let text = keystroke.key_char.as_deref()?;
    if text.is_empty() || text.chars().any(char::is_control) {
        return None;
    }
    let now = Instant::now();
    if state.at.is_none_or(|at| now.duration_since(at) > Duration::from_millis(800)) {
        state.buffer.clear();
    }
    state.buffer.push_str(text);
    state.at = Some(now);
    let needle = state.buffer.to_lowercase();
    let found = labels.iter().position(|label| label.to_lowercase().starts_with(&needle));
    if found.is_none() {
        state.buffer.clear();
    }
    found
}

fn number_text(value: f64) -> String {
    format!("{}", value.round() as i64)
}

fn stepper_range(id: ControlId) -> (f64, f64) {
    match id {
        ControlId::FontSize => (10.0, 24.0),
        _ => (600.0, 4000.0),
    }
}

fn font_index(fonts: &[String], family: &str) -> usize {
    fonts.iter().position(|name| (name == "System" && family.is_empty()) || name == family).unwrap_or(0)
}

const SORT_LABELS: [&str; 5] = ["Name", "Size", "Kind", "Modified", "Added"];

fn sort_labels() -> Vec<String> {
    SORT_LABELS.iter().map(|label| (*label).to_owned()).collect()
}

fn view_index(view: ViewMode) -> usize {
    match view {
        ViewMode::Icons => 0,
        ViewMode::List => 1,
        ViewMode::Columns => 2,
        ViewMode::Gallery => 3,
    }
}

fn sort_key_index(key: SortKey) -> usize {
    match key {
        SortKey::Name => 0,
        SortKey::Size => 1,
        SortKey::Kind => 2,
        SortKey::Modified => 3,
        SortKey::Added => 4,
    }
}

fn sort_key_at(index: usize) -> SortKey {
    match index {
        0 => SortKey::Name,
        1 => SortKey::Size,
        2 => SortKey::Kind,
        3 => SortKey::Modified,
        _ => SortKey::Added,
    }
}

/// 0 = natural (no `ascending` key), 1 = ascending, 2 = descending.
fn direction_index(ascending: Option<bool>) -> usize {
    match ascending {
        None => 0,
        Some(true) => 1,
        Some(false) => 2,
    }
}

fn keymap_index(value: KeymapLayer) -> usize {
    match value {
        KeymapLayer::Finder => 0,
        KeymapLayer::Vim => 1,
    }
}

fn enter_index(value: EnterAction) -> usize {
    match value {
        EnterAction::Rename => 0,
        EnterAction::Open => 1,
    }
}

fn hints_index(value: HintsMode) -> usize {
    match value {
        HintsMode::Auto => 0,
        HintsMode::Always => 1,
        HintsMode::Off => 2,
    }
}

fn tips_index(value: TipsMode) -> usize {
    match value {
        TipsMode::Auto => 0,
        TipsMode::Off => 1,
    }
}

fn folder_sizes_index(value: FolderSizesMode) -> usize {
    match value {
        FolderSizesMode::Off => 0,
        FolderSizesMode::Visible => 1,
    }
}

fn icon_style_index(value: crow_core::IconStyle) -> usize {
    match value {
        crow_core::IconStyle::Outline => 0,
        crow_core::IconStyle::Finder => 1,
    }
}

fn path_bar_index(value: PathBarMode) -> usize {
    match value {
        PathBarMode::Crow => 0,
        PathBarMode::Explorer => 1,
    }
}

fn sidebar_style_index(value: crow_core::SidebarStyle) -> usize {
    crow_core::SidebarStyle::ALL.iter().position(|style| *style == value).unwrap_or(0)
}

fn search_match_index(value: SearchMatch) -> usize {
    match value {
        SearchMatch::All => 0,
        SearchMatch::Names => 1,
        SearchMatch::Contents => 2,
    }
}

fn motion_index(value: MotionMode) -> usize {
    match value {
        MotionMode::Auto => 0,
        MotionMode::Off => 1,
        MotionMode::Low => 2,
        MotionMode::Full => 3,
    }
}

fn sound_index(value: SoundSet) -> usize {
    match value {
        SoundSet::Crow => 0,
        SoundSet::Finder => 1,
        SoundSet::Off => 2,
    }
}

#[allow(clippy::too_many_arguments)]
fn plugin_row(
    index: usize,
    count: usize,
    entry: &PluginEntry,
    selected: bool,
    focused: bool,
    grants_error: bool,
    theme: &Theme,
    handle: Option<PluginHandle>,
) -> gpui::Stateful<gpui::Div> {
    let muted = color(theme.role("fg-muted"));
    let error_color = color(theme.role("error"));
    let state_color = if entry.state.is_error() { error_color } else { muted };
    let actions = plugin_actions(entry, grants_error);
    let mut action_row = div().flex_none().flex().items_center().gap(px(4.));
    for action in actions {
        action_row =
            action_row.child(plugin_button(index, action, entry, action.enabled(entry), theme, handle.clone()));
    }

    let mut details = div().flex_1().min_w_0().flex().flex_col().gap(px(2.)).child(
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .min_w_0()
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(entry.title().to_owned()),
            )
            .when_some(entry.version.clone(), |line, version| {
                line.child(
                    div()
                        .flex_none()
                        .max_w(px(140.))
                        .min_w_0()
                        .truncate()
                        .text_size(px(10.))
                        .text_color(muted)
                        .child(format!("v{version}")),
                )
            })
            .child(div().flex_none().text_size(px(11.)).text_color(state_color).child(entry.state.label())),
    );
    let extensions = entry.descriptors.len();
    details = details.child(div().text_size(px(11.)).text_color(muted).child(format!(
        "{} · {extensions} {}",
        entry.permissions_summary(),
        if extensions == 1 { "extension" } else { "extensions" }
    )));
    if let Some(detail) = &entry.detail {
        details = details.child(div().text_size(px(11.)).text_color(error_color).child(detail.clone()));
    }

    let description = match &entry.detail {
        Some(detail) => format!("{detail}. {}", entry.permissions_summary()),
        None => entry.permissions_summary(),
    };
    div()
        .id(("plugin-row", index))
        .role(Role::ListBoxOption)
        .aria_label(format!("{}, {}", entry.title(), entry.state.label()))
        .aria_description(description)
        .aria_selected(selected)
        .aria_position_in_set(index + 1)
        .aria_size_of_set(count)
        .when(selected && focused, |row| row.aria_active_descendant())
        .flex_none()
        .flex()
        .items_center()
        .gap(px(10.))
        .p(px(8.))
        .rounded(px(6.))
        .overflow_hidden()
        .when(selected, |row| row.bg(if focused { active_selection(theme) } else { inactive_selection(theme) }))
        .child(details)
        .child(action_row)
}

fn plugin_button(
    index: usize,
    action: PluginAction,
    entry: &PluginEntry,
    enabled: bool,
    theme: &Theme,
    handle: Option<PluginHandle>,
) -> gpui::Stateful<gpui::Div> {
    let label = action.label().to_owned();
    let aria_label = format!("{} {}", action.label(), entry.title());
    let id = ("plugin-action", index * 8 + action.index());
    let surface = color(theme.role("surface"));
    let raised = color(theme.role("surface-raised"));
    let entry = entry.clone();
    let mut button = div()
        .id(id)
        .debug_selector(move || format!("plugin-action-{index}-{}", action.index()))
        .role(Role::Button)
        .aria_label(aria_label)
        .aria_disabled(!enabled)
        .aria_keyshortcuts(action.key())
        .px(px(8.))
        .h(px(24.))
        .flex()
        .items_center()
        .rounded(px(5.))
        .border_1()
        .border_color(color(theme.role("border")))
        .bg(surface)
        .text_color(color(theme.role("fg")))
        .text_size(px(11.))
        .when(!enabled, |button| button.opacity(0.45))
        .child(label);
    if enabled && handle.is_some() {
        button = button.cursor_pointer().hover(move |style| style.bg(raised)).on_click(move |_, _, cx| {
            send_plugin_action(handle.clone(), entry.clone(), action, cx);
        });
    }
    button
}

fn open_plugins_button(theme: &Theme) -> gpui::AnyElement {
    let surface = color(theme.role("surface"));
    let raised = color(theme.role("surface-raised"));
    div()
        .id("plugins-open-folder")
        .role(Role::Button)
        .aria_label("Open Plugins Folder")
        .aria_keyshortcuts("O")
        .px(px(8.))
        .h(px(24.))
        .flex()
        .items_center()
        .rounded(px(5.))
        .border_1()
        .border_color(color(theme.role("border")))
        .bg(surface)
        .text_color(color(theme.role("fg")))
        .text_size(px(11.))
        .cursor_pointer()
        .hover(move |style| style.bg(raised))
        .on_click(|_, _, cx| ui::open_plugins_folder(cx))
        .child("Open Plugins Folder")
        .into_any_element()
}

fn reload_all_button(theme: &Theme) -> gpui::AnyElement {
    let surface = color(theme.role("surface"));
    let raised = color(theme.role("surface-raised"));
    div()
        .id("plugins-reload-all")
        .role(Role::Button)
        .aria_label("Reload all plugins")
        .aria_keyshortcuts("L")
        .px(px(8.))
        .h(px(24.))
        .flex()
        .items_center()
        .rounded(px(5.))
        .border_1()
        .border_color(color(theme.role("border")))
        .bg(surface)
        .text_color(color(theme.role("fg")))
        .text_size(px(11.))
        .cursor_pointer()
        .hover(move |style| style.bg(raised))
        .on_click(|_, _, cx| crate::plugins::reload(cx))
        .child("Reload All")
        .into_any_element()
}

fn open_grants_button(theme: &Theme) -> gpui::AnyElement {
    let surface = color(theme.role("surface"));
    let raised = color(theme.role("surface-raised"));
    div()
        .id("plugins-open-grants")
        .role(Role::Button)
        .aria_label("Open plugins.toml")
        .aria_keyshortcuts("G")
        .px(px(8.))
        .h(px(24.))
        .flex()
        .items_center()
        .rounded(px(5.))
        .border_1()
        .border_color(color(theme.role("border")))
        .bg(surface)
        .text_color(color(theme.role("fg")))
        .text_size(px(11.))
        .cursor_pointer()
        .hover(move |style| style.bg(raised))
        .on_click(|_, _, cx| ui::open_config_file("plugins.toml", cx))
        .child("Open plugins.toml")
        .into_any_element()
}

fn send_plugin_action(handle: Option<PluginHandle>, entry: PluginEntry, action: PluginAction, cx: &mut App) {
    let Some(handle) = handle else {
        crate::operations::message("The plugin service is not running.", true, cx);
        cx.refresh_windows();
        return;
    };
    let Some(id) = entry.id.clone() else {
        crate::operations::message("This package has no valid plugin id.", true, cx);
        cx.refresh_windows();
        return;
    };
    let disable_id = id.clone();
    let result = match action {
        PluginAction::Approve => match entry.digest.clone() {
            Some(digest) => handle.approve(id, digest),
            None => Err("This package has no readable snapshot.".to_owned()),
        },
        PluginAction::Enable => handle.enable(id),
        PluginAction::Disable => handle.disable(id),
        PluginAction::Reload => handle.reload(id),
        PluginAction::Retry => handle.retry(id),
    };
    match result {
        Ok(()) if action == PluginAction::Disable => crate::plugins::suppress_disable(disable_id, cx),
        Ok(()) => {}
        Err(error) => {
            crate::operations::message(error, true, cx);
            cx.refresh_windows();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crow_theme::{ThemeInfo, Variant};
    use gpui::{TestAppContext, VisualTestContext, WindowHandle};

    use super::*;

    fn fixture(name: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::Builder::new().prefix("crow-test-").tempdir_in("/tmp").expect("create fixture");
        let path = dir.path().join(name);
        std::fs::create_dir_all(&path).expect("create fixture child");
        (dir, path)
    }

    fn setup(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let config = crate::ui::init_for_tests(cx);
            let trash = config.parent().expect("test root").join("trash");
            std::fs::create_dir_all(&trash).ok();
            crate::operations::init(cx, std::sync::Arc::new(crate::operations::FolderTrash(trash)));
            crate::keymap::install(cx);
            crate::menus::init(cx);
        });
    }

    fn open_settings(cx: &mut TestAppContext) -> (Entity<SettingsWindow>, VisualTestContext) {
        open_settings_after(cx, |_| {})
    }

    fn open_settings_after(
        cx: &mut TestAppContext,
        before: impl FnOnce(&mut TestAppContext),
    ) -> (Entity<SettingsWindow>, VisualTestContext) {
        setup(cx);
        before(cx);
        let window: WindowHandle<SettingsWindow> = cx.update(|cx| {
            let bounds = Bounds::centered(None, size(px(560.), px(760.)), cx);
            let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() };
            cx.open_window(options, |window, cx| cx.new(|cx| SettingsWindow::new(window, cx)))
                .expect("open settings window")
        });
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        let view = window.root(&mut vcx).expect("root view");
        vcx.run_until_parked();
        cx.run_until_parked();
        let focus = view.read_with(&vcx, |view, cx| view.first_focus(cx));
        vcx.update(|window, cx| window.focus(&focus, cx));
        draw(&mut vcx);
        (view, vcx)
    }

    fn draw(vcx: &mut VisualTestContext) {
        vcx.update(|window, cx| {
            let clear = window.draw(cx);
            clear.clear(cx);
        });
        vcx.cx.run_until_parked();
        vcx.run_until_parked();
    }

    fn show_category(view: &Entity<SettingsWindow>, vcx: &mut VisualTestContext, category: Category) {
        vcx.update(|window, cx| view.update(cx, |view, cx| view.select_category(category, window, cx)));
        draw(vcx);
    }

    fn focus_control(view: &Entity<SettingsWindow>, vcx: &mut VisualTestContext, target: ControlId) {
        if let Some(category) = catalog::category_of(target) {
            show_category(view, vcx, category);
        }
        for _ in 0..ControlId::ALL.len() * 2 {
            let focused = vcx.update(|window, cx| view.read(cx).focused_control(window));
            if focused == Some(target) {
                return;
            }
            vcx.simulate_keystrokes("tab");
        }
        panic!("never focused {target:?}");
    }

    fn state(view: &Entity<SettingsWindow>, vcx: &VisualTestContext, id: ControlId) -> ControlState {
        view.read_with(vcx, |view, cx| view.control_state(id, cx))
    }

    fn stepper_text(view: &Entity<SettingsWindow>, vcx: &VisualTestContext, id: ControlId) -> String {
        view.read_with(vcx, |view, cx| view.input(id).read(cx).text().to_owned())
    }

    fn settings_text(vcx: &VisualTestContext) -> String {
        let dir = vcx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
        std::fs::read_to_string(dir.join("settings.toml")).expect("settings file")
    }

    fn wait_for(vcx: &VisualTestContext, needle: &str) {
        for _ in 0..100 {
            vcx.run_until_parked();
            if settings_text(vcx).contains(needle) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("never wrote {needle:?}: {}", settings_text(vcx));
    }

    #[gpui::test]
    fn cmd_comma_opens_one_settings_window(cx: &mut TestAppContext) {
        setup(cx);
        let (_root, root) = fixture("open");
        let browser = cx.update(|cx| crate::window::open(root.clone(), false, None, cx)).expect("browser window");
        let mut vcx = VisualTestContext::from_window(browser.into(), cx);
        vcx.run_until_parked();

        vcx.simulate_keystrokes("cmd-,");
        cx.run_until_parked();
        let settings: Vec<_> =
            cx.update(|cx| cx.windows().into_iter().filter_map(|handle| handle.downcast::<SettingsWindow>()).collect());
        assert_eq!(settings.len(), 1, "⌘, opens one Settings window");
        assert!(
            settings[0]
                .update(cx, |view, window, cx| view.search_handle(cx).is_focused(window))
                .expect("settings window"),
            "the search field is focused"
        );

        vcx.simulate_keystrokes("cmd-,");
        cx.run_until_parked();
        let settings: Vec<_> =
            cx.update(|cx| cx.windows().into_iter().filter_map(|handle| handle.downcast::<SettingsWindow>()).collect());
        assert_eq!(settings.len(), 1, "a second ⌘, focuses the one window");
        assert_eq!(cx.update(|cx| cx.windows().len()), 2, "browser + settings");
    }

    #[gpui::test]
    fn cmd_w_closes_the_settings_window(cx: &mut TestAppContext) {
        let (_view, mut vcx) = open_settings(cx);
        vcx.simulate_keystrokes("cmd-w");
        cx.run_until_parked();
        let count = cx.update(|cx| {
            cx.windows().into_iter().filter(|handle| handle.downcast::<SettingsWindow>().is_some()).count()
        });
        assert_eq!(count, 0);
    }

    #[gpui::test]
    fn escape_closes_the_settings_window(cx: &mut TestAppContext) {
        let (_view, mut vcx) = open_settings(cx);
        vcx.simulate_keystrokes("escape");
        cx.run_until_parked();
        let count = cx.update(|cx| {
            cx.windows().into_iter().filter(|handle| handle.downcast::<SettingsWindow>().is_some()).count()
        });
        assert_eq!(count, 0);
    }

    fn settings_window_count(cx: &mut TestAppContext) -> usize {
        cx.update(|cx| cx.windows().into_iter().filter(|handle| handle.downcast::<SettingsWindow>().is_some()).count())
    }

    #[gpui::test]
    fn cmd_w_leaves_settings_open_behind_the_saved_query_sheet(cx: &mut TestAppContext) {
        let (_dir, path) = fixture("saved-query-cmd-w");
        let (view, mut vcx) = open_settings(cx);
        vcx.update(|_, cx| cx.global_mut::<Ui>().settings_folder = Some(path.clone()));
        show_category(&view, &mut vcx, Category::Search);
        focus_control(&view, &mut vcx, ControlId::NewSavedQuery);
        vcx.simulate_keystrokes("enter");
        assert!(view.read_with(&vcx, |view, _| view.saved_query_form.is_some()), "the sheet is open");
        vcx.simulate_keystrokes("cmd-w");
        cx.run_until_parked();
        assert_eq!(settings_window_count(cx), 1, "the window stays behind its sheet");
        vcx.simulate_keystrokes("escape");
        assert!(view.read_with(&vcx, |view, _| view.saved_query_form.is_none()), "Escape closes the sheet first");
        assert_eq!(settings_window_count(cx), 1);
    }

    #[gpui::test]
    fn escape_in_a_stepper_clears_the_search_before_closing(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        draw(&mut vcx);
        vcx.simulate_input("font size");
        draw(&mut vcx);
        // Tab through the results without leaving the search.
        for _ in 0..8 {
            if vcx.update(|window, cx| view.read(cx).focused_control(window)) == Some(ControlId::FontSize) {
                break;
            }
            vcx.simulate_keystrokes("tab");
        }
        assert_eq!(vcx.update(|window, cx| view.read(cx).focused_control(window)), Some(ControlId::FontSize));
        assert_eq!(view.read_with(&vcx, |view, _| view.query.clone()), "font size");
        vcx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert_eq!(settings_window_count(cx), 1, "the first Escape peels the search, not the window");
        assert_eq!(view.read_with(&vcx, |view, _| view.query.clone()), "");
        focus_control(&view, &mut vcx, ControlId::FontSize);
        vcx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert_eq!(settings_window_count(cx), 0, "with nothing left to peel, Escape closes");
    }

    #[gpui::test]
    fn switch_toggles_with_space(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        focus_control(&view, &mut vcx, ControlId::ShowHidden);
        assert_eq!(state(&view, &vcx, ControlId::ShowHidden).toggled, Some(true));
        vcx.simulate_keystrokes("space");
        assert_eq!(state(&view, &vcx, ControlId::ShowHidden).toggled, Some(false));
        assert!(!vcx.read_global::<Ui, _>(|ui, _| ui.settings.show_hidden_files));
        wait_for(&vcx, "show_hidden_files = false");
    }

    #[gpui::test]
    fn search_controls_write_their_lines(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        focus_control(&view, &mut vcx, ControlId::SearchHidden);
        assert_eq!(state(&view, &vcx, ControlId::SearchHidden).toggled, Some(true), "the default includes hidden");
        vcx.simulate_keystrokes("space");
        assert_eq!(state(&view, &vcx, ControlId::SearchHidden).toggled, Some(false));
        assert!(!vcx.read_global::<Ui, _>(|ui, _| ui.settings.search_includes_hidden));
        wait_for(&vcx, "search_includes_hidden = false");

        focus_control(&view, &mut vcx, ControlId::SearchMatch);
        assert_eq!(state(&view, &vcx, ControlId::SearchMatch).selected, Some(0), "All is the default");
        vcx.simulate_keystrokes("right");
        assert_eq!(state(&view, &vcx, ControlId::SearchMatch).selected, Some(1));
        assert_eq!(vcx.read_global::<Ui, _>(|ui, _| ui.settings.search_match), SearchMatch::Names);
        wait_for(&vcx, "search_match = \"names\"");
        vcx.simulate_keystrokes("right");
        assert_eq!(state(&view, &vcx, ControlId::SearchMatch).selected, Some(2));
        assert_eq!(vcx.read_global::<Ui, _>(|ui, _| ui.settings.search_match), SearchMatch::Contents);
        wait_for(&vcx, "search_match = \"contents\"");
    }

    /// Each category mounts its own units in catalog order, and a control from
    /// another category is not on the page or in the tab order.
    #[gpui::test]
    fn controls_are_tabbed_in_page_order(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        for category in Category::ALL {
            show_category(&view, &mut vcx, category);
            let indices = view.read_with(&vcx, |view, _| {
                catalog::units_in(category)
                    .flat_map(|unit| unit.controls.iter().copied())
                    .map(|id| (id, *view.control_items.get(&id).expect("every visible control is on the page")))
                    .collect::<Vec<_>>()
            });
            assert!(!indices.is_empty(), "{category:?} has controls");
            for pair in indices.windows(2) {
                assert!(
                    pair[0].1 <= pair[1].1,
                    "{:?} (row {}) is tabbed before {:?} (row {})",
                    pair[0].0,
                    pair[0].1,
                    pair[1].0,
                    pair[1].1
                );
            }
            let visible = view.read_with(&vcx, |view, cx| view.visible_controls(cx));
            for id in ControlId::ALL {
                let expected = view.read_with(&vcx, |view, cx| {
                    catalog::category_of(id) == Some(category) && view.preview_control_enabled(id, cx)
                });
                assert_eq!(visible.contains(&id), expected, "{id:?} is mounted only in its category and current scope");
            }
        }
    }

    #[gpui::test]
    fn config_files_reach_saved_queries_by_tab_and_by_search(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        show_category(&view, &mut vcx, Category::ConfigFiles);
        let visible = view.read_with(&vcx, |view, cx| view.visible_controls(cx));
        assert!(visible.contains(&ControlId::OpenSavedQueriesToml), "saved-queries.toml is in the tab walk");
        focus_control(&view, &mut vcx, ControlId::OpenSavedQueriesToml);
        assert_eq!(
            vcx.update(|window, cx| view.read(cx).focused_control(window)),
            Some(ControlId::OpenSavedQueriesToml),
            "its link takes focus"
        );

        vcx.simulate_input("saved-queries.toml");
        draw(&mut vcx);
        let results = view.read_with(&vcx, |view, cx| view.visible_controls(cx));
        assert!(results.contains(&ControlId::OpenSavedQueriesToml), "search finds the Config Files link: {results:?}");
    }

    #[test]
    fn selected_settings_category_has_readable_composited_text_contrast() {
        fn luminance(color: gpui::Rgba) -> f32 {
            let linear = |c: f32| {
                if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
        }
        let schemes = tempfile::tempdir().unwrap();
        for id in ["finder-light", "finder-dark", "catppuccin-mocha"] {
            let theme = crow_theme::load_theme(schemes.path(), id).unwrap();
            for focused in [false, true] {
                let (text, fill) = category_colors(&theme, focused);
                let text = luminance(text);
                let base = luminance(color(theme.role("bg")).blend(fill));
                let contrast = (text.max(base) + 0.05) / (text.min(base) + 0.05);
                assert!(contrast >= 4.5, "{id}, focused={focused}: {contrast:.2}:1");
            }
        }
    }

    #[gpui::test]
    fn sidebar_arrows_move_the_cursor_and_return_chooses(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        draw(&mut vcx);
        assert!(vcx.update(|window, cx| view.read(cx).search_handle(cx).is_focused(window)));
        vcx.simulate_keystrokes("tab");
        assert!(vcx.update(|window, cx| view.read(cx).sidebar.is_focused(window)));
        vcx.simulate_keystrokes("down");
        assert_eq!(view.read_with(&vcx, |view, _| view.sidebar_cursor), Category::Behaviour);
        assert_eq!(
            view.read_with(&vcx, |view, _| view.category),
            Category::Appearance,
            "moving the cursor does not open it"
        );
        vcx.simulate_keystrokes("enter");
        assert_eq!(view.read_with(&vcx, |view, _| view.category), Category::Behaviour);
        assert!(vcx.update(|window, cx| view.read(cx).sidebar.is_focused(window)));

        let state = vcx.read_global::<Ui, _>(|ui, _| ui.state_dir.clone());
        for _ in 0..100 {
            if std::fs::read_to_string(state.join("settings-category")).is_ok_and(|text| text.trim() == "behaviour") {
                break;
            }
            vcx.run_until_parked();
            std::thread::sleep(Duration::from_millis(5));
        }
        let saved = std::fs::read_to_string(state.join("settings-category")).expect("category remembered");
        assert_eq!(saved.trim(), "behaviour");
        let remembered = vcx.read_global::<Ui, _>(|ui, _| ui.settings_category.clone());
        assert_eq!(remembered.as_deref(), Some("behaviour"));
    }

    #[gpui::test]
    fn saved_category_survives_a_simulated_relaunch_and_unknown_ids_fall_back(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings_after(cx, |cx| {
            let state = cx.read_global::<Ui, _>(|ui, _| ui.state_dir.clone());
            std::fs::write(state.join("settings-category"), "views-preview\n").expect("state");
            cx.update(ui::reload_state_for_tests);
        });
        assert_eq!(view.read_with(&vcx, |view, _| view.category), Category::ViewsPreview);
        vcx.simulate_keystrokes("escape");
        vcx.cx.run_until_parked();

        let state = vcx.read_global::<Ui, _>(|ui, _| ui.state_dir.clone());
        std::fs::write(state.join("settings-category"), "nonsense\n").expect("state");
        vcx.cx.update(ui::reload_state_for_tests);
        vcx.cx.update(crate::settings_window::open);
        vcx.cx.run_until_parked();
        let window = vcx
            .cx
            .update(|cx| cx.windows().into_iter().find_map(|handle| handle.downcast::<SettingsWindow>()))
            .expect("reopened");
        let category = window.update(&mut vcx.cx, |view, _, _| view.category).expect("settings");
        assert_eq!(category, Category::Appearance, "an unknown id falls back to the first category");
    }

    #[gpui::test]
    fn search_finds_and_edits_a_control_in_place(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        draw(&mut vcx);
        vcx.simulate_input("vim");
        draw(&mut vcx);
        assert_eq!(view.read_with(&vcx, |view, cx| view.visible_controls(cx)), vec![ControlId::Keymap]);
        assert!(vcx.debug_bounds("settings-keymap-control").is_some(), "the category row is rendered in results");
        assert!(vcx.debug_bounds("settings-theme-control").is_none(), "a nonmatching category control is not mounted");
        assert!(vcx.debug_bounds("Behavior").is_some(), "its category heading is rendered");

        vcx.simulate_keystrokes("tab");
        assert_eq!(vcx.update(|window, cx| view.read(cx).focused_control(window)), Some(ControlId::Keymap));
        vcx.simulate_keystrokes("right");
        assert_eq!(vcx.read_global::<Ui, _>(|ui, _| ui.settings.keymap), KeymapLayer::Vim);
        wait_for(&vcx, "keymap = \"vim\"");

        // Externally editing a value does not disturb the query or the results.
        vcx.simulate_keystrokes("left");
        wait_for(&vcx, "keymap = \"finder\"");
        let dir = vcx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
        std::fs::write(dir.join("settings.toml"), "keymap = \"vim\"\nother_future = 1\n").expect("external edit");
        vcx.update(|_, cx| ui::reload_config(cx));
        vcx.run_until_parked();
        draw(&mut vcx);
        assert_eq!(view.read_with(&vcx, |view, _| view.query.clone()), "vim");
        assert_eq!(state(&view, &vcx, ControlId::Keymap).selected, Some(1), "the result follows the file");
    }

    #[gpui::test]
    fn search_no_match_clears_with_a_button_or_escape(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        draw(&mut vcx);
        vcx.simulate_input("zzzz-no-setting");
        draw(&mut vcx);
        assert!(vcx.debug_bounds("settings-no-match").is_some());
        assert_eq!(
            view.read_with(&vcx, |view, cx| view.focus_targets(cx)),
            vec![FocusTarget::Search, FocusTarget::ClearSearch, FocusTarget::Sidebar],
            "no results leave a clear path and the sidebar reachable"
        );
        vcx.simulate_keystrokes("tab");
        vcx.simulate_keystrokes("enter");
        draw(&mut vcx);
        assert_eq!(view.read_with(&vcx, |view, _| view.query.clone()), "");
        assert!(vcx.debug_bounds("settings-no-match").is_none());

        vcx.simulate_input("nothing-matches");
        draw(&mut vcx);
        vcx.simulate_keystrokes("escape");
        draw(&mut vcx);
        assert_eq!(view.read_with(&vcx, |view, _| view.query.clone()), "", "Esc clears the query before closing");
        vcx.simulate_keystrokes("escape");
        vcx.cx.run_until_parked();
        let open =
            vcx.cx.update(|cx| cx.windows().into_iter().any(|handle| handle.downcast::<SettingsWindow>().is_some()));
        assert!(!open, "the second Esc closes the window");
    }

    #[gpui::test]
    fn search_renders_one_copy_and_keeps_hidden_controls_out_of_tab_order(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        draw(&mut vcx);
        vcx.simulate_input("sound");
        draw(&mut vcx);
        let selectors = vcx.update(|window, _| {
            window
                .debug_interactive_elements()
                .expect("recorded")
                .iter()
                .filter_map(|element| element.selector.clone())
                .collect::<Vec<_>>()
        });
        assert_eq!(selectors.iter().filter(|selector| *selector == "settings-sound-control").count(), 1);
        assert_eq!(selectors.iter().filter(|selector| *selector == "settings-play-sound").count(), 1);
        assert_eq!(selectors.iter().filter(|selector| *selector == "settings-theme-control").count(), 0);
        assert_eq!(
            view.read_with(&vcx, |view, cx| view.visible_controls(cx)),
            vec![ControlId::Sound, ControlId::PlaySound],
            "a composite row is one search result with both controls"
        );
        vcx.simulate_keystrokes("cmd-f");
        assert!(vcx.update(|window, cx| view.read(cx).search_handle(cx).is_focused(window)));
    }

    fn views_text(vcx: &VisualTestContext) -> String {
        let dir = vcx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
        std::fs::read_to_string(dir.join("views.toml")).expect("views file")
    }

    fn wait_for_views(vcx: &VisualTestContext, needle: &str) {
        for _ in 0..100 {
            vcx.run_until_parked();
            if views_text(vcx).contains(needle) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("never wrote {needle:?}: {}", views_text(vcx));
    }

    #[gpui::test]
    fn defaults_controls_write_only_their_keys_and_follow_external_edits(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings_after(cx, |cx| {
            let dir = cx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
            std::fs::write(dir.join("views.toml"), "# mine\n[defaults]\nview = \"list\"\nascending = true\n").unwrap();
            cx.update(ui::reload_config);
        });
        draw(&mut vcx);

        focus_control(&view, &mut vcx, ControlId::DefaultView);
        vcx.simulate_keystrokes("right");
        wait_for_views(&vcx, "view = \"columns\"");
        assert_eq!(state(&view, &vcx, ControlId::DefaultView).selected, Some(2));
        let text = views_text(&vcx);
        assert!(text.starts_with("# mine\n"), "comments survive: {text}");

        // Choosing a sort key leaves the saved explicit direction alone.
        focus_control(&view, &mut vcx, ControlId::DefaultSort);
        vcx.simulate_keystrokes("space");
        vcx.simulate_keystrokes("down");
        vcx.simulate_keystrokes("enter");
        wait_for_views(&vcx, "sort = \"size\"");
        assert!(views_text(&vcx).contains("ascending = true"), "the explicit direction stays: {}", views_text(&vcx));
        assert_eq!(state(&view, &vcx, ControlId::DefaultSort).selected, Some(1));
        assert_eq!(state(&view, &vcx, ControlId::DefaultDirection).selected, Some(1));

        // Natural removes the key, so the natural direction applies again.
        focus_control(&view, &mut vcx, ControlId::DefaultDirection);
        vcx.simulate_keystrokes("left");
        wait_for_views(&vcx, "sort = \"size\"");
        for _ in 0..100 {
            vcx.run_until_parked();
            if !views_text(&vcx).contains("ascending") {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!views_text(&vcx).contains("ascending"), "natural is the absence of the key: {}", views_text(&vcx));
        assert_eq!(state(&view, &vcx, ControlId::DefaultDirection).selected, Some(0));

        focus_control(&view, &mut vcx, ControlId::DefaultFoldersFirst);
        vcx.simulate_keystrokes("space");
        for _ in 0..100 {
            vcx.run_until_parked();
            if views_text(&vcx).contains("folders_first = false") {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(views_text(&vcx).contains("folders_first = false"), "{}", views_text(&vcx));
        assert_eq!(state(&view, &vcx, ControlId::DefaultFoldersFirst).toggled, Some(false));

        // An outside edit updates every control without a relaunch.
        let dir = vcx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
        std::fs::write(
            dir.join("views.toml"),
            "# changed\n[defaults]\nview = \"list\"\nsort = \"modified\"\nascending = false\nfolders_first = true\n",
        )
        .unwrap();
        vcx.update(|_, cx| ui::reload_config(cx));
        vcx.run_until_parked();
        draw(&mut vcx);
        assert_eq!(state(&view, &vcx, ControlId::DefaultView).selected, Some(1));
        assert_eq!(state(&view, &vcx, ControlId::DefaultSort).selected, Some(3));
        assert_eq!(state(&view, &vcx, ControlId::DefaultDirection).selected, Some(2));
        assert_eq!(state(&view, &vcx, ControlId::DefaultFoldersFirst).toggled, Some(true));
    }

    #[gpui::test]
    fn icon_pack_list_is_stable_and_commits_the_drawn_entry(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings_after(cx, |cx| {
            let config = cx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
            for (id, name) in [("alpha", "Same"), ("beta", "Same")] {
                let dir = config.join("icon-packs").join(id);
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(
                    dir.join("pack.toml"),
                    format!("api = 1\nname = \"{name}\"\n[generic]\nfile = {{ icon = \"file.svg\" }}\n"),
                )
                .unwrap();
                std::fs::write(
                    dir.join("file.svg"),
                    r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M4 4h16v16H4z"/></svg>"#,
                )
                .unwrap();
            }
            cx.update(crate::icon_packs::reload);
        });
        for _ in 0..200 {
            vcx.run_until_parked();
            let count = vcx.read_global::<crate::icon_packs::IconPacks, _>(|packs, _| packs.snapshot().choices().len());
            if count == 2 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        draw(&mut vcx);
        let labels = view.read_with(&vcx, |view, _| view.icon_pack_labels.clone());
        assert_eq!(
            labels,
            ["Same (alpha)", "Same (beta)"],
            "only packs are offered: the default icons are a separate setting"
        );

        focus_control(&view, &mut vcx, ControlId::IconPack);
        vcx.simulate_keystrokes("space");
        vcx.simulate_keystrokes("enter");
        wait_for(&vcx, "icon_pack = \"alpha\"");
        let text = settings_text(&vcx);
        assert_eq!(text.matches("icon_pack").count(), 1, "only the icon_pack line changes: {text}");
        assert_eq!(vcx.read_global::<Ui, _>(|ui, _| ui.settings.icon_pack.clone()), "alpha");

        // Several packs can be chosen: the menu stays open, ticks both, and the button names both.
        assert!(view.read_with(&vcx, |view, _| view.menu == Some(ControlId::IconPack)), "the menu stays open");
        vcx.simulate_keystrokes("down");
        vcx.simulate_keystrokes("enter");
        wait_for(&vcx, "icon_pack = \"alpha,beta\"");
        draw(&mut vcx);
        let (labels, value) =
            view.read_with(&vcx, |view, _| (view.icon_pack_labels.clone(), view.icon_pack_value.clone()));
        assert!(labels.contains(&"✓ Same (alpha)".to_owned()), "{labels:?}");
        assert!(labels.contains(&"✓ Same (beta)".to_owned()), "{labels:?}");
        assert_eq!(value, "Same + Same");
        assert!(view.read_with(&vcx, |view, _| view.menu == Some(ControlId::IconPack)), "still open");

        // Picking a ticked pack again takes it out.
        vcx.simulate_keystrokes("enter");
        wait_for(&vcx, "icon_pack = \"alpha\"");
    }

    #[gpui::test]
    fn the_default_icons_are_a_two_way_choice_that_saves_one_key(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings_after(cx, |_| {});
        draw(&mut vcx);
        focus_control(&view, &mut vcx, ControlId::IconStyle);
        assert_eq!(state(&view, &vcx, ControlId::IconStyle).selected, Some(0), "Outline is the default");
        vcx.simulate_keystrokes("right");
        wait_for(&vcx, "icon_style = \"finder\"");
        assert_eq!(vcx.read_global::<Ui, _>(|ui, _| ui.settings.icon_style), crow_core::IconStyle::Finder);
        assert_eq!(state(&view, &vcx, ControlId::IconStyle).selected, Some(1));
    }

    #[gpui::test]
    fn a_missing_icon_pack_stays_visible_and_native_falls_back(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings_after(cx, |cx| {
            let dir = cx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
            std::fs::write(dir.join("settings.toml"), "icon_pack = \"ghost.pack\"\n").unwrap();
            cx.update(ui::reload_config);
        });
        draw(&mut vcx);
        let labels = view.read_with(&vcx, |view, _| view.icon_pack_labels.clone());
        assert!(labels.contains(&"✓ ghost.pack (not installed)".to_owned()), "{labels:?}");
        focus_control(&view, &mut vcx, ControlId::IconPack);
        assert_eq!(state(&view, &vcx, ControlId::IconPack).selected, Some(0), "the saved id stays selected");
        vcx.simulate_keystrokes("space");
        vcx.simulate_keystrokes("enter");
        wait_for(&vcx, "icon_pack = \"native\"");
        assert!(vcx.read_global::<Ui, _>(|ui, _| ui.settings.icon_pack == "native"));
    }

    fn wait_for_preview<F: Fn(&Ui) -> bool>(vcx: &VisualTestContext, check: F) {
        for _ in 0..200 {
            vcx.run_until_parked();
            if vcx.read_global::<Ui, _>(|ui, _| check(ui)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("preview state never reached the expected value");
    }

    #[gpui::test]
    fn search_settings_create_and_list_saved_queries(cx: &mut TestAppContext) {
        let (_dir, path) = fixture("saved-query-settings");
        let (view, mut vcx) = open_settings(cx);
        vcx.update(|_, cx| cx.global_mut::<Ui>().settings_folder = Some(path.clone()));
        show_category(&view, &mut vcx, Category::Search);

        focus_control(&view, &mut vcx, ControlId::NewSavedQuery);
        vcx.simulate_keystrokes("enter");
        vcx.simulate_input("Recent PDFs");
        // Name, three scope options, root, text, kind, days.
        vcx.simulate_keystrokes("tab tab tab tab tab tab");
        vcx.simulate_input("pdf");
        vcx.simulate_keystrokes("tab");
        vcx.simulate_input("30");
        vcx.simulate_keystrokes("enter");
        wait_for_preview(&vcx, |ui| ui.saved_queries.queries.len() == 1);
        let (name, kind, days) = vcx.read_global::<Ui, _>(|ui, _| {
            let query = &ui.saved_queries.queries[0];
            (query.name.clone(), query.kind.clone(), query.modified_within_days)
        });
        assert_eq!(name, "Recent PDFs");
        assert_eq!(kind, Some(crow_core::SavedKind::Pdf));
        assert_eq!(days, Some(30));
        draw(&mut vcx);
        assert!(vcx.debug_bounds("settings-saved-query-recent-pdfs").is_some());

        focus_control(&view, &mut vcx, ControlId::SavedQueryList);
        vcx.simulate_keystrokes("enter");
        assert!(view.read_with(&vcx, |view, _| view.saved_query_form.is_some()));
        vcx.simulate_keystrokes("escape");
        assert!(view.read_with(&vcx, |view, _| view.saved_query_form.is_none()));
    }

    #[gpui::test]
    fn category_changes_fade_the_content_and_motion_off_skips_it(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        vcx.update(|_, cx| ui::set_setting(SettingChange::Motion(MotionMode::Full), cx));
        // Asked at a moment before the change: a slow runner can spend longer than PANEL_TAB inside
        // `show_category`, and the check must not depend on that.
        let before = Instant::now();
        show_category(&view, &mut vcx, Category::Behaviour);
        assert!(
            view.read_with(&vcx, |view, _| view.content_motion_for_tests(Effective::Full, before)).is_some(),
            "a category change starts a content transition"
        );
        let later = Instant::now() + Duration::from_millis(300);
        assert!(
            view.read_with(&vcx, |view, _| view.content_motion_for_tests(Effective::Full, later)).is_none(),
            "the transition ends after PANEL_TAB"
        );

        vcx.update(|_, cx| ui::set_setting(SettingChange::Motion(MotionMode::Off), cx));
        show_category(&view, &mut vcx, Category::Search);
        assert!(
            view.read_with(&vcx, |view, _| view.content_motion_for_tests(Effective::Off, Instant::now())).is_none(),
            "motion off switches content in one frame"
        );
    }

    #[gpui::test]
    fn independent_file_errors_disable_only_their_controls(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings_after(cx, |cx| {
            let config = cx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
            std::fs::write(config.join("views.toml"), "[defaults]\nsort = \"colour\"\n").unwrap();
            std::fs::write(config.join("sidebar.toml"), "[group\n").unwrap();
            std::fs::write(config.join("plugins.toml"), "[plugin.\"bad\"]\nenabled = \"yes\"\n").unwrap();
            cx.update(|cx| {
                ui::reload_config(cx);
                crate::plugins::start_for_tests(cx, config);
            });
        });
        for _ in 0..400 {
            vcx.update(|_, cx| crate::plugins::poll(cx));
            vcx.run_until_parked();
            if grants_error(&vcx) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(grants_error(&vcx), "the plugin grants file is broken");
        draw(&mut vcx);
        let issues = view.read_with(&vcx, |view, cx| view.issues(cx));
        let files: Vec<_> = issues.iter().map(|issue| issue.file).collect();
        assert_eq!(files, ["views.toml", "sidebar.toml", "plugins.toml"], "{issues:?}");
        assert_eq!(issues[0].line, Some(2), "the line is kept for the recovery row");

        assert!(!state(&view, &vcx, ControlId::DefaultView).enabled, "views controls are disabled");
        assert!(state(&view, &vcx, ControlId::SearchHidden).enabled, "an unrelated settings file stays usable");
        assert!(state(&view, &vcx, ControlId::OpenViewsToml).enabled, "the file links stay usable");
        let targets = view.read_with(&vcx, |view, cx| view.focus_targets(cx));
        assert!(targets.contains(&FocusTarget::Error("views.toml")));
        assert!(targets.contains(&FocusTarget::Error("sidebar.toml")));
        assert!(targets.contains(&FocusTarget::Error("plugins.toml")));

        // Fixing one file clears only its error and re-enables only its controls.
        let config = vcx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
        std::fs::write(config.join("views.toml"), "[defaults]\nsort = \"name\"\n").unwrap();
        vcx.update(|_, cx| ui::reload_config(cx));
        vcx.run_until_parked();
        draw(&mut vcx);
        let files: Vec<_> =
            view.read_with(&vcx, |view, cx| view.issues(cx)).into_iter().map(|issue| issue.file).collect();
        assert_eq!(files, ["sidebar.toml", "plugins.toml"]);
        assert!(state(&view, &vcx, ControlId::DefaultView).enabled);
        assert!(state(&view, &vcx, ControlId::PreviewScope).enabled);
        assert!(state(&view, &vcx, ControlId::Theme).enabled);

        std::fs::write(config.join("sidebar.toml"), "[group]\nname = \"Favorites\"\nitems = []\n").unwrap();
        vcx.update(|_, cx| ui::reload_config(cx));
        vcx.run_until_parked();
        draw(&mut vcx);
        assert!(state(&view, &vcx, ControlId::PreviewScope).enabled);
    }

    #[gpui::test]
    fn plugins_and_pack_errors_reach_settings_and_the_bar_and_clear_on_recovery(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings_after(cx, |cx| {
            let config = cx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
            std::fs::write(config.join("plugins.toml"), "# note\n[plugin.\"example.x\"]\nenabled = \"yes\"\n").unwrap();
            let pack = config.join("icon-packs/broken");
            std::fs::create_dir_all(&pack).unwrap();
            std::fs::write(pack.join("pack.toml"), "api = 1\nnonsense = [\n").unwrap();
            cx.update(|cx| {
                crate::plugins::start_for_tests(cx, config);
                crate::icon_packs::reload(cx);
            });
        });
        let mut ready = false;
        for _ in 0..400 {
            vcx.update(|_, cx| crate::plugins::poll(cx));
            vcx.run_until_parked();
            ready = grants_error(&vcx)
                && vcx.read_global::<crate::icon_packs::IconPacks, _>(|packs, _| packs.error().is_some());
            if ready {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(ready, "both files are reported broken");
        draw(&mut vcx);
        let issues = view.read_with(&vcx, |view, cx| view.issues(cx));
        let plugins = issues.iter().find(|issue| issue.file == "plugins.toml").expect("plugins.toml issue");
        assert_eq!(plugins.line, Some(3), "the grants error carries its line: {plugins:?}");
        assert!(plugins.message.starts_with("plugins.toml line 3:"), "the config folder is stripped: {plugins:?}");
        let pack = issues.iter().find(|issue| issue.file == "pack.toml").expect("pack.toml issue");
        assert!(pack.path.ends_with("icon-packs/broken/pack.toml"), "{pack:?}");

        // The status bar's pill reads the same list and points at a file.
        let (_, file) = vcx.update(|_, cx| ui::config_error_file(cx)).expect("a bar error");
        assert!(file.ends_with("plugins.toml"), "{file:?}");

        // Fixing both clears both.
        let config = vcx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
        std::fs::write(config.join("plugins.toml"), "").unwrap();
        std::fs::remove_dir_all(config.join("icon-packs/broken")).unwrap();
        vcx.update(|_, cx| {
            ui::reload_config(cx);
            crate::icon_packs::reload(cx);
        });
        for _ in 0..400 {
            vcx.update(|_, cx| crate::plugins::poll(cx));
            vcx.run_until_parked();
            if vcx.update(|_, cx| ui::config_issues(cx)).is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(vcx.update(|_, cx| ui::config_issues(cx)).is_empty(), "recovery clears the bar and Settings");
        assert!(vcx.update(|_, cx| ui::config_error_file(cx)).is_none());
    }

    #[gpui::test]
    fn escape_closes_the_menu_then_clears_search_then_closes(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        draw(&mut vcx);
        vcx.simulate_input("theme");
        draw(&mut vcx);
        vcx.simulate_keystrokes("tab");
        assert_eq!(vcx.update(|window, cx| view.read(cx).focused_control(window)), Some(ControlId::Theme));
        vcx.simulate_keystrokes("space");
        draw(&mut vcx);
        assert_eq!(view.read_with(&vcx, |view, _| view.menu), Some(ControlId::Theme));

        vcx.simulate_keystrokes("escape");
        draw(&mut vcx);
        assert_eq!(view.read_with(&vcx, |view, _| view.menu), None, "Esc closes the menu first");
        assert_eq!(view.read_with(&vcx, |view, _| view.query.clone()), "theme", "the query stays");

        vcx.simulate_keystrokes("escape");
        draw(&mut vcx);
        assert_eq!(view.read_with(&vcx, |view, _| view.query.clone()), "", "Esc clears the query second");
        assert!(vcx.update(|window, cx| view.read(cx).search_handle(cx).is_focused(window)));

        vcx.simulate_keystrokes("escape");
        vcx.cx.run_until_parked();
        let open =
            vcx.cx.update(|cx| cx.windows().into_iter().any(|handle| handle.downcast::<SettingsWindow>().is_some()));
        assert!(!open, "Esc closes Settings when neither applies");
    }

    #[gpui::test]
    fn preview_defaults_moves_removes_clears_and_shows_native_only(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings_after(cx, |cx| {
            let dir = cx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
            std::fs::write(
                dir.join("views.toml"),
                "# mine\n[defaults]\npreview_providers = [\n  \"example.one.preview\", # one\n  \"example.two.preview\",\n]\n",
            )
            .unwrap();
            cx.update(ui::reload_config);
        });
        draw(&mut vcx);
        focus_control(&view, &mut vcx, ControlId::PreviewScope);
        assert_eq!(view.read_with(&vcx, |view, cx| view.preview_state_for_tests(cx).0), PreviewScope::Defaults);
        assert_eq!(
            view.read_with(&vcx, |view, cx| view.preview_state_for_tests(cx).4),
            ["example.one.preview", "example.two.preview"]
        );
        assert!(vcx.debug_bounds("settings-preview-provider-1").is_some());
        let visible = view.read_with(&vcx, |view, cx| view.visible_controls(cx));
        assert!(visible.contains(&ControlId::PreviewList));
        assert!(visible.contains(&ControlId::PreviewClear));
        assert!(!visible.contains(&ControlId::PreviewAdd), "no descriptor is published in this test");

        focus_control(&view, &mut vcx, ControlId::PreviewList);
        vcx.simulate_keystrokes("down");
        focus_control(&view, &mut vcx, ControlId::PreviewMoveUp);
        vcx.simulate_keystrokes("space");
        wait_for_preview(&vcx, |ui| {
            ui.views.defaults_layer().preview_providers.as_deref()
                == Some(&["example.two.preview".to_owned(), "example.one.preview".to_owned()][..])
        });
        assert!(views_text(&vcx).contains("# one"), "item comments survive: {}", views_text(&vcx));

        focus_control(&view, &mut vcx, ControlId::PreviewRemove);
        vcx.simulate_keystrokes("space");
        wait_for_preview(&vcx, |ui| {
            ui.views.defaults_layer().preview_providers.as_ref().is_some_and(|list| list.len() == 1)
        });

        focus_control(&view, &mut vcx, ControlId::PreviewClear);
        vcx.simulate_keystrokes("space");
        wait_for_preview(&vcx, |ui| ui.views.defaults_layer().preview_providers.is_none());
        assert!(!views_text(&vcx).contains("preview_providers"), "{}", views_text(&vcx));
        assert!(views_text(&vcx).starts_with("# mine\n"), "other content survives: {}", views_text(&vcx));

        // An explicit empty list is Native only, distinct from inheritance.
        let dir = vcx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
        std::fs::write(dir.join("views.toml"), "# mine\n[defaults]\npreview_providers = []\n").unwrap();
        vcx.update(|_, cx| ui::reload_config(cx));
        vcx.run_until_parked();
        draw(&mut vcx);
        assert!(
            view.read_with(&vcx, |view, cx| view.preview_inheritance_text(cx)).contains("Native only"),
            "an empty list is explicit"
        );
        assert!(!view.read_with(&vcx, |view, cx| view.visible_controls(cx)).contains(&ControlId::PreviewRemove));
        focus_control(&view, &mut vcx, ControlId::PreviewClear);
        vcx.simulate_keystrokes("space");
        wait_for_preview(&vcx, |ui| ui.views.defaults_layer().preview_providers.is_none());
    }

    #[gpui::test]
    fn preview_folder_uses_the_browser_folder_and_the_picker(cx: &mut TestAppContext) {
        let (_first_dir, first) = fixture("preview-folder");
        let (_second_dir, second) = fixture("preview-folder-second");
        let folder_key = first.display().to_string();
        let (view, mut vcx) = open_settings_after(cx, |cx| {
            let config = cx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
            std::fs::write(
                config.join("views.toml"),
                format!(
                    "# mine\n[folder.\"{folder_key}\"]\nview = \"list\" # keep\npreview_providers = [\"example.one.preview\"]\n"
                ),
            )
            .unwrap();
            cx.update(|cx| {
                cx.global_mut::<Ui>().settings_folder = Some(first.clone());
                ui::reload_config(cx);
            });
        });
        draw(&mut vcx);
        focus_control(&view, &mut vcx, ControlId::PreviewScope);
        vcx.simulate_keystrokes("right");
        assert_eq!(view.read_with(&vcx, |view, cx| view.preview_state_for_tests(cx).0), PreviewScope::Folder);
        assert_eq!(
            view.read_with(&vcx, |view, cx| view.preview_state_for_tests(cx).1),
            None,
            "Folder starts without a target"
        );
        assert!(view.read_with(&vcx, |view, cx| view.visible_controls(cx)).contains(&ControlId::PreviewCurrentFolder));
        focus_control(&view, &mut vcx, ControlId::PreviewCurrentFolder);
        vcx.simulate_keystrokes("space");
        assert_eq!(view.read_with(&vcx, |view, cx| view.preview_state_for_tests(cx).1), Some(first.clone()));
        draw(&mut vcx);
        assert_eq!(
            view.read_with(&vcx, |view, cx| view.preview_state_for_tests(cx).4),
            ["example.one.preview"],
            "the folder's own list is shown"
        );

        // Cancelling the picker leaves the target alone and returns focus.
        focus_control(&view, &mut vcx, ControlId::PreviewChooseFolder);
        vcx.simulate_keystrokes("space");
        assert!(vcx.cx.did_prompt_for_paths());
        vcx.cx.simulate_path_prompt_response(|_| None);
        vcx.run_until_parked();
        assert_eq!(view.read_with(&vcx, |view, cx| view.preview_state_for_tests(cx).1), Some(first.clone()));
        assert!(vcx.update(|window, cx| view.read(cx).handle(ControlId::PreviewChooseFolder).is_focused(window)));

        // Choosing a folder replaces the target before any write.
        vcx.simulate_keystrokes("space");
        assert!(vcx.cx.did_prompt_for_paths());
        vcx.cx.simulate_path_prompt_response(|_| Some(vec![second.clone()]));
        vcx.run_until_parked();
        assert_eq!(view.read_with(&vcx, |view, cx| view.preview_state_for_tests(cx).1), Some(second.clone()));
        assert_eq!(
            view.read_with(&vcx, |view, cx| view.preview_state_for_tests(cx).4),
            Vec::<String>::new(),
            "the new folder has no list yet"
        );

        // Editing the first folder still writes only that folder, keeping siblings.
        vcx.simulate_keystrokes("space");
        vcx.cx.simulate_path_prompt_response(|_| Some(vec![first.clone()]));
        vcx.run_until_parked();
        focus_control(&view, &mut vcx, ControlId::PreviewList);
        vcx.simulate_keystrokes("down");
        focus_control(&view, &mut vcx, ControlId::PreviewRemove);
        vcx.simulate_keystrokes("space");
        wait_for_preview(&vcx, |ui| {
            ui.views
                .folder_layer(&first)
                .and_then(|layer| layer.preview_providers.as_ref())
                .is_some_and(|list| list.is_empty())
        });
        let text = views_text(&vcx);
        assert!(text.contains("view = \"list\" # keep"), "{text}");
        assert!(!views_text(&vcx).contains(&second.display().to_string()), "the other folder is not added: {text}");
    }

    #[gpui::test]
    fn preview_rule_edits_one_rule_clears_only_preview_and_rejects_a_moved_rule(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings_after(cx, |cx| {
            let dir = cx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
            std::fs::write(
                dir.join("views.toml"),
                r#"# mine
[[rule]]
path = "~/Pictures"
view = "columns"
preview_providers = ["example.one.preview"]

[[rule]]
path = "~/Pictures"
sort = "size"
preview_providers = ["example.two.preview", "example.three.preview"]
"#,
            )
            .unwrap();
            cx.update(ui::reload_config);
        });
        draw(&mut vcx);
        focus_control(&view, &mut vcx, ControlId::PreviewScope);
        vcx.simulate_keystrokes("right");
        vcx.simulate_keystrokes("right");
        assert_eq!(view.read_with(&vcx, |view, cx| view.preview_state_for_tests(cx).0), PreviewScope::Rule);
        assert_eq!(view.read_with(&vcx, |view, cx| view.preview_state_for_tests(cx).2), Some(0), "first rule");

        // Select the second rule; its own ordered list is shown.
        focus_control(&view, &mut vcx, ControlId::PreviewRuleSelect);
        vcx.simulate_keystrokes("space");
        vcx.simulate_keystrokes("down");
        vcx.simulate_keystrokes("enter");
        assert_eq!(view.read_with(&vcx, |view, cx| view.preview_state_for_tests(cx).2), Some(1));
        assert_eq!(
            view.read_with(&vcx, |view, cx| view.preview_state_for_tests(cx).4),
            ["example.two.preview", "example.three.preview"]
        );

        focus_control(&view, &mut vcx, ControlId::PreviewList);
        vcx.simulate_keystrokes("down");
        focus_control(&view, &mut vcx, ControlId::PreviewMoveUp);
        vcx.simulate_keystrokes("space");
        wait_for_preview(&vcx, |ui| {
            ui.views
                .rule_preview_providers(1)
                .flatten()
                .is_some_and(|list| list == ["example.three.preview", "example.two.preview"])
        });
        assert_eq!(
            vcx.read_global::<Ui, _>(|ui, _| ui.views.rule_preview_providers(0).flatten()),
            Some(vec!["example.one.preview".to_owned()]),
            "the other rule is untouched"
        );

        focus_control(&view, &mut vcx, ControlId::PreviewClear);
        vcx.simulate_keystrokes("space");
        wait_for_preview(&vcx, |ui| ui.views.rule_preview_providers(1) == Some(None));
        let text = views_text(&vcx);
        assert!(text.contains("path = \"~/Pictures\"\nsort = \"size\""), "the mixed rule keeps its sort: {text}");
        assert!(
            text.contains("preview_providers = [\"example.one.preview\"]"),
            "the first rule keeps its list: {text}"
        );

        // Select the first rule again, then move it externally.
        focus_control(&view, &mut vcx, ControlId::PreviewRuleSelect);
        vcx.simulate_keystrokes("space");
        vcx.simulate_keystrokes("up");
        vcx.simulate_keystrokes("enter");
        assert_eq!(view.read_with(&vcx, |view, cx| view.preview_state_for_tests(cx).2), Some(0));
        let dir = vcx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
        std::fs::write(
            dir.join("views.toml"),
            r#"[[rule]]
path = "~/Downloads"
preview_providers = ["example.other.preview"]

[[rule]]
path = "~/Pictures"
view = "columns"
preview_providers = ["example.one.preview"]
"#,
        )
        .unwrap();
        vcx.update(|_, cx| ui::reload_config(cx));
        vcx.run_until_parked();
        draw(&mut vcx);
        let (_, _, rule, new, providers) = view.read_with(&vcx, |view, cx| view.preview_state_for_tests(cx));
        assert_eq!(rule, None, "the moved index is not aimed at the new occupant");
        assert!(new, "the editor asks for a fresh rule");
        assert!(providers.is_empty());
        assert!(view.read_with(&vcx, |view, cx| view.preview_inheritance_text(cx)).contains("changed"));
    }

    fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).expect("create fixture dir");
        for entry in std::fs::read_dir(from).expect("read fixture") {
            let entry = entry.expect("entry");
            let target = to.join(entry.file_name());
            if entry.file_type().expect("type").is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).expect("copy fixture file");
            }
        }
    }

    #[gpui::test]
    fn preview_rule_adds_a_provider_from_the_plugin_catalog(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings_after(cx, |cx| {
            let config = cx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
            let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plugins/p1b-fixture");
            copy_dir(&source, &config.join("plugins/p1b-fixture"));
            cx.update(|cx| crate::plugins::start_for_tests(cx, config));
        });
        wait_plugin_state(&mut vcx, "example.p1b-fixture", PluginState::PendingApproval);
        focus_control(&view, &mut vcx, ControlId::Plugins);
        vcx.simulate_keystrokes("enter");
        wait_plugin_state(&mut vcx, "example.p1b-fixture", PluginState::Running);

        focus_control(&view, &mut vcx, ControlId::PreviewScope);
        vcx.simulate_keystrokes("right");
        vcx.simulate_keystrokes("right");
        assert_eq!(view.read_with(&vcx, |view, cx| view.preview_state_for_tests(cx).0), PreviewScope::Rule);
        assert!(view.read_with(&vcx, |view, cx| view.preview_state_for_tests(cx).3), "no rules: a new one");
        assert_eq!(
            view.read_with(&vcx, |view, cx| view.preview_available_for_tests(cx)),
            ["example.p1b-fixture.sample-preview"]
        );

        focus_control(&view, &mut vcx, ControlId::PreviewAdd);
        vcx.simulate_keystrokes("space");
        vcx.simulate_keystrokes("enter");
        assert_eq!(
            view.read_with(&vcx, |view, cx| view.preview_state_for_tests(cx).4),
            ["example.p1b-fixture.sample-preview"],
            "the descriptor id enters the draft list"
        );
        focus_control(&view, &mut vcx, ControlId::PreviewRulePattern);
        vcx.simulate_input("~/[");
        draw(&mut vcx);
        let before = views_text(&vcx);
        vcx.simulate_keystrokes("enter");
        for _ in 0..200 {
            vcx.run_until_parked();
            if view.read_with(&vcx, |view, _| !view.preview_write_pending) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(views_text(&vcx), before);
        assert!(view.read_with(&vcx, |view, cx| view.preview_inheritance_text(cx)).contains("rule"));
        assert_eq!(view.read_with(&vcx, |view, _| view.preview_pattern_text.clone()), "~/[");
        assert_eq!(
            view.read_with(&vcx, |view, cx| view.preview_state_for_tests(cx).4),
            ["example.p1b-fixture.sample-preview"]
        );
        assert!(vcx.read_global::<Ui, _>(|ui, _| ui.views_error().is_none()));
        vcx.simulate_keystrokes("cmd-a");
        vcx.simulate_input("~/Pictures/**");
        draw(&mut vcx);
        focus_control(&view, &mut vcx, ControlId::PreviewAddRule);
        vcx.simulate_keystrokes("space");
        for _ in 0..200 {
            vcx.run_until_parked();
            if views_text(&vcx).contains("sample-preview") {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let text = views_text(&vcx);
        assert!(text.contains("path = \"~/Pictures/**\""), "{text}");
        assert!(text.contains("preview_providers = [\"example.p1b-fixture.sample-preview\"]"), "{text}");
        let views = vcx.read_global::<Ui, _>(|ui, _| ui.views.clone());
        assert_eq!(views.rules().len(), 1);
        assert_eq!(
            views.resolve(&crow_core::home().join("Pictures/Inside")).preview_providers.0,
            ["example.p1b-fixture.sample-preview"]
        );
    }

    #[gpui::test]
    fn segmented_moves_with_arrows(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        focus_control(&view, &mut vcx, ControlId::Keymap);
        vcx.simulate_keystrokes("right");
        assert_eq!(state(&view, &vcx, ControlId::Keymap).selected, Some(1));
        assert_eq!(vcx.read_global::<Ui, _>(|ui, _| ui.settings.keymap), KeymapLayer::Vim);
        wait_for(&vcx, "keymap = \"vim\"");
        vcx.simulate_keystrokes("left");
        assert_eq!(state(&view, &vcx, ControlId::Keymap).selected, Some(0));
        wait_for(&vcx, "keymap = \"finder\"");
    }

    #[gpui::test]
    fn enter_control_writes_only_its_line_and_survives_reload(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        focus_control(&view, &mut vcx, ControlId::Enter);
        assert_eq!(state(&view, &vcx, ControlId::Enter).selected, Some(0), "Rename is the default");
        let before = settings_text(&vcx);

        vcx.simulate_keystrokes("right");
        assert_eq!(state(&view, &vcx, ControlId::Enter).selected, Some(1));
        assert_eq!(vcx.read_global::<Ui, _>(|ui, _| ui.settings.enter), EnterAction::Open);
        wait_for(&vcx, "enter = \"open\"");
        let after = settings_text(&vcx);
        let changed: Vec<_> = before.lines().zip(after.lines()).filter(|(a, b)| a != b).collect();
        assert_eq!(changed.len(), 1, "only one line changes: {changed:?}");
        assert!(changed[0].1.starts_with("enter = "), "and it is the enter line");

        vcx.update(|_, cx| ui::reload_config(cx));
        vcx.run_until_parked();
        assert_eq!(state(&view, &vcx, ControlId::Enter).selected, Some(1), "a reload keeps the control");
    }

    #[gpui::test]
    fn path_bar_control_writes_only_its_line_and_survives_reload(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        let dir = vcx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
        std::fs::write(dir.join("settings.toml"), "# mine\npath_bar = \"crow\" # bottom bar\nkeymap = \"vim\"\n")
            .expect("external edit");
        vcx.update(|_, cx| ui::reload_config(cx));
        vcx.run_until_parked();
        focus_control(&view, &mut vcx, ControlId::PathBar);
        assert_eq!(state(&view, &vcx, ControlId::PathBar).selected, Some(0), "Crow is the default");
        let before = settings_text(&vcx);

        vcx.simulate_keystrokes("right");
        assert_eq!(state(&view, &vcx, ControlId::PathBar).selected, Some(1));
        assert_eq!(vcx.read_global::<Ui, _>(|ui, _| ui.settings.path_bar), PathBarMode::Explorer);
        wait_for(&vcx, "path_bar = \"explorer\"");
        let after = settings_text(&vcx);
        assert!(after.contains("\npath_bar = \"explorer\" # bottom bar\n"), "{after}");
        assert!(after.contains("\nkeymap = \"vim\"\n"), "an unrelated key stays: {after}");
        let changed: Vec<_> = before.lines().zip(after.lines()).filter(|(a, b)| a != b).collect();
        assert_eq!(changed.len(), 1, "only one line changes: {changed:?}");

        vcx.update(|_, cx| ui::reload_config(cx));
        vcx.run_until_parked();
        assert_eq!(state(&view, &vcx, ControlId::PathBar).selected, Some(1), "a reload keeps the control");

        // An external edit back to Crow updates the segmented control.
        std::fs::write(dir.join("settings.toml"), "# mine\npath_bar = \"crow\" # bottom bar\n").expect("external edit");
        vcx.update(|_, cx| ui::reload_config(cx));
        vcx.run_until_parked();
        assert_eq!(state(&view, &vcx, ControlId::PathBar).selected, Some(0), "Crow shows again");
    }

    #[gpui::test]
    fn stepper_arrows_and_typed_digits(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        focus_control(&view, &mut vcx, ControlId::FontSize);
        vcx.simulate_keystrokes("up");
        assert_eq!(state(&view, &vcx, ControlId::FontSize).value, Some(14.0));
        assert_eq!(stepper_text(&view, &vcx, ControlId::FontSize), "14", "the field follows the arrow");
        wait_for(&vcx, "font_size = 14.0");
        vcx.simulate_keystrokes("shift-down");
        assert_eq!(state(&view, &vcx, ControlId::FontSize).value, Some(10.0), "14 - 10 clamps to 10");
        assert_eq!(stepper_text(&view, &vcx, ControlId::FontSize), "10");
        wait_for(&vcx, "font_size = 10.0");
        vcx.simulate_keystrokes("cmd-a");
        vcx.simulate_input("20");
        vcx.simulate_keystrokes("enter");
        assert_eq!(state(&view, &vcx, ControlId::FontSize).value, Some(20.0));
        assert_eq!(stepper_text(&view, &vcx, ControlId::FontSize), "20");
        wait_for(&vcx, "font_size = 20.0");
    }

    #[gpui::test]
    fn terminal_field_writes_the_app(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        focus_control(&view, &mut vcx, ControlId::Terminal);
        vcx.simulate_keystrokes("cmd-a");
        vcx.simulate_input("iTerm");
        vcx.simulate_keystrokes("enter");
        assert_eq!(vcx.read_global::<Ui, _>(|ui, _| ui.settings.terminal.clone()), "iTerm");
        wait_for(&vcx, "terminal = \"iTerm\"");
    }

    fn set_two_themes(view: &Entity<SettingsWindow>, vcx: &mut VisualTestContext) {
        view.update(vcx, |view, cx| {
            view.set_theme_entries_for_tests(
                vec![
                    ThemeInfo {
                        id: "dark-one".into(),
                        name: "Dark One".into(),
                        variant: Variant::Dark,
                        source: crow_theme::Source::Index,
                    },
                    ThemeInfo {
                        id: "light-one".into(),
                        name: "Light One".into(),
                        variant: Variant::Light,
                        source: crow_theme::Source::Index,
                    },
                ],
                "system",
            );
            cx.notify();
        });
        vcx.run_until_parked();
    }

    fn menu(view: &Entity<SettingsWindow>, vcx: &VisualTestContext) -> Option<ControlId> {
        view.read_with(vcx, |view, _| view.menu)
    }

    fn preview(vcx: &VisualTestContext) -> Option<String> {
        vcx.read_global::<Ui, _>(|ui, _| ui.preview_theme.clone())
    }

    fn saved_theme(vcx: &VisualTestContext) -> String {
        vcx.read_global::<Ui, _>(|ui, _| ui.settings.theme.clone())
    }

    #[gpui::test]
    fn theme_dropdown_previews_keeps_on_return_and_reverts_on_escape(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        set_two_themes(&view, &mut vcx);
        focus_control(&view, &mut vcx, ControlId::Theme);

        vcx.simulate_keystrokes("down");
        assert_eq!(menu(&view, &vcx), Some(ControlId::Theme), "↓ opens the menu");
        assert_eq!(state(&view, &vcx, ControlId::Theme).selected, Some(0), "opening does not move");
        assert_eq!(preview(&vcx), None);

        vcx.simulate_keystrokes("down");
        assert_eq!(state(&view, &vcx, ControlId::Theme).selected, Some(1), "the cursor moved");
        assert_eq!(preview(&vcx), Some("dark-one".to_owned()), "moving previews");
        assert_eq!(saved_theme(&vcx), "system", "not kept yet");

        vcx.simulate_keystrokes("escape");
        assert_eq!(menu(&view, &vcx), None, "Esc closes the menu");
        assert_eq!(preview(&vcx), None, "Esc reverts the preview");
        assert_eq!(state(&view, &vcx, ControlId::Theme).selected, Some(0));
        assert_eq!(saved_theme(&vcx), "system");
        let open = cx.update(|cx| cx.windows().iter().any(|handle| handle.downcast::<SettingsWindow>().is_some()));
        assert!(open, "the first Esc only closes the menu");

        vcx.simulate_keystrokes("space down enter");
        assert_eq!(menu(&view, &vcx), None, "Return closes the menu");
        assert_eq!(saved_theme(&vcx), "dark-one", "Return keeps the entry");
        assert_eq!(preview(&vcx), None);
        wait_for(&vcx, "theme = \"dark-one\"");
    }

    #[gpui::test]
    fn leaving_an_open_theme_menu_reverts_it(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        set_two_themes(&view, &mut vcx);
        focus_control(&view, &mut vcx, ControlId::Theme);
        vcx.simulate_keystrokes("enter down");
        assert_eq!(preview(&vcx), Some("dark-one".to_owned()));
        vcx.simulate_keystrokes("tab");
        draw(&mut vcx);
        assert_eq!(menu(&view, &vcx), None);
        assert_eq!(preview(&vcx), None, "Tab away drops the preview");
        assert_eq!(saved_theme(&vcx), "system");

        focus_control(&view, &mut vcx, ControlId::Theme);
        vcx.simulate_keystrokes("enter down cmd-w");
        cx.run_until_parked();
        assert_eq!(preview(&vcx), None, "closing the window drops the preview");
        assert_eq!(saved_theme(&vcx), "system");
    }

    /// H4 #20 critique: the initial list is ["System"] plus the saved family; a
    /// hand-edited `font_family = "System"` must not list System twice.
    #[gpui::test]
    fn a_system_font_family_is_listed_once(cx: &mut TestAppContext) {
        let (view, vcx) = open_settings_after(cx, |cx| {
            cx.update(|cx| cx.global_mut::<Ui>().settings.font_family = "System".into());
        });
        let fonts = view.read_with(&vcx, |view, _| view.fonts.clone());
        assert_eq!(fonts.iter().filter(|name| *name == "System").count(), 1, "{fonts:?}");
    }

    #[gpui::test]
    fn font_dropdown_writes_only_when_kept(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        view.update(&mut vcx, |view, cx| {
            view.set_fonts_for_tests(vec!["System".into(), "Menlo".into(), "Zed Mono".into()], "");
            cx.notify();
        });
        vcx.run_until_parked();
        let font = |vcx: &VisualTestContext| vcx.read_global::<Ui, _>(|ui, _| ui.settings.font_family.clone());
        focus_control(&view, &mut vcx, ControlId::FontFamily);

        vcx.simulate_keystrokes("enter down");
        assert_eq!(menu(&view, &vcx), Some(ControlId::FontFamily));
        assert_eq!(state(&view, &vcx, ControlId::FontFamily).selected, Some(1));
        assert_eq!(font(&vcx), "", "moving in the menu does not write");
        vcx.simulate_keystrokes("enter");
        assert_eq!(font(&vcx), "Menlo");
        wait_for(&vcx, "font_family = \"Menlo\"");

        vcx.simulate_input("zed");
        assert_eq!(menu(&view, &vcx), Some(ControlId::FontFamily), "typing opens the menu");
        assert_eq!(state(&view, &vcx, ControlId::FontFamily).selected, Some(2));
        assert_eq!(font(&vcx), "Menlo", "typing does not write");
        vcx.simulate_keystrokes("enter");
        assert_eq!(font(&vcx), "Zed Mono");
        wait_for(&vcx, "font_family = \"Zed Mono\"");
    }

    #[gpui::test]
    fn sidebar_style_is_a_five_way_choice_that_starts_on_ledger_lines(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        focus_control(&view, &mut vcx, ControlId::SidebarStyle);
        assert_eq!(state(&view, &vcx, ControlId::SidebarStyle).selected, Some(0), "Ledger lines is the default");
        vcx.simulate_keystrokes("right right");
        assert_eq!(
            vcx.read_global::<Ui, _>(|ui, _| ui.settings.sidebar_style),
            crow_core::SidebarStyle::Accordion,
            "two steps right is the third style"
        );
        wait_for(&vcx, "sidebar_style = \"accordion\"");
    }

    #[gpui::test]
    fn theme_dropdown_by_mouse(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        set_two_themes(&view, &mut vcx);
        let button = vcx.debug_bounds("settings-theme-control").expect("theme button").center();
        let click = |vcx: &mut VisualTestContext, at| {
            vcx.simulate_click(at, gpui::Modifiers::none());
            draw(vcx);
        };

        click(&mut vcx, button);
        assert_eq!(menu(&view, &vcx), Some(ControlId::Theme), "a click opens the menu");
        click(&mut vcx, button);
        assert_eq!(menu(&view, &vcx), None, "a second click on the button closes it");

        click(&mut vcx, button);
        // The theme row's label, left of the button: outside the menu, and not a control.
        let label = vcx.debug_bounds("settings-theme-row").expect("theme row");
        let elsewhere = gpui::point(label.left() + px(8.), label.center().y);
        click(&mut vcx, elsewhere);
        assert_eq!(menu(&view, &vcx), None, "a click outside closes it");
        assert_eq!(saved_theme(&vcx), "system");

        click(&mut vcx, button);
        let menu_bounds = vcx.debug_bounds("settings-theme-menu").expect("menu");
        // Three equal rows: System, Dark One, Light One.
        let row = menu_bounds.size.height / 3.;
        let second = gpui::point(menu_bounds.center().x, menu_bounds.top() + row * 1.5);
        click(&mut vcx, second);
        assert_eq!(menu(&view, &vcx), None);
        assert_eq!(saved_theme(&vcx), "dark-one", "a click on an entry keeps it");
    }

    #[gpui::test]
    fn a_long_menu_scrolls_instead_of_squashing_its_rows(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        let fonts: Vec<String> =
            std::iter::once("System".to_owned()).chain((0..300).map(|i| format!("Font {i:03}"))).collect();
        view.update(&mut vcx, |view, cx| {
            view.set_fonts_for_tests(fonts, "");
            cx.notify();
        });
        vcx.run_until_parked();
        focus_control(&view, &mut vcx, ControlId::FontFamily);
        vcx.simulate_keystrokes("space");
        draw(&mut vcx);
        let row = vcx.debug_bounds("settings-font-family-option-0").expect("first entry");
        assert!(row.size.height >= px(16.), "rows keep their height: {:?}", row.size);
    }

    #[gpui::test]
    fn the_menu_scrolls_to_keep_the_cursor_in_view(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        let fonts: Vec<String> =
            std::iter::once("System".to_owned()).chain((0..300).map(|i| format!("Font {i:03}"))).collect();
        view.update(&mut vcx, |view, cx| {
            view.set_fonts_for_tests(fonts, "");
            cx.notify();
        });
        vcx.run_until_parked();
        focus_control(&view, &mut vcx, ControlId::FontFamily);
        vcx.simulate_keystrokes("space");
        draw(&mut vcx);
        for step in 1..=40 {
            vcx.simulate_keystrokes("down");
            draw(&mut vcx);
            draw(&mut vcx);
            let menu = vcx.debug_bounds("settings-font-family-menu").expect("menu");
            let row =
                vcx.debug_bounds(format!("settings-font-family-option-{step}").leak()).expect("cursor row is mounted");
            assert!(
                row.top() >= menu.top() && row.bottom() <= menu.bottom(),
                "step {step}: cursor row {row:?} stays inside the menu {menu:?}"
            );
        }
    }

    #[gpui::test]
    fn the_theme_menu_scrolls_to_keep_the_cursor_in_view(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        view.update(&mut vcx, |view, cx| {
            let entries = (0..60)
                .map(|i| ThemeInfo {
                    id: format!("theme-{i:02}"),
                    name: format!("Theme {i:02}"),
                    variant: if i % 2 == 0 { Variant::Dark } else { Variant::Light },
                    source: crow_theme::Source::Index,
                })
                .collect();
            view.set_theme_entries_for_tests(entries, "theme-00");
            cx.notify();
        });
        vcx.run_until_parked();
        focus_control(&view, &mut vcx, ControlId::Theme);
        vcx.simulate_keystrokes("space");
        draw(&mut vcx);
        for step in 1..=40 {
            vcx.simulate_keystrokes("down");
            draw(&mut vcx);
            draw(&mut vcx);
            let menu = vcx.debug_bounds("settings-theme-menu").expect("menu");
            let row = vcx.debug_bounds(format!("settings-theme-option-{step}").leak()).expect("cursor row is mounted");
            assert!(
                row.top() >= menu.top() && row.bottom() <= menu.bottom(),
                "step {step}: cursor row {row:?} stays inside the menu {menu:?}"
            );
        }
    }

    #[gpui::test]
    fn the_theme_menu_opens_with_the_saved_theme_in_view(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        view.update(&mut vcx, |view, cx| {
            let entries = (0..60)
                .map(|i| ThemeInfo {
                    id: format!("theme-{i:02}"),
                    name: format!("Theme {i:02}"),
                    variant: Variant::Dark,
                    source: crow_theme::Source::Index,
                })
                .collect();
            view.set_theme_entries_for_tests(entries, "theme-45");
            cx.notify();
        });
        vcx.run_until_parked();
        vcx.update_global::<Ui, _>(|ui, _| ui.settings.theme = "theme-45".to_owned());
        focus_control(&view, &mut vcx, ControlId::Theme);
        vcx.simulate_keystrokes("space");
        draw(&mut vcx);
        draw(&mut vcx);
        let menu = vcx.debug_bounds("settings-theme-menu").expect("menu");
        let row = vcx.debug_bounds("settings-theme-option-45").expect("saved row is mounted");
        assert!(row.top() >= menu.top() && row.bottom() <= menu.bottom(), "saved row {row:?} in menu {menu:?}");
        for (key, target) in [("pagedown", 0), ("end", 59), ("home", 0)] {
            vcx.simulate_keystrokes(key);
            draw(&mut vcx);
            draw(&mut vcx);
            let cursor = view.read_with(&vcx, |view, _| view.theme.cursor);
            let _ = target;
            let row = vcx.debug_bounds(format!("settings-theme-option-{cursor}").leak());
            let ok = row.is_some_and(|row| row.top() >= menu.top() && row.bottom() <= menu.bottom());
            assert!(ok, "{key}: cursor {cursor} row {row:?} in menu {menu:?}");
        }
    }

    #[gpui::test]
    fn an_open_menu_floats_over_the_page(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        set_two_themes(&view, &mut vcx);
        draw(&mut vcx);
        let below = vcx.debug_bounds("settings-icon-pack-row").expect("icon pack row");
        focus_control(&view, &mut vcx, ControlId::Theme);
        vcx.simulate_keystrokes("space");
        draw(&mut vcx);
        let menu = vcx.debug_bounds("settings-theme-menu").expect("the open menu draws");
        let button = vcx.debug_bounds("settings-theme-control").expect("theme button");
        assert!(menu.top() >= button.bottom(), "the menu opens under its button");
        assert_eq!(vcx.debug_bounds("settings-icon-pack-row"), Some(below), "opening it moves nothing on the page");
    }

    #[gpui::test]
    fn focus_ring_does_not_resize_a_control(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        draw(&mut vcx);
        let before = vcx.debug_bounds("settings-theme-control").expect("theme control bounds");
        focus_control(&view, &mut vcx, ControlId::IconPack);
        draw(&mut vcx);
        let after = vcx.debug_bounds("settings-theme-control").expect("theme control bounds");
        assert_eq!(before, after, "the focus ring is inset and never resizes the control");
    }

    #[gpui::test]
    fn tabbing_to_a_lower_control_scrolls_the_page(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        draw(&mut vcx);
        let before = view.read_with(&vcx, |view, _| view.page_scroll.offset().y);
        focus_control(&view, &mut vcx, ControlId::WindowHeight);
        draw(&mut vcx);
        let after = view.read_with(&vcx, |view, _| view.page_scroll.offset().y);
        assert!(after < before, "the page scrolls to the focused control: {before:?} -> {after:?}");
    }

    #[gpui::test]
    fn file_buttons_record_their_effects(cx: &mut TestAppContext) {
        use crate::effects::Call;
        let (view, mut vcx) = open_settings(cx);
        let config = vcx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
        let click =
            |view: &Entity<SettingsWindow>, vcx: &mut VisualTestContext, id: ControlId, selector: &'static str| {
                focus_control(view, vcx, id);
                view.update(vcx, |view, cx| {
                    view.page_scroll.scroll_to_bottom();
                    cx.notify();
                });
                draw(vcx);
                let at = vcx.debug_bounds(selector).expect("control on screen").center();
                vcx.simulate_click(at, gpui::Modifiers::default());
                vcx.run_until_parked();
            };
        click(&view, &mut vcx, ControlId::EditSettings, "settings-edit-file");
        click(&view, &mut vcx, ControlId::ShowConfig, "settings-show-config");
        let calls = vcx.update(|_, cx| crate::effects::recording(cx).expect("recording effects").take_calls());
        assert!(calls.contains(&Call::OpenInDefaultApp(config.join("settings.toml"))), "{calls:?}");
        assert!(!calls.iter().any(|call| matches!(call, Call::OpenFolderInFinder(_))), "it opens in Crow: {calls:?}");
        let opened = vcx.update(|_, cx| {
            cx.windows().iter().filter_map(|handle| handle.downcast::<crate::window::CrowWindow>()).count()
        });
        assert!(opened >= 1, "a Crow window opened on the config folder");
    }

    #[gpui::test]
    fn the_settings_titlebar_band_moves_the_window(cx: &mut TestAppContext) {
        use crate::effects::Call;
        let (_view, mut vcx) = open_settings(cx);
        draw(&mut vcx);
        let at = vcx.debug_bounds("settings-titlebar").expect("titlebar band").center();
        vcx.simulate_click(at, gpui::Modifiers::default());
        vcx.run_until_parked();
        let calls = vcx.update(|_, cx| crate::effects::recording(cx).expect("recording effects").take_calls());
        assert!(calls.contains(&Call::StartWindowMove), "{calls:?}");
    }

    #[gpui::test]
    fn every_control_has_a_role_and_label(cx: &mut TestAppContext) {
        let (view, vcx) = open_settings(cx);
        for id in ControlId::ALL {
            let state = state(&view, &vcx, id);
            assert!(!state.label.is_empty(), "{id:?} has a label");
            assert_eq!(state.role, id.role(), "{id:?} has its role");
        }
    }

    #[gpui::test]
    fn broken_file_disables_controls_and_fixing_clears_it(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        let dir = vcx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
        std::fs::write(dir.join("settings.toml"), "motion = \"slow\"\n").expect("break file");
        vcx.update(|_, cx| ui::reload_config(cx));
        vcx.run_until_parked();
        assert!(vcx.read_global::<Ui, _>(|ui, _| ui.settings_error().is_some()));
        assert!(!state(&view, &vcx, ControlId::Theme).enabled, "writing controls are disabled");
        assert!(state(&view, &vcx, ControlId::EditSettings).enabled, "the file button stays usable");
        assert_eq!(
            view.read_with(&vcx, |view, cx| view.first_focus(cx)),
            view.read_with(&vcx, |view, cx| view.search_handle(cx)),
            "search stays a focus target when every write is disabled"
        );
        let error_focus = view.read_with(&vcx, |view, _| view.error_handles["settings.toml"].clone());
        assert_eq!(
            view.read_with(&vcx, |view, cx| view.focus_targets(cx)),
            vec![FocusTarget::Search, FocusTarget::Sidebar, FocusTarget::Error("settings.toml")],
            "the recovery button follows the sidebar in the tab order"
        );
        let sidebar = view.read_with(&vcx, |view, _| view.sidebar.clone());
        vcx.update(|window, cx| window.focus(&sidebar, cx));
        vcx.simulate_keystrokes("tab");
        let on_error = vcx.update(|window, cx| view.read(cx).error_handles["settings.toml"].is_focused(window));
        assert!(on_error, "Tab from the sidebar reaches the file's Open button");
        assert!(vcx.update(|window, _| error_focus.is_focused(window)), "the tested handle is the Open button");
        std::fs::write(dir.join("settings.toml"), "motion = \"auto\"\n").expect("fix file");
        vcx.update(|_, cx| ui::reload_config(cx));
        vcx.run_until_parked();
        assert!(vcx.read_global::<Ui, _>(|ui, _| ui.settings_error().is_none()));
        assert!(state(&view, &vcx, ControlId::Theme).enabled);
    }

    #[gpui::test]
    fn external_edits_update_the_control(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        let dir = vcx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
        std::fs::write(dir.join("settings.toml"), "keymap = \"vim\"\n").expect("edit file");
        vcx.update(|_, cx| ui::reload_config(cx));
        vcx.run_until_parked();
        assert_eq!(state(&view, &vcx, ControlId::Keymap).selected, Some(1), "Vim shows");
        std::fs::write(dir.join("settings.toml"), "\n").expect("delete line");
        vcx.update(|_, cx| ui::reload_config(cx));
        vcx.run_until_parked();
        assert_eq!(state(&view, &vcx, ControlId::Keymap).selected, Some(0), "Finder shows again");
    }

    #[gpui::test]
    fn reload_after_a_write_does_not_flip_the_control(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        focus_control(&view, &mut vcx, ControlId::Keymap);
        vcx.simulate_keystrokes("right");
        wait_for(&vcx, "keymap = \"vim\"");
        vcx.update(|_, cx| ui::reload_config(cx));
        vcx.run_until_parked();
        assert_eq!(state(&view, &vcx, ControlId::Keymap).selected, Some(1));
        assert_eq!(vcx.read_global::<Ui, _>(|ui, _| ui.settings.keymap), KeymapLayer::Vim);
    }

    fn write_named_plugin_bundle(config: &std::path::Path, dir_name: &str, id: &str, name: &str) {
        let dir = config.join("plugins").join(dir_name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("plugin.toml"),
            format!(
                r#"
id = "{id}"
name = "{name}"
version = "0.1.0"
api = 1
entry = "main.rhai"
provides = ["command"]
permissions = {{ fs = "none", spawn = [], net = false }}
"#
            ),
        )
        .unwrap();
        std::fs::write(
            dir.join("main.rhai"),
            r#"
fn register() { [ #{ kind: "command", id: "copy", title: "Copy", function: "run" } ] }
fn run(ctx) { #{ kind: "copy_text", text: "x" } }
"#,
        )
        .unwrap();
    }

    fn write_plugin_bundle(config: &std::path::Path) {
        let dir = config.join("plugins/demo");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("plugin.toml"),
            r#"
id = "example.demo"
name = "Demo"
version = "0.1.0"
api = 1
entry = "main.rhai"
provides = ["command"]
permissions = { fs = "none", spawn = [], net = false }
"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("main.rhai"),
            r#"
fn register() { [ #{ kind: "command", id: "copy", title: "Copy", function: "run" } ] }
fn run(ctx) { #{ kind: "copy_text", text: "x" } }
"#,
        )
        .unwrap();
    }

    fn write_long_name_plugin_bundle(config: &std::path::Path) {
        let dir = config.join("plugins/long-name");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("plugin.toml"),
            r#"
id = "example.long-name"
name = "An Extremely Long Plugin Display Name That Should Not Push The Action Buttons Off The Visible Settings Column Or Overlap Them in Any Way Whatsoever"
version = "0.1.0"
api = 1
entry = "main.rhai"
provides = ["command"]
permissions = { fs = "none", spawn = [], net = false }
"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("main.rhai"),
            r#"
fn register() { [ #{ kind: "command", id: "copy", title: "Copy", function: "run" } ] }
fn run(ctx) { #{ kind: "copy_text", text: "x" } }
"#,
        )
        .unwrap();
    }

    fn plugin_state(vcx: &VisualTestContext, id: &str) -> Option<PluginState> {
        vcx.read_global::<crate::plugins::Plugins, _>(|plugins, _| {
            plugins
                .inventory
                .plugins
                .iter()
                .find(|entry| entry.id.as_ref().is_some_and(|plugin| plugin.as_str() == id))
                .map(|entry| entry.state)
        })
    }

    fn wait_plugin_state(vcx: &mut VisualTestContext, id: &str, wanted: PluginState) {
        for _ in 0..500 {
            vcx.update(|_, cx| crate::plugins::poll(cx));
            vcx.cx.run_until_parked();
            vcx.run_until_parked();
            if plugin_state(vcx, id) == Some(wanted) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("plugin {id} never reached {wanted:?}; got {:?}", plugin_state(vcx, id));
    }

    fn wait_plugin_count(vcx: &mut VisualTestContext, wanted: usize) {
        for _ in 0..500 {
            vcx.update(|_, cx| crate::plugins::poll(cx));
            vcx.cx.run_until_parked();
            vcx.run_until_parked();
            if vcx.read_global::<crate::plugins::Plugins, _>(|plugins, _| plugins.inventory.plugins.len()) == wanted {
                draw(vcx);
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let count = vcx.read_global::<crate::plugins::Plugins, _>(|plugins, _| plugins.inventory.plugins.len());
        panic!("plugin inventory stayed at {count}, wanted {wanted}");
    }

    fn selected_bundle(view: &Entity<SettingsWindow>, vcx: &VisualTestContext) -> Option<std::ffi::OsString> {
        view.read_with(vcx, |view, _| view.selected_plugin.clone())
    }

    fn grants_error(vcx: &VisualTestContext) -> bool {
        vcx.read_global::<crate::plugins::Plugins, _>(|plugins, _| plugins.inventory.grants_error.is_some())
    }

    #[gpui::test]
    fn plugins_inventory_approves_and_disables_from_settings(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings_after(cx, |cx| {
            let config = cx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
            write_plugin_bundle(&config);
            cx.update(|cx| crate::plugins::start_for_tests(cx, config));
        });
        wait_plugin_state(&mut vcx, "example.demo", PluginState::PendingApproval);
        show_category(&view, &mut vcx, Category::Plugins);
        assert!(view.read_with(&vcx, |view, cx| view.debug_selectors(cx)).iter().any(|id| id == "settings-plugins"));
        assert!(
            view.read_with(&vcx, |view, cx| view.debug_selectors(cx))
                .iter()
                .any(|id| id == "settings-category-plugins"),
            "categories are fuzzable"
        );
        let violations = vcx.update(|window, cx| view.read(cx).check_invariants(window, cx));
        assert!(violations.is_empty(), "{violations:?}");

        focus_control(&view, &mut vcx, ControlId::Plugins);
        vcx.simulate_keystrokes("enter");
        wait_plugin_state(&mut vcx, "example.demo", PluginState::Running);
        let config = vcx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
        let grants = std::fs::read_to_string(config.join("plugins.toml")).unwrap();
        assert!(grants.contains(r#"[plugin."example.demo"]"#), "{grants}");
        assert!(grants.contains("enabled = true"), "{grants}");

        vcx.simulate_keystrokes("t");
        std::thread::sleep(Duration::from_millis(50));
        vcx.update(|_, cx| crate::plugins::poll(cx));
        vcx.cx.run_until_parked();
        assert!(
            vcx.read_global::<crate::operations::Operations, _>(|operations, _| operations.message.is_none()),
            "Retry is not an action on a running package, so its key must do nothing"
        );

        vcx.simulate_keystrokes("d");
        wait_plugin_state(&mut vcx, "example.demo", PluginState::Disabled);
        let grants = std::fs::read_to_string(config.join("plugins.toml")).unwrap();
        assert!(grants.contains("enabled = false"), "{grants}");
    }

    #[gpui::test]
    fn a_long_plugin_name_keeps_its_actions_inside_the_list(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings_after(cx, |cx| {
            let config = cx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
            write_long_name_plugin_bundle(&config);
            cx.update(|cx| crate::plugins::start_for_tests(cx, config));
        });
        wait_plugin_state(&mut vcx, "example.long-name", PluginState::PendingApproval);
        focus_control(&view, &mut vcx, ControlId::Plugins);
        draw(&mut vcx);

        let list = vcx.debug_bounds("settings-plugins").expect("the plugin list draws");
        let approve = vcx.debug_bounds("plugin-action-0-0").expect("Approve draws");
        let reload = vcx.debug_bounds("plugin-action-0-3").expect("Reload draws");
        assert!(approve.left() >= list.left(), "Approve stays inside the list: {approve:?} vs {list:?}");
        assert!(approve.right() <= list.right(), "Approve stays inside the list: {approve:?} vs {list:?}");
        assert!(reload.right() <= list.right(), "Reload stays inside the list: {reload:?} vs {list:?}");
    }

    #[gpui::test]
    fn plugins_invalid_package_names_itself_in_the_status_line(cx: &mut TestAppContext) {
        let (_view, mut vcx) = open_settings_after(cx, |cx| {
            let config = cx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
            let dir = config.join("plugins/broken");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("plugin.toml"),
                r#"
id = "example.broken"
name = "Broken Example"
version = "0.1.0"
api = 9
entry = "main.rhai"
provides = ["command"]
permissions = { fs = "none" }
"#,
            )
            .unwrap();
            std::fs::write(dir.join("main.rhai"), "fn register() { [] }").unwrap();
            cx.update(|cx| crate::plugins::start_for_tests(cx, config));
        });
        wait_plugin_state(&mut vcx, "example.broken", PluginState::IncompatibleApi);
        let message = vcx.read_global::<crate::operations::Operations, _>(|operations, _| operations.message.clone());
        let (text, is_error) = message.expect("an error in the status line");
        assert!(is_error);
        assert!(text.contains("Broken Example"), "{text}");
        assert!(text.contains("API"), "{text}");
    }

    #[gpui::test]
    fn keyboard_approval_stays_with_the_rendered_bundle_after_an_insertion(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings_after(cx, |cx| {
            let config = cx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
            write_named_plugin_bundle(&config, "a", "example.a", "A");
            write_named_plugin_bundle(&config, "b", "example.b", "B");
            cx.update(|cx| crate::plugins::start_for_tests(cx, config));
        });
        wait_plugin_count(&mut vcx, 2);
        focus_control(&view, &mut vcx, ControlId::Plugins);
        draw(&mut vcx);
        vcx.simulate_keystrokes("down");
        draw(&mut vcx);
        assert_eq!(selected_bundle(&view, &vcx).as_deref(), Some(std::ffi::OsStr::new("b")));

        let config = vcx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
        write_named_plugin_bundle(&config, "a0", "example.a0", "A0");
        vcx.update(|_, cx| crate::plugins::reload(cx));
        wait_plugin_count(&mut vcx, 3);
        assert_eq!(
            selected_bundle(&view, &vcx).as_deref(),
            Some(std::ffi::OsStr::new("b")),
            "the selected key survives an insertion ahead of it"
        );

        vcx.simulate_keystrokes("a");
        wait_plugin_state(&mut vcx, "example.b", PluginState::Running);
        let grants = std::fs::read_to_string(config.join("plugins.toml")).unwrap();
        assert!(grants.contains(r#"[plugin."example.b"]"#), "{grants}");
        assert!(!grants.contains(r#"[plugin."example.a"]"#), "{grants}");
        assert!(!grants.contains(r#"[plugin."example.a0"]"#), "{grants}");
    }

    #[gpui::test]
    fn a_removed_selected_row_chooses_a_successor_or_the_previous_row(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings_after(cx, |cx| {
            let config = cx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
            write_named_plugin_bundle(&config, "a", "example.a", "A");
            write_named_plugin_bundle(&config, "b", "example.b", "B");
            write_named_plugin_bundle(&config, "c", "example.c", "C");
            cx.update(|cx| crate::plugins::start_for_tests(cx, config));
        });
        wait_plugin_count(&mut vcx, 3);
        focus_control(&view, &mut vcx, ControlId::Plugins);
        draw(&mut vcx);
        vcx.simulate_keystrokes("down");
        draw(&mut vcx);
        assert_eq!(selected_bundle(&view, &vcx).as_deref(), Some(std::ffi::OsStr::new("b")));

        let config = vcx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
        std::fs::remove_dir_all(config.join("plugins/b")).unwrap();
        vcx.update(|_, cx| crate::plugins::reload(cx));
        wait_plugin_count(&mut vcx, 2);
        assert_eq!(
            selected_bundle(&view, &vcx).as_deref(),
            Some(std::ffi::OsStr::new("c")),
            "the row at the previous position is the successor"
        );

        std::fs::remove_dir_all(config.join("plugins/c")).unwrap();
        vcx.update(|_, cx| crate::plugins::reload(cx));
        wait_plugin_count(&mut vcx, 1);
        assert_eq!(
            selected_bundle(&view, &vcx).as_deref(),
            Some(std::ffi::OsStr::new("a")),
            "removing the last row falls back to the previous row"
        );
    }

    #[gpui::test]
    fn a_long_plugin_list_scrolls_the_selected_row_into_view(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings_after(cx, |cx| {
            let config = cx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
            for index in 0..20 {
                write_named_plugin_bundle(
                    &config,
                    &format!("p{index:02}"),
                    &format!("example.p{index:02}"),
                    &format!("P{index:02}"),
                );
            }
            cx.update(|cx| crate::plugins::start_for_tests(cx, config));
        });
        wait_plugin_count(&mut vcx, 20);
        focus_control(&view, &mut vcx, ControlId::Plugins);
        draw(&mut vcx);
        for _ in 0..19 {
            vcx.simulate_keystrokes("down");
            draw(&mut vcx);
        }
        // The scroll requested by the last Down is applied in the next prepaint.
        draw(&mut vcx);

        let list = vcx.debug_bounds("settings-plugins").expect("the list draws");
        let selected = vcx.debug_bounds("plugin-action-19-0").expect("the selected row's Approve draws");
        assert!(selected.top() >= list.top(), "selected row above the list: {selected:?} vs {list:?}");
        assert!(selected.bottom() <= list.bottom(), "selected row below the list: {selected:?} vs {list:?}");
        let offset = view.read_with(&vcx, |view, _| view.plugin_scroll.offset().y);
        assert!(offset < px(0.), "the list scrolled to the selected row: {offset:?}");
    }

    #[gpui::test]
    fn duplicate_id_rows_keep_distinct_bundle_keys_and_block_approval(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings_after(cx, |cx| {
            let config = cx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
            write_named_plugin_bundle(&config, "dup-one", "example.dup", "Dup One");
            write_named_plugin_bundle(&config, "dup-two", "example.dup", "Dup Two");
            cx.update(|cx| crate::plugins::start_for_tests(cx, config));
        });
        wait_plugin_count(&mut vcx, 2);
        focus_control(&view, &mut vcx, ControlId::Plugins);
        draw(&mut vcx);
        vcx.simulate_keystrokes("down");
        draw(&mut vcx);
        assert_eq!(selected_bundle(&view, &vcx).as_deref(), Some(std::ffi::OsStr::new("dup-two")));

        vcx.update(|_, cx| crate::operations::clear_message(cx));
        vcx.simulate_keystrokes("a");
        vcx.update(|_, cx| crate::plugins::poll(cx));
        vcx.cx.run_until_parked();
        assert!(
            vcx.read_global::<crate::operations::Operations, _>(|operations, _| operations.message.is_none()),
            "a duplicate-ID row has no approvable snapshot"
        );
        let config = vcx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
        let grants = std::fs::read_to_string(config.join("plugins.toml")).unwrap_or_default();
        assert!(!grants.contains(r#"[plugin."example.dup"]"#), "{grants}");
    }

    #[gpui::test]
    fn a_grants_error_blocks_every_keyboard_action_except_reload(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings_after(cx, |cx| {
            let config = cx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
            write_named_plugin_bundle(&config, "a", "example.a", "A");
            cx.update(|cx| crate::plugins::start_for_tests(cx, config));
        });
        wait_plugin_count(&mut vcx, 1);
        let config = vcx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
        std::fs::write(
            config.join("plugins.toml"),
            "[plugin.\"example.a\"]\nenabled = true\ndigest = \"short\"\nfs = \"none\"\n",
        )
        .unwrap();
        vcx.update(|_, cx| crate::plugins::reload(cx));
        for _ in 0..500 {
            vcx.update(|_, cx| crate::plugins::poll(cx));
            vcx.run_until_parked();
            if grants_error(&vcx) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(grants_error(&vcx));
        focus_control(&view, &mut vcx, ControlId::Plugins);
        draw(&mut vcx);
        vcx.update(|_, cx| crate::operations::clear_message(cx));

        for key in ["a", "e", "d", "t"] {
            vcx.simulate_keystrokes(key);
            vcx.update(|_, cx| crate::plugins::poll(cx));
            vcx.cx.run_until_parked();
            assert!(
                vcx.read_global::<crate::operations::Operations, _>(|operations, _| operations.message.is_none()),
                "{key} must be blocked while grants are invalid"
            );
        }
        vcx.simulate_keystrokes("r");
        vcx.update(|_, cx| crate::plugins::poll(cx));
        vcx.cx.run_until_parked();
        assert!(grants_error(&vcx), "Reload is still allowed and reads the same bad file");
    }

    #[gpui::test]
    fn tips_control_writes_off_and_follows_reloads(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        focus_control(&view, &mut vcx, ControlId::Tips);
        vcx.simulate_keystrokes("right");
        wait_for(&vcx, "tips = \"off\"");
        vcx.update(|_, cx| ui::reload_config(cx));
        vcx.run_until_parked();
        assert_eq!(state(&view, &vcx, ControlId::Tips).selected, Some(1), "Off shows");
        assert_eq!(vcx.read_global::<Ui, _>(|ui, _| ui.settings.tips), TipsMode::Off);
        assert!(settings_text(&vcx).contains("hints = \"auto\""), "hints stay independent");

        vcx.simulate_keystrokes("left");
        wait_for(&vcx, "tips = \"auto\"");
        assert_eq!(state(&view, &vcx, ControlId::Tips).selected, Some(0), "Auto shows again");
    }

    fn keyboard_rows_text(view: &Entity<SettingsWindow>, vcx: &VisualTestContext) -> Vec<String> {
        view.read_with(vcx, |view, cx| view.keyboard_rows(cx).into_iter().map(|row| row.label.to_owned()).collect())
    }

    fn keys_of(vcx: &VisualTestContext, command: &str) -> Option<Vec<String>> {
        vcx.read(|cx| cx.global::<Ui>().keys.finder.get(command).cloned())
    }

    fn keyboard_row_cursor(view: &Entity<SettingsWindow>, vcx: &mut VisualTestContext, label: &str) {
        let at = keyboard_rows_text(view, vcx).iter().position(|l| l == label).expect("row exists");
        view.update(vcx, |view, _| view.keyboard.cursor = at);
        draw(vcx);
    }

    fn settle_writes(vcx: &mut VisualTestContext) {
        for _ in 0..40 {
            vcx.run_until_parked();
            vcx.cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(5));
        }
        draw(vcx);
    }

    #[gpui::test]
    fn keyboard_rows_show_each_key_once(cx: &mut TestAppContext) {
        let (_view, vcx) = open_settings(cx);
        let rows = vcx.read(|cx| crate::keymap::rows(crow_core::KeymapLayer::Finder, cx));
        for row in rows {
            let mut seen = std::collections::HashSet::new();
            assert!(row.keys.iter().all(|k| seen.insert(k.clone())), "{}: {:?}", row.label, row.keys);
        }
    }

    #[gpui::test]
    fn keyboard_list_fills_the_page_height(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        show_category(&view, &mut vcx, Category::Keyboard);
        draw(&mut vcx);
        let list = vcx.debug_bounds("settings-keyboard-list").expect("list rendered");
        let scroll = vcx.debug_bounds("settings-scroll").expect("page rendered");
        assert!(list.size.height > px(440.), "not capped at a fixed height: {:?}", list.size.height);
        let gap = scroll.bottom() - list.bottom();
        assert!(gap < px(60.), "list reaches the page bottom, gap {gap:?}");
    }

    #[gpui::test]
    fn keyboard_page_lists_every_command_and_filters_by_name_or_key(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        show_category(&view, &mut vcx, Category::Keyboard);
        let all = keyboard_rows_text(&view, &vcx);
        assert_eq!(all.len(), crate::palette::COMMANDS.len());
        view.update(&mut vcx, |view, cx| {
            view.keyboard.search.update(cx, |input, cx| input.set_text_silent("go to", cx))
        });
        view.update(&mut vcx, |view, _| view.keyboard.query = "go to".into());
        let found = keyboard_rows_text(&view, &vcx);
        assert!(found.iter().any(|l| l.contains("Go to")), "{found:?}");
        assert!(found.len() < all.len());
        view.update(&mut vcx, |view, _| view.keyboard.query = "⇧⌘g".into());
        assert!(keyboard_rows_text(&view, &vcx).iter().any(|l| l.contains("Go to Folder")), "search by key text");
        view.update(&mut vcx, |view, _| view.keyboard.query = "zzzz".into());
        assert!(keyboard_rows_text(&view, &vcx).is_empty());
    }

    #[gpui::test]
    fn clicking_a_row_and_typing_rebinds_the_command(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        show_category(&view, &mut vcx, Category::Keyboard);
        focus_control(&view, &mut vcx, ControlId::KeyboardList);
        keyboard_row_cursor(&view, &mut vcx, "View as Columns");
        vcx.simulate_keystrokes("enter");
        assert!(view.read_with(&vcx, |v, _| v.keyboard.capture.is_some()), "Return starts capture");
        vcx.simulate_keystrokes("cmd-shift-9");
        settle_writes(&mut vcx);
        assert!(view.read_with(&vcx, |v, _| v.keyboard.capture.is_none()));
        assert_eq!(keys_of(&vcx, "view-as-columns"), Some(vec!["cmd-shift-9".to_owned()]));
        let text = std::fs::read_to_string(vcx.read(|cx| cx.global::<Ui>().config_dir.join("keys.toml"))).unwrap();
        assert!(text.contains("view-as-columns"), "{text}");
    }

    #[gpui::test]
    fn capture_swallows_window_keys_and_escape_cancels(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        show_category(&view, &mut vcx, Category::Keyboard);
        focus_control(&view, &mut vcx, ControlId::KeyboardList);
        keyboard_row_cursor(&view, &mut vcx, "View as Columns");
        vcx.simulate_keystrokes("enter");
        // ⌘W would close the window, ⌘, reopens Settings; both must reach only the capture.
        vcx.simulate_keystrokes("cmd-w");
        settle_writes(&mut vcx);
        assert!(
            vcx.cx.update(|cx| cx.windows().iter().any(|h| h.downcast::<SettingsWindow>().is_some())),
            "⌘W did not close it"
        );
        assert!(view.read_with(&vcx, |v, _| v.keyboard.prompt.is_some()), "⌘W was captured as a key, not run");
    }

    #[gpui::test]
    fn reserved_keys_are_refused_with_a_reason(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        show_category(&view, &mut vcx, Category::Keyboard);
        focus_control(&view, &mut vcx, ControlId::KeyboardList);
        keyboard_row_cursor(&view, &mut vcx, "View as Columns");
        vcx.simulate_keystrokes("enter cmd-q");
        settle_writes(&mut vcx);
        let notice = view.read_with(&vcx, |v, _| v.keyboard.notice.clone());
        assert!(notice.is_some_and(|(_, text)| text.contains("quits")));
        assert_eq!(keys_of(&vcx, "view-as-columns"), None, "nothing was written");
    }

    #[gpui::test]
    fn escape_cancels_capture_and_backspace_unbinds(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        show_category(&view, &mut vcx, Category::Keyboard);
        focus_control(&view, &mut vcx, ControlId::KeyboardList);
        keyboard_row_cursor(&view, &mut vcx, "View as Columns");
        vcx.simulate_keystrokes("enter escape");
        assert!(view.read_with(&vcx, |v, _| v.keyboard.capture.is_none()));
        assert_eq!(keys_of(&vcx, "view-as-columns"), None);
        vcx.simulate_keystrokes("enter backspace");
        settle_writes(&mut vcx);
        assert_eq!(keys_of(&vcx, "view-as-columns"), Some(Vec::new()), "unbound");
    }

    #[gpui::test]
    fn a_conflicting_key_asks_and_replace_moves_it(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        show_category(&view, &mut vcx, Category::Keyboard);
        focus_control(&view, &mut vcx, ControlId::KeyboardList);
        keyboard_row_cursor(&view, &mut vcx, "View as Columns");
        vcx.simulate_keystrokes("enter cmd-2");
        settle_writes(&mut vcx);
        let asked = view.read_with(&vcx, |v, _| v.keyboard.prompt.is_some());
        assert!(asked, "⌘2 belongs to View as List");
        assert_eq!(keys_of(&vcx, "view-as-columns"), None, "nothing is written until answered");
        vcx.simulate_keystrokes("escape");
        assert!(view.read_with(&vcx, |v, _| v.keyboard.prompt.is_none()));
        vcx.simulate_keystrokes("enter cmd-2");
        vcx.simulate_keystrokes("enter");
        settle_writes(&mut vcx);
        assert_eq!(keys_of(&vcx, "view-as-columns"), Some(vec!["cmd-2".to_owned()]));
        assert_eq!(keys_of(&vcx, "view-as-list"), Some(Vec::new()), "Replace took it off View as List");
    }

    #[gpui::test]
    fn reset_restores_a_row_and_reset_all_asks_first(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        show_category(&view, &mut vcx, Category::Keyboard);
        focus_control(&view, &mut vcx, ControlId::KeyboardList);
        keyboard_row_cursor(&view, &mut vcx, "View as Columns");
        vcx.simulate_keystrokes("enter cmd-shift-9");
        settle_writes(&mut vcx);
        assert!(keys_of(&vcx, "view-as-columns").is_some());
        vcx.simulate_keystrokes("delete");
        settle_writes(&mut vcx);
        assert_eq!(keys_of(&vcx, "view-as-columns"), None, "Delete resets the row");
        vcx.simulate_keystrokes("enter cmd-shift-9");
        settle_writes(&mut vcx);
        view.update(&mut vcx, |view, cx| view.ask_reset_all(cx));
        draw(&mut vcx);
        assert!(keys_of(&vcx, "view-as-columns").is_some(), "asking does not reset");
        vcx.simulate_keystrokes("left enter");
        settle_writes(&mut vcx);
        assert_eq!(keys_of(&vcx, "view-as-columns"), None, "Reset all clears the layer");
    }

    #[gpui::test]
    fn keyboard_rows_have_roles_and_labels(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        show_category(&view, &mut vcx, Category::Keyboard);
        assert!(vcx.debug_bounds("settings-keyboard-list").is_some());
        assert!(vcx.debug_bounds("settings-keyboard-row-go-to-folder").is_some());
    }

    #[gpui::test]
    fn vim_layer_captures_a_sequence_ended_with_return(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        show_category(&view, &mut vcx, Category::Keyboard);
        focus_control(&view, &mut vcx, ControlId::KeyboardList);
        vcx.simulate_keystrokes("right");
        assert_eq!(view.read_with(&vcx, |v, _| v.keyboard.layer), KeymapLayer::Vim, "→ switches the layer");
        keyboard_row_cursor(&view, &mut vcx, "Copy");
        vcx.simulate_keystrokes("enter y c enter");
        settle_writes(&mut vcx);
        let keys = vcx.read(|cx| cx.global::<Ui>().keys.vim.get("copy").cloned());
        assert_eq!(keys, Some(vec!["y c".to_owned()]));
        assert!(vcx.read(|cx| cx.global::<Ui>().keys.finder.is_empty()), "the Finder layer is untouched");
    }

    #[gpui::test]
    fn keyboard_page_fits_a_narrow_window_and_a_broken_file_blocks_edits(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        show_category(&view, &mut vcx, Category::Keyboard);
        let dir = vcx.read(|cx| cx.global::<Ui>().config_dir.clone());
        std::fs::write(dir.join("keys.toml"), "[finder\n").unwrap();
        vcx.update(|_, cx| crate::ui::reload_config(cx));
        draw(&mut vcx);
        focus_control(&view, &mut vcx, ControlId::KeyboardList);
        vcx.simulate_keystrokes("enter");
        assert!(view.read_with(&vcx, |v, _| v.keyboard.capture.is_none()), "a broken keys.toml blocks capture");
        assert!(view.read_with(&vcx, |v, _| v.keyboard.notice.is_some()));
    }
}
