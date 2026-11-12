//! A tab's contents: history, view mode and the open folders. List view shows
//! one folder as a table; column view shows a chain of folders, one per column.
//!
//! Column view focus rules (from the spec):
//! - exactly one column is focused; its selection uses the accent color, others are muted;
//! - a column that only shows the children of a selection is never focused until
//!   you move into it (Right arrow or a click);
//! - typing and filtering always act on the focused column.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crow_core::{Align, Column, ColumnContext, ColumnWidth, History, ResizeDrag, TableLayout, ViewMode};
use crow_fs::{Entry, Sort, SortKey};
use gpui::{
    ClickEvent, Context, Entity, EntityId, EventEmitter, ExternalPaths, FocusHandle, Focusable, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, Role, ScrollHandle,
    SharedString, Subscription, Window, div, img, prelude::*, px, uniform_list,
};

use crate::actions::{
    CopyFiles, CutFiles, DeletePermanently, Duplicate, ExtendDown, ExtendUp, MoveDown, MoveLeft, MoveRight,
    MoveToFirst, MoveToLast, MoveToTrash, MoveUp, NewFolder, OpenSelection, PasteFiles, PasteMoveFiles, QuickLook,
    Rename, SelectAll,
};
use crate::extensions::Ext;
use crate::folder::{Folder, FolderEvent};
use crate::preview::{self, Preview};
use crate::style::{color, color_alpha, format_time, icon, kind, truncate_middle};
use crate::text_input::{TextInput, TextInputEvent};
use crate::ui::Ui;
use crow_fs::ops::Request;

const ROW_HEIGHT: f32 = 24.0;
/// Space before the name for a folder's disclosure triangle (inline expansion comes later).
const NAME_INSET: f32 = 22.0;
const CELL_PADDING: f32 = 8.0;
/// Column view: every column starts this wide; one with long names widens up to the cap.
const COLUMN_WIDTH: f32 = 220.0;
const COLUMN_WIDTH_CAP: f32 = 360.0;
const PREVIEW_COLUMN_WIDTH: f32 = 300.0;

/// Requests the pane can't carry out alone: operations may need a sheet (conflicts,
/// confirmations) and the clipboard is app-wide, so the window handles these.
pub enum PaneEvent {
    Run(Request),
    Copy { paths: Vec<PathBuf>, cut: bool },
    Paste { into: PathBuf, move_items: bool },
}

impl EventEmitter<PaneEvent> for Pane {}

/// An inline rename in progress.
struct Renaming {
    path: PathBuf,
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
    /// A press that may become a drag: (folder index, row, where).
    drag_origin: Option<(usize, usize, Point<Pixels>)>,
    dragging: bool,
    preview: Option<Preview>,
    strip_scroll: ScrollHandle,
    status: Option<SharedString>,
    renaming: Option<Renaming>,
    /// Start renaming this item once it appears (a new folder).
    rename_when_present: Option<PathBuf>,
    bench: Option<Bench>,
}

struct Bench {
    frame: usize,
    intervals: Vec<f64>,
    last: Option<Instant>,
    /// Scroll results, kept while the folder is reopened to time first paint in a warm app.
    scrolled: Option<String>,
}

impl Pane {
    pub fn new(dir: PathBuf, show_hidden: bool, mode: ViewMode, bench: bool, cx: &mut Context<Self>) -> Self {
        let columns = cx.global::<Ext>().0.default_columns();
        let mut pane = Self {
            history: History::new(dir.clone()),
            mode,
            show_hidden,
            sort: Sort::default(),
            folders: Vec::new(),
            subscriptions: Vec::new(),
            focused: 0,
            focus: cx.focus_handle(),
            table: default_table(&columns),
            columns,
            resize: None,
            drag_origin: None,
            dragging: false,
            preview: None,
            strip_scroll: ScrollHandle::new(),
            status: None,
            renaming: None,
            rename_when_present: None,
            bench: bench.then(|| Bench { frame: 0, intervals: Vec::new(), last: None, scrolled: None }),
        };
        pane.enter(dir, cx);
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

    pub fn is_loading(&self, cx: &gpui::App) -> bool {
        self.folders.iter().any(|f| f.read(cx).is_loading())
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

    /// The item the preview and path bar describe: the focused folder's cursor.
    pub fn cursor_entry(&self, cx: &gpui::App) -> Option<Entry> {
        self.focused_folder().read(cx).cursor_entry().cloned()
    }

    pub fn preview(&self, cx: &gpui::App) -> Option<&Preview> {
        let cursor = self.focused_folder().read(cx).cursor_entry().map(|e| e.path.clone())?;
        self.preview.as_ref().filter(|p| p.path == cursor)
    }

    // ----- navigation -----

    /// Go to `dir` as a new history step.
    pub fn navigate(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        self.history.visit(dir.clone());
        self.enter(dir, cx);
    }

    /// Show `path` selected in its folder (Go to Folder with a file, Finder's "Reveal").
    pub fn reveal(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if let Some(parent) = path.parent().map(Path::to_path_buf) {
            self.navigate(parent, cx);
            self.folders[0].update(cx, |f, _| f.select_after_load(path));
        }
    }

    pub fn go_back(&mut self, cx: &mut Context<Self>) {
        let from = self.dir().to_path_buf();
        if let Some(dir) = self.history.back().map(Path::to_path_buf) {
            self.enter(dir, cx);
            self.folders[0].update(cx, |f, _| f.select_after_load(from));
        }
    }

    pub fn go_forward(&mut self, cx: &mut Context<Self>) {
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
        for folder in self.folders.clone() {
            if folder == here && !mine.is_empty() {
                folder.update(cx, |f, cx| f.select_when_present(mine.clone(), cx));
            } else {
                folder.update(cx, |f, cx| f.reload(cx));
            }
        }
    }

    /// The selected items of the focused folder, in display order.
    fn selected_paths(&self, cx: &gpui::App) -> Vec<PathBuf> {
        self.focused_folder().read(cx).selected_paths()
    }

    pub fn set_show_hidden(&mut self, show: bool, cx: &mut Context<Self>) {
        self.show_hidden = show;
        for folder in &self.folders {
            folder.update(cx, |f, cx| f.set_show_hidden(show, cx));
        }
    }

    pub fn set_sort(&mut self, sort: Sort, cx: &mut Context<Self>) {
        self.sort = sort;
        for folder in &self.folders {
            folder.update(cx, |f, cx| f.set_sort(sort, cx));
        }
    }

    /// Switch between list and column view, staying in the same folder with the same selection.
    pub fn set_mode(&mut self, mode: ViewMode, cx: &mut Context<Self>) {
        if self.mode == mode {
            return;
        }
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
    }

    fn enter(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        self.folders.clear();
        self.subscriptions.clear();
        self.focused = 0;
        self.preview = None;
        self.status = None;
        self.push_folder(dir, true, cx);
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
        } else if let Err(e) = std::process::Command::new("open").arg(&entry.path).spawn() {
            self.status = Some(format!("Can't open “{}”: {e}", entry.name).into());
            cx.notify();
        }
    }

    fn with_focused(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Folder, &mut Context<Folder>)) {
        self.status = None;
        self.focused_folder().clone().update(cx, f);
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
        self.with_focused(cx, |f, cx| f.select_only(f.entries().len().saturating_sub(1), cx));
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.with_focused(cx, |f, cx| f.select_all(cx));
    }

    /// Column view: move into the column showing the selected folder's contents.
    fn move_right(&mut self, _: &MoveRight, _: &mut Window, cx: &mut Context<Self>) {
        if self.focused + 1 >= self.folders.len() {
            return;
        }
        self.focused += 1;
        self.with_focused(cx, |f, cx| f.select_first(cx));
        self.strip_scroll.scroll_to_item(self.focused);
        self.load_preview(cx);
        cx.notify();
    }

    /// Column view: back to the parent column. The column you leave stays, unselected.
    fn move_left(&mut self, _: &MoveLeft, _: &mut Window, cx: &mut Context<Self>) {
        if self.focused == 0 {
            return;
        }
        let leaving = self.focused_folder().clone();
        self.focused -= 1;
        leaving.update(cx, |f, cx| f.clear_selection(cx));
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
            });
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.renaming = Some(Renaming { path, input, _subscription: subscription });
        cx.notify();
    }

    fn new_folder(&mut self, _: &NewFolder, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PaneEvent::Run(Request::NewFolder { parent: self.current_dir(cx) }));
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

    /// Files dropped from Finder, another app or another Crow window: moved within
    /// a volume, copied across volumes, like Finder; Option forces a copy.
    fn drop_files(&mut self, k: usize, paths: &ExternalPaths, window: &mut Window, cx: &mut Context<Self>) {
        let Some(into) = self.folders.get(k).map(|f| f.read(cx).dir().to_path_buf()) else { return };
        let paths: Vec<PathBuf> =
            paths.paths().iter().filter(|p| p.parent() != Some(into.as_path())).cloned().collect();
        if paths.is_empty() {
            return;
        }
        let same_volume = paths.iter().all(|p| same_volume(p, &into));
        let request = if same_volume && !window.modifiers().alt {
            Request::Move { paths, into }
        } else {
            Request::Copy { paths, into }
        };
        cx.emit(PaneEvent::Run(request));
    }

    fn quick_look(&mut self, _: &QuickLook, _: &mut Window, cx: &mut Context<Self>) {
        // Mid-word, Space belongs to type-to-select ("my file"), not Quick Look.
        if self.focused_folder().read(cx).type_select_active() {
            self.with_focused(cx, |f, cx| f.type_char(' ', cx));
            return;
        }
        if let Some(entry) = self.cursor_entry(cx) {
            crow_macos::toggle_quick_look(&entry.path);
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        // While a name is being edited, letters belong to the text field, not type-to-select.
        if self.renaming.is_some() {
            return;
        }
        let m = &event.keystroke.modifiers;
        if m.platform || m.control || m.alt {
            return;
        }
        let Some(c) = event.keystroke.key_char.as_deref().and_then(|s| s.chars().next()) else { return };
        if c.is_control() || c == ' ' {
            return;
        }
        self.with_focused(cx, |f, cx| f.type_char(c, cx));
        cx.stop_propagation();
    }

    /// A press on row `ix` of folder `k`: Finder's click, Cmd-click and Shift-click.
    fn press(&mut self, k: usize, ix: usize, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.drag_origin = Some((k, ix, event.position));
        if k != self.focused {
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

    // ----- benchmark -----

    fn bench_step(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(bench) = self.bench.as_mut() else { return };
        window.request_animation_frame();
        let folder = self.folders[0].read(cx);
        let (count, loading) = (folder.entries().len(), folder.is_loading());
        if loading || count == 0 {
            return;
        }
        let now = Instant::now();
        if let Some(prev) = bench.last.replace(now) {
            bench.intervals.push((now - prev).as_secs_f64() * 1000.0);
        }
        bench.frame += 1;
        // About 3 rows per frame: a fast, continuous trackpad scroll.
        folder.scroll_strict((bench.frame * 3) % count);
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
            let dir = self.dir().to_path_buf();
            self.enter(dir, cx);
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
        let muted = color(theme.role("fg-muted"));
        let stripe = color_alpha(theme.role("surface"), 0x70);
        let selection_bg = selection_color(self.focus.is_focused(window), theme);
        let column_cx = ColumnContext { kind: &kind, time: &format_time };
        let widths: Vec<f32> = self.table.columns().iter().map(|c| c.width).collect();
        let last = widths.len() - 1;
        let name_room = px(widths[0] - NAME_INSET - 16.0 - 8.0 - CELL_PADDING);
        let folder = self.folders[0].read(cx);
        let total = folder.entries().len();
        let cursor = folder.cursor_ix();
        let rows: Vec<(usize, Entry, bool)> = range
            .filter_map(|ix| folder.entries().get(ix).map(|e| (ix, e.clone(), folder.is_selected(&e.path))))
            .collect();
        rows.into_iter()
            .map(|(ix, entry, selected)| {
                let cells: Vec<(String, Align)> =
                    self.columns.iter().map(|c| (c.text(&entry, &column_cx), c.align())).collect();
                let spoken =
                    cells.iter().map(|(text, _)| text.as_str()).filter(|t| *t != "--").collect::<Vec<_>>().join(", ");
                let name = truncate_middle(&entry.name, name_room, font_size, window);
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
                    .flex()
                    .items_center()
                    .when(ix % 2 == 1, |row| row.bg(stripe))
                    .when(selected, |row| row.bg(selection_bg))
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
                            .pl(px(NAME_INSET))
                            .pr(px(CELL_PADDING))
                            .flex()
                            .items_center()
                            .gap_2()
                            .overflow_hidden()
                            .child(
                                div()
                                    .size(px(16.))
                                    .flex_none()
                                    .children(icon(&entry.path, entry.is_dir, 16).map(|i| img(i).size(px(16.)))),
                            )
                            .child(match self.renaming.as_ref().filter(|r| r.path == entry.path) {
                                Some(r) => div().flex_1().child(r.input.clone()).into_any_element(),
                                None => div().whitespace_nowrap().child(name).into_any_element(),
                            }),
                    );
                row.children(cells.into_iter().enumerate().map(|(i, (text, align))| {
                    let width = px(widths[i + 1]);
                    div()
                        .when(i + 1 == last, |cell| cell.flex_1().min_w(width))
                        .when(i + 1 != last, |cell| cell.w(width).flex_none())
                        .px(px(CELL_PADDING))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_color(muted)
                        .when(align == Align::Right, |cell| cell.text_right())
                        .child(text)
                }))
            })
            .collect()
    }

    fn render_table_header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.global::<Ui>().theme.clone();
        let sort = self.sort;
        let widths: Vec<f32> = self.table.columns().iter().map(|c| c.width).collect();
        let last = widths.len() - 1;
        let border = color(theme.role("border"));
        // (title, sort key, alignment) for the name column and each shown column.
        let mut specs = vec![("Name".to_owned(), Some(SortKey::Name), Align::Left)];
        specs.extend(self.columns.iter().map(|c| (c.title().to_owned(), c.sort_key(), c.align())));
        let mut header = div()
            .id("table-header")
            .h(px(28.))
            .flex_none()
            .mx_2()
            .flex()
            .items_center()
            .text_size(px(12.))
            .text_color(color(theme.role("fg-muted")))
            .border_b_1()
            .border_color(border);
        for (i, (title, key, align)) in specs.into_iter().enumerate() {
            let sorted = key.is_some_and(|k| k == sort.key);
            let chevron = match (sorted, sort.ascending) {
                (false, _) => "",
                (true, true) => "⌃",
                (true, false) => "⌄",
            };
            let width = px(widths[i]);
            let cell = div()
                .id(("column", i))
                .role(Role::ColumnHeader)
                .aria_label(format!("{title}{}", if sorted { ", sorted" } else { "" }))
                .relative()
                .h_full()
                .when(i == last, |cell| cell.flex_1().min_w(width))
                .when(i != last, |cell| cell.w(width).flex_none())
                .pl(px(if i == 0 { NAME_INSET } else { CELL_PADDING }))
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
        // Spec: only the focused column shows its selection in the accent color.
        let active = k == self.focused && self.focus.is_focused(window);
        let selection_bg = selection_color(active, theme);
        let subtle = color(theme.role("fg-subtle"));
        let Some(folder) = self.folders.get(k).map(|f| f.read(cx)) else { return Vec::new() };
        let width = column_width(folder.entries());
        let name_room = px(width - 16.0 - 8.0 - 16.0 - 2.0 * CELL_PADDING - 8.0);
        let total = folder.entries().len();
        let cursor = folder.cursor_ix();
        let rows: Vec<(usize, Entry, bool)> = range
            .filter_map(|ix| folder.entries().get(ix).map(|e| (ix, e.clone(), folder.is_selected(&e.path))))
            .collect();
        rows.into_iter()
            .map(|(ix, entry, selected)| {
                let name = truncate_middle(&entry.name, name_room, font_size, window);
                let navigable = entry.is_dir && !entry.is_package();
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
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(selected, |row| row.bg(selection_bg))
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
                    .child(
                        div()
                            .size(px(16.))
                            .flex_none()
                            .children(icon(&entry.path, entry.is_dir, 16).map(|i| img(i).size(px(16.)))),
                    )
                    .child(match self.renaming.as_ref().filter(|r| r.path == entry.path) {
                        Some(r) => div().flex_1().child(r.input.clone()).into_any_element(),
                        None => div().flex_1().overflow_hidden().whitespace_nowrap().child(name).into_any_element(),
                    })
                    .child(div().w(px(10.)).flex_none().text_color(subtle).child(if navigable { "›" } else { "" }))
            })
            .collect()
    }

    fn render_columns(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.global::<Ui>().theme.clone();
        let border = color(theme.role("border"));
        let focused = self.focused;
        let mut strip = div().id("columns").size_full().flex().overflow_x_scroll().track_scroll(&self.strip_scroll);
        for (k, folder) in self.folders.iter().enumerate() {
            let f = folder.read(cx);
            let (count, width, scroll) = (f.entries().len(), column_width(f.entries()), f.scroll().clone());
            let title = f.dir().file_name().map_or_else(|| "/".to_owned(), |n| n.to_string_lossy().into_owned());
            strip = strip.child(
                div()
                    .id(("column-view", k))
                    .role(Role::ListBox)
                    .aria_label(format!("{title}, {count} items"))
                    .w(px(width))
                    .flex_none()
                    .h_full()
                    .py_1()
                    .border_r_1()
                    .border_color(border)
                    .when(k == focused, |c| c.bg(color_alpha(theme.role("surface"), 0x40)))
                    .on_drop(
                        cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                            this.drop_files(k, paths, window, cx)
                        }),
                    )
                    .child(
                        uniform_list(
                            ("column-list", k),
                            count,
                            cx.processor(move |this, range, window, cx| this.render_column_rows(k, range, window, cx)),
                        )
                        .track_scroll(&scroll)
                        .h_full()
                        .px_1(),
                    ),
            );
        }
        // The preview is the last column when the focused column's cursor is a file.
        if let Some(entry) = self.cursor_entry(cx).filter(|e| !e.is_dir || e.is_package()) {
            let preview = self.preview(cx);
            strip = strip.child(
                div()
                    .id("preview-column")
                    .role(Role::Group)
                    .aria_label(preview::spoken(&entry, preview))
                    .w(px(PREVIEW_COLUMN_WIDTH))
                    .flex_none()
                    .h_full()
                    .p_4()
                    .overflow_hidden()
                    .child(preview::contents(&entry, preview, &theme)),
            );
        }
        strip
    }
}

fn selection_color(active: bool, theme: &crow_theme::Theme) -> gpui::Rgba {
    if active { color_alpha(theme.role("accent"), 0x55) } else { color(theme.role("selection")) }
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
        if let Some(path) = self.rename_when_present.clone()
            && self.focused_folder().read(cx).entries().iter().any(|e| e.path == path)
        {
            self.rename_when_present = None;
            // Focus can't move during a render; start the rename right after it.
            cx.defer_in(window, move |this, window, cx| this.begin_rename(path, window, cx));
        }
        let focused = self.focused_folder().read(cx);
        let title =
            focused.dir().file_name().map_or_else(|| "Macintosh HD".to_owned(), |n| n.to_string_lossy().into_owned());
        let count = focused.entries().len();
        let context = match self.mode {
            ViewMode::List => "FileList",
            ViewMode::Columns => "FileList ColumnView",
        };
        let body = match self.mode {
            ViewMode::List => {
                let (rows, scroll) = {
                    let root = self.folders[0].read(cx);
                    (root.entries().len(), root.scroll().clone())
                };
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(self.render_table_header(cx))
                    .child(
                        uniform_list("entries", rows, cx.processor(Self::render_table_rows))
                            .track_scroll(&scroll)
                            .flex_1(),
                    )
                    .into_any_element()
            }
            ViewMode::Columns => self.render_columns(cx).into_any_element(),
        };
        div()
            .id("file-list")
            .role(Role::ListBox)
            .aria_label(format!("{title}, {count} items"))
            .key_context(context)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::move_up))
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
            .on_action(cx.listener(Self::duplicate))
            .on_action(cx.listener(Self::move_to_trash))
            .on_action(cx.listener(Self::delete_permanently))
            .on_action(cx.listener(Self::copy_files))
            .on_action(cx.listener(Self::cut_files))
            .on_action(cx.listener(Self::paste_files))
            .on_action(cx.listener(Self::paste_move_files))
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                if let Some(drag) = this.resize {
                    if event.pressed_button == Some(MouseButton::Left) {
                        this.table.drag_to(&drag, f32::from(event.position.x));
                        cx.notify();
                    } else {
                        this.resize = None;
                    }
                    return;
                }
                let Some((k, ix, origin)) = this.drag_origin else { return };
                if this.dragging || event.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                let (dx, dy) = (event.position.x - origin.x, event.position.y - origin.y);
                if f32::from(dx).hypot(f32::from(dy)) > 5.0 {
                    let Some(folder) = this.folders.get(k).map(|f| f.read(cx)) else { return };
                    // Drag the whole selection when the drag starts on a selected row.
                    let paths = match folder.entries().get(ix) {
                        Some(e) if folder.is_selected(&e.path) => folder.selected_paths(),
                        Some(e) => vec![e.path.clone()],
                        None => return,
                    };
                    this.dragging = crow_macos::begin_drag(window, &paths);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| {
                    this.drag_origin = None;
                    this.dragging = false;
                    this.resize = None;
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| {
                    this.drag_origin = None;
                    this.dragging = false;
                    this.resize = None;
                }),
            )
            .size_full()
            // Wider-than-pane tables are clipped for now; horizontal scrolling comes with view settings.
            .overflow_hidden()
            .child(body)
    }
}

#[cfg(test)]
impl Pane {
    pub fn is_renaming(&self) -> bool {
        self.renaming.is_some()
    }

    pub fn rename_text(&self, cx: &gpui::App) -> Option<String> {
        self.renaming.as_ref().map(|r| r.input.read(cx).text().to_owned())
    }

    pub fn folder_count(&self) -> usize {
        self.folders.len()
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

fn same_volume(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (a.symlink_metadata(), b.metadata()) {
        (Ok(x), Ok(y)) => x.dev() == y.dev(),
        _ => false,
    }
}
