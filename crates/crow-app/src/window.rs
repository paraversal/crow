//! One Crow window: title bar, tabs, sidebar, the active pane, preview and path bar.

use std::path::{Path, PathBuf};

use crow_core::motion::Fx;
use crow_core::{HintsMode, SidebarEdit, Tabs, TipId, ViewMode, ZeroMatchStreak};
use gpui::{
    AnyWindowHandle, App, Bounds, Context, CursorStyle, Entity, FocusHandle, Focusable, FontWeight, IntoElement,
    Render, Role, ScrollHandle, SharedString, Subscription, TitlebarOptions, Toggled, Window, WindowBounds,
    WindowOptions, div, img, point, prelude::*, px, size, transparent_black,
};

use crate::actions::{
    AddToSidebar, ApplyThemeEverywhere, CloseTab, GoBack, GoForward, GoToFolder, KeyboardShortcuts, Minimize, MoveDown,
    MoveToTrash, MoveUp, NewTab, NewWindow, NextTab, OpenParent, PaletteOpenInTab, PreviousTab, ResetFolderView,
    SearchEverything, ShowCommandPalette, ShowSearch, ShowThemePicker, SwitchFocus, ToggleHiddenFiles, TogglePreview,
    UseViewAsDefaults, ViewAsColumns, ViewAsList, WhyThisView, Zoom,
};
use crate::bottom_bar::{
    self, BarError, BarFocus, BarState, Count, Path as BarPath, Primary, RunningView, search_count, sidebar_count,
};
use crate::chrome::{self, Keycap, KeycapWeight, SheetWidth, with_action_tooltip, with_tooltip};
use crate::icons::{self, Glyph};
use crate::keys::{self, KeyScope};
use crate::operations::{self, Clipboard, Operations};
use crate::palette::{self, Item, Row, Source};
use crate::pane::Pane;
use crate::pane::PaneEvent;
use crate::preview;
use crate::search::{SearchEvent, SearchView};
use crate::sidebar::{Sidebar, SidebarEvent};
use crate::style::{active_selection, color, color_alpha, highlight_text, icon, truncate_middle_with_ranges, ui_font};
use crate::text_input::{TextInput, TextInputEvent};
use crate::ui::{self, Ui, reload_theme};
use crow_fs::Entry;
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
    /// The contextual tip on screen, if any.
    tip: Option<TipId>,
    /// Consecutive filter sessions that ended with no matches (U9's ⌘F tip).
    zero_streak: ZeroMatchStreak,
    /// The folder a sidebar arrow walk is showing. Walking records no Recent visits
    /// (they would re-rank Recent under the cursor); leaving the sidebar records this one.
    walk_visit: Option<PathBuf>,
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

/// Which half of the palette is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PaletteMode {
    Folders,
    Commands,
}

impl PaletteMode {
    fn placeholder(self) -> &'static str {
        match self {
            PaletteMode::Folders => "Go to a folder, or > for commands",
            PaletteMode::Commands => "Run a command",
        }
    }

    /// The field's spoken name; the placeholder says where `>` leads.
    fn aria_label(self) -> &'static str {
        match self {
            PaletteMode::Folders => "Go to folder",
            PaletteMode::Commands => "Search commands",
        }
    }
}

/// Cmd-K: folders (favorites, Recent, subfolders) or, after `>`, commands.
struct Palette {
    input: Entity<TextInput>,
    mode: PaletteMode,
    listing: palette::Listing,
    /// The focused folder's name, for the "In <name>" header.
    here: String,
    /// Path mode's parent listing, while the query is a path.
    path: Option<PathState>,
    cursor: usize,
    folders: Vec<palette::FolderCandidate>,
    scroll: ScrollHandle,
    _subscription: Subscription,
}

#[cfg(test)]
impl Palette {
    /// The shortcut shown on row `i`, from the live keymap; the layer test reads it.
    fn row_keys(&self, i: usize, cx: &App) -> Option<SharedString> {
        match self.listing.items.get(i)? {
            Item::Command { command, .. } => keys::text(command.action, command.scope, cx),
            Item::Folder { .. } => None,
        }
    }
}

/// Palette layout, in points. U10 tunes these against the assembled boards.
const PALETTE_TOP: f32 = 52.;
const PALETTE_WIDTH: f32 = 620.;
const PALETTE_MARGIN: f32 = 16.;
const PALETTE_BOTTOM: f32 = 24.;
const PALETTE_INPUT: f32 = 50.;
const PALETTE_FOOTER: f32 = 34.;
const PALETTE_DIVIDER: f32 = 1.;
const PALETTE_LIST_PAD: f32 = 6.;
const PALETTE_FOLDER_ROW: f32 = 44.;
const PALETTE_COMMAND_ROW: f32 = 40.;
const PALETTE_HEADER_ROW: f32 = 34.;
const PALETTE_EMPTY_ROW: f32 = 44.;
const PALETTE_MAX_LIST: f32 = 440.;
/// The sheet's width: Wide, narrowed to keep the margins in a small window.
fn palette_width(window: &Window) -> f32 {
    (f32::from(window.viewport_size().width) - 2. * PALETTE_MARGIN).min(PALETTE_WIDTH)
}

/// The path line's width inside a folder row: the sheet minus list padding, row
/// padding, the icon, its gap, the age column and one more gap.
fn palette_path_width(window: &Window) -> f32 {
    palette_width(window) - 2. * PALETTE_LIST_PAD - 20. - 22. - 12. - 70. - 12.
}

fn palette_item_height(item: &Item) -> f32 {
    match item {
        Item::Command { .. } => PALETTE_COMMAND_ROW,
        Item::Folder { .. } => PALETTE_FOLDER_ROW,
    }
}

/// Path mode's listing of one parent, cached while the query stays under it.
struct PathState {
    parent: PathBuf,
    partial: String,
    /// `None` while the background listing runs.
    children: Option<Vec<PathBuf>>,
    /// The parent is not a folder, or cannot be read.
    missing: bool,
    /// Whether the listing includes hidden entries.
    hidden: bool,
    /// Dropped listings (an older hidden setting) carry an older number.
    generation: u64,
}

/// The list's content height, padding included.
fn palette_content_height(listing: &palette::Listing) -> f32 {
    if listing.rows.is_empty() {
        return PALETTE_EMPTY_ROW;
    }
    let rows: f32 = listing
        .rows
        .iter()
        .map(|row| match row {
            Row::Header(_) => PALETTE_HEADER_ROW,
            Row::Status(_) => PALETTE_EMPTY_ROW,
            Row::Item(i) => palette_item_height(&listing.items[*i]),
        })
        .sum();
    rows + 2. * PALETTE_LIST_PAD
}

fn palette_list_height(window: &Window, content: f32) -> f32 {
    let available = f32::from(window.viewport_size().height)
        - PALETTE_TOP
        - PALETTE_INPUT
        - PALETTE_FOOTER
        - 2. * PALETTE_DIVIDER
        - PALETTE_BOTTOM;
    content.min(available.clamp(PALETTE_FOLDER_ROW, PALETTE_MAX_LIST))
}

/// Where the cursor's row starts in the list, and how tall it is.
fn palette_row_y(listing: &palette::Listing, cursor: usize) -> (f32, f32) {
    let mut y = PALETTE_LIST_PAD;
    for row in &listing.rows {
        match row {
            Row::Header(_) => y += PALETTE_HEADER_ROW,
            Row::Status(_) => y += PALETTE_EMPTY_ROW,
            Row::Item(i) => {
                let height = palette_item_height(&listing.items[*i]);
                if *i == cursor {
                    return (y, height);
                }
                y += height;
            }
        }
    }
    (y, PALETTE_FOLDER_ROW)
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
        let start_mode = Some(cx.global::<Ui>().views.default_view());
        let pane = cx.new(|cx| Pane::new(dir.clone(), show_hidden, start_mode, bench, bench_filter, cx));
        let sidebar = cx.new(|cx| {
            let mut sidebar = Sidebar::new(cx);
            sidebar.set_current(dir, cx);
            sidebar
        });
        let focus = pane.read(cx).focus_handle().clone();
        window.focus(&focus, cx);
        // The red close button: the last window asks before cutting an operation off.
        let this = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            this.update(cx, |this, cx| this.may_close(window, cx)).unwrap_or(true)
        });
        let owner = cx.entity_id();
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
            // A closed window must not keep the app-wide tip on screen.
            cx.on_release(move |_, cx| crate::tips::closed(owner, cx)),
        ];
        Self {
            tabs: Tabs::new(pane),
            sidebar,
            bench,
            show_hidden,
            sheet: None,
            show_activity: false,
            why: None,
            picker: None,
            palette: None,
            search: None,
            tip: None,
            zero_streak: ZeroMatchStreak::new(),
            walk_visit: None,
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
        self.close_tip(cx);
        let show_hidden = self.show_hidden;
        // A new tab keeps the window's view when views don't follow folders.
        let start_mode = Some(self.pane().read(cx).mode());
        let pane = cx.new(|cx| Pane::new(dir, show_hidden, start_mode, false, None, cx));
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
        self.close_tip(cx);
        if self.tabs.close_active().is_none() {
            // The last tab closes the window, which may have to ask first.
            if self.may_close(window, cx) {
                window.remove_window();
            }
            return;
        }
        self.focus_center(window, cx);
        cx.notify();
    }

    /// Whether this window may close now. The last window takes Crow with it, so while
    /// an operation runs it asks first (and says no for now).
    fn may_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let last = cx.windows().len() <= 1;
        if !last || cx.global::<Operations>().running.is_none() {
            return true;
        }
        self.ask_to_stop(AfterStop::CloseWindow, window, cx);
        false
    }

    fn ask_to_stop(&mut self, then: AfterStop, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.sheet, Some(Sheet::StopOperation { .. })) {
            return;
        }
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.sheet = Some(Sheet::StopOperation { then, focus });
        window.activate_window();
        cx.notify();
    }

    fn cycle_tab(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.is_some() {
            self.close_search(window, cx);
        }
        self.close_tip(cx);
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
            self.record_walk_visit(cx);
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

    fn toggle_preview(&mut self, _: &TogglePreview, _: &mut Window, cx: &mut Context<Self>) {
        let show = !cx.global::<Ui>().settings.show_preview;
        crate::ui::set_show_preview(show, cx);
        cx.notify();
    }

    /// Shift-Cmd-G: the palette with this folder's path typed, ready for a child's
    /// name, Tab completion or a whole new path (Finder's Go to Folder, in Cmd-K).
    fn go_to_folder(&mut self, _: &GoToFolder, window: &mut Window, cx: &mut Context<Self>) {
        let current = home_relative(&self.pane().read(cx).current_dir(cx));
        let typed = if current.ends_with('/') { current } else { format!("{current}/") };
        self.open_palette(&typed, window, cx);
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
                        this.close_tip(cx);
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
        self.render_preview_pane(entry.as_ref(), preview, &theme)
    }

    /// The 300 px preview chrome, shared by the pane and search.
    fn render_preview_pane(
        &self,
        entry: Option<&Entry>,
        preview: Option<&crate::preview::Preview>,
        theme: &crow_theme::Theme,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id("preview")
            .debug_selector(|| "preview".into())
            .role(Role::Group)
            .aria_label(entry.map_or_else(|| "Preview".to_owned(), |e| preview::spoken(e, preview)))
            .w(px(300.))
            .flex_none()
            .h_full()
            .p_4()
            .border_l_1()
            .border_color(color(theme.role("border")))
            .overflow_hidden()
            .children(entry.map(|e| preview::contents(e, preview, theme)))
    }

    /// Which element owns the keys, for the bar's hints.
    fn bar_focus(&self, window: &Window, pane: &Pane, cx: &App) -> BarFocus {
        let overlay = self.sheet.is_some() || self.why.is_some() || self.picker.is_some() || self.palette.is_some();
        if overlay || pane.is_renaming() {
            return BarFocus::Overlay;
        }
        if let Some(search) = &self.search {
            return if search.read(cx).input_focused(window, cx) {
                BarFocus::SearchField
            } else {
                BarFocus::SearchResults
            };
        }
        if self.sidebar.read(cx).focus_handle(cx).is_focused(window) {
            return BarFocus::Sidebar;
        }
        if pane.is_filtering() {
            return match pane.filter_focus(cx) {
                Some(focus) if focus.is_focused(window) => BarFocus::FilterField,
                _ => BarFocus::ListFiltered,
            };
        }
        BarFocus::List
    }

    /// Everything the bottom bar shows this frame. Rendering reads only this.
    fn bar_state(&self, window: &Window, cx: &App) -> BarState {
        let ui = cx.global::<Ui>();
        let pane = self.pane().read(cx);
        let ops = cx.global::<Operations>();

        // Errors never lose to hints: the token shows beside whatever else is primary.
        let error = if let Some((text, file)) = ui.config_error_file() {
            Some(BarError { text, file: Some(file) })
        } else if let Some(text) = pane.error(cx) {
            Some(BarError { text, file: None })
        } else {
            ops.message
                .as_ref()
                .filter(|(_, is_error)| *is_error)
                .map(|(message, _)| BarError { text: message.clone(), file: None })
        };
        let focus = self.bar_focus(window, pane, cx);

        let sidebar_focused = focus == BarFocus::Sidebar;
        let count = if let Some(search) = &self.search {
            let search = search.read(cx);
            search_count(search.result_count(), search.selected_count(), search.is_searching(), !search.has_query(cx))
        } else if sidebar_focused {
            self.sidebar
                .read(cx)
                .cursor_group(cx)
                .map_or_else(|| bar_pane_count(pane, cx), |(group, rows)| sidebar_count(&group, rows))
        } else {
            bar_pane_count(pane, cx)
        };

        // Q8: search mode shows the selected result's path.
        let search_entry = self.search.as_ref().and_then(|search| search.read(cx).selected_entry());
        let (target, last_is_dir) = match search_entry {
            Some((path, is_dir)) => (path, is_dir),
            None => pane
                .cursor_entry(cx)
                .map_or_else(|| (pane.current_dir(cx), true), |entry| (entry.path.clone(), entry.is_dir)),
        };
        let parts = crow_macos::display_components(&target);
        let spoken = parts.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>().join(", ");
        let path = BarPath { parts, last_is_dir, spoken, hidden: None };

        let primary = if let Some(hint) = pane.pending_hint() {
            Primary::Pending(hint)
        } else if let Some(running) = &ops.running {
            Primary::Running(RunningView::new(running))
        } else if let Some((message, false)) = &ops.message {
            Primary::Message(message.clone())
        } else {
            let scope = match focus {
                BarFocus::List => {
                    if pane.mode() == ViewMode::Columns {
                        KeyScope::Columns
                    } else {
                        KeyScope::List
                    }
                }
                BarFocus::ListFiltered => {
                    if pane.mode() == ViewMode::Columns {
                        KeyScope::ColumnsFiltered
                    } else {
                        KeyScope::ListFiltered
                    }
                }
                BarFocus::FilterField => KeyScope::Filter,
                BarFocus::Sidebar => KeyScope::Sidebar,
                BarFocus::SearchResults => KeyScope::SearchResults,
                BarFocus::SearchField => KeyScope::Search,
                BarFocus::Overlay => KeyScope::Window,
            };
            let sidebar_kind = if sidebar_focused { self.sidebar.read(cx).cursor_kind(cx) } else { None };
            // Hints never show beside an error token; pending, progress and messages outrank them.
            let hints = if error.is_none() { bottom_bar::hints(focus, scope, sidebar_kind, cx) } else { Vec::new() };
            Primary::Hints(hints)
        };

        let mut state = BarState { primary, error, count, path, transient_max: None };
        // Narrow windows lose hints from the right and fold the path's middle (I7).
        bottom_bar::fit_to_window(&mut state, window);
        state
    }

    fn render_bottom_bar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let state = self.bar_state(window, cx);
        bottom_bar::render(state, self.pane().clone(), cx)
    }

    // ----- contextual tips -----

    /// Show `id` when nothing else owns the screen and its key is still unknown.
    /// Retirement, key uses, one-on-screen and the cooldown live in `tips::request`.
    fn offer_tip(&mut self, id: TipId, _window: &mut Window, cx: &mut Context<Self>) {
        if self.tip_blocked(cx) || (id == TipId::Filter && self.pane().read(cx).is_filtering()) {
            return;
        }
        if crate::tips::key_text(id, cx).is_none() {
            return;
        }
        if crate::tips::request(id, cx.entity_id(), cx) {
            self.tip = Some(id);
            cx.notify();
        }
    }

    fn close_tip(&mut self, cx: &mut Context<Self>) {
        if self.tip.take().is_some() {
            crate::tips::closed(cx.entity_id(), cx);
            cx.notify();
        }
    }

    /// States a tip must not appear over, and that close one already up.
    fn tip_blocked(&self, cx: &App) -> bool {
        self.sheet.is_some()
            || self.why.is_some()
            || self.picker.is_some()
            || self.palette.is_some()
            || self.search.is_some()
            || self.show_activity
            || cx.has_active_drag()
            || self.pane().read(cx).is_renaming()
    }
}

fn bar_pane_count(pane: &Pane, cx: &App) -> Count {
    let (bytes, _folders) = pane.selected_bytes(cx);
    bottom_bar::pane_count(bottom_bar::PaneCounts {
        loading: pane.is_loading(cx),
        total: pane.item_count(cx),
        selected: pane.selected_count(cx),
        bytes,
        folder: crate::style::display_name(&pane.current_dir(cx)),
    })
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

/// A question to answer before an operation runs, or before one is cut off.
enum Sheet {
    Conflicts {
        plan: Plan,
        choices: Vec<Resolution>,
        focus: FocusHandle,
    },
    ConfirmDelete {
        paths: Vec<PathBuf>,
        focus: FocusHandle,
    },
    /// Closing the last window or quitting would stop the running operation partway.
    StopOperation {
        then: AfterStop,
        focus: FocusHandle,
    },
}

/// What happens once the running operation has stopped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AfterStop {
    CloseWindow,
    Quit,
}

impl Sheet {
    fn focus(&self) -> &FocusHandle {
        match self {
            Sheet::Conflicts { focus, .. }
            | Sheet::ConfirmDelete { focus, .. }
            | Sheet::StopOperation { focus, .. } => focus,
        }
    }
}

/// Quit, unless an operation is running: then the active Crow window asks first.
pub fn quit(cx: &mut App) {
    if cx.global::<Operations>().running.is_none() {
        cx.quit();
        return;
    }
    let target = cx
        .active_window()
        .and_then(|w| w.downcast::<CrowWindow>())
        .or_else(|| cx.windows().into_iter().find_map(|w| w.downcast::<CrowWindow>()));
    match target {
        // Deferred: ⌘Q and the palette dispatch from inside that window's own update.
        Some(handle) => cx.defer(move |cx| {
            if handle.update(cx, |this, window, cx| this.ask_to_stop(AfterStop::Quit, window, cx)).is_err() {
                cx.quit();
            }
        }),
        None => cx.quit(),
    }
}

/// Cancel the running operation, wait for it to wind down (its undo record lands in
/// the log), then close `window` or quit.
fn stop_then(then: AfterStop, window: AnyWindowHandle, cx: &mut App) {
    operations::cancel_running(cx);
    cx.spawn(async move |cx| {
        while cx.update(|cx| cx.global::<Operations>().running.is_some()) {
            cx.background_executor().timer(std::time::Duration::from_millis(30)).await;
        }
        cx.update(|cx| match then {
            AfterStop::Quit => cx.quit(),
            AfterStop::CloseWindow => {
                window.update(cx, |_, window, _| window.remove_window()).ok();
            }
        });
    })
    .detach();
}

impl CrowWindow {
    fn on_pane_event(&mut self, event: &PaneEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            PaneEvent::Run(request) => self.run_request(request.clone(), window, cx),
            PaneEvent::Copy { paths, cut } => self.copy_paths(paths.clone(), *cut, cx),
            PaneEvent::Entered(dir) => {
                // A stale message would otherwise outlive the place it described (Q2).
                operations::clear_message(cx);
                self.close_tip(cx);
                if self.search.is_some() {
                    self.close_search(window, cx);
                }
                if self.walk_visit.as_ref() != Some(dir) {
                    self.walk_visit = None;
                    if !self.bench {
                        ui::record_visit(dir, cx);
                    }
                }
                let dir = dir.clone();
                self.sidebar.update(cx, |s, cx| s.set_current(dir, cx));
            }
            PaneEvent::Mark { key, set } => self.mark(*key, *set, window, cx),
            PaneEvent::FilterEnded { empty, matches } => {
                if self.zero_streak.end_session(*empty, *matches) {
                    // A navigation's Entered event closes tips; offer after it.
                    cx.defer_in(window, |this, window, cx| this.offer_tip(TipId::Search, window, cx));
                }
            }
            PaneEvent::LongFolderScrolled => self.offer_tip(TipId::Filter, window, cx),
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
                    operations::message(e.to_string(), true, cx);
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
        let rename_result = matches!(plan.request, Request::NewFolder { .. } | Request::NewFile { .. });
        let trash_paths = match &plan.request {
            Request::Trash { paths } => Some(paths.clone()),
            _ => None,
        };
        let move_paths = match &plan.request {
            Request::Move { paths, into } => Some((paths.clone(), into.clone())),
            _ => None,
        };
        let failed_paths = trash_paths.clone().or_else(|| move_paths.as_ref().map(|(paths, _)| paths.clone()));
        let pane = self.pane().downgrade();
        // The effect starts with the operation, not the key press: a conflict or a
        // delete sheet may have come first, and a second operation is refused.
        if cx.global::<Operations>().running.is_none() {
            if let Some(paths) = &trash_paths {
                pane.update(cx, |pane, cx| pane.start_effect(paths.clone(), Fx::Trash, cx)).ok();
            }
            if let Some((paths, into)) = &move_paths {
                pane.update(cx, |pane, cx| pane.start_effect(paths.clone(), Fx::MoveOut, cx)).ok();
                // The folder the items land in pulses, in the list and in the sidebar.
                pane.update(cx, |pane, cx| pane.start_effect(vec![into.clone()], Fx::Receive, cx)).ok();
                self.sidebar.update(cx, |sidebar, cx| sidebar.pulse(into.clone(), cx));
            }
        }
        operations::execute(plan, resolutions, cx, move |outcome, cx| {
            if outcome.error.is_some()
                && let Some(paths) = &failed_paths
            {
                // Whatever is still there refuses.
                pane.update(cx, |pane, cx| pane.start_effect(paths.clone(), Fx::Refuse, cx)).ok();
            }
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
                    Err(e) => operations::message(e.to_string(), true, cx),
                }
            }
            Some(Sheet::StopOperation { then, .. }) => {
                stop_then(then, window.window_handle(), cx);
                return;
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
            Sheet::StopOperation { then, .. } => {
                let label = cx
                    .global::<Operations>()
                    .running
                    .as_ref()
                    .map_or_else(|| "The operation".to_owned(), |r| operations::running_label(&r.label));
                let (verb, confirm) = match then {
                    AfterStop::CloseWindow => ("Closing the last window quits Crow", "Stop and Close"),
                    AfterStop::Quit => ("Quitting", "Stop and Quit"),
                };
                (
                    format!("{label} is still running"),
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(div().text_color(color(muted)).child(format!(
                            "{verb} would cut it off. Stop it cleanly first? Items already finished stay (Undo can reverse them); the one in progress is left out, never half done."
                        )))
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap_2()
                                .child(
                                    button("sheet-cancel", "Keep Running".into(), false)
                                        .on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx))),
                                )
                                .child(
                                    button("sheet-confirm", confirm.into(), true)
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
    /// Tab, Return, → or a click out of the sidebar: if an arrow walk left the pane on
    /// its folder, that folder is a visit.
    fn record_walk_visit(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = self.walk_visit.take() else { return };
        if !self.bench && self.pane().read(cx).current_dir(cx) == dir {
            ui::record_visit(&dir, cx);
        }
    }

    fn on_sidebar_event(&mut self, event: &SidebarEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            SidebarEvent::Show(dir) => {
                let dir = dir.clone();
                if self.search.is_some() {
                    self.close_search(window, cx);
                }
                self.walk_visit = Some(dir.clone());
                self.pane().update(cx, |pane, cx| pane.show_from_sidebar(dir, cx));
                // close_search focuses the center; the cursor walk stays in the sidebar.
                let sidebar = self.sidebar.read(cx).focus_handle(cx);
                window.focus(&sidebar, cx);
                cx.notify();
            }
            SidebarEvent::Open(dir) => {
                let dir = dir.clone();
                if self.search.is_some() {
                    self.close_search(window, cx);
                }
                self.pane().update(cx, |pane, cx| pane.open_from_sidebar(dir, cx));
                self.record_walk_visit(cx);
                self.focus_center(window, cx);
            }
            SidebarEvent::Run(request) => self.run_request(request.clone(), window, cx),
            SidebarEvent::Message(text, error) => {
                operations::message(text.clone(), *error, cx);
                cx.notify();
            }
        }
    }

    /// Vim marks: `m x` marks the folder shown as favorite `x`; `'x` goes there.
    fn mark(&mut self, key: char, set: bool, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self.pane().read(cx).current_dir(cx);
        if set {
            ui::edit_sidebar(SidebarEdit::SetKey { path: dir, key }, cx);
            operations::message(format!("Marked as {key}"), false, cx);
            cx.notify();
            return;
        }
        let ui = cx.global::<Ui>();
        match ui.sidebar.find_key(key, &ui.home) {
            Some(target) => self.pane().update(cx, |pane, cx| pane.navigate(target, cx)),
            None => {
                window.play_system_bell();
                // A typo, not a lasting problem: an info message that retires.
                operations::message(format!("No mark {key}"), false, cx);
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
        operations::message("This view is now the default for all folders", false, cx);
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
        // With `view_per_folder = false` the window's own view is what shows.
        let (view, view_source) = if ui.settings.view_per_folder {
            (resolved.view.0, resolved.view.1.to_string())
        } else {
            (self.pane().read(cx).mode(), "this window (view_per_folder = false in settings.toml)".to_owned())
        };
        let mode = match view {
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
            ("View", mode.into(), view_source),
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
                Err(e) => operations::message(e, true, cx),
            },
            _ => ui::choose_theme("system", cx),
        }
        self.focus_center(window, cx);
        cx.notify();
    }

    // ----- search -----

    fn show_search(&mut self, _: &ShowSearch, window: &mut Window, cx: &mut Context<Self>) {
        crate::usage::record(&ShowSearch, cx);
        if let Some(search) = &self.search {
            search.update(cx, |search, cx| search.select_all_input(window, cx));
            crate::tips::key_used(TipId::Search, cx);
            // The band's focus ring is drawn from the input's focus state.
            cx.notify();
            return;
        }
        // Another overlay owns the keys; don't move the filter's text into a search
        // that cannot open.
        if self.search_blocked() {
            return;
        }
        crate::tips::key_used(TipId::Search, cx);
        let pane = self.pane().clone();
        let folder = pane.read(cx).current_dir(cx);
        let query = pane.update(cx, |pane, cx| pane.take_filter(cx)).unwrap_or_default();
        self.open_search(SearchStart { folder, query, everything: false }, window, cx);
    }

    /// Whether an overlay other than search is open (search itself is checked by `open_search`).
    fn search_blocked(&self) -> bool {
        self.search.is_some() || self.picker.is_some() || self.sheet.is_some() || self.palette.is_some()
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
        cx.global_mut::<Operations>().clipboard = Some(Clipboard { paths, cut, change_count });
        let what = if n == 1 { "1 item".to_owned() } else { format!("{n} items") };
        operations::message(
            if cut { format!("Cut {what}: paste to move") } else { format!("Copied {what}") },
            false,
            cx,
        );
        cx.notify();
    }

    // ----- command palette -----

    fn show_palette(&mut self, _: &ShowCommandPalette, window: &mut Window, cx: &mut Context<Self>) {
        crate::usage::record(&ShowCommandPalette, cx);
        // Another overlay owns the keys; open_palette would refuse below.
        if self.search_blocked() {
            return;
        }
        crate::tips::key_used(TipId::GoTo, cx);
        self.open_palette("", window, cx);
    }

    /// Help ▸ Crow Keyboard Shortcuts starts the palette in command mode.
    fn keyboard_shortcuts(&mut self, _: &KeyboardShortcuts, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette(">", window, cx);
    }

    fn open_palette(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() || self.picker.is_some() || self.sheet.is_some() || self.search.is_some() {
            return;
        }
        let (cwd, here, subfolders) = {
            let pane = self.pane().read(cx);
            let cwd = pane.current_dir(cx);
            (cwd.clone(), crate::style::display_name(&cwd), pane.subfolders(cx))
        };
        let (home, favorites, recents) = {
            let ui = cx.global::<Ui>();
            let favorites: Vec<PathBuf> =
                ui.favorites().into_iter().flat_map(|(_, group)| group).map(|f| f.path).collect();
            // The folder you are in is the most recent visit but never a place to go,
            // so the first row is the one before it.
            let recents: Vec<PathBuf> = ui.palette_recents.iter().filter(|path| **path != cwd).cloned().collect();
            (ui.home.clone(), favorites, recents)
        };
        let folders = {
            let ui = cx.global::<Ui>();
            palette::folder_candidates(favorites, recents, subfolders, &home, |path| ui.recent.last_visit(path))
        };
        let (mode, text) = match query.strip_prefix('>') {
            Some(rest) => (PaletteMode::Commands, rest.to_owned()),
            None => (PaletteMode::Folders, query.to_owned()),
        };
        let listing = match mode {
            PaletteMode::Folders => palette::folder_listing(&text, &folders, &here),
            PaletteMode::Commands => self.palette_command_listing(&text, cx),
        };
        let input = cx.new(|cx| {
            TextInput::new(text.clone(), mode.placeholder(), cx)
                .bare()
                .with_aria_label(mode.aria_label())
                .report_backspace_at_start()
        });
        let subscription = cx.subscribe_in(&input, window, |this, input, event, window, cx| match event {
            TextInputEvent::Changed => {
                let text = input.read(cx).text().to_owned();
                if this.palette.as_ref().is_some_and(|palette| palette.mode == PaletteMode::Folders)
                    && let Some(rest) = text.strip_prefix('>')
                {
                    let rest = rest.to_owned();
                    if let Some(palette) = &mut this.palette {
                        palette.mode = PaletteMode::Commands;
                    }
                    // Emits Changed once more, after this handler: mode is already
                    // Commands then, so the nested pass only refreshes.
                    input.update(cx, |input, cx| input.set_text(rest, window, cx));
                }
                let text = input.read(cx).text().to_owned();
                this.refresh_palette(&text, cx);
                this.update_palette_field(cx);
            }
            TextInputEvent::BackspaceAtStart => {
                let empty = input.read(cx).text().is_empty();
                if empty && this.palette.as_ref().is_some_and(|palette| palette.mode == PaletteMode::Commands) {
                    if let Some(palette) = &mut this.palette {
                        palette.mode = PaletteMode::Folders;
                    }
                    this.refresh_palette("", cx);
                    this.update_palette_field(cx);
                } else {
                    window.play_system_bell();
                }
            }
            TextInputEvent::Confirmed => this.confirm_palette(false, window, cx),
            TextInputEvent::Cancelled => this.close_palette(window, cx),
            TextInputEvent::CompletionRequested => this.complete_palette(window, cx),
        });
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.palette = Some(Palette {
            input,
            mode,
            listing,
            here,
            path: None,
            cursor: 0,
            folders,
            scroll: ScrollHandle::new(),
            _subscription: subscription,
        });
        // A typed path (Shift-Cmd-G) lists that folder's children right away.
        if mode == PaletteMode::Folders && text.starts_with(['/', '~']) {
            self.refresh_palette(&text, cx);
        }
        cx.notify();
    }

    /// Keep the field's placeholder and spoken name in step with the mode.
    fn update_palette_field(&self, cx: &mut Context<Self>) {
        let Some(palette) = self.palette.as_ref() else { return };
        let (mode, input) = (palette.mode, palette.input.clone());
        input.update(cx, |input, cx| {
            input.set_placeholder(mode.placeholder(), cx);
            input.set_aria_label(mode.aria_label(), cx);
        });
    }

    /// Command rows with "Recently used" from the state file, unless the setting is off.
    fn palette_command_listing(&self, query: &str, cx: &App) -> palette::Listing {
        let ui = cx.global::<Ui>();
        let recent: Vec<palette::CommandId> =
            ui.recent_commands.ids().iter().filter_map(|key| palette::CommandId::from_key(key)).collect();
        palette::command_listing(query, &recent, ui.settings.palette_recent_commands)
    }

    fn refresh_palette(&mut self, query: &str, cx: &mut Context<Self>) {
        let started = std::time::Instant::now();
        if self.palette.is_none() {
            return;
        }
        let cwd = self.pane().read(cx).current_dir(cx);
        let home = cx.global::<Ui>().home.clone();
        let show_hidden = self.show_hidden;
        let command_listing = (self.palette.as_ref().map(|palette| palette.mode) == Some(PaletteMode::Commands))
            .then(|| self.palette_command_listing(query, cx));
        let mut spawn: Option<(PathBuf, bool, u64)> = None;
        {
            let palette = self.palette.as_mut().expect("checked above");
            palette.cursor = 0;
            palette.scroll.set_offset(point(px(0.), px(0.)));
            if palette.mode == PaletteMode::Commands {
                palette.path = None;
                if let Some(listing) = command_listing {
                    palette.listing = listing;
                }
            } else if let Some((parent, partial)) = palette::path_query(query, &cwd, &home) {
                let hidden = show_hidden || partial.starts_with('.');
                let relist = match &mut palette.path {
                    Some(state) if state.parent == parent => {
                        state.partial.clone_from(&partial);
                        let relist = state.hidden != hidden;
                        state.hidden = hidden;
                        relist
                    }
                    slot => {
                        *slot = Some(PathState {
                            parent: parent.clone(),
                            partial,
                            children: None,
                            missing: false,
                            hidden,
                            generation: 0,
                        });
                        true
                    }
                };
                let state = palette.path.as_mut().expect("set above");
                if relist {
                    state.generation += 1;
                    spawn = Some((parent.clone(), hidden, state.generation));
                }
                let state = palette.path.as_ref().expect("set above");
                palette.listing = match (&state.children, state.missing) {
                    (None, _) => palette::loading_path_listing(&state.parent),
                    (Some(_), true) => palette::missing_path_listing(&state.parent, &home),
                    (Some(children), false) => palette::path_listing(&state.parent, &state.partial, children, &home),
                };
            } else {
                palette.path = None;
                palette.listing = palette::folder_listing(query, &palette.folders, &palette.here);
            }
        }
        if let Some((parent, hidden, generation)) = spawn {
            self.spawn_path_listing(parent, hidden, generation, cx);
        }
        // Budget (U7): under 4 ms with 10 000 subfolders per keystroke.
        tracing::debug!(
            "palette refresh: {:?} for {} items",
            started.elapsed(),
            self.palette.as_ref().map_or(0, |palette| palette.listing.items.len())
        );
        cx.notify();
    }

    /// List a path-mode parent off the main thread; drop the result if the palette
    /// has moved to another parent (or another hidden setting) meanwhile.
    fn spawn_path_listing(&mut self, parent: PathBuf, hidden: bool, generation: u64, cx: &mut Context<Self>) {
        let target = parent.clone();
        let task = cx.background_spawn(async move {
            let mut children = Vec::new();
            let listed = crow_fs::list_dir(&target, hidden, |batch| {
                children.extend(
                    batch.into_iter().filter(|entry| entry.is_dir && !entry.is_package()).map(|entry| entry.path),
                );
            });
            (children, listed.is_err())
        });
        cx.spawn(async move |this, cx| {
            let (children, missing) = task.await;
            this.update(cx, |this, cx| {
                let applied = match &mut this.palette {
                    Some(palette) => match &mut palette.path {
                        Some(state) if state.parent == parent && state.generation == generation => {
                            state.children = Some(children);
                            state.missing = missing;
                            true
                        }
                        _ => false,
                    },
                    None => false,
                };
                if applied {
                    let query = this.palette.as_ref().map(|p| p.input.read(cx).text().to_owned()).unwrap_or_default();
                    this.refresh_palette(&query, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Tab: put the highlighted folder (and a `/`) in the field, or complete the
    /// typed path like Go to Folder. Command mode ignores Tab.
    fn complete_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(palette) = self.palette.as_ref() else { return };
        if palette.mode == PaletteMode::Commands {
            return;
        }
        if let Some(Item::Folder { label, .. }) = palette.listing.items.get(palette.cursor) {
            let text = match label.as_str() {
                "/" => "/".to_owned(),
                "~" => "~/".to_owned(),
                _ => format!("{label}/"),
            };
            let input = palette.input.clone();
            input.update(cx, |input, cx| input.set_text(text, window, cx));
            return;
        }
        if palette.path.is_none() {
            window.play_system_bell();
            return;
        }
        let input = palette.input.clone();
        let query = palette.input.read(cx).text().to_owned();
        let typed = query.clone();
        let cwd = self.pane().read(cx).current_dir(cx);
        let home = cx.global::<Ui>().home.clone();
        let task = cx.background_spawn(async move { crow_core::complete_typed(&typed, &cwd, &home) });
        cx.spawn_in(window, async move |this, cx| {
            let completed = task.await;
            this.update_in(cx, |this, window, cx| {
                let Some(palette) = this.palette.as_ref() else { return };
                if palette.input.read(cx).text() != query {
                    return;
                }
                if let Some(completed) = completed {
                    input.update(cx, |input, cx| input.set_text(completed, window, cx));
                }
            })
            .ok();
        })
        .detach();
    }

    fn move_palette(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(palette) = &mut self.palette else { return };
        let Some(last) = palette.listing.items.len().checked_sub(1) else { return };
        palette.cursor = palette.cursor.saturating_add_signed(delta).min(last);
        let content = palette_content_height(&palette.listing);
        let visible = palette_list_height(window, content);
        let (y, height) = palette_row_y(&palette.listing, palette.cursor);
        let max = (content - visible).max(0.);
        let current = f32::from(palette.scroll.offset().y);
        let mut target = current;
        if y < -current {
            target = -y;
        } else if y + height > -current + visible {
            target = -(y + height - visible);
        }
        let target = target.clamp(-max, 0.);
        if target != current {
            palette.scroll.set_offset(point(px(0.), px(target)));
        }
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

    /// Whether palette command `id` is a toggle, and its current state.
    fn toggle_state(&self, id: palette::CommandId, cx: &App) -> Option<bool> {
        match id {
            palette::CommandId::ToggleHiddenFiles => Some(self.show_hidden),
            palette::CommandId::ToggleActivity => Some(self.show_activity),
            palette::CommandId::TogglePreview => Some(cx.global::<Ui>().settings.show_preview),
            _ => None,
        }
    }

    fn confirm_palette(&mut self, new_tab: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(palette) = self.palette.as_ref() else { return };
        let item = palette.listing.items.get(palette.cursor).cloned();
        // A typed path with no folder row may name a file: reveal it, as Go to Folder did.
        if item.is_none() && palette.path.is_some() {
            self.reveal_typed_path(window, cx);
            return;
        }
        // Typing a whole path for a folder the palette already offers is the slow way.
        let path_mode = palette.path.is_some();
        self.palette = None;
        self.focus_center(window, cx);
        match item {
            Some(Item::Command { command, .. }) => {
                self.run_command(command.id, window, cx);
                ui::record_command(command.id, cx);
            }
            Some(Item::Folder { path, .. }) if new_tab => self.open_tab_at(path, window, cx),
            Some(Item::Folder { path, .. }) => {
                let known = path_mode && self.known_folder(&path, cx);
                self.pane().update(cx, |pane, cx| pane.navigate(path, cx));
                if known {
                    // Navigating closes tips; offer this one after its Entered event.
                    cx.defer_in(window, |this, window, cx| this.offer_tip(TipId::GoTo, window, cx));
                }
            }
            None => {}
        }
        cx.notify();
    }

    /// Whether `path` is somewhere ⌘K already offers: a recent or favorite folder,
    /// other than the one on screen.
    fn known_folder(&self, path: &Path, cx: &App) -> bool {
        path != self.pane().read(cx).current_dir(cx) && cx.global::<Ui>().palette_recents.iter().any(|p| p == path)
    }

    /// Return on a path that lists no folder: off the main thread, check whether it
    /// names a file; reveal it if so, otherwise beep and leave the palette open.
    fn reveal_typed_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(palette) = self.palette.as_ref() else { return };
        let typed = palette.input.read(cx).text().to_owned();
        let cwd = self.pane().read(cx).current_dir(cx);
        let home = cx.global::<Ui>().home.clone();
        let task = cx.background_spawn(async move {
            let target = crow_core::resolve_typed(&typed, &cwd, &home);
            target.exists().then(|| (target.is_dir(), target, typed))
        });
        cx.spawn_in(window, async move |this, cx| {
            let found = task.await;
            this.update_in(cx, |this, window, cx| {
                let Some(palette) = this.palette.as_ref() else { return };
                match found {
                    Some((is_dir, target, typed)) if palette.input.read(cx).text() == typed => {
                        this.palette = None;
                        this.focus_center(window, cx);
                        if is_dir {
                            let known = this.known_folder(&target, cx);
                            this.pane().update(cx, |pane, cx| pane.navigate(target, cx));
                            if known {
                                cx.defer_in(window, |this, window, cx| this.offer_tip(TipId::GoTo, window, cx));
                            }
                        } else {
                            this.pane().update(cx, |pane, cx| pane.reveal(target, cx));
                        }
                        cx.notify();
                    }
                    Some(_) => {}
                    None => window.play_system_bell(),
                }
            })
            .ok();
        })
        .detach();
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
            CommandId::TogglePreview => self.toggle_preview(&TogglePreview, window, cx),
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
            CommandId::Quit => quit(cx),
            other => self.pane().update(cx, |pane, cx| pane.run_command(other, window, cx)),
        }
    }

    fn render_palette(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let palette = self.palette.as_ref()?;
        let input = palette.input.clone();
        let scroll = palette.scroll.clone();
        let count = palette.listing.count.clone();
        let commands = palette.mode == PaletteMode::Commands;
        let content = palette_content_height(&palette.listing);
        let height = palette_list_height(window, content);
        let width = palette_width(window);
        let rows = self.render_palette_rows(window, cx);
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        let vim = ui.settings.keymap == crow_core::KeymapLayer::Vim;
        let mut context = "Sheet Palette".to_owned();
        if vim {
            context.push_str(" vim");
        }
        let muted = color(theme.role("fg-muted"));
        // `border`, not `surface-raised`: some themes (Solarized Light) make
        // surface-raised the same colour as the sheet's surface.
        let border = color(theme.role("border"));
        let pair = |cap: Option<Keycap>, label: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .children(cap)
                .child(div().text_size(px(12.)).text_color(muted).child(label))
                .into_any_element()
        };
        let outline = |action: &dyn gpui::Action| {
            Keycap::for_action(action, KeyScope::Palette, KeycapWeight::Outline, cx).map(|cap| cap.border("border"))
        };
        let confirm = outline(&crate::text_input::Confirm);
        let footer: gpui::AnyElement = if commands {
            div()
                .h(px(PALETTE_FOOTER))
                .px(px(16.))
                .flex()
                .items_center()
                .gap(px(16.))
                .border_t_1()
                .border_color(border)
                .child(pair(confirm, "run"))
                .child(pair(outline(&crate::text_input::Backspace), "folders"))
                .into_any_element()
        } else {
            div()
                .h(px(PALETTE_FOOTER))
                .px(px(16.))
                .flex()
                .items_center()
                .gap(px(16.))
                .border_t_1()
                .border_color(border)
                .child(pair(confirm, "go"))
                .child(pair(outline(&PaletteOpenInTab), "new tab"))
                .child(pair(outline(&crate::text_input::Complete), "complete"))
                .child(div().flex_1())
                .child(pair(Some(Keycap::text(">", KeycapWeight::Outline).border("border")), "commands"))
                .into_any_element()
        };
        let token = div()
            .id("palette-commands-token")
            .debug_selector(|| "palette-commands-token".into())
            .h(px(24.))
            .px(px(8.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .bg(color_alpha(theme.role("accent"), 0x29))
            .text_size(px(12.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(color(theme.role("accent")))
            .child("Commands");
        let input_row = div()
            .h(px(PALETTE_INPUT))
            .px(px(16.))
            .flex()
            .items_center()
            .gap(px(12.))
            .border_b_1()
            .border_color(border)
            .when(commands, |row| row.child(token))
            .when(!commands, |row| row.child(icons::glyph(Glyph::GoTo, 16., "fg-muted", &theme)))
            .child(div().flex_1().min_w_0().text_size(px(15.)).child(input))
            .child(
                div()
                    .id("palette-count")
                    .role(Role::Status)
                    .aria_label(count.to_string())
                    .text_size(px(12.))
                    .text_color(muted)
                    .child(count),
            );
        let list = div()
            .id("palette-list")
            .role(Role::ListBox)
            .aria_label(if commands { "Commands" } else { "Places" })
            .h(px(height))
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .p(px(PALETTE_LIST_PAD))
            .children(rows);
        Some(chrome::sheet(
            chrome::sheet_panel("palette", SheetWidth::Wide, &theme)
                .w(px(width))
                .aria_label("Go to or Command Palette")
                // keys::context_stack (KeyScope::Palette/ThemePicker) mirrors this.
                .key_context(context.as_str())
                .on_action(cx.listener(|this, _: &MoveUp, window, cx| this.move_palette(-1, window, cx)))
                .on_action(cx.listener(|this, _: &MoveDown, window, cx| this.move_palette(1, window, cx)))
                .on_action(cx.listener(Self::palette_open_in_tab))
                .flex()
                .flex_col()
                .child(input_row)
                .child(list)
                .child(footer),
            &theme,
            cx.listener(|this, _, window, cx| this.close_palette(window, cx)),
        ))
    }

    fn render_palette_rows(&self, window: &mut Window, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let Some(palette) = self.palette.as_ref() else { return Vec::new() };
        let theme = cx.global::<Ui>().theme.clone();
        let muted = color(theme.role("fg-muted"));
        if palette.listing.rows.is_empty() {
            let query = palette.input.read(cx).text().to_owned();
            let text: SharedString = match palette.mode {
                PaletteMode::Commands => format!("No commands match \"{query}\"").into(),
                PaletteMode::Folders if query.is_empty() => "Type a folder name, or > for commands".into(),
                PaletteMode::Folders => format!("No folders match \"{query}\"").into(),
            };
            return vec![
                div()
                    .id("palette-empty")
                    .debug_selector(|| "palette-empty".into())
                    .h(px(PALETTE_EMPTY_ROW))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .role(Role::Label)
                    .aria_label(text.clone())
                    .text_size(px(13.))
                    .text_color(muted)
                    .child(text)
                    .into_any_element(),
            ];
        }
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let mut sections: Vec<(SharedString, Vec<gpui::AnyElement>)> = Vec::new();
        let mut headers = 0usize;
        for row in &palette.listing.rows {
            match row {
                Row::Header(text) => {
                    let label = text.clone();
                    let k = headers;
                    let header = div()
                        .id(("palette-header", k))
                        .debug_selector(|| format!("palette-header-{k}"))
                        .h(px(PALETTE_HEADER_ROW))
                        .px(px(10.))
                        .pt(px(10.))
                        .pb(px(8.))
                        .text_size(px(12.))
                        .line_height(px(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(muted)
                        .child(
                            div()
                                .id(("palette-header-label", k))
                                .debug_selector(|| format!("palette-header-label-{k}"))
                                .role(Role::Label)
                                .child(text.clone()),
                        )
                        .into_any_element();
                    headers += 1;
                    sections.push((label, vec![header]));
                }
                Row::Status(text) => {
                    let k = headers;
                    let row = div()
                        .id(("palette-status", k))
                        .debug_selector(|| format!("palette-status-{k}"))
                        .h(px(PALETTE_EMPTY_ROW))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .role(Role::Label)
                        .aria_label(text.clone())
                        .text_size(px(13.))
                        .text_color(muted)
                        .child(text.clone())
                        .into_any_element();
                    headers += 1;
                    match sections.last_mut() {
                        Some((_, rows)) => rows.push(row),
                        None => sections.push(("Places".into(), vec![row])),
                    }
                }
                Row::Item(i) => {
                    let selected = *i == palette.cursor;
                    let this_folder = *i == 0 && palette.path.as_ref().is_some_and(|state| state.partial.is_empty());
                    let row = match &palette.listing.items[*i] {
                        Item::Command { .. } => {
                            self.render_palette_command_row(&palette.listing, *i, selected, &theme, cx)
                        }
                        Item::Folder { .. } => self.render_palette_folder_row(
                            &palette.listing,
                            *i,
                            &palette.here,
                            selected,
                            &theme,
                            now,
                            this_folder.then_some("this folder"),
                            window,
                            cx,
                        ),
                    };
                    match sections.last_mut() {
                        Some((_, rows)) => rows.push(row),
                        None => sections.push(("Places".into(), vec![row])),
                    }
                }
            }
        }
        sections
            .into_iter()
            .enumerate()
            .map(|(k, (label, rows))| {
                div()
                    .id(("palette-section", k))
                    .role(Role::Group)
                    .aria_label(label)
                    .flex()
                    .flex_col()
                    .children(rows)
                    .into_any_element()
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_palette_folder_row(
        &self,
        listing: &palette::Listing,
        i: usize,
        here: &str,
        selected: bool,
        theme: &crow_theme::Theme,
        now: u64,
        note: Option<&'static str>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Item::Folder { path, label, name_start, source, last_visit, matched, .. } = &listing.items[i] else {
            return div().into_any_element();
        };
        let name = label[*name_start..].to_owned();
        let (name_text, name_ranges) = (name.clone(), listing.items[i].name_ranges());
        let (path_text, path_ranges) =
            truncate_middle_with_ranges(label, matched, px(palette_path_width(window)), px(12.), window);
        let section = palette::section_header(*source, here);
        let age = if *source == Source::Recent {
            last_visit.as_ref().map(|then| palette::relative_age(now, *then))
        } else {
            None
        };
        let aria = match &age {
            Some(age) => format!("{name}, {label}, {section}, visited {age}"),
            None => format!("{name}, {label}, {section}"),
        };
        let right = note.map(str::to_owned).or(age);
        let muted = color(theme.role("fg-muted"));
        let hover = color(theme.role("surface-raised"));
        div()
            .id(("palette-row", i))
            .debug_selector(|| format!("palette-row-{i}"))
            .role(Role::ListBoxOption)
            .aria_label(aria)
            .aria_position_in_set(i + 1)
            .aria_size_of_set(listing.items.len())
            .aria_selected(selected)
            .when(selected, |row| row.aria_active_descendant())
            .h(px(PALETTE_FOLDER_ROW))
            .px(px(10.))
            .rounded(px(8.))
            .flex()
            .items_center()
            .gap(px(12.))
            .when(selected, |row| row.bg(active_selection(theme)))
            .when(!selected, |row| row.hover(move |style| style.bg(hover)))
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some(palette) = &mut this.palette {
                    palette.cursor = i;
                }
                this.confirm_palette(false, window, cx);
            }))
            .child(div().size(px(22.)).flex_none().children(icon(path, true, 22).map(|image| img(image).size(px(22.)))))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .child(div().text_size(px(14.)).overflow_hidden().whitespace_nowrap().child(highlight_text(
                        name_text,
                        &name_ranges,
                        theme,
                        selected,
                    )))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(muted)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(highlight_text(path_text, &path_ranges, theme, selected)),
                    ),
            )
            .children(right.map(|right| div().text_size(px(12.)).text_color(muted).child(right)))
            .into_any_element()
    }

    fn render_palette_command_row(
        &self,
        listing: &palette::Listing,
        i: usize,
        selected: bool,
        theme: &crow_theme::Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Item::Command { command, matched } = &listing.items[i] else { return div().into_any_element() };
        let binding = keys::binding_for(command.action, command.scope, cx);
        let toggle = self.toggle_state(command.id, cx);
        let mut aria = command.label.to_owned();
        if let Some(slot) = command.menu {
            aria.push_str(&format!(", {} menu", slot.menu.title()));
        }
        if let Some(on) = toggle {
            aria.push_str(if on { ", on" } else { ", off" });
        }
        let fg = color(theme.role("fg"));
        let muted = color(theme.role("fg-muted"));
        // `border`, not `surface-raised`: some themes (Solarized Light) make
        // surface-raised the same colour as the sheet's surface.
        let border = color(theme.role("border"));
        let hover = color(theme.role("surface-raised"));
        let chip = toggle.map(|on| {
            div()
                .h(px(18.))
                .px(px(7.))
                .rounded(px(8.))
                .border_1()
                .border_color(border)
                .flex()
                .items_center()
                .text_size(px(12.))
                .text_color(if on { fg } else { muted })
                .child(if on { "on" } else { "off" })
        });
        div()
            .id(("palette-row", i))
            .debug_selector(|| format!("palette-row-{i}"))
            .role(Role::ListBoxOption)
            .aria_label(aria)
            .when_some(binding.as_ref().map(keys::chord_text), |row, shortcut| row.aria_keyshortcuts(shortcut))
            .when_some(toggle, |row, on| row.aria_toggled(if on { Toggled::True } else { Toggled::False }))
            .aria_position_in_set(i + 1)
            .aria_size_of_set(listing.items.len())
            .aria_selected(selected)
            .when(selected, |row| row.aria_active_descendant())
            .h(px(PALETTE_COMMAND_ROW))
            .px(px(10.))
            .rounded(px(8.))
            .flex()
            .items_center()
            .gap(px(12.))
            .when(selected, |row| row.bg(active_selection(theme)))
            .when(!selected, |row| row.hover(move |style| style.bg(hover)))
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some(palette) = &mut this.palette {
                    palette.cursor = i;
                }
                this.confirm_palette(false, window, cx);
            }))
            .child(
                div()
                    .w(px(64.))
                    .flex_none()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(px(12.))
                    .text_color(muted)
                    .children(command.menu.map(|slot| slot.menu.title())),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(px(14.))
                    .child(highlight_text(command.label, matched, theme, selected)),
            )
            .children(chip)
            .children(binding.map(|binding| Keycap::from_binding(&binding, KeycapWeight::Key)))
            .into_any_element()
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
        let show_preview = ui.settings.show_preview;
        let (font, font_size) = (ui_font(&ui.settings.font_family), ui.settings.font_size);
        let title = crate::style::display_name(&self.pane().read(cx).current_dir(cx));
        let list_view = self.pane().read(cx).mode() == ViewMode::List;
        window.set_window_title(&title);
        let title_bar = self.render_title_bar(&title, window, cx);
        let tab_bar = (self.tabs.len() > 1).then(|| self.render_tab_bar(cx));
        let current = self.pane().read(cx).current_dir(cx);
        self.sidebar.update(cx, |s, cx| s.set_current(current, cx));
        let sidebar = self.sidebar.clone();
        let search_preview = self.search.as_ref().and_then(|search| search.read(cx).preview_parts());
        let narrow = window.viewport_size().width < px(900.);
        let preview: Option<gpui::AnyElement> = if !show_preview {
            None
        } else if self.search.is_some() {
            if narrow { None } else { search_preview }.map(|(entry, preview)| {
                self.render_preview_pane(Some(&entry), preview.as_ref(), &theme).into_any_element()
            })
        } else if list_view {
            Some(self.render_preview(cx).into_any_element())
        } else {
            None
        };
        let path_bar = self.render_bottom_bar(window, cx);
        // Close a tip whose reason is gone: hints off, another window owns it, or a
        // blocking state started (rename, drag, overlay, activity).
        if self.tip.is_some()
            && (self.tip_blocked(cx)
                || cx.global::<Ui>().settings.hints == HintsMode::Off
                || crate::tips::owner(cx) != Some(cx.entity_id()))
        {
            self.tip = None;
            crate::tips::closed(cx.entity_id(), cx);
        }
        let tip = match self.tip {
            Some(id) => crate::tips::key_text(id, cx).map(|key| {
                crate::tips::render_tip(
                    id,
                    key,
                    &theme,
                    f32::from(window.viewport_size().width),
                    cx.listener(|this, _, _, cx| this.close_tip(cx)),
                )
            }),
            None => None,
        };
        let sheet = self.render_sheet(cx);
        let activity = self.show_activity.then(|| self.render_activity(cx));
        let why = self.render_why(cx);
        let picker = self.render_picker(cx);
        let palette = self.render_palette(window, cx);
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
            .on_action(cx.listener(Self::toggle_preview))
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
                } else if let Some(search) = &this.search
                    && search.update(cx, |search, cx| {
                        let closed = search.close_skipped();
                        if closed {
                            cx.notify();
                        }
                        closed
                    })
                {
                    // Esc closed the skipped list; the next one closes search.
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
            .children(tip)
            .children(sheet)
            .children(activity)
            .children(why)
            .children(picker)
            .children(palette)
    }
}

#[cfg(test)]
impl CrowWindow {
    /// The palette's rows for tests: "# Header", a folder's abbreviated path or a
    /// command's label.
    fn palette_rows(&self) -> Vec<String> {
        let Some(palette) = self.palette.as_ref() else { return Vec::new() };
        palette
            .listing
            .rows
            .iter()
            .map(|row| match row {
                Row::Header(text) => format!("# {text}"),
                Row::Status(text) => text.to_string(),
                Row::Item(i) => match &palette.listing.items[*i] {
                    Item::Folder { label, .. } => label.clone(),
                    Item::Command { command, .. } => command.label.to_owned(),
                },
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use gpui::{Action, Entity, TestAppContext, VisualTestContext, WindowHandle};

    use crow_search::{Hit, HitKind};

    use crate::actions::StartFilter;

    use super::*;

    fn test_ranges(start: usize, end: usize) -> Vec<std::ops::Range<usize>> {
        std::iter::once(start..end).collect()
    }

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

    /// Wait until the palette is not waiting on a background parent listing.
    fn settle_palette(view: &Entity<CrowWindow>, cx: &mut VisualTestContext) {
        for _ in 0..400 {
            cx.run_until_parked();
            let loading = view.read_with(cx, |w, _| {
                w.palette.as_ref().is_some_and(|palette| {
                    palette
                        .listing
                        .rows
                        .iter()
                        .any(|row| matches!(row, Row::Status(text) if text.as_ref() == "Loading…"))
                })
            });
            if !loading {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("the palette never finished listing");
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

    /// What each selectable sidebar row shows, as the window renders it.
    fn sidebar_looks(view: &Entity<CrowWindow>, cx: &mut VisualTestContext) -> Vec<crate::sidebar::RowLook> {
        let sidebar = view.read_with(cx, |w, _| w.sidebar.clone());
        cx.update(|window, cx| sidebar.read_with(cx, |s, cx| s.looks_for_test(window, cx)))
    }

    #[gpui::test]
    fn sidebar_looks_follow_rows(cx: &mut TestAppContext) {
        let root = fixture("looks");
        let (view, mut cx) = open(cx, &root);
        write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("alpha"), &root.join("beta")]));
        let looks = sidebar_looks(&view, &mut cx);
        let labels: Vec<&str> = looks.iter().map(|look| look.label.as_str()).collect();
        assert_eq!(labels, ["alpha", "beta"]);
        assert!(looks.iter().all(|look| look.spoken == look.label), "no hint or mark yet: {looks:?}");
    }

    #[gpui::test]
    fn sidebar_twins_get_their_parent_in_label_and_aria(cx: &mut TestAppContext) {
        let root = fixture("twins");
        std::fs::create_dir_all(root.join("one/git")).expect("create fixture");
        std::fs::create_dir_all(root.join("two/git")).expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("one/git"), &root.join("two/git")]));
        let looks = sidebar_looks(&view, &mut cx);
        let hints: Vec<Option<&str>> = looks.iter().map(|look| look.hint.as_deref()).collect();
        assert_eq!(hints, [Some("one"), Some("two")]);
        assert_eq!(looks[0].spoken, "git, in one");
        assert_eq!(looks[1].spoken, "git, in two");
    }

    #[gpui::test]
    fn sidebar_twins_span_favorites_and_recent(cx: &mut TestAppContext) {
        let root = fixture("twins-recent");
        std::fs::create_dir_all(root.join("one/git")).expect("create fixture");
        std::fs::create_dir_all(root.join("two/git")).expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        write_config(
            &view,
            &mut cx,
            "sidebar.toml",
            &favorites_toml(&[&root.join("one/git")]).replace("show = false", "show = true"),
        );
        pane(&view, &cx).update(&mut cx, |pane, cx| pane.navigate(root.join("two/git"), cx));
        settle(&view, &mut cx);
        let looks = sidebar_looks(&view, &mut cx);
        let git: Vec<&crate::sidebar::RowLook> = looks.iter().filter(|look| look.label == "git").collect();
        assert_eq!(git.len(), 2, "one favorite, one recent: {looks:?}");
        assert_eq!(git[0].hint.as_deref(), Some("one"));
        assert_eq!(git[1].hint.as_deref(), Some("two"));
    }

    #[gpui::test]
    fn sidebar_twins_walk_up(cx: &mut TestAppContext) {
        let root = fixture("twins-walk");
        std::fs::create_dir_all(root.join("x/a/git")).expect("create fixture");
        std::fs::create_dir_all(root.join("y/a/git")).expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("x/a/git"), &root.join("y/a/git")]));
        let looks = sidebar_looks(&view, &mut cx);
        let hints: Vec<Option<&str>> = looks.iter().map(|look| look.hint.as_deref()).collect();
        assert_eq!(hints, [Some("x"), Some("y")]);
    }

    #[gpui::test]
    fn sidebar_twin_in_home_shows_a_tilde(cx: &mut TestAppContext) {
        let root = fixture("twins-home");
        std::fs::create_dir_all(root.join("git")).expect("create fixture");
        std::fs::create_dir_all(root.join("other/git")).expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        cx.update(|_, cx| cx.global_mut::<Ui>().home = root.clone());
        write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("git"), &root.join("other/git")]));
        let looks = sidebar_looks(&view, &mut cx);
        assert_eq!(looks[0].hint.as_deref(), Some("~"));
        assert_eq!(looks[1].hint.as_deref(), Some("other"));
        assert_eq!(looks[0].spoken, format!("git, in {}", root.file_name().unwrap().to_string_lossy()));
    }

    #[gpui::test]
    fn sidebar_cursor_is_a_ring_and_only_the_current_folder_is_filled(cx: &mut TestAppContext) {
        let root = fixture("ring");
        let (view, mut cx) = open(cx, &root.join("alpha"));
        write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("alpha"), &root.join("beta")]));

        let looks = sidebar_looks(&view, &mut cx);
        assert!(looks[0].current && !looks[0].ring, "unfocused: the current folder is filled: {looks:?}");
        assert!(!looks[1].current && !looks[1].ring);

        cx.simulate_keystrokes("tab");
        let looks = sidebar_looks(&view, &mut cx);
        assert!(looks[0].current && looks[0].ring, "cursor 0 on the current folder: fill and ring");
        assert!(!looks[1].ring);

        cx.simulate_keystrokes("down");
        settle(&view, &mut cx);
        let looks = sidebar_looks(&view, &mut cx);
        assert!(!looks[0].current && !looks[0].ring, "the fill left alpha with the pane");
        assert!(looks[1].current && looks[1].ring, "beta is current and carries the cursor ring");

        cx.simulate_keystrokes("tab");
        let looks = sidebar_looks(&view, &mut cx);
        assert!(looks.iter().all(|look| !look.ring), "no ring while the list has focus");
        assert!(looks[1].current, "the current folder stays filled");
    }

    #[gpui::test]
    fn sidebar_vim_j_moves_the_ring(cx: &mut TestAppContext) {
        let root = fixture("ring-vim");
        let (view, mut cx) = open(cx, &root);
        write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("alpha"), &root.join("beta")]));
        use_vim(&view, &mut cx);
        cx.simulate_keystrokes("tab j");
        let looks = sidebar_looks(&view, &mut cx);
        assert!(looks[1].ring && !looks[0].ring, "j moves the ring: {looks:?}");
    }

    #[gpui::test]
    fn disk_row_shows_free_space_and_keeps_its_aria_label(cx: &mut TestAppContext) {
        let root = fixture("disk");
        let (view, mut cx) = open(cx, &root);
        write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[]));
        cx.update(|_, cx| {
            cx.global_mut::<Ui>().volumes = vec![crow_macos::Volume {
                name: "Test HD".into(),
                path: root.clone(),
                total_bytes: 494_384_795_648,
                free_bytes: 182_400_000_000,
                ejectable: true,
            }];
            cx.refresh_windows();
        });
        cx.run_until_parked();
        let looks = sidebar_looks(&view, &mut cx);
        let disk = looks.iter().find(|look| look.label == "Test HD").expect("volume row");
        assert_eq!(disk.right.as_deref(), Some("182 GB free"));
        let (used, tone) = disk.bar.expect("a bar for a known total");
        assert!((used - 0.631).abs() < 0.001, "used fraction {used}");
        assert_eq!(tone, crate::sidebar::BarTone::Neutral);
        assert_eq!(disk.spoken, "Test HD, 182.4 GB free of 494.4 GB, ejectable");

        cx.update(|_, cx| {
            cx.global_mut::<Ui>().volumes[0].free_bytes = 24_719_239_782;
            cx.refresh_windows();
        });
        cx.run_until_parked();
        let looks = sidebar_looks(&view, &mut cx);
        let disk = looks.iter().find(|look| look.label == "Test HD").expect("volume row");
        assert_eq!(disk.bar.expect("a bar").1, crate::sidebar::BarTone::Warning);
        assert!(disk.spoken.ends_with(", ejectable, almost full"), "{}", disk.spoken);
    }

    #[gpui::test]
    fn vim_marks_show_as_letters_in_the_sidebar(cx: &mut TestAppContext) {
        let root = fixture("marks-letters");
        let (view, mut cx) = open(cx, &root);
        let toml = format!(
            "[[group]]\nname = \"Favorites\"\nitems = [\n  {{ path = \"{}\", key = \"a\" }},\n  {{ path = \"{}\" }},\n]\n[recent]\nshow = false\n",
            root.join("alpha").display(),
            root.join("beta").display()
        );
        write_config(&view, &mut cx, "sidebar.toml", &toml);
        let looks = sidebar_looks(&view, &mut cx);
        assert!(looks.iter().all(|look| look.right.is_none()), "the Finder layer shows no letters: {looks:?}");

        use_vim(&view, &mut cx);
        let looks = sidebar_looks(&view, &mut cx);
        assert_eq!(looks[0].right.as_deref(), Some("a"));
        assert!(looks[1].right.is_none(), "only the marked favorite shows a letter");

        cx.simulate_keystrokes("down cmd-down");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("beta"));
        cx.simulate_keystrokes("m a");
        settle(&view, &mut cx);
        let looks = sidebar_looks(&view, &mut cx);
        assert!(looks[0].right.is_none(), "the mark moved away from alpha");
        assert_eq!(looks[1].right.as_deref(), Some("a"), "beta now shows the mark");
    }

    #[gpui::test]
    fn symlinked_twin_says_alias(cx: &mut TestAppContext) {
        let root = fixture("alias");
        std::fs::create_dir_all(root.join("real/git")).expect("create fixture");
        std::fs::create_dir_all(root.join("link")).expect("create fixture");
        std::os::unix::fs::symlink(root.join("real/git"), root.join("link/git")).expect("create symlink");
        let (view, mut cx) = open(cx, &root);
        write_config(
            &view,
            &mut cx,
            "sidebar.toml",
            &favorites_toml(&[&root.join("real/git"), &root.join("link/git")]),
        );
        for _ in 0..400 {
            cx.run_until_parked();
            if cx.update(|_, cx| cx.global::<Ui>().aliases.contains(&root.join("link/git"))) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let looks = sidebar_looks(&view, &mut cx);
        assert_eq!(looks[0].hint.as_deref(), Some("real"));
        assert_eq!(looks[1].hint.as_deref(), Some("link · alias"));
        assert_eq!(looks[1].spoken, "git, in link, alias");
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
        settle(&view, &mut cx);
        let sidebar = view.read_with(&cx, |w, _| w.sidebar.clone());
        assert_eq!(sidebar.read_with(&cx, |s, _| s.cursor()), 1, "Down in the sidebar moves its cursor");
        assert_eq!(dir(&view, &cx), root.join("beta"), "…and shows beta in the center");
        assert_eq!(selected(&view, &cx).as_deref(), Some("b.txt"), "the list follows the sidebar cursor");

        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("beta"), "Return in the sidebar opens the favorite");
        let list_focus = pane(&view, &cx).read_with(&cx, |p, _| p.focus_handle().clone());
        assert!(cx.update(|window, _| list_focus.is_focused(window)), "…and moves focus to the list");

        cx.simulate_keystrokes("enter");
        assert!(pane(&view, &cx).read_with(&cx, |p, _| p.is_renaming()), "Return in the list means rename");
    }

    #[gpui::test]
    fn sidebar_arrows_show_the_folder_and_keep_focus(cx: &mut TestAppContext) {
        let root = fixture("sidebar-show");
        let (view, mut cx) = open(cx, &root);
        write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("alpha"), &root.join("beta")]));
        assert_eq!(dir(&view, &cx), root);

        cx.simulate_keystrokes("tab");
        assert_eq!(dir(&view, &cx), root, "tabbing into the sidebar does not navigate");
        cx.simulate_keystrokes("down");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("beta"), "Down shows the cursor's folder");
        assert!(sidebar_focused(&view, &mut cx), "…and focus stays in the sidebar");

        cx.simulate_keystrokes("up");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("alpha"));
        assert!(sidebar_focused(&view, &mut cx));

        cx.simulate_keystrokes("tab");
        assert!(!sidebar_focused(&view, &mut cx), "Tab moves to the center");
        assert_eq!(dir(&view, &cx), root.join("alpha"), "…which already shows the cursor's folder");
    }

    #[gpui::test]
    fn sidebar_return_and_right_focus_the_center(cx: &mut TestAppContext) {
        let root = fixture("sidebar-open");
        let (view, mut cx) = open(cx, &root);
        write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("alpha"), &root.join("beta")]));

        cx.simulate_keystrokes("tab down");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("beta"));
        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        assert!(!sidebar_focused(&view, &mut cx), "Return focuses the center");
        assert_eq!(dir(&view, &cx), root.join("beta"));

        cx.simulate_keystrokes("tab");
        assert!(sidebar_focused(&view, &mut cx));
        cx.simulate_keystrokes("right");
        assert!(!sidebar_focused(&view, &mut cx), "→ focuses the center too");
        assert_eq!(dir(&view, &cx), root.join("beta"), "…without navigating again");
    }

    #[gpui::test]
    fn sidebar_arrow_closes_search(cx: &mut TestAppContext) {
        let root = fixture("sidebar-search");
        let (view, mut cx) = open(cx, &root);
        write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("alpha"), &root.join("beta")]));
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("notes");
        wait_for_search(&view, &mut cx);
        cx.simulate_keystrokes("tab tab");
        assert!(sidebar_focused(&view, &mut cx));

        cx.simulate_keystrokes("down");
        settle(&view, &mut cx);
        assert!(view.read_with(&cx, |w, _| w.search.is_none()), "a sidebar move closes search");
        assert_eq!(dir(&view, &cx), root.join("beta"));
        assert!(sidebar_focused(&view, &mut cx), "focus stays in the sidebar");
    }

    #[gpui::test]
    fn sidebar_tab_in_does_not_navigate(cx: &mut TestAppContext) {
        let root = fixture("sidebar-tab");
        let (view, mut cx) = open(cx, &root);
        write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("alpha"), &root.join("beta")]));
        let before = dir(&view, &cx);
        cx.simulate_keystrokes("tab");
        assert_eq!(dir(&view, &cx), before);
    }

    #[gpui::test]
    fn option_arrows_jump_between_sidebar_groups(cx: &mut TestAppContext) {
        let root = fixture("groups");
        let (view, mut cx) = open(cx, &root);
        let toml = format!(
            "[[group]]\nname = \"One\"\nitems = [ {{ path = \"{}\" }} ]\n[[group]]\nname = \"Two\"\nitems = [ {{ path = \"{}\" }} ]\n[recent]\nshow = true\ncount = 6\n[locations]\nshow = false\n",
            root.join("alpha").display(),
            root.join("beta").display()
        );
        write_config(&view, &mut cx, "sidebar.toml", &toml);
        assert_eq!(
            sidebar_paths(&cx),
            [root.join("alpha"), root.join("beta"), root.clone()],
            "two groups, then Recent"
        );

        cx.simulate_keystrokes("tab alt-down");
        settle(&view, &mut cx);
        let sidebar = view.read_with(&cx, |w, _| w.sidebar.clone());
        assert_eq!(sidebar.read_with(&cx, |s, _| s.cursor()), 1, "the first row of the next group");
        assert_eq!(dir(&view, &cx), root.join("beta"));

        cx.simulate_keystrokes("alt-down");
        settle(&view, &mut cx);
        assert_eq!(sidebar.read_with(&cx, |s, _| s.cursor()), 2, "Recent is a group too");
        assert_eq!(dir(&view, &cx), root);

        cx.simulate_keystrokes("alt-up");
        settle(&view, &mut cx);
        assert_eq!(sidebar.read_with(&cx, |s, _| s.cursor()), 1);
        cx.simulate_keystrokes("alt-up");
        settle(&view, &mut cx);
        assert_eq!(sidebar.read_with(&cx, |s, _| s.cursor()), 0);
    }

    #[gpui::test]
    fn walking_recent_keeps_the_cursor_on_the_shown_folder(cx: &mut TestAppContext) {
        let root = fixture("recent-walk");
        let (view, mut cx) = open(cx, &root);
        write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[]).replace("show = false", "show = true"));
        // beta ranks above alpha, which ranks above root: a visit to alpha would tie it with beta.
        for d in ["alpha", "beta", "alpha", "beta", "beta"] {
            pane(&view, &cx).update(&mut cx, |pane, cx| pane.navigate(root.join(d), cx));
            settle(&view, &mut cx);
        }
        assert_eq!(sidebar_paths(&cx), [root.join("beta"), root.join("alpha"), root.clone()]);
        cx.simulate_keystrokes("tab");
        for (step, want) in [root.join("alpha"), root.clone()].into_iter().enumerate() {
            cx.simulate_keystrokes("down");
            settle(&view, &mut cx);
            assert_eq!(dir(&view, &cx), want);
            let looks = sidebar_looks(&view, &mut cx);
            assert!(
                looks[step + 1].ring && looks[step + 1].current,
                "Recent kept its order under the cursor: {looks:?}"
            );
        }
        assert_eq!(
            sidebar_paths(&cx),
            [root.join("beta"), root.join("alpha"), root.clone()],
            "the walk re-ranked nothing"
        );
    }

    #[gpui::test]
    fn a_sidebar_walk_counts_only_the_folder_you_leave_on(cx: &mut TestAppContext) {
        let root = fixture("walk-visits");
        std::fs::create_dir_all(root.join("gamma")).expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        write_config(
            &view,
            &mut cx,
            "sidebar.toml",
            &favorites_toml(&[&root.join("alpha"), &root.join("beta"), &root.join("gamma")]),
        );
        let visited = |cx: &mut VisualTestContext| cx.update(|_, cx| cx.global::<Ui>().palette_recents.clone());
        cx.simulate_keystrokes("tab down down");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("gamma"));
        let during = visited(&mut cx);
        assert!(!during.contains(&root.join("beta")) && !during.contains(&root.join("gamma")), "{during:?}");

        cx.simulate_keystrokes("tab");
        settle(&view, &mut cx);
        let after = visited(&mut cx);
        assert!(after.contains(&root.join("gamma")), "leaving the sidebar counts gamma: {after:?}");
        assert!(!after.contains(&root.join("beta")), "beta was only passed: {after:?}");
    }

    #[gpui::test]
    fn sidebar_arrow_walk_is_one_history_step(cx: &mut TestAppContext) {
        let root = fixture("sidebar-history");
        std::fs::create_dir_all(root.join("gamma")).expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        write_config(
            &view,
            &mut cx,
            "sidebar.toml",
            &favorites_toml(&[&root.join("alpha"), &root.join("beta"), &root.join("gamma")]),
        );
        cx.simulate_keystrokes("tab down down");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("gamma"));
        cx.simulate_keystrokes("cmd-[");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root, "Back returns to where the walk began");
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
    fn dropping_on_a_favorite_in_full_motion_pulses_it(cx: &mut TestAppContext) {
        let root = fixture("drop-favorite-full");
        let (view, mut cx) = open(cx, &root);
        set_motion(&mut cx, "full");
        write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("beta")]));
        let sidebar = view.read_with(&cx, |window, _| window.sidebar.clone());
        drop_on(&mut cx, "sidebar-0", vec![root.join("notes.txt")]);
        for _ in 0..200 {
            if sidebar.read_with(&cx, |sidebar, _| sidebar.receiving(&root.join("beta"))) {
                break;
            }
            cx.run_until_parked();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(sidebar.read_with(&cx, |sidebar, _| sidebar.receiving(&root.join("beta"))), "the favorite pulses");
        settle(&view, &mut cx);
        assert!(root.join("beta/notes.txt").exists());
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
    fn view_per_folder_off_keeps_the_window_view(cx: &mut TestAppContext) {
        let root = fixture("views-locked");
        let (view, mut cx) = open(cx, &root);
        let mode = |cx: &VisualTestContext| pane(&view, cx).read_with(cx, |p, _| p.mode());
        write_config(&view, &mut cx, "settings.toml", "view_per_folder = false\n");
        let views = format!("[folder.\"{}\"]\nview = \"columns\"\n", root.join("beta").display());
        write_config(&view, &mut cx, "views.toml", &views);

        pane(&view, &cx).update(&mut cx, |p, cx| p.navigate(root.join("beta"), cx));
        settle(&view, &mut cx);
        assert_eq!(mode(&cx), ViewMode::List, "the folder's own view is ignored");

        cx.simulate_keystrokes("cmd-3");
        settle(&view, &mut cx);
        assert_eq!(mode(&cx), ViewMode::Columns);
        let text = std::fs::read_to_string(config_dir(&cx).join("views.toml")).unwrap();
        assert_eq!(text, views, "Cmd-3 saves nothing");

        cx.simulate_keystrokes("cmd-[");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root);
        assert_eq!(mode(&cx), ViewMode::Columns, "going back keeps the window's view");

        cx.simulate_keystrokes("cmd-t");
        settle(&view, &mut cx);
        assert_eq!(mode(&cx), ViewMode::Columns, "a new tab copies its window");

        cx.simulate_keystrokes("cmd-2");
        settle(&view, &mut cx);
        write_config(&view, &mut cx, "settings.toml", "view_per_folder = true\n");
        pane(&view, &cx).update(&mut cx, |p, cx| p.navigate(root.join("beta"), cx));
        settle(&view, &mut cx);
        assert_eq!(mode(&cx), ViewMode::Columns, "turned back on, folders decide again");
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

    /// The bar's data for the current frame.
    fn bar_state(view: &Entity<CrowWindow>, cx: &mut VisualTestContext) -> BarState {
        let window = cx.update(|_, cx| cx.windows()[0]);
        cx.update_window(window, |_, window, cx| view.read(cx).bar_state(window, cx)).expect("read the bar")
    }

    fn primary_text(state: &BarState) -> String {
        match &state.primary {
            Primary::Pending(text) => (*text).to_owned(),
            Primary::Message(text) => text.to_string(),
            Primary::Running(running) => format!("{} — {}", running.label, running.detail),
            Primary::Hints(_) => String::new(),
        }
    }

    fn hint_labels(state: &BarState) -> Vec<String> {
        match &state.primary {
            Primary::Hints(hints) => hints.iter().map(|hint| hint.label.to_owned()).collect(),
            _ => Vec::new(),
        }
    }

    fn hint_keys(state: &BarState) -> Vec<String> {
        match &state.primary {
            Primary::Hints(hints) => hints.iter().map(|hint| hint.keys.to_string()).collect(),
            _ => Vec::new(),
        }
    }

    #[gpui::test]
    fn hints_follow_focus(cx: &mut TestAppContext) {
        let root = fixture("bar-hints-focus");
        let (view, mut cx) = open(cx, &root);
        write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("alpha"), &root.join("beta")]));

        let state = bar_state(&view, &mut cx);
        assert_eq!(hint_labels(&state), ["filter"], "⌘F and ⌘K live on the toolbar");
        assert_eq!(hint_keys(&state), ["/"]);

        cx.simulate_keystrokes("/");
        settle(&view, &mut cx);
        let state = bar_state(&view, &mut cx);
        assert_eq!(hint_labels(&state), ["open", "clear", "search deeper"]);
        assert_eq!(hint_keys(&state), ["↩", "esc", "⌘F"]);

        cx.simulate_keystrokes("escape");
        cx.simulate_keystrokes("tab");
        settle(&view, &mut cx);
        let state = bar_state(&view, &mut cx);
        assert_eq!(hint_labels(&state), ["reorder", "remove", "to list"]);
        assert_eq!(hint_keys(&state), ["⌥⌘↑↓", "⌘⌫", "⇥"]);

        cx.simulate_keystrokes("tab");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("a");
        cx.simulate_keystrokes("tab");
        settle(&view, &mut cx);
        let state = bar_state(&view, &mut cx);
        assert_eq!(hint_labels(&state), ["open", "reveal", "Quick Look"]);
        assert_eq!(hint_keys(&state), ["↩", "⌘R", "Space"]);
    }

    #[gpui::test]
    fn vim_layer_hints_use_the_vim_context(cx: &mut TestAppContext) {
        let root = fixture("bar-hints-vim");
        let (view, mut cx) = open(cx, &root);
        write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("alpha"), &root.join("beta")]));
        use_vim(&view, &mut cx);

        let state = bar_state(&view, &mut cx);
        assert_eq!(hint_labels(&state), ["filter"]);
        cx.simulate_keystrokes("tab");
        settle(&view, &mut cx);
        let state = bar_state(&view, &mut cx);
        assert_eq!(hint_labels(&state), ["reorder", "remove", "to list"]);

        cx.simulate_keystrokes("tab m");
        settle(&view, &mut cx);
        let state = bar_state(&view, &mut cx);
        assert!(matches!(state.primary, Primary::Pending(_)), "the mark prompt replaces hints");
        assert!(hint_labels(&state).is_empty());
    }

    #[gpui::test]
    fn idle_hints_retire_after_three_uses(cx: &mut TestAppContext) {
        let root = fixture("bar-hints-retire");
        let (view, mut cx) = open(cx, &root);
        for _ in 0..3 {
            cx.simulate_keystrokes("/");
            cx.simulate_keystrokes("escape");
            settle(&view, &mut cx);
        }
        let state = bar_state(&view, &mut cx);
        assert!(hint_labels(&state).is_empty(), "the used hint retired");

        cx.simulate_keystrokes("/");
        settle(&view, &mut cx);
        let state = bar_state(&view, &mut cx);
        assert_eq!(hint_labels(&state), ["open", "clear", "search deeper"], "focus hints never retire");
    }

    #[gpui::test]
    fn retired_hints_stay_retired_after_reload(cx: &mut TestAppContext) {
        let root = fixture("bar-hints-reload");
        let (view, mut cx) = open(cx, &root);
        for _ in 0..3 {
            cx.simulate_keystrokes("/");
            cx.simulate_keystrokes("escape");
            settle(&view, &mut cx);
        }
        cx.update(|_, cx| crate::usage::reload_from_disk(cx));
        write_config(&view, &mut cx, "settings.toml", "theme = \"system\"\n");
        let state = bar_state(&view, &mut cx);
        assert!(hint_labels(&state).is_empty(), "the count came back from usage.json");
    }

    #[gpui::test]
    fn hints_always_never_retire(cx: &mut TestAppContext) {
        let root = fixture("bar-hints-always");
        let (view, mut cx) = open(cx, &root);
        write_config(&view, &mut cx, "settings.toml", "hints = \"always\"\n");
        for _ in 0..3 {
            cx.simulate_keystrokes("/");
            cx.simulate_keystrokes("escape");
            settle(&view, &mut cx);
        }
        let state = bar_state(&view, &mut cx);
        assert_eq!(hint_labels(&state), ["filter"]);
    }

    #[gpui::test]
    fn hints_off_hides_every_hint(cx: &mut TestAppContext) {
        let root = fixture("bar-hints-off");
        let (view, mut cx) = open(cx, &root);
        write_config(&view, &mut cx, "settings.toml", "keymap = \"vim\"\nhints = \"off\"\n");
        let state = bar_state(&view, &mut cx);
        assert!(hint_labels(&state).is_empty());

        cx.simulate_keystrokes("m");
        settle(&view, &mut cx);
        let state = bar_state(&view, &mut cx);
        assert!(matches!(state.primary, Primary::Pending(_)), "the mark prompt is not a hint");
    }

    #[gpui::test]
    fn error_drops_hints(cx: &mut TestAppContext) {
        let root = fixture("bar-hints-error");
        let (view, mut cx) = open(cx, &root);
        write_config(&view, &mut cx, "views.toml", "[defaults]\nsort = \"colour\"\n");
        let state = bar_state(&view, &mut cx);
        assert!(hint_labels(&state).is_empty(), "no hints beside the error token");
        assert!(state.error.is_some());
    }

    #[gpui::test]
    fn overlay_hides_hints(cx: &mut TestAppContext) {
        let root = fixture("bar-hints-overlay");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-k");
        settle(&view, &mut cx);
        assert!(hint_labels(&bar_state(&view, &mut cx)).is_empty());
    }

    /// Install a running operation and hand back its cancel flag.
    fn inject_running(cx: &mut VisualTestContext) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let handle = cancel.clone();
        cx.update(|_, cx| {
            cx.global_mut::<Operations>().running = Some(crate::operations::Running {
                label: "Move 2 items to Trash".into(),
                progress: crow_fs::ops::Progress {
                    done_items: 1,
                    total_items: 2,
                    total_bytes: 0,
                    ..Default::default()
                },
                cancel: handle,
                started: std::time::Instant::now(),
            });
            cx.refresh_windows();
        });
        cx.run_until_parked();
        cancel
    }

    #[gpui::test]
    fn cancel_button_sets_the_cancel_flag(cx: &mut TestAppContext) {
        let root = fixture("bar-cancel-click");
        let (_view, mut cx) = open(cx, &root);
        let cancel = inject_running(&mut cx);
        let bounds = cx.debug_bounds("bar-cancel").expect("the Cancel button is laid out");
        let progress = cx.debug_bounds("bar-progress").expect("the progress line is laid out");
        assert!(progress.size.width > gpui::px(0.), "some progress is drawn");
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        assert!(cancel.load(std::sync::atomic::Ordering::Relaxed), "the click asked the operation to stop");

        cx.update(|_, cx| cx.global_mut::<Operations>().running = None);
    }

    #[gpui::test]
    fn escape_cancels_from_the_list(cx: &mut TestAppContext) {
        let root = fixture("bar-cancel-escape");
        let (_view, mut cx) = open(cx, &root);
        let cancel = inject_running(&mut cx);
        cx.simulate_keystrokes("escape");
        assert!(cancel.load(std::sync::atomic::Ordering::Relaxed));

        cx.update(|_, cx| cx.global_mut::<Operations>().running = None);
    }

    #[gpui::test]
    fn config_error_token_opens_the_broken_file(cx: &mut TestAppContext) {
        let root = fixture("bar-error-open");
        let (view, mut cx) = open(cx, &root);
        crate::bottom_bar::take_opened();
        write_config(&view, &mut cx, "sidebar.toml", "[[group]\n");

        let state = bar_state(&view, &mut cx);
        let error = state.error.expect("the broken sidebar shows a token");
        assert!(error.text.starts_with("sidebar.toml"), "{}", error.text);
        assert!(error.text.contains("line "), "the parser's line is shown: {}", error.text);
        let file = error.file.expect("a config token can be opened");
        assert!(file.ends_with("sidebar.toml"), "{}", file.display());

        let bounds = cx.debug_bounds("bar-error-open").expect("the Open button is laid out");
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        assert_eq!(crate::bottom_bar::take_opened(), [file]);
    }

    #[gpui::test]
    fn fixing_the_file_clears_the_token(cx: &mut TestAppContext) {
        let root = fixture("bar-error-fixed");
        let (view, mut cx) = open(cx, &root);
        write_config(&view, &mut cx, "sidebar.toml", "[[group]\n");
        assert!(bar_state(&view, &mut cx).error.is_some());

        write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[]));
        let state = bar_state(&view, &mut cx);
        assert!(state.error.is_none(), "fixing the file removes the token");
        assert_eq!(hint_labels(&state), ["filter"], "hints come back");
    }

    #[gpui::test]
    fn pane_error_has_no_open_button(cx: &mut TestAppContext) {
        use std::os::unix::fs::PermissionsExt;
        let root = fixture("bar-error-pane");
        let locked = root.join("locked");
        std::fs::create_dir_all(&locked).expect("create folder");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).expect("lock folder");
        let (view, mut cx) = open(cx, &root);
        pane(&view, &cx).update(&mut cx, |pane, cx| pane.navigate(locked.clone(), cx));
        settle(&view, &mut cx);

        let state = bar_state(&view, &mut cx);
        let error = state.error.expect("the pane's error shows");
        assert!(error.file.is_none(), "a folder error has nothing to open");
        assert!(cx.debug_bounds("bar-error-open").is_none());

        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).expect("unlock folder");
    }

    #[gpui::test]
    fn narrow_window_drops_hints_before_the_path(cx: &mut TestAppContext) {
        let root = fixture("bar-narrow");
        let (view, mut cx) = open(cx, &root);
        let last_segment =
            |state: &BarState| -> &'static str { format!("path-segment-{}", state.path.parts.len() - 1).leak() };

        // Shrink step by step: the middle folds first, then the hint goes; the first and
        // last segments stay on screen throughout.
        let (mut folded_with_hint, mut hint_gone) = (false, false);
        for width in (380..=1200).rev().step_by(20) {
            cx.simulate_resize(gpui::size(px(width as f32), px(500.)));
            cx.run_until_parked();
            let state = bar_state(&view, &mut cx);
            let hints = hint_labels(&state);
            folded_with_hint |= state.path.hidden.is_some() && !hints.is_empty();
            if hints.is_empty() {
                hint_gone = true;
                assert!(state.path.hidden.is_some(), "at {width}: the hint only goes once the path has folded");
            } else {
                assert!(!hint_gone, "at {width}: a dropped hint stays dropped while narrowing");
            }
            assert!(cx.debug_bounds("path-segment-0").is_some(), "at {width}: the first segment stays");
            let tail = cx.debug_bounds(last_segment(&state)).expect("the last segment is drawn");
            assert!(tail.right() <= px(width as f32), "at {width}: the last segment is on screen: {tail:?}");
        }
        assert!(folded_with_hint, "the path folds before the hint is dropped");
        assert!(hint_gone, "a narrow window drops the hint");
        assert!(cx.debug_bounds("path-folded").is_some(), "the fold is drawn as …");

        cx.simulate_resize(gpui::size(px(3000.), px(800.)));
        cx.run_until_parked();
        let state = bar_state(&view, &mut cx);
        assert_eq!(hint_labels(&state), ["filter"], "wide again shows the hint");
        assert_eq!(state.path.hidden, None, "and the whole path");
    }

    #[gpui::test]
    fn error_and_progress_leave_the_path_its_ends(cx: &mut TestAppContext) {
        let root = fixture("bar-squeeze");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_resize(gpui::size(px(700.), px(500.)));
        cx.update(|_, cx| {
            cx.global_mut::<Operations>().running = Some(crate::operations::Running {
                label: "Copy 12 items to a folder with a rather long name indeed".into(),
                progress: crow_fs::ops::Progress { done_bytes: 1, total_bytes: 4, ..Default::default() },
                cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                started: std::time::Instant::now(),
            });
        });
        let dir = config_dir(&cx);
        std::fs::write(dir.join("views.toml"), "[defaults]\nsort = \"a colour that does not exist at all\"\n")
            .expect("break views.toml");
        cx.update(|_, cx| crate::ui::reload_config(cx));
        cx.run_until_parked();
        let state = bar_state(&view, &mut cx);
        let max = state.transient_max.expect("the transient slot is capped");
        assert!(max < px(700.), "{max:?}");
        let last: &'static str = format!("path-segment-{}", state.path.parts.len() - 1).leak();
        let first = cx.debug_bounds("path-segment-0").expect("the first segment is drawn");
        let tail = cx.debug_bounds(last).expect("the last segment is drawn");
        assert!(first.size.width > px(20.), "the first segment keeps its room: {first:?}");
        assert!(tail.size.width > px(20.), "the last segment keeps its room: {tail:?}");
        let counts = cx.debug_bounds("bar-cancel").expect("Cancel is drawn");
        assert!(counts.right() <= px(700.), "the transient stays inside the window: {counts:?}");
        cx.update(|_, cx| cx.global_mut::<Operations>().running = None);
    }

    #[gpui::test]
    fn counts_name_the_focused_folder(cx: &mut TestAppContext) {
        let root = fixture("bar-counts-columns");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-3 right");
        settle(&view, &mut cx);
        let state = bar_state(&view, &mut cx);
        assert_eq!(state.count.text.as_ref(), "1 item");
        assert_eq!(state.count.spoken.as_ref(), "1 item in alpha", "the focused column's folder");
    }

    #[gpui::test]
    fn counts_show_filter_and_selection(cx: &mut TestAppContext) {
        let root = fixture("bar-counts-filter");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("/");
        cx.simulate_input("no");
        settle(&view, &mut cx);
        let state = bar_state(&view, &mut cx);
        assert_eq!(state.count.text.as_ref(), "3 items", "the field says \"1 of 3\"; the bar keeps the folder's count");

        cx.simulate_keystrokes("escape");
        cx.simulate_keystrokes("cmd-a");
        settle(&view, &mut cx);
        let state = bar_state(&view, &mut cx);
        assert_eq!(state.count.text.as_ref(), "3 selected · 6 bytes", "folders add no bytes");
        let folder = crate::style::display_name(&root);
        assert_eq!(state.count.spoken.as_ref(), format!("3 of 3 selected in {folder}, 6 bytes"));
    }

    #[gpui::test]
    fn one_item_is_singular(cx: &mut TestAppContext) {
        let root = fixture("bar-singular");
        std::fs::create_dir_all(root.join("solo")).expect("create folder");
        std::fs::write(root.join("solo/only.txt"), "").expect("create file");
        let (view, mut cx) = open(cx, &root.join("solo"));
        let state = bar_state(&view, &mut cx);
        assert_eq!(state.count.text.as_ref(), "1 item");
        assert_eq!(state.count.spoken.as_ref(), "1 item in solo");
    }

    #[gpui::test]
    fn message_does_not_hide_the_count(cx: &mut TestAppContext) {
        let root = fixture("bar-message");
        let (view, mut cx) = open(cx, &root);
        cx.update(|_, cx| {
            operations::message("Copied 1 item", false, cx);
        });
        cx.simulate_keystrokes("down");
        let state = bar_state(&view, &mut cx);
        assert_eq!(primary_text(&state), "Copied 1 item");
        assert_eq!(state.count.text.as_ref(), "3 items");
    }

    #[gpui::test]
    fn info_messages_expire_but_errors_stay(cx: &mut TestAppContext) {
        let root = fixture("bar-message-expire");
        let (_view, mut cx) = open(cx, &root);
        cx.update(|_, cx| operations::message("Copied 1 item", false, cx));
        let executor = cx.executor();
        executor.advance_clock(std::time::Duration::from_secs(6));
        cx.run_until_parked();
        assert!(cx.read(|cx| cx.global::<Operations>().message.is_none()), "an info message retires");

        cx.update(|_, cx| operations::message("Can't open x", true, cx));
        executor.advance_clock(std::time::Duration::from_secs(6));
        cx.run_until_parked();
        assert!(cx.read(|cx| cx.global::<Operations>().message.is_some()), "an error waits for something else");
    }

    #[gpui::test]
    fn copy_and_missing_mark_messages_retire(cx: &mut TestAppContext) {
        let root = fixture("bar-message-copy");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-c");
        settle(&view, &mut cx);
        let state = bar_state(&view, &mut cx);
        assert!(primary_text(&state).starts_with("Copied"), "{}", primary_text(&state));
        let executor = cx.executor();
        executor.advance_clock(std::time::Duration::from_secs(6));
        cx.run_until_parked();
        assert!(cx.read(|cx| cx.global::<Operations>().message.is_none()), "the copy message retires");

        use_vim(&view, &mut cx);
        cx.simulate_keystrokes("' q");
        settle(&view, &mut cx);
        let state = bar_state(&view, &mut cx);
        assert_eq!(primary_text(&state), "No mark q");
        assert!(state.error.is_none(), "a missing mark is not an error token");
        executor.advance_clock(std::time::Duration::from_secs(6));
        cx.run_until_parked();
        assert!(cx.read(|cx| cx.global::<Operations>().message.is_none()), "and it retires too");
    }

    #[gpui::test]
    fn option_arrows_move_by_word_in_text_fields(cx: &mut TestAppContext) {
        let root = fixture("text-word-moves");
        let (view, mut cx) = open(cx, &root);
        let filter =
            |cx: &VisualTestContext| pane(&view, cx).read_with(cx, |p, cx| p.filter_text(cx)).unwrap_or_default();
        cx.simulate_keystrokes("/");
        cx.simulate_input("m5-fix.md");
        cx.simulate_keystrokes("alt-left alt-left");
        cx.simulate_input("X");
        assert_eq!(filter(&cx), "m5-Xfix.md", "⌥← twice lands before \"fix\"");

        cx.simulate_keystrokes("alt-right");
        cx.simulate_input("Y");
        assert_eq!(filter(&cx), "m5-XfixY.md", "⌥→ goes to the end of the word");

        cx.simulate_keystrokes("alt-backspace");
        assert_eq!(filter(&cx), "m5-.md", "⌥⌫ deletes back to the word start");

        cx.simulate_keystrokes("cmd-left alt-shift-right");
        cx.simulate_input("z");
        assert_eq!(filter(&cx), "z-.md", "⌥⇧→ selects the word");
    }

    #[gpui::test]
    fn caret_blinks_in_text_fields(cx: &mut TestAppContext) {
        let root = fixture("caret-blink");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-k");
        settle(&view, &mut cx);
        let input = view.read_with(&cx, |w, _| w.palette.as_ref().expect("the palette is open").input.clone());
        assert!(input.read_with(&cx, |input, _| input.caret_visible()), "visible while the field has focus");

        let executor = cx.executor();
        executor.advance_clock(std::time::Duration::from_millis(600));
        cx.run_until_parked();
        assert!(!input.read_with(&cx, |input, _| input.caret_visible()), "the caret goes off after 500 ms");
        executor.advance_clock(std::time::Duration::from_millis(600));
        cx.run_until_parked();
        assert!(input.read_with(&cx, |input, _| input.caret_visible()), "and back on after another 500 ms");

        cx.simulate_input("a");
        assert!(input.read_with(&cx, |input, _| input.caret_visible()), "typing shows the caret at once");

        cx.simulate_keystrokes("escape");
        settle(&view, &mut cx);
        executor.advance_clock(std::time::Duration::from_millis(600));
        cx.run_until_parked();
        assert!(!input.read_with(&cx, |input, _| input.caret_visible()), "no caret without focus");
    }

    #[gpui::test]
    fn navigating_clears_an_operation_error(cx: &mut TestAppContext) {
        let root = fixture("bar-message-nav");
        let (view, mut cx) = open(cx, &root);
        cx.update(|_, cx| operations::message("Can't open x", true, cx));
        cx.simulate_keystrokes("cmd-down");
        settle(&view, &mut cx);
        assert!(cx.read(|cx| cx.global::<Operations>().message.is_none()), "navigation drops the old message");
    }

    #[gpui::test]
    fn error_and_running_operation_show_together(cx: &mut TestAppContext) {
        let root = fixture("bar-error-running");
        let (view, mut cx) = open(cx, &root);
        cx.update(|_, cx| {
            cx.global_mut::<Operations>().running = Some(crate::operations::Running {
                label: "Copy 1 item to x".into(),
                progress: crow_fs::ops::Progress { total_items: 1, total_bytes: 1, ..Default::default() },
                cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                started: std::time::Instant::now(),
            });
        });
        let dir = config_dir(&cx);
        std::fs::write(dir.join("views.toml"), "[defaults]\nsort = \"colour\"\n").expect("break views.toml");
        cx.update(|_, cx| crate::ui::reload_config(cx));
        cx.run_until_parked();

        let state = bar_state(&view, &mut cx);
        assert!(matches!(state.primary, Primary::Running(_)), "progress stays primary");
        let error = state.error.expect("the config error is its own token");
        assert!(error.file.as_ref().is_some_and(|f| f.ends_with("views.toml")));
        assert!(error.text.starts_with("views.toml"), "{}", error.text);

        cx.update(|_, cx| cx.global_mut::<Operations>().running = None);
        settle(&view, &mut cx);
    }

    /// The in-memory count for an action name.
    fn key_uses(cx: &VisualTestContext, key: &str) -> u32 {
        cx.read(|cx| crate::usage::count_key(key, cx))
    }

    /// Wait for the background save and the count it wrote to `usage.json`.
    fn saved_uses(cx: &mut VisualTestContext, key: &str, expected: u32) -> u32 {
        let path = cx.read(|cx| cx.global::<Ui>().state_dir.join(crow_core::USAGE_FILE));
        let mut saved = 0;
        for _ in 0..200 {
            cx.run_until_parked();
            saved = crow_core::Usage::load(&path).count(key);
            if saved == expected {
                return saved;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        saved
    }

    #[gpui::test]
    fn pressing_slash_counts_filter_uses(cx: &mut TestAppContext) {
        let root = fixture("usage-filter");
        let (view, mut cx) = open(cx, &root);
        for _ in 0..3 {
            cx.simulate_keystrokes("/");
            cx.simulate_keystrokes("escape");
            settle(&view, &mut cx);
        }
        let key = StartFilter.name();
        assert_eq!(key_uses(&cx, key), 3, "each / counted once");
        assert_eq!(saved_uses(&mut cx, key, 3), 3, "and survived to disk");
    }

    #[gpui::test]
    fn palette_and_dispatch_count_too(cx: &mut TestAppContext) {
        let root = fixture("usage-palette");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input(">filter");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        assert!(pane(&view, &cx).read_with(&cx, |p, _| p.is_filtering()), "the palette opened the filter");
        cx.simulate_keystrokes("escape");

        cx.dispatch_action(ShowSearch);
        settle(&view, &mut cx);
        cx.simulate_keystrokes("escape");

        assert_eq!(key_uses(&cx, ShowCommandPalette.name()), 1, "Cmd-K counted");
        assert_eq!(key_uses(&cx, StartFilter.name()), 1, "the palette run counted");
        assert_eq!(key_uses(&cx, ShowSearch.name()), 1, "a dispatched action counted");
    }

    #[gpui::test]
    fn refocusing_an_open_filter_does_not_count(cx: &mut TestAppContext) {
        let root = fixture("usage-refocus");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("/");
        settle(&view, &mut cx);
        let before = key_uses(&cx, StartFilter.name());
        assert_eq!(before, 1);
        cx.dispatch_action(StartFilter);
        settle(&view, &mut cx);
        assert_eq!(key_uses(&cx, StartFilter.name()), before, "focusing the open field is not a new use");
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

    /// Wait until the search preview for `path` has loaded.
    fn wait_for_preview(view: &Entity<CrowWindow>, cx: &mut VisualTestContext, path: &Path) {
        for _ in 0..400 {
            cx.run_until_parked();
            let loaded =
                view.read_with(cx, |w, cx| w.search.as_ref().and_then(|search| search.read(cx).preview_path()));
            if loaded.as_deref() == Some(path) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("preview for {} never loaded", path.display());
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
    fn search_scope_buttons_switch_scope(cx: &mut TestAppContext) {
        let root = fixture("search-scope");
        std::fs::write(root.join("notes.txt"), "needle\n").expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        settle(&view, &mut cx);
        let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");

        // Empty query: switching scope starts no walk, Home or Spotlight.
        click_selector(&mut cx, "search-scope-home");
        settle(&view, &mut cx);
        assert_eq!(search.read_with(&cx, |search, _| search.scope()), crate::search::Scope::Home);

        click_selector(&mut cx, "search-scope-this-mac");
        settle(&view, &mut cx);
        assert_eq!(search.read_with(&cx, |search, _| search.scope()), crate::search::Scope::ThisMac);

        click_selector(&mut cx, "search-scope-folder");
        settle(&view, &mut cx);
        assert_eq!(search.read_with(&cx, |search, _| search.scope()), crate::search::Scope::Folder);

        cx.simulate_input("notes");
        wait_for_search(&view, &mut cx);
        let paths = search.read_with(&cx, |search, _| search.result_paths());
        assert!(paths.contains(&root.join("notes.txt")), "{paths:?}");
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
    fn search_row_aria_reads_line(cx: &mut TestAppContext) {
        let root = fixture("search-aria");
        std::fs::write(root.join("other.txt"), "needle in beta\n").expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("needle");
        wait_for_search(&view, &mut cx);
        let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
        let paths = search.read_with(&cx, |search, _| search.result_paths());
        let index = paths.iter().position(|path| path.ends_with("other.txt")).expect("content hit");
        let label = search.read_with(&cx, |search, _| search.row_label(index)).expect("a row");
        assert!(label.starts_with("other.txt, in "), "{label}");
        assert!(label.ends_with(", line 1: needle in beta"), "{label}");
    }

    #[gpui::test]
    fn search_preview_follows_the_selection(cx: &mut TestAppContext) {
        let root = fixture("search-preview");
        std::fs::write(root.join("a-notes.txt"), "alpha\n").expect("create fixture");
        std::fs::write(root.join("b-notes.txt"), "beta\n").expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("notes");
        wait_for_search(&view, &mut cx);
        let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
        let paths = search.read_with(&cx, |search, _| search.result_paths());
        assert!(paths.len() >= 2, "{paths:?}");
        wait_for_preview(&view, &mut cx, &paths[0]);

        cx.simulate_keystrokes("tab");
        cx.simulate_keystrokes("down");
        wait_for_preview(&view, &mut cx, &paths[1]);
        assert_eq!(search.read_with(&cx, |search, _| search.cursor()), 1);
    }

    #[gpui::test]
    fn search_preview_shows_in_column_view(cx: &mut TestAppContext) {
        let root = fixture("search-preview-columns");
        std::fs::write(root.join("notes.txt"), "hello\n").expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-3");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("notes");
        wait_for_search(&view, &mut cx);
        let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
        let path = search.read_with(&cx, |search, _| search.result_paths()).first().cloned().expect("a result");
        wait_for_preview(&view, &mut cx, &path);
        settle(&view, &mut cx);
        assert!(cx.debug_bounds("preview").is_some(), "the preview pane is on screen");
    }

    #[gpui::test]
    fn search_preview_shows_the_hit_line(cx: &mut TestAppContext) {
        let root = fixture("search-preview-hit");
        let mut body = String::new();
        for n in 1..=100 {
            body.push_str(if n == 80 { "needle on line 80\n" } else { "filler\n" });
        }
        std::fs::write(root.join("deep.txt"), body).expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("needle");
        wait_for_search(&view, &mut cx);
        let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
        wait_for_preview(&view, &mut cx, &root.join("deep.txt"));
        let (hit, text) = search.read_with(&cx, |search, _| search.preview_excerpt()).expect("an excerpt");
        assert_eq!(hit, 80);
        assert!(text.lines().any(|line| line == "needle on line 80"), "{text}");
    }

    #[gpui::test]
    fn search_results_vim_j_k_move_the_cursor(cx: &mut TestAppContext) {
        let root = fixture("search-vim-results");
        std::fs::write(root.join("notes-a.txt"), "alpha\n").expect("create fixture");
        std::fs::write(root.join("notes-b.txt"), "beta\n").expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        use_vim(&view, &mut cx);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("notes");
        wait_for_search(&view, &mut cx);
        let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
        assert!(search.read_with(&cx, |search, _| search.result_paths()).len() >= 2);
        cx.simulate_keystrokes("tab");
        assert_eq!(search.read_with(&cx, |search, _| search.cursor()), 0);
        cx.simulate_keystrokes("j");
        assert_eq!(search.read_with(&cx, |search, _| search.cursor()), 1);
        cx.simulate_keystrokes("k");
        assert_eq!(search.read_with(&cx, |search, _| search.cursor()), 0);
    }

    #[gpui::test]
    fn search_name_hits_outrank_content_hits(cx: &mut TestAppContext) {
        let root = fixture("search-order");
        std::fs::write(root.join("m5-notes.md"), "nothing\n").expect("create fixture");
        std::fs::write(root.join("deep.txt"), "m5 lives here\n").expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("m5");
        wait_for_search(&view, &mut cx);
        let paths = view.read_with(&cx, |w, cx| w.search.as_ref().expect("search").read(cx).result_paths());
        assert_eq!(paths.first().and_then(|path| path.file_name()), Some("m5-notes.md".as_ref()), "{paths:?}");
        assert!(paths.iter().any(|path| path.ends_with("deep.txt")), "{paths:?}");
    }

    #[gpui::test]
    fn search_selects_the_best_first_row_until_the_user_moves(cx: &mut TestAppContext) {
        let root = fixture("search-selection");
        // Name hits live outside the walked root, so the real "qqzz" search finds nothing.
        let outside = root.parent().expect("temp parent").join(format!("crow-qqzz-{}", std::process::id()));
        std::fs::create_dir_all(&outside).expect("create fixture");
        let name_path = outside.join("qqzz-zebra.txt");
        std::fs::write(&name_path, "nothing\n").expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        settle(&view, &mut cx);
        let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
        cx.simulate_input("qqzz");
        wait_for_search(&view, &mut cx);
        let hit =
            |path: PathBuf, kind: HitKind, score| Hit { path, is_dir: false, score, kind, modified: None, size: 0 };

        // The walker found a content hit first; the name hit arrives in a later batch.
        let content = hit(
            root.join("notes.txt"),
            HitKind::Content { line: 1, text: "qqzz lives here".to_owned(), ranges: test_ranges(0, 4), more: 0 },
            0,
        );
        search.update(&mut cx, |search, cx| search.add_test_hits(vec![content], cx));
        assert_eq!(
            search.read_with(&cx, |search, _| search.selected_entry()).map(|(path, _)| path),
            Some(root.join("notes.txt"))
        );

        let name = hit(name_path.clone(), HitKind::Name { ranges: test_ranges(0, 4) }, 100);
        search.update(&mut cx, |search, cx| search.add_test_hits(vec![name], cx));
        wait_for_preview(&view, &mut cx, &name_path);
        assert_eq!(search.read_with(&cx, |search, _| search.cursor()), 0);
        assert_eq!(search.read_with(&cx, |search, _| search.selected_entry()).map(|(path, _)| path), Some(name_path));
    }

    #[gpui::test]
    fn search_keeps_the_users_row_when_better_rows_arrive(cx: &mut TestAppContext) {
        let root = fixture("search-selection-moved");
        let outside = root.parent().expect("temp parent").join(format!("crow-qqzz-moved-{}", std::process::id()));
        std::fs::create_dir_all(&outside).expect("create fixture");
        let first_path = outside.join("qqzz-zebra.txt");
        let better_path = outside.join("qqzz-better.txt");
        std::fs::write(&first_path, "nothing\n").expect("create fixture");
        std::fs::write(&better_path, "nothing\n").expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        settle(&view, &mut cx);
        let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
        cx.simulate_input("qqzz");
        wait_for_search(&view, &mut cx);
        let hit =
            |path: PathBuf, kind: HitKind, score| Hit { path, is_dir: false, score, kind, modified: None, size: 0 };
        let first = hit(first_path, HitKind::Name { ranges: test_ranges(0, 4) }, 100);
        let content = hit(
            root.join("notes.txt"),
            HitKind::Content { line: 1, text: "qqzz lives here".to_owned(), ranges: test_ranges(0, 4), more: 0 },
            0,
        );
        search.update(&mut cx, |search, cx| search.add_test_hits(vec![content, first], cx));
        settle(&view, &mut cx);

        cx.simulate_keystrokes("tab");
        cx.simulate_keystrokes("down");
        assert_eq!(search.read_with(&cx, |search, _| search.cursor()), 1);
        assert_eq!(
            search.read_with(&cx, |search, _| search.selected_entry()).map(|(path, _)| path),
            Some(root.join("notes.txt"))
        );

        let better = hit(better_path, HitKind::Name { ranges: test_ranges(0, 4) }, 200);
        search.update(&mut cx, |search, cx| search.add_test_hits(vec![better], cx));
        assert_eq!(search.read_with(&cx, |search, _| search.cursor()), 2, "the user's row moved down");
        assert_eq!(
            search.read_with(&cx, |search, _| search.selected_entry()).map(|(path, _)| path),
            Some(root.join("notes.txt"))
        );
    }

    #[gpui::test]
    fn search_contiguous_name_hits_come_before_contents_and_weak_names(cx: &mut TestAppContext) {
        let root = fixture("search-weak-names");
        let outside = root.parent().expect("temp parent").join(format!("crow-qqzz-order-{}", std::process::id()));
        std::fs::create_dir_all(&outside).expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        settle(&view, &mut cx);
        let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
        cx.simulate_input("qqzz");
        wait_for_search(&view, &mut cx);
        let strong = outside.join("qqzz-zebra.txt");
        let weak = outside.join("q-a-q-b-z-c-z.txt");
        let content = root.join("notes.txt");
        let hit =
            |path: PathBuf, kind: HitKind, score| Hit { path, is_dir: false, score, kind, modified: None, size: 0 };
        search.update(&mut cx, |search, cx| {
            search.add_test_hits(
                vec![
                    hit(
                        content.clone(),
                        HitKind::Content {
                            line: 1,
                            text: "qqzz lives here".to_owned(),
                            ranges: test_ranges(0, 4),
                            more: 0,
                        },
                        0,
                    ),
                    hit(weak.clone(), HitKind::Name { ranges: vec![0..1, 2..3, 4..6, 7..8] }, 80),
                    hit(strong.clone(), HitKind::Name { ranges: test_ranges(0, 4) }, 100),
                ],
                cx,
            );
        });
        let paths = search.read_with(&cx, |search, _| search.result_paths());
        assert_eq!(paths, [strong, content, weak]);
    }

    #[gpui::test]
    fn search_footer_counts_hits_and_files(cx: &mut TestAppContext) {
        let root = fixture("search-footer");
        std::fs::write(root.join("a.txt"), "needle\nx\nneedle\nneedle\n").expect("create fixture");
        std::fs::write(root.join("b.txt"), "needle\n").expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("needle");
        wait_for_search(&view, &mut cx);
        let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
        assert_eq!(search.read_with(&cx, |search, _| search.hit_count()), 4);
        assert_eq!(search.read_with(&cx, |search, _| search.result_paths().len()), 2);
        assert_eq!(search.read_with(&cx, |search, _| search.footer_summary_text()), "4 results in 2 files");
        assert!(cx.debug_bounds("search-footer").is_some());
    }

    #[gpui::test]
    fn search_skipped_link_opens_the_list_and_escape_closes_it_first(cx: &mut TestAppContext) {
        let root = fixture("search-skipped");
        std::fs::create_dir_all(root.join("node_modules/pkg")).expect("create fixture");
        std::fs::write(root.join("node_modules/pkg/needle.txt"), "needle\n").expect("create fixture");
        std::fs::write(root.join("outside.txt"), "needle\n").expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("needle");
        wait_for_search(&view, &mut cx);
        let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
        assert!(search.read_with(&cx, |search, _| search.skipped_count()) > 0);

        click_selector(&mut cx, "search-skipped-link");
        settle(&view, &mut cx);
        assert!(search.read_with(&cx, |search, _| search.skipped_open()));
        assert!(cx.debug_bounds("search-skipped-row-0").is_some());

        cx.simulate_keystrokes("escape");
        settle(&view, &mut cx);
        assert!(view.read_with(&cx, |w, _| w.search.is_some()), "Esc closed the list, not search");
        assert!(!search.read_with(&cx, |search, _| search.skipped_open()));

        click_selector(&mut cx, "search-skipped-link");
        settle(&view, &mut cx);
        click_selector(&mut cx, "search-skipped-close");
        settle(&view, &mut cx);
        assert!(!search.read_with(&cx, |search, _| search.skipped_open()));
        cx.simulate_keystrokes("escape");
        settle(&view, &mut cx);
        assert!(view.read_with(&cx, |w, _| w.search.is_none()), "the next Esc closes search");
    }

    #[gpui::test]
    fn search_invalid_regex_shows_the_error(cx: &mut TestAppContext) {
        let root = fixture("search-error");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("/(/");
        wait_for_search(&view, &mut cx);
        let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
        let error = search.read_with(&cx, |search, _| search.error_text()).expect("an error");
        assert_eq!(error, "Invalid pattern: unclosed group");
        assert!(!error.contains('\n'), "{error}");
        assert!(search.read_with(&cx, |search, _| search.result_paths().is_empty()));
        let footer = search.read_with(&cx, |search, _| search.footer_summary_text());
        assert_eq!(footer, "Invalid pattern: unclosed group");
    }

    #[gpui::test]
    fn search_switch_flips_rules(cx: &mut TestAppContext) {
        let root = fixture("search-switch");
        std::fs::create_dir_all(root.join("node_modules/pkg")).expect("create fixture");
        std::fs::write(root.join("node_modules/pkg/needle.txt"), "needle\n").expect("create fixture");
        std::fs::write(root.join("outside.txt"), "needle\n").expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("needle");
        wait_for_search(&view, &mut cx);
        let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
        assert!(!search.read_with(&cx, |search, _| search.everything()));

        click_selector(&mut cx, "search-skip-switch");
        wait_for_search(&view, &mut cx);
        assert!(search.read_with(&cx, |search, _| search.everything()));
        let paths = search.read_with(&cx, |search, _| search.result_paths());
        assert!(paths.contains(&root.join("node_modules/pkg/needle.txt")), "{paths:?}");

        click_selector(&mut cx, "search-skip-switch");
        wait_for_search(&view, &mut cx);
        assert!(!search.read_with(&cx, |search, _| search.everything()));
        let paths = search.read_with(&cx, |search, _| search.result_paths());
        assert!(!paths.iter().any(|path| path.ends_with("node_modules/pkg/needle.txt")), "{paths:?}");
    }

    #[gpui::test]
    fn search_switch_hidden_for_this_mac(cx: &mut TestAppContext) {
        let root = fixture("search-switch-spotlight");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        settle(&view, &mut cx);
        assert!(cx.debug_bounds("search-skip-switch").is_some());
        click_selector(&mut cx, "search-scope-this-mac");
        settle(&view, &mut cx);
        assert!(cx.debug_bounds("search-skip-switch").is_none());
    }

    #[gpui::test]
    fn search_shift_cmd_a_from_toolbar_field_flips_rules(cx: &mut TestAppContext) {
        let root = fixture("search-switch-key");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        focus_search_input(&view, &mut cx);
        cx.simulate_keystrokes("cmd-shift-a");
        let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
        assert!(search.read_with(&cx, |search, _| search.everything()));
    }

    #[gpui::test]
    fn search_opened_with_everything_shows_switch_off(cx: &mut TestAppContext) {
        let root = fixture("search-open-everything");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-shift-a");
        settle(&view, &mut cx);
        let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
        assert!(search.read_with(&cx, |search, _| search.everything()));
        assert!(cx.debug_bounds("search-skip-switch").is_some());
    }

    #[gpui::test]
    fn search_shift_cmd_m_cycles_the_match_mode(cx: &mut TestAppContext) {
        let root = fixture("search-cycle");
        std::fs::write(root.join("needle.txt"), "needle\n").expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        focus_search_input(&view, &mut cx);
        cx.simulate_input("needle");
        wait_for_search(&view, &mut cx);
        let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
        focus_search_input(&view, &mut cx);

        cx.simulate_keystrokes("cmd-shift-m");
        assert_eq!(search.read_with(&cx, |search, _| search.matching()), crate::search::Match::Names);
        focus_search_input(&view, &mut cx);
        cx.simulate_keystrokes("cmd-shift-m");
        assert_eq!(search.read_with(&cx, |search, _| search.matching()), crate::search::Match::Contents);
        focus_search_input(&view, &mut cx);
        cx.simulate_keystrokes("cmd-shift-m");
        assert_eq!(search.read_with(&cx, |search, _| search.matching()), crate::search::Match::All);
    }

    #[gpui::test]
    fn search_idle_example_fills_the_query(cx: &mut TestAppContext) {
        let root = fixture("search-idle");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        settle(&view, &mut cx);
        click_selector(&mut cx, "search-example-0");
        let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
        assert_eq!(search.read_with(&cx, |search, cx| search.query_text(cx)), "config");
        let focus = search.read_with(&cx, |search, cx| search.input_focus_handle(cx));
        assert!(cx.update(|window, _| focus.is_focused(window)), "focus returns to the field");
    }

    #[gpui::test]
    fn search_idle_example_this_mac_switches_scope(cx: &mut TestAppContext) {
        let root = fixture("search-idle-scope");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        settle(&view, &mut cx);
        click_selector(&mut cx, "search-example-2");
        let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
        assert_eq!(search.read_with(&cx, |search, _| search.scope()), crate::search::Scope::ThisMac);
        assert_eq!(search.read_with(&cx, |search, cx| search.query_text(cx)), "", "the field stays empty");
    }

    #[gpui::test]
    fn search_idle_example_everything_flips_rules(cx: &mut TestAppContext) {
        let root = fixture("search-idle-everything");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        settle(&view, &mut cx);
        click_selector(&mut cx, "search-example-3");
        let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
        assert!(search.read_with(&cx, |search, _| search.everything()));
    }

    #[gpui::test]
    fn search_idle_shows_for_empty_query(cx: &mut TestAppContext) {
        let root = fixture("search-idle-again");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        settle(&view, &mut cx);
        cx.simulate_input("x");
        wait_for_search(&view, &mut cx);
        assert!(cx.debug_bounds("search-example-0").is_none());
        cx.simulate_keystrokes("backspace");
        settle(&view, &mut cx);
        assert!(cx.debug_bounds("search-example-0").is_some());
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

        click_selector(&mut cx, "search-scope-this-mac");
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
        click_selector(&mut cx, "search-scope-folder");
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

    fn search_results_focused(view: &Entity<CrowWindow>, cx: &mut VisualTestContext) -> bool {
        let focus = view.read_with(cx, |w, cx| w.search.as_ref().expect("search open").read(cx).results_focus_handle());
        cx.update(|window, _| focus.is_focused(window))
    }

    #[gpui::test]
    fn search_down_from_field_focuses_results(cx: &mut TestAppContext) {
        let root = fixture("search-down-field");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("a");
        wait_for_search(&view, &mut cx);
        let paths = view.read_with(&cx, |w, cx| {
            let search = w.search.as_ref().expect("search open").read(cx);
            assert!(search.result_count() >= 2, "two name hits: {}", search.result_count());
            search.result_paths()
        });

        cx.simulate_keystrokes("down");
        assert!(search_results_focused(&view, &mut cx), "↓ moves into the results");
        let (cursor, selected) = view.read_with(&cx, |w, cx| {
            let search = w.search.as_ref().expect("search open").read(cx);
            (search.cursor(), search.selected_entry().map(|(path, _)| path))
        });
        assert_eq!(cursor, 0, "row 0 unless the user moved");
        assert_eq!(selected.as_deref(), Some(paths[0].as_path()), "row 0 is selected");

        cx.simulate_keystrokes("down");
        let (cursor, selected) = view.read_with(&cx, |w, cx| {
            let search = w.search.as_ref().expect("search open").read(cx);
            (search.cursor(), search.selected_entry().map(|(path, _)| path))
        });
        assert_eq!(cursor, 1, "↓ again moves on");
        assert_eq!(selected.as_deref(), Some(paths[1].as_path()));
    }

    #[gpui::test]
    fn search_down_without_results_stays_in_the_field(cx: &mut TestAppContext) {
        let root = fixture("search-down-empty");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("zzzzz");
        wait_for_search(&view, &mut cx);
        assert_eq!(view.read_with(&cx, |w, cx| w.search.as_ref().expect("search open").read(cx).result_count()), 0);
        cx.simulate_keystrokes("down");
        assert!(search_input_focused(&view, &mut cx), "↓ does nothing without results");
    }

    #[gpui::test]
    fn search_up_from_first_row_returns_to_field(cx: &mut TestAppContext) {
        let root = fixture("search-up-field");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("a");
        wait_for_search(&view, &mut cx);
        cx.simulate_keystrokes("down");
        assert!(search_results_focused(&view, &mut cx));
        cx.simulate_keystrokes("up");
        assert!(search_input_focused(&view, &mut cx), "↑ on the first row returns to the field");
        assert_eq!(view.read_with(&cx, |w, cx| w.search.as_ref().expect("search open").read(cx).cursor()), 0);

        cx.simulate_keystrokes("down down");
        assert_eq!(view.read_with(&cx, |w, cx| w.search.as_ref().expect("search open").read(cx).cursor()), 1);
        cx.simulate_keystrokes("up");
        assert!(search_results_focused(&view, &mut cx), "↑ below the first row just moves up");
        assert_eq!(view.read_with(&cx, |w, cx| w.search.as_ref().expect("search open").read(cx).cursor()), 0);
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
        cx.simulate_keystrokes("cmd-j");
        assert!(view.read_with(&cx, |w, _| w.why.is_some()), "Why This View opens over search");
        cx.simulate_keystrokes("escape");
        assert!(view.read_with(&cx, |w, _| w.why.is_none() && w.search.is_some()));
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
        let (mode, text, rows) = view.read_with(&cx, |w, cx| {
            let palette = w.palette.as_ref().expect("palette opens");
            (palette.mode, palette.input.read(cx).text().to_owned(), w.palette_rows())
        });
        assert_eq!(mode, PaletteMode::Commands, "the palette starts in command mode");
        assert_eq!(text, "", "the > lives in the token, not the field");
        assert!(rows.iter().any(|row| row == "New Window"), "command rows: {rows:?}");
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
                    .listing
                    .items
                    .iter()
                    .position(|item| matches!(item, Item::Command { command, .. } if command.id == crate::palette::CommandId::Copy))
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
        // Visit alpha so Recent has a row above the "In" header.
        pane(&view, &cx).update(&mut cx, |pane, cx| pane.navigate(root.join("alpha"), cx));
        settle(&view, &mut cx);
        pane(&view, &cx).update(&mut cx, |pane, cx| pane.navigate(root.clone(), cx));
        settle(&view, &mut cx);
        cx.simulate_keystrokes("cmd-k");
        settle(&view, &mut cx);
        let cursor = |cx: &VisualTestContext| view.read_with(cx, |w, _| w.palette.as_ref().map(|p| p.cursor));
        assert_eq!(cursor(&cx), Some(0));
        let rows = view.read_with(&cx, |w, _| w.palette_rows());
        assert!(rows[2].starts_with("# "), "a section header sits after the first item: {rows:?}");
        cx.simulate_keystrokes("j");
        assert_eq!(cursor(&cx), Some(1), "j moves down across the header");
        cx.simulate_keystrokes("k");
        assert_eq!(cursor(&cx), Some(0), "k moves up");
        cx.simulate_keystrokes("escape");
        assert!(view.read_with(&cx, |w, _| w.palette.is_none()));
    }

    #[gpui::test]
    fn palette_lists_sections_in_order(cx: &mut TestAppContext) {
        let root = fixture("palette-sections");
        // gamma stays in "In": alpha becomes Recent and beta is a favorite.
        std::fs::create_dir_all(root.join("gamma")).expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("beta")]));
        cx.simulate_keystrokes("cmd-down");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("cmd-up");
        settle(&view, &mut cx);

        cx.simulate_keystrokes("cmd-k");
        settle(&view, &mut cx);
        let rows = view.read_with(&cx, |w, _| w.palette_rows());
        assert_eq!(rows.first().map(String::as_str), Some("# Recent"), "{rows:?}");
        let favorites = rows.iter().position(|row| row == "# Favorites").expect("a Favorites header");
        let here = rows.iter().position(|row| row.starts_with("# In ")).expect("an In header");
        assert!(favorites < here, "Recent, then Favorites, then In: {rows:?}");
        assert_eq!(rows[here], format!("# In {}", root.file_name().unwrap().to_string_lossy()));
        assert!(rows[1..favorites].iter().any(|row| row.ends_with("/alpha")), "alpha is Recent: {rows:?}");
        assert_eq!(rows.iter().filter(|row| row.ends_with("/beta")).count(), 1, "beta once only: {rows:?}");
        assert!(rows[favorites + 1..here].iter().any(|row| row.ends_with("/beta")), "under Favorites: {rows:?}");
        assert!(rows[here + 1..].iter().any(|row| row.ends_with("/gamma")), "gamma is In: {rows:?}");
    }

    #[gpui::test]
    fn palette_twins_get_distinct_labels(cx: &mut TestAppContext) {
        let root = fixture("palette-match");
        for side in ["a", "b"] {
            std::fs::create_dir_all(root.join(side).join("twin")).expect("create fixture");
        }
        let (view, mut cx) = open(cx, &root);
        for side in ["a", "b"] {
            let twin = root.join(side).join("twin");
            pane(&view, &cx).update(&mut cx, |pane, cx| pane.navigate(twin, cx));
            settle(&view, &mut cx);
        }
        // Back to root: the folder you are in is left out of the palette.
        pane(&view, &cx).update(&mut cx, |pane, cx| pane.navigate(root.clone(), cx));
        settle(&view, &mut cx);

        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input("twin");
        settle(&view, &mut cx);
        let rows = view.read_with(&cx, |w, _| w.palette_rows());
        assert_eq!(rows.len(), 2, "two folders named twin: {rows:?}");
        assert!(rows[0].ends_with("/a/twin") && rows[1].ends_with("/b/twin"), "the paths tell them apart: {rows:?}");
    }

    #[gpui::test]
    fn palette_leaves_out_the_current_folder(cx: &mut TestAppContext) {
        let root = fixture("palette-here");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-down");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("cmd-up");
        settle(&view, &mut cx);
        assert!(
            cx.read(|cx| cx.global::<Ui>().palette_recents.contains(&root)),
            "the folder we are in is a recent visit"
        );

        cx.simulate_keystrokes("cmd-k");
        settle(&view, &mut cx);
        let rows = view.read_with(&cx, |w, _| w.palette_rows());
        assert_eq!(rows.first().map(String::as_str), Some("# Recent"), "{rows:?}");
        assert!(rows[1].ends_with("/alpha"), "the first row is the folder before this one: {rows:?}");
        let label = crow_core::abbreviate_home(&root, &cx.read(|cx| cx.global::<Ui>().home.clone()));
        assert!(!rows.contains(&label), "the current folder is not a row: {rows:?}");
        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("alpha"), "Cmd-K then Return goes somewhere");
    }

    #[gpui::test]
    fn palette_rows_are_inset_from_the_sheet(cx: &mut TestAppContext) {
        let root = fixture("palette-inset");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-k");
        settle(&view, &mut cx);
        let sheet = cx.debug_bounds("sheet-panel").expect("the palette sheet");
        let row = cx.debug_bounds("palette-row-0").expect("the first row");
        assert_eq!(row.left() - sheet.left(), px(PALETTE_LIST_PAD + 1.), "list padding plus the sheet's border");
        assert_eq!(sheet.right() - row.right(), px(PALETTE_LIST_PAD + 1.), "the same on the right");
    }

    #[gpui::test]
    fn palette_header_leaves_room_above_the_first_row(cx: &mut TestAppContext) {
        let root = fixture("palette-header-gap");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-k");
        settle(&view, &mut cx);
        let label = cx.debug_bounds("palette-header-label-0").expect("the first header");
        let row = cx.debug_bounds("palette-row-0").expect("the first row");
        assert!(label.bottom() <= row.top(), "the header label sits above the row: {label:?} {row:?}");
        assert!(row.top() - label.bottom() >= px(6.), "room between the label and the row: {label:?} {row:?}");
    }

    #[gpui::test]
    fn palette_count_says_places(cx: &mut TestAppContext) {
        let root = fixture("palette-count");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input("alpha");
        settle(&view, &mut cx);
        let (rows, count) =
            view.read_with(&cx, |w, _| (w.palette_rows(), w.palette.as_ref().map(|p| p.listing.count.to_string())));
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(count.as_deref(), Some("1 place"));
    }

    #[gpui::test]
    fn palette_cursor_skips_headers(cx: &mut TestAppContext) {
        let root = fixture("palette-cursor");
        std::fs::create_dir_all(root.join("gamma")).expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("beta")]));
        cx.simulate_keystrokes("cmd-down");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("cmd-up");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("cmd-k");
        settle(&view, &mut cx);

        // Recent holds alpha (root is where we are, so it is left out): item 1 is
        // the first favorite.
        cx.simulate_keystrokes("down");
        let cursor = view.read_with(&cx, |w, _| {
            let palette = w.palette.as_ref()?;
            match palette.listing.items.get(palette.cursor)? {
                Item::Folder { label, .. } => Some(label.clone()),
                Item::Command { .. } => None,
            }
        });
        assert!(cursor.as_deref().is_some_and(|label| label.ends_with("/beta")), "down crossed the header: {cursor:?}");
        cx.simulate_keystrokes("up up up");
        assert_eq!(view.read_with(&cx, |w, _| w.palette.as_ref().map(|p| p.cursor)), Some(0), "up at the top stays");
    }

    #[gpui::test]
    fn palette_gt_becomes_commands_token(cx: &mut TestAppContext) {
        let root = fixture("palette-token");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input(">");
        settle(&view, &mut cx);
        let (mode, text, count) = view.read_with(&cx, |w, cx| {
            let palette = w.palette.as_ref().expect("palette is open");
            (palette.mode, palette.input.read(cx).text().to_owned(), palette.listing.count.to_string())
        });
        assert_eq!(mode, PaletteMode::Commands, "> switches to commands");
        assert_eq!(text, "", "the > is the token, not text");
        assert!(count.ends_with("commands"), "a command count: {count}");

        cx.simulate_input("hid");
        settle(&view, &mut cx);
        let (first, count) = view.read_with(&cx, |w, _| {
            let palette = w.palette.as_ref().expect("palette is open");
            let first = match &palette.listing.items[0] {
                Item::Command { command, .. } => command.label,
                Item::Folder { .. } => "",
            };
            (first, palette.listing.count.to_string())
        });
        assert!(first.contains("Hidden"), "the hidden-files command comes first: {first}");
        let (matches, total) = count.split_once(" of ").expect("m of n");
        assert!(matches.parse::<usize>().is_ok() && total.parse::<usize>().is_ok(), "m of n: {count}");
    }

    #[gpui::test]
    fn palette_backspace_on_empty_query_removes_token(cx: &mut TestAppContext) {
        let root = fixture("palette-token-back");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input(">");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("backspace");
        settle(&view, &mut cx);
        let (mode, text, rows) = view.read_with(&cx, |w, cx| {
            let palette = w.palette.as_ref().expect("palette is open");
            (palette.mode, palette.input.read(cx).text().to_owned(), w.palette_rows())
        });
        assert_eq!(mode, PaletteMode::Folders);
        assert_eq!(text, "");
        assert!(rows.first().is_some_and(|row| row.starts_with("# ")), "folders are back: {rows:?}");
    }

    #[gpui::test]
    fn palette_backspace_with_text_keeps_token(cx: &mut TestAppContext) {
        let root = fixture("palette-token-text");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input(">ab");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("left left backspace");
        settle(&view, &mut cx);
        let (mode, text) = view.read_with(&cx, |w, cx| {
            let palette = w.palette.as_ref().expect("palette is open");
            (palette.mode, palette.input.read(cx).text().to_owned())
        });
        assert_eq!(mode, PaletteMode::Commands, "text keeps the token");
        assert_eq!(text, "ab");
    }

    #[gpui::test]
    fn palette_toggle_chip_shows_state(cx: &mut TestAppContext) {
        let root = fixture("palette-toggle");
        let (view, mut cx) = open(cx, &root);
        let state = |cx: &VisualTestContext| {
            view.read_with(cx, |w, cx| {
                let palette = w.palette.as_ref()?;
                palette.listing.items.iter().find_map(|item| match item {
                    Item::Command { command, .. } => w.toggle_state(command.id, cx),
                    Item::Folder { .. } => None,
                })
            })
        };
        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input(">hidden");
        settle(&view, &mut cx);
        assert_eq!(state(&cx), Some(false), "hidden files are off");

        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input(">hidden");
        settle(&view, &mut cx);
        assert_eq!(state(&cx), Some(true), "the command turned them on");
    }

    #[gpui::test]
    fn palette_tab_completes_highlighted_folder(cx: &mut TestAppContext) {
        let root = fixture("palette-tab-complete");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-down");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("cmd-up");
        settle(&view, &mut cx);

        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input("alpha");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("tab");
        settle_palette(&view, &mut cx);
        let text = view.read_with(&cx, |w, cx| w.palette.as_ref().unwrap().input.read(cx).text().to_owned());
        assert_eq!(text, format!("{}/alpha/", root.display()), "the highlighted folder and a slash");
        let rows = view.read_with(&cx, |w, _| w.palette_rows());
        assert!(rows.iter().any(|row| row.ends_with("/alpha/inner")), "its subfolders: {rows:?}");

        cx.simulate_input("inn");
        settle_palette(&view, &mut cx);
        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("alpha/inner"));
    }

    #[gpui::test]
    fn palette_tab_does_nothing_in_command_mode(cx: &mut TestAppContext) {
        let root = fixture("palette-tab-commands");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input(">");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("tab");
        settle(&view, &mut cx);
        let (mode, text, open) = view.read_with(&cx, |w, cx| {
            (
                w.palette.as_ref().map(|p| p.mode),
                w.palette.as_ref().map(|p| p.input.read(cx).text().to_owned()),
                w.palette.is_some(),
            )
        });
        assert_eq!(mode, Some(PaletteMode::Commands));
        assert_eq!(text.as_deref(), Some(""), "Tab did not type anything");
        assert!(open, "the palette stays open");
    }

    #[gpui::test]
    fn palette_path_mode_lists_children_off_main_thread(cx: &mut TestAppContext) {
        let root = fixture("palette-path");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input(&format!("{}/", root.display()));
        settle_palette(&view, &mut cx);
        let rows = view.read_with(&cx, |w, _| w.palette_rows());
        assert_eq!(rows.len(), 4, "this folder, alpha, beta — no notes.txt: {rows:?}");
        assert!(rows[0].starts_with("# In "), "{rows:?}");
        assert_eq!(rows[1], root.display().to_string(), "the parent itself comes first");
        assert!(rows[2].ends_with("/alpha") && rows[3].ends_with("/beta"), "{rows:?}");
    }

    #[gpui::test]
    fn palette_path_mode_hides_dot_folders_until_typed(cx: &mut TestAppContext) {
        let root = fixture("palette-path-hidden");
        std::fs::create_dir_all(root.join(".hidden")).expect("create fixture");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input(&format!("{}/", root.display()));
        settle_palette(&view, &mut cx);
        let rows = view.read_with(&cx, |w, _| w.palette_rows());
        assert!(!rows.iter().any(|row| row.ends_with("/.hidden")), "hidden until asked: {rows:?}");

        cx.simulate_input(".h");
        settle_palette(&view, &mut cx);
        let rows = view.read_with(&cx, |w, _| w.palette_rows());
        assert!(rows.iter().any(|row| row.ends_with("/.hidden")), "a dot partial shows hidden folders: {rows:?}");
    }

    #[gpui::test]
    fn palette_lists_recently_used_commands(cx: &mut TestAppContext) {
        let root = fixture("palette-recent-commands");
        let (view, mut cx) = open(cx, &root);
        for query in ["columns", "why this"] {
            cx.simulate_keystrokes("cmd-k");
            cx.simulate_input(&format!(">{query}"));
            settle(&view, &mut cx);
            cx.simulate_keystrokes("enter");
            settle(&view, &mut cx);
            cx.simulate_keystrokes("escape");
            settle(&view, &mut cx);
        }

        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input(">");
        settle(&view, &mut cx);
        let rows = view.read_with(&cx, |w, _| w.palette_rows());
        let recent = rows.iter().position(|row| row == "# Recently used").expect("a Recently used header");
        let all = rows.iter().position(|row| row == "# All commands").expect("an All commands header");
        assert!(recent < all, "{rows:?}");
        assert_eq!(&rows[recent + 1..all], ["Why This View?", "View as Columns"], "newest first: {rows:?}");
        let repeated = rows[all + 1..].iter().filter(|row| *row == "Why This View?" || *row == "View as Columns");
        assert_eq!(repeated.count(), 0, "recent commands are not listed again: {rows:?}");
    }

    #[gpui::test]
    fn palette_recent_commands_survive_relaunch(cx: &mut TestAppContext) {
        let root = fixture("palette-commands-relaunch");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input(">columns");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);

        let key = crate::palette::CommandId::ViewAsColumns.key();
        let state = cx.update(|_, cx| cx.global::<Ui>().state_dir.clone());
        let file = state.join("palette-commands.json");
        let mut saved = String::new();
        for _ in 0..200 {
            cx.run_until_parked();
            if let Ok(text) = std::fs::read_to_string(&file) {
                saved = text;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(saved.contains(key), "saved with its stable key: {saved:?}");

        // Forget it in memory, then reload the state file as a relaunch would.
        cx.update(|_, cx| cx.global_mut::<Ui>().recent_commands = Default::default());
        cx.update(|_, cx| crate::ui::reload_state_for_tests(cx));
        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input(">");
        settle(&view, &mut cx);
        let rows = view.read_with(&cx, |w, _| w.palette_rows());
        assert_eq!(rows.first().map(String::as_str), Some("# Recently used"), "{rows:?}");
        assert_eq!(rows.get(1).map(String::as_str), Some("View as Columns"), "{rows:?}");
    }

    #[gpui::test]
    fn palette_recent_commands_setting_false_hides_and_stops_recording(cx: &mut TestAppContext) {
        let root = fixture("palette-commands-setting");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input(">columns");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        let state = cx.update(|_, cx| cx.global::<Ui>().state_dir.clone());
        let file = state.join("palette-commands.json");
        let mut before = String::new();
        for _ in 0..200 {
            cx.run_until_parked();
            if let Ok(text) = std::fs::read_to_string(&file) {
                before = text;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!before.is_empty(), "one command is remembered");

        write_config(&view, &mut cx, "settings.toml", "palette_recent_commands = false\n");
        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input(">why this");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("escape");
        settle(&view, &mut cx);
        for _ in 0..20 {
            cx.run_until_parked();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input(">");
        settle(&view, &mut cx);
        let rows = view.read_with(&cx, |w, _| w.palette_rows());
        assert!(!rows.iter().any(|row| row == "# Recently used"), "the section is hidden: {rows:?}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), before, "nothing new was recorded");

        cx.simulate_keystrokes("escape");
        write_config(&view, &mut cx, "settings.toml", "");
        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input(">");
        settle(&view, &mut cx);
        let rows = view.read_with(&cx, |w, _| w.palette_rows());
        assert_eq!(rows.first().map(String::as_str), Some("# Recently used"), "{rows:?}");
        assert_eq!(rows.get(1).map(String::as_str), Some("View as Columns"), "only the earlier run: {rows:?}");
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
    fn closing_the_last_window_mid_operation_asks_first(app: &mut TestAppContext) {
        let root = fixture("close-guard");
        let (view, mut cx) = open(app, &root);
        let cancel = inject_running(&mut cx);
        let windows = |cx: &mut VisualTestContext| cx.update(|_, cx| cx.windows().len());
        let asking = |view: &Entity<CrowWindow>, cx: &VisualTestContext| {
            view.read_with(cx, |w, _| {
                matches!(w.sheet, Some(Sheet::StopOperation { then: AfterStop::CloseWindow, .. }))
            })
        };

        cx.simulate_keystrokes("cmd-w");
        assert!(asking(&view, &cx), "the last window asks before cutting the move off");
        assert_eq!(windows(&mut cx), 1);

        cx.simulate_keystrokes("escape");
        assert!(view.read_with(&cx, |w, _| w.sheet.is_none()), "Esc keeps it running");
        assert!(!cancel.load(std::sync::atomic::Ordering::Relaxed), "and does not cancel");

        cx.simulate_keystrokes("cmd-w");
        cx.simulate_keystrokes("enter");
        assert!(cancel.load(std::sync::atomic::Ordering::Relaxed), "Stop and Close cancels the operation");
        assert_eq!(windows(&mut cx), 1, "the window waits for the operation to wind down");

        // The operation finishes cancelling: then the window closes.
        app.update(|cx| cx.global_mut::<Operations>().running = None);
        app.executor().advance_clock(std::time::Duration::from_millis(100));
        app.run_until_parked();
        assert_eq!(app.update(|cx| cx.windows().len()), 0, "closed once the operation stopped");
    }

    #[gpui::test]
    fn closing_with_other_windows_open_just_closes(app: &mut TestAppContext) {
        let root = fixture("close-free");
        let (_view, mut cx) = open(app, &root);
        cx.simulate_keystrokes("cmd-n");
        cx.run_until_parked();
        assert_eq!(app.update(|cx| cx.windows().len()), 2);
        let _cancel = inject_running(&mut cx);
        // Another window stays open, so the operation keeps going without asking.
        cx.simulate_keystrokes("cmd-w");
        app.run_until_parked();
        assert_eq!(app.update(|cx| cx.windows().len()), 1, "closed without a sheet");
        app.update(|cx| cx.global_mut::<Operations>().running = None);
    }

    #[gpui::test]
    fn quitting_mid_operation_asks_first(cx: &mut TestAppContext) {
        let root = fixture("quit-guard");
        let (view, mut cx) = open(cx, &root);
        let cancel = inject_running(&mut cx);
        // Menu, palette and ⌘Q all go through `quit`.
        cx.update(|_, cx| quit(cx));
        cx.run_until_parked();
        assert!(
            view.read_with(&cx, |w, _| matches!(w.sheet, Some(Sheet::StopOperation { then: AfterStop::Quit, .. }))),
            "⌘Q asks while the move runs"
        );
        cx.simulate_keystrokes("escape");
        assert!(!cancel.load(std::sync::atomic::Ordering::Relaxed));
        cx.update(|_, cx| cx.global_mut::<Operations>().running = None);
    }

    #[gpui::test]
    fn list_name_column_follows_the_window_width(cx: &mut TestAppContext) {
        let root = fixture("list-width");
        let (_view, mut cx) = open(cx, &root);
        let widths = |cx: &mut VisualTestContext| {
            cx.run_until_parked();
            // The pane asks for one more frame once it sees the new width; the test
            // platform does not run scheduled frames, so draw it here.
            cx.update(|window, _| window.refresh());
            cx.run_until_parked();
            let name = cx.debug_bounds("column-header-0").expect("Name header").size.width;
            let next = cx.debug_bounds("column-header-1").expect("second header").size.width;
            let table = cx.debug_bounds("table-header").expect("header").size.width;
            (name, next, table)
        };
        cx.simulate_resize(gpui::size(px(1400.), px(700.)));
        let (wide_name, wide_next, wide_table) = widths(&mut cx);
        cx.simulate_resize(gpui::size(px(1100.), px(700.)));
        let (narrow_name, narrow_next, narrow_table) = widths(&mut cx);
        assert!(wide_table > narrow_table, "the list got narrower");
        assert_eq!(wide_next, narrow_next, "the other columns keep their widths");
        assert_eq!(wide_name - narrow_name, wide_table - narrow_table, "Name absorbs the whole change");

        // Narrower than every column's minimum: whole columns hide from the right, so no
        // cell runs past the list's edge.
        for width in [900., 760., 640.] {
            cx.simulate_resize(gpui::size(px(width), px(700.)));
            cx.run_until_parked();
            cx.update(|window, _| window.refresh());
            cx.run_until_parked();
            let table = cx.debug_bounds("table-header").expect("header");
            let shown: Vec<_> = (0..6).filter_map(|i| cx.debug_bounds(format!("column-header-{i}").leak())).collect();
            assert!(!shown.is_empty(), "Name always shows");
            for cell in &shown {
                assert!(cell.right() <= table.right() + px(0.5), "at {width}: {cell:?} past {table:?}");
            }
        }
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
    fn click_in_a_scrolled_filter_field_lands_on_the_visible_text(cx: &mut TestAppContext) {
        let root = fixture("filter-scrolled-click");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-3");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("/");
        // Far wider than the column, so the field scrolls to keep the caret at the end.
        let long = "abcdefghij".repeat(12);
        cx.simulate_input(&long);
        cx.run_until_parked();
        let field = cx.debug_bounds("filter-field").expect("field on screen");
        // Just right of the accent `/`: the start of the visible text, not of the query.
        cx.simulate_click(gpui::point(field.left() + px(30.), field.center().y), gpui::Modifiers::default());
        cx.simulate_input("X");
        let query = pane(&view, &cx).read_with(&cx, |p, cx| p.filter_query(cx)).expect("filtering");
        let at = query.find('X').expect("typed");
        assert!(at > 20, "the click landed on the scrolled-away start (X at {at}): {query}");
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
        assert!(view.read_with(&cx, |w, _| w.palette.is_some()), "Go to Folder (the palette) is open");
        click_scrim(&mut cx);
        assert!(view.read_with(&cx, |w, _| w.palette.is_none()), "a scrim click closes it like Escape");
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
    fn go_to_folder_opens_the_palette_on_this_folder(cx: &mut TestAppContext) {
        let root = fixture("goto");
        let (view, mut cx) = open(cx, &root);
        let typed = |cx: &VisualTestContext| {
            view.read_with(cx, |w, cx| w.palette.as_ref().map(|p| p.input.read(cx).text().to_owned()))
        };

        cx.simulate_keystrokes("cmd-shift-g");
        settle(&view, &mut cx);
        assert_eq!(typed(&cx), Some(format!("{}/", root.display())), "starts on this folder's path");
        cx.simulate_input("al");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("tab");
        settle(&view, &mut cx);
        assert!(typed(&cx).is_some_and(|t| t.ends_with("/alpha/")), "Tab completes a folder: {:?}", typed(&cx));
        cx.simulate_input("inner");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("alpha/inner"));
        assert!(typed(&cx).is_none(), "Return closes the palette");

        cx.simulate_keystrokes("cmd-shift-g cmd-a");
        cx.simulate_input(&root.join("notes.txt").display().to_string());
        settle(&view, &mut cx);
        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root, "a file opens its folder…");
        assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"), "…with the file selected");

        cx.simulate_keystrokes("cmd-shift-g cmd-a");
        cx.simulate_input(&root.join("nothing-here").display().to_string());
        settle(&view, &mut cx);
        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        assert!(typed(&cx).is_some(), "a missing path keeps the palette open");
        cx.simulate_keystrokes("escape");
        assert!(typed(&cx).is_none(), "Escape closes it");
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
    fn new_folder_arrives_and_its_rename_takes_typing_at_once(cx: &mut TestAppContext) {
        let root = fixture("newfolder-arrive");
        let (view, mut cx) = open(cx, &root);
        set_motion(&mut cx, "full");
        cx.simulate_keystrokes("cmd-shift-n");
        settle(&view, &mut cx);
        cx.run_until_parked();
        let effects = pane(&view, &cx).read_with(&cx, |pane, cx| pane.effects(cx));
        assert!(
            effects.iter().any(|(name, fx)| name == "untitled folder" && *fx == Fx::Arrive),
            "the new folder arrives: {effects:?}"
        );
        cx.simulate_input("Projects");
        assert_eq!(
            pane(&view, &cx).read_with(&cx, |pane, cx| pane.rename_text(cx)).as_deref(),
            Some("Projects"),
            "typing goes into the field without waiting for the animation"
        );
        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        assert!(root.join("Projects").is_dir());
    }

    #[gpui::test]
    fn undo_arrives_in_full_motion(cx: &mut TestAppContext) {
        let root = fixture("undo-arrive");
        let (view, mut cx) = open(cx, &root);
        set_motion(&mut cx, "full");
        cx.simulate_keystrokes("down down cmd-backspace");
        settle(&view, &mut cx);
        assert!(!root.join("notes.txt").exists());
        cx.simulate_keystrokes("cmd-z");
        settle(&view, &mut cx);
        assert!(root.join("notes.txt").exists(), "undo put it back");
        let effects = pane(&view, &cx).read_with(&cx, |pane, cx| pane.effects(cx));
        assert!(
            effects.iter().any(|(name, fx)| name == "notes.txt" && *fx == Fx::Arrive),
            "the restored row arrives: {effects:?}"
        );
    }

    #[gpui::test]
    fn new_file_creates_and_renames(cx: &mut TestAppContext) {
        let root = fixture("newfile");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-alt-n");
        settle(&view, &mut cx);
        cx.run_until_parked();
        let renaming = pane(&view, &cx).read_with(&cx, |p, cx| p.rename_text(cx));
        assert_eq!(renaming.as_deref(), Some("untitled"), "the new file is created and its rename starts");
        assert!(root.join("untitled").is_file(), "an empty file, not a folder");
        cx.simulate_input("Notes");
        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        assert!(root.join("Notes").is_file(), "renamed on Return");
        assert!(!root.join("untitled").exists());
        assert_eq!(selected(&view, &cx).as_deref(), Some("Notes"));

        // The palette lists it too.
        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input(">new file");
        settle(&view, &mut cx);
        let rows = view.read_with(&cx, |w, _| w.palette_rows());
        assert!(rows.first().is_some_and(|row| row.contains("New File")), "{rows:?}");
    }

    #[gpui::test]
    fn a_late_change_batch_does_not_bring_back_a_deleted_file(cx: &mut TestAppContext) {
        let root = fixture("change-race");
        let (view, mut cx) = open(cx, &root);
        settle(&view, &mut cx);
        let folder = pane(&view, &cx).read_with(&cx, |p, _| p.test_folder());
        folder.update(&mut cx, |folder, _| folder.set_apply_delay(std::time::Duration::from_millis(300)));
        let executor = cx.executor();

        // Delete notes.txt and queue it; the poll picks the batch up but the apply
        // lands 300 ms later, so it is still in flight for the next batch.
        std::fs::remove_file(root.join("notes.txt")).expect("delete");
        folder.update(&mut cx, |folder, _| folder.queue_changes(vec![root.join("notes.txt")]));
        executor.advance_clock(std::time::Duration::from_millis(100));
        cx.run_until_parked();

        // A second batch whose stat predates the first apply used to carry the old
        // listing (with notes.txt) and put the deleted file back.
        std::fs::write(root.join("later.txt"), "x").expect("create");
        folder.update(&mut cx, |folder, _| folder.queue_changes(vec![root.join("later.txt")]));
        executor.advance_clock(std::time::Duration::from_millis(100));
        cx.run_until_parked();
        executor.advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();

        let names = folder.read_with(&cx, |folder, _| folder.names());
        assert!(!names.contains(&"notes.txt".to_owned()), "the deleted file stays gone: {names:?}");
        assert!(names.contains(&"later.txt".to_owned()), "the second batch still lands: {names:?}");
    }

    #[gpui::test]
    fn preview_toggle_hides_it_in_list_and_columns(cx: &mut TestAppContext) {
        let root = fixture("preview-toggle");
        let (view, mut cx) = open(cx, &root);
        assert!(cx.debug_bounds("preview").is_some(), "list view shows the preview");
        cx.simulate_keystrokes("cmd-shift-p");
        settle(&view, &mut cx);
        assert!(cx.debug_bounds("preview").is_none(), "hiding removes the list preview");
        assert!(!cx.read(|cx| cx.global::<Ui>().settings.show_preview), "the setting is off");
        let text = std::fs::read_to_string(config_dir(&cx).join("settings.toml")).expect("settings file");
        assert!(text.contains("\nshow_preview = false\n"), "the choice is saved as a bool: {text}");
        assert_eq!(
            view.read_with(&cx, |w, cx| w.toggle_state(crate::palette::CommandId::TogglePreview, cx)),
            Some(false),
            "the palette chip follows"
        );

        cx.simulate_keystrokes("cmd-shift-p");
        settle(&view, &mut cx);
        assert!(cx.debug_bounds("preview").is_some(), "showing it again");

        cx.simulate_keystrokes("cmd-3");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("down down");
        settle(&view, &mut cx);
        assert!(cx.debug_bounds("preview-column").is_some(), "a file's preview is the last column");
        cx.simulate_keystrokes("cmd-shift-p");
        settle(&view, &mut cx);
        assert!(cx.debug_bounds("preview-column").is_none(), "hiding removes the preview column");
        assert_eq!(view.read_with(&cx, |w, cx| w.pane().read(cx).mode()), ViewMode::Columns);
    }

    #[gpui::test]
    fn show_preview_false_in_settings_hides_it(cx: &mut TestAppContext) {
        let root = fixture("preview-setting");
        let (view, mut cx) = open(cx, &root);
        let text = std::fs::read_to_string(config_dir(&cx).join("settings.toml")).expect("settings file");
        assert!(text.contains("show_preview = true"), "the documented default: {text}");
        write_config(&view, &mut cx, "settings.toml", &text.replace("show_preview = true", "show_preview = false"));
        assert!(cx.debug_bounds("preview").is_none(), "live reload hides the preview");
        let focus = cx.update(|_, cx| cx.global::<Ui>().settings.show_preview);
        assert!(!focus);
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

        // Go to Folder (the palette on a path).
        cx.simulate_keystrokes("cmd-shift-g");
        cx.run_until_parked();
        cx.simulate_input("/ ");
        let text = view
            .read_with(&cx, |w, cx| w.palette.as_ref().map(|p| p.input.read(cx).text().to_owned()))
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

    /// Set `motion` in the test's settings.toml and reload, as a user would.
    fn set_motion(cx: &mut VisualTestContext, mode: &str) {
        cx.update(|_, cx| {
            let dir = cx.global::<crate::ui::Ui>().config_dir.clone();
            std::fs::write(dir.join("settings.toml"), format!("motion = \"{mode}\"\n")).expect("write settings");
            crate::ui::reload_config(cx);
        });
    }

    fn trash_in_mode(mode: &str, cx: &mut TestAppContext) {
        let root = fixture(&format!("trash-{mode}"));
        let (view, mut cx) = open(cx, &root);
        set_motion(&mut cx, mode);
        cx.simulate_keystrokes("down down"); // notes.txt
        assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"));
        cx.simulate_keystrokes("cmd-backspace");
        settle(&view, &mut cx);
        assert!(!root.join("notes.txt").exists(), "{mode}: trashed");
        assert!(!names(&view, &cx).contains(&"notes.txt".to_owned()), "{mode}: gone from the list");
        assert_eq!(selected(&view, &cx).as_deref(), Some("alpha"), "{mode}: the cursor lands where it does today");
        assert!(pane(&view, &cx).read_with(&cx, |pane, cx| pane.error(cx).is_none()), "{mode}: no error");
    }

    #[gpui::test]
    fn trash_in_off_motion_leaves_the_listing(cx: &mut TestAppContext) {
        trash_in_mode("off", cx);
    }

    #[gpui::test]
    fn trash_in_low_motion_leaves_the_listing(cx: &mut TestAppContext) {
        trash_in_mode("low", cx);
    }

    #[gpui::test]
    fn trash_in_full_motion_leaves_the_listing(cx: &mut TestAppContext) {
        trash_in_mode("full", cx);
    }

    #[gpui::test]
    fn trash_in_full_motion_in_columns_leaves_the_listing(cx: &mut TestAppContext) {
        let root = fixture("trash-full-columns");
        let (view, mut cx) = open(cx, &root);
        set_motion(&mut cx, "full");
        cx.simulate_keystrokes("cmd-3");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("down down cmd-backspace");
        settle(&view, &mut cx);
        assert!(!root.join("notes.txt").exists(), "trashed from the column");
        assert!(!names(&view, &cx).contains(&"notes.txt".to_owned()), "gone from the column");
        assert!(pane(&view, &cx).read_with(&cx, |pane, cx| pane.error(cx).is_none()), "no error");
    }

    #[gpui::test]
    fn trash_three_items_in_full_motion(cx: &mut TestAppContext) {
        let root = fixture("trash-three");
        let (view, mut cx) = open(cx, &root);
        set_motion(&mut cx, "full");
        cx.simulate_keystrokes("cmd-a cmd-backspace");
        settle(&view, &mut cx);
        assert!(names(&view, &cx).is_empty(), "all three rows left the list");
        assert!(!root.join("alpha").exists() && !root.join("beta").exists() && !root.join("notes.txt").exists());
    }

    #[gpui::test]
    fn trash_five_hundred_items_in_full_motion(cx: &mut TestAppContext) {
        let root = std::env::temp_dir().join(format!("crow-app-trash-500-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create fixture");
        for i in 0..500 {
            std::fs::write(root.join(format!("file-{i:03}.txt")), "x").expect("create fixture");
        }
        let root = root.canonicalize().expect("canonical fixture path");
        let (view, mut cx) = open(cx, &root);
        set_motion(&mut cx, "full");
        cx.simulate_keystrokes("cmd-a cmd-backspace");
        settle(&view, &mut cx);
        assert!(names(&view, &cx).is_empty(), "all 500 rows left the list");
        assert!(!root.join("file-000.txt").exists() && !root.join("file-499.txt").exists());
        assert!(pane(&view, &cx).read_with(&cx, |pane, cx| pane.error(cx).is_none()), "no error");
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
        assert!(sidebar_looks(&view, &mut cx)[1].drop, "the row's look carries the drop fill");
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
        assert!(sidebar_looks(&view, &mut cx).iter().all(|look| !look.drop), "…and every row's drop fill goes");
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
    fn dropping_on_a_folder_row_in_full_motion_moves_and_leaves_the_listing(cx: &mut TestAppContext) {
        let root = fixture("drop-row-full");
        let (view, mut cx) = open(cx, &root);
        set_motion(&mut cx, "full");
        drop_on(&mut cx, "row-alpha", vec![root.join("notes.txt")]);
        settle(&view, &mut cx);
        assert!(root.join("alpha/notes.txt").exists(), "dropped onto alpha: moved into it");
        assert!(!root.join("notes.txt").exists(), "…and left its old place");
        assert!(!names(&view, &cx).contains(&"notes.txt".to_owned()), "the source row left the list");
    }

    #[gpui::test]
    fn a_move_starts_effects_on_the_source_and_the_target(cx: &mut TestAppContext) {
        let root = fixture("move-effects");
        let (view, mut cx) = open(cx, &root);
        set_motion(&mut cx, "full");
        let pane = pane(&view, &cx);
        cx.update(|_, cx| {
            pane.update(cx, |pane, cx| {
                pane.start_effect(vec![root.join("notes.txt")], Fx::MoveOut, cx);
                pane.start_effect(vec![root.join("alpha")], Fx::Receive, cx);
            });
        });
        let effects = pane.read_with(&cx, |pane, cx| pane.effects(cx));
        assert!(effects.iter().any(|(name, fx)| name == "notes.txt" && *fx == Fx::MoveOut), "{effects:?}");
        assert!(effects.iter().any(|(name, fx)| name == "alpha" && *fx == Fx::Receive), "{effects:?}");
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

    // ----- U9: contextual tips -----

    /// `root/` with `n` files, for the scroll trigger.
    fn long_fixture(name: &str, n: usize) -> PathBuf {
        let root = std::env::temp_dir().join(format!("crow-app-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create fixture");
        for i in 0..n {
            std::fs::write(root.join(format!("f{i:03}.txt")), "").expect("create fixture");
        }
        root.canonicalize().expect("canonical fixture path")
    }

    fn tip(view: &Entity<CrowWindow>, cx: &VisualTestContext) -> Option<TipId> {
        view.read_with(cx, |w, _| w.tip)
    }

    fn tips_json(cx: &VisualTestContext) -> String {
        let file = cx.read(|cx| cx.global::<Ui>().state_dir.join(crow_core::TIPS_FILE));
        std::fs::read_to_string(file).unwrap_or_default()
    }

    /// A pointer scroll of `lines` up (negative delta) over `selector`.
    fn scroll(cx: &mut VisualTestContext, selector: &'static str, lines: f32) {
        let bounds = cx.debug_bounds(selector).unwrap_or_else(|| panic!("{selector} is laid out"));
        cx.simulate_event(gpui::ScrollEvent {
            position: bounds.center(),
            delta: gpui::ScrollDelta::Lines(gpui::point(0., -lines)),
            ..Default::default()
        });
    }

    /// Call `offer_tip` as a trigger would, with the test window.
    fn offer(view: &Entity<CrowWindow>, id: TipId, cx: &mut VisualTestContext) {
        let window = cx.update(|_, cx| cx.windows()[0]);
        cx.update_window(window, |_, window, cx| {
            view.update(cx, |view, cx| view.offer_tip(id, window, cx));
        })
        .expect("offer the tip");
    }

    #[gpui::test]
    fn tip_got_it_retires_it(cx: &mut TestAppContext) {
        let root = long_fixture("tip-got-it", 250);
        let (view, mut cx) = open(cx, &root);
        scroll(&mut cx, "file-list", 60.);
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), Some(TipId::Filter), "a long folder scroll suggests /");

        let button = cx.debug_bounds("tip-got-it").expect("the tip's button is laid out");
        cx.simulate_click(button.center(), Default::default());
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), None, "Got it closes the tip");
        assert!(tips_json(&cx).contains("\"filter\""), "retired in the state file: {}", tips_json(&cx));

        scroll(&mut cx, "file-list", 60.);
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), None, "shown once, never again");
    }

    #[gpui::test]
    fn tip_closes_when_the_folder_changes(cx: &mut TestAppContext) {
        let root = long_fixture("tip-folder-change", 250);
        let (view, mut cx) = open(cx, &root);
        scroll(&mut cx, "file-list", 60.);
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), Some(TipId::Filter));

        cx.simulate_keystrokes("cmd-up");
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), None, "leaving the folder closes it");

        cx.simulate_keystrokes("cmd-[");
        settle(&view, &mut cx);
        scroll(&mut cx, "file-list", 60.);
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), None, "and it was already spent");
    }

    #[gpui::test]
    fn tip_hints_off_suppresses_and_hides(cx: &mut TestAppContext) {
        let root = long_fixture("tip-hints-off", 250);
        let (view, mut cx) = open(cx, &root);
        write_config(&view, &mut cx, "settings.toml", "hints = \"off\"\n");
        scroll(&mut cx, "file-list", 60.);
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), None, "hints = off shows no tips");

        write_config(&view, &mut cx, "settings.toml", "hints = \"auto\"\n");
        scroll(&mut cx, "file-list", 60.);
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), Some(TipId::Filter), "auto allows tips");

        write_config(&view, &mut cx, "settings.toml", "hints = \"off\"\n");
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), None, "turning hints off hides a tip on screen");
    }

    #[gpui::test]
    fn tip_never_shows_while_busy(cx: &mut TestAppContext) {
        let root = long_fixture("tip-busy", 250);
        let (view, mut cx) = open(cx, &root);

        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        assert!(pane(&view, &cx).read_with(&cx, |pane, _| pane.is_renaming()), "a rename is open");
        offer(&view, TipId::Filter, &mut cx);
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), None, "no tip during a rename");
        assert!(!tips_json(&cx).contains("\"filter\""), "and it is not retired");
        cx.simulate_keystrokes("escape");
        settle(&view, &mut cx);

        for (open_keys, close_keys, open) in
            [("cmd-f", "escape", "search"), ("cmd-k", "escape", "palette"), ("cmd-alt-l", "cmd-alt-l", "activity")]
        {
            cx.simulate_keystrokes(open_keys);
            settle(&view, &mut cx);
            offer(&view, TipId::Filter, &mut cx);
            settle(&view, &mut cx);
            assert_eq!(tip(&view, &cx), None, "no tip while {open} is open");
            assert!(!tips_json(&cx).contains("\"filter\""), "{open} does not retire it");
            cx.simulate_keystrokes(close_keys);
            settle(&view, &mut cx);
        }

        scroll(&mut cx, "file-list", 60.);
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), Some(TipId::Filter), "the trigger still works after the overlays");

        cx.simulate_keystrokes("cmd-f");
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), None, "opening search closes a shown tip");
        cx.simulate_keystrokes("escape");
        settle(&view, &mut cx);
    }

    #[gpui::test]
    fn tip_only_one_on_screen(cx: &mut TestAppContext) {
        let root = long_fixture("tip-one", 250);
        let (view, mut cx) = open(cx, &root);
        scroll(&mut cx, "file-list", 60.);
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), Some(TipId::Filter));

        offer(&view, TipId::Search, &mut cx);
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), Some(TipId::Filter), "the first tip stays; the second is refused");
    }

    #[gpui::test]
    fn tip_scrolling_a_long_folder_suggests_the_filter_once(cx: &mut TestAppContext) {
        let root = long_fixture("tip-scroll", 250);
        let (view, mut cx) = open(cx, &root);
        scroll(&mut cx, "file-list", 60.);
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), Some(TipId::Filter));

        let button = cx.debug_bounds("tip-got-it").expect("the tip's button");
        cx.simulate_click(button.center(), Default::default());
        settle(&view, &mut cx);
        scroll(&mut cx, "file-list", 60.);
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), None);
    }

    #[gpui::test]
    fn tip_short_folder_scroll_does_nothing(cx: &mut TestAppContext) {
        let root = fixture("tip-short");
        let (view, mut cx) = open(cx, &root);
        scroll(&mut cx, "file-list", 60.);
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), None, "three entries are not a long folder");
    }

    #[gpui::test]
    fn tip_using_the_key_retires_it(cx: &mut TestAppContext) {
        let root = long_fixture("tip-key", 250);
        let (view, mut cx) = open(cx, &root);
        scroll(&mut cx, "file-list", 60.);
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), Some(TipId::Filter));

        cx.simulate_keystrokes("/");
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), None, "using / closes the tip");
        assert!(tips_json(&cx).contains("\"filter\""), "and retires it");
        cx.simulate_keystrokes("escape");
        settle(&view, &mut cx);
    }

    #[gpui::test]
    fn tip_scroll_in_column_view(cx: &mut TestAppContext) {
        let root = long_fixture("tip-columns", 250);
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-3");
        settle(&view, &mut cx);
        scroll(&mut cx, "column-0", 60.);
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), Some(TipId::Filter), "any column's scroll counts");
    }

    #[gpui::test]
    fn tip_two_zero_match_filters_suggest_search(cx: &mut TestAppContext) {
        let root = fixture("tip-two-filters");
        let (view, mut cx) = open(cx, &root);
        for query in ["zzz", "qqq"] {
            cx.simulate_keystrokes("/");
            settle(&view, &mut cx);
            cx.simulate_input(query);
            settle(&view, &mut cx);
            cx.simulate_keystrokes("escape");
            settle(&view, &mut cx);
        }
        assert_eq!(tip(&view, &cx), Some(TipId::Search), "two zero-match filters suggest search");
        assert_eq!(
            view.read_with(&cx, |_, cx| crate::tips::key_text(TipId::Search, cx)),
            Some("⌘F".into()),
            "the tip shows the live key"
        );
    }

    #[gpui::test]
    fn tip_zero_match_streak_resets_on_a_match(cx: &mut TestAppContext) {
        let root = fixture("tip-streak");
        let (view, mut cx) = open(cx, &root);
        for query in ["zzz", "al", "qqq"] {
            cx.simulate_keystrokes("/");
            settle(&view, &mut cx);
            cx.simulate_input(query);
            settle(&view, &mut cx);
            cx.simulate_keystrokes("escape");
            settle(&view, &mut cx);
        }
        assert_eq!(tip(&view, &cx), None, "a filter with matches resets the streak");
    }

    #[gpui::test]
    fn tip_search_key_retires_it(cx: &mut TestAppContext) {
        let root = fixture("tip-search-key");
        let (view, mut cx) = open(cx, &root);
        for query in ["zzz", "qqq"] {
            cx.simulate_keystrokes("/");
            settle(&view, &mut cx);
            cx.simulate_input(query);
            settle(&view, &mut cx);
            cx.simulate_keystrokes("escape");
            settle(&view, &mut cx);
        }
        assert_eq!(tip(&view, &cx), Some(TipId::Search));

        cx.simulate_keystrokes("cmd-f");
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), None, "using ⌘F closes the tip");
        assert!(tips_json(&cx).contains("\"search\""), "and retires it");
        cx.simulate_keystrokes("escape");
        settle(&view, &mut cx);

        for query in ["zzz", "qqq"] {
            cx.simulate_keystrokes("/");
            settle(&view, &mut cx);
            cx.simulate_input(query);
            settle(&view, &mut cx);
            cx.simulate_keystrokes("escape");
            settle(&view, &mut cx);
        }
        assert_eq!(tip(&view, &cx), None, "no second suggestion");
    }

    #[gpui::test]
    fn tip_go_to_folder_for_a_recent_folder_suggests_the_palette(cx: &mut TestAppContext) {
        let root = fixture("tip-goto");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-down");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("alpha"), "visited alpha");
        cx.simulate_keystrokes("cmd-up");
        settle(&view, &mut cx);

        let recents = cx.read(|cx| cx.global::<Ui>().palette_recents.clone());
        assert!(recents.iter().any(|p| *p == root.join("alpha")), "alpha is recent: {recents:?}");
        cx.simulate_keystrokes("cmd-shift-g cmd-a");
        cx.simulate_input(&root.join("alpha").display().to_string());
        settle(&view, &mut cx);
        assert!(
            view.read_with(&cx, |w, _| w.palette.as_ref().is_some_and(|p| p.path.is_some())),
            "the typed path lists its parent"
        );
        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("alpha"));
        assert_eq!(tip(&view, &cx), Some(TipId::GoTo), "a typed path for a recent folder suggests ⌘K");
        assert_eq!(view.read_with(&cx, |_, cx| crate::tips::key_text(TipId::GoTo, cx)), Some("⌘K".into()));
    }

    #[gpui::test]
    fn tip_go_to_folder_for_a_new_folder_does_nothing(cx: &mut TestAppContext) {
        let root = fixture("tip-goto-new");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-shift-g cmd-a");
        cx.simulate_input(&root.join("beta").display().to_string());
        settle(&view, &mut cx);
        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), root.join("beta"));
        assert_eq!(tip(&view, &cx), None, "a folder the palette did not know teaches nothing");
    }

    #[gpui::test]
    fn tip_palette_key_retires_it(cx: &mut TestAppContext) {
        let root = fixture("tip-goto-key");
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-down");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("cmd-up");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("cmd-shift-g cmd-a");
        cx.simulate_input(&root.join("alpha").display().to_string());
        settle(&view, &mut cx);
        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), Some(TipId::GoTo));

        cx.simulate_keystrokes("cmd-k");
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), None, "using ⌘K closes the tip");
        assert!(tips_json(&cx).contains("\"go-to\""), "and retires it");
        cx.simulate_keystrokes("escape");
        settle(&view, &mut cx);
    }

    #[gpui::test]
    fn tip_state_survives_reload(cx: &mut TestAppContext) {
        let root = long_fixture("tip-reload", 250);
        let (view, mut cx) = open(cx, &root);
        scroll(&mut cx, "file-list", 60.);
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), Some(TipId::Filter));
        let button = cx.debug_bounds("tip-got-it").expect("the tip's button");
        cx.simulate_click(button.center(), Default::default());
        settle(&view, &mut cx);
        assert!(tips_json(&cx).contains("\"filter\""));

        cx.update(|_, cx| crate::tips::reload_for_tests(cx));
        scroll(&mut cx, "file-list", 60.);
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), None, "the retired set survives a relaunch");

        // The relaunch cleared the cooldown, so a never-shown tip can still appear.
        for query in ["zzz", "qqq"] {
            cx.simulate_keystrokes("/");
            settle(&view, &mut cx);
            cx.simulate_input(query);
            settle(&view, &mut cx);
            cx.simulate_keystrokes("escape");
            settle(&view, &mut cx);
        }
        assert_eq!(tip(&view, &cx), Some(TipId::Search));
    }

    #[gpui::test]
    fn tip_filter_dropped_by_navigation_counts(cx: &mut TestAppContext) {
        let root = fixture("tip-filter-nav");
        let (view, mut cx) = open(cx, &root);
        for _ in 0..2 {
            cx.simulate_keystrokes("/");
            settle(&view, &mut cx);
            cx.simulate_input("zzz");
            settle(&view, &mut cx);
            cx.simulate_keystrokes("cmd-up");
            settle(&view, &mut cx);
        }
        assert_eq!(tip(&view, &cx), Some(TipId::Search), "a filter dropped by navigation ends the session too");
    }

    #[gpui::test]
    fn tip_closes_when_a_tab_is_clicked(cx: &mut TestAppContext) {
        let root = long_fixture("tip-tab-click", 250);
        let (view, mut cx) = open(cx, &root);
        cx.simulate_keystrokes("cmd-t");
        settle(&view, &mut cx);
        scroll(&mut cx, "file-list", 60.);
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), Some(TipId::Filter));

        let bounds = cx.debug_bounds("tab-0").expect("other tab on screen");
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), None, "switching tabs by click closes the tip");
        assert_eq!(view.read_with(&cx, |_, cx| crate::tips::owner(cx)), None, "and frees it app-wide");
    }

    #[gpui::test]
    fn tip_sits_above_the_bar_at_the_right_edge(cx: &mut TestAppContext) {
        let root = long_fixture("tip-layout", 250);
        let (view, mut cx) = open(cx, &root);
        scroll(&mut cx, "file-list", 60.);
        settle(&view, &mut cx);
        let tip = cx.debug_bounds("tip").expect("the tip is laid out");
        let viewport = cx.update(|window, _| window.viewport_size());
        assert_eq!(tip.right(), viewport.width - px(12.), "12 px from the right edge");
        assert_eq!(tip.bottom(), viewport.height - px(crate::bottom_bar::HEIGHT + 8.), "8 px above the bottom bar");
        assert!(tip.size.width >= px(280.) && tip.size.width <= px(440.), "{:?}", tip.size);
    }
}
