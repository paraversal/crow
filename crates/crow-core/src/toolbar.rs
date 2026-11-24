//! What the toolbar's folder-menu group holds: the menu button, then the buttons
//! pinned from it. UI-free, so the layout and the pin list are tested here; the app
//! maps each id to a command, an icon and a label. The toolbar's other buttons are not
//! items yet; a customizable toolbar can take them in as more variants.

/// The folder menu's commands that can sit on the toolbar, by the id the palette and
/// `settings.toml` use. Their order is the order of the menu.
pub const PINNABLE: &[&str] = &["new-folder", "new-file", "paste", "copy-path", "show-in-finder", "open-terminal"];

/// One thing on the toolbar's folder group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolbarItem {
    /// The button that opens the folder's whole menu.
    FolderMenu,
    /// A command pinned from that menu.
    Pinned(&'static str),
}

/// The folder group, left to right: the menu button, then the pins in the order
/// they were pinned. Unknown ids (a newer or hand-edited file) and repeats are
/// dropped. Narrow windows show only the menu button; see [`FolderGroup::collapsed`].
pub fn folder_group(pins: &[String]) -> Vec<ToolbarItem> {
    let mut items = vec![ToolbarItem::FolderMenu];
    for pin in pins {
        if let Some(id) = PINNABLE.iter().find(|id| **id == pin) {
            let item = ToolbarItem::Pinned(id);
            if !items.contains(&item) {
                items.push(item);
            }
        }
    }
    items
}

/// The same group in a window too narrow for pins: the menu button alone, which still
/// reaches every pinned command.
pub fn collapsed(items: &[ToolbarItem]) -> Vec<ToolbarItem> {
    items.iter().copied().filter(|item| *item == ToolbarItem::FolderMenu).collect()
}

/// Whether `id` is pinned.
pub fn is_pinned(pins: &[String], id: &str) -> bool {
    pins.iter().any(|pin| pin == id)
}

/// `pins` with `id` pinned (at the end) or, when it already is, unpinned. Ids the
/// toolbar cannot hold leave the list as it was.
pub fn toggled(pins: &[String], id: &str) -> Vec<String> {
    if !PINNABLE.contains(&id) {
        return pins.to_vec();
    }
    if is_pinned(pins, id) {
        pins.iter().filter(|pin| *pin != id).cloned().collect()
    } else {
        let mut pins = pins.to_vec();
        pins.push(id.to_owned());
        pins
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pins(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn the_group_is_the_menu_then_the_pins_in_order() {
        assert_eq!(folder_group(&[]), [ToolbarItem::FolderMenu]);
        assert_eq!(
            folder_group(&pins(&["paste", "new-folder"])),
            [ToolbarItem::FolderMenu, ToolbarItem::Pinned("paste"), ToolbarItem::Pinned("new-folder")]
        );
    }

    #[test]
    fn unknown_and_repeated_ids_are_dropped() {
        assert_eq!(
            folder_group(&pins(&["new-folder", "nope", "new-folder", "rename"])),
            [ToolbarItem::FolderMenu, ToolbarItem::Pinned("new-folder")]
        );
    }

    #[test]
    fn a_narrow_window_keeps_only_the_menu_button() {
        let group = folder_group(&pins(&["new-folder", "copy-path"]));
        assert_eq!(collapsed(&group), [ToolbarItem::FolderMenu]);
    }

    #[test]
    fn toggling_pins_at_the_end_and_unpins_in_place() {
        let one = toggled(&[], "new-file");
        assert_eq!(one, ["new-file"]);
        let two = toggled(&one, "new-folder");
        assert_eq!(two, ["new-file", "new-folder"]);
        assert_eq!(toggled(&two, "new-file"), ["new-folder"]);
        assert_eq!(toggled(&two, "rename"), two, "an id the toolbar cannot hold changes nothing");
    }
}
