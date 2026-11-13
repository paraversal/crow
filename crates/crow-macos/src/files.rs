//! The system Trash and the file pasteboard, so Crow works together with Finder:
//! items Crow trashes can be put back in Finder, and Cmd-C / Cmd-V work between both.

use std::io;
use std::path::{Path, PathBuf};

use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeFileURL, NSPasteboardWriting};
use objc2_foundation::{NSArray, NSFileManager, NSURL};

use crate::{main_thread, ns_url};

/// The macOS Trash, through `NSFileManager`, so Finder's Put Back works.
pub struct SystemTrash;

impl crow_fs::ops::Trash for SystemTrash {
    fn trash(&self, path: &Path) -> io::Result<PathBuf> {
        autoreleasepool(|_| {
            let mut resulting: Option<Retained<NSURL>> = None;
            NSFileManager::defaultManager()
                .trashItemAtURL_resultingItemURL_error(&ns_url(path), Some(&mut resulting))
                .map_err(|e| io::Error::other(e.localizedDescription().to_string()))?;
            resulting
                .and_then(|url| url.path())
                .map(|p| PathBuf::from(p.to_string()))
                .ok_or_else(|| io::Error::other("the Trash did not say where the item went"))
        })
    }
}

/// Open `path` with its default app, or with the default text editor (`open -t`).
/// Waits for `open` to say whether it worked.
pub fn open_path(path: &Path, as_text: bool) -> io::Result<()> {
    let mut command = std::process::Command::new("/usr/bin/open");
    if as_text {
        command.arg("-t");
    }
    let status = command.arg(path).status()?;
    if status.success() { Ok(()) } else { Err(io::Error::other(format!("open exited with {status}"))) }
}

/// Put `paths` on the general pasteboard as file URLs (what Finder's Copy does).
/// Returns the pasteboard's change count afterwards, to recognize our own contents later.
pub fn write_file_urls(paths: &[PathBuf]) -> Option<isize> {
    main_thread()?;
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    let urls: Vec<Retained<NSURL>> = paths.iter().map(|p| ns_url(p)).collect();
    let writers: Vec<&ProtocolObject<dyn NSPasteboardWriting>> =
        urls.iter().map(|u| ProtocolObject::from_ref(&**u)).collect();
    pasteboard.writeObjects(&NSArray::from_slice(&writers));
    Some(pasteboard.changeCount())
}

/// File paths on the general pasteboard (from Crow, Finder or any other app).
pub fn read_file_urls() -> Vec<PathBuf> {
    if main_thread().is_none() {
        return Vec::new();
    }
    let pasteboard = NSPasteboard::generalPasteboard();
    let Some(items) = pasteboard.pasteboardItems() else { return Vec::new() };
    items
        .iter()
        // SAFETY: NSPasteboardTypeFileURL is a valid, static pasteboard type.
        .filter_map(|item| item.stringForType(unsafe { NSPasteboardTypeFileURL }))
        .filter_map(|s| NSURL::URLWithString(&s))
        .filter_map(|url| url.path())
        .map(|p| PathBuf::from(p.to_string()))
        .collect()
}

/// The pasteboard's change count: it changes whenever anything is copied anywhere.
pub fn pasteboard_change_count() -> Option<isize> {
    main_thread()?;
    Some(NSPasteboard::generalPasteboard().changeCount())
}
