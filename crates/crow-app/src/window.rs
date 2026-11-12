//! One Crow window: title bar, tabs, sidebar, the active pane, preview and path bar.

use std::path::{Path, PathBuf};

use crow_core::{SidebarEdit, Tabs, ViewMode};
use gpui::{
    App, Bounds, Context, Entity, FocusHandle, Focusable, FontWeight, IntoElement, Render, Role, SharedString,
    Subscription, TitlebarOptions, Window, WindowBounds, WindowOptions, div, img, point, prelude::*, px, size,
};

use crate::actions::{
    AddToSidebar, ApplyThemeEverywhere, CloseTab, GoBack, GoForward, GoToFolder, MoveDown, MoveUp, NewTab, NextTab,
    OpenParent, PreviousTab, ResetFolderView, ShowThemePicker, SwitchFocus, ToggleHiddenFiles, UseViewAsDefaults,
    ViewAsColumns, ViewAsList, WhyThisView,
};
use crate::operations::{self, Clipboard, Operations};
use crate::pane::Pane;
use crate::pane::PaneEvent;
use crate::preview;
use crate::sidebar::{Sidebar, SidebarEvent};
use crate::style::{color, color_alpha, icon, ui_font};
use crate::text_input::{TextInput, TextInputEvent};
use crate::ui::{self, Ui, reload_theme};
use crow_fs::ops::{Plan, Request, Resolution};
use std::collections::HashMap;

/// Open a window on `dir`. New windows share one size from settings and step
/// 24 pt down and right from the previous one, so they never land at random sizes.
pub fn open(dir: PathBuf, bench: bool, cx: &mut App) {
    let defaults = cx.global::<Ui>().settings.window.clone();
    let step = px(24.0 * cx.windows().len() as f32);
    let mut bounds = Bounds::centered(None, size(px(defaults.width as f32), px(defaults.height as f32)), cx);
    bounds.origin = point(bounds.origin.x + step, bounds.origin.y + step);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some("Crow".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(14.), px(16.))),
        }),
        ..Default::default()
    };
    if let Err(e) = cx.open_window(options, |window, cx| cx.new(|cx| CrowWindow::new(dir, bench, window, cx))) {
        tracing::error!("open window: {e}");
    }
}

pub struct CrowWindow {
    tabs: Tabs<Entity<Pane>>,
    sidebar: Entity<Sidebar>,
    /// Benchmark runs leave Recent alone, so its saves don't land in the measurement.
    bench: bool,
    show_hidden: bool,
    /// The Go to Folder sheet, while open.
    go_to: Option<(Entity<TextInput>, Subscription)>,
    /// A sheet asking before an operation runs (conflicts, permanent delete).
    sheet: Option<Sheet>,
    show_activity: bool,
    /// "Why this view?", while open.
    why: Option<FocusHandle>,
    picker: Option<ThemePicker>,
    _subscriptions: Vec<Subscription>,
}

/// View ▸ Theme…: "System" first, then every theme in the index. Moving the
/// cursor shows a theme at once; Return keeps it, Escape goes back.
struct ThemePicker {
    focus: FocusHandle,
    /// `None` is "System".
    entries: Vec<Option<crow_theme::ThemeInfo>>,
    cursor: usize,
}

impl ThemePicker {
    fn id(&self, i: usize) -> String {
        match self.entries.get(i) {
            Some(Some(info)) => info.id.clone(),
            _ => "system".into(),
        }
    }
}

impl CrowWindow {
    fn new(dir: PathBuf, bench: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let show_hidden = cx.global::<Ui>().settings.show_hidden_files;
        if !bench {
            ui::record_visit(&dir, cx);
        }
        let pane = cx.new(|cx| Pane::new(dir.clone(), show_hidden, bench, cx));
        let sidebar = cx.new(|cx| {
            let mut sidebar = Sidebar::new(cx);
            sidebar.set_current(dir, cx);
            sidebar
        });
        let focus = pane.read(cx).focus_handle().clone();
        window.focus(&focus, cx);
        let subscriptions = vec![
            cx.observe(&pane, |_, _, cx| cx.notify()),
            cx.subscribe_in(&pane, window, |this, _, event, window, cx| this.on_pane_event(event, window, cx)),
            cx.subscribe_in(&sidebar, window, |this, _, event, window, cx| this.on_sidebar_event(event, window, cx)),
            cx.observe_window_appearance(window, |_, _, cx| reload_theme(cx)),
            // A reloaded views.toml applies to every tab's folder at once.
            cx.observe_global::<Ui>(|this, cx| {
                for (_, pane) in this.tabs.iter() {
                    pane.update(cx, |pane, cx| pane.apply_view(cx));
                }
            }),
        ];
        Self {
            tabs: Tabs::new(pane),
            sidebar,
            bench,
            show_hidden,
            go_to: None,
            sheet: None,
            show_activity: false,
            why: None,
            picker: None,
            _subscriptions: subscriptions,
        }
    }

    fn pane(&self) -> &Entity<Pane> {
        self.tabs.active()
    }

    fn focus_pane(&self, window: &mut Window, cx: &mut App) {
        let handle = self.pane().read(cx).focus_handle().clone();
        window.focus(&handle, cx);
    }

    // ----- window actions -----

    fn new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self.pane().read(cx).current_dir(cx);
        let show_hidden = self.show_hidden;
        let pane = cx.new(|cx| Pane::new(dir, show_hidden, false, cx));
        self._subscriptions.push(cx.observe(&pane, |_, _, cx| cx.notify()));
        self._subscriptions
            .push(cx.subscribe_in(&pane, window, |this, _, event, window, cx| this.on_pane_event(event, window, cx)));
        self.tabs.open(pane);
        self.focus_pane(window, cx);
        cx.notify();
    }

    fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.close_active().is_none() {
            window.remove_window();
            return;
        }
        self.focus_pane(window, cx);
        cx.notify();
    }

    fn cycle_tab(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs.cycle(delta);
        self.focus_pane(window, cx);
        cx.notify();
    }

    fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.cycle_tab(1, window, cx);
    }

    fn previous_tab(&mut self, _: &PreviousTab, window: &mut Window, cx: &mut Context<Self>) {
        self.cycle_tab(-1, window, cx);
    }

    fn switch_focus(&mut self, _: &SwitchFocus, window: &mut Window, cx: &mut Context<Self>) {
        let sidebar = self.sidebar.read(cx).focus_handle(cx);
        if sidebar.is_focused(window) {
            self.focus_pane(window, cx);
        } else {
            window.focus(&sidebar, cx);
        }
        cx.notify();
    }

    fn toggle_hidden(&mut self, _: &ToggleHiddenFiles, _: &mut Window, cx: &mut Context<Self>) {
        self.show_hidden = !self.show_hidden;
        let show = self.show_hidden;
        for (_, pane) in self.tabs.iter() {
            pane.update(cx, |pane, cx| pane.set_show_hidden(show, cx));
        }
    }

    fn go_to_folder(&mut self, _: &GoToFolder, window: &mut Window, cx: &mut Context<Self>) {
        let current = home_relative(&self.pane().read(cx).current_dir(cx));
        let input = cx.new(|cx| TextInput::new(current, "Go to Folder", cx));
        let subscription = cx.subscribe_in(&input, window, |this, input, event: &TextInputEvent, window, cx| {
            let typed = input.read(cx).text().to_owned();
            let cwd = this.pane().read(cx).current_dir(cx);
            let home = home_dir();
            match event {
                TextInputEvent::CompletionRequested => {
                    if let Some(completed) = crow_core::complete_typed(&typed, &cwd, &home) {
                        input.update(cx, |input, cx| input.set_text(completed, window, cx));
                    }
                }
                TextInputEvent::Cancelled => this.close_go_to(window, cx),
                TextInputEvent::Confirmed => {
                    let target = crow_core::resolve_typed(&typed, &cwd, &home);
                    if target.is_dir() {
                        this.pane().update(cx, |pane, cx| pane.navigate(target, cx));
                    } else if target.exists() {
                        this.pane().update(cx, |pane, cx| pane.reveal(target, cx));
                    } else {
                        window.play_system_bell();
                        return;
                    }
                    this.close_go_to(window, cx);
                }
            }
        });
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.go_to = Some((input, subscription));
        cx.notify();
    }

    fn close_go_to(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.go_to = None;
        self.focus_pane(window, cx);
        cx.notify();
    }

    fn render_go_to(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let (input, _) = self.go_to.as_ref()?;
        let theme = &cx.global::<Ui>().theme;
        Some(
            div().absolute().top(px(52.)).left_0().right_0().flex().justify_center().child(
                div()
                    .id("go-to-folder")
                    .role(Role::Dialog)
                    .aria_label("Go to Folder")
                    .w(px(520.))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .rounded_lg()
                    .bg(color(theme.role("surface")))
                    .border_1()
                    .border_color(color(theme.role("border")))
                    .shadow_lg()
                    .child(div().text_size(px(12.)).text_color(color(theme.role("fg-muted"))).child("Go to Folder"))
                    .child(input.clone()),
            ),
        )
    }

    fn view_as_list(&mut self, _: &ViewAsList, _: &mut Window, cx: &mut Context<Self>) {
        self.pane().update(cx, |pane, cx| pane.set_mode(ViewMode::List, cx));
    }

    fn view_as_columns(&mut self, _: &ViewAsColumns, _: &mut Window, cx: &mut Context<Self>) {
        self.pane().update(cx, |pane, cx| pane.set_mode(ViewMode::Columns, cx));
    }

    fn open_parent(&mut self, _: &OpenParent, _: &mut Window, cx: &mut Context<Self>) {
        self.pane().update(cx, |pane, cx| pane.open_parent(cx));
    }

    fn go_back(&mut self, _: &GoBack, _: &mut Window, cx: &mut Context<Self>) {
        self.pane().update(cx, |pane, cx| pane.go_back(cx));
    }

    fn go_forward(&mut self, _: &GoForward, _: &mut Window, cx: &mut Context<Self>) {
        self.pane().update(cx, |pane, cx| pane.go_forward(cx));
    }

    // ----- rendering -----

    fn render_title_bar(&self, title: &str, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = &cx.global::<Ui>().theme;
        let pane = self.pane().read(cx);
        let (back, forward, mode) = (pane.can_go_back(), pane.can_go_forward(), pane.mode());
        let (fg, muted, subtle) = (theme.role("fg"), theme.role("fg-muted"), theme.role("fg-subtle"));
        let nav_button = |id: &'static str, label: &'static str, glyph: &'static str, enabled: bool| {
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(label)
                .w(px(26.))
                .h(px(26.))
                .rounded_md()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(18.))
                .text_color(color(if enabled { muted } else { subtle }))
                .child(glyph)
        };
        div()
            .h(px(44.))
            .flex_none()
            .pl(px(86.))
            .pr_3()
            .flex()
            .items_center()
            .gap_1()
            .border_b_1()
            .border_color(color(theme.role("border")))
            .child(
                nav_button("back", "Back", "‹", back)
                    .on_click(cx.listener(|this, _, _, cx| this.pane().update(cx, |p, cx| p.go_back(cx)))),
            )
            .child(
                nav_button("forward", "Forward", "›", forward)
                    .on_click(cx.listener(|this, _, _, cx| this.pane().update(cx, |p, cx| p.go_forward(cx)))),
            )
            .child(
                div()
                    .id("title")
                    .role(Role::Heading)
                    .aria_level(1)
                    .aria_label(title.to_owned())
                    .pl_2()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(14.))
                    .text_color(color(fg))
                    .child(title.to_owned()),
            )
            .child(div().flex_1())
            .child(
                div()
                    .id("view-switcher")
                    .role(Role::RadioGroup)
                    .aria_label("View")
                    .flex()
                    .gap(px(2.))
                    .p(px(2.))
                    .rounded_md()
                    .bg(color(theme.role("surface")))
                    .child(view_button("view-list", "as List", "☰", mode == ViewMode::List, theme).on_click(
                        cx.listener(|this, _, _, cx| this.pane().update(cx, |p, cx| p.set_mode(ViewMode::List, cx))),
                    ))
                    .child(view_button("view-columns", "as Columns", "▥", mode == ViewMode::Columns, theme).on_click(
                        cx.listener(|this, _, _, cx| this.pane().update(cx, |p, cx| p.set_mode(ViewMode::Columns, cx))),
                    )),
            )
    }

    fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.global::<Ui>().theme.clone();
        let active = self.tabs.active_index();
        let count = self.tabs.len();
        let tabs: Vec<(usize, String)> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(i, (_, pane))| (i, folder_name(&pane.read(cx).current_dir(cx))))
            .collect();
        div()
            .id("tabs")
            .role(Role::TabList)
            .aria_label("Tabs")
            .h(px(30.))
            .flex_none()
            .flex()
            .border_b_1()
            .border_color(color(theme.role("border")))
            .bg(color(theme.role("bg-inset")))
            .children(tabs.into_iter().map(|(i, name)| {
                let selected = i == active;
                div()
                    .id(("tab", i))
                    .role(Role::Tab)
                    .aria_label(name.clone())
                    .aria_selected(selected)
                    .aria_position_in_set(i + 1)
                    .aria_size_of_set(count)
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(12.))
                    .border_r_1()
                    .border_color(color(theme.role("border")))
                    .text_color(color(if selected { theme.role("fg") } else { theme.role("fg-muted") }))
                    .when(selected, |tab| tab.bg(color(theme.role("bg"))))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.tabs.activate(i);
                        this.focus_pane(window, cx);
                        cx.notify();
                    }))
                    .child(name)
            }))
    }

    /// List view's right-hand preview pane (column view shows it as its last column).
    fn render_preview(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.global::<Ui>().theme.clone();
        let pane = self.pane().read(cx);
        let entry = pane.cursor_entry(cx);
        let preview = pane.preview(cx);
        div()
            .id("preview")
            .role(Role::Group)
            .aria_label(entry.as_ref().map_or_else(|| "Preview".to_owned(), |e| preview::spoken(e, preview)))
            .w(px(300.))
            .flex_none()
            .h_full()
            .p_4()
            .border_l_1()
            .border_color(color(theme.role("border")))
            .overflow_hidden()
            .children(entry.map(|e| preview::contents(&e, preview, &theme)))
    }

    fn render_path_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        let settings_error = ui.config_error();
        let pane = self.pane().read(cx);
        let target = pane.cursor_entry(cx).map_or_else(|| pane.current_dir(cx), |e| e.path.clone());
        let parts = crow_macos::display_components(&target);
        let last = parts.len().saturating_sub(1);
        let spoken = parts.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>().join(", ");
        let (count, selected) = (pane.item_count(cx), pane.selected_count(cx));
        let ops = cx.global::<Operations>();
        let running = ops.running.as_ref().map(|r| {
            let p = &r.progress;
            let percent = (p.done_bytes * 100).checked_div(p.total_bytes).unwrap_or(0);
            format!("{} — {} of {} · {percent}%", r.label, p.done_items, p.total_items)
        });
        let op_message = ops.message.clone();
        let hint = pane.pending_hint();
        let (status, is_error): (SharedString, bool) = match (pane.error(cx), settings_error) {
            _ if hint.is_some() => (hint.unwrap_or_default().into(), false),
            _ if running.is_some() => (running.clone().unwrap_or_default().into(), false),
            _ if op_message.is_some() => op_message.clone().unwrap_or_default(),
            (Some(e), _) | (None, Some(e)) => (e, true),
            _ if pane.is_loading(cx) && count == 0 => ("Loading…".into(), false),
            _ if selected > 1 => (format!("{selected} of {count} selected").into(), false),
            _ => (format!("{count} items").into(), false),
        };
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
            .border_color(color(theme.role("border")))
            .text_size(px(12.))
            .text_color(color(theme.role("fg-muted")))
            .child(
                // Long paths give way to the status line instead of pushing it out.
                div().flex_1().min_w_0().overflow_hidden().flex().items_center().gap_1().children(
                    parts.into_iter().enumerate().map({
                        let fg = color(theme.role("fg"));
                        move |(i, (name, path))| {
                            let is_dir = i < last || path.is_dir();
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .when(i == last, |part| part.text_color(fg))
                                .children(icon(&path, is_dir, 14).map(|i| img(i).size(px(14.))))
                                .child(name)
                                .when(i < last, |part| part.child(div().pl_1().child("›")))
                        }
                    }),
                ),
            )
            .child(
                div()
                    .id("status")
                    .flex_none()
                    .pl_3()
                    .role(Role::Status)
                    .aria_label(status.clone())
                    .when(is_error, |s| s.text_color(color(theme.role("error"))))
                    .child(status),
            )
            .when(running.is_some(), |bar| {
                bar.child(
                    div()
                        .id("cancel-operation")
                        .role(Role::Button)
                        .aria_label("Cancel operation")
                        .ml_2()
                        .px_2()
                        .rounded_sm()
                        .bg(color(theme.role("surface-raised")))
                        .text_color(color(theme.role("fg")))
                        .on_click(|_, _, cx| operations::cancel_running(cx))
                        .child("Cancel"),
                )
            })
    }
}

fn view_button(
    id: &'static str,
    label: &'static str,
    glyph: &'static str,
    selected: bool,
    theme: &crow_theme::Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .role(Role::RadioButton)
        .aria_label(label)
        .aria_selected(selected)
        .w(px(28.))
        .h(px(22.))
        .rounded_sm()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(13.))
        .text_color(color(if selected { theme.role("fg") } else { theme.role("fg-muted") }))
        .when(selected, |b| b.bg(color(theme.role("surface-raised"))))
        .child(glyph)
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

/// `~/Documents` for paths in the home folder, like Finder's Go to Folder.
fn home_relative(path: &Path) -> String {
    match path.strip_prefix(home_dir()) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

fn folder_name(dir: &Path) -> String {
    crow_macos::display_components(dir).pop().map_or_else(|| dir.display().to_string(), |(name, _)| name)
}

/// A question to answer before an operation runs.
enum Sheet {
    Conflicts { plan: Plan, choices: Vec<Resolution>, focus: FocusHandle },
    ConfirmDelete { paths: Vec<PathBuf>, focus: FocusHandle },
}

impl Sheet {
    fn focus(&self) -> &FocusHandle {
        match self {
            Sheet::Conflicts { focus, .. } | Sheet::ConfirmDelete { focus, .. } => focus,
        }
    }
}

impl CrowWindow {
    fn on_pane_event(&mut self, event: &PaneEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            PaneEvent::Run(request) => self.run_request(request.clone(), window, cx),
            PaneEvent::Copy { paths, cut } => {
                let change_count = crow_macos::write_file_urls(paths);
                let n = paths.len();
                let ops = cx.global_mut::<Operations>();
                ops.clipboard = Some(Clipboard { paths: paths.clone(), cut: *cut, change_count });
                let what = if n == 1 { "1 item".to_owned() } else { format!("{n} items") };
                ops.message = Some((
                    if *cut { format!("Cut {what}: paste to move") } else { format!("Copied {what}") }.into(),
                    false,
                ));
                cx.notify();
            }
            PaneEvent::Entered(dir) => {
                if !self.bench {
                    ui::record_visit(dir, cx);
                }
                let dir = dir.clone();
                self.sidebar.update(cx, |s, cx| s.set_current(dir, cx));
            }
            PaneEvent::Mark { key, set } => self.mark(*key, *set, window, cx),
            PaneEvent::Paste { into, move_items } => {
                // Our clipboard if the pasteboard still holds what we put there; otherwise
                // whatever another app (Finder) copied.
                let ops = cx.global::<Operations>();
                let ours = ops.clipboard.as_ref().filter(|c| c.change_count == crow_macos::pasteboard_change_count());
                let (paths, cut) = match ours {
                    Some(c) => (c.paths.clone(), c.cut),
                    None => (crow_macos::read_file_urls(), false),
                };
                if paths.is_empty() {
                    return;
                }
                if cut {
                    cx.global_mut::<Operations>().clipboard = None;
                }
                let into = into.clone();
                let request =
                    if cut || *move_items { Request::Move { paths, into } } else { Request::Copy { paths, into } };
                self.run_request(request, window, cx);
            }
        }
    }

    /// Plan in the background; ask first when there are conflicts or the change is
    /// permanent; otherwise run it.
    fn run_request(&mut self, request: Request, window: &mut Window, cx: &mut Context<Self>) {
        if let Request::DeletePermanently { paths } = request {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            self.sheet = Some(Sheet::ConfirmDelete { paths, focus });
            cx.notify();
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            let planned = cx.background_spawn(async move { crow_fs::ops::plan(request) }).await;
            this.update_in(cx, |this, window, cx| match planned {
                Err(e) => {
                    cx.global_mut::<Operations>().message = Some((e.to_string().into(), true));
                    cx.notify();
                }
                Ok(plan) if plan.items.is_empty() => {}
                Ok(plan) if plan.conflicts().next().is_some() => {
                    let focus = cx.focus_handle();
                    window.focus(&focus, cx);
                    let choices = vec![Resolution::KeepBoth; plan.conflicts().count()];
                    this.sheet = Some(Sheet::Conflicts { plan, choices, focus });
                    cx.notify();
                }
                Ok(plan) => this.start(plan, HashMap::new(), cx),
            })
            .ok();
        })
        .detach();
    }

    fn start(&mut self, plan: Plan, resolutions: HashMap<PathBuf, Resolution>, cx: &mut Context<Self>) {
        let rename_result = matches!(plan.request, Request::NewFolder { .. });
        let pane = self.pane().downgrade();
        operations::execute(plan, resolutions, cx, move |outcome, cx| {
            let results = outcome.record.results();
            pane.update(cx, |pane, cx| pane.show_results(results, rename_result, cx)).ok();
        });
    }

    fn undo(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        let pane = self.pane().downgrade();
        operations::undo(index, cx, move |restored, cx| {
            pane.update(cx, |pane, cx| pane.show_results(restored, false, cx)).ok();
        });
    }

    fn redo(&mut self, cx: &mut Context<Self>) {
        let pane = self.pane().downgrade();
        operations::redo(cx, move |results, cx| {
            pane.update(cx, |pane, cx| pane.show_results(results, false, cx)).ok();
        });
    }

    fn close_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sheet = None;
        self.focus_pane(window, cx);
        cx.notify();
    }

    fn confirm_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.sheet.take() {
            Some(Sheet::Conflicts { plan, choices, .. }) => {
                let targets: Vec<PathBuf> = plan.conflicts().map(|i| i.target.clone()).collect();
                let resolutions = targets.into_iter().zip(choices).collect();
                self.start(plan, resolutions, cx);
            }
            Some(Sheet::ConfirmDelete { paths, .. }) => {
                match crow_fs::ops::plan(Request::DeletePermanently { paths }) {
                    Ok(plan) => self.start(plan, HashMap::new(), cx),
                    Err(e) => cx.global_mut::<Operations>().message = Some((e.to_string().into(), true)),
                }
            }
            None => {}
        }
        self.focus_pane(window, cx);
        cx.notify();
    }

    fn set_choice(&mut self, index: Option<usize>, choice: Resolution, cx: &mut Context<Self>) {
        if let Some(Sheet::Conflicts { choices, .. }) = &mut self.sheet {
            match index {
                Some(i) => {
                    if let Some(c) = choices.get_mut(i) {
                        *c = choice;
                    }
                }
                None => choices.iter_mut().for_each(|c| *c = choice),
            }
            cx.notify();
        }
    }

    fn render_sheet(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let sheet = self.sheet.as_ref()?;
        let theme = cx.global::<Ui>().theme.clone();
        let (fg, muted, accent) = (theme.role("fg"), theme.role("fg-muted"), theme.role("accent"));
        let button = |id: &'static str, label: String, primary: bool| {
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(label.clone())
                .px_3()
                .py_1()
                .rounded_md()
                .bg(color(if primary { accent } else { theme.role("surface-raised") }))
                .text_color(color(if primary { theme.role("on-accent") } else { fg }))
                .child(label)
        };
        let (title, body): (String, gpui::AnyElement) = match sheet {
            Sheet::ConfirmDelete { paths, .. } => {
                let what = match paths.as_slice() {
                    [one] => format!("“{}”", one.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()),
                    many => format!("{} items", many.len()),
                };
                (
                    format!("Delete {what} permanently?"),
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(div().text_color(color(muted)).child("This can't be undone. Move to Trash (⌘⌫) can."))
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap_2()
                                .child(
                                    button("sheet-cancel", "Cancel".into(), false)
                                        .on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx))),
                                )
                                .child(
                                    button("sheet-confirm", "Delete".into(), true)
                                        .on_click(cx.listener(|this, _, window, cx| this.confirm_sheet(window, cx))),
                                ),
                        )
                        .into_any_element(),
                )
            }
            Sheet::Conflicts { plan, choices, .. } => {
                let into = plan.items.first().and_then(|i| i.target.parent()).map(folder_name).unwrap_or_default();
                let rows: Vec<(usize, String, Resolution)> = plan
                    .conflicts()
                    .zip(choices.iter().copied())
                    .enumerate()
                    .map(|(i, (item, choice))| {
                        (
                            i,
                            item.target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                            choice,
                        )
                    })
                    .collect();
                let option = |row: usize,
                              label: &'static str,
                              value: Resolution,
                              current: Resolution,
                              cx: &mut Context<Self>| {
                    let on = value == current;
                    div()
                        .id(("choice", row * 3 + value as usize))
                        .role(Role::RadioButton)
                        .aria_label(label)
                        .aria_selected(on)
                        .px_2()
                        .rounded_sm()
                        .text_size(px(12.))
                        .when(on, |o| o.bg(color_alpha(accent, 0x55)).text_color(color(fg)))
                        .on_click(cx.listener(move |this, _, _, cx| this.set_choice(Some(row), value, cx)))
                        .child(label)
                };
                let n = rows.len();
                (
                    if n == 1 {
                        format!("An item named “{}” already exists in “{into}”", rows[0].1)
                    } else {
                        format!("{n} items already exist in “{into}”")
                    },
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .children(rows.into_iter().map(|(i, name, choice)| {
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(div().flex_1().overflow_hidden().whitespace_nowrap().child(name))
                                .child(option(i, "Keep Both", Resolution::KeepBoth, choice, cx))
                                .child(option(i, "Replace", Resolution::Replace, choice, cx))
                                .child(option(i, "Skip", Resolution::Skip, choice, cx))
                        }))
                        .child(div().text_size(px(12.)).text_color(color(muted)).child(
                            "Keep Both adds a number to the new name. Replace moves the existing item to the Trash.",
                        ))
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .pt_2()
                                .child(button("all-keep", "Keep Both for All".into(), false).on_click(
                                    cx.listener(|this, _, _, cx| this.set_choice(None, Resolution::KeepBoth, cx)),
                                ))
                                .child(button("all-skip", "Skip All".into(), false).on_click(
                                    cx.listener(|this, _, _, cx| this.set_choice(None, Resolution::Skip, cx)),
                                ))
                                .child(div().flex_1())
                                .child(
                                    button("sheet-cancel", "Cancel".into(), false)
                                        .on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx))),
                                )
                                .child(
                                    button("sheet-confirm", "Continue".into(), true)
                                        .on_click(cx.listener(|this, _, window, cx| this.confirm_sheet(window, cx))),
                                ),
                        )
                        .into_any_element(),
                )
            }
        };
        Some(
            div().absolute().top(px(52.)).left_0().right_0().flex().justify_center().child(
                div()
                    .id("sheet")
                    .role(Role::Dialog)
                    .aria_label(title.clone())
                    .key_context("Sheet")
                    .track_focus(sheet.focus())
                    .on_action(
                        cx.listener(|this, _: &crate::actions::SheetConfirm, window, cx| {
                            this.confirm_sheet(window, cx)
                        }),
                    )
                    .on_action(
                        cx.listener(|this, _: &crate::actions::SheetCancel, window, cx| this.close_sheet(window, cx)),
                    )
                    .w(px(520.))
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .rounded_lg()
                    .bg(color(theme.role("surface")))
                    .border_1()
                    .border_color(color(theme.role("border")))
                    .shadow_lg()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                    .child(body),
            ),
        )
    }

    /// Recent operations, newest first, each with its own Undo.
    fn render_activity(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.global::<Ui>().theme.clone();
        let muted = color(theme.role("fg-muted"));
        let entries: Vec<(usize, String, String, bool)> = cx
            .global::<Operations>()
            .log
            .iter()
            .enumerate()
            .rev()
            .take(50)
            .map(|(i, e)| {
                let when: chrono::DateTime<chrono::Local> = e.at.into();
                let state = match (&e.error, e.cancelled, e.undone) {
                    (_, _, true) => "undone".to_owned(),
                    (Some(err), _, _) => format!("stopped: {err}"),
                    (None, true, _) => "cancelled".to_owned(),
                    _ => String::new(),
                };
                (
                    i,
                    e.record.label.clone(),
                    format!("{} {state}", when.format("%H:%M")),
                    e.record.is_undoable() && !e.undone,
                )
            })
            .collect();
        div()
            .id("activity")
            .role(Role::List)
            .aria_label("Activity")
            .absolute()
            .top(px(52.))
            .right(px(12.))
            .w(px(360.))
            .max_h(px(420.))
            .overflow_y_scroll()
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .rounded_lg()
            .bg(color(theme.role("surface")))
            .border_1()
            .border_color(color(theme.role("border")))
            .shadow_lg()
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Activity"))
            .when(entries.is_empty(), |list| list.child(div().text_color(muted).child("Nothing yet")))
            .children(entries.into_iter().map(|(i, label, when, undoable)| {
                div()
                    .id(("activity-entry", i))
                    .role(Role::ListItem)
                    .aria_label(format!("{label}, {when}"))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(label)
                            .child(div().text_size(px(11.)).text_color(muted).child(when)),
                    )
                    .when(undoable, |row| {
                        row.child(
                            div()
                                .id(("activity-undo", i))
                                .role(Role::Button)
                                .aria_label("Undo")
                                .px_2()
                                .rounded_sm()
                                .bg(color(theme.role("surface-raised")))
                                .on_click(cx.listener(move |this, _, _, cx| this.undo(Some(i), cx)))
                                .child("Undo"),
                        )
                    })
            }))
    }
}

impl CrowWindow {
    fn on_sidebar_event(&mut self, event: &SidebarEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            SidebarEvent::Open(dir) => {
                let dir = dir.clone();
                self.pane().update(cx, |pane, cx| pane.navigate(dir, cx));
                self.focus_pane(window, cx);
            }
            SidebarEvent::Run(request) => self.run_request(request.clone(), window, cx),
            SidebarEvent::Message(text, error) => {
                cx.global_mut::<Operations>().message = Some((text.clone(), *error));
                cx.notify();
            }
        }
    }

    /// Vim marks: `m x` marks the folder shown as favorite `x`; `'x` goes there.
    fn mark(&mut self, key: char, set: bool, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self.pane().read(cx).current_dir(cx);
        if set {
            ui::edit_sidebar(SidebarEdit::SetKey { path: dir, key }, cx);
            cx.global_mut::<Operations>().message = Some((format!("Marked as {key}").into(), false));
            cx.notify();
            return;
        }
        let ui = cx.global::<Ui>();
        match ui.sidebar.find_key(key, &ui.home) {
            Some(target) => self.pane().update(cx, |pane, cx| pane.navigate(target, cx)),
            None => {
                window.play_system_bell();
                cx.global_mut::<Operations>().message = Some((format!("No mark {key}").into(), true));
                cx.notify();
            }
        }
    }

    /// Finder's File ▸ Add to Sidebar: the selected folders, or the folder shown.
    fn add_to_sidebar(&mut self, _: &AddToSidebar, _: &mut Window, cx: &mut Context<Self>) {
        let pane = self.pane().read(cx);
        let selected: Vec<PathBuf> = pane.selected_paths(cx).into_iter().filter(|p| p.is_dir()).collect();
        let paths = if selected.is_empty() { vec![pane.current_dir(cx)] } else { selected };
        let at = cx.global::<Ui>().favorites().first().map_or(0, |(_, f)| f.len());
        for (n, path) in paths.into_iter().enumerate() {
            ui::edit_sidebar(SidebarEdit::Add { path, at: (0, at + n) }, cx);
        }
    }

    // ----- view settings -----

    fn why_this_view(&mut self, _: &WhyThisView, window: &mut Window, cx: &mut Context<Self>) {
        if self.why.take().is_some() {
            self.focus_pane(window, cx);
        } else {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            self.why = Some(focus);
        }
        cx.notify();
    }

    fn close_why(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.why = None;
        self.focus_pane(window, cx);
        cx.notify();
    }

    fn use_view_as_defaults(&mut self, _: &UseViewAsDefaults, _: &mut Window, cx: &mut Context<Self>) {
        let dir = self.pane().read(cx).dir().to_path_buf();
        ui::use_view_as_defaults(dir, cx);
        cx.global_mut::<Operations>().message = Some(("This view is now the default for all folders".into(), false));
    }

    fn reset_folder_view(&mut self, _: &ResetFolderView, _: &mut Window, cx: &mut Context<Self>) {
        let dir = self.pane().read(cx).dir().to_path_buf();
        ui::reset_view(dir, cx);
    }

    fn render_why(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let focus = self.why.as_ref()?;
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        let dir = self.pane().read(cx).dir().to_path_buf();
        let resolved = ui.views.resolve(&dir);
        let has_override = ui.views.has_folder_override(&dir);
        let views_file = ui.config_dir.join("views.toml");
        let folder = crow_core::abbreviate_home(&dir, &ui.home);
        let (fg, muted) = (theme.role("fg"), theme.role("fg-muted"));
        let mode = match resolved.view.0 {
            ViewMode::List => "List",
            ViewMode::Columns => "Columns",
        };
        let sort_title = |key: crow_fs::SortKey| match key {
            crow_fs::SortKey::Name => "Name",
            crow_fs::SortKey::Size => "Size",
            crow_fs::SortKey::Kind => "Kind",
            crow_fs::SortKey::Modified => "Date Modified",
            crow_fs::SortKey::Added => "Date Added",
        };
        let direction = match (resolved.sort.0, resolved.ascending.0) {
            (crow_fs::SortKey::Name | crow_fs::SortKey::Kind, true) => "A to Z",
            (crow_fs::SortKey::Name | crow_fs::SortKey::Kind, false) => "Z to A",
            (crow_fs::SortKey::Size, true) => "smallest first",
            (crow_fs::SortKey::Size, false) => "largest first",
            (_, true) => "oldest first",
            (_, false) => "newest first",
        };
        let columns = resolved.columns.0.join(", ");
        let lines: Vec<(&str, String, String)> = vec![
            ("View", mode.into(), resolved.view.1.to_string()),
            ("Sort", sort_title(resolved.sort.0).into(), resolved.sort.1.to_string()),
            ("Order", direction.into(), resolved.ascending.1.to_string()),
            (
                "Folders",
                if resolved.folders_first.0 { "first" } else { "mixed with files" }.into(),
                resolved.folders_first.1.to_string(),
            ),
            ("Columns", columns, resolved.columns.1.to_string()),
        ];
        let rules = if resolved.matched_rules.is_empty() {
            "No rules in views.toml match this folder.".to_owned()
        } else {
            format!("Matching rules, in order: {}", resolved.matched_rules.join(", "))
        };
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(label)
                .px_3()
                .py_1()
                .rounded_md()
                .bg(color(theme.role("surface-raised")))
                .child(label)
        };
        Some(
            div().absolute().top(px(52.)).left_0().right_0().flex().justify_center().child(
                div()
                    .id("why-this-view")
                    .role(Role::Dialog)
                    .aria_label(format!("Why this view: {folder}"))
                    .key_context("Sheet")
                    .track_focus(focus)
                    .on_action(
                        cx.listener(|this, _: &crate::actions::SheetConfirm, window, cx| this.close_why(window, cx)),
                    )
                    .on_action(
                        cx.listener(|this, _: &crate::actions::SheetCancel, window, cx| this.close_why(window, cx)),
                    )
                    .w(px(560.))
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .rounded_lg()
                    .bg(color(theme.role("surface")))
                    .border_1()
                    .border_color(color(theme.role("border")))
                    .shadow_lg()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(format!("Why this view? {folder}")))
                    .children(lines.into_iter().enumerate().map(|(i, (what, value, source))| {
                        div()
                            .id(("why-line", i))
                            .role(Role::ListItem)
                            .aria_label(format!("{what}: {value}, from {source}"))
                            .flex()
                            .gap_3()
                            .child(div().w(px(70.)).text_color(color(muted)).child(what))
                            .child(div().w(px(190.)).text_color(color(fg)).child(value))
                            .child(div().flex_1().text_size(px(12.)).text_color(color(muted)).child(source))
                    }))
                    .child(div().pt_1().text_size(px(12.)).text_color(color(muted)).child(rules))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .pt_2()
                            .child(button("why-open-file", "Edit views.toml").on_click(move |_, _, _| {
                                if let Err(e) = std::process::Command::new("open").arg("-t").arg(&views_file).spawn() {
                                    tracing::warn!("open views.toml: {e}");
                                }
                            }))
                            .child(div().flex_1())
                            .when(has_override, |row| {
                                row.child(button("why-reset", "Reset This Folder").on_click(cx.listener(
                                    |this, _, window, cx| {
                                        this.reset_folder_view(&ResetFolderView, window, cx);
                                        this.close_why(window, cx);
                                    },
                                )))
                            })
                            .child(button("why-defaults", "Use as Defaults").on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.use_view_as_defaults(&UseViewAsDefaults, window, cx);
                                    this.close_why(window, cx);
                                },
                            )))
                            .child(
                                button("why-close", "Close")
                                    .on_click(cx.listener(|this, _, window, cx| this.close_why(window, cx))),
                            ),
                    ),
            ),
        )
    }

    // ----- theme picker -----

    fn show_theme_picker(&mut self, _: &ShowThemePicker, window: &mut Window, cx: &mut Context<Self>) {
        if self.picker.is_some() {
            self.cancel_picker(window, cx);
            return;
        }
        let dir = crow_theme::themes_dir();
        let task = cx.background_spawn(async move { crow_theme::list_theme_infos(&dir) });
        cx.spawn_in(window, async move |this, cx| {
            let infos = task.await;
            this.update_in(cx, |this, window, cx| this.open_picker(infos, window, cx)).ok();
        })
        .detach();
    }

    fn open_picker(&mut self, infos: Vec<crow_theme::ThemeInfo>, window: &mut Window, cx: &mut Context<Self>) {
        // Grouped like the theme index's own split: dark themes, then light.
        let mut infos = infos;
        infos.sort_by_key(|i| (i.variant == crow_theme::Variant::Light, i.name.to_lowercase()));
        let mut entries: Vec<Option<crow_theme::ThemeInfo>> = vec![None];
        entries.extend(infos.into_iter().map(Some));
        let current = cx.global::<Ui>().settings.theme.clone();
        let cursor = entries.iter().position(|e| e.as_ref().is_some_and(|i| i.id == current)).unwrap_or(0);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.picker = Some(ThemePicker { focus, entries, cursor });
        cx.notify();
    }

    fn move_picker(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(picker) = &mut self.picker else { return };
        let last = picker.entries.len().saturating_sub(1);
        picker.cursor = picker.cursor.saturating_add_signed(delta).min(last);
        let id = picker.id(picker.cursor);
        cx.global_mut::<Ui>().preview_theme = Some(id);
        reload_theme(cx);
        cx.notify();
    }

    fn select_picker(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(picker) = &self.picker {
            let delta = index as isize - picker.cursor as isize;
            self.move_picker(delta, cx);
        }
    }

    fn confirm_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = self.picker.take() else { return };
        ui::choose_theme(&picker.id(picker.cursor), cx);
        self.focus_pane(window, cx);
        cx.notify();
    }

    fn cancel_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.picker = None;
        cx.global_mut::<Ui>().preview_theme = None;
        reload_theme(cx);
        self.focus_pane(window, cx);
        cx.notify();
    }

    /// "Apply to All Apps": the chosen theme becomes the global pick for its variant,
    /// and Crow follows "System" from then on.
    fn apply_everywhere(&mut self, _: &ApplyThemeEverywhere, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = self.picker.take() else { return };
        match picker.entries.get(picker.cursor) {
            Some(Some(info)) => match crow_theme::load_theme(&crow_theme::themes_dir(), &info.id) {
                Ok(theme) => ui::apply_theme_everywhere(&theme, cx),
                Err(e) => cx.global_mut::<Operations>().message = Some((e.into(), true)),
            },
            _ => ui::choose_theme("system", cx),
        }
        self.focus_pane(window, cx);
        cx.notify();
    }

    fn render_picker(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let picker = self.picker.as_ref()?;
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        let vim = ui.settings.keymap == crow_core::KeymapLayer::Vim;
        let (fg, muted, accent) = (theme.role("fg"), theme.role("fg-muted"), theme.role("accent"));
        let count = picker.entries.len();
        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        let mut last_variant = None;
        for (i, entry) in picker.entries.iter().enumerate() {
            let variant = entry.as_ref().map(|e| e.variant);
            if variant.is_some() && variant != last_variant {
                let title = if variant == Some(crow_theme::Variant::Light) { "Light" } else { "Dark" };
                rows.push(
                    div()
                        .px_2()
                        .pt_2()
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(color(muted))
                        .child(title)
                        .into_any_element(),
                );
                last_variant = variant;
            }
            let label = entry.as_ref().map_or_else(|| "System (follows macOS)".to_owned(), |e| e.name.clone());
            let selected = i == picker.cursor;
            rows.push(
                div()
                    .id(("theme", i))
                    .role(Role::ListBoxOption)
                    .aria_label(label.clone())
                    .aria_selected(selected)
                    .aria_position_in_set(i + 1)
                    .aria_size_of_set(count)
                    .when(selected, |r| r.aria_active_descendant())
                    .px_2()
                    .py(px(3.))
                    .rounded_md()
                    .text_color(color(fg))
                    .when(selected, |r| r.bg(color_alpha(accent, 0x55)))
                    .on_click(cx.listener(move |this, _, _, cx| this.select_picker(i, cx)))
                    .child(label)
                    .into_any_element(),
            );
        }
        let mut context = "Sheet ThemePicker".to_owned();
        if vim {
            context.push_str(" vim");
        }
        let button = |id: &'static str, label: &'static str, primary: bool| {
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(label)
                .px_3()
                .py_1()
                .rounded_md()
                .bg(color(if primary { accent } else { theme.role("surface-raised") }))
                .text_color(color(if primary { theme.role("on-accent") } else { fg }))
                .child(label)
        };
        Some(
            div().absolute().top(px(52.)).left_0().right_0().flex().justify_center().child(
                div()
                    .id("theme-picker")
                    .role(Role::Dialog)
                    .aria_label("Theme")
                    .key_context(context.as_str())
                    .track_focus(&picker.focus)
                    .on_action(cx.listener(|this, _: &MoveUp, _, cx| this.move_picker(-1, cx)))
                    .on_action(cx.listener(|this, _: &MoveDown, _, cx| this.move_picker(1, cx)))
                    .on_action(cx.listener(|this, _: &crate::actions::SheetConfirm, window, cx| {
                        this.confirm_picker(window, cx)
                    }))
                    .on_action(
                        cx.listener(|this, _: &crate::actions::SheetCancel, window, cx| this.cancel_picker(window, cx)),
                    )
                    .on_action(cx.listener(Self::apply_everywhere))
                    .w(px(360.))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .rounded_lg()
                    .bg(color(theme.role("surface")))
                    .border_1()
                    .border_color(color(theme.role("border")))
                    .shadow_lg()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Theme"))
                    .child(
                        div()
                            .id("theme-list")
                            .role(Role::ListBox)
                            .aria_label("Themes")
                            .max_h(px(380.))
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap(px(1.))
                            .children(rows),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(color(muted))
                            .child("Return keeps it for Crow. Apply to All Apps (⌘↩) sets it for every app that reads the theme index."),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(button("theme-everywhere", "Apply to All Apps", false).on_click(cx.listener(
                                |this, _, window, cx| this.apply_everywhere(&ApplyThemeEverywhere, window, cx),
                            )))
                            .child(div().flex_1())
                            .child(
                                button("theme-cancel", "Cancel", false)
                                    .on_click(cx.listener(|this, _, window, cx| this.cancel_picker(window, cx))),
                            )
                            .child(
                                button("theme-ok", "Choose", true)
                                    .on_click(cx.listener(|this, _, window, cx| this.confirm_picker(window, cx))),
                            ),
                    ),
            ),
        )
    }
}

impl Render for CrowWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        let (font, font_size) = (ui_font(&ui.settings.font_family), ui.settings.font_size);
        let title = folder_name(&self.pane().read(cx).current_dir(cx));
        let list_view = self.pane().read(cx).mode() == ViewMode::List;
        window.set_window_title(&title);
        let title_bar = self.render_title_bar(&title, cx);
        let tab_bar = (self.tabs.len() > 1).then(|| self.render_tab_bar(cx));
        let current = self.pane().read(cx).current_dir(cx);
        self.sidebar.update(cx, |s, cx| s.set_current(current, cx));
        let sidebar = self.sidebar.clone();
        let preview = list_view.then(|| self.render_preview(cx));
        let path_bar = self.render_path_bar(cx);
        let go_to = self.render_go_to(cx);
        let sheet = self.render_sheet(cx);
        let activity = self.show_activity.then(|| self.render_activity(cx));
        let why = self.render_why(cx);
        let picker = self.render_picker(cx);
        div()
            .id("crow")
            .role(Role::Application)
            .aria_label("Crow")
            .key_context("Crow")
            .on_action(cx.listener(Self::new_tab))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::previous_tab))
            .on_action(cx.listener(Self::switch_focus))
            .on_action(cx.listener(Self::toggle_hidden))
            .on_action(cx.listener(Self::open_parent))
            .on_action(cx.listener(Self::go_back))
            .on_action(cx.listener(Self::go_forward))
            .on_action(cx.listener(Self::go_to_folder))
            .on_action(cx.listener(Self::view_as_list))
            .on_action(cx.listener(Self::view_as_columns))
            .on_action(cx.listener(|this, _: &crate::actions::Undo, _, cx| this.undo(None, cx)))
            .on_action(cx.listener(|this, _: &crate::actions::Redo, _, cx| this.redo(cx)))
            .on_action(cx.listener(|_, _: &crate::actions::CancelOperation, _, cx| operations::cancel_running(cx)))
            .on_action(cx.listener(Self::add_to_sidebar))
            .on_action(cx.listener(Self::why_this_view))
            .on_action(cx.listener(Self::use_view_as_defaults))
            .on_action(cx.listener(Self::reset_folder_view))
            .on_action(cx.listener(Self::show_theme_picker))
            .on_action(cx.listener(|this, _: &crate::actions::ToggleActivity, _, cx| {
                this.show_activity = !this.show_activity;
                cx.notify();
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(color(theme.role("bg")))
            .text_color(color(theme.role("fg")))
            .font_family(font)
            .text_size(px(font_size))
            .child(title_bar)
            .children(tab_bar)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(sidebar)
                    .child(div().flex_1().min_w_0().h_full().child(self.pane().clone()))
                    .children(preview),
            )
            .child(path_bar)
            .children(go_to)
            .children(sheet)
            .children(activity)
            .children(why)
            .children(picker)
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use gpui::{Entity, TestAppContext, VisualTestContext, WindowHandle};

    use super::*;

    /// `root/{alpha/inner/, beta/, notes.txt}`, canonicalized (macOS temp dirs sit behind /private).
    fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("crow-app-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("alpha/inner")).expect("create fixture");
        std::fs::create_dir_all(root.join("beta")).expect("create fixture");
        std::fs::write(root.join("notes.txt"), "hello\n").expect("create fixture");
        root.canonicalize().expect("canonical fixture path")
    }

    fn open(cx: &mut TestAppContext, dir: &Path) -> (Entity<CrowWindow>, VisualTestContext) {
        let trash = dir
            .parent()
            .unwrap_or(dir)
            .join(format!(".trash-{}", dir.file_name().unwrap_or_default().to_string_lossy()));
        std::fs::create_dir_all(&trash).expect("create test trash");
        cx.update(|cx| {
            crate::ui::init_for_tests(cx);
            crate::operations::init(cx, std::sync::Arc::new(crate::operations::FolderTrash(trash)));
            cx.bind_keys(crate::actions::finder_keymap());
            cx.bind_keys(crate::actions::vim_keymap());
            cx.bind_keys(crate::text_input::keymap());
        });
        let dir = dir.to_path_buf();
        let window: WindowHandle<CrowWindow> = cx
            .update(|cx| {
                cx.open_window(Default::default(), |window, cx| cx.new(|cx| CrowWindow::new(dir, false, window, cx)))
            })
            .expect("open window");
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        let view = window.root(&mut cx).expect("root view");
        settle(&view, &mut cx);
        (view, cx)
    }

    /// Folders load on real threads; wait until every tab has finished.
    fn settle(view: &Entity<CrowWindow>, cx: &mut VisualTestContext) {
        for _ in 0..400 {
            cx.run_until_parked();
            let busy = view.read_with(cx, |w, cx| {
                w.tabs.iter().any(|(_, pane)| pane.read(cx).is_loading(cx))
                    || cx.global::<Operations>().running.is_some()
            });
            if !busy {
                cx.run_until_parked();
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("folders never finished loading");
    }

    fn pane(view: &Entity<CrowWindow>, cx: &VisualTestContext) -> Entity<Pane> {
        view.read_with(cx, |w, _| w.pane().clone())
    }

    fn selected(view: &Entity<CrowWindow>, cx: &VisualTestContext) -> Option<String> {
        pane(view, cx).read_with(cx, |p, cx| p.cursor_entry(cx).map(|e| e.name))
    }

    fn dir(view: &Entity<CrowWindow>, cx: &VisualTestContext) -> PathBuf {
        pane(view, cx).read_with(cx, |p, cx| p.current_dir(cx))
    }

    fn config_dir(cx: &VisualTestContext) -> PathBuf {
        cx.read(|cx| cx.global::<Ui>().config_dir.clone())
    }

    /// Replace a config file and reload, as the file watcher would.
    fn write_config(view: &Entity<CrowWindow>, cx: &mut VisualTestContext, name: &str, text: &str) {
        let dir = config_dir(cx);
        std::fs::create_dir_all(&dir).expect("config dir");
        std::fs::write(dir.join(name), text).expect("write config");
        cx.update(|_, cx| crate::ui::reload_config(cx));
        settle(view, cx);
    }

    fn favorites_toml(paths: &[&Path]) -> String {
        let items: String = paths.iter().map(|p| format!("  {{ path = \"{}\" }},\n", p.display())).collect();
        format!("[[group]]\nname = \"Favorites\"\nitems = [\n{items}]\n[recent]\nshow = false\n")
    }

    fn sidebar_paths(cx: &VisualTestContext) -> Vec<PathBuf> {
        cx.read(Sidebar::paths)
    }

    #[gpui::test]
    fn same_keys_mean_different_things_in_sidebar_and_list(cx: &mut TestAppContext) {
        let root = fixture("contexts");
        std::fs::write(root.join("beta/b.txt"), "").expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("alpha"), &root.join("beta")]));
        assert_eq!(selected(&view, &cx).as_deref(), Some("alpha"));

        cx.simulate_keystrokes("down");
        assert_eq!(selected(&view, &cx).as_deref(), Some("beta"), "Down in the list moves the selection");

        cx.simulate_keystrokes("tab down");
        let sidebar = view.read_with(&cx, |w, _| w.sidebar.clone());
        assert_eq!(sidebar.read_with(&cx, |s, _| s.cursor()), 1, "Down in the sidebar moves its cursor");
        assert_eq!(selected(&view, &cx).as_deref(), Some("beta"), "…and leaves the list alone");

        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("beta"), "Return in the sidebar opens the favorite");
        let list_focus = pane(&view, &cx).read_with(&cx, |p, _| p.focus_handle().clone());
        assert!(cx.update(|window, _| list_focus.is_focused(window)), "…and moves focus to the list");

        cx.simulate_keystrokes("enter");
        assert!(pane(&view, &cx).read_with(&cx, |p, _| p.is_renaming()), "Return in the list means rename");
    }

    #[gpui::test]
    fn favorites_reorder_remove_and_add_from_the_keyboard(cx: &mut TestAppContext) {
        let root = fixture("favorites");
        let (view, mut cx) = open(cx, &root);
        let (alpha, beta) = (root.join("alpha"), root.join("beta"));
        write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&alpha, &beta]));
        assert_eq!(sidebar_paths(&cx), [alpha.clone(), beta.clone()]);

        cx.simulate_keystrokes("tab cmd-alt-down");
        settle(&view, &mut cx);
        assert_eq!(sidebar_paths(&cx), [beta.clone(), alpha.clone()], "Cmd-Option-Down moves the favorite down");
        let sidebar = view.read_with(&cx, |w, _| w.sidebar.clone());
        assert_eq!(sidebar.read_with(&cx, |s, _| s.cursor()), 1, "…and the cursor goes with it");
        let text = std::fs::read_to_string(config_dir(&cx).join("sidebar.toml")).unwrap();
        assert!(text.find("beta").unwrap() < text.find("alpha").unwrap(), "saved in sidebar.toml: {text}");

        cx.simulate_keystrokes("cmd-backspace");
        settle(&view, &mut cx);
        assert_eq!(sidebar_paths(&cx), std::slice::from_ref(&beta), "Cmd-Delete removes it from the sidebar");
        assert!(alpha.is_dir(), "…and only from the sidebar");

        // In the list, Ctrl-Cmd-T adds the selected folder (Finder's Add to Sidebar).
        cx.simulate_keystrokes("tab");
        assert_eq!(selected(&view, &cx).as_deref(), Some("alpha"));
        cx.simulate_keystrokes("ctrl-cmd-t");
        settle(&view, &mut cx);
        assert_eq!(sidebar_paths(&cx), [beta, alpha]);
    }

    #[gpui::test]
    fn dropping_on_a_favorite_moves_into_it(cx: &mut TestAppContext) {
        let root = fixture("drop-favorite");
        let (view, mut cx) = open(cx, &root);
        write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("beta")]));
        drop_on(&mut cx, "sidebar-0", vec![root.join("notes.txt")]);
        settle(&view, &mut cx);
        assert!(root.join("beta/notes.txt").exists());

        // Folders dropped on a group's title become favorites.
        drop_on(&mut cx, "sidebar-header-Favorites", vec![root.join("alpha")]);
        settle(&view, &mut cx);
        assert_eq!(sidebar_paths(&cx), [root.join("beta"), root.join("alpha")]);
        assert!(root.join("alpha").is_dir(), "added, not moved");
    }

    #[gpui::test]
    fn recent_lists_visited_folders_that_are_not_favorites(cx: &mut TestAppContext) {
        let root = fixture("recent");
        let (view, mut cx) = open(cx, &root);
        write_config(
            &view,
            &mut cx,
            "sidebar.toml",
            &favorites_toml(&[&root.join("beta")]).replace("show = false", "show = true"),
        );
        cx.simulate_keystrokes("cmd-down");
        settle(&view, &mut cx);
        let paths = sidebar_paths(&cx);
        assert!(paths.contains(&root.join("alpha")), "visited: {paths:?}");
        assert_eq!(paths.iter().filter(|p| **p == root.join("beta")).count(), 1, "favorites aren't repeated");
        let state = cx.update(|_, cx| cx.global::<Ui>().state_dir.clone());
        assert!(state.join("recent.json").exists(), "saved for next time");
    }

    #[gpui::test]
    fn view_changes_are_remembered_per_folder(cx: &mut TestAppContext) {
        let root = fixture("views");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-3");
        settle(&view, &mut cx);
        let text = std::fs::read_to_string(config_dir(&cx).join("views.toml")).unwrap();
        assert!(text.contains("view = \"columns\""), "saved as this folder's own setting: {text}");

        // A folder without its own setting shows the defaults…
        let mode = |cx: &VisualTestContext| pane(&view, cx).read_with(cx, |p, _| p.mode());
        pane(&view, &cx).update(&mut cx, |p, cx| p.navigate(root.join("beta"), cx));
        settle(&view, &mut cx);
        assert_eq!(mode(&cx), ViewMode::List);
        // …and going back shows the remembered one.
        cx.simulate_keystrokes("cmd-[");
        settle(&view, &mut cx);
        assert_eq!(mode(&cx), ViewMode::Columns);
    }

    #[gpui::test]
    fn rules_apply_live_and_why_explains(cx: &mut TestAppContext) {
        let root = fixture("rules");
        let (view, mut cx) = open(cx, &root);
        let sort = |cx: &VisualTestContext| pane(&view, cx).read_with(cx, |p, _| p.sort());
        assert_eq!(sort(&cx).key, crow_fs::SortKey::Name);

        let rules = format!("{}\n[[rule]]\npath = \"{}\"\nsort = \"size\"\n", crow_core::DEFAULT_VIEWS, root.display());
        write_config(&view, &mut cx, "views.toml", &rules);
        assert_eq!(sort(&cx).key, crow_fs::SortKey::Size, "an edited views.toml applies to the open folder");
        assert!(!sort(&cx).ascending, "sizes start largest first");

        cx.simulate_keystrokes("cmd-j");
        assert!(view.read_with(&cx, |w, _| w.why.is_some()), "Cmd-J asks why");
        let spoken = cx.update(|window, cx| {
            let _ = window;
            let w = view.read(cx);
            let ui = cx.global::<Ui>();
            ui.views.resolve(&w.pane().read(cx).current_dir(cx)).sort.1.to_string()
        });
        assert_eq!(spoken, format!("rule \"{}\" in views.toml", root.display()));
        cx.simulate_keystrokes("escape");
        assert!(view.read_with(&cx, |w, _| w.why.is_none()));

        // A broken file keeps the last good settings and says why.
        write_config(&view, &mut cx, "views.toml", "[defaults]\nsort = \"colour\"\n");
        assert_eq!(sort(&cx).key, crow_fs::SortKey::Size);
        let error = cx.update(|_, cx| cx.global::<Ui>().config_error());
        assert!(error.is_some_and(|e| e.contains("colour")));
    }

    fn use_vim(view: &Entity<CrowWindow>, cx: &mut VisualTestContext) {
        write_config(view, cx, "settings.toml", "keymap = \"vim\"\n");
    }

    #[gpui::test]
    fn vim_layer_moves_opens_and_goes_up(cx: &mut TestAppContext) {
        let root = fixture("vim");
        let (view, mut cx) = open(cx, &root);
        use_vim(&view, &mut cx);
        cx.simulate_keystrokes("j");
        assert_eq!(selected(&view, &cx).as_deref(), Some("beta"));
        cx.simulate_keystrokes("shift-g");
        assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"));
        cx.simulate_keystrokes("g g");
        assert_eq!(selected(&view, &cx).as_deref(), Some("alpha"));
        cx.simulate_keystrokes("n");
        assert_eq!(selected(&view, &cx).as_deref(), Some("alpha"), "letters don't select by name");
        cx.simulate_keystrokes("l");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("alpha"));
        cx.simulate_keystrokes("h");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root);
        assert_eq!(selected(&view, &cx).as_deref(), Some("alpha"), "h selects where you came from");
    }

    #[gpui::test]
    fn vim_marks_set_favorites_and_jump(cx: &mut TestAppContext) {
        let root = fixture("marks");
        let (view, mut cx) = open(cx, &root.join("alpha"));
        write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[]));
        use_vim(&view, &mut cx);
        cx.simulate_keystrokes("m");
        assert!(pane(&view, &cx).read_with(&cx, |p, _| p.pending_hint().is_some()), "m waits for a letter");
        cx.simulate_keystrokes("j");
        settle(&view, &mut cx);
        assert_eq!(sidebar_paths(&cx), [root.join("alpha")], "the folder became a favorite with mark j");
        let text = std::fs::read_to_string(config_dir(&cx).join("sidebar.toml")).unwrap();
        assert!(text.contains("key = \"j\""), "{text}");

        cx.simulate_keystrokes("h");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root);
        cx.simulate_keystrokes("' j");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("alpha"), "'j jumps to the mark");
        cx.simulate_keystrokes("' q");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("alpha"), "an unknown mark goes nowhere");
    }

    /// Two themes in a temp theme index, shared by every test (the variable is process-wide).
    fn theme_index() -> &'static Path {
        static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        DIR.get_or_init(|| {
            let dir = std::env::temp_dir().join(format!("crow-app-theme-index-{}", std::process::id()));
            std::fs::create_dir_all(dir.join("schemes")).unwrap();
            let base = include_str!("../../crow-theme/src/fallback.json");
            std::fs::write(dir.join("schemes/firebox.json"), base).unwrap();
            let light = base.replace("\"dark\"", "\"light\"").replace("FireBox", "Paper").replace("#111111", "#fafafa");
            std::fs::write(dir.join("schemes/paper.json"), light).unwrap();
            std::fs::write(dir.join("defaults.json"), r#"{"dark":"firebox","light":"paper"}"#).unwrap();
            // SAFETY: set once, before any test reads it; all tests see the same value.
            unsafe {
                std::env::set_var("THEMES_DIR", &dir);
                std::env::set_var("THEMES_STATE_DIR", dir.join("state"));
            }
            dir
        })
    }

    #[gpui::test]
    fn theme_picker_previews_reverts_and_saves(cx: &mut TestAppContext) {
        theme_index();
        let root = fixture("themes");
        let (view, mut cx) = open(cx, &root);
        let theme = |cx: &VisualTestContext| cx.read(|cx| cx.global::<Ui>().theme.name.clone());
        let start = theme(&cx);

        cx.simulate_keystrokes("cmd-alt-t");
        settle(&view, &mut cx);
        assert!(view.read_with(&cx, |w, _| w.picker.is_some()));
        // System, then FireBox (dark), then Paper (light).
        cx.simulate_keystrokes("down down");
        assert_eq!(theme(&cx), "Paper", "moving the cursor shows the theme");
        cx.simulate_keystrokes("escape");
        assert_eq!(theme(&cx), start, "Escape goes back");

        cx.simulate_keystrokes("cmd-alt-t");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("down down enter");
        settle(&view, &mut cx);
        assert_eq!(theme(&cx), "Paper");
        let settings = std::fs::read_to_string(config_dir(&cx).join("settings.toml")).unwrap();
        assert!(settings.contains("theme = \"paper\""), "{settings}");
        assert!(view.read_with(&cx, |w, _| w.picker.is_none()));
    }

    #[gpui::test]
    fn parent_selects_where_you_came_from_and_history_works(cx: &mut TestAppContext) {
        let root = fixture("history");
        let (view, mut cx) = open(cx, &root.join("alpha"));

        cx.simulate_keystrokes("cmd-up");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root);
        assert_eq!(selected(&view, &cx).as_deref(), Some("alpha"), "Going up selects the folder you came from");

        cx.simulate_keystrokes("down cmd-down");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("beta"));

        cx.simulate_keystrokes("cmd-[");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root);
        assert_eq!(selected(&view, &cx).as_deref(), Some("beta"), "Back selects the folder you left");

        cx.simulate_keystrokes("cmd-]");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("beta"));
    }

    #[gpui::test]
    fn tabs_open_next_to_the_active_one_and_close(cx: &mut TestAppContext) {
        let root = fixture("tabs");
        let (view, mut cx) = open(cx, &root);

        cx.simulate_keystrokes("cmd-t");
        settle(&view, &mut cx);
        assert_eq!(view.read_with(&cx, |w, _| w.tabs.len()), 2);
        assert_eq!(dir(&view, &cx), root, "A new tab opens on the current folder");

        cx.simulate_keystrokes("down cmd-down");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("cmd-shift-[");
        assert_eq!(dir(&view, &cx), root, "Each tab keeps its own folder");

        cx.simulate_keystrokes("cmd-w");
        assert_eq!(view.read_with(&cx, |w, _| w.tabs.len()), 1);
    }

    #[gpui::test]
    fn hidden_files_toggle_applies_to_every_tab(cx: &mut TestAppContext) {
        let root = fixture("hidden");
        std::fs::write(root.join(".secret"), "").expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        let count = |cx: &VisualTestContext| pane(&view, cx).read_with(cx, |p, cx| p.item_count(cx));
        assert_eq!(count(&cx), 3);

        cx.simulate_keystrokes("cmd-t cmd-shift-.");
        settle(&view, &mut cx);
        assert_eq!(count(&cx), 4);
        cx.simulate_keystrokes("cmd-shift-[");
        assert_eq!(count(&cx), 4, "The other tab shows hidden files too");
    }

    fn selected_names(view: &Entity<CrowWindow>, cx: &VisualTestContext) -> Vec<String> {
        pane(view, cx).read_with(cx, |p, cx| p.selected_names(cx))
    }

    fn names(view: &Entity<CrowWindow>, cx: &VisualTestContext) -> Vec<String> {
        pane(view, cx).read_with(cx, |p, cx| p.names(cx))
    }

    #[gpui::test]
    fn shift_arrows_extend_and_cmd_a_selects_all(cx: &mut TestAppContext) {
        let root = fixture("multi");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("down shift-down");
        assert_eq!(selected_names(&view, &cx), ["beta", "notes.txt"]);
        cx.simulate_keystrokes("shift-up shift-up");
        assert_eq!(selected_names(&view, &cx), ["alpha", "beta"], "the range pivots on where it started");
        cx.simulate_keystrokes("cmd-a");
        assert_eq!(selected_names(&view, &cx), ["alpha", "beta", "notes.txt"]);
        cx.simulate_keystrokes("down");
        assert_eq!(selected_names(&view, &cx), ["beta"], "a plain arrow collapses to one item");
    }

    #[gpui::test]
    fn typing_selects_by_name_and_space_continues_a_name(cx: &mut TestAppContext) {
        let root = fixture("typeselect");
        std::fs::write(root.join("my file.txt"), "").expect("create fixture");
        std::fs::write(root.join("my_other.txt"), "").expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("b e");
        assert_eq!(selected(&view, &cx).as_deref(), Some("beta"));
        std::thread::sleep(crow_core::TypeSelect::TIMEOUT + std::time::Duration::from_millis(50));
        cx.simulate_keystrokes("m y space f");
        assert_eq!(selected(&view, &cx).as_deref(), Some("my file.txt"), "Space mid-name is part of the name");
    }

    #[gpui::test]
    fn sorting_keeps_folders_first_and_the_selection(cx: &mut TestAppContext) {
        let root = fixture("sort");
        std::fs::write(root.join("big.bin"), vec![0u8; 5000]).expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("down down");
        let picked = selected(&view, &cx);
        pane(&view, &cx)
            .update(&mut cx, |p, cx| p.set_sort(crow_fs::Sort::default().toggled(crow_fs::SortKey::Size), cx));
        settle(&view, &mut cx);
        assert_eq!(names(&view, &cx), ["alpha", "beta", "big.bin", "notes.txt"], "largest file first, folders on top");
        assert_eq!(selected(&view, &cx), picked, "the selection follows the item, not the row");
    }

    #[gpui::test]
    fn go_to_folder_completes_navigates_and_reveals_files(cx: &mut TestAppContext) {
        let root = fixture("goto");
        let (view, mut cx) = open(cx, &root);
        let sheet_open = |cx: &VisualTestContext| view.read_with(cx, |w, _| w.go_to.is_some());

        cx.simulate_keystrokes("cmd-shift-g cmd-a");
        cx.simulate_input("al");
        cx.simulate_keystrokes("tab");
        let typed = view.read_with(&cx, |w, cx| w.go_to.as_ref().map(|(i, _)| i.read(cx).text().to_owned()));
        assert_eq!(typed.as_deref(), Some("alpha/"), "Tab completes a unique folder");
        cx.simulate_input("inner");
        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("alpha/inner"));
        assert!(!sheet_open(&cx), "Return closes the sheet");

        cx.simulate_keystrokes("cmd-shift-g cmd-a");
        cx.simulate_input(&root.join("notes.txt").display().to_string());
        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root, "a file opens its folder…");
        assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"), "…with the file selected");

        cx.simulate_keystrokes("cmd-shift-g escape");
        assert!(!sheet_open(&cx), "Escape closes the sheet");
        assert_eq!(dir(&view, &cx), root, "…without going anywhere");
    }

    /// (number of columns, focused column, folder shown in column `k`, cursor name in column `k`).
    fn columns(
        view: &Entity<CrowWindow>,
        cx: &VisualTestContext,
        k: usize,
    ) -> (usize, usize, Option<PathBuf>, Option<String>) {
        pane(view, cx)
            .read_with(cx, |p, cx| (p.folder_count(), p.focused_index(), p.folder_dir(k, cx), p.cursor_name_in(k, cx)))
    }

    #[gpui::test]
    fn column_view_follows_the_focus_rules(cx: &mut TestAppContext) {
        let root = fixture("columns");
        std::fs::write(root.join("alpha/zeta.txt"), "z").expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-3");
        settle(&view, &mut cx);
        let (count, focused, child, child_cursor) = columns(&view, &cx, 1);
        assert_eq!((count, focused), (2, 0), "the selected folder's contents show in a second column");
        assert_eq!(child, Some(root.join("alpha")));
        assert_eq!(child_cursor, None, "…which is not focused and has nothing selected");

        cx.simulate_keystrokes("down");
        settle(&view, &mut cx);
        assert_eq!(columns(&view, &cx, 1).2, Some(root.join("beta")), "the child column follows the selection");
        cx.simulate_keystrokes("down");
        settle(&view, &mut cx);
        assert_eq!(columns(&view, &cx, 0).0, 1, "a file ends the chain (its preview is the last column)");

        cx.simulate_keystrokes("up up right");
        settle(&view, &mut cx);
        let (count, focused, _, cursor) = columns(&view, &cx, 1);
        assert_eq!(focused, 1, "Right moves into the child column");
        assert_eq!(cursor.as_deref(), Some("inner"), "…selecting its first item");
        assert_eq!(count, 3, "…whose own contents open to the right");

        cx.simulate_keystrokes("z");
        assert_eq!(columns(&view, &cx, 1).3.as_deref(), Some("zeta.txt"), "typing selects in the focused column");
        assert_eq!(columns(&view, &cx, 0).3.as_deref(), Some("alpha"), "…and leaves the parent alone");
        assert_eq!(dir(&view, &cx), root.join("alpha"), "the window shows the focused column's folder");

        cx.simulate_keystrokes("left");
        let (count, focused, _, cursor) = columns(&view, &cx, 1);
        assert_eq!((count, focused), (2, 0), "Left moves back; the child column stays");
        assert_eq!(cursor, None, "…with its selection cleared");
    }

    #[gpui::test]
    fn switching_to_list_view_keeps_the_folder_you_are_in(cx: &mut TestAppContext) {
        let root = fixture("switch");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-3");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("right");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("cmd-2");
        settle(&view, &mut cx);
        assert_eq!(pane(&view, &cx).read_with(&cx, |p, _| p.mode()), ViewMode::List);
        assert_eq!(dir(&view, &cx), root.join("alpha"));
        assert_eq!(selected(&view, &cx).as_deref(), Some("inner"), "the selection comes along");
        cx.simulate_keystrokes("cmd-[");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root, "and Back returns to where the columns started");
    }

    fn sheet_open(view: &Entity<CrowWindow>, cx: &VisualTestContext) -> bool {
        view.read_with(cx, |w, _| w.sheet.is_some())
    }

    #[gpui::test]
    fn new_folder_starts_a_rename_and_return_applies_it(cx: &mut TestAppContext) {
        let root = fixture("newfolder");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-shift-n");
        settle(&view, &mut cx);
        cx.run_until_parked();
        let renaming = pane(&view, &cx).read_with(&cx, |p, cx| p.rename_text(cx));
        assert_eq!(renaming.as_deref(), Some("untitled folder"), "the new folder is created and its rename starts");
        cx.simulate_input("Projects");
        let typed = pane(&view, &cx).read_with(&cx, |p, cx| p.rename_text(cx));
        assert_eq!(typed.as_deref(), Some("Projects"), "the whole name was selected, so typing replaced it");
        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        let entries =
            std::fs::read_dir(&root).unwrap().filter_map(Result::ok).map(|e| e.file_name()).collect::<Vec<_>>();
        assert!(root.join("Projects").is_dir(), "renamed on Return; folder now has {entries:?}");
        assert!(!root.join("untitled folder").exists());
        assert_eq!(selected(&view, &cx).as_deref(), Some("Projects"));
    }

    #[gpui::test]
    fn trash_and_undo(cx: &mut TestAppContext) {
        let root = fixture("trash");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("down down"); // notes.txt
        assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"));
        cx.simulate_keystrokes("cmd-backspace");
        settle(&view, &mut cx);
        assert!(!root.join("notes.txt").exists());
        assert!(!names(&view, &cx).contains(&"notes.txt".to_owned()));
        cx.simulate_keystrokes("cmd-z");
        settle(&view, &mut cx);
        assert_eq!(
            std::fs::read_to_string(root.join("notes.txt")).ok().as_deref(),
            Some("hello\n"),
            "undo puts it back"
        );
        assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"), "…and selects it");
    }

    #[gpui::test]
    fn duplicate_and_paste_into_another_folder(cx: &mut TestAppContext) {
        let root = fixture("dup");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("down down cmd-d");
        settle(&view, &mut cx);
        assert!(root.join("notes copy.txt").exists());
        assert_eq!(selected(&view, &cx).as_deref(), Some("notes copy.txt"), "the duplicate is selected");

        cx.simulate_keystrokes("cmd-c up up up cmd-down"); // copy, then into alpha
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("alpha"));
        cx.simulate_keystrokes("cmd-v");
        settle(&view, &mut cx);
        assert!(root.join("alpha/notes copy.txt").exists(), "pasted into the folder you are in");
        assert!(root.join("notes copy.txt").exists(), "copy leaves the original");
    }

    #[gpui::test]
    fn conflicts_ask_first_and_keep_both_by_default(cx: &mut TestAppContext) {
        let root = fixture("conflict");
        std::fs::write(root.join("alpha/notes.txt"), "already here").expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("down down cmd-c up up cmd-down cmd-v");
        settle(&view, &mut cx);
        assert!(sheet_open(&view, &cx), "a conflict opens a sheet instead of overwriting");
        assert_eq!(std::fs::read_to_string(root.join("alpha/notes.txt")).ok().as_deref(), Some("already here"));
        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        assert!(!sheet_open(&view, &cx));
        assert_eq!(std::fs::read_to_string(root.join("alpha/notes 2.txt")).ok().as_deref(), Some("hello\n"));
        assert_eq!(std::fs::read_to_string(root.join("alpha/notes.txt")).ok().as_deref(), Some("already here"));
    }

    #[gpui::test]
    fn permanent_delete_needs_confirmation(cx: &mut TestAppContext) {
        let root = fixture("delete");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("down down cmd-alt-backspace");
        settle(&view, &mut cx);
        assert!(sheet_open(&view, &cx));
        cx.simulate_keystrokes("escape");
        settle(&view, &mut cx);
        assert!(root.join("notes.txt").exists(), "Escape cancels");
        cx.simulate_keystrokes("cmd-alt-backspace enter");
        settle(&view, &mut cx);
        assert!(!root.join("notes.txt").exists(), "Return confirms");
    }

    /// Drag `paths` in from outside the window and drop them on the element `selector`.
    ///
    /// The test platform makes a newly drawn frame the one that is hit-tested only at
    /// the next input event (the app presents frames continuously). So the pointer
    /// first moves over the target, as it does in a real drag, before the drop events.
    fn drop_on(cx: &mut VisualTestContext, selector: &'static str, paths: Vec<PathBuf>) {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let at = cx.debug_bounds(selector).unwrap_or_else(|| panic!("no element {selector}")).center();
        cx.simulate_mouse_move(at, None, gpui::Modifiers::default());
        cx.run_until_parked();
        cx.simulate_event(gpui::FileDropEvent::Entered { position: at, paths: gpui::ExternalPaths(paths.into()) });
        cx.simulate_event(gpui::FileDropEvent::Pending { position: at });
        cx.simulate_event(gpui::FileDropEvent::Submit { position: at });
    }

    #[gpui::test]
    fn dropping_on_a_folder_row_moves_into_that_folder(cx: &mut TestAppContext) {
        let root = fixture("drop-row");
        let (view, mut cx) = open(cx, &root);
        drop_on(&mut cx, "row-alpha", vec![root.join("notes.txt")]);
        settle(&view, &mut cx);
        assert!(root.join("alpha/notes.txt").exists(), "dropped onto alpha: moved into it");
        assert!(!root.join("notes.txt").exists(), "same volume: a move, like Finder");

        drop_on(&mut cx, "row-alpha", vec![root.join("alpha")]);
        settle(&view, &mut cx);
        assert!(root.join("alpha").is_dir(), "a folder dropped onto itself stays put");
        let message = cx.update(|_, cx| cx.global::<Operations>().message.clone());
        assert!(message.is_none_or(|(_, error)| !error), "…without an error");
    }

    #[gpui::test]
    fn dropping_on_the_list_moves_into_the_open_folder(cx: &mut TestAppContext) {
        let root = fixture("drop-list");
        let elsewhere = fixture("drop-elsewhere");
        let (view, mut cx) = open(cx, &root);
        drop_on(&mut cx, "row-notes.txt", vec![elsewhere.join("notes.txt")]);
        settle(&view, &mut cx);
        assert!(
            sheet_open(&view, &cx),
            "a file row is not a target: the drop lands in the folder, where the name clashes"
        );
        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        assert!(root.join("notes 2.txt").exists(), "kept both");
        assert!(!elsewhere.join("notes.txt").exists(), "moved, not copied");
    }

    #[gpui::test]
    fn dropping_in_column_view_targets_the_folder_under_the_pointer(cx: &mut TestAppContext) {
        let root = fixture("drop-columns");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-3");
        settle(&view, &mut cx);
        drop_on(&mut cx, "column-0-row-beta", vec![root.join("notes.txt")]);
        settle(&view, &mut cx);
        assert!(root.join("beta/notes.txt").exists());
    }
}
