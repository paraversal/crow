use std::path::PathBuf;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSDragOperation, NSDraggingContext, NSDraggingItem, NSDraggingSession, NSDraggingSource, NSView,
    NSWorkspace,
};
use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::{main_thread, ns_string, ns_url};

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
            if context == NSDraggingContext::OutsideApplication {
                // Like Finder: the destination decides between copy and move.
                NSDragOperation::Copy | NSDragOperation::Move | NSDragOperation::Link | NSDragOperation::Generic
            } else {
                NSDragOperation::None
            }
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
    static SOURCE: Retained<DragSource> = DragSource::new(MainThreadMarker::new().expect("only touched after a main-thread check"));
}

fn ns_view(window: &impl HasWindowHandle) -> Option<Retained<NSView>> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else { return None };
    // SAFETY: the window owns this live NSView; retaining it only extends its lifetime.
    unsafe { Retained::retain(appkit.ns_view.as_ptr().cast::<NSView>()) }
}

/// Start a system drag of `paths` from the mouse-dragged event being handled.
/// Returns false when there is no current event or native view to start from.
pub fn begin_drag(window: &impl HasWindowHandle, paths: &[PathBuf]) -> bool {
    let Some(mtm) = main_thread() else { return false };
    let Some(view) = ns_view(window) else { return false };
    let Some(event) = NSApplication::sharedApplication(mtm).currentEvent() else { return false };
    let at = view.convertPoint_fromView(event.locationInWindow(), None);
    let items: Vec<Retained<NSDraggingItem>> = paths
        .iter()
        .enumerate()
        .map(|(i, path)| {
            let url = ns_url(path);
            let item =
                NSDraggingItem::initWithPasteboardWriter(NSDraggingItem::alloc(), ProtocolObject::from_ref(&*url));
            let icon = NSWorkspace::sharedWorkspace().iconForFile(&ns_string(path));
            let offset = 4.0 * i as f64;
            let frame = NSRect::new(NSPoint::new(at.x - 16.0 + offset, at.y - 16.0), NSSize::new(32.0, 32.0));
            // SAFETY: an NSImage is valid dragging-image content.
            unsafe { item.setDraggingFrame_contents(frame, Some(&icon)) };
            item
        })
        .collect();
    let items = NSArray::from_retained_slice(&items);
    SOURCE.with(|source| {
        view.beginDraggingSessionWithItems_event_source(&items, &event, ProtocolObject::from_ref(&**source));
    });
    true
}
