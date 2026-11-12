use std::cell::RefCell;
use std::path::Path;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSInteger, NSURL};
use objc2_quick_look_ui::{QLPreviewItem, QLPreviewPanel, QLPreviewPanelDataSource};

use crate::{main_thread, ns_url};

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
            usize::try_from(index).ok().and_then(|i| urls.get(i).cloned()).map(ProtocolObject::from_retained)
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
    // The panel does not retain its data source, so it lives here for the whole run.
    static SOURCE: Retained<PreviewSource> = PreviewSource::new(MainThreadMarker::new().expect("only touched after a main-thread check"));
}

/// Toggle the system Quick Look panel for `path` (Space in Finder).
pub fn toggle_quick_look(path: &Path) {
    let Some(mtm) = main_thread() else { return };
    // SAFETY: main thread; the data source outlives the panel's use of it.
    unsafe {
        let Some(panel) = QLPreviewPanel::sharedPreviewPanel(mtm) else { return };
        if panel.isVisible() {
            panel.orderOut(None);
            return;
        }
        SOURCE.with(|source| {
            *source.ivars().borrow_mut() = vec![ns_url(path)];
            panel.setDataSource(Some(ProtocolObject::from_ref(&**source)));
        });
        panel.reloadData();
        panel.makeKeyAndOrderFront(None);
    }
}

/// Point an open Quick Look panel at a new selection; does nothing when it is closed.
pub fn update_quick_look(path: &Path) {
    let Some(mtm) = main_thread() else { return };
    // SAFETY: main thread.
    unsafe {
        if !QLPreviewPanel::sharedPreviewPanelExists(mtm) {
            return;
        }
        let Some(panel) = QLPreviewPanel::sharedPreviewPanel(mtm) else { return };
        if panel.isVisible() {
            SOURCE.with(|source| *source.ivars().borrow_mut() = vec![ns_url(path)]);
            panel.reloadData();
        }
    }
}
