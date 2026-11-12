//! Actions and the Finder keymap. Key contexts nest as
//! `Crow > {Sidebar, FileList}`: the same key can mean different things in each.

use gpui::{KeyBinding, actions};

actions!(
    crow,
    [
        // File list and sidebar (meaning depends on context)
        MoveUp,
        MoveDown,
        MoveToFirst,
        MoveToLast,
        OpenSelection,
        Rename,
        QuickLook,
        // Navigation within a tab
        OpenParent,
        GoBack,
        GoForward,
        // Window
        NewTab,
        CloseTab,
        NextTab,
        PreviousTab,
        NewWindow,
        SwitchFocus,
        ToggleHiddenFiles,
        Quit,
    ]
);

pub fn finder_keymap() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("up", MoveUp, Some("FileList")),
        KeyBinding::new("down", MoveDown, Some("FileList")),
        KeyBinding::new("alt-up", MoveToFirst, Some("FileList")),
        KeyBinding::new("alt-down", MoveToLast, Some("FileList")),
        KeyBinding::new("cmd-down", OpenSelection, Some("FileList")),
        KeyBinding::new("cmd-o", OpenSelection, Some("FileList")),
        KeyBinding::new("enter", Rename, Some("FileList")),
        KeyBinding::new("space", QuickLook, Some("FileList")),
        KeyBinding::new("up", MoveUp, Some("Sidebar")),
        KeyBinding::new("down", MoveDown, Some("Sidebar")),
        KeyBinding::new("enter", OpenSelection, Some("Sidebar")),
        KeyBinding::new("cmd-down", OpenSelection, Some("Sidebar")),
        KeyBinding::new("cmd-up", OpenParent, Some("Crow")),
        KeyBinding::new("cmd-[", GoBack, Some("Crow")),
        KeyBinding::new("cmd-]", GoForward, Some("Crow")),
        KeyBinding::new("cmd-t", NewTab, Some("Crow")),
        KeyBinding::new("cmd-w", CloseTab, Some("Crow")),
        KeyBinding::new("cmd-shift-]", NextTab, Some("Crow")),
        KeyBinding::new("cmd-shift-[", PreviousTab, Some("Crow")),
        KeyBinding::new("ctrl-tab", NextTab, Some("Crow")),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, Some("Crow")),
        KeyBinding::new("cmd-n", NewWindow, None),
        KeyBinding::new("tab", SwitchFocus, Some("Crow")),
        KeyBinding::new("cmd-shift-.", ToggleHiddenFiles, Some("Crow")),
        KeyBinding::new("cmd-q", Quit, None),
    ]
}
