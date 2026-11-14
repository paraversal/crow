//! A tab's contents: history, view mode and the open folders. List view shows
//! one folder as a table; column view shows a chain of folders, one per column.
//!
//! Column view focus rules (from the spec):
//! - exactly one column is focused; its selection uses the accent color, others are muted;
//! - a column that only shows the children of a selection is never focused until
//!   you move into it (Right arrow or a click);
//! - typing and filtering always act on the focused column.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use crow_core::motion::{Effective, Fx, Look};
use crow_core::{
    Align, Column, ColumnContext, ColumnWidth, History, KeymapLayer, ResizeDrag, TIP_SCROLL_ENTRIES, TIP_SCROLL_ROWS,
    TableLayout, TipId, ViewChange, ViewMode,
};
use crow_fs::{Entry, Sort, SortKey};
use crow_theme::Theme;
use gpui::{
    Action, AnimationExt, AnyElement, Bounds, ClickEvent, Context, Div, Entity, EntityId, EventEmitter,
    ExternalDragPayload, ExternalPaths, FileDragPaths, FocusHandle, Focusable, Image, IntoElement, KeyDownEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point, Render, Role, ScrollHandle,
    SharedString, SpringAnimation, SpringConfig, Subscription, UniformListScrollHandle, Window, canvas, div, img,
    point, prelude::*, px, uniform_list,
};

use crate::actions::{
    BeginJump, BeginMark, ClearFilter, CopyFiles, CutFiles, DeletePermanently, Duplicate, ExtendDown, ExtendUp,
    MoveDown, MoveLeft, MoveRight, MoveToFirst, MoveToLast, MoveToTrash, MoveUp, NewFile, NewFolder, OpenSelection,
    PasteFiles, PasteMoveFiles, QuickLook, Rename, SelectAll, StartFilter,
};
use crate::extensions::Ext;
use crate::folder::{Folder, FolderEvent};
use crate::icons::{Glyph, glyph};
use crate::palette::CommandId;
use crate::preview::{self, Preview};
use crate::style::{
    active_selection, color, color_alpha, content_opacity, drop_target, effect_tint, format_time, highlight_text, icon,
    inactive_selection, kind, map_range, truncate_middle, truncate_middle_parts,
};
use crate::text_input::{TextInput, TextInputEvent};
use crate::ui::Ui;
use crow_fs::ops::Request;

mod pane_filter;
use pane_filter::FilterPlace;

pub(crate) const ROW_HEIGHT: f32 = 24.0;

/// The first row a uniform list shows. Its scroll offset is negative once
/// scrolled down (`drag_row_at` adds it to a pointer position).
fn first_visible_row(handle: &UniformListScrollHandle) -> usize {
    let offset = f32::from(handle.0.borrow().base_handle.offset().y);
    (-offset / ROW_HEIGHT).max(0.).floor() as usize
}

/// Space between the list header's line and the first row, so a highlighted first row's
/// rounded corners don't meet the straight line and leave dark wedges at its ends.
pub(crate) const LIST_TOP_INSET: f32 = 4.0;
/// Left padding before a row's disclosure chevron, and the indent per tree level.
const NAME_INSET: f32 = 4.0;
const INDENT: f32 = 18.0;
const TRIANGLE_WIDTH: f32 = 20.0;
const CELL_PADDING: f32 = 8.0;
/// Column view: every column starts this wide; one with long names widens up to the cap.
const COLUMN_WIDTH: f32 = 220.0;
const COLUMN_WIDTH_CAP: f32 = 360.0;
const PREVIEW_COLUMN_WIDTH: f32 = 300.0;

/// Requests the pane can't carry out alone: operations may need a sheet (conflicts,
/// confirmations) and the clipboard is app-wide, so the window handles these.
pub enum PaneEvent {
    Run(Request),
    Copy {
        paths: Vec<PathBuf>,
        cut: bool,
    },
    Paste {
        into: PathBuf,
        move_items: bool,
    },
    /// The tab went to a folder (for Recent).
    Entered(PathBuf),
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

impl EventEmitter<PaneEvent> for Pane {}

/// Files dragged from inside Crow: paths and whether each is a folder. The selection is
/// shared, so rendering a selected row does not clone the whole selection.
#[derive(Clone)]
pub(crate) struct DraggedSelection {
    pub entries: Arc<Vec<(PathBuf, bool)>>,
}

impl DraggedSelection {
    /// What the platform and `ExternalPaths` drop handlers expect.
    pub fn external(&self) -> ExternalPaths {
        ExternalPaths(self.entries.iter().map(|(path, _)| path.clone()).collect::<Vec<_>>().into())
    }
}

/// Handed from the drag constructor to the native-promotion resolver: the measured
/// combo and the grab point, so the native image starts in exactly the same place.
#[derive(Clone)]
struct DragHandoff {
    cursor: Rc<Cell<Point<Pixels>>>,
    width: Rc<Cell<f32>>,
    pill_width: Rc<Cell<f32>>,
    label: Rc<RefCell<SharedString>>,
}

impl DragHandoff {
    fn new() -> Self {
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
struct DraggedFiles {
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

pub struct Pane {
    history: History,
    mode: ViewMode,
    show_hidden: bool,
    sort: Sort,
    /// List view: just the folder shown. Column view: the chain from the root rightwards.
    folders: Vec<Entity<Folder>>,
    subscriptions: Vec<Subscription>,
    /// Which folder has keyboard focus (always 0 in list view).
    focused: usize,
    focus: FocusHandle,
    /// The list view's columns after Name, and the widths shared by header and rows.
    columns: Vec<Arc<dyn Column>>,
    table: TableLayout,
    resize: Option<ResizeDrag>,
    preview: Option<Preview>,
    strip_scroll: ScrollHandle,
    status: Option<SharedString>,
    renaming: Option<Renaming>,
    /// Start renaming this item once it appears (a new folder).
    rename_when_present: Option<PathBuf>,
    bench: Option<Bench>,
    /// The `views.toml` reload and folder whose view was applied last.
    applied_view: Option<(u64, PathBuf)>,
    /// List view: the uniform list's viewport, captured at prepaint.
    list_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The list row a drag would drop on, drawn from state rather than drag-over styles.
    drop_row: Option<usize>,
    /// Column view: each column's list viewport, captured at prepaint.
    column_bounds: Rc<RefCell<HashMap<usize, Bounds<Pixels>>>>,
    /// The column and row a drag would drop on in column view.
    drop_column: Option<(usize, usize)>,
    mark_pending: Option<MarkPending>,
    /// The `/` filter field, while open.
    filtering: Option<Filtering>,
    /// The filter count VoiceOver hears: the live count, but only once typing
    /// stops, so fast typing does not queue one announcement per keystroke.
    filter_announce: SharedString,
    filter_announce_gen: u64,
    /// Focus the list after a render because a navigation closed the filter.
    focus_list_after_filter: bool,
    /// The sidebar cursor is walking this pane: later moves replace the history step.
    sidebar_walk: bool,
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
        cx: &mut Context<Self>,
    ) -> Self {
        let columns = cx.global::<Ext>().0.default_columns();
        let mut pane = Self {
            history: History::new(dir.clone()),
            mode: ViewMode::List,
            show_hidden,
            sort: Sort::default(),
            folders: Vec::new(),
            subscriptions: Vec::new(),
            focused: 0,
            focus: cx.focus_handle(),
            table: default_table(&columns),
            columns,
            resize: None,
            preview: None,
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
            drop_row: None,
            column_bounds: Rc::new(RefCell::new(HashMap::new())),
            drop_column: None,
            mark_pending: None,
            filtering: None,
            filter_announce: SharedString::default(),
            filter_announce_gen: 0,
            focus_list_after_filter: false,
            sidebar_walk: false,
        };
        pane.enter(dir, cx);
        if let Some(mode) = start_mode.filter(|_| !cx.global::<Ui>().settings.view_per_folder) {
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

    /// The tab's folder: the list view's folder, or the column view's root.
    pub fn dir(&self) -> &Path {
        self.history.current()
    }

    fn focused_folder(&self) -> &Entity<Folder> {
        &self.folders[self.focused]
    }

    /// The folder the user is in: the focused column in column view.
    pub fn current_dir(&self, cx: &gpui::App) -> PathBuf {
        self.focused_folder().read(cx).dir().to_path_buf()
    }

    pub fn item_count(&self, cx: &gpui::App) -> usize {
        self.focused_folder().read(cx).entries().len()
    }

    pub fn selected_count(&self, cx: &gpui::App) -> usize {
        self.focused_folder().read(cx).selected_count()
    }

    /// The selected files' bytes and how many selected items are folders.
    pub fn selected_bytes(&self, cx: &gpui::App) -> (u64, usize) {
        self.focused_folder().read(cx).selected_bytes()
    }

    /// Whether the `/` filter field is open.
    pub fn is_filtering(&self) -> bool {
        self.filtering.is_some()
    }

    /// The filter field's focus handle while it is open.
    pub fn filter_focus(&self, cx: &gpui::App) -> Option<FocusHandle> {
        self.filtering.as_ref().map(|filtering| filtering.input.read(cx).focus_handle(cx))
    }

    /// The list table's column widths, fitted to the list's width from the last frame.
    fn table_widths(&self) -> Vec<f32> {
        match self.list_bounds.get() {
            // Rows and header sit 8 px in from each side (`mx_2`).
            Some(bounds) => self.table.fitted(f32::from(bounds.size.width) - 16.0),
            None => self.table.columns().iter().map(|c| c.width).collect(),
        }
    }

    /// Whether a row is being renamed.
    pub fn is_renaming(&self) -> bool {
        self.renaming.is_some()
    }

    pub fn is_loading(&self, cx: &gpui::App) -> bool {
        self.folders.iter().any(|f| f.read(cx).is_loading_deep(cx))
    }

    /// The visible (name, depth) rows of the focused folder, for tests.
    #[cfg(test)]
    pub fn rows(&self, cx: &gpui::App) -> Vec<(String, usize)> {
        let folder = self.focused_folder().read(cx);
        (0..folder.row_count()).filter_map(|ix| folder.row(ix).map(|(e, d)| (e.name.clone(), d))).collect()
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

    /// The folder Back/Forward would show, for tooltip labels.
    pub fn back_target(&self) -> Option<&std::path::Path> {
        self.history.back_target()
    }

    pub fn forward_target(&self) -> Option<&std::path::Path> {
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
        let active = self.focus.is_focused(window) || self.filtering.is_some();
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
        format!("{} of {} shown", folder.row_count(), folder.entries().len())
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
                hidden: false,
                size: 0,
                modified: None,
                created: None,
                added: None,
            });
        Some((entry, Some(focused.entries().len())))
    }

    /// The preview column's aria label: a folder's summary or a file's preview.
    pub fn preview_label(&self, cx: &gpui::App) -> Option<String> {
        let (entry, items) = self.preview_parts(cx)?;
        if entry.is_dir && !entry.is_package() {
            Some(preview::folder_spoken(&entry.name, items))
        } else {
            Some(preview::spoken(&entry, self.preview(cx)))
        }
    }

    /// The child column's item count, when that column is loaded for `entry`.
    /// The column is already on screen, so this costs no disk I/O.
    fn child_count(&self, entry: &Entry, cx: &gpui::App) -> Option<usize> {
        let child = self.folders.get(self.focused + 1)?.read(cx);
        (!child.is_loading() && child.dir() == entry.path).then(|| child.entries().len())
    }

    pub fn preview(&self, cx: &gpui::App) -> Option<&Preview> {
        let cursor = self.focused_folder().read(cx).cursor_entry().map(|e| e.path.clone())?;
        self.preview.as_ref().filter(|p| p.path == cursor)
    }

    // ----- navigation -----

    /// Go to `dir` as a new history step.
    pub fn navigate(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        self.sidebar_walk = false;
        self.history.visit(dir.clone());
        self.enter(dir, cx);
    }

    /// Show `dir` while the sidebar cursor moves. The first move is a normal history
    /// step (Back returns to where the walk began); later moves replace it, so a walk
    /// past five favorites is one Cmd-[.
    pub fn show_from_sidebar(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        if dir == self.history.current() {
            return;
        }
        if self.sidebar_walk {
            self.history.replace_current(dir.clone());
        } else {
            self.history.visit(dir.clone());
            self.sidebar_walk = true;
        }
        self.enter(dir, cx);
    }

    /// Return, Cmd-Down, → or a click in the sidebar: end the walk and go there if
    /// the pane isn't showing it already.
    pub fn open_from_sidebar(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        self.sidebar_walk = false;
        if dir != self.history.current() {
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

    pub fn go_back(&mut self, cx: &mut Context<Self>) {
        self.sidebar_walk = false;
        let from = self.dir().to_path_buf();
        if let Some(dir) = self.history.back().map(Path::to_path_buf) {
            self.enter(dir, cx);
            self.folders[0].update(cx, |f, _| f.select_after_load(from));
        }
    }

    pub fn go_forward(&mut self, cx: &mut Context<Self>) {
        self.sidebar_walk = false;
        if let Some(dir) = self.history.forward().map(Path::to_path_buf) {
            self.enter(dir, cx);
        }
    }

    pub fn open_parent(&mut self, cx: &mut Context<Self>) {
        let child = self.current_dir(cx);
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
        for folder in self.folders.clone() {
            if folder == here && !mine.is_empty() {
                folder.update(cx, |f, cx| f.select_when_present(mine.clone(), mode, cx));
            } else {
                folder.update(cx, |f, cx| f.reload(cx));
            }
        }
    }

    /// Start a feedback effect on the focused folder's visible rows among `paths`.
    pub fn start_effect(&mut self, paths: Vec<PathBuf>, fx: Fx, cx: &mut Context<Self>) {
        let mode = cx.global::<Ui>().feedback.mode;
        let folder = self.focused_folder().clone();
        folder.update(cx, |folder, cx| folder.start_effect(&paths, fx, mode, cx));
    }

    /// The selected items of the focused folder, in display order.
    pub fn selected_paths(&self, cx: &gpui::App) -> Vec<PathBuf> {
        self.focused_folder().read(cx).selected_paths()
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
            CommandId::SelectAll => self.select_all(&SelectAll, window, cx),
            CommandId::StartFilter => self.start_filter(&StartFilter, window, cx),
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
        for folder in &self.folders {
            folder.update(cx, |f, cx| f.set_sort(sort, cx));
        }
        cx.notify();
        true
    }

    #[cfg(test)]
    pub fn sort(&self) -> Sort {
        self.sort
    }

    /// Switch view as the user asked (Cmd-2, Cmd-3), and remember it for the folder
    /// shown, unless views don't follow folders: then it is this window's alone.
    pub fn set_mode(&mut self, mode: ViewMode, cx: &mut Context<Self>) {
        if self.apply_mode(mode, cx) && cx.global::<Ui>().settings.view_per_folder {
            self.remember(ViewChange::View(mode), cx);
        }
    }

    /// Write a view change to `views.toml` as this folder's own setting.
    fn remember(&mut self, change: ViewChange, cx: &mut Context<Self>) {
        let dir = self.dir().to_path_buf();
        crate::ui::save_view_change(dir, change, cx);
    }

    /// Show this folder the way `views.toml` says, unless that was already done for
    /// this folder and this version of the file.
    pub fn apply_view(&mut self, cx: &mut Context<Self>) {
        let ui = cx.global::<Ui>();
        let key = (ui.views_generation, self.dir().to_path_buf());
        if self.applied_view.as_ref() == Some(&key) {
            return;
        }
        let resolved = ui.views.resolve(&key.1);
        self.applied_view = Some(key);
        self.apply_sort(resolved.sort(), cx);
        let ids: Vec<&str> = self.columns.iter().map(|c| c.id()).collect();
        if ids != resolved.columns.0 {
            let available = cx.global::<Ext>().0.columns().to_vec();
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
            self.apply_mode(resolved.view.0, cx);
        }
        cx.notify();
    }

    /// Switch between list and column view, staying in the same folder with the same selection.
    fn apply_mode(&mut self, mode: ViewMode, cx: &mut Context<Self>) -> bool {
        if self.mode == mode {
            return false;
        }
        self.drop_filter(cx);
        self.mode = mode;
        match mode {
            // The list shows the folder you were in: the focused column becomes the root.
            ViewMode::List => {
                let keep = self.folders.remove(self.focused);
                let subscription = self.subscriptions.remove(self.focused);
                let dir = keep.read(cx).dir().to_path_buf();
                self.folders = vec![keep];
                self.subscriptions = vec![subscription];
                self.focused = 0;
                self.history.visit(dir);
            }
            ViewMode::Columns => self.update_chain(0, cx),
        }
        cx.notify();
        true
    }

    fn enter(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        if self.filtering.take().is_some() {
            // Navigating away ends the filter session like Esc does.
            self.emit_filter_end(cx);
            self.focus_list_after_filter = true;
        }
        self.folders.clear();
        self.subscriptions.clear();
        self.focused = 0;
        self.preview = None;
        self.status = None;
        // The folder's view decides the sort before the listing starts.
        let ui = cx.global::<Ui>();
        let resolved = ui.views.resolve(&dir);
        self.sort = resolved.sort();
        self.push_folder(dir.clone(), true, cx);
        self.apply_view(cx);
        cx.emit(PaneEvent::Entered(dir));
        cx.notify();
    }

    fn push_folder(&mut self, dir: PathBuf, auto_select: bool, cx: &mut Context<Self>) {
        let (hidden, sort) = (self.show_hidden, self.sort);
        let folder = cx.new(|cx| Folder::new(dir, hidden, sort, auto_select, cx));
        self.subscriptions
            .push(cx.subscribe(&folder, |this, folder, event, cx| this.on_folder_event(folder, event, cx)));
        self.folders.push(folder);
        // Keep the newest column in view.
        self.strip_scroll.scroll_to_item(self.folders.len() - 1);
    }

    fn truncate(&mut self, len: usize) {
        self.folders.truncate(len);
        self.subscriptions.truncate(len);
        self.focused = self.focused.min(len.saturating_sub(1));
    }

    fn index_of(&self, id: EntityId) -> Option<usize> {
        self.folders.iter().position(|f| f.entity_id() == id)
    }

    fn on_folder_event(&mut self, folder: Entity<Folder>, event: &FolderEvent, cx: &mut Context<Self>) {
        let Some(k) = self.index_of(folder.entity_id()) else { return };
        if let FolderEvent::CursorMoved = event {
            if self.mode == ViewMode::Columns {
                self.update_chain(k, cx);
            }
            if k == self.focused {
                self.load_preview(cx);
            }
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
                self.truncate(k + 1);
                self.push_folder(path, false, cx);
            }
            None => self.truncate(k + 1),
        }
    }

    fn load_preview(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self.cursor_entry(cx) else {
            self.preview = None;
            return;
        };
        if self.preview.as_ref().is_some_and(|p| p.path == entry.path) {
            return;
        }
        let extensions = cx.global::<Ext>().0.clone();
        cx.spawn(async move |this, cx| {
            let path = entry.path.clone();
            let fetched = cx.background_spawn(async move { preview::fetch(&entry, &extensions) }).await;
            this.update(cx, |this, cx| {
                if this.cursor_entry(cx).is_some_and(|e| e.path == path) {
                    this.preview = Some(fetched);
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
        if let Err(e) = std::process::Command::new("open").arg(&path).spawn() {
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
            TextInputEvent::Confirmed => {
                this.open_selected(cx);
                if this.filtering.is_none() {
                    this.focus_list_after_filter = false;
                    window.focus(&this.focus, cx);
                }
            }
            TextInputEvent::Cancelled => this.close_filter(window, cx),
            TextInputEvent::CompletionRequested => {}
            TextInputEvent::BackspaceAtStart => {}
        });
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.filtering = Some(Filtering { input, _subscription: subscription });
        cx.notify();
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
            folder.update(cx, |folder, cx| folder.set_filter(None, cx));
        }
        self.focus_list_after_filter = false;
        cx.notify();
        Some(query)
    }

    fn clear_filter(&mut self, _: &ClearFilter, window: &mut Window, cx: &mut Context<Self>) {
        self.close_filter(window, cx);
    }

    fn close_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.filtering.take().is_none() {
            return;
        }
        self.emit_filter_end(cx);
        self.filter_announce = SharedString::default();
        self.filter_announce_gen += 1;
        for folder in &self.folders {
            folder.update(cx, |folder, cx| folder.set_filter(None, cx));
        }
        self.focus_list_after_filter = false;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Close the filter from a navigation path that has no window (the list
    /// gets its focus back after the next render).
    fn drop_filter(&mut self, cx: &mut Context<Self>) {
        if self.filtering.take().is_none() {
            return;
        }
        self.emit_filter_end(cx);
        self.filter_announce = SharedString::default();
        self.filter_announce_gen += 1;
        for folder in &self.folders {
            folder.update(cx, |folder, cx| folder.set_filter(None, cx));
        }
        self.focus_list_after_filter = true;
        cx.notify();
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
        self.with_focused(cx, |f, cx| f.select_only(f.step(-1), cx));
    }

    fn move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        self.with_focused(cx, |f, cx| f.select_only(f.step(1), cx));
    }

    fn extend_up(&mut self, _: &ExtendUp, _: &mut Window, cx: &mut Context<Self>) {
        self.with_focused(cx, |f, cx| f.extend_to(f.step(-1), cx));
    }

    fn extend_down(&mut self, _: &ExtendDown, _: &mut Window, cx: &mut Context<Self>) {
        self.with_focused(cx, |f, cx| f.extend_to(f.step(1), cx));
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

    /// Column view: move into the column showing the selected folder's contents.
    fn move_right(&mut self, _: &MoveRight, _: &mut Window, cx: &mut Context<Self>) {
        if self.mode == ViewMode::List {
            self.list_right(cx);
            return;
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
    fn move_left(&mut self, _: &MoveLeft, _: &mut Window, cx: &mut Context<Self>) {
        if self.mode == ViewMode::List {
            self.list_left(cx);
            return;
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
            self.history.visit(parent.clone());
            self.push_folder(parent.clone(), false, cx);
            let folder = self.folders.pop().expect("pushed above");
            let subscription = self.subscriptions.pop().expect("pushed above");
            self.folders.insert(0, folder.clone());
            self.subscriptions.insert(0, subscription);
            folder.update(cx, |f, _| f.select_after_load(child));
            self.focused = 0;
            cx.emit(PaneEvent::Entered(parent));
        }
        self.strip_scroll.scroll_to_item(self.focused);
        self.load_preview(cx);
        cx.notify();
    }

    fn open(&mut self, _: &OpenSelection, _: &mut Window, cx: &mut Context<Self>) {
        self.open_selected(cx);
    }

    fn rename(&mut self, _: &Rename, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(entry) = self.cursor_entry(cx) {
            self.begin_rename(entry.path, window, cx);
        }
    }

    /// Replace the item's name with a text field; the name without its extension is selected.
    fn begin_rename(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let stem_end = match name.rfind('.') {
            Some(i) if i > 0 && !path.is_dir() => i,
            _ => name.len(),
        };
        let input = cx.new(|cx| TextInput::new(name.clone(), "Name", cx));
        input.update(cx, |input, cx| input.select_range(0..stem_end, window, cx));
        let subscription =
            cx.subscribe_in(&input, window, move |this, input, event: &TextInputEvent, window, cx| match event {
                TextInputEvent::Changed => {}
                TextInputEvent::Confirmed => {
                    let new_name = input.read(cx).text().trim().to_owned();
                    let Some(renaming) = this.renaming.take() else { return };
                    let old = renaming.path.file_name().map(|n| n.to_string_lossy().into_owned());
                    if old.as_deref() != Some(new_name.as_str()) {
                        cx.emit(PaneEvent::Run(Request::Rename { path: renaming.path, new_name }));
                    }
                    window.focus(&this.focus, cx);
                    cx.notify();
                }
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

    fn new_folder(&mut self, _: &NewFolder, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PaneEvent::Run(Request::NewFolder { parent: self.current_dir(cx) }));
    }

    fn new_file(&mut self, _: &NewFile, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PaneEvent::Run(Request::NewFile { parent: self.current_dir(cx) }));
    }

    fn duplicate(&mut self, _: &Duplicate, _: &mut Window, cx: &mut Context<Self>) {
        let paths = self.selected_paths(cx);
        if !paths.is_empty() {
            cx.emit(PaneEvent::Run(Request::Duplicate { paths }));
        }
    }

    fn move_to_trash(&mut self, _: &MoveToTrash, _: &mut Window, cx: &mut Context<Self>) {
        let paths = self.selected_paths(cx);
        if !paths.is_empty() {
            cx.emit(PaneEvent::Run(Request::Trash { paths }));
        }
    }

    fn delete_permanently(&mut self, _: &DeletePermanently, _: &mut Window, cx: &mut Context<Self>) {
        let paths = self.selected_paths(cx);
        if !paths.is_empty() {
            cx.emit(PaneEvent::Run(Request::DeletePermanently { paths }));
        }
    }

    fn copy_files(&mut self, _: &CopyFiles, _: &mut Window, cx: &mut Context<Self>) {
        let paths = self.selected_paths(cx);
        if !paths.is_empty() {
            cx.emit(PaneEvent::Copy { paths, cut: false });
        }
    }

    fn cut_files(&mut self, _: &CutFiles, _: &mut Window, cx: &mut Context<Self>) {
        let paths = self.selected_paths(cx);
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

    /// Files dropped onto folder `k`'s list (as opposed to onto one of its folders).
    fn drop_files(&mut self, k: usize, paths: &ExternalPaths, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(into) = self.folders.get(k).map(|f| f.read(cx).dir().to_path_buf()) {
            self.drop_into(into, paths, window, cx);
        }
    }

    /// Files dropped from Crow, Finder or another app (see `operations::drop_request`).
    fn drop_into(&mut self, into: PathBuf, paths: &ExternalPaths, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(request) = crate::operations::drop_request(paths.paths(), into, window.modifiers().alt) {
            cx.emit(PaneEvent::Run(request));
        }
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
        let handle = window.window_handle();
        let async_cx = cx.to_async();
        crow_macos::set_quick_look_key_handler(move |key| {
            async_cx.update(|cx| {
                let action: Box<dyn Action> = match key {
                    crow_macos::QuickLookKey::Up => Box::new(MoveUp),
                    crow_macos::QuickLookKey::Down => Box::new(MoveDown),
                    crow_macos::QuickLookKey::Left => Box::new(MoveLeft),
                    crow_macos::QuickLookKey::Right => Box::new(MoveRight),
                };
                let _ = handle.update(cx, |_, window, cx| window.dispatch_action(action, cx));
            });
        });
        crow_macos::toggle_quick_look(&entry.path);
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
        // AppKit wants the mouse-down event when a GPUI drag later leaves the window.
        crow_macos::remember_mouse_down();
        window.focus(&self.focus, cx);
        if k != self.focused {
            self.drop_filter(cx);
            self.focused = k;
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

    // ----- scroll -----

    /// A pointer or touch scroll in folder `k`'s list: suggest `/` once a long
    /// folder is scrolled well past its first screen.
    fn on_list_scroll(&mut self, k: usize, cx: &mut Context<Self>) {
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
        let y = pointer.y - bounds.top() - px(LIST_TOP_INSET) + folder.scroll().0.borrow().base_handle.offset().y;
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
        let y = pointer.y - bounds.top() + folder.scroll().0.borrow().base_handle.offset().y;
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

    fn update_drag_row(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let row = if self.mode == ViewMode::List { self.drag_row_at(cx, position) } else { None };
        let column = if self.mode == ViewMode::Columns { self.column_row_at(cx, position) } else { None };
        if row != self.drop_row || column != self.drop_column {
            self.drop_row = row;
            self.drop_column = column;
            cx.notify();
        }
    }

    #[cfg(test)]
    pub fn drop_row(&self) -> Option<usize> {
        self.drop_row
    }

    #[cfg(test)]
    pub fn drop_column(&self) -> Option<(usize, usize)> {
        self.drop_column
    }

    fn bench_step(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        folder.scroll_strict((bench.frame * 3) % shown);
        if bench.frame == 600 {
            let mut v = std::mem::take(&mut bench.intervals);
            v.sort_by(f64::total_cmp);
            let at = |q: f64| v[((v.len() - 1) as f64 * q) as usize];
            let avg = v.iter().sum::<f64>() / v.len() as f64;
            bench.scrolled = Some(format!(
                "items={count} cold_first_paint={:.0}ms cold_load={:.0}ms frames={} avg={avg:.2}ms ({:.0} fps) p50={:.2}ms p95={:.2}ms p99={:.2}ms max={:.2}ms",
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
            println!(
                "BENCH {} warm_first_paint={:.0}ms warm_load={:.0}ms",
                bench.scrolled.as_deref().unwrap_or_default(),
                folder.first_paint_ms().unwrap_or_default(),
                folder.load_ms()
            );
            cx.quit();
        }
    }

    // ----- list view -----

    fn render_table_rows(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<impl IntoElement + use<>> {
        let ui = cx.global::<Ui>();
        let theme = &ui.theme;
        let font_size = px(ui.settings.font_size);
        let mode = ui.feedback.mode;
        let now = Instant::now();
        let muted = color(theme.role("fg-muted"));
        let stripe = color_alpha(theme.role("surface"), 0x70);
        let active = self.focus.is_focused(window) || self.filtering.is_some();
        let glide = mode == Effective::Full && active;
        let selection_bg = if active { active_selection(theme) } else { inactive_selection(theme) };
        let drop_bg = drop_target(theme);
        let column_cx = ColumnContext { kind: &kind, time: &format_time };
        let widths = self.table_widths();
        // Columns hidden for a narrow pane have width 0; the last shown one stretches.
        let last = widths.iter().rposition(|w| *w > 0.0).unwrap_or(0);
        let folder = self.folders[0].read(cx);
        let list_folder = self.folders[0].clone();
        let selected_entries = folder.selected_entries();
        let total = folder.row_count();
        let cursor = folder.cursor_ix();
        let rows: Vec<(usize, Entry, usize, bool)> = range
            .filter_map(|ix| {
                folder.row(ix).map(|(entry, depth)| (ix, entry.clone(), depth, folder.is_selected(&entry.path)))
            })
            .collect();
        rows.into_iter()
            .map(|(ix, entry, depth, selected)| {
                let cells: Vec<(String, Align)> =
                    self.columns.iter().map(|c| (c.text(&entry, &column_cx), c.align())).collect();
                let spoken =
                    cells.iter().map(|(text, _)| text.as_str()).filter(|t| *t != "--").collect::<Vec<_>>().join(", ");
                let indent = NAME_INSET + depth as f32 * INDENT;
                let name_room = px(widths[0] - indent - TRIANGLE_WIDTH - 8.0 - 16.0 - 8.0 - CELL_PADDING);
                let name = truncate_middle_parts(&entry.name, name_room, font_size, window);
                let ranges = folder.row_match(ix).map(|range| map_range(&name, range));
                let expanded = folder.is_expanded(&entry.path, cx);
                let drag_entries = if selected {
                    selected_entries.clone()
                } else {
                    Arc::new(vec![(entry.path.clone(), entry.is_dir)])
                };
                let drag_icon = icon(&entry.path, entry.is_dir, 22);
                let drag_count = drag_entries.len();
                let drag_label: SharedString =
                    if drag_count > 1 { format!("{drag_count} items").into() } else { entry.name.clone().into() };
                let handoff = DragHandoff::new();
                let look = folder.effect_look(&entry.path, mode, now).unwrap_or_default();
                let shift = folder.row_shift(ix, mode, now, ROW_HEIGHT);
                let tint = effect_tint(look, theme);
                let dim = content_opacity(
                    entry.hidden,
                    self.renaming.as_ref().is_some_and(|renaming| renaming.path == entry.path),
                );
                let reveal = folder.disclosure_opacity(ix, now);
                let turn = folder.chevron_turn(&entry.path, now).unwrap_or(if expanded { 1.0 } else { 0.0 });
                let row = div()
                    .id(ix)
                    // Only visible rows exist, so each row tells VoiceOver where it sits in the folder.
                    .role(Role::ListBoxOption)
                    .aria_label(format!("{}, {spoken}", entry.name))
                    .aria_position_in_set(ix + 1)
                    .aria_size_of_set(total)
                    .aria_selected(selected)
                    .when(cursor == Some(ix), |row| row.aria_active_descendant())
                    .h(px(ROW_HEIGHT))
                    .mx_2()
                    .rounded_md()
                    .relative()
                    .flex()
                    .items_center()
                    .when(shift != 0.0, |row| row.top(px(shift)))
                    .when(look.opacity * reveal < 1.0, |row| row.opacity(look.opacity * reveal))
                    .when(ix % 2 == 1 && !(selected && glide && cursor == Some(ix)), |row| row.bg(stripe))
                    .when(draws_selection_fill(selected, glide, cursor == Some(ix)), |row| row.bg(selection_bg))
                    .when_some(tint, |row, tint| row.child(div().absolute().inset_0().rounded_md().bg(tint)))
                    .debug_selector(|| format!("row-{}", entry.name))
                    .on_drag(DraggedSelection { entries: drag_entries }, {
                        let handoff = handoff.clone();
                        move |_value, offset, window, cx| {
                            let (display, pill_width, width) = measure_combo(&drag_label, window);
                            handoff.cursor.set(offset);
                            handoff.width.set(width);
                            handoff.pill_width.set(pill_width);
                            *handoff.label.borrow_mut() = display.clone();
                            cx.new(|_| DraggedFiles {
                                icon: drag_icon.clone(),
                                label: display,
                                pill_width,
                                width,
                                cursor: offset,
                            })
                        }
                    })
                    .external_drag_payload::<DraggedSelection>(move |selection, window, cx| {
                        let width = handoff.width.get();
                        let pill_width = handoff.pill_width.get();
                        let cursor = handoff.cursor.get();
                        let label = handoff.label.borrow().clone();
                        let entries = selection.entries.as_slice();
                        if let Some((path, is_dir)) = entries.first()
                            && let Some(png) = crow_macos::icon_png(path, *is_dir, 22)
                            && crow_macos::begin_drag(
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
                    })
                    .when(entry.is_dir && !entry.is_package(), |row| {
                        let target = entry.path.clone();
                        let target_selection = entry.path.clone();
                        row.on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                            this.drop_into(target.clone(), paths, window, cx);
                            cx.stop_propagation();
                        }))
                        .on_drop(cx.listener(
                            move |this, selection: &DraggedSelection, window, cx| {
                                this.drop_into(target_selection.clone(), &selection.external(), window, cx);
                                cx.stop_propagation();
                            },
                        ))
                    })
                    .when(entry.is_dir && !entry.is_package() && self.drop_row == Some(ix), |row| row.bg(drop_bg))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            this.press(0, ix, event, window, cx)
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
                            .overflow_hidden()
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
                                        format!("Collapse {}", entry.name)
                                    } else {
                                        format!("Expand {}", entry.name)
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
                            .child(row_icon(&entry, look, theme))
                            .child(
                                div()
                                    .debug_selector({
                                        let name = entry.name.clone();
                                        move || format!("row-name-{name}")
                                    })
                                    .relative()
                                    .left(px(look.dx))
                                    .opacity(dim)
                                    .child(match self.renaming.as_ref().filter(|r| r.path == entry.path) {
                                        Some(r) => div().flex_1().child(r.input.clone()).into_any_element(),
                                        None => match ranges {
                                            Some(ranges) if ranges.iter().any(|range| !range.is_empty()) => div()
                                                .whitespace_nowrap()
                                                .child(highlight_text(
                                                    name.text.clone(),
                                                    &ranges,
                                                    theme,
                                                    selected && active,
                                                ))
                                                .into_any_element(),
                                            _ => div().whitespace_nowrap().child(name.text.clone()).into_any_element(),
                                        },
                                    }),
                            ),
                    );
                row.children(cells.into_iter().enumerate().filter(|(i, _)| widths[i + 1] > 0.0).map(
                    |(i, (text, align))| {
                        let width = px(widths[i + 1]);
                        div()
                            .when(i + 1 == last, |cell| cell.flex_1().min_w(width))
                            .when(i + 1 != last, |cell| cell.w(width).flex_none())
                            .px(px(CELL_PADDING))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_color(muted)
                            .opacity(dim)
                            .when(align == Align::Right, |cell| cell.text_right())
                            .child(text)
                    },
                ))
            })
            .collect()
    }

    fn render_table_header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.global::<Ui>().theme.clone();
        let sort = self.sort;
        let widths = self.table_widths();
        // Columns hidden for a narrow pane have width 0; the last shown one stretches.
        let last = widths.iter().rposition(|w| *w > 0.0).unwrap_or(0);
        let border = color(theme.role("border"));
        // (title, sort key, alignment) for the name column and each shown column.
        let mut specs = vec![("Name".to_owned(), Some(SortKey::Name), Align::Left)];
        specs.extend(self.columns.iter().map(|c| (c.title().to_owned(), c.sort_key(), c.align())));
        let mut header = div()
            .id("table-header")
            .debug_selector(|| "table-header".into())
            .h(px(28.))
            .flex_none()
            .mx_2()
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
                .when(i == last, |cell| cell.flex_1().min_w(width))
                .when(i != last, |cell| cell.w(width).flex_none())
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
                .child(div().w(px(10.)).flex_none().child(chevron))
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
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<impl IntoElement + use<>> {
        let ui = cx.global::<Ui>();
        let theme = &ui.theme;
        let font_size = px(ui.settings.font_size);
        let mode = ui.feedback.mode;
        let now = Instant::now();
        // Spec: only the focused column shows its selection in the accent color.
        let active = k == self.focused && (self.focus.is_focused(window) || self.filtering.is_some());
        let glide = mode == Effective::Full && active;
        let selection_bg = if active { active_selection(theme) } else { inactive_selection(theme) };
        let drop_bg = drop_target(theme);
        let subtle = color(theme.role("fg-subtle"));
        let Some(folder) = self.folders.get(k).map(|f| f.read(cx)) else { return Vec::new() };
        let width = column_width(folder.entries());
        let name_room = px(width - 16.0 - 8.0 - 16.0 - 2.0 * CELL_PADDING - 8.0);
        let total = folder.row_count();
        let cursor = folder.cursor_ix();
        let selected_entries = folder.selected_entries();
        let rows: Vec<(usize, Entry, bool)> =
            range.filter_map(|ix| folder.row(ix).map(|(e, _)| (ix, e.clone(), folder.is_selected(&e.path)))).collect();
        rows.into_iter()
            .map(|(ix, entry, selected)| {
                let name = truncate_middle_parts(&entry.name, name_room, font_size, window);
                let ranges = folder.row_match(ix).map(|range| map_range(&name, range));
                let navigable = entry.is_dir && !entry.is_package();
                let drag_entries = if selected {
                    selected_entries.clone()
                } else {
                    Arc::new(vec![(entry.path.clone(), entry.is_dir)])
                };
                let drag_icon = icon(&entry.path, entry.is_dir, 22);
                let drag_count = drag_entries.len();
                let drag_label: SharedString =
                    if drag_count > 1 { format!("{drag_count} items").into() } else { entry.name.clone().into() };
                let handoff = DragHandoff::new();
                let look = folder.effect_look(&entry.path, mode, now).unwrap_or_default();
                let shift = folder.row_shift(ix, mode, now, ROW_HEIGHT);
                let tint = effect_tint(look, theme);
                let dim = content_opacity(
                    entry.hidden,
                    self.renaming.as_ref().is_some_and(|renaming| renaming.path == entry.path),
                );
                div()
                    .id(ix)
                    .role(Role::ListBoxOption)
                    .aria_label(format!("{}, {}", entry.name, kind(&entry)))
                    .aria_position_in_set(ix + 1)
                    .aria_size_of_set(total)
                    .aria_selected(selected)
                    .when(active && cursor == Some(ix), |row| row.aria_active_descendant())
                    .h(px(ROW_HEIGHT))
                    .w_full()
                    .px(px(CELL_PADDING))
                    .rounded_md()
                    .relative()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(shift != 0.0, |row| row.top(px(shift)))
                    .when(look.opacity < 1.0, |row| row.opacity(look.opacity))
                    .when(draws_selection_fill(selected, glide, cursor == Some(ix)), |row| row.bg(selection_bg))
                    .when_some(tint, |row, tint| row.child(div().absolute().inset_0().rounded_md().bg(tint)))
                    .debug_selector(|| format!("column-{k}-row-{}", entry.name))
                    .on_drag(DraggedSelection { entries: drag_entries }, {
                        let handoff = handoff.clone();
                        move |_value, offset, window, cx| {
                            let (display, pill_width, width) = measure_combo(&drag_label, window);
                            handoff.cursor.set(offset);
                            handoff.width.set(width);
                            handoff.pill_width.set(pill_width);
                            *handoff.label.borrow_mut() = display.clone();
                            cx.new(|_| DraggedFiles {
                                icon: drag_icon.clone(),
                                label: display,
                                pill_width,
                                width,
                                cursor: offset,
                            })
                        }
                    })
                    .external_drag_payload::<DraggedSelection>(move |selection, window, cx| {
                        let width = handoff.width.get();
                        let pill_width = handoff.pill_width.get();
                        let cursor = handoff.cursor.get();
                        let label = handoff.label.borrow().clone();
                        let entries = selection.entries.as_slice();
                        if let Some((path, is_dir)) = entries.first()
                            && let Some(png) = crow_macos::icon_png(path, *is_dir, 22)
                            && crow_macos::begin_drag(
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
                    })
                    .when(navigable, |row| {
                        let target = entry.path.clone();
                        let target_selection = entry.path.clone();
                        row.on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                            this.drop_into(target.clone(), paths, window, cx);
                            cx.stop_propagation();
                        }))
                        .on_drop(cx.listener(
                            move |this, selection: &DraggedSelection, window, cx| {
                                this.drop_into(target_selection.clone(), &selection.external(), window, cx);
                                cx.stop_propagation();
                            },
                        ))
                    })
                    .when(navigable && self.drop_column == Some((k, ix)), |row| row.bg(drop_bg))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            this.press(k, ix, event, window, cx)
                        }),
                    )
                    .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                        if event.click_count() == 2 {
                            this.open_selected(cx);
                        }
                    }))
                    .child(row_icon(&entry, look, theme))
                    .child(
                        div()
                            .debug_selector({
                                let name = entry.name.clone();
                                move || format!("column-{k}-row-name-{name}")
                            })
                            .relative()
                            .left(px(look.dx))
                            .flex_1()
                            .opacity(dim)
                            .child(match self.renaming.as_ref().filter(|r| r.path == entry.path) {
                                Some(r) => div().flex_1().child(r.input.clone()).into_any_element(),
                                None => match ranges {
                                    Some(ranges) if ranges.iter().any(|range| !range.is_empty()) => div()
                                        .flex_1()
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .child(highlight_text(name.text.clone(), &ranges, theme, selected && active))
                                        .into_any_element(),
                                    _ => div()
                                        .flex_1()
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .child(name.text.clone())
                                        .into_any_element(),
                                },
                            }),
                    )
                    .child(div().w(px(10.)).flex_none().text_color(subtle).child(if navigable { "›" } else { "" }))
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
        let mut strip = div()
            .id("columns")
            .debug_selector(|| "columns".into())
            .size_full()
            .flex()
            .overflow_x_scroll()
            .track_scroll(&self.strip_scroll);
        for (k, folder) in self.folders.iter().enumerate() {
            let (count, shown, width, scroll, title, filtered) = {
                let f = folder.read(cx);
                (
                    f.entries().len(),
                    f.row_count(),
                    column_width(f.entries()),
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
                mode == Effective::Full && k == focused && (self.focus.is_focused(window) || self.filtering.is_some());
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
                            side_inset: 4.0,
                            top_inset: 0.0,
                        }
                        .render()
                    };
                    let glide_fill = if glide {
                        let f = folder.read(cx);
                        f.cursor_ix().map(|ix| {
                            let offset = shown_offset(&scroll);
                            let y = ix as f32 * ROW_HEIGHT;
                            div()
                                .absolute()
                                .left(px(4.))
                                .right(px(4.))
                                .h(px(ROW_HEIGHT))
                                .rounded_md()
                                .bg(active_selection(&theme))
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
                        .children(glide_fill)
                        .child(
                            uniform_list(
                                ("column-list", k),
                                shown,
                                cx.processor(move |this, range, window, cx| {
                                    this.render_column_rows(k, range, window, cx)
                                }),
                            )
                            .track_scroll(&scroll)
                            .flex_1()
                            .min_h_0()
                            .px_1(),
                        )
                        .children(ghosts)
                        .into_any_element()
                }
            };
            strip =
                strip.child(
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
                        .w(px(width))
                        .flex_none()
                        .h_full()
                        .flex()
                        .flex_col()
                        .py_1()
                        .border_r_1()
                        .border_color(border)
                        .when(k == focused, |c| c.bg(color_alpha(theme.role("surface"), 0x40)))
                        .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                            this.drop_files(k, paths, window, cx)
                        }))
                        .on_drop(cx.listener(move |this, selection: &DraggedSelection, window, cx| {
                            this.drop_files(k, &selection.external(), window, cx)
                        }))
                        .on_scroll(cx.listener(move |this, _, _, cx| this.on_list_scroll(k, cx)))
                        .children(field)
                        .child(body)
                        .children(footer),
                );
        }
        // The preview is the last column: a file's preview, or a folder's summary.
        // It takes the rest of the strip when the columns leave room, and keeps
        // PREVIEW_COLUMN_WIDTH when they don't (the strip scrolls instead).
        let show_preview = cx.global::<Ui>().settings.show_preview;
        if show_preview && let Some((entry, items)) = self.preview_parts(cx) {
            let folder = entry.is_dir && !entry.is_package();
            let label = self.preview_label(cx).unwrap_or_default();
            let body = if folder {
                let summary =
                    preview::FolderSummary { path: &entry.path, name: &entry.name, modified: entry.modified, items };
                preview::folder_summary(summary, &theme).into_any_element()
            } else {
                preview::contents(&entry, self.preview(cx), &theme).into_any_element()
            };
            strip = strip.child(
                div()
                    .id("preview-column")
                    .debug_selector(|| "preview-column".into())
                    .role(Role::Group)
                    .aria_label(label)
                    .flex_1()
                    .min_w(px(PREVIEW_COLUMN_WIDTH))
                    .h_full()
                    .p_4()
                    .overflow_hidden()
                    .child(body),
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

/// The file icon with the trash glyph drawn over it while a row is swallowed.
fn row_icon(entry: &Entry, look: Look, theme: &Theme) -> Div {
    div()
        .relative()
        .size(px(16.))
        .flex_none()
        .child(
            div()
                .opacity((1.0 - look.trash) * content_opacity(entry.hidden, false))
                .children(icon(&entry.path, entry.is_dir, 16).map(|i| img(i).size(px(16.)))),
        )
        .when(look.trash > 0.0, |slot| {
            slot.child(
                div().absolute().inset_0().flex().items_center().justify_center().opacity(look.trash).child(glyph(
                    Glyph::Trash,
                    look.trash_size,
                    "fg-muted",
                    theme,
                )),
            )
        })
}

/// A row drawn over the list while its swallow finishes after the listing dropped it.
fn ghost_row(entry: &Entry, look: Look, theme: &Theme, font_size: Pixels) -> Div {
    div()
        .h(px(ROW_HEIGHT))
        .w_full()
        .flex()
        .items_center()
        .gap_2()
        .px(px(CELL_PADDING))
        .opacity(look.opacity)
        .child(row_icon(entry, look, theme))
        .child(
            div()
                .relative()
                .left(px(look.dx))
                .opacity(content_opacity(entry.hidden, false))
                .text_size(font_size)
                .whitespace_nowrap()
                .child(entry.name.clone()),
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
    side_inset: f32,
    top_inset: f32,
}

impl Ghosts<'_> {
    fn render(&self) -> Vec<AnyElement> {
        if self.mode != Effective::Full {
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
                    .child(ghost_row(&effect.entry, look, self.theme, self.font_size))
                    .into_any_element()
            })
            .collect()
    }
}

/// Column view: the shared width, widened for long names up to a cap. Estimated from
/// name lengths (about 7 pt per character) so it costs nothing per frame.
fn column_width(entries: &[Entry]) -> f32 {
    let longest = entries.iter().take(2000).map(|e| e.name.chars().count()).max().unwrap_or(0) as f32;
    (longest * 7.0 + 16.0 + 8.0 + 16.0 + 2.0 * CELL_PADDING + 8.0).clamp(COLUMN_WIDTH, COLUMN_WIDTH_CAP)
}

/// The name column plus the shown columns, at their default widths.
fn default_table(shown: &[Arc<dyn Column>]) -> TableLayout {
    let mut columns = vec![ColumnWidth { id: "name".into(), width: 300.0, min_width: 140.0 }];
    columns.extend(shown.iter().map(|c| ColumnWidth {
        id: c.id().to_owned(),
        width: c.width(),
        min_width: c.min_width(),
    }));
    TableLayout::new(columns)
}

impl Render for Pane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        self.drop_column = if cx.has_active_drag() && self.mode == ViewMode::Columns {
            self.column_row_at(cx, window.mouse_position())
        } else {
            None
        };
        if let Some(path) = self.rename_when_present.clone()
            && self.focused_folder().read(cx).entries().iter().any(|e| e.path == path)
        {
            self.rename_when_present = None;
            // Focus can't move during a render; start the rename right after it.
            cx.defer_in(window, move |this, window, cx| this.begin_rename(path, window, cx));
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
        let glide = mode == Effective::Full && (self.focus.is_focused(window) || self.filtering.is_some());
        // The field is inside the list now: while it has focus, the list's actions must be
        // off the focus path so AppKit's menu equivalents let those keys through.
        let filter_focused = self
            .filtering
            .as_ref()
            .is_some_and(|filtering| filtering.input.read(cx).focus_handle(cx).is_focused(window));
        let mut context = match self.mode {
            ViewMode::List => "FileList".to_owned(),
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
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(self.render_table_header(cx))
                    .children(field)
                    .child({
                        let bounds = self.list_bounds.clone();
                        let body = match no_matches {
                            Some(block) => block,
                            None => {
                                let ghosts = {
                                    let folder = self.folders[0].read(cx);
                                    Ghosts {
                                        folder,
                                        mode,
                                        now,
                                        scroll: &scroll,
                                        theme: &theme,
                                        font_size,
                                        side_inset: 8.0,
                                        top_inset: LIST_TOP_INSET,
                                    }
                                    .render()
                                };
                                let glide_fill = if glide {
                                    let folder = self.folders[0].read(cx);
                                    folder.cursor_ix().map(|ix| {
                                        let offset = shown_offset(&scroll);
                                        let y = ix as f32 * ROW_HEIGHT;
                                        div()
                                            .absolute()
                                            .left(px(8.))
                                            .right(px(8.))
                                            .h(px(ROW_HEIGHT))
                                            .rounded_md()
                                            .bg(active_selection(&theme))
                                            .debug_selector(|| "glide".into())
                                            .with_spring(
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
                                div()
                                    .relative()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .overflow_hidden()
                                    .children(glide_fill)
                                    .child(
                                        uniform_list("entries", rows, cx.processor(Self::render_table_rows))
                                            .track_scroll(&scroll)
                                            .pt(px(LIST_TOP_INSET))
                                            .flex_1(),
                                    )
                                    .children(ghosts)
                                    .into_any_element()
                            }
                        };
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .on_children_prepainted(move |children, window, _| {
                                let new = children.first().copied();
                                // The table fits its columns to this width; a new width
                                // needs one more frame to draw them fitted.
                                // (A refresh during the draw itself is ignored, hence next frame.)
                                if new.map(|b| b.size.width) != bounds.get().map(|b| b.size.width) {
                                    window.on_next_frame(|window, _| window.refresh());
                                }
                                bounds.set(new);
                            })
                            // The list's own scroll handler runs first (deeper hitbox), so
                            // the offset read here is this event's.
                            .on_scroll(cx.listener(|this, _, _, cx| this.on_list_scroll(0, cx)))
                            .child(body)
                    })
                    // Dropped on the list but not on a folder row: into the folder shown.
                    .on_drop(
                        cx.listener(|this, paths: &ExternalPaths, window, cx| this.drop_files(0, paths, window, cx)),
                    )
                    .on_drop(cx.listener(|this, selection: &DraggedSelection, window, cx| {
                        this.drop_files(0, &selection.external(), window, cx)
                    }))
                    .children(footer)
                    .into_any_element()
            }
            ViewMode::Columns => self.render_columns(window, cx).into_any_element(),
        };
        let list = div()
            .id("file-list")
            .debug_selector(|| "file-list".into())
            .role(Role::ListBox)
            .aria_label(if filtered {
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
                    .on_action(cx.listener(Self::extend_up))
                    .on_action(cx.listener(Self::extend_down))
                    .on_action(cx.listener(Self::move_to_first))
                    .on_action(cx.listener(Self::move_to_last))
                    .on_action(cx.listener(Self::select_all))
                    .on_action(cx.listener(Self::move_left))
                    .on_action(cx.listener(Self::move_right))
                    .on_action(cx.listener(Self::open))
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
                    .on_action(cx.listener(Self::begin_mark))
                    .on_action(cx.listener(Self::begin_jump))
                    .on_action(cx.listener(Self::start_filter))
                    .on_action(cx.listener(Self::clear_filter))
            })
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if cx.has_active_drag() {
                    this.update_drag_row(event.position, cx);
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
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _: &MouseUpEvent, _, _| this.resize = None))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _: &MouseUpEvent, _, _| this.resize = None))
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
        div().size_full().flex().flex_col().child(list)
    }
}

#[cfg(test)]
impl Pane {
    pub fn rename_text(&self, cx: &gpui::App) -> Option<String> {
        self.renaming.as_ref().map(|r| r.input.read(cx).text().to_owned())
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

    pub fn names(&self, cx: &gpui::App) -> Vec<String> {
        self.focused_folder().read(cx).names()
    }

    pub fn selected_names(&self, cx: &gpui::App) -> Vec<String> {
        self.focused_folder().read(cx).selected_names()
    }
}

#[cfg(test)]
mod tests {
    use super::{drag_combo_left, draws_selection_fill};

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
