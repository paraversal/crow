use std::cell::RefCell;
use std::path::PathBuf;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSBezierPath, NSColor, NSDragOperation, NSDraggingContext, NSDraggingItem, NSDraggingSession,
    NSDraggingSource, NSEvent, NSEventModifierFlags, NSEventType, NSFont, NSFontAttributeName,
    NSForegroundColorAttributeName, NSImage, NSStringDrawing, NSView,
};
use objc2_foundation::{
    NSArray, NSAttributedStringKey, NSData, NSDictionary, NSPoint, NSProcessInfo, NSRect, NSSize, NSString,
};
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

define_class!(
    // SAFETY: NSObject has no subclassing requirements; no Drop impl.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "CrowDragSource"]
    struct DragSource;

    unsafe impl NSObjectProtocol for DragSource {}

    unsafe impl NSDraggingSource for DragSource {
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn operation_mask(&self, _session: &NSDraggingSession, context: NSDraggingContext) -> NSDragOperation {
            let allowed = NSDragOperation::Copy | NSDragOperation::Move | NSDragOperation::Generic;
            if context == NSDraggingContext::OutsideApplication { allowed | NSDragOperation::Link } else { allowed }
        }

        #[unsafe(method(draggingSession:endedAtPoint:operation:))]
        fn ended(&self, _session: &NSDraggingSession, _point: NSPoint, _operation: NSDragOperation) {
            SESSION.with(|slot| slot.borrow_mut().take());
            session_ended();
        }
    }
);

impl DragSource {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: plain NSObject init.
        unsafe { msg_send![super(this), init] }
    }
}

thread_local! {
    static SOURCE: Retained<DragSource> =
        DragSource::new(MainThreadMarker::new().expect("only touched after a main-thread check"));
}

const ICON: f64 = 22.0;
const GAP: f64 = 8.0;

pub(crate) fn ns_view(window: &impl HasWindowHandle) -> Option<Retained<NSView>> {
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

/// Owned copy of a [`ComboDrag`], kept so an interrupted session can be started again.
struct OwnedDrag {
    paths: Vec<(PathBuf, bool)>,
    icon_png: Vec<u8>,
    label: String,
    width: f32,
    height: f32,
    pill_width: f32,
    cursor_from_left: f32,
    cursor_from_top: f32,
}

impl OwnedDrag {
    fn new(drag: &ComboDrag) -> Self {
        Self {
            paths: drag.paths.to_vec(),
            icon_png: drag.icon_png.to_vec(),
            label: drag.label.to_string(),
            width: drag.width,
            height: drag.height,
            pill_width: drag.pill_width,
            cursor_from_left: drag.cursor_from_left,
            cursor_from_top: drag.cursor_from_top,
        }
    }

    fn as_combo(&self) -> ComboDrag<'_> {
        ComboDrag {
            paths: &self.paths,
            icon_png: &self.icon_png,
            label: &self.label,
            width: self.width,
            height: self.height,
            pill_width: self.pill_width,
            cursor_from_left: self.cursor_from_left,
            cursor_from_top: self.cursor_from_top,
        }
    }
}

/// The native session Crow started, while it runs, with what it needs to start it again.
struct Resume {
    drag: OwnedDrag,
    view: Retained<NSView>,
    restarts: u8,
}

thread_local! {
    static RESUME: RefCell<Option<Resume>> = const { RefCell::new(None) };
    static SESSION: RefCell<Option<Retained<NSDraggingSession>>> = const { RefCell::new(None) };
}

/// A session that is cancelled while the window lost key was cut short by a window switch,
/// not by the user, so its image must not fly back to where it started before it restarts.
fn stop_slide_back() {
    SESSION.with(|session| {
        if let Some(session) = session.borrow().as_ref() {
            session.setAnimatesToStartingPositionsOnCancelOrFail(false);
        }
    });
}

fn install_key_observers() {
    use block2::RcBlock;
    use objc2_app_kit::{NSApplicationDidResignActiveNotification, NSWindowDidResignKeyNotification};
    use objc2_foundation::{NSNotification, NSNotificationCenter};
    use std::ptr::NonNull;
    thread_local!(static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });
    if DONE.with(|done| done.replace(true)) {
        return;
    }
    let center = NSNotificationCenter::defaultCenter();
    // SAFETY: framework constants, valid for the whole process.
    let names = unsafe { [NSWindowDidResignKeyNotification, NSApplicationDidResignActiveNotification] };
    for name in names {
        let block = RcBlock::new(|_: NonNull<NSNotification>| stop_slide_back());
        // SAFETY: the block only touches this thread's session handle and lives for the process.
        let token = unsafe { center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block) };
        std::mem::forget(token);
        std::mem::forget(block);
    }
}

/// A window-switching utility such as AltTab focuses a window by posting a synthetic
/// click into the app. When that app owns the running drag session, AppKit reads the
/// mouse-up as the end of the drag, either dropping it on whatever is under the pointer
/// or cancelling it at a nonsense point, while the real button is still held.
const MAX_RESTARTS: u8 = 8;

unsafe extern "C" {
    fn CGEventSourceKeyState(state: i32, key: u16) -> bool;
}

/// Escape cancels a drag for real; it is the one cancel that must not restart it.
fn escape_down() -> bool {
    const HID_SYSTEM_STATE: i32 = 1;
    const ESCAPE: u16 = 53;
    // SAFETY: a plain query of the keyboard state.
    unsafe { CGEventSourceKeyState(HID_SYSTEM_STATE, ESCAPE) }
}

fn primary_button_down() -> bool {
    NSEvent::pressedMouseButtons() & 1 != 0
}

/// True when a drop arrives while the primary button is still physically down: the
/// mouse-up that caused it was synthetic, so the drop must not act. The session then
/// starts again at the pointer.
pub fn drop_is_premature() -> bool {
    primary_button_down()
}

/// Whether a session that just ended was cut short and should start again: the user is
/// still holding the button and did not press Escape.
fn should_resume(button_down: bool, escape_down: bool, restarts: u8) -> bool {
    button_down && !escape_down && restarts < MAX_RESTARTS
}

fn session_ended() {
    let Some(mut resume) = RESUME.with(|resume| resume.borrow_mut().take()) else { return };
    if !should_resume(primary_button_down(), escape_down(), resume.restarts) {
        return;
    }
    resume.restarts += 1;
    RESUME.with(|slot| *slot.borrow_mut() = Some(resume));
    // Starting a session from inside AppKit's own end-of-session callback panics, so wait
    // for the run loop.
    dispatch2::DispatchQueue::main().exec_async(resume_session);
}

fn resume_session() {
    if main_thread().is_none() {
        return;
    }
    let Some(resume) = RESUME.with(|resume| resume.borrow_mut().take()) else { return };
    if !primary_button_down() {
        return;
    }
    let Some(window) = resume.view.window() else { return };
    // A fresh drag event at the pointer: AppKit draws the image at the frame, then keeps it at
    // that offset from the event's location. With the old mouse-down as the event the image
    // showed over the dragged item for a frame or two before it followed the cursor.
    let location = window.convertPointFromScreen(NSEvent::mouseLocation());
    let Some(event) = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        NSEventType::LeftMouseDragged,
        location,
        NSEventModifierFlags::empty(),
        NSProcessInfo::processInfo().systemUptime(),
        window.windowNumber(),
        None,
        0,
        1,
        1.0,
    ) else {
        return;
    };
    let at = resume.view.convertPoint_fromView(location, None);
    if start_session(&resume.view, &resume.drag.as_combo(), at, &event) {
        RESUME.with(|slot| *slot.borrow_mut() = Some(resume));
    }
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
    if !start_session(&view, drag, at, &event) {
        return false;
    }
    RESUME.with(|resume| {
        *resume.borrow_mut() = Some(Resume { drag: OwnedDrag::new(drag), view: view.clone(), restarts: 0 });
    });
    true
}

fn start_session(view: &Retained<NSView>, drag: &ComboDrag, at: NSPoint, event: &NSEvent) -> bool {
    let Some(image) = combo_image(drag) else { return false };
    let empty: Retained<NSImage> = NSImage::initWithSize(NSImage::alloc(), NSSize::new(1., 1.));
    let height = drag.height as f64;
    // The GPUI view is flipped, so its y grows downwards.
    let flipped: bool = unsafe { msg_send![&**view, isFlipped] };
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
    // A dedicated source object (not the window) lets the window accept this drag
    // when it comes back: AppKit does not deliver destination callbacks for a drag
    // whose source is the window itself.
    install_key_observers();
    let session = SOURCE.with(|source| {
        view.beginDraggingSessionWithItems_event_source(&items, event, ProtocolObject::from_ref(&**source))
    });
    SESSION.with(|slot| *slot.borrow_mut() = Some(session));
    if !view.window().is_some_and(|window| window.isKeyWindow()) {
        stop_slide_back();
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_cut_short_with_the_button_held_resumes() {
        assert!(should_resume(true, false, 0));
        assert!(should_resume(true, false, MAX_RESTARTS - 1));
    }

    #[test]
    fn a_released_button_or_escape_ends_it_for_good() {
        assert!(!should_resume(false, false, 0), "the user let go: a normal end");
        assert!(!should_resume(true, true, 0), "Escape is a real cancel");
        assert!(!should_resume(false, true, 0));
    }

    #[test]
    fn restarts_are_bounded() {
        assert!(!should_resume(true, false, MAX_RESTARTS));
    }
}
