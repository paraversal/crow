//! The bottom bar's data and rendering: the path, the transient slot (hints,
//! progress, messages, errors) and the counts. `CrowWindow::bar_state` builds
//! the state; rendering here reads only that.

use std::path::PathBuf;

use gpui::{App, Entity, IntoElement, Role, SharedString, div, img, prelude::*, px};

use crate::chrome::with_tooltip;
use crate::operations::{self, Running};
use crate::pane::Pane;
use crate::style::{color, icon};
use crate::ui::Ui;
use crow_fs::human_size;

/// Everything the bar draws this frame, computed on the main thread.
pub struct BarState {
    pub primary: Primary,
    pub error: Option<BarError>,
    pub count: Count,
    pub path: Path,
}

/// The transient slot's one primary item.
// `None` and the hint/progress fields arrive with the next items.
#[allow(dead_code)]
pub enum Primary {
    None,
    /// Vim's mark pending: "Mark this folder: type a letter".
    Pending(&'static str),
    Running(RunningView),
    /// An operation or clipboard message.
    Message(SharedString),
    /// Keys for the focused element (U4 fills these).
    Hints(Vec<Hint>),
}

#[allow(dead_code)] // `fraction` and `percent` draw the line and the aria value in U5's next step.
pub struct RunningView {
    pub label: SharedString,
    pub detail: SharedString,
    pub fraction: f32,
    pub percent: u8,
}

impl RunningView {
    pub fn new(running: &Running) -> Self {
        let p = &running.progress;
        let (done, total) =
            if p.total_bytes > 0 { (p.done_bytes, p.total_bytes) } else { (p.done_items as u64, p.total_items as u64) };
        let fraction = if total == 0 { 0.0 } else { (done as f32 / total as f32).clamp(0.0, 1.0) };
        let percent = (fraction * 100.0).round() as u8;
        let detail = if p.total_bytes > 0 {
            format!("{} · {percent} %", crow_fs::human_size_pair(p.done_bytes, p.total_bytes))
        } else if p.total_items == 0 {
            String::new()
        } else {
            format!("{} of {} items · {percent} %", p.done_items, p.total_items)
        };
        Self { label: operations::running_label(&running.label).into(), detail: detail.into(), fraction, percent }
    }
}

#[allow(dead_code)] // U5's hint step fills and reads these.
pub struct Hint {
    pub keys: SharedString,
    pub spoken: SharedString,
    pub label: &'static str,
    pub idle: bool,
}

pub struct BarError {
    /// Short message, file name first and the config dir stripped.
    pub text: SharedString,
    /// The config file at fault, when Open can help.
    #[allow(dead_code)] // U5's error token draws the Open button from this.
    pub file: Option<PathBuf>,
}

pub struct Count {
    pub text: SharedString,
    pub spoken: SharedString,
}

/// The path slot's data: the segments and the spoken form.
pub struct Path {
    pub parts: Vec<(String, PathBuf)>,
    pub last_is_dir: bool,
    pub spoken: String,
}

fn items(n: usize) -> String {
    if n == 1 { "1 item".to_owned() } else { format!("{n} items") }
}

fn size_suffix(bytes: u64) -> String {
    if bytes == 0 { String::new() } else { format!(" · {}", human_size(bytes)) }
}

fn spoken_size(bytes: u64) -> String {
    if bytes == 0 { String::new() } else { format!(", {}", human_size(bytes)) }
}

/// Counts for the list or a column.
pub struct PaneCounts {
    pub loading: bool,
    pub filtering: bool,
    /// Visible rows while filtering.
    pub shown: usize,
    pub total: usize,
    pub selected: usize,
    pub bytes: u64,
    pub folder: String,
}

pub fn pane_count(c: PaneCounts) -> Count {
    if c.loading && c.total == 0 {
        return Count { text: "Loading…".into(), spoken: format!("Loading {}", c.folder).into() };
    }
    if c.filtering {
        let selected = if c.selected >= 2 { format!(" · {} selected", c.selected) } else { String::new() };
        let selected_spoken = if c.selected >= 2 { format!(", {} selected", c.selected) } else { String::new() };
        return Count {
            text: format!("{} of {} shown{selected}{}", c.shown, c.total, size_suffix(c.bytes)).into(),
            spoken: format!(
                "{} of {} shown in {}{selected_spoken}{}",
                c.shown,
                c.total,
                c.folder,
                spoken_size(c.bytes)
            )
            .into(),
        };
    }
    if c.selected >= 2 {
        return Count {
            text: format!("{} selected{}", c.selected, size_suffix(c.bytes)).into(),
            spoken: format!("{} of {} selected in {}{}", c.selected, c.total, c.folder, spoken_size(c.bytes)).into(),
        };
    }
    let text = items(c.total);
    if c.selected == 1 && c.bytes > 0 {
        let size = human_size(c.bytes);
        return Count {
            text: format!("{text} · {size}").into(),
            spoken: format!("{text} in {}, selected item {size}", c.folder).into(),
        };
    }
    Count { text: text.clone().into(), spoken: format!("{text} in {}", c.folder).into() }
}

pub fn search_count(results: usize, selected: usize, searching: bool) -> Count {
    if searching && results == 0 {
        return Count { text: "Searching…".into(), spoken: "Searching".into() };
    }
    if selected > 0 {
        return Count {
            text: format!("{selected} of {results} selected").into(),
            spoken: format!("{selected} of {results} results selected").into(),
        };
    }
    let text = if results == 1 { "1 result".to_owned() } else { format!("{results} results") };
    Count { text: text.clone().into(), spoken: text.into() }
}

pub fn sidebar_count(group: &str, rows: usize) -> Count {
    Count { text: format!("{group} · {rows}").into(), spoken: format!("{group}, {}", items(rows)).into() }
}

/// One bar: path · transient · counts.
pub fn render(state: BarState, pane: Entity<Pane>, cx: &mut App) -> impl IntoElement + use<> {
    let ui = cx.global::<Ui>();
    let theme = ui.theme.clone();
    let border = color(theme.role("border"));
    let fg = color(theme.role("fg"));
    let muted = color(theme.role("fg-muted"));
    let error = color(theme.role("error"));
    let hover = color(theme.role("surface-raised"));
    let last = state.path.parts.len().saturating_sub(1);
    let parts = state.path.parts;
    let fg_path = fg;
    let last_is_dir = state.path.last_is_dir;

    let path = div()
        .id("bar-path")
        .role(Role::Navigation)
        .aria_label(format!("Path: {}", state.path.spoken))
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .flex()
        .items_center()
        .gap_1()
        .children(parts.into_iter().enumerate().map(move |(i, (name, target))| {
            let pane = pane.clone();
            let is_dir = i < last || last_is_dir;
            let spoke = if is_dir { format!("Go to {name}") } else { format!("Open {name}") };
            let mut part = div()
                .id(("path-segment", i))
                .debug_selector(move || format!("path-segment-{i}"))
                .role(Role::Button)
                .aria_label(spoke.clone())
                .flex()
                .items_center()
                .gap_1()
                .px_1()
                .rounded_sm()
                .when(i == last, |part| part.text_color(fg_path))
                .hover(move |s| s.bg(hover))
                .children(icon(&target, is_dir, 14).map(|i| img(i).size(px(14.))))
                .child(name)
                .when(i < last, |part| part.child(div().pl_1().child("›")));
            let target = target.clone();
            part = part.on_click(move |_, _, cx| {
                let target = target.clone();
                pane.update(cx, |pane, cx| {
                    if is_dir {
                        // Clicking where you already are changes nothing.
                        if pane.current_dir(cx) != target {
                            pane.navigate(target.clone(), cx);
                        }
                    } else {
                        pane.open_file(target.clone(), cx);
                    }
                });
            });
            with_tooltip(part, spoke, None)
        }));

    let (primary, primary_color) = match &state.primary {
        Primary::None | Primary::Hints(_) => (None, muted),
        Primary::Pending(text) => (Some(SharedString::from(*text)), fg),
        Primary::Message(text) => (Some(text.clone()), muted),
        Primary::Running(running) => {
            let text = if running.detail.is_empty() {
                running.label.clone()
            } else {
                format!("{} — {}", running.label, running.detail).into()
            };
            (Some(text), fg)
        }
    };
    let error_text = state.error.as_ref().map(|e| e.text.clone());
    let transient = div()
        .id("bar-transient")
        .debug_selector(|| "bar-transient".into())
        .role(Role::Status)
        .aria_label(primary.clone().or_else(|| error_text.clone()).unwrap_or_default())
        .flex_none()
        .flex()
        .items_center()
        .gap(px(14.))
        .children(primary.map(|text| div().text_color(primary_color).child(text)))
        .children(error_text.map(|text| div().text_color(error).child(text)));

    let counts_spoken = state.count.spoken.clone();
    let counts = with_tooltip(
        div()
            .id("bar-counts")
            .role(Role::Status)
            .aria_label(counts_spoken.clone())
            .flex_none()
            .pl_3()
            .border_l_1()
            .border_color(border)
            .text_color(fg)
            .child(state.count.text.clone()),
        counts_spoken,
        None,
    );

    div()
        .id("path-bar")
        .role(Role::Group)
        .aria_label("Status bar")
        .h(px(30.))
        .flex_none()
        .px_3()
        .flex()
        .items_center()
        .gap(px(14.))
        .border_t_1()
        .border_color(border)
        .text_size(px(12.))
        .text_color(muted)
        .child(path)
        .child(transient)
        .child(counts)
}
