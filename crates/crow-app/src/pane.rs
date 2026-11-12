//! A file list pane: one folder, its selection and history. One per tab.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use crow_core::History;
use crow_fs::{Entry, FolderWatcher, Snapshot};
use gpui::{
    ClickEvent, Context, FocusHandle, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Point, Render, Role, ScrollStrategy, SharedString, Task, UniformListScrollHandle, Window, div, img, prelude::*, px,
    uniform_list,
};

use crate::actions::{MoveDown, MoveToFirst, MoveToLast, MoveUp, OpenSelection, QuickLook, Rename};
use crate::style::{color, color_alpha, format_time, icon, kind_of};
use crate::ui::Ui;

const ROW_HEIGHT: f32 = 24.0;
/// Text previews read at most this much of a file.
const PREVIEW_BYTES: usize = 4096;

pub struct Pane {
    history: History,
    entries: Arc<Vec<Entry>>,
    load_ms: f64,
    loading: bool,
    rescan_pending: bool,
    /// After going up, Finder selects the folder you came from.
    select_after_load: Option<PathBuf>,
    selected: Option<usize>,
    show_hidden: bool,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    fs_tx: Sender<()>,
    fs_rx: Receiver<()>,
    watcher: Option<FolderWatcher>,
    error: Option<SharedString>,
    drag_origin: Option<(usize, Point<Pixels>)>,
    dragging: bool,
    preview_text: Option<(PathBuf, SharedString)>,
    bench: Option<Bench>,
    _poll: Task<()>,
}

struct Bench {
    frame: usize,
    intervals: Vec<f64>,
    last: Option<Instant>,
}

impl Pane {
    pub fn new(dir: PathBuf, show_hidden: bool, bench: bool, cx: &mut Context<Self>) -> Self {
        let (fs_tx, fs_rx) = mpsc::channel();
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                let Ok(()) = this.update(cx, |this, cx| {
                    if this.fs_rx.try_iter().count() > 0 {
                        this.load(cx);
                    }
                }) else {
                    break;
                };
            }
        });
        let mut pane = Self {
            history: History::new(dir.clone()),
            entries: Arc::new(Vec::new()),
            load_ms: 0.0,
            loading: false,
            rescan_pending: false,
            select_after_load: None,
            selected: None,
            show_hidden,
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            fs_tx,
            fs_rx,
            watcher: None,
            error: None,
            drag_origin: None,
            dragging: false,
            preview_text: None,
            bench: bench.then(|| Bench { frame: 0, intervals: Vec::new(), last: None }),
            _poll: poll,
        };
        pane.enter(dir, cx);
        pane
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    pub fn dir(&self) -> &Path {
        self.history.current()
    }

    pub fn item_count(&self) -> usize {
        self.entries.len()
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

    pub fn selected_entry(&self) -> Option<&Entry> {
        self.selected.and_then(|i| self.entries.get(i))
    }

    pub fn preview_text(&self) -> Option<&SharedString> {
        let entry = self.selected_entry()?;
        self.preview_text.as_ref().filter(|(path, _)| *path == entry.path).map(|(_, text)| text)
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
            self.select_after_load = Some(from);
            self.enter(dir, cx);
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

    pub fn set_show_hidden(&mut self, show: bool, cx: &mut Context<Self>) {
        if self.show_hidden != show {
            self.show_hidden = show;
            self.load(cx);
        }
    }

    fn enter(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        self.entries = Arc::new(Vec::new());
        self.selected = None;
        self.preview_text = None;
        self.error = None;
        self.watcher = crow_fs::watch_folder(&dir, self.fs_tx.clone())
            .inspect_err(|e| tracing::warn!("watch {}: {e}", dir.display()))
            .ok();
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.load(cx);
        cx.notify();
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            self.rescan_pending = true;
            return;
        }
        self.loading = true;
        let (dir, hidden) = (self.dir().to_path_buf(), self.show_hidden);
        cx.spawn(async move |this, cx| {
            let snapshot = cx.background_spawn(async move { crow_fs::read_dir(&dir, hidden) }).await;
            this.update(cx, |this, cx| this.apply(snapshot, cx)).ok();
        })
        .detach();
    }

    fn apply(&mut self, snapshot: std::io::Result<Snapshot>, cx: &mut Context<Self>) {
        self.loading = false;
        match snapshot {
            Ok(snapshot) if snapshot.dir == self.dir() => {
                let keep = self.select_after_load.take().or_else(|| self.selected_entry().map(|e| e.path.clone()));
                self.entries = snapshot.entries;
                self.load_ms = snapshot.load_ms;
                self.selected = keep
                    .and_then(|path| self.entries.iter().position(|e| e.path == path))
                    .or((!self.entries.is_empty()).then_some(0));
                if let Some(ix) = self.selected {
                    self.scroll.scroll_to_item(ix, ScrollStrategy::Center);
                }
                tracing::debug!(items = self.entries.len(), ms = self.load_ms, dir = %self.dir().display(), "loaded");
                self.load_preview(cx);
            }
            // A load for a folder we have already left.
            Ok(_) => {}
            Err(e) => {
                tracing::warn!("open {}: {e}", self.dir().display());
                self.error = Some(format!("Can't open “{}”: {e}", self.dir().display()).into());
            }
        }
        if std::mem::take(&mut self.rescan_pending) {
            self.load(cx);
        }
        cx.notify();
    }

    fn load_preview(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self.selected_entry().filter(|e| !e.is_dir && e.size > 0).cloned() else {
            self.preview_text = None;
            return;
        };
        cx.spawn(async move |this, cx| {
            let path = entry.path.clone();
            let text = cx
                .background_spawn(async move {
                    use std::io::Read;
                    let mut head = Vec::with_capacity(PREVIEW_BYTES);
                    std::fs::File::open(&path).ok()?.take(PREVIEW_BYTES as u64).read_to_end(&mut head).ok()?;
                    // Cut at the last full line so a split UTF-8 sequence never reaches the check.
                    let end = head.iter().rposition(|b| *b == b'\n').map_or(head.len(), |i| i + 1);
                    let text = std::str::from_utf8(&head[..end]).ok()?;
                    Some(text.lines().take(40).collect::<Vec<_>>().join("\n"))
                })
                .await;
            this.update(cx, |this, cx| {
                this.preview_text = text.map(|t| (entry.path, t.into()));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.entries.len() || self.selected == Some(ix) {
            return;
        }
        self.selected = Some(ix);
        // Non-strict: scrolls only when the row is out of view, like Finder.
        self.scroll.scroll_to_item(ix, ScrollStrategy::Top);
        if let Some(entry) = self.selected_entry() {
            crow_macos::update_quick_look(&entry.path);
        }
        self.load_preview(cx);
        cx.notify();
    }

    fn open_selected(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self.selected_entry().cloned() else { return };
        if entry.is_dir && !entry.is_package() {
            self.navigate(entry.path, cx);
        } else if let Err(e) = std::process::Command::new("open").arg(&entry.path).spawn() {
            self.error = Some(format!("Can't open “{}”: {e}", entry.name).into());
            cx.notify();
        }
    }

    // ----- actions -----

    fn move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        self.select(self.selected.map_or(0, |i| i.saturating_sub(1)), cx);
    }

    fn move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        let last = self.entries.len().saturating_sub(1);
        self.select(self.selected.map_or(0, |i| (i + 1).min(last)), cx);
    }

    fn move_to_first(&mut self, _: &MoveToFirst, _: &mut Window, cx: &mut Context<Self>) {
        self.select(0, cx);
    }

    fn move_to_last(&mut self, _: &MoveToLast, _: &mut Window, cx: &mut Context<Self>) {
        self.select(self.entries.len().saturating_sub(1), cx);
    }

    fn open(&mut self, _: &OpenSelection, _: &mut Window, cx: &mut Context<Self>) {
        self.open_selected(cx);
    }

    fn rename(&mut self, _: &Rename, _: &mut Window, cx: &mut Context<Self>) {
        // Inline rename arrives with the operation engine in M4.
        self.error = Some("Rename arrives in milestone M4".into());
        cx.notify();
    }

    fn quick_look(&mut self, _: &QuickLook, _: &mut Window, _: &mut Context<Self>) {
        if let Some(entry) = self.selected_entry() {
            crow_macos::toggle_quick_look(&entry.path);
        }
    }

    // ----- rendering -----

    fn bench_step(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(bench) = self.bench.as_mut() else { return };
        window.request_animation_frame();
        if self.entries.is_empty() {
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
        self.selected = Some(ix);
        if bench.frame >= 600 {
            let mut v = std::mem::take(&mut bench.intervals);
            v.sort_by(f64::total_cmp);
            let at = |q: f64| v[((v.len() - 1) as f64 * q) as usize];
            let avg = v.iter().sum::<f64>() / v.len() as f64;
            println!(
                "BENCH items={} load={:.0}ms frames={} avg={avg:.2}ms ({:.0} fps) p50={:.2}ms p95={:.2}ms p99={:.2}ms max={:.2}ms",
                self.entries.len(),
                self.load_ms,
                v.len(),
                1000.0 / avg,
                at(0.5),
                at(0.95),
                at(0.99),
                at(1.0),
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
        let muted = color(theme.role("fg-muted"));
        let selection = if self.focus.is_focused(window) {
            color_alpha(theme.role("accent"), 0x55)
        } else {
            color(theme.role("selection"))
        };
        let total = self.entries.len();
        let entries = self.entries.clone();
        range
            .filter_map(|ix| entries.get(ix).map(|e| (ix, e)))
            .map(|(ix, entry)| {
                let selected = self.selected == Some(ix);
                let size = if entry.is_dir { "--".to_owned() } else { crow_fs::human_size(entry.size) };
                let spoken_size = if entry.is_dir { String::new() } else { format!(", {size}") };
                let modified = format_time(entry.modified);
                div()
                    .id(ix)
                    // Only visible rows exist, so each row tells VoiceOver where it sits in the folder.
                    .role(Role::ListBoxOption)
                    .aria_label(format!("{}, {}{spoken_size}, modified {modified}", entry.name, kind_of(entry)))
                    .aria_position_in_set(ix + 1)
                    .aria_size_of_set(total)
                    .aria_selected(selected)
                    .when(selected, |row| row.aria_active_descendant().bg(selection))
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
                            this.select(ix, cx);
                        }),
                    )
                    .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                        if event.click_count() == 2 {
                            this.selected = Some(ix);
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
                    .child(div().w(px(90.)).flex_none().text_right().text_color(muted).child(size))
                    .child(div().w(px(190.)).flex_none().pl_4().text_color(muted).child(modified))
            })
            .collect()
    }
}

impl Render for Pane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.bench_step(window, cx);
        let theme = cx.global::<Ui>().theme.clone();
        let title =
            self.dir().file_name().map_or_else(|| "Macintosh HD".to_owned(), |n| n.to_string_lossy().into_owned());
        div()
            .id("file-list")
            .role(Role::ListBox)
            .aria_label(format!("{title}, {} items", self.entries.len()))
            .key_context("FileList")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::move_up))
            .on_action(cx.listener(Self::move_down))
            .on_action(cx.listener(Self::move_to_first))
            .on_action(cx.listener(Self::move_to_last))
            .on_action(cx.listener(Self::open))
            .on_action(cx.listener(Self::rename))
            .on_action(cx.listener(Self::quick_look))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, _| {
                let Some((ix, origin)) = this.drag_origin else { return };
                if this.dragging || event.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                let (dx, dy) = (event.position.x - origin.x, event.position.y - origin.y);
                if f32::from(dx).hypot(f32::from(dy)) > 5.0
                    && let Some(entry) = this.entries.get(ix)
                {
                    this.dragging = crow_macos::begin_drag(window, std::slice::from_ref(&entry.path));
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
            .child(
                div()
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
                    .child(div().flex_1().child("Name"))
                    .child(div().w(px(90.)).text_right().child("Size"))
                    .child(div().w(px(190.)).pl_4().child("Date Modified")),
            )
            .child(
                uniform_list("entries", self.entries.len(), cx.processor(Self::render_rows))
                    .track_scroll(&self.scroll)
                    .flex_1(),
            )
    }
}
