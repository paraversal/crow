use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crow_fs::IconKind;
use objc2::AnyThread;
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSColor, NSFontWeightSemibold, NSImage, NSImageSymbolConfiguration,
    NSWorkspace,
};
use objc2_foundation::{NSArray, NSDictionary, NSPoint, NSRect, NSSize, NSString};
use objc2_uniform_type_identifiers::{UTType, UTTypeData, UTTypeFolder, UTTypeUnixExecutable};

use crate::{main_thread, ns_string};

#[derive(Hash, PartialEq, Eq)]
enum IconKey {
    Data,
    Folder,
    Executable,
    Extension(String),
    Path(PathBuf),
    /// An SF Symbol drawn in one color (RGBA bytes).
    Symbol(String, [u8; 4]),
}

/// PNG bytes per icon and size. A cached None means AppKit could not render it.
type IconCache = HashMap<(IconKey, u32), Option<Arc<Vec<u8>>>>;
const MAX_CACHED_ICONS: usize = 4096;

thread_local! {
    static CACHE: RefCell<IconCache> = RefCell::new(HashMap::new());
    /// Each symbol's natural size in points, recorded when it is first drawn.
    static SYMBOL_SIZES: RefCell<HashMap<String, (f32, f32)>> = RefCell::new(HashMap::new());
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

/// The SF Symbol `name` (what Finder's sidebar draws) as PNG bytes in one color at its own
/// proportions, at `points` nominal size (16 is the symbol's natural size) × 2 pixels. `None` when the symbol does not exist on this macOS or off the main thread.
///
/// Returns the PNG with the size to draw it at, in points: the PNG holds more pixels than that on
/// a Retina screen, so its pixel size is not the draw size.
pub fn symbol_png(name: &str, points: u32, rgba: [u8; 4]) -> Option<(Arc<Vec<u8>>, f32, f32)> {
    let png = cached_icon(IconKey::Symbol(name.to_owned(), rgba), points)?;
    let (width, height) = SYMBOL_SIZES.with_borrow(|sizes| sizes.get(name).copied())?;
    let scale = points as f32 / 16.0;
    Some((png, width * scale, height * scale))
}

fn symbol_image(name: &str, rgba: [u8; 4]) -> Option<objc2::rc::Retained<NSImage>> {
    let symbol = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None)?;
    let [r, g, b, a] = rgba.map(|channel| f64::from(channel) / 255.0);
    let color = NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, a);
    // Finder's sidebar weight: Semibold at the default 13 pt keeps the natural footprint with
    // Finder's stroke. Every layer gets the full color (a hierarchical color would fade a
    // symbol's secondary layers, such as Desktop's frame).
    // SAFETY: the font weight constants are immutable for the process lifetime.
    let weight = NSImageSymbolConfiguration::configurationWithPointSize_weight(13.0, unsafe { NSFontWeightSemibold });
    let palette = NSImageSymbolConfiguration::configurationWithPaletteColors(&NSArray::from_retained_slice(&[
        color.clone(),
        color.clone(),
        color,
    ]));
    symbol.imageWithSymbolConfiguration(&weight.configurationByApplyingConfiguration(&palette))
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
            if has_own_icon(path, home.as_deref()) { IconKey::Path(path.to_path_buf()) } else { IconKey::Folder }
        }
        IconKind::Path => IconKey::Path(path.to_path_buf()),
    };
    cached_icon(key, points)
}

/// System folders Finder gives their own icon, besides the root, volumes, home and what sits
/// directly in it (Desktop, Documents, Downloads, Movies, Music, Pictures, Public, Library…).
const SPECIAL_SYSTEM_FOLDERS: [&str; 7] =
    ["/Applications", "/Applications/Utilities", "/Developer", "/Library", "/System", "/Users", "/Users/Shared"];

/// Whether Finder draws `path` with an icon of its own rather than the shared folder icon.
fn has_own_icon(path: &Path, home: Option<&Path>) -> bool {
    path == Path::new("/")
        || path.starts_with("/Volumes")
        || SPECIAL_SYSTEM_FOLDERS.iter().any(|special| path == Path::new(special))
        || home.is_some_and(|home| path == home || path.parent() == Some(home))
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
            IconKey::Symbol(name, rgba) => match symbol_image(name, *rgba) {
                Some(image) => image,
                None => {
                    CACHE.with_borrow_mut(|cache| cache.insert(key, None));
                    return None;
                }
            },
        }
    };
    let png = match &key.0 {
        // A symbol keeps its own proportions: `points` is the nominal point size (16 draws the
        // symbol at its natural size, like Finder's sidebar), and the PNG is as wide as the
        // symbol is, so a house is wider than a document.
        IconKey::Symbol(name, _) => {
            let size = image.size();
            SYMBOL_SIZES.with_borrow_mut(|sizes| sizes.insert(name.clone(), (size.width as f32, size.height as f32)));
            let scale = f64::from(points) / 16.0;
            png_of_size(&image, (size.width * scale).round().max(1.0), (size.height * scale).round().max(1.0))
        }
        _ => png_of(&image, points * 2),
    }
    .map(Arc::new);
    CACHE.with_borrow_mut(|cache| {
        if cache.len() >= MAX_CACHED_ICONS {
            cache.clear();
        }
        cache.insert(key, png.clone());
    });
    png
}

fn png_of(image: &NSImage, pixels: u32) -> Option<Vec<u8>> {
    png_of_size(image, f64::from(pixels), f64::from(pixels))
}

fn png_of_size(image: &NSImage, width: f64, height: f64) -> Option<Vec<u8>> {
    let mut rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height));
    // SAFETY: `rect` is a valid, live pointer; no context or hints are passed.
    let cg = unsafe { image.CGImageForProposedRect_context_hints(&mut rect, None, None) }?;
    let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg);
    // SAFETY: an empty property dictionary is valid for PNG encoding.
    let data = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }?;
    Some(data.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finders_special_folders_get_their_own_icon() {
        let home = Path::new("/Users/me");
        for special in
            ["/", "/Volumes/Disk", "/Applications", "/Library", "/System", "/Users", "/Users/me", "/Users/me/Desktop"]
        {
            assert!(has_own_icon(Path::new(special), Some(home)), "{special}");
        }
        for plain in ["/Applications/Foo", "/usr/local", "/Users/me/Desktop/x", "/Users/other"] {
            assert!(!has_own_icon(Path::new(plain), Some(home)), "{plain}");
        }
    }
}
