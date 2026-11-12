//! Crow's UI-free model. Everything here is plain data with unit tests;
//! `crow-app` renders it and feeds it events.

mod extensions;
mod history;
mod paths;
mod selection;
mod settings;
mod table;
mod tabs;

pub use extensions::{Align, Column, ColumnContext, Extensions, PreviewContent, Previewer, TextPreviewer};
pub use history::History;
pub use paths::{complete_typed, resolve_typed};
pub use selection::{Selection, TypeSelect};
pub use settings::{DEFAULT_SETTINGS, KeymapLayer, Settings, SettingsError, ViewMode, config_dir, load_settings};
pub use table::{ColumnWidth, ResizeDrag, TableLayout};
pub use tabs::{TabId, Tabs};

use std::path::{Path, PathBuf};

/// Natural order for names, shared with `crow-fs` so type-to-select and sorting agree.
pub use crow_fs::natural_cmp as natural_order;

/// A sidebar favorite. M5 replaces the built-in list with `sidebar.toml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Favorite {
    pub name: String,
    pub path: PathBuf,
}

/// The favorites shown until the user configures their own: the ones that exist.
pub fn default_favorites(home: &Path) -> Vec<Favorite> {
    let candidates = [
        ("Home", home.to_path_buf()),
        ("Desktop", home.join("Desktop")),
        ("Documents", home.join("Documents")),
        ("Downloads", home.join("Downloads")),
        ("Applications", PathBuf::from("/Applications")),
    ];
    candidates
        .into_iter()
        .filter(|(_, path)| path.is_dir())
        .map(|(name, path)| Favorite { name: name.to_owned(), path })
        .collect()
}
