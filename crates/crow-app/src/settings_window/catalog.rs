//! The Settings catalog: one entry per control row ("unit") with its category,
//! labels, search aliases and backing keys. The category pages, search results,
//! tab order and the accessibility walk all read this, so a control cannot be
//! visible in one place and missing from another.

use super::ControlId;

/// The sidebar's categories, in reading order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Category {
    Appearance,
    Behaviour,
    Keyboard,
    ViewsPreview,
    Search,
    Plugins,
    ConfigFiles,
}

impl Category {
    pub(super) const ALL: [Category; 7] = [
        Category::Appearance,
        Category::Behaviour,
        Category::Keyboard,
        Category::ViewsPreview,
        Category::Search,
        Category::Plugins,
        Category::ConfigFiles,
    ];

    /// The stable id in the state folder.
    pub(super) fn id(self) -> &'static str {
        match self {
            Category::Appearance => "appearance",
            Category::Behaviour => "behaviour",
            Category::Keyboard => "keyboard",
            Category::ViewsPreview => "views-preview",
            Category::Search => "search",
            Category::Plugins => "plugins",
            Category::ConfigFiles => "config-files",
        }
    }

    pub(super) fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|category| category.id() == id)
    }

    /// The first category, used for a missing, corrupt or unknown state file.
    pub(super) fn first() -> Self {
        Category::Appearance
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Category::Appearance => "Appearance",
            Category::Behaviour => "Behavior",
            Category::Keyboard => "Keyboard",
            Category::ViewsPreview => "Views & Preview",
            Category::Search => "Search",
            Category::Plugins => "Plugins",
            Category::ConfigFiles => "Config Files",
        }
    }
}

/// One row on a category page. Composite rows (`controls`) move as a unit in
/// search and keep their internal reading order in the tab walk.
pub(super) struct Unit {
    pub id: ControlId,
    pub category: Category,
    pub label: &'static str,
    pub aliases: &'static [&'static str],
    /// Exact backing keys, for people coming from the TOML files.
    pub keys: &'static [&'static str],
    pub controls: &'static [ControlId],
}

const fn unit(
    id: ControlId,
    category: Category,
    label: &'static str,
    aliases: &'static [&'static str],
    keys: &'static [&'static str],
    controls: &'static [ControlId],
) -> Unit {
    Unit { id, category, label, aliases, keys, controls }
}

/// The category pages in reading order, and the search catalog.
pub(super) const UNITS: &[Unit] = &[
    unit(
        ControlId::Theme,
        Category::Appearance,
        "Theme",
        &["appearance", "colors", "colour", "scheme", "dark", "light"],
        &["theme"],
        &[ControlId::Theme],
    ),
    unit(
        ControlId::IconStyle,
        Category::Appearance,
        "Default icons",
        &["icons", "outline", "finder", "sidebar icons", "sf symbols", "glyphs"],
        &["icon_style"],
        &[ControlId::IconStyle],
    ),
    unit(
        ControlId::IconPack,
        Category::Appearance,
        "Icon packs",
        &["icons", "glyphs", "vector icons", "icon source", "pack", "override"],
        &["icon_pack", "icon_source"],
        &[ControlId::IconPack],
    ),
    unit(
        ControlId::PathBar,
        Category::Appearance,
        "Path bar",
        &["breadcrumb", "explorer", "path", "bottom bar", "path field"],
        &["path_bar"],
        &[ControlId::PathBar],
    ),
    unit(
        ControlId::SidebarStyle,
        Category::Appearance,
        "Sidebar style",
        &["sidebar", "groups", "cards", "ledger", "accordion", "pills", "floating", "headings"],
        &["sidebar_style"],
        &[ControlId::SidebarStyle],
    ),
    unit(
        ControlId::TranslucentSidebar,
        Category::Appearance,
        "Translucent sidebar",
        &["vibrancy", "blur", "transparency", "transparent", "frosted"],
        &["translucent_sidebar"],
        &[ControlId::TranslucentSidebar],
    ),
    unit(
        ControlId::FontFamily,
        Category::Appearance,
        "Font",
        &["typeface", "family", "type"],
        &["font_family"],
        &[ControlId::FontFamily],
    ),
    unit(
        ControlId::FontSize,
        Category::Appearance,
        "Font size",
        &["text size", "type size"],
        &["font_size"],
        &[ControlId::FontSize],
    ),
    unit(
        ControlId::Keymap,
        Category::Behaviour,
        "Keymap",
        &["vim", "finder", "keys", "hjkl"],
        &["keymap"],
        &[ControlId::Keymap],
    ),
    unit(
        ControlId::Enter,
        Category::Behaviour,
        "What Return does",
        &["return", "enter", "rename", "open"],
        &["enter"],
        &[ControlId::Enter],
    ),
    unit(
        ControlId::Hints,
        Category::Behaviour,
        "Key hints",
        &["hints", "bottom bar", "keyboard hints"],
        &["hints"],
        &[ControlId::Hints],
    ),
    unit(ControlId::Tips, Category::Behaviour, "Tips", &["onboarding", "first time"], &["tips"], &[ControlId::Tips]),
    unit(
        ControlId::Terminal,
        Category::Behaviour,
        "Terminal app",
        &["iterm", "shell", "open terminal here"],
        &["terminal"],
        &[ControlId::Terminal],
    ),
    unit(
        ControlId::PaletteRecent,
        Category::Behaviour,
        "Remember recent commands in ⌘K",
        &["palette", "recent commands", "command palette", "cmd-k"],
        &["palette_recent_commands"],
        &[ControlId::PaletteRecent],
    ),
    unit(
        ControlId::Motion,
        Category::Behaviour,
        "Motion",
        &["animation", "reduce motion", "fade", "effects"],
        &["motion"],
        &[ControlId::Motion],
    ),
    unit(
        ControlId::Sound,
        Category::Behaviour,
        "Sounds",
        &["sound", "audio", "play", "feedback", "beep"],
        &["sound"],
        &[ControlId::Sound, ControlId::PlaySound],
    ),
    unit(
        ControlId::WindowWidth,
        Category::Behaviour,
        "Width",
        &["new window", "window size", "size"],
        &["window.width"],
        &[ControlId::WindowWidth],
    ),
    unit(
        ControlId::WindowHeight,
        Category::Behaviour,
        "Height",
        &["new window", "window size", "size"],
        &["window.height"],
        &[ControlId::WindowHeight],
    ),
    unit(
        ControlId::ShowHidden,
        Category::ViewsPreview,
        "Show hidden files",
        &["hidden", "dotfiles", "dot files"],
        &["show_hidden_files"],
        &[ControlId::ShowHidden],
    ),
    unit(
        ControlId::FolderSizes,
        Category::ViewsPreview,
        "Folder sizes",
        &["size column", "measure", "calculate"],
        &["folder_sizes"],
        &[ControlId::FolderSizes],
    ),
    unit(
        ControlId::KeyboardList,
        Category::Keyboard,
        "Keyboard shortcuts",
        &["shortcuts", "keymap", "keybindings", "key bindings", "hotkeys", "rebind", "remap", "conflict"],
        &["keys.toml"],
        &[ControlId::KeyboardSearch, ControlId::KeyboardList, ControlId::KeyboardResetAll],
    ),
    unit(
        ControlId::DefaultView,
        Category::ViewsPreview,
        "Default view",
        &["layout", "list", "columns", "view", "defaults"],
        &["views.toml view", "view", "defaults.view"],
        &[ControlId::DefaultView],
    ),
    unit(
        ControlId::DefaultSort,
        Category::ViewsPreview,
        "Sort by",
        &["sort", "order", "ascending", "descending", "direction", "name", "size", "kind", "modified", "added"],
        &["sort", "ascending", "defaults.sort", "defaults.ascending"],
        &[ControlId::DefaultSort, ControlId::DefaultDirection],
    ),
    unit(
        ControlId::DefaultFoldersFirst,
        Category::ViewsPreview,
        "Folders first",
        &["folders first", "group folders", "sort"],
        &["folders_first", "defaults.folders_first"],
        &[ControlId::DefaultFoldersFirst],
    ),
    unit(
        ControlId::ShowPreview,
        Category::ViewsPreview,
        "Show preview",
        &["preview", "right column", "preview pane"],
        &["show_preview"],
        &[ControlId::ShowPreview],
    ),
    unit(
        ControlId::ViewPerFolder,
        Category::ViewsPreview,
        "Folders keep their own view",
        &["per folder", "view per folder", "folder view"],
        &["view_per_folder"],
        &[ControlId::ViewPerFolder],
    ),
    unit(
        ControlId::PreviewScope,
        Category::ViewsPreview,
        "Preview providers",
        &["preview", "quick look", "provider", "preview order", "inherited"],
        &["preview_providers"],
        &[
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
        ],
    ),
    unit(
        ControlId::SearchHidden,
        Category::Search,
        "Search always includes hidden files",
        &["hidden", "dotfiles", "dot files", "search hidden"],
        &["search_includes_hidden"],
        &[ControlId::SearchHidden],
    ),
    unit(
        ControlId::SearchMatch,
        Category::Search,
        "Search matches",
        &["names", "contents", "all", "search mode"],
        &["search_match"],
        &[ControlId::SearchMatch],
    ),
    unit(
        ControlId::SavedQueryList,
        Category::Search,
        "Saved Queries",
        &["saved searches", "new saved query", "smart folders"],
        &["saved-queries.toml"],
        &[ControlId::NewSavedQuery, ControlId::SavedQueryList],
    ),
    unit(
        ControlId::Plugins,
        Category::Plugins,
        "Plugins",
        &["plugin", "approval", "permissions", "enable", "disable", "reload", "retry", "git"],
        &["plugins.toml", "grants", "digest"],
        &[ControlId::Plugins],
    ),
    unit(
        ControlId::EditSettings,
        Category::ConfigFiles,
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
