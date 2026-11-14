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
    AddToSidebar, CloseTab, CopyFiles, CutFiles, DeletePermanently, Duplicate, Eject, GoBack, GoForward, GoToFolder,
    MoveFavoriteDown, MoveFavoriteUp, MoveToTrash, NewFolder, NewTab, NewWindow, OpenParent, OpenSelection, PasteFiles,
    PasteMoveFiles, QuickLook, Quit, Redo, RemoveFromSidebar, Rename, ResetFolderView, SearchEverything, SearchReveal,
    SelectAll, ShowCommandPalette, ShowSearch, ShowThemePicker, StartFilter, SwitchFocus, ToggleActivity,
    ToggleHiddenFiles, Undo, UseViewAsDefaults, ViewAsColumns, ViewAsList, WhyThisView,
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
    Duplicate,
    MoveToTrash,
    DeletePermanently,
    Copy,
    Cut,
    Paste,
    PasteMove,
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
    Command(Command),
    // The U7 I3/I4 render reads the folder fields; the model lands first.
    #[allow(dead_code)]
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
    #[allow(dead_code)] // U7 I3 renders the name line.
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

/// One line of the list: a section header or an item, by index into `items`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Header(SharedString),
    Item(usize),
}

#[derive(Clone, Debug)]
pub struct Listing {
    // U7 I3 renders headers and the count; the model lands first.
    #[allow(dead_code)]
    pub rows: Vec<Row>,
    pub items: Vec<Item>,
    /// The input row's count: "5 places", "2 of 34".
    #[allow(dead_code)]
    pub count: SharedString,
}

/// Empty query: at most this many Recent rows, and this many "In" rows.
const RECENT_LIMIT: usize = 8;
const HERE_LIMIT: usize = 20;
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
#[allow(dead_code)] // U7 I3 renders it on Recent rows.
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
        Source::Here => format!("In {here}").into(),
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
                    Source::Here => HERE_LIMIT,
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

/// Items for `query`: a leading `>` switches to commands, everything else is a
/// folder query.
pub fn listing(query: &str, folders: &[FolderCandidate], here: &str) -> Listing {
    match query.strip_prefix('>') {
        Some(commands) => command_listing(commands),
        None => folder_listing(query, folders, here),
    }
}

/// Commands matching `query`, best first; an empty query lists every command
/// in registry order.
fn command_listing(query: &str) -> Listing {
    let query = query.trim();
    let listed = || COMMANDS.iter().filter(|command| command.palette);
    let total = listed().count();
    let mut matched: Vec<(u32, &Command)> = match (!query.is_empty()).then(|| FuzzyMatcher::new(query)) {
        Some(mut fuzzy) => {
            let mut matched: Vec<(u32, &Command)> =
                listed().filter_map(|command| fuzzy.score(command.label).map(|score| (score, command))).collect();
            matched.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
            matched
        }
        None => listed().map(|command| (0, command)).collect(),
    };
    let count = if query.is_empty() { format!("{total} commands") } else { format!("{} of {total}", matched.len()) };
    let items: Vec<Item> = matched.drain(..).map(|(_, command)| Item::Command(*command)).collect();
    Listing { rows: (0..items.len()).map(Row::Item).collect(), items, count: count.into() }
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
                Row::Item(i) => match &listing.items[*i] {
                    Item::Folder { label, .. } => label.clone(),
                    Item::Command(command) => command.label.to_owned(),
                },
            })
            .collect()
    }

    fn command_ids(query: &str) -> Vec<CommandId> {
        listing(query, &[], "")
            .items
            .into_iter()
            .filter_map(|item| match item {
                Item::Command(command) => Some(command.id),
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

        // With a query, 20 per section and 50 in all.
        let recents: Vec<PathBuf> = (0..30).map(|i| p(&format!("/home/me/Files/rec{i}"))).collect();
        let favorites: Vec<PathBuf> = (0..30).map(|i| p(&format!("/home/me/Files/fav{i}"))).collect();
        let candidates = folders(&favorites, &recents, &here);
        let matched = folder_listing("Files", &candidates, "Files");
        assert_eq!(matched.items.len(), TOTAL_LIMIT);
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)] // The matched list is the point of the test.
    fn name_ranges_are_the_part_inside_the_name() {
        let candidates = folders(&[], &[p("/home/me/Files/Coding/git")], &[]);
        let listing = folder_listing("git", &candidates, "Coding");
        let Item::Folder { label, matched, .. } = &listing.items[0] else { panic!("a folder row") };
        assert_eq!(label, "~/Files/Coding/git");
        assert_eq!(matched.first(), Some(&(15..18)), "the path line highlights the name in place");
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
        let all = command_ids(">");
        assert!(!all.contains(&CommandId::ShowCommandPalette), "Go to… would reopen the palette");
        assert!(!all.contains(&CommandId::SearchReveal), "Reveal Result cannot run from the palette");

        let find = command_ids(">find");
        assert!(find.contains(&CommandId::ShowSearch), "Find… is listed");
        assert!(find.contains(&CommandId::SearchEverything), "Find Everything… is listed");
    }
}
