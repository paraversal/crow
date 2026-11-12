//! Theme helpers. Views style themselves with theme-index roles only.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crow_theme::{Rgb, Variant};
use gpui::{Image, ImageFormat, Pixels, Rgba, SharedString, TextRun, Window, WindowAppearance, rgb, rgba};

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

/// Finder's kind for an entry, from the system's type descriptions (cached per extension).
pub fn kind(entry: &crow_fs::Entry) -> Arc<str> {
    let ext = if entry.is_dir && !entry.is_package() { "" } else { entry.ext() };
    crow_macos::kind_name(ext, entry.is_dir && !entry.is_package())
}

/// Shorten `text` in the middle to fit `max` (Finder keeps both the start and the
/// extension visible: `8b184f2a-90…app_icon.png`). Only long names are measured twice or more.
pub fn truncate_middle(text: &str, max: Pixels, font_size: Pixels, window: &Window) -> SharedString {
    let font = window.text_style().font();
    let measure = |s: &str| {
        let run = TextRun {
            len: s.len(),
            font: font.clone(),
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window.text_system().shape_line(SharedString::from(s.to_owned()), font_size, &[run], None).width
    };
    if measure(text) <= max {
        return SharedString::from(text.to_owned());
    }
    let chars: Vec<char> = text.chars().collect();
    let shortened = |keep: usize| -> String {
        let head = keep.div_ceil(2);
        let tail = keep - head;
        chars[..head].iter().chain(['…'].iter()).chain(chars[chars.len() - tail..].iter()).collect()
    };
    // The longest shortening that still fits.
    let (mut lo, mut hi) = (0, chars.len().saturating_sub(1));
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if measure(&shortened(mid)) <= max { lo = mid } else { hi = mid - 1 }
    }
    shortened(lo).into()
}
