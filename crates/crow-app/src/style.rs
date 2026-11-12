//! Theme helpers. Views style themselves with theme-index roles only.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crow_theme::{Rgb, Theme, Variant};
use gpui::{
    BoxShadow, Image, ImageFormat, Pixels, Rgba, SharedString, TextRun, Window, WindowAppearance, px, rgb, rgba,
};

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

/// The dimming layer under a sheet. Dark themes darken with `bg`; light themes
/// darken with `fg`, because a `bg` scrim would lighten a light window.
pub fn scrim(theme: &Theme) -> Rgba {
    match theme.variant {
        Variant::Dark => color_alpha(theme.role("bg"), 0x8c),
        Variant::Light => color_alpha(theme.role("fg"), 0x40),
    }
}

/// Where the keys go: the focused selection.
pub fn active_selection(theme: &Theme) -> Rgba {
    color_alpha(theme.role("accent"), 0x55)
}

/// A selection without focus.
pub fn inactive_selection(theme: &Theme) -> Rgba {
    color(theme.role("selection"))
}

/// The folder you are in (the sidebar's current row).
pub fn current_location(theme: &Theme) -> Rgba {
    color(theme.role("selection"))
}

/// A drag's target.
pub fn drop_target(theme: &Theme) -> Rgba {
    color_alpha(theme.role("accent"), 0x40)
}

/// A keyboard cursor ring, for a focused item drawn without a fill. An inset
/// zero-blur shadow, so it never changes layout and list overflow can't clip it.
#[allow(dead_code)] // U8 draws it.
pub fn cursor_ring(theme: &Theme) -> Vec<BoxShadow> {
    vec![BoxShadow::new(px(0.), px(0.), color_alpha(theme.role("accent"), 0x99).into()).spread_radius(px(1.5)).inset()]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tints_follow_roles() {
        let theme = crow_theme::fallback();
        assert_eq!(active_selection(&theme), color_alpha(theme.role("accent"), 0x55));
        assert_eq!(inactive_selection(&theme), color(theme.role("selection")));
        assert_eq!(current_location(&theme), color(theme.role("selection")));
        assert_eq!(drop_target(&theme), color_alpha(theme.role("accent"), 0x40));
        let ring = cursor_ring(&theme);
        assert_eq!(ring.len(), 1);
        assert!(ring[0].inset);
        assert_eq!(ring[0].spread_radius, px(1.5));
        assert_eq!(ring[0].blur_radius, px(0.));
    }
}
