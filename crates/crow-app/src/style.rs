//! Theme helpers. Views style themselves with theme-index roles only.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crow_theme::{Rgb, Theme, Variant};
use gpui::{
    BoxShadow, FontWeight, HighlightStyle, Image, ImageFormat, Pixels, Rgba, SharedString, StyledText, TextRun, Window,
    WindowAppearance, px, rgb, rgba,
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

/// An error token's fill: the error role, faint.
pub fn error_soft(theme: &Theme) -> Rgba {
    color_alpha(theme.role("error"), 0x24)
}

/// A drag's target.
pub fn drop_target(theme: &Theme) -> Rgba {
    color_alpha(theme.role("accent"), 0x40)
}

/// The fill a feedback effect puts over a row: accent for a receive, error for a
/// refuse. `None` when the effect has no tint at this moment.
pub fn effect_tint(look: crow_core::motion::Look, theme: &Theme) -> Option<Rgba> {
    if look.tint <= 0.0 {
        return None;
    }
    Some(match look.tint_kind {
        crow_core::motion::Tint::Accent => color_alpha(theme.role("accent"), (look.tint * 0x50 as f32) as u8),
        crow_core::motion::Tint::Error => {
            let mut color = error_soft(theme);
            color.a *= look.tint * 2.0;
            color
        }
    })
}

/// A keyboard cursor ring, for a focused item drawn without a fill. An inset
/// zero-blur shadow, so it never changes layout and list overflow can't clip it.
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

/// A narrow column's date: "Today 14:32", "Yesterday", "4. Nov", "4. Nov 2024".
pub fn format_time_short(t: Option<std::time::SystemTime>) -> String {
    format_time_short_at(t, std::time::SystemTime::now())
}

/// `format_time_short` with an explicit "now", so the buckets are testable.
pub fn format_time_short_at(t: Option<std::time::SystemTime>, now: std::time::SystemTime) -> String {
    use chrono::Datelike as _;
    let Some(t) = t else { return "—".into() };
    let local: chrono::DateTime<chrono::Local> = t.into();
    let now: chrono::DateTime<chrono::Local> = now.into();
    let date = local.date_naive();
    let today = now.date_naive();
    if date == today {
        local.format("Today %H:%M").to_string()
    } else if Some(date) == today.pred_opt() {
        "Yesterday".into()
    } else if date.year() == today.year() {
        local.format("%-d. %b").to_string()
    } else {
        local.format("%-d. %b %Y").to_string()
    }
}

/// A folder's own name for labels: the volume name at the root, else the last
/// component as Finder writes it.
pub fn display_name(dir: &Path) -> String {
    crow_macos::display_components(dir).pop().map_or_else(|| dir.display().to_string(), |(name, _)| name)
}

/// Finder's kind for an entry, from the system's type descriptions (cached per extension).
pub fn kind(entry: &crow_fs::Entry) -> Arc<str> {
    let ext = if entry.is_dir && !entry.is_package() { "" } else { entry.ext() };
    crow_macos::kind_name(ext, entry.is_dir && !entry.is_package())
}

/// Middle-truncated text, with the byte offsets needed to map highlight ranges back.
pub struct Truncated {
    pub text: SharedString,
    /// `Some((head bytes, tail start))`: the original is `original[..head] + "…" + original[tail..]`.
    pub cut: Option<(usize, usize)>,
}

/// Shorten `text` in the middle to fit `max` (Finder keeps both the start and the
/// extension visible: `8b184f2a-90…app_icon.png`). Only long names are measured twice or more.
pub fn truncate_middle_parts(text: &str, max: Pixels, font_size: Pixels, window: &Window) -> Truncated {
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
        return Truncated { text: SharedString::from(text.to_owned()), cut: None };
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let shortened = |keep: usize| -> (String, (usize, usize)) {
        let head = keep.div_ceil(2);
        let tail = keep - head;
        let head_end = if head == 0 { 0 } else { chars[head - 1].0 + chars[head - 1].1.len_utf8() };
        let tail_start = if tail == 0 { text.len() } else { chars[chars.len() - tail].0 };
        (format!("{}\u{2026}{}", &text[..head_end], &text[tail_start..]), (head_end, tail_start))
    };
    // The longest shortening that still fits.
    let (mut lo, mut hi) = (0, chars.len().saturating_sub(1));
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if measure(&shortened(mid).0) <= max { lo = mid } else { hi = mid - 1 }
    }
    let (text, cut) = shortened(lo);
    Truncated { text: text.into(), cut: Some(cut) }
}

/// `truncate_middle` without the range mapping, for callers that only draw text.
pub fn truncate_middle(text: &str, max: Pixels, font_size: Pixels, window: &Window) -> SharedString {
    truncate_middle_parts(text, max, font_size, window).text
}

/// Middle-truncate `text` and move `ranges` into the displayed text: ranges under
/// the ellipsis are dropped, ranges after it are shifted.
pub fn truncate_middle_with_ranges(
    text: &str,
    ranges: &[std::ops::Range<usize>],
    max: Pixels,
    font_size: Pixels,
    window: &Window,
) -> (SharedString, Vec<std::ops::Range<usize>>) {
    let truncated = truncate_middle_parts(text, max, font_size, window);
    let mut out = Vec::new();
    for range in ranges {
        for mapped in map_range(&truncated, range.clone()) {
            if !mapped.is_empty() {
                out.push(mapped);
            }
        }
    }
    (truncated.text, out)
}

/// Map a byte range in the original text to the truncated display text, dropping the
/// part under the ellipsis. Unused slots are `0..0`.
pub fn map_range(t: &Truncated, range: std::ops::Range<usize>) -> [std::ops::Range<usize>; 2] {
    let mut out = [0..0, 0..0];
    let intersect = |lo: usize, hi: usize| {
        let start = range.start.max(lo);
        let end = range.end.min(hi);
        (start < end).then_some(start..end)
    };
    match t.cut {
        None => {
            if let Some(part) = intersect(0, t.text.len()) {
                out[0] = part;
            }
        }
        Some((head_end, tail_start)) => {
            if let Some(part) = intersect(0, head_end) {
                out[0] = part;
            }
            if let Some(part) = intersect(tail_start, range.end.max(tail_start))
                && part.end > tail_start
            {
                let offset = head_end + '\u{2026}'.len_utf8();
                out[1] = (offset + part.start - tail_start)..(offset + part.end - tail_start);
            }
        }
    }
    out
}

/// Accent + semibold matches; `fg` on an accent-tinted selection so it stays legible.
pub fn highlight_text(
    text: impl Into<SharedString>,
    ranges: &[std::ops::Range<usize>],
    theme: &Theme,
    on_selection: bool,
) -> StyledText {
    let text = text.into();
    let color: gpui::Hsla = color(if on_selection { theme.role("fg") } else { theme.role("accent") }).into();
    let style = HighlightStyle { color: Some(color), font_weight: Some(FontWeight::SEMIBOLD), ..Default::default() };
    StyledText::new(text.clone()).with_highlights(
        ranges
            .iter()
            .filter(|range| !range.is_empty())
            .map(|range| (clamped(&text, range.clone()), style))
            .filter(|(range, _)| !range.is_empty()),
    )
}

/// Pull both ends of `range` back to char boundaries inside `text`.
fn clamped(text: &str, range: std::ops::Range<usize>) -> std::ops::Range<usize> {
    let mut start = range.start.min(text.len());
    let mut end = range.end.min(text.len());
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    start..end.max(start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_range_through_truncation() {
        let t = Truncated { text: "abc\u{2026}xyz".into(), cut: Some((3, 6)) };
        assert_eq!(map_range(&t, 1..2), [1..2, 0..0], "range in the head");
        assert_eq!(map_range(&t, 7..8), [0..0, 7..8], "range in the tail");
        assert_eq!(map_range(&t, 2..7), [2..3, 6..7], "range across the ellipsis keeps both parts");
        assert_eq!(map_range(&t, 3..6), [0..0, 0..0], "range entirely under the ellipsis");
        let whole = Truncated { text: "alpha".into(), cut: None };
        assert_eq!(map_range(&whole, 2..4), [2..4, 0..0], "uncut text maps directly");
        assert_eq!(map_range(&whole, 2..99), [2..5, 0..0], "clamped to the text");
    }

    #[test]
    fn highlight_text_clamps_ranges() {
        let theme = crow_theme::fallback();
        // Mid-char and past-the-end ranges must not trip StyledText's boundary asserts.
        let mid_char = 1..2;
        let past_end = 0..99;
        let _ = highlight_text("\u{e9}", std::slice::from_ref(&mid_char), &theme, false);
        let _ = highlight_text("alpha", std::slice::from_ref(&past_end), &theme, true);
    }

    #[test]
    fn format_time_short_buckets() {
        use chrono::TimeZone as _;
        let at = |y, m, d, hour, minute| -> std::time::SystemTime {
            chrono::Local.with_ymd_and_hms(y, m, d, hour, minute, 0).unwrap().into()
        };
        let now = at(2026, 11, 14, 18, 0);
        assert_eq!(format_time_short_at(Some(at(2026, 11, 14, 14, 32)), now), "Today 14:32");
        assert_eq!(format_time_short_at(Some(at(2026, 11, 13, 9, 5)), now), "Yesterday");
        assert_eq!(format_time_short_at(Some(at(2026, 11, 4, 9, 0)), now), "4. Nov");
        assert_eq!(format_time_short_at(Some(at(2024, 11, 4, 9, 0)), now), "4. Nov 2024");
        assert_eq!(format_time_short_at(None, now), "—");
    }

    #[test]
    fn tint_follows_roles() {
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
