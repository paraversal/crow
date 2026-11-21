//! A tab's contents: history, view mode and the open folders. List view shows
//! one folder as a table; column view shows a chain of folders, one per column.
//!
//! Column view focus rules (from the spec):
//! - exactly one column is focused; its selection uses the accent color, others are muted;
//! - a column that only shows the children of a selection is never focused until
//!   you move into it (Right arrow or a click);
//! - typing and filtering always act on the focused column.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crow_core::motion::{self, Effective, Fx, Look};
use crow_core::{
    Align, Column, ColumnCell, ColumnContext, ColumnWidth, EnterAction, GridGeometry, GridRect, History, IconSize,
    KeymapLayer, KnownSize, Location, ResizeDrag, SavedQueryId, TIP_SCROLL_ENTRIES, TIP_SCROLL_ROWS, TableLayout,
    TagId, TipId, ViewChange, ViewMode,
};
use crow_fs::{AllowAll, CloudStatus, Entry, ReadGate, Sort, SortKey};
use crow_theme::Theme;
use gpui::{
    Action, Animation, AnimationExt, AnyElement, App, Bounds, ClickEvent, Context, Div, Entity, EntityId, EventEmitter,
    ExternalDragPayload, FileDragPaths, FocusHandle, Focusable, Image, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point, Render, Role, ScrollHandle,
    ScrollStrategy, SharedString, SpringAnimation, SpringConfig, Subscription, UniformListScrollHandle, Window, canvas,
    div, img, point, prelude::*, px, size, uniform_list,
};

use crate::actions::{
    BeginJump, BeginMark, ChooseTags, ClearFilter, CopyFiles, CopyPath, CutFiles, DeletePermanently, Duplicate,
    EnterRow, ExtendDown, ExtendUp, FilterFocusField, FilterFocusResults, MoveDown, MoveLeft, MoveRight, MoveToFirst,
    MoveToLast, MoveToTrash, MoveUp, NewFile, NewFolder, OpenSelection, OpenTerminal, PasteFiles, PasteMoveFiles,
    QuickLook, Rename, SelectAll, ShowInFinder, StartFilter,
};
use crate::chrome::with_tooltip;
use crate::cloud_status::{self, CloudSource, CloudStatusService};
use crate::context_menu::{MenuKind, MenuState};
use crate::extensions::Ext;
use crate::folder::{Folder, FolderEvent};
use crate::folder_sizes::{SizeService, SourceKey};
use crate::icons::{Glyph, glyph};
use crate::info_pane::{self, InfoKey, InfoPane, InfoTab, InfoTarget};
use crate::keys::KeyScope;
use crate::operations::Operations;
use crate::palette::CommandId;
use crate::preview::{self, Preview};
use crate::search::{SearchEvent, SearchView};
use crate::style::{
    active_selection, cloud_icon, color, color_alpha, content_opacity, cut_outline, display_text, drop_outline,
    effect_tint, focused_column, format_time, highlight_text, inactive_selection, kind, map_range, truncate_middle,
    truncate_middle_parts, truncate_middle_parts_to_fit,
};
use crate::text_input::{TextInput, TextInputEvent};
use crate::ui::Ui;
use crow_fs::ops::Request;

mod gallery;
mod pane_filter;
mod view_motion;
use pane_filter::FilterPlace;
use view_motion::ViewMotion;

pub(crate) const ROW_HEIGHT: f32 = 24.0;
/// The reserved caption line under tile names; blank keeps every tile aligned.
const CAPTION_LINE_HEIGHT: f32 = 16.0;
const ICON_TILE_TOP: f32 = 3.0;
const ICON_TILE_GAP: f32 = 2.0;
const ICON_BOX_PADDING: f32 = 4.0;

fn icon_name_line_height(font_size: f32) -> f32 {
    (font_size * 1.25).ceil()
}

/// The first row a uniform list shows. Its scroll offset is negative once
/// scrolled down (`drag_row_at` subtracts it from a pointer position).
fn first_visible_row(handle: &UniformListScrollHandle) -> usize {
    let offset = f32::from(handle.0.borrow().base_handle.offset().y);
    (-offset / ROW_HEIGHT).max(0.).floor() as usize
}

/// Space between the list header's line and the first row, so a highlighted first row's
/// rounded corners don't meet the straight line and leave dark wedges at its ends.
pub(crate) const LIST_TOP_INSET: f32 = 4.0;
const LIST_SIDE_INSET: f32 = 8.0;
/// Left padding before a row's disclosure chevron, and the indent per tree level.
const NAME_INSET: f32 = 4.0;
const INDENT: f32 = 18.0;
const TRIANGLE_WIDTH: f32 = 20.0;
const CELL_PADDING: f32 = 8.0;
/// Browse columns grow with the strip, keeping two readable alongside Preview.
const COLUMN_MIN_WIDTH: f32 = 196.0;
const COLUMN_MAX_WIDTH: f32 = 360.0;
const PREVIEW_COLUMN_WIDTH: f32 = 300.0;

/// Requests the pane can't carry out alone: operations may need a sheet (conflicts,
/// confirmations) and the clipboard is app-wide, so the window handles these.
pub enum PaneEvent {
    Run(Request),
    /// Open the batch-rename sheet for a frozen, display-ordered selection.
    BatchRename(Arc<[crow_core::batch_rename::RenameInput]>),
    Copy {
        paths: Arc<[PathBuf]>,
        cut: bool,
    },
    Paste {
        into: PathBuf,
        move_items: bool,
    },
    /// The tab went to a folder (for Recent).
    Entered(PathBuf),
    /// The tab's destination changed to a virtual location (sidebar current,
    /// title and band follow it, but Recent does not).
    DestinationChanged,
    /// The saved destination's band asked for its edit form.
    SaveSearch,
    /// The focused folder filter wants its tag picker opened.
    ChooseTags,
    FocusSidebar,
    /// Vim marks: `m x` (set) or `'x` (jump) for the letter `key`.
    Mark {
        key: char,
        set: bool,
    },
    /// A pointer or touch scroll left a long folder scrolled past the tip threshold.
    LongFolderScrolled,
    /// A filter session ended: `empty` is an empty query, `matches` the focused
    /// folder's row count at close.
    FilterEnded {
        empty: bool,
        matches: usize,
    },
}

/// Vim: after `m` or `'`, the next letter names a mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MarkPending {
    Set,
    Jump,
}

/// Icon grid arrow directions, before geometry turns them into indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IconMove {
    Up,
    Down,
    Left,
    Right,
}

impl EventEmitter<PaneEvent> for Pane {}

/// Files dragged from inside Crow: paths and whether each is a folder. The selection is
/// shared, so rendering a selected row does not clone the whole selection.
#[derive(Clone)]
pub(crate) struct DraggedSelection {
    pub entries: Arc<Vec<(PathBuf, bool)>>,
}

/// Handed from the drag constructor to the native-promotion resolver: the measured
/// combo and the grab point, so the native image starts in exactly the same place.
#[derive(Clone)]
pub(crate) struct DragHandoff {
    cursor: Rc<Cell<Point<Pixels>>>,
    width: Rc<Cell<f32>>,
    pill_width: Rc<Cell<f32>>,
    label: Rc<RefCell<SharedString>>,
}

impl DragHandoff {
    pub(crate) fn new() -> Self {
        Self {
            cursor: Rc::new(Cell::new(Point::default())),
            width: Rc::new(Cell::new(0.0)),
            pill_width: Rc::new(Cell::new(0.0)),
            label: Rc::new(RefCell::new(SharedString::default())),
        }
    }
}

/// The image that follows the pointer while dragging with GPUI inside the window:
/// a small icon and a pill with the name.
pub(crate) struct DraggedFiles {
    icon: Option<Arc<Image>>,
    label: SharedString,
    pill_width: f32,
    width: f32,
    cursor: Point<Pixels>,
}

impl Render for DraggedFiles {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = &cx.global::<Ui>().theme;
        let left = drag_combo_left(f32::from(self.cursor.x), self.width);
        // Once the pointer leaves the window, the native image takes over; don't let the
        // GPUI image clip at the edge for that frame. `bounds()` is in screen coordinates,
        // while the mouse position is window-relative, so compare against the size only.
        let position = window.mouse_position();
        let size = window.bounds().size;
        let inside =
            position.x >= px(0.) && position.y >= px(0.) && position.x < size.width && position.y < size.height;
        div()
            .debug_selector(|| "drag-combo".into())
            .when(!inside, |element| element.invisible())
            .ml(px(left))
            .flex()
            .items_center()
            .gap_2()
            .when_some(self.icon.clone(), |element, icon| element.child(img(icon).size(px(22.))))
            .child(
                div()
                    .w(px(self.pill_width))
                    .h(px(20.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(color(theme.role("surface-raised")))
                    .text_color(color(theme.role("fg")))
                    .text_size(px(12.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(self.label.clone()),
            )
    }
}

/// Measure the name pill before the first frame so the combo appears already anchored.
fn measure_combo(label: &SharedString, window: &mut Window) -> (SharedString, f32, f32) {
    let font_size = px(12.);
    let run = gpui::TextRun {
        len: label.len(),
        font: gpui::Font::default(),
        color: gpui::Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let raw = f32::from(window.text_system().layout_line(label.clone(), font_size, &[run], None).width);
    let max_text = 200.0;
    let (display, text_width) = if raw > max_text {
        (truncate_middle(label, px(max_text), font_size, window), max_text)
    } else {
        (label.clone(), raw)
    };
    let pill = text_width + 16.0;
    let width = 22.0 + 8.0 + pill;
    (display, pill, width)
}

/// The combo that follows the pointer for a drag of `entries`, and the measurements
/// the native promotion needs. Shared by rows, search results and the path bar.
pub(crate) fn drag_combo(
    icon: Option<Arc<Image>>,
    label: &SharedString,
    offset: Point<Pixels>,
    handoff: &DragHandoff,
    window: &mut Window,
    cx: &mut App,
) -> Entity<DraggedFiles> {
    let (display, pill_width, width) = measure_combo(label, window);
    handoff.cursor.set(offset);
    handoff.width.set(width);
    handoff.pill_width.set(pill_width);
    *handoff.label.borrow_mut() = display.clone();
    cx.new(|_| DraggedFiles { icon, label: display, pill_width, width, cursor: offset })
}

/// Hand a dragged selection to the platform. When AppKit can start a native session,
/// it does so now with the combo image; otherwise GPUI promotes the paths itself.
pub(crate) fn promote_drag(
    selection: &DraggedSelection,
    handoff: &DragHandoff,
    window: &mut Window,
    cx: &mut App,
) -> Option<ExternalDragPayload> {
    let width = handoff.width.get();
    let pill_width = handoff.pill_width.get();
    let cursor = handoff.cursor.get();
    let label = handoff.label.borrow().clone();
    let entries = selection.entries.as_slice();
    if let Some((path, is_dir)) = entries.first()
        && let Some(png) = crate::style::drag_icon_png(cx, path, *is_dir, 22)
        && crate::effects::effects(cx).begin_drag(
            window,
            &crow_macos::ComboDrag {
                paths: entries,
                icon_png: png.as_slice(),
                label: label.as_ref(),
                width,
                height: 24.0,
                pill_width,
                cursor_from_left: f32::from(cursor.x).min(width),
                cursor_from_top: f32::from(cursor.y).min(24.0),
            },
        )
    {
        cx.stop_active_drag(window);
        return None;
    }
    Some(ExternalDragPayload::Files(FileDragPaths::new(entries.iter().cloned())))
}

/// An inline rename in progress.
struct Renaming {
    path: PathBuf,
    input: Entity<TextInput>,
    _subscription: Subscription,
}

/// The `/` filter field, while open.
struct Filtering {
    input: Entity<TextInput>,
    _subscription: Subscription,
}

/// Paths from the last painted viewport, with their row tops relative to it.
struct IconViewport {
    folder: EntityId,
    generation: u64,
    anchors: Vec<(PathBuf, f32)>,
    visible_cursor: Option<PathBuf>,
}

/// One empty-space drag in the icon grid. The anchor stays in content space, so
/// scrolling grows the rectangle instead of moving its origin.
struct Marquee {
    anchor: Point<Pixels>,
    pointer: Point<Pixels>,
    base_selection: Vec<PathBuf>,
    base_cursor: Option<PathBuf>,
    base: Arc<HashSet<PathBuf>>,
    additive: bool,
    /// Past the motion threshold: the rectangle is live.
    started: bool,
    layout_generation: u64,
    hits: HashMap<PathBuf, (u64, bool)>,
}

/// Where a drop would land, drawn as an outline. Only set when the drop would act.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DropTarget {
    pub folder: PathBuf,
    /// List view: the column, always 0. Column view: the column under the pointer.
    pub column: usize,
    /// The folder's row and its expanded subtree; `None` for the folder the list or
    /// column shows, outlined as a whole.
    pub rows: Option<std::ops::Range<usize>>,
}

/// What a spring-loaded presentation moved away from. The origin's listings stay
/// alive, so cancelling restores the exact rows, selection and scroll.
struct SpringOrigin {
    history: History<Location>,
    folders: Vec<Entity<Folder>>,
    subscriptions: Vec<Subscription>,
    focused: usize,
    mode: ViewMode,
    sort: Sort,
    applied_view: Option<(u64, u64, PathBuf)>,
    virtual_origin: bool,
}

/// A saved-query result session retained while its location stays in history.
struct SavedSession {
    view: Entity<SearchView>,
    _subscription: Subscription,
}

pub struct Pane {
    history: History<Location>,
    /// Result sessions for saved destinations in this tab's history. Only the
    /// active one runs; others keep their last accepted snapshot.
    saved_sessions: HashMap<SavedQueryId, SavedSession>,
    /// Result sessions for tag destinations in this tab's history.
    tag_sessions: HashMap<TagId, SavedSession>,
    /// The saved destination currently shown, if any.
    active_saved: Option<SavedQueryId>,
    /// The tag destination currently shown, if any.
    active_tag: Option<TagId>,
    mode: ViewMode,
    show_hidden: bool,
    sort: Sort,
    /// List view: just the folder shown. Column view: the chain from the root rightwards.
    folders: Vec<Entity<Folder>>,
    subscriptions: Vec<Subscription>,
    /// Observers of app-wide services (sizes, cloud status). Kept apart from
    /// `subscriptions`, which is per open folder and is rewritten on navigation.
    _service_subscriptions: Vec<Subscription>,
    /// Which folder has keyboard focus (always 0 in list view).
    focused: usize,
    focus: FocusHandle,
    /// Retry and Open Full Disk Access in the unreadable-folder state.
    unreadable: [FocusHandle; 2],
    /// The list view's columns after Name, and the widths shared by header and rows.
    columns: Vec<Arc<dyn Column>>,
    table: TableLayout,
    content_width: Pixels,
    resize: Option<ResizeDrag>,
    preview: Option<Preview>,
    preview_key: Option<(PathBuf, u64, u32)>,
    preview_generation: u64,
    /// Plugin/views generations at the last preview load, so registration and
    /// `preview_providers` edits refresh an already visible preview.
    preview_plugin_generation: u64,
    preview_views_generation: u64,
    preview_cancel: Arc<AtomicBool>,
    /// The window's right column: tab choice and the Info tab's target.
    info: Entity<InfoPane>,
    /// This pane's id in the folder-size service's visible-row sources.
    size_source: u64,
    /// This pane's id in the cloud-status service's visible-row sources.
    cloud_source: u64,
    /// The service generation whose visible sizes were already sorted (Q3).
    size_sort_generation: u64,
    /// `folder_sizes = "visible"` at the last render; turning it off pauses the service.
    size_enabled: bool,
    strip_scroll: ScrollHandle,
    status: Option<SharedString>,
    renaming: Option<Renaming>,
    /// Start renaming this item once it appears (a new folder).
    rename_when_present: Option<PathBuf>,
    bench: Option<Bench>,
    /// The `views.toml` reload and folder whose view was applied last.
    applied_view: Option<(u64, u64, PathBuf)>,
    /// List view: the uniform list's viewport, captured at prepaint.
    list_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// Icon view: its own scroll position and viewport, kept across mode switches.
    icon_scroll: UniformListScrollHandle,
    icon_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    icon_viewport: Rc<RefCell<Option<IconViewport>>>,
    /// Gallery: rail scroll, preview pair and folder samples.
    gallery: gallery::Gallery,
    /// The geometry the last icon frame drew, shared by keys, hit tests and anchors.
    grid: Option<GridGeometry>,
    /// The effective icon size for this pane (from views.toml when folders decide).
    icon_size: IconSize,
    /// A view forced by `CROW_BENCH_VIEW`, so the bench measures the named layout.
    bench_view: Option<ViewMode>,
    /// The plugin column under tile names, when configured.
    icon_caption: Option<String>,
    /// The list row under a drag, drawn from state rather than drag-over styles.
    drop_row: Option<usize>,
    /// Where a drop would land in either view, and the rows it covers.
    drop_target: Option<DropTarget>,
    /// Column view: each column's list viewport, captured at prepaint.
    column_bounds: Rc<RefCell<HashMap<usize, Bounds<Pixels>>>>,
    presented_layout: Option<(ViewMode, EntityId)>,
    view_motion: Option<ViewMotion>,
    /// What the last icon reflow captured; unlike `view_motion` it never expires,
    /// so tests don't race the glide's wall clock on a loaded machine.
    #[cfg(test)]
    reflow_capture: Option<ViewMotion>,
    child_reveal: Option<(EntityId, Instant)>,
    /// The column and row a drag would drop on in column view.
    drop_column: Option<(usize, usize)>,
    /// A drag's temporary folder presentation, with the origin kept for cancel.
    spring: Option<SpringOrigin>,
    /// The origin and source paths to reselect when Back returns after a sprung drop.
    return_selection: Option<(Location, Vec<PathBuf>)>,
    mark_pending: Option<MarkPending>,
    /// The `/` filter field, while open.
    filtering: Option<Filtering>,
    /// An empty-space drag selecting tiles in icon view.
    marquee: Option<Marquee>,
    marquee_scroll_gen: u64,
    /// Reserve the list's bottom inset while a tip card covers it.
    tip_inset: bool,
    /// The filter count VoiceOver hears: the live count, but only once typing
    /// stops, so fast typing does not queue one announcement per keystroke.
    filter_announce: SharedString,
    filter_announce_gen: u64,
    /// Focus the list after a render because a navigation closed the filter.
    focus_list_after_filter: bool,
    /// The sidebar cursor is walking this pane: later moves replace the history step.
    sidebar_walk: bool,
    /// The folder and directory slice a plugin sidebar section keeps while the
    /// sidebar cursor walks the pane, so contributed rows do not move under it.
    plugin_walk_dirs: Option<(PathBuf, u64, Vec<crate::plugins::SidebarDir>)>,
    /// A row took the current mouse-down; the list behind it must not treat it as a
    /// click on empty space. Rows handle the event before the list (deeper hitbox).
    row_pressed: bool,
    /// The rename field took the current mouse-down: place the caret, don't commit.
    rename_pressed: bool,
    /// The context menu on screen and what it was opened for, until it closes.
    context_menu: Option<MenuKind>,
    /// Plugin rows appended after the built-in context-menu slots.
    context_menu_plugins: Vec<crate::actions::PluginCommand>,
    /// The last selection signature announced to plugin hooks, per column.
    plugin_selection: HashMap<usize, u64>,
    /// The focused surface whose plugin scope was last announced.
    plugin_scope: Option<(crate::plugins::SurfaceId, crate::plugins::ScopeKey)>,
}

struct Bench {
    frame: usize,
    intervals: Vec<f64>,
    last: Option<Instant>,
    /// Scroll results, kept while the folder is reopened to time first paint in a warm app.
    scrolled: Option<String>,
    /// The name filter applied before scrolling, so highlighted rows are what is timed.
    filter: Option<String>,
}

impl Pane {
    /// `start_mode` is the view a window keeps when `view_per_folder` is off; `None`
    /// lets the folder decide.
    pub fn new(
        dir: PathBuf,
        show_hidden: bool,
        start_mode: Option<ViewMode>,
        bench: bool,
        bench_filter: Option<String>,
        info: Entity<InfoPane>,
        cx: &mut Context<Self>,
    ) -> Self {
        let columns = cx.global::<Ext>().0.default_columns();
        let size_service = cx.try_global::<SizeService>().map(|service| service.0.clone());
        let size_source = size_service.as_ref().map(|service| service.read(cx).source()).unwrap_or(0);
        let cloud_service = cx.try_global::<CloudStatusService>().map(|service| service.0.clone());
        let cloud_source =
            cloud_service.as_ref().map(|service| crate::cloud_status::register_source(service, cx)).unwrap_or(0);
        let thumbnail_service = crate::thumbnails::service(cx);
        let subscriptions = Vec::new();
        let mut service_subscriptions = Vec::new();
        if let Some(service) = size_service {
            service_subscriptions.push(cx.observe(&service, |pane, _, cx| pane.sizes_changed(cx)));
        }
        if let Some(service) = cloud_service {
            service_subscriptions.push(cx.observe(&service, |pane, _, cx| pane.cloud_changed(cx)));
        }
        if let Some(service) = thumbnail_service {
            service_subscriptions.push(cx.observe(&service, |_, _, cx| cx.notify()));
        }
        let mut pane = Self {
            history: History::new(Location::Folder(dir.clone())),
            saved_sessions: HashMap::new(),
            tag_sessions: HashMap::new(),
            active_saved: None,
            active_tag: None,
            mode: ViewMode::List,
            show_hidden,
            sort: Sort::default(),
            folders: Vec::new(),
            subscriptions,
            _service_subscriptions: service_subscriptions,
            focused: 0,
            focus: cx.focus_handle(),
            unreadable: [cx.focus_handle(), cx.focus_handle()],
            table: default_table(&columns),
            columns,
            content_width: Pixels::ZERO,
            resize: None,
            preview: None,
            preview_key: None,
            preview_generation: 0,
            preview_plugin_generation: 0,
            preview_views_generation: 0,
            preview_cancel: Arc::new(AtomicBool::new(false)),
            info,
            size_source,
            cloud_source,
            size_sort_generation: 0,
            size_enabled: cx.global::<Ui>().settings.folder_sizes == crow_core::FolderSizesMode::Visible,
            strip_scroll: ScrollHandle::new(),
            status: None,
            renaming: None,
            rename_when_present: None,
            bench: bench.then(|| Bench {
                frame: 0,
                intervals: Vec::new(),
                last: None,
                scrolled: None,
                filter: bench_filter,
            }),
            applied_view: None,
            list_bounds: Rc::new(Cell::new(None)),
            icon_scroll: UniformListScrollHandle::new(),
            icon_bounds: Rc::new(Cell::new(None)),
            icon_viewport: Rc::new(RefCell::new(None)),
            gallery: gallery::Gallery::new(),
            grid: None,
            icon_size: IconSize::Standard,
            icon_caption: None,
            bench_view: bench
                .then(|| {
                    std::env::var("CROW_BENCH_VIEW").ok().and_then(|view| match view.as_str() {
                        "icons" => Some(ViewMode::Icons),
                        "list" => Some(ViewMode::List),
                        "columns" => Some(ViewMode::Columns),
                        "gallery" => Some(ViewMode::Gallery),
                        _ => None,
                    })
                })
                .flatten(),
            drop_row: None,
            drop_target: None,
            column_bounds: Rc::new(RefCell::new(HashMap::new())),
            presented_layout: None,
            view_motion: None,
            #[cfg(test)]
            reflow_capture: None,
            child_reveal: None,
            drop_column: None,
            spring: None,
            return_selection: None,
            mark_pending: None,
            filtering: None,
            marquee: None,
            marquee_scroll_gen: 0,
            tip_inset: false,
            filter_announce: SharedString::default(),
            filter_announce_gen: 0,
            focus_list_after_filter: false,
            row_pressed: false,
            rename_pressed: false,
            context_menu: None,
            context_menu_plugins: Vec::new(),
            plugin_selection: HashMap::new(),
            plugin_scope: None,
            sidebar_walk: false,
            plugin_walk_dirs: None,
        };
        pane.enter(dir, cx);
        if let Some(mode) = start_mode.filter(|_| !cx.global::<Ui>().settings.view_per_folder) {
            pane.apply_mode(mode, cx);
        }
        if let Some(mode) = pane.bench_view {
            pane.apply_mode(mode, cx);
        }
        if let Some(filter) = pane.bench.as_ref().and_then(|bench| bench.filter.clone()) {
            let folder = pane.folders[0].clone();
            folder.update(cx, |folder, cx| folder.set_filter(Some(filter), cx));
        }
        pane
    }

    // ----- read access for the window -----

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    pub fn mode(&self) -> ViewMode {
        self.mode
    }

    pub fn icon_size(&self) -> IconSize {
        self.icon_size
    }

    /// The tab's destination: a real folder or a saved query id.
    pub fn location(&self) -> &Location {
        self.history.current()
    }

    /// The real folder this tab shows, if it is a folder destination.
    pub fn folder(&self) -> Option<&Path> {
        self.location().as_folder()
    }

    /// The real folder the user is in, including the focused column in Columns.
    pub fn focused_path(&self, cx: &gpui::App) -> Option<PathBuf> {
        self.folder().map(|_| self.current_dir(cx))
    }

    /// The tab's folder: the list view's folder, or the column view's root.
    /// Callers must have checked `folder()`; a virtual destination has no path.
    pub fn dir(&self) -> &Path {
        self.location().as_folder().expect("a folder destination")
    }

    /// The active virtual destination's result session, if the tab shows one.
    pub fn saved_session(&self) -> Option<Entity<SearchView>> {
        self.active_saved
            .as_ref()
            .and_then(|id| self.saved_sessions.get(id))
            .or_else(|| self.active_tag.as_ref().and_then(|id| self.tag_sessions.get(id)))
            .map(|session| session.view.clone())
    }

    pub fn saved_unavailable(&self, cx: &gpui::App) -> Option<SharedString> {
        let id = self.active_saved.as_ref()?;
        (!cx.global::<Ui>().saved_queries.contains(id))
            .then(|| SharedString::from("This saved search was removed. Back restores the previous location."))
    }

    /// The icon source this pane shows: the focused folder's `views.toml`
    /// override, else `settings.toml`'s pack.
    pub fn icon_source(&self, cx: &gpui::App) -> String {
        self.folders
            .get(self.focused)
            .and_then(|folder| folder.read(cx).icon_source().map(str::to_owned))
            .unwrap_or_else(|| cx.global::<Ui>().settings.icon_pack.clone())
    }

    fn icon_resolver(&self, column: usize, cx: &gpui::App) -> crate::icon_packs::Resolver {
        let source = self
            .folders
            .get(column)
            .and_then(|folder| folder.read(cx).icon_source().map(str::to_owned))
            .unwrap_or_else(|| cx.global::<Ui>().settings.icon_pack.clone());
        crate::icon_packs::Resolver::for_source(cx, &source)
    }

    /// Apply the `/` filter's tag constraint to the focused column.
    pub fn set_folder_tag_filter(&mut self, tags: Vec<String>, cx: &mut Context<Self>) {
        let folder = self.focused_folder().clone();
        folder.update(cx, |folder, cx| folder.set_tag_filter(tags, cx));
        self.schedule_filter_announce(cx);
        cx.notify();
    }

    pub fn folder_tag_filter(&self, cx: &gpui::App) -> Vec<String> {
        self.focused_folder().read(cx).tag_filter().to_vec()
    }

    fn choose_tags(&mut self, _: &ChooseTags, window: &mut Window, cx: &mut Context<Self>) {
        if self.filtering.is_none() {
            self.start_filter(&StartFilter, window, cx);
        }
        cx.emit(PaneEvent::ChooseTags);
    }

    /// Open saved-query and tag sessions, for release tests.
    #[cfg(test)]
    pub fn session_counts_for_test(&self) -> (usize, usize) {
        (self.saved_sessions.len(), self.tag_sessions.len())
    }

    #[cfg(test)]
    pub fn focused_folder_for_test(&self) -> Entity<Folder> {
        self.focused_folder().clone()
    }

    fn focused_folder(&self) -> &Entity<Folder> {
        &self.folders[self.focused]
    }

    /// The folder the user is in: the focused column in column view.
    pub fn current_dir(&self, cx: &gpui::App) -> PathBuf {
        self.focused_folder().read(cx).dir().to_path_buf()
    }

    pub fn current_plugin_surface(&self, cx: &gpui::App) -> crate::plugins::SurfaceId {
        self.focused_folder().read(cx).plugin_surface()
    }

    pub fn sidebar_walk(&self) -> bool {
        self.sidebar_walk
    }

    /// The sidebar lost focus: the preview walk is over and its frozen section
    /// slice must start following the shown folder again.
    pub fn end_sidebar_walk(&mut self) {
        self.sidebar_walk = false;
        self.plugin_walk_dirs = None;
    }

    /// The first 128 ordinary immediate folders for a sidebar section, with a
    /// revision that changes on relist or watcher apply. While the sidebar
    /// cursor is walking the pane, the slice captured at the walk's start stays
    /// fixed so moving onto one contributed row does not remove the next one.
    pub fn sidebar_plugin_dirs(
        &self,
        walking: bool,
        cx: &gpui::App,
    ) -> (PathBuf, u64, Vec<crate::plugins::SidebarDir>) {
        if walking && let Some(frozen) = &self.plugin_walk_dirs {
            return frozen.clone();
        }
        let folder = self.focused_folder().read(cx);
        let dir = folder.dir().to_path_buf();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        folder.plugin_scope_version().hash(&mut hasher);
        dir.hash(&mut hasher);
        let dirs = folder
            .entries()
            .iter()
            .filter(|entry| entry.is_dir && !entry.is_symlink && !entry.is_package())
            .take(128)
            .map(|entry| {
                let version = folder.plugin_entry_version(entry);
                version.hash(&mut hasher);
                crate::plugins::SidebarDir {
                    path: entry.path.clone(),
                    name: crate::style::display_text(&entry.name).into_owned(),
                    version,
                    identity: None,
                }
            })
            .collect();
        (dir, hasher.finish(), dirs)
    }

    pub fn item_count(&self, cx: &gpui::App) -> usize {
        self.focused_folder().read(cx).entries().len()
    }

    pub fn selected_count(&self, cx: &gpui::App) -> usize {
        self.focused_folder().read(cx).selected_count()
    }

    /// The selected files' bytes plus the known sizes of selected folders, and
    /// how many selected items are folders. Off: file bytes only, as before.
    pub fn selected_bytes(&self, cx: &gpui::App) -> (u64, usize) {
        let folder = self.focused_folder().read(cx);
        if cx.global::<Ui>().settings.folder_sizes != crow_core::FolderSizesMode::Visible {
            return folder.selected_bytes();
        }
        let Some(service) = cx.try_global::<SizeService>() else {
            return folder.selected_bytes();
        };
        let service = service.0.read(cx);
        folder.selected_bytes_with_sizes(service)
    }

    /// The current folder's folder rows, for View ▸ Calculate Folder Sizes.
    pub fn folder_paths(&self, cx: &gpui::App) -> Vec<PathBuf> {
        let folder = self.focused_folder().read(cx);
        folder
            .entries()
            .iter()
            .filter(|entry| entry.is_dir && !entry.is_symlink)
            .map(|entry| entry.path.clone())
            .collect()
    }

    /// Report the folder rows this pane shows to the size service. Called from
    /// the row renderers; it only enqueues work. Off pauses the service once.
    fn observe_sizes(&mut self, source: SourceKey, paths: &[PathBuf], cx: &mut Context<Self>) {
        let enabled = cx.global::<Ui>().settings.folder_sizes == crow_core::FolderSizesMode::Visible;
        if !enabled {
            if self.size_enabled {
                if let Some(service) = cx.try_global::<SizeService>().map(|service| service.0.clone()) {
                    service.update(cx, |service, _| service.pause());
                }
                self.size_enabled = false;
            }
            return;
        }
        self.size_enabled = true;
        if let Some(service) = cx.try_global::<SizeService>() {
            let service = service.0.clone();
            service.update(cx, |service, cx| service.observe(source, paths, cx));
        }
    }

    fn observe_cloud(&mut self, source: CloudSource, paths: &[PathBuf], cx: &mut Context<Self>) {
        if let Some(service) = cx.try_global::<CloudStatusService>() {
            let service = service.0.clone();
            service.update(cx, |service, cx| service.observe(source, paths, cx));
        }
    }

    fn cloud_status(&self, path: &Path, cx: &gpui::App) -> Option<CloudStatus> {
        cx.try_global::<CloudStatusService>().and_then(|service| service.0.read(cx).status(path))
    }

    /// Whether the `/` filter field is open.
    pub fn is_filtering(&self) -> bool {
        self.filtering.is_some()
    }

    /// Reserve the list's bottom inset while a tip card covers it.
    pub fn set_tip_inset(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.tip_inset != visible {
            self.tip_inset = visible;
            cx.notify();
        }
    }

    /// The filter field's focus handle while it is open.
    pub fn filter_focus(&self, cx: &gpui::App) -> Option<FocusHandle> {
        self.filtering.as_ref().map(|filtering| filtering.input.read(cx).focus_handle(cx))
    }

    /// The containing window sets this before rendering the pane.
    pub(crate) fn set_content_width(&mut self, width: Pixels) {
        self.content_width = width;
    }

    /// Whether a row is being renamed.
    pub fn is_renaming(&self) -> bool {
        self.renaming.is_some()
    }

    /// The path whose name field is open, for tests that check what Return acted on.
    #[cfg(test)]
    pub(crate) fn renaming_path(&self) -> Option<PathBuf> {
        self.renaming.as_ref().map(|renaming| renaming.path.clone())
    }

    pub fn is_loading(&self, cx: &gpui::App) -> bool {
        self.folders.iter().any(|f| f.read(cx).is_loading_deep(cx))
    }

    /// Like `is_loading`, but also waits for a background row build: a test or
    /// the fuzzer that reads rows after an apply needs this.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn is_settling(&self, cx: &gpui::App) -> bool {
        self.folders.iter().any(|f| {
            let folder = f.read(cx);
            folder.is_settling() || folder.children_settling(cx)
        })
    }

    /// The visible (name, depth) rows of the focused folder, for tests.
    #[cfg(test)]
    pub fn rows(&self, cx: &gpui::App) -> Vec<(String, usize)> {
        let folder = self.focused_folder().read(cx);
        (0..folder.row_count()).filter_map(|ix| folder.row(ix).map(|(e, d)| (e.name.clone(), d))).collect()
    }

    /// The selectors a fuzzer can click in this pane, in render order. Rows are
    /// capped so a 100k folder does not turn one step into a huge list; the
    /// runner still drops selectors that are not laid out.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn debug_selectors(&self, cx: &gpui::App) -> Vec<String> {
        let mut out = Vec::new();
        match self.mode {
            ViewMode::List => {
                out.push("file-list".to_owned());
                out.push("table-header".to_owned());
                if let Some(folder) = self.folders.first() {
                    let folder = folder.read(cx);
                    out.extend(
                        (0..folder.row_count().min(200))
                            .filter_map(|ix| folder.row(ix).map(|(entry, _)| format!("row-{}", entry.name))),
                    );
                }
            }
            ViewMode::Icons => {
                out.push("file-list".to_owned());
                if let Some(folder) = self.folders.first() {
                    let folder = folder.read(cx);
                    out.extend(
                        (0..folder.row_count().min(200))
                            .filter_map(|ix| folder.row(ix).map(|(entry, _)| format!("tile-{}", entry.name))),
                    );
                }
            }
            ViewMode::Gallery => {
                out.push("gallery-rail".to_owned());
                out.push("gallery-preview".to_owned());
                if let Some(folder) = self.folders.first() {
                    let folder = folder.read(cx);
                    out.extend(
                        (0..folder.row_count().min(200))
                            .filter_map(|ix| folder.row(ix).map(|(entry, _)| format!("gallery-row-{}", entry.name))),
                    );
                }
            }
            ViewMode::Columns => {
                out.push("columns".to_owned());
                for (k, folder) in self.folders.iter().enumerate().take(8) {
                    let folder = folder.read(cx);
                    out.extend(
                        (0..folder.row_count().min(80))
                            .filter_map(|ix| folder.row(ix).map(|(entry, _)| format!("column-{k}-row-{}", entry.name))),
                    );
                }
            }
        }
        if self.filtering.is_some() {
            out.push("filter-field".to_owned());
            out.push("filter-search-footer".to_owned());
        }
        out
    }

    /// Whether a text field inside this pane (the filter or a rename) has focus.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn text_focused(&self, window: &gpui::Window, cx: &gpui::App) -> bool {
        self.filtering.as_ref().is_some_and(|f| f.input.read(cx).focus_handle(cx).is_focused(window))
            || self.renaming.as_ref().is_some_and(|r| r.input.read(cx).focus_handle(cx).is_focused(window))
    }

    /// The focused folder's disclosure probe (see `Folder::disclosure_probe`), for tests.
    #[cfg(test)]
    pub fn disclosure_probe(
        &self,
        path: &std::path::Path,
        ix: usize,
        after: std::time::Duration,
        cx: &gpui::App,
    ) -> Option<(f32, f32, Option<f32>)> {
        self.focused_folder().read(cx).disclosure_probe(path, ix, after)
    }

    /// Whether the focused folder has motion running, for tests.
    #[cfg(test)]
    pub fn animating(&self, cx: &gpui::App) -> bool {
        self.focused_folder().read(cx).animating(Instant::now())
    }

    /// Where the last icon reflow found `path`, even after its glide expired.
    #[cfg(test)]
    pub fn reflow_origin(&self, path: &Path) -> Option<Bounds<Pixels>> {
        self.reflow_capture.as_ref()?.captured_row(path)
    }

    #[cfg(test)]
    pub fn view_motion_origin(&self, path: &Path) -> Option<Bounds<Pixels>> {
        self.view_motion.as_ref()?.captured_row(path)
    }

    /// The feedback effects running on the focused folder's rows, for tests.
    #[cfg(test)]
    pub fn effects(&self, cx: &gpui::App) -> Vec<(String, Fx)> {
        self.focused_folder().read(cx).effects().iter().map(|effect| (effect.entry.name.clone(), effect.fx)).collect()
    }

    pub fn error(&self, cx: &gpui::App) -> Option<SharedString> {
        self.status.clone().or_else(|| self.folders.iter().find_map(|f| f.read(cx).error().cloned()))
    }

    pub fn can_go_back(&self) -> bool {
        self.history.can_go_back()
    }

    pub fn can_go_forward(&self) -> bool {
        self.history.can_go_forward()
    }

    /// The destination Back/Forward would show, for tooltip labels.
    /// The real folders Back would walk through, nearest first; a virtual
    /// location ends the walk, since a spring can only show folders.
    pub fn back_folders(&self) -> Vec<PathBuf> {
        self.history.back_stack().map_while(|location| location.as_folder().map(Path::to_path_buf)).collect()
    }

    pub fn back_target(&self) -> Option<&Location> {
        self.history.back_target()
    }

    pub fn forward_target(&self) -> Option<&Location> {
        self.history.forward_target()
    }

    /// The item the preview and path bar describe: the focused folder's cursor.
    pub fn cursor_entry(&self, cx: &gpui::App) -> Option<Entry> {
        self.focused_folder().read(cx).cursor_entry().cloned()
    }

    /// Test-only: how many rows draw the accent fill and how many draw the
    /// inactive fill, mirroring `render_column_rows`'s predicate.
    #[cfg(test)]
    pub fn selection_fills(&self, window: &gpui::Window, cx: &gpui::App) -> (usize, usize) {
        let active = self.focus.is_focused(window);
        let mut accent = 0;
        let mut inactive = 0;
        for (k, folder) in self.folders.iter().enumerate() {
            let folder = folder.read(cx);
            let rows = folder.entries().iter().filter(|entry| folder.is_selected(&entry.path)).count();
            if k == self.focused && active {
                accent += rows;
            } else {
                inactive += rows;
            }
        }
        (accent, inactive)
    }

    /// The filter count as the live field shows it ("2 of 9 shown").
    fn filter_count_text(&self, cx: &gpui::App) -> String {
        let folder = self.focused_folder().read(cx);
        let mut text = format!("{} of {} shown", folder.row_count(), folder.entries().len());
        if folder.tag_scanning() {
            text.push_str(&format!(", scanning tags {} of {}", folder.tag_scanned(), folder.entries().len()));
        }
        if folder.tag_errors() > 0 {
            text.push_str(&format!(", {} with unreadable tags", folder.tag_errors()));
        }
        text
    }

    /// Queue the filter count for VoiceOver 300 ms after the last keystroke. The
    /// visible count stays live; only the announced label waits, so a burst of
    /// typing produces one announcement instead of one per letter.
    fn schedule_filter_announce(&mut self, cx: &mut Context<Self>) {
        self.filter_announce_gen += 1;
        let generation = self.filter_announce_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(300)).await;
            this.update(cx, |this, cx| {
                if this.filter_announce_gen != generation {
                    return;
                }
                let text: SharedString = this.filter_count_text(cx).into();
                if this.filter_announce != text {
                    this.filter_announce = text;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Test-only: what VoiceOver would hear for the filter count.
    #[cfg(test)]
    pub fn filter_announcement(&self) -> &str {
        &self.filter_announce
    }

    /// What the preview column shows: the cursor entry, or the focused folder
    /// itself when the column has no rows (no matches yet, or still loading), so
    /// the right-hand strip is never an empty quarter. The count is the child
    /// column's for a cursor folder, and the focused folder's own for the fallback.
    pub fn preview_parts(&self, cx: &gpui::App) -> Option<(Entry, Option<usize>)> {
        if let Some(entry) = self.cursor_entry(cx) {
            let items = (entry.is_dir && !entry.is_package()).then(|| self.child_count(&entry, cx)).flatten();
            return Some((entry, items));
        }
        let focused = self.focused_folder().read(cx);
        let dir = focused.dir();
        // The folder's own row in the parent column carries its modified date; the
        // root has no parent, so it shows the count alone.
        let parent = self.focused.checked_sub(1).and_then(|k| self.folders.get(k)).map(|f| f.read(cx));
        let entry = parent
            .and_then(|parent| parent.entries().iter().find(|entry| entry.path == dir).cloned())
            .unwrap_or_else(|| Entry {
                name: crate::style::display_name(dir),
                path: dir.to_path_buf(),
                is_dir: true,
                is_symlink: false,
                hidden: false,
                dataless: false,
                size: 0,
                modified: None,
                created: None,
                added: None,
            });
        // No count while it is still listing or could not be listed: "0 items" would be wrong.
        let items = (!focused.is_loading() && focused.error().is_none()).then(|| focused.entries().len());
        Some((entry, items))
    }

    /// The preview column's aria label: a folder's summary or a file's preview.
    pub fn preview_label(&self, cx: &gpui::App) -> Option<String> {
        let (entry, items) = self.preview_parts(cx)?;
        let size = if entry.is_dir && cx.global::<Ui>().settings.folder_sizes == crow_core::FolderSizesMode::Visible {
            cx.try_global::<SizeService>().and_then(|service| service.0.read(cx).size(&entry.path)).map(Into::into)
        } else {
            None
        };
        if entry.is_dir && !entry.is_package() {
            Some(preview::folder_spoken(&entry.name, items, size))
        } else {
            Some(preview::spoken(&entry, self.preview(cx), size, self.cloud_status(&entry.path, cx)))
        }
    }

    /// The child column's item count, when that column is loaded for `entry`.
    /// The column is already on screen, so this costs no disk I/O.
    fn child_count(&self, entry: &Entry, cx: &gpui::App) -> Option<usize> {
        let child = self.folders.get(self.focused + 1)?.read(cx);
        (!child.is_loading() && child.error().is_none() && child.dir() == entry.path).then(|| child.entries().len())
    }

    pub fn preview(&self, cx: &gpui::App) -> Option<&Preview> {
        let cursor = self.focused_folder().read(cx).cursor_entry().map(|e| e.path.clone())?;
        self.preview.as_ref().filter(|p| p.path == cursor)
    }

    /// What the Info tab shows for this pane: the cursor item, the focused folder
    /// when there is no row, or a multi-selection summary. The key is the path or the
    /// selection generation, so the expensive metadata reloads only on a real change.
    pub fn info_target(&self, cx: &gpui::App) -> (Option<InfoTarget>, Option<InfoKey>) {
        let folder = self.focused_folder().read(cx);
        if folder.selected_count() > 1 {
            let summary = folder.selection_summary();
            let key = summary.key(folder.selection_generation());
            return (Some(InfoTarget::Many(summary)), Some(key));
        }
        let Some((entry, items)) = self.preview_parts(cx) else { return (None, None) };
        let key = InfoKey::Single(entry.path.clone());
        (Some(InfoTarget::Single { entry, items }), Some(key))
    }

    /// A folder's item count when the focused column is that folder and its listing
    /// is loaded; the sidebar's Info target uses it without a disk walk.
    pub fn folder_items(&self, path: &Path, cx: &gpui::App) -> Option<usize> {
        let folder = self.focused_folder().read(cx);
        (folder.dir() == path && !folder.is_loading() && folder.error().is_none()).then(|| folder.entries().len())
    }

    // ----- navigation -----

    /// Go to `dir` as a new history step.
    pub fn navigate(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        self.sidebar_walk = false;
        self.plugin_walk_dirs = None;
        self.visit_location(Location::Folder(dir), false, None, cx);
    }

    fn visit_location(
        &mut self,
        location: Location,
        replace: bool,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        if replace {
            self.history.replace_current(location.clone());
        } else {
            self.history.visit(location.clone());
        }
        self.apply_location(location, None, window, cx);
    }

    /// Open a saved search as the tab's destination. The sidebar arrow only moves
    /// its cursor; activation calls this and starts the result session.
    pub fn open_saved_query(&mut self, id: SavedQueryId, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_walk = false;
        self.plugin_walk_dirs = None;
        self.visit_location(Location::SavedQuery(id), false, Some(window), cx);
    }

    /// Open a Finder tag destination as the tab's location.
    pub fn open_tag(&mut self, id: TagId, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_walk = false;
        self.plugin_walk_dirs = None;
        self.visit_location(Location::Tag(id), false, Some(window), cx);
    }

    fn apply_location(
        &mut self,
        location: Location,
        restore_from: Option<PathBuf>,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        match location {
            Location::Folder(dir) => {
                self.deactivate_saved(cx);
                self.enter(dir, cx);
                if let Some(from) = restore_from {
                    self.folders[0].update(cx, |folder, _| folder.select_after_load(from));
                }
            }
            Location::SavedQuery(id) => self.activate_saved(id, window, cx),
            Location::Tag(id) => self.activate_tag(id, window, cx),
        }
        self.prune_saved_sessions(cx);
        cx.emit(PaneEvent::DestinationChanged);
    }

    fn activate_saved(&mut self, id: SavedQueryId, window: Option<&mut Window>, cx: &mut Context<Self>) {
        self.deactivate_saved(cx);
        self.active_saved = Some(id.clone());
        let Some(window) = window else { return };
        let (home, spec) = {
            let ui = cx.global::<Ui>();
            let Some(spec) = ui.saved_queries.get(&id).cloned() else {
                // A deleted id stays in history as an unavailable destination.
                window.focus(&self.focus, cx);
                return;
            };
            (ui.home.clone(), spec)
        };
        if !self.saved_sessions.contains_key(&id) {
            let view = cx.new(|cx| SearchView::saved(spec, home, self.show_hidden, cx));
            let subscription = cx.subscribe_in(&view, window, |pane, view, event, window, cx| {
                pane.on_saved_event(view.clone(), event, window, cx)
            });
            self.saved_sessions.insert(id.clone(), SavedSession { view, _subscription: subscription });
        }
        if let Some(session) = self.saved_sessions.get(&id) {
            let view = session.view.clone();
            view.update(cx, |view, cx| view.set_active(true, cx));
            if !view.update(cx, |view, cx| view.focus_results(window, cx)) {
                view.update(cx, |view, cx| view.focus_input(window, cx));
            }
        }
    }

    fn activate_tag(&mut self, id: TagId, window: Option<&mut Window>, cx: &mut Context<Self>) {
        self.deactivate_saved(cx);
        self.active_tag = Some(id.clone());
        let Some(window) = window else { return };
        let (home, show_hidden) = {
            let ui = cx.global::<Ui>();
            (ui.home.clone(), self.show_hidden)
        };
        if !self.tag_sessions.contains_key(&id) {
            let view = cx.new(|cx| SearchView::tag(id.clone(), home, show_hidden, cx));
            let subscription = cx.subscribe_in(&view, window, |pane, view, event, window, cx| {
                pane.on_saved_event(view.clone(), event, window, cx)
            });
            self.tag_sessions.insert(id.clone(), SavedSession { view, _subscription: subscription });
        }
        if let Some(session) = self.tag_sessions.get(&id) {
            let view = session.view.clone();
            view.update(cx, |view, cx| view.set_active(true, cx));
            if !view.update(cx, |view, cx| view.focus_results(window, cx)) {
                view.update(cx, |view, cx| view.focus_input(window, cx));
            }
        }
    }

    fn deactivate_saved(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.active_saved.take()
            && let Some(session) = self.saved_sessions.get(&id)
        {
            session.view.update(cx, |view, cx| view.leave_destination(cx));
        }
        if let Some(id) = self.active_tag.take()
            && let Some(session) = self.tag_sessions.get(&id)
        {
            session.view.update(cx, |view, cx| view.leave_destination(cx));
        }
    }

    fn prune_saved_sessions(&mut self, cx: &mut Context<Self>) {
        let retained: HashSet<SavedQueryId> =
            self.history.locations().filter_map(|location| location.saved_query_id().cloned()).collect();
        let remove: Vec<SavedQueryId> =
            self.saved_sessions.keys().filter(|id| !retained.contains(*id)).cloned().collect();
        for id in remove {
            if let Some(session) = self.saved_sessions.remove(&id) {
                session.view.update(cx, |view, cx| {
                    view.set_active(false, cx);
                    view.cancel(cx);
                });
            }
        }
        let retained: HashSet<TagId> =
            self.history.locations().filter_map(|location| location.tag_id().cloned()).collect();
        let remove: Vec<TagId> = self.tag_sessions.keys().filter(|id| !retained.contains(*id)).cloned().collect();
        for id in remove {
            if let Some(session) = self.tag_sessions.remove(&id) {
                session.view.update(cx, |view, cx| {
                    view.set_active(false, cx);
                    view.cancel(cx);
                });
            }
        }
    }

    fn on_saved_event(
        &mut self,
        view: Entity<SearchView>,
        event: &SearchEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SearchEvent::Close => {}
            SearchEvent::FocusResults => {
                if !view.update(cx, |search, cx| search.focus_results(window, cx)) {
                    cx.emit(PaneEvent::FocusSidebar);
                }
            }
            SearchEvent::Open { path, is_dir } => {
                if *is_dir {
                    self.navigate(path.clone(), cx);
                } else {
                    self.open_file(path.clone(), cx);
                }
            }
            SearchEvent::Reveal(path) => self.reveal(path.clone(), cx),
            SearchEvent::Rename(path) => self.rename_search_result(path.clone(), cx),
            SearchEvent::Run(request) => cx.emit(PaneEvent::Run(request.clone())),
            SearchEvent::Copy { paths, cut } => cx.emit(PaneEvent::Copy { paths: paths.clone(), cut: *cut }),
            SearchEvent::Save => cx.emit(PaneEvent::SaveSearch),
            SearchEvent::BatchRename(entries) => cx.emit(PaneEvent::BatchRename(entries.clone())),
        }
    }

    /// A tab switch: only the active tab's saved session may rerun on its own.
    pub fn set_tab_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if let Some(session) = self.saved_session() {
            session.update(cx, |view, cx| view.set_active(active, cx));
        }
    }

    /// After a file operation from a saved result, rerun the visible destination.
    pub fn refresh_saved(&mut self, cx: &mut Context<Self>) {
        if let Some(view) = self.saved_session() {
            view.update(cx, |view, cx| view.refresh_now(cx));
        }
    }

    /// Show `dir` while the sidebar cursor moves. The first move is a normal history
    /// step (Back returns to where the walk began); later moves replace it, so a walk
    /// past five favorites is one Cmd-[.
    pub fn show_from_sidebar(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        if self.location() == &Location::Folder(dir.clone()) {
            return;
        }
        if self.sidebar_walk {
            self.history.replace_current(Location::Folder(dir.clone()));
        } else {
            // Freeze the section's contributing folder before the first preview
            // changes the focused listing.
            self.plugin_walk_dirs = Some(self.sidebar_plugin_dirs(false, cx));
            self.history.visit(Location::Folder(dir.clone()));
            self.sidebar_walk = true;
        }
        self.deactivate_saved(cx);
        self.enter(dir, cx);
    }

    /// Return, Cmd-Down, → or a click in the sidebar: end the walk and go there if
    /// the pane isn't showing it already.
    pub fn open_from_sidebar(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        self.sidebar_walk = false;
        self.plugin_walk_dirs = None;
        if self.location() != &Location::Folder(dir.clone()) {
            self.navigate(dir, cx);
        }
    }

    /// Show `path` selected in its folder (Go to Folder with a file, Finder's "Reveal").
    pub fn reveal(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if let Some(parent) = path.parent().map(Path::to_path_buf) {
            self.navigate(parent, cx);
            self.folders[0].update(cx, |f, _| f.select_after_load(path));
        }
    }

    pub fn reveal_search_result(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if let Some(parent) = path.parent().map(Path::to_path_buf) {
            self.navigate(parent, cx);
            self.folders[0].update(cx, |folder, _| folder.reveal_after_load(path));
        }
    }

    pub fn rename_search_result(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.reveal_search_result(path.clone(), cx);
        self.rename_when_present = Some(path);
        cx.notify();
    }

    pub fn go_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_walk = false;
        self.plugin_walk_dirs = None;
        let from = self.location().as_folder().map(Path::to_path_buf);
        let Some(location) = self.history.back().cloned() else { return };
        self.apply_location(location.clone(), from, Some(window), cx);
        if let Some((origin, paths)) = self.return_selection.take()
            && origin == location
            && origin.as_folder().is_some()
            && let Some(folder) = self.folders.first().cloned()
        {
            // Surviving source rows come back selected; moved-away paths simply
            // are not present.
            folder.update(cx, |folder, cx| folder.select_survivors_after_load(paths, cx));
        }
    }

    pub fn go_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_walk = false;
        self.plugin_walk_dirs = None;
        let Some(location) = self.history.forward().cloned() else { return };
        self.apply_location(location, None, Some(window), cx);
    }

    /// Show a spring-loaded folder. The first spring keeps the origin's history,
    /// live listings, selection and scroll; later springs only replace the
    /// temporary presentation. No Recent entry or history step is recorded.
    pub fn spring_enter(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        if self.spring.is_none() {
            let virtual_origin = self.location().as_folder().is_none();
            let origin = SpringOrigin {
                history: self.history.clone(),
                folders: std::mem::take(&mut self.folders),
                subscriptions: std::mem::take(&mut self.subscriptions),
                focused: self.focused,
                mode: self.mode,
                sort: self.sort,
                applied_view: self.applied_view.clone(),
                virtual_origin,
            };
            self.spring = Some(origin);
            self.deactivate_saved(cx);
        }
        self.sidebar_walk = false;
        self.plugin_walk_dirs = None;
        self.history.replace_current(Location::Folder(dir.clone()));
        self.enter_listing(dir, false, cx);
        cx.emit(PaneEvent::DestinationChanged);
        cx.notify();
    }

    /// Accept the drop: the final sprung folder becomes one normal Back step from
    /// the origin. The folder is already listed, so this does not enter it again.
    pub fn spring_commit(&mut self, dropped: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(origin) = self.spring.take() else { return };
        let Some(final_dir) = self.folders.get(self.focused).map(|folder| folder.read(cx).dir().to_path_buf()) else {
            return;
        };
        let origin_location = origin.history.current().clone();
        for folder in origin.folders {
            folder.update(cx, |folder, cx| folder.release_listing(cx));
        }
        let mut history = origin.history;
        history.visit(Location::Folder(final_dir.clone()));
        self.history = history;
        self.return_selection = Some((origin_location, dropped));
        self.sidebar_walk = false;
        self.plugin_walk_dirs = None;
        cx.emit(PaneEvent::DestinationChanged);
        cx.emit(PaneEvent::Entered(final_dir));
        cx.notify();
    }

    /// Cancel the drag: restore the origin exactly, including its live listings'
    /// selection and scroll. A virtual origin reopens its saved result session.
    pub fn spring_cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(origin) = self.spring.take() else { return };
        self.release_listings(cx);
        self.folders.clear();
        self.subscriptions.clear();
        self.focused = 0;
        self.history = origin.history;
        if origin.virtual_origin {
            let location = self.history.current().clone();
            self.apply_location(location, None, Some(window), cx);
        } else {
            self.folders = origin.folders;
            self.subscriptions = origin.subscriptions;
            self.focused = origin.focused.min(self.folders.len().saturating_sub(1));
            self.mode = origin.mode;
            self.sort = origin.sort;
            self.applied_view = origin.applied_view;
            self.sidebar_walk = false;
            self.plugin_walk_dirs = None;
            self.plugin_scope_changed(cx);
            cx.emit(PaneEvent::DestinationChanged);
            cx.notify();
        }
    }

    pub fn open_parent(&mut self, cx: &mut Context<Self>) {
        let Some(child) = self.focused_path(cx) else { return };
        if let Some(parent) = child.parent().map(Path::to_path_buf) {
            self.navigate(parent, cx);
            self.folders[0].update(cx, |f, _| f.select_after_load(child));
        }
    }

    /// After an operation: list again now and select what it produced; a new folder
    /// also starts its rename, like Finder.
    pub fn show_results(&mut self, paths: Vec<PathBuf>, rename: bool, cx: &mut Context<Self>) {
        self.rename_when_present = if rename { paths.first().cloned() } else { None };
        let here = self.focused_folder().clone();
        let dir = here.read(cx).dir().to_path_buf();
        let mine: Vec<PathBuf> = paths.into_iter().filter(|p| p.parent() == Some(dir.as_path())).collect();
        let mode = cx.global::<Ui>().feedback.mode;
        let mut mine = Some(mine);
        for folder in self.folders.clone() {
            if folder == here
                && let Some(mine) = mine.take().filter(|mine| !mine.is_empty())
            {
                folder.update(cx, |f, cx| f.select_when_present(mine, mode, cx));
                continue;
            }
            folder.update(cx, |f, cx| f.reload(cx));
        }
    }

    /// Start a feedback effect on the focused folder's visible rows among `paths`.
    pub fn start_effect(&mut self, paths: &[PathBuf], fx: Fx, cx: &mut Context<Self>) {
        let mode = cx.global::<Ui>().feedback.mode;
        let folder = self.focused_folder().clone();
        folder.update(cx, |folder, cx| folder.start_effect(paths, fx, mode, cx));
    }

    /// Free the open folders' listings off the main thread before this pane is
    /// dropped (closing a tab or a window).
    pub fn release_listings(&mut self, cx: &mut Context<Self>) {
        if cx.try_global::<crate::plugins::Plugins>().is_some() {
            let ids: Vec<_> = self.folders.iter().map(|folder| folder.read(cx).plugin_surface()).collect();
            let surfaces = &mut cx.global_mut::<crate::plugins::Plugins>().surfaces;
            for id in ids {
                surfaces.drop_surface(id);
            }
        }
        self.plugin_selection.clear();
        self.plugin_scope = None;
        let sessions: Vec<SavedSession> = self
            .saved_sessions
            .drain()
            .map(|(_, session)| session)
            .chain(self.tag_sessions.drain().map(|(_, session)| session))
            .collect();
        for session in sessions {
            if cx.try_global::<crate::plugins::Plugins>().is_some() {
                let id = session.view.read(cx).plugin_surface();
                cx.global_mut::<crate::plugins::Plugins>().surfaces.drop_surface(id);
            }
            session.view.update(cx, |view, cx| {
                view.set_active(false, cx);
                view.cancel(cx);
            });
        }
        self.active_saved = None;
        self.active_tag = None;
        self.preview_cancel.store(true, Ordering::Relaxed);
        self.clear_preview(cx);
        self.preview_key = None;
        gallery::release(self, cx);
        if cx.try_global::<crate::plugins::Plugins>().is_some() {
            let id = self.gallery.replace_surface();
            cx.global_mut::<crate::plugins::Plugins>().surfaces.drop_surface(id);
        }
        for folder in &self.folders {
            folder.update(cx, |folder, cx| folder.release_listing(cx));
        }
    }

    /// Keep the selection and cursor on the same items after a batch rename.
    /// The listing itself settles on the watcher's final names.
    pub fn apply_rename_map(
        &mut self,
        map: &std::collections::HashMap<PathBuf, PathBuf>,
        cursor: Option<&Path>,
        cx: &mut Context<Self>,
    ) {
        if map.is_empty() {
            return;
        }
        for folder in self.folders.clone() {
            folder.update(cx, |folder, cx| folder.remap_selection(map, cursor, cx));
        }
        // A saved search or tag shows its results from its own session, which keeps
        // selection and cursor by path; bring them to the new names first.
        if let Some(session) = self.saved_session() {
            session.update(cx, |session, cx| session.apply_rename_map(map, cx));
        }
        self.load_preview(cx);
        self.refresh_saved(cx);
        cx.notify();
    }

    /// The selection cursor's path, even before the listing catches up.
    pub fn selection_cursor(&self, cx: &gpui::App) -> Option<PathBuf> {
        self.focused_folder().read(cx).selection_cursor().map(Path::to_path_buf)
    }

    /// The selected items of the focused folder, in display order.
    pub fn selected_paths(&self, cx: &gpui::App) -> Vec<PathBuf> {
        self.focused_folder().read(cx).selected_paths()
    }

    /// The selected items of the focused folder as (path, is a folder), in display order.
    pub fn selected_entries(&self, cx: &gpui::App) -> crate::folder::SelectionEntries {
        self.focused_folder().read(cx).selected_entries()
    }

    /// Cancel a Vim mark waiting for its letter. Returns whether one was pending.
    pub fn cancel_pending(&mut self, cx: &mut Context<Self>) -> bool {
        let marquee = self.cancel_marquee(cx);
        let pending = self.mark_pending.take().is_some();
        if pending {
            cx.notify();
        }
        pending || marquee
    }

    #[cfg(test)]
    pub fn marquee_active(&self) -> bool {
        self.marquee.as_ref().is_some_and(|marquee| marquee.started)
    }

    #[cfg(test)]
    pub fn marquee_hit_names(&self) -> Vec<String> {
        let Some(marquee) = &self.marquee else { return Vec::new() };
        let mut names: Vec<String> =
            marquee.hits.keys().filter_map(|path| path.file_name()).map(|name| name.to_string_lossy().into()).collect();
        names.sort();
        names
    }

    /// The focused folder's type-to-select prefix, while its run is active.
    pub fn type_select_prefix(&self, cx: &App) -> Option<String> {
        let folder = self.focused_folder().read(cx);
        folder.type_select_active().then(|| folder.type_select_prefix().to_owned())
    }

    /// End the focused folder's type-to-select run. Returns whether one was active.
    pub fn cancel_type_select(&mut self, cx: &mut Context<Self>) -> bool {
        let folder = self.focused_folder().clone();
        folder.update(cx, |folder, cx| folder.cancel_type_select(cx))
    }

    /// Clear the focused folder's selection. Returns whether anything was selected.
    pub fn clear_selection(&mut self, cx: &mut Context<Self>) -> bool {
        let had_selection = self.focused_folder().read(cx).selected_count() > 0;
        if had_selection {
            self.with_focused(cx, |folder, cx| folder.clear_selection(cx));
            self.load_preview(cx);
            cx.notify();
        }
        had_selection
    }

    /// The current folder's subfolders, for the command palette's go-to list.
    pub fn subfolders(&self, cx: &gpui::App) -> Vec<PathBuf> {
        self.focused_folder()
            .read(cx)
            .entries()
            .iter()
            .filter(|entry| entry.is_dir && !entry.is_package())
            .map(|entry| entry.path.clone())
            .collect()
    }

    /// Run one of the palette's pane commands through the same path as its key.
    pub fn run_command(&mut self, command: CommandId, window: &mut Window, cx: &mut Context<Self>) {
        match command {
            CommandId::Open => self.open_selected(cx),
            CommandId::QuickLook => self.quick_look(&QuickLook, window, cx),
            CommandId::Rename => self.rename(&Rename, window, cx),
            CommandId::NewFolder => self.new_folder(&NewFolder, window, cx),
            CommandId::NewFile => self.new_file(&NewFile, window, cx),
            CommandId::Duplicate => self.duplicate(&Duplicate, window, cx),
            CommandId::MoveToTrash => self.move_to_trash(&MoveToTrash, window, cx),
            CommandId::DeletePermanently => self.delete_permanently(&DeletePermanently, window, cx),
            CommandId::Copy => self.copy_files(&CopyFiles, window, cx),
            CommandId::Cut => self.cut_files(&CutFiles, window, cx),
            CommandId::Paste => self.paste_files(&PasteFiles, window, cx),
            CommandId::PasteMove => self.paste_move_files(&PasteMoveFiles, window, cx),
            CommandId::CopyPath => self.copy_path(&CopyPath, window, cx),
            CommandId::ShowInFinder => self.show_in_finder(&ShowInFinder, window, cx),
            CommandId::OpenTerminal => self.open_terminal(&OpenTerminal, window, cx),
            CommandId::SelectAll => self.select_all(&SelectAll, window, cx),
            CommandId::StartFilter => self.start_filter(&StartFilter, window, cx),
            CommandId::ChooseTags => self.choose_tags(&ChooseTags, window, cx),
            _ => {}
        }
    }

    pub fn set_show_hidden(&mut self, show: bool, cx: &mut Context<Self>) {
        self.show_hidden = show;
        for folder in &self.folders {
            folder.update(cx, |f, cx| f.set_show_hidden(show, cx));
        }
    }

    /// Sort as the user asked (a header click), and remember it for this folder.
    pub fn set_sort(&mut self, sort: Sort, cx: &mut Context<Self>) {
        if self.apply_sort(sort, cx) {
            self.remember(ViewChange::Sort(sort), cx);
        }
    }

    fn apply_sort(&mut self, sort: Sort, cx: &mut Context<Self>) -> bool {
        if self.sort == sort {
            return false;
        }
        self.sort = sort;
        let service = cx.try_global::<SizeService>().map(|service| service.0.clone());
        for folder in &self.folders {
            let sizes = if sort.key == SortKey::Size {
                service.as_ref().map(|service| {
                    let service = service.read(cx);
                    let folder = folder.read(cx);
                    Arc::new(
                        folder
                            .entries()
                            .iter()
                            .filter(|entry| entry.is_dir && !entry.is_symlink)
                            .filter_map(|entry| service.size(&entry.path).map(|size| (entry.path.clone(), size.bytes)))
                            .collect::<HashMap<_, _>>(),
                    )
                })
            } else {
                None
            };
            folder.update(cx, |f, cx| f.set_sort_with_sizes(sort, sizes, cx));
        }
        cx.notify();
        true
    }

    /// The cloud-status service changed: repaint, and re-arm a blocked preview
    /// once the cursor item is positively local/downloaded. It does not open the
    /// explicit Quick Look panel.
    fn cloud_changed(&mut self, cx: &mut Context<Self>) {
        if self.folder().is_none() {
            return;
        }
        let Some(entry) = self.cursor_entry(cx) else {
            cx.notify();
            return;
        };
        if let Some(status) = self.cloud_status(&entry.path, cx)
            && let Some(preview) = self.preview.as_mut().filter(|preview| preview.path == entry.path)
            && preview.update_cloud(status)
        {
            self.clear_preview(cx);
            self.preview_key = None;
            self.load_preview(cx);
        }
        gallery::cloud_changed(self, cx);
        cx.notify();
    }

    /// The size service changed: once every visible size is in, re-sort by size
    /// once (Q3). Scrolling in more rows repeats this when their sizes arrive.
    fn sizes_changed(&mut self, cx: &mut Context<Self>) {
        if cx.global::<Ui>().settings.folder_sizes != crow_core::FolderSizesMode::Visible {
            return;
        }
        let service = cx.global::<SizeService>().0.clone();
        let generation = service.read(cx).generation();
        if generation == self.size_sort_generation || self.sort.key != SortKey::Size {
            return;
        }
        if !service.read(cx).visible_complete() {
            return;
        }
        self.size_sort_generation = generation;
        for folder in &self.folders {
            let sizes = {
                let service = service.read(cx);
                let folder = folder.read(cx);
                Arc::new(
                    folder
                        .entries()
                        .iter()
                        .filter(|entry| entry.is_dir && !entry.is_symlink)
                        .filter_map(|entry| service.size(&entry.path).map(|size| (entry.path.clone(), size.bytes)))
                        .collect::<HashMap<_, _>>(),
                )
            };
            folder.update(cx, |f, cx| f.resort_with_sizes(Some(sizes), 0, cx));
        }
        cx.notify();
    }

    #[cfg(test)]
    pub fn sort(&self) -> Sort {
        self.sort
    }

    /// Switch view as the user asked (Cmd-2, Cmd-3), and remember it for the folder
    /// shown, unless views don't follow folders: then it is this window's alone.
    pub fn set_mode(&mut self, mode: ViewMode, cx: &mut Context<Self>) {
        if self.folder().is_none() || self.mode == mode {
            return;
        }
        self.cancel_marquee(cx);
        let motion = ViewMotion::capture(self, cx.global::<Ui>().feedback.mode, Instant::now(), cx);
        self.apply_mode(mode, cx);
        self.view_motion = motion;
        self.child_reveal = None;
        let folder = self.focused_folder().read(cx);
        if let Some(ix) = folder.cursor_ix() {
            folder.scroll().scroll_to_item(ix, ScrollStrategy::Nearest);
        }
        if mode == ViewMode::Columns {
            self.strip_scroll.scroll_to_item(self.focused);
        }
        if mode == ViewMode::Icons
            && let Some(ix) = self.focused_folder().read(cx).cursor_ix()
        {
            let count = self.focused_folder().read(cx).row_count();
            self.icon_scroll.scroll_to_item(self.icon_grid(count, cx).row_of(ix), ScrollStrategy::Nearest);
        }
        if mode == ViewMode::Gallery {
            self.gallery.reveal = Some(Instant::now());
            if let Some(ix) = self.focused_folder().read(cx).cursor_ix() {
                self.gallery.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
            }
        }
        if cx.global::<Ui>().settings.view_per_folder {
            self.remember(ViewChange::View(mode), cx);
        }
    }

    /// Write a view change to `views.toml` as this folder's own setting.
    fn remember(&mut self, change: ViewChange, cx: &mut Context<Self>) {
        let Some(dir) = self.location().as_folder().map(Path::to_path_buf) else { return };
        crate::ui::save_view_change(dir, change, cx);
    }

    /// Show this folder the way `views.toml` says, unless that was already done for
    /// this folder and this version of the file.
    pub fn apply_view(&mut self, cx: &mut Context<Self>) {
        if self.location().as_folder().is_none() {
            return;
        }
        let ui = cx.global::<Ui>();
        let plugin_generation =
            cx.try_global::<crate::plugins::Plugins>().map_or(0, |plugins| plugins.surfaces.generation());
        let key = (ui.views_generation, plugin_generation, self.dir().to_path_buf());
        if self.applied_view.as_ref() == Some(&key) {
            return;
        }
        let resolved = ui.views.resolve(&key.2);
        let sources: Vec<(Entity<Folder>, Option<String>)> = self
            .folders
            .iter()
            .map(|folder| {
                let dir = folder.read(cx).dir().to_path_buf();
                (folder.clone(), ui.views.resolve(&dir).icon_source.map(|(source, _)| source))
            })
            .collect();
        for (folder, source) in sources {
            folder.update(cx, |folder, _| folder.set_icon_source(source));
        }
        self.applied_view = Some(key);
        if cx.global::<Ui>().settings.view_per_folder {
            self.icon_size = resolved.icon_size.0;
            self.icon_caption = resolved.icon_caption.0.clone();
        }
        self.apply_sort(resolved.sort(), cx);
        let ids: Vec<&str> = self.columns.iter().map(|c| c.id()).collect();
        if ids != resolved.columns.0 {
            let mut available = cx.global::<Ext>().0.columns().to_vec();
            if let Some(plugins) = cx.try_global::<crate::plugins::Plugins>() {
                let snapshot = plugins.surfaces.snapshot();
                for (_, published) in snapshot.of_kind(crow_core::ExtensionKind::Column) {
                    if !plugins.surfaces.is_suppressed(&published.descriptor.id.plugin) {
                        available.push(Arc::new(crate::plugins::PluginColumn::new(&published.descriptor)));
                    }
                }
            }
            self.columns =
                resolved.columns.0.iter().filter_map(|id| available.iter().find(|c| c.id() == id).cloned()).collect();
            self.table = default_table(&self.columns);
        }
        // Columns are one browsing session: entering a folder may switch to column
        // view, but a views.toml reload while its columns are open (the parent's
        // setting, say) must not yank the mode out from under the chain.
        // With `view_per_folder = false` the window keeps its view.
        let per_folder = cx.global::<Ui>().settings.view_per_folder;
        if per_folder && (self.mode != ViewMode::Columns || self.folders.len() == 1) {
            self.apply_mode(self.bench_view.unwrap_or(resolved.view.0), cx);
        }
        cx.notify();
    }

    /// Switch between list and column view, staying in the same folder with the same selection.
    fn apply_mode(&mut self, mode: ViewMode, cx: &mut Context<Self>) -> bool {
        if self.mode == mode {
            return false;
        }
        self.cancel_marquee(cx);
        self.view_motion = None;
        self.child_reveal = None;
        self.drop_filter(cx);
        if self.mode == ViewMode::Gallery {
            gallery::release(self, cx);
        }
        self.mode = mode;
        match mode {
            // The single-folder views show the folder you were in: the focused
            // column becomes the root. Icons projects its direct entries only.
            ViewMode::List | ViewMode::Icons | ViewMode::Gallery => {
                self.collapse_to_focused(mode == ViewMode::List, cx)
            }
            ViewMode::Columns => self.update_chain(0, cx),
        }
        self.plugin_scope_changed(cx);
        if mode == ViewMode::Gallery {
            self.load_preview(cx);
            gallery::refresh(self, cx);
        }
        cx.notify();
        true
    }

    /// The grid the pane would draw right now. Until the first icon frame, the
    /// last known viewport height (or the window's) decides the visible rows;
    /// column count and tile bounds only need the width and the listing.
    fn icon_grid(&self, count: usize, cx: &App) -> GridGeometry {
        let width = f32::from(self.content_width).max(1.0);
        let height = self
            .icon_bounds
            .get()
            .map(|bounds| f32::from(bounds.size.height))
            .or_else(|| self.grid.map(|grid| grid.viewport_height()))
            .unwrap_or(1.0)
            .max(1.0);
        let mut metrics = self.icon_size.metrics();
        let name_height = 2.0 * icon_name_line_height(cx.global::<Ui>().settings.font_size);
        let content_height = ICON_TILE_TOP + metrics.icon_size + ICON_BOX_PADDING + ICON_TILE_GAP + name_height;
        metrics.tile_height = metrics.tile_height.max(content_height + 1.0);
        if self.icon_caption.is_some() {
            metrics.tile_height += ICON_TILE_GAP + CAPTION_LINE_HEIGHT;
        }
        GridGeometry::new(count, width, height, metrics)
    }

    /// Configure the plugin column under tile names; `None` is Off. The tile
    /// height changes when the slot appears, so the renderer restores the anchor.
    pub fn set_icon_caption(&mut self, caption: Option<String>, cx: &mut Context<Self>) {
        if self.icon_caption == caption {
            return;
        }
        self.cancel_marquee(cx);
        self.icon_caption = caption;
        if cx.global::<Ui>().settings.view_per_folder {
            self.remember(ViewChange::IconCaption(self.icon_caption.clone()), cx);
        }
        cx.notify();
    }

    pub fn icon_caption(&self) -> Option<&str> {
        self.icon_caption.as_deref()
    }

    /// Switch the grid preset. Selected paths and the filter stay; the renderer
    /// restores the visible anchor against the new geometry.
    pub fn set_icon_size(&mut self, size: IconSize, cx: &mut Context<Self>) {
        if self.icon_size == size {
            return;
        }
        self.cancel_marquee(cx);
        self.icon_size = size;
        if cx.global::<Ui>().settings.view_per_folder {
            self.remember(ViewChange::IconSize(size), cx);
        }
        cx.notify();
    }

    /// Collapse a column chain to the focused folder, for List and Icons.
    fn collapse_to_focused(&mut self, show_tree: bool, cx: &mut Context<Self>) {
        for (index, folder) in self.folders.iter().enumerate() {
            if index != self.focused {
                folder.update(cx, |folder, cx| folder.release_listing(cx));
            }
        }
        let keep = self.folders.remove(self.focused);
        let subscription = self.subscriptions.remove(self.focused);
        let dir = keep.read(cx).dir().to_path_buf();
        keep.update(cx, |folder, cx| folder.set_show_tree(show_tree, cx));
        self.folders = vec![keep];
        self.subscriptions = vec![subscription];
        self.focused = 0;
        self.history.visit(Location::Folder(dir));
    }

    fn enter(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        self.enter_listing(dir.clone(), true, cx);
        cx.emit(PaneEvent::Entered(dir));
        cx.notify();
    }

    /// Show `dir`, optionally applying its folder view. Spring uses the current
    /// presentation and emits no navigation event of its own.
    fn enter_listing(&mut self, dir: PathBuf, apply_folder_view: bool, cx: &mut Context<Self>) {
        self.cancel_marquee(cx);
        self.view_motion = None;
        self.child_reveal = None;
        if self.filtering.take().is_some() {
            // Navigating away ends the filter session like Esc does.
            self.emit_filter_end(cx);
            self.focus_list_after_filter = true;
        }
        self.release_listings(cx);
        if let Some(service) = cx.try_global::<CloudStatusService>() {
            service.0.clone().update(cx, |service, cx| service.forget(self.cloud_source, cx));
        }
        self.folders.clear();
        self.subscriptions.clear();
        self.focused = 0;
        self.preview_generation += 1;
        self.status = None;
        // The folder's view decides the sort before the listing starts.
        let ui = cx.global::<Ui>();
        let resolved = ui.views.resolve(&dir);
        self.sort = resolved.sort();
        self.push_folder(dir, true, cx);
        self.plugin_scope_changed(cx);
        if apply_folder_view {
            self.apply_view(cx);
        }
        cx.notify();
    }

    fn push_folder(&mut self, dir: PathBuf, auto_select: bool, cx: &mut Context<Self>) {
        let (hidden, sort) = (self.show_hidden, self.sort);
        let icon_source = cx.global::<Ui>().views.resolve(&dir).icon_source.map(|(source, _)| source);
        let show_tree = self.mode == ViewMode::List || self.mode == ViewMode::Columns;
        let folder = cx.new(|cx| Folder::new(dir, hidden, sort, auto_select, cx));
        folder.update(cx, |folder, cx| {
            folder.set_icon_source(icon_source);
            folder.set_show_tree(show_tree, cx);
        });
        self.subscriptions
            .push(cx.subscribe(&folder, |this, folder, event, cx| this.on_folder_event(folder, event, cx)));
        self.folders.push(folder);
        // Keep the newest column in view.
        self.strip_scroll.scroll_to_item(self.folders.len() - 1);
    }

    fn truncate(&mut self, len: usize, cx: &mut Context<Self>) {
        for k in len..self.folders.len() {
            self.observe_cloud((self.cloud_source, k + 1), &[], cx);
        }
        for folder in &self.folders[len..] {
            folder.update(cx, |folder, cx| folder.release_listing(cx));
        }
        self.folders.truncate(len);
        self.subscriptions.truncate(len);
        self.focused = self.focused.min(len.saturating_sub(1));
    }

    fn index_of(&self, id: EntityId) -> Option<usize> {
        self.folders.iter().position(|f| f.entity_id() == id)
    }

    /// Announce the focused folder again even when it did not change in this
    /// pane. A tab activation or the end of a search is a scope transition for
    /// consumers even if the same pane already announced that folder.
    pub fn announce_plugin_scope(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.saved_session() {
            session.update(cx, |session, cx| session.notify_plugin_scope(cx));
            return;
        }
        self.plugin_scope = None;
        self.plugin_scope_changed(cx);
    }

    /// Announce a committed change of the focused column's real folder.
    fn plugin_scope_changed(&mut self, cx: &mut Context<Self>) {
        let Some(folder) = self.folders.get(self.focused).cloned() else { return };
        let (surface, dir) = {
            let folder = folder.read(cx);
            (folder.plugin_surface(), folder.dir().to_path_buf())
        };
        let scope = crate::plugins::ScopeKey::Folder(dir.clone());
        if self.plugin_scope.as_ref().is_some_and(|(s, q)| *s == surface && q == &scope) {
            return;
        }
        self.plugin_scope = Some((surface, scope));
        if cx.try_global::<crate::plugins::Plugins>().is_some() {
            cx.global_mut::<crate::plugins::Plugins>().surfaces.notify_scope_changed(
                surface,
                dir.display().to_string(),
                Instant::now(),
            );
        }
    }

    fn on_folder_event(&mut self, folder: Entity<Folder>, event: &FolderEvent, cx: &mut Context<Self>) {
        let Some(k) = self.index_of(folder.entity_id()) else { return };
        if self.marquee.is_some()
            && matches!(event, FolderEvent::Loaded | FolderEvent::Changed | FolderEvent::WatcherApplied { .. })
        {
            // The rows under the rectangle changed: indices from the old layout
            // must not select whatever moved into them.
            self.cancel_marquee(cx);
        }
        if matches!(event, FolderEvent::Loaded | FolderEvent::Changed)
            && let Some(service) = cx.try_global::<CloudStatusService>()
        {
            let dir = folder.read(cx).dir().to_path_buf();
            let service = service.0.clone();
            service.update(cx, |service, cx| service.invalidate_under(&dir, cx));
        }
        match event {
            FolderEvent::SelectionChanged => {
                if k == self.focused {
                    self.view_motion = None;
                }
                let (surface, dir, paths) = {
                    let folder = folder.read(cx);
                    (folder.plugin_surface(), folder.dir().to_path_buf(), folder.selected_paths())
                };
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                paths.hash(&mut hasher);
                let signature = hasher.finish();
                let previous = self.plugin_selection.insert(k, signature);
                if previous != Some(signature)
                    && k == self.focused
                    && cx.try_global::<crate::plugins::Plugins>().is_some()
                {
                    cx.global_mut::<crate::plugins::Plugins>().surfaces.notify_selection_changed(
                        surface,
                        dir.display().to_string(),
                        Instant::now(),
                    );
                }
            }
            FolderEvent::WatcherApplied { paths } => {
                let (surface, dir) = {
                    let folder = folder.read(cx);
                    (folder.plugin_surface(), folder.dir().to_path_buf())
                };
                let scope = crate::plugins::ScopeKey::Folder(dir.clone());
                if cx.try_global::<crate::plugins::Plugins>().is_some() {
                    let surfaces = &mut cx.global_mut::<crate::plugins::Plugins>().surfaces;
                    if paths.is_empty() {
                        surfaces.bump_scope(surface, scope.clone(), 0, Instant::now());
                    } else {
                        surfaces.invalidate_paths(&scope, paths);
                    }
                    surfaces.notify_files_changed(scope, dir.display().to_string(), Instant::now());
                }
            }
            _ => {}
        }
        if self.folder().is_none() {
            return;
        }
        if let FolderEvent::CursorMoved = event {
            if self.mode == ViewMode::Columns {
                self.update_chain(k, cx);
            }
            if self.mode == ViewMode::Icons
                && k == self.focused
                && let Some(ix) = folder.read(cx).cursor_ix()
            {
                let count = folder.read(cx).row_count();
                self.icon_scroll.scroll_to_item(self.icon_grid(count, cx).row_of(ix), ScrollStrategy::Nearest);
            }
            if self.mode == ViewMode::Gallery
                && k == self.focused
                && let Some(ix) = folder.read(cx).cursor_ix()
            {
                self.gallery.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
            }
            if k == self.focused && self.mode != ViewMode::Gallery {
                self.load_preview(cx);
            }
        }
        if self.mode == ViewMode::Gallery {
            self.load_preview(cx);
            gallery::refresh(self, cx);
        }
        cx.notify();
    }

    /// Column view: after folder `k`'s cursor moved, show the selected folder's
    /// contents in the next column (unfocused), or nothing if a file or several
    /// items are selected. Deeper columns that no longer follow are closed.
    fn update_chain(&mut self, k: usize, cx: &mut Context<Self>) {
        let folder = self.folders[k].read(cx);
        let child = folder
            .cursor_entry()
            .filter(|e| e.is_dir && !e.is_package() && folder.selected_count() == 1)
            .map(|e| e.path.clone());
        let next_matches = |this: &Self, path: &Path| this.folders.get(k + 1).is_some_and(|f| f.read(cx).dir() == path);
        match child {
            Some(path) if next_matches(self, &path) => {}
            Some(path) => {
                let opens_child = k == self.focused
                    && k + 1 == self.folders.len()
                    && self.presented_layout == Some((ViewMode::Columns, self.folders[k].entity_id()))
                    && self.view_motion.is_none()
                    && cx.global::<Ui>().feedback.mode != Effective::Off;
                self.truncate(k + 1, cx);
                self.push_folder(path, false, cx);
                if opens_child {
                    self.child_reveal = Some((self.folders[k + 1].entity_id(), Instant::now()));
                }
            }
            None => self.truncate(k + 1, cx),
        }
    }

    fn clear_preview(&mut self, cx: &mut gpui::App) {
        if let Some(mut preview) = self.preview.take() {
            preview.release_image(cx);
        }
    }

    fn load_preview(&mut self, cx: &mut Context<Self>) {
        let surface = self.current_plugin_surface(cx);
        let Some(entry) = self.cursor_entry(cx).filter(|_| self.folder().is_some()) else {
            self.clear_preview(cx);
            self.preview_key = None;
            self.preview_generation += 1;
            self.preview_cancel.store(true, Ordering::Relaxed);
            crate::plugins::cancel_previews(surface, cx);
            return;
        };
        let revision = self.focused_folder().read(cx).plugin_entry_version(&entry);
        let image_size = if self.mode == ViewMode::Gallery {
            preview::ImageSize::Gallery(self.gallery.primary_worker.clone())
        } else {
            preview::ImageSize::Pane
        };
        let image_points = image_size.points();
        if let Some((path, old_revision, old_points)) = &self.preview_key {
            if *path == entry.path && *old_revision == revision {
                if *old_points >= image_points {
                    return;
                }
            } else {
                self.clear_preview(cx);
            }
        }
        self.preview_key = Some((entry.path.clone(), revision, image_points));
        self.preview_generation += 1;
        let generation = self.preview_generation;
        self.preview_cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.preview_cancel = cancel.clone();
        crate::plugins::cancel_previews(surface, cx);
        let extensions = cx.global::<Ext>().0.clone();
        let Some(dir) = self.folder() else { return };
        let configured = cx.global::<Ui>().views.resolve(dir).preview_providers.0.clone();
        let candidates =
            crate::plugins::preview_candidates(cx, &configured, &entry.path, entry.is_dir, entry.is_symlink);
        let (gate, cloud) = match cx.try_global::<CloudStatusService>() {
            Some(service) => {
                let service = service.0.read(cx);
                (service.gate(), service.status(&entry.path))
            }
            None => (Arc::new(AllowAll) as Arc<dyn ReadGate>, None),
        };
        cx.spawn(async move |this, cx| {
            let path = entry.path.clone();
            let fetched = preview::load_with_plugins(
                cx,
                surface,
                candidates,
                entry,
                extensions,
                gate,
                cloud,
                None,
                generation,
                image_size,
                cancel.clone(),
            )
            .await;
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let Some(fetched) = fetched else { return };
            this.update(cx, |this, cx| {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                if this.preview_generation == generation && this.cursor_entry(cx).is_some_and(|e| e.path == path) {
                    let mut fetched = fetched;
                    if this.cloud_status(&path, cx).is_some_and(|status| fetched.update_cloud(status)) {
                        this.clear_preview(cx);
                        this.preview_key = None;
                        this.load_preview(cx);
                    } else {
                        this.clear_preview(cx);
                        this.preview = Some(fetched);
                    }
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn open_selected(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self.cursor_entry(cx) else { return };
        if entry.is_dir && !entry.is_package() {
            self.navigate(entry.path, cx);
        } else {
            self.open_file(entry.path, cx);
        }
    }

    /// Open `path` in the app that owns it, as a double-click on that row would.
    pub fn open_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        if let Err(e) = crate::effects::effects(cx).open_path(&path) {
            self.status = Some(format!("Can't open “{name}”: {e}").into());
            cx.notify();
        }
    }

    fn with_focused(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Folder, &mut Context<Folder>)) {
        self.status = None;
        self.focused_folder().clone().update(cx, f);
    }

    // ----- filter -----

    fn start_filter(&mut self, _: &StartFilter, window: &mut Window, cx: &mut Context<Self>) {
        // The filter field has its own query; a type-to-select run must not linger beside it.
        self.cancel_type_select(cx);
        if let Some(filtering) = &self.filtering {
            let focus = filtering.input.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            return;
        }
        crate::usage::record(&StartFilter, cx);
        crate::tips::key_used(TipId::Filter, cx);
        self.filter_announce = self.filter_count_text(cx).into();
        let folder = crate::style::display_name(self.focused_folder().read(cx).dir());
        let input = cx.new(|cx| TextInput::new("", format!("Filter {folder}"), cx).bare());
        let subscription = cx.subscribe_in(&input, window, |this, input, event, window, cx| match event {
            TextInputEvent::Changed => {
                let query = input.read(cx).text().to_owned();
                let folder = this.focused_folder().clone();
                folder.update(cx, |folder, cx| folder.set_filter(Some(query), cx));
                this.schedule_filter_announce(cx);
                cx.notify();
            }
            TextInputEvent::Confirmed => this.focus_matches(window, cx),
            TextInputEvent::Cancelled => this.close_filter(window, cx),
            TextInputEvent::CompletionRequested => {}
            TextInputEvent::BackspaceAtStart => {}
        });
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.filtering = Some(Filtering { input, _subscription: subscription });
        cx.notify();
    }

    /// Return in the filter field: move into the matches, on the selected one (the
    /// first, unless the arrows moved). The filter stays; Esc in the list clears it.
    /// With no matches the field keeps focus.
    fn focus_matches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let folder = self.focused_folder().clone();
        if folder.read(cx).row_count() == 0 {
            if let Some(input) = self.filtering.as_ref().map(|filtering| filtering.input.clone()) {
                crate::feedback::refuse(crate::feedback::RefuseTarget::Field(&input), window, cx);
            }
            return;
        }
        if self.cursor_entry(cx).is_none() {
            folder.update(cx, |folder, cx| folder.select_only(0, cx));
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// ↓ in the filter field: enter the matches on the current row, like the search
    /// field. With no matches nothing happens and the field keeps focus.
    fn filter_focus_results(&mut self, _: &FilterFocusResults, window: &mut Window, cx: &mut Context<Self>) {
        let folder = self.focused_folder().clone();
        if folder.read(cx).row_count() == 0 {
            return;
        }
        self.focus_matches(window, cx);
        if let Some(ix) = folder.read(cx).cursor_ix() {
            folder.read(cx).scroll().scroll_to_item(ix, ScrollStrategy::Top);
        }
    }

    /// ↑ on the first filtered row: back to the filter field. On a later row it
    /// moves up as usual. Vim's `k` stays `MoveUp`, as in search.
    fn filter_focus_field(&mut self, _: &FilterFocusField, window: &mut Window, cx: &mut Context<Self>) {
        if self.focused_folder().read(cx).cursor_ix() == Some(0) {
            if let Some(focus) = self.filter_focus(cx) {
                window.focus(&focus, cx);
            }
        } else {
            self.move_up(&MoveUp, window, cx);
        }
    }

    /// The query in the open filter field, if any (U5 and search escalation read it).
    pub fn filter_query(&self, cx: &gpui::App) -> Option<String> {
        self.filtering.as_ref().map(|filtering| filtering.input.read(cx).text().to_owned())
    }

    /// Close the filter without taking focus, returning its query. ⌘F uses this to hand
    /// the text to search; the search field gets focus instead.
    pub fn take_filter(&mut self, cx: &mut Context<Self>) -> Option<String> {
        let filtering = self.filtering.take()?;
        let query = filtering.input.read(cx).text().to_owned();
        for folder in &self.folders {
            folder.update(cx, |folder, cx| {
                folder.set_filter(None, cx);
                folder.set_tag_filter(Vec::new(), cx);
            });
        }
        self.focus_list_after_filter = false;
        cx.notify();
        Some(query)
    }

    fn clear_filter(&mut self, _: &ClearFilter, window: &mut Window, cx: &mut Context<Self>) {
        self.close_filter(window, cx);
    }

    /// Close the filter field and clear every folder's filter. Callers decide
    /// where focus goes next. Returns whether a filter was open.
    fn end_filter(&mut self, cx: &mut Context<Self>) -> bool {
        if self.filtering.take().is_none() {
            return false;
        }
        self.emit_filter_end(cx);
        self.filter_announce = SharedString::default();
        self.filter_announce_gen += 1;
        for folder in &self.folders {
            folder.update(cx, |folder, cx| {
                folder.set_filter(None, cx);
                folder.set_tag_filter(Vec::new(), cx);
            });
        }
        self.focus_list_after_filter = false;
        cx.notify();
        true
    }

    fn close_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.end_filter(cx) {
            window.focus(&self.focus, cx);
        }
    }

    /// Close the filter from a navigation path that has no window (the list
    /// gets its focus back after the next render).
    fn drop_filter(&mut self, cx: &mut Context<Self>) {
        if self.end_filter(cx) {
            self.focus_list_after_filter = true;
        }
    }

    /// ⌘⇧N/⌥⌘N with a filter open: the new item must be visible, so the filter
    /// ends. The rename that follows owns focus; there is no hand-off to the list.
    pub fn clear_filter_for_new_item(&mut self, cx: &mut Context<Self>) {
        self.end_filter(cx);
    }

    /// Tell the window how the filter session ended, before the folders' filters
    /// are cleared (the query and matches are gone after that).
    fn emit_filter_end(&self, cx: &mut Context<Self>) {
        let (empty, matches) = {
            let folder = self.focused_folder().read(cx);
            (folder.filter().is_none(), folder.row_count())
        };
        cx.emit(PaneEvent::FilterEnded { empty, matches });
    }

    // ----- actions -----

    fn move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        if self.mode == ViewMode::Icons {
            let target = self.icon_neighbor(IconMove::Up, cx);
            self.with_focused(cx, |f, cx| f.select_only(target, cx));
            return;
        }
        self.with_focused(cx, |f, cx| f.select_only(f.step(-1), cx));
    }

    fn move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        if self.mode == ViewMode::Icons {
            let target = self.icon_neighbor(IconMove::Down, cx);
            self.with_focused(cx, |f, cx| f.select_only(target, cx));
            return;
        }
        self.with_focused(cx, |f, cx| f.select_only(f.step(1), cx));
    }

    fn extend_up(&mut self, _: &ExtendUp, _: &mut Window, cx: &mut Context<Self>) {
        if self.mode == ViewMode::Icons {
            let target = self.icon_neighbor(IconMove::Up, cx);
            self.with_focused(cx, |f, cx| f.extend_to(target, cx));
            return;
        }
        self.with_focused(cx, |f, cx| f.extend_to(f.step(-1), cx));
    }

    fn extend_down(&mut self, _: &ExtendDown, _: &mut Window, cx: &mut Context<Self>) {
        if self.mode == ViewMode::Icons {
            let target = self.icon_neighbor(IconMove::Down, cx);
            self.with_focused(cx, |f, cx| f.extend_to(target, cx));
            return;
        }
        self.with_focused(cx, |f, cx| f.extend_to(f.step(1), cx));
    }

    /// The tile arrow keys land on: vertical moves keep the column, horizontal
    /// moves stop at the row's edge, and a short final row clamps.
    fn icon_neighbor(&self, movement: IconMove, cx: &gpui::App) -> usize {
        let folder = self.focused_folder().read(cx);
        let count = folder.row_count();
        let Some(from) = folder.cursor_ix() else {
            return if movement == IconMove::Up { count.saturating_sub(1) } else { 0 };
        };
        if count == 0 {
            return 0;
        }
        let grid = self.icon_grid(count, cx);
        let columns = grid.columns().max(1);
        let last = count - 1;
        let row = grid.row_of(from);
        match movement {
            IconMove::Up => {
                if row == 0 {
                    from
                } else {
                    from - columns
                }
            }
            IconMove::Down => {
                if row + 1 >= grid.rows() {
                    from
                } else {
                    (from + columns).min(last)
                }
            }
            IconMove::Left => from.saturating_sub(usize::from(from % columns != 0)),
            IconMove::Right => (from + 1).min(last).min(from + usize::from((from + 1) % columns != 0)),
        }
    }

    fn move_to_first(&mut self, _: &MoveToFirst, _: &mut Window, cx: &mut Context<Self>) {
        self.with_focused(cx, |f, cx| f.select_only(0, cx));
    }

    fn move_to_last(&mut self, _: &MoveToLast, _: &mut Window, cx: &mut Context<Self>) {
        self.with_focused(cx, |f, cx| f.select_only(f.row_count().saturating_sub(1), cx));
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.with_focused(cx, |f, cx| f.select_all(cx));
    }

    fn gallery_row_at(&self, cx: &gpui::App, pointer: Point<Pixels>) -> Option<usize> {
        let bounds = self.gallery.bounds.get()?;
        if pointer.x < bounds.left()
            || pointer.x >= bounds.right()
            || pointer.y < bounds.top()
            || pointer.y >= bounds.bottom()
        {
            return None;
        }
        let folder = self.folders[0].read(cx);
        let y = pointer.y - bounds.top() - px(shown_offset(&self.gallery.scroll));
        let row = (f32::from(y) / gallery::ROW_HEIGHT).floor();
        if row < 0.0 {
            return None;
        }
        let row = row as usize;
        (row < folder.row_count()).then_some(row)
    }

    /// Column view: move into the column showing the selected folder's contents.
    fn move_right(&mut self, _: &MoveRight, window: &mut Window, cx: &mut Context<Self>) {
        match self.mode {
            ViewMode::List => {
                self.list_right(cx);
                return;
            }
            ViewMode::Icons => {
                let target = self.icon_neighbor(IconMove::Right, cx);
                self.with_focused(cx, |f, cx| f.select_only(target, cx));
                return;
            }
            ViewMode::Gallery => {
                self.move_down(&MoveDown, window, cx);
                return;
            }
            ViewMode::Columns => {}
        }
        if self.focused + 1 >= self.folders.len() {
            return;
        }
        self.drop_filter(cx);
        self.focused += 1;
        self.with_focused(cx, |f, cx| f.select_first(cx));
        self.strip_scroll.scroll_to_item(self.focused);
        self.load_preview(cx);
        cx.notify();
    }

    /// List view: open the selected folder inline, or go into it when it is already open.
    fn list_right(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self.cursor_entry(cx) else { return };
        if !entry.is_dir || entry.is_package() {
            return;
        }
        if self.filtering.is_some() {
            self.navigate(entry.path, cx);
            return;
        }
        let folder = self.focused_folder().clone();
        let expanded = folder.read(cx).is_expanded(&entry.path, cx);
        if expanded {
            self.navigate(entry.path, cx);
        } else {
            folder.update(cx, |folder, cx| folder.expand(entry.path, cx));
        }
    }

    /// List view: close an open folder, step to the row that contains a nested one, or
    /// leave the folder from the top level, as Finder does.
    fn list_left(&mut self, cx: &mut Context<Self>) {
        let folder = self.focused_folder().clone();
        let read = folder.read(cx);
        let Some(ix) = read.cursor_ix() else { return };
        let Some((entry, _)) = read.row(ix) else { return };
        let (path, expanded, parent) = (entry.path.clone(), read.is_expanded(&entry.path, cx), read.parent_row(ix));
        if expanded {
            folder.update(cx, |folder, cx| folder.collapse(&path, cx));
        } else if let Some(parent) = parent {
            folder.update(cx, |folder, cx| folder.select_only(parent, cx));
        } else {
            self.open_parent(cx);
        }
    }

    /// Column view: back to the parent column. The column you leave stays, unselected.
    /// On the leftmost column there is no column to go back to, so the parent folder
    /// is pushed in from the left, with the folder we came from selected in it.
    fn move_left(&mut self, _: &MoveLeft, window: &mut Window, cx: &mut Context<Self>) {
        match self.mode {
            ViewMode::List => {
                self.list_left(cx);
                return;
            }
            ViewMode::Icons => {
                let target = self.icon_neighbor(IconMove::Left, cx);
                self.with_focused(cx, |f, cx| f.select_only(target, cx));
                return;
            }
            ViewMode::Gallery => {
                self.move_up(&MoveUp, window, cx);
                return;
            }
            ViewMode::Columns => {}
        }
        if self.focused > 0 {
            self.drop_filter(cx);
            let leaving = self.focused_folder().clone();
            self.focused -= 1;
            leaving.update(cx, |f, cx| f.clear_selection(cx));
        } else {
            let child = self.folders[0].read(cx).dir().to_path_buf();
            let Some(parent) = child.parent().map(Path::to_path_buf) else { return };
            self.drop_filter(cx);
            self.history.visit(Location::Folder(parent.clone()));
            self.push_folder(parent.clone(), false, cx);
            let folder = self.folders.pop().expect("pushed above");
            let subscription = self.subscriptions.pop().expect("pushed above");
            self.folders.insert(0, folder.clone());
            self.subscriptions.insert(0, subscription);
            folder.update(cx, |f, _| f.select_after_load(child));
            self.focused = 0;
            self.plugin_scope_changed(cx);
            cx.emit(PaneEvent::Entered(parent));
        }
        self.strip_scroll.scroll_to_item(self.focused);
        self.load_preview(cx);
        cx.notify();
    }

    fn open(&mut self, _: &OpenSelection, _: &mut Window, cx: &mut Context<Self>) {
        self.open_selected(cx);
    }

    /// Return in the list: the setting decides whether it renames or opens.
    fn enter_row(&mut self, _: &EnterRow, window: &mut Window, cx: &mut Context<Self>) {
        match cx.global::<Ui>().settings.enter {
            EnterAction::Rename => self.rename(&Rename, window, cx),
            EnterAction::Open => self.open_selected(cx),
        }
    }

    fn rename(&mut self, _: &Rename, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.focused_folder().read(cx).selected_count();
        if selected > 1 {
            let inputs = self.batch_rename_inputs(cx);
            if inputs.len() < 2 {
                crate::feedback::refuse(crate::feedback::RefuseTarget::None, window, cx);
                crate::operations::message("Select two or more items to rename", false, cx);
                cx.notify();
                return;
            }
            cx.emit(PaneEvent::BatchRename(inputs.into()));
            return;
        }
        if selected == 0 {
            return;
        }
        if let Some(entry) = self.cursor_entry(cx) {
            self.begin_rename(entry.path, entry.is_dir, window, cx);
        }
    }

    /// The selected rows frozen in display order for the batch sheet.
    fn batch_rename_inputs(&self, cx: &App) -> Vec<crow_core::batch_rename::RenameInput> {
        let mut seen = HashSet::new();
        self.selected_entries(cx)
            .iter()
            .filter(|(path, _)| seen.insert(path.clone()))
            .map(|(path, is_dir)| crow_core::batch_rename::RenameInput {
                source: path.clone(),
                name: path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(),
                is_dir: *is_dir,
            })
            .collect()
    }

    /// Replace the item's name with a text field; the name without its extension is selected.
    /// `is_dir` comes from the listing, so the main thread never stats the path.
    fn begin_rename(&mut self, path: PathBuf, is_dir: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_type_select(cx);
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let stem_end = match name.rfind('.') {
            Some(i) if i > 0 && !is_dir => i,
            _ => name.len(),
        };
        let input = cx.new(|cx| TextInput::new(name.clone(), "Name", cx));
        input.update(cx, |input, cx| input.select_range(0..stem_end, window, cx));
        let subscription =
            cx.subscribe_in(&input, window, move |this, _input, event: &TextInputEvent, window, cx| match event {
                TextInputEvent::Changed => {}
                TextInputEvent::Confirmed => this.commit_rename(window, cx),
                TextInputEvent::Cancelled => {
                    this.renaming = None;
                    window.focus(&this.focus, cx);
                    cx.notify();
                }
                TextInputEvent::CompletionRequested => {}
                TextInputEvent::BackspaceAtStart => {}
            });
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.renaming = Some(Renaming { path, input, _subscription: subscription });
        cx.notify();
    }

    /// Apply the open rename and close the field; a no-op when nothing is being renamed.
    fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(renaming) = self.renaming.take() else { return };
        let new_name = renaming.input.read(cx).text().trim().to_owned();
        let old = renaming.path.file_name().map(|n| n.to_string_lossy().into_owned());
        if old.as_deref() != Some(new_name.as_str()) {
            cx.emit(PaneEvent::Run(Request::Rename { path: renaming.path, new_name }));
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn new_folder(&mut self, _: &NewFolder, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PaneEvent::Run(Request::NewFolder { parent: self.current_dir(cx) }));
    }

    fn new_file(&mut self, _: &NewFile, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PaneEvent::Run(Request::NewFile { parent: self.current_dir(cx) }));
    }

    fn duplicate(&mut self, _: &Duplicate, _: &mut Window, cx: &mut Context<Self>) {
        let paths = self.selected_paths(cx);
        if !paths.is_empty() {
            cx.emit(PaneEvent::Run(Request::Duplicate { paths: paths.into() }));
        }
    }

    fn move_to_trash(&mut self, _: &MoveToTrash, _: &mut Window, cx: &mut Context<Self>) {
        let paths = self.selected_paths(cx);
        if !paths.is_empty() {
            cx.emit(PaneEvent::Run(Request::Trash { paths: paths.into() }));
        }
    }

    fn delete_permanently(&mut self, _: &DeletePermanently, _: &mut Window, cx: &mut Context<Self>) {
        let paths = self.selected_paths(cx);
        if !paths.is_empty() {
            cx.emit(PaneEvent::Run(Request::DeletePermanently { paths: paths.into() }));
        }
    }

    fn copy_files(&mut self, _: &CopyFiles, _: &mut Window, cx: &mut Context<Self>) {
        let paths: Arc<[PathBuf]> = self.selected_paths(cx).into();
        if !paths.is_empty() {
            cx.emit(PaneEvent::Copy { paths, cut: false });
        }
    }

    fn cut_files(&mut self, _: &CutFiles, _: &mut Window, cx: &mut Context<Self>) {
        let paths: Arc<[PathBuf]> = self.selected_paths(cx).into();
        if !paths.is_empty() {
            cx.emit(PaneEvent::Copy { paths, cut: true });
        }
    }

    fn paste_files(&mut self, _: &PasteFiles, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PaneEvent::Paste { into: self.current_dir(cx), move_items: false });
    }

    fn paste_move_files(&mut self, _: &PasteMoveFiles, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PaneEvent::Paste { into: self.current_dir(cx), move_items: true });
    }

    fn quick_look(&mut self, _: &QuickLook, window: &mut Window, cx: &mut Context<Self>) {
        // Mid-word, Space belongs to type-to-select ("my file"), not Quick Look.
        if self.focused_folder().read(cx).type_select_active() {
            self.with_focused(cx, |f, cx| f.type_char(' ', cx));
            return;
        }
        let Some(entry) = self.cursor_entry(cx) else { return };
        // The panel is key while it is open, so the arrows come back through its
        // delegate instead of the window. Run them as the arrows those keys are bound to.
        let effects = crate::effects::effects(cx);
        let handle = window.window_handle();
        let async_cx = cx.to_async();
        effects.set_quick_look_key_handler(Box::new(move |key| {
            async_cx.update(|cx| {
                let action: Box<dyn Action> = match key {
                    crow_macos::QuickLookKey::Up => Box::new(MoveUp),
                    crow_macos::QuickLookKey::Down => Box::new(MoveDown),
                    crow_macos::QuickLookKey::Left => Box::new(MoveLeft),
                    crow_macos::QuickLookKey::Right => Box::new(MoveRight),
                };
                let _ = handle.update(cx, |_, window, cx| window.dispatch_action(action, cx));
            });
        }));
        effects.toggle_quick_look(&entry.path);
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // While a name is being edited, letters belong to the text field, not type-to-select.
        if self.renaming.is_some() {
            return;
        }
        // The filter field is inside this list; its unhandled keys must not type-to-select.
        if let Some(filtering) = &self.filtering
            && filtering.input.read(cx).focus_handle(cx).is_focused(window)
        {
            return;
        }
        let m = &event.keystroke.modifiers;
        if m.platform || m.control || m.alt {
            return;
        }
        let c = event.keystroke.key_char.as_deref().and_then(|s| s.chars().next());
        if let Some(pending) = self.mark_pending.take() {
            if let Some(key) = c.filter(char::is_ascii_alphanumeric) {
                cx.emit(PaneEvent::Mark { key, set: pending == MarkPending::Set });
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        // In the Vim layer letters are commands, never type-to-select.
        if cx.global::<Ui>().settings.keymap == KeymapLayer::Vim {
            return;
        }
        let Some(c) = c else { return };
        if c.is_control() || c == ' ' {
            return;
        }
        self.with_focused(cx, |f, cx| f.type_char(c, cx));
        cx.stop_propagation();
    }

    fn begin_mark(&mut self, _: &BeginMark, _: &mut Window, cx: &mut Context<Self>) {
        self.mark_pending = Some(MarkPending::Set);
        cx.notify();
    }

    fn begin_jump(&mut self, _: &BeginJump, _: &mut Window, cx: &mut Context<Self>) {
        self.mark_pending = Some(MarkPending::Jump);
        cx.notify();
    }

    /// What the status line should say while a Vim mark waits for its letter.
    pub fn pending_hint(&self) -> Option<&'static str> {
        self.mark_pending.map(|p| match p {
            MarkPending::Set => "Mark this folder: type a letter",
            MarkPending::Jump => "Go to mark: type a letter",
        })
    }

    /// A press on row `ix` of folder `k`: Finder's click, Cmd-click and Shift-click.
    fn press(&mut self, k: usize, ix: usize, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // A click in the open rename field places the caret; it must not commit the rename
        // or move focus. The row still took the press, so the list must not see empty space.
        if std::mem::take(&mut self.rename_pressed) {
            self.row_pressed = true;
            return;
        }
        if self.renaming.is_some() {
            self.commit_rename(window, cx);
        }
        if event.modifiers.control {
            self.context_press(k, Some(ix), window, cx);
            return;
        }
        self.row_pressed = true;
        // AppKit wants the mouse-down event when a GPUI drag later leaves the window.
        crow_macos::remember_mouse_down();
        window.focus(&self.focus, cx);
        if k != self.focused {
            self.drop_filter(cx);
            self.cancel_type_select(cx);
            self.focused = k;
            self.plugin_scope_changed(cx);
            self.strip_scroll.scroll_to_item(k);
        }
        let (cmd, shift) = (event.modifiers.platform, event.modifiers.shift);
        self.with_focused(cx, |f, cx| {
            if cmd {
                f.toggle(ix, cx);
            } else if shift {
                f.extend_to(ix, cx);
            } else {
                f.click(ix, cx);
            }
        });
        self.load_preview(cx);
        cx.notify();
    }

    /// A press in folder `k`'s list that no row took: Finder's click on empty space.
    /// A plain click clears the selection and keeps focus on the list; in column view
    /// the column becomes the focused one and the columns to its right close.
    /// Cmd- and Shift-clicks keep the selection.
    fn press_empty(&mut self, k: usize, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.empty_space_pressed(k, event) {
            return;
        }
        if event.modifiers.control {
            self.context_press(k, None, window, cx);
            return;
        }
        if self.mode == ViewMode::Icons {
            self.begin_marquee(event, window, cx);
            return;
        }
        if self.renaming.is_some() {
            self.commit_rename(window, cx);
        }
        window.focus(&self.focus, cx);
        if event.modifiers.platform || event.modifiers.shift {
            return;
        }
        if k != self.focused {
            self.drop_filter(cx);
            self.cancel_type_select(cx);
            self.focused = k;
            self.plugin_scope_changed(cx);
        }
        // Clearing emits CursorMoved, which closes the columns right of `k`.
        self.with_focused(cx, |f, cx| f.clear_selection(cx));
        self.load_preview(cx);
        cx.notify();
    }

    /// Whether this press in folder `k` landed on its list but on no row.
    fn empty_space_pressed(&mut self, k: usize, event: &MouseDownEvent) -> bool {
        if std::mem::take(&mut self.row_pressed) {
            return false;
        }
        // A column also holds the filter field and its footer: only its list counts.
        self.mode != ViewMode::Columns
            || self.column_bounds.borrow().get(&k).is_some_and(|bounds| bounds.contains(&event.position))
    }

    fn context_press_empty(&mut self, k: usize, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.empty_space_pressed(k, event) {
            self.context_press(k, None, window, cx);
        }
    }

    // ----- context menu -----

    /// A right-click or Ctrl-click in folder `k`: on row `row`, or on empty space. Like
    /// Finder, an unselected row is selected first and a row in the selection keeps it;
    /// empty space deselects and the menu is about the folder shown.
    fn context_press(&mut self, k: usize, row: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        if row.is_some() {
            self.row_pressed = true;
        }
        if self.renaming.is_some() {
            self.commit_rename(window, cx);
        }
        window.focus(&self.focus, cx);
        if k != self.focused {
            self.drop_filter(cx);
            self.cancel_type_select(cx);
            self.focused = k;
            self.plugin_scope_changed(cx);
        }
        let kind = match row {
            Some(ix) => {
                self.with_focused(cx, |f, cx| {
                    let selected = f.row(ix).is_some_and(|(entry, _)| f.is_selected(&entry.path));
                    if !selected {
                        f.select_only(ix, cx);
                    }
                });
                MenuKind::Items
            }
            None => {
                self.with_focused(cx, |f, cx| f.clear_selection(cx));
                MenuKind::Folder
            }
        };
        self.load_preview(cx);
        cx.notify();
        self.open_context_menu(kind, window, cx);
    }

    fn open_context_menu(&mut self, kind: MenuKind, window: &mut Window, cx: &mut Context<Self>) {
        let state = {
            let folder = self.focused_folder().read(cx);
            let (_, selected_folders) = folder.selected_bytes();
            MenuState {
                selected: folder.selected_count(),
                selected_folders,
                can_paste: !crate::operations::pasteboard_paths(cx).0.is_empty(),
            }
        };
        let scope = match (self.mode, self.filtering.is_some()) {
            (ViewMode::List | ViewMode::Icons | ViewMode::Gallery, false) => KeyScope::List,
            (ViewMode::List | ViewMode::Icons | ViewMode::Gallery, true) => KeyScope::ListFiltered,
            (ViewMode::Columns, false) => KeyScope::Columns,
            (ViewMode::Columns, true) => KeyScope::ColumnsFiltered,
        };
        let mut entries = crate::context_menu::entries(kind, state, scope, cx);
        let selected = self.focused_folder().read(cx).selected_entries();
        let plugins = crate::plugins::context_menu_commands(cx, selected.as_slice());
        self.context_menu_plugins = plugins.iter().map(|(_, action)| action.clone()).collect();
        if !plugins.is_empty() {
            entries.push(crow_macos::ContextMenuEntry::Separator);
            entries.extend(plugins.into_iter().map(|(title, _)| crow_macos::ContextMenuEntry::Item {
                title,
                shortcut: None,
                enabled: true,
            }));
        }
        self.context_menu = Some(kind);
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
            this.update_in(cx, |this, window, cx| this.context_menu_closed(chosen, window, cx)).ok();
        })
        .detach();
    }

    // ----- icon marquee -----

    fn begin_marquee(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_marquee(cx);
        if self.renaming.is_some() {
            self.commit_rename(window, cx);
        }
        window.focus(&self.focus, cx);
        self.cancel_type_select(cx);
        let Some(content) = self.icon_content_point(event.position) else { return };
        let folder = self.focused_folder().clone();
        let (base_selection, base_cursor, layout_generation) = {
            let folder = folder.read(cx);
            (folder.selected_paths(), folder.selection_cursor().map(Path::to_path_buf), folder.layout_generation())
        };
        let additive = event.modifiers.platform || event.modifiers.shift;
        let base = Arc::new(if additive { base_selection.iter().cloned().collect() } else { HashSet::new() });
        if !additive {
            folder.update(cx, |folder, cx| folder.clear_selection(cx));
            self.load_preview(cx);
        }
        self.marquee = Some(Marquee {
            anchor: content,
            pointer: event.position,
            base_selection,
            base_cursor,
            base,
            additive,
            started: false,
            layout_generation,
            hits: HashMap::new(),
        });
        cx.notify();
    }

    /// The pointer in the grid's content coordinates (x from the content's left,
    /// y from the first row, both before the scroll offset is applied).
    fn icon_content_point(&self, pointer: Point<Pixels>) -> Option<Point<Pixels>> {
        let bounds = self.icon_bounds.get()?;
        let offset = self.icon_scroll.0.borrow().base_handle.offset().y;
        Some(point(pointer.x - bounds.left(), pointer.y - bounds.top() - offset))
    }

    fn update_marquee(&mut self, pointer: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(marquee) = &self.marquee else { return };
        let was_started = marquee.started;
        let previous_pointer = marquee.pointer;
        let anchor = marquee.anchor;
        let additive = marquee.additive;
        let base = marquee.base.clone();
        let old: HashMap<PathBuf, (u64, bool)> = marquee.hits.clone();
        let layout_generation = marquee.layout_generation;
        let base_cursor = marquee.base_cursor.clone();
        let base_first = marquee.base_selection.first().cloned();
        if !was_started {
            let dx = f32::from(pointer.x - previous_pointer.x);
            let dy = f32::from(pointer.y - previous_pointer.y);
            if dx * dx + dy * dy < 9.0 {
                return;
            }
        }
        let Some(current) = self.icon_content_point(pointer) else { return };
        let folder_entity = self.focused_folder().clone();
        if folder_entity.read(cx).layout_generation() != layout_generation {
            self.cancel_marquee(cx);
            return;
        }
        let (count, rect) = {
            let folder = folder_entity.read(cx);
            (
                folder.row_count(),
                GridRect {
                    x: f32::from(anchor.x.min(current.x)),
                    y: f32::from(anchor.y.min(current.y)),
                    width: f32::from((anchor.x - current.x).abs()).max(1.0),
                    height: f32::from((anchor.y - current.y).abs()).max(1.0),
                },
            )
        };
        let grid = self.icon_grid(count, cx);
        let mut ordered = Vec::new();
        let mut hits: HashMap<PathBuf, (u64, bool)> = HashMap::new();
        {
            let folder = folder_entity.read(cx);
            grid.tiles_intersecting(&rect, |ix, _| {
                if let Some((entry, _)) = folder.row(ix) {
                    hits.insert(entry.path.clone(), (entry.size, entry.is_dir));
                    ordered.push(entry.path.clone());
                }
            });
        }
        let mut added = Vec::new();
        let mut removed = Vec::new();
        for (path, entry) in &hits {
            if !old.contains_key(path) && !base.contains(path) {
                added.push((path.clone(), entry.0, entry.1));
            }
        }
        for (path, entry) in &old {
            if !hits.contains_key(path) && !(additive && base.contains(path)) {
                removed.push((path.clone(), entry.0, entry.1));
            }
        }
        let preferred = base_cursor
            .clone()
            .filter(|path| (additive && base.contains(path)) || hits.contains_key(path))
            .or_else(|| ordered.first().cloned())
            .or_else(|| additive.then_some(base_cursor).flatten())
            .or_else(|| additive.then_some(base_first).flatten());
        folder_entity.update(cx, |folder, cx| folder.apply_marquee_delta(&added, &removed, preferred, cx));
        if let Some(marquee) = &mut self.marquee {
            marquee.started = true;
            marquee.pointer = pointer;
            marquee.hits = hits;
        }
        self.schedule_marquee_scroll(pointer, cx);
        cx.notify();
    }

    /// Edge autoscroll as one-shot timers: a stationary pointer at an edge steps
    /// one grid row and schedules the next step; leaving the edge stops it. An
    /// endless timer would keep the test executor from ever parking.
    fn schedule_marquee_scroll(&mut self, pointer: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(bounds) = self.icon_bounds.get() else { return };
        let edge = px(24.);
        if pointer.y >= bounds.top() + edge && pointer.y <= bounds.bottom() - edge {
            return;
        }
        let Some(marquee) = &self.marquee else { return };
        if !marquee.started {
            return;
        }
        self.marquee_scroll_gen += 1;
        let generation = self.marquee_scroll_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(40)).await;
            this.update(cx, |this, cx| {
                if this.marquee_scroll_gen != generation {
                    return;
                }
                let Some(marquee) = &this.marquee else { return };
                if marquee.pointer != pointer {
                    return;
                }
                let Some(bounds) = this.icon_bounds.get() else { return };
                let direction = if pointer.y < bounds.top() + edge {
                    -1.0
                } else if pointer.y > bounds.bottom() - edge {
                    1.0
                } else {
                    return;
                };
                let (layout_changed, count) = {
                    let folder = this.focused_folder().read(cx);
                    (folder.layout_generation() != marquee.layout_generation, folder.row_count())
                };
                if layout_changed {
                    this.cancel_marquee(cx);
                    return;
                }
                let grid = this.icon_grid(count, cx);
                {
                    let scroll = &this.icon_scroll.0.borrow().base_handle;
                    let mut offset = scroll.offset();
                    let top = -f32::from(offset.y);
                    let max = (grid.content_height() - grid.viewport_height()).max(0.0);
                    let next = (top + direction * grid.row_pitch()).clamp(0.0, max);
                    if next == top {
                        return;
                    }
                    offset.y = px(-next);
                    scroll.set_offset(offset);
                }
                this.update_marquee(pointer, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Cancel the drag and restore the surviving starting selection and cursor.
    pub(crate) fn cancel_marquee(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(marquee) = self.marquee.take() else { return false };
        self.marquee_scroll_gen += 1;
        let folder = self.focused_folder().clone();
        let mut survivors = Vec::new();
        let mut totals = (0u64, 0usize);
        {
            let read = folder.read(cx);
            for path in &marquee.base_selection {
                let Some(ix) = read.row_of_path(path) else { continue };
                let Some((entry, _)) = read.row(ix) else { continue };
                survivors.push(path.clone());
                if entry.is_dir {
                    totals.1 += 1;
                } else {
                    totals.0 += entry.size;
                }
            }
        }
        let cursor = marquee.base_cursor.filter(|path| survivors.contains(path)).or_else(|| survivors.first().cloned());
        folder.update(cx, |folder, cx| folder.set_selection_paths(survivors, cursor, totals, cx));
        self.load_preview(cx);
        cx.notify();
        true
    }

    /// Release after a drag: the preview selection is already the committed one.
    fn commit_marquee(&mut self, cx: &mut Context<Self>) {
        if self.marquee.take().is_some() {
            self.marquee_scroll_gen += 1;
            cx.notify();
        }
    }

    fn marquee_rect(&self) -> Option<Bounds<Pixels>> {
        let marquee = self.marquee.as_ref().filter(|marquee| marquee.started)?;
        let bounds = self.icon_bounds.get()?;
        let offset = self.icon_scroll.0.borrow().base_handle.offset().y;
        let current = point(marquee.pointer.x - bounds.left(), marquee.pointer.y - bounds.top());
        let start = point(marquee.anchor.x, marquee.anchor.y + offset);
        let left = start.x.min(current.x).max(px(0.));
        let top = start.y.min(current.y).max(px(0.));
        if left >= bounds.size.width || top >= bounds.size.height {
            return None;
        }
        let width = (start.x - current.x).abs().max(px(1.)).min(bounds.size.width - left);
        let height = (start.y - current.y).abs().max(px(1.)).min(bounds.size.height - top);
        Some(Bounds::new(point(left, top), size(width, height)))
    }

    fn marquee_overlay(&self, theme: &Theme) -> Option<Div> {
        let rect = self.marquee_rect()?;
        Some(
            div()
                .absolute()
                .debug_selector(|| "marquee".into())
                .left(rect.left())
                .top(rect.top())
                .w(rect.size.width)
                .h(rect.size.height)
                .bg(color_alpha(theme.role("accent"), 0x24))
                .border_1()
                .border_color(color_alpha(theme.role("accent"), 0xcc)),
        )
    }

    /// The menu closed, with the item at `chosen` picked (an index into its slots), or none.
    fn context_menu_closed(&mut self, chosen: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(kind) = self.context_menu.take() else { return };
        let builtins = crate::context_menu::slots(kind).len();
        let plugin = chosen
            .and_then(|ix| ix.checked_sub(builtins + 1))
            .and_then(|ix| self.context_menu_plugins.get(ix))
            .cloned();
        self.context_menu_plugins.clear();
        window.focus(&self.focus, cx);
        if let Some(action) = plugin {
            window.dispatch_action(Box::new(action), cx);
            return;
        }
        let Some(id) = chosen.and_then(|ix| crate::context_menu::slots(kind).get(ix).copied().flatten()) else {
            return;
        };
        window.dispatch_action(crate::context_menu::command(id).action.boxed_clone(), cx);
    }

    #[cfg(test)]
    pub fn context_menu(&self) -> Option<MenuKind> {
        self.context_menu
    }

    #[cfg(test)]
    pub fn choose_context_item(&mut self, chosen: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.context_menu_closed(chosen, window, cx);
    }

    fn copy_path(&mut self, _: &CopyPath, _: &mut Window, cx: &mut Context<Self>) {
        let mut paths = self.selected_paths(cx);
        if paths.is_empty() {
            paths.push(self.current_dir(cx));
        }
        let text = paths.iter().map(|path| path.to_string_lossy()).collect::<Vec<_>>().join("\n");
        crate::effects::write_text(text, cx);
        let what = if paths.len() == 1 { "path".to_owned() } else { format!("{} paths", paths.len()) };
        crate::operations::message(format!("Copied {what}"), false, cx);
        cx.notify();
    }

    fn show_in_finder(&mut self, _: &ShowInFinder, _: &mut Window, cx: &mut Context<Self>) {
        let paths = self.selected_paths(cx);
        let effects = crate::effects::effects(cx);
        if paths.is_empty() {
            effects.open_folder_in_finder(&self.current_dir(cx));
        } else {
            effects.reveal_in_finder(&paths);
        }
    }

    /// Open a terminal at the selected folder, a selected file's parent, or the
    /// shown folder. A multi-selection has no single "here": the shown folder.
    fn open_terminal(&mut self, _: &OpenTerminal, _: &mut Window, cx: &mut Context<Self>) {
        let folder = if self.selected_count(cx) > 1 {
            self.current_dir(cx)
        } else {
            match self.cursor_entry(cx) {
                Some(entry) if entry.is_dir => entry.path,
                Some(entry) => entry.path.parent().map_or_else(|| self.current_dir(cx), Path::to_path_buf),
                None => self.current_dir(cx),
            }
        };
        let app = cx.global::<Ui>().settings.terminal.trim().to_owned();
        let app = if app.is_empty() { "Terminal".to_owned() } else { app };
        let effects = crate::effects::effects(cx);
        // `open -a` waits for Launch Services, so it runs off the main thread.
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { effects.open_terminal(&app, &folder) }).await;
            if let Err(error) = result {
                this.update(cx, |_, cx| {
                    crate::operations::message(error, true, cx);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    // ----- scroll -----

    /// A pointer or touch scroll in folder `k`'s list: suggest `/` once a long
    /// folder is scrolled well past its first screen.
    fn on_list_scroll(&mut self, k: usize, cx: &mut Context<Self>) {
        if self.view_motion.take().is_some() {
            cx.notify();
        }
        if self.filtering.is_some() || !crate::tips::wants(TipId::Filter, cx) {
            return;
        }
        let Some(folder) = self.folders.get(k) else { return };
        let folder = folder.read(cx);
        if folder.entries().len() < TIP_SCROLL_ENTRIES {
            return;
        }
        if first_visible_row(folder.scroll()) >= TIP_SCROLL_ROWS {
            cx.emit(PaneEvent::LongFolderScrolled);
        }
    }

    // ----- benchmark -----

    /// The list row under `pointer`: rows are uniform, and the scroll offset is known.
    fn drag_row_at(&self, cx: &gpui::App, pointer: Point<Pixels>) -> Option<usize> {
        let bounds = self.list_bounds.get()?;
        if pointer.x < bounds.left()
            || pointer.x >= bounds.right()
            || pointer.y < bounds.top()
            || pointer.y >= bounds.bottom()
        {
            return None;
        }
        let folder = self.folders[0].read(cx);
        let row_height = px(ROW_HEIGHT);
        // The offset is negative once scrolled down: subtracting it moves into the content.
        let y = pointer.y - bounds.top() - px(LIST_TOP_INSET) - folder.scroll().0.borrow().base_handle.offset().y;
        let row = (f32::from(y) / f32::from(row_height)).floor();
        if row < 0.0 {
            return None;
        }
        let row = row as usize;
        (row < folder.row_count()).then_some(row)
    }

    /// The column row under `pointer`, if any: each column has its own list and scroll.
    fn column_row_at(&self, cx: &gpui::App, pointer: Point<Pixels>) -> Option<(usize, usize)> {
        let (k, bounds) = self
            .column_bounds
            .borrow()
            .iter()
            .find(|(_, bounds)| bounds.contains(&pointer))
            .map(|(k, bounds)| (*k, *bounds))?;
        let folder = self.folders.get(k)?.read(cx);
        let row_height = px(ROW_HEIGHT);
        let y = pointer.y - bounds.top() - folder.scroll().0.borrow().base_handle.offset().y;
        let row = (f32::from(y) / f32::from(row_height)).floor();
        if row < 0.0 {
            return None;
        }
        let row = row as usize;
        (row < folder.row_count()).then_some((k, row))
    }

    pub fn update_drag_target(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.update_drag_row(position, cx);
    }

    /// The typed drop destination under `pointer`, with the layout generation a
    /// spring timer must capture.
    pub(crate) fn drop_hit_for<'a>(
        &self,
        pointer: Point<Pixels>,
        dragged: impl Iterator<Item = &'a Path>,
        cx: &gpui::App,
    ) -> Option<crate::drag_drop::DropHit> {
        if self.renaming.is_some() || self.folder().is_none() {
            return None;
        }
        let target = self.drop_target_at(pointer, dragged, cx)?;
        let generation = self.drop_generation(target.column, cx);
        let spring = target.folder != self.current_dir(cx);
        Some(crate::drag_drop::DropHit::folder(target.folder, generation, spring))
    }

    /// The layout generation for one column: the folder identity and path as well
    /// as its row generation, so a spring into a different folder always counts as
    /// fresh geometry even when both listings have the same row count.
    fn drop_generation(&self, column: usize, cx: &gpui::App) -> u64 {
        use std::hash::{Hash, Hasher};
        let Some(folder) = self.folders.get(column).map(|folder| folder.read(cx)) else { return 0 };
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        folder.dir().hash(&mut hasher);
        folder.layout_generation().hash(&mut hasher);
        (self.mode as u64).hash(&mut hasher);
        hasher.finish()
    }

    /// Where a drop at `pointer` lands, if dropping the dragged items there would act:
    /// a folder row (with its expanded subtree), the expanded folder that lists a nested
    /// row, or else the folder the list or column shows.
    fn drop_target_at<'a>(
        &self,
        pointer: Point<Pixels>,
        dragged: impl Iterator<Item = &'a Path>,
        cx: &gpui::App,
    ) -> Option<DropTarget> {
        let target = match self.mode {
            ViewMode::List => {
                let bounds = self.list_bounds.get()?;
                if !bounds.contains(&pointer) {
                    return None;
                }
                let folder = self.folders.first()?.read(cx);
                match self.drag_row_at(cx, pointer).and_then(|ix| folder.drop_header(ix)) {
                    Some(header) => DropTarget {
                        folder: folder.row(header)?.0.path.clone(),
                        column: 0,
                        rows: Some(header..folder.subtree_end(header)),
                    },
                    None => DropTarget { folder: folder.dir().to_path_buf(), column: 0, rows: None },
                }
            }
            ViewMode::Icons => {
                let bounds = self.icon_bounds.get()?;
                if !bounds.contains(&pointer) {
                    return None;
                }
                let offset = self.icon_scroll.0.borrow().base_handle.offset().y;
                let grid = self.grid?;
                let index = grid.tile_at(
                    f32::from(pointer.x - bounds.left()),
                    f32::from(pointer.y - bounds.top()) - f32::from(offset),
                );
                let folder = self.folders.first()?.read(cx);
                match index.and_then(|ix| folder.drop_header(ix)) {
                    Some(header) => DropTarget {
                        folder: folder.row(header)?.0.path.clone(),
                        column: 0,
                        rows: Some(header..folder.subtree_end(header)),
                    },
                    None => DropTarget { folder: folder.dir().to_path_buf(), column: 0, rows: None },
                }
            }
            ViewMode::Gallery => {
                if !self.gallery.bounds.get().is_some_and(|bounds| bounds.contains(&pointer))
                    && !self.gallery.preview_bounds.get().is_some_and(|bounds| bounds.contains(&pointer))
                {
                    return None;
                }
                let folder = self.folders.first()?.read(cx);
                match self.gallery_row_at(cx, pointer).and_then(|ix| folder.drop_header(ix)) {
                    Some(header) => DropTarget {
                        folder: folder.row(header)?.0.path.clone(),
                        column: 0,
                        rows: Some(header..header + 1),
                    },
                    None => DropTarget { folder: folder.dir().to_path_buf(), column: 0, rows: None },
                }
            }
            ViewMode::Columns => {
                let column = self
                    .column_bounds
                    .borrow()
                    .iter()
                    .find(|(_, bounds)| bounds.contains(&pointer))
                    .map(|(k, _)| *k)?;
                let folder = self.folders.get(column)?.read(cx);
                let row = self
                    .column_row_at(cx, pointer)
                    .map(|(_, ix)| ix)
                    .filter(|ix| folder.row(*ix).is_some_and(|(entry, _)| entry.is_dir && !entry.is_package()));
                match row {
                    Some(ix) => DropTarget { folder: folder.row(ix)?.0.path.clone(), column, rows: Some(ix..ix + 1) },
                    None => DropTarget { folder: folder.dir().to_path_buf(), column, rows: None },
                }
            }
        };
        crate::operations::drop_would_act(dragged, &target.folder).then_some(target)
    }

    fn update_drag_row(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let row = if self.mode == ViewMode::List { self.drag_row_at(cx, position) } else { None };
        let target = self.drop_target_at(position, crate::drag_drop::active_paths(cx), cx);
        let column = if self.mode == ViewMode::Columns { self.column_row_at(cx, position) } else { None };
        if row != self.drop_row || target != self.drop_target || column != self.drop_column {
            self.drop_row = row;
            self.drop_target = target;
            self.drop_column = column;
            cx.notify();
        }
    }

    #[cfg(test)]
    pub fn drop_row(&self) -> Option<usize> {
        self.drop_row
    }

    #[cfg(test)]
    pub fn drop_target(&self) -> Option<&DropTarget> {
        self.drop_target.as_ref()
    }

    #[cfg(test)]
    pub fn drop_column(&self) -> Option<(usize, usize)> {
        self.drop_column
    }

    fn bench_step(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = self.mode;
        let icon_scroll = self.icon_scroll.clone();
        let icon_rows =
            self.folders.first().map(|folder| self.icon_grid(folder.read(cx).row_count(), cx).rows()).unwrap_or(0);
        let Some(bench) = self.bench.as_mut() else { return };
        window.request_animation_frame();
        let folder = self.folders[0].read(cx);
        let (count, shown, loading) = (folder.entries().len(), folder.row_count(), folder.is_loading());
        if loading || count == 0 || shown == 0 {
            return;
        }
        let now = Instant::now();
        if let Some(prev) = bench.last.replace(now) {
            bench.intervals.push((now - prev).as_secs_f64() * 1000.0);
        }
        bench.frame += 1;
        // About 3 rows per frame: a fast, continuous trackpad scroll.
        if mode == ViewMode::Icons {
            if icon_rows > 0 {
                icon_scroll.scroll_to_item_strict((bench.frame * 3) % icon_rows, ScrollStrategy::Top);
            }
        } else if mode == ViewMode::Gallery {
            if shown > 0 {
                self.gallery.scroll.scroll_to_item_strict((bench.frame * 3) % shown, ScrollStrategy::Top);
            }
        } else {
            folder.scroll_strict((bench.frame * 3) % shown);
        }
        if bench.frame == 600 {
            let mut v = std::mem::take(&mut bench.intervals);
            v.sort_by(f64::total_cmp);
            let at = |q: f64| v[((v.len() - 1) as f64 * q) as usize];
            let avg = v.iter().sum::<f64>() / v.len() as f64;
            bench.scrolled = Some(format!(
                "view={mode:?} items={count} cold_first_paint={:.0}ms cold_load={:.0}ms frames={} avg={avg:.2}ms ({:.0} fps) p50={:.2}ms p95={:.2}ms p99={:.2}ms max={:.2}ms",
                folder.first_paint_ms().unwrap_or_default(),
                folder.load_ms(),
                v.len(),
                1000.0 / avg,
                at(0.5),
                at(0.95),
                at(0.99),
                at(1.0),
            ));
            // Reopen the folder: the first paint of a warm app is what navigating feels like.
            let (dir, filter) = (folder.dir().to_path_buf(), bench.filter.clone());
            self.enter(dir, cx);
            if let Some(filter) = filter {
                let folder = self.folders[0].clone();
                folder.update(cx, |folder, cx| folder.set_filter(Some(filter), cx));
            }
        } else if bench.frame > 600 {
            let plugin = cx.try_global::<crate::plugins::Plugins>().map_or_else(String::new, |plugins| {
                let counters = plugins.surfaces.counters();
                format!(
                    " plugin_rows={} plugin_hooks={} plugin_cache={} plugin_cache_bytes={}",
                    counters.submitted_rows,
                    counters.submitted_hooks,
                    plugins.surfaces.row_count(),
                    plugins.surfaces.row_bytes()
                )
            });
            let line = format!(
                "BENCH {} warm_first_paint={:.0}ms warm_load={:.0}ms{plugin}",
                bench.scrolled.as_deref().unwrap_or_default(),
                folder.first_paint_ms().unwrap_or_default(),
                folder.load_ms()
            );
            println!("{line}");
            // The bundle launched through `open` has no captured stdout; the log keeps the result.
            tracing::info!("{line}");
            cx.quit();
        }
    }

    // ----- list view -----

    fn icon_row_decoration(
        &self,
        caption: Option<&str>,
        columns: usize,
        cx: &App,
    ) -> Option<crate::plugins::PluginRows> {
        let descriptors = crate::plugins::icon_row_descriptors(cx, caption);
        if descriptors.is_empty() {
            return None;
        }
        let folder = self.folders.first()?.clone();
        let (surface, dir) = {
            let folder = folder.read(cx);
            (folder.plugin_surface(), folder.dir().to_path_buf())
        };
        Some(crate::plugins::PluginRows::new_grid(
            folder,
            surface,
            crate::plugins::ScopeKey::Folder(dir),
            descriptors,
            columns,
        ))
    }

    fn plugin_row_decoration(&self, k: usize, cx: &App) -> Option<crate::plugins::PluginRows> {
        let descriptors = crate::plugins::row_descriptors(cx);
        if descriptors.is_empty() {
            return None;
        }
        let folder = self.folders.get(k)?.clone();
        let (surface, dir) = {
            let folder = folder.read(cx);
            (folder.plugin_surface(), folder.dir().to_path_buf())
        };
        Some(crate::plugins::PluginRows::new(folder, surface, crate::plugins::ScopeKey::Folder(dir), descriptors))
    }

    fn request_plugin_status(&self, cx: &mut Context<Self>) {
        if cx.try_global::<crate::plugins::Plugins>().is_none() {
            return;
        }
        let (surface, scope, revision, display) = {
            let folder = self.focused_folder().read(cx);
            let dir = folder.dir().to_path_buf();
            (
                folder.plugin_surface(),
                crate::plugins::ScopeKey::Folder(dir.clone()),
                folder.plugin_scope_version(),
                dir.display().to_string(),
            )
        };
        crate::plugins::request_status(cx, surface, scope, revision, display);
    }

    fn update_plugin_command_context(&self, window: &mut Window, cx: &mut Context<Self>) {
        if cx.try_global::<crate::plugins::Plugins>().is_none() {
            return;
        }
        if window.is_window_active() && self.focus.is_focused(window) {
            let folder = self.focused_folder().read(cx);
            let selection = folder.selected_entries().iter().map(|(path, is_dir)| (path.clone(), *is_dir)).collect();
            let context = crate::plugins::FileCommandContext {
                window: window.window_handle().window_id(),
                surface: folder.plugin_surface(),
                scope: crate::plugins::ScopeKey::Folder(folder.dir().to_path_buf()),
                selection,
            };
            crate::plugins::set_command_context(window, Some(context), cx);
        } else {
            crate::plugins::set_command_context(window, None, cx);
        }
    }

    fn render_table_rows(
        &mut self,
        widths: &[f32],
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<impl IntoElement + use<>> {
        let cloud_visible: Vec<PathBuf> = {
            let folder = self.folders[0].read(cx);
            range.clone().filter_map(|ix| folder.row(ix)).map(|(entry, _)| entry.path.clone()).collect()
        };
        self.observe_cloud((self.cloud_source, 0), &cloud_visible, cx);
        let visible: Vec<PathBuf> = {
            let folder = self.folders[0].read(cx);
            range
                .clone()
                .filter_map(|ix| folder.row(ix))
                .filter(|(entry, _)| entry.is_dir && !entry.is_symlink)
                .map(|(entry, _)| entry.path.clone())
                .collect()
        };
        let sizes_enabled = cx.global::<Ui>().settings.folder_sizes == crow_core::FolderSizesMode::Visible;
        let size_snapshot: HashMap<PathBuf, KnownSize> = match cx.try_global::<SizeService>() {
            Some(service) => {
                let service = service.0.read(cx);
                visible
                    .iter()
                    .filter(|path| sizes_enabled || service.manual(path))
                    .filter_map(|path| service.size(path).map(|size| (path.clone(), size.into())))
                    .collect()
            }
            None => HashMap::new(),
        };
        self.observe_sizes((self.size_source, 0), &visible, cx);
        let (scope, versions) = {
            let folder = self.folders[0].read(cx);
            let versions = range
                .clone()
                .filter_map(|ix| folder.row(ix))
                .map(|(entry, _)| (entry.path.clone(), folder.plugin_row_version(entry)))
                .collect::<HashMap<_, _>>();
            (crate::plugins::ScopeKey::Folder(folder.dir().to_path_buf()), versions)
        };
        let decoration_ids = crate::plugins::decoration_ids(cx);
        let presentations: HashMap<PathBuf, crate::plugins::RowPresentation> = versions
            .iter()
            .map(|(path, version)| {
                (path.clone(), crate::plugins::row_presentation(cx, &scope, path, *version, &decoration_ids))
            })
            .collect();
        let has_decorations = !decoration_ids.is_empty();
        let plugin_value = |extension: &crow_core::ExtensionId, path: &Path, version: u64| {
            let plugins = cx.try_global::<crate::plugins::Plugins>()?;
            let value = plugins.surfaces.row_value_peek(extension, &scope, path, version)?;
            Some(crow_core::ColumnCell { text: value.text, spoken: value.spoken, opacity: 1.0, fade: None })
        };
        let plugin_version = |entry: &Entry| versions.get(&entry.path).copied().unwrap_or(0);
        let ui = cx.global::<Ui>();
        let theme = &ui.theme;
        let font_size = px(ui.settings.font_size);
        let mode = ui.feedback.mode;
        let now = Instant::now();
        let icons = self.icon_resolver(0, cx);
        let muted = color(theme.role("fg-muted"));
        let stripe = color_alpha(theme.role("surface"), 0x70);
        let rule = color_alpha(theme.role("fg"), 0x30);
        let active = self.focus.is_focused(window);
        let glide = mode == Effective::Full && self.view_motion.is_none() && active;
        let selection_bg = if active { active_selection(theme) } else { inactive_selection(theme) };
        let size = |entry: &Entry| size_snapshot.get(&entry.path).copied();
        let column_cx = ColumnContext {
            kind: &kind,
            time: &format_time,
            size: &size,
            plugin_value: Some(&plugin_value),
            plugin_version: Some(&plugin_version),
        };
        let folder = self.folders[0].read(cx);
        let list_folder = self.folders[0].clone();
        let motion = self.view_motion.as_ref().and_then(|motion| motion.progress(mode, now).map(|t| (motion, t)));
        let selected_entries = folder.selected_entries();
        let total = folder.row_count();
        let cursor = folder.cursor_ix();
        let range_start = range.start;
        let (flags_first, edges) = {
            let (first, flags) = island_flags(folder, range.clone());
            (first, island_edges(&flags))
        };
        let (flash, cut_edges, flash_flags) = {
            let clipboard = cx.global::<Operations>().clipboard.as_ref();
            let flash = clipboard
                .and_then(|clipboard| clipboard.flash)
                .map(|started| crow_core::motion::copy_flash(mode, started, now))
                .unwrap_or(0.0);
            let cut = clipboard.is_some_and(|clipboard| clipboard.cut && clipboard.marked);
            let last = (range.end + 1).min(folder.row_count());
            let cut_flags: Vec<bool> = (flags_first..last)
                .map(|ix| {
                    cut && folder.row(ix).is_some_and(|(entry, _)| clipboard.is_some_and(|c| c.contains(&entry.path)))
                })
                .collect();
            let flash_flags: Vec<bool> = (range.start..range.end)
                .map(|ix| {
                    flash > 0.0
                        && folder.row(ix).is_some_and(|(entry, _)| clipboard.is_some_and(|c| c.contains(&entry.path)))
                })
                .collect();
            (flash, island_edges(&cut_flags), flash_flags)
        };
        let rows: Vec<(usize, Entry, usize, bool)> = range
            .filter_map(|ix| {
                folder.row(ix).map(|(entry, depth)| (ix, entry.clone(), depth, folder.is_selected(&entry.path)))
            })
            .collect();
        rows.into_iter()
            .map(|(ix, entry, depth, selected)| {
                let cells: Vec<(ColumnCell, Align)> =
                    self.columns.iter().map(|c| (c.cell(&entry, &column_cx), c.align())).collect();
                let spoken = cells
                    .iter()
                    .map(|(cell, _)| cell.spoken.as_deref().unwrap_or(&cell.text))
                    .filter(|text| *text != "--")
                    .collect::<Vec<_>>()
                    .join(", ");
                let indent = NAME_INSET + depth as f32 * INDENT;
                let name_room = px(widths[0]
                    - indent
                    - TRIANGLE_WIDTH
                    - 8.0
                    - 16.0
                    - 8.0
                    - CELL_PADDING
                    - if has_decorations { 32.0 } else { 0.0 });
                let cloud = self.cloud_status(&entry.path, cx).or(entry.dataless.then_some(CloudStatus::NotDownloaded));
                let name_room = cloud_status::name_room(cloud, name_room);
                let name = truncate_middle_parts(&entry.name, name_room, font_size, window);
                let ranges = folder.row_match(ix).map(|range| map_range(&name, range));
                let expanded = folder.is_expanded(&entry.path, cx);
                let drag_entries = if selected {
                    selected_entries.clone()
                } else {
                    Arc::new(vec![(entry.path.clone(), entry.is_dir)])
                };
                let drag_icon = cloud_icon(cloud, &entry.path, entry.is_dir, 22);
                let drag_count = drag_entries.len();
                let drag_label: SharedString = if drag_count > 1 {
                    format!("{drag_count} items").into()
                } else {
                    display_text(&entry.name).into_owned().into()
                };
                let handoff = DragHandoff::new();
                let look = folder.effect_look(&entry.path, mode, now).unwrap_or_default();
                let shift = folder.row_shift(ix, mode, now, ROW_HEIGHT);
                let tint = effect_tint(look, theme);
                let is_flash = flash_flags[ix - range_start];
                let cut_edge = cut_edges.get(ix - flags_first).copied().flatten();
                let presentation = presentations.get(&entry.path).cloned().unwrap_or_default();
                let dim = if presentation.dim {
                    content_opacity(true, false)
                } else {
                    content_opacity(
                        entry.hidden,
                        self.renaming.as_ref().is_some_and(|renaming| renaming.path == entry.path),
                    )
                };
                let plugin_spoken = if presentation.spoken.is_empty() {
                    String::new()
                } else {
                    format!(", {}", presentation.spoken.join(", "))
                };
                let cloud_spoken =
                    cloud.and_then(CloudStatus::spoken_suffix).map(|suffix| format!(", {suffix}")).unwrap_or_default();
                let flash_tint = is_flash.then(|| color_alpha(theme.role("accent"), (flash * 0x40 as f32) as u8));
                let reveal = folder.disclosure_opacity(ix, now);
                let turn = folder.chevron_turn(&entry.path, now).unwrap_or(if expanded { 1.0 } else { 0.0 });
                let island = edges.get(ix - flags_first).copied().flatten();
                let moving = motion.and_then(|(motion, t)| motion.row(list_folder.entity_id(), &entry.path, t));
                let moving_fill = moving.filter(|row| selected && row.tracks_selection());
                let fade = motion.filter(|_| moving.is_none()).map_or(1.0, |(_, t)| 0.4 + 0.6 * t);
                let icon = row_icon(&entry, look, theme, dim, cloud, &icons).into_any_element();
                let icon = match moving {
                    Some(row) => row.icon(icon),
                    None => icon,
                };
                let row = div()
                    .id(ix)
                    // Only visible rows exist, so each row tells VoiceOver where it sits in the folder.
                    .role(Role::ListBoxOption)
                    .aria_label(format!("{}, {spoken}{plugin_spoken}{cloud_spoken}", display_text(&entry.name)))
                    .aria_position_in_set(ix + 1)
                    .aria_size_of_set(total)
                    .aria_selected(selected)
                    .when(cursor == Some(ix), |row| row.aria_active_descendant())
                    .h(px(ROW_HEIGHT))
                    .mx(px(LIST_SIDE_INSET))
                    .relative()
                    .flex()
                    .items_center()
                    .when_some(island, |row, island| with_island_rounding(row, Some(island)))
                    .when(island.is_some_and(|island| island.rule_above), |row| {
                        row.child(
                            div()
                                .absolute()
                                .top_0()
                                .left(px(8.))
                                .right(px(8.))
                                .h(px(1.))
                                .bg(rule)
                                .debug_selector(|| format!("island-rule-{}", entry.name)),
                        )
                    })
                    .when(island.is_some_and(|island| island.top), |row| {
                        row.child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .size(px(1.))
                                .debug_selector(|| format!("island-top-{}", entry.name)),
                        )
                    })
                    .when(island.is_some_and(|island| island.bottom), |row| {
                        row.child(
                            div()
                                .absolute()
                                .bottom_0()
                                .left_0()
                                .size(px(1.))
                                .debug_selector(|| format!("island-bottom-{}", entry.name)),
                        )
                    })
                    .when(shift != 0.0, |row| row.top(px(shift)))
                    .when(look.opacity * reveal * fade < 1.0, |row| row.opacity(look.opacity * reveal * fade))
                    .when(ix % 2 == 1 && !(selected && glide && cursor == Some(ix)), |row| row.bg(stripe))
                    .when(moving_fill.is_none() && draws_selection_fill(selected, glide, cursor == Some(ix)), |row| {
                        row.bg(selection_bg)
                    })
                    .children(moving_fill.map(|moving| moving.selection(selection_bg, island)))
                    .when_some(tint, |row, tint| {
                        row.child(with_island_rounding(div().absolute().inset_0().bg(tint), island))
                    })
                    .when_some(flash_tint, |row, tint| {
                        row.child(
                            with_island_rounding(div().absolute().inset_0().bg(tint), island)
                                .debug_selector(|| format!("flash-{}", entry.name)),
                        )
                    })
                    .when_some(cut_edge, |row, edge| {
                        row.child(
                            cut_outline(theme, edge.top, edge.bottom)
                                .debug_selector(|| format!("cut-outline-{}", entry.name)),
                        )
                    })
                    .debug_selector(|| format!("row-{}", entry.name))
                    .on_drag(DraggedSelection { entries: drag_entries }, {
                        let handoff = handoff.clone();
                        move |_value, offset, window, cx| {
                            drag_combo(drag_icon.clone(), &drag_label, offset, &handoff, window, cx)
                        }
                    })
                    .external_drag_payload::<DraggedSelection>(move |selection, window, cx| {
                        promote_drag(selection, &handoff, window, cx)
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            this.press(0, ix, event, window, cx)
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                            this.context_press(0, Some(ix), window, cx)
                        }),
                    )
                    .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                        if event.click_count() == 2 {
                            this.open_selected(cx);
                        }
                    }))
                    .child(
                        div()
                            .w(px(widths[0]))
                            .flex_none()
                            .pl(px(indent))
                            .pr(px(CELL_PADDING))
                            .flex()
                            .items_center()
                            .gap_2()
                            .when(moving.is_none(), |cell| cell.overflow_hidden())
                            .child(if entry.is_dir && !entry.is_package() {
                                let path = entry.path.clone();
                                let pane = list_folder.clone();
                                div()
                                    .id(("disclosure", ix))
                                    .debug_selector({
                                        let name = entry.name.clone();
                                        move || format!("disclosure-{name}")
                                    })
                                    .role(Role::Button)
                                    .aria_label(if expanded {
                                        format!("Collapse {}", display_text(&entry.name))
                                    } else {
                                        format!("Expand {}", display_text(&entry.name))
                                    })
                                    .w(px(TRIANGLE_WIDTH))
                                    .h(px(ROW_HEIGHT))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_sm()
                                    .hover(move |s| s.bg(color(theme.role("surface-raised"))))
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.stop_propagation();
                                        pane.update(cx, |folder, cx| folder.toggle_expansion(path.clone(), cx));
                                    }))
                                    .child(chevron_glyph(turn, muted))
                                    .into_any_element()
                            } else {
                                div().w(px(TRIANGLE_WIDTH)).flex_none().into_any_element()
                            })
                            .child(icon)
                            .child(
                                div()
                                    .debug_selector(|| format!("row-name-{}", entry.name))
                                    .relative()
                                    .left(px(look.dx))
                                    .opacity(dim)
                                    .when_some(presentation.tint.filter(|_| !selected), |cell, tint| {
                                        cell.text_color(color(theme.role(tint.key())))
                                    })
                                    // The rename field fills the name column; without this it
                                    // has no intrinsic width and collapses to zero.
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child({
                                        let label = match self.renaming.as_ref().filter(|r| r.path == entry.path) {
                                            Some(r) => div()
                                                .flex_1()
                                                .on_mouse_down(
                                                    MouseButton::Left,
                                                    cx.listener(|this, _, _, _| this.rename_pressed = true),
                                                )
                                                .child(r.input.clone())
                                                .into_any_element(),
                                            None => match ranges {
                                                Some(ranges) if ranges.iter().any(|range| !range.is_empty()) => div()
                                                    .min_w_0()
                                                    .overflow_hidden()
                                                    .whitespace_nowrap()
                                                    .child(highlight_text(
                                                        name.text.clone(),
                                                        &ranges,
                                                        theme,
                                                        selected && active,
                                                    ))
                                                    .into_any_element(),
                                                _ => div()
                                                    .min_w_0()
                                                    .overflow_hidden()
                                                    .whitespace_nowrap()
                                                    .child(name.text.clone())
                                                    .into_any_element(),
                                            },
                                        };
                                        match moving {
                                            Some(row) => row.label(label),
                                            None => label,
                                        }
                                    })
                                    .children(crate::cloud_status::badge(cloud, theme))
                                    .when(has_decorations, |cell| {
                                        cell.child(
                                            div()
                                                .w(px(32.))
                                                .flex_none()
                                                .overflow_hidden()
                                                .whitespace_nowrap()
                                                .text_ellipsis()
                                                .text_size(px(11.))
                                                .text_color(muted)
                                                .id("plugin-row-badge")
                                                .child(presentation.badge.clone().unwrap_or_default())
                                                .when(entry.is_dir && presentation.git_badge, |badge| {
                                                    let path = entry.path.clone();
                                                    badge
                                                        .role(Role::Button)
                                                        .aria_label("Review folder Git changes")
                                                        .on_click(move |_, window, cx| {
                                                            cx.stop_propagation();
                                                            crate::window::git_review::open_folder(
                                                                path.clone(),
                                                                window,
                                                                cx,
                                                            );
                                                        })
                                                }),
                                        )
                                    }),
                            ),
                    );
                row.children(cells.into_iter().enumerate().filter(|(i, _)| widths[i + 1] > 0.0).map(
                    |(i, (cell, align))| {
                        let width = px(widths[i + 1]);
                        let metadata_fade = motion.filter(|_| moving.is_some()).map_or(1.0, |(_, t)| 0.4 + 0.6 * t);
                        let opacity = dim * cell.opacity * metadata_fade;
                        let fade = cell.fade.filter(|_| mode != Effective::Off);
                        let cell = div()
                            .w(width)
                            .flex_none()
                            .px(px(CELL_PADDING))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_color(muted)
                            .opacity(opacity)
                            .when(align == Align::Right, |cell| cell.text_right())
                            .child(cell.text);
                        match fade {
                            Some(version) => {
                                let key = SharedString::from(format!("size-fade-{ix}-{i}-{version}"));
                                cell.with_animation(
                                    key,
                                    Animation::new(Duration::from_millis(crow_core::motion::ms::STATUS)),
                                    move |el, t| el.opacity(opacity * (0.3 + 0.7 * crow_core::motion::ease_out(t))),
                                )
                                .into_any_element()
                            }
                            None => cell.into_any_element(),
                        }
                    },
                ))
            })
            .collect()
    }

    // ----- icon view -----

    /// Standard grid rows. One uniform-list item is one row of tiles, so only the
    /// visible rows and a small overscan are laid out; `GridGeometry` is the one
    /// source for tile bounds, hit tests and scroll rows.
    fn render_icons(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let folder_entity = self.folders[0].clone();
        let geometry = self.icon_grid(folder_entity.read(cx).row_count(), cx);
        if let Some(old) = self.grid {
            if old != geometry {
                self.cancel_marquee(cx);
                // A preset change or a window resize that changes the column count
                // glides the visible tiles from where the last frame drew them. A
                // listing change alone (rows added) keeps the grid and does not.
                let (before, after) = (old.metrics(), geometry.metrics());
                let reflowed = old.columns() != geometry.columns()
                    || before.tile_width != after.tile_width
                    || before.tile_height != after.tile_height
                    || before.icon_size != after.icon_size;
                if reflowed
                    && let Some(motion) = ViewMotion::capture(self, cx.global::<Ui>().feedback.mode, Instant::now(), cx)
                {
                    #[cfg(test)]
                    {
                        self.reflow_capture = Some(motion.clone());
                    }
                    self.view_motion = Some(motion);
                }
            }
            let folder = folder_entity.read(cx);
            let viewport = self.icon_viewport.borrow();
            if let Some(viewport) = viewport.as_ref().filter(|viewport| viewport.folder == folder_entity.entity_id())
                && (old.columns() != geometry.columns()
                    || old.row_pitch() != geometry.row_pitch()
                    || viewport.generation != folder.layout_generation())
            {
                if let Some((index, top)) =
                    viewport.anchors.iter().find_map(|(path, top)| folder.row_of_path(path).map(|index| (index, top)))
                {
                    let scroll = &self.icon_scroll.0.borrow().base_handle;
                    let mut offset = scroll.offset();
                    offset.y = px((*top - geometry.row_top(geometry.row_of(index))).min(0.0));
                    scroll.set_offset(offset);
                }
                if old.metrics().icon_size != geometry.metrics().icon_size
                    && let Some(path) = &viewport.visible_cursor
                    && folder.selection_cursor() == Some(path)
                    && let Some(index) = folder.row_of_path(path)
                {
                    self.icon_scroll.scroll_to_item(geometry.row_of(index), ScrollStrategy::Nearest);
                }
            }
        }
        self.grid = Some(geometry);
        let rows = geometry.rows();
        let scroll = self.icon_scroll.clone();
        let bounds = self.icon_bounds.clone();
        let viewport = self.icon_viewport.clone();
        let viewport_scroll = scroll.clone();
        let render_rows =
            cx.processor(move |this, range, window, cx| this.render_icon_rows(geometry, range, window, cx));
        let mut body = uniform_list("icon-entries", rows, render_rows)
            .track_scroll(&scroll)
            .on_scroll(cx.listener(|this, _, _, cx| this.on_icon_scroll(cx)))
            .flex_1()
            .min_h_0();
        let caption = self.icon_caption.clone();
        if let Some(decoration) = self.icon_row_decoration(caption.as_deref(), geometry.columns(), cx) {
            body = body.with_decoration(decoration);
        }
        let field = self.render_filter_field(FilterPlace::List, window, cx);
        let no_matches = self.render_no_matches(cx);
        let footer = self.render_filter_footer(FilterPlace::List, cx);
        let theme = cx.global::<Ui>().theme.clone();
        let overlay = self.marquee_overlay(&theme);
        let body = match no_matches {
            Some(block) => block,
            None => div()
                .relative()
                .flex_1()
                .min_h_0()
                .w_full()
                .flex()
                .flex_col()
                .overflow_hidden()
                .on_children_prepainted(move |children, _, cx| {
                    let viewport_bounds = children.first().copied();
                    bounds.set(viewport_bounds);
                    let Some(viewport_bounds) = viewport_bounds else { return };
                    let folder = folder_entity.read(cx);
                    let offset = f32::from(viewport_scroll.0.borrow().base_handle.offset().y);
                    let visible =
                        geometry.with_viewport_height(f32::from(viewport_bounds.size.height)).visible_rows(-offset);
                    let mut anchors = Vec::new();
                    for row in visible.clone() {
                        let Some(last) = geometry.last_index_of_row(row) else { break };
                        let top = geometry.row_top(row) + offset;
                        for index in geometry.first_index_of_row(row)..=last {
                            if let Some((entry, _)) = folder.row(index) {
                                anchors.push((entry.path.clone(), top));
                            }
                        }
                    }
                    let visible_cursor = folder
                        .cursor_ix()
                        .filter(|index| visible.contains(&geometry.row_of(*index)))
                        .and_then(|_| folder.selection_cursor())
                        .map(Path::to_path_buf);
                    viewport.replace(Some(IconViewport {
                        folder: folder_entity.entity_id(),
                        generation: folder.layout_generation(),
                        anchors,
                        visible_cursor,
                    }));
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseDownEvent, window, cx| this.press_empty(0, event, window, cx)),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, event: &MouseDownEvent, window, cx| {
                        this.context_press_empty(0, event, window, cx)
                    }),
                )
                .when_some(self.drop_target.as_ref().filter(|target| target.rows.is_none()), |body, target| {
                    let opacity = crate::drag_drop::cue_opacity(cx, &target.folder);
                    body.child(
                        drop_outline(&theme).left(px(4.)).right(px(4.)).top(px(2.)).bottom(px(2.)).opacity(opacity),
                    )
                })
                .child(body)
                .children(overlay)
                .into_any_element(),
        };
        div()
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .children(field)
            .child(body)
            .children(footer)
    }

    fn render_icon_rows(
        &mut self,
        geometry: GridGeometry,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<impl IntoElement + use<>> {
        let metrics = geometry.metrics();
        let indices: Vec<usize> = range
            .clone()
            .flat_map(|row| {
                let first = geometry.first_index_of_row(row);
                let last = geometry.last_index_of_row(row).unwrap_or(first);
                first..=last
            })
            .collect();
        let cloud_visible: Vec<PathBuf> = {
            let folder = self.folders[0].read(cx);
            indices.iter().filter_map(|ix| folder.row(*ix)).map(|(entry, _)| entry.path.clone()).collect()
        };
        self.observe_cloud((self.cloud_source, 0), &cloud_visible, cx);
        let visible: Vec<PathBuf> = {
            let folder = self.folders[0].read(cx);
            indices
                .iter()
                .filter_map(|ix| folder.row(*ix))
                .filter(|(entry, _)| entry.is_dir && !entry.is_symlink)
                .map(|(entry, _)| entry.path.clone())
                .collect()
        };
        self.observe_sizes((self.size_source, 0), &visible, cx);
        let (scope, versions) = {
            let folder = self.folders[0].read(cx);
            let versions = indices
                .iter()
                .filter_map(|ix| folder.row(*ix))
                .map(|(entry, _)| (entry.path.clone(), folder.plugin_row_version(entry)))
                .collect::<HashMap<_, _>>();
            (crate::plugins::ScopeKey::Folder(folder.dir().to_path_buf()), versions)
        };
        let decoration_ids = crate::plugins::decoration_ids(cx);
        let presentations: HashMap<PathBuf, crate::plugins::RowPresentation> = versions
            .iter()
            .map(|(path, version)| {
                (path.clone(), crate::plugins::row_presentation(cx, &scope, path, *version, &decoration_ids))
            })
            .collect();
        let thumbnail_service = crate::thumbnails::service(cx);
        let thumbnail_gate = crate::thumbnails::readable_gate(cx);
        let mut thumbnail_requests = Vec::new();
        let ui = cx.global::<Ui>();
        let theme = &ui.theme;
        let font_size = px(ui.settings.font_size);
        let mode = ui.feedback.mode;
        let now = Instant::now();
        let icons = self.icon_resolver(0, cx);
        let active = self.focus.is_focused(window);
        let selection_bg = if active { active_selection(theme) } else { inactive_selection(theme) };
        let selected_entries = self.folders[0].read(cx).selected_entries();
        let (folder_id, total, cursor) = {
            let folder = self.folders[0].read(cx);
            (self.folders[0].entity_id(), folder.row_count(), folder.cursor_ix())
        };
        let text_width = px(metrics.tile_width - 8.0);
        let line_height = px(icon_name_line_height(ui.settings.font_size));
        let caption_id = self.icon_caption.as_deref().and_then(|id| crow_core::ExtensionId::parse(id).ok());
        let caption_reserved = self.icon_caption.is_some();
        let rows = range
            .map(|row| {
                let first = geometry.first_index_of_row(row);
                let last = geometry.last_index_of_row(row).unwrap_or(first);
                let tiles = (first..=last).filter_map(|ix| {
                    let folder = self.folders[0].read(cx);
                    let (entry, _) = folder.row(ix)?;
                    let entry = entry.clone();
                    let selected = folder.is_selected(&entry.path);
                    let cloud =
                        self.cloud_status(&entry.path, cx).or(entry.dataless.then_some(CloudStatus::NotDownloaded));
                    let presentation = presentations.get(&entry.path).cloned().unwrap_or_default();
                    let version = versions.get(&entry.path).copied().unwrap_or(0);
                    let caption = caption_id
                        .as_ref()
                        .and_then(|id| crate::plugins::column_value(cx, id, &scope, &entry.path, version));
                    let caption_text = caption
                        .as_ref()
                        .map(|value| format!("{} · {}", display_text(&value.title), display_text(&value.text)))
                        .unwrap_or_default();
                    let caption_spoken = caption.as_ref().and_then(|value| {
                        value.spoken.clone().or_else(|| Some(format!("{} {}", value.title, value.text)))
                    });
                    let dim = if presentation.dim {
                        content_opacity(true, false)
                    } else {
                        content_opacity(
                            entry.hidden,
                            self.renaming.as_ref().is_some_and(|renaming| renaming.path == entry.path),
                        )
                    };
                    let plugin_spoken = if presentation.spoken.is_empty() {
                        String::new()
                    } else {
                        format!(", {}", presentation.spoken.join(", "))
                    };
                    let cloud_spoken = cloud
                        .and_then(CloudStatus::spoken_suffix)
                        .map(|suffix| format!(", {suffix}"))
                        .unwrap_or_default();
                    let look = folder.effect_look(&entry.path, mode, now).unwrap_or_default();
                    let tint = effect_tint(look, theme);
                    let drag_entries = if selected {
                        selected_entries.clone()
                    } else {
                        Arc::new(vec![(entry.path.clone(), entry.is_dir)])
                    };
                    let drag_icon = cloud_icon(cloud, &entry.path, entry.is_dir, 22);
                    let drag_count = drag_entries.len();
                    let drag_label: SharedString = if drag_count > 1 {
                        format!("{drag_count} items").into()
                    } else {
                        display_text(&entry.name).into_owned().into()
                    };
                    let handoff = DragHandoff::new();
                    let points = metrics.icon_size as u32;
                    let revision = folder.plugin_entry_version(&entry);
                    let key = crate::thumbnails::ThumbnailKey::new(entry.path.clone(), revision, points);
                    let thumbnail = thumbnail_service.as_ref().and_then(|service| service.read(cx).image(&key));
                    let thumbnail_known =
                        thumbnail_service.as_ref().is_some_and(|service| service.read(cx).is_cached(&key));
                    let thumbnailable = thumbnail_service.is_some()
                        && (!entry.is_dir || entry.is_package())
                        && !entry.is_symlink
                        && !entry.dataless
                        && cloud.is_none_or(CloudStatus::is_local)
                        && !crate::style::native_cloud_icon(cloud, &entry.path);
                    if thumbnail.is_none() && !thumbnail_known && thumbnailable {
                        thumbnail_requests.push(crate::thumbnails::ThumbnailRequest {
                            key: key.clone(),
                            gate: thumbnail_gate.clone(),
                        });
                    }
                    let packed = (!crate::style::native_cloud_icon(cloud, &entry.path))
                        .then(|| icons.resolve(&entry.path, entry.is_dir, entry.is_package()))
                        .flatten();
                    let thumb = match thumbnail {
                        Some(image) => img(image).size(px(metrics.icon_size)).into_any_element(),
                        None => match packed {
                            Some(icon) => {
                                crate::icons::pack_glyph(&icon.asset, metrics.icon_size, color(icon.color.rgb(theme)))
                                    .into_any_element()
                            }
                            None => match cloud_icon(cloud, &entry.path, entry.is_dir, points) {
                                Some(image) => img(image).size(px(metrics.icon_size)).into_any_element(),
                                None => div().size(px(metrics.icon_size)).into_any_element(),
                            },
                        },
                    };
                    let thumb = div()
                        .relative()
                        .w(px(metrics.icon_size + ICON_BOX_PADDING))
                        .h(px(metrics.icon_size + ICON_BOX_PADDING))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity((1.0 - look.trash) * dim)
                        .child(thumb)
                        .when(look.trash > 0.0, |slot| {
                            slot.child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .opacity(look.trash)
                                    .child(glyph(Glyph::Trash, look.trash_size, "fg-muted", theme)),
                            )
                        })
                        .children(
                            crate::cloud_status::badge(cloud, theme)
                                .map(|badge| div().absolute().top(px(0.)).right(px(0.)).child(badge)),
                        )
                        .children(presentation.badge.clone().map(|badge| {
                            div()
                                .absolute()
                                .top(px(0.))
                                .left(px(0.))
                                .text_size(px(11.))
                                .text_color(color(theme.role("fg-muted")))
                                .child(badge)
                        }));
                    let name = fit_icon_name(&entry.name, text_width, font_size, window);
                    let clipped = name.cut.is_some();
                    let label = div().w(text_width).h(line_height * 2).flex_none().overflow_hidden();
                    let label = match self.renaming.as_ref().filter(|r| r.path == entry.path) {
                        Some(r) => label.child(r.input.clone()),
                        None => {
                            let ranges = folder
                                .row_match(ix)
                                .into_iter()
                                .flat_map(|range| map_range(&name, range))
                                .filter(|range| !range.is_empty())
                                .collect::<Vec<_>>();
                            label
                                .line_height(line_height)
                                .line_clamp(2)
                                .text_center()
                                .text_size(font_size)
                                .text_color(color(theme.role("fg")))
                                .opacity(dim)
                                .child(if ranges.is_empty() {
                                    name.text.into_any_element()
                                } else {
                                    highlight_text(name.text, &ranges, theme, selected && active).into_any_element()
                                })
                        }
                    }
                    .into_any_element();
                    let caption_display = if caption_reserved {
                        let text = truncate_middle(&caption_text, text_width, px(11.), window);
                        let nonempty = !caption_text.is_empty();
                        let name = entry.name.clone();
                        Some(
                            div()
                                .debug_selector(move || {
                                    if nonempty {
                                        format!("caption-text-{name}")
                                    } else {
                                        format!("caption-slot-{name}")
                                    }
                                })
                                .w(text_width)
                                .h(px(CAPTION_LINE_HEIGHT))
                                .line_height(px(CAPTION_LINE_HEIGHT))
                                .flex_none()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_center()
                                .text_size(px(11.))
                                .text_color(color(theme.role("fg-muted")))
                                .opacity(dim)
                                .child(text),
                        )
                    } else {
                        None
                    };
                    let clipboard = cx.global::<Operations>().clipboard.as_ref();
                    let cut = clipboard
                        .is_some_and(|clipboard| clipboard.cut && clipboard.marked && clipboard.contains(&entry.path));
                    let flash = clipboard
                        .and_then(|clipboard| {
                            clipboard.contains(&entry.path).then(|| {
                                clipboard
                                    .flash
                                    .map(|started| crow_core::motion::copy_flash(mode, started, now))
                                    .unwrap_or(0.0)
                            })
                        })
                        .filter(|flash| *flash > 0.0)
                        .map(|flash| color_alpha(theme.role("accent"), (flash * 0x40 as f32) as u8));
                    let moving = self.view_motion.as_ref().and_then(|motion| {
                        motion.progress(mode, now).and_then(|t| motion.row(folder_id, &entry.path, t))
                    });
                    let moving_fill = moving.filter(|row| selected && row.tracks_selection());
                    let (marquee_hit, marquee_base) =
                        self.marquee.as_ref().filter(|marquee| marquee.started).map_or((false, false), |marquee| {
                            (
                                marquee.hits.contains_key(&entry.path),
                                marquee.additive && marquee.base.contains(&entry.path),
                            )
                        });
                    let preview_only = marquee_hit && !marquee_base;
                    let drop_here = self.drop_target.as_ref().is_some_and(|target| target.folder == entry.path);
                    let icon_visual = match moving {
                        Some(row) => row.icon(thumb),
                        None => thumb.into_any_element(),
                    };
                    let label_visual = match moving {
                        Some(row) => row.label(label),
                        None => label,
                    };
                    let tile = div()
                        .id(("tile", ix))
                        .debug_selector({
                            let name = entry.name.clone();
                            move || format!("tile-{name}")
                        })
                        .role(Role::ListBoxOption)
                        .aria_label(format!(
                            "{}, {kind}{plugin_spoken}{caption_spoken}{cloud_spoken}",
                            display_text(&entry.name),
                            kind = kind(&entry),
                            caption_spoken = caption_spoken.map(|spoken| format!(", {spoken}")).unwrap_or_default()
                        ))
                        .aria_position_in_set(ix + 1)
                        .aria_size_of_set(total)
                        .aria_selected(selected)
                        .when(active && cursor == Some(ix), |tile| tile.aria_active_descendant())
                        .relative()
                        .w(px(metrics.tile_width))
                        .h(px(metrics.tile_height))
                        .flex_none()
                        .rounded_md()
                        .when(selected && moving_fill.is_none() && !preview_only, |tile| tile.bg(selection_bg))
                        .when(preview_only, |tile| {
                            tile.child(
                                div()
                                    .debug_selector({
                                        let name = entry.name.clone();
                                        move || format!("marquee-hit-{name}")
                                    })
                                    .absolute()
                                    .inset(px(2.))
                                    .rounded_md()
                                    .border_1()
                                    .border_color(color(theme.role("accent")))
                                    .bg(color_alpha(theme.role("accent"), 0x18)),
                            )
                        })
                        .when(drop_here, |tile| {
                            let opacity = crate::drag_drop::cue_opacity(cx, &entry.path);
                            tile.child(drop_outline(theme).absolute().inset(px(1.)).rounded_md().opacity(opacity))
                        })
                        .children(moving_fill.map(|row| {
                            row.selection(
                                selection_bg,
                                Some(IslandEdges { top: true, bottom: true, rule_above: false }),
                            )
                        }))
                        .when_some(tint, |tile, tint| tile.child(div().absolute().inset_0().rounded_md().bg(tint)))
                        .when_some(flash, |tile, tint| tile.child(div().absolute().inset_0().rounded_md().bg(tint)))
                        .when(cut, |tile| {
                            tile.child(
                                cut_outline(&theme, true, true)
                                    .debug_selector(|| format!("tile-cut-outline-{}", entry.name)),
                            )
                        })
                        .when(look.opacity < 1.0, |tile| tile.opacity(look.opacity))
                        .flex()
                        .flex_col()
                        .items_center()
                        .pt(px(ICON_TILE_TOP))
                        .gap(px(ICON_TILE_GAP))
                        .on_drag(DraggedSelection { entries: drag_entries }, {
                            let handoff = handoff.clone();
                            move |_value, offset, window, cx| {
                                drag_combo(drag_icon.clone(), &drag_label, offset, &handoff, window, cx)
                            }
                        })
                        .external_drag_payload::<DraggedSelection>(move |selection, window, cx| {
                            promote_drag(selection, &handoff, window, cx)
                        })
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                this.press(0, ix, event, window, cx)
                            }),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                this.context_press(0, Some(ix), window, cx)
                            }),
                        )
                        .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                            if event.click_count() == 2 {
                                this.open_selected(cx);
                            }
                        }))
                        .child(icon_visual)
                        .child(label_visual)
                        .children(caption_display);
                    let tile = if (clipped || !caption_text.is_empty()) && self.renaming.is_none() {
                        let tooltip = if caption_text.is_empty() {
                            display_text(&entry.name).into_owned()
                        } else {
                            format!("{} — {}", display_text(&entry.name), caption_text)
                        };
                        with_tooltip(tile, tooltip, None)
                    } else {
                        tile
                    };
                    Some(tile.into_any_element())
                });
                div()
                    .h(px(geometry.row_pitch()))
                    .w_full()
                    .flex()
                    .items_start()
                    .gap(px(metrics.gap))
                    .px(px(metrics.inset_x))
                    .children(tiles)
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        if let Some(service) = thumbnail_service {
            service.update(cx, |service, cx| service.request(thumbnail_requests, cx));
        }
        rows
    }

    fn on_icon_scroll(&mut self, cx: &mut Context<Self>) {
        if self.view_motion.take().is_some() {
            cx.notify();
        }
        if self.filtering.is_some() || !crate::tips::wants(TipId::Filter, cx) {
            return;
        }
        let Some(folder) = self.folders.first() else { return };
        let folder = folder.read(cx);
        if folder.entries().len() < TIP_SCROLL_ENTRIES {
            return;
        }
        let offset = f32::from(self.icon_scroll.0.borrow().base_handle.offset().y);
        if self.grid.is_some_and(|grid| grid.first_visible_row(-offset) >= TIP_SCROLL_ROWS) {
            cx.emit(PaneEvent::LongFolderScrolled);
        }
    }

    fn render_table_header(&self, widths: &[f32], cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.global::<Ui>().theme.clone();
        let sort = self.sort;
        // Columns hidden for a narrow pane have width 0.
        let last = widths.iter().rposition(|w| *w > 0.0).unwrap_or(0);
        let border = color(theme.role("border"));
        // (title, sort key, alignment) for the name column and each shown column.
        let mut specs = vec![("Name".to_owned(), Some(SortKey::Name), Align::Left)];
        specs.extend(self.columns.iter().map(|c| (c.title().to_owned(), c.sort_key(), c.align())));
        let mut header = div()
            .id("table-header")
            .role(Role::Group)
            .aria_label("List columns")
            .on_mouse_down(MouseButton::Right, |_, window, cx| {
                cx.stop_propagation();
                crate::window::column_picker::open(window, cx);
            })
            .debug_selector(|| "table-header".into())
            .h(px(28.))
            .flex_none()
            .mx(px(LIST_SIDE_INSET))
            .flex()
            .items_center()
            .text_size(px(12.))
            .text_color(color(theme.role("fg-muted")))
            .border_b_1()
            .border_color(border);
        for (i, (title, key, align)) in specs.into_iter().enumerate().filter(|(i, _)| widths[*i] > 0.0) {
            let sorted = key.is_some_and(|k| k == sort.key);
            let chevron = match (sorted, sort.ascending) {
                (false, _) => "",
                (true, true) => "⌃",
                (true, false) => "⌄",
            };
            let width = px(widths[i]);
            let cell = div()
                .id(("column", i))
                .debug_selector(move || format!("column-header-{i}"))
                .role(Role::ColumnHeader)
                .aria_label(format!("{title}{}", if sorted { ", sorted" } else { "" }))
                .relative()
                .h_full()
                .w(width)
                .flex_none()
                .pl(px(if i == 0 { NAME_INSET + TRIANGLE_WIDTH + 8.0 } else { CELL_PADDING }))
                .pr(px(CELL_PADDING))
                .flex()
                .items_center()
                .gap_1()
                .when(i != last, |cell| cell.border_r_1().border_color(border))
                .when(sorted, |cell| cell.text_color(color(theme.role("fg"))).font_weight(gpui::FontWeight::SEMIBOLD))
                .when_some(key, |cell, key| {
                    cell.on_click(cx.listener(move |this, _, _, cx| this.set_sort(this.sort.toggled(key), cx)))
                })
                .child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .when(align == Align::Right, |t| t.text_right())
                        .child(title),
                )
                .when(key.is_some(), |cell| cell.child(div().w(px(10.)).flex_none().child(chevron)))
                // The divider's grab area, a little wider than the line.
                .when(i != last, |cell| {
                    cell.child(
                        div()
                            .id(("resize", i))
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .right(px(-4.))
                            .w(px(8.))
                            .cursor(gpui::CursorStyle::ResizeLeftRight)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                    this.resize = this.table.begin_resize(i, f32::from(event.position.x));
                                    cx.stop_propagation();
                                }),
                            ),
                    )
                });
            header = header.child(cell);
        }
        header
    }

    // ----- column view -----

    fn render_column_rows(
        &mut self,
        k: usize,
        width: Pixels,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<impl IntoElement + use<>> {
        let cloud_visible: Vec<PathBuf> = self
            .folders
            .get(k)
            .map(|folder| {
                let folder = folder.read(cx);
                range.clone().filter_map(|ix| folder.row(ix)).map(|(entry, _)| entry.path.clone()).collect()
            })
            .unwrap_or_default();
        self.observe_cloud((self.cloud_source, k + 1), &cloud_visible, cx);
        let visible: Vec<PathBuf> = self
            .folders
            .get(k)
            .map(|folder| {
                let folder = folder.read(cx);
                range
                    .clone()
                    .filter_map(|ix| folder.row(ix))
                    .filter(|(entry, _)| entry.is_dir && !entry.is_symlink)
                    .map(|(entry, _)| entry.path.clone())
                    .collect()
            })
            .unwrap_or_default();
        self.observe_sizes((self.size_source, k + 1), &visible, cx);
        let ui = cx.global::<Ui>();
        let theme = &ui.theme;
        let font_size = px(ui.settings.font_size);
        let mode = ui.feedback.mode;
        let now = Instant::now();
        // Spec: only the focused column shows its selection in the accent color.
        let active = k == self.focused && self.focus.is_focused(window);
        let glide = mode == Effective::Full && self.view_motion.is_none() && active;
        let selection_bg = if active { active_selection(theme) } else { inactive_selection(theme) };
        let subtle = color(theme.role("fg-subtle"));
        let muted = color(theme.role("fg-muted"));
        let rule = color_alpha(theme.role("fg"), 0x30);
        let Some(folder) = self.folders.get(k).map(|f| f.read(cx)) else { return Vec::new() };
        let icons = self.icon_resolver(k, cx);
        let folder_id = self.folders[k].entity_id();
        let motion = self
            .view_motion
            .as_ref()
            .filter(|_| k == self.focused)
            .and_then(|motion| motion.progress(mode, now).map(|t| (motion, t)));
        let total = folder.row_count();
        let scope = crate::plugins::ScopeKey::Folder(folder.dir().to_path_buf());
        let versions: HashMap<PathBuf, u64> = range
            .clone()
            .filter_map(|ix| folder.row(ix))
            .map(|(entry, _)| (entry.path.clone(), folder.plugin_row_version(entry)))
            .collect();
        let decoration_ids = crate::plugins::decoration_ids(cx);
        let presentations: HashMap<PathBuf, crate::plugins::RowPresentation> = versions
            .iter()
            .map(|(path, version)| {
                (path.clone(), crate::plugins::row_presentation(cx, &scope, path, *version, &decoration_ids))
            })
            .collect();
        let arrow_width = px(10.);
        let badge_width = px(32.);
        // Column border, list gutters, row padding, icon and its gap.
        let name_room = width - px(1. + 8. + 2. * CELL_PADDING + 16. + 8.);
        let cursor = folder.cursor_ix();
        let range_start = range.start;
        let (flags_first, edges) = {
            let (first, flags) = island_flags(folder, range.clone());
            (first, island_edges(&flags))
        };
        let (flash, cut_edges, flash_flags) = {
            let clipboard = cx.global::<Operations>().clipboard.as_ref();
            let flash = clipboard
                .and_then(|clipboard| clipboard.flash)
                .map(|started| crow_core::motion::copy_flash(mode, started, now))
                .unwrap_or(0.0);
            let cut = clipboard.is_some_and(|clipboard| clipboard.cut && clipboard.marked);
            let last = (range.end + 1).min(folder.row_count());
            let cut_flags: Vec<bool> = (flags_first..last)
                .map(|ix| {
                    cut && folder.row(ix).is_some_and(|(entry, _)| clipboard.is_some_and(|c| c.contains(&entry.path)))
                })
                .collect();
            let flash_flags: Vec<bool> = (range.start..range.end)
                .map(|ix| {
                    flash > 0.0
                        && folder.row(ix).is_some_and(|(entry, _)| clipboard.is_some_and(|c| c.contains(&entry.path)))
                })
                .collect();
            (flash, island_edges(&cut_flags), flash_flags)
        };
        let selected_entries = folder.selected_entries();
        let rows: Vec<(usize, Entry, bool)> =
            range.filter_map(|ix| folder.row(ix).map(|(e, _)| (ix, e.clone(), folder.is_selected(&e.path)))).collect();
        rows.into_iter()
            .map(|(ix, entry, selected)| {
                let cloud = self.cloud_status(&entry.path, cx).or(entry.dataless.then_some(CloudStatus::NotDownloaded));
                let navigable = entry.is_dir && !entry.is_package();
                let presentation = presentations.get(&entry.path).cloned().unwrap_or_default();
                let mut room = cloud_status::name_room(cloud, name_room);
                if navigable {
                    room -= arrow_width + px(8.);
                }
                if presentation.badge.is_some() {
                    room -= badge_width + px(4.);
                }
                let name = truncate_middle_parts(&entry.name, room.max(px(0.)), font_size, window);
                let ranges = folder.row_match(ix).map(|range| map_range(&name, range));
                let renaming = self.renaming.as_ref().filter(|r| r.path == entry.path);
                let drag_entries = if selected {
                    selected_entries.clone()
                } else {
                    Arc::new(vec![(entry.path.clone(), entry.is_dir)])
                };
                let drag_icon = cloud_icon(cloud, &entry.path, entry.is_dir, 22);
                let drag_count = drag_entries.len();
                let drag_label: SharedString = if drag_count > 1 {
                    format!("{drag_count} items").into()
                } else {
                    display_text(&entry.name).into_owned().into()
                };
                let handoff = DragHandoff::new();
                let look = folder.effect_look(&entry.path, mode, now).unwrap_or_default();
                let shift = folder.row_shift(ix, mode, now, ROW_HEIGHT);
                let tint = effect_tint(look, theme);
                let is_flash = flash_flags[ix - range_start];
                let cut_edge = cut_edges.get(ix - flags_first).copied().flatten();
                let dim = if presentation.dim {
                    content_opacity(true, false)
                } else {
                    content_opacity(
                        entry.hidden,
                        self.renaming.as_ref().is_some_and(|renaming| renaming.path == entry.path),
                    )
                };
                let plugin_spoken = if presentation.spoken.is_empty() {
                    String::new()
                } else {
                    format!(", {}", presentation.spoken.join(", "))
                };
                let cloud_spoken =
                    cloud.and_then(CloudStatus::spoken_suffix).map(|suffix| format!(", {suffix}")).unwrap_or_default();
                let flash_tint = is_flash.then(|| color_alpha(theme.role("accent"), (flash * 0x40 as f32) as u8));
                let island = edges.get(ix - flags_first).copied().flatten();
                let moving = motion.and_then(|(motion, t)| motion.row(folder_id, &entry.path, t));
                let moving_fill = moving.filter(|row| selected && row.tracks_selection());
                let fade = motion.filter(|_| moving.is_none()).map_or(1.0, |(_, t)| 0.4 + 0.6 * t);
                let icon = row_icon(&entry, look, theme, dim, cloud, &icons).into_any_element();
                let icon = match moving {
                    Some(row) => row.icon(icon),
                    None => icon,
                };
                div()
                    .id(ix)
                    .role(Role::ListBoxOption)
                    .aria_label(format!("{}, {}{plugin_spoken}{cloud_spoken}", display_text(&entry.name), kind(&entry)))
                    .aria_position_in_set(ix + 1)
                    .aria_size_of_set(total)
                    .aria_selected(selected)
                    .when(active && cursor == Some(ix), |row| row.aria_active_descendant())
                    .h(px(ROW_HEIGHT))
                    .w_full()
                    .px(px(CELL_PADDING))
                    .relative()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when_some(island, |row, island| with_island_rounding(row, Some(island)))
                    .when(island.is_some_and(|island| island.rule_above), |row| {
                        row.child(
                            div()
                                .absolute()
                                .top_0()
                                .left(px(CELL_PADDING))
                                .right(px(CELL_PADDING))
                                .h(px(1.))
                                .bg(rule)
                                .debug_selector(|| format!("column-{k}-island-rule-{}", entry.name)),
                        )
                    })
                    .when(island.is_some_and(|island| island.top), |row| {
                        row.child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .size(px(1.))
                                .debug_selector(|| format!("column-{k}-island-top-{}", entry.name)),
                        )
                    })
                    .when(island.is_some_and(|island| island.bottom), |row| {
                        row.child(
                            div()
                                .absolute()
                                .bottom_0()
                                .left_0()
                                .size(px(1.))
                                .debug_selector(|| format!("column-{k}-island-bottom-{}", entry.name)),
                        )
                    })
                    .when(shift != 0.0, |row| row.top(px(shift)))
                    .when(look.opacity * fade < 1.0, |row| row.opacity(look.opacity * fade))
                    .when(moving_fill.is_none() && draws_selection_fill(selected, glide, cursor == Some(ix)), |row| {
                        row.bg(selection_bg)
                    })
                    .children(moving_fill.map(|moving| moving.selection(selection_bg, island)))
                    .when_some(tint, |row, tint| {
                        row.child(with_island_rounding(div().absolute().inset_0().bg(tint), island))
                    })
                    .when_some(flash_tint, |row, tint| {
                        row.child(
                            with_island_rounding(div().absolute().inset_0().bg(tint), island)
                                .debug_selector(|| format!("column-{k}-flash-{}", entry.name)),
                        )
                    })
                    .when_some(cut_edge, |row, edge| {
                        row.child(
                            cut_outline(theme, edge.top, edge.bottom)
                                .debug_selector(|| format!("column-{k}-cut-outline-{}", entry.name)),
                        )
                    })
                    .debug_selector(|| format!("column-{k}-row-{}", entry.name))
                    .when(name.cut.is_some() && renaming.is_none(), |row| {
                        crate::chrome::with_tooltip(row, display_text(&entry.name).into_owned(), None)
                    })
                    .on_drag(DraggedSelection { entries: drag_entries }, {
                        let handoff = handoff.clone();
                        move |_value, offset, window, cx| {
                            drag_combo(drag_icon.clone(), &drag_label, offset, &handoff, window, cx)
                        }
                    })
                    .external_drag_payload::<DraggedSelection>(move |selection, window, cx| {
                        promote_drag(selection, &handoff, window, cx)
                    })
                    .when(
                        self.drop_target.as_ref().is_some_and(|t| t.column == k && t.rows == Some(ix..ix + 1)),
                        |row| {
                            row.child(
                                drop_outline(theme).inset_0().opacity(crate::drag_drop::cue_opacity(cx, &entry.path)),
                            )
                        },
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            this.press(k, ix, event, window, cx)
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                            this.context_press(k, Some(ix), window, cx)
                        }),
                    )
                    .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                        if event.click_count() == 2 {
                            this.open_selected(cx);
                        }
                    }))
                    .child(icon)
                    .child(
                        div()
                            .debug_selector(|| format!("column-{k}-row-name-{}", entry.name))
                            .relative()
                            .left(px(look.dx))
                            .flex_1()
                            .min_w_0()
                            .opacity(dim)
                            .when_some(presentation.tint.filter(|_| !selected), |cell, tint| {
                                cell.text_color(color(theme.role(tint.key())))
                            })
                            .flex()
                            .items_center()
                            .gap_1()
                            .child({
                                let label = match renaming {
                                    Some(r) => div()
                                        .flex_1()
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(|this, _, _, _| this.rename_pressed = true),
                                        )
                                        .child(r.input.clone())
                                        .into_any_element(),
                                    None => match ranges {
                                        Some(ranges) if ranges.iter().any(|range| !range.is_empty()) => div()
                                            .flex_1()
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .child(highlight_text(
                                                name.text.clone(),
                                                &ranges,
                                                theme,
                                                selected && active,
                                            ))
                                            .into_any_element(),
                                        _ => div()
                                            .flex_1()
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .child(name.text.clone())
                                            .into_any_element(),
                                    },
                                };
                                match moving {
                                    Some(row) => row.label(label),
                                    None => label,
                                }
                            })
                            .children(crate::cloud_status::badge(cloud, theme))
                            .when_some(presentation.badge, |cell, text| {
                                cell.child(
                                    div()
                                        .w(badge_width)
                                        .flex_none()
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .text_size(px(11.))
                                        .text_color(muted)
                                        .id("plugin-row-badge")
                                        .child(text)
                                        .when(entry.is_dir && presentation.git_badge, |badge| {
                                            let path = entry.path.clone();
                                            badge.role(Role::Button).aria_label("Review folder Git changes").on_click(
                                                move |_, window, cx| {
                                                    cx.stop_propagation();
                                                    crate::window::git_review::open_folder(path.clone(), window, cx);
                                                },
                                            )
                                        }),
                                )
                            }),
                    )
                    .when(navigable, |row| {
                        row.child(div().w(arrow_width).flex_none().opacity(dim).text_color(subtle).child("›"))
                    })
            })
            .collect()
    }

    fn render_columns(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.global::<Ui>().theme.clone();
        let (mode, font_size) = {
            let ui = cx.global::<Ui>();
            (ui.feedback.mode, px(ui.settings.font_size))
        };
        let now = Instant::now();
        let border = color(theme.role("border"));
        let focused = self.focused;
        // Keep widths independent of selection depth and the Info toggle.
        // Deeper chains still scroll horizontally on narrow windows.
        let width = px(((f32::from(self.content_width) - PREVIEW_COLUMN_WIDTH) / 2.0)
            .floor()
            .clamp(COLUMN_MIN_WIDTH, COLUMN_MAX_WIDTH));
        let mut strip = div()
            .id("columns")
            .debug_selector(|| "columns".into())
            .size_full()
            .flex()
            .overflow_x_scroll()
            .track_scroll(&self.strip_scroll);
        for (k, folder) in self.folders.iter().enumerate() {
            let (count, shown, scroll, title, filtered) = {
                let f = folder.read(cx);
                (
                    f.entries().len(),
                    f.row_count(),
                    f.scroll().clone(),
                    f.dir().file_name().map_or_else(|| "/".to_owned(), |n| n.to_string_lossy().into_owned()),
                    f.filter().is_some(),
                )
            };
            // Only the focused column holds the field; the others render as always.
            let (field, no_matches, footer) = if k == focused {
                (
                    self.render_filter_field(FilterPlace::Column, window, cx),
                    self.render_no_matches(cx),
                    self.render_filter_footer(FilterPlace::Column, cx),
                )
            } else {
                (None, None, None)
            };
            let has_field = field.is_some();
            let glide =
                mode == Effective::Full && self.view_motion.is_none() && k == focused && self.focus.is_focused(window);
            let icons = self.icon_resolver(k, cx);
            let body = match no_matches {
                Some(block) => block,
                None => {
                    let ghosts = {
                        let f = folder.read(cx);
                        Ghosts {
                            folder: f,
                            mode,
                            now,
                            scroll: &scroll,
                            theme: &theme,
                            font_size,
                            icons: &icons,
                            side_inset: 4.0,
                            top_inset: 0.0,
                        }
                        .render()
                    };
                    let glide_fill = if glide {
                        let f = folder.read(cx);
                        let edges = cursor_island_edges(f);
                        f.cursor_ix().map(|ix| {
                            let offset = shown_offset(&scroll);
                            let y = ix as f32 * ROW_HEIGHT;
                            let fill = div()
                                .absolute()
                                .left(px(4.))
                                .right(px(4.))
                                .h(px(ROW_HEIGHT))
                                .bg(active_selection(&theme));
                            with_island_rounding(fill, edges)
                                .debug_selector(move || format!("column-glide-{k}"))
                                .with_spring(
                                    ("glide", k),
                                    SpringAnimation::new(SpringConfig::new(2500.0, 2.0 * 2500.0f32.sqrt(), 1.0))
                                        .to(px(y)),
                                    move |el, y| el.top(px(f32::from(y) + offset)),
                                )
                        })
                    } else {
                        None
                    };
                    div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .when(self.tip_inset, |body| body.pb(px(crate::tips::INSET)))
                        .children(glide_fill)
                        .child({
                            let mut list = uniform_list(
                                ("column-list", k),
                                shown,
                                cx.processor(move |this, range, window, cx| {
                                    this.render_column_rows(k, width, range, window, cx)
                                }),
                            )
                            .track_scroll(&scroll)
                            .flex_1()
                            .min_h_0()
                            .px_1();
                            if let Some(plugin_rows) = self.plugin_row_decoration(k, cx) {
                                list = list.with_decoration(plugin_rows);
                            }
                            list
                        })
                        .children(ghosts)
                        .into_any_element()
                }
            };
            let body = if let Some(t) = self.child_reveal.and_then(|(id, started)| {
                (id == folder.entity_id())
                    .then(|| motion::navigation_progress(mode, started, now, motion::ms::CHILD_COLUMN))
                    .flatten()
            }) {
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .debug_selector(move || format!("column-reveal-content-{k}"))
                            .when(mode == Effective::Full, |body| body.left(px(-8.0 * (1.0 - t))))
                            .opacity(0.3 + 0.7 * t)
                            .child(body),
                    )
                    .into_any_element()
            } else {
                body
            };
            strip = strip.child(
                div()
                    .on_children_prepainted({
                        let bounds = self.column_bounds.clone();
                        move |children, _, _| {
                            // The field (when present) is the first child, then the list.
                            if let Some(child) = children.get(usize::from(has_field)) {
                                bounds.borrow_mut().insert(k, *child);
                            }
                        }
                    })
                    .id(("column-view", k))
                    .debug_selector(|| format!("column-{k}"))
                    .role(Role::ListBox)
                    .aria_label(if filtered {
                        format!("{title}, {shown} of {count} items")
                    } else {
                        format!("{title}, {count} items")
                    })
                    .w(width)
                    .flex_none()
                    .h_full()
                    .flex()
                    .flex_col()
                    .py_1()
                    .border_r_1()
                    .border_color(border)
                    .when(k == focused, |c| c.bg(focused_column(&theme)))
                    .relative()
                    .when_some(self.drop_target.as_ref().filter(|t| t.column == k && t.rows.is_none()), |c, t| {
                        let opacity = crate::drag_drop::cue_opacity(cx, &t.folder);
                        c.child(
                            drop_outline(&theme).left(px(4.)).right(px(4.)).top(px(2.)).bottom(px(2.)).opacity(opacity),
                        )
                    })
                    .on_scroll(cx.listener(move |this, _, _, cx| this.on_list_scroll(k, cx)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            this.press_empty(k, event, window, cx)
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            this.context_press_empty(k, event, window, cx)
                        }),
                    )
                    .children(field)
                    .child(body)
                    .children(footer),
            );
        }
        // The right column is the last column: a file's preview or a folder's
        // summary on the Preview tab, the item's metadata on the Info tab. It takes
        // the rest of the strip when the columns leave room, and keeps
        // PREVIEW_COLUMN_WIDTH when they don't (the strip scrolls instead).
        let visible = self.info.read(cx).visible();
        if visible && let Some((entry, items)) = self.preview_parts(cx) {
            let icons = self.icon_resolver(self.focused, cx);
            let tab = self.info.read(cx).tab();
            let label = match tab {
                InfoTab::Preview => self.preview_label(cx).unwrap_or_default(),
                InfoTab::Info => self.info.read(cx).label(cx),
                InfoTab::GitDiff => "Git Diff".into(),
            };
            let body = match tab {
                InfoTab::Preview => {
                    let folder = entry.is_dir && !entry.is_package();
                    let size = if entry.is_dir
                        && cx.global::<Ui>().settings.folder_sizes == crow_core::FolderSizesMode::Visible
                    {
                        cx.try_global::<SizeService>()
                            .and_then(|service| service.0.read(cx).size(&entry.path))
                            .map(Into::into)
                    } else {
                        None
                    };
                    if folder {
                        let name = display_text(&entry.name);
                        let summary = preview::FolderSummary {
                            path: &entry.path,
                            name: &name,
                            modified: entry.modified,
                            items,
                            size,
                        };
                        preview::folder_summary(summary, &theme, &icons).into_any_element()
                    } else {
                        preview::contents(
                            &entry,
                            self.preview(cx),
                            &theme,
                            size,
                            self.cloud_status(&entry.path, cx),
                            &icons,
                        )
                        .into_any_element()
                    }
                }
                InfoTab::Info => self.info.clone().into_any_element(),
                InfoTab::GitDiff => InfoPane::git_diff_body(&self.info, &theme, cx),
            };
            let (preferred_width, splitter_focus) = {
                let info = self.info.read(cx);
                (info.preferred_width, info.splitter_focus().clone())
            };
            strip = strip.child(
                div()
                    .id("preview-column")
                    .debug_selector(|| "preview-column".into())
                    .role(Role::Group)
                    .aria_label(label)
                    .relative()
                    .when_none(&preferred_width, |column| column.flex_1().min_w(px(PREVIEW_COLUMN_WIDTH)))
                    .when_some(preferred_width, |column, width| {
                        column.flex_none().w(width.min(window.viewport_size().width / 2.))
                    })
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(info_pane::tab_strip(&self.info, &theme, cx))
                    .child(crate::window::side_panes::splitter(
                        crate::window::side_panes::Side::Right,
                        &splitter_focus,
                        &theme,
                    ))
                    .child(info_pane::tab_body(&self.info, body, cx)),
            );
        }
        strip
    }
}

/// Where the drag combo's left edge goes: the grab point stays under the cursor, but a
/// combo narrower than that is shifted until its right side just touches the cursor.
fn drag_combo_left(cursor_x: f32, width: f32) -> f32 {
    if cursor_x > width { cursor_x - width } else { 0.0 }
}

/// A chevron drawn as a path: expanding rotates the same glyph instead of swapping in a
/// different one that sits lower in its box.
/// The disclosure chevron, turned `turn` of the way from closed (0, pointing
/// right) to open (1, pointing down); Full motion turns it in between.
fn chevron_glyph(turn: f32, chevron: gpui::Rgba) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let (width, height) = (bounds.size.width, bounds.size.height);
            let origin = bounds.origin;
            let (sin, cos) = (turn * std::f32::consts::FRAC_PI_2).sin_cos();
            // The closed chevron's points, turned clockwise about the center.
            let p = |x: f32, y: f32| {
                let (dx, dy) = (x - 0.5, y - 0.5);
                point(origin.x + width * (0.5 + dx * cos - dy * sin), origin.y + height * (0.5 + dx * sin + dy * cos))
            };
            let mut builder = PathBuilder::stroke(px(2.));
            builder.move_to(p(0.35, 0.2));
            builder.line_to(p(0.65, 0.5));
            builder.line_to(p(0.35, 0.8));
            if let Ok(path) = builder.build() {
                window.paint_path(path, chevron);
            }
        },
    )
    .w(px(14.))
    .h(px(14.))
}

/// The vertical scroll offset the list's rows are drawn at. A trackpad scroll
/// adds to the raw offset even past the ends (a short list cannot scroll at
/// all); `uniform_list` clamps it only when it lays out, so overlays drawn over
/// the rows (the glide fill, ghost rows) clamp it the same way or they drift.
fn shown_offset(scroll: &UniformListScrollHandle) -> f32 {
    let state = scroll.0.borrow();
    let max = f32::from(state.base_handle.max_offset().y).max(0.0);
    f32::from(state.base_handle.offset().y).clamp(-max, 0.0)
}

/// Whether a row draws its own selection fill. While the cursor glides (Full motion,
/// focused list), the cursor row leaves its fill to the gliding one under the rows;
/// every other selected row keeps its own, so a multiple selection still shows.
fn draws_selection_fill(selected: bool, glide: bool, is_cursor: bool) -> bool {
    selected && !(glide && is_cursor)
}

/// Contiguous runs of selected rows in display order: one rounded box per run.
fn selection_islands(rows: usize, selected: impl Fn(usize) -> bool) -> Vec<Range<usize>> {
    let mut islands = Vec::new();
    let mut start = None;
    for ix in 0..rows {
        if selected(ix) {
            start.get_or_insert(ix);
        } else if let Some(from) = start.take() {
            islands.push(from..ix);
        }
    }
    if let Some(from) = start {
        islands.push(from..rows);
    }
    islands
}

/// A selected row's place in its island: the first row rounds its top, the last
/// its bottom, and every row after the first draws the faint rule above it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct IslandEdges {
    pub(crate) top: bool,
    pub(crate) bottom: bool,
    rule_above: bool,
}

/// Per-row island edges for `flags` (`None` for an unselected row).
pub(crate) fn island_edges(flags: &[bool]) -> Vec<Option<IslandEdges>> {
    let mut edges = vec![None; flags.len()];
    for island in selection_islands(flags.len(), |ix| flags[ix]) {
        for ix in island.clone() {
            edges[ix] = Some(IslandEdges {
                top: ix == island.start,
                bottom: ix + 1 == island.end,
                rule_above: ix > island.start,
            });
        }
    }
    edges
}

/// The selected flags for `range`, plus one row on each side so the first and
/// last visible rows know whether their island continues off screen.
fn island_flags(folder: &Folder, range: Range<usize>) -> (usize, Vec<bool>) {
    let first = range.start.saturating_sub(1);
    let last = (range.end + 1).min(folder.row_count());
    let flags =
        (first..last).map(|ix| folder.row(ix).is_some_and(|(entry, _)| folder.is_selected(&entry.path))).collect();
    (first, flags)
}

/// The cursor row's place in its island, for the glide fill's rounding.
fn cursor_island_edges(folder: &Folder) -> Option<IslandEdges> {
    let ix = folder.cursor_ix()?;
    let selected = |ix: usize| folder.row(ix).is_some_and(|(entry, _)| folder.is_selected(&entry.path));
    selected(ix).then(|| IslandEdges {
        top: ix == 0 || !selected(ix - 1),
        bottom: ix + 1 >= folder.row_count() || !selected(ix + 1),
        rule_above: ix > 0 && selected(ix - 1),
    })
}

/// Apply an island's rounding to a row or a fill inside it.
fn with_island_rounding<E: Styled>(element: E, island: Option<IslandEdges>) -> E {
    match island {
        Some(IslandEdges { top: true, bottom: true, .. }) => element.rounded_md(),
        Some(IslandEdges { top: true, .. }) => element.rounded_t_md(),
        Some(IslandEdges { bottom: true, .. }) => element.rounded_b_md(),
        _ => element,
    }
}

/// Fit the same wrapped lines that the tile paints, including space lost at a word break.
fn fit_icon_name(name: &str, width: Pixels, font_size: Pixels, window: &Window) -> crate::style::Truncated {
    let style = window.text_style();
    truncate_middle_parts_to_fit(name, |text| {
        window
            .text_system()
            .shape_text(text.to_owned().into(), font_size, &[style.to_run(text.len())], Some(width), None)
            .is_ok_and(|lines| lines.iter().map(|line| f32::from(line.size(px(1.)).height)).sum::<f32>() <= 2.0)
    })
}

/// The file icon with the trash glyph drawn over it while a row is swallowed.
fn row_icon(
    entry: &Entry,
    look: Look,
    theme: &Theme,
    dim: f32,
    cloud: Option<CloudStatus>,
    icons: &crate::icon_packs::Resolver,
) -> Div {
    let packed = (!crate::style::native_cloud_icon(cloud, &entry.path))
        .then(|| icons.resolve(&entry.path, entry.is_dir, entry.is_package()))
        .flatten();
    let content = match packed {
        Some(icon) => crate::icons::pack_glyph(&icon.asset, 16., color(icon.color.rgb(theme))).into_any_element(),
        None => div()
            .children(cloud_icon(cloud, &entry.path, entry.is_dir, 16).map(|i| img(i).size(px(16.))))
            .into_any_element(),
    };
    div().relative().size(px(16.)).flex_none().child(div().opacity((1.0 - look.trash) * dim).child(content)).when(
        look.trash > 0.0,
        |slot| {
            slot.child(
                div().absolute().inset_0().flex().items_center().justify_center().opacity(look.trash).child(glyph(
                    Glyph::Trash,
                    look.trash_size,
                    "fg-muted",
                    theme,
                )),
            )
        },
    )
}

/// A row drawn over the list while its swallow finishes after the listing dropped it.
fn ghost_row(entry: &Entry, look: Look, theme: &Theme, font_size: Pixels, icons: &crate::icon_packs::Resolver) -> Div {
    div()
        .h(px(ROW_HEIGHT))
        .w_full()
        .flex()
        .items_center()
        .gap_2()
        .px(px(CELL_PADDING))
        .opacity(look.opacity)
        .child(row_icon(
            entry,
            look,
            theme,
            content_opacity(entry.hidden, false),
            entry.dataless.then_some(CloudStatus::NotDownloaded),
            icons,
        ))
        .child(
            div()
                .relative()
                .left(px(look.dx))
                .opacity(content_opacity(entry.hidden, false))
                .text_size(font_size)
                .whitespace_nowrap()
                .child(display_text(&entry.name).into_owned()),
        )
}

/// Ghost rows for effects whose entry left the listing before the swallow ended.
struct Ghosts<'a> {
    folder: &'a Folder,
    mode: Effective,
    now: Instant,
    scroll: &'a UniformListScrollHandle,
    theme: &'a Theme,
    font_size: Pixels,
    icons: &'a crate::icon_packs::Resolver,
    side_inset: f32,
    top_inset: f32,
}

impl Ghosts<'_> {
    fn render(&self) -> Vec<AnyElement> {
        // Off changes state in one frame. Low fades the removed row in place;
        // Full plays the swallow here while the rows below wait to close.
        if self.mode == Effective::Off {
            return Vec::new();
        }
        let offset = shown_offset(self.scroll);
        self.folder
            .effects()
            .iter()
            .filter(|effect| effect.fx.leaving() && effect.removed && effect.entering(self.now))
            .map(|effect| {
                let look = effect.look(self.mode, self.now);
                let top = effect.row_index as f32 * ROW_HEIGHT + offset + self.top_inset;
                div()
                    .absolute()
                    .left(px(self.side_inset))
                    .right(px(self.side_inset))
                    .top(px(top))
                    .h(px(ROW_HEIGHT))
                    .overflow_hidden()
                    .debug_selector(|| format!("ghost-{}", effect.entry.name))
                    .child(ghost_row(&effect.entry, look, self.theme, self.font_size, self.icons))
                    .into_any_element()
            })
            .collect()
    }
}

/// The name column plus the shown columns, at their default widths.
fn default_table(shown: &[Arc<dyn Column>]) -> TableLayout {
    let mut columns = vec![ColumnWidth { id: "name".into(), width: 220.0, min_width: 140.0 }];
    columns.extend(shown.iter().map(|c| ColumnWidth {
        id: c.id().to_owned(),
        width: c.width(),
        min_width: c.min_width(),
    }));
    TableLayout::new(columns)
}

impl Pane {
    /// The in-pane state for a folder macOS will not list, shared by every view
    /// mode. Offers Retry, and Full Disk Access when privacy protection is the cause.
    fn render_unreadable(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use crate::folder::FolderErrorKind;
        if self.active_saved.is_some() || self.active_tag.is_some() {
            return None;
        }
        let root = self.folders.first()?.clone();
        let tcc_likely = match root.read(cx).error_kind()? {
            FolderErrorKind::PermissionDenied { tcc_likely } => tcc_likely,
            FolderErrorKind::Other => return None,
        };
        let theme = cx.global::<Ui>().theme.clone();
        let [retry_focus, access_focus] = self.unreadable.clone();
        let button = |id: &'static str, label: &'static str, focus: &FocusHandle, primary: bool| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .role(Role::Button)
                .aria_label(label)
                .track_focus(focus)
                .tab_stop(true)
                .focus_visible(|style| style.shadow(crate::style::cursor_ring(&theme)))
                .px_3()
                .py_1()
                .rounded_md()
                .bg(color(if primary { theme.role("accent") } else { theme.role("surface-raised") }))
                .text_color(color(if primary { theme.role("on-accent") } else { theme.role("fg") }))
                .child(label)
        };
        let retry = {
            let root = root.clone();
            let again = move |cx: &mut gpui::App| root.update(cx, |folder, cx| folder.reload(cx));
            let on_key = again.clone();
            button("folder-retry", "Retry", &retry_focus, !tcc_likely).on_click(move |_, _, cx| again(cx)).on_key_down(
                move |event, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        on_key(cx);
                        cx.stop_propagation();
                    }
                },
            )
        };
        let access = tcc_likely.then(|| {
            button("folder-full-disk-access", "Open Full Disk Access…", &access_focus, true)
                .on_click(|_, _, cx| cx.open_url(crate::trash::PRIVACY_SETTINGS_URL))
                .on_key_down(|event, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        cx.open_url(crate::trash::PRIVACY_SETTINGS_URL);
                        cx.stop_propagation();
                    }
                })
        });
        Some(
            div()
                .id("folder-unreadable")
                .debug_selector(|| "folder-unreadable".into())
                .role(Role::Group)
                .aria_label("You don't have permission to view this folder.")
                .track_focus(&self.focus)
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .bg(color(theme.role("bg")))
                .text_color(color(theme.role("fg-muted")))
                .child(glyph(Glyph::Lock, 28., "fg-muted", &theme))
                .child("You don't have permission to view this folder.")
                .child(div().flex().gap_2().child(retry).children(access))
                .into_any_element(),
        )
    }
}

impl Render for Pane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(message) = self.saved_unavailable(cx) {
            let theme = cx.global::<Ui>().theme.clone();
            return div()
                .id("saved-query-unavailable")
                .track_focus(&self.focus)
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .bg(color(theme.role("bg")))
                .text_color(color(theme.role("fg-muted")))
                .child(message)
                .into_any_element();
        }
        if let Some(state) = self.render_unreadable(cx) {
            return state;
        }
        if let Some(view) = self.saved_session() {
            return div().size_full().child(view).into_any_element();
        }
        let plugin_generation =
            cx.try_global::<crate::plugins::Plugins>().map_or(0, |plugins| plugins.surfaces.generation());
        if self.applied_view.as_ref().is_some_and(|(_, applied, _)| *applied != plugin_generation) {
            self.apply_view(cx);
        }
        let views_generation = cx.global::<Ui>().views_generation;
        if self.preview_plugin_generation != plugin_generation || self.preview_views_generation != views_generation {
            self.preview_plugin_generation = plugin_generation;
            self.preview_views_generation = views_generation;
            // A registration or provider-list change refreshes the visible preview
            // instead of leaving a withdrawn provider's output on screen.
            self.clear_preview(cx);
            self.preview_key = None;
            self.load_preview(cx);
            gallery::invalidate(self, cx);
        }
        if self.marquee.is_some() && !self.focus.is_focused(window) {
            self.cancel_marquee(cx);
        }
        self.request_plugin_status(cx);
        self.update_plugin_command_context(window, cx);
        self.bench_step(window, cx);
        // Row effects and disclosures are drawn from the clock: keep frames coming
        // while one runs (GPUI only redraws on its own for its animation elements).
        let frame_now = Instant::now();
        if self.folders.iter().any(|folder| folder.read(cx).animating(frame_now)) {
            window.request_animation_frame();
        }
        // A drag inside the list highlights the row under the pointer from this state; the
        // style-based drag-over highlight flickers on the platform drag path used when a
        // drag leaves the window and comes back.
        self.drop_row = if cx.has_active_drag() && self.mode == ViewMode::List {
            self.drag_row_at(cx, window.mouse_position())
        } else {
            None
        };
        self.drop_target = if cx.has_active_drag() {
            self.drop_target_at(window.mouse_position(), crate::drag_drop::active_paths(cx), cx)
        } else {
            None
        };
        self.drop_column = if cx.has_active_drag() && self.mode == ViewMode::Columns {
            self.column_row_at(cx, window.mouse_position())
        } else {
            None
        };
        // The row must be built, not just the entry: a rename whose row is not in the
        // list yet has no field to focus (an apply builds its rows off the main thread).
        let pending_rename = self.rename_when_present.clone().and_then(|path| {
            let folder = self.focused_folder().read(cx);
            let ix = folder.row_of_path(&path)?;
            folder.row(ix).map(|(entry, _)| (path, entry.is_dir))
        });
        if let Some((path, is_dir)) = pending_rename {
            self.rename_when_present = None;
            // Focus can't move during a render; start the rename right after it.
            cx.defer_in(window, move |this, window, cx| this.begin_rename(path, is_dir, window, cx));
        }
        let (title, total, shown, filtered) = {
            let focused = self.focused_folder().read(cx);
            let title = focused
                .dir()
                .file_name()
                .map_or_else(|| "Macintosh HD".to_owned(), |n| n.to_string_lossy().into_owned());
            (title, focused.entries().len(), focused.row_count(), focused.filter().is_some())
        };
        let filtering = self.filtering.is_some();
        let (mode, font_size, theme) = {
            let ui = cx.global::<Ui>();
            (ui.feedback.mode, px(ui.settings.font_size), ui.theme.clone())
        };
        let now = Instant::now();
        let focused_id = self.focused_folder().entity_id();
        if self.view_motion.as_ref().is_some_and(|motion| {
            motion.folder == focused_id
                && self.renaming.is_none()
                && !cx.has_active_drag()
                && motion.progress(mode, now).is_some()
        }) {
            window.request_animation_frame();
        } else {
            self.view_motion = None;
        }
        if self.child_reveal.is_some_and(|(id, started)| {
            self.index_of(id).is_some()
                && motion::navigation_progress(mode, started, now, motion::ms::CHILD_COLUMN).is_some()
        }) {
            window.request_animation_frame();
        } else {
            self.child_reveal = None;
        }
        self.presented_layout = Some((self.mode, focused_id));
        if cx
            .global::<Operations>()
            .clipboard
            .as_ref()
            .and_then(|clipboard| clipboard.flash)
            .is_some_and(|started| crow_core::motion::copy_flash(mode, started, now) > 0.0)
        {
            window.request_animation_frame();
        }
        let glide = mode == Effective::Full && self.view_motion.is_none() && self.focus.is_focused(window);
        // The field is inside the list now: while it has focus, the list's actions must be
        // off the focus path so AppKit's menu equivalents let those keys through.
        let filter_focused = self
            .filtering
            .as_ref()
            .is_some_and(|filtering| filtering.input.read(cx).focus_handle(cx).is_focused(window));
        let mut context = match self.mode {
            ViewMode::List | ViewMode::Icons | ViewMode::Gallery => "FileList".to_owned(),
            ViewMode::Columns => "FileList ColumnView".to_owned(),
        };
        if cx.global::<Ui>().settings.keymap == KeymapLayer::Vim {
            // While a mark waits for its letter, the Vim bindings step aside so the letter arrives as typed.
            context.push_str(if self.mark_pending.is_some() { " VimPending" } else { " vim" });
        }
        if filtering {
            context.push_str(" Filtered");
        }
        let body = match self.mode {
            ViewMode::List => {
                let (rows, scroll) = {
                    let root = self.folders[0].read(cx);
                    (root.row_count(), root.scroll().clone())
                };
                let field = self.render_filter_field(FilterPlace::List, window, cx);
                let no_matches = self.render_no_matches(cx);
                let footer = self.render_filter_footer(FilterPlace::List, cx);
                let widths = self.table.fitted(f32::from(self.content_width) - 2.0 * LIST_SIDE_INSET);
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(self.render_table_header(&widths, cx))
                    .children(field)
                    .child({
                        let bounds = self.list_bounds.clone();
                        let body = match no_matches {
                            Some(block) => block,
                            None => {
                                let ghosts = {
                                    let folder = self.folders[0].read(cx);
                                    let icons = self.icon_resolver(0, cx);
                                    Ghosts {
                                        folder,
                                        mode,
                                        now,
                                        scroll: &scroll,
                                        theme: &theme,
                                        font_size,
                                        icons: &icons,
                                        side_inset: 8.0,
                                        top_inset: LIST_TOP_INSET,
                                    }
                                    .render()
                                };
                                let glide_fill = if glide {
                                    let folder = self.folders[0].read(cx);
                                    let edges = cursor_island_edges(folder);
                                    folder.cursor_ix().map(|ix| {
                                        let offset = shown_offset(&scroll);
                                        let y = ix as f32 * ROW_HEIGHT;
                                        let fill = div()
                                            .absolute()
                                            .left(px(8.))
                                            .right(px(8.))
                                            .h(px(ROW_HEIGHT))
                                            .bg(active_selection(&theme));
                                        with_island_rounding(fill, edges).debug_selector(|| "glide".into()).with_spring(
                                            "glide",
                                            SpringAnimation::new(SpringConfig::new(
                                                2500.0,
                                                2.0 * 2500.0f32.sqrt(),
                                                1.0,
                                            ))
                                            .to(px(y)),
                                            move |el, y| el.top(px(f32::from(y) + offset + LIST_TOP_INSET)),
                                        )
                                    })
                                } else {
                                    None
                                };
                                let drop_outline = self.drop_target.as_ref().map(|t| {
                                    let outline = drop_outline(&theme)
                                        .left(px(8.))
                                        .right(px(8.))
                                        .opacity(crate::drag_drop::cue_opacity(cx, &t.folder));
                                    match &t.rows {
                                        Some(rows) => {
                                            let offset = shown_offset(&scroll);
                                            let top = rows.start as f32 * ROW_HEIGHT + offset + LIST_TOP_INSET;
                                            outline.top(px(top)).h(px(rows.len() as f32 * ROW_HEIGHT))
                                        }
                                        None => outline.top(px(LIST_TOP_INSET / 2.)).bottom(px(LIST_TOP_INSET / 2.)),
                                    }
                                });
                                div()
                                    .relative()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .overflow_hidden()
                                    .when(self.tip_inset, |body| body.pb(px(crate::tips::INSET)))
                                    .children(glide_fill)
                                    .child({
                                        let render_rows = cx.processor(move |this, range, window, cx| {
                                            this.render_table_rows(&widths, range, window, cx)
                                        });
                                        let mut list = uniform_list("entries", rows, render_rows)
                                            .track_scroll(&scroll)
                                            .pt(px(LIST_TOP_INSET))
                                            .flex_1();
                                        if let Some(plugin_rows) = self.plugin_row_decoration(0, cx) {
                                            list = list.with_decoration(plugin_rows);
                                        }
                                        list
                                    })
                                    .children(ghosts)
                                    .children(drop_outline)
                                    .into_any_element()
                            }
                        };
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .on_children_prepainted(move |children, _, _| {
                                bounds.set(children.first().copied());
                            })
                            // The list's own scroll handler runs first (deeper hitbox), so
                            // the offset read here is this event's.
                            .on_scroll(cx.listener(|this, _, _, cx| this.on_list_scroll(0, cx)))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                                    this.press_empty(0, event, window, cx)
                                }),
                            )
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                                    this.context_press_empty(0, event, window, cx)
                                }),
                            )
                            .child(body)
                    })
                    .children(footer)
                    .into_any_element()
            }
            ViewMode::Icons => self.render_icons(window, cx).into_any_element(),
            ViewMode::Gallery => gallery::render(self, window, cx).into_any_element(),
            ViewMode::Columns => self.render_columns(window, cx).into_any_element(),
        };
        let list = div()
            .id("file-list")
            .debug_selector(|| "file-list".into())
            // In column view the columns are the lists; the focusable wrapper
            // must not repeat the active column's folder name and count.
            .role(if matches!(self.mode, ViewMode::Columns | ViewMode::Gallery) { Role::Group } else { Role::ListBox })
            .aria_label(if self.mode == ViewMode::Columns {
                "Columns".to_owned()
            } else if self.mode == ViewMode::Gallery {
                if filtered {
                    format!("Gallery, {title}, {shown} of {total} items")
                } else {
                    format!("Gallery, {title}, {total} items")
                }
            } else if filtered {
                format!("{title}, {shown} of {total} items")
            } else {
                format!("{title}, {total} items")
            })
            // keys::context_stack (KeyScope::List/Columns) mirrors this.
            .key_context(context.as_str())
            .track_focus(&self.focus)
            // While a name is being edited, every list shortcut must be off the focus path so
            // AppKit passes plain keys and its menu equivalents to the text field. The text
            // field supplies its own Cut/Copy/Paste/Select All listeners.
            .when(self.renaming.is_none() && !filter_focused, |list| {
                list.on_action(cx.listener(Self::move_up))
                    .on_action(cx.listener(Self::move_down))
                    .on_action(cx.listener(Self::filter_focus_field))
                    .on_action(cx.listener(Self::extend_up))
                    .on_action(cx.listener(Self::extend_down))
                    .on_action(cx.listener(Self::move_to_first))
                    .on_action(cx.listener(Self::move_to_last))
                    .on_action(cx.listener(Self::select_all))
                    .on_action(cx.listener(Self::move_left))
                    .on_action(cx.listener(Self::move_right))
                    .on_action(cx.listener(Self::open))
                    .on_action(cx.listener(Self::enter_row))
                    .on_action(cx.listener(Self::rename))
                    .on_action(cx.listener(Self::quick_look))
                    .on_action(cx.listener(Self::new_folder))
                    .on_action(cx.listener(Self::new_file))
                    .on_action(cx.listener(Self::duplicate))
                    .on_action(cx.listener(Self::move_to_trash))
                    .on_action(cx.listener(Self::delete_permanently))
                    .on_action(cx.listener(Self::copy_files))
                    .on_action(cx.listener(Self::cut_files))
                    .on_action(cx.listener(Self::paste_files))
                    .on_action(cx.listener(Self::paste_move_files))
                    .on_action(cx.listener(Self::copy_path))
                    .on_action(cx.listener(Self::show_in_finder))
                    .on_action(cx.listener(Self::begin_mark))
                    .on_action(cx.listener(Self::begin_jump))
                    .on_action(cx.listener(Self::start_filter))
                    .on_action(cx.listener(Self::clear_filter))
            })
            // The tag control lives inside the open filter field, so this must be
            // active while the field has focus (unlike the list shortcuts above).
            .on_action(cx.listener(Self::choose_tags))
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if cx.has_active_drag() {
                    this.update_drag_row(event.position, cx);
                }
                if this.marquee.is_some() {
                    this.update_marquee(event.position, cx);
                }
                if let Some(drag) = this.resize {
                    if event.pressed_button == Some(MouseButton::Left) {
                        this.table.drag_to(&drag, f32::from(event.position.x));
                        cx.notify();
                    } else {
                        this.resize = None;
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.resize = None;
                    this.commit_marquee(cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.resize = None;
                    this.cancel_marquee(cx);
                }),
            )
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .w_full()
            // Wider-than-pane tables are clipped for now; horizontal scrolling comes with view settings.
            .overflow_hidden()
            .child(body);
        if self.focus_list_after_filter {
            self.focus_list_after_filter = false;
            cx.defer_in(window, |this, window, cx| window.focus(&this.focus, cx));
        }
        div().size_full().flex().flex_col().child(list).into_any_element()
    }
}

#[cfg(any(test, feature = "fuzz"))]
impl Pane {
    pub fn rename_text(&self, cx: &gpui::App) -> Option<String> {
        self.renaming.as_ref().map(|r| r.input.read(cx).text().to_owned())
    }

    /// The filter and rename fields' focus handles, when open. Both can exist at
    /// once (create a folder while a filter is showing), and either may hold focus.
    pub fn text_focus_handles(&self, cx: &gpui::App) -> Vec<gpui::FocusHandle> {
        let mut handles = Vec::new();
        if let Some(filtering) = &self.filtering {
            handles.push(filtering.input.read(cx).focus_handle(cx));
        }
        if let Some(renaming) = &self.renaming {
            handles.push(renaming.input.read(cx).focus_handle(cx));
        }
        handles
    }

    /// Bounds for a generated row selector, from the pane's own layout state.
    /// `debug_bounds` needs a `&'static str`; row names are dynamic.
    pub fn debug_selector_bounds(&self, selector: &str, cx: &gpui::App) -> Option<gpui::Bounds<gpui::Pixels>> {
        if let Some(name) = selector.strip_prefix("tile-") {
            let bounds = self.icon_bounds.get()?;
            let grid = self.grid?;
            let folder = self.focused_folder().read(cx);
            let ix = (0..folder.row_count()).find(|ix| folder.row(*ix).is_some_and(|(entry, _)| entry.name == name))?;
            let rect = grid.tile_rect(ix)?;
            let offset = f32::from(self.icon_scroll.0.borrow().base_handle.offset().y);
            return Some(gpui::Bounds::new(
                point(bounds.left() + px(rect.x), bounds.top() + px(rect.y) + px(offset)),
                gpui::size(px(rect.width), px(rect.height)),
            ));
        }
        if let Some(name) = selector.strip_prefix("gallery-row-") {
            let bounds = self.gallery.bounds.get()?;
            let folder = self.focused_folder().read(cx);
            let ix = (0..folder.row_count()).find(|ix| folder.row(*ix).is_some_and(|(entry, _)| entry.name == name))?;
            let top = bounds.top() + px(gallery::ROW_HEIGHT * ix as f32) + px(shown_offset(&self.gallery.scroll));
            return Some(gpui::Bounds::new(
                point(bounds.left() + px(LIST_SIDE_INSET), top),
                gpui::size((bounds.size.width - px(2. * LIST_SIDE_INSET)).max(px(0.)), px(gallery::ROW_HEIGHT)),
            ));
        }
        if let Some(name) = selector.strip_prefix("row-") {
            let bounds = self.list_bounds.get()?;
            let folder = self.focused_folder().read(cx);
            let ix = (0..folder.row_count()).find(|ix| folder.row(*ix).is_some_and(|(entry, _)| entry.name == name))?;
            let offset = folder.scroll().0.borrow().base_handle.offset().y;
            let top = bounds.top() + px(LIST_TOP_INSET) + offset + px(ROW_HEIGHT * ix as f32);
            return Some(gpui::Bounds::new(point(bounds.left(), top), gpui::size(bounds.size.width, px(ROW_HEIGHT))));
        }
        let rest = selector.strip_prefix("column-")?;
        let (k, name) = rest.split_once("-row-")?;
        let k: usize = k.parse().ok()?;
        let column = *self.column_bounds.borrow().get(&k)?;
        let folder = self.folders.get(k)?.read(cx);
        let ix = (0..folder.row_count()).find(|ix| folder.row(*ix).is_some_and(|(entry, _)| entry.name == name))?;
        let offset = folder.scroll().0.borrow().base_handle.offset().y;
        let top = column.top() + px(ROW_HEIGHT * ix as f32) + offset;
        Some(gpui::Bounds::new(
            point(column.left() + px(4.), top),
            gpui::size(column.size.width - px(8.), px(ROW_HEIGHT)),
        ))
    }

    #[cfg(test)]
    pub fn gallery_sample_summary(&self) -> Option<(usize, bool)> {
        self.gallery.sample_summary()
    }

    /// Every shown folder's cursor and selection must name a listed row.
    pub fn invariant_violations(&self, cx: &gpui::App) -> Vec<crate::fuzz::Violation> {
        use crate::fuzz::Violation;
        let mut out = Vec::new();
        for (k, folder) in self.folders.iter().enumerate() {
            let folder = folder.read(cx);
            if folder.is_settling() {
                continue;
            }
            if let Some(ix) = folder.cursor_ix()
                && ix >= folder.row_count()
            {
                out.push(Violation::new(
                    "cursor",
                    format!("pane {k}: cursor {ix} is past {} rows", folder.row_count()),
                ));
            }
            // Enter (Rename) and every other row action use the cursor row, and the
            // row draws as selected: the two must name the same entry. A highlight
            // that moves without the cursor would act on a row the user is not on.
            if let Some(entry) = folder.cursor_entry()
                && !folder.is_selected(&entry.path)
            {
                out.push(Violation::new(
                    "cursor",
                    format!(
                        "pane {k}: cursor {} (row {:?}) is not the selected row; selection cursor {:?}, {} rows, {} selected, dir {}, gen {} rows_gen {} loading {}",
                        entry.path.display(),
                        folder.cursor_ix(),
                        folder.selection_cursor_path(),
                        folder.row_count(),
                        folder.selected_count(),
                        folder.dir().display(),
                        folder.generation_for_test(),
                        folder.rows_generation_for_test(),
                        folder.is_loading()
                    ),
                ));
            }
            let selection = folder.selection_paths();
            if selection.len() > 16 {
                // A select-all on 100k rows: one path set, not 100k scans.
                let rows = folder.row_paths();
                let listed: std::collections::HashSet<&std::path::Path> =
                    rows.iter().map(|path| path.as_path()).collect();
                for path in selection {
                    if !listed.contains(path.as_path()) {
                        out.push(Violation::new(
                            "selection",
                            format!("pane {k}: {} is selected but not in the listing", path.display()),
                        ));
                    }
                }
            } else {
                for path in selection {
                    if !folder.has_row_path(&path) {
                        out.push(Violation::new(
                            "selection",
                            format!("pane {k}: {} is selected but not in the listing", path.display()),
                        ));
                    }
                }
            }
        }
        out
    }

    /// Every shown folder's listing, without touching the disk. The fuzz runner
    /// compares it to `read_dir` on a background thread; `listing_violations`
    /// does the same comparison on the calling thread for the tests.
    pub fn listing_snapshot(
        &self,
        cx: &gpui::App,
        root: Option<&std::path::Path>,
    ) -> Vec<(PathBuf, Vec<String>, bool)> {
        let hidden = cx.global::<Ui>().settings.show_hidden_files;
        let mut out = Vec::new();
        for folder in &self.folders {
            let folder = folder.read(cx);
            if folder.is_loading() || folder.error().is_some() {
                continue;
            }
            let dir = folder.dir();
            if root.is_some_and(|root| !dir.starts_with(root)) {
                continue;
            }
            let listed = folder.entries().iter().map(|entry| entry.name.clone()).collect();
            out.push((dir.to_path_buf(), listed, hidden));
        }
        out
    }

    /// Every shown folder's listing must equal `read_dir`, honoring hidden files.
    /// The name filter is a view over `entries` and has its own tests. `root`
    /// limits the check to the fuzzer's temp tree.
    pub fn listing_violations(&self, cx: &gpui::App, root: Option<&std::path::Path>) -> Vec<crate::fuzz::Violation> {
        use crate::fuzz::Violation;
        let mut out = Vec::new();
        for (k, (dir, mut listed, hidden)) in self.listing_snapshot(cx, root).into_iter().enumerate() {
            // io-ok: fuzz invariant, test/fuzz builds only (docs/testing.md calls this main-thread work deliberate)
            let Ok(read) = std::fs::read_dir(&dir) else { continue };
            let mut disk: Vec<String> = read
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| hidden || !name.starts_with('.'))
                .collect();
            disk.sort();
            listed.sort();
            if disk != listed {
                out.push(Violation::new(
                    "listing",
                    format!("pane {k} {}: disk {disk:?}, listing {listed:?}", dir.display()),
                ));
            }
        }
        out
    }

    pub fn filter_text(&self, cx: &gpui::App) -> Option<String> {
        self.filter_query(cx)
    }

    pub fn row_matches(&self, cx: &gpui::App) -> Vec<Option<Range<usize>>> {
        let folder = self.focused_folder().read(cx);
        (0..folder.row_count()).map(|ix| folder.row_match(ix)).collect()
    }

    #[cfg(test)]
    pub fn folder_count(&self) -> usize {
        self.folders.len()
    }

    /// The focused folder entity, for tests that drive the watcher directly.
    #[cfg(test)]
    pub(crate) fn test_folder(&self) -> Entity<Folder> {
        self.focused_folder().clone()
    }

    pub fn focused_index(&self) -> usize {
        self.focused
    }

    pub fn folder_dir(&self, k: usize, cx: &gpui::App) -> Option<PathBuf> {
        self.folders.get(k).map(|f| f.read(cx).dir().to_path_buf())
    }

    pub fn cursor_name_in(&self, k: usize, cx: &gpui::App) -> Option<String> {
        self.folders.get(k).and_then(|f| f.read(cx).cursor_entry().map(|e| e.name.clone()))
    }

    #[cfg(test)]
    pub fn names(&self, cx: &gpui::App) -> Vec<String> {
        self.focused_folder().read(cx).names()
    }

    #[cfg(test)]
    pub fn selected_names(&self, cx: &gpui::App) -> Vec<String> {
        self.focused_folder().read(cx).selected_names()
    }
}

#[cfg(test)]
mod tests {
    use super::{IslandEdges, drag_combo_left, draws_selection_fill, island_edges, selection_islands};

    #[gpui::test]
    fn icon_names_fit_two_real_lines_and_keep_the_extension(cx: &mut gpui::TestAppContext) {
        cx.add_empty_window().update(|window, _| {
            let name = super::fit_icon_name(
                "Quarterly financial projections (revised).pdf",
                gpui::px(92.),
                gpui::px(13.),
                window,
            );
            assert!(name.cut.is_some());
            assert!(name.text.starts_with("Quarterly") && name.text.ends_with(".pdf"), "{}", name.text);
        });
    }

    #[test]
    fn selection_islands_are_contiguous_runs() {
        let flags = [false, true, true, false, true, false, true, true, true];
        assert_eq!(selection_islands(flags.len(), |ix| flags[ix]), vec![1..3, 4..5, 6..9]);
        assert!(selection_islands(0, |_| true).is_empty());
        assert_eq!(selection_islands(3, |_| true), vec![0..3]);
        assert!(selection_islands(2, |_| false).is_empty());
    }

    #[test]
    fn island_edges_round_the_ends_and_rule_the_middle() {
        let edges = island_edges(&[false, true, true, true, false, true]);
        assert_eq!(edges[0], None, "an unselected row has no island");
        assert_eq!(edges[1], Some(IslandEdges { top: true, bottom: false, rule_above: false }));
        assert_eq!(edges[2], Some(IslandEdges { top: false, bottom: false, rule_above: true }));
        assert_eq!(edges[3], Some(IslandEdges { top: false, bottom: true, rule_above: true }));
        assert_eq!(edges[4], None);
        assert_eq!(edges[5], Some(IslandEdges { top: true, bottom: true, rule_above: false }));
    }

    #[test]
    fn gliding_hands_only_the_cursor_rows_fill_to_the_glide() {
        assert!(!draws_selection_fill(true, true, true), "the cursor row: the glide draws its fill");
        assert!(draws_selection_fill(true, true, false), "other selected rows keep theirs (⌘A in Full)");
        assert!(draws_selection_fill(true, false, true), "no glide: the cursor row fills itself");
        assert!(!draws_selection_fill(false, true, false), "unselected rows draw none");
    }

    #[test]
    fn drag_combo_touches_the_cursor_only_when_it_has_to() {
        assert_eq!(drag_combo_left(200.0, 80.0), 120.0, "right edge lands on the cursor");
        assert_eq!(drag_combo_left(80.0, 80.0), 0.0, "cursor at the right edge needs no shift");
        assert_eq!(drag_combo_left(30.0, 80.0), 0.0, "inside the combo: the grab point tracks");
    }
}
