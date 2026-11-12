//! File metadata from the system: kind names, Finder tags, Quick Look thumbnails.
//! Unlike the rest of this crate these are safe off the main thread (and the
//! thumbnail call must run off it, because it waits for Quick Look).

use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use block2::RcBlock;
use objc2::AnyThread;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
use objc2_foundation::{NSArray, NSDictionary, NSError, NSSize, NSString, NSURLTagNamesKey};
use objc2_quick_look_thumbnailing::{
    QLThumbnailGenerationRequest, QLThumbnailGenerationRequestRepresentationTypes, QLThumbnailGenerator,
    QLThumbnailRepresentation,
};
use objc2_uniform_type_identifiers::UTType;

use crate::ns_url;

/// Kind names by (lowercased extension, is folder).
type KindCache = Mutex<HashMap<(String, bool), Arc<str>>>;

static KINDS: LazyLock<KindCache> = LazyLock::new(Default::default);

/// Finder's "Kind" for a file extension (`"Markdown Document"`, `"Folder"`, …),
/// localized by the system and cached per extension.
pub fn kind_name(ext: &str, is_dir: bool) -> Arc<str> {
    let key = (ext.to_lowercase(), is_dir);
    if let Some(hit) = KINDS.lock().ok().and_then(|k| k.get(&key).cloned()) {
        return hit;
    }
    let name: Arc<str> = autoreleasepool(|_| {
        let described = if is_dir && key.0.is_empty() {
            None
        } else {
            UTType::typeWithFilenameExtension(&NSString::from_str(&key.0))
                .filter(|t| !t.isDynamic())
                .and_then(|t| t.localizedDescription())
                .map(|d| d.to_string())
        };
        Arc::from(described.unwrap_or_else(|| fallback_kind(&key.0, is_dir)))
    });
    if let Ok(mut kinds) = KINDS.lock() {
        kinds.insert(key, name.clone());
    }
    name
}

fn fallback_kind(ext: &str, is_dir: bool) -> String {
    match (is_dir, ext) {
        (true, _) => "Folder".into(),
        (false, "") => "Document".into(),
        (false, ext) => format!("{} file", ext.to_uppercase()),
    }
}

/// The item's Finder tags, in Finder's order. Empty when it has none or cannot be read.
pub fn tags(path: &Path) -> Vec<String> {
    autoreleasepool(|_| {
        let url = ns_url(path);
        let mut value: Option<Retained<AnyObject>> = None;
        // SAFETY: NSURLTagNamesKey yields an NSArray<NSString> (or nil); checked by downcast below.
        if unsafe { url.getResourceValue_forKey_error(&mut value, NSURLTagNamesKey) }.is_err() {
            return Vec::new();
        }
        value
            .and_then(|v| v.downcast::<NSArray>().ok())
            .map(|array| {
                array.iter().filter_map(|item| item.downcast::<NSString>().ok()).map(|s| s.to_string()).collect()
            })
            .unwrap_or_default()
    })
}

/// A Quick Look thumbnail for `path` as PNG bytes, at most `points` square (rendered
/// at 2x). Waits up to two seconds; `None` if Quick Look has nothing or takes longer.
/// Returns `None` on the main thread rather than blocking it.
pub fn thumbnail_png(path: &Path, points: f64) -> Option<Vec<u8>> {
    if objc2::MainThreadMarker::new().is_some() {
        return None;
    }
    let (tx, rx) = mpsc::channel::<Option<Vec<u8>>>();
    autoreleasepool(|_| {
        let url = ns_url(path);
        // SAFETY: valid URL and size; the request is retained for the call.
        let request = unsafe {
            QLThumbnailGenerationRequest::initWithFileAtURL_size_scale_representationTypes(
                QLThumbnailGenerationRequest::alloc(),
                &url,
                NSSize::new(points, points),
                2.0,
                QLThumbnailGenerationRequestRepresentationTypes::Thumbnail,
            )
        };
        let handler = RcBlock::new(move |representation: *mut QLThumbnailRepresentation, _error: *mut NSError| {
            // SAFETY: Quick Look passes a valid representation or null.
            let png = unsafe { representation.as_ref() }.and_then(|rep| {
                // SAFETY: a representation always carries a CGImage on macOS.
                let image = unsafe { rep.CGImage() };
                let bitmap = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &image);
                // SAFETY: an empty property dictionary is valid for PNG.
                unsafe { bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }
                    .map(|data| data.to_vec())
            });
            tx.send(png).ok();
        });
        // SAFETY: the shared generator is thread-safe; the block is retained by Quick Look.
        unsafe {
            QLThumbnailGenerator::sharedGenerator()
                .generateBestRepresentationForRequest_completionHandler(&request, &handler)
        };
    });
    rx.recv_timeout(Duration::from_secs(2)).ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_come_from_the_system() {
        assert_eq!(&*kind_name("", true), "Folder");
        assert_ne!(&*kind_name("md", false), "MD file", "the system knows Markdown");
        assert_eq!(&*kind_name("zzqq", false), "ZZQQ file", "unknown extensions get a readable fallback");
        assert!(Arc::ptr_eq(&kind_name("md", false), &kind_name("MD", false)), "cached case-insensitively");
    }

    #[test]
    fn untagged_file_has_no_tags() {
        let file = std::env::temp_dir().join(format!("crow-tags-{}", std::process::id()));
        std::fs::write(&file, "x").unwrap();
        assert!(tags(&file).is_empty());
        std::fs::remove_file(&file).ok();
    }
}
