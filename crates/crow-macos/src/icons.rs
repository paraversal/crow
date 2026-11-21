use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crow_fs::IconKind;
use objc2::AnyThread;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage, NSWorkspace};
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};
use objc2_uniform_type_identifiers::{UTType, UTTypeData, UTTypeFolder, UTTypeUnixExecutable};

use crate::{main_thread, ns_string};

#[derive(Hash, PartialEq, Eq)]
enum IconKey {
    Data,
    Folder,
    Executable,
    Extension(String),
    Path(PathBuf),
}

/// PNG bytes per icon and size. A cached None means AppKit could not render it.
type IconCache = HashMap<(IconKey, u32), Option<Arc<Vec<u8>>>>;

thread_local! {
    static CACHE: RefCell<IconCache> = RefCell::new(HashMap::new());
}

fn type_key(ext: &str, is_dir: bool) -> IconKey {
    if is_dir {
        IconKey::Folder
    } else if ext.is_empty() {
        IconKey::Data
    } else {
        IconKey::Extension(ext.to_lowercase())
    }
}

/// Finder's icon for a content type, without inspecting the path. Used for
/// cloud placeholders and ordinary files that can share an icon.
pub fn type_icon_png(ext: &str, is_dir: bool, points: u32) -> Option<Arc<Vec<u8>>> {
    cached_icon(type_key(ext, is_dir), points)
}

/// Finder's icon for `path` as PNG bytes, rendered at `points` × 2 for Retina.
/// The listing supplies the icon class so aliases and custom icons cannot
/// enter a shared type cache. Callers without metadata use `IconKind::Path`.
pub fn icon_png(path: &Path, kind: IconKind, points: u32) -> Option<Arc<Vec<u8>>> {
    let key = match kind {
        IconKind::File => type_key(path.extension().and_then(|ext| ext.to_str()).unwrap_or(""), false),
        IconKind::Executable => IconKey::Executable,
        IconKind::Folder => {
            let home = std::env::var_os("HOME").map(PathBuf::from);
            if path == Path::new("/")
                || path.starts_with("/Volumes")
                || home.as_deref().is_some_and(|home| path == home || path.parent() == Some(home))
            {
                IconKey::Path(path.to_path_buf())
            } else {
                IconKey::Folder
            }
        }
        IconKind::Path => IconKey::Path(path.to_path_buf()),
    };
    cached_icon(key, points)
}

fn cached_icon(key: IconKey, points: u32) -> Option<Arc<Vec<u8>>> {
    main_thread()?;
    let key = (key, points);
    if let Some(hit) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return hit;
    }
    let _span = tracing::debug_span!("icon fetch").entered();
    let workspace = NSWorkspace::sharedWorkspace();
    // SAFETY: the UTType constants are immutable for the process lifetime.
    let image = unsafe {
        match &key.0 {
            IconKey::Data => workspace.iconForContentType(UTTypeData),
            IconKey::Folder => workspace.iconForContentType(UTTypeFolder),
            IconKey::Executable => workspace.iconForContentType(UTTypeUnixExecutable),
            IconKey::Extension(ext) => {
                let content_type = UTType::typeWithFilenameExtension(&NSString::from_str(ext));
                workspace.iconForContentType(content_type.as_deref().unwrap_or(UTTypeData))
            }
            IconKey::Path(path) => workspace.iconForFile(&ns_string(path)),
        }
    };
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
