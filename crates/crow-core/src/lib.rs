//! Crow's UI-free model. Everything here is plain data with unit tests;
//! `crow-app` renders it and feeds it events.

pub mod batch_rename;
mod config;
pub mod drag;
mod exclude;
mod extensions;
mod grid;
mod history;
pub mod icon_pack;
pub mod keymap_edit;
mod location;
pub mod motion;
mod onboarding;
mod paths;
mod recent;
mod recent_commands;
mod saved_queries;
mod selection;
mod settings;
pub mod shelf;
mod sidebar;
mod table;
mod tabs;
mod tags;
mod tips;
mod usage;
mod views;

pub use config::{
    abbreviate_home, cache_dir, edit_toml, expand_home, file_message, home, parse_error, set_value, state_dir,
    write_atomic,
};
pub use exclude::{DEFAULT_EXCLUDE, Exclude, ExcludeSet, SetName, SetRules, exclude_path, load_exclude};
pub use extensions::{
    Align, Column, ColumnCell, ColumnContext, CommandIntent, CommandTarget, ContributionValues, Descriptor,
    DescriptorSource, EntryData, EntryHandle, EntryValue, Excerpt, ExtensionId, ExtensionKind, Extensions, HookEvent,
    InfoRow, KnownSize, LocalId, PluginId, PluginPreview, PreviewContent, Previewer, SidebarRow, Snapshot, StatusItem,
    TextPreviewer, Tint, text_excerpt,
};
pub use grid::{GridGeometry, GridMetrics, GridRect, IconSize};
pub use history::History;
pub use icon_pack::{
    ICON_PACK_API, IconColor, IconEntry, IconPack, MAX_ASSET_REF_BYTES, MAX_GLYPH_BYTES, MAX_GLYPHS, MAX_PACK_BYTES,
    NATIVE_ICON_SOURCE, is_native, split_sources, toggle_source, validate_asset_ref, validate_source, validate_svg,
};
pub use location::Location;
pub use onboarding::{ONBOARDING_FILE, OnboardingState};
pub use paths::{clean_pasted_path, complete_typed, resolve_typed};
pub use recent::{Recent, parse_zoxide, query_zoxide};
pub use recent_commands::RecentCommands;
pub use saved_queries::{
    DEFAULT_SAVED_QUERIES, SavedExclusions, SavedKind, SavedMatch, SavedQueries, SavedQuery, SavedQueryId, SavedScope,
    TagId, load_saved_queries, remove_saved_query, save_saved_query, saved_queries_path,
};
pub use selection::{Selection, TypeSelect};
pub use settings::{
    DEFAULT_SETTINGS, EnterAction, FolderSizesMode, HintsMode, KeymapLayer, PathBarMode, SearchMatch, Settings,
    SettingsError, SidebarStyle, TipsMode, ViewMode, WindowDefaults, config_dir, load_settings, save_bool_setting,
    save_number_setting, save_setting, save_table_number,
};
pub use shelf::{Shelf, ShelfItem};
pub use sidebar::{
    DEFAULT_SIDEBAR, Favorite, FavoriteItem, Group, Section, SectionDestination, SectionRow, SidebarConfig,
    SidebarEdit, Slot, TwinInput, edit_sidebar, load_sidebar, sidebar_path, twin_hints,
};
pub use table::{ColumnWidth, ResizeDrag, TableLayout};
pub use tabs::{TabChanges, TabId, Tabs, tab_changes};
pub use tags::{MAX_TAG_LEN, MAX_TAGS, normalize_tag, normalize_tags, tags_contain_all, validate_tag};
pub use tips::{
    TIP_COOLDOWN, TIP_SCROLL_ENTRIES, TIP_SCROLL_ROWS, TIPS_FILE, TipBook, TipId, TipState, ZeroMatchStreak,
};
pub use usage::{RETIRE_AFTER, USAGE_CAP, USAGE_FILE, Usage, idle_hint_visible};
pub use views::{
    DEFAULT_VIEWS, DefaultsChange, ResolvedView, RuleInfo, RuleLocator, Source, ViewChange, ViewLayer, Views,
    add_preview_rule, load_views, natural_ascending, reset_folder, save_as_defaults, save_default_columns,
    save_defaults_change, save_folder_change, save_folder_preview_providers, save_rule_preview_providers,
    sort_key_name,
};

/// Natural order for names, shared with `crow-fs` so type-to-select and sorting agree.
pub use crow_fs::natural_cmp as natural_order;

/// An owned temporary folder for tests: the directory is removed when the
/// guard drops, including during unwinding.
#[cfg(test)]
pub(crate) mod test_support {
    use std::ops::Deref;
    use std::path::Path;

    pub(crate) struct TempDir(tempfile::TempDir);

    impl TempDir {
        pub(crate) fn new(prefix: &str) -> Self {
            let dir = tempfile::Builder::new().prefix(prefix).tempdir().expect("temp dir");
            Self(dir)
        }
    }

    impl Deref for TempDir {
        type Target = Path;

        fn deref(&self) -> &Path {
            self.0.path()
        }
    }

    impl AsRef<Path> for TempDir {
        fn as_ref(&self) -> &Path {
            self.0.path()
        }
    }
}
