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
use crow_core::{Favorite, Location, SavedQueryId, Section, SectionDestination, SectionRow, SidebarEdit, Slot, TagId};
use gpui::{
    Animation, AnimationExt, Bounds, ClickEvent, Context, EventEmitter, FocusHandle, Focusable, FontWeight,
    IntoElement, MouseButton, MouseDownEvent, Pixels, Point, Render, Role, SharedString, SpringAnimation, SpringConfig,
    Task, Window, div, img, prelude::*, px,
};

use crate::actions::{
    CollapseSection, Eject, MoveDown, MoveFavoriteDown, MoveFavoriteUp, MoveUp, NextGroup, OpenSelection,
    PreviousGroup, RemoveFromSidebar,
};
use crate::chrome::with_tooltip;
use crate::context_menu::{SidebarMenuKind, SidebarMenuState};
use crate::palette::CommandId;
use crate::style::{color, current_location, cursor_ring, display_text, drop_outline, effect_tint, icon};
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
    /// A section title; `Some(group)` for favorites groups, which accept folders.
    Header(String, Option<usize>),
    SectionHeader {
        id: String,
        title: String,
        collapsed: bool,
    },
    Favorite(Slot, Favorite),
    Recent(PathBuf),
    Volume(crow_macos::Volume),
    PluginItem {
        section: String,
        row: SectionRow,
    },
}

impl Row {
    fn path(&self) -> Option<&Path> {
        match self {
            Row::Header(..) | Row::SectionHeader { .. } => None,
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
            Row::Header(..) | Row::SectionHeader { .. } => None,
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

/// A cheap identity for the drawn rows: a reorder, added favorite or section
/// change invalidates a pending spring timer without invalidating a stationary
/// pointer over an unchanged row.
fn sidebar_layout_generation(rows: &[Row]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    rows.len().hash(&mut hasher);
    for row in rows {
        match row {
            Row::Header(name, group) => {
                0u8.hash(&mut hasher);
                name.hash(&mut hasher);
                group.hash(&mut hasher);
            }
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

fn rows(ui: &Ui, sections: &[Section]) -> Vec<Row> {
    let mut rows = Vec::new();
    for (g, (name, favorites)) in ui.favorites().into_iter().enumerate() {
        rows.push(Row::Header(name, Some(g)));
        rows.extend(favorites.into_iter().enumerate().map(|(i, f)| Row::Favorite((g, i), f)));
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

pub struct Sidebar {
    focus: FocusHandle,
    /// Index into the selectable rows (favorites, recents, volumes).
    cursor: usize,
    /// The folder the window shows, highlighted when it's in the sidebar.
    current: Location,
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
            focus: cx.focus_handle(),
            cursor: 0,
            current: Location::Folder(PathBuf::new()),
            layout: Rc::default(),
            drop_target: None,
            dragging_favorite: Rc::new(Cell::new(false)),
            insert_slot: None,
            names: RefCell::new(HashMap::new()),
            name_paths: RefCell::new(Vec::new()),
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
        let selectable: Vec<&Row> = rows.iter().filter(|row| !matches!(row, Row::Header(..))).collect();
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
        self.set_current_location(Location::Folder(dir), cx);
    }

    pub fn set_current_location(&mut self, location: Location, cx: &mut Context<Self>) {
        if self.current != location {
            self.current = location;
            cx.notify();
        }
    }

    fn selectable(&self, cx: &gpui::App) -> Vec<Row> {
        rows(cx.global::<Ui>(), &self.plugin_sections).into_iter().filter(|r| !matches!(r, Row::Header(..))).collect()
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
    pub fn debug_row_bounds(&self, i: usize, cx: &gpui::App) -> Option<gpui::Bounds<gpui::Pixels>> {
        let mut selectable = 0;
        for (layout_index, row) in rows(cx.global::<Ui>(), &self.plugin_sections).iter().enumerate() {
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
                Row::SectionHeader { title, .. } => display_text(title).into_owned(),
                Row::PluginItem { row, .. } => display_text(&row.text).into_owned(),
                Row::Header(..) => unreachable!(),
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
                current: row.location().as_ref() == Some(&self.current),
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
        self.looks(&rows, self.focus.is_focused(window), ui)
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
            Row::Header(..) | Row::SectionHeader { .. } => None,
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
            Row::Header(..) | Row::SectionHeader { .. } => None,
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
                let row = rows.into_iter().filter(|row| !matches!(row, Row::Header(..))).nth(index)?;
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
            if matches!(row, Row::Header(..)) {
                groups.push(Vec::new());
            } else if matches!(row, Row::SectionHeader { .. }) {
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
        ui::edit_sidebar(SidebarEdit::SetSectionCollapsed { id, collapsed: true }, cx);
    }

    /// Move to `index`, or validate a plugin destination off-thread before the
    /// window navigates. `navigation_generation` makes a later move discard a
    /// stale result; the live row and section revision are checked again on the
    /// main thread.
    fn navigate_row(&mut self, index: usize, nav: SidebarNav, cx: &mut Context<Self>) {
        self.cursor_key = None;
        self.navigation_generation = self.navigation_generation.wrapping_add(1);
        let generation = self.navigation_generation;
        let Some(row) = self.selectable(cx).into_iter().nth(index) else { return };
        if let Row::SectionHeader { id, collapsed, .. } = row {
            if nav == SidebarNav::Open && collapsed {
                ui::edit_sidebar(SidebarEdit::SetSectionCollapsed { id, collapsed: false }, cx);
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
                if this.navigation_generation != generation || this.section_revision() != revision {
                    return;
                }
                let live = this.selectable(cx).into_iter().nth(index);
                let live = matches!(
                    live,
                    Some(Row::PluginItem { section: s, row })
                        if s == section && row.id == row_id && row.folder_path() == Some(expected_path.as_path())
                );
                if !live {
                    return;
                }
                if matches {
                    emit_nav(nav, Location::Folder(expected_path), cx);
                } else if nav != SidebarNav::Show {
                    cx.emit(SidebarEvent::Message("That folder is no longer available".into(), true));
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
            Row::Header(..) | Row::SectionHeader { .. } | Row::PluginItem { .. } => return,
        };
        let target = match &row {
            Row::Favorite(_, favorite) => MenuTarget::Favorite(favorite.path.clone()),
            Row::Recent(path) => MenuTarget::Recent(path.clone()),
            Row::Volume(volume) => MenuTarget::Volume(volume.path.clone()),
            Row::PluginItem { section, row } => MenuTarget::Plugin { section: section.clone(), id: row.id.clone() },
            Row::Header(..) | Row::SectionHeader { .. } => return,
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

impl Render for Sidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The receive pulse is drawn from the clock: keep frames coming while it runs.
        if self.receive.is_some() {
            window.request_animation_frame();
        }
        self.update_plugin_sections(cx);
        self.request_inactive_counts(cx);
        let rows = rows(cx.global::<Ui>(), &self.plugin_sections);
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
        let focused = self.focus.is_focused(window);
        let selectable_count = rows.iter().filter(|r| !matches!(r, Row::Header(..))).count();
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
                    DropTarget::Row(index) => rows
                        .iter()
                        .filter(|row| !matches!(row, Row::Header(..)))
                        .nth(*index)
                        .and_then(Row::path)
                        .is_some_and(|path| {
                            crate::operations::drop_would_act(crate::drag_drop::active_paths(cx), path)
                        }),
                }),
            };
        } else {
            self.drop_target = None;
            self.insert_slot = None;
            self.dragging_favorite.set(false);
        }
        let current_child = rows.iter().position(|row| row.location().as_ref() == Some(&self.current));
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
                        .on_drop(cx.listener(move |_, dragged: &DraggedFavorite, _, cx| {
                            ui::edit_sidebar(SidebarEdit::Move { from: dragged.slot, to: (g, 0) }, cx);
                        })),
                    None => header,
                };
                kinds.push((group.map(DropTarget::Group), None));
                children.push(header.into_any_element());
                continue;
            }
            if let Row::SectionHeader { id, title, collapsed } = &row {
                let id = id.clone();
                let collapsed = *collapsed;
                kinds.push((None, None));
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
                        .when(cursor, |header| header.aria_active_descendant().shadow(cursor_ring(&theme)))
                        .px_2()
                        .pt_3()
                        .pb_1()
                        .rounded_md()
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(color(muted))
                        .cursor_pointer()
                        .hover(move |style| style.text_color(color(fg)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.cursor = i;
                            this.cursor_key = None;
                            ui::edit_sidebar(
                                SidebarEdit::SetSectionCollapsed { id: id.clone(), collapsed: !collapsed },
                                cx,
                            );
                        }))
                        .child(if collapsed { "▸" } else { "▾" })
                        .child(title.clone())
                        .into_any_element(),
                );
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
            // Favorites and Recent use the board's line icons; a favorite's
            // custom text icon and its palette color still win. Volumes and the
            // file list keep Finder's icons.
            let (glyph, line_icon, tint) = match &row {
                Row::Favorite(_, f) => (
                    f.icon.clone(),
                    f.icon.is_none().then_some(crate::icons::Glyph::Folder),
                    f.color.as_deref().and_then(|c| favorite_color(c, &theme)),
                ),
                Row::Recent(_) => (None, Some(crate::icons::Glyph::Clock), None),
                Row::PluginItem { row, .. } => (
                    None,
                    Some(if matches!(row.destination, SectionDestination::Tag { .. }) {
                        crate::icons::Glyph::Tag
                    } else {
                        crate::icons::Glyph::Folder
                    }),
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
                .px_2()
                .py(px(3.))
                .rounded_md()
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
                        this.navigate_row(i, SidebarNav::Open, cx);
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
                                .child(crate::icons::pack_glyph(&packed.asset, 15., icon_tint))
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
                    None if navigable => div()
                        .w(px(16.))
                        .children(icon(&path, true, 16).map(|i| img(i).size(px(16.))))
                        .into_any_element(),
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
            children.push(item.into_any_element());
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
                let _fill = bounds.next();
                let was_empty = layout.borrow().is_empty();
                let bounds: Vec<_> = bounds.collect();
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
