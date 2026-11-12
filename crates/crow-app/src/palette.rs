//! Cmd-K's two kinds of item: commands (`>` prefix) and folders scored with
//! nucleo. Folder candidates come from favorites, Recent (frecency, zoxide
//! merged) and the current folder's subfolders; ties break by frecency.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crow_core::abbreviate_home;
use crow_search::FuzzyMatcher;

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
    /// The current Finder-layer shortcut, or empty when the command has none.
    pub keys: &'static str,
}

/// User-facing commands, in the order an empty `>` query lists them.
pub const COMMANDS: &[Command] = &[
    Command { id: CommandId::NewTab, label: "New Tab", keys: "⌘T" },
    Command { id: CommandId::CloseTab, label: "Close Tab", keys: "⌘W" },
    Command { id: CommandId::NextTab, label: "Next Tab", keys: "⌃⇥" },
    Command { id: CommandId::PreviousTab, label: "Previous Tab", keys: "⌃⇧⇥" },
    Command { id: CommandId::NewWindow, label: "New Window", keys: "⌘N" },
    Command { id: CommandId::SwitchFocus, label: "Switch Focus", keys: "⇥" },
    Command { id: CommandId::ToggleHiddenFiles, label: "Toggle Hidden Files", keys: "⌘⇧." },
    Command { id: CommandId::OpenParent, label: "Go to Parent Folder", keys: "⌘↑" },
    Command { id: CommandId::GoBack, label: "Back", keys: "⌘[" },
    Command { id: CommandId::GoForward, label: "Forward", keys: "⌘]" },
    Command { id: CommandId::GoToFolder, label: "Go to Folder…", keys: "⌘⇧G" },
    Command { id: CommandId::ViewAsList, label: "View as List", keys: "⌘2" },
    Command { id: CommandId::ViewAsColumns, label: "View as Columns", keys: "⌘3" },
    Command { id: CommandId::Open, label: "Open Selected", keys: "⌘↓" },
    Command { id: CommandId::QuickLook, label: "Quick Look", keys: "Space" },
    Command { id: CommandId::Rename, label: "Rename", keys: "↩" },
    Command { id: CommandId::NewFolder, label: "New Folder", keys: "⌘⇧N" },
    Command { id: CommandId::Duplicate, label: "Duplicate", keys: "⌘D" },
    Command { id: CommandId::MoveToTrash, label: "Move to Trash", keys: "⌘⌫" },
    Command { id: CommandId::DeletePermanently, label: "Delete Permanently", keys: "⌘⌥⌫" },
    Command { id: CommandId::Copy, label: "Copy", keys: "⌘C" },
    Command { id: CommandId::Cut, label: "Cut", keys: "⌘X" },
    Command { id: CommandId::Paste, label: "Paste", keys: "⌘V" },
    Command { id: CommandId::PasteMove, label: "Move Here", keys: "⌘⌥V" },
    Command { id: CommandId::SelectAll, label: "Select All", keys: "⌘A" },
    Command { id: CommandId::StartFilter, label: "Filter", keys: "/" },
    Command { id: CommandId::Undo, label: "Undo", keys: "⌘Z" },
    Command { id: CommandId::Redo, label: "Redo", keys: "⌘⇧Z" },
    Command { id: CommandId::AddToSidebar, label: "Add to Sidebar", keys: "⌃⌘T" },
    Command { id: CommandId::WhyThisView, label: "Why This View?", keys: "⌘J" },
    Command { id: CommandId::UseViewAsDefaults, label: "Use View as Defaults", keys: "" },
    Command { id: CommandId::ResetFolderView, label: "Reset This Folder's View", keys: "" },
    Command { id: CommandId::ShowThemePicker, label: "Theme…", keys: "⌘⌥T" },
    Command { id: CommandId::ToggleActivity, label: "Show Activity", keys: "⌘⌥L" },
    Command { id: CommandId::RemoveFromSidebar, label: "Remove from Sidebar", keys: "⌘⌫" },
    Command { id: CommandId::MoveFavoriteUp, label: "Move Favorite Up", keys: "⌘⌥↑" },
    Command { id: CommandId::MoveFavoriteDown, label: "Move Favorite Down", keys: "⌘⌥↓" },
    Command { id: CommandId::Eject, label: "Eject Volume", keys: "⌘E" },
    Command { id: CommandId::Quit, label: "Quit Crow", keys: "⌘Q" },
];

#[derive(Clone, Debug)]
pub struct FolderCandidate {
    pub path: PathBuf,
    pub label: String,
    /// Higher is more recent/frequent; 0 for favorites and subfolders not in Recent.
    pub frecency: f64,
}

#[derive(Clone, Debug)]
pub enum Item {
    Command(Command),
    Folder { path: PathBuf, label: String },
}

/// Favorites first, then Recent's frecency list, then the current folder's
/// subfolders. Paths appear once; a favorite that is also recent takes its frecency.
pub fn folder_candidates(
    favorites: impl IntoIterator<Item = PathBuf>,
    recents: impl IntoIterator<Item = PathBuf>,
    subfolders: impl IntoIterator<Item = PathBuf>,
    home: &Path,
) -> Vec<FolderCandidate> {
    let mut seen = HashSet::new();
    let mut out: Vec<FolderCandidate> = Vec::new();
    for path in favorites {
        if seen.insert(path.clone()) {
            out.push(FolderCandidate { label: abbreviate_home(&path, home), frecency: 0.0, path });
        }
    }
    let recents: Vec<PathBuf> = recents.into_iter().collect();
    let count = recents.len();
    for (i, path) in recents.into_iter().enumerate() {
        let frecency = (count - i) as f64;
        if seen.insert(path.clone()) {
            out.push(FolderCandidate { label: abbreviate_home(&path, home), frecency, path });
        } else if let Some(existing) = out.iter_mut().find(|candidate| candidate.path == path) {
            existing.frecency = frecency;
        }
    }
    for path in subfolders {
        if seen.insert(path.clone()) {
            out.push(FolderCandidate { label: abbreviate_home(&path, home), frecency: 0.0, path });
        }
    }
    out
}

/// Items for `query`. A leading `>` switches to commands; an empty query lists
/// folders in candidate order, or every command after `>`.
pub fn items(query: &str, folders: &[FolderCandidate]) -> Vec<Item> {
    if let Some(commands) = query.strip_prefix('>') {
        let mut fuzzy = (!commands.trim().is_empty()).then(|| FuzzyMatcher::new(commands.trim()));
        let mut matched: Vec<(u32, &Command)> = COMMANDS
            .iter()
            .filter_map(|command| match fuzzy.as_mut() {
                Some(fuzzy) => fuzzy.score(command.label).map(|score| (score, command)),
                None => Some((0, command)),
            })
            .collect();
        matched.sort_by_key(|a| std::cmp::Reverse(a.0));
        return matched.into_iter().map(|(_, command)| Item::Command(*command)).collect();
    }
    let mut fuzzy = (!query.is_empty()).then(|| FuzzyMatcher::new(query));
    let mut matched: Vec<(u32, &FolderCandidate)> = folders
        .iter()
        .filter_map(|folder| match fuzzy.as_mut() {
            Some(fuzzy) => fuzzy.score(&folder.label).map(|score| (score, folder)),
            None => Some((0, folder)),
        })
        .collect();
    if fuzzy.is_some() {
        matched.sort_by(|a, b| {
            b.0.cmp(&a.0).then_with(|| b.1.frecency.total_cmp(&a.1.frecency)).then_with(|| a.1.label.cmp(&b.1.label))
        });
    }
    matched
        .into_iter()
        .take(50)
        .map(|(_, folder)| Item::Folder { path: folder.path.clone(), label: folder.label.clone() })
        .collect()
}
