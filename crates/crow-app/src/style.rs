//! Theme helpers. Views style themselves with theme-index roles only.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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

/// The toolbar band's height. Sheets, the palette and the activity list start
/// below it, so one number keeps them aligned.
pub const BAND_HEIGHT: f32 = 52.;

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

/// How solid the sidebar stays while the window is translucent: enough of the blurred desktop
/// shows to read as glass, enough tone stays for the labels to keep their contrast on any backdrop.
pub const TRANSLUCENT_SIDEBAR_ALPHA: u8 = 0xcc;

/// The thin lines that frame the window: the sidebar edge, the bars, the preview edge. Mixed from
/// `bg` toward `fg` so every theme gets a Finder-like hairline; a theme's `border` role is often a
/// selection-grade slot (Solarized's `base02`), far too heavy for a 1 px frame line.
pub fn hairline(theme: &Theme) -> Rgba {
    color(theme.role("bg").mix(theme.role("fg"), HAIRLINE_MIX))
}

/// How far from `bg` to `fg` the hairline sits: Finder's own lines (#d9d9d9 on white, #3d3d3d on
/// #1e1e1e) are about this far.
const HAIRLINE_MIX: f32 = 0.15;

/// A selection without focus.
pub fn inactive_selection(theme: &Theme) -> Rgba {
    color(theme.role("selection"))
}

/// The folder you are in (the sidebar's current row).
pub fn current_location(theme: &Theme) -> Rgba {
    color(theme.role("selection"))
}

/// The focused column's tint: the `surface` role over the column background, as
/// the boards show it. Surface is lighter than the background in dark themes and
/// darker in light ones, so one role reads in both; 0x50 is the board's contrast.
pub fn focused_column(theme: &Theme) -> Rgba {
    color_alpha(theme.role("surface"), 0x50)
}

/// An error token's fill: the error role, faint.
pub fn error_soft(theme: &Theme) -> Rgba {
    color_alpha(theme.role("error"), 0x24)
}

/// The mark for a drag's target: an accent outline that leaves the selection or
/// current-folder fill underneath visible. Absolute, so it never shifts layout;
/// callers place it inside a `relative()` parent.
pub fn drop_outline(theme: &Theme) -> gpui::Div {
    use gpui::{InteractiveElement, Styled, div};
    div()
        .absolute()
        .rounded_md()
        .border_1()
        .border_color(color(theme.role("accent")))
        .debug_selector(|| "drop-outline".into())
}

/// How much of its contrast a hidden item's icon and text keep (Finder dims them;
/// the selection fill behind them stays full).
pub const HIDDEN_DIM: f32 = 0.5;

/// The mark for a cut row: a dashed outline around the run. A cut item is pending
/// a move, so it must not read as a hidden item (which only dims its content).
/// Absolute, so it never shifts layout; the caller adds the run's rounding.
pub fn cut_outline(theme: &Theme, top: bool, bottom: bool) -> gpui::Div {
    use gpui::prelude::FluentBuilder;
    use gpui::{Styled, div};
    div()
        .absolute()
        .inset_0()
        .border_l_2()
        .border_r_2()
        .when(top, |el| el.border_t_2())
        .when(bottom, |el| el.border_b_2())
        .border_dashed()
        .border_color(color_alpha(theme.role("fg"), 0x99))
        .when(top && bottom, |el| el.rounded_md())
        .when(top && !bottom, |el| el.rounded_t_md())
        .when(bottom && !top, |el| el.rounded_b_md())
}

/// The opacity of a row's icon and text: hidden items are dimmed, except while
/// being renamed, so the field reads at full contrast.
pub fn content_opacity(hidden: bool, renaming: bool) -> f32 {
    if hidden && !renaming { HIDDEN_DIM } else { 1.0 }
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

/// A text field's focus ring: the accent at the boards' ring alpha, drawn 2 px
/// wide on both text fields (`/` filter and search). Rows and the sidebar cursor
/// use `cursor_ring` instead.
pub fn field_ring(theme: &Theme) -> Rgba {
    color_alpha(theme.role("accent"), 0x8c)
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
    cloud_icon(None, path, is_dir, points)
}

pub fn entry_cloud_icon(
    status: Option<crow_fs::CloudStatus>,
    entry: &crow_fs::Entry,
    points: u32,
) -> Option<Arc<Image>> {
    native_icon(status, &entry.path, entry.is_dir, entry.icon_kind, points).and_then(image_from_png)
}

/// Use a type icon while a potential cloud item is unprobed or unavailable.
pub fn cloud_icon(status: Option<crow_fs::CloudStatus>, path: &Path, is_dir: bool, points: u32) -> Option<Arc<Image>> {
    native_icon(status, path, is_dir, crow_fs::IconKind::Path, points).and_then(image_from_png)
}

fn native_icon(
    status: Option<crow_fs::CloudStatus>,
    path: &Path,
    is_dir: bool,
    kind: crow_fs::IconKind,
    points: u32,
) -> Option<Arc<Vec<u8>>> {
    if native_cloud_icon(status, path) {
        let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
        return crow_macos::type_icon_png(ext, is_dir, points);
    }
    // io-ok: render-time AppKit icon, cached by type or by path for item-specific icons (#47).
    crow_macos::icon_png(path, kind, points)
}

/// PNG bytes of the Finder icon for a drag, never inspecting a cloud
/// placeholder's own icon resource.
pub fn drag_icon_png(cx: &gpui::App, path: &Path, is_dir: bool, points: u32) -> Option<Arc<Vec<u8>>> {
    let status =
        cx.try_global::<crate::cloud_status::CloudStatusService>().and_then(|service| service.0.read(cx).status(path));
    native_icon(status, path, is_dir, crow_fs::IconKind::Path, points)
}

/// A cloud placeholder (or an unprobed cloud candidate) keeps its native
/// type/cloud icon: an icon pack must not hide the download affordance.
pub fn native_cloud_icon(status: Option<crow_fs::CloudStatus>, path: &Path) -> bool {
    status.map_or_else(|| crow_fs::is_cloud_candidate(path), |status| !status.is_local())
}

fn image_from_png(png: Arc<Vec<u8>>) -> Option<Arc<Image>> {
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

/// The Gallery rail leaves the full modified time available to accessibility.
pub fn format_date_short(t: Option<std::time::SystemTime>) -> String {
    format_short_at(t, std::time::SystemTime::now(), false)
}

/// `format_time_short` with an explicit "now", so the buckets are testable.
pub fn format_time_short_at(t: Option<std::time::SystemTime>, now: std::time::SystemTime) -> String {
    format_short_at(t, now, true)
}

fn format_short_at(t: Option<std::time::SystemTime>, now: std::time::SystemTime, show_time: bool) -> String {
    use chrono::Datelike as _;
    let Some(t) = t else { return "—".into() };
    let local: chrono::DateTime<chrono::Local> = t.into();
    let now: chrono::DateTime<chrono::Local> = now.into();
    let date = local.date_naive();
    let today = now.date_naive();
    if date == today {
        if show_time { local.format("Today %H:%M").to_string() } else { "Today".into() }
    } else if Some(date) == today.pred_opt() {
        "Yesterday".into()
    } else if date.year() == today.year() {
        local.format("%-d. %b").to_string()
    } else {
        local.format(if show_time { "%-d. %b %Y" } else { "%-d %b %y" }).to_string()
    }
}

/// A name as the UI draws it: ASCII control characters (newlines, tabs) become
/// spaces. GPUI's text shaping asserts that no newline reaches it, and a space
/// keeps the name readable. The byte length is unchanged, so match ranges and
/// text-field cursors still line up with the original name.
pub fn display_text(text: &str) -> Cow<'_, str> {
    if !text.bytes().any(|byte| byte.is_ascii_control()) {
        return Cow::Borrowed(text);
    }
    Cow::Owned(text.chars().map(|c| if c.is_ascii_control() { ' ' } else { c }).collect())
}

/// Finder's display names for one path: (name, path) from the volume down.
type DisplayNames = Arc<[(String, PathBuf)]>;
type DisplayCache = RefCell<HashMap<PathBuf, String>>;

/// Finder's display names for a path's ancestors, volume first, cached per thread.
/// Each name is a LaunchServices filesystem query (`displayNameAtPath`), and the tab
/// strip, title bar, bottom bar and path bar ask for the same paths every frame.
/// The cache holds one name per path, not per whole path: the bottom bar follows the
/// cursor, so every arrow key asks for a new leaf under the same ancestors, and a
/// whole-path key re-queried every ancestor (up to a second of main-thread stall in the
/// live fuzz). A renamed folder or volume changes the path, so a cached name stays valid.
pub fn display_components(path: &Path) -> DisplayNames {
    thread_local! {
        static CACHE: DisplayCache = RefCell::new(HashMap::new());
    }
    CACHE.with_borrow_mut(|cache| display_components_cached(path, cache, crow_macos::display_name_at))
}

fn display_components_cached(
    path: &Path,
    cache: &mut HashMap<PathBuf, String>,
    lookup: impl Fn(&Path) -> String,
) -> DisplayNames {
    const MAX: usize = 2048;
    crow_macos::display_component_paths(path)
        .into_iter()
        .map(|part| {
            if cache.len() >= MAX && !cache.contains_key(&part) {
                cache.clear();
            }
            let name = cache.entry(part.clone()).or_insert_with(|| lookup(&part)).clone();
            (name, part)
        })
        .collect()
}

/// A folder's own name for labels: the volume name at the root, else the last
/// component as Finder writes it.
pub fn display_name(dir: &Path) -> String {
    let name = display_components(dir).last().map_or_else(|| dir.display().to_string(), |(name, _)| name.clone());
    display_text(&name).into_owned()
}

/// An abbreviated path (`~/Pictures`) as the UI draws it.
pub fn display_path(path: &Path, home: &Path) -> String {
    display_text(&crow_core::abbreviate_home(path, home)).into_owned()
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
    truncate_middle_parts_to_fit(text, |s| measure(s) <= max)
}

/// Middle-elide text against the caller's layout, keeping grapheme clusters intact.
pub fn truncate_middle_parts_to_fit(text: &str, fits: impl Fn(&str) -> bool) -> Truncated {
    use unicode_segmentation::UnicodeSegmentation;

    let clean = display_text(text);
    let text = clean.as_ref();
    if fits(text) {
        return Truncated { text: SharedString::from(text.to_owned()), cut: None };
    }
    let boundaries = text.grapheme_indices(true).map(|(index, _)| index).chain([text.len()]).collect::<Vec<_>>();
    let graphemes = boundaries.len() - 1;
    let shortened = |keep: usize| -> (String, (usize, usize)) {
        let head = keep.div_ceil(2);
        let tail = keep - head;
        let head_end = boundaries[head];
        let tail_start = boundaries[graphemes - tail];
        (format!("{}\u{2026}{}", &text[..head_end], &text[tail_start..]), (head_end, tail_start))
    };
    // The longest shortening that still fits.
    let (mut lo, mut hi) = (0, graphemes.saturating_sub(1));
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if fits(&shortened(mid).0) { lo = mid } else { hi = mid - 1 }
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
    use std::cell::Cell;

    use super::*;

    #[test]
    fn display_names_are_cached_per_ancestor_so_a_new_leaf_costs_one_query() {
        let queries = Cell::new(0);
        let lookup = |path: &Path| {
            queries.set(queries.get() + 1);
            path.file_name().map_or_else(|| "/".to_owned(), |name| name.to_string_lossy().into_owned())
        };
        let mut cache = HashMap::new();
        let first = display_components_cached(Path::new("/Users/me/Notes/a.txt"), &mut cache, lookup);
        assert_eq!(
            first.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(),
            ["/", "Users", "me", "Notes", "a.txt"]
        );
        assert_eq!(queries.get(), 5);
        // The cursor moves to the next file: only the new leaf is asked.
        display_components_cached(Path::new("/Users/me/Notes/b.txt"), &mut cache, lookup);
        assert_eq!(queries.get(), 6);
        // The same path again asks nothing.
        display_components_cached(Path::new("/Users/me/Notes/b.txt"), &mut cache, lookup);
        assert_eq!(queries.get(), 6);
    }

    #[test]
    fn hidden_items_are_dimmed_except_while_renamed() {
        assert_eq!(content_opacity(true, false), HIDDEN_DIM, "a hidden item is dimmed, like Finder");
        assert_eq!(content_opacity(true, true), 1.0, "its rename field reads at full contrast");
        assert_eq!(content_opacity(false, false), 1.0);
    }

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
    fn the_frame_hairline_is_subtle_in_every_builtin_theme() {
        let lum = |c: Rgba| 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
        for info in crow_theme::list_theme_infos(std::path::Path::new("/nonexistent")) {
            let theme = crow_theme::load_theme(std::path::Path::new("/nonexistent"), &info.id).unwrap();
            let (line, bg) = (lum(hairline(&theme)), lum(color(theme.role("bg"))));
            // Finder's own lines are about 0.14 apart in sRGB lightness; none may get heavy.
            assert!((line - bg).abs() < 0.2, "{} hairline too heavy: {line} vs {bg}", info.id);
            assert!((line - bg).abs() > 0.02, "{} hairline vanishes", info.id);
        }
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
        let base = color(theme.role("bg"));
        let focused = focused_column(&theme);
        assert_eq!(focused, color_alpha(theme.role("surface"), 0x50));
        assert!(focused.r > base.r && focused.g > base.g && focused.b > base.b, "lighter in a dark theme");
        let ring = cursor_ring(&theme);
        assert_eq!(ring.len(), 1);
        assert!(ring[0].inset);
        assert_eq!(ring[0].spread_radius, px(1.5));
        assert_eq!(ring[0].blur_radius, px(0.));
    }
}
