//! The only crate that talks to AppKit. Everything else in Crow is portable Rust.
//!
//! AppKit only works on the main thread. Called from anywhere else (tests, a
//! background task by mistake) every function here degrades instead of
//! crashing: no icon, plain path names, no panel, no drag.

mod drag;
mod files;
mod icons;
mod metadata;
mod names;
mod quick_look;
mod sound;
mod spotlight;
mod volumes;

pub use drag::{ComboDrag, begin_drag, remember_mouse_down};
pub use files::{SystemTrash, open_path, pasteboard_change_count, read_file_urls, write_file_urls};
pub use icons::icon_png;
pub use metadata::{kind_name, tags, thumbnail_png};
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
