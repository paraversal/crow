//! The menu bar, built from `palette::COMMANDS` so menus and the palette cannot drift.
//!
//! Run `init` after every `cx.bind_keys`: GPUI reads the keymap when `set_menus`
//! is called, and the menus are static for the life of the app.

use gpui::{App, Menu, MenuItem, OsAction, SystemMenuType};

use crate::actions as act;
use crate::palette::{COMMANDS, Command, CommandId, MenuName, MenuSlot};
use crate::ui::Ui;

/// The toggle names the action it will take, so the menu says what a click does.
fn menu_title(command: &Command, slot: MenuSlot, show_preview: bool) -> &'static str {
    match command.id {
        CommandId::TogglePreview if show_preview => "Hide Preview",
        CommandId::TogglePreview => "Show Preview",
        _ => slot.title,
    }
}

fn command_item(command: &Command, slot: MenuSlot, show_preview: bool) -> MenuItem {
    let title = menu_title(command, slot, show_preview);
    let os_action = match command.id {
        CommandId::Undo => Some(OsAction::Undo),
        CommandId::Redo => Some(OsAction::Redo),
        CommandId::Cut => Some(OsAction::Cut),
        CommandId::Copy => Some(OsAction::Copy),
        CommandId::Paste => Some(OsAction::Paste),
        CommandId::SelectAll => Some(OsAction::SelectAll),
        _ => None,
    };
    MenuItem::Action {
        name: title.into(),
        action: command.action.boxed_clone(),
        os_action,
        checked: false,
        disabled: false,
    }
}

/// The whole menu bar in macOS order: Crow, File, Edit, View, Go, Window, Help.
pub fn app_menus(show_preview: bool) -> Vec<Menu> {
    let mut menus = Vec::with_capacity(MenuName::ALL.len());
    for menu in MenuName::ALL {
        let mut items = Vec::new();
        match menu {
            MenuName::Crow => {
                items.push(MenuItem::action("About Crow", act::About));
                items.push(MenuItem::action("Settings…", act::OpenSettings));
                items.push(MenuItem::separator());
                items.push(MenuItem::os_submenu("Services", SystemMenuType::Services));
                items.push(MenuItem::separator());
                items.push(MenuItem::action("Hide Crow", act::HideApp));
                items.push(MenuItem::action("Hide Others", act::HideOthers));
                items.push(MenuItem::action("Show All", act::ShowAll));
                items.push(MenuItem::separator());
            }
            MenuName::Window => {
                items.push(MenuItem::action("Minimize", act::Minimize));
                items.push(MenuItem::action("Zoom", act::Zoom));
                items.push(MenuItem::separator());
            }
            MenuName::Help => {
                items.push(MenuItem::action("Crow Keyboard Shortcuts", act::KeyboardShortcuts));
            }
            _ => {}
        }

        let mut section = None;
        for command in COMMANDS.iter().filter(|command| command.menu.is_some_and(|slot| slot.menu == *menu)) {
            let slot = command.menu.expect("filtered to slots");
            if section.is_some_and(|previous| previous != slot.section) {
                items.push(MenuItem::separator());
            }
            items.push(command_item(command, slot, show_preview));
            section = Some(slot.section);
        }

        menus.push(Menu { name: menu.title().into(), items, disabled: false });
    }
    menus
}

/// Register the global handlers and install the menu bar. Call after `cx.bind_keys`.
pub fn init(cx: &mut App) {
    cx.on_action(|_: &act::Quit, cx| crate::window::quit(cx));
    // With a Crow window focused the root's NewWindow listener opens the current folder;
    // this fires only when there is no window (or none is listening).
    cx.on_action(|_: &act::NewWindow, cx| {
        let home = std::env::var_os("HOME").map_or_else(|| std::path::PathBuf::from("/"), std::path::PathBuf::from);
        crate::window::open(home, false, None, cx);
    });
    cx.on_action(|_: &act::About, _| crow_macos::show_about_panel());
    cx.on_action(|_: &act::OpenSettings, cx| crate::settings_window::open(cx));
    cx.on_action(|_: &act::EditSettings, cx| crate::ui::edit_settings_file(cx));
    cx.on_action(|_: &act::HideApp, cx| cx.hide());
    cx.on_action(|_: &act::HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &act::ShowAll, cx| cx.unhide_other_apps());
    refresh(cx);
}

/// Rebuild the menu bar; View ▸ Show/Hide Preview follows the setting.
pub fn refresh(cx: &mut App) {
    let show_preview = cx.global::<Ui>().settings.show_preview;
    cx.set_menus(app_menus(show_preview));
}

#[cfg(test)]
pub fn init_for_tests(cx: &mut App) {
    refresh(cx);
}

#[cfg(test)]
pub fn is_app_action(action: &dyn gpui::Action) -> bool {
    action.partial_eq(&act::About)
        || action.partial_eq(&act::OpenSettings)
        || action.partial_eq(&act::HideApp)
        || action.partial_eq(&act::HideOthers)
        || action.partial_eq(&act::ShowAll)
        || action.partial_eq(&act::Minimize)
        || action.partial_eq(&act::Zoom)
        || action.partial_eq(&act::KeyboardShortcuts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(items: &[MenuItem]) -> Vec<&str> {
        items
            .iter()
            .map(|item| match item {
                MenuItem::Separator => "—",
                MenuItem::Action { name, .. } => name.as_ref(),
                MenuItem::SystemMenu(menu) => menu.name.as_ref(),
                MenuItem::Submenu(menu) => menu.name.as_ref(),
            })
            .collect()
    }

    #[test]
    fn every_menu_command_is_in_the_menus() {
        let menus = app_menus(true);
        for command in COMMANDS {
            let Some(slot) = command.menu else { continue };
            let menu = menus.iter().find(|menu| menu.name.as_ref() == slot.menu.title()).expect("menu exists");
            let matches: Vec<_> = menu
                .items
                .iter()
                .filter_map(|item| match item {
                    MenuItem::Action { name, action, .. } if name.as_ref() == menu_title(command, slot, true) => {
                        Some(action)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(matches.len(), 1, "{:?} appears once in {:?}", slot.title, slot.menu);
            assert!(matches[0].partial_eq(command.action), "{} runs the registry action", slot.title);
        }

        for menu in &menus {
            for item in &menu.items {
                if let MenuItem::Action { name, action, .. } = item {
                    let from_registry = COMMANDS.iter().any(|command| {
                        command.menu.is_some_and(|slot| {
                            slot.menu.title() == menu.name.as_ref() && menu_title(command, slot, true) == name.as_ref()
                        })
                    });
                    assert!(
                        from_registry || is_app_action(&**action),
                        "{:?} > {:?} is neither a command nor an app item",
                        menu.name,
                        name
                    );
                }
            }
        }
    }

    #[test]
    fn preview_menu_names_the_action() {
        let view_items = |show_preview: bool| {
            let menus = app_menus(show_preview);
            let view = menus.iter().find(|menu| menu.name.as_ref() == "View").expect("View menu");
            titles(&view.items).into_iter().map(str::to_owned).collect::<Vec<_>>()
        };
        assert!(view_items(true).iter().any(|title| title == "Hide Preview"), "shown: the menu hides it");
        assert!(view_items(false).iter().any(|title| title == "Show Preview"), "hidden: the menu shows it");
    }

    #[test]
    fn menus_are_in_bar_order() {
        let menus = app_menus(true);
        let names: Vec<&str> = menus.iter().map(|menu| menu.name.as_ref()).collect();
        assert_eq!(names, ["Crow", "File", "Edit", "View", "Go", "Window", "Help"]);

        let by_name = |name: &str| titles(&menus.iter().find(|menu| menu.name.as_ref() == name).unwrap().items);
        assert_eq!(
            by_name("File"),
            [
                "New Window",
                "New Tab",
                "New Folder",
                "New File",
                "Open",
                "—",
                "Rename",
                "Duplicate",
                "Quick Look",
                "Move to Trash",
                "Delete Permanently…",
                "—",
                "Add to Sidebar",
                "Eject",
                "Show in Finder",
            ]
        );
        assert_eq!(
            by_name("Edit"),
            [
                "Undo",
                "Redo",
                "—",
                "Cut",
                "Copy",
                "Paste",
                "Move Item Here",
                "Copy Path",
                "Select All",
                "—",
                "Filter This Folder",
                "Find…",
                "Find Everything…",
            ]
        );
        assert_eq!(
            by_name("View"),
            [
                "as List",
                "as Columns",
                "—",
                "Why This View?",
                "Use as Defaults",
                "Reset View for This Folder",
                "—",
                "Show Hidden Files",
                "Hide Preview",
                "Show Activity",
                "Theme…",
            ]
        );
        assert_eq!(
            by_name("Go"),
            ["Back", "Forward", "Enclosing Folder", "—", "Go to…", "Go to Folder…", "Reveal Result"]
        );
        assert_eq!(
            by_name("Crow"),
            ["About Crow", "Settings…", "—", "Services", "—", "Hide Crow", "Hide Others", "Show All", "—", "Quit Crow",]
        );
        assert_eq!(by_name("Window"), ["Minimize", "Zoom", "—", "Show Previous Tab", "Show Next Tab", "Close Tab"]);
        assert_eq!(by_name("Help"), ["Crow Keyboard Shortcuts"]);
    }
}
