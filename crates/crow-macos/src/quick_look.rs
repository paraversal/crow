use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSEvent, NSEventType, NSWindowDelegate};
use objc2_foundation::{NSInteger, NSOperationQueue, NSURL};
use objc2_quick_look_ui::{QLPreviewItem, QLPreviewPanel, QLPreviewPanelDataSource, QLPreviewPanelDelegate};

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

/// Arrow keys pressed while the Quick Look panel is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuickLookKey {
    Up,
    Down,
    Left,
    Right,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; no Drop impl.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "CrowPreviewDelegate"]
    struct PreviewDelegate;

    unsafe impl NSObjectProtocol for PreviewDelegate {}

    unsafe impl NSWindowDelegate for PreviewDelegate {}

    unsafe impl QLPreviewPanelDelegate for PreviewDelegate {
        // Quick Look takes the keyboard while it is open and passes the events it
        // does not use (the arrows) on to its delegate, which is where Finder moves
        // its selection from too. Space is Quick Look's own close key.
        #[unsafe(method(previewPanel:handleEvent:))]
        fn handle_event(&self, _panel: Option<&QLPreviewPanel>, event: Option<&NSEvent>) -> Bool {
            let Some(event) = event else { return false.into() };
            if event.r#type() != NSEventType::KeyDown {
                return false.into();
            }
            let key = match event.keyCode() {
                126 => QuickLookKey::Up,
                125 => QuickLookKey::Down,
                123 => QuickLookKey::Left,
                124 => QuickLookKey::Right,
                _ => return false.into(),
            };
            KEY_HANDLER
                .with(|slot| {
                    let handler = slot.borrow();
                    handler.as_ref().map(|handler| {
                        handler(key);
                        true
                    })
                })
                .unwrap_or(false)
                .into()
        }
    }
);

impl PreviewDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: plain NSObject init.
        unsafe { msg_send![super(this), init] }
    }
}

thread_local! {
    // The panel does not retain its data source or delegate, so they live here for the whole run.
    static SOURCE: Retained<PreviewSource> = PreviewSource::new(MainThreadMarker::new().expect("only touched after a main-thread check"));
    static DELEGATE: Retained<PreviewDelegate> = PreviewDelegate::new(MainThreadMarker::new().expect("only touched after a main-thread check"));
    // What the app does with an arrow key that the panel passes on.
    static KEY_HANDLER: RefCell<Option<KeyHandler>> = const { RefCell::new(None) };
    // Latest selection waiting for the next main-queue turn, and whether a turn is queued.
    static PENDING: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static PENDING_SCHEDULED: Cell<bool> = const { Cell::new(false) };
}

type KeyHandler = Box<dyn Fn(QuickLookKey)>;

/// Set what happens when an arrow key reaches an open Quick Look panel.
pub fn set_quick_look_key_handler(handler: impl Fn(QuickLookKey) + 'static) {
    if main_thread().is_none() {
        return;
    }
    KEY_HANDLER.with(|slot| *slot.borrow_mut() = Some(Box::new(handler)));
}

/// Run `f` from a fresh main-queue turn.
///
/// Quick Look loads its preview plugins on a nested run loop. Calling it straight
/// from a GPUI event handler runs that loop while GPUI still holds its `App`
/// borrow, so GPUI's own main-queue timers fire into a borrowed `App` and panic.
/// A main-queue operation starts after the handler (and the borrow) has returned.
pub(crate) fn on_main_queue(f: impl Fn() + 'static) {
    let block = RcBlock::new(f);
    // SAFETY: NSObject operations on the main queue run on the main thread.
    unsafe { NSOperationQueue::mainQueue().addOperationWithBlock(&block) };
}

fn set_item(panel: &QLPreviewPanel, path: &Path) {
    SOURCE.with(|source| {
        *source.ivars().borrow_mut() = vec![ns_url(path)];
        // SAFETY: main thread, and the data source is retained for the whole run.
        unsafe { panel.setDataSource(Some(ProtocolObject::from_ref(&**source))) };
    });
}

/// Toggle the system Quick Look panel for `path` (Space in Finder).
pub fn toggle_quick_look(path: &Path) {
    if main_thread().is_none() {
        return;
    }
    let path = path.to_path_buf();
    on_main_queue(move || {
        let Some(mtm) = main_thread() else { return };
        // SAFETY: main thread; the data source and delegate outlive the panel's use of them.
        unsafe {
            let Some(panel) = QLPreviewPanel::sharedPreviewPanel(mtm) else { return };
            if panel.isVisible() {
                panel.orderOut(None);
                log_panel("closed");
                return;
            }
            set_item(&panel, &path);
            DELEGATE.with(|delegate| {
                panel.setDelegate(Some(ProtocolObject::<dyn QLPreviewPanelDelegate>::from_ref(&**delegate).as_ref()))
            });
            panel.reloadData();
            panel.makeKeyAndOrderFront(None);
            log_panel("opened");
        }
    });
}

/// The panel takes the key window while it is open, so a shortcut pressed then goes to it
/// rather than to Crow. A trace of when that starts and ends helps with "shortcuts stopped".
fn log_panel(what: &str) {
    let snapshot = crate::input_snapshot().map_or_else(|| "unavailable".into(), |snapshot| snapshot.to_string());
    tracing::info!(target: "input", "quick look {what}: macos: {snapshot}");
}

/// Point an open Quick Look panel at a new selection; does nothing when it is closed.
pub fn update_quick_look(path: &Path) {
    if main_thread().is_none() {
        return;
    }
    PENDING.with(|pending| *pending.borrow_mut() = Some(path.to_path_buf()));
    if PENDING_SCHEDULED.replace(true) {
        return;
    }
    on_main_queue(|| {
        PENDING_SCHEDULED.set(false);
        let Some(path) = PENDING.with(|pending| pending.borrow_mut().take()) else { return };
        let Some(mtm) = main_thread() else { return };
        // SAFETY: main thread.
        unsafe {
            if !QLPreviewPanel::sharedPreviewPanelExists(mtm) {
                return;
            }
            let Some(panel) = QLPreviewPanel::sharedPreviewPanel(mtm) else { return };
            if panel.isVisible() {
                set_item(&panel, &path);
                panel.reloadData();
            }
        }
    });
}
