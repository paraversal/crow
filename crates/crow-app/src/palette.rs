//! Cmd-K's two kinds of item: commands (`>` prefix) and folders scored with
//! nucleo. Folder candidates come from favorites, Recent (frecency, zoxide
//! merged) and the current folder's subfolders; ties break by frecency.

use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};

use chrono::{Datelike as _, Local, TimeZone as _};
use crow_core::abbreviate_home;
use crow_search::FuzzyMatcher;
use gpui::{Action, SharedString};

use crate::actions::{
    AddToSidebar, CloseTab, CopyFiles, CopyPath, CutFiles, DeletePermanently, Duplicate, Eject, GoBack, GoForward,
    GoToFolder, MoveFavoriteDown, MoveFavoriteUp, MoveToTrash, NewFile, NewFolder, NewTab, NewWindow, OpenParent,
    OpenSelection, PasteFiles, PasteMoveFiles, QuickLook, Quit, Redo, RemoveFromSidebar, Rename, ResetFolderView,
    SearchEverything, SearchReveal, SelectAll, ShowCommandPalette, ShowInFinder, ShowSearch, ShowThemePicker,
    StartFilter, SwitchFocus, ToggleActivity, ToggleHiddenFiles, TogglePreview, Undo, UseViewAsDefaults, ViewAsColumns,
    ViewAsList, WhyThisView,
};
use crate::keys::KeyScope;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MenuName {
    Crow,
    File,
    Edit,
    View,
    Go,
    Window,
    Help,
}

impl MenuName {
    pub const ALL: &[MenuName] = &[
        MenuName::Crow,
        MenuName::File,
        MenuName::Edit,
        MenuName::View,
        MenuName::Go,
        MenuName::Window,
        MenuName::Help,
    ];

    pub fn title(self) -> &'static str {
        match self {
            MenuName::Crow => "Crow",
            MenuName::File => "File",
            MenuName::Edit => "Edit",
            MenuName::View => "View",
            MenuName::Go => "Go",
            MenuName::Window => "Window",
            MenuName::Help => "Help",
        }
    }
}

/// Where a command appears in the menu bar. Items in the same menu sort by `section`;
/// `menus::app_menus` puts a separator between sections.
#[derive(Clone, Copy, Debug)]
pub struct MenuSlot {
    pub menu: MenuName,
    pub section: u8,
    pub title: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandId {
    NewTab,
    CloseTab,
    NextTab,
    PreviousTab,
    NewWindow,
    SwitchFocus,
    ToggleHiddenFiles,
    TogglePreview,
    OpenParent,
    GoBack,
    GoForward,
    GoToFolder,
    ViewAsList,
    ViewAsColumns,
    Open,
    QuickLook,
    Rename,
    NewFolder,
    NewFile,
    Duplicate,
    MoveToTrash,
    DeletePermanently,
    Copy,
    Cut,
    Paste,
    PasteMove,
    CopyPath,
    ShowInFinder,
    SelectAll,
    StartFilter,
    Undo,
    Redo,
    AddToSidebar,
    ShowSearch,
    SearchEverything,
    ShowCommandPalette,
    SearchReveal,
    WhyThisView,
    UseViewAsDefaults,
    ResetFolderView,
    ShowThemePicker,
    ToggleActivity,
    RemoveFromSidebar,
    MoveFavoriteUp,
    MoveFavoriteDown,
    Eject,
    Quit,
}

#[derive(Clone, Copy, Debug)]
pub struct Command {
    pub id: CommandId,
    pub label: &'static str,
    /// The action the palette shows and the menus run.
    pub action: &'static dyn Action,
    /// The context its shortcut is looked up in.
    pub scope: KeyScope,
    /// Its place in the menu bar; `None` for palette-only commands.
    pub menu: Option<MenuSlot>,
    /// Whether an empty `>` query lists it. Menu-only commands are hidden.
    pub palette: bool,
}

impl CommandId {
    /// The stable id remembered in `palette-commands.json`: renaming a variant
    /// must not change this string.
    pub fn key(self) -> &'static str {
        match self {
            CommandId::NewTab => "new-tab",
            CommandId::CloseTab => "close-tab",
            CommandId::NextTab => "next-tab",
            CommandId::PreviousTab => "previous-tab",
            CommandId::NewWindow => "new-window",
            CommandId::SwitchFocus => "switch-focus",
            CommandId::ToggleHiddenFiles => "toggle-hidden-files",
            CommandId::TogglePreview => "toggle-preview",
            CommandId::OpenParent => "open-parent",
            CommandId::GoBack => "go-back",
            CommandId::GoForward => "go-forward",
            CommandId::GoToFolder => "go-to-folder",
            CommandId::ViewAsList => "view-as-list",
            CommandId::ViewAsColumns => "view-as-columns",
            CommandId::Open => "open",
            CommandId::QuickLook => "quick-look",
            CommandId::Rename => "rename",
            CommandId::NewFolder => "new-folder",
            CommandId::NewFile => "new-file",
            CommandId::Duplicate => "duplicate",
            CommandId::MoveToTrash => "move-to-trash",
            CommandId::DeletePermanently => "delete-permanently",
            CommandId::Copy => "copy",
            CommandId::Cut => "cut",
            CommandId::Paste => "paste",
            CommandId::PasteMove => "paste-move",
            CommandId::CopyPath => "copy-path",
            CommandId::ShowInFinder => "show-in-finder",
            CommandId::SelectAll => "select-all",
            CommandId::StartFilter => "start-filter",
            CommandId::Undo => "undo",
            CommandId::Redo => "redo",
            CommandId::AddToSidebar => "add-to-sidebar",
            CommandId::ShowSearch => "show-search",
            CommandId::SearchEverything => "search-everything",
            CommandId::ShowCommandPalette => "show-command-palette",
            CommandId::SearchReveal => "search-reveal",
            CommandId::WhyThisView => "why-this-view",
            CommandId::UseViewAsDefaults => "use-view-as-defaults",
            CommandId::ResetFolderView => "reset-folder-view",
            CommandId::ShowThemePicker => "show-theme-picker",
            CommandId::ToggleActivity => "toggle-activity",
            CommandId::RemoveFromSidebar => "remove-from-sidebar",
            CommandId::MoveFavoriteUp => "move-favorite-up",
            CommandId::MoveFavoriteDown => "move-favorite-down",
            CommandId::Eject => "eject",
            CommandId::Quit => "quit",
        }
    }

    /// The command whose key is `key`; `None` for a command this build no longer has.
    pub fn from_key(key: &str) -> Option<CommandId> {
        COMMANDS.iter().find(|command| command.id.key() == key).map(|command| command.id)
    }
}

/// User-facing commands in menu-bar order (the empty `>` palette order too).
pub const COMMANDS: &[Command] = &[
    // Crow
    Command {
        id: CommandId::Quit,
        label: "Quit Crow",
        action: &Quit,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::Crow, section: 0, title: "Quit Crow" }),
        palette: true,
    },
    // File
    Command {
        id: CommandId::NewWindow,
        label: "New Window",
        action: &NewWindow,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::File, section: 0, title: "New Window" }),
        palette: true,
    },
    Command {
        id: CommandId::NewTab,
        label: "New Tab",
        action: &NewTab,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::File, section: 0, title: "New Tab" }),
        palette: true,
    },
    Command {
        id: CommandId::NewFolder,
        label: "New Folder",
        action: &NewFolder,
        scope: KeyScope::List,
        menu: Some(MenuSlot { menu: MenuName::File, section: 0, title: "New Folder" }),
        palette: true,
    },
    Command {
        id: CommandId::NewFile,
        label: "New File",
        action: &NewFile,
        scope: KeyScope::List,
        menu: Some(MenuSlot { menu: MenuName::File, section: 0, title: "New File" }),
        palette: true,
    },
    Command {
        id: CommandId::Open,
        label: "Open Selected",
        action: &OpenSelection,
        scope: KeyScope::List,
        menu: Some(MenuSlot { menu: MenuName::File, section: 0, title: "Open" }),
        palette: true,
    },
    Command {
        id: CommandId::Rename,
        label: "Rename",
        action: &Rename,
        scope: KeyScope::List,
        menu: Some(MenuSlot { menu: MenuName::File, section: 1, title: "Rename" }),
        palette: true,
    },
    Command {
        id: CommandId::Duplicate,
        label: "Duplicate",
        action: &Duplicate,
        scope: KeyScope::List,
        menu: Some(MenuSlot { menu: MenuName::File, section: 1, title: "Duplicate" }),
        palette: true,
    },
    Command {
        id: CommandId::QuickLook,
        label: "Quick Look",
        action: &QuickLook,
        scope: KeyScope::List,
        menu: Some(MenuSlot { menu: MenuName::File, section: 1, title: "Quick Look" }),
        palette: true,
    },
    Command {
        id: CommandId::MoveToTrash,
        label: "Move to Trash",
        action: &MoveToTrash,
        scope: KeyScope::List,
        menu: Some(MenuSlot { menu: MenuName::File, section: 1, title: "Move to Trash" }),
        palette: true,
    },
    Command {
        id: CommandId::DeletePermanently,
        label: "Delete Permanently",
        action: &DeletePermanently,
        scope: KeyScope::List,
        menu: Some(MenuSlot { menu: MenuName::File, section: 1, title: "Delete Permanently…" }),
        palette: true,
    },
    Command {
        id: CommandId::AddToSidebar,
        label: "Add to Sidebar",
        action: &AddToSidebar,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::File, section: 2, title: "Add to Sidebar" }),
        palette: true,
    },
    Command {
        id: CommandId::Eject,
        label: "Eject Volume",
        action: &Eject,
        scope: KeyScope::Sidebar,
        menu: Some(MenuSlot { menu: MenuName::File, section: 2, title: "Eject" }),
        palette: true,
    },
    // Edit
    Command {
        id: CommandId::Undo,
        label: "Undo",
        action: &Undo,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::Edit, section: 0, title: "Undo" }),
        palette: true,
    },
    Command {
        id: CommandId::Redo,
        label: "Redo",
        action: &Redo,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::Edit, section: 0, title: "Redo" }),
        palette: true,
    },
    Command {
        id: CommandId::Cut,
        label: "Cut",
        action: &CutFiles,
        scope: KeyScope::List,
        menu: Some(MenuSlot { menu: MenuName::Edit, section: 1, title: "Cut" }),
        palette: true,
    },
    Command {
        id: CommandId::Copy,
        label: "Copy",
        action: &CopyFiles,
        scope: KeyScope::List,
        menu: Some(MenuSlot { menu: MenuName::Edit, section: 1, title: "Copy" }),
        palette: true,
    },
    Command {
        id: CommandId::Paste,
        label: "Paste",
        action: &PasteFiles,
        scope: KeyScope::List,
        menu: Some(MenuSlot { menu: MenuName::Edit, section: 1, title: "Paste" }),
        palette: true,
    },
    Command {
        id: CommandId::PasteMove,
        label: "Move Here",
        action: &PasteMoveFiles,
        scope: KeyScope::List,
        menu: Some(MenuSlot { menu: MenuName::Edit, section: 1, title: "Move Item Here" }),
        palette: true,
    },
    Command {
        id: CommandId::CopyPath,
        label: "Copy Path",
        action: &CopyPath,
        scope: KeyScope::List,
        menu: Some(MenuSlot { menu: MenuName::Edit, section: 1, title: "Copy Path" }),
        palette: true,
    },
    Command {
        id: CommandId::SelectAll,
        label: "Select All",
        action: &SelectAll,
        scope: KeyScope::List,
        menu: Some(MenuSlot { menu: MenuName::Edit, section: 1, title: "Select All" }),
        palette: true,
    },
    Command {
        id: CommandId::StartFilter,
        label: "Filter",
        action: &StartFilter,
        scope: KeyScope::List,
        menu: Some(MenuSlot { menu: MenuName::Edit, section: 2, title: "Filter This Folder" }),
        palette: true,
    },
    Command {
        id: CommandId::ShowSearch,
        label: "Find…",
        action: &ShowSearch,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::Edit, section: 2, title: "Find…" }),
        palette: true,
    },
    // After Find…: both score the same for "find", and ties keep this order. Its menu
    // place is set by the slot (File, after Eject), not by this position.
    Command {
        id: CommandId::ShowInFinder,
        label: "Show in Finder",
        action: &ShowInFinder,
        scope: KeyScope::List,
        menu: Some(MenuSlot { menu: MenuName::File, section: 2, title: "Show in Finder" }),
        palette: true,
    },
    Command {
        id: CommandId::SearchEverything,
        label: "Find Everything…",
        action: &SearchEverything,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::Edit, section: 2, title: "Find Everything…" }),
        palette: true,
    },
    // View
    Command {
        id: CommandId::ViewAsList,
        label: "View as List",
        action: &ViewAsList,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::View, section: 0, title: "as List" }),
        palette: true,
    },
    Command {
        id: CommandId::ViewAsColumns,
        label: "View as Columns",
        action: &ViewAsColumns,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::View, section: 0, title: "as Columns" }),
        palette: true,
    },
    Command {
        id: CommandId::WhyThisView,
        label: "Why This View?",
        action: &WhyThisView,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::View, section: 1, title: "Why This View?" }),
        palette: true,
    },
    Command {
        id: CommandId::UseViewAsDefaults,
        label: "Use View as Defaults",
        action: &UseViewAsDefaults,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::View, section: 1, title: "Use as Defaults" }),
        palette: true,
    },
    Command {
        id: CommandId::ResetFolderView,
        label: "Reset This Folder's View",
        action: &ResetFolderView,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::View, section: 1, title: "Reset View for This Folder" }),
        palette: true,
    },
    Command {
        id: CommandId::ToggleHiddenFiles,
        label: "Toggle Hidden Files",
        action: &ToggleHiddenFiles,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::View, section: 2, title: "Show Hidden Files" }),
        palette: true,
    },
    Command {
        id: CommandId::TogglePreview,
        label: "Show or Hide Preview",
        action: &TogglePreview,
        scope: KeyScope::Window,
        // menus::app_menus turns this into "Hide Preview" while the preview shows.
        menu: Some(MenuSlot { menu: MenuName::View, section: 2, title: "Show Preview" }),
        palette: true,
    },
    Command {
        id: CommandId::ToggleActivity,
        label: "Show Activity",
        action: &ToggleActivity,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::View, section: 2, title: "Show Activity" }),
        palette: true,
    },
    Command {
        id: CommandId::ShowThemePicker,
        label: "Theme…",
        action: &ShowThemePicker,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::View, section: 2, title: "Theme…" }),
        palette: true,
    },
    // Go
    Command {
        id: CommandId::GoBack,
        label: "Back",
        action: &GoBack,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::Go, section: 0, title: "Back" }),
        palette: true,
    },
    Command {
        id: CommandId::GoForward,
        label: "Forward",
        action: &GoForward,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::Go, section: 0, title: "Forward" }),
        palette: true,
    },
    Command {
        id: CommandId::OpenParent,
        label: "Go to Parent Folder",
        action: &OpenParent,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::Go, section: 0, title: "Enclosing Folder" }),
        palette: true,
    },
    Command {
        id: CommandId::ShowCommandPalette,
        label: "Go to…",
        action: &ShowCommandPalette,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::Go, section: 1, title: "Go to…" }),
        palette: false,
    },
    Command {
        id: CommandId::GoToFolder,
        label: "Go to Folder…",
        action: &GoToFolder,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::Go, section: 1, title: "Go to Folder…" }),
        palette: true,
    },
    Command {
        id: CommandId::SearchReveal,
        label: "Reveal Result",
        action: &SearchReveal,
        scope: KeyScope::SearchResults,
        menu: Some(MenuSlot { menu: MenuName::Go, section: 1, title: "Reveal Result" }),
        palette: false,
    },
    // Window
    Command {
        id: CommandId::PreviousTab,
        label: "Previous Tab",
        action: &crate::actions::PreviousTab,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::Window, section: 0, title: "Show Previous Tab" }),
        palette: true,
    },
    Command {
        id: CommandId::NextTab,
        label: "Next Tab",
        action: &crate::actions::NextTab,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::Window, section: 0, title: "Show Next Tab" }),
        palette: true,
    },
    Command {
        id: CommandId::CloseTab,
        label: "Close Tab",
        action: &CloseTab,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::Window, section: 0, title: "Close Tab" }),
        palette: true,
    },
    // Palette-only
    Command {
        id: CommandId::SwitchFocus,
        label: "Switch Focus",
        action: &SwitchFocus,
        scope: KeyScope::Window,
        menu: None,
        palette: true,
    },
    Command {
        id: CommandId::RemoveFromSidebar,
        label: "Remove from Sidebar",
        action: &RemoveFromSidebar,
        scope: KeyScope::Sidebar,
        menu: None,
        palette: true,
    },
    Command {
        id: CommandId::MoveFavoriteUp,
        label: "Move Favorite Up",
        action: &MoveFavoriteUp,
        scope: KeyScope::Sidebar,
        menu: None,
        palette: true,
    },
    Command {
        id: CommandId::MoveFavoriteDown,
        label: "Move Favorite Down",
        action: &MoveFavoriteDown,
        scope: KeyScope::Sidebar,
        menu: None,
        palette: true,
    },
];

/// Where a folder row comes from. Declaration order is the section order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Recent,
    Favorite,
    Here,
    /// A file in the typed path's folder: Return reveals it there.
    File,
}

#[derive(Clone, Debug)]
pub struct FolderCandidate {
    pub path: PathBuf,
    pub label: String,
    /// Byte offset of the folder's name inside `label`.
    pub name_start: usize,
    pub source: Source,
    /// Higher is more recent/frequent; 0 for favorites and subfolders not in Recent.
    pub frecency: f64,
    /// Seconds since the epoch, from Recent's own visits. A zoxide-only entry has none.
    pub last_visit: Option<u64>,
}

#[derive(Clone, Debug)]
pub enum Item {
    Command {
        command: Command,
        /// Matched letters as byte ranges in `command.label`.
        matched: Vec<Range<usize>>,
    },
    Folder {
        path: PathBuf,
        label: String,
        name_start: usize,
        source: Source,
        last_visit: Option<u64>,
        /// Matched letters as byte ranges in `label`.
        matched: Vec<Range<usize>>,
    },
}

impl Item {
    /// The matches inside the folder's name, shifted to name-relative bytes; a
    /// range that straddles the name's start is clipped.
    pub fn name_ranges(&self) -> Vec<Range<usize>> {
        let Item::Folder { name_start, matched, .. } = self else { return Vec::new() };
        let name_start = *name_start;
        matched
            .iter()
            .filter(|range| range.end > name_start)
            .map(|range| range.start.max(name_start) - name_start..range.end - name_start)
            .collect()
    }
}

/// One line of the list: a section header, a status line ("Loading…", a missing
/// folder) or an item, by index into `items`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Header(SharedString),
    Status(SharedString),
    Item(usize),
}

#[derive(Clone, Debug)]
pub struct Listing {
    pub rows: Vec<Row>,
    pub items: Vec<Item>,
    /// The input row's count: "5 places", "2 of 34".
    pub count: SharedString,
}

/// Empty query: at most this many Recent rows, and this many "In" rows.
const RECENT_LIMIT: usize = 8;
const HERE_LIMIT: usize = 20;
/// How many remembered commands the "Recently used" section holds.
pub const RECENT_COMMANDS: usize = 5;
/// The list never holds more than this many items, headers aside.
const TOTAL_LIMIT: usize = 50;

/// The byte offset of the last component of an abbreviated path. `~` and `/`
/// are their own names.
fn name_start(label: &str) -> usize {
    match label.rfind('/') {
        Some(i) if i + 1 < label.len() => i + 1,
        _ => 0,
    }
}

/// Favorites first, then Recent's frecency list, then the current folder's
/// subfolders. Paths appear once, under the first source that has them
/// (Favorites > Recent > Here); a favorite that is also recent takes its
/// frecency for tie-breaks.
pub fn folder_candidates(
    favorites: impl IntoIterator<Item = PathBuf>,
    recents: impl IntoIterator<Item = PathBuf>,
    subfolders: impl IntoIterator<Item = PathBuf>,
    home: &Path,
    last_visit: impl Fn(&Path) -> Option<u64>,
) -> Vec<FolderCandidate> {
    let mut seen = HashSet::new();
    let mut out: Vec<FolderCandidate> = Vec::new();
    let push = |path: PathBuf, source: Source, frecency: f64, out: &mut Vec<FolderCandidate>| {
        let label = abbreviate_home(&path, home);
        out.push(FolderCandidate {
            name_start: name_start(&label),
            label,
            source,
            frecency,
            last_visit: last_visit(&path),
            path,
        });
    };
    for path in favorites {
        if seen.insert(path.clone()) {
            push(path, Source::Favorite, 0.0, &mut out);
        }
    }
    let recents: Vec<PathBuf> = recents.into_iter().collect();
    let count = recents.len();
    for (i, path) in recents.into_iter().enumerate() {
        let frecency = (count - i) as f64;
        if seen.insert(path.clone()) {
            push(path, Source::Recent, frecency, &mut out);
        } else if let Some(existing) = out.iter_mut().find(|candidate| candidate.path == path) {
            existing.frecency = frecency;
        }
    }
    for path in subfolders {
        if seen.insert(path.clone()) {
            push(path, Source::Here, 0.0, &mut out);
        }
    }
    out
}

/// The input row's count for folders.
pub fn count_text(count: usize) -> SharedString {
    match count {
        0 => "No places".into(),
        1 => "1 place".into(),
        n => format!("{n} places").into(),
    }
}

/// "just now", "5 min ago", "2 h ago", "yesterday", "3 days ago", then "12 Oct".
pub fn relative_age(now: u64, then: u64) -> String {
    let age = now.saturating_sub(then);
    if then >= now || age < 60 {
        "just now".into()
    } else if age < 3_600 {
        format!("{} min ago", age / 60)
    } else if age < 86_400 {
        format!("{} h ago", age / 3_600)
    } else if age < 2 * 86_400 {
        "yesterday".into()
    } else if age < 7 * 86_400 {
        format!("{} days ago", age / 86_400)
    } else {
        match Local.timestamp_opt(then as i64, 0).single() {
            Some(date) => format!("{} {}", date.day(), date.format("%b")),
            None => String::new(),
        }
    }
}

/// The section title: "Recent", "Favorites" or "In <here>".
pub fn section_header(source: Source, here: &str) -> SharedString {
    match source {
        Source::Recent => "Recent".into(),
        Source::Favorite => "Favorites".into(),
        Source::Here | Source::File => format!("In {here}").into(),
    }
}

/// Folder rows for `query`. An empty query groups matches into Recent /
/// Favorites / In `<here>`; a typed query is one flat list, best score first
/// (sections would let a weak match in an earlier section outrank the name you
/// typed). Every candidate is scored; match ranges are computed only for kept rows.
pub fn folder_listing(query: &str, folders: &[FolderCandidate], here: &str) -> Listing {
    let mut items: Vec<Item> = Vec::new();
    let mut rows: Vec<Row> = Vec::new();
    match (!query.is_empty()).then(|| FuzzyMatcher::new(query)) {
        Some(mut fuzzy) => {
            let mut matched: Vec<(u32, &FolderCandidate)> =
                folders.iter().filter_map(|folder| fuzzy.score(&folder.label).map(|score| (score, folder))).collect();
            matched.sort_by(|a, b| {
                b.0.cmp(&a.0)
                    .then_with(|| b.1.frecency.total_cmp(&a.1.frecency))
                    .then_with(|| a.1.label.cmp(&b.1.label))
            });
            matched.truncate(TOTAL_LIMIT);
            for (_, folder) in matched {
                let ranges = fuzzy.indices(&folder.label).map_or_else(Vec::new, |(_, ranges)| ranges);
                items.push(folder_item(folder, ranges));
            }
            rows = (0..items.len()).map(Row::Item).collect();
        }
        None => {
            for source in [Source::Recent, Source::Favorite, Source::Here] {
                let mut group: Vec<&FolderCandidate> =
                    folders.iter().filter(|folder| folder.source == source).collect();
                // Recent ranks by frecency; Favorites keep sidebar order and In keeps listing order.
                if source == Source::Recent {
                    group.sort_by(|a, b| b.frecency.total_cmp(&a.frecency));
                }
                let cap = match source {
                    Source::Recent => RECENT_LIMIT,
                    Source::Favorite => usize::MAX,
                    Source::Here | Source::File => HERE_LIMIT,
                };
                group.truncate(cap.min(TOTAL_LIMIT.saturating_sub(items.len())));
                if group.is_empty() {
                    continue;
                }
                rows.push(Row::Header(section_header(source, here)));
                for folder in group {
                    rows.push(Row::Item(items.len()));
                    items.push(folder_item(folder, Vec::new()));
                }
            }
        }
    }
    let count = count_text(items.len());
    Listing { rows, items, count }
}

fn folder_item(folder: &FolderCandidate, matched: Vec<Range<usize>>) -> Item {
    Item::Folder {
        path: folder.path.clone(),
        label: folder.label.clone(),
        name_start: folder.name_start,
        source: folder.source,
        last_visit: folder.last_visit,
        matched,
    }
}

/// Command rows for `query`. An empty query lists every command in registry
/// order except Quit (so `>` then Return never quits; typing finds it); with
/// `show_recent`, up to five remembered commands come first under a
/// "Recently used" header and are not repeated below. A typed query is a flat
/// list, best score first.
pub fn command_listing(query: &str, recent_ids: &[CommandId], show_recent: bool) -> Listing {
    let listed: Vec<&Command> = COMMANDS.iter().filter(|command| command.palette).collect();
    let total = listed.len();
    let mut items: Vec<Item> = Vec::new();
    let mut rows: Vec<Row> = Vec::new();
    let push = |items: &mut Vec<Item>, rows: &mut Vec<Row>, command: &Command, matched: Vec<Range<usize>>| {
        rows.push(Row::Item(items.len()));
        items.push(Item::Command { command: *command, matched });
    };

    let query = query.trim();
    if query.is_empty() {
        let listed: Vec<&Command> = listed.iter().copied().filter(|command| command.id != CommandId::Quit).collect();
        let mut recent: Vec<&Command> = Vec::new();
        if show_recent {
            for id in recent_ids {
                if let Some(command) = listed.iter().find(|command| command.id == *id)
                    && !recent.iter().any(|seen| seen.id == command.id)
                {
                    recent.push(command);
                }
                if recent.len() == RECENT_COMMANDS {
                    break;
                }
            }
        }
        if !recent.is_empty() {
            rows.push(Row::Header("Recently used".into()));
            for command in &recent {
                push(&mut items, &mut rows, command, Vec::new());
            }
            if recent.len() < total {
                rows.push(Row::Header("All commands".into()));
            }
        }
        for command in &listed {
            if !recent.iter().any(|seen| seen.id == command.id) {
                push(&mut items, &mut rows, command, Vec::new());
            }
        }
        return Listing { rows, items, count: format!("{total} commands").into() };
    }

    let mut fuzzy = FuzzyMatcher::new(query);
    let mut matched: Vec<(u32, &Command)> =
        listed.iter().filter_map(|command| fuzzy.score(command.label).map(|score| (score, *command))).collect();
    matched.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    matched.truncate(TOTAL_LIMIT);
    let count = matched.len();
    for (_, command) in matched {
        let ranges = fuzzy.indices(command.label).map_or_else(Vec::new, |(_, ranges)| ranges);
        push(&mut items, &mut rows, command, ranges);
    }
    Listing { rows, items, count: format!("{count} of {total}").into() }
}

/// The path mode's parent and partial, when `query` looks like a path: `/…`,
/// `~/…` or exactly `~`, also pasted in quotes, shell-escaped or as a `file://` URL.
/// The text before the last `/` is the parent.
pub fn path_query(query: &str, cwd: &Path, home: &Path) -> Option<(PathBuf, String)> {
    let query = crow_core::clean_pasted_path(query)?;
    let (parent, partial) = match query.rfind('/') {
        Some(i) => (&query[..=i], &query[i + 1..]),
        None => (query.as_str(), ""),
    };
    Some((crow_core::resolve_typed(parent, cwd, home), partial.to_owned()))
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Files shown under a typed path: those whose name starts with the partial.
const FILE_LIMIT: usize = 5;

/// The children of `parent` matching `partial` by name. With an empty partial the
/// parent itself comes first ("this folder"), so Return goes there. Files whose
/// name starts with the partial follow the folders, except that a file named
/// exactly as typed comes first: a pasted file path lands on its file.
pub fn path_listing(parent: &Path, partial: &str, children: &[PathBuf], files: &[PathBuf], home: &Path) -> Listing {
    let mut rows = vec![Row::Header(section_header(Source::Here, &crate::style::display_name(parent)))];
    let mut items: Vec<Item> = Vec::new();
    let typed = partial.to_lowercase();
    let mut file_rows: Vec<&PathBuf> = if partial.is_empty() {
        Vec::new()
    } else {
        files.iter().filter(|path| file_name(path).to_lowercase().starts_with(&typed)).collect()
    };
    file_rows.sort_by_key(|path| {
        let name = file_name(path).to_lowercase();
        (name != typed, name)
    });
    file_rows.truncate(FILE_LIMIT);
    let exact = file_rows.first().is_some_and(|path| file_name(path).to_lowercase() == typed);
    let file_item = |path: &PathBuf| {
        let label = abbreviate_home(path, home);
        let start = name_start(&label);
        Item::Folder {
            path: path.clone(),
            // The typed prefix, counted in characters so a case change in the name can't split one.
            matched: std::iter::once(
                start
                    ..label[start..]
                        .char_indices()
                        .nth(partial.chars().count())
                        .map_or(label.len(), |(i, _)| start + i),
            )
            .collect(),
            label,
            name_start: start,
            source: Source::File,
            last_visit: None,
        }
    };
    if exact {
        rows.push(Row::Item(items.len()));
        items.push(file_item(file_rows[0]));
    }
    if partial.is_empty() {
        let label = abbreviate_home(parent, home);
        items.push(Item::Folder {
            path: parent.to_path_buf(),
            label: label.clone(),
            name_start: name_start(&label),
            source: Source::Here,
            last_visit: None,
            matched: Vec::new(),
        });
        rows.push(Row::Item(0));
    }
    let mut fuzzy = (!partial.is_empty()).then(|| FuzzyMatcher::new(partial));
    let mut matched: Vec<(u32, &PathBuf)> = children
        .iter()
        .filter_map(|path| match fuzzy.as_mut() {
            Some(fuzzy) => fuzzy.score(&file_name(path)).map(|score| (score, path)),
            None => Some((0, path)),
        })
        .collect();
    let by_name = fuzzy.is_none();
    matched.sort_by(|a, b| {
        if by_name {
            file_name(a.1).to_lowercase().cmp(&file_name(b.1).to_lowercase())
        } else {
            b.0.cmp(&a.0).then_with(|| file_name(a.1).to_lowercase().cmp(&file_name(b.1).to_lowercase()))
        }
    });
    matched.truncate(TOTAL_LIMIT.saturating_sub(items.len()));
    for (_, path) in matched {
        let label = abbreviate_home(path, home);
        let start = name_start(&label);
        let matched = match fuzzy.as_mut() {
            Some(fuzzy) => fuzzy.indices(&file_name(path)).map_or_else(Vec::new, |(_, ranges)| {
                ranges.into_iter().map(|range| (range.start + start)..(range.end + start)).collect()
            }),
            None => Vec::new(),
        };
        rows.push(Row::Item(items.len()));
        items.push(Item::Folder {
            path: path.clone(),
            label,
            name_start: start,
            source: Source::Here,
            last_visit: None,
            matched,
        });
    }
    for path in file_rows.iter().skip(usize::from(exact)) {
        if items.len() >= TOTAL_LIMIT {
            break;
        }
        rows.push(Row::Item(items.len()));
        items.push(file_item(path));
    }
    let count = count_text(items.len());
    Listing { rows, items, count }
}

/// Path mode while the parent is being listed.
pub fn loading_path_listing(parent: &Path) -> Listing {
    status_path_listing(parent, "Loading…".into())
}

/// Path mode when the parent is not a folder.
pub fn missing_path_listing(parent: &Path, home: &Path) -> Listing {
    status_path_listing(parent, format!("No folder at \"{}\"", abbreviate_home(parent, home)).into())
}

fn status_path_listing(parent: &Path, text: SharedString) -> Listing {
    let rows = vec![Row::Header(section_header(Source::Here, &crate::style::display_name(parent))), Row::Status(text)];
    Listing { rows, items: Vec::new(), count: count_text(0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(path: &str) -> PathBuf {
        PathBuf::from(path)
    }

    fn folders(favorites: &[PathBuf], recents: &[PathBuf], here: &[PathBuf]) -> Vec<FolderCandidate> {
        folder_candidates(favorites.to_vec(), recents.to_vec(), here.to_vec(), Path::new("/home/me"), |_| None)
    }

    /// Rows as `"# Header"` and labels, for order assertions.
    fn row_text(listing: &Listing) -> Vec<String> {
        listing
            .rows
            .iter()
            .map(|row| match row {
                Row::Header(text) => format!("# {text}"),
                Row::Status(text) => text.to_string(),
                Row::Item(i) => match &listing.items[*i] {
                    Item::Folder { label, .. } => label.clone(),
                    Item::Command { command, .. } => command.label.to_owned(),
                },
            })
            .collect()
    }

    #[test]
    fn path_query_splits_parent_and_partial() {
        let (cwd, home) = (Path::new("/home/me/Files"), Path::new("/home/me"));
        assert_eq!(path_query("~/Doc", cwd, home), Some((PathBuf::from("/home/me"), "Doc".into())));
        assert_eq!(path_query("~", cwd, home), Some((PathBuf::from("/home/me"), "".into())));
        assert_eq!(path_query("/", cwd, home), Some((PathBuf::from("/"), "".into())));
        assert_eq!(path_query("/home/me/x/", cwd, home), Some((PathBuf::from("/home/me/x"), "".into())));
        assert_eq!(path_query("alpha", cwd, home), None, "a plain query is not a path");
        assert_eq!(path_query("a/b", cwd, home), None);
    }

    #[test]
    fn path_listing_puts_this_folder_first() {
        let children = [p("/home/me/Files/beta"), p("/home/me/Files/alpha")];
        let parent = Path::new("/home/me/Files");
        let listing = path_listing(parent, "", &children, &[], Path::new("/home/me"));
        assert_eq!(row_text(&listing), ["# In Files", "~/Files", "~/Files/alpha", "~/Files/beta"]);
        assert_eq!(listing.items.len(), 3);
        assert_eq!(listing.count, "3 places");

        let listing = path_listing(parent, "alp", &children, &[], Path::new("/home/me"));
        assert_eq!(row_text(&listing), ["# In Files", "~/Files/alpha"]);
        assert_eq!(listing.items[0].name_ranges().first(), Some(&(0..3)), "the match is highlighted in the name");
    }

    #[test]
    fn a_file_named_as_typed_comes_first_and_others_follow_folders() {
        let parent = Path::new("/home/me/Files");
        let children = [p("/home/me/Files/license-old")];
        let files = [p("/home/me/Files/LICENSE.md"), p("/home/me/Files/LICENSE"), p("/home/me/Files/notes.txt")];
        let listing = path_listing(parent, "LICENSE", &children, &files, Path::new("/home/me"));
        assert_eq!(
            row_text(&listing),
            ["# In Files", "~/Files/LICENSE", "~/Files/license-old", "~/Files/LICENSE.md"],
            "exact file, then folders, then other files starting with the text"
        );
        let Item::Folder { source, .. } = &listing.items[0] else { panic!("a row") };
        assert_eq!(*source, Source::File);
        let listing = path_listing(parent, "", &children, &files, Path::new("/home/me"));
        assert!(!row_text(&listing).iter().any(|row| row.contains("notes")), "no files until a name is typed");
    }

    #[test]
    fn missing_parent_says_so() {
        let listing = missing_path_listing(Path::new("/home/me/nope"), Path::new("/home/me"));
        assert_eq!(row_text(&listing), ["# In nope", "No folder at \"~/nope\""]);
        assert!(listing.items.is_empty());
    }

    fn command_ids(query: &str) -> Vec<CommandId> {
        command_listing(query, &[], false)
            .items
            .into_iter()
            .filter_map(|item| match item {
                Item::Command { command, .. } => Some(command.id),
                Item::Folder { .. } => None,
            })
            .collect()
    }

    #[test]
    fn sections_follow_source_order_and_skip_empty() {
        let candidates = folders(&[p("/home/me/Files/fav")], &[p("/home/me/Files/rec")], &[p("/home/me/Files/here")]);
        let listing = folder_listing("", &candidates, "Files");
        assert_eq!(
            row_text(&listing),
            ["# Recent", "~/Files/rec", "# Favorites", "~/Files/fav", "# In Files", "~/Files/here"],
        );

        let no_here = folders(&[p("/home/me/Files/fav")], &[p("/home/me/Files/rec")], &[]);
        assert_eq!(
            row_text(&folder_listing("", &no_here, "Files")),
            ["# Recent", "~/Files/rec", "# Favorites", "~/Files/fav"]
        );
    }

    #[test]
    fn a_favorite_that_is_recent_shows_once_under_favorites() {
        let candidates =
            folders(&[p("/home/me/Files/beta")], &[p("/home/me/Files/beta"), p("/home/me/Files/alpha")], &[]);
        let listing = folder_listing("", &candidates, "Files");
        assert_eq!(row_text(&listing), ["# Recent", "~/Files/alpha", "# Favorites", "~/Files/beta"]);
        let beta = candidates.iter().find(|c| c.path == p("/home/me/Files/beta")).expect("beta");
        assert_eq!(beta.source, Source::Favorite);
        assert_eq!(beta.frecency, 2.0, "a favorite that is recent keeps its frecency");
    }

    #[test]
    fn section_caps_apply() {
        let recents: Vec<PathBuf> = (0..10).map(|i| p(&format!("/home/me/Files/rec{i}"))).collect();
        let here: Vec<PathBuf> = (0..30).map(|i| p(&format!("/home/me/Files/here{i}"))).collect();
        let candidates = folders(&[p("/home/me/Files/fav")], &recents, &here);

        let listing = folder_listing("", &candidates, "Files");
        let count = |source: Source| {
            listing.items.iter().filter(|item| matches!(item, Item::Folder { source: s, .. } if *s == source)).count()
        };
        assert_eq!(count(Source::Recent), RECENT_LIMIT);
        assert_eq!(count(Source::Favorite), 1, "every favorite is listed");
        assert_eq!(count(Source::Here), HERE_LIMIT);
        assert_eq!(listing.count, "29 places");

        // A typed query is one flat list, capped at 50.
        let recents: Vec<PathBuf> = (0..30).map(|i| p(&format!("/home/me/Files/rec{i}"))).collect();
        let favorites: Vec<PathBuf> = (0..30).map(|i| p(&format!("/home/me/Files/fav{i}"))).collect();
        let candidates = folders(&favorites, &recents, &here);
        let matched = folder_listing("Files", &candidates, "Files");
        assert_eq!(matched.items.len(), TOTAL_LIMIT);
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)] // The matched list is the point of the test.
    fn name_ranges_are_the_part_inside_the_name() {
        let candidates = folders(&[], &[p("/home/me/Projects/git")], &[]);
        let listing = folder_listing("git", &candidates, "Coding");
        let Item::Folder { label, matched, .. } = &listing.items[0] else { panic!("a folder row") };
        assert_eq!(label, "~/Projects/git");
        assert_eq!(matched.first(), Some(&(11..14)), "the path line highlights the name in place");
        assert_eq!(listing.items[0].name_ranges().first(), Some(&(0..3)), "the name line gets it shifted");

        let straddling = Item::Folder {
            path: PathBuf::new(),
            label: "~/ab/cd".into(),
            name_start: 5,
            source: Source::Recent,
            last_visit: None,
            matched: vec![3..7],
        };
        assert_eq!(straddling.name_ranges().first(), Some(&(0..2)), "a range across the name's start is clipped");
    }

    #[test]
    fn relative_age_strings() {
        use chrono::TimeZone as _;
        let at = |y, m, d, hour, minute| -> u64 {
            chrono::Local.with_ymd_and_hms(y, m, d, hour, minute, 0).unwrap().timestamp() as u64
        };
        let now = at(2026, 11, 14, 18, 0);
        assert_eq!(relative_age(now, now - 59), "just now");
        assert_eq!(relative_age(now, now - 60), "1 min ago");
        assert_eq!(relative_age(now, now - 3_600), "1 h ago");
        assert_eq!(relative_age(now, now - (24 * 3_600 - 60)), "23 h ago");
        assert_eq!(relative_age(now, now - 24 * 3_600), "yesterday");
        assert_eq!(relative_age(now, now - 6 * 24 * 3_600), "6 days ago");
        assert_eq!(relative_age(now, now - 8 * 24 * 3_600), "6 Nov");
        assert_eq!(relative_age(now, now + 60), "just now", "clock skew");
    }

    #[test]
    fn count_text_strings() {
        assert_eq!(count_text(0), "No places");
        assert_eq!(count_text(1), "1 place");
        assert_eq!(count_text(5), "5 places");
    }

    #[test]
    fn an_empty_command_query_leaves_out_quit() {
        assert!(!command_ids("").contains(&CommandId::Quit), "> then Return must not quit");
        let recent = command_listing("", &[CommandId::Quit, CommandId::NewTab], true);
        let ids: Vec<CommandId> = recent
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Command { command, .. } => Some(command.id),
                Item::Folder { .. } => None,
            })
            .collect();
        assert_eq!(ids.first(), Some(&CommandId::NewTab), "not even under Recently used");
        assert!(!ids.contains(&CommandId::Quit));
        assert!(command_ids("quit").contains(&CommandId::Quit), "typing still finds it");
    }

    #[test]
    fn command_keys_are_stable_and_unique() {
        for (i, command) in COMMANDS.iter().enumerate() {
            assert_eq!(CommandId::from_key(command.id.key()), Some(command.id), "round trip for {}", command.id.key());
            assert!(
                !COMMANDS[..i].iter().any(|other| other.id.key() == command.id.key()),
                "duplicate key {}",
                command.id.key()
            );
        }
    }

    #[test]
    fn command_ids_are_unique() {
        for (i, command) in COMMANDS.iter().enumerate() {
            assert!(!COMMANDS[..i].iter().any(|other| other.id == command.id), "duplicate command id {:?}", command.id);
        }
    }

    #[test]
    fn menu_slots_are_unique() {
        for (i, command) in COMMANDS.iter().enumerate() {
            let Some(slot) = command.menu else { continue };
            for other in &COMMANDS[..i] {
                let Some(other_slot) = other.menu else { continue };
                assert!(
                    slot.menu != other_slot.menu || slot.title != other_slot.title,
                    "duplicate menu item {:?} > {}",
                    slot.menu,
                    slot.title
                );
            }
        }
    }

    #[test]
    fn menu_sections_are_contiguous() {
        for menu in MenuName::ALL {
            let mut last = None;
            for command in COMMANDS.iter().filter(|command| command.menu.is_some_and(|slot| slot.menu == *menu)) {
                let section = command.menu.expect("filtered to menu items").section;
                if let Some(previous) = last {
                    assert!(section >= previous, "sections go backwards in {:?}", menu);
                }
                last = Some(section);
            }
        }
    }

    #[test]
    fn palette_hides_menu_only_commands() {
        let all = command_ids("");
        assert!(!all.contains(&CommandId::ShowCommandPalette), "Go to… would reopen the palette");
        assert!(!all.contains(&CommandId::SearchReveal), "Reveal Result cannot run from the palette");

        let find = command_ids("find");
        assert!(find.contains(&CommandId::ShowSearch), "Find… is listed");
        assert!(find.contains(&CommandId::SearchEverything), "Find Everything… is listed");
    }
}
