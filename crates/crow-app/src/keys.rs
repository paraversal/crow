//! Key text from the live keymap. The palette, tooltips and hints show the
//! binding that will actually fire in a given context, including the Vim layer,
//! so displayed keys can never drift from `actions.rs`.

use crow_core::KeymapLayer;
use gpui::{Action, App, KeyBinding, KeyContext, SharedString};

use crate::ui::Ui;

/// The context a shortcut is shown for. Each variant mirrors a `key_context`
/// in the views; keep `context_stack` in step with those calls.
// Scopes for U3–U7 as well as the palette and tooltips built here.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyScope {
    Window,
    List,
    Columns,
    ListFiltered,
    ColumnsFiltered,
    Sidebar,
    Filter,
    /// The bottom bar's path field (⌘L): the window root plus the text input.
    PathBar,
    Search,
    SearchResults,
    Palette,
    ThemePicker,
    Sheet,
}

fn context(source: &str) -> KeyContext {
    KeyContext::parse(source).expect("key context literals parse")
}

/// The real nesting for `scope`, as the views build it: `Crow` is the window
/// root, and the Vim layer adds `vim` to the views that append it.
pub fn context_stack(scope: KeyScope, layer: KeymapLayer) -> Vec<KeyContext> {
    let vim = layer == KeymapLayer::Vim;
    match scope {
        KeyScope::Window => vec![context("Crow")],
        KeyScope::List => vec![context("Crow"), context(if vim { "FileList vim" } else { "FileList" })],
        KeyScope::Columns => {
            vec![context("Crow"), context(if vim { "FileList ColumnView vim" } else { "FileList ColumnView" })]
        }
        KeyScope::ListFiltered => {
            vec![context("Crow"), context(if vim { "FileList Filtered vim" } else { "FileList Filtered" })]
        }
        KeyScope::ColumnsFiltered => vec![
            context("Crow"),
            context(if vim { "FileList ColumnView Filtered vim" } else { "FileList ColumnView Filtered" }),
        ],
        KeyScope::Sidebar => vec![context("Crow"), context(if vim { "Sidebar vim" } else { "Sidebar" })],
        KeyScope::Filter => vec![
            context("Crow"),
            // The field lives inside the list/column, so the list's contexts are on its path.
            context(if vim { "FileList vim Filtered" } else { "FileList Filtered" }),
            context("Filter"),
            context("TextInput"),
        ],
        KeyScope::PathBar => vec![context("Crow"), context("TextInput")],
        KeyScope::Search => vec![context("Crow"), context("Search"), context("TextInput")],
        KeyScope::SearchResults => {
            vec![context("Crow"), context("Search"), context(if vim { "SearchResults vim" } else { "SearchResults" })]
        }
        KeyScope::Palette => vec![
            context("Crow"),
            context(if vim { "Sheet Palette vim" } else { "Sheet Palette" }),
            context("TextInput"),
        ],
        KeyScope::ThemePicker => {
            vec![context("Crow"), context(if vim { "Sheet ThemePicker vim" } else { "Sheet ThemePicker" })]
        }
        KeyScope::Sheet => vec![context("Crow"), context("Sheet")],
    }
}

/// The binding that wins for `action` in `scope`: the last one added whose own
/// context applies and whose keystrokes resolve to this action. The context
/// check keeps another scope's binding (the search results' Return) from
/// standing in for this scope's key (the sidebar's →).
pub fn binding_for(action: &dyn Action, scope: KeyScope, cx: &App) -> Option<KeyBinding> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let stack = context_stack(scope, cx.global::<Ui>().settings.keymap);
    keymap
        .bindings_for_action(action)
        .rev()
        .find(|binding| {
            // A binding for another scope can share this action's keystrokes (the
            // search results' Return and the sidebar's Return); only its own
            // predicate decides whether it is the one shown.
            if !binding.predicate().is_none_or(|predicate| predicate.depth_of(&stack).is_some()) {
                return false;
            }
            let (resolved, _) = keymap.bindings_for_input(binding.keystrokes(), &stack);
            resolved.first().is_some_and(|found| found.action().partial_eq(action))
        })
        .cloned()
}

/// Modifiers as symbols, in Apple's order: ⌃ ⌥ ⇧ ⌘.
fn symbols(modifiers: &gpui::Modifiers) -> Vec<&'static str> {
    let mut out = Vec::with_capacity(4);
    if modifiers.control {
        out.push("⌃");
    }
    if modifiers.alt {
        out.push("⌥");
    }
    if modifiers.shift {
        out.push("⇧");
    }
    if modifiers.platform {
        out.push("⌘");
    }
    out
}

/// Modifier words, in the same order as the symbols: Control Option Shift Command.
fn words(modifiers: &gpui::Modifiers) -> Vec<&'static str> {
    let mut out = Vec::with_capacity(4);
    if modifiers.control {
        out.push("Control");
    }
    if modifiers.alt {
        out.push("Option");
    }
    if modifiers.shift {
        out.push("Shift");
    }
    if modifiers.platform {
        out.push("Command");
    }
    out
}

fn is_letter(key: &str) -> bool {
    key.len() == 1 && key.as_bytes()[0].is_ascii_alphabetic()
}

fn function_key(key: &str) -> Option<&'static str> {
    Some(match key {
        "backspace" => "⌫",
        "delete" => "⌦",
        "enter" => "↩",
        "escape" => "esc",
        "tab" => "⇥",
        "space" => "Space",
        "up" => "↑",
        "down" => "↓",
        "left" => "←",
        "right" => "→",
        "home" => "↖",
        "end" => "↘",
        "pageup" => "⇞",
        "pagedown" => "⇟",
        _ => return None,
    })
}

/// Function keys display as `F2`, as on the keyboard.
fn is_function_key(key: &str) -> bool {
    key.strip_prefix('f').is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

fn spoken_key(key: &str) -> &'static str {
    match key {
        "backspace" => "Delete",
        "delete" => "Forward Delete",
        "enter" => "Return",
        "escape" => "Escape",
        "tab" => "Tab",
        "space" => "Space",
        "up" => "Up Arrow",
        "down" => "Down Arrow",
        "left" => "Left Arrow",
        "right" => "Right Arrow",
        "home" => "Home",
        "end" => "End",
        "pageup" => "Page Up",
        "pagedown" => "Page Down",
        _ => "",
    }
}

/// One keystroke's caps, modifiers first.
fn keystroke_caps(keystroke: &gpui::KeybindingKeystroke) -> Vec<SharedString> {
    let key = keystroke.key();
    let modifiers = keystroke.modifiers();
    // Vim-style keys carry no ⌃⌥⌘: a bare letter stays lower-case and Shift
    // turns it into the upper-case letter, without the ⇧ symbol.
    if !modifiers.control && !modifiers.alt && !modifiers.platform && is_letter(key) {
        let text = if modifiers.shift { key.to_ascii_uppercase() } else { key.to_owned() };
        return vec![text.into()];
    }
    let mut caps: Vec<SharedString> = symbols(modifiers).into_iter().map(SharedString::from).collect();
    let text = match function_key(key) {
        Some(symbol) => symbol.to_owned(),
        None if is_letter(key) || is_function_key(key) => key.to_ascii_uppercase(),
        None => key.to_owned(),
    };
    caps.push(text.into());
    caps
}

fn keystroke_text(keystroke: &gpui::KeybindingKeystroke) -> String {
    keystroke_caps(keystroke).concat()
}

fn keystroke_spoken(keystroke: &gpui::KeybindingKeystroke) -> String {
    let key = keystroke.key();
    let modifiers = keystroke.modifiers();
    if !modifiers.control && !modifiers.alt && !modifiers.platform && is_letter(key) {
        return if modifiers.shift { key.to_ascii_uppercase() } else { key.to_owned() };
    }
    let mut text = words(modifiers).join(" ");
    let spoken = spoken_key(key);
    let key_word = if !spoken.is_empty() {
        spoken.to_owned()
    } else if is_letter(key) || is_function_key(key) {
        key.to_ascii_uppercase()
    } else {
        key.to_owned()
    };
    if !text.is_empty() {
        text.push(' ');
    }
    text.push_str(&key_word);
    text
}

/// One cap per key, modifiers first. A multi-key sequence (Vim's `gg`) is one
/// cap, glued, because it reads as a single shortcut.
pub fn caps(binding: &KeyBinding) -> Vec<SharedString> {
    let keystrokes = binding.keystrokes();
    if keystrokes.len() > 1 {
        let mut glued = String::new();
        for keystroke in keystrokes {
            glued.push_str(&keystroke_text(keystroke));
        }
        return vec![glued.into()];
    }
    let Some(keystroke) = keystrokes.first() else { return Vec::new() };
    keystroke_caps(keystroke)
}

/// The whole shortcut as one string: `⇧⌘A`, `gg`, `esc`.
pub fn chord_text(binding: &KeyBinding) -> SharedString {
    caps(binding).concat().into()
}

/// The shortcut in words, for aria labels: "Shift Command A", "Escape".
pub fn spoken(binding: &KeyBinding) -> String {
    binding.keystrokes().iter().map(keystroke_spoken).collect::<Vec<_>>().join(" ")
}

/// The displayed shortcut for `action` in `scope`, when it has one there.
pub fn text(action: &dyn Action, scope: KeyScope, cx: &App) -> Option<SharedString> {
    binding_for(action, scope, cx).as_ref().map(chord_text)
}

/// `text` in words, for aria labels.
pub fn spoken_for(action: &dyn Action, scope: KeyScope, cx: &App) -> Option<String> {
    binding_for(action, scope, cx).as_ref().map(spoken)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::*;
    use crate::palette::{COMMANDS, CommandId};
    use gpui::{App, TestAppContext};

    fn setup(cx: &mut TestAppContext) {
        cx.update(|cx: &mut App| {
            crate::ui::init_for_tests(cx);
            cx.bind_keys(crate::actions::finder_keymap());
            cx.bind_keys(crate::actions::vim_keymap());
            cx.bind_keys(crate::text_input::keymap());
        });
    }

    fn binding(keys: &'static str) -> KeyBinding {
        KeyBinding::new(keys, MoveUp, None)
    }

    fn format(keys: &'static str) -> String {
        chord_text(&binding(keys)).to_string()
    }

    #[gpui::test]
    fn formats_mac_symbols_in_apple_order(_cx: &mut TestAppContext) {
        assert_eq!(format("cmd-shift-a"), "⇧⌘A");
        assert_eq!(format("ctrl-cmd-t"), "⌃⌘T");
        assert_eq!(format("cmd-alt-backspace"), "⌥⌘⌫");
        assert_eq!(format("ctrl-shift-tab"), "⌃⇧⇥");
        assert_eq!(format("cmd-shift-."), "⇧⌘.");
        assert_eq!(format("cmd-up"), "⌘↑");
        assert_eq!(format("escape"), "esc");
        assert_eq!(format("space"), "Space");
        assert_eq!(format("enter"), "↩");
        assert_eq!(format("/"), "/");
        assert_eq!(format("shift-g"), "G");
        assert_eq!(format("g g"), "gg");
        let caps: Vec<String> = caps(&binding("cmd-shift-a")).iter().map(ToString::to_string).collect();
        assert_eq!(caps, vec!["⇧", "⌘", "A"]);
        assert_eq!(spoken(&binding("cmd-f")), "Command F");
    }

    #[gpui::test]
    fn finds_bindings_by_scope(cx: &mut TestAppContext) {
        setup(cx);
        cx.update(|cx| {
            assert_eq!(text(&ShowSearch, KeyScope::Window, cx).as_deref(), Some("⌘F"));
            assert_eq!(text(&StartFilter, KeyScope::List, cx).as_deref(), Some("/"));
            assert_eq!(text(&StartFilter, KeyScope::Window, cx), None);
            assert_eq!(text(&MoveToTrash, KeyScope::List, cx).as_deref(), Some("⌘⌫"));
            assert_eq!(text(&RemoveFromSidebar, KeyScope::Sidebar, cx).as_deref(), Some("⌘⌫"));
            assert_eq!(text(&CancelOperation, KeyScope::List, cx).as_deref(), Some("esc"));
            assert_eq!(text(&SheetCancel, KeyScope::Sheet, cx).as_deref(), Some("esc"));
            assert_eq!(text(&OpenSelection, KeyScope::List, cx).as_deref(), Some("⌘↓"));
        });
    }

    #[gpui::test]
    fn vim_layer_shows_vim_keys(cx: &mut TestAppContext) {
        setup(cx);
        cx.update(|cx| cx.global_mut::<Ui>().settings.keymap = KeymapLayer::Vim);
        cx.update(|cx| {
            assert_eq!(text(&MoveToFirst, KeyScope::List, cx).as_deref(), Some("gg"));
            assert_eq!(text(&CopyFiles, KeyScope::List, cx).as_deref(), Some("yy"));
            assert_eq!(text(&MoveDown, KeyScope::Sidebar, cx).as_deref(), Some("j"));
            assert_eq!(text(&Undo, KeyScope::Window, cx).as_deref(), Some("⌘Z"));
        });
    }

    /// `docs/keys.md`, written from the live keymap. Run with `CROW_UPDATE_DOCS=1` to rewrite it
    /// after a keymap change; otherwise the test fails when the file drifts.
    #[gpui::test]
    fn keys_doc_matches_the_live_keymap(cx: &mut TestAppContext) {
        use crate::palette::MenuName;
        setup(cx);
        let mut out = String::from(
            "# Keys\n\n\
             Generated from Crow's keymap by `cargo test -p crow-app keys_doc` (`CROW_UPDATE_DOCS=1` rewrites it); do not edit by hand.\n\n\
             Every command is in the menu bar and in the palette (`>`). The Finder layer is the default; `keymap = \"vim\"` adds the Vim layer, which keeps the Finder keys for menu commands. \
             Rebind any row in Settings ▸ Keyboard or in `keys.toml` with the id in the last column.\n\n\
             Movement keys (arrows, Return, Space and Esc in lists, the sidebar, search results and sheets) are not rebindable.\n",
        );
        let layer_text = |cx: &mut TestAppContext, layer: KeymapLayer, command: &crate::palette::Command| {
            cx.update(|cx| {
                cx.global_mut::<Ui>().settings.keymap = layer;
                text(command.action, command.scope, cx).map(|t| t.to_string())
            })
        };
        let mut menus: Vec<(String, Option<MenuName>)> =
            MenuName::ALL.iter().map(|m| (format!("{m:?}"), Some(*m))).collect();
        menus.push(("Palette only".to_owned(), None));
        for (title, menu) in menus {
            let mut rows = String::new();
            for command in COMMANDS.iter().filter(|c| c.menu.map(|slot| slot.menu) == menu) {
                let finder = layer_text(cx, KeymapLayer::Finder, command);
                let vim = layer_text(cx, KeymapLayer::Vim, command);
                if finder.is_none() && vim.is_none() {
                    continue;
                }
                let vim_cell = match (&finder, &vim) {
                    (a, b) if a == b => String::new(),
                    (_, Some(v)) => format!("`{v}`"),
                    (_, None) => String::new(),
                };
                rows.push_str(&format!(
                    "| {} | {} | {} | `{}` |\n",
                    command.label,
                    finder.map(|f| format!("`{f}`")).unwrap_or_default(),
                    vim_cell,
                    command.id.key()
                ));
            }
            if !rows.is_empty() {
                out.push_str(&format!(
                    "\n## {title}\n\n| Command | Finder | Vim layer | Id |\n| --- | --- | --- | --- |\n{rows}"
                ));
            }
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/keys.md");
        if std::env::var_os("CROW_UPDATE_DOCS").is_some() {
            std::fs::write(&path, &out).unwrap();
        }
        let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
        assert_eq!(on_disk, out, "docs/keys.md is out of date: run CROW_UPDATE_DOCS=1 cargo test -p crow-app keys_doc");
    }

    fn action(id: CommandId) -> &'static dyn Action {
        COMMANDS.iter().find(|c| c.id == id).expect("command is registered").action
    }

    #[gpui::test]
    fn palette_keys_match_the_old_strings(cx: &mut TestAppContext) {
        setup(cx);
        cx.update(|cx| {
            let expected: &[(CommandId, &str)] = &[
                (CommandId::NewTab, "⌘T"),
                (CommandId::CloseTab, "⌘W"),
                (CommandId::NextTab, "⌃⇥"),
                (CommandId::PreviousTab, "⌃⇧⇥"),
                (CommandId::NewWindow, "⌘N"),
                (CommandId::SwitchFocus, "⇥"),
                (CommandId::ToggleHiddenFiles, "⇧⌘."),
                (CommandId::TogglePreview, "⇧⌘P"),
                (CommandId::ToggleSidebar, "⌃⌘S"),
                (CommandId::ShowInfo, "⌘I"),
                (CommandId::OpenParent, "⌘↑"),
                (CommandId::GoBack, "⌘["),
                (CommandId::GoForward, "⌘]"),
                (CommandId::GoToFolder, "⇧⌘G"),
                (CommandId::EditPath, "⌘L"),
                (CommandId::ViewAsIcons, "⌘1"),
                (CommandId::ViewAsList, "⌘2"),
                (CommandId::ViewAsColumns, "⌘3"),
                (CommandId::ViewAsGallery, "⌘4"),
                (CommandId::Open, "⌘↓"),
                (CommandId::QuickLook, "Space"),
                (CommandId::Rename, "F2"),
                (CommandId::NewFolder, "⇧⌘N"),
                (CommandId::NewFile, "⌥⌘N"),
                (CommandId::Duplicate, "⌘D"),
                (CommandId::MoveToTrash, "⌘⌫"),
                (CommandId::DeletePermanently, "⌥⌘⌫"),
                (CommandId::EmptyTrash, "⇧⌘⌫"),
                (CommandId::Copy, "⌘C"),
                (CommandId::Cut, "⌘X"),
                (CommandId::Paste, "⌘V"),
                (CommandId::PasteMove, "⌥⌘V"),
                (CommandId::CopyPath, "⌥⌘C"),
                (CommandId::SelectAll, "⌘A"),
                (CommandId::ExtendSelectionToStart, "⌥⇧↑"),
                (CommandId::ExtendSelectionToEnd, "⌥⇧↓"),
                (CommandId::StartFilter, "/"),
                (CommandId::ShowSearch, "⌘F"),
                (CommandId::SearchEverything, "⇧⌘A"),
                (CommandId::SearchHome, "⇧⌘H"),
                (CommandId::ShowCommandPalette, "⌘K"),
                (CommandId::SearchReveal, "⌘R"),
                (CommandId::Undo, "⌘Z"),
                (CommandId::Redo, "⇧⌘Z"),
                (CommandId::AddToSidebar, "⌃⌘T"),
                (CommandId::WhyThisView, "⌘J"),
                (CommandId::ShowThemePicker, "⌥⌘T"),
                (CommandId::OpenSettings, "⌘,"),
                (CommandId::ToggleActivity, "⌥⌘L"),
                (CommandId::RemoveFromSidebar, "⌘⌫"),
                (CommandId::MoveFavoriteUp, "⌥⌘↑"),
                (CommandId::MoveFavoriteDown, "⌥⌘↓"),
                (CommandId::Eject, "⌘E"),
                (CommandId::Quit, "⌘Q"),
            ];
            for (id, keys) in expected {
                assert_eq!(
                    text(action(*id), COMMANDS.iter().find(|c| c.id == *id).unwrap().scope, cx).as_deref(),
                    Some(*keys),
                    "{id:?}"
                );
            }
            for id in [
                CommandId::UseViewAsDefaults,
                CommandId::ResetFolderView,
                CommandId::ShowInFinder,
                CommandId::OpenTerminal,
                CommandId::EditSettings,
                CommandId::Acknowledgements,
                CommandId::CalculateFolderSizes,
                CommandId::AddProjectRoot,
                CommandId::RemoveProjectRoot,
                CommandId::RefreshProjects,
                CommandId::ReviewGitChanges,
                CommandId::ShowGitDiff,
                CommandId::ChooseListColumns,
                CommandId::IconSizeCompact,
                CommandId::IconSizeStandard,
                CommandId::IconSizeLarge,
                CommandId::IconCaptionOff,
                CommandId::IconCaptionLines,
                CommandId::SaveSearch,
                CommandId::EditSavedSearch,
                CommandId::RemoveSavedSearch,
                CommandId::ChooseTags,
                CommandId::AddToShelf,
                CommandId::MoveShelfHere,
                CommandId::CopyShelfHere,
                CommandId::ClearShelf,
            ] {
                let command = COMMANDS.iter().find(|c| c.id == id).unwrap();
                assert_eq!(text(command.action, command.scope, cx), None, "{id:?} has no binding");
            }
            assert_eq!(COMMANDS.len(), expected.len() + 26, "every command is in the table");
        });
    }
}
