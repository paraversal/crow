use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use objc2::AnyThread;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage, NSWorkspace};
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};
use objc2_uniform_type_identifiers::{UTType, UTTypeFolder};

use crate::{main_thread, ns_string};

/// PNG bytes per (cache key, size); `None` remembers that AppKit could not render it.
type IconCache = HashMap<(String, u32), Option<Arc<Vec<u8>>>>;

thread_local! {
    static CACHE: RefCell<IconCache> = RefCell::new(HashMap::new());
}

/// Icons depend on file type, except for bundles, volumes and home folders,
/// which can carry their own icon and are keyed by path.
fn cache_key(path: &Path, is_dir: bool) -> String {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if !is_dir {
        return format!("ext:{ext}");
    }
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let special = path == Path::new("/")
        || path.starts_with("/Volumes")
        || home.as_deref().is_some_and(|h| path == h || path.parent() == Some(h));
    if special || !ext.is_empty() { format!("path:{}", path.display()) } else { "dir".to_owned() }
}

/// Finder's icon for a content type rather than a path. Used when a cloud
/// placeholder must not make AppKit inspect the item's custom icon resource.
pub fn type_icon_png(ext: &str, is_dir: bool, points: u32) -> Option<Arc<Vec<u8>>> {
    main_thread()?;
    let key = (format!("type:{}", if is_dir { "dir" } else { ext }), points);
    if let Some(hit) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return hit;
    }
    // SAFETY: the UTType constants are immutable for the process lifetime.
    let identifier = if is_dir {
        unsafe { UTTypeFolder }.identifier().to_string()
    } else {
        UTType::typeWithFilenameExtension(&NSString::from_str(ext))
            .map(|content_type| content_type.identifier().to_string())
            .unwrap_or_else(|| "public.data".into())
    };
    #[allow(deprecated)]
    let image = NSWorkspace::sharedWorkspace().iconForFileType(&NSString::from_str(&identifier));
    let png = png_of(&image, points * 2).map(Arc::new);
    CACHE.with(|c| c.borrow_mut().insert(key, png.clone()));
    png
}

/// Finder's icon for `path` as PNG bytes, rendered at `points` × 2 for Retina.
/// Cached per type; `None` if AppKit could not render it.
pub fn icon_png(path: &Path, is_dir: bool, points: u32) -> Option<Arc<Vec<u8>>> {
    main_thread()?;
    let key = (cache_key(path, is_dir), points);
    if let Some(hit) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return hit;
    }
    let _span = tracing::debug_span!("icon fetch").entered();
    let image = NSWorkspace::sharedWorkspace().iconForFile(&ns_string(path));
    let png = png_of(&image, points * 2).map(Arc::new);
    CACHE.with(|c| c.borrow_mut().insert(key, png.clone()));
    png
}

fn png_of(image: &NSImage, pixels: u32) -> Option<Vec<u8>> {
    let mut rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(f64::from(pixels), f64::from(pixels)));
    // SAFETY: `rect` is a valid, live pointer; no context or hints are passed.
    let cg = unsafe { image.CGImageForProposedRect_context_hints(&mut rect, None, None) }?;
    let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg);
    // SAFETY: an empty property dictionary is valid for PNG encoding.
    let data = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }?;
    Some(data.to_vec())
}
