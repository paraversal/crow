//! Crow Phase 0 spike: can GPUI carry a Finder-class file browser?
//!
//! Usage:
//!   crow-spike [DIR]                 browse DIR (default: the current directory)
//!   crow-spike --make-fixture DIR N  create N empty files in DIR
//!   crow-spike --bench DIR           auto-scroll DIR for 600 frames, print frame times, quit

mod fsmodel;
mod macos;

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use crow_theme::{Rgb, Theme, Variant};
use fsmodel::{Entry, Snapshot};
use gpui::{
    App, Bounds, ClickEvent, Context, FocusHandle, KeyBinding, Menu, MenuItem, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, Rgba, Role, ScrollStrategy, SharedString, Subscription,
    TitlebarOptions, UniformListScrollHandle, Window, WindowAppearance, WindowBounds, WindowOptions, actions, div, img,
    point, prelude::*, px, rgb, rgba, size, uniform_list,
};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

actions!(crow, [MoveUp, MoveDown, Open, Parent, QuickLook, SwitchPane, ToggleHidden, Rename, Quit]);

const ROW_H: f32 = 24.0;

fn c(color: Rgb) -> Rgba {
    rgb(color.0)
}

fn ca(color: Rgb, alpha: u8) -> Rgba {
    rgba((color.0 << 8) | alpha as u32)
}

fn variant_of(a: WindowAppearance) -> Variant {
    match a {
        WindowAppearance::Light | WindowAppearance::VibrantLight => Variant::Light,
        _ => Variant::Dark,
    }
}

fn fmt_time(t: Option<std::time::SystemTime>) -> String {
    let Some(t) = t else { return "—".into() };
    let dt: chrono::DateTime<chrono::Local> = t.into();
    let today = chrono::Local::now().date_naive();
    if dt.date_naive() == today {
        dt.format("Today at %H:%M").to_string()
    } else {
        dt.format("%-d. %B %Y at %H:%M").to_string()
    }
}

fn kind_of(e: &Entry) -> String {
    if e.is_dir {
        return if e.ext() == "app" { "Application".into() } else { "Folder".into() };
    }
    match e.ext().to_lowercase().as_str() {
        "rs" => "Rust source".into(),
        "md" => "Markdown".into(),
        "toml" => "TOML".into(),
        "json" | "jsonl" => "JSON".into(),
        "txt" => "Plain text".into(),
        "png" | "jpg" | "jpeg" | "heic" | "gif" => "Image".into(),
        "pdf" => "PDF document".into(),
        "" => "Document".into(),
        other => format!("{} file", other.to_uppercase()),
    }
}

#[derive(Default)]
struct FrameStats {
    last: Option<Instant>,
    recent: Vec<f64>,
}

impl FrameStats {
    fn tick(&mut self) {
        let now = Instant::now();
        if let Some(prev) = self.last.replace(now) {
            let ms = (now - prev).as_secs_f64() * 1000.0;
            // Only frames rendered back to back count; idle gaps are not frames.
            if ms < 100.0 {
                self.recent.push(ms);
                if self.recent.len() > 120 {
                    self.recent.remove(0);
                }
            }
        }
    }

    fn fps(&self) -> Option<f64> {
        (self.recent.len() >= 10).then(|| 1000.0 / (self.recent.iter().sum::<f64>() / self.recent.len() as f64))
    }
}

struct Bench {
    frame: usize,
    intervals: Vec<f64>,
    last: Option<Instant>,
}

struct Crow {
    theme: Theme,
    appearance: Variant,
    dir: PathBuf,
    entries: Arc<Vec<Entry>>,
    load_ms: f64,
    loading: bool,
    rescan_pending: bool,
    selected: Option<usize>,
    show_hidden: bool,
    favorites: Vec<(String, PathBuf)>,
    sidebar_sel: usize,
    list_focus: FocusHandle,
    sidebar_focus: FocusHandle,
    scroll: UniformListScrollHandle,
    drag_origin: Option<(usize, Point<Pixels>)>,
    dragging: bool,
    status: SharedString,
    fs_tx: mpsc::Sender<()>,
    fs_rx: mpsc::Receiver<()>,
    fs_watcher: Option<RecommendedWatcher>,
    theme_rx: mpsc::Receiver<()>,
    _theme_watcher: Option<RecommendedWatcher>,
    preview_text: Option<(PathBuf, String)>,
    frames: FrameStats,
    bench: Option<Bench>,
    _appearance_sub: Option<Subscription>,
}

impl Crow {
    fn new(start: PathBuf, bench: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let appearance = variant_of(window.appearance());
        let (fs_tx, fs_rx) = mpsc::channel();
        let (theme_tx, theme_rx) = mpsc::channel();
        let theme_watcher = crow_theme::watch(theme_tx).map_err(|e| eprintln!("theme watch failed: {e}")).ok();
        let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()));
        let cwd = std::env::current_dir().unwrap_or_else(|_| home.clone());
        let mut favorites: Vec<(String, PathBuf)> = vec![
            ("Home".into(), home.clone()),
            ("Applications".into(), PathBuf::from("/Applications")),
            ("file-crow".into(), cwd.clone()),
            ("Macintosh HD".into(), PathBuf::from("/")),
        ];
        let fixture = cwd.join("target/spike-100k");
        if fixture.is_dir() {
            favorites.push(("100k files".into(), fixture));
        }
        let list_focus = cx.focus_handle();
        let sidebar_focus = cx.focus_handle();
        window.focus(&list_focus, cx);

        let sub = cx.observe_window_appearance(window, |this: &mut Self, window, cx| {
            this.appearance = variant_of(window.appearance());
            this.reload_theme(cx);
        });

        // One poll loop for watcher events: cheap, and keeps all state changes on the main thread.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                if this.update(cx, |this, cx| this.poll(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        let mut this = Crow {
            theme: crow_theme::current(None, appearance),
            appearance,
            dir: start.clone(),
            entries: Arc::new(Vec::new()),
            load_ms: 0.0,
            loading: false,
            rescan_pending: false,
            selected: None,
            show_hidden: false,
            favorites,
            sidebar_sel: 0,
            list_focus,
            sidebar_focus,
            scroll: UniformListScrollHandle::new(),
            drag_origin: None,
            dragging: false,
            status: "".into(),
            fs_tx,
            fs_rx,
            fs_watcher: None,
            theme_rx,
            _theme_watcher: theme_watcher,
            preview_text: None,
            frames: FrameStats::default(),
            bench: bench.then(|| Bench { frame: 0, intervals: Vec::new(), last: None }),
            _appearance_sub: Some(sub),
        };
        this.navigate(start, cx);
        this
    }

    // ----- folder loading -----

    fn navigate(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        self.dir = dir.clone();
        self.selected = None;
        self.entries = Arc::new(Vec::new());
        self.preview_text = None;
        let tx = self.fs_tx.clone();
        self.fs_watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if res.is_ok() {
                let _ = tx.send(());
            }
        })
        .and_then(|mut w| w.watch(&dir, RecursiveMode::NonRecursive).map(|_| w))
        .map_err(|e| eprintln!("folder watch failed: {e}"))
        .ok();
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.load(cx);
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            self.rescan_pending = true;
            return;
        }
        self.loading = true;
        let (dir, hidden) = (self.dir.clone(), self.show_hidden);
        cx.spawn(async move |this, cx| {
            let snap = cx.background_executor().spawn(async move { fsmodel::read_dir(&dir, hidden) }).await;
            let _ = this.update(cx, |this, cx| this.apply(snap, cx));
        })
        .detach();
    }

    fn apply(&mut self, snap: std::io::Result<Snapshot>, cx: &mut Context<Self>) {
        self.loading = false;
        match snap {
            Ok(snap) if snap.dir == self.dir => {
                let keep = self.selected_entry().map(|e| e.path.clone());
                self.entries = snap.entries;
                self.load_ms = snap.load_ms;
                self.selected = keep
                    .and_then(|p| self.entries.iter().position(|e| e.path == p))
                    .or((!self.entries.is_empty()).then_some(0));
                eprintln!("loaded {} items from {} in {:.1} ms", self.entries.len(), self.dir.display(), self.load_ms);
                self.refresh_preview();
            }
            Ok(_) => {} // a stale load for a folder we already left
            Err(e) => self.status = format!("Can't open folder: {e}").into(),
        }
        if std::mem::take(&mut self.rescan_pending) {
            self.load(cx);
        }
        cx.notify();
    }

    fn poll(&mut self, cx: &mut Context<Self>) {
        if self.fs_rx.try_iter().count() > 0 {
            self.load(cx);
        }
        if self.theme_rx.try_iter().count() > 0 {
            self.reload_theme(cx);
        }
    }

    fn reload_theme(&mut self, cx: &mut Context<Self>) {
        let (dir, state) = (crow_theme::themes_dir(), crow_theme::state_dir());
        let loaded = crow_theme::resolve_theme_id(None, self.appearance, &dir, &state)
            .ok_or_else(|| "no themes".to_string())
            .and_then(|id| crow_theme::load_theme(&dir, &id));
        match loaded {
            Ok(theme) => {
                eprintln!("theme: {}", theme.name);
                self.theme = theme;
            }
            // Half-written file or bad JSON: keep the theme we have.
            Err(e) => eprintln!("theme reload skipped: {e}"),
        }
        cx.notify();
    }

    fn selected_entry(&self) -> Option<&Entry> {
        self.selected.and_then(|i| self.entries.get(i))
    }

    fn refresh_preview(&mut self) {
        self.preview_text = self.selected_entry().filter(|e| !e.is_dir && e.size < 256_000).and_then(|e| {
            let bytes = std::fs::read(&e.path).ok()?;
            let head = &bytes[..bytes.len().min(4096)];
            let text = std::str::from_utf8(head).ok()?;
            Some((e.path.clone(), text.lines().take(40).collect::<Vec<_>>().join("\n")))
        });
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.entries.len() {
            self.selected = Some(ix);
            // Non-strict: scrolls only if the row is out of view, like Finder.
            self.scroll.scroll_to_item(ix, ScrollStrategy::Top);
            self.refresh_preview();
            if let Some(e) = self.selected_entry() {
                macos::update_quick_look(&e.path);
            }
            cx.notify();
        }
    }

    // ----- actions: file list -----

    fn list_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        let ix = self.selected.map_or(0, |i| i.saturating_sub(1));
        self.select(ix, cx);
    }

    fn list_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        let ix = self.selected.map_or(0, |i| (i + 1).min(self.entries.len().saturating_sub(1)));
        self.select(ix, cx);
    }

    fn list_open(&mut self, _: &Open, _: &mut Window, cx: &mut Context<Self>) {
        self.open_selected(cx);
    }

    fn open_selected(&mut self, cx: &mut Context<Self>) {
        let Some(e) = self.selected_entry().cloned() else { return };
        if e.is_dir && e.ext() != "app" {
            self.navigate(e.path, cx);
        } else {
            let _ = std::process::Command::new("open").arg(&e.path).spawn();
        }
    }

    fn list_rename(&mut self, _: &Rename, _: &mut Window, cx: &mut Context<Self>) {
        self.status = "Return → rename (in the file list context; not built in the spike)".into();
        cx.notify();
    }

    fn quick_look(&mut self, _: &QuickLook, _: &mut Window, _: &mut Context<Self>) {
        if let Some(e) = self.selected_entry() {
            macos::toggle_quick_look(&e.path);
        }
    }

    // ----- actions: sidebar (same keys, different context) -----

    fn sidebar_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_sel = self.sidebar_sel.saturating_sub(1);
        cx.notify();
    }

    fn sidebar_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_sel = (self.sidebar_sel + 1).min(self.favorites.len() - 1);
        cx.notify();
    }

    fn sidebar_open(&mut self, _: &Open, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self.favorites[self.sidebar_sel].1.clone();
        self.navigate(dir, cx);
        window.focus(&self.list_focus, cx);
    }

    // ----- actions: window -----

    fn parent(&mut self, _: &Parent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.dir.parent().map(Path::to_path_buf) {
            let child = self.dir.clone();
            self.navigate(p, cx);
            self.status = format!("up from {}", child.display()).into();
        }
    }

    fn switch_pane(&mut self, _: &SwitchPane, window: &mut Window, cx: &mut Context<Self>) {
        if self.list_focus.is_focused(window) {
            window.focus(&self.sidebar_focus, cx);
        } else {
            window.focus(&self.list_focus, cx);
        }
        cx.notify();
    }

    fn toggle_hidden(&mut self, _: &ToggleHidden, _: &mut Window, cx: &mut Context<Self>) {
        self.show_hidden = !self.show_hidden;
        self.load(cx);
    }

    // ----- rendering -----

    fn bench_step(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(b) = self.bench.as_mut() else { return };
        if self.entries.is_empty() {
            window.request_animation_frame();
            return;
        }
        let now = Instant::now();
        if let Some(prev) = b.last.replace(now) {
            b.intervals.push((now - prev).as_secs_f64() * 1000.0);
        }
        b.frame += 1;
        // ~3 rows per frame: a fast, continuous trackpad scroll.
        let ix = (b.frame * 3) % self.entries.len();
        self.scroll.scroll_to_item_strict(ix, ScrollStrategy::Top);
        self.selected = Some(ix);
        if b.frame >= 600 {
            let mut v = b.intervals.clone();
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let avg = v.iter().sum::<f64>() / v.len() as f64;
            let p = |q: f64| v[((v.len() - 1) as f64 * q) as usize];
            println!(
                "BENCH items={} frames={} avg={:.2}ms ({:.0} fps) p50={:.2}ms p95={:.2}ms p99={:.2}ms max={:.2}ms",
                self.entries.len(),
                v.len(),
                avg,
                1000.0 / avg,
                p(0.5),
                p(0.95),
                p(0.99),
                v[v.len() - 1]
            );
            cx.quit();
        } else {
            window.request_animation_frame();
        }
    }

    fn render_rows(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<impl IntoElement + use<>> {
        let t = &self.theme;
        let focused = self.list_focus.is_focused(window);
        let sel_bg = if focused { ca(t.role("accent"), 0x55) } else { c(t.role("selection")) };
        let total = self.entries.len();
        range
            .filter_map(|ix| self.entries.get(ix).map(|e| (ix, e)))
            .map(|(ix, e)| {
                let selected = self.selected == Some(ix);
                let icon = macos::icon(&e.path, e.is_dir, 16);
                let size = if e.is_dir { String::new() } else { format!(", {}", fsmodel::human_size(e.size)) };
                div()
                    .id(ix)
                    // Only visible rows exist, so each row tells VoiceOver where it sits in the whole folder.
                    .role(Role::ListBoxOption)
                    .aria_label(format!("{}, {}{}, modified {}", e.name, kind_of(e), size, fmt_time(e.modified)))
                    .aria_position_in_set(ix + 1)
                    .aria_size_of_set(total)
                    .aria_selected(selected)
                    .when(selected, |d| d.aria_active_descendant())
                    .h(px(ROW_H))
                    .px_2()
                    .mx_1()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(selected, |d| d.bg(sel_bg))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                            window.focus(&this.list_focus, cx);
                            this.drag_origin = Some((ix, ev.position));
                            this.select(ix, cx);
                        }),
                    )
                    .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                        if ev.click_count() == 2 {
                            this.selected = Some(ix);
                            this.open_selected(cx);
                        }
                    }))
                    .child(div().w(px(16.)).h(px(16.)).flex_none().children(icon.map(|i| img(i).size(px(16.)))))
                    .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().child(e.name.clone()))
                    .child(
                        div()
                            .w(px(90.))
                            .flex_none()
                            .text_right()
                            .text_color(c(t.role("fg-muted")))
                            .child(if e.is_dir { "--".to_string() } else { fsmodel::human_size(e.size) }),
                    )
                    .child(
                        div()
                            .w(px(190.))
                            .flex_none()
                            .pl_4()
                            .text_color(c(t.role("fg-muted")))
                            .child(fmt_time(e.modified)),
                    )
            })
            .collect()
    }

    fn render_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = &self.theme;
        let focused = self.sidebar_focus.is_focused(window);
        let n = self.favorites.len();
        div()
            .id("sidebar")
            .role(Role::ListBox)
            .aria_label("Favorites")
            .key_context("Sidebar")
            .track_focus(&self.sidebar_focus)
            .on_action(cx.listener(Self::sidebar_up))
            .on_action(cx.listener(Self::sidebar_down))
            .on_action(cx.listener(Self::sidebar_open))
            .w(px(200.))
            .flex_none()
            .h_full()
            .p_2()
            .bg(c(t.role("bg-inset")))
            .border_r_1()
            .border_color(c(t.role("border")))
            .flex()
            .flex_col()
            .gap(px(1.))
            .child(div().px_2().pb_1().text_xs().text_color(c(t.role("fg-muted"))).child("Favorites"))
            .children(self.favorites.iter().enumerate().map(|(i, (name, path))| {
                let active = path == &self.dir;
                let cursor = focused && i == self.sidebar_sel;
                let path2 = path.clone();
                div()
                    .id(("fav", i))
                    .role(Role::ListBoxOption)
                    .aria_label(name.clone())
                    .aria_position_in_set(i + 1)
                    .aria_size_of_set(n)
                    .aria_selected(i == self.sidebar_sel)
                    .when(i == self.sidebar_sel, |d| d.aria_active_descendant())
                    .h(px(26.))
                    .px_2()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(active, |d| d.bg(c(t.role("selection"))))
                    .when(cursor, |d| d.bg(ca(t.role("accent"), 0x55)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.sidebar_sel = i;
                        this.navigate(path2.clone(), cx);
                        window.focus(&this.list_focus, cx);
                    }))
                    .children(macos::icon(path, true, 16).map(|i| img(i).size(px(16.))))
                    .child(name.clone())
            }))
    }

    fn render_preview(&self) -> impl IntoElement + use<> {
        let t = &self.theme;
        let muted = c(t.role("fg-muted"));
        let mut pane = div()
            .id("preview")
            .role(Role::Group)
            .aria_label("Preview")
            .w(px(300.))
            .flex_none()
            .h_full()
            .p_4()
            .border_l_1()
            .border_color(c(t.role("border")))
            .flex()
            .flex_col()
            .gap_3()
            .overflow_hidden();
        let Some(e) = self.selected_entry() else { return pane };
        let preview = match &self.preview_text {
            Some((p, text)) if p == &e.path => div()
                .h(px(220.))
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(c(t.role("border")))
                .bg(c(t.role("bg-inset")))
                .overflow_hidden()
                .font_family("Menlo")
                .text_size(px(11.))
                .child(text.clone()),
            _ => div()
                .h(px(220.))
                .flex()
                .items_center()
                .justify_center()
                .children(macos::icon(&e.path, e.is_dir, 128).map(|i| img(i).size(px(128.)))),
        };
        let info_row = |k: &str, v: String| {
            div()
                .flex()
                .justify_between()
                .py_1()
                .border_b_1()
                .border_color(c(t.role("border")))
                .child(div().text_color(muted).child(k.to_string()))
                .child(div().child(v))
        };
        let size = if e.is_dir { "--".into() } else { fsmodel::human_size(e.size) };
        pane = pane
            .aria_label(format!(
                "Preview: {}, {}, {}, created {}, modified {}",
                e.name,
                kind_of(e),
                size,
                fmt_time(e.created),
                fmt_time(e.modified)
            ))
            .child(preview)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(div().text_size(px(15.)).child(e.name.clone()))
                    .child(div().text_color(muted).child(format!("{} · {}", kind_of(e), size))),
            )
            .child(div().pt_2().font_weight(gpui::FontWeight::SEMIBOLD).child("Information"))
            .child(info_row("Created", fmt_time(e.created)))
            .child(info_row("Modified", fmt_time(e.modified)))
            .child(div().pt_2().font_weight(gpui::FontWeight::SEMIBOLD).child("Tags"))
            .child(div().text_color(muted).child("Add Tags…"));
        pane
    }

    fn render_path_bar(&self) -> impl IntoElement + use<> {
        let t = &self.theme;
        let target = self.selected_entry().map(|e| e.path.clone()).unwrap_or_else(|| self.dir.clone());
        let parts = macos::display_components(&target);
        let n = parts.len();
        let fps = self.frames.fps().map(|f| format!(" · {f:.0} fps")).unwrap_or_default();
        let spoken = parts.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ");
        div()
            .id("path-bar")
            .role(Role::Navigation)
            .aria_label(format!("Path: {spoken}"))
            .h(px(28.))
            .flex_none()
            .px_3()
            .flex()
            .items_center()
            .gap_1()
            .border_t_1()
            .border_color(c(t.role("border")))
            .text_size(px(12.))
            .text_color(c(t.role("fg-muted")))
            .children(parts.into_iter().enumerate().map(move |(i, (name, path))| {
                let is_dir = i + 1 < n || path.is_dir();
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .children(macos::icon(&path, is_dir, 14).map(|i| img(i).size(px(14.))))
                    .child(name)
                    .when(i + 1 < n, |d| d.child(div().pl_1().child("›")))
            }))
            .child(div().flex_1())
            .child(format!("{} items · {:.0} ms{} · {}", self.entries.len(), self.load_ms, fps, self.theme.name))
    }
}

impl Render for Crow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frames.tick();
        self.bench_step(window, cx);
        // The tree only exists while an assistive technology (e.g. VoiceOver) is connected.
        if let Some(out) = std::env::var_os("CROW_A11Y_DUMP") {
            if let Some(json) = window.debug_a11y_tree_json() {
                let _ = std::fs::write(out, json);
            }
        }
        let t = self.theme.clone();
        let title = macos::display_components(&self.dir).pop().map(|(n, _)| n).unwrap_or_default();
        let list_focused = self.list_focus.is_focused(window);
        let count = self.entries.len();
        let sidebar = self.render_sidebar(window, cx);
        let preview = self.render_preview();
        let path_bar = self.render_path_bar();

        div()
            .id("crow")
            .role(Role::Application)
            .aria_label("Crow")
            .key_context("Crow")
            .on_action(cx.listener(Self::parent))
            .on_action(cx.listener(Self::switch_pane))
            .on_action(cx.listener(Self::toggle_hidden))
            .size_full()
            .flex()
            .flex_col()
            .bg(c(t.role("bg")))
            .text_color(c(t.role("fg")))
            .font_family(".SystemUIFont")
            .text_size(px(13.))
            .child(
                // Title bar: traffic lights sit in the left 80 px.
                div()
                    .h(px(44.))
                    .flex_none()
                    .pl(px(90.))
                    .flex()
                    .items_center()
                    .gap_3()
                    .border_b_1()
                    .border_color(c(t.role("border")))
                    .child(
                        div()
                            .id("title")
                            .role(Role::Heading)
                            .aria_level(1)
                            .aria_label(title.clone())
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_size(px(14.))
                            .child(title.clone()),
                    )
                    .child(div().text_color(c(t.role("fg-muted"))).text_size(px(12.)).child(self.status.clone())),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(sidebar)
                    .child(
                        div()
                            .id("file-list")
                            .role(Role::ListBox)
                            .aria_label(format!("{title}, {count} items"))
                            .key_context("FileList")
                            .track_focus(&self.list_focus)
                            .on_action(cx.listener(Self::list_up))
                            .on_action(cx.listener(Self::list_down))
                            .on_action(cx.listener(Self::list_open))
                            .on_action(cx.listener(Self::list_rename))
                            .on_action(cx.listener(Self::quick_look))
                            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, window, _| {
                                let Some((ix, origin)) = this.drag_origin else { return };
                                if this.dragging || ev.pressed_button != Some(MouseButton::Left) {
                                    return;
                                }
                                let (dx, dy) = (ev.position.x - origin.x, ev.position.y - origin.y);
                                if f32::from(dx).hypot(f32::from(dy)) > 5.0 {
                                    if let Some(e) = this.entries.get(ix) {
                                        this.dragging = macos::begin_drag(window, &[e.path.clone()]);
                                    }
                                }
                            }))
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseUpEvent, _, _| {
                                    this.drag_origin = None;
                                    this.dragging = false;
                                }),
                            )
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .when(list_focused, |d| d.bg(c(t.role("bg"))))
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
                                    .text_color(c(t.role("fg-muted")))
                                    .border_b_1()
                                    .border_color(c(t.role("border")))
                                    .child(div().w(px(16.)))
                                    .child(div().flex_1().child("Name"))
                                    .child(div().w(px(90.)).text_right().child("Size"))
                                    .child(div().w(px(190.)).pl_4().child("Date Modified")),
                            )
                            .child(
                                uniform_list("entries", count, cx.processor(Self::render_rows))
                                    .track_scroll(&self.scroll)
                                    .flex_1(),
                            ),
                    )
                    .child(preview),
            )
            .child(path_bar)
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--make-fixture") {
        let dir = PathBuf::from(args.get(1).expect("--make-fixture DIR N"));
        let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(100_000);
        let start = Instant::now();
        fsmodel::make_fixture(&dir, n).expect("create fixture");
        println!("created {n} files in {} in {:.1}s", dir.display(), start.elapsed().as_secs_f64());
        return;
    }
    let bench = args.iter().any(|a| a == "--bench");
    let start = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap())
        .canonicalize()
        .expect("start folder exists");

    gpui_platform::application().run(move |cx: &mut App| {
        cx.bind_keys([
            // Same keys, different meaning per context: that is what the spike checks.
            KeyBinding::new("up", MoveUp, Some("FileList")),
            KeyBinding::new("down", MoveDown, Some("FileList")),
            KeyBinding::new("cmd-down", Open, Some("FileList")),
            KeyBinding::new("enter", Rename, Some("FileList")),
            KeyBinding::new("space", QuickLook, Some("FileList")),
            KeyBinding::new("up", MoveUp, Some("Sidebar")),
            KeyBinding::new("down", MoveDown, Some("Sidebar")),
            KeyBinding::new("enter", Open, Some("Sidebar")),
            KeyBinding::new("cmd-up", Parent, Some("Crow")),
            KeyBinding::new("tab", SwitchPane, Some("Crow")),
            KeyBinding::new("cmd-shift-.", ToggleHidden, Some("Crow")),
            KeyBinding::new("cmd-q", Quit, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.set_menus(vec![Menu {
            name: "Crow Spike".into(),
            items: vec![MenuItem::action("Quit Crow Spike", Quit)],
            disabled: false,
        }]);
        let bounds = Bounds::centered(None, size(px(1200.), px(760.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Crow".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(14.), px(14.))),
                }),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Crow::new(start.clone(), bench, window, cx)),
        )
        .unwrap();
        cx.activate(true);
    });
}
