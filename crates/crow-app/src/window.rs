//! One Crow window: title bar, tabs, sidebar, the active pane, preview and path bar.

use std::path::{Path, PathBuf};

use crow_core::{SidebarEdit, Tabs, ViewMode};
use gpui::{
    App, Bounds, Context, CursorStyle, Entity, FocusHandle, Focusable, FontWeight, IntoElement, Render, Role,
    ScrollStrategy, SharedString, Subscription, TitlebarOptions, UniformListScrollHandle, Window, WindowBounds,
    WindowOptions, div, img, point, prelude::*, px, size, transparent_black, uniform_list,
};

use crate::actions::{
    AddToSidebar, ApplyThemeEverywhere, CancelOperation, CloseTab, GoBack, GoForward, GoToFolder, KeyboardShortcuts,
    Minimize, MoveDown, MoveToTrash, MoveUp, NewTab, NewWindow, NextTab, OpenParent, PaletteOpenInTab, PreviousTab,
    ResetFolderView, SearchEverything, ShowCommandPalette, ShowSearch, ShowThemePicker, SwitchFocus, ToggleHiddenFiles,
    UseViewAsDefaults, ViewAsColumns, ViewAsList, WhyThisView, Zoom,
};
use crate::chrome::{self, Keycap, KeycapWeight, SheetWidth, with_action_tooltip, with_tooltip};
use crate::icons::{self, Glyph};
use crate::keys::{self, KeyScope};
use crate::operations::{self, Clipboard, Operations};
use crate::palette::{self, Item};
use crate::pane::Pane;
use crate::pane::PaneEvent;
use crate::preview;
use crate::search::{SearchEvent, SearchView};
use crate::sidebar::{Sidebar, SidebarEvent};
use crate::style::{active_selection, color, icon, ui_font};
use crate::text_input::{TextInput, TextInputEvent};
use crate::ui::{self, Ui, reload_theme};
use crow_fs::ops::{Plan, Request, Resolution};
use std::collections::HashMap;

/// Open a window on `dir`. New windows share one size from settings and step
/// 24 pt down and right from the previous one, so they never land at random sizes.
pub fn open(dir: PathBuf, bench: bool, bench_filter: Option<String>, cx: &mut App) {
    let defaults = cx.global::<Ui>().settings.window.clone();
    let step = px(24.0 * cx.windows().len() as f32);
    let mut bounds = Bounds::centered(None, size(px(defaults.width as f32), px(defaults.height as f32)), cx);
    bounds.origin = point(bounds.origin.x + step, bounds.origin.y + step);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some("Crow".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(14.), px(19.))),
        }),
        // Sidebar 210 plus a usable center; below this the band's parts no longer fit.
        window_min_size: Some(size(px(640.), px(400.))),
        ..Default::default()
    };
    if let Err(e) =
        cx.open_window(options, |window, cx| cx.new(|cx| CrowWindow::new(dir, bench, bench_filter, window, cx)))
    {
        tracing::error!("open window: {e}");
    }
}

/// What `CrowWindow::open_search` starts from. U3 reuses it with a query.
pub struct SearchStart {
    pub folder: PathBuf,
    pub query: String,
    pub everything: bool,
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
    /// Cmd-K, while open.
    palette: Option<Palette>,
    /// Cmd-F, while its results replace the pane.
    search: Option<Entity<SearchView>>,
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

/// Cmd-K: folders (favorites, Recent, subfolders) or, after `>`, commands.
struct Palette {
    input: Entity<TextInput>,
    items: Vec<Item>,
    cursor: usize,
    folders: Vec<palette::FolderCandidate>,
    scroll: UniformListScrollHandle,
    _subscription: Subscription,
}

impl Palette {
    /// The shortcut shown on row `i`, from the live keymap. Shared by
    /// `render_palette_rows` and the layer test.
    fn row_keys(&self, i: usize, cx: &App) -> Option<SharedString> {
        match self.items.get(i)? {
            Item::Command(command) => keys::text(command.action, command.scope, cx),
            Item::Folder { .. } => None,
        }
    }
}

impl CrowWindow {
    fn new(
        dir: PathBuf,
        bench: bool,
        bench_filter: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let show_hidden = cx.global::<Ui>().settings.show_hidden_files;
        if !bench {
            ui::record_visit(&dir, cx);
        }
        let pane = cx.new(|cx| Pane::new(dir.clone(), show_hidden, bench, bench_filter, cx));
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
            // Focus on an element that left the tree (an emptied results list, a hidden pane)
            // dispatches keys from the bare root, above the Crow context, so every key goes dead.
            cx.on_focus_lost(window, |this, window, cx| this.focus_center(window, cx)),
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
            palette: None,
            search: None,
            _subscriptions: subscriptions,
        }
    }

    fn pane(&self) -> &Entity<Pane> {
        self.tabs.active()
    }

    /// Focus whatever fills the center: the search field while search covers the pane,
    /// else the active pane. Never the pane under an open search, which isn't rendered.
    fn focus_center(&self, window: &mut Window, cx: &mut App) {
        if let Some(search) = &self.search {
            search.update(cx, |search, cx| search.focus_input(window, cx));
            return;
        }
        let handle = self.pane().read(cx).focus_handle().clone();
        window.focus(&handle, cx);
    }

    // ----- window actions -----

    fn new_window(&mut self, _: &NewWindow, _window: &mut Window, cx: &mut Context<Self>) {
        let dir = self.pane().read(cx).current_dir(cx);
        open(dir, false, None, cx);
    }

    fn minimize(&mut self, _: &Minimize, window: &mut Window, _: &mut Context<Self>) {
        window.minimize_window();
    }

    fn zoom(&mut self, _: &Zoom, window: &mut Window, _: &mut Context<Self>) {
        window.zoom_window();
    }

    fn new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self.pane().read(cx).current_dir(cx);
        self.open_tab_at(dir, window, cx);
    }

    fn open_tab_at(&mut self, dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.is_some() {
            self.close_search(window, cx);
        }
        let show_hidden = self.show_hidden;
        let pane = cx.new(|cx| Pane::new(dir, show_hidden, false, None, cx));
        self._subscriptions.push(cx.observe(&pane, |_, _, cx| cx.notify()));
        self._subscriptions
            .push(cx.subscribe_in(&pane, window, |this, _, event, window, cx| this.on_pane_event(event, window, cx)));
        self.tabs.open(pane);
        self.focus_center(window, cx);
        cx.notify();
    }

    fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.is_some() {
            self.close_search(window, cx);
        }
        if self.tabs.close_active().is_none() {
            window.remove_window();
            return;
        }
        self.focus_center(window, cx);
        cx.notify();
    }

    fn cycle_tab(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.is_some() {
            self.close_search(window, cx);
        }
        self.tabs.cycle(delta);
        self.focus_center(window, cx);
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
            self.focus_center(window, cx);
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
        if let Some(search) = &self.search {
            search.update(cx, |search, cx| search.set_show_hidden(show, cx));
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
                TextInputEvent::Changed => {}
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
        self.focus_center(window, cx);
        cx.notify();
    }

    fn render_go_to(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let (input, _) = self.go_to.as_ref()?;
        let theme = cx.global::<Ui>().theme.clone();
        Some(chrome::sheet(
            chrome::sheet_panel("go-to-folder", chrome::SheetWidth::Narrow, &theme)
                .aria_label("Go to Folder")
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
                .child(div().text_size(px(12.)).text_color(color(theme.role("fg-muted"))).child("Go to Folder"))
                .child(input.clone()),
            &theme,
            cx.listener(|this, _, window, cx| this.close_go_to(window, cx)),
        ))
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

    fn render_title_bar(&self, title: &str, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.global::<Ui>().theme.clone();
        let viewport = window.viewport_size().width;
        let narrow = viewport < px(760.);
        let (back, forward, mode, back_label, forward_label) = {
            let pane = self.pane().read(cx);
            let back_label = pane
                .back_target()
                .map_or_else(|| "Back".to_owned(), |dir| format!("Back to {}", crate::style::display_name(dir)));
            let forward_label = pane
                .forward_target()
                .map_or_else(|| "Forward".to_owned(), |dir| format!("Forward to {}", crate::style::display_name(dir)));
            (pane.can_go_back(), pane.can_go_forward(), pane.mode(), back_label, forward_label)
        };
        let fg = color(theme.role("fg"));
        let muted = color(theme.role("fg-muted"));
        let surface = color(theme.role("surface"));
        let hover_icon = fg;
        let hover_surface = surface;
        let pressed_surface = color(theme.role("surface-raised"));
        let nav_button = |id: &'static str,
                          aria: String,
                          tooltip: String,
                          icon: Glyph,
                          enabled: bool,
                          action: &dyn gpui::Action,
                          cx: &App|
         -> gpui::Stateful<gpui::Div> {
            let icon = icons::glyph(icon, 16., "fg-muted", &theme).id("icon");
            let icon = if enabled { icon.group_hover(id, move |s| s.text_color(hover_icon)) } else { icon };
            let aria = if enabled { aria } else { format!("{aria}, unavailable") };
            let mut button = div()
                .id(id)
                .debug_selector(move || id.into())
                .role(Role::Button)
                .aria_label(aria)
                .w(px(28.))
                .h(px(28.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .justify_center()
                .child(icon);
            if enabled {
                button = button.group(id).hover(move |s| s.bg(hover_surface)).active(move |s| s.bg(pressed_surface));
            } else {
                button = button.opacity(0.4);
            }
            if enabled {
                with_action_tooltip(button, tooltip, action, KeyScope::Window, cx)
            } else {
                with_tooltip(button, tooltip, None)
            }
        };
        let nav = div()
            .flex()
            .gap(px(2.))
            .child(
                nav_button("back", "Back".to_owned(), back_label, Glyph::Back, back, &GoBack, cx)
                    .on_click(cx.listener(|this, _, _, cx| this.pane().update(cx, |p, cx| p.go_back(cx)))),
            )
            .child(
                nav_button("forward", "Forward".to_owned(), forward_label, Glyph::Forward, forward, &GoForward, cx)
                    .on_click(cx.listener(|this, _, _, cx| this.pane().update(cx, |p, cx| p.go_forward(cx)))),
            );

        let title_label = div()
            .id("title")
            .role(Role::Heading)
            .aria_level(1)
            .aria_label(title.to_owned())
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .font_weight(FontWeight::SEMIBOLD)
            .text_size(px(15.))
            .text_color(fg)
            .child(title.to_owned());
        let views = div()
            .id("view-switcher")
            .role(Role::RadioGroup)
            .aria_label("View")
            .flex()
            .flex_none()
            .gap(px(2.))
            .p(px(2.))
            .rounded(px(8.))
            .bg(surface)
            .child(
                with_action_tooltip(
                    view_button("view-list", "as List", Glyph::List, mode == ViewMode::List, &theme),
                    "View as List",
                    &ViewAsList,
                    KeyScope::Window,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.pane().update(cx, |p, cx| p.set_mode(ViewMode::List, cx));
                })),
            )
            .child(
                with_action_tooltip(
                    view_button("view-columns", "as Columns", Glyph::Columns, mode == ViewMode::Columns, &theme),
                    "View as Columns",
                    &ViewAsColumns,
                    KeyScope::Window,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.pane().update(cx, |p, cx| p.set_mode(ViewMode::Columns, cx));
                })),
            );

        let go_to_keys = Keycap::for_action(&ShowCommandPalette, KeyScope::Window, KeycapWeight::Outline, cx);
        let go_to = div()
            .id("toolbar-go-to")
            .debug_selector(|| "toolbar-go-to".into())
            .role(Role::Button)
            .aria_label("Go to, Command K")
            .aria_keyshortcuts("Command+K")
            .flex_none()
            .h(px(30.))
            .rounded(px(8.))
            .flex()
            .items_center()
            .text_size(px(13.))
            .text_color(muted)
            .group("go-to")
            .hover(move |style| style.bg(surface))
            .on_click(|_, window, cx| window.dispatch_action(Box::new(ShowCommandPalette), cx))
            .when(narrow, |button| button.w(px(30.)).justify_center())
            .when(!narrow, |button| button.pl(px(10.)).pr(px(8.)).gap(px(6.)))
            .child(
                icons::glyph(Glyph::GoTo, 15., "fg-muted", &theme)
                    .id("go-to-icon")
                    .group_hover("go-to", move |style| style.text_color(fg)),
            )
            .when(!narrow, |button| {
                button.child(
                    div().id("go-to-label").group_hover("go-to", move |style| style.text_color(fg)).child("Go to"),
                )
            })
            .when(!narrow, |button| button.children(go_to_keys));
        let go_to = with_action_tooltip(go_to, "Go to folder or command", &ShowCommandPalette, KeyScope::Window, cx);

        let search_keys = Keycap::for_action(&ShowSearch, KeyScope::Window, KeycapWeight::Outline, cx);
        let search_field = div()
            .id("toolbar-search")
            .debug_selector(|| "toolbar-search".into())
            .role(Role::Button)
            .aria_label(format!("Search {title}, Command F"))
            .aria_keyshortcuts("Command+F")
            .w(px(240.))
            .min_w(px(160.))
            .h(px(30.))
            .flex_shrink(1.)
            .rounded(px(8.))
            .bg(surface)
            .pl(px(10.))
            .pr(px(6.))
            .gap(px(8.))
            .flex()
            .items_center()
            .border_1()
            .border_color(transparent_black())
            .hover(|style| style.border_color(color(theme.role("border"))))
            .cursor(CursorStyle::IBeam)
            .on_click(|_, window, cx| window.dispatch_action(Box::new(ShowSearch), cx))
            .child(icons::glyph(Glyph::Search, 14., "fg-muted", &theme))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(13.))
                    .text_color(muted)
                    .child(format!("Search {title}")),
            )
            .children(search_keys);
        let search_field =
            with_action_tooltip(search_field, "Search names and contents", &ShowSearch, KeyScope::Window, cx);

        let band = self
            .search
            .as_ref()
            .map(|search| search.update(cx, |search, cx| search.render_band(window, cx).into_any_element()));

        div()
            .id("toolbar")
            .role(Role::Toolbar)
            .aria_label("Toolbar")
            .h(px(52.))
            .flex_none()
            .pl(px(86.))
            .pr_3()
            .flex()
            .items_center()
            .gap(px(10.))
            .border_b_1()
            .border_color(color(theme.role("border")))
            .child(nav)
            .when(self.search.is_some(), |bar| bar.when_some(band, |bar, band| bar.child(band)))
            .when(self.search.is_none(), |bar| bar.child(title_label).child(views).child(go_to).child(search_field))
    }

    fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.global::<Ui>().theme.clone();
        let active = self.tabs.active_index();
        let count = self.tabs.len();
        let tabs: Vec<(usize, String)> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(i, (_, pane))| (i, crate::style::display_name(&pane.read(cx).current_dir(cx))))
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
                    .debug_selector(move || format!("tab-{i}"))
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
                        if i == this.tabs.active_index() {
                            return;
                        }
                        if this.search.is_some() {
                            this.close_search(window, cx);
                        }
                        this.tabs.activate(i);
                        this.focus_center(window, cx);
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
        let selected_entry = pane.cursor_entry(cx);
        let target = selected_entry.as_ref().map_or_else(|| pane.current_dir(cx), |e| e.path.clone());
        let parts = crow_macos::display_components(&target);
        let last = parts.len().saturating_sub(1);
        // The last segment is a file when a file is selected; every other segment is a folder.
        let last_is_dir = selected_entry.as_ref().is_none_or(|e| e.is_dir);
        let path_pane = self.pane().clone();
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
                        let hover = color(theme.role("surface-raised"));
                        move |(i, (name, path))| {
                            let pane = path_pane.clone();
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
                                .when(i == last, |part| part.text_color(fg))
                                .hover(move |s| s.bg(hover))
                                .children(icon(&path, is_dir, 14).map(|i| img(i).size(px(14.))))
                                .child(name)
                                .when(i < last, |part| part.child(div().pl_1().child("›")));
                            let target = path.clone();
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
                bar.child(with_action_tooltip(
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
                    "Cancel operation",
                    &CancelOperation,
                    KeyScope::Window,
                    cx,
                ))
            })
    }
}

fn view_button(
    id: &'static str,
    label: &'static str,
    icon: Glyph,
    selected: bool,
    theme: &crow_theme::Theme,
) -> gpui::Stateful<gpui::Div> {
    let hover_icon = color(theme.role("fg"));
    div()
        .id(id)
        .debug_selector(move || id.into())
        .role(Role::RadioButton)
        .aria_label(label)
        .aria_selected(selected)
        .w(px(32.))
        .h(px(26.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .group(id)
        .when(selected, |b| b.bg(color(theme.role("surface-raised"))))
        .child(
            icons::glyph(icon, 16., if selected { "fg" } else { "fg-muted" }, theme)
                .id("icon")
                .group_hover(id, move |s| s.text_color(hover_icon)),
        )
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
            PaneEvent::Copy { paths, cut } => self.copy_paths(paths.clone(), *cut, cx),
            PaneEvent::Entered(dir) => {
                if self.search.is_some() {
                    self.close_search(window, cx);
                }
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
        self.focus_center(window, cx);
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
        self.focus_center(window, cx);
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
        let trash_hint = match keys::text(&MoveToTrash, KeyScope::List, cx) {
            Some(keys) => format!("This can't be undone. Move to Trash ({keys}) can."),
            None => "This can't be undone. Move to Trash can.".to_owned(),
        };
        let button = |id: &'static str, label: String, primary: bool| {
            div()
                .id(id)
                .debug_selector(move || id.into())
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
                        .child(div().text_color(color(muted)).child(trash_hint))
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
                let into = plan
                    .items
                    .first()
                    .and_then(|i| i.target.parent())
                    .map(crate::style::display_name)
                    .unwrap_or_default();
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
                        .when(on, |o| o.bg(active_selection(&theme)).text_color(color(fg)))
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
        Some(chrome::sheet(
            chrome::sheet_panel("sheet", SheetWidth::Narrow, &theme)
                .aria_label(title.clone())
                // keys::context_stack (KeyScope::Sheet) mirrors this.
                .key_context("Sheet")
                .track_focus(sheet.focus())
                .on_action(
                    cx.listener(|this, _: &crate::actions::SheetConfirm, window, cx| this.confirm_sheet(window, cx)),
                )
                .on_action(
                    cx.listener(|this, _: &crate::actions::SheetCancel, window, cx| this.close_sheet(window, cx)),
                )
                .p_4()
                .flex()
                .flex_col()
                .gap_3()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                .child(body),
            &theme,
            cx.listener(|this, _, window, cx| this.close_sheet(window, cx)),
        ))
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
                self.focus_center(window, cx);
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
            self.focus_center(window, cx);
        } else {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            self.why = Some(focus);
        }
        cx.notify();
    }

    fn close_why(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.why = None;
        self.focus_center(window, cx);
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
        Some(chrome::sheet(
            chrome::sheet_panel("why-this-view", SheetWidth::Wide, &theme)
                .aria_label(format!("Why this view: {folder}"))
                // keys::context_stack (KeyScope::Sheet) mirrors this.
                .key_context("Sheet")
                .track_focus(focus)
                .on_action(cx.listener(|this, _: &crate::actions::SheetConfirm, window, cx| this.close_why(window, cx)))
                .on_action(cx.listener(|this, _: &crate::actions::SheetCancel, window, cx| this.close_why(window, cx)))
                .p_4()
                .flex()
                .flex_col()
                .gap_2()
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
            &theme,
            cx.listener(|this, _, window, cx| this.close_why(window, cx)),
        ))
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
        self.focus_center(window, cx);
        cx.notify();
    }

    fn cancel_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.picker = None;
        cx.global_mut::<Ui>().preview_theme = None;
        reload_theme(cx);
        self.focus_center(window, cx);
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
        self.focus_center(window, cx);
        cx.notify();
    }

    // ----- search -----

    fn show_search(&mut self, _: &ShowSearch, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(search) = &self.search {
            search.update(cx, |search, cx| search.select_all_input(window, cx));
            // The band's focus ring is drawn from the input's focus state.
            cx.notify();
            return;
        }
        // Another overlay owns the keys; don't move the filter's text into a search
        // that cannot open.
        if self.search_blocked() {
            return;
        }
        let pane = self.pane().clone();
        let folder = pane.read(cx).current_dir(cx);
        let query = pane.update(cx, |pane, cx| pane.take_filter(cx)).unwrap_or_default();
        self.open_search(SearchStart { folder, query, everything: false }, window, cx);
    }

    /// Whether an overlay other than search is open (search itself is checked by `open_search`).
    fn search_blocked(&self) -> bool {
        self.search.is_some()
            || self.picker.is_some()
            || self.go_to.is_some()
            || self.sheet.is_some()
            || self.palette.is_some()
    }

    fn search_everything(&mut self, _: &SearchEverything, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(search) = &self.search {
            search.update(cx, |search, cx| {
                let on = !search.everything();
                search.set_everything(on, cx);
                search.focus_input(window, cx);
            });
            return;
        }
        let folder = self.pane().read(cx).current_dir(cx);
        self.open_search(SearchStart { folder, query: String::new(), everything: true }, window, cx);
    }

    /// Opens the search panel (Scope Folder) on `folder`, with `query` in the field and
    /// the rules set by `everything`. Does nothing while another overlay is open or
    /// search is already open.
    fn open_search(&mut self, start: SearchStart, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.search_blocked() {
            return false;
        }
        let (home, show_hidden) = (cx.global::<Ui>().home.clone(), self.show_hidden);
        let search =
            cx.new(|cx| SearchView::new(start.folder, home, show_hidden, start.query, start.everything, window, cx));
        self._subscriptions.push(cx.observe(&search, |_, _, cx| cx.notify()));
        self._subscriptions.push(cx.subscribe_in(&search, window, |this, search, event, window, cx| {
            this.on_search_event(search.clone(), event, window, cx)
        }));
        search.update(cx, |search, cx| search.focus_input(window, cx));
        self.search = Some(search);
        cx.notify();
        true
    }

    fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(search) = self.search.take() {
            search.update(cx, |search, cx| search.cancel(cx));
            self.focus_center(window, cx);
            cx.notify();
        }
    }

    fn on_search_event(
        &mut self,
        search: Entity<SearchView>,
        event: &SearchEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SearchEvent::Close => self.close_search(window, cx),
            SearchEvent::FocusResults => {
                // With no results there is no list to focus; the Tab cycle goes on to the sidebar.
                if !search.update(cx, |search, cx| search.focus_results(window, cx)) {
                    let sidebar = self.sidebar.read(cx).focus_handle(cx);
                    window.focus(&sidebar, cx);
                    cx.notify();
                }
            }
            SearchEvent::Reveal(path) => {
                self.pane().update(cx, |pane, cx| pane.reveal(path.clone(), cx));
                self.close_search(window, cx);
                self.focus_center(window, cx);
            }
            SearchEvent::Open { path, is_dir } => {
                if *is_dir {
                    self.pane().update(cx, |pane, cx| pane.navigate(path.clone(), cx));
                    self.close_search(window, cx);
                    self.focus_center(window, cx);
                } else {
                    self.pane().update(cx, |pane, cx| pane.open_file(path.clone(), cx));
                }
            }
            SearchEvent::Run(request) => {
                self.close_search(window, cx);
                self.run_request(request.clone(), window, cx);
            }
            SearchEvent::Copy { paths, cut } => self.copy_paths(paths.clone(), *cut, cx),
        }
    }

    fn copy_paths(&mut self, paths: Vec<PathBuf>, cut: bool, cx: &mut Context<Self>) {
        let change_count = crow_macos::write_file_urls(&paths);
        let n = paths.len();
        let ops = cx.global_mut::<Operations>();
        ops.clipboard = Some(Clipboard { paths, cut, change_count });
        let what = if n == 1 { "1 item".to_owned() } else { format!("{n} items") };
        ops.message =
            Some((if cut { format!("Cut {what}: paste to move") } else { format!("Copied {what}") }.into(), false));
        cx.notify();
    }

    // ----- command palette -----

    fn show_palette(&mut self, _: &ShowCommandPalette, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette("", window, cx);
    }

    /// Help ▸ Crow Keyboard Shortcuts starts the palette in command mode.
    fn keyboard_shortcuts(&mut self, _: &KeyboardShortcuts, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette(">", window, cx);
    }

    fn open_palette(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some()
            || self.picker.is_some()
            || self.go_to.is_some()
            || self.sheet.is_some()
            || self.search.is_some()
        {
            return;
        }
        let (home, favorites, recents) = {
            let ui = cx.global::<Ui>();
            let favorites: Vec<PathBuf> =
                ui.favorites().into_iter().flat_map(|(_, group)| group).map(|f| f.path).collect();
            (ui.home.clone(), favorites, ui.palette_recents.clone())
        };
        let subfolders = self.pane().read(cx).subfolders(cx);
        let folders = palette::folder_candidates(favorites, recents, subfolders, &home);
        let items = palette::items(query, &folders);
        let input = cx.new(|cx| TextInput::new(query, "Go to Folder or > for Commands", cx));
        let subscription = cx.subscribe_in(&input, window, |this, input, event, window, cx| match event {
            TextInputEvent::Changed => {
                let query = input.read(cx).text().to_owned();
                this.refresh_palette(&query, cx);
            }
            TextInputEvent::Confirmed => this.confirm_palette(false, window, cx),
            TextInputEvent::Cancelled => this.close_palette(window, cx),
            TextInputEvent::CompletionRequested => {}
        });
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.palette = Some(Palette {
            input,
            items,
            cursor: 0,
            folders,
            scroll: UniformListScrollHandle::new(),
            _subscription: subscription,
        });
        cx.notify();
    }

    fn refresh_palette(&mut self, query: &str, cx: &mut Context<Self>) {
        let Some(palette) = &mut self.palette else { return };
        let items = palette::items(query, &palette.folders);
        palette.items = items;
        palette.cursor = 0;
        palette.scroll.scroll_to_item(0, ScrollStrategy::Center);
        cx.notify();
    }

    fn move_palette(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(palette) = &mut self.palette else { return };
        let last = palette.items.len().saturating_sub(1);
        palette.cursor = palette.cursor.saturating_add_signed(delta).min(last);
        palette.scroll.scroll_to_item(palette.cursor, ScrollStrategy::Center);
        cx.notify();
    }

    fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.take().is_none() {
            return;
        }
        self.focus_center(window, cx);
        cx.notify();
    }

    fn palette_open_in_tab(&mut self, _: &PaletteOpenInTab, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_palette(true, window, cx);
    }

    fn confirm_palette(&mut self, new_tab: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(palette) = self.palette.take() else { return };
        let item = palette.items.get(palette.cursor).cloned();
        self.focus_center(window, cx);
        match item {
            Some(Item::Command(command)) => self.run_command(command.id, window, cx),
            Some(Item::Folder { path, .. }) if new_tab => self.open_tab_at(path, window, cx),
            Some(Item::Folder { path, .. }) => self.pane().update(cx, |pane, cx| pane.navigate(path, cx)),
            None => {}
        }
        cx.notify();
    }

    fn run_command(&mut self, command: palette::CommandId, window: &mut Window, cx: &mut Context<Self>) {
        use palette::CommandId;
        match command {
            CommandId::NewTab => self.new_tab(&NewTab, window, cx),
            CommandId::CloseTab => self.close_tab(&CloseTab, window, cx),
            CommandId::NextTab => self.next_tab(&NextTab, window, cx),
            CommandId::PreviousTab => self.previous_tab(&PreviousTab, window, cx),
            CommandId::NewWindow => {
                let dir = self.pane().read(cx).current_dir(cx);
                open(dir, false, None, cx);
            }
            CommandId::SwitchFocus => self.switch_focus(&SwitchFocus, window, cx),
            CommandId::ToggleHiddenFiles => self.toggle_hidden(&ToggleHiddenFiles, window, cx),
            CommandId::OpenParent => self.open_parent(&OpenParent, window, cx),
            CommandId::GoBack => self.go_back(&GoBack, window, cx),
            CommandId::GoForward => self.go_forward(&GoForward, window, cx),
            CommandId::GoToFolder => self.go_to_folder(&GoToFolder, window, cx),
            CommandId::ViewAsList => self.view_as_list(&ViewAsList, window, cx),
            CommandId::ViewAsColumns => self.view_as_columns(&ViewAsColumns, window, cx),
            CommandId::Undo => self.undo(None, cx),
            CommandId::Redo => self.redo(cx),
            CommandId::AddToSidebar => self.add_to_sidebar(&AddToSidebar, window, cx),
            CommandId::WhyThisView => self.why_this_view(&WhyThisView, window, cx),
            CommandId::UseViewAsDefaults => self.use_view_as_defaults(&UseViewAsDefaults, window, cx),
            CommandId::ResetFolderView => self.reset_folder_view(&ResetFolderView, window, cx),
            CommandId::ShowThemePicker => self.show_theme_picker(&ShowThemePicker, window, cx),
            CommandId::ShowSearch => self.show_search(&ShowSearch, window, cx),
            CommandId::SearchEverything => self.search_everything(&SearchEverything, window, cx),
            // Reached from the palette? No: ShowCommandPalette is menu-only and SearchReveal needs results.
            CommandId::ShowCommandPalette | CommandId::SearchReveal => {}
            CommandId::ToggleActivity => {
                self.show_activity = !self.show_activity;
                cx.notify();
            }
            CommandId::RemoveFromSidebar
            | CommandId::MoveFavoriteUp
            | CommandId::MoveFavoriteDown
            | CommandId::Eject => {
                self.sidebar.update(cx, |sidebar, cx| sidebar.run_command(command, window, cx));
            }
            CommandId::Quit => cx.quit(),
            other => self.pane().update(cx, |pane, cx| pane.run_command(other, window, cx)),
        }
    }

    fn render_palette(&mut self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let (input, scroll, count) = {
            let palette = self.palette.as_ref()?;
            (palette.input.clone(), palette.scroll.clone(), palette.items.len())
        };
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        let vim = ui.settings.keymap == crow_core::KeymapLayer::Vim;
        let muted = color(theme.role("fg-muted"));
        let mut context = "Sheet Palette".to_owned();
        if vim {
            context.push_str(" vim");
        }
        let list: gpui::AnyElement = if count == 0 {
            div()
                .id("palette-empty")
                .p_2()
                .role(Role::Label)
                .aria_label("No matches")
                .child("No matches")
                .into_any_element()
        } else {
            let height = (count.min(14) as f32) * 26.0;
            div()
                .id("palette-list")
                .role(Role::ListBox)
                .aria_label("Commands and folders")
                .h(px(height))
                .flex()
                .flex_col()
                .child(
                    uniform_list("palette-items", count, cx.processor(Self::render_palette_rows))
                        .track_scroll(&scroll)
                        .flex_1(),
                )
                .into_any_element()
        };
        let run = keys::text(&crate::text_input::Confirm, KeyScope::Palette, cx)
            .map_or_else(|| "Return".to_owned(), |key| key.to_string());
        let new_tab = keys::text(&PaletteOpenInTab, KeyScope::Palette, cx)
            .map_or_else(|| "Return".to_owned(), |key| key.to_string());
        let footer = format!("> commands · {run} runs · {new_tab} opens a folder in a new tab");
        Some(chrome::sheet(
            chrome::sheet_panel("palette", SheetWidth::Wide, &theme)
                .aria_label("Command Palette")
                // keys::context_stack (KeyScope::Palette/ThemePicker) mirrors this.
                .key_context(context.as_str())
                .on_action(cx.listener(|this, _: &MoveUp, _, cx| this.move_palette(-1, cx)))
                .on_action(cx.listener(|this, _: &MoveDown, _, cx| this.move_palette(1, cx)))
                .on_action(cx.listener(Self::palette_open_in_tab))
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
                .child(input)
                .child(list)
                .child(div().text_size(px(12.)).text_color(muted).child(footer)),
            &theme,
            cx.listener(|this, _, window, cx| this.close_palette(window, cx)),
        ))
    }

    fn render_palette_rows(
        &mut self,
        range: std::ops::Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<impl IntoElement + use<>> {
        let Some(palette) = self.palette.as_ref() else { return Vec::new() };
        let ui = cx.global::<Ui>();
        let theme = &ui.theme;
        let (fg, muted) = (color(theme.role("fg")), color(theme.role("fg-muted")));
        let (cursor, count) = (palette.cursor, palette.items.len());
        range
            .filter_map(|i| {
                let item = palette.items.get(i)?;
                let selected = i == cursor;
                let keys = palette.row_keys(i, cx);
                let (icon_path, label) = match item {
                    Item::Command(command) => (None, command.label.to_owned()),
                    Item::Folder { path, label } => (Some(path.clone()), label.clone()),
                };
                let row = div()
                    .id(("palette-row", i))
                    .role(Role::ListBoxOption)
                    .aria_label(label.clone())
                    .aria_position_in_set(i + 1)
                    .aria_size_of_set(count)
                    .aria_selected(selected)
                    .when(selected, |row| row.aria_active_descendant())
                    .h(px(26.))
                    .px_2()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(selected, |row| row.bg(active_selection(theme)))
                    .debug_selector(|| format!("palette-row-{i}"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(palette) = &mut this.palette {
                            palette.cursor = i;
                        }
                        this.confirm_palette(false, window, cx);
                    }))
                    .child(div().size(px(16.)).flex_none().children(
                        icon_path.and_then(|path| icon(&path, true, 16)).map(|image| img(image).size(px(16.))),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_color(fg)
                            .child(label),
                    )
                    .child(div().text_size(px(11.)).text_color(muted).children(keys));
                Some(row.into_any_element())
            })
            .collect()
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
                    .when(selected, |r| r.bg(active_selection(&theme)))
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
        let keep = keys::text(&crate::actions::SheetConfirm, KeyScope::ThemePicker, cx);
        let everywhere = keys::text(&ApplyThemeEverywhere, KeyScope::ThemePicker, cx);
        let hint = match (keep, everywhere) {
            (Some(keep), Some(everywhere)) => format!(
                "{keep} keeps it for Crow. Apply to All Apps ({everywhere}) sets it for every app that reads the theme index."
            ),
            _ => "Apply to All Apps sets it for every app that reads the theme index.".to_owned(),
        };
        Some(chrome::sheet(
            chrome::sheet_panel("theme-picker", SheetWidth::Compact, &theme)
                .aria_label("Theme")
                // keys::context_stack (KeyScope::Palette/ThemePicker) mirrors this.
                .key_context(context.as_str())
                .track_focus(&picker.focus)
                .on_action(cx.listener(|this, _: &MoveUp, _, cx| this.move_picker(-1, cx)))
                .on_action(cx.listener(|this, _: &MoveDown, _, cx| this.move_picker(1, cx)))
                .on_action(
                    cx.listener(|this, _: &crate::actions::SheetConfirm, window, cx| this.confirm_picker(window, cx)),
                )
                .on_action(
                    cx.listener(|this, _: &crate::actions::SheetCancel, window, cx| this.cancel_picker(window, cx)),
                )
                .on_action(cx.listener(Self::apply_everywhere))
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
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
                .child(div().text_size(px(12.)).text_color(color(muted)).child(hint))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(button("theme-everywhere", "Apply to All Apps", false).on_click(
                            cx.listener(|this, _, window, cx| this.apply_everywhere(&ApplyThemeEverywhere, window, cx)),
                        ))
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
            &theme,
            cx.listener(|this, _, window, cx| this.cancel_picker(window, cx)),
        ))
    }
}

impl Render for CrowWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        let (font, font_size) = (ui_font(&ui.settings.font_family), ui.settings.font_size);
        let title = crate::style::display_name(&self.pane().read(cx).current_dir(cx));
        let list_view = self.pane().read(cx).mode() == ViewMode::List;
        window.set_window_title(&title);
        let title_bar = self.render_title_bar(&title, window, cx);
        let tab_bar = (self.tabs.len() > 1).then(|| self.render_tab_bar(cx));
        let current = self.pane().read(cx).current_dir(cx);
        self.sidebar.update(cx, |s, cx| s.set_current(current, cx));
        let sidebar = self.sidebar.clone();
        let preview = (list_view && self.search.is_none()).then(|| self.render_preview(cx));
        let path_bar = self.render_path_bar(cx);
        let go_to = self.render_go_to(cx);
        let sheet = self.render_sheet(cx);
        let activity = self.show_activity.then(|| self.render_activity(cx));
        let why = self.render_why(cx);
        let picker = self.render_picker(cx);
        let palette = self.render_palette(cx);
        let center: gpui::AnyElement = match &self.search {
            Some(search) => div().flex_1().min_w_0().h_full().child(search.clone()).into_any_element(),
            None => div().flex_1().min_w_0().h_full().child(self.pane().clone()).into_any_element(),
        };
        div()
            .id("crow")
            .role(Role::Application)
            .aria_label("Crow")
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                if cx.has_active_drag() {
                    this.pane().update(cx, |pane, cx| pane.update_drag_target(event.position, cx));
                }
            }))
            // keys::context_stack (KeyScope::Window) mirrors this; every overlay nests under it.
            .key_context("Crow")
            .on_action(cx.listener(Self::new_tab))
            .on_action(cx.listener(Self::new_window))
            .on_action(cx.listener(Self::minimize))
            .on_action(cx.listener(Self::zoom))
            .on_action(cx.listener(Self::keyboard_shortcuts))
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
            .on_action(cx.listener(|this, _: &crate::actions::CancelOperation, window, cx| {
                // Esc stops a running operation first; with none, it closes an open search
                // from wherever focus is (the sidebar, say).
                if cx.global::<Operations>().running.is_some() {
                    operations::cancel_running(cx);
                } else {
                    this.close_search(window, cx);
                }
            }))
            .on_action(cx.listener(Self::add_to_sidebar))
            .on_action(cx.listener(Self::why_this_view))
            .on_action(cx.listener(Self::use_view_as_defaults))
            .on_action(cx.listener(Self::reset_folder_view))
            .on_action(cx.listener(Self::show_theme_picker))
            .on_action(cx.listener(Self::show_palette))
            .on_action(cx.listener(Self::show_search))
            .on_action(cx.listener(Self::search_everything))
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
            .child(div().flex_1().min_h_0().flex().child(sidebar).child(center).children(preview))
            .child(path_bar)
            .children(go_to)
            .children(sheet)
            .children(activity)
            .children(why)
            .children(picker)
            .children(palette)
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

    #[gpui::test]
    fn first_row_sits_below_the_header_line(cx: &mut TestAppContext) {
        let root = fixture("list-inset");
        let (_view, mut cx) = open(cx, &root);
        let header = cx.debug_bounds("table-header").expect("header");
        let row = cx.debug_bounds("row-alpha").expect("first row");
        assert_eq!(row.top() - header.bottom(), px(crate::pane::LIST_TOP_INSET));
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
                cx.open_window(Default::default(), |window, cx| {
                    cx.new(|cx| CrowWindow::new(dir, false, None, window, cx))
                })
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

    /// Whether the menu item for `action` would be enabled with the current focus.
    fn action_available(action: &dyn gpui::Action, cx: &mut VisualTestContext) -> bool {
        let window = cx.update(|_, cx| cx.windows()[0]);
        cx.update_window(window, |_, window, cx| window.is_action_available(action, cx)).expect("check action")
    }

    /// The visible list rows as (name, indent depth).
    fn rows(view: &Entity<CrowWindow>, cx: &VisualTestContext) -> Vec<(String, usize)> {
        pane(view, cx).read_with(cx, |p, cx| p.rows(cx))
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
    fn dragging_a_favorite_down_inserts_below_the_row(cx: &mut TestAppContext) {
        let root = fixture("favorite-drag");
        let (view, mut cx) = open(cx, &root);
        let (alpha, beta) = (root.join("alpha"), root.join("beta"));
        write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&alpha, &beta]));
        let from = cx.debug_bounds("sidebar-0").expect("first favorite row");
        let to = cx.debug_bounds("sidebar-1").expect("second favorite row");
        let start = gpui::point(from.left() + gpui::px(20.), from.top() + gpui::px(6.));
        let over = gpui::point(to.left() + gpui::px(20.), to.bottom() - gpui::px(3.));

        cx.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_move(over, Some(gpui::MouseButton::Left), gpui::Modifiers::default());
        let sidebar = view.read_with(&cx, |w, _| w.sidebar.clone());
        assert_eq!(sidebar.read_with(&cx, |s, _| s.insert_slot()), Some((0, 2)), "one slot below beta");
        cx.simulate_mouse_up(over, gpui::MouseButton::Left, gpui::Modifiers::default());
        settle(&view, &mut cx);
        assert_eq!(sidebar_paths(&cx), [beta.clone(), alpha.clone()], "the lower half of a row drops after it");

        // And the upper half of a row is the single slot before it.
        let alpha_row = cx.debug_bounds("sidebar-1").expect("alpha row");
        let start = gpui::point(alpha_row.left() + gpui::px(20.), alpha_row.center().y);
        let beta_row = cx.debug_bounds("sidebar-0").expect("beta row");
        let top = gpui::point(beta_row.left() + gpui::px(20.), beta_row.top() + gpui::px(6.));
        cx.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_move(top, Some(gpui::MouseButton::Left), gpui::Modifiers::default());
        let sidebar = view.read_with(&cx, |w, _| w.sidebar.clone());
        assert_eq!(sidebar.read_with(&cx, |s, _| s.insert_slot()), Some((0, 0)), "one slot above alpha");
        cx.simulate_mouse_up(top, gpui::MouseButton::Left, gpui::Modifiers::default());
        settle(&view, &mut cx);
        assert_eq!(sidebar_paths(&cx), [alpha, beta], "…and dropping there moves it back");
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
    fn toolbar_view_buttons_switch_views(cx: &mut TestAppContext) {
        let root = fixture("toolbar-views");
        let (view, mut cx) = open(cx, &root);
        let mode = |cx: &VisualTestContext| pane(&view, cx).read_with(cx, |p, _| p.mode());
        assert_eq!(mode(&cx), ViewMode::List);
        let columns = cx.debug_bounds("view-columns").expect("columns button");
        cx.simulate_click(columns.center(), gpui::Modifiers::default());
        settle(&view, &mut cx);
        assert_eq!(mode(&cx), ViewMode::Columns);
        let list = cx.debug_bounds("view-list").expect("list button");
        cx.simulate_click(list.center(), gpui::Modifiers::default());
        settle(&view, &mut cx);
        assert_eq!(mode(&cx), ViewMode::List);
    }

    #[gpui::test]
    fn back_button_tooltip_names_the_folder(cx: &mut TestAppContext) {
        let root = fixture("back-tooltip");
        let executor = cx.executor();
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-down");
        settle(&view, &mut cx);
        let back = view.read_with(&cx, |w, cx| w.pane().read(cx).back_target().map(Path::to_path_buf));
        assert_eq!(back.as_deref(), Some(root.as_path()), "Back points at the folder we came from");

        let bounds = cx.debug_bounds("back").expect("the back button is laid out");
        cx.simulate_mouse_move(bounds.center(), None, gpui::Modifiers::default());
        executor.advance_clock(std::time::Duration::from_millis(600));
        cx.run_until_parked();
        cx.simulate_mouse_move(bounds.center(), None, gpui::Modifiers::default());
        assert!(cx.debug_bounds("tooltip").is_some(), "the tooltip is drawn after the 500 ms delay");
    }

    #[gpui::test]
    fn path_bar_segments_navigate_as_history_steps(cx: &mut TestAppContext) {
        let root = fixture("path-bar");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-down");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("alpha"));

        // The bar ends in the selected item, so its first segment is `/`'s own entry.
        let bounds = cx.debug_bounds("path-segment-0").expect("path bar segment is laid out");
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), PathBuf::from("/"), "clicking a segment opens that folder");
        cx.simulate_keystrokes("cmd-[");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("alpha"), "and it left a history step to go back to");
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
        assert_eq!(dir(&view, &cx), root, "l expands the folder first");
        cx.simulate_keystrokes("l");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("alpha"), "l again goes into it");
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

    fn wait_for_search(view: &Entity<CrowWindow>, cx: &mut VisualTestContext) {
        for _ in 0..400 {
            cx.run_until_parked();
            let busy =
                view.read_with(cx, |w, cx| w.search.as_ref().is_some_and(|search| search.read(cx).is_searching()));
            if !busy {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("search never finished");
    }

    /// Click the element with this debug selector, failing if it is not on screen.
    fn click_selector(cx: &mut VisualTestContext, selector: &'static str) {
        let bounds = cx.debug_bounds(selector).unwrap_or_else(|| panic!("{selector} is on screen"));
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    }

    #[gpui::test]
    fn search_panel_streams_names_and_contents(cx: &mut TestAppContext) {
        let root = fixture("search");
        std::fs::write(root.join("notes.txt"), "needle here\n").expect("create fixture");
        std::fs::write(root.join("alpha/needle.txt"), "nothing\n").expect("create fixture");
        std::fs::write(root.join("beta/other.txt"), "needle in beta\n").expect("create fixture");
        let (view, mut cx) = open(cx, &root);

        cx.simulate_keystrokes("cmd-f");
        settle(&view, &mut cx);
        let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
        assert_eq!(search.read_with(&cx, |search, _| search.scope()), crate::search::Scope::Folder);
        cx.simulate_input("needle");
        wait_for_search(&view, &mut cx);

        let paths = search.read_with(&cx, |search, _| search.result_paths());
        assert!(paths.contains(&root.join("notes.txt")), "{paths:?}");
        assert!(paths.contains(&root.join("alpha/needle.txt")), "{paths:?}");
        assert!(paths.contains(&root.join("beta/other.txt")), "contents are searched too: {paths:?}");
        assert_eq!(
            search.read_with(&cx, |search, _| search.result_content(&root.join("beta/other.txt"))),
            Some((1, "needle in beta".to_owned()))
        );
    }

    #[gpui::test]
    fn search_everything_turns_the_rules_off(cx: &mut TestAppContext) {
        let root = fixture("search-rules");
        std::fs::create_dir_all(root.join("node_modules/pkg")).expect("create fixture");
        std::fs::write(root.join("node_modules/pkg/needle.txt"), "needle\n").expect("create fixture");
        std::fs::write(root.join("outside.txt"), "needle\n").expect("create fixture");
        let (view, mut cx) = open(cx, &root);

        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("needle");
        wait_for_search(&view, &mut cx);
        let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
        let paths = search.read_with(&cx, |search, _| search.result_paths());
        assert!(paths.contains(&root.join("outside.txt")));
        assert!(!paths.iter().any(|path| path.ends_with("node_modules/pkg/needle.txt")), "{paths:?}");
        assert!(search.read_with(&cx, |search, _| search.skipped_count()) > 0);

        cx.simulate_keystrokes("cmd-shift-a");
        wait_for_search(&view, &mut cx);
        assert!(search.read_with(&cx, |search, _| search.everything()));
        let paths = search.read_with(&cx, |search, _| search.result_paths());
        assert!(paths.contains(&root.join("node_modules/pkg/needle.txt")), "{paths:?}");
        assert_eq!(search.read_with(&cx, |search, _| search.skipped_count()), 0);
    }

    #[gpui::test]
    fn search_field_click_opens_search_with_focus_in_the_field(cx: &mut TestAppContext) {
        let root = fixture("toolbar-search-click");
        std::fs::write(root.join("notes.txt"), "needle\n").expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        click_selector(&mut cx, "toolbar-search");
        settle(&view, &mut cx);
        assert!(view.read_with(&cx, |w, _| w.search.is_some()), "the field opens search");
        focus_search_input(&view, &mut cx);
        cx.simulate_input("needle");
        wait_for_search(&view, &mut cx);
        let paths = view.read_with(&cx, |w, cx| w.search.as_ref().expect("search").read(cx).result_paths());
        assert!(paths.contains(&root.join("notes.txt")), "{paths:?}");
    }

    #[gpui::test]
    fn cmd_f_focuses_the_field_when_search_is_open(cx: &mut TestAppContext) {
        let root = fixture("toolbar-cmd-f");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("notes");
        wait_for_search(&view, &mut cx);
        cx.simulate_keystrokes("tab");
        let focus = view.read_with(&cx, |w, cx| w.search.as_ref().expect("search").read(cx).input_focus_handle(cx));
        assert!(!cx.update(|window, _| focus.is_focused(window)), "tab moved focus to the results");
        cx.simulate_keystrokes("cmd-f");
        focus_search_input(&view, &mut cx);
        cx.simulate_input("x");
        let query = view.read_with(&cx, |w, cx| w.search.as_ref().expect("search").read(cx).query_text(cx));
        assert_eq!(query, "x", "the whole query was selected");
    }

    #[gpui::test]
    fn escape_in_the_field_closes_search_and_restores_pane_focus(cx: &mut TestAppContext) {
        let root = fixture("search-close");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("notes");
        cx.simulate_keystrokes("escape");
        assert!(view.read_with(&cx, |w, _| w.search.is_none()));
        cx.simulate_keystrokes("n");
        assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"), "type-to-select proves the pane has focus");
    }

    #[gpui::test]
    fn done_button_closes_search(cx: &mut TestAppContext) {
        let root = fixture("search-done");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("notes");
        wait_for_search(&view, &mut cx);
        click_selector(&mut cx, "search-done");
        settle(&view, &mut cx);
        assert!(view.read_with(&cx, |w, _| w.search.is_none()));
        cx.simulate_keystrokes("n");
        assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"));
    }

    #[gpui::test]
    fn vim_letters_type_into_the_band_field(cx: &mut TestAppContext) {
        let root = fixture("search-vim");
        let (view, mut cx) = open(cx, &root);
        let before = (dir(&view, &cx), selected(&view, &cx));
        use_vim(&view, &mut cx);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_keystrokes("j k / -");
        let query = view.read_with(&cx, |w, cx| w.search.as_ref().expect("search").read(cx).query_text(cx));
        assert_eq!(query, "jk/-");
        assert_eq!((dir(&view, &cx), selected(&view, &cx)), before, "no Vim motion reached the pane");
    }

    #[gpui::test]
    fn go_to_button_opens_the_palette(cx: &mut TestAppContext) {
        let root = fixture("toolbar-go-to");
        let (view, mut cx) = open(cx, &root);
        click_selector(&mut cx, "toolbar-go-to");
        settle(&view, &mut cx);
        let focus =
            view.read_with(&cx, |w, cx| w.palette.as_ref().expect("palette opens").input.read(cx).focus_handle(cx));
        assert!(cx.update(|window, _| focus.is_focused(window)), "focus is in the palette field");
        cx.simulate_keystrokes("escape");
        assert!(view.read_with(&cx, |w, _| w.palette.is_none()));
        cx.simulate_keystrokes("n");
        assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"), "the pane has focus again");
    }

    #[gpui::test]
    fn go_to_button_is_hidden_while_searching(cx: &mut TestAppContext) {
        let root = fixture("toolbar-search-band");
        let (_view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        assert!(cx.debug_bounds("toolbar-go-to").is_none());
        assert!(cx.debug_bounds("toolbar-search").is_none());
        assert!(cx.debug_bounds("search-field").is_some(), "the band's own field is there instead");
    }

    #[gpui::test]
    fn match_names_and_contents_change_the_results(cx: &mut TestAppContext) {
        let root = fixture("search-match");
        std::fs::write(root.join("alpha/needle.txt"), "nothing\n").expect("create fixture");
        std::fs::write(root.join("beta/other.txt"), "needle in beta\n").expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("needle");
        wait_for_search(&view, &mut cx);
        let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
        let has = |cx: &VisualTestContext, path: &Path| {
            search.read_with(cx, |search, _| search.result_paths().iter().any(|p| p == path))
        };
        let (name_hit, content_hit) = (root.join("alpha/needle.txt"), root.join("beta/other.txt"));
        assert!(has(&cx, &name_hit) && has(&cx, &content_hit), "All searches names and contents");

        click_selector(&mut cx, "search-match-names");
        wait_for_search(&view, &mut cx);
        assert!(has(&cx, &name_hit), "the name hit stays");
        assert!(!has(&cx, &content_hit), "no file is read, so the content hit is gone");
        focus_search_input(&view, &mut cx);

        click_selector(&mut cx, "search-match-contents");
        wait_for_search(&view, &mut cx);
        assert!(!has(&cx, &name_hit), "name-only hits are dropped");
        assert!(has(&cx, &content_hit));
        focus_search_input(&view, &mut cx);

        click_selector(&mut cx, "search-match-all");
        wait_for_search(&view, &mut cx);
        assert!(has(&cx, &name_hit) && has(&cx, &content_hit));
        focus_search_input(&view, &mut cx);
    }

    #[gpui::test]
    fn spotlight_scope_disables_the_match_control(cx: &mut TestAppContext) {
        let root = fixture("search-match-spotlight");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        settle(&view, &mut cx);
        let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
        click_selector(&mut cx, "search-match-contents");
        assert_eq!(search.read_with(&cx, |search, _| search.effective_match()), crate::search::Match::Contents);

        click_selector(&mut cx, "search-scope-Everywhere");
        settle(&view, &mut cx);
        assert_eq!(
            search.read_with(&cx, |search, _| search.effective_match()),
            crate::search::Match::Names,
            "Spotlight searches names only"
        );
        click_selector(&mut cx, "search-match-all");
        assert_eq!(
            search.read_with(&cx, |search, _| search.matching()),
            crate::search::Match::Contents,
            "the stored choice survives, and clicks on Spotlight change nothing"
        );
        click_selector(&mut cx, "search-scope-This Folder");
        settle(&view, &mut cx);
        assert_eq!(search.read_with(&cx, |search, _| search.effective_match()), crate::search::Match::Contents);
    }

    #[gpui::test]
    fn switching_tabs_closes_search(cx: &mut TestAppContext) {
        let root = fixture("search-tabs");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-t");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("notes");
        wait_for_search(&view, &mut cx);
        cx.simulate_keystrokes("cmd-shift-]");
        settle(&view, &mut cx);
        assert!(view.read_with(&cx, |w, _| w.search.is_none()), "switching tabs closes search");
        cx.simulate_keystrokes("n");
        assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"), "the new tab's pane has focus");
    }

    #[gpui::test]
    fn clicking_the_active_tab_keeps_search_open(cx: &mut TestAppContext) {
        let root = fixture("search-tab-click");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-t");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("notes");
        let bounds = cx.debug_bounds("tab-1").expect("active tab on screen");
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        settle(&view, &mut cx);
        assert!(view.read_with(&cx, |w, _| w.search.is_some()), "the active tab is not a switch");
        let bounds = cx.debug_bounds("tab-0").expect("other tab on screen");
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        settle(&view, &mut cx);
        assert!(view.read_with(&cx, |w, _| w.search.is_none()), "switching tabs closes search");
    }

    fn sidebar_focused(view: &Entity<CrowWindow>, cx: &mut VisualTestContext) -> bool {
        let focus = view.read_with(cx, |w, cx| w.sidebar.read(cx).focus_handle(cx));
        cx.update(|window, _| focus.is_focused(window))
    }

    fn search_input_focused(view: &Entity<CrowWindow>, cx: &mut VisualTestContext) -> bool {
        let focus = view.read_with(cx, |w, cx| w.search.as_ref().expect("search open").read(cx).input_focus_handle(cx));
        cx.update(|window, _| focus.is_focused(window))
    }

    #[gpui::test]
    fn tab_on_an_empty_search_goes_to_the_sidebar_and_esc_still_closes(cx: &mut TestAppContext) {
        let root = fixture("search-empty-tab");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_keystrokes("tab");
        assert!(sidebar_focused(&view, &mut cx), "no results to visit, so Tab goes to the sidebar");
        cx.simulate_keystrokes("tab");
        assert!(search_input_focused(&view, &mut cx), "Tab from the sidebar comes back to the field");
        cx.simulate_keystrokes("tab");
        cx.simulate_keystrokes("escape");
        assert!(view.read_with(&cx, |w, _| w.search.is_none()), "Esc in the sidebar closes search");
        cx.simulate_keystrokes("n");
        assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"), "the pane has focus");
    }

    #[gpui::test]
    fn tab_cycles_field_results_sidebar_while_searching(cx: &mut TestAppContext) {
        let root = fixture("search-tab-cycle");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("notes");
        wait_for_search(&view, &mut cx);
        cx.simulate_keystrokes("tab");
        assert!(!search_input_focused(&view, &mut cx) && !sidebar_focused(&view, &mut cx), "results");
        cx.simulate_keystrokes("tab");
        assert!(sidebar_focused(&view, &mut cx), "results, then the sidebar");
        cx.simulate_keystrokes("tab");
        assert!(search_input_focused(&view, &mut cx), "the hidden pane is skipped: back to the field");
        cx.simulate_keystrokes("tab tab");
        assert!(sidebar_focused(&view, &mut cx));
        cx.simulate_keystrokes("cmd-f");
        assert!(search_input_focused(&view, &mut cx), "Cmd-F from the sidebar refocuses the field");
        cx.simulate_keystrokes("escape");
        assert!(view.read_with(&cx, |w, _| w.search.is_none()));
    }

    #[gpui::test]
    fn closing_an_overlay_over_search_returns_to_the_field(cx: &mut TestAppContext) {
        let root = fixture("search-overlays");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("notes");
        wait_for_search(&view, &mut cx);
        cx.simulate_keystrokes("cmd-shift-g");
        assert!(view.read_with(&cx, |w, _| w.go_to.is_some()), "Go to Folder opens over search");
        cx.simulate_keystrokes("escape");
        assert!(view.read_with(&cx, |w, _| w.go_to.is_none() && w.search.is_some()));
        assert!(search_input_focused(&view, &mut cx), "focus goes back to the field, not the hidden pane");
        cx.simulate_keystrokes("escape");
        assert!(view.read_with(&cx, |w, _| w.search.is_none()), "Esc in the field still closes search");
    }

    #[gpui::test]
    fn results_emptied_under_focus_fall_back_to_the_field(cx: &mut TestAppContext) {
        let root = fixture("search-results-vanish");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("notes");
        wait_for_search(&view, &mut cx);
        cx.simulate_keystrokes("tab");
        // Rerunning clears the list, so the focused results element leaves the tree.
        click_selector(&mut cx, "search-match-contents");
        settle(&view, &mut cx);
        wait_for_search(&view, &mut cx);
        assert!(search_input_focused(&view, &mut cx), "focus never dies with the list");
        cx.simulate_keystrokes("escape");
        assert!(view.read_with(&cx, |w, _| w.search.is_none()));
    }

    fn focus_search_input(view: &Entity<CrowWindow>, cx: &mut VisualTestContext) {
        let focus = view.read_with(cx, |w, cx| w.search.as_ref().expect("search open").read(cx).input_focus_handle(cx));
        assert!(cx.update(|window, _| focus.is_focused(window)), "focus is in the search field");
    }

    #[gpui::test]
    fn find_everything_from_the_list_opens_search_with_rules_off(cx: &mut TestAppContext) {
        let root = fixture("find-everything");
        std::fs::create_dir_all(root.join("node_modules/pkg")).expect("create fixture");
        std::fs::write(root.join("node_modules/pkg/needle.txt"), "needle\n").expect("create fixture");
        let (view, mut cx) = open(cx, &root);

        cx.simulate_keystrokes("cmd-shift-a");
        cx.run_until_parked();
        let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
        assert!(search.read_with(&cx, |search, _| search.everything()), "every rule is off");
        assert_eq!(search.read_with(&cx, |search, _| search.scope()), crate::search::Scope::Folder);
        focus_search_input(&view, &mut cx);

        cx.simulate_input("needle");
        wait_for_search(&view, &mut cx);
        let paths = search.read_with(&cx, |search, _| search.result_paths());
        assert!(paths.contains(&root.join("node_modules/pkg/needle.txt")), "{paths:?}");
        assert_eq!(search.read_with(&cx, |search, _| search.skipped_count()), 0);
    }

    #[gpui::test]
    fn find_everything_does_nothing_over_the_palette(cx: &mut TestAppContext) {
        let root = fixture("find-everything-palette");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-k");
        cx.run_until_parked();
        cx.simulate_keystrokes("cmd-shift-a");
        cx.run_until_parked();
        assert!(view.read_with(&cx, |w, _| w.search.is_none()), "search stays closed");
        assert!(view.read_with(&cx, |w, _| w.palette.is_some()), "the palette stays open");
    }

    #[gpui::test]
    fn find_everything_from_the_sidebar_with_search_open(cx: &mut TestAppContext) {
        let root = fixture("find-everything-sidebar");
        std::fs::write(root.join("notes.txt"), "needle\n").expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("needle");
        wait_for_search(&view, &mut cx);
        let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
        assert!(!search.read_with(&cx, |search, _| search.everything()));

        let sidebar = view.read_with(&cx, |w, _| w.sidebar.clone());
        let focus = sidebar.read_with(&cx, |sidebar, cx| sidebar.focus_handle(cx));
        cx.update(|window, cx| window.focus(&focus, cx));
        cx.simulate_keystrokes("cmd-shift-a");
        wait_for_search(&view, &mut cx);
        assert!(search.read_with(&cx, |search, _| search.everything()), "the open search flips its rules");
        focus_search_input(&view, &mut cx);
    }

    #[gpui::test]
    fn open_search_with_a_query_runs_it(cx: &mut TestAppContext) {
        let root = fixture("open-search-query");
        std::fs::write(root.join("notes.txt"), "needle\n").expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        let window = cx.update(|_, cx| cx.windows()[0]);
        cx.update_window(window, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.open_search(
                    SearchStart { folder: root.clone(), query: "needle".to_owned(), everything: false },
                    window,
                    cx,
                );
            });
        })
        .expect("open search with a query");
        wait_for_search(&view, &mut cx);

        let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
        assert_eq!(search.read_with(&cx, |search, cx| search.query_text(cx)), "needle");
        assert!(!search.read_with(&cx, |search, _| search.everything()));
        let paths = search.read_with(&cx, |search, _| search.result_paths());
        assert!(paths.contains(&root.join("notes.txt")), "{paths:?}");
    }

    fn menu_action(menu: &str, title: &str, cx: &VisualTestContext) -> Box<dyn gpui::Action> {
        cx.read(|cx| {
            let menus = cx.get_menus().expect("menus are installed");
            menus
                .iter()
                .find(|candidate| candidate.name.as_ref() == menu)
                .and_then(|menu| {
                    menu.items.iter().find_map(|item| match item {
                        gpui::OwnedMenuItem::Action { name, action, .. } if name == title => Some(action.boxed_clone()),
                        _ => None,
                    })
                })
                .unwrap_or_else(|| panic!("{menu} > {title} exists"))
        })
    }

    fn dispatch_menu(menu: &str, title: &str, cx: &mut VisualTestContext) {
        let action = menu_action(menu, title, cx);
        let window = cx.update(|_, cx| cx.windows()[0]);
        cx.update_window(window, |_, window, cx| window.dispatch_action(action, cx)).expect("dispatch menu item");
        cx.run_until_parked();
    }

    #[gpui::test]
    fn menu_keys_are_the_finder_keys(cx: &mut TestAppContext) {
        let root = fixture("menu-keys");
        let (_view, cx) = open(cx, &root);
        let checks = [
            (crate::palette::CommandId::ShowSearch, "cmd-f"),
            (crate::palette::CommandId::SearchEverything, "cmd-shift-a"),
            (crate::palette::CommandId::OpenParent, "cmd-up"),
            (crate::palette::CommandId::Undo, "cmd-z"),
            (crate::palette::CommandId::Copy, "cmd-c"),
            (crate::palette::CommandId::ShowCommandPalette, "cmd-k"),
            (crate::palette::CommandId::GoToFolder, "cmd-shift-g"),
            (crate::palette::CommandId::ToggleActivity, "alt-cmd-l"),
        ];
        for (id, expected) in checks {
            let actual = cx.read(|cx| {
                let command = crate::palette::COMMANDS.iter().find(|command| command.id == id).expect("command");
                let keymap = cx.key_bindings();
                let keymap = keymap.borrow();
                keymap.bindings_for_action(command.action).next().map(|binding| {
                    binding
                        .keystrokes()
                        .iter()
                        .map(|keystroke| keystroke.unparse(gpui::OperatingSystem::Mac))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
            });
            assert_eq!(actual.as_deref(), Some(expected), "{id:?} shows its Finder key");
        }
    }

    #[gpui::test]
    fn menu_items_are_enabled_by_focus(cx: &mut TestAppContext) {
        let root = fixture("menu-focus");
        std::fs::write(root.join("notes.txt"), "needle\n").expect("create fixture");
        let (view, mut cx) = open(cx, &root);

        assert!(action_available(&crate::actions::StartFilter, &mut cx), "list: Filter");
        assert!(action_available(&crate::actions::Rename, &mut cx), "list: Rename");
        assert!(action_available(&crate::actions::MoveToTrash, &mut cx), "list: Move to Trash");
        assert!(action_available(&crate::actions::GoBack, &mut cx), "any: Back");

        cx.simulate_keystrokes("tab");
        assert!(!action_available(&crate::actions::StartFilter, &mut cx), "sidebar: no Filter");
        assert!(!action_available(&crate::actions::Rename, &mut cx), "sidebar: no Rename");
        assert!(!action_available(&crate::actions::CopyFiles, &mut cx), "sidebar: no Copy");
        assert!(!action_available(&crate::actions::PasteFiles, &mut cx), "sidebar: no Paste");
        assert!(!action_available(&crate::actions::NewFolder, &mut cx), "sidebar: no New Folder");
        assert!(!action_available(&crate::actions::QuickLook, &mut cx), "sidebar: no Quick Look");
        assert!(action_available(&crate::actions::GoBack, &mut cx), "sidebar: Back stays");
        assert!(action_available(&crate::actions::ShowSearch, &mut cx), "sidebar: Find stays");
        assert!(action_available(&crate::actions::SearchEverything, &mut cx), "sidebar: Find Everything stays");
        assert!(action_available(&crate::actions::Undo, &mut cx), "sidebar: Undo stays");

        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("needle");
        wait_for_search(&view, &mut cx);
        cx.simulate_keystrokes("tab");
        assert!(action_available(&crate::actions::ShowSearch, &mut cx), "results: Find stays available");
        assert!(action_available(&crate::actions::SearchEverything, &mut cx), "results: Find Everything stays");
        assert!(action_available(&crate::actions::CopyFiles, &mut cx), "results: Copy");
        assert!(action_available(&crate::actions::SelectAll, &mut cx), "results: Select All");
        assert!(action_available(&crate::actions::MoveToTrash, &mut cx), "results: Move to Trash");
        assert!(action_available(&crate::actions::QuickLook, &mut cx), "results: Quick Look");
        assert!(!action_available(&crate::actions::PasteFiles, &mut cx), "results: no Paste");
        assert!(!action_available(&crate::actions::StartFilter, &mut cx), "results: no Filter");
    }

    #[gpui::test]
    fn menu_items_dispatch(cx: &mut TestAppContext) {
        let root = fixture("menu-dispatch");
        let (view, mut cx) = open(cx, &root);
        cx.update(|_, cx| crate::menus::init_for_tests(cx));

        cx.simulate_keystrokes("cmd-down");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("alpha"));
        dispatch_menu("Go", "Enclosing Folder", &mut cx);
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root);
        dispatch_menu("Go", "Back", &mut cx);
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("alpha"));

        dispatch_menu("Edit", "Filter This Folder", &mut cx);
        assert!(pane(&view, &cx).read_with(&cx, |p, _| p.is_filtering()), "Filter opens");
        cx.simulate_keystrokes("escape");

        dispatch_menu("Edit", "Find…", &mut cx);
        assert!(view.read_with(&cx, |w, _| w.search.is_some()), "Find opens search");
        cx.simulate_keystrokes("escape");
        dispatch_menu("Go", "Go to…", &mut cx);
        assert!(view.read_with(&cx, |w, _| w.palette.is_some()), "Go to opens the palette");
        cx.simulate_keystrokes("escape");

        let activity = view.read_with(&cx, |w, _| w.show_activity);
        dispatch_menu("View", "Show Activity", &mut cx);
        assert_ne!(view.read_with(&cx, |w, _| w.show_activity), activity, "Show Activity flips");

        dispatch_menu("File", "New Tab", &mut cx);
        assert_eq!(view.read_with(&cx, |w, _| w.tabs.len()), 2, "New Tab opens a tab");
    }

    #[gpui::test]
    fn window_menu_tab_items_switch_tabs(cx: &mut TestAppContext) {
        let root = fixture("menu-tabs");
        let (view, mut cx) = open(cx, &root);
        cx.update(|_, cx| crate::menus::init_for_tests(cx));
        cx.simulate_keystrokes("cmd-t");
        settle(&view, &mut cx);
        assert_eq!(view.read_with(&cx, |w, _| w.tabs.len()), 2);

        dispatch_menu("Window", "Show Previous Tab", &mut cx);
        assert_eq!(view.read_with(&cx, |w, _| w.tabs.active_index()), 0);
        dispatch_menu("Window", "Show Next Tab", &mut cx);
        assert_eq!(view.read_with(&cx, |w, _| w.tabs.active_index()), 1);
    }

    #[gpui::test]
    fn help_opens_palette_in_command_mode(cx: &mut TestAppContext) {
        let root = fixture("menu-help");
        let (view, mut cx) = open(cx, &root);
        let window = cx.update(|_, cx| cx.windows()[0]);
        cx.update_window(window, |_, window, cx| {
            window.dispatch_action(Box::new(crate::actions::KeyboardShortcuts), cx)
        })
        .expect("dispatch Help item");
        cx.run_until_parked();
        let text =
            view.read_with(&cx, |w, cx| w.palette.as_ref().expect("palette opens").input.read(cx).text().to_owned());
        assert_eq!(text, ">", "the palette starts in command mode");
    }

    #[gpui::test]
    fn search_results_copy_and_trash_the_selection(cx: &mut TestAppContext) {
        let root = fixture("search-actions");
        std::fs::write(root.join("notes.txt"), "needle\n").expect("create fixture");
        std::fs::write(root.join("alpha/needle.txt"), "needle\n").expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("needle");
        wait_for_search(&view, &mut cx);
        cx.simulate_keystrokes("tab cmd-a cmd-c");

        let copied = cx.read(|cx| cx.global::<Operations>().clipboard.as_ref().map(|clip| clip.paths.clone()));
        let copied = copied.expect("copy filled the clipboard");
        assert!(copied.contains(&root.join("notes.txt")), "{copied:?}");
        assert!(copied.contains(&root.join("alpha/needle.txt")), "{copied:?}");

        cx.simulate_keystrokes("cmd-backspace");
        settle(&view, &mut cx);
        assert!(!root.join("notes.txt").exists(), "trash acted on the results");
        assert!(!root.join("alpha/needle.txt").exists());
        assert!(view.read_with(&cx, |w, _| w.search.is_none()), "an operation closes the search");
    }

    #[gpui::test]
    fn search_result_return_navigates_and_closes(cx: &mut TestAppContext) {
        let root = fixture("search-open");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("alpha");
        wait_for_search(&view, &mut cx);

        cx.simulate_keystrokes("tab");
        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("alpha"));
        assert!(view.read_with(&cx, |w, _| w.search.is_none()));
    }

    #[gpui::test]
    fn search_result_cmd_r_reveals_the_file(cx: &mut TestAppContext) {
        let root = fixture("search-reveal");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("notes");
        wait_for_search(&view, &mut cx);

        cx.simulate_keystrokes("tab");
        cx.simulate_keystrokes("cmd-r");
        settle(&view, &mut cx);
        assert!(view.read_with(&cx, |w, _| w.search.is_none()));
        assert_eq!(dir(&view, &cx), root);
        assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"));
    }

    #[gpui::test]
    fn command_palette_runs_a_command(cx: &mut TestAppContext) {
        let root = fixture("palette-command");
        std::fs::write(root.join(".secret"), "").expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        let count = |cx: &VisualTestContext| pane(&view, cx).read_with(cx, |p, cx| p.item_count(cx));
        assert_eq!(count(&cx), 3);

        cx.simulate_keystrokes("cmd-k");
        settle(&view, &mut cx);
        assert!(view.read_with(&cx, |w, _| w.palette.is_some()));
        cx.simulate_input(">toggle hidden");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);

        assert!(view.read_with(&cx, |w, _| w.palette.is_none()), "Return closes the palette");
        assert_eq!(count(&cx), 4, "the command ran");
    }

    #[gpui::test]
    fn command_palette_goes_to_a_recent_folder(cx: &mut TestAppContext) {
        let root = fixture("palette-go");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-down");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("alpha"));
        cx.simulate_keystrokes("cmd-up");
        settle(&view, &mut cx);
        assert!(
            cx.read(|cx| cx.global::<Ui>().palette_recents.contains(&root.join("alpha"))),
            "the visit is in the palette's recent list"
        );

        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input("alpha");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("alpha"));
        assert!(view.read_with(&cx, |w, _| w.palette.is_none()));
    }

    #[gpui::test]
    fn command_palette_cmd_return_opens_a_new_tab(cx: &mut TestAppContext) {
        let root = fixture("palette-tab");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input("beta");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("cmd-enter");
        settle(&view, &mut cx);

        assert_eq!(view.read_with(&cx, |w, _| w.tabs.len()), 2);
        assert_eq!(dir(&view, &cx), root.join("beta"));
        assert!(view.read_with(&cx, |w, _| w.palette.is_none()));
    }

    #[gpui::test]
    fn command_palette_keys_follow_the_layer(cx: &mut TestAppContext) {
        let root = fixture("palette-keys");
        let (view, mut cx) = open(cx, &root);
        let copy_keys = |cx: &VisualTestContext| {
            view.read_with(cx, |w, cx| {
                let palette = w.palette.as_ref().expect("palette is open");
                let i = palette
                    .items
                    .iter()
                    .position(
                        |item| matches!(item, Item::Command(command) if command.id == crate::palette::CommandId::Copy),
                    )
                    .expect("the Copy row is listed");
                palette.row_keys(i, cx).map(|keys| keys.to_string())
            })
        };
        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input(">copy");
        settle(&view, &mut cx);
        assert_eq!(copy_keys(&cx).as_deref(), Some("⌘C"));
        cx.simulate_keystrokes("escape");

        use_vim(&view, &mut cx);
        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input(">copy");
        settle(&view, &mut cx);
        assert_eq!(copy_keys(&cx).as_deref(), Some("yy"), "the palette follows keymap = vim");
    }

    #[gpui::test]
    fn command_palette_vim_keys_move_the_cursor(cx: &mut TestAppContext) {
        let root = fixture("palette-vim");
        let (view, mut cx) = open(cx, &root);
        use_vim(&view, &mut cx);
        cx.simulate_keystrokes("cmd-k");
        settle(&view, &mut cx);
        let cursor = |cx: &VisualTestContext| view.read_with(cx, |w, _| w.palette.as_ref().map(|p| p.cursor));
        assert_eq!(cursor(&cx), Some(0));
        cx.simulate_keystrokes("j");
        assert_eq!(cursor(&cx), Some(1), "j moves down");
        cx.simulate_keystrokes("k");
        assert_eq!(cursor(&cx), Some(0), "k moves up");
        cx.simulate_keystrokes("escape");
        assert!(view.read_with(&cx, |w, _| w.palette.is_none()));
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
    fn filter_narrows_rows_and_escape_restores_focus(cx: &mut TestAppContext) {
        let root = fixture("filter");
        let (view, mut cx) = open(cx, &root);

        cx.simulate_keystrokes("/");
        cx.simulate_input("AL");
        settle(&view, &mut cx);
        assert_eq!(rows(&view, &cx), [("alpha".to_owned(), 0)], "matching is case-insensitive");
        assert_eq!(pane(&view, &cx).read_with(&cx, |p, cx| p.item_count(cx)), 3, "the total stays visible");

        cx.simulate_keystrokes("escape");
        settle(&view, &mut cx);
        assert_eq!(rows(&view, &cx).len(), 3, "Escape clears the filter");
        cx.simulate_keystrokes("n");
        assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"), "focus is back on the list for type-to-select");
    }

    #[gpui::test]
    fn filter_exposes_match_ranges_per_row(cx: &mut TestAppContext) {
        let root = fixture("filter-ranges");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("/");
        cx.simulate_input("PH");
        settle(&view, &mut cx);
        assert_eq!(rows(&view, &cx), [("alpha".to_owned(), 0)]);
        assert_eq!(pane(&view, &cx).read_with(&cx, |p, cx| p.row_matches(cx)), [Some(2..4)]);
    }

    #[gpui::test]
    fn filter_arrows_and_return_act_on_matching_rows(cx: &mut TestAppContext) {
        let root = fixture("filter-enter");
        let (view, mut cx) = open(cx, &root);

        cx.simulate_keystrokes("/");
        cx.simulate_input("a");
        settle(&view, &mut cx);
        assert_eq!(rows(&view, &cx).iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["alpha", "beta"]);

        cx.simulate_keystrokes("down");
        assert_eq!(selected(&view, &cx).as_deref(), Some("beta"), "arrows move within the matches");
        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("beta"), "Return opens the selected match");
        assert!(rows(&view, &cx).is_empty(), "the filter does not follow the navigation");

        cx.simulate_keystrokes("cmd-up");
        settle(&view, &mut cx);
        assert_eq!(rows(&view, &cx).len(), 3, "going back shows the whole folder again");
    }

    #[gpui::test]
    fn filter_commands_and_escape_work_with_the_list_focused(cx: &mut TestAppContext) {
        let root = fixture("filter-list-focus");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("/");
        cx.simulate_input("a");
        settle(&view, &mut cx);

        let focus = pane(&view, &cx).read_with(&cx, |p, _| p.focus_handle().clone());
        cx.update(|window, cx| window.focus(&focus, cx));
        cx.simulate_keystrokes("cmd-a");
        assert_eq!(selected_names(&view, &cx), ["alpha", "beta"], "Cmd-A selects matching rows only");

        cx.simulate_keystrokes("escape");
        settle(&view, &mut cx);
        assert_eq!(rows(&view, &cx).len(), 3, "Escape on the list clears the filter too");
    }

    #[gpui::test]
    fn filter_ignores_inline_children_until_cleared(cx: &mut TestAppContext) {
        let root = fixture("filter-inline");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("right");
        settle(&view, &mut cx);
        assert!(rows(&view, &cx).contains(&("inner".to_owned(), 1)), "alpha is open inline");

        cx.simulate_keystrokes("/");
        cx.simulate_input("inner");
        settle(&view, &mut cx);
        assert!(rows(&view, &cx).is_empty(), "a filter shows this folder's own matches, flat");

        cx.simulate_keystrokes("escape");
        settle(&view, &mut cx);
        assert!(rows(&view, &cx).contains(&("inner".to_owned(), 1)), "clearing restores the open folder");
    }

    #[gpui::test]
    fn filter_letters_stay_in_the_field_in_the_vim_layer(cx: &mut TestAppContext) {
        let root = fixture("filter-vim");
        let (view, mut cx) = open(cx, &root);
        use_vim(&view, &mut cx);

        cx.simulate_keystrokes("/");
        cx.simulate_input("al");
        settle(&view, &mut cx);
        assert_eq!(rows(&view, &cx), [("alpha".to_owned(), 0)], "l filters instead of moving right");

        cx.simulate_keystrokes("escape");
        cx.simulate_keystrokes("j");
        assert_eq!(selected(&view, &cx).as_deref(), Some("beta"), "Vim keys work again after the filter");
    }

    #[gpui::test]
    fn filter_narrows_the_focused_column(cx: &mut TestAppContext) {
        let root = fixture("filter-columns");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-3");
        settle(&view, &mut cx);

        cx.simulate_keystrokes("/");
        cx.simulate_input("al");
        settle(&view, &mut cx);
        assert_eq!(rows(&view, &cx), [("alpha".to_owned(), 0)]);

        cx.simulate_keystrokes("escape");
        settle(&view, &mut cx);
        assert_eq!(rows(&view, &cx).len(), 3);
    }

    #[gpui::test]
    fn filter_field_opens_in_the_focused_column(cx: &mut TestAppContext) {
        let root = fixture("filter-field-column");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-3 right");
        settle(&view, &mut cx);
        let before0 = cx.debug_bounds("column-0").expect("first column");
        let before1 = cx.debug_bounds("column-1").expect("second column");
        let count_before = pane(&view, &cx).read_with(&cx, |p, _| p.folder_count());
        assert_eq!(pane(&view, &cx).read_with(&cx, |p, _| p.focused_index()), 1, "moved into the second column");

        cx.simulate_keystrokes("/");
        settle(&view, &mut cx);
        let field = cx.debug_bounds("filter-field").expect("the field is in the column");
        let after0 = cx.debug_bounds("column-0").expect("first column");
        let after1 = cx.debug_bounds("column-1").expect("second column");
        assert!(field.left() >= after1.left() && field.right() <= after1.right(), "{field:?} inside {after1:?}");
        assert_eq!(after0, before0, "the column to the left did not move");
        assert_eq!(after0.size.width, before0.size.width, "widths do not change while filtering");
        assert_eq!(after1, before1, "the filtered column does not move or grow");
        assert_eq!(pane(&view, &cx).read_with(&cx, |p, _| p.folder_count()), count_before, "no column opened");
        assert_eq!(pane(&view, &cx).read_with(&cx, |p, _| p.focused_index()), 1);
    }

    #[gpui::test]
    fn filter_field_opens_above_the_list_rows(cx: &mut TestAppContext) {
        let root = fixture("filter-field-list");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("/");
        settle(&view, &mut cx);
        let header = cx.debug_bounds("table-header").expect("header");
        let field = cx.debug_bounds("filter-field").expect("field");
        let row = cx.debug_bounds("row-alpha").expect("first row");
        assert!(field.top() >= header.bottom(), "{field:?} is below the {header:?}");
        assert!(field.bottom() <= row.top(), "{field:?} is above the {row:?}");
    }

    #[gpui::test]
    fn filter_vim_letters_stay_in_the_column_field(cx: &mut TestAppContext) {
        let root = fixture("filter-vim-column-field");
        let (view, mut cx) = open(cx, &root);
        use_vim(&view, &mut cx);
        cx.simulate_keystrokes("cmd-3 right");
        settle(&view, &mut cx);
        let before = (dir(&view, &cx), pane(&view, &cx).read_with(&cx, |p, _| p.focused_index()));
        cx.simulate_keystrokes("/");
        cx.simulate_keystrokes("h l -");
        assert_eq!(pane(&view, &cx).read_with(&cx, |p, cx| p.filter_query(cx)).as_deref(), Some("hl-"));
        assert_eq!((dir(&view, &cx), pane(&view, &cx).read_with(&cx, |p, _| p.focused_index())), before);
        assert!(rows(&view, &cx).is_empty(), "no Vim motion or filter match ran");
    }

    #[gpui::test]
    fn filter_field_takes_space_slash_and_list_keys_as_text(cx: &mut TestAppContext) {
        let root = fixture("filter-field-keys");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("/");
        cx.simulate_keystrokes("a space /");
        assert_eq!(pane(&view, &cx).read_with(&cx, |p, cx| p.filter_query(cx)).as_deref(), Some("a /"));
        cx.simulate_keystrokes("cmd-backspace");
        assert!(root.join("notes.txt").exists(), "Cmd-Backspace did not trash a row from the field");
    }

    #[gpui::test]
    fn filter_closes_when_the_focused_column_changes(cx: &mut TestAppContext) {
        let root = fixture("filter-column-change");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-3 right");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("/");
        cx.simulate_input("x");
        settle(&view, &mut cx);
        assert!(pane(&view, &cx).read_with(&cx, |p, _| p.is_filtering()));

        // The field owns Left while it has focus; click the column's list first.
        let focus = pane(&view, &cx).read_with(&cx, |p, _| p.focus_handle().clone());
        cx.update(|window, cx| window.focus(&focus, cx));
        cx.simulate_keystrokes("left");
        settle(&view, &mut cx);
        assert!(!pane(&view, &cx).read_with(&cx, |p, _| p.is_filtering()));
        assert!(cx.debug_bounds("filter-field").is_none(), "the field closed");
        assert_eq!(rows(&view, &cx).len(), 3, "the left column shows all of its rows");
    }

    #[gpui::test]
    fn filter_follows_live_changes(cx: &mut TestAppContext) {
        let root = fixture("filter-live");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("/");
        cx.simulate_input("al");
        settle(&view, &mut cx);
        std::fs::write(root.join("alpine.txt"), "").expect("create fixture");
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            // The folder's watcher polls every 100 ms on the test clock.
            cx.executor().advance_clock(std::time::Duration::from_millis(150));
            cx.run_until_parked();
            if rows(&view, &cx).iter().any(|(name, _)| name == "alpine.txt") {
                break;
            }
        }
        assert_eq!(rows(&view, &cx).iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["alpha", "alpine.txt"]);
        let matches = pane(&view, &cx).read_with(&cx, |p, cx| p.row_matches(cx));
        assert_eq!(matches, [Some(0..2), Some(0..2)], "both rows carry the live match range");
    }

    #[gpui::test]
    fn cmd_f_while_filtering_searches_the_focused_folder_with_the_query(cx: &mut TestAppContext) {
        let root = fixture("filter-to-search");
        std::fs::write(root.join("alpha/needle.txt"), "nothing\n").expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-3 right");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("/");
        cx.simulate_input("nee");
        settle(&view, &mut cx);
        assert!(pane(&view, &cx).read_with(&cx, |p, _| p.is_filtering()));

        cx.simulate_keystrokes("cmd-f");
        wait_for_search(&view, &mut cx);
        let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
        assert_eq!(search.read_with(&cx, |search, cx| search.query_text(cx)), "nee");
        assert_eq!(search.read_with(&cx, |search, _| search.folder().to_path_buf()), root.join("alpha"));
        assert_eq!(search.read_with(&cx, |search, _| search.scope()), crate::search::Scope::Folder);
        let paths = search.read_with(&cx, |search, _| search.result_paths());
        assert!(paths.contains(&root.join("alpha/needle.txt")), "{paths:?}");
        assert!(!pane(&view, &cx).read_with(&cx, |p, _| p.is_filtering()), "the filter was handed over, not left open");

        cx.simulate_keystrokes("escape");
        settle(&view, &mut cx);
        assert_eq!(rows(&view, &cx).len(), 2, "the focused column shows all of its rows again");
        cx.simulate_keystrokes("n");
        assert_eq!(selected(&view, &cx).as_deref(), Some("needle.txt"), "type-to-select proves the list has focus");
    }

    #[gpui::test]
    fn cmd_f_from_the_list_while_filtering_also_escalates(cx: &mut TestAppContext) {
        let root = fixture("filter-list-to-search");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("/");
        cx.simulate_input("a");
        settle(&view, &mut cx);
        let focus = pane(&view, &cx).read_with(&cx, |p, _| p.focus_handle().clone());
        cx.update(|window, cx| window.focus(&focus, cx));

        cx.simulate_keystrokes("cmd-f");
        wait_for_search(&view, &mut cx);
        let query = view.read_with(&cx, |w, cx| w.search.as_ref().expect("search").read(cx).query_text(cx));
        assert_eq!(query, "a");
        assert!(!pane(&view, &cx).read_with(&cx, |p, _| p.is_filtering()));
    }

    #[gpui::test]
    fn filter_footer_click_opens_search(cx: &mut TestAppContext) {
        let root = fixture("filter-footer-click");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("/");
        cx.simulate_input("al");
        settle(&view, &mut cx);
        click_selector(&mut cx, "filter-search-footer");
        wait_for_search(&view, &mut cx);
        let query = view.read_with(&cx, |w, cx| w.search.as_ref().expect("search").read(cx).query_text(cx));
        assert_eq!(query, "al");
    }

    #[gpui::test]
    fn filter_with_no_matches_offers_search(cx: &mut TestAppContext) {
        let root = fixture("filter-no-matches");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("/");
        cx.simulate_input("zzz");
        settle(&view, &mut cx);
        assert!(cx.debug_bounds("filter-no-matches").is_some(), "D replaces the list");
        assert!(cx.debug_bounds("filter-search-footer").is_none(), "there is no footer in D");

        click_selector(&mut cx, "filter-search-subfolders");
        wait_for_search(&view, &mut cx);
        let query = view.read_with(&cx, |w, cx| w.search.as_ref().expect("search").read(cx).query_text(cx));
        assert_eq!(query, "zzz");
        assert!(!pane(&view, &cx).read_with(&cx, |p, _| p.is_filtering()));
    }

    #[gpui::test]
    fn filter_no_match_note_follows_hidden_files(cx: &mut TestAppContext) {
        let root = fixture("filter-no-matches-hidden");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("/");
        cx.simulate_input("zzz");
        settle(&view, &mut cx);
        assert!(cx.debug_bounds("filter-hidden-note").is_some(), "hidden files are off");
        cx.simulate_keystrokes("cmd-shift-.");
        settle(&view, &mut cx);
        assert!(cx.debug_bounds("filter-hidden-note").is_some(), "the way out is still named");
        assert!(cx.debug_bounds("filter-no-matches").is_some());
    }

    #[gpui::test]
    fn filter_footer_hides_for_an_empty_query(cx: &mut TestAppContext) {
        let root = fixture("filter-footer-empty");
        let (_view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("/");
        settle(&_view, &mut cx);
        assert!(cx.debug_bounds("filter-field").is_some(), "the field is open");
        assert!(cx.debug_bounds("filter-search-footer").is_none(), "nothing to carry deeper");
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
    fn exclude_config_reloads_live_and_keeps_the_last_good_rules(cx: &mut TestAppContext) {
        let root = fixture("exclude-reload");
        let (view, mut cx) = open(cx, &root);
        let names = |cx: &VisualTestContext| cx.read(|cx| cx.global::<Ui>().exclude.set(crow_core::SetName::Dev).names);
        assert!(names(&cx).contains(&"node_modules".to_owned()));

        write_config(&view, &mut cx, "exclude.toml", "[dev]\nnames = [\"only_this\"]\n");
        assert_eq!(names(&cx), vec!["only_this".to_owned()]);

        write_config(&view, &mut cx, "exclude.toml", "this is not toml");
        assert_eq!(names(&cx), vec!["only_this".to_owned()], "a broken file keeps the last good rules");
        let error = cx.read(|cx| cx.global::<Ui>().config_error().map(|e| e.to_string()).unwrap_or_default());
        assert!(error.contains("exclude.toml"), "{error}");
    }

    #[gpui::test]
    fn clicking_the_scrim_closes_every_overlay(cx: &mut TestAppContext) {
        theme_index();
        let root = fixture("scrim");
        let (view, mut cx) = open(cx, &root);
        let start_theme = cx.read(|cx| cx.global::<Ui>().theme.name.clone());
        let pane_focus = view.read_with(&cx, |w, cx| w.pane().read(cx).focus_handle().clone());
        let click_scrim = |cx: &mut VisualTestContext| {
            let bounds = cx.debug_bounds("sheet-scrim").expect("the scrim is laid out");
            cx.simulate_click(gpui::point(bounds.origin.x + px(10.), bounds.bottom() - px(10.)), Default::default());
            settle(&view, cx);
        };
        let focused = |cx: &mut VisualTestContext| cx.update(|window, _| pane_focus.is_focused(window));

        cx.simulate_keystrokes("cmd-shift-g");
        settle(&view, &mut cx);
        assert!(view.read_with(&cx, |w, _| w.go_to.is_some()), "Go to Folder is open");
        click_scrim(&mut cx);
        assert!(view.read_with(&cx, |w, _| w.go_to.is_none()), "a scrim click closes it like Escape");
        assert!(focused(&mut cx), "focus goes back to the pane");
        assert!(root.join("notes.txt").exists());

        cx.simulate_keystrokes("down down cmd-alt-backspace");
        settle(&view, &mut cx);
        assert!(sheet_open(&view, &cx), "the delete sheet is open");
        click_scrim(&mut cx);
        assert!(!sheet_open(&view, &cx), "a scrim click cancels it");
        assert!(root.join("notes.txt").exists(), "cancel runs nothing");
        assert!(focused(&mut cx));

        cx.simulate_keystrokes("cmd-j");
        settle(&view, &mut cx);
        assert!(view.read_with(&cx, |w, _| w.why.is_some()), "Why This View? is open");
        click_scrim(&mut cx);
        assert!(view.read_with(&cx, |w, _| w.why.is_none()));
        assert!(focused(&mut cx));

        cx.simulate_keystrokes("cmd-alt-t");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("down");
        assert!(view.read_with(&cx, |w, _| w.picker.is_some()), "the theme picker is open");
        assert_ne!(cx.read(|cx| cx.global::<Ui>().theme.name.clone()), start_theme, "a theme is previewed");
        click_scrim(&mut cx);
        assert!(view.read_with(&cx, |w, _| w.picker.is_none()));
        assert_eq!(cx.read(|cx| cx.global::<Ui>().theme.name.clone()), start_theme, "the preview is reverted");
        assert!(focused(&mut cx));

        cx.simulate_keystrokes("cmd-k");
        settle(&view, &mut cx);
        assert!(view.read_with(&cx, |w, _| w.palette.is_some()), "the palette is open");
        click_scrim(&mut cx);
        assert!(view.read_with(&cx, |w, _| w.palette.is_none()));
        assert!(focused(&mut cx));
    }

    #[gpui::test]
    fn clicking_inside_a_sheet_keeps_it_open(cx: &mut TestAppContext) {
        let root = fixture("sheet-inside");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-j");
        settle(&view, &mut cx);
        let panel = cx.debug_bounds("sheet-panel").expect("the panel is laid out");
        cx.simulate_click(panel.center(), Default::default());
        settle(&view, &mut cx);
        assert!(view.read_with(&cx, |w, _| w.why.is_some()), "a click inside does not dismiss");

        cx.simulate_keystrokes("escape");
        cx.simulate_keystrokes("down down cmd-alt-backspace");
        settle(&view, &mut cx);
        assert!(sheet_open(&view, &cx));
        let cancel = cx.debug_bounds("sheet-cancel").expect("the Cancel button is laid out");
        cx.simulate_click(cancel.center(), Default::default());
        settle(&view, &mut cx);
        assert!(!sheet_open(&view, &cx), "a button inside still runs its own click");
        assert!(root.join("notes.txt").exists());
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
    fn column_view_left_on_the_leftmost_column_goes_up(cx: &mut TestAppContext) {
        let root = fixture("columns-left");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-3");
        settle(&view, &mut cx);
        assert_eq!(columns(&view, &cx, 0).2, Some(root.clone()));

        let parent = root.parent().expect("the fixture has a parent").to_path_buf();
        cx.simulate_keystrokes("left");
        settle(&view, &mut cx);
        let (_, focused, first, cursor) = columns(&view, &cx, 0);
        assert_eq!(focused, 0);
        assert_eq!(first, Some(parent.clone()), "the parent is pushed in from the left");
        assert_eq!(
            cursor.as_deref(),
            Some(root.file_name().unwrap().to_str().unwrap()),
            "…with the old folder selected"
        );
        assert_eq!(pane(&view, &cx).read_with(&cx, |p, _| p.mode()), ViewMode::Columns, "and column view stays on");
        assert_eq!(dir(&view, &cx), parent);

        cx.simulate_keystrokes("cmd-[");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root, "Back returns to where the columns started");

        // At the root of the disk there is nowhere to go up to.
        pane(&view, &cx).update(&mut cx, |p, cx| p.navigate(PathBuf::from("/"), cx));
        settle(&view, &mut cx);
        cx.simulate_keystrokes("cmd-3");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("left");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), PathBuf::from("/"), "Left at / does nothing");
    }

    #[gpui::test]
    fn list_view_expands_folders_inline(cx: &mut TestAppContext) {
        let root = fixture("expand");
        std::fs::write(root.join("alpha/a.txt"), "").expect("create fixture");
        std::fs::write(root.join("alpha/b.txt"), "").expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        assert_eq!(rows(&view, &cx), [("alpha".to_owned(), 0), ("beta".to_owned(), 0), ("notes.txt".to_owned(), 0)]);

        cx.simulate_keystrokes("right");
        settle(&view, &mut cx);
        assert_eq!(
            rows(&view, &cx),
            [
                ("alpha".to_owned(), 0),
                ("inner".to_owned(), 1),
                ("a.txt".to_owned(), 1),
                ("b.txt".to_owned(), 1),
                ("beta".to_owned(), 0),
                ("notes.txt".to_owned(), 0),
            ],
            "Right opens the folder inline, indented under it"
        );
        assert_eq!(dir(&view, &cx), root, "…without going anywhere");
        assert_eq!(selected(&view, &cx).as_deref(), Some("alpha"), "the folder stays selected");

        cx.simulate_keystrokes("down");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("right");
        settle(&view, &mut cx);
        assert_eq!(selected(&view, &cx).as_deref(), Some("inner"), "Right expands a nested folder");
        cx.simulate_keystrokes("right");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("alpha/inner"), "Right again goes into the open folder");

        cx.simulate_keystrokes("cmd-[");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root);
        assert!(!rows(&view, &cx).iter().any(|(_, depth)| *depth > 0), "coming back starts collapsed");
    }

    #[gpui::test]
    fn list_view_left_steps_to_the_parent_row_then_collapses(cx: &mut TestAppContext) {
        let root = fixture("left-parent");
        std::fs::write(root.join("alpha/a.txt"), "").expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("right");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("down");
        settle(&view, &mut cx);
        assert_eq!(selected(&view, &cx).as_deref(), Some("inner"), "a nested row is selected");

        cx.simulate_keystrokes("left");
        settle(&view, &mut cx);
        assert_eq!(selected(&view, &cx).as_deref(), Some("alpha"), "Left selects the row that contains it");
        assert!(rows(&view, &cx).iter().any(|(name, _)| name == "inner"), "…and leaves it open");

        cx.simulate_keystrokes("left");
        settle(&view, &mut cx);
        assert!(!rows(&view, &cx).iter().any(|(name, _)| name == "inner"), "Left again collapses it");

        cx.simulate_keystrokes("down down");
        settle(&view, &mut cx);
        assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"));
        cx.simulate_keystrokes("left");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.parent().unwrap(), "Left on a top-level row goes up a folder");
        assert_eq!(selected(&view, &cx).as_deref(), Some(root.file_name().unwrap().to_str().unwrap()));
    }

    #[gpui::test]
    fn selection_and_trash_span_expanded_levels(cx: &mut TestAppContext) {
        let root = fixture("expand-select");
        let (a, b) = (root.join("alpha/a.txt"), root.join("alpha/b.txt"));
        std::fs::write(&a, "").expect("create fixture");
        std::fs::write(&b, "").expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("right");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("down down");
        settle(&view, &mut cx);
        assert_eq!(selected(&view, &cx).as_deref(), Some("a.txt"));
        cx.simulate_keystrokes("shift-down");
        let selected_paths = pane(&view, &cx).read_with(&cx, |p, cx| p.selected_paths(cx));
        assert_eq!(selected_paths, [a.clone(), b.clone()], "a range over rows of the expanded folder");

        cx.simulate_keystrokes("cmd-backspace");
        settle(&view, &mut cx);
        assert!(!a.exists() && !b.exists(), "both nested files went to the trash");
        assert!(root.join("alpha").is_dir(), "their folder stays");
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
    fn palette_find_opens_search(cx: &mut TestAppContext) {
        let root = fixture("palette-find");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-k");
        cx.run_until_parked();
        cx.simulate_input(">find");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();

        assert!(view.read_with(&cx, |w, _| w.palette.is_none()), "the palette closes");
        let focus =
            view.read_with(&cx, |w, cx| w.search.as_ref().expect("Find… opens search").read(cx).input_focus_handle(cx));
        assert!(cx.update(|window, _| focus.is_focused(window)), "focus is in the search field");
    }

    #[gpui::test]
    fn plain_keys_stay_in_every_text_field(cx: &mut TestAppContext) {
        use crate::actions::{CopyFiles, CutFiles, MoveToTrash, PasteFiles, QuickLook, Rename, SelectAll, StartFilter};

        let root = fixture("plain-keys");
        let (view, mut cx) = open(cx, &root);
        // simulate_input skips AppKit's menu equivalents, so the real guard is that no list
        // action is on the focus path: a disabled menu item lets its plain key through.
        fn list_actions_off(field: &str, cx: &mut VisualTestContext) {
            for (action, name) in [
                (&StartFilter as &dyn gpui::Action, "Filter This Folder"),
                (&QuickLook, "Quick Look"),
                (&Rename, "Rename"),
                (&MoveToTrash, "Move to Trash"),
            ] {
                assert!(!action_available(action, cx), "{field}: {name} is off the path");
            }
        }

        // Rename: the list's actions must be off the focus path so AppKit passes plain keys through.
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(pane(&view, &cx).read_with(&cx, |p, _| p.is_renaming()), "Return starts rename");
        cx.simulate_input("a/b c");
        let (name, filtering) = pane(&view, &cx).read_with(&cx, |p, cx| (p.rename_text(cx), p.is_filtering()));
        assert_eq!(name.as_deref(), Some("a/b c"), "slash and space land in the name");
        assert!(!filtering, "slash did not start the filter");
        assert!(view.read_with(&cx, |w, _| w.search.is_none()), "space did not open Quick Look");
        list_actions_off("rename", &mut cx);
        assert!(action_available(&CopyFiles, &mut cx), "the field handles Copy");
        assert!(action_available(&CutFiles, &mut cx), "the field handles Cut");
        assert!(action_available(&PasteFiles, &mut cx), "the field handles Paste");
        assert!(action_available(&SelectAll, &mut cx), "the field handles Select All");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(!pane(&view, &cx).read_with(&cx, |p, _| p.is_renaming()), "Return ends the rename");
        settle(&view, &mut cx);

        // Filter field.
        cx.simulate_keystrokes("/");
        cx.run_until_parked();
        cx.simulate_input("a/b c");
        let text = pane(&view, &cx).read_with(&cx, |p, cx| p.filter_text(cx));
        assert_eq!(text.as_deref(), Some("a/b c"), "slash and space land in the filter");
        list_actions_off("filter", &mut cx);
        cx.simulate_keystrokes("escape");

        // Go to Folder field.
        cx.simulate_keystrokes("cmd-shift-g");
        cx.run_until_parked();
        cx.simulate_input("/ ");
        let text = view
            .read_with(&cx, |w, cx| w.go_to.as_ref().map(|(input, _)| input.read(cx).text().to_owned()))
            .expect("Go to Folder stays open");
        assert!(text.ends_with("/ "), "slash and space land in Go to Folder, got {text:?}");
        list_actions_off("Go to Folder", &mut cx);
        cx.simulate_keystrokes("escape");

        // Search field.
        cx.simulate_keystrokes("cmd-f");
        cx.run_until_parked();
        cx.simulate_input("/ ");
        let text = view.read_with(&cx, |w, cx| w.search.as_ref().expect("search open").read(cx).query_text(cx));
        assert_eq!(text, "/ ", "slash and space land in the search field");
        assert!(!pane(&view, &cx).read_with(&cx, |p, _| p.is_filtering()), "search did not start the filter");
        list_actions_off("search", &mut cx);
        cx.simulate_keystrokes("escape");

        // Palette field.
        cx.simulate_keystrokes("cmd-k");
        cx.run_until_parked();
        cx.simulate_input("/ ");
        let text =
            view.read_with(&cx, |w, cx| w.palette.as_ref().expect("palette open").input.read(cx).text().to_owned());
        assert_eq!(text, "/ ", "slash and space land in the palette");
        list_actions_off("palette", &mut cx);
        cx.simulate_keystrokes("escape");
    }

    #[gpui::test]
    fn cmd_z_in_a_text_field_undoes_typing_not_files(cx: &mut TestAppContext) {
        let root = fixture("field-undo");
        let (view, mut cx) = open(cx, &root);
        // A file operation to (wrongly) undo: trash notes.txt.
        cx.simulate_keystrokes("down down cmd-backspace");
        settle(&view, &mut cx);
        assert!(!root.join("notes.txt").exists());

        // Rename beta: "beta" is selected; typing replaces it in one step, then a second run.
        cx.simulate_keystrokes("home down");
        assert_eq!(selected(&view, &cx).as_deref(), Some("beta"));
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        cx.simulate_input("gamma");
        cx.simulate_keystrokes("backspace backspace");
        let name = |cx: &VisualTestContext| pane(&view, cx).read_with(cx, |p, cx| p.rename_text(cx));
        assert_eq!(name(&cx).as_deref(), Some("gam"));

        cx.simulate_keystrokes("cmd-z");
        assert_eq!(name(&cx).as_deref(), Some("gamma"), "undo takes back the deletions as one step");
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(name(&cx).as_deref(), Some("beta"), "undo takes back the typing as one step");
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(name(&cx).as_deref(), Some("beta"), "nothing left to undo in the field");
        cx.simulate_keystrokes("cmd-shift-z");
        assert_eq!(name(&cx).as_deref(), Some("gamma"), "redo brings the typing back");
        settle(&view, &mut cx);
        assert!(!root.join("notes.txt").exists(), "the trash was never undone");
        cx.simulate_keystrokes("escape");

        // The same in the filter field, with the menu's Undo action.
        cx.simulate_keystrokes("/");
        cx.run_until_parked();
        cx.simulate_input("al");
        cx.dispatch_action(crate::actions::Undo);
        let text = pane(&view, &cx).read_with(&cx, |p, cx| p.filter_text(cx));
        assert_eq!(text.as_deref(), Some(""), "Edit ▸ Undo undoes the filter text");
        settle(&view, &mut cx);
        assert!(!root.join("notes.txt").exists(), "the trash was never undone");
        cx.simulate_keystrokes("escape");

        // Back in the list, ⌘Z undoes the file operation again.
        cx.simulate_keystrokes("cmd-z");
        settle(&view, &mut cx);
        assert!(root.join("notes.txt").exists(), "list ⌘Z undoes the trash");
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
    fn platform_drag_updates_keep_one_list_target(cx: &mut TestAppContext) {
        let root = fixture("drop-steady-list");
        let (view, mut cx) = open(cx, &root);
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let alpha = cx.debug_bounds("row-alpha").expect("alpha row").center();
        let notes = cx.debug_bounds("row-notes.txt").expect("notes row").center();
        let pane = view.read_with(&cx, |window, _| window.tabs.active().clone());
        let target = |cx: &VisualTestContext| pane.read_with(cx, |pane, _| pane.drop_row());

        cx.simulate_event(gpui::FileDropEvent::Entered {
            position: alpha,
            paths: gpui::ExternalPaths(vec![root.join("dropped.txt")].into()),
        });
        cx.simulate_event(gpui::FileDropEvent::Pending { position: alpha });
        cx.run_until_parked();
        assert_eq!(target(&cx), Some(0), "the row under the pointer is the target");
        for _ in 0..8 {
            cx.simulate_event(gpui::FileDropEvent::Pending { position: alpha });
            cx.run_until_parked();
            assert_eq!(target(&cx), Some(0), "platform updates keep one steady target");
        }
        cx.simulate_event(gpui::FileDropEvent::Pending { position: notes });
        cx.run_until_parked();
        assert_eq!(target(&cx), Some(2), "moving the pointer moves the target");
        cx.simulate_event(gpui::FileDropEvent::Exited);
        cx.run_until_parked();
        assert_eq!(target(&cx), None, "leaving the window clears it");
    }

    #[gpui::test]
    fn platform_drag_updates_keep_one_column_target(cx: &mut TestAppContext) {
        let root = fixture("drop-steady-columns");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-3");
        settle(&view, &mut cx);
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let pane = view.read_with(&cx, |window, _| window.tabs.active().clone());
        assert_eq!(pane.read_with(&cx, |pane, _| pane.mode()), ViewMode::Columns, "column view is active");
        let at = gpui::point(px(320.), px(90.));
        cx.simulate_event(gpui::FileDropEvent::Entered {
            position: at,
            paths: gpui::ExternalPaths(vec![root.join("dropped.txt")].into()),
        });
        cx.simulate_event(gpui::FileDropEvent::Pending { position: at });
        cx.run_until_parked();
        assert_eq!(pane.read_with(&cx, |pane, _| pane.drop_column()), Some((0, 1)));
        for _ in 0..8 {
            cx.simulate_event(gpui::FileDropEvent::Pending { position: at });
            cx.run_until_parked();
            assert_eq!(
                pane.read_with(&cx, |pane, _| pane.drop_column()),
                Some((0, 1)),
                "platform updates keep one steady target in a column"
            );
        }
        cx.simulate_event(gpui::FileDropEvent::Exited);
        cx.run_until_parked();
        assert_eq!(pane.read_with(&cx, |pane, _| pane.drop_column()), None);
    }

    #[gpui::test]
    fn sidebar_drop_highlight_is_steady_while_the_pointer_stays_on_a_row(cx: &mut TestAppContext) {
        let root = fixture("drop-steady");
        let (view, mut cx) = open(cx, &root);
        write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("alpha"), &root.join("beta")]));
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let first = cx.debug_bounds("sidebar-0").expect("first favorite row").center();
        let second = cx.debug_bounds("sidebar-1").expect("second favorite row").center();
        let header = cx.debug_bounds("sidebar-header-Favorites").expect("favorites title").center();
        let sidebar = view.read_with(&cx, |w, _| w.sidebar.clone());
        let target = |cx: &VisualTestContext| sidebar.read_with(cx, |s, _| s.drop_target());

        let paths = gpui::ExternalPaths(vec![root.join("notes.txt")].into());
        cx.simulate_event(gpui::FileDropEvent::Entered { position: second, paths });
        cx.simulate_event(gpui::FileDropEvent::Pending { position: second });
        cx.run_until_parked();
        assert_eq!(target(&cx), Some((true, 1)), "the row under the pointer is highlighted");
        for _ in 0..8 {
            cx.simulate_event(gpui::FileDropEvent::Pending { position: second });
            cx.run_until_parked();
            assert_eq!(target(&cx), Some((true, 1)), "…and stays highlighted while the pointer rests on it");
        }
        cx.simulate_event(gpui::FileDropEvent::Pending { position: first });
        cx.run_until_parked();
        assert_eq!(target(&cx), Some((true, 0)), "moving to another row moves the highlight");
        cx.simulate_event(gpui::FileDropEvent::Pending { position: header });
        cx.run_until_parked();
        assert_eq!(target(&cx), Some((false, 0)), "a group title highlights as a drop target");
        cx.simulate_event(gpui::FileDropEvent::Exited);
        cx.run_until_parked();
        assert_eq!(target(&cx), None, "leaving the window clears it");
    }

    #[gpui::test]
    fn dragging_a_row_onto_a_sidebar_favorite_moves_it(cx: &mut TestAppContext) {
        let root = fixture("drag-sidebar");
        let (view, mut cx) = open(cx, &root);
        write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("beta")]));
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let from = cx.debug_bounds("row-notes.txt").expect("notes.txt row").center();
        let to = cx.debug_bounds("sidebar-0").expect("beta favorite row").center();
        cx.simulate_mouse_down(from, gpui::MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_move(to, Some(gpui::MouseButton::Left), gpui::Modifiers::default());
        cx.simulate_mouse_up(to, gpui::MouseButton::Left, gpui::Modifiers::default());
        settle(&view, &mut cx);
        assert!(root.join("beta/notes.txt").exists(), "the dragged file moved into the favorite's folder");
    }

    #[gpui::test]
    fn dragging_a_row_onto_a_folder_moves_it(cx: &mut TestAppContext) {
        let root = fixture("drag-row");
        let (view, mut cx) = open(cx, &root);
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let from = cx.debug_bounds("row-notes.txt").expect("notes.txt row").center();
        let to = cx.debug_bounds("row-alpha").expect("alpha row").center();
        cx.simulate_mouse_down(from, gpui::MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_move(to, Some(gpui::MouseButton::Left), gpui::Modifiers::default());
        cx.simulate_mouse_up(to, gpui::MouseButton::Left, gpui::Modifiers::default());
        settle(&view, &mut cx);
        assert!(root.join("alpha/notes.txt").exists(), "the dragged file moved into the folder under the pointer");
        assert!(!root.join("notes.txt").exists(), "…and left its old place");
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
