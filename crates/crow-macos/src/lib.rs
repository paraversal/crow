//! The only crate that talks to AppKit. Everything else in Crow is portable Rust.
//!
//! AppKit only works on the main thread. Called from anywhere else (tests, a
//! background task by mistake) every function here degrades instead of
//! crashing: no icon, plain path names, no panel, no drag.

mod drag;
mod icons;
mod metadata;
mod names;
mod quick_look;

pub use drag::begin_drag;
pub use icons::icon_png;
pub use metadata::{kind_name, tags, thumbnail_png};
pub use names::display_components;
pub use quick_look::{toggle_quick_look, update_quick_look};

use objc2::MainThreadMarker;
use objc2::rc::Retained;
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
