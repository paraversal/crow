//! The file list's right-click menus, built from `palette::COMMANDS` so their titles,
//! actions and shortcuts are the menu bar's and the palette's. `crow_macos` shows
//! them as native menus; the chosen command runs as its action on the list.

use crow_macos::{ContextMenuEntry, MenuShortcut};
use gpui::{App, KeyBinding};

use crate::keys::{self, KeyScope};
use crate::palette::{COMMANDS, Command, CommandId};
use crate::ui::Ui;

/// What was right-clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKind {
    /// A row, or the selection it belongs to.
    Items,
    /// Empty space: the folder shown.
    Folder,
}

/// One line of a menu: a command, or a separator (`None`).
pub type Slot = Option<CommandId>;

const ITEMS: &[Slot] = &[
    Some(CommandId::Open),
    Some(CommandId::QuickLook),
    None,
    Some(CommandId::Rename),
    Some(CommandId::Duplicate),
    Some(CommandId::Copy),
    Some(CommandId::Cut),
    Some(CommandId::Paste),
    None,
    Some(CommandId::CopyPath),
    Some(CommandId::ShowInFinder),
    Some(CommandId::AddToSidebar),
    None,
    Some(CommandId::MoveToTrash),
];

const FOLDER: &[Slot] = &[
    Some(CommandId::NewFolder),
    Some(CommandId::NewFile),
    None,
    Some(CommandId::Paste),
    None,
    Some(CommandId::ShowInFinder),
];

pub fn slots(kind: MenuKind) -> &'static [Slot] {
    match kind {
        MenuKind::Items => ITEMS,
        MenuKind::Folder => FOLDER,
    }
}

/// What decides which items can act right now.
#[derive(Clone, Copy, Debug, Default)]
pub struct MenuState {
    pub selected: usize,
    pub selected_folders: usize,
    pub can_paste: bool,
}

pub fn enabled(id: CommandId, state: MenuState) -> bool {
    match id {
        CommandId::Paste => state.can_paste,
        // Rename edits one name in place.
        CommandId::Rename => state.selected == 1,
        // Only folders go in the sidebar.
        CommandId::AddToSidebar => state.selected_folders > 0,
        _ => true,
    }
}

pub fn command(id: CommandId) -> &'static Command {
    COMMANDS.iter().find(|command| command.id == id).expect("every menu command is registered")
}

/// The title the menu bar uses, so both menus read the same.
pub fn title(command: &Command) -> &'static str {
    command.menu.map_or(command.label, |slot| slot.title)
}

/// The native menu's lines for `kind`.
pub fn entries(kind: MenuKind, state: MenuState, scope: KeyScope, cx: &App) -> Vec<ContextMenuEntry> {
    slots(kind)
        .iter()
        .map(|slot| match slot {
            None => ContextMenuEntry::Separator,
            Some(id) => {
                let command = command(*id);
                ContextMenuEntry::Item {
                    title: title(command).to_owned(),
                    shortcut: shortcut(command, scope, cx),
                    enabled: enabled(*id, state),
                }
            }
        })
        .collect()
}

/// The shortcut a native menu can show: the winning single-keystroke binding in
/// `scope`. Vim chords like `y y` can't be shown, so ⌘C stays visible with the Vim layer on.
fn shortcut(command: &Command, scope: KeyScope, cx: &App) -> Option<MenuShortcut> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let stack = keys::context_stack(scope, cx.global::<Ui>().settings.keymap);
    let binding: Option<KeyBinding> = keymap
        .bindings_for_action(command.action)
        .rev()
        .filter(|binding| binding.keystrokes().len() == 1)
        .find(|binding| {
            let (resolved, _) = keymap.bindings_for_input(binding.keystrokes(), &stack);
            resolved.first().is_some_and(|found| found.action().partial_eq(command.action))
        })
        .cloned();
    let keystroke = binding?.keystrokes().first()?.clone();
    let modifiers = keystroke.modifiers();
    Some(MenuShortcut {
        key: keystroke.key().to_owned(),
        cmd: modifiers.platform,
        shift: modifiers.shift,
        alt: modifiers.alt,
        ctrl: modifiers.control,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_slot_is_a_registered_command_without_doubled_separators() {
        for kind in [MenuKind::Items, MenuKind::Folder] {
            let slots = slots(kind);
            assert!(slots.first().is_some_and(Option::is_some) && slots.last().is_some_and(Option::is_some));
            assert!(!slots.windows(2).any(|pair| pair[0].is_none() && pair[1].is_none()));
            for id in slots.iter().flatten() {
                assert!(command(*id).menu.is_some(), "{id:?} is in the menu bar too");
            }
        }
    }

    #[test]
    fn items_that_cannot_act_are_disabled() {
        let one_file = MenuState { selected: 1, selected_folders: 0, can_paste: false };
        assert!(!enabled(CommandId::Paste, one_file));
        assert!(!enabled(CommandId::AddToSidebar, one_file));
        assert!(enabled(CommandId::Rename, one_file));
        let two = MenuState { selected: 2, selected_folders: 1, can_paste: true };
        assert!(enabled(CommandId::Paste, two));
        assert!(enabled(CommandId::AddToSidebar, two));
        assert!(!enabled(CommandId::Rename, two), "one name at a time");
    }
}
