//! The sidebar: groups of favorites from `sidebar.toml`, Recent (frecency, with
//! zoxide's list when installed) and Locations (mounted volumes with free space
//! and Eject).
//!
//! Favorites are reordered by dragging them or with Cmd-Option-Up/Down. Files
//! dropped on any row move or copy into that folder; folders dropped on a group's
//! title become favorites.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crow_core::motion::{self, Effective, Fx};
use crow_core::{
    Favorite, Location, SavedQueryId, Section, SectionDestination, SectionRow, SidebarEdit, SidebarStyle, Slot, TagId,
};
use gpui::{
    Animation, AnimationExt, Bounds, ClickEvent, Context, EventEmitter, FocusHandle, Focusable, FontWeight,
    IntoElement, MouseButton, MouseDownEvent, Pixels, Point, Render, Role, SharedString, Task, Window, div, img,
    prelude::*, px,
};

use crate::actions::{
    CollapseSection, Eject, MoveDown, MoveFavoriteDown, MoveFavoriteUp, MoveUp, NextGroup, OpenSelection,
    PreviousGroup, RemoveFromSidebar,
};
use crate::chrome::with_tooltip;
use crate::context_menu::{SidebarMenuKind, SidebarMenuState};
use crate::palette::CommandId;
use crate::style::{color, color_alpha, current_location, cursor_ring, display_text, drop_outline, effect_tint, icon};
use crate::ui::{self, Ui};

pub enum SidebarEvent {
    /// The cursor moved onto a folder: show it in the center, keep focus here.
    Show(Location),
    Open(Location),
    /// The context menu's Open in New Tab.
    OpenInNewTab(Location),
    /// Files dropped on a row.
    Message(SharedString, bool),
    /// The sidebar's saved-query context menu.
    EditSavedQuery(SavedQueryId),
    RemoveSavedQuery(SavedQueryId),
}

impl EventEmitter<SidebarEvent> for Sidebar {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SidebarNav {
    Show,
    Open,
    NewTab,
}

fn emit_nav(nav: SidebarNav, location: Location, cx: &mut Context<Sidebar>) {
    match nav {
        SidebarNav::Show => cx.emit(SidebarEvent::Show(location)),
        SidebarNav::Open => cx.emit(SidebarEvent::Open(location)),
        SidebarNav::NewTab => cx.emit(SidebarEvent::OpenInNewTab(location)),
    }
}

/// What a drag would drop on in the sidebar: a selectable row or a favorite group title.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DropTarget {
    Row(usize),
    Group(usize),
}

/// The row a sidebar menu was opened on, independent of its display index.
#[derive(Clone, Debug, PartialEq, Eq)]
enum MenuTarget {
    Favorite(PathBuf),
    Recent(PathBuf),
    Volume(PathBuf),
    Plugin { section: String, id: String },
}

/// The sidebar's direct children as last prepainted: what a drop there does, whether it
/// is a favorite and where it sits, and its bounds.
type RowLayout = Rc<RefCell<Vec<(Option<DropTarget>, Option<Slot>, Bounds<Pixels>)>>>;

/// A favorite being dragged to a new place.
#[derive(Clone)]
pub struct DraggedFavorite {
    slot: Slot,
    label: SharedString,
}

impl Render for DraggedFavorite {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = &cx.global::<Ui>().theme;
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(color(theme.role("surface-raised")))
            .text_color(color(theme.role("fg")))
            .text_size(px(12.))
            .child(self.label.clone())
    }
}

/// What the cursor row is, for the bar's hints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorKind {
    Favorite,
    Recent,
    Volume,
    Plugin,
}

#[derive(Clone, Debug, PartialEq)]
enum Row {
    SectionHeader { id: String, title: String, collapsed: bool },
    Favorite(Slot, Favorite),
    Recent(PathBuf),
    Volume(crow_macos::Volume),
    PluginItem { section: String, row: SectionRow },
}

impl Row {
    fn path(&self) -> Option<&Path> {
        match self {
            Row::SectionHeader { .. } => None,
            Row::Favorite(_, f) => Some(&f.path),
            Row::Recent(p) => Some(p),
            Row::Volume(v) => Some(&v.path),
            Row::PluginItem { row, .. } => row.folder_path(),
        }
    }

    fn location(&self) -> Option<Location> {
        match self {
            Row::Favorite(_, f) => Some(Location::Folder(f.path.clone())),
            Row::Recent(path) | Row::Volume(crow_macos::Volume { path, .. }) => Some(Location::Folder(path.clone())),
            Row::PluginItem { row, .. } => match &row.destination {
                SectionDestination::Folder { path, .. } => Some(Location::Folder(path.clone())),
                SectionDestination::SavedQuery { id } => Some(Location::SavedQuery(id.clone())),
                SectionDestination::Tag { id } => Some(Location::Tag(id.clone())),
                SectionDestination::DisplayOnly => None,
            },
            Row::SectionHeader { .. } => None,
        }
    }

    fn plugin_destination(&self) -> Option<(&Path, (u64, u64))> {
        match self {
            Row::PluginItem { row, .. } => match &row.destination {
                SectionDestination::Folder { path, identity } => Some((path, *identity)),
                SectionDestination::SavedQuery { .. }
                | SectionDestination::Tag { .. }
                | SectionDestination::DisplayOnly => None,
            },
            _ => None,
        }
    }
}

/// One collapsible section in the drawn rows: where its children start and how many.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SectionSpan {
    id: String,
    collapsed: bool,
    /// Index into the rows of the first child (just after the header).
    start: usize,
    len: usize,
}

/// The collapsible sections in `rows`, in order. A section's children run up to
/// the next header of any kind.
fn section_spans(rows: &[Row]) -> Vec<SectionSpan> {
    let mut spans: Vec<SectionSpan> = Vec::new();
    let mut open = false;
    for (ix, row) in rows.iter().enumerate() {
        match row {
            Row::SectionHeader { id, collapsed, .. } => {
                spans.push(SectionSpan { id: id.clone(), collapsed: *collapsed, start: ix + 1, len: 0 });
                open = true;
            }
            _ if open => {
                if let Some(span) = spans.last_mut() {
                    span.len += 1;
                }
            }
            _ => {}
        }
    }
    spans
}

/// How a row sits in its group's card when `cards` is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CardPart {
    /// The tinted heading strip; `alone` when the group has no rows to show (collapsed).
    Strip {
        alone: bool,
    },
    Body {
        first: bool,
        last: bool,
    },
}

/// The card part of each row: headings are strips, their rows the body.
fn card_parts(rows: &[Row]) -> Vec<Option<CardPart>> {
    let mut parts = vec![None; rows.len()];
    for span in section_spans(rows) {
        let header = span.start - 1;
        parts[header] = Some(CardPart::Strip { alone: span.len == 0 });
        for k in 0..span.len {
            parts[span.start + k] = Some(CardPart::Body { first: k == 0, last: k + 1 == span.len });
        }
    }
    parts
}

/// Card corner radius, the strip's and the body's.
const CARD_RADIUS: f32 = 10.;
/// Space inside a card above its first row and below its last.
const CARD_PAD_TOP: f32 = 5.;
const CARD_PAD_BOTTOM: f32 = 6.;
/// Rows inside a card are inset this far from its edge.
const CARD_ROW_INSET: f32 = 6.;

/// Sections whose collapsed flag changed between two draws: `(id, opening, old
/// span)`. A section that appears or disappears is not a transition.
fn section_transitions(before: &[SectionSpan], now: &[SectionSpan]) -> Vec<(String, bool, SectionSpan)> {
    now.iter()
        .filter_map(|span| {
            let old = before.iter().find(|old| old.id == span.id)?;
            (old.collapsed != span.collapsed).then(|| (span.id.clone(), !span.collapsed, old.clone()))
        })
        .collect()
}

/// A section's rows opening or closing. Closing keeps the old rows' labels so
/// they can fade out while the gap closes.
struct SectionMotion {
    id: String,
    opening: bool,
    started: Instant,
    /// Closing: the rows' labels and the height they took.
    ghost: Vec<String>,
    height: f32,
}

/// Favorites that moved: each moved row starts this far (px) from where it now sits.
struct Reorder {
    started: Instant,
    offsets: HashMap<PathBuf, f32>,
}

/// The favorites of `rows` as `(group, path)` in drawn order.
fn favorite_order(rows: &[Row]) -> Vec<(usize, PathBuf)> {
    rows.iter()
        .filter_map(|row| match row {
            Row::Favorite((group, _), favorite) => Some((*group, favorite.path.clone())),
            _ => None,
        })
        .collect()
}

/// For rows that changed places within the same set of favorites: how far above
/// (negative) or below (positive) its new place each one was. `old_tops` and
/// `heights` are the rows' drawn tops and heights, by path, in the old order.
fn reorder_offsets(
    before: &[(usize, PathBuf)],
    after: &[(usize, PathBuf)],
    old_tops: &HashMap<PathBuf, f32>,
    heights: &HashMap<PathBuf, f32>,
    gap: f32,
) -> HashMap<PathBuf, f32> {
    let mut offsets = HashMap::new();
    let (mut sorted_before, mut sorted_after) = (before.to_vec(), after.to_vec());
    sorted_before.sort();
    sorted_after.sort();
    if before == after || sorted_before != sorted_after {
        return offsets;
    }
    let mut cursor: Option<(usize, f32)> = None;
    for (group, path) in after {
        let Some(old_top) = old_tops.get(path) else { continue };
        let top = match cursor {
            Some((current, next)) if current == *group => next,
            _ => {
                // The group's first row now sits where the group began: the highest old top in it.
                let start = before
                    .iter()
                    .filter(|(g, _)| g == group)
                    .filter_map(|(_, p)| old_tops.get(p))
                    .fold(f32::INFINITY, |least, top| least.min(*top));
                if start.is_finite() { start } else { *old_top }
            }
        };
        let height = heights.get(path).copied().unwrap_or(0.0);
        cursor = Some((*group, top + height + gap));
        if (old_top - top).abs() > 0.5 {
            offsets.insert(path.clone(), old_top - top);
        }
    }
    offsets
}

/// How tall an opening row may grow to; real rows are about 26 px.
const SECTION_ROW_MAX: f32 = 30.;

/// A cheap identity for the drawn rows: a reorder, added favorite or section
/// change invalidates a pending spring timer without invalidating a stationary
/// pointer over an unchanged row.
fn sidebar_layout_generation(rows: &[Row]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    rows.len().hash(&mut hasher);
    for row in rows {
        match row {
            Row::SectionHeader { id, collapsed, .. } => {
                1u8.hash(&mut hasher);
                id.hash(&mut hasher);
                collapsed.hash(&mut hasher);
            }
            Row::Favorite(slot, favorite) => {
                2u8.hash(&mut hasher);
                slot.hash(&mut hasher);
                favorite.path.hash(&mut hasher);
            }
            Row::Recent(path) => {
                3u8.hash(&mut hasher);
                path.hash(&mut hasher);
            }
            Row::Volume(volume) => {
                4u8.hash(&mut hasher);
                volume.path.hash(&mut hasher);
            }
            Row::PluginItem { section, row } => {
                5u8.hash(&mut hasher);
                section.hash(&mut hasher);
                row.id.hash(&mut hasher);
                row.folder_path().hash(&mut hasher);
            }
        }
    }
    hasher.finish()
}

fn section_order(ui: &Ui, sections: &[Section]) -> Vec<String> {
    let mut order = Vec::new();
    for id in &ui.sidebar.sections.order {
        if !order.contains(id) {
            order.push(id.clone());
        }
    }
    for id in ["tags", "saved-queries", "recent", "locations"] {
        if !order.iter().any(|seen| seen == id) {
            order.push(id.to_owned());
        }
    }
    for section in sections {
        if !order.contains(&section.id) {
            order.push(section.id.clone());
        }
    }
    order
}

/// Space above a section title, a margin so the cursor ring hugs the label.
const HEADER_GAP: f32 = 10.;

/// The same gap when groups are cards: they need more air between them.
const CARD_GAP: f32 = 14.;

/// Space above the first heading in the ledger, card and pill styles.
const TOP_GAP: f32 = 2.;

/// How far a heading's title slides right when its chevron shows.
const CHEVRON_ROOM: f32 = 20.;

/// The section id of favorites group `g`: its header collapses like any section.
fn group_section_id(g: usize) -> String {
    format!("favorites-group-{g}")
}

fn group_of_section(id: &str) -> Option<usize> {
    id.strip_prefix("favorites-group-")?.parse().ok()
}

/// Write a section's or favorites group's collapse flag.
fn set_collapsed(id: String, collapsed: bool, cx: &mut gpui::App) {
    let edit = match group_of_section(&id) {
        Some(group) => SidebarEdit::SetGroupCollapsed { group, collapsed },
        None => SidebarEdit::SetSectionCollapsed { id, collapsed },
    };
    ui::edit_sidebar(edit, cx);
}

fn rows(ui: &Ui, sections: &[Section]) -> Vec<Row> {
    let mut rows = Vec::new();
    for (g, (name, favorites)) in ui.favorites().into_iter().enumerate() {
        let collapsed = ui.sidebar.group_collapsed(g);
        rows.push(Row::SectionHeader { id: group_section_id(g), title: name, collapsed });
        if !collapsed {
            rows.extend(favorites.into_iter().enumerate().map(|(i, f)| Row::Favorite((g, i), f)));
        }
    }
    for id in section_order(ui, sections) {
        match id.as_str() {
            "tags" if ui.sidebar.section_shown("tags") && (!ui.tags.is_empty() || ui.tags.error.is_some()) => {
                let collapsed = ui.sidebar.section_collapsed("tags");
                rows.push(Row::SectionHeader {
                    id: "tags".into(),
                    title: ui.tags.section_title().to_owned(),
                    collapsed,
                });
                if !collapsed {
                    rows.extend(ui.tags.all().into_iter().map(|tag| Row::PluginItem {
                        section: "tags".into(),
                        row: SectionRow {
                            id: tag.clone(),
                            text: tag.clone(),
                            meta: None,
                            badge: None,
                            destination: SectionDestination::Tag { id: TagId::new(tag) },
                        },
                    }));
                }
            }
            "saved-queries" if ui.sidebar.section_shown("saved-queries") && !ui.saved_queries.is_empty() => {
                let collapsed = ui.sidebar.section_collapsed("saved-queries");
                rows.push(Row::SectionHeader { id: "saved-queries".into(), title: "Saved Queries".into(), collapsed });
                if !collapsed {
                    rows.extend(ui.saved_queries.queries.iter().map(|query| Row::PluginItem {
                        section: "saved-queries".into(),
                        row: SectionRow {
                            id: query.id.as_str().to_owned(),
                            text: query.name.clone(),
                            meta: ui.saved_query_counts.label(&query.id),
                            badge: None,
                            destination: SectionDestination::SavedQuery { id: query.id.clone() },
                        },
                    }));
                }
            }
            "recent" if ui.sidebar.section_shown("recent") && ui.sidebar.recent.show && !ui.recent_list.is_empty() => {
                let collapsed = ui.sidebar.section_collapsed("recent");
                rows.push(Row::SectionHeader { id: "recent".into(), title: "Recent".into(), collapsed });
                if !collapsed {
                    rows.extend(ui.recent_list.iter().cloned().map(Row::Recent));
                }
            }
            "locations"
                if ui.sidebar.section_shown("locations") && ui.sidebar.locations.show && !ui.volumes.is_empty() =>
            {
                let collapsed = ui.sidebar.section_collapsed("locations");
                rows.push(Row::SectionHeader { id: "locations".into(), title: "Locations".into(), collapsed });
                if !collapsed {
                    rows.extend(ui.volumes.iter().cloned().map(Row::Volume));
                }
            }
            _ => {
                let Some(section) = sections.iter().find(|section| section.id == id) else { continue };
                rows.push(Row::SectionHeader {
                    id: section.id.clone(),
                    title: section.title.clone(),
                    collapsed: section.collapsed,
                });
                if !section.collapsed {
                    rows.extend(
                        section.rows.iter().cloned().map(|row| Row::PluginItem { section: section.id.clone(), row }),
                    );
                }
            }
        }
    }
    rows
}

/// What a selectable row shows, separate from how it is drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct RowLook {
    pub label: String,
    pub hint: Option<String>,
    pub right: Option<String>,
    pub badge: Option<String>,
    pub spoken: String,
    pub current: bool,
    pub ring: bool,
    pub drop: bool,
    pub bar: Option<(f32, BarTone)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarTone {
    Neutral,
    Warning,
}

/// A favorite's `color`: a theme role, a color name or `#rrggbb`.
fn favorite_color(spec: &str, theme: &crow_theme::Theme) -> Option<crow_theme::Rgb> {
    if let Some(c) = crow_theme::Rgb::parse(spec) {
        return Some(c);
    }
    let slot = match spec {
        "red" => "base08",
        "orange" => "base09",
        "yellow" => "base0A",
        "green" => "base0B",
        "cyan" => "base0C",
        "blue" => "base0D",
        "purple" => "base0E",
        "pink" => "base17",
        role if crow_theme::ROLES.contains(&role) => return Some(theme.role(role)),
        _ => return None,
    };
    theme.slot(slot)
}

fn human_bytes(bytes: u64) -> String {
    // Finder counts in powers of 1000.
    let units = ["bytes", "KB", "MB", "GB", "TB", "PB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < units.len() {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} bytes") } else { format!("{value:.1} {}", units[unit]) }
}

/// The sidebar's short free-space text: no decimal from 10 up ("182 GB"), one below
/// ("4.2 GB"), powers of 1000 like Finder.
fn short_bytes(bytes: u64) -> String {
    let units = ["bytes", "KB", "MB", "GB", "TB", "PB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < units.len() {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        return format!("{bytes} bytes");
    }
    // Round at the precision the row will show, then re-check the unit: 999.95 KB
    // must read "1.0 MB", not "1000 KB". Rounding can also cross 10 ("9.96 GB" ->
    // "10 GB"), so the decimals are chosen again after it.
    loop {
        let decimals = usize::from(value < 10.0);
        let scale = 10f64.powi(decimals as i32);
        let rounded = (value * scale).round() / scale;
        if rounded >= 1000.0 && unit + 1 < units.len() {
            value = rounded / 1000.0;
            unit += 1;
            continue;
        }
        return if rounded >= 10.0 {
            format!("{rounded:.0} {}", units[unit])
        } else {
            format!("{rounded:.1} {}", units[unit])
        };
    }
}

/// At or above this share of the disk, the bar warns.
const DISK_WARNING_USED: f32 = 0.9;

fn bar_tone(used: f32) -> BarTone {
    if used >= DISK_WARNING_USED { BarTone::Warning } else { BarTone::Neutral }
}

struct PendingCurrent {
    target: Location,
    previous: Location,
    rejected: bool,
}

pub struct Sidebar {
    focus: FocusHandle,
    /// Index into the selectable rows (favorites, recents, volumes).
    cursor: usize,
    /// The folder the window shows, highlighted when it's in the sidebar.
    current: Location,
    pending_current: Option<PendingCurrent>,
    /// Where each direct child of the list sits, captured when it is prepainted.
    /// A cell so prepaint can fill it without scheduling another render.
    layout: RowLayout,
    /// The rows of the last draw, to see which sections opened or closed.
    last_rows: Vec<Row>,
    section_motions: Vec<SectionMotion>,
    /// How far each heading's chevron has slid in (0 hidden, 1 shown), easing with the pointer.
    header_hover: Vec<HeaderHover>,
    /// Favorites that changed places: each row's offset from its new place, easing to 0.
    reorder: Option<Reorder>,
    /// What a drag outside the list would drop on, drawn from state rather than a
    /// drag-over style so the highlight cannot flicker between frames.
    drop_target: Option<DropTarget>,
    /// A favorite is being reordered: its insertion line is the indicator, not a row.
    dragging_favorite: Rc<Cell<bool>>,
    /// Where a dragged favorite would be inserted: group and index before the move.
    insert_slot: Option<Slot>,
    /// Finder's display names per destination, volume first.
    names: HashMap<PathBuf, Rc<[String]>>,
    name_paths: Vec<PathBuf>,
    name_worker: Option<Task<()>>,
    /// A row pulsing because a move landed in its folder, and when it started.
    receive: Option<(PathBuf, Instant)>,
    /// Bumped per pulse, so an older timer cannot clear a newer one.
    receive_generation: u64,
    /// The row whose context menu is open (its index and what it is), outlined meanwhile.
    menu: Option<(usize, SidebarMenuKind)>,
    /// The same row by stable identity, so a refresh cannot retarget its menu.
    menu_target: Option<MenuTarget>,
    plugin_sections: Vec<Section>,
    plugin_surfaces: HashMap<String, crate::plugins::SurfaceId>,
    plugin_dirs: Vec<crate::plugins::SidebarDir>,
    plugin_dirs_key: Option<(PathBuf, u64)>,
    plugin_dirs_dir: PathBuf,
    plugin_dirs_revision: u64,
    /// No-follow identities for the captured directory slice, filled off the
    /// main thread before a row can become navigable.
    plugin_identities: HashMap<PathBuf, (u64, u64)>,
    plugin_identity_generation: u64,
    plugin_identity_epoch: u64,
    /// Section headers and rows that intersected the sidebar viewport at the
    /// last prepaint.
    visible_sections: Rc<RefCell<HashSet<String>>>,
    /// Cancels an outdated sidebar destination validation when the cursor moves
    /// again or the captured section revision changes.
    navigation_generation: u64,
    /// Stable identity of a plugin row across reorder/refresh.
    cursor_key: Option<(String, String)>,
    /// The saved query the active tab is viewing; its own session streams counts.
    active_saved: Option<SavedQueryId>,
    /// One bounded worker refreshes the visible inactive saved-query counts.
    count_worker: Option<Task<()>>,
    count_cancel: Arc<AtomicBool>,
    count_revision: u64,
    count_generation: (u64, u64),
    count_done: HashSet<SavedQueryId>,
}

impl Drop for Sidebar {
    fn drop(&mut self) {
        self.count_cancel.store(true, Ordering::Relaxed);
    }
}

impl Focusable for Sidebar {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Sidebar {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            // Row 0 is the first group's title; the cursor starts on the first row under it.
            focus: cx.focus_handle(),
            cursor: 1,
            current: Location::Folder(PathBuf::new()),
            pending_current: None,
            layout: Rc::default(),
            last_rows: Vec::new(),
            section_motions: Vec::new(),
            header_hover: Vec::new(),
            reorder: None,
            drop_target: None,
            dragging_favorite: Rc::new(Cell::new(false)),
            insert_slot: None,
            names: HashMap::new(),
            name_paths: Vec::new(),
            name_worker: None,
            receive: None,
            receive_generation: 0,
            menu: None,
            menu_target: None,
            plugin_sections: Vec::new(),
            plugin_surfaces: HashMap::new(),
            plugin_dirs: Vec::new(),
            plugin_dirs_key: None,
            plugin_dirs_dir: PathBuf::new(),
            plugin_dirs_revision: 0,
            plugin_identities: HashMap::new(),
            plugin_identity_generation: 0,
            plugin_identity_epoch: 0,
            visible_sections: Rc::default(),
            navigation_generation: 0,
            cursor_key: None,
            active_saved: None,
            count_worker: None,
            count_cancel: Arc::new(AtomicBool::new(false)),
            count_revision: 0,
            count_generation: (0, 0),
            count_done: HashSet::new(),
        }
    }

    pub fn set_active_saved(&mut self, id: Option<SavedQueryId>, cx: &mut Context<Self>) {
        if self.active_saved == id {
            return;
        }
        if let Some(previous) = self.active_saved.take() {
            self.count_done.remove(&previous);
        }
        self.active_saved = id;
        self.cancel_counts();
        cx.notify();
    }

    /// Whether the running count worker was told to stop, for release tests.
    #[cfg(test)]
    pub fn counts_cancelled_for_test(&self) -> bool {
        self.count_cancel.load(Ordering::Relaxed)
    }

    fn cancel_counts(&mut self) {
        self.count_cancel.store(true, Ordering::Relaxed);
        self.count_worker = None;
        self.count_revision = self.count_revision.wrapping_add(1);
    }

    fn request_inactive_counts(&mut self, cx: &mut Context<Self>) {
        let ui = cx.global::<Ui>();
        if !ui.sidebar.section_shown("saved-queries") || ui.sidebar.section_collapsed("saved-queries") {
            if self.count_worker.is_some() {
                self.cancel_counts();
            }
            return;
        }
        let generation = (ui.saved_queries_generation, ui.exclude_generation);
        if self.count_generation != generation {
            self.count_generation = generation;
            self.count_done.clear();
            self.cancel_counts();
        }
        if self.count_worker.is_some() {
            return;
        }
        let active = self.active_saved.clone();
        let queries: Vec<crow_core::SavedQuery> = ui
            .saved_queries
            .queries
            .iter()
            .filter(|query| Some(&query.id) != active.as_ref() && !self.count_done.contains(&query.id))
            .take(16)
            .cloned()
            .collect();
        if queries.is_empty() {
            return;
        }
        let (home, exclude) = (ui.home.clone(), ui.exclude.clone());
        let cancel = Arc::new(AtomicBool::new(false));
        self.count_cancel = cancel.clone();
        let worker_cancel = cancel.clone();
        let gate = crate::cloud_status::read_gate(cx);
        let revision = self.count_revision;
        let task = cx.background_spawn(async move {
            let mut out = Vec::new();
            for query in queries {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let id = query.id.clone();
                out.push((
                    id,
                    crate::saved_query_counts::count_saved_query(&query, &home, &exclude, &cancel, gate.clone()),
                ));
            }
            out
        });
        self.count_worker = Some(cx.spawn(async move |this, cx| {
            let results = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.count_revision != revision {
                    return;
                }
                this.count_worker = None;
                for (id, result) in results {
                    if worker_cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    this.count_done.insert(id.clone());
                    cx.global_mut::<Ui>().saved_query_counts.set(id, result.count, result.partial, false);
                }
                cx.notify();
            });
        }));
    }

    /// Keep the last visible-folder slice the plugin sections may offer. The
    /// window passes the focused listing's first 128 ordinary directories.
    fn update_plugin_sections(&mut self, cx: &mut Context<Self>) {
        let config = cx.global::<Ui>().sidebar.clone();
        let descriptors = crate::plugins::sidebar_descriptors(cx);
        let scope_dir = if self.plugin_dirs_dir.as_os_str().is_empty() {
            self.current.as_folder().map(Path::to_path_buf).unwrap_or_default()
        } else {
            self.plugin_dirs_dir.clone()
        };
        let scope = crate::plugins::ScopeKey::Folder(scope_dir.clone());
        let display = scope_dir.display().to_string();
        let revision = self.section_revision();
        let dirs = self.dirs_with_identities();
        let visible = self.visible_sections.borrow().clone();
        let mut sections = Vec::with_capacity(descriptors.len());
        for (id, title) in descriptors {
            let key = id.to_string();
            let shown = config.section_shown(&key);
            if !shown {
                if let Some(surface) = self.plugin_surfaces.get(&key).copied()
                    && cx.try_global::<crate::plugins::Plugins>().is_some()
                {
                    cx.global_mut::<crate::plugins::Plugins>().surfaces.drop_surface(surface);
                }
                continue;
            }
            let collapsed = config.section_collapsed(&key);
            let surface = *self.plugin_surfaces.entry(key.clone()).or_insert_with(crate::plugins::new_surface);
            if collapsed {
                if cx.try_global::<crate::plugins::Plugins>().is_some() {
                    cx.global_mut::<crate::plugins::Plugins>().surfaces.drop_surface(surface);
                }
                sections.push(Section { id: key, title, collapsed, rows: Vec::new() });
                continue;
            }
            // A section scrolled out of the sidebar viewport keeps its ready
            // rows but stops asking the plugin for more.
            if visible.contains(&key) {
                crate::plugins::request_sidebar(cx, surface, &id, scope.clone(), revision, display.clone(), &dirs);
            }
            sections.push(Section { id: key, title, collapsed, rows: crate::plugins::sidebar_rows(cx, surface) });
        }
        let home = cx.global::<Ui>().home.clone();
        let roots = config.project_roots(&home);
        if !roots.is_empty() && config.section_shown(crate::git::PROJECTS_SECTION) && crate::git::provider_enabled(cx) {
            sections.push(Section {
                id: crate::git::PROJECTS_SECTION.into(),
                title: "Projects".into(),
                collapsed: config.section_collapsed(crate::git::PROJECTS_SECTION),
                rows: crate::git::projects_rows(cx, roots, &home),
            });
        }
        self.plugin_sections = sections;
    }

    fn section_revision(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.plugin_dirs_revision.hash(&mut hasher);
        self.plugin_identity_epoch.hash(&mut hasher);
        hasher.finish()
    }

    fn dirs_with_identities(&self) -> Vec<crate::plugins::SidebarDir> {
        let mut dirs = self.plugin_dirs.clone();
        for dir in &mut dirs {
            dir.identity = self.plugin_identities.get(&dir.path).copied();
        }
        dirs
    }

    fn stabilize_plugin_cursor(&mut self, rows: &[Row]) {
        let selectable: Vec<&Row> = rows.iter().collect();
        if let Some((section, id)) = &self.cursor_key
            && let Some(index) = selectable
                .iter()
                .position(|row| matches!(row, Row::PluginItem { section: s, row } if s == section && &row.id == id))
        {
            self.cursor = index;
        }
        self.cursor_key = selectable.get(self.cursor).and_then(|row| match row {
            Row::PluginItem { section, row } => Some((section.clone(), row.id.clone())),
            _ => None,
        });
    }

    pub fn set_plugin_dirs(
        &mut self,
        dir: PathBuf,
        dirs: Vec<crate::plugins::SidebarDir>,
        revision: u64,
        cx: &mut Context<Self>,
    ) {
        if self.plugin_dirs_key.as_ref() == Some(&(dir.clone(), revision)) {
            return;
        }
        self.plugin_dirs_key = Some((dir.clone(), revision));
        self.plugin_dirs_dir = dir;
        self.plugin_dirs = dirs;
        self.plugin_dirs_revision = revision;
        self.plugin_identities.clear();
        self.plugin_identity_epoch = self.plugin_identity_epoch.wrapping_add(1);
        self.plugin_identity_generation = self.plugin_identity_generation.wrapping_add(1);
        let generation = self.plugin_identity_generation;
        let paths: Vec<PathBuf> = self.plugin_dirs.iter().map(|dir| dir.path.clone()).collect();
        if paths.is_empty() {
            cx.notify();
            return;
        }
        let task = cx.background_spawn(async move {
            paths
                .into_iter()
                .map(|path| {
                    // io-ok: inside background_spawn
                    let identity = std::fs::symlink_metadata(&path).ok().and_then(|meta| {
                        // io-ok: inside background_spawn
                        (meta.file_type().is_dir() && !meta.file_type().is_symlink())
                            .then_some((meta.dev(), meta.ino()))
                    });
                    (path, identity)
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let identities = task.await;
            this.update(cx, |this, cx| {
                if this.plugin_identity_generation != generation {
                    return;
                }
                let mut changed = false;
                for (path, identity) in identities {
                    match identity {
                        Some(identity) => changed |= this.plugin_identities.insert(path, identity).is_none(),
                        None => changed |= this.plugin_identities.remove(&path).is_some(),
                    }
                }
                if changed {
                    this.plugin_identity_epoch = this.plugin_identity_epoch.wrapping_add(1);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Window teardown: drop every section surface and stop pending work.
    pub fn release_plugins(&mut self, cx: &mut Context<Self>) {
        if cx.try_global::<crate::plugins::Plugins>().is_some() {
            let surfaces: Vec<_> = self.plugin_surfaces.values().copied().collect();
            let coordinator = &mut cx.global_mut::<crate::plugins::Plugins>().surfaces;
            for surface in surfaces {
                coordinator.drop_surface(surface);
            }
        }
        self.plugin_surfaces.clear();
        self.plugin_sections.clear();
        self.plugin_dirs.clear();
        self.plugin_identities.clear();
        self.visible_sections.borrow_mut().clear();
        self.navigation_generation = self.navigation_generation.wrapping_add(1);
        self.cancel_counts();
    }

    /// Pulse the row for `path` after a move lands in its folder. Full waits out the
    /// fly phase; Low tints at once; Off does nothing.
    pub fn pulse(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let mode = cx.global::<Ui>().feedback.mode;
        let total: Duration = motion::phases(mode, Fx::Receive).iter().sum();
        if total.is_zero() {
            return;
        }
        self.receive = Some((path, Instant::now()));
        self.receive_generation += 1;
        let generation = self.receive_generation;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(total).await;
            this.update(cx, |this, cx| {
                if this.receive_generation == generation {
                    this.receive = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Whether the row for `path` is pulsing, for tests.
    #[cfg(test)]
    pub fn receiving(&self, path: &Path) -> bool {
        self.receive.as_ref().is_some_and(|(target, _)| target == path)
    }

    /// The saved-query id under the cursor, if the row is a saved destination.
    pub fn cursor_saved_query(&self, cx: &gpui::App) -> Option<SavedQueryId> {
        match self.cursor_row(cx)? {
            Row::PluginItem { row, .. } => row.saved_query_id().cloned(),
            _ => None,
        }
    }

    pub fn set_current(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        self.sync_current(Location::Folder(dir), Some(true), cx);
    }

    fn visible_current(&self) -> &Location {
        match &self.pending_current {
            Some(pending) if pending.rejected => &pending.previous,
            Some(pending) => &pending.target,
            None => &self.current,
        }
    }

    /// A folder reports None while its listing is loading, then success or failure.
    pub fn sync_current(&mut self, location: Location, loaded: Option<bool>, cx: &mut Context<Self>) {
        let before = self.visible_current().clone();
        if let Some(pending) = &mut self.pending_current {
            if pending.target == location {
                match loaded {
                    Some(true) => self.pending_current = None,
                    Some(false) => pending.rejected = true,
                    None => {}
                }
            } else if self.current != location {
                self.pending_current = None;
                self.navigation_generation = self.navigation_generation.wrapping_add(1);
            }
        }
        self.current = location;
        if *self.visible_current() != before {
            cx.notify();
        }
    }

    fn selectable(&self, cx: &gpui::App) -> Vec<Row> {
        rows(cx.global::<Ui>(), &self.plugin_sections).into_iter().collect()
    }

    /// The selectors a fuzzer can click in the sidebar.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn debug_selectors(&self, cx: &gpui::App) -> Vec<String> {
        let mut out = vec!["sidebar".to_owned()];
        out.extend((0..self.selectable(cx).len()).map(|i| format!("sidebar-{i}")));
        out
    }

    /// The sidebar's focus handle, for the fuzzer's focus invariant.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn debug_focus_handle(&self, cx: &gpui::App) -> gpui::FocusHandle {
        self.focus_handle(cx)
    }

    /// Bounds for selectable row `i`, from the prepaint layout.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn debug_row_bounds(&self, i: usize, _cx: &gpui::App) -> Option<gpui::Bounds<gpui::Pixels>> {
        // Every row is selectable, so the layout index is the selectable index.
        self.layout.borrow().get(i).map(|(_, _, bounds)| *bounds)
    }

    fn refresh_names(&mut self, rows: &[Row], cx: &mut Context<Self>) {
        let paths = rows.iter().filter(|row| !matches!(row, Row::Volume(_))).filter_map(Row::path);
        if !paths.clone().eq(self.name_paths.iter().map(PathBuf::as_path)) {
            self.name_paths = paths.map(Path::to_path_buf).collect();
            let keep: HashSet<_> = self.name_paths.iter().map(PathBuf::as_path).collect();
            self.names.retain(|path, _| keep.contains(path.as_path()));
        }
        if self.name_worker.is_some() {
            return;
        }
        let missing: HashSet<_> =
            self.name_paths.iter().filter(|path| !self.names.contains_key(*path)).cloned().collect();
        if missing.is_empty() {
            return;
        }
        let load = cx.background_spawn(async move {
            let mut ancestors = HashMap::new();
            missing
                .into_iter()
                .map(|path| {
                    let names = crow_macos::display_component_paths(&path)
                        .into_iter()
                        .map(|part| {
                            ancestors
                                .entry(part)
                                .or_insert_with_key(|part| {
                                    display_text(&crow_macos::localized_display_name_at(part)).into_owned()
                                })
                                .clone()
                        })
                        .collect::<Vec<_>>();
                    (path, names)
                })
                .collect::<Vec<_>>()
        });
        self.name_worker = Some(cx.spawn(async move |this, cx| {
            let names = load.await;
            this.update(cx, |this, cx| {
                let keep: HashSet<_> = this.name_paths.iter().map(PathBuf::as_path).collect();
                this.names.extend(
                    names
                        .into_iter()
                        .filter(|(path, _)| keep.contains(path.as_path()))
                        .map(|(path, names)| (path, names.into())),
                );
                this.name_worker = None;
                cx.notify();
            })
            .ok();
        }));
    }

    /// Use the path components while Finder's names are loading.
    fn components(&self, path: &Path) -> Rc<[String]> {
        self.names.get(path).cloned().unwrap_or_else(|| {
            crow_macos::display_component_paths(path)
                .iter()
                .map(|part| {
                    let name = part.file_name().map_or_else(|| part.to_string_lossy(), |name| name.to_string_lossy());
                    display_text(&name).into_owned()
                })
                .collect::<Vec<_>>()
                .into()
        })
    }

    /// A label for a row that is closing away.
    fn ghost_label(&self, row: &Row) -> String {
        match row {
            Row::Favorite(_, f) => {
                f.name.as_deref().map_or_else(|| self.display_name(&f.path), |name| display_text(name).into_owned())
            }
            Row::Recent(path) => self.display_name(path),
            Row::Volume(v) => display_text(&v.name).into_owned(),
            Row::PluginItem { row, .. } => display_text(&row.text).into_owned(),
            Row::SectionHeader { title, .. } => display_text(title).into_owned(),
        }
    }

    /// Notice sections that opened or closed since the last draw and start their
    /// motion; drop the finished ones. Off has none.
    fn track_sections(&mut self, rows: &[Row], mode: Effective, now: Instant) {
        if mode == Effective::Full {
            let (before, after) = (favorite_order(&self.last_rows), favorite_order(rows));
            if before != after && !before.is_empty() {
                let layout = self.layout.borrow();
                let mut tops = HashMap::new();
                let mut heights = HashMap::new();
                let mut previous_bottom: Option<f32> = None;
                let mut gap = 0.0;
                for (row, (_, _, bounds)) in self.last_rows.iter().zip(layout.iter()) {
                    if let Row::Favorite(_, favorite) = row {
                        let top = f32::from(bounds.top());
                        if let Some(bottom) = previous_bottom {
                            gap = top - bottom;
                        }
                        previous_bottom = Some(f32::from(bounds.bottom()));
                        tops.insert(favorite.path.clone(), top);
                        heights.insert(favorite.path.clone(), f32::from(bounds.size.height));
                    } else {
                        previous_bottom = None;
                    }
                }
                let offsets = reorder_offsets(&before, &after, &tops, &heights, gap.max(0.0));
                self.reorder = (!offsets.is_empty()).then_some(Reorder { started: now, offsets });
            }
        }
        if self.reorder.as_ref().is_some_and(|reorder| {
            motion::progress(mode, reorder.started, now, motion::ms::REORDER, motion::ms::REORDER).is_none()
        }) {
            self.reorder = None;
        }
        if mode != Effective::Off {
            let before = section_spans(&self.last_rows);
            for (id, opening, old) in section_transitions(&before, &section_spans(rows)) {
                let (ghost, height) = if opening || old.len == 0 {
                    (Vec::new(), 0.0)
                } else {
                    let ghost = self.last_rows[old.start..old.start + old.len]
                        .iter()
                        .map(|row| self.ghost_label(row))
                        .collect();
                    let layout = self.layout.borrow();
                    let height = match (layout.get(old.start), layout.get(old.start + old.len - 1)) {
                        (Some(first), Some(last)) => f32::from(last.2.bottom() - first.2.top()),
                        _ => old.len as f32 * SECTION_ROW_MAX,
                    };
                    (ghost, height)
                };
                self.section_motions.retain(|motion| motion.id != id);
                self.section_motions.push(SectionMotion { id, opening, started: now, ghost, height });
            }
        }
        self.section_motions.retain(|motion| {
            motion::progress(mode, motion.started, now, motion::ms::SECTION, motion::ms::LOW_IN).is_some()
        });
        self.last_rows = rows.to_vec();
    }

    fn display_name(&self, path: &Path) -> String {
        self.components(path).last().cloned().unwrap_or_else(|| display_text(&path.display().to_string()).into_owned())
    }

    /// What every selectable row shows, in row order. Headers get no look.
    fn looks(&self, rows: &[Row], focused: bool, ui: &Ui) -> Vec<RowLook> {
        let vim = ui.settings.keymap == crow_core::KeymapLayer::Vim;
        let selectable: Vec<&Row> = rows.iter().collect();
        let labels: Vec<String> = selectable
            .iter()
            .map(|row| match row {
                Row::Favorite(_, f) => {
                    f.name.as_deref().map_or_else(|| self.display_name(&f.path), |name| display_text(name).into_owned())
                }
                Row::Recent(path) => self.display_name(path),
                Row::Volume(v) => display_text(&v.name).into_owned(),
                Row::SectionHeader { title, .. } => display_text(title).into_owned(),
                Row::PluginItem { row, .. } => display_text(&row.text).into_owned(),
            })
            .collect();

        // Twins are compared across Favorites and Recent, never volumes.
        let mut ancestors_for: Vec<Option<Vec<String>>> = Vec::with_capacity(selectable.len());
        for row in &selectable {
            let ancestors = match row {
                Row::Favorite(..) | Row::Recent(..) | Row::PluginItem { .. } if row.path().is_some() => {
                    let mut names = self.components(row.path().expect("a row with a path")).to_vec();
                    names.reverse();
                    Some(names)
                }
                _ => None,
            };
            ancestors_for.push(ancestors);
        }
        let mut inputs = Vec::new();
        let mut input_rows = Vec::new();
        for (i, row) in selectable.iter().enumerate() {
            if let Some(ancestors) = &ancestors_for[i] {
                inputs.push(crow_core::TwinInput {
                    label: &labels[i],
                    path: row.path().expect("a row with a path"),
                    ancestors,
                });
                input_rows.push(i);
            }
        }
        let mut hint_by_row: Vec<Option<(String, String, bool)>> = vec![None; selectable.len()];
        for (n, hint) in crow_core::twin_hints(&inputs).into_iter().enumerate() {
            let Some(distance) = hint else { continue };
            let input = &inputs[n];
            let alias = ui.aliases.contains(input.path);
            let (shown, name) = if distance == usize::MAX {
                let parent = input.path.parent().unwrap_or(input.path);
                let text = crate::style::display_path(parent, &ui.home);
                (text.clone(), text)
            } else {
                let name =
                    input.ancestors.get(distance).or_else(|| input.ancestors.last()).cloned().unwrap_or_default();
                let shown = if input.path.ancestors().nth(distance) == Some(ui.home.as_path()) {
                    "~".to_owned()
                } else {
                    name.clone()
                };
                (shown, name)
            };
            let shown = if alias { format!("{shown} · alias") } else { shown };
            let shown = display_text(&shown).into_owned();
            let name = display_text(&name).into_owned();
            hint_by_row[input_rows[n]] = Some((shown, name, alias));
        }

        let mut looks = Vec::with_capacity(selectable.len());
        for (i, row) in selectable.into_iter().enumerate() {
            let label = labels[i].clone();
            let bar = match row {
                Row::Volume(v) if v.total_bytes > 0 => {
                    let used = (1.0 - v.free_bytes as f32 / v.total_bytes as f32).clamp(0.0, 1.0);
                    Some((used, bar_tone(used)))
                }
                _ => None,
            };
            let right = match row {
                Row::Volume(v) if v.total_bytes > 0 => Some(format!("{} free", short_bytes(v.free_bytes))),
                Row::Favorite(_, f) => f.key.filter(|_| vim).map(|key| key.to_string()),
                Row::PluginItem { row, .. } => row.meta.clone(),
                _ => None,
            };
            let (hint, mut spoken) = match &hint_by_row[i] {
                Some((shown, name, alias)) => {
                    let alias = if *alias { ", alias" } else { "" };
                    let spoken = match row {
                        Row::Favorite(_, f) => match f.key {
                            Some(k) if vim => format!("{label}, in {name}{alias}, mark {k}"),
                            _ => format!("{label}, in {name}{alias}"),
                        },
                        _ => format!("{label}, in {name}{alias}"),
                    };
                    (Some(shown.clone()), spoken)
                }
                None => {
                    let spoken = match row {
                        Row::Volume(v) => format!(
                            "{label}, {} free of {}{}{}",
                            human_bytes(v.free_bytes),
                            human_bytes(v.total_bytes),
                            if v.ejectable { ", ejectable" } else { "" },
                            if bar.is_some_and(|(_, tone)| tone == BarTone::Warning) { ", almost full" } else { "" }
                        ),
                        Row::Favorite(_, f) => match f.key {
                            Some(k) if vim => format!("{label}, mark {k}"),
                            _ => label.clone(),
                        },
                        Row::SectionHeader { collapsed: true, .. } => format!("{label} section, collapsed"),
                        Row::SectionHeader { .. } => format!("{label} section, expanded"),
                        _ => label.clone(),
                    };
                    (None, spoken)
                }
            };
            let badge = match row {
                Row::PluginItem { row, .. } => row.badge.clone(),
                _ => None,
            };
            if let Row::PluginItem { row, .. } = row {
                for extra in
                    [row.meta.as_deref(), row.badge.as_deref()].into_iter().flatten().filter(|text| !text.is_empty())
                {
                    spoken.push_str(", ");
                    spoken.push_str(extra);
                }
            }
            looks.push(RowLook {
                label,
                hint,
                right,
                badge,
                spoken,
                current: row.location().as_ref() == Some(self.visible_current()),
                ring: focused && i == self.cursor,
                drop: !self.dragging_favorite.get() && self.drop_target == Some(DropTarget::Row(i)),
                bar,
            });
        }
        looks
    }

    /// The looks of the rows as the window shows them, for tests.
    #[cfg(test)]
    pub fn looks_for_test(&self, window: &Window, cx: &gpui::App) -> Vec<RowLook> {
        let ui = cx.global::<Ui>();
        let rows = rows(ui, &self.plugin_sections);
        self.looks(&rows, self.focus.is_focused(window) && window.last_input_was_keyboard(), ui)
    }

    fn cursor_row(&self, cx: &gpui::App) -> Option<Row> {
        self.selectable(cx).into_iter().nth(self.cursor)
    }

    #[cfg(test)]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The cursor row's folder, for the Info tab.
    pub fn cursor_path(&self, cx: &gpui::App) -> Option<PathBuf> {
        self.cursor_row(cx)?.path().map(Path::to_path_buf)
    }

    /// What the cursor row is, for which hints apply.
    pub fn cursor_kind(&self, cx: &gpui::App) -> Option<CursorKind> {
        match self.cursor_row(cx)? {
            Row::Favorite(..) => Some(CursorKind::Favorite),
            Row::Recent(_) => Some(CursorKind::Recent),
            Row::Volume(_) => Some(CursorKind::Volume),
            Row::PluginItem { .. } => Some(CursorKind::Plugin),
            Row::SectionHeader { .. } => None,
        }
    }

    /// The sidebar group the cursor row belongs to, and how many rows it has.
    pub fn cursor_group(&self, cx: &gpui::App) -> Option<(String, usize)> {
        match self.cursor_row(cx)? {
            Row::Favorite(slot, _) => {
                let groups = cx.global::<Ui>().favorites();
                let (name, rows) = groups.get(slot.0)?;
                Some((name.clone(), rows.len()))
            }
            Row::Recent(_) => Some(("Recent".into(), cx.global::<Ui>().recent_list.len())),
            Row::Volume(_) => Some(("Locations".into(), cx.global::<Ui>().volumes.len())),
            Row::PluginItem { section, .. } => Some((section.clone(), 1)),
            Row::SectionHeader { .. } => None,
        }
    }

    /// Index of a path in the selectable rows, including section headers.
    #[cfg(test)]
    pub fn index_of_path(&self, path: &Path, cx: &gpui::App) -> Option<usize> {
        self.selectable(cx).iter().position(|row| row.path() == Some(path))
    }

    #[cfg(test)]
    pub fn section_surface_for_test(&self, id: &str) -> Option<crate::plugins::SurfaceId> {
        self.plugin_surfaces.get(id).copied()
    }

    #[cfg(test)]
    pub fn add_surface_for_test(&mut self, id: &str, surface: crate::plugins::SurfaceId) {
        self.plugin_surfaces.insert(id.to_owned(), surface);
    }

    #[cfg(test)]
    pub fn set_cursor_for_test(&mut self, index: usize, cx: &mut Context<Self>) {
        self.cursor = index;
        cx.notify();
    }

    #[cfg(test)]
    pub fn index_of_section_for_test(&self, id: &str, cx: &gpui::App) -> Option<usize> {
        self.selectable(cx).iter().position(|row| matches!(row, Row::SectionHeader { id: actual, .. } if actual == id))
    }

    /// Paths of the selectable rows, in order.
    #[cfg(test)]
    pub fn paths(cx: &gpui::App) -> Vec<PathBuf> {
        rows(cx.global::<Ui>(), &[]).into_iter().filter_map(|row| row.path().map(Path::to_path_buf)).collect()
    }

    /// The row or group title the last drag would drop on, for tests: (is a row, index).
    #[cfg(test)]
    pub fn drop_target(&self) -> Option<(bool, usize)> {
        self.drop_target.map(|target| match target {
            DropTarget::Row(i) => (true, i),
            DropTarget::Group(g) => (false, g),
        })
    }

    /// Where a dragged favorite would be inserted, for tests.
    #[cfg(test)]
    pub fn insert_slot(&self) -> Option<Slot> {
        self.insert_slot
    }

    /// Where a drag would insert a favorite, from the pointer. A dragged favorite always
    /// lands between rows: the upper half of a favorite inserts before it, the lower half
    /// after it, and a group title inserts first. Other drags (`edges_only`) insert only
    /// from a favorite's upper or lower quarter, as in Finder; the middle of the row is
    /// left to moving into its folder, and a group title to adding at the group's end.
    fn favorite_slot_at(&self, position: Point<Pixels>, edges_only: bool) -> Option<Slot> {
        let layout = self.layout.borrow();
        let slot_of = |target: &Option<DropTarget>, favorite: &Option<Slot>, bounds: &Bounds<Pixels>| {
            if let Some(slot) = favorite {
                let band = bounds.size.height / if edges_only { 4.0 } else { 2.0 };
                if position.y < bounds.top() + band {
                    Some(*slot)
                } else if position.y >= bounds.bottom() - band {
                    Some((slot.0, slot.1 + 1))
                } else {
                    None
                }
            } else if let (Some(DropTarget::Group(group)), false) = (target, edges_only) {
                Some((*group, 0))
            } else {
                None
            }
        };
        let mut nearest: Option<(f32, Slot)> = None;
        for (target, favorite, bounds) in layout.iter() {
            if position.x < bounds.left() || position.x >= bounds.right() {
                continue;
            }
            if bounds.contains(&position) {
                return slot_of(target, favorite, bounds);
            }
            let gap = if position.y < bounds.top() { bounds.top() - position.y } else { position.y - bounds.bottom() };
            let gap = f32::from(gap);
            if gap <= 4.0
                && let Some(slot) = slot_of(target, favorite, bounds)
                && nearest.is_none_or(|(best, _)| gap < best)
            {
                nearest = Some((gap, slot));
            }
        }
        nearest.map(|(_, slot)| slot)
    }

    /// The typed drop destination under `position`, from the same row geometry as
    /// the highlight. Insertion edges win over the row's folder, as they do in Finder.
    pub(crate) fn drop_hit(&self, position: Point<Pixels>, cx: &gpui::App) -> Option<crate::drag_drop::DropHit> {
        let rows = rows(cx.global::<Ui>(), &self.plugin_sections);
        let generation = sidebar_layout_generation(&rows);
        if let Some(slot) = self.favorite_slot_at(position, true) {
            return Some(crate::drag_drop::DropHit::insertion(slot, generation));
        }
        match self.target_at(position)? {
            DropTarget::Group(group) => {
                Some(crate::drag_drop::DropHit::insertion(self.group_end(group, cx), generation))
            }
            DropTarget::Row(index) => {
                let row = rows.into_iter().nth(index)?;
                let path = row.path()?.to_path_buf();
                Some(crate::drag_drop::DropHit::folder(path, generation, true))
            }
        }
    }

    /// The row or group title under `position`; the 1 px gaps between rows snap to
    /// the nearest side so a small pointer wobble cannot lose the highlight.
    fn target_at(&self, position: Point<Pixels>) -> Option<DropTarget> {
        let layout = self.layout.borrow();
        let mut nearest: Option<(f32, DropTarget)> = None;
        for (target, _, bounds) in layout.iter() {
            let Some(target) = target else { continue };
            if position.x < bounds.left() || position.x >= bounds.right() {
                continue;
            }
            if position.y >= bounds.top() && position.y < bounds.bottom() {
                return Some(*target);
            }
            let gap = if position.y < bounds.top() { bounds.top() - position.y } else { position.y - bounds.bottom() };
            let gap = f32::from(gap);
            if gap <= 4.0 && nearest.is_none_or(|(best, _)| gap < best) {
                nearest = Some((gap, *target));
            }
        }
        nearest.map(|(_, target)| target)
    }

    fn move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        let next = self.cursor.saturating_sub(1);
        if next == self.cursor {
            return;
        }
        self.cursor = next;
        self.show_cursor(cx);
        cx.notify();
    }

    fn move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        let next = (self.cursor + 1).min(self.selectable(cx).len().saturating_sub(1));
        if next == self.cursor {
            return;
        }
        self.cursor = next;
        self.show_cursor(cx);
        cx.notify();
    }

    /// The selectable indices of each non-empty group, in sidebar order.
    fn group_rows(&self, cx: &gpui::App) -> Vec<Vec<usize>> {
        let mut groups: Vec<Vec<usize>> = Vec::new();
        let mut index = 0;
        for row in rows(cx.global::<Ui>(), &self.plugin_sections) {
            if matches!(row, Row::SectionHeader { .. }) {
                // Section headers are selectable, so they take an index, but an
                // Option-Up/Down jump moves to content, not to a heading.
                groups.push(Vec::new());
                index += 1;
            } else {
                if let Some(group) = groups.last_mut() {
                    group.push(index);
                }
                index += 1;
            }
        }
        groups.retain(|group| !group.is_empty());
        groups
    }

    /// Move to the first row of the previous or next group.
    fn jump_group(&mut self, delta: isize, cx: &mut Context<Self>) {
        let groups = self.group_rows(cx);
        let Some(current) = groups.iter().position(|group| group.contains(&self.cursor)) else { return };
        let Some(target) = current.checked_add_signed(delta).filter(|target| *target < groups.len()) else {
            return;
        };
        self.cursor = groups[target][0];
        self.show_cursor(cx);
        cx.notify();
    }

    fn previous_group(&mut self, _: &PreviousGroup, _: &mut Window, cx: &mut Context<Self>) {
        self.jump_group(-1, cx);
    }

    fn next_group(&mut self, _: &NextGroup, _: &mut Window, cx: &mut Context<Self>) {
        self.jump_group(1, cx);
    }

    /// Ask the window to show the cursor's folder without leaving the sidebar.
    fn show_cursor(&mut self, cx: &mut Context<Self>) {
        self.navigate_row(self.cursor, SidebarNav::Show, cx);
    }

    fn open(&mut self, _: &OpenSelection, _: &mut Window, cx: &mut Context<Self>) {
        self.navigate_row(self.cursor, SidebarNav::Open, cx);
    }

    fn collapse_section(&mut self, _: &CollapseSection, _: &mut Window, cx: &mut Context<Self>) {
        let Some(Row::SectionHeader { id, collapsed: false, .. }) = self.cursor_row(cx) else { return };
        set_collapsed(id, true, cx);
    }

    /// Move to `index`, or validate a plugin destination off-thread before the
    /// window navigates. `navigation_generation` makes a later move discard a
    /// stale result; the live row and section revision are checked again on the
    /// main thread.
    fn navigate_row(&mut self, index: usize, nav: SidebarNav, cx: &mut Context<Self>) {
        self.navigate(index, nav, false, cx);
    }

    fn navigate(&mut self, index: usize, nav: SidebarNav, clicked: bool, cx: &mut Context<Self>) {
        let previous = self.pending_current.take().map_or_else(|| self.current.clone(), |pending| pending.previous);
        self.cursor_key = None;
        self.navigation_generation = self.navigation_generation.wrapping_add(1);
        let generation = self.navigation_generation;
        let Some(row) = self.selectable(cx).into_iter().nth(index) else { return };
        if clicked {
            self.pending_current = row.location().map(|target| PendingCurrent { target, previous, rejected: false });
        }
        if let Row::SectionHeader { id, collapsed, .. } = row {
            if nav == SidebarNav::Open {
                set_collapsed(id, !collapsed, cx);
            }
            return;
        }
        if let Row::PluginItem { row, .. } = &row {
            let destination = match &row.destination {
                SectionDestination::SavedQuery { id } => Some(Location::SavedQuery(id.clone())),
                SectionDestination::Tag { id } => Some(Location::Tag(id.clone())),
                _ => None,
            };
            if let Some(destination) = destination {
                // An arrow only moves the cursor over a virtual destination; activating
                // it opens the destination (and starts its result session).
                if nav != SidebarNav::Show {
                    emit_nav(nav, destination, cx);
                }
                return;
            }
        }
        let Some((path, identity)) = row.plugin_destination().map(|(path, identity)| (path.to_path_buf(), identity))
        else {
            if let Some(path) = row.path() {
                emit_nav(nav, Location::Folder(path.to_path_buf()), cx);
            }
            return;
        };
        let SectionRow { id: row_id, .. } = match &row {
            Row::PluginItem { row, .. } => row,
            _ => return,
        };
        let row_id = row_id.clone();
        let section = match &row {
            Row::PluginItem { section, .. } => section.clone(),
            _ => return,
        };
        let scope = if self.plugin_dirs_dir.as_os_str().is_empty() {
            self.current.as_folder().map(Path::to_path_buf).unwrap_or_default()
        } else {
            self.plugin_dirs_dir.clone()
        };
        let revision = self.section_revision();
        let expected_path = path.clone();
        let task = cx.background_spawn(async move {
            // io-ok: inside background_spawn
            std::fs::symlink_metadata(&path).ok().is_some_and(|meta| {
                // io-ok: inside background_spawn
                meta.file_type().is_dir()
                    && !meta.file_type().is_symlink()
                    && (meta.dev(), meta.ino()) == identity
                    && path.parent() == Some(scope.as_path())
            })
        });
        cx.spawn(async move |this, cx| {
            let matches = task.await;
            this.update(cx, |this, cx| {
                if this.navigation_generation != generation {
                    return;
                }
                let live = this.selectable(cx).into_iter().nth(index);
                let live = matches!(
                    live,
                    Some(Row::PluginItem { section: s, row })
                        if s == section && row.id == row_id && row.folder_path() == Some(expected_path.as_path())
                );
                if this.section_revision() != revision || !live {
                    this.pending_current = None;
                    cx.notify();
                    return;
                }
                if matches {
                    emit_nav(nav, Location::Folder(expected_path), cx);
                } else {
                    this.pending_current = None;
                    cx.notify();
                    if nav != SidebarNav::Show {
                        cx.emit(SidebarEvent::Message("That folder is no longer available".into(), true));
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    fn remove(&mut self, _: &RemoveFromSidebar, _: &mut Window, cx: &mut Context<Self>) {
        match self.cursor_row(cx) {
            Some(Row::Favorite(slot, _)) => ui::edit_sidebar(SidebarEdit::Remove(slot), cx),
            Some(Row::Recent(path)) => {
                cx.global_mut::<Ui>().recent.forget(&path);
                ui::refresh_recent(cx);
            }
            _ => return,
        }
        cx.notify();
    }

    /// Move the favorite under the cursor one place up or down within its group;
    /// the cursor follows it.
    fn shift_favorite(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(Row::Favorite((g, i), _)) = self.cursor_row(cx) else { return };
        let len = cx.global::<Ui>().favorites().get(g).map_or(0, |(_, f)| f.len());
        let Some(target) = i.checked_add_signed(delta).filter(|t| *t < len) else { return };
        // `to` counts positions before the move, so moving down skips one more.
        let to = if delta > 0 { target + 1 } else { target };
        ui::edit_sidebar(SidebarEdit::Move { from: (g, i), to: (g, to) }, cx);
        self.cursor = self.cursor.saturating_add_signed(delta);
        cx.notify();
    }

    fn favorite_up(&mut self, _: &MoveFavoriteUp, _: &mut Window, cx: &mut Context<Self>) {
        self.shift_favorite(-1, cx);
    }

    fn favorite_down(&mut self, _: &MoveFavoriteDown, _: &mut Window, cx: &mut Context<Self>) {
        self.shift_favorite(1, cx);
    }

    fn eject_cursor(&mut self, _: &Eject, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(Row::Volume(v)) = self.cursor_row(cx) {
            self.eject(v, cx);
        }
    }

    /// Run a palette command that acts on the sidebar's cursor row.
    pub fn run_command(&mut self, command: CommandId, window: &mut Window, cx: &mut Context<Self>) {
        match command {
            CommandId::RemoveFromSidebar => self.remove(&RemoveFromSidebar, window, cx),
            CommandId::MoveFavoriteUp => self.favorite_up(&MoveFavoriteUp, window, cx),
            CommandId::MoveFavoriteDown => self.favorite_down(&MoveFavoriteDown, window, cx),
            CommandId::Eject => self.eject_cursor(&Eject, window, cx),
            _ => {}
        }
    }

    // ----- context menu -----

    /// A right-click or Ctrl-click on row `i`. The row is outlined while its menu is
    /// open, as in Finder; the cursor and the folder shown stay as they are.
    fn open_menu(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.selectable(cx).into_iter().nth(i) else { return };
        let (kind, state) = match &row {
            Row::Favorite((g, n), _) => {
                let len = cx.global::<Ui>().favorites().get(*g).map_or(0, |(_, f)| f.len());
                let state =
                    SidebarMenuState { can_move_up: *n > 0, can_move_down: n + 1 < len, ..SidebarMenuState::default() };
                (SidebarMenuKind::Favorite, state)
            }
            Row::Recent(_) => (SidebarMenuKind::Recent, SidebarMenuState::default()),
            Row::Volume(v) => {
                (SidebarMenuKind::Volume, SidebarMenuState { ejectable: v.ejectable, ..SidebarMenuState::default() })
            }
            Row::PluginItem { row, .. } if row.saved_query_id().is_some() => {
                (SidebarMenuKind::SavedQuery, SidebarMenuState::default())
            }
            Row::PluginItem { section, row } if section == crate::git::PROJECTS_SECTION => {
                let removable_root = row.folder_path().is_some_and(|path| self.project_root_of(path, cx).is_some());
                (SidebarMenuKind::Project, SidebarMenuState { removable_root, ..SidebarMenuState::default() })
            }
            Row::PluginItem { row, .. } if row.folder_path().is_some() => {
                (SidebarMenuKind::Plugin, SidebarMenuState::default())
            }
            Row::SectionHeader { .. } | Row::PluginItem { .. } => return,
        };
        let target = match &row {
            Row::Favorite(_, favorite) => MenuTarget::Favorite(favorite.path.clone()),
            Row::Recent(path) => MenuTarget::Recent(path.clone()),
            Row::Volume(volume) => MenuTarget::Volume(volume.path.clone()),
            Row::PluginItem { section, row } => MenuTarget::Plugin { section: section.clone(), id: row.id.clone() },
            Row::SectionHeader { .. } => return,
        };
        let entries = crate::context_menu::sidebar_entries(kind, state, cx);
        self.menu = Some((i, kind));
        self.menu_target = Some(target);
        cx.notify();
        let (sender, receiver) = futures::channel::oneshot::channel();
        // Off the main thread (tests) the recorder answers at once; in the app the
        // native menu calls back when it closes.
        if !crate::effects::effects(cx).show_context_menu(
            window,
            entries,
            Box::new(move |chosen| {
                sender.send(chosen).ok();
            }),
        ) {
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            let chosen = receiver.await.ok().flatten();
            this.update_in(cx, |this, window, cx| this.menu_closed(chosen, window, cx)).ok();
        })
        .detach();
    }

    /// Resolve the row a menu was opened on after any refresh.
    fn resolve_menu_target(&self, target: &MenuTarget, cx: &gpui::App) -> Option<(usize, Row)> {
        self.selectable(cx).into_iter().enumerate().find(|(_, row)| match (target, row) {
            (MenuTarget::Favorite(path), Row::Favorite(_, actual)) => path == &actual.path,
            (MenuTarget::Recent(path), Row::Recent(actual)) => path == actual,
            (MenuTarget::Volume(path), Row::Volume(actual)) => path == &actual.path,
            (MenuTarget::Plugin { section, id }, Row::PluginItem { section: actual_section, row }) => {
                section == actual_section && id == &row.id
            }
            _ => false,
        })
    }

    /// The menu closed with the line at `chosen` picked (an index into its slots), or none.
    fn menu_closed(&mut self, chosen: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let Some((menu_index, kind)) = self.menu.take() else { return };
        let target = self.menu_target.take();
        cx.notify();
        let Some((id, _)) = chosen.and_then(|ix| crate::context_menu::sidebar_slots(kind).get(ix).copied().flatten())
        else {
            return;
        };
        let Some((i, row)) = target
            .as_ref()
            .and_then(|target| self.resolve_menu_target(target, cx))
            .or_else(|| self.selectable(cx).into_iter().enumerate().nth(menu_index))
        else {
            return;
        };
        if let Row::PluginItem { row, .. } = &row
            && let Some(saved_id) = row.saved_query_id().cloned()
        {
            self.cursor = i;
            match id {
                CommandId::Open => self.navigate_row(i, SidebarNav::Open, cx),
                CommandId::NewTab => self.navigate_row(i, SidebarNav::NewTab, cx),
                CommandId::EditSavedSearch => cx.emit(SidebarEvent::EditSavedQuery(saved_id)),
                CommandId::RemoveSavedSearch => cx.emit(SidebarEvent::RemoveSavedQuery(saved_id)),
                _ => {}
            }
            return;
        }
        let Some(path) = row.path().map(Path::to_path_buf) else { return };
        match id {
            CommandId::RefreshProjects => crate::git::refresh_projects(cx),
            CommandId::RemoveProjectRoot => {
                if let Some(root) = self.project_root_of(&path, cx) {
                    ui::edit_sidebar(crow_core::SidebarEdit::RemoveProjectRoot(root), cx);
                    crate::git::refresh_projects(cx);
                }
            }
            CommandId::Open => {
                self.cursor = i;
                self.navigate_row(i, SidebarNav::Open, cx);
            }
            CommandId::NewTab => self.navigate_row(i, SidebarNav::NewTab, cx),
            CommandId::CopyPath => {
                crate::effects::write_text(path.to_string_lossy().into_owned(), cx);
                crate::operations::message("Copied path", false, cx);
            }
            CommandId::ShowInFinder => crate::effects::effects(cx).reveal_in_finder(&[path]),
            CommandId::AddToSidebar => {
                let at = self.group_end(0, cx);
                self.add_folders(at, vec![path], cx);
            }
            // These act on the cursor row, as their keys do.
            CommandId::MoveFavoriteUp
            | CommandId::MoveFavoriteDown
            | CommandId::RemoveFromSidebar
            | CommandId::Eject => {
                self.cursor = i;
                self.run_command(id, window, cx);
            }
            _ => {}
        }
    }

    /// The configured Projects root that holds `path`: the deepest one when roots nest.
    fn project_root_of(&self, path: &Path, cx: &gpui::App) -> Option<PathBuf> {
        let ui = cx.global::<Ui>();
        ui.sidebar
            .project_roots(&ui.home)
            .into_iter()
            .filter(|root| path.starts_with(root))
            .max_by_key(|root| root.as_os_str().len())
    }

    #[cfg(test)]
    pub fn menu(&self) -> Option<(usize, SidebarMenuKind)> {
        self.menu
    }

    #[cfg(test)]
    pub fn choose_menu_item(&mut self, chosen: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.menu_closed(chosen, window, cx);
    }

    fn eject(&mut self, volume: crow_macos::Volume, cx: &mut Context<Self>) {
        if !volume.ejectable {
            return;
        }
        cx.emit(SidebarEvent::Message(format!("Ejecting “{}”…", volume.name).into(), false));
        let path = volume.path.clone();
        let effects = crate::effects::effects(cx);
        let task = cx.background_spawn(async move { effects.eject(&path) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |_, cx| {
                let message = match result {
                    Ok(()) => (format!("Ejected “{}”", volume.name), false),
                    Err(e) => (format!("Can't eject “{}”: {e}", volume.name), true),
                };
                cx.emit(SidebarEvent::Message(message.0.into(), message.1));
                ui::refresh_volumes(cx);
            })
            .ok();
        })
        .detach();
    }

    /// The slot after a group's last favorite.
    fn group_end(&self, group: usize, cx: &gpui::App) -> Slot {
        (group, cx.global::<Ui>().favorites().get(group).map_or(0, |(_, f)| f.len()))
    }

    /// Folders dropped on a group's title or between two favorites join the group there.
    fn add_folders(&mut self, at: Slot, folders: Vec<PathBuf>, cx: &mut Context<Self>) {
        if folders.is_empty() {
            cx.emit(SidebarEvent::Message("Only folders can be added to the sidebar".into(), true));
            return;
        }
        ui::edit_sidebar(SidebarEdit::AddMany { paths: folders, at }, cx);
    }

    /// The folders among paths dropped from another app: ask the disk off the main
    /// thread, then add them on it.
    pub(crate) fn add_dropped_folders(&mut self, at: Slot, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let task = cx.background_spawn(async move { paths.into_iter().filter(|p| p.is_dir()).collect::<Vec<_>>() }); // io-ok: inside background_spawn
        cx.spawn(async move |this, cx| {
            let folders = task.await;
            this.update(cx, |this, cx| this.add_folders(at, folders, cx)).ok();
        })
        .detach();
    }
}

/// A heading's chevron reveal: easing from `from` to `target` since `since`.
struct HeaderHover {
    id: String,
    from: f32,
    target: f32,
    since: Instant,
}

impl HeaderHover {
    /// The slide amount now, and whether it is still moving.
    fn amount(&self, mode: Effective, now: Instant) -> (f32, bool) {
        match motion::progress(mode, self.since, now, motion::ms::SECTION, motion::ms::LOW_IN) {
            Some(t) => (self.from + (self.target - self.from) * t, true),
            None => (self.target, false),
        }
    }
}

impl Sidebar {
    fn header_amount(&self, id: &str, mode: Effective, now: Instant) -> f32 {
        self.header_hover.iter().find(|h| h.id == id).map_or(0.0, |h| h.amount(mode, now).0)
    }

    fn set_header_hover(&mut self, id: &str, hovered: bool, cx: &mut Context<Self>) {
        let mode = cx.global::<Ui>().feedback.mode;
        let now = Instant::now();
        let target = f32::from(hovered);
        let from = self.header_amount(id, mode, now);
        self.header_hover.retain(|h| h.id != id);
        self.header_hover.push(HeaderHover { id: id.to_owned(), from, target, since: now });
        cx.notify();
    }
}

impl Render for Sidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The receive pulse is drawn from the clock: keep frames coming while it runs.
        if self.receive.is_some() {
            window.request_animation_frame();
        }
        self.update_plugin_sections(cx);
        self.request_inactive_counts(cx);
        let rows = rows(cx.global::<Ui>(), &self.plugin_sections);
        self.refresh_names(&rows, cx);
        let (motion_mode, motion_now) = (cx.global::<Ui>().feedback.mode, Instant::now());
        self.track_sections(&rows, motion_mode, motion_now);
        self.header_hover.retain(|h| h.amount(motion_mode, motion_now).1 || h.target > 0.0);
        let hovering_moves = self.header_hover.iter().any(|h| h.amount(motion_mode, motion_now).1);
        if !self.section_motions.is_empty() || self.reorder.is_some() || hovering_moves {
            window.request_animation_frame();
        }
        let wants_pin_lines = self.layout.borrow().first().is_some_and(|(_, _, bounds)| bounds.size.width >= px(300.));
        let pin_summaries: HashMap<_, _> = if crate::git::provider_enabled(cx) {
            rows.iter()
                .filter_map(|row| match row {
                    Row::Favorite(_, favorite) => Some(favorite.path.as_path()),
                    _ => None,
                })
                .filter_map(|path| {
                    crate::git::pin_summary(cx, path, wants_pin_lines).map(|summary| (path.to_owned(), summary))
                })
                .collect()
        } else {
            HashMap::new()
        };
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        let mode = ui.feedback.mode;
        let now = Instant::now();
        let icons = crate::icon_packs::Resolver::for_source(cx, &ui.settings.icon_pack);
        let vim = ui.settings.keymap == crow_core::KeymapLayer::Vim;
        self.stabilize_plugin_cursor(&rows);
        let focused = self.focus.is_focused(window) && window.last_input_was_keyboard();
        let selectable_count = rows.len();
        self.cursor = self.cursor.min(selectable_count.saturating_sub(1));
        let section_of: Vec<Option<String>> = rows
            .iter()
            .map(|row| match row {
                Row::SectionHeader { id, .. } => Some(id.clone()),
                Row::PluginItem { section, .. } => Some(section.clone()),
                _ => None,
            })
            .collect();
        let (fg, muted, accent) = (theme.role("fg"), theme.role("fg-muted"), theme.role("accent"));
        let subtle = theme.role("fg-subtle");

        // A drag's target comes from the pointer and the last layout, so the
        // highlight only changes when the pointer enters a different row.
        if cx.has_active_drag() {
            let position = window.mouse_position();
            self.insert_slot = self.favorite_slot_at(position, !self.dragging_favorite.get());
            // Between two favorites the line shows where the drop goes; no row fills.
            self.drop_target = match self.insert_slot {
                Some(_) if !self.dragging_favorite.get() => None,
                _ => self.target_at(position).filter(|target| match target {
                    DropTarget::Group(_) => true,
                    DropTarget::Row(index) => rows.get(*index).and_then(Row::path).is_some_and(|path| {
                        crate::operations::drop_would_act(crate::drag_drop::active_paths(cx), path)
                    }),
                }),
            };
        } else {
            self.drop_target = None;
            self.insert_slot = None;
            self.dragging_favorite.set(false);
        }
        let style = ui.settings.sidebar_style;
        let (cards, accordion, pills) =
            (style == SidebarStyle::Cards, style == SidebarStyle::Accordion, style == SidebarStyle::Pills);
        let ledger = style == SidebarStyle::Ledger;
        let header_gap = match style {
            SidebarStyle::Cards => CARD_GAP,
            SidebarStyle::Accordion => 0.,
            _ => HEADER_GAP,
        };
        // The first heading sits closer to the top in the styles that float their headings.
        let first_gap = match style {
            SidebarStyle::Ledger | SidebarStyle::Cards | SidebarStyle::Pills => TOP_GAP,
            _ => header_gap,
        };
        let current_child = rows.iter().position(|row| row.location().as_ref() == Some(self.visible_current()));
        let fill = {
            let layout = self.layout.borrow();
            let current_bounds = current_child.and_then(|ix| layout.get(ix).map(|(_, _, bounds)| *bounds));
            let first_bounds = layout.first().map(|(_, _, bounds)| *bounds);
            match (current_bounds, first_bounds) {
                (Some(current), Some(first)) => {
                    // Section titles keep their gap above as a margin, outside their bounds.
                    let gap = if matches!(rows.first(), Some(Row::SectionHeader { .. })) { first_gap } else { 0.0 };
                    let top = f32::from(current.top() - first.top()) + gap;
                    let base = div()
                        .absolute()
                        .top(px(top))
                        .left(px(if cards {
                            8. + CARD_ROW_INSET
                        } else if accordion {
                            0.
                        } else {
                            8.
                        }))
                        .right(px(if cards {
                            8. + CARD_ROW_INSET
                        } else if accordion {
                            0.
                        } else {
                            8.
                        }))
                        .h(current.size.height)
                        .when(!accordion, |fill| fill.rounded_md())
                        .bg(current_location(&theme))
                        .debug_selector(|| "sidebar-fill".into());
                    match mode {
                        Effective::Full => base
                            .with_spring("sidebar-fill", crate::motion_ui::glide(px(top)), |el, y| {
                                el.top(px(f32::from(y)))
                            })
                            .into_any_element(),
                        Effective::Low => base
                            .with_animation(
                                ("sidebar-fill", current_child.unwrap_or(usize::MAX)),
                                Animation::new(Duration::from_millis(crow_core::motion::ms::LOW_IN)),
                                |el, t| el.opacity(motion::ease_out(t)),
                            )
                            .into_any_element(),
                        Effective::Off => base.into_any_element(),
                    }
                }
                _ => div().absolute().size(px(0.)).debug_selector(|| "sidebar-fill".into()).into_any_element(),
            }
        };
        let parts = if cards || accordion { card_parts(&rows) } else { vec![None; rows.len()] };
        let section_counts: HashMap<String, usize> =
            section_spans(&rows).into_iter().map(|span| (span.id, span.len)).collect();
        let (card_body, card_strip) = (theme.role("bg"), theme.role("surface"));
        let card_border = theme.role("border");
        // Each card's body is drawn behind the rows from the last layout, like the fill.
        let backdrops: Vec<gpui::AnyElement> = {
            let layout = self.layout.borrow();
            let first = layout.first().map(|(_, _, bounds)| *bounds);
            let gap = if matches!(rows.first(), Some(Row::SectionHeader { .. })) { first_gap } else { 0.0 };
            section_spans(&rows)
                .into_iter()
                .filter(|span| span.len > 0)
                .enumerate()
                .filter_map(|(n, span)| {
                    let first = first?;
                    let header = layout.get(span.start - 1)?.2;
                    let last = layout.get(span.start + span.len - 1)?.2;
                    let top = f32::from(header.bottom() - first.top()) + gap;
                    let height = f32::from(last.bottom() - header.bottom()) + CARD_PAD_BOTTOM;
                    Some(
                        div()
                            .absolute()
                            .top(px(top))
                            .left(px(8.))
                            .right(px(8.))
                            .h(px(height))
                            .bg(color(card_body))
                            .border_1()
                            .border_color(color(card_border))
                            .rounded_bl(px(CARD_RADIUS))
                            .rounded_br(px(CARD_RADIUS))
                            .debug_selector(move || format!("sidebar-card-{n}"))
                            .into_any_element(),
                    )
                })
                .filter(|_| cards)
                .collect()
        };
        let backdrop_count = backdrops.len();
        let mut kinds: Vec<(Option<DropTarget>, Option<Slot>)> = Vec::with_capacity(rows.len());
        let mut looks = self.looks(&rows, focused, ui).into_iter();
        let mut index = 0;
        let mut children = Vec::with_capacity(rows.len());
        let mut ghost_child: Option<usize> = None;
        let mut current_section: Option<String> = None;
        let section_progress = |id: &str| {
            self.section_motions.iter().find(|m| m.id == id).and_then(|m| {
                motion::progress(mode, m.started, now, motion::ms::SECTION, motion::ms::LOW_IN).map(|t| (m, t))
            })
        };
        for row in rows {
            if let Row::SectionHeader { id, title, collapsed } = &row {
                let id = id.clone();
                let collapsed = *collapsed;
                current_section = Some(id.clone());
                let moving = section_progress(&id);
                // One fixed-size box in every state, so the label never shifts. Full
                // crossfades the two glyphs as the section moves (text glyphs don't rotate).
                let open = match moving {
                    Some((motion, t)) if mode == Effective::Full => {
                        if motion.opening {
                            t
                        } else {
                            1.0 - t
                        }
                    }
                    _ => f32::from(!collapsed),
                };
                let group = group_of_section(&id);
                let cursor_here = self.cursor == index;
                let ring_here = cursor_here && focused;
                let section_count = section_counts.get(&id).copied().unwrap_or(0);
                let slide = if accordion || style == SidebarStyle::FloatingPanel {
                    1.0
                } else {
                    self.header_amount(&id, mode, now).max(f32::from(ring_here))
                };
                let icon_text = match group {
                    Some(g) => ui.sidebar.group_icon(g),
                    None => ui.sidebar.section_icon(&id),
                }
                .map(str::to_owned);
                // The chevron is out of the flow: it fades in over the heading's left edge
                // while the title (and icon) slide right to make room, so nothing around
                // the heading moves and the row's height stays the same.
                let chevron = div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(8. - 6. * (1.0 - slide)
                        + if pills {
                            11.
                        } else if cards {
                            4.
                        } else {
                            0.
                        }))
                    .flex()
                    .items_center()
                    .opacity(slide)
                    .debug_selector({
                        let id = id.clone();
                        move || format!("sidebar-chevron-{id}")
                    })
                    .child(crate::pane::chevron_glyph(open, color(muted)));
                let lead = div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .min_h(px(14.))
                    .debug_selector({
                        let id = id.clone();
                        move || format!("sidebar-heading-{id}")
                    })
                    .pl(px(CHEVRON_ROOM * slide + if pills { 11. } else { 0. }))
                    .when(pills, |lead| {
                        lead.pr(px(11.)).py(px(1.)).rounded_full().bg(color_alpha(theme.role("accent"), 42))
                    })
                    .when_some(icon_text, |lead, text| {
                        lead.child(
                            div()
                                .flex_none()
                                .text_size(px(11.))
                                .debug_selector({
                                    let id = id.clone();
                                    move || format!("sidebar-header-icon-{id}")
                                })
                                .child(text),
                        )
                    })
                    .child(title.clone());
                kinds.push((group.map(DropTarget::Group), None));
                let i = index;
                index += 1;
                let cursor = i == self.cursor;
                let look = looks.next().expect("one look per selectable row");
                children.push(
                    div()
                        .id(SharedString::from(format!("sidebar-header-{id}")))
                        .debug_selector({
                            let id = id.clone();
                            move || format!("sidebar-header-{id}")
                        })
                        .role(Role::Button)
                        .aria_label(look.spoken)
                        .aria_expanded(!collapsed)
                        .aria_position_in_set(i + 1)
                        .aria_size_of_set(selectable_count)
                        .aria_selected(cursor)
                        .when(cursor && self.focus.is_focused(window), |header| header.aria_active_descendant())
                        .when(look.ring, |header| header.shadow(cursor_ring(&theme)))
                        .mt(px(if i == 0 { first_gap } else { header_gap }))
                        .mb(px(if pills { 5. } else { 0. }))
                        .px_2()
                        .py(px(if cards { 5. } else { 3. }))
                        .when(!cards, |header| header.rounded_md())
                        .when(cards, |header| header.pl(px(12.)).pb(px(7.)))
                        .when(accordion, |header| {
                            header
                                .ml(px(-8.))
                                .mr(px(-8.))
                                .px(px(12.))
                                .py(px(4.))
                                .rounded_none()
                                .bg(color(card_strip))
                                .border_t_1()
                                .border_color(color(card_border))
                        })
                        .when_some(parts[i].filter(|_| cards), |header, part| {
                            let alone = matches!(part, CardPart::Strip { alone: true });
                            let header = header
                                .bg(color(card_strip))
                                .border_1()
                                .border_color(color(card_border))
                                .rounded_tl(px(CARD_RADIUS))
                                .rounded_tr(px(CARD_RADIUS));
                            if alone {
                                header.rounded_bl(px(CARD_RADIUS)).rounded_br(px(CARD_RADIUS))
                            } else {
                                header.border_b_0()
                            }
                        })
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_size(px(if cards || accordion { 12. } else { 11. }))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(color(if cards || accordion { fg } else { muted }))
                        .cursor_pointer()
                        .hover(move |style| style.text_color(color(fg)))
                        .on_hover({
                            let id = id.clone();
                            cx.listener(move |this, hovered: &bool, _, cx| this.set_header_hover(&id, *hovered, cx))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.cursor = i;
                            this.cursor_key = None;
                            set_collapsed(id.clone(), !collapsed, cx);
                        }))
                        .relative()
                        .child(chevron)
                        .child(lead)
                        .when(ledger, |header| {
                            header.child(div().flex_1().h(px(1.)).ml_1().bg(crate::style::hairline(&theme)))
                        })
                        .when(accordion, |header| {
                            header.child(
                                div()
                                    .ml_auto()
                                    .font_weight(FontWeight::NORMAL)
                                    .text_color(color(muted))
                                    .child(section_count.to_string()),
                            )
                        })
                        .when_some(group, |header, g| {
                            header
                                .when(
                                    !self.dragging_favorite.get() && self.drop_target == Some(DropTarget::Group(g)),
                                    |header| header.child(drop_outline(&theme).inset_0()),
                                )
                                .on_drop(cx.listener(move |_, dragged: &DraggedFavorite, _, cx| {
                                    ui::edit_sidebar(SidebarEdit::Move { from: dragged.slot, to: (g, 0) }, cx);
                                }))
                        })
                        .into_any_element(),
                );
                // Full: the closing rows fade away while the gap shrinks. Low has no
                // room to fade in (gaps snap), so its rows go at once.
                if let Some((motion, t)) = moving.filter(|(motion, _)| mode == Effective::Full && !motion.opening)
                    && !motion.ghost.is_empty()
                {
                    let left = 1.0 - t;
                    let each = motion.height / motion.ghost.len() as f32;
                    ghost_child = Some(children.len());
                    children.push(
                        div()
                            .h(px(motion.height * left))
                            .overflow_hidden()
                            .opacity(left)
                            .debug_selector(|| "sidebar-section-ghost".into())
                            .children(motion.ghost.iter().map(|label| {
                                div()
                                    .h(px(each))
                                    .px_2()
                                    .flex()
                                    .items_center()
                                    .text_size(px(13.))
                                    .text_color(color(fg))
                                    .child(label.clone())
                            }))
                            .into_any_element(),
                    );
                }
                continue;
            }
            let i = index;
            index += 1;
            let favorite_slot = match &row {
                Row::Favorite(slot, _) => Some(*slot),
                _ => None,
            };
            // Plugin rows are navigable but never accept favorite or file drops.
            let droppable = matches!(&row, Row::Favorite(..) | Row::Recent(_) | Row::Volume(_))
                || matches!(&row, Row::PluginItem { row, .. } if row.folder_path().is_some());
            kinds.push((droppable.then_some(DropTarget::Row(i)), favorite_slot));
            let path = row.path().map(Path::to_path_buf).unwrap_or_default();
            let navigable = row.location().is_some();
            let path_less = row.path().is_none();
            let current_description = if row.path().is_some() {
                "current folder"
            } else if matches!(&row, Row::PluginItem { row, .. } if matches!(row.destination, SectionDestination::Tag { .. }))
            {
                "current tag"
            } else {
                "current saved query"
            };
            let cursor = i == self.cursor;
            let mut look = looks.next().expect("one look per selectable row");
            let pin = row.path().and_then(|path| pin_summaries.get(path)).filter(|_| matches!(row, Row::Favorite(..)));
            if let Some(pin) = pin {
                if look.hint.is_none() {
                    look.hint = Some(pin.branch.clone());
                }
                look.spoken.push_str(", ");
                look.spoken.push_str(&pin.spoken);
            }
            // Favorites draw Finder's own icons (Home, Desktop, Documents…), an icon
            // pack's match winning. A favorite's custom text icon wins over both,
            // and a favorite with a palette color keeps the tintable line folder
            // (a native icon cannot be recolored). Recent keeps its clock.
            let (glyph, line_icon, tint) = match &row {
                Row::Favorite(_, f) => {
                    let tint = f.color.as_deref().and_then(|c| favorite_color(c, &theme));
                    (f.icon.clone(), (f.icon.is_none() && tint.is_some()).then_some(crate::icons::Glyph::Folder), tint)
                }
                Row::Recent(_) => (None, Some(crate::icons::Glyph::Clock), None),
                Row::PluginItem { row: item, .. } => (
                    None,
                    if matches!(item.destination, SectionDestination::Tag { .. }) {
                        Some(crate::icons::Glyph::Tag)
                    } else {
                        // A saved query has no folder to ask Finder about.
                        path_less.then_some(crate::icons::Glyph::Folder)
                    },
                    None,
                ),
                _ => (None, None, None),
            };
            let volume_label = match &row {
                Row::Volume(v) => Some(format!("{} free of {}", human_bytes(v.free_bytes), human_bytes(v.total_bytes))),
                _ => None,
            };
            let receive_tint = self.receive.as_ref().filter(|(target, _)| target == &path).and_then(|(_, started)| {
                let phases = motion::phases(mode, Fx::Receive);
                effect_tint(motion::look_in(&phases, mode, Fx::Receive, *started, now), &theme)
            });
            let mut item = div()
                .id(SharedString::from(format!("sidebar-{i}")))
                .debug_selector(move || format!("sidebar-{i}"))
                .role(Role::ListBoxOption)
                .aria_label(look.spoken.clone())
                .aria_position_in_set(i + 1)
                .aria_size_of_set(selectable_count)
                .aria_selected(cursor)
                .when(look.current, |row| row.aria_description(current_description))
                .when(cursor, |row| row.aria_active_descendant())
                .when_some(parts[i], |row, part| match part {
                    CardPart::Body { first, last } => row
                        .mx(px(CARD_ROW_INSET))
                        .when(first, |row| row.mt(px(CARD_PAD_TOP)))
                        .when(last, |row| row.mb(px(CARD_PAD_BOTTOM))),
                    CardPart::Strip { .. } => row,
                })
                .px_2()
                .py(px(3.))
                .rounded_md()
                .when(accordion && matches!(parts[i], Some(CardPart::Body { .. })), |row| {
                    row.ml(px(-8.)).mr(px(-8.)).pl(px(26.)).pr(px(12.)).rounded_none()
                })
                .flex()
                .flex_col()
                .relative()
                .when(look.drop, |row| {
                    row.child(drop_outline(&theme).inset_0().opacity(crate::drag_drop::cue_opacity(cx, &path)))
                })
                .when(look.ring, |row| row.shadow(cursor_ring(&theme)))
                .when(self.menu.is_some_and(|(m, _)| m == i), |row| row.child(drop_outline(&theme).inset_0()))
                .when_some(receive_tint, |row, tint| row.child(div().absolute().inset_0().rounded_md().bg(tint)))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| this.open_menu(i, window, cx)),
                )
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    if event.modifiers().control {
                        this.open_menu(i, window, cx);
                        return;
                    }
                    this.cursor = i;
                    if navigable {
                        this.navigate(i, SidebarNav::Open, true, cx);
                    }
                    cx.notify();
                }));
            if let Row::Favorite(slot, _) = &row {
                let slot = *slot;
                let dragging = self.dragging_favorite.clone();
                item = item.on_drag(
                    DraggedFavorite { slot, label: look.label.clone().into() },
                    move |dragged, _, _, cx| {
                        dragging.set(true);
                        cx.new(|_| dragged.clone())
                    },
                );
                // The one insertion line: above the first favorite of a group for slot 0,
                // below a favorite for the boundary after it. No row changes height.
                let line = self.insert_slot.and_then(|target| {
                    if target == slot && slot.1 == 0 {
                        Some(false)
                    } else if target == (slot.0, slot.1 + 1) {
                        Some(true)
                    } else {
                        None
                    }
                });
                if let Some(at_bottom) = line {
                    item = item.child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .h(px(2.))
                            .rounded_full()
                            .bg(color(accent))
                            .when(at_bottom, |line| line.bottom_0())
                            .when(!at_bottom, |line| line.top_0()),
                    );
                }
                let target_slot = self.insert_slot;
                item = item.on_drop(cx.listener(move |_, dragged: &DraggedFavorite, _, cx| {
                    if let Some(to) = target_slot {
                        ui::edit_sidebar(SidebarEdit::Move { from: dragged.slot, to }, cx);
                    }
                }));
            }
            let icon_element = match glyph {
                Some(text) => div()
                    .w(px(16.))
                    .flex()
                    .justify_center()
                    .text_size(px(13.))
                    .text_color(color(tint.unwrap_or(muted)))
                    .child(text)
                    .into_any_element(),
                None => match line_icon {
                    Some(glyph) => {
                        // Tag and Saved Query rows have no folder path: an icon pack's
                        // `[generic].folder` must not replace their own glyph.
                        let packed = (matches!(row, Row::Favorite(..) | Row::PluginItem { .. })
                            && row.path().is_some())
                        .then(|| icons.resolve(&path, true, false))
                        .flatten();
                        if let Some(packed) = packed {
                            let icon_tint = tint.map(color).unwrap_or_else(|| color(packed.color.rgb(&theme)));
                            div()
                                .w(px(16.))
                                .flex()
                                .justify_center()
                                .debug_selector(move || format!("sidebar-pack-icon-{i}"))
                                .child(crate::icons::pack_glyph(&packed, 15., icon_tint))
                                .into_any_element()
                        } else {
                            let icon_tint = tint.map(color).unwrap_or_else(|| color(theme.role("link")));
                            let name = match glyph {
                                crate::icons::Glyph::Folder => "folder-icon",
                                crate::icons::Glyph::Clock => "clock-icon",
                                _ => "line-icon",
                            };
                            div()
                                .w(px(16.))
                                .flex()
                                .justify_center()
                                .debug_selector(move || format!("sidebar-{name}-{i}"))
                                .child(crate::icons::glyph_color(glyph, 15., icon_tint))
                                .into_any_element()
                        }
                    }
                    None if navigable => {
                        let packed = matches!(row, Row::Favorite(..) | Row::PluginItem { .. })
                            .then(|| icons.resolve(&path, true, false))
                            .flatten();
                        if let Some(packed) = packed {
                            let icon_tint = color(packed.color.rgb(&theme));
                            div()
                                .w(px(16.))
                                .flex()
                                .justify_center()
                                .debug_selector(move || format!("sidebar-pack-icon-{i}"))
                                .child(crate::icons::pack_glyph(&packed, 15., icon_tint))
                                .into_any_element()
                        } else {
                            div()
                                .w(px(16.))
                                .debug_selector(move || format!("sidebar-native-icon-{i}"))
                                .children(icon(&path, true, 16).map(|i| img(i).size(px(16.))))
                                .into_any_element()
                        }
                    }
                    None => div().w(px(16.)).into_any_element(),
                },
            };
            let mut line = div().flex().items_center().h(px(20.)).child(div().mr(px(8.)).child(icon_element)).child(
                div()
                    .flex_shrink_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(13.))
                    .text_color(color(fg))
                    .child(look.label.clone()),
            );
            match &look.hint {
                Some(hint) => {
                    line = line.child(
                        div()
                            .flex_1()
                            .flex_basis(px(0.))
                            .min_w_0()
                            .ml(px(6.))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(12.))
                            .text_color(color(subtle))
                            .child(hint.clone()),
                    );
                }
                None => line = line.child(div().flex_1()),
            }
            if let Some(pin) = pin {
                line = line.child(with_tooltip(
                    div()
                        .id(("git-pin", i))
                        .flex_none()
                        .ml(px(6.))
                        .text_size(px(11.))
                        .text_color(color(muted))
                        .role(Role::Button)
                        .aria_label(format!("Review Git changes for {}", look.label))
                        .on_click({
                            let path = path.clone();
                            move |_, window, cx| {
                                cx.stop_propagation();
                                crate::window::git_review::open_folder(path.clone(), window, cx);
                            }
                        })
                        .child(pin.badge.clone()),
                    pin.spoken.clone(),
                    None,
                ));
            }
            if let Some(badge) = &look.badge {
                line = line.child(
                    div()
                        .w(px(32.))
                        .flex_shrink_0()
                        .ml(px(6.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(11.))
                        .text_color(color(muted))
                        .child(badge.clone()),
                );
            }
            if let Some(right) = &look.right {
                line = line.child(
                    div().flex_shrink_0().ml(px(6.)).text_size(px(11.)).text_color(color(subtle)).child(right.clone()),
                );
            }
            if let Row::Volume(v) = &row
                && v.ejectable
            {
                let volume = v.clone();
                line = line.child(
                    div()
                        .id(SharedString::from(format!("eject-{i}")))
                        .ml(px(8.))
                        .flex_shrink_0()
                        .role(Role::Button)
                        .aria_label(format!("Eject {}", display_text(&v.name)))
                        .px_1()
                        .rounded_sm()
                        .text_color(color(muted))
                        .hover(|s| s.text_color(color(fg)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.eject(volume.clone(), cx);
                        }))
                        .child("⏏"),
                );
            }
            item = item.child(line);
            if let Some((used, tone)) = look.bar {
                let fill = match tone {
                    BarTone::Warning => theme.role("warning"),
                    BarTone::Neutral => subtle,
                };
                item = item.child(
                    div()
                        .ml(px(24.))
                        .mt(px(2.))
                        .h(px(3.))
                        .rounded_full()
                        .bg(color(theme.role("surface-raised")))
                        .child(div().h_full().rounded_full().w(gpui::relative(used)).bg(color(fill))),
                );
            }
            if let Some(label) = volume_label {
                item = with_tooltip(item, label, None);
            }
            // A favorite that changed places glides in from its old place.
            let item = match (&row, &self.reorder) {
                (Row::Favorite(_, favorite), Some(reorder)) => {
                    match reorder.offsets.get(&favorite.path).zip(motion::progress(
                        mode,
                        reorder.started,
                        now,
                        motion::ms::REORDER,
                        motion::ms::REORDER,
                    )) {
                        Some((offset, t)) => item.relative().top(px(offset * (1.0 - t))),
                        None => item,
                    }
                }
                _ => item,
            };
            // Rows of a section that is opening grow and fade in (Low only fades).
            let item = match current_section.as_deref().and_then(&section_progress) {
                Some((motion, t)) if motion.opening => div()
                    .overflow_hidden()
                    .opacity(t)
                    .when(mode == Effective::Full, |wrap| wrap.max_h(px(SECTION_ROW_MAX * t)))
                    .child(item)
                    .into_any_element(),
                _ => item.into_any_element(),
            };
            children.push(item);
        }

        let mut context = "Sidebar".to_owned();
        if vim {
            context.push_str(" vim");
        }
        let layout = self.layout.clone();
        let visible_sections = self.visible_sections.clone();
        div()
            .on_children_prepainted(move |bounds, window, _| {
                // The first child is the current-folder fill; the rest are rows.
                let mut bounds = bounds.into_iter();
                for _ in 0..backdrop_count {
                    bounds.next();
                }
                let _fill = bounds.next();
                let was_empty = layout.borrow().is_empty();
                let mut bounds: Vec<_> = bounds.collect();
                // A closing section's ghost rows are not a row of the list.
                if let Some(ghost) = ghost_child
                    && ghost < bounds.len()
                {
                    bounds.remove(ghost);
                }
                *layout.borrow_mut() = kinds
                    .iter()
                    .copied()
                    .zip(bounds.iter().copied())
                    .map(|((target, favorite), bounds)| (target, favorite, bounds))
                    .collect();
                let mut visible = HashSet::new();
                let viewport = window.viewport_size().height;
                for (bounds, section) in bounds.iter().zip(section_of.iter()) {
                    let Some(section) = section else { continue };
                    if bounds.bottom() > px(0.) && bounds.top() < viewport {
                        visible.insert(section.clone());
                    }
                }
                if *visible_sections.borrow() != visible {
                    *visible_sections.borrow_mut() = visible;
                    window.on_next_frame(|window, _| window.refresh());
                }
                // The fill needs the bounds from this first layout; draw it next frame.
                if was_empty {
                    window.on_next_frame(|window, _| window.refresh());
                }
            })
            .id("sidebar")
            .role(Role::ListBox)
            .aria_label("Sidebar")
            // keys::context_stack (KeyScope::Sidebar) mirrors this.
            .key_context(context.as_str())
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::move_up))
            .on_action(cx.listener(Self::move_down))
            .on_action(cx.listener(Self::open))
            .on_action(cx.listener(Self::remove))
            .on_action(cx.listener(Self::favorite_up))
            .on_action(cx.listener(Self::favorite_down))
            .on_action(cx.listener(Self::previous_group))
            .on_action(cx.listener(Self::next_group))
            .on_action(cx.listener(Self::collapse_section))
            .on_action(cx.listener(Self::eject_cursor))
            .debug_selector(|| "sidebar".into())
            .w_full()
            .flex_none()
            .h_full()
            .px_2()
            .pb_2()
            .overflow_y_scroll()
            // The window paints the column's tone and its right border, so the traffic lights'
            // strip above shares both.
            .flex()
            .flex_col()
            .gap(px(if cards { 0. } else { 1. }))
            .children(backdrops)
            .child(fill)
            .children(children)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(id: &str, collapsed: bool) -> Row {
        Row::SectionHeader { id: id.into(), title: id.into(), collapsed }
    }

    fn recent(name: &str) -> Row {
        Row::Recent(PathBuf::from(format!("/{name}")))
    }

    #[gpui::test]
    fn background_names_only_cache_sidebar_destinations_still_present(cx: &mut gpui::TestAppContext) {
        let old = [recent("old")];
        let new = [recent("new")];
        let sidebar = cx.update(|cx| cx.new(Sidebar::new));
        sidebar.update(cx, |this, cx| {
            this.refresh_names(&old, cx);
            assert!(this.names.is_empty(), "the first frame uses the path");
            assert_eq!(this.display_name(Path::new("/old")), "old");
            this.refresh_names(&new, cx);
        });
        cx.run_until_parked();
        sidebar.update(cx, |this, cx| {
            assert!(!this.names.contains_key(Path::new("/old")));
            this.refresh_names(&new, cx);
        });
        cx.run_until_parked();
        sidebar.read_with(cx, |this, _| {
            let names = this.names.get(Path::new("/new")).expect("worker loaded the current destination");
            assert_eq!(names.last().map(String::as_str), Some("new"));
        });
    }

    #[test]
    fn a_swapped_pair_of_favorites_starts_each_one_at_the_other_s_place() {
        let path = |name: &str| (0usize, PathBuf::from(name));
        let (before, after) = (vec![path("a"), path("b"), path("c")], vec![path("a"), path("c"), path("b")]);
        let tops: HashMap<_, _> =
            [("a", 100.0), ("b", 126.0), ("c", 152.0)].into_iter().map(|(n, t)| (PathBuf::from(n), t)).collect();
        let heights: HashMap<_, _> = ["a", "b", "c"].into_iter().map(|n| (PathBuf::from(n), 24.0)).collect();
        let offsets = reorder_offsets(&before, &after, &tops, &heights, 2.0);
        assert_eq!(offsets.get(&PathBuf::from("c")), Some(&26.0), "c was 26 px below where it now sits");
        assert_eq!(offsets.get(&PathBuf::from("b")), Some(&-26.0), "b was 26 px above");
        assert!(!offsets.contains_key(&PathBuf::from("a")), "a did not move");
        assert!(reorder_offsets(&before, &before, &tops, &heights, 2.0).is_empty(), "no change");
        assert!(
            reorder_offsets(&before, &[path("a")], &tops, &heights, 2.0).is_empty(),
            "added or removed rows don't glide"
        );
    }

    #[test]
    fn card_parts_make_headings_strips_and_rows_the_body() {
        let rows =
            vec![header("a", false), recent("1"), recent("2"), header("b", true), header("c", false), recent("3")];
        assert_eq!(
            card_parts(&rows),
            vec![
                Some(CardPart::Strip { alone: false }),
                Some(CardPart::Body { first: true, last: false }),
                Some(CardPart::Body { first: false, last: true }),
                Some(CardPart::Strip { alone: true }),
                Some(CardPart::Strip { alone: false }),
                Some(CardPart::Body { first: true, last: true }),
            ]
        );
        assert_eq!(card_parts(&[recent("x")]), vec![None], "a row outside any group has no card");
    }

    #[test]
    fn section_spans_count_the_rows_up_to_the_next_header() {
        let rows = vec![recent("a"), header("recent", false), recent("b"), recent("c"), header("locations", true)];
        let spans = section_spans(&rows);
        assert_eq!(
            spans,
            vec![
                SectionSpan { id: "recent".into(), collapsed: false, start: 2, len: 2 },
                SectionSpan { id: "locations".into(), collapsed: true, start: 5, len: 0 },
            ],
            "the favorite before and the row after a plain header belong to no section"
        );
    }

    #[test]
    fn only_a_flipped_collapsed_flag_is_a_section_transition() {
        let open = vec![header("recent", false), recent("b"), recent("c")];
        let shut = vec![header("recent", true)];
        let (open_spans, shut_spans) = (section_spans(&open), section_spans(&shut));
        let closing = section_transitions(&open_spans, &shut_spans);
        assert_eq!(closing.len(), 1);
        assert_eq!(
            (closing[0].0.as_str(), closing[0].1, closing[0].2.len),
            ("recent", false, 2),
            "closing remembers its rows"
        );
        let opening = section_transitions(&shut_spans, &open_spans);
        assert_eq!((opening[0].1, opening[0].2.len), (true, 0));
        assert!(section_transitions(&open_spans, &open_spans).is_empty(), "no change, no motion");
        assert!(section_transitions(&[], &open_spans).is_empty(), "a section that just appeared does not move");
    }

    #[test]
    fn sizes_read_like_finder() {
        assert_eq!(human_bytes(512), "512 bytes");
        assert_eq!(human_bytes(1_500_000), "1.5 MB");
        assert_eq!(human_bytes(494_384_795_648), "494.4 GB");
    }

    #[test]
    fn short_sizes_for_the_sidebar() {
        assert_eq!(short_bytes(182_400_000_000), "182 GB");
        assert_eq!(short_bytes(4_200_000_000), "4.2 GB");
        assert_eq!(short_bytes(980_000_000), "980 MB");
        assert_eq!(short_bytes(1_000_000_000_000), "1.0 TB");
        assert_eq!(short_bytes(0), "0 bytes");
    }

    #[test]
    fn short_sizes_round_before_choosing_the_unit() {
        // 999.949 KB and 999.95 KB both round to 1000 KB, which is 1.0 MB.
        assert_eq!(short_bytes(999_499), "999 KB");
        assert_eq!(short_bytes(999_949), "1.0 MB");
        assert_eq!(short_bytes(999_950), "1.0 MB");
        assert_eq!(short_bytes(1_000_000), "1.0 MB");
        // The same boundary at GB -> TB, and the 9.96 GB -> 10 GB case from U8 C7.
        assert_eq!(short_bytes(999_400_000_000), "999 GB");
        assert_eq!(short_bytes(999_949_000_000), "1.0 TB");
        assert_eq!(short_bytes(999_950_000_000), "1.0 TB");
        assert_eq!(short_bytes(1_000_000_000_000), "1.0 TB");
        assert_eq!(short_bytes(9_960_000_000), "10 GB");
        assert_eq!(short_bytes(999_600_000_000), "1.0 TB");
    }

    #[test]
    fn disk_turns_warning_at_ninety_percent() {
        assert_eq!(bar_tone(0.899), BarTone::Neutral);
        assert_eq!(bar_tone(0.9), BarTone::Warning);
    }

    #[test]
    fn colors_by_name_role_or_hex() {
        let theme = crow_theme::fallback();
        assert_eq!(favorite_color("#112233", &theme), Some(crow_theme::Rgb(0x112233)));
        assert_eq!(favorite_color("accent", &theme), Some(theme.role("accent")));
        assert_eq!(favorite_color("blue", &theme), theme.slot("base0D"));
        assert_eq!(favorite_color("chartreuse", &theme), None);
    }
}
