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
mod input_diag;
mod metadata;
mod names;
mod quick_look;
mod sound;
mod spotlight;
mod system_colors;
mod titlebar_drag;
mod volumes;

pub use cloud::{SystemReadGate, status as cloud_status, status_batch as cloud_status_batch};
pub use context_menu::{
    ContextMenuEntry, MenuShortcut, SUBMENU_BASE, SUBMENU_STRIDE, show_context_menu, submenu_choice,
};
pub use drag::{ComboDrag, begin_drag, drop_is_premature, remember_mouse_down};
pub use files::{SystemTrash, open_path, pasteboard_change_count, read_file_urls, write_file_urls};
pub use fonts::available_font_families;
pub use fsevents::{Event, EventFlags, FsEvents, current_event_id, volume_uuid, watch, watch_under};
pub use icons::{icon_png, symbol_png, type_icon_png};
pub use input_diag::{InputSnapshot, input_snapshot};
pub use metadata::{Permissions, kind_name, permissions, tags, tags_checked, thumbnail_png, type_identifiers};
pub use names::{display_component_paths, display_components, display_name_at, localized_display_name_at};
pub use quick_look::{QuickLookKey, set_quick_look_key_handler, toggle_quick_look, update_quick_look};
pub use sound::{Sound, beep, play, preload, set_volume, stop, ui_sounds_enabled};
pub use spotlight::{SpotlightHit, SpotlightPredicate, spotlight_expression, spotlight_names};
pub use system_colors::{SystemColorWatcher, accent_color, unfocused_selection_color, watch_system_colors};
pub use titlebar_drag::TitlebarDrag;
pub use volumes::{MountWatcher, Volume, eject, reduce_motion, reduce_transparency, volumes, watch_mounts};

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSBitmapImageRep, NSImage, NSWorkspace};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString, NSURL};
use std::path::Path;

fn main_thread() -> Option<MainThreadMarker> {
    let marker = MainThreadMarker::new();
    if marker.is_none() && cfg!(debug_assertions) && !cfg!(test) {
        // Visible in development, harmless in tests, which never run on the main thread.
        eprintln!("crow-macos called off the main thread; degrading");
    }
    marker
}

/// The path as an `NSString`. Foundation cannot hold bytes that are not UTF-8 in a
/// string, so those become U+FFFD; use `ns_url` where the item itself is addressed.
fn ns_string(path: &Path) -> Retained<NSString> {
    NSString::from_str(&path.to_string_lossy())
}

/// A file URL built from the file system representation, so a name that is not
/// UTF-8 keeps its bytes (percent-encoded).
fn ns_url(path: &Path) -> Retained<NSURL> {
    use std::os::unix::ffi::OsStrExt;
    match std::ffi::CString::new(path.as_os_str().as_bytes()) {
        Ok(bytes) if !bytes.as_bytes().is_empty() => {
            // SAFETY: a valid NUL-terminated buffer; AppKit copies it.
            unsafe {
                NSURL::fileURLWithFileSystemRepresentation_isDirectory_relativeToURL(
                    std::ptr::NonNull::new_unchecked(bytes.as_ptr().cast_mut()),
                    path.is_dir(),
                    None,
                )
            }
        }
        // An empty path would mean the working directory, an interior NUL a shorter
        // path. Callers that act on the file check `valid_path` first.
        _ => NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy())),
    }
}

/// Whether `path` can be handed to AppKit as it is: not empty, no NUL byte.
pub(crate) fn valid_path(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    !path.as_os_str().is_empty() && !path.as_os_str().as_bytes().contains(&0)
}

/// The Dock draws every icon inside a template: artwork about 80% of the canvas, centered,
/// with a transparent margin. Art that fills its canvas looks a quarter too big next to
/// its neighbours, so `set_app_icon` adds that margin unless the picture already has one.
const ICON_ARTWORK: f64 = 824. / 1024.;

/// Whether an icon needs the Dock margin added, from the alpha at the middle of its four
/// edges (sampled 2% inside): any visible pixel there means the artwork reaches the edge.
fn icon_needs_margin(edge_alphas: [f64; 4]) -> bool {
    edge_alphas.iter().any(|alpha| *alpha > 0.1)
}

/// Show `png` as the app's icon in the Dock (and the About panel and app switcher)
/// while Crow runs, drawn at the Dock's template size. The bundle's own icon, which
/// Finder shows, is made by `scripts/bundle.sh` the same way.
pub fn set_app_icon(png: &[u8]) {
    let Some(mtm) = main_thread() else { return };
    let data = objc2_foundation::NSData::with_bytes(png);
    let Some(image) = NSImage::initWithData(<NSImage as objc2::AnyThread>::alloc(), &data) else {
        return;
    };
    let image = with_dock_margin(&data, image);
    // SAFETY: the image is a valid NSImage; AppKit retains it.
    unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image)) };
}

/// `image` redrawn at `ICON_ARTWORK` of a square canvas, or `image` itself when its
/// artwork already leaves a margin (or cannot be read).
// `lockFocus` is deprecated for resolution-independent drawing, but this canvas is drawn
// once at 1024 px and handed to the Dock, which scales it; a drawing-handler image would
// redraw on every Dock repaint.
#[allow(deprecated)]
fn with_dock_margin(data: &objc2_foundation::NSData, image: Retained<NSImage>) -> Retained<NSImage> {
    let Some(rep) = NSBitmapImageRep::imageRepWithData(data) else { return image };
    let (w, h) = (rep.pixelsWide(), rep.pixelsHigh());
    if w < 2 || h < 2 {
        return image;
    }
    let alpha = |x: isize, y: isize| rep.colorAtX_y(x, y).map_or(0., |c| c.alphaComponent());
    // Sampled a little inside the edge: a rounded, anti-aliased icon is nearly transparent
    // exactly at the edge yet still fills the canvas.
    let (dx, dy) = ((w / 50).max(1), (h / 50).max(1));
    if !icon_needs_margin([alpha(w / 2, dy), alpha(w / 2, h - 1 - dy), alpha(dx, h / 2), alpha(w - 1 - dx, h / 2)]) {
        return image;
    }
    let side = 1024.;
    let art = side * ICON_ARTWORK;
    let inset = (side - art) / 2.;
    let canvas = NSImage::initWithSize(<NSImage as objc2::AnyThread>::alloc(), NSSize::new(side, side));
    canvas.lockFocus();
    image.drawInRect_fromRect_operation_fraction(
        NSRect::new(NSPoint::new(inset, inset), NSSize::new(art, art)),
        NSRect::ZERO,
        objc2_app_kit::NSCompositingOperation::SourceOver,
        1.0,
    );
    canvas.unlockFocus();
    canvas
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

/// Ask Finder to empty the Trash through an Apple event. Finder holds the
/// access Crow lacks without Full Disk Access. Blocks until Finder answers, so
/// call it off the main thread. The first use asks the user to allow Crow to
/// control Finder.
pub fn finder_empty_trash() -> Result<(), String> {
    let output = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", "tell application \"Finder\" to empty the trash"])
        .output()
        .map_err(|e| format!("Finder: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(if detail.is_empty() { "Finder could not empty the Trash".to_owned() } else { detail })
}

/// Open the folder `dir` itself in a Finder window.
pub fn open_folder_in_finder(dir: &Path) -> bool {
    main_thread().is_some() && NSWorkspace::sharedWorkspace().selectFile_inFileViewerRootedAtPath(None, &ns_string(dir))
}

#[cfg(test)]
mod icon_margin_tests {
    use super::icon_needs_margin;

    #[test]
    fn art_that_reaches_an_edge_gets_the_margin_and_art_with_one_does_not() {
        assert!(icon_needs_margin([1., 1., 1., 1.]), "full-bleed art");
        assert!(icon_needs_margin([0., 0., 0., 1.]), "one edge touched");
        assert!(!icon_needs_margin([0., 0., 0., 0.]), "a transparent border is already there");
        assert!(!icon_needs_margin([0.05, 0., 0.08, 0.]), "antialiasing at the border is not art");
    }
}

#[cfg(test)]
mod path_tests {
    use super::{ns_url, valid_path};
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    #[test]
    fn non_utf8_names_keep_their_bytes_in_file_urls() {
        let a = ns_url(Path::new(OsStr::from_bytes(b"/tmp/a\xfe")));
        let b = ns_url(Path::new(OsStr::from_bytes(b"/tmp/a\xff")));
        let (a, b) = (a.absoluteString().unwrap().to_string(), b.absoluteString().unwrap().to_string());
        assert_ne!(a, b, "lossy conversion would make both U+FFFD");
        assert_eq!(ns_url(Path::new("/tmp/plain")).path().unwrap().to_string(), "/tmp/plain");
    }

    #[test]
    fn empty_and_nul_paths_are_not_usable() {
        assert!(!valid_path(Path::new("")));
        assert!(!valid_path(Path::new(OsStr::from_bytes(b"/tmp/a\0b"))));
        assert!(valid_path(Path::new("/tmp/a")));
        let refused = crow_fs::ops::Trash::trash(&crate::files::SystemTrash, Path::new("")).unwrap_err();
        assert_eq!(refused.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn main_thread_only_calls_degrade_off_the_main_thread() {
        // Tests never run on the main thread.
        assert!(!crate::drag::drop_is_premature());
    }
}
