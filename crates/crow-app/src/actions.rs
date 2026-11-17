//! Actions and the Finder keymap. Key contexts nest as
//! `Crow > {Sidebar, FileList [ColumnView]}`: the same key can mean different things in each.
//! The Vim layer's bindings also need `vim` in the context, which views add only
//! when `keymap = "vim"`, so switching layers needs no rebinding.

use gpui::{KeyBinding, NoAction, actions};

actions!(
    crow,
    [
        // File list and sidebar (meaning depends on context)
        MoveUp,
        MoveDown,
        MoveToFirst,
        MoveToLast,
        ExtendUp,
        ExtendDown,
        SelectAll,
        OpenSelection,
        Rename,
        QuickLook,
        // File operations
        NewFolder,
        NewFile,
        Duplicate,
        MoveToTrash,
        DeletePermanently,
        CopyFiles,
        CutFiles,
        PasteFiles,
        PasteMoveFiles,
        CopyPath,
        ShowInFinder,
        Undo,
        Redo,
        CancelOperation,
        ToggleActivity,
        // Sheets (conflicts, confirmations)
        SheetConfirm,
        SheetCancel,
        // Sidebar
        AddToSidebar,
        RemoveFromSidebar,
        MoveFavoriteUp,
        MoveFavoriteDown,
        PreviousGroup,
        NextGroup,
        Eject,
        // View settings and themes
        WhyThisView,
        UseViewAsDefaults,
        ResetFolderView,
        ShowThemePicker,
        ApplyThemeEverywhere,
        // Vim layer
        BeginMark,
        BeginJump,
        // Column view
        MoveLeft,
        MoveRight,
        // Navigation within a tab
        OpenParent,
        GoBack,
        GoForward,
        GoToFolder,
        /// ⌘L or a double-click on empty path space: edit the path in the bottom bar.
        EditPath,
        ViewAsList,
        ViewAsColumns,
        // List filter
        StartFilter,
        ClearFilter,
        // Command palette
        ShowCommandPalette,
        PaletteOpenInTab,
        // Search
        ShowSearch,
        SearchEverything,
        CycleMatch,
        CloseSearch,
        SearchReveal,
        /// ↓ in the search field: focus the results without the Tab cycle's sidebar fallback.
        SearchFocusResults,
        /// ↑ on the first result: back to the search field.
        SearchFocusField,
        // Window
        NewTab,
        CloseTab,
        NextTab,
        PreviousTab,
        NewWindow,
        SwitchFocus,
        ToggleHiddenFiles,
        TogglePreview,
        Minimize,
        Zoom,
        KeyboardShortcuts,
        Quit,
        // App menu
        About,
        OpenSettings,
        EditSettings,
        HideApp,
        HideOthers,
        ShowAll,
    ]
);

/// List bindings, except while the filter field has focus: the field is inside the
/// list, so a plain character (`/`, letters, Space) must reach the text input.
const LIST: &str = "FileList && !Filter";
const LIST_FILTERED: &str = "FileList && Filtered && !Filter";
const COLUMNS: &str = "ColumnView && !Filter";
const LIST_NOT_COLUMNS: &str = "FileList && !ColumnView && !Filter";

pub fn finder_keymap() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("up", MoveUp, Some(LIST)),
        KeyBinding::new("down", MoveDown, Some(LIST)),
        KeyBinding::new("alt-up", MoveToFirst, Some(LIST)),
        KeyBinding::new("alt-down", MoveToLast, Some(LIST)),
        KeyBinding::new("shift-up", ExtendUp, Some(LIST)),
        KeyBinding::new("shift-down", ExtendDown, Some(LIST)),
        KeyBinding::new("cmd-a", SelectAll, Some(LIST)),
        KeyBinding::new("/", StartFilter, Some(LIST)),
        KeyBinding::new("escape", ClearFilter, Some(LIST_FILTERED)),
        KeyBinding::new("up", MoveUp, Some("Filter")),
        KeyBinding::new("down", MoveDown, Some("Filter")),
        // Finder's File > Open is ⌘O, while its list views open with ⌘↓. GPUI's
        // native menu takes the first binding (⌘O) and the hints take the last
        // (⌘↓), so both surfaces match Finder; keep cmd-o first.
        KeyBinding::new("cmd-o", OpenSelection, Some(LIST)),
        KeyBinding::new("cmd-down", OpenSelection, Some(LIST)),
        KeyBinding::new("enter", Rename, Some(LIST)),
        KeyBinding::new("space", QuickLook, Some(LIST)),
        KeyBinding::new("cmd-shift-n", NewFolder, Some(LIST)),
        KeyBinding::new("cmd-alt-n", NewFile, Some(LIST)),
        KeyBinding::new("cmd-d", Duplicate, Some(LIST)),
        KeyBinding::new("cmd-backspace", MoveToTrash, Some(LIST)),
        KeyBinding::new("cmd-alt-backspace", DeletePermanently, Some(LIST)),
        KeyBinding::new("cmd-c", CopyFiles, Some(LIST)),
        KeyBinding::new("cmd-x", CutFiles, Some(LIST)),
        KeyBinding::new("cmd-v", PasteFiles, Some(LIST)),
        KeyBinding::new("cmd-alt-v", PasteMoveFiles, Some(LIST)),
        KeyBinding::new("cmd-alt-c", CopyPath, Some(LIST)),
        KeyBinding::new("cmd-z", Undo, Some("Crow")),
        KeyBinding::new("cmd-shift-z", Redo, Some("Crow")),
        KeyBinding::new("escape", CancelOperation, Some("Crow")),
        KeyBinding::new("cmd-alt-l", ToggleActivity, Some("Crow")),
        KeyBinding::new("enter", SheetConfirm, Some("Sheet")),
        KeyBinding::new("escape", SheetCancel, Some("Sheet")),
        // A sheet owns focus; arrows and Tab must not move it behind the scrim.
        // The theme picker's own up/down bindings come later and out-rank these.
        KeyBinding::new("tab", NoAction {}, Some("Sheet")),
        KeyBinding::new("shift-tab", NoAction {}, Some("Sheet")),
        KeyBinding::new("up", NoAction {}, Some("Sheet")),
        KeyBinding::new("down", NoAction {}, Some("Sheet")),
        KeyBinding::new("left", NoAction {}, Some("Sheet")),
        KeyBinding::new("right", NoAction {}, Some("Sheet")),
        KeyBinding::new("left", MoveLeft, Some(COLUMNS)),
        KeyBinding::new("right", MoveRight, Some(COLUMNS)),
        KeyBinding::new("left", MoveLeft, Some(LIST_NOT_COLUMNS)),
        KeyBinding::new("right", MoveRight, Some(LIST_NOT_COLUMNS)),
        KeyBinding::new("cmd-2", ViewAsList, Some("Crow")),
        KeyBinding::new("cmd-3", ViewAsColumns, Some("Crow")),
        KeyBinding::new("up", MoveUp, Some("Sidebar")),
        KeyBinding::new("down", MoveDown, Some("Sidebar")),
        KeyBinding::new("enter", OpenSelection, Some("Sidebar")),
        KeyBinding::new("right", OpenSelection, Some("Sidebar")),
        KeyBinding::new("cmd-down", OpenSelection, Some("Sidebar")),
        KeyBinding::new("cmd-up", OpenParent, Some("Crow")),
        KeyBinding::new("cmd-[", GoBack, Some("Crow")),
        KeyBinding::new("cmd-]", GoForward, Some("Crow")),
        KeyBinding::new("cmd-shift-g", GoToFolder, Some("Crow")),
        KeyBinding::new("cmd-l", EditPath, Some("Crow")),
        KeyBinding::new("cmd-t", NewTab, Some("Crow")),
        KeyBinding::new("cmd-w", CloseTab, Some("Crow")),
        KeyBinding::new("cmd-shift-]", NextTab, Some("Crow")),
        KeyBinding::new("cmd-shift-[", PreviousTab, Some("Crow")),
        KeyBinding::new("ctrl-tab", NextTab, Some("Crow")),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, Some("Crow")),
        KeyBinding::new("cmd-n", NewWindow, None),
        KeyBinding::new("cmd-m", Minimize, Some("Crow")),
        KeyBinding::new("cmd-h", HideApp, None),
        KeyBinding::new("cmd-alt-h", HideOthers, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("tab", SwitchFocus, Some("Crow")),
        KeyBinding::new("cmd-shift-.", ToggleHiddenFiles, Some("Crow")),
        KeyBinding::new("cmd-shift-p", TogglePreview, Some("Crow")),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("ctrl-cmd-t", AddToSidebar, Some("Crow")),
        KeyBinding::new("cmd-backspace", RemoveFromSidebar, Some("Sidebar")),
        KeyBinding::new("cmd-alt-up", MoveFavoriteUp, Some("Sidebar")),
        KeyBinding::new("cmd-alt-down", MoveFavoriteDown, Some("Sidebar")),
        KeyBinding::new("alt-up", PreviousGroup, Some("Sidebar")),
        KeyBinding::new("alt-down", NextGroup, Some("Sidebar")),
        KeyBinding::new("cmd-e", Eject, Some("Sidebar")),
        KeyBinding::new("cmd-j", WhyThisView, Some("Crow")),
        KeyBinding::new("cmd-alt-t", ShowThemePicker, Some("Crow")),
        KeyBinding::new("up", MoveUp, Some("ThemePicker")),
        KeyBinding::new("down", MoveDown, Some("ThemePicker")),
        // A modal sheet owns Tab; it must not move focus to the window behind it.
        KeyBinding::new("tab", NoAction {}, Some("ThemePicker")),
        KeyBinding::new("shift-tab", NoAction {}, Some("ThemePicker")),
        KeyBinding::new("cmd-enter", ApplyThemeEverywhere, Some("ThemePicker")),
        KeyBinding::new("cmd-k", ShowCommandPalette, Some("Crow")),
        KeyBinding::new("cmd-f", ShowSearch, Some("Crow")),
        KeyBinding::new("cmd-shift-a", SearchEverything, Some("Search")),
        KeyBinding::new("cmd-shift-m", CycleMatch, Some("Search")),
        KeyBinding::new("cmd-shift-a", SearchEverything, Some("Crow")),
        KeyBinding::new("escape", CloseSearch, Some("SearchResults")),
        KeyBinding::new("up", MoveUp, Some("SearchResults")),
        KeyBinding::new("down", MoveDown, Some("SearchResults")),
        // ↓ in the field enters the results (no sidebar fallback); ↑ on the first row
        // comes back. "Search" is the band's own context level, so it reaches the field
        // without stealing the results list's plain ↓ (that binding matches deeper).
        KeyBinding::new("down", SearchFocusResults, Some("Search")),
        KeyBinding::new("up", SearchFocusField, Some("SearchResults")),
        KeyBinding::new("alt-up", MoveToFirst, Some("SearchResults")),
        KeyBinding::new("alt-down", MoveToLast, Some("SearchResults")),
        KeyBinding::new("enter", OpenSelection, Some("SearchResults")),
        KeyBinding::new("space", QuickLook, Some("SearchResults")),
        KeyBinding::new("cmd-r", SearchReveal, Some("SearchResults")),
        KeyBinding::new("cmd-a", SelectAll, Some("SearchResults")),
        KeyBinding::new("cmd-c", CopyFiles, Some("SearchResults")),
        KeyBinding::new("cmd-x", CutFiles, Some("SearchResults")),
        KeyBinding::new("cmd-backspace", MoveToTrash, Some("SearchResults")),
        KeyBinding::new("cmd-alt-backspace", DeletePermanently, Some("SearchResults")),
        KeyBinding::new("up", MoveUp, Some("Palette")),
        KeyBinding::new("down", MoveDown, Some("Palette")),
        KeyBinding::new("cmd-enter", PaletteOpenInTab, Some("Palette")),
        // A sheet owns the keyboard: window-level keys must not act behind it.
        // Enter/Escape are the sheet's own; the picker's up/down and cmd-enter
        // are bound later and still win there.
        KeyBinding::new("cmd-z", NoAction {}, Some("Sheet")),
        KeyBinding::new("cmd-shift-z", NoAction {}, Some("Sheet")),
        KeyBinding::new("cmd-alt-l", NoAction {}, Some("Sheet")),
        KeyBinding::new("cmd-2", NoAction {}, Some("Sheet")),
        KeyBinding::new("cmd-3", NoAction {}, Some("Sheet")),
        KeyBinding::new("cmd-up", NoAction {}, Some("Sheet")),
        KeyBinding::new("cmd-[", NoAction {}, Some("Sheet")),
        KeyBinding::new("cmd-]", NoAction {}, Some("Sheet")),
        KeyBinding::new("cmd-shift-g", NoAction {}, Some("Sheet")),
        KeyBinding::new("cmd-l", NoAction {}, Some("Sheet")),
        KeyBinding::new("cmd-t", NoAction {}, Some("Sheet")),
        KeyBinding::new("cmd-w", NoAction {}, Some("Sheet")),
        KeyBinding::new("cmd-shift-]", NoAction {}, Some("Sheet")),
        KeyBinding::new("cmd-shift-[", NoAction {}, Some("Sheet")),
        KeyBinding::new("ctrl-tab", NoAction {}, Some("Sheet")),
        KeyBinding::new("ctrl-shift-tab", NoAction {}, Some("Sheet")),
        KeyBinding::new("cmd-n", NoAction {}, Some("Sheet")),
        KeyBinding::new("cmd-,", NoAction {}, Some("Sheet")),
        KeyBinding::new("cmd-shift-.", NoAction {}, Some("Sheet")),
        KeyBinding::new("cmd-shift-p", NoAction {}, Some("Sheet")),
        KeyBinding::new("ctrl-cmd-t", NoAction {}, Some("Sheet")),
        KeyBinding::new("cmd-j", NoAction {}, Some("Sheet")),
        KeyBinding::new("cmd-alt-t", NoAction {}, Some("Sheet")),
        KeyBinding::new("cmd-k", NoAction {}, Some("Sheet")),
        KeyBinding::new("cmd-f", NoAction {}, Some("Sheet")),
        KeyBinding::new("cmd-shift-a", NoAction {}, Some("Sheet")),
    ]
}

/// The optional Vim layer, on top of the Finder keys.
pub fn vim_keymap() -> Vec<KeyBinding> {
    const LIST: &str = "FileList && vim && !ColumnView && !Filter";
    const COLUMNS: &str = "ColumnView && vim && !Filter";
    const ANY: &str = "FileList && vim && !Filter";
    vec![
        KeyBinding::new("j", MoveDown, Some(ANY)),
        KeyBinding::new("k", MoveUp, Some(ANY)),
        KeyBinding::new("shift-j", ExtendDown, Some(ANY)),
        KeyBinding::new("shift-k", ExtendUp, Some(ANY)),
        KeyBinding::new("h", MoveLeft, Some(LIST)),
        KeyBinding::new("l", MoveRight, Some(LIST)),
        KeyBinding::new("h", MoveLeft, Some(COLUMNS)),
        KeyBinding::new("l", MoveRight, Some(COLUMNS)),
        KeyBinding::new("-", OpenParent, Some(ANY)),
        KeyBinding::new("g g", MoveToFirst, Some(ANY)),
        KeyBinding::new("shift-g", MoveToLast, Some(ANY)),
        KeyBinding::new("y y", CopyFiles, Some(ANY)),
        // Like Vim's dd: the items go to the clipboard and move on paste; nothing is deleted.
        KeyBinding::new("d d", CutFiles, Some(ANY)),
        KeyBinding::new("p", PasteFiles, Some(ANY)),
        KeyBinding::new("u", Undo, Some(ANY)),
        KeyBinding::new("ctrl-r", Redo, Some(ANY)),
        KeyBinding::new("c w", Rename, Some(ANY)),
        KeyBinding::new("m", BeginMark, Some(ANY)),
        KeyBinding::new("'", BeginJump, Some(ANY)),
        KeyBinding::new("j", MoveDown, Some("Sidebar && vim")),
        KeyBinding::new("k", MoveUp, Some("Sidebar && vim")),
        KeyBinding::new("l", OpenSelection, Some("Sidebar && vim")),
        KeyBinding::new("j", MoveDown, Some("ThemePicker && vim")),
        KeyBinding::new("k", MoveUp, Some("ThemePicker && vim")),
        KeyBinding::new("j", MoveDown, Some("Palette && vim")),
        KeyBinding::new("k", MoveUp, Some("Palette && vim")),
        KeyBinding::new("j", MoveDown, Some("SearchResults && vim")),
        KeyBinding::new("k", MoveUp, Some("SearchResults && vim")),
    ]
}

#[cfg(test)]
mod tests {
    use gpui::{Action, KeyContext, Keymap, Keystroke};

    use super::*;

    fn keymap() -> Keymap {
        let mut keymap = Keymap::default();
        keymap.add_bindings(finder_keymap());
        keymap.add_bindings(vim_keymap());
        keymap.add_bindings(crate::text_input::keymap());
        keymap
    }

    fn resolved(keymap: &Keymap, input: &'static str, stack: &[&'static str]) -> Vec<Box<dyn Action>> {
        let stack: Vec<KeyContext> = stack.iter().map(|source| KeyContext::parse(source).expect("context")).collect();
        let (bindings, _) = keymap.bindings_for_input(&[Keystroke::parse(input).expect("keystroke")], &stack);
        bindings.iter().map(|binding| binding.action().boxed_clone()).collect()
    }

    fn wins(keymap: &Keymap, input: &'static str, stack: &[&'static str], action: &dyn Action) -> bool {
        resolved(keymap, input, stack).first().is_some_and(|found| found.partial_eq(action))
    }

    #[test]
    fn search_field_arrows_enter_and_leave_the_results() {
        let keymap = keymap();
        let field = ["Crow", "Search", "TextInput"];
        let results = ["Crow", "Search", "SearchResults"];
        assert!(wins(&keymap, "down", &field, &SearchFocusResults), "↓ in the field enters the results");
        assert!(wins(&keymap, "up", &results, &SearchFocusField), "↑ on a row leaves them");
        // The Vim layer's letters keep typing in the field.
        assert!(!wins(&keymap, "j", &field, &MoveDown));
    }

    #[test]
    fn filter_field_context_disables_list_keys() {
        let keymap = keymap();
        let field = ["Crow", "FileList ColumnView vim Filtered", "Filter", "TextInput"];
        // The filter field is inside the list. Plain characters must reach it, not
        // trigger list actions.
        for input in ["j", "l", "/", "space", "cmd-backspace", "cmd-d"] {
            assert!(!wins(&keymap, input, &field, &MoveDown), "{input} must not move down");
            assert!(!wins(&keymap, input, &field, &MoveRight), "{input} must not move right");
            assert!(!wins(&keymap, input, &field, &StartFilter), "{input} must not reopen the filter");
            assert!(!wins(&keymap, input, &field, &QuickLook), "{input} must not quick look");
            assert!(!wins(&keymap, input, &field, &MoveToTrash), "{input} must not trash");
            assert!(!wins(&keymap, input, &field, &Duplicate), "{input} must not duplicate");
        }
        assert!(wins(&keymap, "up", &field, &MoveUp));
        assert!(wins(&keymap, "cmd-f", &field, &ShowSearch));
        assert!(wins(&keymap, "escape", &field, &crate::text_input::Cancel));

        // List focused while filtering: the M6 keys still work.
        let list = ["Crow", "FileList vim Filtered"];
        assert!(wins(&keymap, "/", &list, &StartFilter));
        assert!(wins(&keymap, "j", &list, &MoveDown));
        assert!(wins(&keymap, "escape", &list, &ClearFilter));

        // `!Filter` must not match `Filtered`.
        let unfiltered = ["Crow", "FileList vim"];
        assert!(wins(&keymap, "/", &unfiltered, &StartFilter));
        assert!(wins(&keymap, "j", &unfiltered, &MoveDown));
    }
}
