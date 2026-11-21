//! The user's system colours for themes that follow them (Finder): the accent colour and the
//! unfocused selection grey, plus the notification that fires when either changes.

use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2_app_kit::{NSColor, NSColorSpace, NSSystemColorsDidChangeNotification};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSOperationQueue};

use crate::main_thread;

/// `0xRRGGBB` of `color` in sRGB, or `None` when it has no sRGB form.
fn srgb(color: &NSColor) -> Option<u32> {
    let color = color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    let byte = |component: f64| (component.clamp(0.0, 1.0) * 255.0).round() as u32;
    Some(byte(color.redComponent()) << 16 | byte(color.greenComponent()) << 8 | byte(color.blueComponent()))
}

/// The system accent colour (System Settings ▸ Appearance ▸ Accent colour).
pub fn accent_color() -> Option<u32> {
    main_thread()?;
    srgb(&NSColor::controlAccentColor())
}

/// The selection colour for a window that isn't focused: Finder's grey row.
pub fn unfocused_selection_color() -> Option<u32> {
    main_thread()?;
    srgb(&NSColor::unemphasizedSelectedContentBackgroundColor())
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
