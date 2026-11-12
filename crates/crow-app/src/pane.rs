//! A file list pane: one folder, its selection, sort and history. One per tab.

use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use crow_core::{Align, ColumnContext, History, PreviewContent, Selection, TypeSelect};
use crow_fs::{Entry, FolderChange, FolderWatcher, Sort, SortKey};
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{
    ClickEvent, Context, FocusHandle, Image, ImageFormat, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, Role, ScrollStrategy, SharedString, Task,
    UniformListScrollHandle, Window, div, img, prelude::*, px, uniform_list,
};

use crate::actions::{
    ExtendDown, ExtendUp, MoveDown, MoveToFirst, MoveToLast, MoveUp, OpenSelection, QuickLook, Rename, SelectAll,
};
use crate::extensions::Ext;
use crate::style::{color, color_alpha, format_time, icon, kind};
use crate::ui::Ui;

const ROW_HEIGHT: f32 = 24.0;
/// Show what has been read so far at most this often while a big folder loads.
const PARTIAL_EVERY: Duration = Duration::from_millis(50);
/// More changes than this at once: relisting is cheaper than diffing.
const MAX_DIFF: usize = 500;

enum Load {
    Partial(Vec<Entry>),
    Done(std::io::Result<(Vec<Entry>, f64)>),
}

/// What the preview pane shows for the cursor item, loaded off the main thread.
pub struct Preview {
    pub path: PathBuf,
    pub content: Option<PreviewContent>,
    pub image: Option<Arc<Image>>,
    pub tags: Vec<String>,
}

pub struct Pane {
    history: History,
    entries: Arc<Vec<Entry>>,
    load_ms: f64,
    first_paint_ms: Option<f64>,
    loading: bool,
    /// Increments per load, so results from an earlier load are dropped.
    generation: u64,
    sort: Sort,
    selection: Selection<PathBuf>,
    /// Row of the selection's cursor in `entries`, kept in step with it.
    cursor_ix: Option<usize>,
    /// After going up or back, Finder selects the folder you came from.
    select_after_load: Option<PathBuf>,
    type_select: TypeSelect,
    show_hidden: bool,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    changes_tx: Sender<FolderChange>,
    changes_rx: Receiver<FolderChange>,
    watcher: Option<FolderWatcher>,
    error: Option<SharedString>,
    drag_origin: Option<(usize, Point<Pixels>)>,
    dragging: bool,
    preview: Option<Preview>,
    bench: Option<Bench>,
    _poll: Task<()>,
}

struct Bench {
    frame: usize,
    intervals: Vec<f64>,
    last: Option<Instant>,
    /// Scroll results, kept while the folder is reopened to time first paint in a warm app.
    scrolled: Option<String>,
}

impl Pane {
    pub fn new(dir: PathBuf, show_hidden: bool, bench: bool, cx: &mut Context<Self>) -> Self {
        let (changes_tx, changes_rx) = mpsc::channel();
        // 100 ms between checks doubles as the debounce for bursts of file changes.
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                if this.update(cx, |this, cx| this.apply_folder_changes(cx)).is_err() {
                    break;
                }
            }
        });
        let mut pane = Self {
            history: History::new(dir.clone()),
            entries: Arc::new(Vec::new()),
            load_ms: 0.0,
            first_paint_ms: None,
            loading: false,
            generation: 0,
            sort: Sort::default(),
            selection: Selection::default(),
            cursor_ix: None,
            select_after_load: None,
            type_select: TypeSelect::default(),
            show_hidden,
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            changes_tx,
            changes_rx,
            watcher: None,
            error: None,
            drag_origin: None,
            dragging: false,
            preview: None,
            bench: bench.then(|| Bench { frame: 0, intervals: Vec::new(), last: None, scrolled: None }),
            _poll: poll,
        };
        pane.enter(dir, cx);
        pane
    }

    // ----- read access for the window -----

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    pub fn dir(&self) -> &Path {
        self.history.current()
    }

    pub fn item_count(&self) -> usize {
        self.entries.len()
    }

    pub fn selected_count(&self) -> usize {
        self.selection.len()
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    pub fn can_go_back(&self) -> bool {
        self.history.can_go_back()
    }

    pub fn can_go_forward(&self) -> bool {
        self.history.can_go_forward()
    }

    /// The item the preview and path bar describe.
    pub fn cursor_entry(&self) -> Option<&Entry> {
        self.cursor_ix.and_then(|i| self.entries.get(i))
    }

    pub fn preview(&self) -> Option<&Preview> {
        self.preview.as_ref().filter(|p| self.cursor_entry().is_some_and(|e| e.path == p.path))
    }

    #[cfg(test)]
    pub fn selected_names(&self) -> Vec<String> {
        let mut names: Vec<String> =
            self.entries.iter().filter(|e| self.selection.contains(&e.path)).map(|e| e.name.clone()).collect();
        names.sort();
        names
    }

    #[cfg(test)]
    pub fn names(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.name.clone()).collect()
    }

    // ----- navigation -----

    /// Go to `dir` as a new history step.
    pub fn navigate(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        self.history.visit(dir.clone());
        self.enter(dir, cx);
    }

    pub fn go_back(&mut self, cx: &mut Context<Self>) {
        let from = self.dir().to_path_buf();
        if let Some(dir) = self.history.back().map(Path::to_path_buf) {
            self.enter(dir, cx);
            self.select_after_load = Some(from);
        }
    }

    pub fn go_forward(&mut self, cx: &mut Context<Self>) {
        if let Some(dir) = self.history.forward().map(Path::to_path_buf) {
            self.enter(dir, cx);
        }
    }

    pub fn open_parent(&mut self, cx: &mut Context<Self>) {
        let child = self.dir().to_path_buf();
        if let Some(parent) = child.parent().map(Path::to_path_buf) {
            self.navigate(parent, cx);
            self.select_after_load = Some(child);
        }
    }

    /// Show `path` selected in its folder (Go to Folder with a file, Finder's "Reveal").
    pub fn reveal(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if let Some(parent) = path.parent().map(Path::to_path_buf) {
            self.navigate(parent, cx);
            self.select_after_load = Some(path);
        }
    }

    pub fn set_show_hidden(&mut self, show: bool, cx: &mut Context<Self>) {
        if self.show_hidden != show {
            self.show_hidden = show;
            self.load(cx);
        }
    }

    pub fn set_sort(&mut self, sort: Sort, cx: &mut Context<Self>) {
        if self.sort == sort {
            return;
        }
        self.sort = sort;
        let mut entries = self.entries.as_ref().clone();
        cx.spawn(async move |this, cx| {
            let entries = cx
                .background_spawn(async move {
                    crow_fs::sort_entries(&mut entries, &sort, &kind);
                    entries
                })
                .await;
            this.update(cx, |this, cx| {
                if this.sort == sort {
                    this.replace_entries(entries, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn enter(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        self.entries = Arc::new(Vec::new());
        self.selection.clear();
        self.cursor_ix = None;
        self.select_after_load = None;
        self.preview = None;
        self.error = None;
        self.first_paint_ms = None;
        self.watcher = crow_fs::watch_folder(&dir, self.changes_tx.clone())
            .inspect_err(|e| tracing::warn!("watch {}: {e}", dir.display()))
            .ok();
        // Changes queued for the folder we are leaving no longer apply.
        while self.changes_rx.try_recv().is_ok() {}
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.load(cx);
        cx.notify();
    }

    // ----- loading -----

    /// List the folder on a background thread. Big folders appear in stages:
    /// what has been read so far is shown (sorted) every 50 ms until done.
    fn load(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self.loading = true;
        let (generation, dir, hidden, sort) = (self.generation, self.dir().to_path_buf(), self.show_hidden, self.sort);
        let (tx, mut rx) = unbounded();
        let started = Instant::now();
        cx.background_spawn(async move { list_in_stages(&dir, hidden, sort, started, &tx) }).detach();
        cx.spawn(async move |this, cx| {
            while let Some(update) = rx.next().await {
                let current = this.update(cx, |this, cx| {
                    if this.generation != generation {
                        return false;
                    }
                    if this.first_paint_ms.is_none() {
                        this.first_paint_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
                    }
                    match update {
                        Load::Partial(entries) => this.replace_entries(entries, cx),
                        Load::Done(Ok((entries, ms))) => {
                            this.loading = false;
                            this.load_ms = ms;
                            tracing::debug!(items = entries.len(), ms, first_paint = ?this.first_paint_ms, "loaded");
                            this.replace_entries(entries, cx);
                        }
                        Load::Done(Err(e)) => {
                            this.loading = false;
                            tracing::warn!("open {}: {e}", this.dir().display());
                            this.error = Some(format!("Can't open “{}”: {e}", this.dir().display()).into());
                            cx.notify();
                        }
                    }
                    true
                });
                if !matches!(current, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    /// Swap in a new listing, keeping the selection on the same items.
    fn replace_entries(&mut self, entries: Vec<Entry>, cx: &mut Context<Self>) {
        self.entries = Arc::new(entries);
        let present: HashSet<&PathBuf> = self.entries.iter().map(|e| &e.path).collect();
        self.selection.retain(|p| present.contains(p));
        if let Some(target) = self.select_after_load.clone()
            && present.contains(&target)
        {
            self.select_after_load = None;
            self.selection.select_only(target);
        }
        if self.selection.is_empty()
            && !self.loading
            && let Some(first) = self.entries.first()
        {
            self.selection.select_only(first.path.clone());
        }
        let previous = self.cursor_ix;
        self.cursor_ix = self.selection.cursor().and_then(|c| self.entries.iter().position(|e| &e.path == c));
        if self.cursor_ix != previous
            && let Some(ix) = self.cursor_ix
        {
            self.scroll.scroll_to_item(ix, ScrollStrategy::Center);
        }
        self.load_preview(cx);
        cx.notify();
    }

    fn apply_folder_changes(&mut self, cx: &mut Context<Self>) {
        let mut paths = Vec::new();
        let mut rescan = false;
        for change in self.changes_rx.try_iter() {
            match change {
                FolderChange::Paths(p) => paths.extend(p),
                FolderChange::Rescan => rescan = true,
            }
        }
        if paths.is_empty() && !rescan {
            return;
        }
        if rescan || self.loading || paths.len() > MAX_DIFF {
            self.load(cx);
            return;
        }
        let (entries, dir, hidden, sort, generation) =
            (self.entries.clone(), self.dir().to_path_buf(), self.show_hidden, self.sort, self.generation);
        cx.spawn(async move |this, cx| {
            let updated = cx
                .background_spawn(async move { crow_fs::apply_changes(&entries, &dir, &paths, hidden, &sort, &kind) })
                .await;
            this.update(cx, |this, cx| {
                if this.generation == generation && this.sort == sort {
                    this.replace_entries(updated, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn load_preview(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self.cursor_entry().cloned() else {
            self.preview = None;
            return;
        };
        if self.preview.as_ref().is_some_and(|p| p.path == entry.path) {
            return;
        }
        let extensions = cx.global::<Ext>().0.clone();
        cx.spawn(async move |this, cx| {
            let path = entry.path.clone();
            let (content, tags) =
                cx.background_spawn(async move { (extensions.preview(&entry), crow_macos::tags(&entry.path)) }).await;
            let image = match &content {
                Some(PreviewContent::Image(png)) => Some(Arc::new(Image::from_bytes(ImageFormat::Png, png.to_vec()))),
                _ => None,
            };
            this.update(cx, |this, cx| {
                if this.cursor_entry().is_some_and(|e| e.path == path) {
                    this.preview = Some(Preview { path, content, image, tags });
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    // ----- selection -----

    fn set_cursor(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.cursor_ix = Some(ix);
        // Non-strict: scrolls only when the row is out of view, like Finder.
        self.scroll.scroll_to_item(ix, ScrollStrategy::Top);
        if let Some(entry) = self.entries.get(ix) {
            crow_macos::update_quick_look(&entry.path);
        }
        self.load_preview(cx);
        cx.notify();
    }

    fn select_only(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(entry) = self.entries.get(ix) else { return };
        self.selection.select_only(entry.path.clone());
        self.set_cursor(ix, cx);
    }

    fn toggle(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(entry) = self.entries.get(ix) else { return };
        self.selection.toggle(entry.path.clone());
        let cursor = self.selection.cursor().and_then(|c| self.entries.iter().position(|e| &e.path == c));
        match cursor {
            Some(ix) => self.set_cursor(ix, cx),
            None => {
                self.cursor_ix = None;
                cx.notify();
            }
        }
    }

    fn extend_to(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.entries.len() {
            return;
        }
        let entries = self.entries.clone();
        self.selection.extend_to(ix, |p| entries.iter().position(|e| &e.path == p), |i| entries[i].path.clone());
        self.set_cursor(ix, cx);
    }

    fn step(&self, delta: isize) -> usize {
        let last = self.entries.len().saturating_sub(1);
        match self.cursor_ix {
            Some(ix) => ix.saturating_add_signed(delta).min(last),
            None if delta < 0 => last,
            None => 0,
        }
    }

    fn open_selected(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self.cursor_entry().cloned() else { return };
        if entry.is_dir && !entry.is_package() {
            self.navigate(entry.path, cx);
        } else if let Err(e) = std::process::Command::new("open").arg(&entry.path).spawn() {
            self.error = Some(format!("Can't open “{}”: {e}", entry.name).into());
            cx.notify();
        }
    }

    // ----- actions -----

    fn move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        self.select_only(self.step(-1), cx);
    }

    fn move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        self.select_only(self.step(1), cx);
    }

    fn extend_up(&mut self, _: &ExtendUp, _: &mut Window, cx: &mut Context<Self>) {
        self.extend_to(self.step(-1), cx);
    }

    fn extend_down(&mut self, _: &ExtendDown, _: &mut Window, cx: &mut Context<Self>) {
        self.extend_to(self.step(1), cx);
    }

    fn move_to_first(&mut self, _: &MoveToFirst, _: &mut Window, cx: &mut Context<Self>) {
        self.select_only(0, cx);
    }

    fn move_to_last(&mut self, _: &MoveToLast, _: &mut Window, cx: &mut Context<Self>) {
        self.select_only(self.entries.len().saturating_sub(1), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selection.select_all(self.entries.iter().map(|e| e.path.clone()));
        cx.notify();
    }

    fn open(&mut self, _: &OpenSelection, _: &mut Window, cx: &mut Context<Self>) {
        self.open_selected(cx);
    }

    fn rename(&mut self, _: &Rename, _: &mut Window, cx: &mut Context<Self>) {
        // Inline rename arrives with the operation engine in M4.
        self.error = Some("Rename arrives in milestone M4".into());
        cx.notify();
    }

    fn quick_look(&mut self, _: &QuickLook, _: &mut Window, cx: &mut Context<Self>) {
        // Mid-word, Space belongs to type-to-select ("my file"), not Quick Look.
        if self.type_select.is_active(Instant::now()) {
            self.type_char(' ', cx);
            return;
        }
        if let Some(entry) = self.cursor_entry() {
            crow_macos::toggle_quick_look(&entry.path);
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let m = &event.keystroke.modifiers;
        if m.platform || m.control || m.alt {
            return;
        }
        let Some(c) = event.keystroke.key_char.as_deref().and_then(|s| s.chars().next()) else { return };
        if c.is_control() || (c == ' ' && !self.type_select.is_active(Instant::now())) {
            return;
        }
        self.type_char(c, cx);
        cx.stop_propagation();
    }

    fn type_char(&mut self, c: char, cx: &mut Context<Self>) {
        let prefix = self.type_select.push(c, Instant::now()).to_owned();
        if let Some(ix) = TypeSelect::find(&prefix, self.entries.iter().map(|e| e.name.as_str())) {
            self.select_only(ix, cx);
        }
    }

    // ----- rendering -----

    fn bench_step(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(bench) = self.bench.as_mut() else { return };
        window.request_animation_frame();
        if self.loading || self.entries.is_empty() {
            return;
        }
        let now = Instant::now();
        if let Some(prev) = bench.last.replace(now) {
            bench.intervals.push((now - prev).as_secs_f64() * 1000.0);
        }
        bench.frame += 1;
        // About 3 rows per frame: a fast, continuous trackpad scroll.
        let ix = (bench.frame * 3) % self.entries.len();
        self.scroll.scroll_to_item_strict(ix, ScrollStrategy::Top);
        if bench.frame == 600 {
            let mut v = std::mem::take(&mut bench.intervals);
            v.sort_by(f64::total_cmp);
            let at = |q: f64| v[((v.len() - 1) as f64 * q) as usize];
            let avg = v.iter().sum::<f64>() / v.len() as f64;
            bench.scrolled = Some(format!(
                "items={} cold_first_paint={:.0}ms cold_load={:.0}ms frames={} avg={avg:.2}ms ({:.0} fps) p50={:.2}ms p95={:.2}ms p99={:.2}ms max={:.2}ms",
                self.entries.len(),
                self.first_paint_ms.unwrap_or_default(),
                self.load_ms,
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
                self.first_paint_ms.unwrap_or_default(),
                self.load_ms
            );
            cx.quit();
        }
    }

    fn render_rows(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<impl IntoElement + use<>> {
        let theme = &cx.global::<Ui>().theme;
        let extensions = cx.global::<Ext>().0.clone();
        let muted = color(theme.role("fg-muted"));
        let focused = self.focus.is_focused(window);
        let selection_bg =
            if focused { color_alpha(theme.role("accent"), 0x55) } else { color(theme.role("selection")) };
        let column_cx = ColumnContext { kind: &kind, time: &format_time };
        let total = self.entries.len();
        let entries = self.entries.clone();
        range
            .filter_map(|ix| entries.get(ix).map(|e| (ix, e)))
            .map(|(ix, entry)| {
                let selected = self.selection.contains(&entry.path);
                let is_cursor = self.cursor_ix == Some(ix);
                let cells: Vec<(String, f32, Align)> =
                    extensions.columns().iter().map(|c| (c.text(entry, &column_cx), c.width(), c.align())).collect();
                let spoken =
                    cells.iter().map(|(text, ..)| text.as_str()).filter(|t| *t != "--").collect::<Vec<_>>().join(", ");
                div()
                    .id(ix)
                    // Only visible rows exist, so each row tells VoiceOver where it sits in the folder.
                    .role(Role::ListBoxOption)
                    .aria_label(format!("{}, {spoken}", entry.name))
                    .aria_position_in_set(ix + 1)
                    .aria_size_of_set(total)
                    .aria_selected(selected)
                    .when(is_cursor, |row| row.aria_active_descendant())
                    .when(selected, |row| row.bg(selection_bg))
                    .h(px(ROW_HEIGHT))
                    .px_2()
                    .mx_1()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .gap_2()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            window.focus(&this.focus, cx);
                            this.drag_origin = Some((ix, event.position));
                            if event.modifiers.platform {
                                this.toggle(ix, cx);
                            } else if event.modifiers.shift {
                                this.extend_to(ix, cx);
                            } else if !this.entries.get(ix).is_some_and(|e| this.selection.contains(&e.path)) {
                                // Clicking inside a multi-selection keeps it, so it can be dragged.
                                this.select_only(ix, cx);
                            } else {
                                this.set_cursor(ix, cx);
                            }
                        }),
                    )
                    .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                        if event.click_count() == 2 {
                            this.select_only(ix, cx);
                            this.open_selected(cx);
                        }
                    }))
                    .child(
                        div()
                            .size(px(16.))
                            .flex_none()
                            .children(icon(&entry.path, entry.is_dir, 16).map(|i| img(i).size(px(16.)))),
                    )
                    .child(
                        div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().child(entry.name.clone()),
                    )
                    .children(cells.into_iter().map(move |(text, width, align)| {
                        div()
                            .w(px(width))
                            .flex_none()
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

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.global::<Ui>().theme.clone();
        let extensions = cx.global::<Ext>().0.clone();
        let sort = self.sort;
        let arrow = move |key: SortKey| match (sort.key == key, sort.ascending) {
            (false, _) => "",
            (true, true) => " ▲",
            (true, false) => " ▼",
        };
        let header_cell = |id: &'static str, title: String, key: SortKey, cx: &mut Context<Self>| {
            div()
                .id(id)
                .role(gpui::Role::ColumnHeader)
                .aria_label(format!("{title}{}", if sort.key == key { ", sorted" } else { "" }))
                .when(sort.key == key, |cell| cell.text_color(color(theme.role("fg"))))
                .on_click(cx.listener(move |this, _, _, cx| this.set_sort(this.sort.toggled(key), cx)))
                .child(format!("{title}{}", arrow(key)))
        };
        let mut header = div()
            .h(px(26.))
            .flex_none()
            .px_3()
            .mx_1()
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(12.))
            .text_color(color(theme.role("fg-muted")))
            .border_b_1()
            .border_color(color(theme.role("border")))
            .child(div().w(px(16.)))
            .child(header_cell("sort-name", "Name".into(), SortKey::Name, cx).flex_1());
        for column in extensions.columns() {
            let cell = match column.sort_key() {
                Some(key) => header_cell(column.id(), column.title().to_owned(), key, cx).into_any_element(),
                None => div().child(column.title().to_owned()).into_any_element(),
            };
            header = header.child(
                div()
                    .w(px(column.width()))
                    .flex_none()
                    .when(column.align() == Align::Right, |c| c.flex().justify_end())
                    .child(cell),
            );
        }
        header
    }
}

/// Runs on a background thread: list `dir`, sending sorted partial listings at most every
/// 50 ms, then the full sorted listing.
fn list_in_stages(dir: &Path, hidden: bool, sort: Sort, started: Instant, tx: &UnboundedSender<Load>) {
    let mut all: Vec<Entry> = Vec::new();
    let mut last_partial = started;
    let result = crow_fs::list_dir(dir, hidden, |batch| {
        all.extend(batch);
        if last_partial.elapsed() >= PARTIAL_EVERY || (last_partial == started && !all.is_empty()) {
            last_partial = Instant::now();
            let mut partial = all.clone();
            crow_fs::sort_entries(&mut partial, &sort, &kind);
            // The pane may have moved on; nobody listening is fine.
            tx.unbounded_send(Load::Partial(partial)).ok();
        }
    });
    let done = result.map(|()| {
        crow_fs::sort_entries(&mut all, &sort, &kind);
        (all, started.elapsed().as_secs_f64() * 1000.0)
    });
    tx.unbounded_send(Load::Done(done)).ok();
}

impl Render for Pane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.bench_step(window, cx);
        let title =
            self.dir().file_name().map_or_else(|| "Macintosh HD".to_owned(), |n| n.to_string_lossy().into_owned());
        let header = self.render_header(cx);
        div()
            .id("file-list")
            .role(Role::ListBox)
            .aria_label(format!("{title}, {} items", self.entries.len()))
            .key_context("FileList")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::move_up))
            .on_action(cx.listener(Self::move_down))
            .on_action(cx.listener(Self::extend_up))
            .on_action(cx.listener(Self::extend_down))
            .on_action(cx.listener(Self::move_to_first))
            .on_action(cx.listener(Self::move_to_last))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::open))
            .on_action(cx.listener(Self::rename))
            .on_action(cx.listener(Self::quick_look))
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, _| {
                let Some((ix, origin)) = this.drag_origin else { return };
                if this.dragging || event.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                let (dx, dy) = (event.position.x - origin.x, event.position.y - origin.y);
                if f32::from(dx).hypot(f32::from(dy)) > 5.0 {
                    // Drag the whole selection when the drag starts on a selected row.
                    let paths: Vec<PathBuf> = match this.entries.get(ix) {
                        Some(e) if this.selection.contains(&e.path) => this
                            .entries
                            .iter()
                            .filter(|e| this.selection.contains(&e.path))
                            .map(|e| e.path.clone())
                            .collect(),
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
                }),
            )
            .size_full()
            .flex()
            .flex_col()
            .child(header)
            .child(
                uniform_list("entries", self.entries.len(), cx.processor(Self::render_rows))
                    .track_scroll(&self.scroll)
                    .flex_1(),
            )
    }
}
