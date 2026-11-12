//! Theme helpers. Views style themselves with theme-index roles only.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crow_theme::{Rgb, Variant};
use gpui::{Image, ImageFormat, Rgba, WindowAppearance, rgb, rgba};

pub fn color(c: Rgb) -> Rgba {
    rgb(c.0)
}

pub fn color_alpha(c: Rgb, alpha: u8) -> Rgba {
    rgba((c.0 << 8) | u32::from(alpha))
}

pub fn variant_of(appearance: WindowAppearance) -> Variant {
    match appearance {
        WindowAppearance::Light | WindowAppearance::VibrantLight => Variant::Light,
        _ => Variant::Dark,
    }
}

pub fn ui_font(family: &str) -> String {
    if family.is_empty() { ".SystemUIFont".to_owned() } else { family.to_owned() }
}

thread_local! {
    // crow-macos caches PNG bytes per file type; this wraps each in a GPUI image once.
    static IMAGES: RefCell<HashMap<usize, Arc<Image>>> = RefCell::new(HashMap::new());
}

/// Finder's icon for `path` at `points`, as a GPUI image.
pub fn icon(path: &Path, is_dir: bool, points: u32) -> Option<Arc<Image>> {
    let png = crow_macos::icon_png(path, is_dir, points)?;
    let key = Arc::as_ptr(&png) as usize;
    Some(IMAGES.with(|images| {
        images
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| Arc::new(Image::from_bytes(ImageFormat::Png, png.as_ref().clone())))
            .clone()
    }))
}

pub fn format_time(t: Option<std::time::SystemTime>) -> String {
    let Some(t) = t else { return "—".into() };
    let local: chrono::DateTime<chrono::Local> = t.into();
    if local.date_naive() == chrono::Local::now().date_naive() {
        local.format("Today at %H:%M").to_string()
    } else {
        local.format("%-d. %B %Y at %H:%M").to_string()
    }
}

/// A Finder-like kind. M2 replaces this with the system's UTType descriptions.
pub fn kind_of(entry: &crow_fs::Entry) -> String {
    if entry.is_dir {
        return if entry.ext() == "app" { "Application".into() } else { "Folder".into() };
    }
    match entry.ext().to_lowercase().as_str() {
        "rs" => "Rust source".into(),
        "md" => "Markdown".into(),
        "toml" => "TOML".into(),
        "json" | "jsonl" => "JSON".into(),
        "txt" => "Plain text".into(),
        "png" | "jpg" | "jpeg" | "heic" | "gif" | "webp" => "Image".into(),
        "pdf" => "PDF document".into(),
        "" => "Document".into(),
        other => format!("{} file", other.to_uppercase()),
    }
}
