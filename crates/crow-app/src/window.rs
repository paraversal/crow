//! One Crow window: title bar, tabs, sidebar, the active pane, preview and path bar.

use std::path::{Path, PathBuf};

use crow_core::{Favorite, PreviewContent, Tabs};
use gpui::{
    App, Bounds, Context, Entity, FocusHandle, Focusable, FontWeight, IntoElement, Render, Role, SharedString,
    Subscription, TitlebarOptions, Window, WindowBounds, WindowOptions, div, img, point, prelude::*, px, size,
};

use crate::actions::{
    CloseTab, GoBack, GoForward, GoToFolder, MoveDown, MoveUp, NewTab, NextTab, OpenParent, OpenSelection, PreviousTab,
    SwitchFocus, ToggleHiddenFiles,
};
use crate::pane::Pane;
use crate::style::{color, color_alpha, format_time, icon, kind, ui_font};
use crate::text_input::{TextInput, TextInputEvent};
use crate::ui::{Ui, reload_theme};

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
    favorites: Vec<Favorite>,
    sidebar_focus: FocusHandle,
    sidebar_cursor: usize,
    show_hidden: bool,
    /// The Go to Folder sheet, while open.
    go_to: Option<(Entity<TextInput>, Subscription)>,
    _subscriptions: Vec<Subscription>,
}

impl CrowWindow {
    fn new(dir: PathBuf, bench: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/".into()));
        let mut favorites = crow_core::default_favorites(&home);
        favorites.push(Favorite { name: "Macintosh HD".into(), path: PathBuf::from("/") });
        let show_hidden = cx.global::<Ui>().settings.show_hidden_files;
        let pane = cx.new(|cx| Pane::new(dir, show_hidden, bench, cx));
        let focus = pane.read(cx).focus_handle().clone();
        window.focus(&focus, cx);
        let subscriptions = vec![
            cx.observe(&pane, |_, _, cx| cx.notify()),
            cx.observe_window_appearance(window, |_, _, cx| reload_theme(cx)),
        ];
        Self {
            tabs: Tabs::new(pane),
            favorites,
            sidebar_focus: cx.focus_handle(),
            sidebar_cursor: 0,
            show_hidden,
            go_to: None,
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
        let dir = self.pane().read(cx).dir().to_path_buf();
        let show_hidden = self.show_hidden;
        let pane = cx.new(|cx| Pane::new(dir, show_hidden, false, cx));
        self._subscriptions.push(cx.observe(&pane, |_, _, cx| cx.notify()));
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
        if self.sidebar_focus.is_focused(window) {
            self.focus_pane(window, cx);
        } else {
            window.focus(&self.sidebar_focus, cx);
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
        let current = home_relative(self.pane().read(cx).dir());
        let input = cx.new(|cx| TextInput::new(current, "Go to Folder", cx));
        let subscription = cx.subscribe_in(&input, window, |this, input, event: &TextInputEvent, window, cx| {
            let typed = input.read(cx).text().to_owned();
            let cwd = this.pane().read(cx).dir().to_path_buf();
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

    fn open_parent(&mut self, _: &OpenParent, _: &mut Window, cx: &mut Context<Self>) {
        self.pane().update(cx, |pane, cx| pane.open_parent(cx));
    }

    fn go_back(&mut self, _: &GoBack, _: &mut Window, cx: &mut Context<Self>) {
        self.pane().update(cx, |pane, cx| pane.go_back(cx));
    }

    fn go_forward(&mut self, _: &GoForward, _: &mut Window, cx: &mut Context<Self>) {
        self.pane().update(cx, |pane, cx| pane.go_forward(cx));
    }

    // ----- sidebar actions (same keys as the list, different meaning) -----

    fn sidebar_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_cursor = self.sidebar_cursor.saturating_sub(1);
        cx.notify();
    }

    fn sidebar_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_cursor = (self.sidebar_cursor + 1).min(self.favorites.len().saturating_sub(1));
        cx.notify();
    }

    fn sidebar_open(&mut self, _: &OpenSelection, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(favorite) = self.favorites.get(self.sidebar_cursor) {
            let dir = favorite.path.clone();
            self.pane().update(cx, |pane, cx| pane.navigate(dir, cx));
            self.focus_pane(window, cx);
        }
    }

    // ----- rendering -----

    fn render_title_bar(&self, title: &str, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = &cx.global::<Ui>().theme;
        let pane = self.pane().read(cx);
        let (back, forward) = (pane.can_go_back(), pane.can_go_forward());
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
    }

    fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.global::<Ui>().theme.clone();
        let active = self.tabs.active_index();
        let count = self.tabs.len();
        let tabs: Vec<(usize, String)> =
            self.tabs.iter().enumerate().map(|(i, (_, pane))| (i, folder_name(pane.read(cx).dir()))).collect();
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

    fn render_sidebar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.global::<Ui>().theme.clone();
        let focused = self.sidebar_focus.is_focused(window);
        let current = self.pane().read(cx).dir().to_path_buf();
        let count = self.favorites.len();
        let rows: Vec<(usize, Favorite)> = self.favorites.iter().cloned().enumerate().collect();
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
            .bg(color(theme.role("bg-inset")))
            .border_r_1()
            .border_color(color(theme.role("border")))
            .flex()
            .flex_col()
            .gap(px(1.))
            .child(div().px_2().pb_1().text_size(px(11.)).text_color(color(theme.role("fg-muted"))).child("Favorites"))
            .children(rows.into_iter().map(|(i, favorite)| {
                let here = favorite.path == current;
                let cursor = focused && i == self.sidebar_cursor;
                let target = favorite.path.clone();
                div()
                    .id(("favorite", i))
                    .role(Role::ListBoxOption)
                    .aria_label(favorite.name.clone())
                    .aria_position_in_set(i + 1)
                    .aria_size_of_set(count)
                    .aria_selected(i == self.sidebar_cursor)
                    .when(i == self.sidebar_cursor, |row| row.aria_active_descendant())
                    .h(px(26.))
                    .px_2()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(here, |row| row.bg(color(theme.role("selection"))))
                    .when(cursor, |row| row.bg(color_alpha(theme.role("accent"), 0x55)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.sidebar_cursor = i;
                        let dir = target.clone();
                        this.pane().update(cx, |pane, cx| pane.navigate(dir, cx));
                        this.focus_pane(window, cx);
                    }))
                    .children(icon(&favorite.path, true, 16).map(|i| img(i).size(px(16.))))
                    .child(favorite.name)
            }))
    }

    fn render_preview(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.global::<Ui>().theme.clone();
        let muted = color(theme.role("fg-muted"));
        let border = color(theme.role("border"));
        let pane = self.pane().read(cx);
        let base = div()
            .id("preview")
            .role(Role::Group)
            .w(px(300.))
            .flex_none()
            .h_full()
            .p_4()
            .border_l_1()
            .border_color(border)
            .flex()
            .flex_col()
            .gap_3()
            .overflow_hidden();
        let Some(entry) = pane.cursor_entry().cloned() else { return base.aria_label("Preview") };
        let size = if entry.is_dir { "--".to_owned() } else { crow_fs::human_size(entry.size) };
        let kind = kind(&entry);
        let (created, modified, added) =
            (format_time(entry.created), format_time(entry.modified), format_time(entry.added));
        let preview = pane.preview();
        let tags = preview.map(|p| p.tags.join(", ")).filter(|t| !t.is_empty());
        let picture = match (preview.and_then(|p| p.image.clone()), preview.and_then(|p| p.content.clone())) {
            (Some(image), _) => {
                div().h(px(220.)).flex().items_center().justify_center().child(img(image).max_w_full().max_h(px(220.)))
            }
            (None, Some(PreviewContent::Text(text))) => div()
                .h(px(220.))
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(border)
                .bg(color(theme.role("bg-inset")))
                .overflow_hidden()
                .font_family("Menlo")
                .text_size(px(11.))
                .child(text),
            _ => div()
                .h(px(220.))
                .flex()
                .items_center()
                .justify_center()
                .children(icon(&entry.path, entry.is_dir, 128).map(|i| img(i).size(px(128.)))),
        };
        let info_row = |label: &str, value: String| {
            div()
                .flex()
                .justify_between()
                .gap_2()
                .py_1()
                .border_b_1()
                .border_color(border)
                .child(div().flex_none().text_color(muted).child(label.to_owned()))
                .child(div().text_right().child(value))
        };
        base.aria_label(format!(
            "Preview: {}, {kind}, {size}, created {created}, modified {modified}{}",
            entry.name,
            tags.as_ref().map(|t| format!(", tags {t}")).unwrap_or_default()
        ))
        .child(picture)
        .child(
            div()
                .flex()
                .flex_col()
                .child(div().text_size(px(15.)).child(entry.name.clone()))
                .child(div().text_color(muted).child(format!("{kind} · {size}"))),
        )
        .child(div().pt_2().font_weight(FontWeight::SEMIBOLD).child("Information"))
        .child(info_row("Created", created))
        .child(info_row("Modified", modified))
        .when(entry.added.is_some(), |pane| pane.child(info_row("Added", added)))
        .child(div().pt_2().font_weight(FontWeight::SEMIBOLD).child("Tags"))
        .child(match tags {
            Some(tags) => div().child(tags),
            None => div().text_color(muted).child("No tags"),
        })
    }

    fn render_path_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        let settings_error = ui.settings_error.clone();
        let pane = self.pane().read(cx);
        let target = pane.cursor_entry().map_or_else(|| pane.dir().to_path_buf(), |e| e.path.clone());
        let parts = crow_macos::display_components(&target);
        let last = parts.len().saturating_sub(1);
        let spoken = parts.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>().join(", ");
        let (status, is_error): (SharedString, bool) = match (pane.error().cloned(), settings_error) {
            (Some(e), _) | (None, Some(e)) => (e, true),
            _ if pane.is_loading() && pane.item_count() == 0 => ("Loading…".into(), false),
            _ if pane.selected_count() > 1 => {
                (format!("{} of {} selected", pane.selected_count(), pane.item_count()).into(), false)
            }
            _ => (format!("{} items", pane.item_count()).into(), false),
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
            .children(parts.into_iter().enumerate().map({
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
            }))
            .child(div().flex_1())
            .child(
                div()
                    .id("status")
                    .role(Role::Status)
                    .aria_label(status.clone())
                    .when(is_error, |s| s.text_color(color(theme.role("error"))))
                    .child(status),
            )
    }
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

impl Render for CrowWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        let (font, font_size) = (ui_font(&ui.settings.font_family), ui.settings.font_size);
        let title = folder_name(self.pane().read(cx).dir());
        window.set_window_title(&title);
        let title_bar = self.render_title_bar(&title, cx);
        let tab_bar = (self.tabs.len() > 1).then(|| self.render_tab_bar(cx));
        let sidebar = self.render_sidebar(window, cx);
        let preview = self.render_preview(cx);
        let path_bar = self.render_path_bar(cx);
        let go_to = self.render_go_to(cx);
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
                    .child(preview),
            )
            .child(path_bar)
            .children(go_to)
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
        cx.update(|cx| {
            crate::ui::init_for_tests(cx);
            cx.bind_keys(crate::actions::finder_keymap());
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
            let busy = view.read_with(cx, |w, cx| w.tabs.iter().any(|(_, pane)| pane.read(cx).is_loading()));
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
        pane(view, cx).read_with(cx, |p, _| p.cursor_entry().map(|e| e.name.clone()))
    }

    fn dir(view: &Entity<CrowWindow>, cx: &VisualTestContext) -> PathBuf {
        pane(view, cx).read_with(cx, |p, _| p.dir().to_path_buf())
    }

    #[gpui::test]
    fn same_keys_mean_different_things_in_sidebar_and_list(cx: &mut TestAppContext) {
        let root = fixture("contexts");
        let (view, mut cx) = open(cx, &root);
        view.update(&mut cx, |w, _| {
            w.favorites = vec![
                Favorite { name: "alpha".into(), path: root.join("alpha") },
                Favorite { name: "beta".into(), path: root.join("beta") },
            ];
        });
        assert_eq!(selected(&view, &cx).as_deref(), Some("alpha"));

        cx.simulate_keystrokes("down");
        assert_eq!(selected(&view, &cx).as_deref(), Some("beta"), "Down in the list moves the selection");

        cx.simulate_keystrokes("tab down");
        assert_eq!(view.read_with(&cx, |w, _| w.sidebar_cursor), 1, "Down in the sidebar moves its cursor");
        assert_eq!(selected(&view, &cx).as_deref(), Some("beta"), "…and leaves the list alone");

        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("beta"), "Return in the sidebar opens the favorite");
        let list_focus = pane(&view, &cx).read_with(&cx, |p, _| p.focus_handle().clone());
        assert!(cx.update(|window, _| list_focus.is_focused(window)), "…and moves focus to the list");

        cx.simulate_keystrokes("enter");
        let error = pane(&view, &cx).read_with(&cx, |p, _| p.error().cloned());
        assert!(error.is_some_and(|e| e.contains("Rename")), "Return in the list means rename");
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
        let count = |cx: &VisualTestContext| pane(&view, cx).read_with(cx, |p, _| p.item_count());
        assert_eq!(count(&cx), 3);

        cx.simulate_keystrokes("cmd-t cmd-shift-.");
        settle(&view, &mut cx);
        assert_eq!(count(&cx), 4);
        cx.simulate_keystrokes("cmd-shift-[");
        assert_eq!(count(&cx), 4, "The other tab shows hidden files too");
    }

    fn selected_names(view: &Entity<CrowWindow>, cx: &VisualTestContext) -> Vec<String> {
        pane(view, cx).read_with(cx, |p, _| p.selected_names())
    }

    fn names(view: &Entity<CrowWindow>, cx: &VisualTestContext) -> Vec<String> {
        pane(view, cx).read_with(cx, |p, _| p.names())
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
}
