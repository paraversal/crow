//! Cmd-K's two kinds of item: commands (`>` prefix) and folders scored with
//! nucleo. Folder candidates come from favorites, Recent (frecency, zoxide
//! merged) and the current folder's subfolders; ties break by frecency.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crow_core::abbreviate_home;
use crow_search::FuzzyMatcher;
use gpui::Action;

use crate::actions::{
    AddToSidebar, CloseTab, CopyFiles, CutFiles, DeletePermanently, Duplicate, Eject, GoBack, GoForward, GoToFolder,
    MoveFavoriteDown, MoveFavoriteUp, MoveToTrash, NewFolder, NewTab, NewWindow, OpenParent, OpenSelection, PasteFiles,
    PasteMoveFiles, QuickLook, Quit, Redo, RemoveFromSidebar, Rename, ResetFolderView, SelectAll, ShowThemePicker,
    StartFilter, SwitchFocus, ToggleActivity, ToggleHiddenFiles, Undo, UseViewAsDefaults, ViewAsColumns, ViewAsList,
    WhyThisView,
};
use crate::keys::KeyScope;

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
    /// The action the palette shows (and its future menus run).
    pub action: &'static dyn Action,
    /// The context its shortcut is looked up in.
    pub scope: KeyScope,
}

/// User-facing commands, in the order an empty `>` query lists them.
pub const COMMANDS: &[Command] = &[
    Command { id: CommandId::NewTab, label: "New Tab", action: &NewTab, scope: KeyScope::Window },
    Command { id: CommandId::CloseTab, label: "Close Tab", action: &CloseTab, scope: KeyScope::Window },
    Command { id: CommandId::NextTab, label: "Next Tab", action: &crate::actions::NextTab, scope: KeyScope::Window },
    Command {
        id: CommandId::PreviousTab,
        label: "Previous Tab",
        action: &crate::actions::PreviousTab,
        scope: KeyScope::Window,
    },
    Command { id: CommandId::NewWindow, label: "New Window", action: &NewWindow, scope: KeyScope::Window },
    Command { id: CommandId::SwitchFocus, label: "Switch Focus", action: &SwitchFocus, scope: KeyScope::Window },
    Command {
        id: CommandId::ToggleHiddenFiles,
        label: "Toggle Hidden Files",
        action: &ToggleHiddenFiles,
        scope: KeyScope::Window,
    },
    Command { id: CommandId::OpenParent, label: "Go to Parent Folder", action: &OpenParent, scope: KeyScope::Window },
    Command { id: CommandId::GoBack, label: "Back", action: &GoBack, scope: KeyScope::Window },
    Command { id: CommandId::GoForward, label: "Forward", action: &GoForward, scope: KeyScope::Window },
    Command { id: CommandId::GoToFolder, label: "Go to Folder…", action: &GoToFolder, scope: KeyScope::Window },
    Command { id: CommandId::ViewAsList, label: "View as List", action: &ViewAsList, scope: KeyScope::Window },
    Command { id: CommandId::ViewAsColumns, label: "View as Columns", action: &ViewAsColumns, scope: KeyScope::Window },
    Command { id: CommandId::Open, label: "Open Selected", action: &OpenSelection, scope: KeyScope::List },
    Command { id: CommandId::QuickLook, label: "Quick Look", action: &QuickLook, scope: KeyScope::List },
    Command { id: CommandId::Rename, label: "Rename", action: &Rename, scope: KeyScope::List },
    Command { id: CommandId::NewFolder, label: "New Folder", action: &NewFolder, scope: KeyScope::List },
    Command { id: CommandId::Duplicate, label: "Duplicate", action: &Duplicate, scope: KeyScope::List },
    Command { id: CommandId::MoveToTrash, label: "Move to Trash", action: &MoveToTrash, scope: KeyScope::List },
    Command {
        id: CommandId::DeletePermanently,
        label: "Delete Permanently",
        action: &DeletePermanently,
        scope: KeyScope::List,
    },
    Command { id: CommandId::Copy, label: "Copy", action: &CopyFiles, scope: KeyScope::List },
    Command { id: CommandId::Cut, label: "Cut", action: &CutFiles, scope: KeyScope::List },
    Command { id: CommandId::Paste, label: "Paste", action: &PasteFiles, scope: KeyScope::List },
    Command { id: CommandId::PasteMove, label: "Move Here", action: &PasteMoveFiles, scope: KeyScope::List },
    Command { id: CommandId::SelectAll, label: "Select All", action: &SelectAll, scope: KeyScope::List },
    Command { id: CommandId::StartFilter, label: "Filter", action: &StartFilter, scope: KeyScope::List },
    Command { id: CommandId::Undo, label: "Undo", action: &Undo, scope: KeyScope::Window },
    Command { id: CommandId::Redo, label: "Redo", action: &Redo, scope: KeyScope::Window },
    Command { id: CommandId::AddToSidebar, label: "Add to Sidebar", action: &AddToSidebar, scope: KeyScope::Window },
    Command { id: CommandId::WhyThisView, label: "Why This View?", action: &WhyThisView, scope: KeyScope::Window },
    Command {
        id: CommandId::UseViewAsDefaults,
        label: "Use View as Defaults",
        action: &UseViewAsDefaults,
        scope: KeyScope::Window,
    },
    Command {
        id: CommandId::ResetFolderView,
        label: "Reset This Folder's View",
        action: &ResetFolderView,
        scope: KeyScope::Window,
    },
    Command { id: CommandId::ShowThemePicker, label: "Theme…", action: &ShowThemePicker, scope: KeyScope::Window },
    Command { id: CommandId::ToggleActivity, label: "Show Activity", action: &ToggleActivity, scope: KeyScope::Window },
    Command {
        id: CommandId::RemoveFromSidebar,
        label: "Remove from Sidebar",
        action: &RemoveFromSidebar,
        scope: KeyScope::Sidebar,
    },
    Command {
        id: CommandId::MoveFavoriteUp,
        label: "Move Favorite Up",
        action: &MoveFavoriteUp,
        scope: KeyScope::Sidebar,
    },
    Command {
        id: CommandId::MoveFavoriteDown,
        label: "Move Favorite Down",
        action: &MoveFavoriteDown,
        scope: KeyScope::Sidebar,
    },
    Command { id: CommandId::Eject, label: "Eject Volume", action: &Eject, scope: KeyScope::Sidebar },
    Command { id: CommandId::Quit, label: "Quit Crow", action: &Quit, scope: KeyScope::Window },
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
