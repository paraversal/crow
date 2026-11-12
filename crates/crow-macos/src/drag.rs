use std::cell::RefCell;
use std::path::PathBuf;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AnyThread, msg_send};
use objc2_app_kit::{
    NSApplication, NSBezierPath, NSColor, NSDraggingItem, NSEvent, NSFont, NSFontAttributeName,
    NSForegroundColorAttributeName, NSImage, NSStringDrawing, NSView,
};
use objc2_foundation::{NSArray, NSAttributedStringKey, NSData, NSDictionary, NSPoint, NSRect, NSSize, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::{main_thread, ns_url};

thread_local! {
    /// AppKit starts a drag from the mouse-down event; the drag itself begins on a later move.
    static LAST_MOUSE_DOWN: RefCell<Option<Retained<NSEvent>>> = const { RefCell::new(None) };
}

/// Remember the current mouse-down so a drag that starts later can be handed to AppKit properly.
pub fn remember_mouse_down() {
    let Some(mtm) = main_thread() else { return };
    let Some(event) = NSApplication::sharedApplication(mtm).currentEvent() else { return };
    LAST_MOUSE_DOWN.with(|down| *down.borrow_mut() = Some(event));
}

/// What the native drag image shows, matching the GPUI onhover combo.
pub struct ComboDrag<'a> {
    pub paths: &'a [(PathBuf, bool)],
    /// PNG bytes for the first item's icon, at the combo's 22 pt size.
    pub icon_png: &'a [u8],
    pub label: &'a str,
    /// Combo size in points.
    pub width: f32,
    pub height: f32,
    /// The pill's width, already including its padding.
    pub pill_width: f32,
    /// Where the cursor sits inside the combo.
    pub cursor_from_left: f32,
    pub cursor_from_top: f32,
}

const ICON: f64 = 22.0;
const GAP: f64 = 8.0;

fn ns_view(window: &impl HasWindowHandle) -> Option<Retained<NSView>> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else { return None };
    // SAFETY: the window owns this live NSView; retaining it only extends its lifetime.
    unsafe { Retained::retain(appkit.ns_view.as_ptr().cast::<NSView>()) }
}

/// Draw the same icon + name pill the GPUI drag shows in the window.
#[allow(deprecated)] // NSImage's focus-based drawing is the simplest way to compose the combo.
fn combo_image(drag: &ComboDrag) -> Option<Retained<NSImage>> {
    let height = drag.height as f64;
    let image = NSImage::initWithSize(NSImage::alloc(), NSSize::new(drag.width as f64, height));
    image.lockFocus();
    let data = NSData::with_bytes(drag.icon_png);
    if let Some(icon) = NSImage::initWithData(NSImage::alloc(), &data) {
        icon.drawInRect(NSRect::new(NSPoint::new(0., (height - ICON) / 2.), NSSize::new(ICON, ICON)));
    }

    let pill = NSRect::new(NSPoint::new(ICON + GAP, 2.), NSSize::new(drag.pill_width as f64, height - 4.));
    let radius = (height - 4.) / 2.;
    let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(pill, radius, radius);
    NSColor::colorWithWhite_alpha(0.12, 0.92).setFill();
    path.fill();

    let font = NSFont::systemFontOfSize(12.);
    let color = NSColor::whiteColor();
    // SAFETY: these framework constants are valid attribute keys for the whole process.
    let attrs: Retained<NSDictionary<NSAttributedStringKey, AnyObject>> = unsafe {
        NSDictionary::from_slices(&[NSFontAttributeName, NSForegroundColorAttributeName], &[&*font, &*color])
    };
    let text = NSString::from_str(drag.label);
    // SAFETY: the attributes dictionary holds a font and a color.
    let size = unsafe { text.sizeWithAttributes(Some(&attrs)) };
    let y = (height - size.height) / 2.;
    // SAFETY: as above.
    unsafe { text.drawAtPoint_withAttributes(NSPoint::new(ICON + GAP + 8., y), Some(&attrs)) };
    image.unlockFocus();
    Some(image)
}

/// Start a native file drag whose image is the same icon + pill combo as the in-window one.
/// Call with the current mouse event; returns false when no native view/event is available.
pub fn begin_drag(window: &impl HasWindowHandle, drag: &ComboDrag) -> bool {
    let Some(mtm) = main_thread() else { return false };
    let Some(view) = ns_view(window) else { return false };
    let event = LAST_MOUSE_DOWN
        .with(|down| down.borrow().clone())
        .or_else(|| NSApplication::sharedApplication(mtm).currentEvent());
    let Some(event) = event else { return false };
    let at = view.convertPoint_fromView(event.locationInWindow(), None);
    let Some(image) = combo_image(drag) else { return false };
    let empty: Retained<NSImage> = NSImage::initWithSize(NSImage::alloc(), NSSize::new(1., 1.));
    let height = drag.height as f64;
    // The GPUI view is flipped, so its y grows downwards.
    let flipped: bool = unsafe { msg_send![&*view, isFlipped] };
    let y = if flipped {
        at.y - drag.cursor_from_top as f64 - height
    } else {
        at.y - (height - drag.cursor_from_top as f64)
    };
    let combo_frame =
        NSRect::new(NSPoint::new(at.x - drag.cursor_from_left as f64, y), NSSize::new(drag.width as f64, height));

    let items: Vec<Retained<NSDraggingItem>> = drag
        .paths
        .iter()
        .enumerate()
        .map(|(i, (path, _is_dir))| {
            let url = ns_url(path);
            let item =
                NSDraggingItem::initWithPasteboardWriter(NSDraggingItem::alloc(), ProtocolObject::from_ref(&*url));
            // The first item's image is the combo; the rest only contribute their file URL
            // to the pasteboard, so their image is a transparent 1 pt square.
            let (frame, contents) = if i == 0 {
                (combo_frame, &image)
            } else {
                (NSRect::new(NSPoint::new(at.x, at.y), NSSize::new(1., 1.)), &empty)
            };
            // SAFETY: `contents` is an NSImage, as AppKit expects.
            unsafe { item.setDraggingFrame_contents(frame, Some(&**contents)) };
            item
        })
        .collect();
    let items = NSArray::from_retained_slice(&items);
    let Some(native_window) = view.window() else { return false };
    // GPUI's window implements the dragging source protocol; using it, like GPUI's own
    // promotion does, keeps AppKit's tracking of the image consistent.
    // SAFETY: the view, array, event and window are all live for this call.
    let session: *mut AnyObject = unsafe {
        msg_send![
            &*view,
            beginDraggingSessionWithItems: Retained::as_ptr(&items),
            event: Retained::as_ptr(&event),
            source: Retained::as_ptr(&native_window),
        ]
    };
    !session.is_null()
}
