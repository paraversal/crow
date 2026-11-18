//! The only crate that talks to AppKit. Everything else in Crow is portable Rust.
//!
//! AppKit only works on the main thread. Called from anywhere else (tests, a
//! background task by mistake) every function here degrades instead of
//! crashing: no icon, plain path names, no panel, no drag.

mod cloud;
mod context_menu;
mod drag;
mod files;
mod fonts;
mod fsevents;
mod icons;
mod metadata;
mod names;
mod quick_look;
mod sound;
mod spotlight;
mod volumes;

pub use cloud::{SystemReadGate, status as cloud_status, status_batch as cloud_status_batch};
pub use context_menu::{ContextMenuEntry, MenuShortcut, show_context_menu};
pub use drag::{ComboDrag, begin_drag, remember_mouse_down};
pub use files::{SystemTrash, open_path, pasteboard_change_count, read_file_urls, write_file_urls};
pub use fonts::available_font_families;
pub use fsevents::{Event, EventFlags, FsEvents, current_event_id, volume_uuid, watch};
pub use icons::{icon_png, type_icon_png};
pub use metadata::{Permissions, kind_name, permissions, tags, thumbnail_png};
pub use names::display_components;
pub use quick_look::{QuickLookKey, set_quick_look_key_handler, toggle_quick_look, update_quick_look};
pub use sound::{Sound, beep, play, preload, set_volume, stop, ui_sounds_enabled};
pub use spotlight::{SpotlightHit, spotlight_names};
pub use volumes::{MountWatcher, Volume, eject, reduce_motion, volumes, watch_mounts};

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSWorkspace};
use objc2_foundation::{NSString, NSURL};
use std::path::Path;

fn main_thread() -> Option<MainThreadMarker> {
    let marker = MainThreadMarker::new();
    if marker.is_none() && cfg!(debug_assertions) && !cfg!(test) {
        // Visible in development, harmless in tests, which never run on the main thread.
        eprintln!("crow-macos called off the main thread; degrading");
    }
    marker
}

fn ns_string(path: &Path) -> Retained<NSString> {
    NSString::from_str(&path.to_string_lossy())
}

fn ns_url(path: &Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&ns_string(path))
}

/// Show `png` as the app's icon in the Dock (and the About panel and app switcher)
/// while Crow runs. The bundle's own icon, which Finder shows, is unchanged.
pub fn set_app_icon(png: &[u8]) {
    let Some(mtm) = main_thread() else { return };
    let data = objc2_foundation::NSData::with_bytes(png);
    let Some(image) =
        objc2_app_kit::NSImage::initWithData(<objc2_app_kit::NSImage as objc2::AnyThread>::alloc(), &data)
    else {
        return;
    };
    // SAFETY: the image is a valid NSImage; AppKit retains it.
    unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image)) };
}

/// The standard About panel, filled from the app bundle's Info.plist.
pub fn show_about_panel() {
    let Some(mtm) = main_thread() else { return };
    NSApplication::sharedApplication(mtm).orderFrontStandardAboutPanel(None);
}

/// Open `path` with the application the user has associated with it.
/// False when nothing opened it (no app for its type, or not on the main thread).
pub fn open_in_default_app(path: &Path) -> bool {
    main_thread().is_some() && NSWorkspace::sharedWorkspace().openURL(&ns_url(path))
}

/// Show `paths` selected in Finder windows (Finder's own "Show in Enclosing Folder").
pub fn reveal_in_finder(paths: &[std::path::PathBuf]) {
    if main_thread().is_none() || paths.is_empty() {
        return;
    }
    let urls: Vec<Retained<NSURL>> = paths.iter().map(|path| ns_url(path)).collect();
    NSWorkspace::sharedWorkspace()
        .activateFileViewerSelectingURLs(&objc2_foundation::NSArray::from_retained_slice(&urls));
}

/// Open the folder `dir` itself in a Finder window.
pub fn open_folder_in_finder(dir: &Path) -> bool {
    main_thread().is_some() && NSWorkspace::sharedWorkspace().selectFile_inFileViewerRootedAtPath(None, &ns_string(dir))
}
