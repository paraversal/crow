//! Cmd-K's two kinds of item: commands (`>` prefix) and folders scored with
//! nucleo. Folder candidates come from favorites, Recent (frecency, zoxide
//! merged) and the current folder's subfolders; ties break by frecency.

use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};

use chrono::{Datelike as _, Local, TimeZone as _};
use crow_search::FuzzyMatcher;
use gpui::{Action, SharedString};

use crate::actions::{
    Acknowledgements, AddToShelf, AddToSidebar, CalculateFolderSizes, ChooseTags, ClearShelf, CloseTab, CopyFiles,
    CopyPath, CopyShelfHere, CutFiles, CycleRightSidebar, DeletePermanently, Duplicate, EditPath, EditSavedSearch,
    EditSettings, Eject, EmptyTrash, ExtendToFirst, ExtendToLast, GoBack, GoForward, GoToFolder, IconCaptionLines,
    IconCaptionOff, IconSizeCompact, IconSizeLarge, IconSizeStandard, MoveFavoriteDown, MoveFavoriteUp, MoveShelfHere,
    MoveToTrash, NewFile, NewFolder, NewTab, NewWindow, OpenParent, OpenSelection, OpenSettings, OpenTerminal,
    PasteFiles, PasteMoveFiles, QuickLook, Quit, Redo, RemoveFromSidebar, RemoveSavedSearch, Rename, ResetFolderView,
    SaveSearch, SearchEverything, SearchHome, SearchReveal, SelectAll, ShowCommandPalette, ShowInFinder, ShowInfo,
    ShowSearch, ShowThemePicker, StartFilter, SwitchFocus, ToggleActivity, ToggleHiddenFiles, TogglePreview,
    ToggleSidebar, Undo, UseViewAsDefaults, ViewAsColumns, ViewAsGallery, ViewAsIcons, ViewAsList, WhyThisView,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommandId {
    NewTab,
    CloseTab,
    NextTab,
    PreviousTab,
    NewWindow,
    SwitchFocus,
    ToggleHiddenFiles,
    TogglePreview,
    ToggleSidebar,
    ShowInfo,
    CycleRightSidebar,
    CalculateFolderSizes,
    ReviewGitChanges,
    ShowGitDiff,
    ChooseListColumns,
    OpenParent,
    GoBack,
    GoForward,
    GoToFolder,
    EditPath,
    ViewAsIcons,
    ViewAsList,
    ViewAsColumns,
    ViewAsGallery,
    IconSizeCompact,
    IconSizeStandard,
    IconSizeLarge,
    IconCaptionOff,
    IconCaptionLines,
    Open,
    QuickLook,
    Rename,
    NewFolder,
    NewFile,
    Duplicate,
    MoveToTrash,
    DeletePermanently,
    EmptyTrash,
    Copy,
    Cut,
    Paste,
    PasteMove,
    CopyPath,
    ShowInFinder,
    OpenTerminal,
    SelectAll,
    ExtendSelectionToStart,
    ExtendSelectionToEnd,
    StartFilter,
    ChooseTags,
    Undo,
    Redo,
    AddToSidebar,
    AddToShelf,
    MoveShelfHere,
    CopyShelfHere,
    ClearShelf,
    ShowSearch,
    SearchEverything,
    SearchHome,
    ShowCommandPalette,
    SearchReveal,
    SaveSearch,
    EditSavedSearch,
    RemoveSavedSearch,
    WhyThisView,
    UseViewAsDefaults,
    ResetFolderView,
    ShowThemePicker,
    OpenSettings,
    EditSettings,
    Acknowledgements,
    ToggleActivity,
    RemoveFromSidebar,
    MoveFavoriteUp,
    MoveFavoriteDown,
    Eject,
    AddProjectRoot,
    RemoveProjectRoot,
    RefreshProjects,
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

impl Command {
    pub fn listed(&self, folder_sizes_visible: bool, virtual_destination: bool) -> bool {
        self.palette
            && !(folder_sizes_visible && self.id == CommandId::CalculateFolderSizes)
            && !(virtual_destination
                && matches!(
                    self.id,
                    CommandId::ViewAsIcons
                        | CommandId::ViewAsList
                        | CommandId::ViewAsColumns
                        | CommandId::ViewAsGallery
                ))
    }
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
            CommandId::ToggleSidebar => "toggle-sidebar",
            CommandId::ShowInfo => "show-info",
            CommandId::CycleRightSidebar => "cycle-right-sidebar",
            CommandId::CalculateFolderSizes => "calculate-folder-sizes",
            CommandId::ReviewGitChanges => "review-git-changes",
            CommandId::ShowGitDiff => "show-git-diff",
            CommandId::ChooseListColumns => "choose-list-columns",
            CommandId::OpenParent => "open-parent",
            CommandId::GoBack => "go-back",
            CommandId::GoForward => "go-forward",
            CommandId::GoToFolder => "go-to-folder",
            CommandId::EditPath => "edit-path",
            CommandId::ViewAsIcons => "view-as-icons",
            CommandId::ViewAsList => "view-as-list",
            CommandId::ViewAsColumns => "view-as-columns",
            CommandId::ViewAsGallery => "view-as-gallery",
            CommandId::IconSizeCompact => "icon-size-compact",
            CommandId::IconSizeStandard => "icon-size-standard",
            CommandId::IconSizeLarge => "icon-size-large",
            CommandId::IconCaptionOff => "icon-caption-off",
            CommandId::IconCaptionLines => "icon-caption-lines",
            CommandId::Open => "open",
            CommandId::QuickLook => "quick-look",
            CommandId::Rename => "rename",
            CommandId::NewFolder => "new-folder",
            CommandId::NewFile => "new-file",
            CommandId::Duplicate => "duplicate",
            CommandId::MoveToTrash => "move-to-trash",
            CommandId::DeletePermanently => "delete-permanently",
            CommandId::EmptyTrash => "empty-trash",
            CommandId::Copy => "copy",
            CommandId::Cut => "cut",
            CommandId::Paste => "paste",
            CommandId::PasteMove => "paste-move",
            CommandId::CopyPath => "copy-path",
            CommandId::ShowInFinder => "show-in-finder",
            CommandId::OpenTerminal => "open-terminal",
            CommandId::SelectAll => "select-all",
            CommandId::ExtendSelectionToStart => "extend-selection-to-start",
            CommandId::ExtendSelectionToEnd => "extend-selection-to-end",
            CommandId::StartFilter => "start-filter",
            CommandId::ChooseTags => "choose-tags",
            CommandId::Undo => "undo",
            CommandId::Redo => "redo",
            CommandId::AddToSidebar => "add-to-sidebar",
            CommandId::AddToShelf => "add-to-shelf",
            CommandId::MoveShelfHere => "move-shelf-here",
            CommandId::CopyShelfHere => "copy-shelf-here",
            CommandId::ClearShelf => "clear-shelf",
            CommandId::ShowSearch => "show-search",
            CommandId::SearchEverything => "search-everything",
            CommandId::SearchHome => "search-home",
            CommandId::ShowCommandPalette => "show-command-palette",
            CommandId::SearchReveal => "search-reveal",
            CommandId::SaveSearch => "save-search",
            CommandId::EditSavedSearch => "edit-saved-search",
            CommandId::RemoveSavedSearch => "remove-saved-search",
            CommandId::WhyThisView => "why-this-view",
            CommandId::UseViewAsDefaults => "use-view-as-defaults",
            CommandId::ResetFolderView => "reset-folder-view",
            CommandId::ShowThemePicker => "show-theme-picker",
            CommandId::OpenSettings => "open-settings",
            CommandId::EditSettings => "edit-settings",
            CommandId::Acknowledgements => "acknowledgements",
            CommandId::ToggleActivity => "toggle-activity",
            CommandId::RemoveFromSidebar => "remove-from-sidebar",
            CommandId::MoveFavoriteUp => "move-favorite-up",
            CommandId::MoveFavoriteDown => "move-favorite-down",
            CommandId::Eject => "eject",
            CommandId::AddProjectRoot => "add-project-root",
            CommandId::RemoveProjectRoot => "remove-project-root",
            CommandId::RefreshProjects => "refresh-projects",
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
        id: CommandId::EmptyTrash,
        label: "Empty Trash…",
        action: &EmptyTrash,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::File, section: 1, title: "Empty Trash…" }),
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
    Command {
        id: CommandId::ReviewGitChanges,
        label: "Review Git Changes",
        action: &crate::actions::ReviewGitChanges,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::File, section: 2, title: "Review Git Changes…" }),
        palette: true,
    },
    Command {
        id: CommandId::AddProjectRoot,
        label: "Set Current Folder as Projects Root",
        action: &crate::actions::AddProjectRoot,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::File, section: 2, title: "Set Folder as Projects Root" }),
        palette: true,
    },
    Command {
        id: CommandId::RemoveProjectRoot,
        label: "Clear Projects Root",
        action: &crate::actions::RemoveProjectRoot,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::File, section: 2, title: "Clear Projects Root" }),
        palette: true,
    },
    Command {
        id: CommandId::RefreshProjects,
        label: "Refresh Projects",
        action: &crate::actions::RefreshProjects,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::File, section: 2, title: "Refresh Projects" }),
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
        id: CommandId::ExtendSelectionToStart,
        label: "Extend Selection to Start",
        action: &ExtendToFirst,
        scope: KeyScope::List,
        menu: Some(MenuSlot { menu: MenuName::Edit, section: 1, title: "Extend Selection to Start" }),
        palette: true,
    },
    Command {
        id: CommandId::ExtendSelectionToEnd,
        label: "Extend Selection to End",
        action: &ExtendToLast,
        scope: KeyScope::List,
        menu: Some(MenuSlot { menu: MenuName::Edit, section: 1, title: "Extend Selection to End" }),
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
        id: CommandId::ChooseTags,
        label: "Filter by Tag…",
        action: &ChooseTags,
        scope: KeyScope::List,
        menu: Some(MenuSlot { menu: MenuName::Edit, section: 2, title: "Filter by Tag…" }),
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
        id: CommandId::OpenTerminal,
        label: "Open Terminal Here",
        action: &OpenTerminal,
        scope: KeyScope::List,
        menu: Some(MenuSlot { menu: MenuName::File, section: 2, title: "Open Terminal Here" }),
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
    Command {
        id: CommandId::SearchHome,
        label: "Search Home",
        action: &SearchHome,
        scope: KeyScope::Search,
        menu: Some(MenuSlot { menu: MenuName::Edit, section: 2, title: "Search Home" }),
        palette: true,
    },
    // View
    Command {
        id: CommandId::ViewAsIcons,
        label: "View as Icons",
        action: &ViewAsIcons,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::View, section: 0, title: "as Icons" }),
        palette: true,
    },
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
        id: CommandId::ViewAsGallery,
        label: "View as Gallery",
        action: &ViewAsGallery,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::View, section: 0, title: "as Gallery" }),
        palette: true,
    },
    Command {
        id: CommandId::IconSizeCompact,
        label: "Icon Size: Compact",
        action: &IconSizeCompact,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::View, section: 1, title: "Icon Size: Compact" }),
        palette: true,
    },
    Command {
        id: CommandId::IconSizeStandard,
        label: "Icon Size: Standard",
        action: &IconSizeStandard,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::View, section: 1, title: "Icon Size: Standard" }),
        palette: true,
    },
    Command {
        id: CommandId::IconSizeLarge,
        label: "Icon Size: Large",
        action: &IconSizeLarge,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::View, section: 1, title: "Icon Size: Large" }),
        palette: true,
    },
    Command {
        id: CommandId::IconCaptionOff,
        label: "Icon Caption: Off",
        action: &IconCaptionOff,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::View, section: 1, title: "Icon Caption: Off" }),
        palette: true,
    },
    Command {
        id: CommandId::IconCaptionLines,
        label: "Icon Caption: Lines Changed",
        action: &IconCaptionLines,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::View, section: 1, title: "Icon Caption: Lines Changed" }),
        palette: true,
    },
    Command {
        id: CommandId::ChooseListColumns,
        label: "Choose List Columns",
        action: &crate::actions::ChooseListColumns,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::View, section: 1, title: "Choose List Columns…" }),
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
        label: "Show or Hide Right Sidebar",
        action: &TogglePreview,
        scope: KeyScope::Window,
        // menus::app_menus turns this into "Hide Right Sidebar" while it shows.
        menu: Some(MenuSlot { menu: MenuName::View, section: 2, title: "Show Right Sidebar" }),
        palette: true,
    },
    Command {
        id: CommandId::ToggleSidebar,
        label: "Show or Hide Left Sidebar",
        action: &ToggleSidebar,
        scope: KeyScope::Window,
        // menus::app_menus turns this into "Hide Left Sidebar" while the sidebar shows.
        menu: Some(MenuSlot { menu: MenuName::View, section: 2, title: "Show Left Sidebar" }),
        palette: true,
    },
    Command {
        id: CommandId::ShowInfo,
        label: "Show Info",
        action: &ShowInfo,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::View, section: 2, title: "Show Info" }),
        palette: true,
    },
    Command {
        id: CommandId::CycleRightSidebar,
        label: "Cycle Right Sidebar",
        action: &CycleRightSidebar,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::View, section: 2, title: "Cycle Right Sidebar" }),
        palette: true,
    },
    Command {
        id: CommandId::ShowGitDiff,
        label: "Show Git Diff",
        action: &crate::actions::ShowGitDiff,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::View, section: 2, title: "Show Git Diff" }),
        palette: true,
    },
    Command {
        id: CommandId::CalculateFolderSizes,
        label: "Calculate Folder Sizes",
        action: &CalculateFolderSizes,
        scope: KeyScope::List,
        menu: Some(MenuSlot { menu: MenuName::View, section: 2, title: "Calculate Folder Sizes" }),
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
    Command {
        id: CommandId::OpenSettings,
        label: "Settings",
        action: &OpenSettings,
        scope: KeyScope::Window,
        menu: None,
        palette: true,
    },
    Command {
        id: CommandId::EditSettings,
        label: "Edit settings.toml",
        action: &EditSettings,
        scope: KeyScope::Window,
        menu: None,
        palette: true,
    },
    Command {
        id: CommandId::Acknowledgements,
        label: "Acknowledgements",
        action: &Acknowledgements,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::Help, section: 0, title: "Acknowledgements" }),
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
        id: CommandId::EditPath,
        label: "Edit Path",
        action: &EditPath,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::Go, section: 1, title: "Edit Path" }),
        palette: true,
    },
    Command {
        id: CommandId::SaveSearch,
        label: "Save Search…",
        action: &SaveSearch,
        scope: KeyScope::Search,
        menu: Some(MenuSlot { menu: MenuName::File, section: 5, title: "Save Search…" }),
        palette: true,
    },
    Command {
        id: CommandId::EditSavedSearch,
        label: "Edit Saved Search…",
        action: &EditSavedSearch,
        scope: KeyScope::Sidebar,
        menu: None,
        palette: true,
    },
    Command {
        id: CommandId::RemoveSavedSearch,
        label: "Remove Saved Search",
        action: &RemoveSavedSearch,
        scope: KeyScope::Sidebar,
        menu: None,
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
    Command {
        id: CommandId::AddToShelf,
        label: "Add to Shelf",
        action: &AddToShelf,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::File, section: 6, title: "Add to Shelf" }),
        palette: true,
    },
    Command {
        id: CommandId::MoveShelfHere,
        label: "Move Shelf Here",
        action: &MoveShelfHere,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::File, section: 6, title: "Move Shelf Here" }),
        palette: true,
    },
    Command {
        id: CommandId::CopyShelfHere,
        label: "Copy Shelf Here",
        action: &CopyShelfHere,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::File, section: 6, title: "Copy Shelf Here" }),
        palette: true,
    },
    Command {
        id: CommandId::ClearShelf,
        label: "Clear Shelf",
        action: &ClearShelf,
        scope: KeyScope::Window,
        menu: Some(MenuSlot { menu: MenuName::File, section: 6, title: "Clear Shelf" }),
        palette: true,
    },
];

/// Where a folder row comes from. Declaration order is the section order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Recent,
    Favorite,
    Project,
    Here,
    /// A file in the typed path's folder: Return reveals it there.
    File,
}

#[derive(Clone, Debug)]
pub struct FolderCandidate {
    pub path: PathBuf,
    pub label: String,
    /// The folder's own name for the row's first line. The home folder shows the
    /// account's short name instead of `~`.
    pub name: String,
    /// Byte offset of the name inside `label`; `None` when the name is not a
    /// suffix of the label (the home folder).
    pub name_start: Option<usize>,
    pub source: Source,
    /// Higher is more recent/frequent; 0 for favorites and subfolders not in Recent.
    pub frecency: f64,
    /// A dot name or the Finder hidden flag; dimmed and demoted like search rows.
    pub hidden: bool,
    /// Seconds since the epoch, from Recent's own visits. A zoxide-only entry has none.
    pub last_visit: Option<u64>,
}

/// One Running plugin command offered by the palette. `title` is the
/// descriptor title; `package` is the display name shown on the second line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginCommandItem {
    pub extension: crow_core::ExtensionId,
    pub generation: u64,
    pub title: String,
    pub package: String,
}

#[derive(Clone, Debug)]
pub enum Item {
    Command {
        command: Command,
        /// Matched letters as byte ranges in `command.label`.
        matched: Vec<Range<usize>>,
    },
    PluginCommand {
        command: PluginCommandItem,
        /// Matched letters as byte ranges in `command.title`.
        matched: Vec<Range<usize>>,
    },
    Folder {
        path: PathBuf,
        label: String,
        /// The row's first line; the home folder uses the account's short name.
        name: String,
        /// Byte offset of the name inside `label`; `None` for the home folder.
        name_start: Option<usize>,
        source: Source,
        /// A dot name or the Finder hidden flag; drawn dimmed, spoken as "hidden".
        hidden: bool,
        last_visit: Option<u64>,
        /// Matched letters as byte ranges in `label`.
        matched: Vec<Range<usize>>,
        /// Matched letters as byte ranges in `name`, when the name is not part of
        /// the label (the home row shows the account name but its path is `~`).
        name_matched: Vec<Range<usize>>,
    },
}

impl Item {
    /// The matches inside the folder's name, shifted to name-relative bytes; a
    /// range that straddles the name's start is clipped. A name that is not part
    /// of the label (the home row) carries its own ranges.
    pub fn name_ranges(&self) -> Vec<Range<usize>> {
        let Item::Folder { name_start, matched, name_matched, .. } = self else { return Vec::new() };
        if !name_matched.is_empty() {
            return name_matched.clone();
        }
        let Some(name_start) = *name_start else { return Vec::new() };
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

/// The name line for a folder. The home folder shows the account's short name
/// (Finder's sidebar does the same) instead of the label's `~`; its path line
/// stays `~`, so the name is not a suffix of the label.
fn folder_name(path: &Path, label: &str, home: &Path) -> (String, Option<usize>) {
    if path == home {
        let name = crate::style::display_name(home);
        (if name.is_empty() { "Home".to_owned() } else { name }, None)
    } else {
        let start = name_start(label);
        (label[start..].to_owned(), Some(start))
    }
}

/// Favorites first, then Recent's frecency list, then the current folder's
/// subfolders. Paths appear once, under the first source that has them
/// (Favorites > Recent > Here); a favorite that is also recent takes its
/// frecency for tie-breaks.
pub fn folder_candidates(
    favorites: impl IntoIterator<Item = PathBuf>,
    recents: impl IntoIterator<Item = PathBuf>,
    projects: impl IntoIterator<Item = PathBuf>,
    subfolders: impl IntoIterator<Item = PathBuf>,
    home: &Path,
    last_visit: impl Fn(&Path) -> Option<u64>,
) -> Vec<FolderCandidate> {
    let mut seen = HashSet::new();
    let mut out: Vec<FolderCandidate> = Vec::new();
    let push = |path: PathBuf, source: Source, frecency: f64, out: &mut Vec<FolderCandidate>| {
        let label = crate::style::display_path(&path, home);
        let (name, name_start) = folder_name(&path, &label, home);
        // A dot name is the part a listing can know without a stat; the palette
        // candidates come from paths, not `crow_fs::Entry`.
        let hidden = path.file_name().is_some_and(|name| name.to_string_lossy().starts_with('.'));
        out.push(FolderCandidate {
            name,
            name_start,
            label,
            source,
            frecency,
            hidden,
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
    for path in projects {
        if seen.insert(path.clone()) {
            push(path, Source::Project, 0.0, &mut out);
        }
    }
    for path in subfolders {
        if seen.insert(path.clone()) {
            push(path, Source::Here, 0.0, &mut out);
        }
    }
    out
}

/// A hidden folder found by the palette's own listing of the current folder
/// (the pane's subfolders leave hidden ones out while Show Hidden Files is off).
pub fn hidden_folder_candidate(path: &Path, hidden: bool, home: &Path) -> FolderCandidate {
    let label = crate::style::display_path(path, home);
    let (name, name_start) = folder_name(path, &label, home);
    FolderCandidate {
        path: path.to_path_buf(),
        label,
        name,
        name_start,
        source: Source::Here,
        frecency: 0.0,
        hidden,
        last_visit: None,
    }
}

/// The input row's count for folders.
pub fn count_text(count: usize) -> SharedString {
    match count {
        0 => "No places".into(),
        1 => "1 place".into(),
        n => format!("{n} places").into(),
    }
}

/// The input row's count when files are listed too: "3 places · 2 files".
/// Files are not places; an empty query has none.
pub fn count_text_with_files(places: usize, files: usize) -> SharedString {
    let places = count_text(places);
    match files {
        0 => places,
        1 => format!("{places} · 1 file").into(),
        n => format!("{places} · {n} files").into(),
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
        Source::Project => "Projects".into(),
        Source::Here | Source::File => format!("In {here}").into(),
    }
}

/// Folder rows for `query`. An empty query groups matches into Recent /
/// Favorites / In `<here>`; a typed query is one flat list, best score first
/// (sections would let a weak match in an earlier section outrank the name you
/// typed). Every candidate is scored; match ranges are computed only for kept rows.
pub fn folder_listing(
    query: &str,
    folders: &[FolderCandidate],
    here: &str,
    files: &[PathEntry],
    home: &Path,
    hidden: HiddenMatch,
    file_candidates: Option<&[u32]>,
) -> Listing {
    let mut items: Vec<Item> = Vec::new();
    let mut rows: Vec<Row> = Vec::new();
    let mut file_count = 0usize;
    match (!query.is_empty()).then(|| FuzzyMatcher::new(query)) {
        Some(mut fuzzy) => {
            let needle = query.to_lowercase();
            // Files in the current folder follow the folders; the typed query is
            // fuzzy, and the incremental narrowing keeps its per-keystroke cost down.
            let file_matches = if files.is_empty() {
                Vec::new()
            } else {
                match_entries(query, files, hidden, file_candidates, TYPED_FILE_LIMIT)
            };
            // The home row's name is not part of its label (`~`), so score both; every
            // other name is a suffix of the label, and scoring it twice would change
            // the ranking and double the work on a folder with 100k subfolders.
            let mut matched: Vec<(u32, &FolderCandidate, bool)> = folders
                .iter()
                .filter(|folder| hidden != HiddenMatch::Exclude || !folder.hidden)
                .filter_map(|folder| {
                    let label = fuzzy.score(&folder.label).map(|score| (score, folder, false));
                    let name = folder
                        .name_start
                        .is_none()
                        .then(|| fuzzy.score(&folder.name).map(|score| (score, folder, true)))
                        .flatten();
                    match (label, name) {
                        (Some(label), Some(name)) => Some(if name.0 > label.0 { name } else { label }),
                        (Some(label), None) => Some(label),
                        (None, Some(name)) => Some(name),
                        (None, None) => None,
                    }
                })
                .collect();
            matched.sort_by(|a, b| {
                let exact = |folder: &FolderCandidate| folder.name.to_lowercase() != needle;
                let demoted = |folder: &FolderCandidate| hidden == HiddenMatch::Demote && folder.hidden;
                exact(a.1)
                    .cmp(&exact(b.1))
                    .then_with(|| demoted(a.1).cmp(&demoted(b.1)))
                    .then_with(|| b.0.cmp(&a.0))
                    .then_with(|| b.1.frecency.total_cmp(&a.1.frecency))
                    .then_with(|| a.1.label.cmp(&b.1.label))
            });
            matched.truncate(TOTAL_LIMIT.saturating_sub(file_matches.len()));
            for (_, folder, by_name) in matched {
                let (matched, name_matched) = if by_name {
                    (Vec::new(), fuzzy.indices(&folder.name).map_or_else(Vec::new, |(_, ranges)| ranges))
                } else {
                    (fuzzy.indices(&folder.label).map_or_else(Vec::new, |(_, ranges)| ranges), Vec::new())
                };
                rows.push(Row::Item(items.len()));
                items.push(folder_item(folder, matched, name_matched));
            }
            if !file_matches.is_empty() {
                rows.push(Row::Header(format!("Files in {here}").into()));
                for matched in file_matches {
                    rows.push(Row::Item(items.len()));
                    items.push(file_item(matched.entry, &matched.ranges, home));
                    file_count += 1;
                }
            }
        }
        None => {
            for source in [Source::Recent, Source::Favorite, Source::Project, Source::Here] {
                let mut group: Vec<&FolderCandidate> = folders
                    .iter()
                    .filter(|folder| folder.source == source)
                    // An empty listing leaves hidden entries out unless Show Hidden
                    // Files is on; "search includes hidden" applies to typed queries.
                    .filter(|folder| hidden == HiddenMatch::Include || !folder.hidden)
                    .collect();
                // Recent ranks by frecency; Favorites keep sidebar order and In keeps listing order.
                if source == Source::Recent {
                    group.sort_by(|a, b| b.frecency.total_cmp(&a.frecency));
                }
                let cap = match source {
                    Source::Recent => RECENT_LIMIT,
                    Source::Favorite | Source::Project => usize::MAX,
                    Source::Here | Source::File => HERE_LIMIT,
                };
                group.truncate(cap.min(TOTAL_LIMIT.saturating_sub(items.len())));
                if group.is_empty() {
                    continue;
                }
                rows.push(Row::Header(section_header(source, here)));
                for folder in group {
                    rows.push(Row::Item(items.len()));
                    items.push(folder_item(folder, Vec::new(), Vec::new()));
                }
            }
        }
    }
    let count = if file_count == 0 {
        count_text(items.len())
    } else {
        count_text_with_files(items.len() - file_count, file_count)
    };
    Listing { rows, items, count }
}

/// One file row, from a fuzzy match over its name. Return reveals it in its folder.
fn file_item(file: &PathEntry, ranges: &[Range<usize>], home: &Path) -> Item {
    let label = crate::style::display_path(&file.path, home);
    let (name, name_start) = folder_name(&file.path, &label, home);
    let start = name_start.unwrap_or(label.len());
    Item::Folder {
        path: file.path.clone(),
        matched: ranges.iter().map(|range| (range.start + start)..(range.end + start)).collect(),
        label,
        name,
        name_start,
        source: Source::File,
        hidden: file.hidden,
        last_visit: None,
        name_matched: Vec::new(),
    }
}

fn folder_item(folder: &FolderCandidate, matched: Vec<Range<usize>>, name_matched: Vec<Range<usize>>) -> Item {
    Item::Folder {
        path: folder.path.clone(),
        label: folder.label.clone(),
        name: folder.name.clone(),
        name_start: folder.name_start,
        source: folder.source,
        hidden: folder.hidden,
        last_visit: folder.last_visit,
        matched,
        name_matched,
    }
}

/// Command rows for `query`. An empty query lists every command in registry
/// order except Quit (so `>` then Return never quits; typing finds it); with
/// `show_recent`, up to five remembered commands come first under a
/// "Recently used" header and are not repeated below. A typed query is a flat
/// list, best score first.
pub fn command_listing(
    query: &str,
    recent_ids: &[CommandId],
    show_recent: bool,
    folder_sizes_visible: bool,
    virtual_destination: bool,
) -> Listing {
    let listed: Vec<&Command> =
        COMMANDS.iter().filter(|command| command.listed(folder_sizes_visible, virtual_destination)).collect();
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

/// Plugin commands matching `query`, after built-in commands. The title and
/// package name both score; the row always shows the package on its second line.
pub fn plugin_command_matches(query: &str, commands: &[PluginCommandItem]) -> Vec<(Item, u32)> {
    let query = query.trim();
    if query.is_empty() {
        return commands
            .iter()
            .cloned()
            .map(|command| (Item::PluginCommand { command, matched: Vec::new() }, 0))
            .collect();
    }
    let mut fuzzy = FuzzyMatcher::new(query);
    let mut matched: Vec<(u32, Item)> = commands
        .iter()
        .filter_map(|command| {
            let title_score = fuzzy.score(&command.title);
            let package_score = fuzzy.score(&command.package);
            let score = title_score.into_iter().chain(package_score).max()?;
            let matched = fuzzy.indices(&command.title).map_or_else(Vec::new, |(_, ranges)| ranges);
            Some((score, Item::PluginCommand { command: command.clone(), matched }))
        })
        .collect();
    matched.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    matched.truncate(TOTAL_LIMIT);
    matched.into_iter().map(|(score, item)| (item, score)).collect()
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

/// The palette scores and shows at most this many query characters. A 100k-file
/// clipboard is ~10 MB; scoring the folder or shaping a missing-path label with
/// it blocked the main thread for 30 s (findings #25). No path or query is close.
pub const MAX_QUERY_CHARS: usize = 1024;

/// A file or folder under a typed path, with its name and lowercased name
/// computed once off the main thread. Filtering 78k files per keystroke used to
/// lowercase every name (55 ms on the 100k fixture); the palette now reuses these.
#[derive(Clone, Debug)]
pub struct PathEntry {
    pub path: PathBuf,
    pub name: String,
    pub lower: String,
    /// A dot name or the Finder hidden flag, from the listing.
    pub hidden: bool,
}

impl PathEntry {
    pub fn new(path: PathBuf, name: String, hidden: bool) -> Self {
        let lower = name.to_lowercase();
        Self { path, name, lower, hidden }
    }
}

/// What a palette listing does with hidden entries.
///
/// Show Hidden Files on is `Include`. With it off, a typed query matches them
/// while `search_includes_hidden` is on, but they stay dimmed and rank after
/// visible matches (`Demote`); with the setting off they are left out
/// (`Exclude`). An empty query always leaves them out unless Show Hidden Files
/// is on, and a partial starting with `.` asks for them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HiddenMatch {
    Include,
    Demote,
    Exclude,
}

/// The policy for `partial` under the two settings.
pub fn hidden_match(partial: &str, show_hidden: bool, search_includes_hidden: bool) -> HiddenMatch {
    if show_hidden || partial.starts_with('.') {
        HiddenMatch::Include
    } else if partial.is_empty() {
        HiddenMatch::Exclude
    } else if search_includes_hidden {
        HiddenMatch::Demote
    } else {
        HiddenMatch::Exclude
    }
}

/// One scored entry. Ranges are computed only for the rows the listing keeps.
// The score and the demotion flag are the matcher's contract; the unit tests
// read them, while the listings themselves only use the sorted order and the
// hidden flag the entry carries.
#[allow(dead_code)]
pub struct EntryMatch<'a> {
    pub entry: &'a PathEntry,
    pub score: u32,
    /// Matched bytes in `entry.name`; empty for an empty partial.
    pub ranges: Vec<Range<usize>>,
    /// Hidden and Show Hidden Files is off: drawn dimmed and ranked after visible matches.
    pub demoted: bool,
}

/// Entries matching `partial`, best first, at most `limit`.
///
/// Empty partial: every entry the policy allows (hidden ones only for
/// `Include`), by lowercased name. Typed partial: fuzzy by name. Order:
/// exact name (lowercased) first, then not demoted before demoted, then score
/// (high first), then lowercased name. `candidates` limits scoring to those
/// indices — the typed file rows narrow incrementally; `None` scores every entry.
pub fn match_entries<'a>(
    partial: &str,
    entries: &'a [PathEntry],
    hidden: HiddenMatch,
    candidates: Option<&[u32]>,
    limit: usize,
) -> Vec<EntryMatch<'a>> {
    let all = || 0..entries.len() as u32;
    let indices: Vec<u32> = candidates.map_or_else(|| all().collect(), <[u32]>::to_vec);
    let allowed = |entry: &PathEntry| match hidden {
        HiddenMatch::Include => true,
        HiddenMatch::Demote | HiddenMatch::Exclude => !entry.hidden,
    };
    if partial.is_empty() {
        let mut kept: Vec<&PathEntry> =
            indices.iter().filter_map(|index| entries.get(*index as usize)).filter(|entry| allowed(entry)).collect();
        kept.sort_by(|a, b| a.lower.cmp(&b.lower));
        kept.truncate(limit);
        return kept
            .into_iter()
            .map(|entry| EntryMatch { entry, score: 0, ranges: Vec::new(), demoted: false })
            .collect();
    }

    let needle = partial.to_lowercase();
    let mut fuzzy = FuzzyMatcher::new(partial);
    let mut scored: Vec<(u32, &PathEntry)> = indices
        .iter()
        .filter_map(|index| entries.get(*index as usize))
        .filter(|entry| hidden != HiddenMatch::Exclude || !entry.hidden)
        .filter_map(|entry| fuzzy.score(&entry.name).map(|score| (score, entry)))
        .collect();
    scored.sort_by(|(score_a, a), (score_b, b)| {
        (a.lower != needle)
            .cmp(&(b.lower != needle))
            .then_with(|| demoted(hidden, a).cmp(&demoted(hidden, b)))
            .then_with(|| score_b.cmp(score_a))
            .then_with(|| a.lower.cmp(&b.lower))
    });
    scored.truncate(limit);
    scored
        .into_iter()
        .map(|(score, entry)| {
            let ranges = fuzzy.indices(&entry.name).map_or_else(Vec::new, |(_, ranges)| ranges);
            EntryMatch { entry, score, ranges, demoted: demoted(hidden, entry) }
        })
        .collect()
}

fn demoted(hidden: HiddenMatch, entry: &PathEntry) -> bool {
    hidden == HiddenMatch::Demote && entry.hidden
}

/// The indices of `files` matching `query`, the cheap first half of scoring
/// them. When `query` extends `previous.0`, only the files that matched the
/// prefix can match the longer needle (a prefix of a subsequence is one), so
/// only `previous.1` is scanned; otherwise every file is.
pub fn narrow_files(query: &str, files: &[PathEntry], previous: Option<(&str, &[u32])>) -> Vec<u32> {
    let indices: Vec<u32> = match previous {
        // Appending a letter can change the lowercase form of an earlier one,
        // such as Greek final sigma. Only reuse candidates if both prefixes hold.
        Some((old, indices))
            if query.starts_with(old) && (old.is_ascii() || query.to_lowercase().starts_with(&old.to_lowercase())) =>
        {
            indices.to_vec()
        }
        _ => (0..files.len() as u32).collect(),
    };
    let mut fuzzy = FuzzyMatcher::new(query);
    indices
        .into_iter()
        .filter(|index| files.get(*index as usize).is_some_and(|file| fuzzy.score(&file.name).is_some()))
        .collect()
}

/// Files shown under a typed path: the best fuzzy matches.
const FILE_LIMIT: usize = 10;

/// Files listed under a typed Cmd-K query: the best fuzzy matches in the folder.
const TYPED_FILE_LIMIT: usize = 10;

/// Path mode's folder rows, and files matching the typed name. With an empty
/// partial the parent itself comes first ("this folder"), so Return goes there.
/// A file named exactly as typed comes first, before the folders; the other
/// file matches follow the folders, so no row above them moves.
pub fn path_listing(
    parent: &Path,
    partial: &str,
    children: &[PathEntry],
    files: &[PathEntry],
    home: &Path,
    hidden: HiddenMatch,
) -> Listing {
    let mut rows = vec![Row::Header(section_header(Source::Here, &crate::style::display_name(parent)))];
    let mut items: Vec<Item> = Vec::new();
    let typed = partial.to_lowercase();
    let mut places = 0usize;
    let mut file_count = 0usize;
    let file_matches =
        if partial.is_empty() { Vec::new() } else { match_entries(partial, files, hidden, None, FILE_LIMIT) };
    let exact = file_matches.first().filter(|file| file.entry.lower == typed).map(|file| file.entry.path.clone());
    if let Some(file) = file_matches.first().filter(|file| Some(&file.entry.path) == exact.as_ref()) {
        rows.push(Row::Item(items.len()));
        items.push(file_item(file.entry, &file.ranges, home));
        file_count += 1;
    }
    if partial.is_empty() {
        let label = crate::style::display_path(parent, home);
        let (name, name_start) = folder_name(parent, &label, home);
        let hidden = parent.file_name().is_some_and(|name| name.to_string_lossy().starts_with('.'));
        items.push(Item::Folder {
            path: parent.to_path_buf(),
            label,
            name,
            name_start,
            source: Source::Here,
            hidden,
            last_visit: None,
            matched: Vec::new(),
            name_matched: Vec::new(),
        });
        rows.push(Row::Item(items.len() - 1));
        places += 1;
    }
    let folder_limit = TOTAL_LIMIT.saturating_sub(items.len());
    for matched in match_entries(partial, children, hidden, None, folder_limit) {
        let label = crate::style::display_path(&matched.entry.path, home);
        let (name, name_start) = folder_name(&matched.entry.path, &label, home);
        let start = name_start.unwrap_or(label.len());
        rows.push(Row::Item(items.len()));
        items.push(Item::Folder {
            path: matched.entry.path.clone(),
            label,
            name,
            name_start,
            source: Source::Here,
            hidden: matched.entry.hidden,
            last_visit: None,
            matched: matched.ranges.iter().map(|range| (range.start + start)..(range.end + start)).collect(),
            name_matched: Vec::new(),
        });
        places += 1;
    }
    for matched in &file_matches {
        if Some(&matched.entry.path) == exact.as_ref() {
            continue;
        }
        if items.len() >= TOTAL_LIMIT {
            break;
        }
        rows.push(Row::Item(items.len()));
        items.push(file_item(matched.entry, &matched.ranges, home));
        file_count += 1;
    }
    let count = count_text_with_files(places, file_count);
    Listing { rows, items, count }
}

/// Path mode while the parent is being listed.
pub fn loading_path_listing(parent: &Path) -> Listing {
    status_path_listing(parent, "Loading…".into())
}

/// Path mode when the parent is not a folder.
pub fn missing_path_listing(parent: &Path, home: &Path) -> Listing {
    status_path_listing(parent, format!("No folder at \"{}\"", crate::style::display_path(parent, home)).into())
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

    fn pe(path: &str) -> PathEntry {
        let path = PathBuf::from(path);
        let name = path.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let hidden = name.starts_with('.');
        PathEntry::new(path, name, hidden)
    }

    fn folders(favorites: &[PathBuf], recents: &[PathBuf], here: &[PathBuf]) -> Vec<FolderCandidate> {
        folder_candidates(
            favorites.to_vec(),
            recents.to_vec(),
            Vec::new(),
            here.to_vec(),
            Path::new("/home/me"),
            |_| None,
        )
    }

    /// Folder-only listing with Show Hidden Files' policy, the common test case.
    fn folders_listing(query: &str, folders: &[FolderCandidate], here: &str, hidden: HiddenMatch) -> Listing {
        folder_listing(query, folders, here, &[], Path::new("/home/me"), hidden, None)
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
                    Item::PluginCommand { command, .. } => command.title.clone(),
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
        let children = [pe("/home/me/Files/beta"), pe("/home/me/Files/alpha")];
        let parent = Path::new("/home/me/Files");
        let listing = path_listing(parent, "", &children, &[], Path::new("/home/me"), HiddenMatch::Include);
        assert_eq!(row_text(&listing), ["# In Files", "~/Files", "~/Files/alpha", "~/Files/beta"]);
        assert_eq!(listing.items.len(), 3);
        assert_eq!(listing.count, "3 places");

        let listing = path_listing(parent, "alp", &children, &[], Path::new("/home/me"), HiddenMatch::Include);
        assert_eq!(row_text(&listing), ["# In Files", "~/Files/alpha"]);
        assert_eq!(listing.items[0].name_ranges().first(), Some(&(0..3)), "the match is highlighted in the name");
    }

    #[test]
    fn a_file_named_as_typed_comes_first_and_others_follow_folders() {
        let parent = Path::new("/home/me/Files");
        let children = [pe("/home/me/Files/license-old")];
        let files = [pe("/home/me/Files/LICENSE.md"), pe("/home/me/Files/LICENSE"), pe("/home/me/Files/notes.txt")];
        let listing = path_listing(parent, "LICENSE", &children, &files, Path::new("/home/me"), HiddenMatch::Include);
        assert_eq!(
            row_text(&listing),
            ["# In Files", "~/Files/LICENSE", "~/Files/license-old", "~/Files/LICENSE.md"],
            "exact file, then folders, then other files starting with the text"
        );
        let Item::Folder { source, .. } = &listing.items[0] else { panic!("a row") };
        assert_eq!(*source, Source::File);
        let listing = path_listing(parent, "", &children, &files, Path::new("/home/me"), HiddenMatch::Include);
        assert!(!row_text(&listing).iter().any(|row| row.contains("notes")), "no files until a name is typed");
    }

    /// H4 #20 and B2: exercise typed path matching on 78k file names.
    /// Elapsed time is reported for local release measurements (see docs/testing.md).
    #[test]
    fn path_completion_filters_many_files() {
        let files: Vec<PathEntry> = (0..78_000).map(|i| pe(&format!("/home/me/Files/file-{i:06}.txt"))).collect();
        let cases = [
            ("f", Some((0, 0..1))),
            ("fi", Some((0, 0..2))),
            ("file-0", Some((0, 0..6))),
            ("-🙂", None),
            ("file-077", Some((77_000, 0..8))),
        ];
        let input = std::hint::black_box(&files);
        let started = std::time::Instant::now();
        for (partial, expected) in cases {
            let rows = match_entries(std::hint::black_box(partial), input, HiddenMatch::Include, None, FILE_LIMIT);
            if let Some((first, range)) = expected {
                assert_eq!(rows.len(), FILE_LIMIT, "{partial}");
                assert_eq!(rows[0].entry.path, files[first].path, "{partial}");
                assert_eq!(rows[0].ranges, vec![range], "{partial}");
            } else {
                assert!(rows.is_empty(), "{partial}");
            }
        }
        let elapsed = started.elapsed();
        println!("match_entries over 78k files, five partials: {elapsed:?}");
    }

    fn entry_names(matches: &[EntryMatch<'_>]) -> Vec<String> {
        matches.iter().map(|matched| matched.entry.name.clone()).collect()
    }

    #[test]
    fn hidden_match_follows_the_two_settings() {
        assert_eq!(hidden_match("backup", true, false), HiddenMatch::Include, "Show Hidden Files wins");
        assert_eq!(hidden_match("backup", false, true), HiddenMatch::Demote);
        assert_eq!(hidden_match("backup", false, false), HiddenMatch::Exclude);
        assert_eq!(hidden_match(".b", false, false), HiddenMatch::Include, "a typed dot asks for them");
        assert_eq!(hidden_match("", false, true), HiddenMatch::Exclude, "an empty listing stays visible-only");
    }

    #[test]
    fn match_entries_hides_hidden_for_an_empty_partial() {
        let entries = [pe("/x/alpha"), pe("/x/.hidden")];
        assert_eq!(entry_names(&match_entries("", &entries, HiddenMatch::Exclude, None, 10)), ["alpha"]);
        assert_eq!(
            entry_names(&match_entries("", &entries, HiddenMatch::Include, None, 10)),
            [".hidden", "alpha"],
            "Show Hidden Files lists them, by name"
        );
    }

    #[test]
    fn match_entries_matches_dot_names_without_the_dot() {
        let entries = [pe("/x/backup.txt"), pe("/x/Backups"), pe("/x/.backup_excludes"), pe("/x/notes.txt")];
        let rows = match_entries("backup", &entries, HiddenMatch::Demote, None, 10);
        assert_eq!(entry_names(&rows), ["backup.txt", "Backups", ".backup_excludes"]);
        assert!(!rows[0].demoted && !rows[1].demoted);
        assert!(rows[2].demoted, "hidden and Show Hidden Files is off");
        assert!(rows[2].score > 0, "it is a real fuzzy match");
        assert!(!rows[2].ranges.is_empty(), "the row highlights the match");
    }

    #[test]
    fn a_dot_partial_demotes_nothing() {
        let entries = [pe("/x/.backup"), pe("/x/.backups"), pe("/x/notes.txt")];
        let rows = match_entries(".b", &entries, HiddenMatch::Include, None, 10);
        assert_eq!(entry_names(&rows), [".backup", ".backups"]);
        assert!(rows.iter().all(|matched| !matched.demoted), "the typed dot asked for hidden entries");
    }

    #[test]
    fn exact_name_comes_first() {
        let entries = [pe("/x/LICENSE-MIT"), pe("/x/LICENSE"), pe("/x/license.txt")];
        let rows = match_entries("LICENSE", &entries, HiddenMatch::Include, None, 10);
        assert_eq!(rows[0].entry.name, "LICENSE");
        assert!(entry_names(&rows).contains(&"LICENSE-MIT".to_owned()));
    }

    #[test]
    fn folder_listing_demotes_hidden_candidates() {
        let candidates = folders(&[], &[], &[p("/home/me/.config"), p("/home/me/config-notes")]);
        let listing = folders_listing("config", &candidates, "me", HiddenMatch::Demote);
        let names: Vec<String> = listing
            .items
            .iter()
            .map(|item| match item {
                Item::Folder { name, .. } => name.clone(),
                Item::Command { .. } | Item::PluginCommand { .. } => String::new(),
            })
            .collect();
        assert_eq!(names, ["config-notes", ".config"], "the hidden folder comes after visible matches");
        assert!(matches!(listing.items.last(), Some(Item::Folder { hidden: true, .. })));

        let listing = folders_listing("config", &candidates, "me", HiddenMatch::Exclude);
        assert_eq!(listing.items.len(), 1, "the setting off leaves it out");
        let listing = folders_listing("", &candidates, "me", HiddenMatch::Exclude);
        assert_eq!(listing.items.len(), 1, "an empty listing leaves it out too");
    }

    #[test]
    fn narrow_files_agrees_with_a_full_scan() {
        let files = [pe("/x/back.txt"), pe("/x/backup.txt"), pe("/x/Backups"), pe("/x/notes.txt")];
        let full = |query: &str| -> Vec<u32> {
            let mut fuzzy = crow_search::FuzzyMatcher::new(query);
            (0..files.len() as u32).filter(|index| fuzzy.score(&files[*index as usize].name).is_some()).collect()
        };
        let mut previous: Option<(String, Vec<u32>)> = None;
        for query in ["b", "ba", "bac"] {
            let narrowed =
                narrow_files(query, &files, previous.as_ref().map(|(old, at)| (old.as_str(), at.as_slice())));
            assert_eq!(narrowed, full(query), "{query}");
            previous = Some((query.to_owned(), narrowed));
        }
        // A query that does not extend the last one must scan every file again:
        // `n` matches notes.txt, which the `bac` narrowing left out.
        let narrowed = narrow_files("n", &files, previous.as_ref().map(|(old, at)| (old.as_str(), at.as_slice())));
        assert_eq!(narrowed, full("n"));
        assert_eq!(narrowed, [3], "notes.txt is back");
    }

    #[test]
    fn narrow_files_handles_context_sensitive_lowercase() {
        let files = [pe("/x/οσα.txt")];
        let previous = narrow_files("ΟΣ", &files, None);
        assert!(previous.is_empty());
        assert_eq!(narrow_files("ΟΣΑ", &files, None), [0]);
        assert_eq!(narrow_files("ΟΣΑ", &files, Some(("ΟΣ", &previous))), [0]);
    }

    #[test]
    fn count_text_names_files_separately() {
        assert_eq!(count_text_with_files(3, 0), "3 places");
        assert_eq!(count_text_with_files(3, 1), "3 places · 1 file");
        assert_eq!(count_text_with_files(1, 2), "1 place · 2 files");
        assert_eq!(count_text_with_files(0, 1), "No places · 1 file");
    }

    #[test]
    fn folder_listing_puts_files_after_the_folders() {
        let candidates = folders(&[], &[], &[p("/home/me/Files/backup")]);
        let files = [pe("/home/me/Files/backup.txt"), pe("/home/me/Files/notes.txt")];
        let listing =
            folder_listing("backup", &candidates, "Files", &files, Path::new("/home/me"), HiddenMatch::Include, None);
        assert_eq!(row_text(&listing), ["~/Files/backup", "# Files in Files", "~/Files/backup.txt"]);
        assert_eq!(listing.count, "1 place · 1 file");
        let Item::Folder { source, .. } = &listing.items[1] else { panic!("a file row") };
        assert_eq!(*source, Source::File);

        let empty = folder_listing("", &candidates, "Files", &files, Path::new("/home/me"), HiddenMatch::Include, None);
        assert!(!row_text(&empty).iter().any(|row| row.contains("notes")), "no files for an empty query");
    }

    #[test]
    fn missing_parent_says_so() {
        let listing = missing_path_listing(Path::new("/home/me/nope"), Path::new("/home/me"));
        assert_eq!(row_text(&listing), ["# In nope", "No folder at \"~/nope\""]);
        assert!(listing.items.is_empty());
    }

    fn command_ids(query: &str) -> Vec<CommandId> {
        command_listing(query, &[], false, false, false)
            .items
            .into_iter()
            .filter_map(|item| match item {
                Item::Command { command, .. } => Some(command.id),
                Item::Folder { .. } | Item::PluginCommand { .. } => None,
            })
            .collect()
    }

    #[test]
    fn sections_follow_source_order_and_skip_empty() {
        let candidates = folders(&[p("/home/me/Files/fav")], &[p("/home/me/Files/rec")], &[p("/home/me/Files/here")]);
        let listing = folders_listing("", &candidates, "Files", HiddenMatch::Include);
        assert_eq!(
            row_text(&listing),
            ["# Recent", "~/Files/rec", "# Favorites", "~/Files/fav", "# In Files", "~/Files/here"],
        );

        let no_here = folders(&[p("/home/me/Files/fav")], &[p("/home/me/Files/rec")], &[]);
        assert_eq!(
            row_text(&folders_listing("", &no_here, "Files", HiddenMatch::Include)),
            ["# Recent", "~/Files/rec", "# Favorites", "~/Files/fav"]
        );
    }

    #[test]
    fn a_favorite_that_is_recent_shows_once_under_favorites() {
        let candidates =
            folders(&[p("/home/me/Files/beta")], &[p("/home/me/Files/beta"), p("/home/me/Files/alpha")], &[]);
        let listing = folders_listing("", &candidates, "Files", HiddenMatch::Include);
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

        let listing = folders_listing("", &candidates, "Files", HiddenMatch::Include);
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
        let matched = folders_listing("Files", &candidates, "Files", HiddenMatch::Include);
        assert_eq!(matched.items.len(), TOTAL_LIMIT);
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)] // The matched list is the point of the test.
    fn name_ranges_are_the_part_inside_the_name() {
        let candidates = folders(&[], &[p("/home/me/Projects/git")], &[]);
        let listing = folders_listing("git", &candidates, "Coding", HiddenMatch::Include);
        let Item::Folder { label, matched, .. } = &listing.items[0] else { panic!("a folder row") };
        assert_eq!(label, "~/Projects/git");
        assert_eq!(matched.first(), Some(&(11..14)), "the path line highlights the name in place");
        assert_eq!(listing.items[0].name_ranges().first(), Some(&(0..3)), "the name line gets it shifted");

        let straddling = Item::Folder {
            path: PathBuf::new(),
            label: "~/ab/cd".into(),
            name: "cd".into(),
            name_start: Some(5),
            source: Source::Recent,
            hidden: false,
            last_visit: None,
            matched: vec![3..7],
            name_matched: Vec::new(),
        };
        assert_eq!(straddling.name_ranges().first(), Some(&(0..2)), "a range across the name's start is clipped");
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)] // The matched list is the point of the test.
    fn home_row_matches_its_name_and_its_path() {
        let home = Path::new("/home/jules");
        let candidates =
            folder_candidates(vec![home.to_path_buf()], Vec::new(), Vec::new(), Vec::new(), home, |_| None);

        let listing = folders_listing("jul", &candidates, "jules", HiddenMatch::Include);
        assert_eq!(listing.items.len(), 1, "the account name finds the home row");
        let Item::Folder { label, name, name_start, matched, .. } = &listing.items[0] else { panic!("the home row") };
        assert_eq!((label.as_str(), name.as_str(), *name_start), ("~", "jules", None));
        assert!(matched.is_empty(), "the path line has no match");
        assert_eq!(listing.items[0].name_ranges(), vec![0..3], "the name line highlights jul");

        let listing = folders_listing("~", &candidates, "jules", HiddenMatch::Include);
        assert_eq!(listing.items.len(), 1, "~ still finds it");
        let Item::Folder { matched, .. } = &listing.items[0] else { panic!("the home row") };
        assert_eq!(matched, &vec![0..1], "the path line highlights ~");
        assert!(listing.items[0].name_ranges().is_empty(), "the name line has no match");
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
        let recent = command_listing("", &[CommandId::Quit, CommandId::NewTab], true, false, false);
        let ids: Vec<CommandId> = recent
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Command { command, .. } => Some(command.id),
                Item::Folder { .. } | Item::PluginCommand { .. } => None,
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
    fn plugin_commands_match_title_or_package_and_keep_title_ranges() {
        let item = |local: &str, title: &str, package: &str| PluginCommandItem {
            extension: crow_core::ExtensionId::new(
                crow_core::PluginId::parse("example.test").unwrap(),
                crow_core::LocalId::parse(local).unwrap(),
            ),
            generation: 1,
            title: title.into(),
            package: package.into(),
        };
        let commands = vec![item("relative", "Copy Relative Path", "Example Tools"), item("other", "Open", "Other")];
        let matched = plugin_command_matches("relative", &commands);
        assert_eq!(matched.len(), 1);
        let Item::PluginCommand { command, matched: ranges } = &matched[0].0 else { panic!() };
        assert_eq!(command.title, "Copy Relative Path");
        assert!(!ranges.is_empty(), "title matches carry ranges");
        let by_package = plugin_command_matches("example tools", &commands);
        assert_eq!(by_package.len(), 1);
        assert!(plugin_command_matches("zzz", &commands).is_empty());
    }

    #[test]
    fn settings_commands_are_in_the_palette() {
        let commands = |query: &str| -> Vec<CommandId> {
            command_listing(query, &[], true, false, false)
                .items
                .iter()
                .filter_map(|item| match item {
                    Item::Command { command, .. } => Some(command.id),
                    Item::Folder { .. } | Item::PluginCommand { .. } => None,
                })
                .collect()
        };
        assert!(commands("settings").contains(&CommandId::OpenSettings));
        assert!(commands("edit settings").contains(&CommandId::EditSettings));
        assert!(commands("search home").contains(&CommandId::SearchHome));
    }

    #[test]
    fn calculate_folder_sizes_is_hidden_when_sizes_are_automatic() {
        let commands = |folder_sizes_visible: bool| -> Vec<CommandId> {
            command_listing("", &[], false, folder_sizes_visible, false)
                .items
                .iter()
                .filter_map(|item| match item {
                    Item::Command { command, .. } => Some(command.id),
                    Item::Folder { .. } | Item::PluginCommand { .. } => None,
                })
                .collect()
        };
        assert!(commands(false).contains(&CommandId::CalculateFolderSizes), "off: the command is the way to measure");
        assert!(
            !commands(true).contains(&CommandId::CalculateFolderSizes),
            "visible: the setting already measures; the one-off command would duplicate it"
        );
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
