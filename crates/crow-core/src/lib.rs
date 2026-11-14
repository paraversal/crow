//! Crow's UI-free model. Everything here is plain data with unit tests;
//! `crow-app` renders it and feeds it events.

mod config;
mod exclude;
mod extensions;
mod history;
mod paths;
mod recent;
mod recent_commands;
mod selection;
mod settings;
mod sidebar;
mod table;
mod tabs;
mod usage;
mod views;

pub use config::{abbreviate_home, edit_toml, expand_home, home, state_dir, write_atomic};
pub use exclude::{DEFAULT_EXCLUDE, Exclude, ExcludeSet, SetName, SetRules, exclude_path, load_exclude};
pub use extensions::{
    Align, Column, ColumnContext, Excerpt, Extensions, PreviewContent, Previewer, TextPreviewer, text_excerpt,
};
pub use history::History;
pub use paths::{complete_typed, resolve_typed};
pub use recent::{Recent, parse_zoxide, query_zoxide};
pub use recent_commands::RecentCommands;
pub use selection::{Selection, TypeSelect};
pub use settings::{
    DEFAULT_SETTINGS, HintsMode, KeymapLayer, Settings, SettingsError, ViewMode, config_dir, load_settings,
    save_setting,
};
pub use sidebar::{
    DEFAULT_SIDEBAR, Favorite, FavoriteItem, Group, SidebarConfig, SidebarEdit, Slot, edit_sidebar, load_sidebar,
    sidebar_path,
};
pub use table::{ColumnWidth, ResizeDrag, TableLayout};
pub use tabs::{TabId, Tabs};
pub use usage::{RETIRE_AFTER, USAGE_CAP, USAGE_FILE, Usage, idle_hint_visible};
pub use views::{
    DEFAULT_VIEWS, ResolvedView, Source, ViewChange, ViewLayer, Views, load_views, natural_ascending, reset_folder,
    save_as_defaults, save_folder_change, sort_key_name,
};

/// Natural order for names, shared with `crow-fs` so type-to-select and sorting agree.
pub use crow_fs::natural_cmp as natural_order;
