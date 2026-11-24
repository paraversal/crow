//! The bottom bar's data and rendering: the path, the transient slot (hints,
//! progress, messages, errors) and the counts. `CrowWindow::bar_state` builds
//! the state; rendering here reads only that.

use std::cell::RefCell;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    Action, Animation, AnimationExt, AnyElement, App, Bounds, Entity, FontWeight, IntoElement, KeyBinding, Pixels,
    Role, SharedString, SpringAnimation, SpringConfig, TextRun, Window, div, prelude::*, px, relative,
    transparent_black,
};

use crate::actions::{
    CancelOperation, ClearFilter, EditPath, Eject, MoveFavoriteDown, MoveFavoriteUp, NextGroup, OpenSelection,
    PathHover, PathUnhover, PreviousGroup, QuickLook, RemoveFromSidebar, SearchReveal, ShowSearch, StartFilter,
    SwitchFocus,
};
use crate::chrome::{Keycap, KeycapWeight, with_action_tooltip, with_tooltip};
use crate::icons::{self, Glyph};
use crate::keys::{self, KeyScope};
use crate::operations::{self, Running};
use crate::pane::{DragHandoff, DraggedSelection, Pane, drag_combo, promote_drag};
use crate::sidebar::CursorKind;
use crate::style::{color, color_alpha, error_soft, icon};
use crate::text_input::{self, Complete, Confirm, TextInput};
use crate::ui::Ui;
use crate::usage;
use crow_core::motion::{self, Effective};
use crow_core::{HintsMode, PathBarMode, idle_hint_visible};
use crow_fs::human_size;

/// The bar's height; tips sit above it.
pub const HEIGHT: f32 = 38.;
/// The path pill (and the field that replaces it): a calm band inside the bar, its segments'
/// highlights inset by the pill's padding on every side so their corners run parallel.
const PILL_HEIGHT: f32 = 26.;
const SEGMENT_HEIGHT: f32 = 20.;
/// How far a segment's highlight sits from the pill's outer edge, the same on all sides.
const PILL_INSET: f32 = (PILL_HEIGHT - SEGMENT_HEIGHT) / 2.;
/// The pill's padding: the inset less its 1 px border.
const PILL_PAD: f32 = PILL_INSET - 1.;
const SEGMENT_RADIUS: f32 = 5.;
const PILL_RADIUS: f32 = SEGMENT_RADIUS + PILL_INSET;

/// One path segment's drawn bounds and destination.
#[derive(Clone, Debug)]
pub struct PathHit {
    pub bounds: Bounds<Pixels>,
    pub path: PathBuf,
    pub is_dir: bool,
}

/// Opens the menu for a segment: its path and whether it is a folder. The window
/// owns the menu, since its lines act on tabs, the sidebar and the shown folder.
pub type PathMenu = Rc<dyn Fn(PathBuf, bool, &mut Window, &mut App)>;

/// The path hits from the last painted bar, shared with the drop router.
pub type PathHits = Rc<RefCell<Vec<PathHit>>>;

fn register_path_hit(
    hits: &PathHits,
    target: PathBuf,
    is_dir: bool,
    last: bool,
    child: impl IntoElement,
    drop: Option<(&crow_theme::Theme, f32)>,
) -> AnyElement {
    let hits = hits.clone();
    div()
        .relative()
        .flex()
        .items_center()
        .map(|wrapper| if last { wrapper.min_w_0() } else { wrapper.flex_none() })
        .on_children_prepainted(move |children, _, _| {
            if let Some(bounds) = children.first() {
                hits.borrow_mut().push(PathHit { bounds: *bounds, path: target.clone(), is_dir });
            }
        })
        .child(child)
        .when_some(drop, |wrapper, (theme, opacity)| {
            wrapper.child(crate::style::drop_outline(theme).inset_0().opacity(opacity))
        })
        .into_any_element()
}

/// Everything the bar draws this frame, computed on the main thread.
pub struct BarState {
    pub primary: Primary,
    /// The type-to-select prefix while a typing run is active, left of the hints.
    pub type_select: Option<SharedString>,
    pub error: Option<BarError>,
    pub count: Count,
    pub path: Path,
    /// A virtual destination's title (a saved search): the path slot shows this
    /// instead of a path, and it is not editable or draggable.
    pub virtual_path: Option<SharedString>,
    /// The breadcrumb style chosen in `settings.toml`.
    pub path_bar: PathBarMode,
    /// The path field while ⌘L edits it; the segments give way to it.
    pub editing: Option<Entity<TextInput>>,
    /// The most the transient slot may take, so the path keeps its first and last
    /// segments (set by `fit_to_window`).
    pub transient_max: Option<Pixels>,
    /// The path pill's width at rest, so the field can expand from it.
    pub pill_width: Pixels,
    /// The left section's width, the field's final width.
    pub field_width: Pixels,
    /// Bumped by every message; the transient fades in when it changes.
    pub message_id: u64,
    /// A finished operation's fraction while its bar fades, and its id.
    pub progress_done: Option<f32>,
    pub progress_id: u64,
    /// The first nonempty plugin Status value and its descriptor title.
    pub status: Option<SharedString>,
    pub status_spoken: Option<SharedString>,
}

/// The transient slot's one primary item.
pub enum Primary {
    /// Vim's mark pending: "Mark this folder: type a letter".
    Pending(&'static str),
    Running(RunningView),
    /// An operation or clipboard message.
    Message(SharedString),
    /// Keys for the focused element.
    Hints(Vec<Hint>),
}

pub struct RunningView {
    pub label: SharedString,
    pub detail: SharedString,
    pub fraction: f32,
    pub percent: u8,
    pub queued: usize,
    /// Planning has no totals yet; the bar shows a moving segment.
    pub indeterminate: bool,
}

impl RunningView {
    pub fn new(running: &Running) -> Self {
        let p = &running.progress;
        let (done, total) =
            if p.total_bytes > 0 { (p.done_bytes, p.total_bytes) } else { (p.done_items as u64, p.total_items as u64) };
        let fraction = if total == 0 { 0.0 } else { (done as f32 / total as f32).clamp(0.0, 1.0) };
        let percent = (fraction * 100.0).round() as u8;
        let mut detail = if running.planning {
            "Planning…".to_owned()
        } else if running.waiting {
            "Waiting for your choice".to_owned()
        } else if p.total_bytes > 0 {
            format!("{} · {percent} %", crow_fs::human_size_pair(p.done_bytes, p.total_bytes))
        } else if p.total_items == 0 {
            String::new()
        } else {
            format!("{} of {} items · {percent} %", p.done_items, p.total_items)
        };
        if running.pausing {
            detail = if detail.is_empty() { "Pausing…".to_owned() } else { format!("Pausing… · {detail}") };
        } else if running.paused {
            detail = if detail.is_empty() { "Paused".to_owned() } else { format!("Paused · {detail}") };
        }
        if running.queued == 1 {
            detail = if detail.is_empty() { "1 waiting".to_owned() } else { format!("{detail} · 1 waiting") };
        } else if running.queued > 1 {
            detail = if detail.is_empty() {
                format!("{} waiting", running.queued)
            } else {
                format!("{detail} · {} waiting", running.queued)
            };
        }
        Self {
            label: operations::running_label(&running.label).into(),
            detail: detail.into(),
            fraction,
            percent,
            queued: running.queued,
            indeterminate: running.planning,
        }
    }
}

pub struct Hint {
    pub keys: SharedString,
    pub spoken: SharedString,
    pub label: &'static str,
    pub idle: bool,
}

pub struct BarError {
    /// Short message, file name first and the config dir stripped.
    pub text: SharedString,
    /// The config file at fault, when Open can help.
    pub file: Option<PathBuf>,
}

pub struct Count {
    pub text: SharedString,
    pub spoken: SharedString,
}

/// The path slot's data: the segments and the spoken form.
pub struct Path {
    pub parts: Vec<(String, PathBuf)>,
    pub last_is_dir: bool,
    pub spoken: String,
    /// Segments folded into one "…" when the bar is short of room (never the first or last).
    pub hidden: Option<Range<usize>>,
}

fn items(n: usize) -> String {
    if n == 1 { "1 item".to_owned() } else { format!("{n} items") }
}

fn size_suffix(bytes: u64) -> String {
    if bytes == 0 { String::new() } else { format!(" · {}", human_size(bytes)) }
}

fn spoken_size(bytes: u64) -> String {
    if bytes == 0 { String::new() } else { format!(", {}", human_size(bytes)) }
}

/// Counts for the list or a column.
pub struct PaneCounts {
    pub loading: bool,
    pub total: usize,
    pub selected: usize,
    pub bytes: u64,
    pub folder: String,
}

pub fn pane_count(c: PaneCounts) -> Count {
    if c.loading && c.total == 0 {
        return Count { text: "Loading…".into(), spoken: format!("Loading {}", c.folder).into() };
    }
    // While filtering the field already says "n of N"; the bar keeps the folder's count.
    if c.selected >= 2 {
        return Count {
            text: format!("{} selected{}", c.selected, size_suffix(c.bytes)).into(),
            spoken: format!("{} of {} selected in {}{}", c.selected, c.total, c.folder, spoken_size(c.bytes)).into(),
        };
    }
    let text = items(c.total);
    if c.selected == 1 && c.bytes > 0 {
        let size = human_size(c.bytes);
        return Count {
            text: format!("{text} · {size}").into(),
            spoken: format!("{text} in {}, selected item {size}", c.folder).into(),
        };
    }
    Count { text: text.clone().into(), spoken: format!("{text} in {}", c.folder).into() }
}

pub fn search_count(results: usize, selected: usize, searching: bool, query_empty: bool) -> Count {
    if query_empty {
        return Count { text: "".into(), spoken: "".into() };
    }
    if searching && results == 0 {
        return Count { text: "Searching…".into(), spoken: "Searching".into() };
    }
    if selected > 0 {
        return Count {
            text: format!("{selected} of {results} selected").into(),
            spoken: format!("{selected} of {results} results selected").into(),
        };
    }
    let text = if results == 1 { "1 result".to_owned() } else { format!("{results} results") };
    Count { text: text.clone().into(), spoken: text.into() }
}

pub fn sidebar_count(group: &str, rows: usize) -> Count {
    Count { text: format!("{group} · {rows}").into(), spoken: format!("{group}, {}", items(rows)).into() }
}

/// Which element owns the keys, for choosing the hint set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarFocus {
    List,
    FilterField,
    ListFiltered,
    PathEdit,
    Sidebar,
    SearchResults,
    SearchField,
    Overlay,
}

#[derive(Clone, Copy)]
enum HintKind {
    Action(&'static dyn Action),
    /// Two bindings in one hint: the shared modifier prefix once, then both keys.
    Merged(&'static dyn Action, &'static dyn Action),
}

#[derive(Clone, Copy)]
struct Spec {
    kind: HintKind,
    label: &'static str,
    /// Idle hints retire after their key has been used.
    idle: bool,
}

const FILTER: Spec = Spec { kind: HintKind::Action(&StartFilter), label: "filter", idle: true };
// ⌘F and ⌘K already sit on the toolbar's Search and Go to buttons; only the filter
// has no other place on screen.
const LIST_HINTS: &[Spec] = &[FILTER];

// Return in the filter and search fields moves into the matches; Return there opens.
const TO_MATCHES: Spec = Spec { kind: HintKind::Action(&Confirm), label: "to matches", idle: false };
const CANCEL: Spec = Spec { kind: HintKind::Action(&text_input::Cancel), label: "clear", idle: false };
const SEARCH_DEEPER: Spec = Spec { kind: HintKind::Action(&ShowSearch), label: "search deeper", idle: false };
const FILTER_FIELD_HINTS: &[Spec] = &[TO_MATCHES, CANCEL, SEARCH_DEEPER];

const OPEN: Spec = Spec { kind: HintKind::Action(&OpenSelection), label: "open", idle: false };
const CLEAR: Spec = Spec { kind: HintKind::Action(&ClearFilter), label: "clear", idle: false };
const FILTERED_HINTS: &[Spec] = &[OPEN, CLEAR, SEARCH_DEEPER];

const GROUPS: Spec = Spec { kind: HintKind::Merged(&PreviousGroup, &NextGroup), label: "groups", idle: false };
const REORDER: Spec =
    Spec { kind: HintKind::Merged(&MoveFavoriteUp, &MoveFavoriteDown), label: "reorder", idle: false };
const REMOVE: Spec = Spec { kind: HintKind::Action(&RemoveFromSidebar), label: "remove", idle: false };
const EJECT: Spec = Spec { kind: HintKind::Action(&Eject), label: "eject", idle: false };
// Tab and → both leave the sidebar for the list; one hint shows both keys.
const TO_LIST: Spec = Spec { kind: HintKind::Merged(&SwitchFocus, &OpenSelection), label: "to list", idle: false };

const REVEAL: Spec = Spec { kind: HintKind::Action(&SearchReveal), label: "reveal", idle: false };
const QUICK_LOOK: Spec = Spec { kind: HintKind::Action(&QuickLook), label: "Quick Look", idle: false };
const RESULT_HINTS: &[Spec] = &[OPEN, REVEAL, QUICK_LOOK];

const TO_RESULTS: Spec = Spec { kind: HintKind::Action(&Confirm), label: "to results", idle: false };
const DONE: Spec = Spec { kind: HintKind::Action(&text_input::Cancel), label: "done", idle: false };
const FIELD_HINTS: &[Spec] = &[TO_RESULTS, DONE];

// The bar's path field (⌘L): Return goes, Tab completes, Escape puts the segments back.
const GO: Spec = Spec { kind: HintKind::Action(&Confirm), label: "go", idle: false };
const COMPLETE: Spec = Spec { kind: HintKind::Action(&Complete), label: "complete", idle: false };
const CANCEL_EDIT: Spec = Spec { kind: HintKind::Action(&text_input::Cancel), label: "cancel", idle: false };
const PATH_EDIT_HINTS: &[Spec] = &[GO, COMPLETE, CANCEL_EDIT];

/// The hint set for a focus. Sidebar hints follow the cursor row (Q6).
fn hint_set(focus: BarFocus, sidebar: Option<CursorKind>) -> Vec<Spec> {
    match focus {
        BarFocus::List => LIST_HINTS.to_vec(),
        BarFocus::FilterField => FILTER_FIELD_HINTS.to_vec(),
        BarFocus::ListFiltered => FILTERED_HINTS.to_vec(),
        BarFocus::PathEdit => PATH_EDIT_HINTS.to_vec(),
        BarFocus::Sidebar => match sidebar {
            Some(CursorKind::Favorite) => vec![GROUPS, TO_LIST, REORDER, REMOVE],
            Some(CursorKind::Recent) => vec![GROUPS, TO_LIST, REMOVE],
            Some(CursorKind::Volume) => vec![EJECT, GROUPS, TO_LIST],
            Some(CursorKind::Plugin) => vec![GROUPS, TO_LIST],
            None => vec![GROUPS, TO_LIST],
        },
        BarFocus::SearchResults => RESULT_HINTS.to_vec(),
        BarFocus::SearchField => FIELD_HINTS.to_vec(),
        BarFocus::Overlay => Vec::new(),
    }
}

/// The keys for two bindings in one cap: the shared modifier prefix once.
fn merged_keys(up: &KeyBinding, down: &KeyBinding) -> (SharedString, String) {
    let up_caps = keys::caps(up);
    let down_caps = keys::caps(down);
    let shared = up_caps.iter().zip(&down_caps).take_while(|(a, b)| a == b).count();
    let mut keys = up_caps[..shared].concat();
    keys.push_str(&up_caps[shared..].concat());
    keys.push_str(&down_caps[shared..].concat());

    let up_spoken = keys::spoken(up);
    let down_spoken = keys::spoken(down);
    let up_words: Vec<&str> = up_spoken.split(' ').collect();
    let down_words: Vec<&str> = down_spoken.split(' ').collect();
    let shared = up_words.iter().zip(&down_words).take_while(|(a, b)| a == b).count();
    let spoken = format!("{} {}", up_words.join(" "), down_words[shared..].join(" "));
    (keys.into(), spoken)
}

/// Gap between the bar's slots and between hints.
const GAP: f32 = 14.;

/// The path field's minimum width while editing: enough for a folder name and the
/// caret, while the hints keep their room.
const PATH_FIELD_MIN: f32 = 180.;

/// The Explorer Go button's extra width while editing: its 20 px target and the
/// field's 14 px gap. Crow has no button, so its field keeps the older minimum.
const PATH_GO_WIDTH: f32 = 34.;

/// How the bar shares its room this frame.
#[derive(Debug, PartialEq)]
pub struct Fit {
    /// Hints kept, from the left.
    pub hints: usize,
    /// Path segments folded into "…".
    pub hidden: Option<Range<usize>>,
    /// The most the transient slot may take.
    pub transient_max: Pixels,
}

/// Width of the path showing `segments[..1]`, "…" and `segments[from..]` (`from == 1`: everything).
fn path_width(segments: &[Pixels], ellipsis: Pixels, from: usize) -> Pixels {
    let first = segments.first().copied().unwrap_or_default();
    let rest: Pixels = segments[from.min(segments.len())..].iter().copied().sum();
    if from <= 1 { first + rest } else { first + ellipsis + rest }
}

/// Share the bar. The path always keeps its first segment and its last; then the
/// transient's own content (progress, messages, errors) gets room; hints come next
/// while the path still shows the parent of its last segment; the path takes the
/// rest, folding middle segments into "…" from the left.
///
/// `segments` are each path segment's width including its separator, `fixed` the
/// counts slot and the gaps around the transient, `other` the transient's content
/// besides hints, `hints` each hint's width.
pub fn fit(bar: Pixels, fixed: Pixels, segments: &[Pixels], ellipsis: Pixels, other: Pixels, hints: &[Pixels]) -> Fit {
    let n = segments.len();
    let full = path_width(segments, ellipsis, 1);
    // First and last; with the parent too for the hint reserve.
    let min_path = if n <= 2 { full } else { full.min(path_width(segments, ellipsis, n - 1)) };
    let reserve = if n <= 3 { full } else { full.min(path_width(segments, ellipsis, n - 2)) };

    let transient_max = (bar - fixed - min_path).max(px(0.));
    let other = other.min(transient_max);

    let mut room = bar - fixed - other - reserve;
    if other > px(0.) {
        room -= px(GAP);
    }
    let mut kept = 0;
    let mut used = px(0.);
    for (i, width) in hints.iter().enumerate() {
        let needed = if i == 0 { *width } else { *width + px(GAP) };
        if used + needed > room {
            break;
        }
        used += needed;
        kept += 1;
    }

    let path_room = bar - fixed - other - used;
    let from = (1..n.max(2)).find(|&from| path_width(segments, ellipsis, from) <= path_room).unwrap_or(n.max(2) - 1);
    let hidden = (from > 1).then_some(1..from);
    Fit { hints: kept, hidden, transient_max }
}

/// Fit the bar to its room (the window minus `left`, the sidebar column beside it): measure with the bar's 12 px text, then drop hints,
/// fold the path and cap the transient slot.
pub fn fit_to_window(state: &mut BarState, window: &Window, left: Pixels) {
    let font = window.text_style().font();
    let measure = |text: &str| {
        let text = crate::style::display_text(text);
        let run = TextRun {
            len: text.len(),
            font: font.clone(),
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window.text_system().shape_line(text.into_owned().into(), px(12.), &[run], None).width
    };
    let cap = |keys: &str| (measure(keys) + px(12.)).max(px(18.));
    // The sibling separator: 6 px on both sides and "›".
    let separator = px(12.) + measure("›");
    let last = state.path.parts.len().saturating_sub(1);
    // The pill's padding and border on both sides (on the first segment), the
    // segment's 6 + 4 px padding, the icon (18 px: the wider of a 14 px Finder icon
    // and an outline glyph), its 4 px gap, the name, the separator, and the pencil
    // (20 px + its 4 px margin) on the last segment.
    let segments: Vec<Pixels> = state
        .path
        .parts
        .iter()
        .enumerate()
        .map(|(i, (name, _))| {
            px(10. + 18. + 4.)
                + measure(name)
                + if i < last { separator } else { px(24.) }
                + if i == 0 { px(PILL_INSET * 2.) } else { px(0.) }
        })
        .collect();
    // The "…" segment: its px_1, the "…", the separator's 12 px and "›".
    let ellipsis = px(8.) + measure("…") + px(12.) + measure("›");

    let bar = window.viewport_size().width - left - px(24.);
    let fixed = measure(&state.count.text) + px(12. + 1.) + px(2. * GAP);
    let mut other = match &state.primary {
        Primary::Pending(text) => measure(text),
        Primary::Message(text) => measure(text),
        Primary::Running(running) => {
            measure(&running.label)
                + px(10.)
                + if running.detail.is_empty() { px(0.) } else { measure(&running.detail) + px(10.) }
                + px(14. + 4. + 10. + 10.)
                + px(12. + 6.)
                + measure("Cancel")
                + cap("esc")
        }
        Primary::Hints(_) => px(0.),
    };
    if let Some(query) = &state.type_select {
        if other > px(0.) {
            other += px(GAP);
        }
        other += measure(query);
    }
    let mut error_controls = px(0.);
    if let Some(error) = &state.error {
        if other > px(0.) {
            other += px(GAP);
        }
        let open = if error.file.is_some() { measure("Open") + px(12. + 8.) } else { px(0.) };
        error_controls = px(8. + 13. + 8. + 4.) + open;
        other += error_controls + measure(&error.text).min(px(420.));
    }
    let hint_widths: Vec<Pixels> = match &state.primary {
        Primary::Hints(hints) => hints.iter().map(|hint| cap(&hint.keys) + px(5.) + measure(hint.label)).collect(),
        _ => Vec::new(),
    };

    // The path's own fit gives the pill's width at rest even while the field replaces it.
    let path_fit = fit(bar, fixed, &segments, ellipsis, other, &hint_widths);
    state.pill_width = match &path_fit.hidden {
        Some(hidden) => path_width(&segments, ellipsis, hidden.end),
        None => segments.iter().copied().sum(),
    };
    // The field spans the left section: the bar less the counts and one gap.
    state.field_width = (bar - fixed + px(GAP)).max(px(0.));
    // The field takes the left section: the input's minimum plus the hints inside
    // it, the Explorer Go button, and the field's 8 px padding on both sides and
    // 14 px gaps.
    let field_min = px(PATH_FIELD_MIN + 30.)
        + if state.path_bar == PathBarMode::Explorer { px(PATH_GO_WIDTH) } else { px(0.) }
        + hint_widths.iter().copied().sum::<Pixels>();
    let base_fit =
        if state.editing.is_some() { fit(bar, fixed, &[field_min], ellipsis, other, &hint_widths) } else { path_fit };
    // The status slot is the first to yield: it only appears when it costs no
    // hint and no path segment.
    let fit = if let Some(text) = &state.status {
        let with_status = other + measure(text) + px(GAP);
        let status_fit = if state.editing.is_some() {
            fit(bar, fixed, &[field_min], ellipsis, with_status, &hint_widths)
        } else {
            fit(bar, fixed, &segments, ellipsis, with_status, &hint_widths)
        };
        if status_fit.hints < base_fit.hints || status_fit.hidden != base_fit.hidden {
            state.status = None;
            state.status_spoken = None;
            base_fit
        } else {
            status_fit
        }
    } else {
        base_fit
    };
    if let Primary::Hints(hints) = &mut state.primary {
        hints.truncate(fit.hints);
    }
    state.path.hidden = if state.editing.is_some() { None } else { fit.hidden };
    // Let the breadcrumb clip further before clipping the error's Open button.
    state.transient_max = Some(fit.transient_max.max(error_controls.min((bar - fixed).max(px(0.)))));
}

/// The bar's hints in `scope`, with idle ones filtered by the setting and use counts.
pub fn hints(focus: BarFocus, scope: KeyScope, sidebar: Option<CursorKind>, cx: &App) -> Vec<Hint> {
    let mode = cx.global::<Ui>().settings.hints;
    if mode == HintsMode::Off {
        return Vec::new();
    }
    let mut out = Vec::new();
    for spec in hint_set(focus, sidebar) {
        match spec.kind {
            HintKind::Action(action) => {
                if spec.idle && !idle_hint_visible(mode, usage::uses(action, cx)) {
                    continue;
                }
                let Some(binding) = keys::binding_for(action, scope, cx) else { continue };
                out.push(Hint {
                    keys: keys::chord_text(&binding),
                    spoken: keys::spoken(&binding).into(),
                    label: spec.label,
                    idle: spec.idle,
                });
            }
            HintKind::Merged(first_action, second_action) => {
                match (keys::binding_for(first_action, scope, cx), keys::binding_for(second_action, scope, cx)) {
                    (Some(first), Some(second)) => {
                        let (keys, spoken) = merged_keys(&first, &second);
                        out.push(Hint { keys, spoken: spoken.into(), label: spec.label, idle: spec.idle });
                    }
                    // One key is still a usable hint if the other is unbound.
                    (Some(only), None) | (None, Some(only)) => out.push(Hint {
                        keys: keys::chord_text(&only),
                        spoken: keys::spoken(&only).into(),
                        label: spec.label,
                        idle: spec.idle,
                    }),
                    (None, None) => {}
                }
            }
        }
    }
    out
}

/// The error pill: icon, short message, and Open for a config file.
fn error_token(error: &BarError, theme: &crow_theme::Theme) -> AnyElement {
    let text = error.text.clone();
    let mut pill = div()
        .id("bar-error")
        .debug_selector(|| "bar-error".into())
        .role(Role::Group)
        .aria_label(format!("Error: {text}"))
        .h(px(20.))
        // The message gives way; the icon and Open never do.
        .min_w_0()
        .rounded(px(10.))
        .pl(px(8.))
        .pr(px(4.))
        .flex()
        .items_center()
        .gap(px(8.))
        .bg(error_soft(theme))
        .text_color(color(theme.role("error")))
        .child(icons::glyph(Glyph::AlertCircle, 13., "error", theme).flex_none())
        .child(
            div().min_w_0().max_w(px(420.)).overflow_hidden().whitespace_nowrap().text_ellipsis().child(text.clone()),
        );
    if let Some(path) = &error.file {
        let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        let path = path.clone();
        pill = pill.child(
            div()
                .id("bar-error-open")
                .debug_selector(|| "bar-error-open".into())
                .role(Role::Button)
                .aria_label(format!("Open {name}"))
                .h(px(16.))
                .flex_none()
                .rounded(px(8.))
                .px(px(6.))
                .flex()
                .items_center()
                .bg(color(theme.role("error")))
                .text_color(color(theme.role("bg")))
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .on_click(move |_, _, cx| open_config(path.clone(), cx))
                .child("Open"),
        );
    }
    with_tooltip(pill, text, None).into_any_element()
}

/// Open a config file at fault: its default app, then the text editor (Q10).
pub fn open_config(path: PathBuf, cx: &mut App) {
    let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
    let effects = crate::effects::effects(cx);
    let task = cx.background_spawn(async move { effects.open_config(&path) });
    cx.spawn(async move |cx| {
        if let Err(e) = task.await {
            cx.update(|cx| {
                operations::message(format!("Can't open {name}: {e}"), true, cx);
                cx.refresh_windows();
            });
        }
    })
    .detach();
}

/// Cancel for the running operation: "Cancel esc", mouse target only (Esc works anywhere).
fn cancel_button(cx: &App) -> gpui::Stateful<gpui::Div> {
    let theme = cx.global::<Ui>().theme.clone();
    let keys = keys::text(&CancelOperation, KeyScope::Window, cx);
    let spoken = keys::spoken_for(&CancelOperation, KeyScope::Window, cx).unwrap_or_else(|| "Escape".into());
    let hover = color_alpha(theme.role("fg"), 0x14);
    let mut button = div()
        .id("bar-cancel")
        .debug_selector(|| "bar-cancel".into())
        .role(Role::Button)
        .aria_label(format!("Cancel operation, {spoken}"))
        .h(px(20.))
        .px(px(6.))
        .rounded(px(5.))
        .flex()
        .items_center()
        .gap(px(6.))
        .bg(color(theme.role("surface-raised")))
        .text_color(color(theme.role("fg")))
        .hover(move |style| style.bg(hover))
        .on_click(|_, _, cx| operations::cancel_active(cx))
        .child("Cancel");
    if let Some(keys) = keys.clone() {
        button = button.child(Keycap::text(keys, KeycapWeight::Text));
    }
    with_tooltip(button, "Cancel operation", keys)
}

/// Opens Activity from the status bar; the queued count rides beside the icon.
fn activity_button(queued: usize, cx: &App) -> gpui::Stateful<gpui::Div> {
    let theme = cx.global::<Ui>().theme.clone();
    let hover = color_alpha(theme.role("fg"), 0x14);
    let hover_theme = theme.clone();
    let label = if queued == 0 { "Show Activity".to_owned() } else { format!("Show Activity, {queued} waiting") };
    let button = div()
        .id("bar-activity")
        .debug_selector(|| "bar-activity".into())
        .role(Role::Button)
        .aria_label(label.clone())
        .h(px(20.))
        .px(px(5.))
        .rounded(px(5.))
        .flex()
        .items_center()
        .gap(px(4.))
        .text_color(color(theme.role("fg-muted")))
        .hover(move |style| style.bg(hover).text_color(color(hover_theme.role("fg"))))
        .on_click(|_, window, cx| window.dispatch_action(Box::new(crate::actions::ToggleActivity), cx))
        .child(icons::glyph(Glyph::List, 14., "fg-muted", &theme))
        // A fixed slot: the queue count must not move Cancel as it changes.
        .child(div().w(px(10.)).flex().justify_center().text_size(px(10.)).child(if queued == 0 {
            String::new()
        } else {
            queued.to_string()
        }));
    with_tooltip(button, label.clone(), None)
}

/// Text that gives way with an ellipsis when the transient slot is squeezed.
fn shrinking_text(text: impl Into<SharedString>) -> gpui::Div {
    div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(text.into())
}

/// The "…" standing for folded path segments: goes to the deepest of them; the
/// tooltip names them all.
fn folded_segment(
    parts: &[(String, PathBuf)],
    pane: &Entity<Pane>,
    path_menu: &PathMenu,
    hover: gpui::Rgba,
) -> impl IntoElement {
    let names = parts.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>().join(" › ");
    let (deepest, target) = parts.last().cloned().unwrap_or_default();
    let pane = pane.clone();
    // The "…" drags the deepest folder it stands for, the same one its click goes to.
    let drag_icon = icon(&target, true, 22);
    let drag_label: SharedString = deepest.clone().into();
    let drag_entries = Arc::new(vec![(target.clone(), true)]);
    let handoff = DragHandoff::new();
    let part = div()
        .id("path-folded")
        .debug_selector(|| "path-folded".into())
        .role(Role::Button)
        .aria_label(format!("Go to {deepest}, hidden: {names}"))
        .flex()
        .flex_none()
        .items_center()
        .px_1()
        .h(px(SEGMENT_HEIGHT))
        .rounded(px(SEGMENT_RADIUS))
        .hover(move |s| s.bg(hover))
        .child("…")
        .on_drag(DraggedSelection { entries: drag_entries }, {
            let handoff = handoff.clone();
            move |_value, offset, window, cx| drag_combo(drag_icon.clone(), &drag_label, offset, &handoff, window, cx)
        })
        .external_drag_payload::<DraggedSelection>(move |selection, window, cx| {
            promote_drag(selection, &handoff, window, cx)
        })
        .on_mouse_down(gpui::MouseButton::Right, {
            let (target, menu) = (target.clone(), path_menu.clone());
            move |_, window, cx| {
                cx.stop_propagation();
                menu(target.clone(), true, window, cx);
            }
        })
        .on_click({
            let menu = path_menu.clone();
            move |event, window, cx| {
                // The "…" is not empty path space: a double-click must not edit the path.
                cx.stop_propagation();
                if event.modifiers().control {
                    menu(target.clone(), true, window, cx);
                    return;
                }
                let target = target.clone();
                pane.update(cx, |pane, cx| pane.navigate(target, cx));
            }
        });
    with_tooltip(part, names, None)
}

/// The pill and pencil settle in about 120 ms, critically damped.
fn path_spring() -> SpringConfig {
    SpringConfig::new(5000., 141.42, 1.)
}

/// The path area's hover, so the pill fades in and out.
fn hover_path(hovered: &bool, window: &mut Window, cx: &mut App) {
    if *hovered {
        window.dispatch_action(Box::new(PathHover), cx);
    } else {
        window.dispatch_action(Box::new(PathUnhover), cx);
    }
}

/// One bar: path · transient · counts.
pub fn render(
    state: BarState,
    pane: Entity<Pane>,
    path_menu: PathMenu,
    hovered: bool,
    path_hits: &PathHits,
    drop_hover: Option<&std::path::Path>,
    cx: &mut App,
) -> impl IntoElement + use<> {
    path_hits.borrow_mut().clear();
    // The segment under a drag gets the same outline as a folder row.
    let drop = drop_hover.map(|path| (path.to_path_buf(), crate::drag_drop::cue_opacity(cx, path)));
    let drop_on = |target: &PathBuf| drop.as_ref().filter(|(path, _)| path == target).map(|(_, opacity)| *opacity);
    let ui = cx.global::<Ui>();
    let theme = ui.theme.clone();
    let border = crate::style::hairline(&theme);
    let fg = color(theme.role("fg"));
    let muted = color(theme.role("fg-muted"));
    let hover = color(theme.role("surface-raised"));
    let last = state.path.parts.len().saturating_sub(1);
    let parts = state.path.parts;
    let virtual_path = state.virtual_path.clone();
    let path_is_virtual = virtual_path.is_some();
    let explorer = state.path_bar == PathBarMode::Explorer;
    let fg_path = fg;
    let last_is_dir = state.path.last_is_dir;
    let icon_style = ui.settings.icon_style;
    let home = ui.home.clone();

    let path = {
        let hidden = state.path.hidden.clone().unwrap_or(0..0);
        let subtle = color(theme.role("fg-subtle"));
        let separator = || div().px(px(6.)).flex_none().text_color(subtle).child("›");
        let mut children: Vec<AnyElement> = Vec::new();
        for (i, (name, target)) in parts.iter().enumerate() {
            if hidden.contains(&i) {
                if i == hidden.start {
                    let folded = &parts[hidden.clone()];
                    let deepest = folded.last().map(|(_, path)| path.clone()).unwrap_or_default();
                    let outline = drop_on(&deepest).map(|opacity| (&theme, opacity));
                    children.push(register_path_hit(
                        path_hits,
                        deepest,
                        true,
                        false,
                        folded_segment(folded, &pane, &path_menu, hover).into_any_element(),
                        outline,
                    ));
                    children.push(separator().into_any_element());
                }
                continue;
            }
            let pane = pane.clone();
            let is_dir = i < last || last_is_dir;
            let is_package = is_dir && crow_fs::is_package_path(target);
            let source = crate::icon_packs::effective_icon_source(cx.global::<Ui>(), target);
            let icons = crate::icon_packs::Resolver::for_source(cx, &source);
            let spoke = if is_dir { format!("Go to {name}") } else { format!("Open {name}") };
            let part = div()
                .id(("path-segment", i))
                .debug_selector(move || format!("path-segment-{i}"))
                .role(Role::Button)
                .aria_label(spoke.clone())
                .flex()
                .items_center()
                .gap_1()
                // Two points more on the left: the icon sits off the highlight's edge.
                .pl(px(6.))
                .pr(px(4.))
                .h(px(SEGMENT_HEIGHT))
                .rounded(px(SEGMENT_RADIUS))
                // Only the last segment gives way (with an ellipsis) once nothing else can fold.
                .map(|part| if i == last { part.min_w_0().text_color(fg_path) } else { part.flex_none() })
                .hover(move |s| s.bg(hover))
                .child(crate::preview::styled_path_icon(
                    &icons, target, is_dir, is_package, icon_style, &home, &theme, 14.,
                ))
                .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(name.clone()));
            // A segment drags the entry it stands for: the folder, or the file the
            // last segment shows. The click below still navigates; GPUI clears the
            // click once a drag starts, so a drag never also navigates.
            let drag_icon = icon(target, is_dir, 22);
            let drag_label: SharedString = name.clone().into();
            let drag_entries = Arc::new(vec![(target.clone(), is_dir)]);
            let handoff = DragHandoff::new();
            let part = part
                .on_drag(DraggedSelection { entries: drag_entries }, {
                    let handoff = handoff.clone();
                    move |_value, offset, window, cx| {
                        drag_combo(drag_icon.clone(), &drag_label, offset, &handoff, window, cx)
                    }
                })
                .external_drag_payload::<DraggedSelection>(move |selection, window, cx| {
                    promote_drag(selection, &handoff, window, cx)
                });
            let hit_target = target.clone();
            let target = target.clone();
            let menu_target = target.clone();
            let part = part.on_mouse_down(gpui::MouseButton::Right, {
                let (target, menu) = (target.clone(), path_menu.clone());
                move |_, window, cx| {
                    cx.stop_propagation();
                    menu(target.clone(), is_dir, window, cx);
                }
            });
            let menu = path_menu.clone();
            let part = part.on_click(move |event, window, cx| {
                // A segment is not empty space: a double-click must not edit the path.
                cx.stop_propagation();
                if event.modifiers().control {
                    menu(menu_target.clone(), is_dir, window, cx);
                    return;
                }
                let target = target.clone();
                pane.update(cx, |pane, cx| {
                    if is_dir {
                        // Clicking where you already are changes nothing.
                        if pane.current_dir(cx) != target {
                            pane.navigate(target.clone(), cx);
                        }
                    } else {
                        pane.open_file(target.clone(), cx);
                    }
                });
            });
            let outline = drop_on(&hit_target).map(|opacity| (&theme, opacity));
            children.push(register_path_hit(
                path_hits,
                hit_target,
                is_dir,
                i == last,
                with_tooltip(part, spoke, None).into_any_element(),
                outline,
            ));
            if i < last {
                children.push(separator().into_any_element());
            }
        }
        // The pencil sits at the end of the path and is always visible. On hover the
        // pill around the path fades in (instant for now); a click on the pill's
        // padding or the empty space edits the path.
        let pencil = with_action_tooltip(
            div()
                .id("path-edit")
                .debug_selector(|| "path-edit".into())
                .role(Role::Button)
                .aria_label("Edit path")
                .aria_keyshortcuts("Command+L")
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .ml_1()
                .size(px(SEGMENT_HEIGHT))
                .rounded(px(SEGMENT_RADIUS))
                .hover(move |s| s.bg(hover))
                .on_click(|_, window, cx| {
                    cx.stop_propagation();
                    window.dispatch_action(Box::new(EditPath), cx);
                })
                .child({
                    let pencil_theme = theme.clone();
                    let pencil = icons::glyph(Glyph::Pencil, 14., "fg-subtle", &theme);
                    if ui.feedback.mode == Effective::Off {
                        pencil.into_any_element()
                    } else {
                        let rest = color(pencil_theme.role("fg-subtle"));
                        let hot = color(pencil_theme.role("fg"));
                        pencil
                            .with_spring(
                                "path-pencil",
                                SpringAnimation::new(path_spring()).to(hovered),
                                move |el, phase| {
                                    let t = phase.0.clamp(0., 1.);
                                    el.text_color(rest.blend(hot.alpha(t)))
                                },
                            )
                            .into_any_element()
                    }
                }),
            "Edit path",
            &EditPath,
            KeyScope::Window,
            cx,
        );
        children.push(pencil.into_any_element());
        if let Some(label) = virtual_path.clone() {
            children.clear();
            children.push(
                div()
                    .id("saved-path")
                    .debug_selector(|| "saved-path".into())
                    .role(Role::Label)
                    .aria_label(format!("Saved search: {label}"))
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_1()
                    .h(px(20.))
                    .text_color(fg_path)
                    .child(icons::glyph(crate::icons::Glyph::Search, 14., "fg-muted", &theme))
                    .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(label))
                    .into_any_element(),
            );
        }
        let pill_theme = theme.clone();
        let pill = div()
            .id("path-pill")
            .debug_selector(|| "path-pill".into())
            .flex_none()
            .h(px(PILL_HEIGHT))
            .flex()
            .items_center()
            .px(px(PILL_PAD))
            .rounded(px(PILL_RADIUS))
            .border_1()
            .border_color(transparent_black())
            .bg(transparent_black())
            .children(children);
        // Explorer's breadcrumb is a field at rest: the same surface and border the
        // hover pill uses, at full alpha. A virtual destination is not editable, so
        // it keeps the plain label. Crow fades the pill in only on hover.
        let outlined = explorer && !path_is_virtual;
        let rest_alpha = if outlined { 0xff } else { 0 };
        let pill = if ui.feedback.mode == Effective::Off {
            if outlined {
                pill.bg(color(pill_theme.role("surface")))
                    .border_color(color(pill_theme.role("border")))
                    .into_any_element()
            } else {
                pill.into_any_element()
            }
        } else {
            pill.with_spring("path-pill", SpringAnimation::new(path_spring()).to(hovered), move |el, phase| {
                let hover_alpha = (phase.0.clamp(0., 1.) * 255.) as u8;
                let alpha = rest_alpha.max(hover_alpha);
                el.bg(color_alpha(pill_theme.role("surface"), alpha))
                    .border_color(color_alpha(pill_theme.role("border"), alpha))
            })
            .into_any_element()
        };
        let empty = div()
            .id("path-empty")
            .debug_selector(|| "path-empty".into())
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .items_center()
            .overflow_hidden();
        div()
            .id("bar-path")
            .debug_selector(|| "bar-path".into())
            .role(Role::Navigation)
            .aria_label(format!("Path: {}", state.path.spoken))
            .flex_1()
            .min_w_0()
            .h(px(32.))
            .overflow_hidden()
            .flex()
            .items_center()
            .child(pill)
            .child(empty)
            // A click on the pill's padding or the empty space edits the path; ⌘L
            // and the pencil do the same. Segments stop propagation.
            .on_click(move |_, window, cx| {
                if !path_is_virtual {
                    window.dispatch_action(Box::new(EditPath), cx);
                }
            })
            .on_hover(hover_path)
    };

    let idle_text = color(theme.role("fg-subtle"));
    let type_select_el = state.type_select.as_ref().map(|query| {
        div()
            .id("bar-type-select")
            .debug_selector(|| "bar-type-select".into())
            .role(Role::Group)
            .aria_label(format!("Type to select: {query}"))
            .flex_none()
            .text_color(fg)
            .child(query.clone())
            .into_any_element()
    });
    let (primary_aria, primary_el) = match &state.primary {
        Primary::Hints(hints) if hints.is_empty() => (None, None),
        Primary::Hints(hints) => {
            let aria = format!(
                "Keys: {}",
                hints.iter().map(|hint| format!("{} {}", hint.spoken, hint.label)).collect::<Vec<_>>().join(", ")
            );
            let el = div()
                .flex()
                .items_center()
                .gap(px(14.))
                .children(hints.iter().enumerate().map(|(i, hint)| {
                    let key_role = if hint.idle { "fg-subtle" } else { "fg" };
                    div()
                        .id(("bar-hint", i))
                        .debug_selector(move || format!("bar-hint-{i}"))
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .text_color(if hint.idle { idle_text } else { muted })
                        .child(Keycap::text(hint.keys.clone(), KeycapWeight::Outline).text_role(key_role))
                        .child(hint.label)
                }))
                .into_any_element();
            (Some(SharedString::from(aria)), Some(el))
        }
        Primary::Pending(text) => {
            (Some(SharedString::from(*text)), Some(shrinking_text(*text).text_color(fg).into_any_element()))
        }
        Primary::Message(text) => {
            (Some(text.clone()), Some(shrinking_text(text.clone()).text_color(muted).into_any_element()))
        }
        Primary::Running(running) => {
            let label = running.label.clone();
            let detail = running.detail.clone();
            let percent = running.percent;
            let mut group = div()
                .id("bar-progress-text")
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(10.))
                .role(Role::ProgressIndicator)
                .aria_label(label.clone())
                .aria_numeric_value(f64::from(percent))
                .aria_min_numeric_value(0.)
                .aria_max_numeric_value(100.)
                .child(shrinking_text(label.clone()).text_color(fg))
                .children((!detail.is_empty()).then(|| shrinking_text(detail).text_color(muted)));
            group = group.child(activity_button(running.queued, cx).flex_none());
            group = group.child(cancel_button(cx).flex_none());
            (Some(label), Some(group.into_any_element()))
        }
    };
    let error_el = state.error.as_ref().map(|error| error_token(error, &theme));
    // Hints change with every focus move, so they are a plain group; only progress,
    // messages and errors live in the status region VoiceOver announces.
    let role = if matches!(state.primary, Primary::Hints(_)) { Role::Group } else { Role::Status };
    let transient = div()
        .id("bar-transient")
        .debug_selector(|| "bar-transient".into())
        .role(role)
        .aria_label(primary_aria.unwrap_or_default())
        .flex_none()
        .min_w_0()
        .when_some(state.transient_max, |el, max| el.flex_shrink_1().max_w(max))
        .overflow_hidden()
        .flex()
        .items_center()
        .gap(px(14.))
        .children(type_select_el)
        .children(primary_el)
        .children(error_el);
    // A new message or error fades in; hints appear at once (they follow focus).
    let error_key = state.error.as_ref().map(|error| error.text.clone()).unwrap_or_default();
    let transient = if (state.error.is_some() || !matches!(state.primary, Primary::Hints(_)))
        && (state.message_id > 0 || state.error.is_some())
        && ui.feedback.mode != Effective::Off
    {
        let key = SharedString::from(format!("bar-message-{}-{error_key}", state.message_id));
        transient
            .with_animation(key, Animation::new(Duration::from_millis(motion::ms::STATUS)), |el, t| {
                el.opacity(0.3 + 0.7 * motion::ease_out(t))
            })
            .into_any_element()
    } else {
        transient.into_any_element()
    };

    let counts_spoken = state.count.spoken.clone();
    let counts = (!state.count.text.is_empty()).then(|| {
        with_tooltip(
            div()
                .id("bar-counts")
                .role(Role::Status)
                .aria_label(counts_spoken.clone())
                .flex_none()
                .pl_3()
                .border_l_1()
                .border_color(border)
                .text_color(fg)
                .child(state.count.text.clone()),
            counts_spoken,
            None,
        )
    });

    let progress: Option<AnyElement> = match &state.primary {
        Primary::Running(running) if running.indeterminate => {
            let segment = div()
                .id("bar-progress")
                .debug_selector(|| "bar-progress".into())
                .absolute()
                .top_0()
                .left_0()
                .h(px(2.))
                .w(relative(0.3))
                .bg(color(theme.role("accent")));
            Some(
                segment
                    .with_animation(
                        "bar-progress-planning",
                        Animation::new(Duration::from_millis(900)).repeat_synced(),
                        |el, t| el.left(relative(t * 0.7)),
                    )
                    .into_any_element(),
            )
        }
        Primary::Running(running) => {
            let bar = div()
                .id("bar-progress")
                .debug_selector(|| "bar-progress".into())
                .absolute()
                .top_0()
                .left_0()
                .h(px(2.))
                .bg(color(theme.role("accent")));
            // Full eases toward each new value (a critically damped spring); Low and Off step.
            Some(if ui.feedback.mode == Effective::Full {
                bar.with_spring(
                    "bar-progress-fill",
                    crate::motion_ui::ease_progress(running.fraction),
                    |el, fraction| el.w(relative(fraction)),
                )
                .into_any_element()
            } else {
                bar.w(relative(running.fraction)).into_any_element()
            })
        }
        _ => state.progress_done.and_then(|fraction| {
            if ui.feedback.mode == Effective::Off {
                return None;
            }
            let bar = div()
                .id("bar-progress")
                .debug_selector(|| "bar-progress".into())
                .absolute()
                .top_0()
                .left_0()
                .h(px(2.))
                .w(relative(fraction))
                .bg(color(theme.role("accent")));
            Some(
                bar.with_animation(
                    ("bar-progress-done", state.progress_id),
                    Animation::new(Duration::from_millis(motion::ms::COPY_FADE)),
                    |el, t| el.opacity(1.0 - motion::ease_in(t)),
                )
                .into_any_element(),
            )
        }),
    };

    // The left section: the path slot at rest, or one field across the path and
    // the transient (up to the counts' divider) while editing.
    let mut bar_children: Vec<AnyElement> = Vec::new();
    if let Some(input) = state.editing.clone() {
        // The hints fade in with the field instead of popping in at full opacity.
        let field_hints = if ui.feedback.mode == Effective::Off {
            transient
        } else {
            div()
                .flex_none()
                .min_w_0()
                .child(transient)
                .with_animation(
                    "bar-path-field-hints",
                    Animation::new(Duration::from_millis(motion::ms::FIELD)),
                    |el, t| el.opacity(motion::ease_out(t)),
                )
                .into_any_element()
        };
        // Explorer's field carries a Go chevron; it dispatches the same confirmation
        // as Return, so both share the background check and refusal path. Crow's
        // field keeps its keys-only affordance.
        let go = explorer.then(|| {
            let go_theme = theme.clone();
            with_tooltip(
                div()
                    .id("path-go")
                    .debug_selector(|| "path-go".into())
                    .role(Role::Button)
                    .aria_label("Go")
                    .aria_keyshortcuts("Return")
                    .flex_none()
                    .size(px(20.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(move |style| style.bg(hover))
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        window.dispatch_action(Box::new(crate::actions::PathEditGo), cx);
                    })
                    .child(icons::glyph(Glyph::ChevronRight, 14., "fg", &go_theme)),
                "Go",
                None,
            )
        });
        let field = div()
            .id("bar-path-box")
            .flex_1()
            .min_w_0()
            .h(px(PILL_HEIGHT))
            .overflow_hidden()
            .flex()
            .items_center()
            .gap(px(14.))
            .px(px(8.))
            .rounded(px(PILL_RADIUS))
            .border_2()
            .border_color(crate::style::field_ring(&theme))
            .bg(color(theme.role("surface")))
            .child(
                div()
                    .id("bar-path-field")
                    .debug_selector(|| "bar-path-field".into())
                    .role(Role::Group)
                    .aria_label("Edit path")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_center()
                    .child(input),
            )
            .children(go)
            .child(field_hints);
        let field = if ui.feedback.mode == Effective::Off {
            field.into_any_element()
        } else {
            let start = state.pill_width.as_f32();
            let end = state.field_width.as_f32();
            field
                .with_animation(
                    "bar-path-field",
                    Animation::new(Duration::from_millis(motion::ms::FIELD)),
                    move |el, t| {
                        let t = motion::ease_out(t);
                        el.max_w(px(start + (end - start) * t))
                    },
                )
                .into_any_element()
        };
        // The slot keeps the resting path's 32 px; the field is the pill's height inside it.
        let slot = div()
            .id("bar-path")
            .debug_selector(|| "bar-path".into())
            .role(Role::Navigation)
            .aria_label(format!("Path: {}", state.path.spoken))
            .flex_1()
            .min_w_0()
            .h(px(32.))
            .flex()
            .items_center()
            .child(field);
        bar_children.push(slot.into_any_element());
    } else {
        bar_children.push(path.into_any_element());
        if let Some(text) = state.status.clone() {
            bar_children.push(
                div()
                    .id("bar-plugin-status")
                    .debug_selector(|| "bar-plugin-status".into())
                    .role(Role::Status)
                    .aria_label(state.status_spoken.clone().unwrap_or_else(|| text.clone()))
                    .flex_none()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(muted)
                    .child(text)
                    .into_any_element(),
            );
        }
        bar_children.push(transient);
    }
    bar_children.extend(counts.map(IntoElement::into_any_element));

    div()
        .id("path-bar")
        .debug_selector(|| "path-bar".into())
        .role(Role::Group)
        .aria_label("Status bar")
        .relative()
        .h(px(HEIGHT))
        .flex_none()
        .px_3()
        .flex()
        .items_center()
        .gap(px(14.))
        .border_t_1()
        .border_color(border)
        .text_size(px(12.))
        .text_color(muted)
        .children(bar_children)
        .children(progress)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crow_fs::ops::Progress;

    fn running(label: &str, progress: Progress) -> Running {
        Running {
            label: label.to_owned(),
            progress,
            queued: 0,
            planning: false,
            pausing: false,
            paused: false,
            waiting: false,
        }
    }

    #[test]
    fn fit_shares_the_bar() {
        let segments = [px(100.); 5];
        let fit5 =
            |bar: f32, other: f32, hints: &[Pixels]| fit(px(bar), px(100.), &segments, px(30.), px(other), hints);

        // Room for everything: nothing folds.
        assert_eq!(fit5(1000., 0., &[px(80.)]), Fit { hints: 1, hidden: None, transient_max: px(670.) });
        // Hints stay while the path still shows first … parent › last; the middle folds.
        assert_eq!(fit5(600., 0., &[px(80.)]).hints, 1);
        assert_eq!(fit5(600., 0., &[px(80.)]).hidden, Some(1..3));
        // Narrower: hints go before the parent does.
        assert_eq!(fit5(450., 0., &[px(80.)]), Fit { hints: 0, hidden: Some(1..3), transient_max: px(120.) });
        // A long error or progress line is capped so first and last stay.
        assert_eq!(fit5(600., 500., &[]), Fit { hints: 0, hidden: Some(1..4), transient_max: px(270.) });
        // Hints drop from the right.
        assert_eq!(fit5(1000., 0., &[px(300.), px(300.)]).hints, 1);
        // Two segments never fold.
        assert_eq!(fit(px(100.), px(50.), &[px(100.), px(100.)], px(30.), px(0.), &[]).hidden, None);
    }

    #[test]
    fn search_count_hides_in_idle() {
        assert_eq!(search_count(0, 0, false, true).text.as_ref(), "");
        assert_eq!(search_count(0, 0, false, true).spoken.as_ref(), "");
        assert_eq!(search_count(0, 0, false, false).text.as_ref(), "0 results");
        assert_eq!(search_count(0, 0, true, false).text.as_ref(), "Searching…");
        assert_eq!(search_count(2, 1, false, false).text.as_ref(), "1 of 2 selected");
    }

    #[test]
    fn trash_progress_counts_items() {
        let view = RunningView::new(&running(
            "Move 2 items to Trash",
            Progress { done_items: 1, total_items: 2, total_bytes: 0, ..Default::default() },
        ));
        assert_eq!(view.label.as_ref(), "Moving 2 items to Trash");
        assert_eq!(view.detail.as_ref(), "1 of 2 items · 50 %");
        assert_eq!(view.percent, 50);
        assert_eq!(view.fraction, 0.5);
    }

    #[test]
    fn bytes_progress_uses_one_unit() {
        let view = RunningView::new(&running(
            "Copy 3 items to Backup",
            Progress {
                done_items: 1,
                total_items: 3,
                done_bytes: 1_200_000_000,
                total_bytes: 4_800_000_000,
                ..Default::default()
            },
        ));
        assert_eq!(view.detail.as_ref(), "1.2 of 4.8 GB · 25 %");
    }

    #[test]
    fn zero_total_shows_no_detail() {
        let view = RunningView::new(&running("Copy 0 items", Progress::default()));
        assert_eq!(view.detail.as_ref(), "");
        assert_eq!(view.fraction, 0.0);
        assert_eq!(view.percent, 0);
    }
}
