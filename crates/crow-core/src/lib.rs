//! Crow's UI-free model. Everything here is plain data with unit tests;
//! `crow-app` renders it and feeds it events.

mod config;
mod exclude;
mod extensions;
mod history;
pub mod icon_pack;
pub mod motion;
mod onboarding;
mod paths;
mod recent;
mod recent_commands;
mod selection;
mod settings;
mod sidebar;
mod table;
mod tabs;
mod tips;
mod usage;
mod views;

pub use config::{abbreviate_home, cache_dir, edit_toml, expand_home, home, state_dir, write_atomic};
pub use exclude::{DEFAULT_EXCLUDE, Exclude, ExcludeSet, SetName, SetRules, exclude_path, load_exclude};
pub use extensions::{
    Align, Column, ColumnCell, ColumnContext, CommandIntent, CommandTarget, ContributionValues, Descriptor,
    DescriptorSource, EntryData, EntryHandle, EntryValue, Excerpt, ExtensionId, ExtensionKind, Extensions, HookEvent,
    InfoRow, KnownSize, LocalId, PluginId, PluginPreview, PreviewContent, Previewer, SidebarRow, Snapshot, StatusItem,
    TextPreviewer, Tint, text_excerpt,
};
pub use history::History;
pub use icon_pack::{
    ICON_PACK_API, IconColor, IconEntry, IconPack, MAX_ASSET_REF_BYTES, MAX_GLYPH_BYTES, MAX_GLYPHS, MAX_PACK_BYTES,
    NATIVE_ICON_SOURCE, is_native, validate_asset_ref, validate_source, validate_svg,
};
pub use onboarding::{ONBOARDING_FILE, OnboardingState};
pub use paths::{clean_pasted_path, complete_typed, resolve_typed};
pub use recent::{Recent, parse_zoxide, query_zoxide};
pub use recent_commands::RecentCommands;
pub use selection::{Selection, TypeSelect};
pub use settings::{
    AppIcon, DEFAULT_SETTINGS, EnterAction, FolderSizesMode, HintsMode, KeymapLayer, SearchMatch, Settings,
    SettingsError, TipsMode, ViewMode, WindowDefaults, config_dir, load_settings, save_bool_setting,
    save_number_setting, save_setting, save_table_number,
};
pub use sidebar::{
    DEFAULT_SIDEBAR, Favorite, FavoriteItem, Group, Section, SectionDestination, SectionRow, SidebarConfig,
    SidebarEdit, Slot, TwinInput, edit_sidebar, load_sidebar, sidebar_path, twin_hints,
};
pub use table::{ColumnWidth, ResizeDrag, TableLayout};
pub use tabs::{TabId, Tabs};
pub use tips::{
    TIP_COOLDOWN, TIP_SCROLL_ENTRIES, TIP_SCROLL_ROWS, TIPS_FILE, TipBook, TipId, TipState, ZeroMatchStreak,
};
pub use usage::{RETIRE_AFTER, USAGE_CAP, USAGE_FILE, Usage, idle_hint_visible};
pub use views::{
    DEFAULT_VIEWS, ResolvedView, Source, ViewChange, ViewLayer, Views, load_views, natural_ascending, reset_folder,
    save_as_defaults, save_default_columns, save_folder_change, sort_key_name,
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
