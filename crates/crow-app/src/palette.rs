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
            .filter(|command| command.palette)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn command_ids(query: &str) -> Vec<CommandId> {
        items(query, &[])
            .into_iter()
            .filter_map(|item| match item {
                Item::Command(command) => Some(command.id),
                Item::Folder { .. } => None,
            })
            .collect()
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
