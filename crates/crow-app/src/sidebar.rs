//! The sidebar: groups of favorites from `sidebar.toml`, Recent (frecency, with
//! zoxide's list when installed) and Locations (mounted volumes with free space
//! and Eject).
//!
//! Favorites are reordered by dragging them or with Cmd-Option-Up/Down. Files
//! dropped on any row move or copy into that folder; folders dropped on a group's
//! title become favorites.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use crow_core::motion::{self, Effective, Fx};
use crow_core::{Favorite, SidebarEdit, Slot};
use crow_fs::ops::Request;
use gpui::{
    Animation, AnimationExt, Bounds, ClickEvent, Context, EventEmitter, ExternalPaths, FocusHandle, Focusable,
    FontWeight, IntoElement, MouseButton, MouseDownEvent, Pixels, Point, Render, Role, SharedString, SpringAnimation,
    SpringConfig, Window, div, img, prelude::*, px,
};

use crate::actions::{
    Eject, MoveDown, MoveFavoriteDown, MoveFavoriteUp, MoveUp, NextGroup, OpenSelection, PreviousGroup,
    RemoveFromSidebar,
};
use crate::chrome::with_tooltip;
use crate::context_menu::{SidebarMenuKind, SidebarMenuState};
use crate::palette::CommandId;
use crate::pane::DraggedSelection;
use crate::style::{color, current_location, cursor_ring, display_text, drop_outline, effect_tint, icon};
use crate::ui::{self, Ui};

pub enum SidebarEvent {
    /// The cursor moved onto a folder: show it in the center, keep focus here.
    Show(PathBuf),
    Open(PathBuf),
    /// The context menu's Open in New Tab.
    OpenInNewTab(PathBuf),
    /// Files dropped on a row.
    Run(Request),
    Message(SharedString, bool),
}

impl EventEmitter<SidebarEvent> for Sidebar {}

/// What a drag would drop on in the sidebar: a selectable row or a favorite group title.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DropTarget {
    Row(usize),
    Group(usize),
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
}

#[derive(Clone, Debug, PartialEq)]
enum Row {
    /// A section title; `Some(group)` for favorites groups, which accept folders.
    Header(String, Option<usize>),
    Favorite(Slot, Favorite),
    Recent(PathBuf),
    Volume(crow_macos::Volume),
}

impl Row {
    fn path(&self) -> Option<&Path> {
        match self {
            Row::Header(..) => None,
            Row::Favorite(_, f) => Some(&f.path),
            Row::Recent(p) => Some(p),
            Row::Volume(v) => Some(&v.path),
        }
    }
}

fn rows(ui: &Ui) -> Vec<Row> {
    let mut rows = Vec::new();
    for (g, (name, favorites)) in ui.favorites().into_iter().enumerate() {
        rows.push(Row::Header(name, Some(g)));
        rows.extend(favorites.into_iter().enumerate().map(|(i, f)| Row::Favorite((g, i), f)));
    }
    if ui.sidebar.recent.show && !ui.recent_list.is_empty() {
        rows.push(Row::Header("Recent".into(), None));
        rows.extend(ui.recent_list.iter().cloned().map(Row::Recent));
    }
    if ui.sidebar.locations.show && !ui.volumes.is_empty() {
        rows.push(Row::Header("Locations".into(), None));
        rows.extend(ui.volumes.iter().cloned().map(Row::Volume));
    }
    rows
}

/// What a selectable row shows, separate from how it is drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct RowLook {
    pub label: String,
    pub hint: Option<String>,
    pub right: Option<String>,
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

pub struct Sidebar {
    focus: FocusHandle,
    /// Index into the selectable rows (favorites, recents, volumes).
    cursor: usize,
    /// The folder the window shows, highlighted when it's in the sidebar.
    current: PathBuf,
    /// Where each direct child of the list sits, captured when it is prepainted.
    /// A cell so prepaint can fill it without scheduling another render.
    layout: RowLayout,
    /// What a drag outside the list would drop on, drawn from state rather than a
    /// drag-over style so the highlight cannot flicker between frames.
    drop_target: Option<DropTarget>,
    /// A favorite is being reordered: its insertion line is the indicator, not a row.
    dragging_favorite: Rc<Cell<bool>>,
    /// Where a dragged favorite would be inserted: group and index before the move.
    insert_slot: Option<Slot>,
    /// Finder's display names per path, volume first. Filled on demand; `displayNameAtPath`
    /// runs once per path instead of once per row per render.
    names: RefCell<HashMap<PathBuf, Rc<[String]>>>,
    /// The paths in `names`, in row order, so a changed sidebar prunes stale entries.
    name_paths: RefCell<Vec<PathBuf>>,
    /// A row pulsing because a move landed in its folder, and when it started.
    receive: Option<(PathBuf, Instant)>,
    /// Bumped per pulse, so an older timer cannot clear a newer one.
    receive_generation: u64,
    /// The row whose context menu is open (its index and what it is), outlined meanwhile.
    menu: Option<(usize, SidebarMenuKind)>,
}

impl Focusable for Sidebar {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Sidebar {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            cursor: 0,
            current: PathBuf::new(),
            layout: Rc::default(),
            drop_target: None,
            dragging_favorite: Rc::new(Cell::new(false)),
            insert_slot: None,
            names: RefCell::new(HashMap::new()),
            name_paths: RefCell::new(Vec::new()),
            receive: None,
            receive_generation: 0,
            menu: None,
        }
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

    pub fn set_current(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        if self.current != dir {
            self.current = dir;
            cx.notify();
        }
    }

    fn selectable(cx: &gpui::App) -> Vec<Row> {
        rows(cx.global::<Ui>()).into_iter().filter(|r| !matches!(r, Row::Header(..))).collect()
    }

    /// The selectors a fuzzer can click in the sidebar.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn debug_selectors(&self, cx: &gpui::App) -> Vec<String> {
        let mut out = vec!["sidebar".to_owned()];
        out.extend((0..Self::selectable(cx).len()).map(|i| format!("sidebar-{i}")));
        out
    }

    /// The sidebar's focus handle, for the fuzzer's focus invariant.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn debug_focus_handle(&self, cx: &gpui::App) -> gpui::FocusHandle {
        self.focus_handle(cx)
    }

    /// Bounds for selectable row `i`, from the prepaint layout.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn debug_row_bounds(&self, i: usize, cx: &gpui::App) -> Option<gpui::Bounds<gpui::Pixels>> {
        let mut selectable = 0;
        for (layout_index, row) in rows(cx.global::<Ui>()).iter().enumerate() {
            if matches!(row, Row::Header(..)) {
                continue;
            }
            if selectable == i {
                return self.layout.borrow().get(layout_index).map(|(_, _, bounds)| *bounds);
            }
            selectable += 1;
        }
        None
    }

    /// Finder's display names for `path`, volume first. Cached: the ancestors are
    /// filesystem calls on the main thread, so each path is asked once.
    fn components(&self, path: &Path) -> Rc<[String]> {
        if let Some(names) = self.names.borrow().get(path) {
            return names.clone();
        }
        let names: Rc<[String]> = crow_macos::display_components(path)
            .into_iter()
            .map(|(name, _)| display_text(&name).into_owned())
            .collect::<Vec<_>>()
            .into();
        self.names.borrow_mut().insert(path.to_path_buf(), names.clone());
        names
    }

    /// Finder's name for a folder ("Macintosh HD" for `/`).
    fn display_name(&self, path: &Path) -> String {
        self.components(path).last().cloned().unwrap_or_else(|| display_text(&path.display().to_string()).into_owned())
    }

    /// Drop cached names for paths that left the sidebar.
    fn prune_names(&self, rows: &[Row]) {
        let paths: Vec<PathBuf> = rows.iter().filter_map(|r| r.path().map(Path::to_path_buf)).collect();
        if *self.name_paths.borrow() == paths {
            return;
        }
        let keep: HashSet<&Path> = paths.iter().map(PathBuf::as_path).collect();
        self.names.borrow_mut().retain(|path, _| keep.contains(path.as_path()));
        *self.name_paths.borrow_mut() = paths;
    }

    /// What every selectable row shows, in row order. Headers get no look.
    fn looks(&self, rows: &[Row], focused: bool, ui: &Ui) -> Vec<RowLook> {
        self.prune_names(rows);
        let vim = ui.settings.keymap == crow_core::KeymapLayer::Vim;
        let selectable: Vec<&Row> = rows.iter().filter(|row| !matches!(row, Row::Header(..))).collect();
        let labels: Vec<String> = selectable
            .iter()
            .map(|row| match row {
                Row::Favorite(_, f) => {
                    f.name.as_deref().map_or_else(|| self.display_name(&f.path), |name| display_text(name).into_owned())
                }
                Row::Recent(path) => self.display_name(path),
                Row::Volume(v) => display_text(&v.name).into_owned(),
                Row::Header(..) => unreachable!(),
            })
            .collect();

        // Twins are compared across Favorites and Recent, never volumes.
        let mut ancestors_for: Vec<Option<Vec<String>>> = Vec::with_capacity(selectable.len());
        for row in &selectable {
            let ancestors = match row {
                Row::Favorite(..) | Row::Recent(..) => {
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
                _ => None,
            };
            let (hint, spoken) = match &hint_by_row[i] {
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
                        _ => label.clone(),
                    };
                    (None, spoken)
                }
            };
            looks.push(RowLook {
                label,
                hint,
                right,
                spoken,
                current: row.path().is_some_and(|path| path == self.current),
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
        let rows = rows(ui);
        self.looks(&rows, self.focus.is_focused(window), ui)
    }

    fn cursor_row(&self, cx: &gpui::App) -> Option<Row> {
        Self::selectable(cx).into_iter().nth(self.cursor)
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
            Row::Header(..) => None,
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
            Row::Header(..) => None,
        }
    }

    /// Paths of the selectable rows, in order.
    #[cfg(test)]
    pub fn paths(cx: &gpui::App) -> Vec<PathBuf> {
        Self::selectable(cx).iter().filter_map(|r| r.path().map(Path::to_path_buf)).collect()
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
        let next = (self.cursor + 1).min(Self::selectable(cx).len().saturating_sub(1));
        if next == self.cursor {
            return;
        }
        self.cursor = next;
        self.show_cursor(cx);
        cx.notify();
    }

    /// The selectable indices of each non-empty group, in sidebar order.
    fn group_rows(cx: &gpui::App) -> Vec<Vec<usize>> {
        let mut groups: Vec<Vec<usize>> = Vec::new();
        let mut index = 0;
        for row in rows(cx.global::<Ui>()) {
            if matches!(row, Row::Header(..)) {
                groups.push(Vec::new());
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
        let groups = Self::group_rows(cx);
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
        if let Some(path) = self.cursor_row(cx).and_then(|row| row.path().map(Path::to_path_buf)) {
            cx.emit(SidebarEvent::Show(path));
        }
    }

    fn open(&mut self, _: &OpenSelection, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.cursor_row(cx).and_then(|r| r.path().map(Path::to_path_buf)) {
            cx.emit(SidebarEvent::Open(path));
        }
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
        let Some(row) = Self::selectable(cx).into_iter().nth(i) else { return };
        let (kind, state) = match &row {
            Row::Favorite((g, n), _) => {
                let len = cx.global::<Ui>().favorites().get(*g).map_or(0, |(_, f)| f.len());
                let state = SidebarMenuState { can_move_up: *n > 0, can_move_down: n + 1 < len, ejectable: false };
                (SidebarMenuKind::Favorite, state)
            }
            Row::Recent(_) => (SidebarMenuKind::Recent, SidebarMenuState::default()),
            Row::Volume(v) => {
                (SidebarMenuKind::Volume, SidebarMenuState { ejectable: v.ejectable, ..SidebarMenuState::default() })
            }
            Row::Header(..) => return,
        };
        let entries = crate::context_menu::sidebar_entries(kind, state, cx);
        self.menu = Some((i, kind));
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

    /// The menu closed with the line at `chosen` picked (an index into its slots), or none.
    fn menu_closed(&mut self, chosen: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let Some((i, kind)) = self.menu.take() else { return };
        cx.notify();
        let Some((id, _)) = chosen.and_then(|ix| crate::context_menu::sidebar_slots(kind).get(ix).copied().flatten())
        else {
            return;
        };
        let Some(path) = Self::selectable(cx).into_iter().nth(i).and_then(|row| row.path().map(Path::to_path_buf))
        else {
            return;
        };
        match id {
            CommandId::Open => {
                self.cursor = i;
                cx.emit(SidebarEvent::Open(path));
            }
            CommandId::NewTab => cx.emit(SidebarEvent::OpenInNewTab(path)),
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

    /// Files dropped on a row's folder: moved on the same disk, copied to another, Option copies.
    fn drop_on(&mut self, into: PathBuf, paths: &ExternalPaths, window: &mut Window, cx: &mut Context<Self>) {
        // Move vs copy depends on the volumes, which means a stat: ask off the main thread.
        let paths = paths.paths().to_vec();
        let copy = window.modifiers().alt;
        let task = cx.background_spawn(async move { crate::operations::drop_request(&paths, into, copy) });
        cx.spawn(async move |this, cx| {
            let request = task.await;
            this.update(cx, |_, cx| {
                if let Some(request) = request {
                    cx.emit(SidebarEvent::Run(request));
                }
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
    fn add_dropped_folders(&mut self, at: Slot, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let task = cx.background_spawn(async move { paths.into_iter().filter(|p| p.is_dir()).collect::<Vec<_>>() }); // io-ok: inside background_spawn
        cx.spawn(async move |this, cx| {
            let folders = task.await;
            this.update(cx, |this, cx| this.add_folders(at, folders, cx)).ok();
        })
        .detach();
    }
}

/// The folders in a selection dragged from a pane, which already knows which are folders.
fn folders_of_selection(selection: &DraggedSelection) -> Vec<PathBuf> {
    selection.entries.iter().filter(|(_, dir)| *dir).map(|(path, _)| path.clone()).collect()
}

impl Render for Sidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The receive pulse is drawn from the clock: keep frames coming while it runs.
        if self.receive.is_some() {
            window.request_animation_frame();
        }
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        let mode = ui.feedback.mode;
        let now = Instant::now();
        let vim = ui.settings.keymap == crow_core::KeymapLayer::Vim;
        let rows = rows(ui);
        let focused = self.focus.is_focused(window);
        let selectable_count = rows.iter().filter(|r| !matches!(r, Row::Header(..))).count();
        self.cursor = self.cursor.min(selectable_count.saturating_sub(1));
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
                _ => self.target_at(position),
            };
        } else {
            self.drop_target = None;
            self.insert_slot = None;
            self.dragging_favorite.set(false);
        }
        let current_child = rows.iter().position(|row| row.path() == Some(self.current.as_path()));
        let fill = {
            let layout = self.layout.borrow();
            let current_bounds = current_child.and_then(|ix| layout.get(ix).map(|(_, _, bounds)| *bounds));
            let first_bounds = layout.first().map(|(_, _, bounds)| *bounds);
            match (current_bounds, first_bounds) {
                (Some(current), Some(first)) => {
                    let top = f32::from(current.top() - first.top());
                    let base = div()
                        .absolute()
                        .top(px(top))
                        .left(px(8.))
                        .right(px(8.))
                        .h(current.size.height)
                        .rounded_md()
                        .bg(current_location(&theme))
                        .debug_selector(|| "sidebar-fill".into());
                    match mode {
                        Effective::Full => base
                            .with_spring(
                                "sidebar-fill",
                                SpringAnimation::new(SpringConfig::new(2500.0, 2.0 * 2500.0f32.sqrt(), 1.0))
                                    .to(px(top)),
                                |el, y| el.top(px(f32::from(y))),
                            )
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
        let mut kinds: Vec<(Option<DropTarget>, Option<Slot>)> = Vec::with_capacity(rows.len());
        let mut looks = self.looks(&rows, focused, ui).into_iter();
        let mut index = 0;
        let mut children = Vec::with_capacity(rows.len());
        for row in rows {
            if let Row::Header(title, group) = &row {
                let header = div()
                    .id(SharedString::from(format!("sidebar-header-{title}")))
                    .debug_selector({
                        let title = title.clone();
                        move || format!("sidebar-header-{title}")
                    })
                    .role(Role::Heading)
                    .aria_level(2)
                    .aria_label(title.clone())
                    .px_2()
                    .pt_3()
                    .pb_1()
                    .rounded_md()
                    .relative()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(color(muted))
                    .child(title.clone());
                let header = match *group {
                    Some(g) => header
                        .when(
                            !self.dragging_favorite.get() && self.drop_target == Some(DropTarget::Group(g)),
                            |header| header.child(drop_outline(&theme).inset_0()),
                        )
                        .on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| {
                            let at = this.group_end(g, cx);
                            this.add_dropped_folders(at, paths.paths().to_vec(), cx)
                        }))
                        .on_drop(cx.listener(move |this, selection: &DraggedSelection, _, cx| {
                            let at = this.group_end(g, cx);
                            this.add_folders(at, folders_of_selection(selection), cx)
                        }))
                        .on_drop(cx.listener(move |_, dragged: &DraggedFavorite, _, cx| {
                            ui::edit_sidebar(SidebarEdit::Move { from: dragged.slot, to: (g, 0) }, cx);
                        })),
                    None => header,
                };
                kinds.push((group.map(DropTarget::Group), None));
                children.push(header.into_any_element());
                continue;
            }
            let i = index;
            index += 1;
            let favorite_slot = match &row {
                Row::Favorite(slot, _) => Some(*slot),
                _ => None,
            };
            kinds.push((Some(DropTarget::Row(i)), favorite_slot));
            let path = row.path().map(Path::to_path_buf).unwrap_or_default();
            let cursor = i == self.cursor;
            let look = looks.next().expect("one look per selectable row");
            let (glyph, tint) = match &row {
                Row::Favorite(_, f) => (f.icon.clone(), f.color.as_deref().and_then(|c| favorite_color(c, &theme))),
                _ => (None, None),
            };
            let volume_label = match &row {
                Row::Volume(v) => Some(format!("{} free of {}", human_bytes(v.free_bytes), human_bytes(v.total_bytes))),
                _ => None,
            };
            let target = path.clone();
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
                .when(look.current, |row| row.aria_description("current folder"))
                .when(cursor, |row| row.aria_active_descendant())
                .px_2()
                .py(px(3.))
                .rounded_md()
                .flex()
                .flex_col()
                .relative()
                .when(look.drop, |row| row.child(drop_outline(&theme).inset_0()))
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
                    cx.emit(SidebarEvent::Open(target.clone()));
                }))
                .on_drop({
                    let into = path.clone();
                    cx.listener(move |this, paths: &ExternalPaths, window, cx| match this.insert_slot {
                        Some(at) => this.add_dropped_folders(at, paths.paths().to_vec(), cx),
                        None => this.drop_on(into.clone(), paths, window, cx),
                    })
                })
                .on_drop({
                    let into = path.clone();
                    cx.listener(move |this, selection: &DraggedSelection, window, cx| match this.insert_slot {
                        Some(at) => this.add_folders(at, folders_of_selection(selection), cx),
                        None => this.drop_on(into.clone(), &selection.external(), window, cx),
                    })
                });
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
                None => {
                    div().w(px(16.)).children(icon(&path, true, 16).map(|i| img(i).size(px(16.)))).into_any_element()
                }
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
            if let (Some(tint), Row::Favorite(_, Favorite { icon: None, .. })) = (tint, &row) {
                line = line.child(div().ml(px(8.)).size(px(7.)).rounded_full().bg(color(tint)));
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
            children.push(item.into_any_element());
        }

        let mut context = "Sidebar".to_owned();
        if vim {
            context.push_str(" vim");
        }
        let layout = self.layout.clone();
        div()
            .on_children_prepainted(move |bounds, window, _| {
                // The first child is the current-folder fill; the rest are rows.
                let mut bounds = bounds.into_iter();
                let _fill = bounds.next();
                let was_empty = layout.borrow().is_empty();
                *layout.borrow_mut() = kinds
                    .iter()
                    .copied()
                    .zip(bounds)
                    .map(|((target, favorite), bounds)| (target, favorite, bounds))
                    .collect();
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
            .on_action(cx.listener(Self::eject_cursor))
            .debug_selector(|| "sidebar".into())
            .w(px(210.))
            .flex_none()
            .h_full()
            .px_2()
            .pb_2()
            .overflow_y_scroll()
            .bg(color(theme.role("bg-inset")))
            .border_r_1()
            .border_color(color(theme.role("border")))
            .flex()
            .flex_col()
            .gap(px(1.))
            .child(fill)
            .children(children)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
