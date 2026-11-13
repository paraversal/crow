//! The sidebar: groups of favorites from `sidebar.toml`, Recent (frecency, with
//! zoxide's list when installed) and Locations (mounted volumes with free space
//! and Eject).
//!
//! Favorites are reordered by dragging them or with Cmd-Option-Up/Down. Files
//! dropped on any row move or copy into that folder; folders dropped on a group's
//! title become favorites.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crow_core::{Favorite, SidebarEdit, Slot};
use crow_fs::ops::Request;
use gpui::{
    Bounds, Context, EventEmitter, ExternalPaths, FocusHandle, Focusable, FontWeight, IntoElement, Pixels, Point,
    Render, Role, SharedString, Window, div, img, prelude::*, px,
};

use crate::actions::{Eject, MoveDown, MoveFavoriteDown, MoveFavoriteUp, MoveUp, OpenSelection, RemoveFromSidebar};
use crate::chrome::with_tooltip;
use crate::palette::CommandId;
use crate::pane::DraggedSelection;
use crate::style::{active_selection, color, current_location, drop_target, icon};
use crate::ui::{self, Ui};

pub enum SidebarEvent {
    Open(PathBuf),
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

/// Finder's name for a folder ("Macintosh HD" for `/`).
fn display_name(path: &Path) -> String {
    crow_macos::display_components(path).pop().map_or_else(|| path.display().to_string(), |(name, _)| name)
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
        }
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

    fn cursor_row(&self, cx: &gpui::App) -> Option<Row> {
        Self::selectable(cx).into_iter().nth(self.cursor)
    }

    #[cfg(test)]
    pub fn cursor(&self) -> usize {
        self.cursor
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

    /// The one place a dragged favorite would go, from the pointer: the upper half of a
    /// favorite inserts before it, the lower half after it; a group title inserts first.
    fn favorite_slot_at(&self, position: Point<Pixels>) -> Option<Slot> {
        let layout = self.layout.borrow();
        let slot_of = |target: &Option<DropTarget>, favorite: &Option<Slot>, bounds: &Bounds<Pixels>| {
            if let Some(slot) = favorite {
                Some(if position.y < bounds.center().y { *slot } else { (slot.0, slot.1 + 1) })
            } else if let Some(DropTarget::Group(group)) = target {
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
        self.cursor = self.cursor.saturating_sub(1);
        cx.notify();
    }

    fn move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        self.cursor = (self.cursor + 1).min(Self::selectable(cx).len().saturating_sub(1));
        cx.notify();
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

    fn eject(&mut self, volume: crow_macos::Volume, cx: &mut Context<Self>) {
        if !volume.ejectable {
            return;
        }
        cx.emit(SidebarEvent::Message(format!("Ejecting “{}”…", volume.name).into(), false));
        let path = volume.path.clone();
        let task = cx.background_spawn(async move { crow_macos::eject(&path) });
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
        if let Some(request) = crate::operations::drop_request(paths.paths(), into, window.modifiers().alt) {
            cx.emit(SidebarEvent::Run(request));
        }
    }

    /// Folders dropped on a group's title join that group.
    fn add_dropped(&mut self, group: usize, paths: &ExternalPaths, cx: &mut Context<Self>) {
        let at = cx.global::<Ui>().favorites().get(group).map_or(0, |(_, f)| f.len());
        for (n, path) in paths.paths().iter().filter(|p| p.is_dir()).enumerate() {
            ui::edit_sidebar(SidebarEdit::Add { path: path.clone(), at: (group, at + n) }, cx);
        }
    }
}

impl Render for Sidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        let vim = ui.settings.keymap == crow_core::KeymapLayer::Vim;
        let rows = rows(ui);
        let focused = self.focus.is_focused(window);
        let selectable_count = rows.iter().filter(|r| !matches!(r, Row::Header(..))).count();
        self.cursor = self.cursor.min(selectable_count.saturating_sub(1));
        let (fg, muted, accent) = (theme.role("fg"), theme.role("fg-muted"), theme.role("accent"));
        let drop_bg = drop_target(&theme);

        // A drag's target comes from the pointer and the last layout, so the
        // highlight only changes when the pointer enters a different row.
        if cx.has_active_drag() {
            self.drop_target = self.target_at(window.mouse_position());
            self.insert_slot =
                if self.dragging_favorite.get() { self.favorite_slot_at(window.mouse_position()) } else { None };
        } else {
            self.drop_target = None;
            self.insert_slot = None;
            self.dragging_favorite.set(false);
        }
        let mut kinds: Vec<(Option<DropTarget>, Option<Slot>)> = Vec::with_capacity(rows.len());
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
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(color(muted))
                    .child(title.clone());
                let header = match *group {
                    Some(g) => header
                        .when(
                            !self.dragging_favorite.get() && self.drop_target == Some(DropTarget::Group(g)),
                            |header| header.bg(drop_bg),
                        )
                        .on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| this.add_dropped(g, paths, cx)))
                        .on_drop(cx.listener(move |this, selection: &DraggedSelection, _, cx| {
                            this.add_dropped(g, &selection.external(), cx)
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
            let here = path == self.current;
            let cursor = i == self.cursor;
            let (label, glyph, tint) = match &row {
                Row::Favorite(_, f) => (
                    f.name.clone().unwrap_or_else(|| display_name(&f.path)),
                    f.icon.clone(),
                    f.color.as_deref().and_then(|c| favorite_color(c, &theme)),
                ),
                Row::Volume(v) => (v.name.clone(), None, None),
                _ => (display_name(&path), None, None),
            };
            let volume_label = match &row {
                Row::Volume(v) => Some(format!("{} free of {}", human_bytes(v.free_bytes), human_bytes(v.total_bytes))),
                _ => None,
            };
            let spoken = match &row {
                Row::Volume(v) => format!(
                    "{label}, {} free of {}{}",
                    human_bytes(v.free_bytes),
                    human_bytes(v.total_bytes),
                    if v.ejectable { ", ejectable" } else { "" }
                ),
                Row::Favorite(_, f) => match f.key {
                    Some(k) if vim => format!("{label}, mark {k}"),
                    _ => label.clone(),
                },
                _ => label.clone(),
            };
            let target = path.clone();
            let mut item = div()
                .id(SharedString::from(format!("sidebar-{i}")))
                .debug_selector(move || format!("sidebar-{i}"))
                .role(Role::ListBoxOption)
                .aria_label(spoken)
                .aria_position_in_set(i + 1)
                .aria_size_of_set(selectable_count)
                .aria_selected(cursor)
                .when(cursor, |row| row.aria_active_descendant())
                .px_2()
                .py(px(3.))
                .rounded_md()
                .flex()
                .flex_col()
                .relative()
                .when(here, |row| row.bg(current_location(&theme)))
                .when(focused && cursor, |row| row.bg(active_selection(&theme)))
                .when(!self.dragging_favorite.get() && self.drop_target == Some(DropTarget::Row(i)), |row| {
                    row.bg(drop_bg)
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.cursor = i;
                    cx.emit(SidebarEvent::Open(target.clone()));
                }))
                .on_drop({
                    let into = path.clone();
                    cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                        this.drop_on(into.clone(), paths, window, cx)
                    })
                })
                .on_drop({
                    let into = path.clone();
                    cx.listener(move |this, selection: &DraggedSelection, window, cx| {
                        this.drop_on(into.clone(), &selection.external(), window, cx)
                    })
                });
            if let Row::Favorite(slot, _) = &row {
                let slot = *slot;
                let dragging = self.dragging_favorite.clone();
                item = item.on_drag(DraggedFavorite { slot, label: label.clone().into() }, move |dragged, _, _, cx| {
                    dragging.set(true);
                    cx.new(|_| dragged.clone())
                });
                // The one insertion line: above the first favorite of a group for slot 0,
                // below a favorite for the boundary after it. No row changes height.
                let line = self.insert_slot.and_then(|target| {
                    if target == slot {
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
            let mut line = div().flex().items_center().gap_2().h(px(20.)).child(icon_element).child(
                div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_color(color(fg)).child(label),
            );
            if let (Some(tint), Row::Favorite(_, Favorite { icon: None, .. })) = (tint, &row) {
                line = line.child(div().size(px(7.)).rounded_full().bg(color(tint)));
            }
            if let Row::Volume(v) = &row
                && v.ejectable
            {
                let volume = v.clone();
                line = line.child(
                    div()
                        .id(SharedString::from(format!("eject-{i}")))
                        .role(Role::Button)
                        .aria_label(format!("Eject {}", v.name))
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
            if let Row::Volume(v) = &row
                && v.total_bytes > 0
            {
                let used = 1.0 - v.free_bytes as f32 / v.total_bytes as f32;
                // Nearly full disks show in the warning color, as in Finder's storage bar.
                let fill = if used > 0.9 { theme.role("warning") } else { accent };
                item =
                    item.child(
                        div().ml(px(24.)).mt(px(2.)).h(px(3.)).rounded_full().bg(color(theme.role("border"))).child(
                            div().h_full().rounded_full().w(gpui::relative(used.clamp(0.0, 1.0))).bg(color(fill)),
                        ),
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
            .on_children_prepainted(move |bounds, _, _| {
                *layout.borrow_mut() = kinds
                    .iter()
                    .copied()
                    .zip(bounds)
                    .map(|((target, favorite), bounds)| (target, favorite, bounds))
                    .collect();
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
            .on_action(cx.listener(Self::eject_cursor))
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
    fn colors_by_name_role_or_hex() {
        let theme = crow_theme::fallback();
        assert_eq!(favorite_color("#112233", &theme), Some(crow_theme::Rgb(0x112233)));
        assert_eq!(favorite_color("accent", &theme), Some(theme.role("accent")));
        assert_eq!(favorite_color("blue", &theme), theme.slot("base0D"));
        assert_eq!(favorite_color("chartreuse", &theme), None);
    }
}
