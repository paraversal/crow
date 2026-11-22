//! The menu bar, built from `palette::COMMANDS` so menus and the palette cannot drift.
//!
//! Run `init` after every `cx.bind_keys`: GPUI reads the keymap when `set_menus`
//! is called, and the menus are static for the life of the app.

use gpui::{App, Menu, MenuItem, OsAction, SystemMenuType};

use crate::actions as act;
use crate::palette::{COMMANDS, Command, CommandId, MenuName, MenuSlot};
use crate::ui::Ui;

/// One command row in the native `File ▸ Plugins` submenu.
pub struct MenuPluginCommand {
    pub extension: crow_core::ExtensionId,
    pub generation: u64,
    pub title: String,
}

fn plugin_menu_commands(cx: &App) -> Vec<MenuPluginCommand> {
    let Some(plugins) = cx.try_global::<crate::plugins::Plugins>() else { return Vec::new() };
    let snapshot = plugins.surfaces.snapshot();
    let mut commands: Vec<(String, String, crow_core::ExtensionId, u64)> = snapshot
        .of_kind(crow_core::ExtensionKind::Command)
        .filter(|(id, _)| !plugins.surfaces.is_suppressed(&id.plugin))
        .map(|(id, published)| {
            (published.package.clone(), published.descriptor.title.clone(), id.clone(), published.generation)
        })
        .collect();
    commands.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)).then(left.2.cmp(&right.2)));
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (_, title, _, _) in &commands {
        *counts.entry(title.clone()).or_insert(0usize) += 1;
    }
    commands
        .into_iter()
        .map(|(package, title, extension, generation)| {
            let title =
                if counts.get(&title).copied().unwrap_or(0) > 1 { format!("{title} — {package}") } else { title };
            MenuPluginCommand { extension, generation, title }
        })
        .collect()
}

/// The toggle names the action it will take, so the menu says what a click does.
fn menu_title(command: &Command, slot: MenuSlot, show_preview: bool, sidebar_visible: bool) -> &'static str {
    match command.id {
        CommandId::TogglePreview if show_preview => "Hide Right Sidebar",
        CommandId::TogglePreview => "Show Right Sidebar",
        CommandId::ToggleSidebar if sidebar_visible => "Hide Left Sidebar",
        CommandId::ToggleSidebar => "Show Left Sidebar",
        _ => slot.title,
    }
}

fn command_item(
    command: &Command,
    slot: MenuSlot,
    show_preview: bool,
    sidebar_visible: bool,
    folder_sizes_visible: bool,
) -> MenuItem {
    let title = menu_title(command, slot, show_preview, sidebar_visible);
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
        // The setting already measures every visible folder; the one-off command
        // would duplicate that work.
        disabled: folder_sizes_visible && command.id == CommandId::CalculateFolderSizes,
    }
}

/// The whole menu bar in macOS order: Crow, File, Edit, View, Go, Window, Help.
pub fn app_menus(
    show_preview: bool,
    sidebar_visible: bool,
    folder_sizes_visible: bool,
    plugin_commands: &[MenuPluginCommand],
) -> Vec<Menu> {
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
            items.push(command_item(command, slot, show_preview, sidebar_visible, folder_sizes_visible));
            section = Some(slot.section);
        }

        if *menu == MenuName::File && !plugin_commands.is_empty() {
            items.push(MenuItem::separator());
            items.push(MenuItem::submenu(Menu::new("Plugins").items(plugin_commands.iter().map(|command| {
                MenuItem::action(
                    command.title.clone(),
                    act::PluginCommand { extension: command.extension.clone(), generation: command.generation },
                )
            }))));
        }
        menus.push(Menu { name: menu.title().into(), items, disabled: false });
    }
    menus
}

/// Register the global handlers and install the menu bar. Call after `cx.bind_keys`.
pub fn init(cx: &mut App) {
    cx.on_action(|_: &act::Quit, cx| crate::window::operation_ui::quit(cx));
    // With a Crow window focused the root's NewWindow listener opens the current folder;
    // this fires only when there is no window (or none is listening).
    cx.on_action(|_: &act::NewWindow, cx| {
        let home = std::env::var_os("HOME").map_or_else(|| std::path::PathBuf::from("/"), std::path::PathBuf::from);
        crate::window::open(home, false, None, cx);
    });
    cx.on_action(|_: &act::About, cx| crate::effects::effects(cx).show_about_panel());
    cx.on_action(|_: &act::OpenSettings, cx| crate::settings_window::open(cx));
    cx.on_action(|_: &act::EditSettings, cx| crate::ui::edit_settings_file(cx));
    cx.on_action(|_: &act::Acknowledgements, cx| crate::ui::open_acknowledgements(cx));
    cx.on_action(|_: &act::HideApp, cx| cx.hide());
    cx.on_action(|_: &act::HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &act::ShowAll, cx| cx.unhide_other_apps());
    refresh(cx);
}

/// Rebuild the menu bar; View ▸ Show/Hide Preview follows the setting, and
/// View ▸ Calculate Folder Sizes is disabled while the setting measures on its own.
pub fn refresh(cx: &mut App) {
    let (show_preview, folder_sizes_visible) = {
        let settings = &cx.global::<Ui>().settings;
        (settings.show_preview, settings.folder_sizes == crow_core::FolderSizesMode::Visible)
    };
    let sidebar_visible = !cx.global::<Ui>().sidebar_hidden;
    cx.set_menus(app_menus(show_preview, sidebar_visible, folder_sizes_visible, &plugin_menu_commands(cx)));
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
    fn plugin_commands_form_one_file_submenu_in_package_order() {
        let id = |local: &str| {
            crow_core::ExtensionId::new(
                crow_core::PluginId::parse("example.test").unwrap(),
                crow_core::LocalId::parse(local).unwrap(),
            )
        };
        let commands = vec![
            MenuPluginCommand { extension: id("b"), generation: 2, title: "Copy".into() },
            MenuPluginCommand { extension: id("a"), generation: 1, title: "Copy".into() },
            MenuPluginCommand { extension: id("c"), generation: 3, title: "Only".into() },
        ];
        let menus = app_menus(false, true, false, &commands);
        let file = menus.iter().find(|menu| menu.name.as_ref() == "File").expect("File menu");
        let MenuItem::Submenu(plugins) = file
            .items
            .iter()
            .find(|item| matches!(item, MenuItem::Submenu(menu) if menu.name.as_ref() == "Plugins"))
            .expect("Plugins submenu")
        else {
            unreachable!()
        };
        assert_eq!(titles(&plugins.items), ["Copy", "Copy", "Only"]);
        assert!(matches!(
            &plugins.items[0],
            MenuItem::Action { action, .. } if action.partial_eq(&act::PluginCommand {
                extension: id("b"),
                generation: 2,
            })
        ));
    }

    #[test]
    fn every_menu_command_is_in_the_menus() {
        let menus = app_menus(true, true, false, &[]);
        for command in COMMANDS {
            let Some(slot) = command.menu else { continue };
            let menu = menus.iter().find(|menu| menu.name.as_ref() == slot.menu.title()).expect("menu exists");
            let matches: Vec<_> = menu
                .items
                .iter()
                .filter_map(|item| match item {
                    MenuItem::Action { name, action, .. } if name.as_ref() == menu_title(command, slot, true, true) => {
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
                            slot.menu.title() == menu.name.as_ref()
                                && menu_title(command, slot, true, true) == name.as_ref()
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
            let menus = app_menus(show_preview, true, false, &[]);
            let view = menus.iter().find(|menu| menu.name.as_ref() == "View").expect("View menu");
            titles(&view.items).into_iter().map(str::to_owned).collect::<Vec<_>>()
        };
        assert!(view_items(true).iter().any(|title| title == "Hide Right Sidebar"), "shown: the menu hides it");
        assert!(view_items(false).iter().any(|title| title == "Show Right Sidebar"), "hidden: the menu shows it");
    }

    #[test]
    fn calculate_folder_sizes_is_disabled_when_the_setting_is_visible() {
        let disabled = |folder_sizes_visible: bool| {
            let menus = app_menus(false, true, folder_sizes_visible, &[]);
            let view = menus.iter().find(|menu| menu.name.as_ref() == "View").expect("View menu");
            view.items.iter().find_map(|item| match item {
                MenuItem::Action { name, disabled, .. } if name.as_ref() == "Calculate Folder Sizes" => Some(*disabled),
                _ => None,
            })
        };
        assert_eq!(disabled(false), Some(false), "off: the one-off command is usable");
        assert_eq!(disabled(true), Some(true), "visible: the setting already measures");
    }

    #[test]
    fn menus_are_in_bar_order() {
        let menus = app_menus(true, true, false, &[]);
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
                "Empty Trash…",
                "—",
                "Add to Sidebar",
                "Eject",
                "Review Git Changes…",
                "Add Folder as Project Root",
                "Remove Folder from Project Roots",
                "Refresh Projects",
                "Show in Finder",
                "Open Terminal Here",
                "—",
                "Save Search…",
                "—",
                "Add to Shelf",
                "Move Shelf Here",
                "Copy Shelf Here",
                "Clear Shelf",
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
                "Extend Selection to Start",
                "Extend Selection to End",
                "—",
                "Filter This Folder",
                "Filter by Tag…",
                "Find…",
                "Find Everything…",
                "Search Home",
            ]
        );
        assert_eq!(
            by_name("View"),
            [
                "as Icons",
                "as List",
                "as Columns",
                "as Gallery",
                "—",
                "Icon Size: Compact",
                "Icon Size: Standard",
                "Icon Size: Large",
                "Icon Caption: Off",
                "Icon Caption: Lines Changed",
                "Choose List Columns…",
                "Why This View?",
                "Use as Defaults",
                "Reset View for This Folder",
                "—",
                "Show Hidden Files",
                "Hide Right Sidebar",
                "Hide Left Sidebar",
                "Show Info",
                "Cycle Right Sidebar",
                "Show Git Diff",
                "Calculate Folder Sizes",
                "Show Activity",
                "Theme…",
            ]
        );
        assert_eq!(
            by_name("Go"),
            ["Back", "Forward", "Enclosing Folder", "—", "Go to…", "Go to Folder…", "Edit Path", "Reveal Result"]
        );
        assert_eq!(
            by_name("Crow"),
            ["About Crow", "Settings…", "—", "Services", "—", "Hide Crow", "Hide Others", "Show All", "—", "Quit Crow",]
        );
        assert_eq!(by_name("Window"), ["Minimize", "Zoom", "—", "Show Previous Tab", "Show Next Tab", "Close Tab"]);
        assert_eq!(by_name("Help"), ["Crow Keyboard Shortcuts", "Acknowledgements"]);
    }
}
