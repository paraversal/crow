//! The user's system colours for themes that follow them (Finder): the accent colour and the
//! unfocused selection grey, plus the notification that fires when either changes.

use std::cell::Cell;
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2_app_kit::{
    NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSColor, NSColorSpace,
    NSSystemColorsDidChangeNotification,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSOperationQueue};

use crate::main_thread;

/// `0xRRGGBB` of `color` in sRGB, or `None` when it has no sRGB form.
fn srgb(color: &NSColor) -> Option<u32> {
    let color = color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    let byte = |component: f64| (component.clamp(0.0, 1.0) * 255.0).round() as u32;
    Some(byte(color.redComponent()) << 16 | byte(color.greenComponent()) << 8 | byte(color.blueComponent()))
}

/// Read `color` as it looks in the Light or Dark appearance. System colours answer for the
/// current drawing appearance (the OS's), which is wrong when a theme is Light on a Dark Mac.
fn in_appearance(dark: bool, color: impl Fn() -> Retained<NSColor>) -> Option<u32> {
    main_thread()?;
    // SAFETY: framework constants.
    let name = unsafe { if dark { NSAppearanceNameDarkAqua } else { NSAppearanceNameAqua } };
    let appearance = NSAppearance::appearanceNamed(name)?;
    let out = Cell::new(None);
    appearance.performAsCurrentDrawingAppearance(&RcBlock::new(|| out.set(srgb(&color()))));
    out.get()
}

/// The system accent colour (System Settings ▸ Appearance ▸ Accent colour), in the Light or
/// Dark appearance.
pub fn accent_color(dark: bool) -> Option<u32> {
    in_appearance(dark, NSColor::controlAccentColor)
}

/// The selection colour for a window that isn't focused: Finder's grey row, in the Light or
/// Dark appearance.
pub fn unfocused_selection_color(dark: bool) -> Option<u32> {
    in_appearance(dark, NSColor::unemphasizedSelectedContentBackgroundColor)
}

/// Observer of system colour changes; dropping it stops it.
pub struct SystemColorWatcher {
    token: Retained<ProtocolObject<dyn NSObjectProtocol>>,
}

impl Drop for SystemColorWatcher {
    fn drop(&mut self) {
        // SAFETY: the token came from this center's addObserver.
        unsafe { NSNotificationCenter::defaultCenter().removeObserver(self.token.as_ref() as &AnyObject) };
    }
}

/// Call `changed` on the main thread whenever the accent or another system colour changes.
/// `None` off the main thread.
pub fn watch_system_colors(changed: impl Fn() + 'static) -> Option<SystemColorWatcher> {
    main_thread()?;
    let block = RcBlock::new(move |_: NonNull<NSNotification>| changed());
    // SAFETY: framework constant; the block runs on the main queue, where `changed` was made.
    let token = unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(NSSystemColorsDidChangeNotification),
            None,
            Some(&NSOperationQueue::mainQueue()),
            &block,
        )
    };
    Some(SystemColorWatcher { token })
}
