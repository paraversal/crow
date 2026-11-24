//! The Settings catalog: one entry per control row ("unit") with its category,
//! group, labels, search aliases and backing keys. The category pages, search
//! results, tab order and the accessibility walk all read this, so a control
//! cannot be visible in one place and missing from another.

use super::ControlId;
use crate::icons::Glyph;

/// The sidebar's categories, in reading order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Category {
    General,
    Appearance,
    Browsing,
    Previews,
    Keyboard,
    Search,
    Plugins,
    Advanced,
}

impl Category {
    pub(super) const ALL: [Category; 8] = [
        Category::General,
        Category::Appearance,
        Category::Browsing,
        Category::Previews,
        Category::Keyboard,
        Category::Search,
        Category::Plugins,
        Category::Advanced,
    ];

    /// The stable id in the state folder.
    pub(super) fn id(self) -> &'static str {
        match self {
            Category::General => "general",
            Category::Appearance => "appearance",
            Category::Browsing => "browsing",
            Category::Previews => "previews",
            Category::Keyboard => "keyboard",
            Category::Search => "search",
            Category::Plugins => "plugins",
            Category::Advanced => "advanced",
        }
    }

    /// Reads current ids and the ids of the categories before the revamp.
    pub(super) fn from_id(id: &str) -> Option<Self> {
        match id {
            "behaviour" => Some(Category::General),
            "views-preview" => Some(Category::Browsing),
            "config-files" => Some(Category::Advanced),
            _ => Self::ALL.into_iter().find(|category| category.id() == id),
        }
    }

    /// The first category, used for a missing, corrupt or unknown state file.
    pub(super) fn first() -> Self {
        Category::General
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Category::General => "General",
            Category::Appearance => "Appearance",
            Category::Browsing => "Browsing",
            Category::Previews => "Previews",
            Category::Keyboard => "Keyboard",
            Category::Search => "Search",
            Category::Plugins => "Plugins",
            Category::Advanced => "Advanced",
        }
    }

    pub(super) fn glyph(self) -> Glyph {
        match self {
            Category::General => Glyph::Settings,
            Category::Appearance => Glyph::Palette,
            Category::Browsing => Glyph::List,
            Category::Previews => Glyph::Eye,
            Category::Keyboard => Glyph::Keyboard,
            Category::Search => Glyph::Search,
            Category::Plugins => Glyph::Puzzle,
            Category::Advanced => Glyph::FileCode,
        }
    }
}

/// One row on a category page. Consecutive units with the same `group` share
/// one card under that heading. Composite rows (`controls`) move as a unit in
/// search and keep their internal reading order in the tab walk.
pub(super) struct Unit {
    pub id: ControlId,
    pub category: Category,
    pub group: &'static str,
    pub label: &'static str,
    pub aliases: &'static [&'static str],
    /// Exact backing keys, for people coming from the TOML files.
    pub keys: &'static [&'static str],
    pub controls: &'static [ControlId],
}

const fn unit(
    id: ControlId,
    (category, group): (Category, &'static str),
    label: &'static str,
    aliases: &'static [&'static str],
    keys: &'static [&'static str],
    controls: &'static [ControlId],
) -> Unit {
    Unit { id, category, group, label, aliases, keys, controls }
}

const FILES: (Category, &str) = (Category::General, "Files");
const WINDOWS: (Category, &str) = (Category::General, "Windows");
const FEEDBACK: (Category, &str) = (Category::General, "Feedback");
const THEME: (Category, &str) = (Category::Appearance, "Theme");
const ICONS: (Category, &str) = (Category::Appearance, "Icons");
const WINDOW: (Category, &str) = (Category::Appearance, "Window");
const NEW_FOLDERS: (Category, &str) = (Category::Browsing, "New folders open as");
const SHOW: (Category, &str) = (Category::Browsing, "Show");
const PREVIEW_SCOPE: (Category, &str) = (Category::Previews, "Applies to");
const PREVIEWS: (Category, &str) = (Category::Previews, "Providers");
const KEYS: (Category, &str) = (Category::Keyboard, "Keys");
const SHORTCUTS: (Category, &str) = (Category::Keyboard, "Shortcuts");
const FIND: (Category, &str) = (Category::Search, "Find");
const SKIP: (Category, &str) = (Category::Search, "Skip");
const SAVED: (Category, &str) = (Category::Search, "Saved queries");
const PLUGINS: (Category, &str) = (Category::Plugins, "Installed");
const CONFIG: (Category, &str) = (Category::Advanced, "Config files");

/// The category pages in reading order, and the search catalog. Labels from
/// before the revamp stay as aliases so old words still find their row.
pub(super) const UNITS: &[Unit] = &[
    unit(
        ControlId::Enter,
        FILES,
        "Return key",
        &["return", "enter", "rename", "open", "what return does"],
        &["enter"],
        &[ControlId::Enter],
    ),
    unit(
        ControlId::FolderTarget,
        FILES,
        "New items go in",
        &["new folder", "new file", "paste", "selected folder", "current folder", "shown folder", "finder"],
        &["folder_target"],
        &[ControlId::FolderTarget],
    ),
    unit(
        ControlId::Terminal,
        FILES,
        "Terminal",
        &["iterm", "shell", "open terminal here", "terminal app"],
        &["terminal"],
        &[ControlId::Terminal],
    ),
    unit(
        ControlId::WindowWidth,
        WINDOWS,
        "New window size",
        &["new window", "window size", "size", "width", "height"],
        &["window.width", "window.height"],
        &[ControlId::WindowWidth, ControlId::WindowHeight],
    ),
    unit(
        ControlId::Motion,
        FEEDBACK,
        "Motion",
        &["animation", "reduce motion", "reduced", "fade", "effects"],
        &["motion"],
        &[ControlId::Motion],
    ),
    unit(
        ControlId::Sound,
        FEEDBACK,
        "Sounds",
        &["sound", "audio", "play", "feedback", "beep"],
        &["sound"],
        &[ControlId::Sound, ControlId::PlaySound],
    ),
    unit(
        ControlId::Theme,
        THEME,
        "Theme",
        &["appearance", "colors", "colour", "scheme", "dark", "light"],
        &["theme"],
        &[ControlId::Theme],
    ),
    unit(
        ControlId::FontFamily,
        THEME,
        "Font",
        &["typeface", "family", "type"],
        &["font_family"],
        &[ControlId::FontFamily],
    ),
    unit(ControlId::FontSize, THEME, "Font size", &["text size", "type size"], &["font_size"], &[ControlId::FontSize]),
    unit(
        ControlId::IconStyle,
        ICONS,
        "Icon style",
        &["icons", "outline", "colored", "default icons", "sidebar icons", "sf symbols", "glyphs"],
        &["icon_style"],
        &[ControlId::IconStyle],
    ),
    unit(
        ControlId::IconPack,
        ICONS,
        "Icon packs",
        &["icons", "glyphs", "vector icons", "icon source", "pack", "override"],
        &["icon_pack", "icon_source"],
        &[ControlId::IconPack],
    ),
    unit(
        ControlId::SidebarStyle,
        WINDOW,
        "Sidebar style",
        &["sidebar", "groups", "cards", "ledger", "accordion", "pills", "floating", "headings"],
        &["sidebar_style"],
        &[ControlId::SidebarStyle],
    ),
    unit(
        ControlId::TranslucentSidebar,
        WINDOW,
        "Translucent sidebar",
        &["vibrancy", "blur", "transparency", "transparent", "frosted"],
        &["translucent_sidebar"],
        &[ControlId::TranslucentSidebar],
    ),
    unit(
        ControlId::PathBar,
        WINDOW,
        "Path bar",
        &["breadcrumb", "text field", "explorer", "path", "bottom bar", "path field"],
        &["path_bar"],
        &[ControlId::PathBar],
    ),
    unit(
        ControlId::DefaultView,
        NEW_FOLDERS,
        "View",
        &["default view", "layout", "list", "columns", "icons", "gallery", "defaults"],
        &["views.toml view", "view", "defaults.view"],
        &[ControlId::DefaultView],
    ),
    unit(
        ControlId::DefaultSort,
        NEW_FOLDERS,
        "Sort by",
        &[
            "sort",
            "order",
            "usual order",
            "ascending",
            "descending",
            "direction",
            "name",
            "size",
            "kind",
            "modified",
            "added",
        ],
        &["sort", "ascending", "defaults.sort", "defaults.ascending"],
        &[ControlId::DefaultSort, ControlId::DefaultDirection],
    ),
    unit(
        ControlId::DefaultFoldersFirst,
        NEW_FOLDERS,
        "Keep folders on top",
        &["folders first", "group folders", "sort"],
        &["folders_first", "defaults.folders_first"],
        &[ControlId::DefaultFoldersFirst],
    ),
    unit(
        ControlId::ViewPerFolder,
        NEW_FOLDERS,
        "Remember each folder's view",
        &["per folder", "view per folder", "folder view", "folders keep their own view"],
        &["view_per_folder"],
        &[ControlId::ViewPerFolder],
    ),
    unit(
        ControlId::ShowHidden,
        SHOW,
        "Show hidden files",
        &["hidden", "dotfiles", "dot files"],
        &["show_hidden_files"],
        &[ControlId::ShowHidden],
    ),
    unit(
        ControlId::FolderSizes,
        SHOW,
        "Calculate folder sizes",
        &["folder sizes", "size column", "measure", "calculate"],
        &["folder_sizes"],
        &[ControlId::FolderSizes],
    ),
    unit(
        ControlId::ShowPreview,
        SHOW,
        "Open the right sidebar in list and column view",
        &["preview", "show preview", "right column", "right sidebar", "preview pane", "info"],
        &["show_preview"],
        &[ControlId::ShowPreview],
    ),
    unit(
        ControlId::PreviewScope,
        PREVIEW_SCOPE,
        "Applies to",
        &[
            "preview",
            "previews",
            "preview providers",
            "quick look",
            "all folders",
            "one folder",
            "path rule",
            "inherited",
        ],
        &["preview_providers"],
        &[
            ControlId::PreviewScope,
            ControlId::PreviewCurrentFolder,
            ControlId::PreviewChooseFolder,
            ControlId::PreviewRuleSelect,
            ControlId::PreviewRulePattern,
            ControlId::PreviewAddRule,
        ],
    ),
    unit(
        ControlId::PreviewList,
        PREVIEWS,
        "Providers",
        &["preview", "previews", "preview providers", "quick look", "provider", "preview order"],
        &["preview_providers"],
        &[
            ControlId::PreviewList,
            ControlId::PreviewMoveUp,
            ControlId::PreviewMoveDown,
            ControlId::PreviewRemove,
            ControlId::PreviewAdd,
            ControlId::PreviewClear,
        ],
    ),
    unit(
        ControlId::Keymap,
        KEYS,
        "Vim motions",
        &["vim", "keymap", "finder", "keys", "hjkl"],
        &["keymap"],
        &[ControlId::Keymap],
    ),
    unit(
        ControlId::Hints,
        KEYS,
        "Key hints",
        &["hints", "bottom bar", "keyboard hints", "until learned"],
        &["hints"],
        &[ControlId::Hints],
    ),
    unit(ControlId::Tips, KEYS, "Tips", &["onboarding", "first time"], &["tips"], &[ControlId::Tips]),
    unit(
        ControlId::PaletteRecent,
        KEYS,
        "Recent commands in ⌘K",
        &["palette", "recent commands", "command palette", "cmd-k", "remember recent commands"],
        &["palette_recent_commands"],
        &[ControlId::PaletteRecent],
    ),
    unit(
        ControlId::KeyboardList,
        SHORTCUTS,
        "Keyboard shortcuts",
        &["shortcuts", "keymap", "keybindings", "key bindings", "hotkeys", "rebind", "remap", "conflict"],
        &["keys.toml"],
        &[ControlId::KeyboardSearch, ControlId::KeyboardList, ControlId::KeyboardResetAll],
    ),
    unit(
        ControlId::SearchMatch,
        FIND,
        "Find looks in",
        &["names", "contents", "names and contents", "all", "search mode", "search matches"],
        &["search_match"],
        &[ControlId::SearchMatch],
    ),
    unit(
        ControlId::SearchHidden,
        FIND,
        "Include hidden items",
        &["hidden", "dotfiles", "dot files", "search hidden", "search always includes hidden files"],
        &["search_includes_hidden"],
        &[ControlId::SearchHidden],
    ),
    unit(
        ControlId::ExcludeDev,
        SKIP,
        "Developer folders",
        &["node_modules", "target", "git", "build", "skipped folders", "exclude", "skip developer folders"],
        &["exclude"],
        &[ControlId::ExcludeDev],
    ),
    unit(
        ControlId::ExcludeNames,
        SKIP,
        "Names",
        &["skipped folder names", "developer folder names", "node_modules", "add folder", "remove folder", "glob"],
        &["exclude"],
        &[ControlId::ExcludeNames],
    ),
    unit(
        ControlId::ExcludeDependencies,
        SKIP,
        "Dependency folders",
        &["vendor", "bower", "skipped folders", "exclude", "skip dependency folders"],
        &["exclude"],
        &[ControlId::ExcludeDependencies],
    ),
    unit(
        ControlId::ExcludeMedia,
        SKIP,
        "Media libraries",
        &["photos library", "imovie", "final cut", "skipped folders", "exclude", "skip media libraries"],
        &["exclude"],
        &[ControlId::ExcludeMedia],
    ),
    unit(
        ControlId::ExcludeHidden,
        SKIP,
        "Hidden folders",
        &["dot folders", "skipped folders", "exclude", "skip hidden folders"],
        &["exclude"],
        &[ControlId::ExcludeHidden],
    ),
    unit(
        ControlId::ExcludeSystem,
        SKIP,
        "System folders",
        &["library caches", "trash", "skipped folders", "exclude", "skip system folders"],
        &["exclude"],
        &[ControlId::ExcludeSystem],
    ),
    unit(
        ControlId::ExcludeBackup,
        SKIP,
        "Folders Time Machine skips",
        &["backup", "time machine", "asimeow", "tmutil", "skipped folders", "exclude"],
        &["exclude"],
        &[ControlId::ExcludeBackup],
    ),
    unit(
        ControlId::SavedQueryList,
        SAVED,
        "Saved queries",
        &["saved searches", "new saved query", "smart folders"],
        &["saved-queries.toml"],
        &[ControlId::NewSavedQuery, ControlId::SavedQueryList],
    ),
    unit(
        ControlId::Plugins,
        PLUGINS,
        "Plugins",
        &["plugin", "approval", "permissions", "enable", "disable", "reload", "retry", "git"],
        &["plugins.toml", "grants", "digest"],
        &[ControlId::Plugins],
    ),
    unit(
        ControlId::EditSettings,
        CONFIG,
        "Config files",
        &[
            "settings.toml",
            "views.toml",
            "sidebar.toml",
            "exclude.toml",
            "plugins.toml",
            "saved-queries.toml",
            "keys.toml",
            "config folder",
            "reveal",
            "open file",
        ],
        &[],
        &[
            ControlId::EditSettings,
            ControlId::OpenViewsToml,
            ControlId::OpenSidebarToml,
            ControlId::OpenExcludeToml,
            ControlId::OpenPluginsToml,
            ControlId::OpenSavedQueriesToml,
            ControlId::OpenKeysToml,
            ControlId::ShowConfig,
        ],
    ),
];

/// The units of one category in reading order.
pub(super) fn units_in(category: Category) -> impl Iterator<Item = &'static Unit> {
    UNITS.iter().filter(move |unit| unit.category == category)
}

/// The unit that owns `id`, if any.
pub(super) fn unit_for(id: ControlId) -> Option<&'static Unit> {
    UNITS.iter().find(|unit| unit.id == id || unit.controls.contains(&id))
}

/// The category page that holds `id`.
#[cfg(test)]
pub(super) fn category_of(id: ControlId) -> Option<Category> {
    unit_for(id).map(|unit| unit.category)
}

/// Every tab stop in one category, following the units and their internal order.
pub(super) fn controls_in(category: Category) -> Vec<ControlId> {
    units_in(category).flat_map(|unit| unit.controls.iter().copied()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_category_ids_open_their_new_section() {
        assert_eq!(Category::from_id("behaviour"), Some(Category::General));
        assert_eq!(Category::from_id("views-preview"), Some(Category::Browsing));
        assert_eq!(Category::from_id("config-files"), Some(Category::Advanced));
        for category in Category::ALL {
            assert_eq!(Category::from_id(category.id()), Some(category));
        }
        assert_eq!(Category::from_id("nonsense"), None);
    }

    #[test]
    fn every_category_has_units_and_each_group_is_one_run() {
        for category in Category::ALL {
            let groups: Vec<&str> = units_in(category).map(|unit| unit.group).collect();
            assert!(!groups.is_empty(), "{category:?} has rows");
            let mut seen: Vec<&str> = Vec::new();
            for (index, group) in groups.iter().enumerate() {
                if index == 0 || groups[index - 1] != *group {
                    assert!(!seen.contains(group), "{category:?}: group {group} is split in two cards");
                    seen.push(group);
                }
            }
        }
    }
}
