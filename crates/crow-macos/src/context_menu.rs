//! Native context menus (`NSMenu popUpContextMenu:withEvent:forView:`).
//!
//! The menu runs AppKit's tracking loop until the user picks an item or dismisses
//! it. That loop must not start inside a GPUI event handler (GPUI holds its `App`
//! borrow there, and its own main-queue timers would fire into it), so the menu
//! opens from a fresh main-queue turn, like Quick Look, and reports back through
//! `done`.

use std::cell::Cell;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem};
use objc2_foundation::{NSInteger, NSString};
use raw_window_handle::HasWindowHandle;

use crate::drag::ns_view;
use crate::main_thread;
use crate::quick_look::on_main_queue;

/// A key shortcut shown next to an item, in GPUI's key names (`"backspace"`, `"down"`, `"c"`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MenuShortcut {
    pub key: String,
    pub cmd: bool,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextMenuEntry {
    Item {
        title: String,
        shortcut: Option<MenuShortcut>,
        enabled: bool,
    },
    Separator,
    /// A line that opens a submenu of checkable lines (`(title, checked)`). Choosing one
    /// reports `SUBMENU_BASE + entry * SUBMENU_STRIDE + line`; see [`submenu_choice`].
    Submenu {
        title: String,
        lines: Vec<(String, bool)>,
    },
}

/// A submenu line's choice is reported above any top-level index.
pub const SUBMENU_BASE: usize = 1_000_000;
/// Room for this many lines per submenu in the reported choice.
pub const SUBMENU_STRIDE: usize = 1_000;

/// The `(entry, line)` a choice names, when it came from a submenu.
pub fn submenu_choice(chosen: usize) -> Option<(usize, usize)> {
    let offset = chosen.checked_sub(SUBMENU_BASE)?;
    Some((offset / SUBMENU_STRIDE, offset % SUBMENU_STRIDE))
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; no Drop impl.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "CrowContextMenuTarget"]
    #[ivars = Cell<Option<NSInteger>>]
    struct MenuTarget;

    unsafe impl NSObjectProtocol for MenuTarget {}

    impl MenuTarget {
        #[unsafe(method(crowContextMenuChose:))]
        fn chose(&self, sender: &NSMenuItem) {
            self.ivars().set(Some(sender.tag()));
        }
    }
);

impl MenuTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Cell::new(None));
        // SAFETY: plain NSObject init.
        unsafe { msg_send![super(this), init] }
    }
}

/// Show `entries` as a context menu at the current mouse event in `window`.
/// `done` gets the index (into `entries`) of the chosen item, or `None` when the
/// menu was dismissed. Returns false (and never calls `done`) when there is no
/// event or view to open it from, e.g. off the main thread or in tests.
pub fn show_context_menu(
    window: &impl HasWindowHandle,
    entries: Vec<ContextMenuEntry>,
    done: impl FnOnce(Option<usize>) + 'static,
) -> bool {
    let Some(mtm) = main_thread() else { return false };
    let Some(view) = ns_view(window) else { return false };
    let Some(event) = NSApplication::sharedApplication(mtm).currentEvent() else { return false };
    let done = Cell::new(Some(done));
    on_main_queue(move || {
        let Some(done) = done.take() else { return };
        let target = MenuTarget::new(mtm);
        let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(""));
        menu.setAutoenablesItems(false);
        for (ix, entry) in entries.iter().enumerate() {
            let item = match entry {
                ContextMenuEntry::Separator => NSMenuItem::separatorItem(mtm),
                ContextMenuEntry::Submenu { title, lines } => {
                    let item = NSMenuItem::new(mtm);
                    item.setTitle(&NSString::from_str(title));
                    let sub = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title));
                    sub.setAutoenablesItems(false);
                    for (line, (line_title, checked)) in lines.iter().enumerate() {
                        // SAFETY: the selector is implemented by MenuTarget, set as the target below.
                        let child = unsafe {
                            NSMenuItem::initWithTitle_action_keyEquivalent(
                                NSMenuItem::alloc(mtm),
                                &NSString::from_str(line_title),
                                Some(sel!(crowContextMenuChose:)),
                                &NSString::from_str(""),
                            )
                        };
                        // SAFETY: the target outlives the menu: both live until this block returns.
                        unsafe { child.setTarget(Some(&*target as &AnyObject)) };
                        child.setTag((SUBMENU_BASE + ix * SUBMENU_STRIDE + line) as NSInteger);
                        child.setState(if *checked { 1 } else { 0 });
                        sub.addItem(&child);
                    }
                    item.setSubmenu(Some(&sub));
                    item
                }
                ContextMenuEntry::Item { title, shortcut, enabled } => {
                    let key = shortcut.as_ref().map_or_else(String::new, |s| key_equivalent(&s.key));
                    // SAFETY: the selector is implemented by MenuTarget, set as the target below.
                    let item = unsafe {
                        NSMenuItem::initWithTitle_action_keyEquivalent(
                            NSMenuItem::alloc(mtm),
                            &NSString::from_str(title),
                            Some(sel!(crowContextMenuChose:)),
                            &NSString::from_str(&key),
                        )
                    };
                    if let Some(shortcut) = shortcut {
                        item.setKeyEquivalentModifierMask(modifier_mask(shortcut));
                        item.setAllowsAutomaticKeyEquivalentLocalization(false);
                    }
                    // SAFETY: the target outlives the menu: both live until this block returns.
                    unsafe { item.setTarget(Some(&*target as &AnyObject)) };
                    item.setTag(ix as NSInteger);
                    item.setEnabled(*enabled);
                    item
                }
            };
            menu.addItem(&item);
        }
        // Blocks in AppKit's tracking loop until the menu closes.
        NSMenu::popUpContextMenu_withEvent_forView(&menu, &event, &view);
        done(target.ivars().get().and_then(|tag| usize::try_from(tag).ok()));
    });
    true
}

fn modifier_mask(shortcut: &MenuShortcut) -> NSEventModifierFlags {
    let mut mask = NSEventModifierFlags::empty();
    for (on, flag) in [
        (shortcut.cmd, NSEventModifierFlags::Command),
        (shortcut.ctrl, NSEventModifierFlags::Control),
        (shortcut.alt, NSEventModifierFlags::Option),
        (shortcut.shift, NSEventModifierFlags::Shift),
    ] {
        if on {
            mask |= flag;
        }
    }
    mask
}

/// AppKit's key-equivalent string for a GPUI key name (as GPUI's own menu bar does).
fn key_equivalent(key: &str) -> String {
    let code: u32 = match key {
        "space" => 0x20,
        "backspace" => 0x08,
        "enter" => 0x0d,
        "escape" => 0x1b,
        "tab" => 0x09,
        "delete" => 0x7f,
        "up" => 0xF700,
        "down" => 0xF701,
        "left" => 0xF702,
        "right" => 0xF703,
        "home" => 0xF729,
        "end" => 0xF72B,
        "pageup" => 0xF72C,
        "pagedown" => 0xF72D,
        other => return other.to_owned(),
    };
    char::from_u32(code).map(String::from).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_keys_become_appkit_equivalents() {
        assert_eq!(key_equivalent("backspace"), "\u{8}");
        assert_eq!(key_equivalent("down"), "\u{F701}");
        assert_eq!(key_equivalent("c"), "c");
        assert_eq!(key_equivalent("["), "[");
    }
}
