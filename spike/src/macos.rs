//! The only module that talks to AppKit: native icons, Finder display names,
//! the Quick Look panel and dragging files out to other apps.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{Image, ImageFormat, Window};
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSBitmapImageFileType, NSBitmapImageRep, NSDragOperation, NSDraggingContext, NSDraggingItem,
    NSDraggingSession, NSDraggingSource, NSImage, NSView, NSWorkspace,
};
use objc2_foundation::{NSArray, NSDictionary, NSFileManager, NSInteger, NSPoint, NSRect, NSSize, NSString, NSURL};
use objc2_quick_look_ui::{QLPreviewItem, QLPreviewPanel, QLPreviewPanelDataSource};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("AppKit calls must run on the main thread")
}

fn ns_url(path: &Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}

fn ns_view(window: &Window) -> Option<Retained<NSView>> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return None };
    // SAFETY: GPUI hands out its live NSView; we only retain it.
    unsafe { Retained::retain(h.ns_view.as_ptr().cast::<NSView>()) }
}

// ---------- Icons ----------

thread_local! {
    static ICONS: RefCell<HashMap<(String, u32), Arc<Image>>> = RefCell::new(HashMap::new());
}

/// Cache key: icons depend on type, except for bundles and special folders,
/// which can carry their own icon and are keyed by path.
fn icon_key(path: &Path, is_dir: bool) -> String {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if is_dir {
        let home = std::env::var("HOME").unwrap_or_default();
        let special = path.parent().is_some_and(|p| p == Path::new(&home))
            || path == Path::new("/")
            || path.starts_with("/Volumes");
        if !ext.is_empty() || special { format!("path:{}", path.display()) } else { "dir".into() }
    } else {
        format!("ext:{ext}")
    }
}

/// The Finder icon for a file, rendered to PNG at `px` points (2x for Retina).
pub fn icon(path: &Path, is_dir: bool, px: u32) -> Option<Arc<Image>> {
    let key = (icon_key(path, is_dir), px);
    if let Some(hit) = ICONS.with(|c| c.borrow().get(&key).cloned()) {
        return Some(hit);
    }
    let image = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(&path.to_string_lossy()));
    let png = png_of(&image, px * 2)?;
    let arc = Arc::new(Image::from_bytes(ImageFormat::Png, png));
    ICONS.with(|c| c.borrow_mut().insert(key, arc.clone()));
    Some(arc)
}

fn png_of(image: &NSImage, px: u32) -> Option<Vec<u8>> {
    let mut rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(px as f64, px as f64));
    // SAFETY: rect is a valid pointer; no context or hints.
    let cg = unsafe { image.CGImageForProposedRect_context_hints(&mut rect, None, None) }?;
    let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg);
    let props = NSDictionary::new();
    // SAFETY: empty property dictionary is valid for PNG.
    let data = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &props) }?;
    Some(data.to_vec())
}

// ---------- Finder names ----------

/// Finder's path components: `Macintosh HD › Users › jules › …`, localized
/// the same way Finder shows them. Paths on other volumes start at the volume.
pub fn display_components(path: &Path) -> Vec<(String, PathBuf)> {
    let fm = NSFileManager::defaultManager();
    let ancestors: Vec<&Path> = path.ancestors().collect();
    let mut out: Vec<(String, PathBuf)> = ancestors
        .into_iter()
        .rev()
        .filter(|p| *p != Path::new("/Volumes"))
        .filter(|p| !(path.starts_with("/Volumes") && *p == Path::new("/")))
        .map(|p| {
            let name = fm.displayNameAtPath(&NSString::from_str(&p.to_string_lossy())).to_string();
            (name, p.to_path_buf())
        })
        .collect();
    out.dedup_by(|a, b| a.1 == b.1);
    out
}

// ---------- Quick Look ----------

define_class!(
    // SAFETY: NSObject has no subclassing requirements; no Drop impl.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "CrowPreviewSource"]
    #[ivars = RefCell<Vec<Retained<NSURL>>>]
    struct PreviewSource;

    unsafe impl NSObjectProtocol for PreviewSource {}

    unsafe impl QLPreviewPanelDataSource for PreviewSource {
        #[unsafe(method(numberOfPreviewItemsInPreviewPanel:))]
        fn number_of_items(&self, _panel: Option<&QLPreviewPanel>) -> NSInteger {
            self.ivars().borrow().len() as NSInteger
        }

        #[unsafe(method_id(previewPanel:previewItemAtIndex:))]
        fn item_at(&self, _panel: Option<&QLPreviewPanel>, index: NSInteger) -> Option<Retained<ProtocolObject<dyn QLPreviewItem>>> {
            let urls = self.ivars().borrow();
            urls.get(index as usize).cloned().map(ProtocolObject::from_retained)
        }
    }
);

impl PreviewSource {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(RefCell::new(Vec::new()));
        // SAFETY: plain NSObject init.
        unsafe { msg_send![super(this), init] }
    }
}

thread_local! {
    // The panel does not retain its data source, so we keep it alive here.
    static PREVIEW_SOURCE: Retained<PreviewSource> = PreviewSource::new(mtm());
}

/// Toggle the system Quick Look panel for `path` (Space in Finder).
pub fn toggle_quick_look(path: &Path) {
    let mtm = mtm();
    // SAFETY: main thread; the data source outlives the panel's use of it.
    unsafe {
        let Some(panel) = QLPreviewPanel::sharedPreviewPanel(mtm) else { return };
        if panel.isVisible() {
            panel.orderOut(None);
            return;
        }
        PREVIEW_SOURCE.with(|src| {
            *src.ivars().borrow_mut() = vec![ns_url(path)];
            panel.setDataSource(Some(ProtocolObject::from_ref(&**src)));
        });
        panel.reloadData();
        panel.makeKeyAndOrderFront(None);
    }
}

/// Point an open Quick Look panel at a new selection (arrow keys in Finder).
pub fn update_quick_look(path: &Path) {
    let mtm = mtm();
    // SAFETY: main thread.
    unsafe {
        if !QLPreviewPanel::sharedPreviewPanelExists(mtm) {
            return;
        }
        let Some(panel) = QLPreviewPanel::sharedPreviewPanel(mtm) else { return };
        if panel.isVisible() {
            PREVIEW_SOURCE.with(|src| *src.ivars().borrow_mut() = vec![ns_url(path)]);
            panel.reloadData();
        }
    }
}

// ---------- Drag out ----------

define_class!(
    // SAFETY: NSObject has no subclassing requirements; no Drop impl.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "CrowDragSource"]
    struct DragSource;

    unsafe impl NSObjectProtocol for DragSource {}

    unsafe impl NSDraggingSource for DragSource {
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn op_mask(&self, _session: &NSDraggingSession, context: NSDraggingContext) -> NSDragOperation {
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
    static DRAG_SOURCE: Retained<DragSource> = DragSource::new(mtm());
}

/// Start a system drag of `paths` from the current mouse-dragged event.
/// Returns false if there is no event or view to start from.
pub fn begin_drag(window: &Window, paths: &[PathBuf]) -> bool {
    let mtm = mtm();
    let Some(view) = ns_view(window) else { return false };
    let Some(event) = NSApplication::sharedApplication(mtm).currentEvent() else { return false };
    let at = view.convertPoint_fromView(event.locationInWindow(), None);
    let items: Vec<Retained<NSDraggingItem>> = paths
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let url = ns_url(p);
            let item =
                NSDraggingItem::initWithPasteboardWriter(NSDraggingItem::alloc(), ProtocolObject::from_ref(&*url));
            let icon = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(&p.to_string_lossy()));
            let frame = NSRect::new(NSPoint::new(at.x - 16.0 + 4.0 * i as f64, at.y - 16.0), NSSize::new(32.0, 32.0));
            // SAFETY: NSImage is a valid dragging image.
            unsafe { item.setDraggingFrame_contents(frame, Some(&icon)) };
            item
        })
        .collect();
    let array = NSArray::from_retained_slice(&items);
    DRAG_SOURCE.with(|src| {
        view.beginDraggingSessionWithItems_event_source(&array, &event, ProtocolObject::from_ref(&**src));
    });
    true
}
