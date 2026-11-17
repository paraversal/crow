//! One Crow window: title bar, tabs, sidebar, the active pane, preview and path bar.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crow_core::motion::Fx;
use crow_core::{SidebarEdit, Tabs, TipId, TipsMode, ViewMode, ZeroMatchStreak};
use gpui::{
    App, Bounds, Context, CursorStyle, Entity, FocusHandle, Focusable, FontWeight, IntoElement, Render, Role,
    ScrollHandle, SharedString, Subscription, TitlebarOptions, Toggled, Window, WindowBounds, WindowOptions, div, img,
    point, prelude::*, px, size, transparent_black,
};

use crate::actions::{
    AddToSidebar, ApplyThemeEverywhere, CloseTab, EditPath, GoBack, GoForward, GoToFolder, KeyboardShortcuts, Minimize,
    MoveDown, MoveToTrash, MoveUp, NewTab, NewWindow, NextTab, OpenParent, PaletteOpenInTab, PathHover, PathUnhover,
    PreviousTab, ResetFolderView, SearchEverything, ShowCommandPalette, ShowSearch, ShowThemePicker, SwitchFocus,
    ToggleHiddenFiles, TogglePreview, UseViewAsDefaults, ViewAsColumns, ViewAsList, WhyThisView, Zoom,
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
use crate::style::{
    BAND_HEIGHT, active_selection, color, color_alpha, cursor_ring, highlight_text, icon, truncate_middle_with_ranges,
    ui_font,
};
use crate::text_input::{TextInput, TextInputEvent};
use crate::ui::{self, SettingChange, Ui, reload_theme};
use crow_fs::Entry;
use crow_fs::ops::{Plan, Request, Resolution};
use std::collections::HashMap;

/// Open a window on `dir`. New windows share one size from settings and step
/// 24 pt down and right from the previous one, so they never land at random sizes.
/// The folder Crow was started on. With no window open, the Dock icon and a ⌘Q that
/// has to ask about a running operation open a window here.
pub struct LaunchFolder(pub PathBuf);

impl gpui::Global for LaunchFolder {}

pub fn launch_folder(cx: &App) -> PathBuf {
    cx.try_global::<LaunchFolder>()
        .map(|folder| folder.0.clone())
        .unwrap_or_else(|| std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from))
}

pub fn open(
    dir: PathBuf,
    bench: bool,
    bench_filter: Option<String>,
    cx: &mut App,
) -> Option<gpui::WindowHandle<CrowWindow>> {
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
    cx.open_window(options, |window, cx| cx.new(|cx| CrowWindow::new(dir, bench, bench_filter, window, cx)))
        .inspect_err(|e| tracing::error!("open window: {e}"))
        .ok()
}

/// Open a path from the command line: a folder in a new window, a file in its
/// folder with the file selected. `is_dir` comes from the caller's background stat.
pub fn open_path(path: PathBuf, is_dir: bool, cx: &mut App) {
    let (dir, reveal) = if is_dir {
        (path, None)
    } else {
        let parent = path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("/"));
        (parent, Some(path))
    };
    let Some(handle) = open(dir, false, None, cx) else { return };
    cx.defer(move |cx| {
        handle
            .update(cx, |this, window, cx| {
                if let Some(path) = reveal {
                    this.pane().update(cx, |pane, cx| pane.reveal(path, cx));
                }
                window.activate_window();
            })
            .ok();
    });
}

/// Apply the global `show_hidden_files` setting to every open browser window.
/// Idempotent, so a settings write and its reload can both call it.
pub fn apply_show_hidden(show: bool, cx: &mut App) {
    for handle in cx.windows() {
        let Some(handle) = handle.downcast::<CrowWindow>() else { continue };
        handle
            .update(cx, |window, _, cx| {
                if window.show_hidden == show {
                    return;
                }
                window.show_hidden = show;
                for (_, pane) in window.tabs.iter() {
                    pane.update(cx, |pane, cx| pane.set_show_hidden(show, cx));
                }
                if let Some(search) = &window.search {
                    search.update(cx, |search, cx| search.set_show_hidden(show, cx));
                }
                cx.notify();
            })
            .ok();
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
    /// ⌘L or a double-click on empty path space: the bar's path slot is a field.
    path_edit: Option<PathEdit>,
    /// The path bar's hover state, so the pill can fade.
    path_hovered: bool,
    /// A deferred re-focus into an open overlay is already scheduled.
    refocus_overlay: bool,
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
    choice: crate::theme_choice::ThemeChoice,
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

/// ⌘L or a double-click on empty path space: the bottom bar's path slot becomes
/// a field. Escape and a focus loss put the segments back without navigating.
struct PathEdit {
    input: Entity<TextInput>,
    /// Where Escape returns focus, if the window had one when editing began.
    return_focus: Option<FocusHandle>,
    /// The bar error this field set for a missing path, if it is still the one shown.
    refused: Option<u64>,
    _subscriptions: Vec<Subscription>,
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
const PALETTE_TOP: f32 = BAND_HEIGHT;
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
    children: Option<Vec<palette::PathEntry>>,
    /// The parent's files (and packages), listed with `children`.
    files: Vec<palette::PathEntry>,
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
        let mut sheet = None;
        if !bench && crate::onboarding::take_pending(cx) {
            let focus = cx.focus_handle();
            let show = cx.focus_handle();
            let no = cx.focus_handle();
            window.focus(&focus, cx);
            sheet = Some(Sheet::Onboarding { focus, show, no });
        }
        let owner = cx.entity_id();
        let subscriptions = vec![
            cx.observe(&pane, |_, _, cx| cx.notify()),
            cx.subscribe_in(&pane, window, |this, _, event, window, cx| this.on_pane_event(event, window, cx)),
            cx.subscribe_in(&sidebar, window, |this, _, event, window, cx| this.on_sidebar_event(event, window, cx)),
            cx.observe_window_appearance(window, |_, _, cx| reload_theme(cx)),
            // Another window coming to the front mid-drag must be able to receive it,
            // even while the pointer stays inside Crow's bounds.
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    window.promote_active_drag_to_platform(cx);
                    // The pointer can stay over the path while another app is active;
                    // the pill must not stay on screen with it.
                    this.path_hovered = false;
                    cx.notify();
                }
            }),
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
            sheet,
            show_activity: false,
            why: None,
            picker: None,
            palette: None,
            search: None,
            path_edit: None,
            path_hovered: false,
            refocus_overlay: false,
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

    /// The focus surfaces the topmost overlay owns, in the precedence the fuzzer's
    /// focus invariant uses. Empty when no overlay is open: the pane, the sidebar
    /// and their text fields own focus then.
    fn overlay_focus_surfaces(&self, cx: &App) -> Vec<(&'static str, FocusHandle)> {
        let mut allowed: Vec<(&'static str, FocusHandle)> = Vec::new();
        if let Some(sheet) = &self.sheet {
            allowed.push(("sheet", sheet.focus().clone()));
            // The first-launch sheet's two choices are its tab ring; the render
            // invariant must not pull focus back to the panel.
            if let Sheet::Onboarding { show, no, .. } = sheet {
                allowed.push(("sheet choice", show.clone()));
                allowed.push(("sheet choice", no.clone()));
            }
        } else if let Some(palette) = &self.palette {
            allowed.push(("palette", palette.input.read(cx).focus_handle(cx)));
        } else if let Some(picker) = &self.picker {
            allowed.push(("picker", picker.focus.clone()));
        } else if let Some(why) = &self.why {
            allowed.push(("why", why.clone()));
        } else if let Some(search) = &self.search {
            let search = search.read(cx);
            allowed.push(("search field", search.input_focus_handle(cx)));
            allowed.push(("search results", search.results_focus_handle()));
            // The documented Tab cycle goes field -> results -> sidebar -> field.
            allowed.push(("sidebar", self.sidebar.read(cx).focus_handle(cx)));
            // ⌘L can edit the path while search is open; the bar's field owns focus then.
            if let Some(edit) = &self.path_edit {
                allowed.push(("path field", edit.input.read(cx).focus_handle(cx)));
            }
        }
        allowed
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
        if let Some(closed) = self.tabs.close_active() {
            // Free the closed tab's 100k listing off the main thread.
            closed.update(cx, |pane, cx| pane.release_listings(cx));
        } else {
            // The last tab closes the window; Crow stays running with no windows.
            // Release its listing before the window drops.
            self.pane().update(cx, |pane, cx| pane.release_listings(cx));
            window.remove_window();
            return;
        }
        self.focus_center(window, cx);
        cx.notify();
    }

    fn ask_to_stop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.sheet, Some(Sheet::StopOperation { .. })) {
            return;
        }
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.sheet = Some(Sheet::StopOperation { focus });
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
        let show = !cx.global::<Ui>().settings.show_hidden_files;
        // This window is already borrowed: update it directly, then let the
        // setting update the other windows (its own handle update fails safely).
        self.show_hidden = show;
        for (_, pane) in self.tabs.iter() {
            pane.update(cx, |pane, cx| pane.set_show_hidden(show, cx));
        }
        if let Some(search) = &self.search {
            search.update(cx, |search, cx| search.set_show_hidden(show, cx));
        }
        crate::ui::set_setting(crate::ui::SettingChange::ShowHiddenFiles(show), cx);
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

    /// ⌘L: edit the path the bar shows in the bottom bar. A second ⌘L selects
    /// the field's text again; overlays and a rename own the screen, so they keep it.
    fn edit_path(&mut self, _: &EditPath, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(edit) = &self.path_edit {
            let input = edit.input.clone();
            let len = input.read(cx).text().len();
            input.update(cx, |input, cx| input.select_range(0..len, window, cx));
            let focus = input.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            return;
        }
        if self.sheet.is_some()
            || self.why.is_some()
            || self.picker.is_some()
            || self.palette.is_some()
            || self.pane().read(cx).is_renaming()
        {
            return;
        }
        let typed = {
            let pane = self.pane().read(cx);
            let (target, _) = self.bar_target(pane, cx);
            home_relative(&target)
        };
        let input = cx.new(|cx| TextInput::new(typed.clone(), "Path", cx).bare().with_aria_label("Path"));
        input.update(cx, |input, cx| input.select_range(0..typed.len(), window, cx));
        let subscription = cx.subscribe_in(&input, window, |this, _input, event, window, cx| match event {
            TextInputEvent::Confirmed => this.confirm_path_edit(window, cx),
            TextInputEvent::Cancelled => this.close_path_edit(true, window, cx),
            TextInputEvent::CompletionRequested => this.complete_path_edit(window, cx),
            TextInputEvent::Changed => this.complete_path_preview(window, cx),
            TextInputEvent::BackspaceAtStart => {}
        });
        let focus = input.read(cx).focus_handle(cx);
        let blur = cx.on_focus_out(&focus, window, |this, _event, window, cx| {
            this.close_path_edit(false, window, cx);
        });
        let return_focus = window.focused(cx);
        self.path_edit =
            Some(PathEdit { input, return_focus, refused: None, _subscriptions: vec![subscription, blur] });
        window.focus(&focus, cx);
        cx.notify();
    }

    /// Leave edit mode and put the segments back. Escape returns focus to where it
    /// was; a click elsewhere and a focus loss leave focus where the user put it.
    fn close_path_edit(&mut self, restore_focus: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.path_edit.take() else { return };
        // The field replaced the path area, so its hover state is stale; the pill
        // fades back in on the next hover instead of sticking where the pointer left.
        self.path_hovered = false;
        // The refusal described the typed path; it goes with the field, unless
        // something newer replaced it.
        if let Some(generation) = edit.refused
            && cx.global::<Operations>().message_generation == generation
        {
            operations::clear_message(cx);
        }
        if restore_focus {
            match &edit.return_focus {
                Some(focus) => window.focus(focus, cx),
                None => self.focus_center(window, cx),
            }
        }
        cx.notify();
    }

    /// Return in the path field: go to the folder, or reveal the file. A path that
    /// doesn't exist beeps and leaves the field open, with the reason in the bar.
    fn confirm_path_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = &self.path_edit else { return };
        let input = edit.input.clone();
        let typed = input.read(cx).text().to_owned();
        let query = typed.clone();
        let cwd = self.pane().read(cx).current_dir(cx);
        let home = cx.global::<Ui>().home.clone();
        let task = cx.background_spawn(async move {
            let cleaned = crow_core::clean_pasted_path(&query).unwrap_or_else(|| query.clone());
            let target = crow_core::resolve_typed(&cleaned, &cwd, &home);
            // io-ok: inside background_spawn
            match std::fs::metadata(&target) {
                Ok(meta) => {
                    // io-ok: inside background_spawn
                    let is_dir = meta.is_dir();
                    Ok((is_dir, target))
                }
                Err(error) => Err((query, error.to_string())),
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let found = task.await;
            this.update_in(cx, |this, window, cx| {
                let Some(edit) = &this.path_edit else { return };
                if edit.input.read(cx).text() != typed {
                    return;
                }
                match found {
                    Ok((is_dir, target)) => {
                        this.close_path_edit(false, window, cx);
                        if is_dir {
                            if this.pane().read(cx).current_dir(cx) != target {
                                this.pane().update(cx, |pane, cx| pane.navigate(target, cx));
                            }
                        } else {
                            this.pane().update(cx, |pane, cx| pane.reveal(target, cx));
                        }
                        this.focus_center(window, cx);
                        cx.notify();
                    }
                    Err((typed, error)) => {
                        crate::feedback::refuse(crate::feedback::RefuseTarget::Field(&input), window, cx);
                        operations::message(format!("Can't open “{typed}”: {error}"), true, cx);
                        if let Some(edit) = &mut this.path_edit {
                            edit.refused = Some(cx.global::<Operations>().message_generation);
                        }
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Tab in the path field: complete the last component from disk, like ⌘K.
    fn complete_path_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = &self.path_edit else { return };
        let input = edit.input.clone();
        let query = input.read(cx).text().to_owned();
        let task = self.complete_typed_path(query.clone(), cx);
        cx.spawn_in(window, async move |this, cx| {
            let completed = task.await;
            this.update_in(cx, |this, window, cx| {
                let Some(edit) = &this.path_edit else { return };
                if edit.input.read(cx).text() != query {
                    return;
                }
                if let Some(completed) = completed {
                    input.update(cx, |input, cx| {
                        input.set_text(completed, window, cx);
                        input.set_completion(None, cx);
                    });
                }
            })
            .ok();
        })
        .detach();
    }

    /// Recompute the field's muted completion preview as the typed path changes.
    fn complete_path_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = &self.path_edit else { return };
        let input = edit.input.clone();
        let typed = input.read(cx).text().to_owned();
        let task = self.complete_typed_path(typed.clone(), cx);
        cx.spawn_in(window, async move |this, cx| {
            let completed = task.await;
            this.update_in(cx, |this, window, cx| {
                let Some(edit) = &this.path_edit else { return };
                if edit.input.read(cx).text() != typed {
                    return;
                }
                let suffix = completed.and_then(|completed| {
                    let prefix = completed.get(..typed.len())?;
                    prefix.eq_ignore_ascii_case(&typed).then(|| SharedString::from(&completed[typed.len()..]))
                });
                input.update(cx, |input, cx| input.set_completion(suffix, cx));
                let _ = window;
            })
            .ok();
        })
        .detach();
    }

    /// Tab completion for a typed path, off the main thread. The palette and the
    /// bar's field share it.
    fn complete_typed_path(&self, typed: String, cx: &mut Context<Self>) -> gpui::Task<Option<String>> {
        let cwd = self.pane().read(cx).current_dir(cx);
        let home = cx.global::<Ui>().home.clone();
        cx.background_spawn(async move { crow_core::complete_typed(&typed, &cwd, &home) })
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
            let mut button = div()
                .id(id)
                .debug_selector(move || id.into())
                .role(Role::Button)
                .aria_label(aria)
                .aria_disabled(!enabled)
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
            .debug_selector(|| "toolbar".into())
            .role(Role::Toolbar)
            .aria_label("Toolbar")
            .h(px(BAND_HEIGHT))
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
        if self.path_edit.is_some() {
            return BarFocus::PathEdit;
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
        // A pane's load error names its folder; a newline there reaches GPUI's shaping.
        let error = error.map(|error| BarError {
            text: crate::style::display_text(&error.text).into_owned().into(),
            file: error.file,
        });
        let focus = self.bar_focus(window, pane, cx);
        // A typing run is the list's own state; the bar shows it only while the list owns the keys.
        let type_select = (focus == BarFocus::List).then(|| pane.type_select_prefix(cx)).flatten().map(Into::into);

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

        // Q8: search mode shows the selected result's path; otherwise the cursor's.
        let (target, last_is_dir) = self.bar_target(pane, cx);
        let parts: Vec<(String, std::path::PathBuf)> = crate::style::display_components(&target)
            .iter()
            .map(|(name, path)| (crate::style::display_text(name).into_owned(), path.clone()))
            .collect();
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
                BarFocus::PathEdit => KeyScope::PathBar,
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

        let (progress_done, progress_id) = ops.progress_done.map_or((None, 0), |(fraction, id)| (Some(fraction), id));
        let mut state = BarState {
            primary,
            type_select,
            error,
            count,
            path,
            editing: self.path_edit.as_ref().map(|edit| edit.input.clone()),
            transient_max: None,
            message_id: ops.message_generation,
            progress_done,
            progress_id,
        };
        // Narrow windows lose hints from the right and fold the path's middle (I7).
        bottom_bar::fit_to_window(&mut state, window);
        state
    }

    /// The path the bar shows and whether its last segment is a folder: the
    /// selected search result, else the cursor entry, else the folder. ⌘L edits
    /// this path, so entering edit mode never changes what the bar says.
    fn bar_target(&self, pane: &Pane, cx: &App) -> (PathBuf, bool) {
        if let Some(entry) = self.search.as_ref().and_then(|search| search.read(cx).selected_entry()) {
            return entry;
        }
        pane.cursor_entry(cx).map_or_else(|| (pane.current_dir(cx), true), |entry| (entry.path, entry.is_dir))
    }

    fn render_bottom_bar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let state = self.bar_state(window, cx);
        bottom_bar::render(state, self.pane().clone(), self.path_hovered, cx)
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
            self.pane().update(cx, |pane, cx| pane.set_tip_inset(true, cx));
            cx.notify();
        }
    }

    fn close_tip(&mut self, cx: &mut Context<Self>) {
        if self.tip.take().is_some() {
            self.pane().update(cx, |pane, cx| pane.set_tip_inset(false, cx));
            crate::tips::closed(cx.entity_id(), cx);
            cx.notify();
        }
    }

    /// "Don't show tips" on the card: close it and mute every later tip. Key
    /// hints stay as the `hints` setting has them.
    fn mute_tips(&mut self, cx: &mut Context<Self>) {
        self.close_tip(cx);
        ui::set_setting(SettingChange::Tips(TipsMode::Off), cx);
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
        paths: Arc<[PathBuf]>,
        focus: FocusHandle,
    },
    /// Quitting would stop the running operation partway.
    StopOperation {
        focus: FocusHandle,
    },
    /// The first launch asks whether tips should show.
    Onboarding {
        focus: FocusHandle,
        show: FocusHandle,
        no: FocusHandle,
    },
}

impl Sheet {
    fn focus(&self) -> &FocusHandle {
        match self {
            Sheet::Conflicts { focus, .. }
            | Sheet::ConfirmDelete { focus, .. }
            | Sheet::StopOperation { focus, .. }
            | Sheet::Onboarding { focus, .. } => focus,
        }
    }
}

/// Quit, unless an operation is running: then the active Crow window asks first.
/// With no window open, one opens at the launch folder to ask.
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
            if handle.update(cx, |this, window, cx| this.ask_to_stop(window, cx)).is_err() {
                cx.quit();
            }
        }),
        None => match open(launch_folder(cx), false, None, cx) {
            Some(handle) => cx.defer(move |cx| {
                if handle.update(cx, |this, window, cx| this.ask_to_stop(window, cx)).is_err() {
                    stop_then_quit(cx);
                }
            }),
            None => stop_then_quit(cx),
        },
    }
}

/// Cancel the running operation, wait for it to wind down (its undo record lands in
/// the log), then quit.
fn stop_then_quit(cx: &mut App) {
    operations::cancel_running(cx);
    cx.spawn(async move |cx| {
        while cx.update(|cx| cx.global::<Operations>().running.is_some()) {
            cx.background_executor().timer(std::time::Duration::from_millis(30)).await;
        }
        cx.update(|cx| cx.quit());
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
                // Navigating behind the field (a sidebar click, a tab) leaves it stale.
                self.close_path_edit(false, window, cx);
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
                let (paths, cut) = operations::pasteboard_paths(cx);
                if paths.is_empty() {
                    return;
                }
                if cut {
                    cx.global_mut::<Operations>().clipboard = None;
                    self.notify_views(cx);
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
        if rename_result {
            // The new item must be visible: a filter that hides it would hide the
            // rename field that follows (findings #10).
            self.pane().update(cx, |pane, cx| pane.clear_filter_for_new_item(cx));
        }
        let trash_paths = match &plan.request {
            Request::Trash { paths } => Some(&paths[..]),
            _ => None,
        };
        let move_paths = match &plan.request {
            Request::Move { paths, into } => Some((&paths[..], into.as_path())),
            _ => None,
        };
        // Only the error callback needs an owned copy; the effects borrow the plan.
        let failed_paths = match &plan.request {
            Request::Trash { paths }
            | Request::Move { paths, .. }
            | Request::Copy { paths, .. }
            | Request::Duplicate { paths } => Some(paths.clone()),
            _ => None,
        };
        let pane = self.pane().downgrade();
        // The effect starts with the operation, not the key press: a conflict or a
        // delete sheet may have come first, and a second operation is refused.
        if cx.global::<Operations>().running.is_none() {
            if let Some(paths) = trash_paths {
                pane.update(cx, |pane, cx| pane.start_effect(paths, Fx::Trash, cx)).ok();
            }
            if let Some((paths, into)) = move_paths {
                pane.update(cx, |pane, cx| pane.start_effect(paths, Fx::MoveOut, cx)).ok();
                // The folder the items land in pulses, in the list and in the sidebar.
                let into = into.to_path_buf();
                pane.update(cx, |pane, cx| pane.start_effect(std::slice::from_ref(&into), Fx::Receive, cx)).ok();
                self.sidebar.update(cx, |sidebar, cx| sidebar.pulse(into, cx));
            }
        }
        operations::execute(plan, resolutions, cx, move |record, error, cx| {
            if error.is_some()
                && let Some(paths) = &failed_paths
            {
                // Whatever is still there refuses.
                pane.update(cx, |pane, cx| pane.start_effect(paths, Fx::Refuse, cx)).ok();
            }
            let results = record.results();
            pane.update(cx, |pane, cx| pane.show_results(results, rename_result, cx)).ok();
        });
    }

    fn undo(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        // Undo ends the cut mark; the clipboard itself stays for a later paste.
        if let Some(clipboard) = &mut cx.global_mut::<Operations>().clipboard {
            clipboard.marked = false;
        }
        self.notify_views(cx);
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
        // Escape or a click outside the first-launch sheet keeps the default:
        // record that it was asked, leave `tips` as the file has it.
        if matches!(self.sheet, Some(Sheet::Onboarding { .. })) {
            let show = cx.global::<Ui>().settings.tips == TipsMode::Auto;
            crate::onboarding::answer(show, cx);
        }
        self.sheet = None;
        self.focus_center(window, cx);
        cx.notify();
    }

    /// The first-launch sheet's answer: record it, write `tips`, close.
    fn choose_tips(&mut self, show: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.sheet = None;
        crate::onboarding::answer(show, cx);
        ui::set_setting(SettingChange::Tips(if show { TipsMode::Auto } else { TipsMode::Off }), cx);
        self.focus_center(window, cx);
        cx.notify();
    }

    fn move_onboarding_focus(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Sheet::Onboarding { show, no, .. }) = &self.sheet else { return };
        let (show, no) = (show.clone(), no.clone());
        let target = if forward {
            if show.is_focused(window) { no } else { show }
        } else if no.is_focused(window) {
            show
        } else {
            no
        };
        window.focus(&target, cx);
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
            Some(Sheet::StopOperation { .. }) => {
                stop_then_quit(cx);
                return;
            }
            Some(Sheet::Onboarding { no, .. }) => {
                // Return activates the focused choice; the panel focus means the
                // default, Show tips.
                let show_tips = !no.is_focused(window);
                self.choose_tips(show_tips, window, cx);
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

    fn render_sheet(&self, window: &Window, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let sheet = self.sheet.as_ref()?;
        let theme = cx.global::<Ui>().theme.clone();
        let (fg, muted, accent) = (theme.role("fg"), theme.role("fg-muted"), theme.role("accent"));
        let trash_hint = match keys::text(&MoveToTrash, KeyScope::List, cx) {
            Some(keys) => format!("This can't be undone. Move to Trash ({keys}) can."),
            None => "This can't be undone. Move to Trash can.".to_owned(),
        };
        let button = |id: &'static str, label: String, primary: bool, focus: Option<&FocusHandle>| {
            let focused = focus.is_some_and(|focus| focus.is_focused(window));
            div()
                .id(id)
                .debug_selector(move || id.into())
                .role(Role::Button)
                .aria_label(label.clone())
                .when_some(focus, |button, focus| button.track_focus(focus).tab_stop(true))
                .when(focused, |button| button.shadow(cursor_ring(&theme)))
                .px_3()
                .py_1()
                .rounded_md()
                .bg(color(if primary { accent } else { theme.role("surface-raised") }))
                .text_color(color(if primary { theme.role("on-accent") } else { fg }))
                .child(label)
        };
        let (title, body): (String, gpui::AnyElement) = match sheet {
            Sheet::ConfirmDelete { paths, .. } => {
                let what = match &paths[..] {
                    [one] => {
                        let name = one.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                        format!("“{}”", crate::style::display_text(&name))
                    }
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
                                    button("sheet-cancel", "Cancel".into(), false, None)
                                        .on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx))),
                                )
                                .child(
                                    button("sheet-confirm", "Delete".into(), true, None)
                                        .on_click(cx.listener(|this, _, window, cx| this.confirm_sheet(window, cx))),
                                ),
                        )
                        .into_any_element(),
                )
            }
            Sheet::StopOperation { .. } => {
                let label = cx
                    .global::<Operations>()
                    .running
                    .as_ref()
                    .map_or_else(|| "The operation".to_owned(), |r| operations::running_label(&r.label));
                let (verb, confirm) = ("Quitting", "Stop and Quit");
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
                                    button("sheet-cancel", "Keep Running".into(), false, None)
                                        .on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx))),
                                )
                                .child(
                                    button("sheet-confirm", confirm.into(), true, None)
                                        .on_click(cx.listener(|this, _, window, cx| this.confirm_sheet(window, cx))),
                                ),
                        )
                        .into_any_element(),
                )
            }
            Sheet::Onboarding { show, no, .. } => (
                "Show tips as you go?".to_owned(),
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(div().text_color(color(muted)).child(
                        "Crow can point out a few keys the first time they matter. You can change this later in Settings.",
                    ))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                button("onboarding-no", "No tips".into(), false, Some(no))
                                    .on_click(cx.listener(|this, _, window, cx| this.choose_tips(false, window, cx))),
                            )
                            .child(
                                button("onboarding-show", "Show tips".into(), true, Some(show))
                                    .on_click(cx.listener(|this, _, window, cx| this.choose_tips(true, window, cx))),
                            ),
                    )
                    .into_any_element(),
            ),
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
                            item.target.file_name().map_or_else(String::new, |name| {
                                crate::style::display_text(&name.to_string_lossy()).into_owned()
                            }),
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
                                .child(button("all-keep", "Keep Both for All".into(), false, None).on_click(
                                    cx.listener(|this, _, _, cx| this.set_choice(None, Resolution::KeepBoth, cx)),
                                ))
                                .child(button("all-skip", "Skip All".into(), false, None).on_click(
                                    cx.listener(|this, _, _, cx| this.set_choice(None, Resolution::Skip, cx)),
                                ))
                                .child(div().flex_1())
                                .child(
                                    button("sheet-cancel", "Cancel".into(), false, None)
                                        .on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx))),
                                )
                                .child(
                                    button("sheet-confirm", "Continue".into(), true, None)
                                        .on_click(cx.listener(|this, _, window, cx| this.confirm_sheet(window, cx))),
                                ),
                        )
                        .into_any_element(),
                )
            }
        };
        let onboarding_tab = cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
            if event.keystroke.key == "tab" {
                this.move_onboarding_focus(!event.keystroke.modifiers.shift, window, cx);
                cx.stop_propagation();
            }
        });
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
                .when(matches!(sheet, Sheet::Onboarding { .. }), |panel| panel.on_key_down(onboarding_tab))
                .p_4()
                // The first-launch question sits a little lower than the other
                // sheets, so it reads as an invitation rather than an alert.
                .when(matches!(sheet, Sheet::Onboarding { .. }), |panel| panel.mt_2())
                .flex()
                .flex_col()
                .gap_3()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                .child(body),
            &theme,
            cx.global::<Ui>().feedback.mode,
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
                // Rename labels carry both names; errors carry the path at fault.
                let label = crate::style::display_text(&e.record.label).into_owned();
                let state = crate::style::display_text(&state).into_owned();
                (i, label, format!("{} {state}", when.format("%H:%M")), e.record.is_undoable() && !e.undone)
            })
            .collect();
        div()
            .id("activity")
            .role(Role::List)
            .aria_label("Activity")
            .absolute()
            .top(px(BAND_HEIGHT))
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
            SidebarEvent::OpenInNewTab(dir) => {
                if self.search.is_some() {
                    self.close_search(window, cx);
                }
                self.open_tab_at(dir.clone(), window, cx);
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
                crate::feedback::refuse(crate::feedback::RefuseTarget::None, window, cx);
                // A typo, not a lasting problem: an info message that retires.
                operations::message(format!("No mark {key}"), false, cx);
                cx.notify();
            }
        }
    }

    /// Finder's File ▸ Add to Sidebar: the selected folders, or the folder shown.
    fn add_to_sidebar(&mut self, _: &AddToSidebar, _: &mut Window, cx: &mut Context<Self>) {
        let pane = self.pane().read(cx);
        let selected: Vec<PathBuf> =
            pane.selected_entries(cx).iter().filter(|(_, is_dir)| *is_dir).map(|(path, _)| path.clone()).collect();
        let paths = if selected.is_empty() { vec![pane.current_dir(cx)] } else { selected };
        let at = cx.global::<Ui>().favorites().first().map_or(0, |(_, f)| f.len());
        ui::edit_sidebar(SidebarEdit::AddMany { paths, at: (0, at) }, cx);
    }

    /// The path counts the fuzz runner's bulk tag needs: the focused folder's
    /// selection and entries, the clipboard, and the next undo/redo record.
    #[cfg(feature = "fuzz")]
    pub fn fuzz_path_counts(&self, cx: &App) -> (usize, usize, usize, usize, usize) {
        let pane = self.pane().read(cx);
        let ops = cx.global::<Operations>();
        (pane.selected_count(cx), pane.item_count(cx), ops.clipboard_paths(), ops.undo_paths(), ops.redo_paths())
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
        let folder = crate::style::display_path(&dir, &ui.home);
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
                .debug_selector(move || id.into())
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
                        .child(button("why-open-file", "Edit views.toml").on_click(move |_, _, cx| {
                            if let Err(e) = crate::effects::effects(cx).open_text(&views_file) {
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
            cx.global::<Ui>().feedback.mode,
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
        let current = cx.global::<Ui>().settings.theme.clone();
        let choice = crate::theme_choice::ThemeChoice::new(infos, &current);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.picker = Some(ThemePicker { focus, choice });
        cx.notify();
    }

    fn move_picker(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(picker) = &mut self.picker else { return };
        if picker.choice.move_cursor(delta) {
            crate::theme_choice::preview(&picker.choice.current_id(), cx);
            cx.notify();
        }
    }

    fn select_picker(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(picker) = &mut self.picker else { return };
        if picker.choice.select(index) {
            crate::theme_choice::preview(&picker.choice.current_id(), cx);
            cx.notify();
        }
    }

    fn confirm_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = self.picker.take() else { return };
        ui::choose_theme(&picker.choice.current_id(), cx);
        self.focus_center(window, cx);
        cx.notify();
    }

    fn cancel_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.picker = None;
        crate::theme_choice::clear_preview(cx);
        self.focus_center(window, cx);
        cx.notify();
    }

    /// "Apply to All Apps": the chosen theme becomes the global pick for its variant,
    /// and Crow follows "System" from then on.
    fn apply_everywhere(&mut self, _: &ApplyThemeEverywhere, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = self.picker.take() else { return };
        match picker.choice.entries.get(picker.choice.cursor) {
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

    /// Redraw every pane and the search results after app-wide clipboard state
    /// changes (a copy's flash, a cut's mark, Escape/undo clearing it).
    fn notify_views(&self, cx: &mut App) {
        for (_, pane) in self.tabs.iter() {
            pane.update(cx, |_, cx| cx.notify());
        }
        if let Some(search) = &self.search {
            search.update(cx, |_, cx| cx.notify());
        }
    }

    fn copy_paths(&mut self, paths: Arc<[PathBuf]>, cut: bool, cx: &mut Context<Self>) {
        /// Above this many paths the system pasteboard write blocks the main
        /// thread for seconds and 100k items wedge the pasteboard server; Crow's
        /// own clipboard still pastes them.
        const PASTEBOARD_MAX: usize = 1000;
        let n = paths.len();
        let effects = crate::effects::effects(cx);
        let (change_count, crow_only) = if n <= PASTEBOARD_MAX {
            (effects.write_file_urls(&paths), false)
        } else {
            // Remember the count without writing: a paste uses our paths until
            // another app copies (its count differs, so the pasteboard is read).
            (effects.pasteboard_change_count(), true)
        };
        cx.global_mut::<Operations>().clipboard = Some(Clipboard::new(paths, cut, change_count));
        let what = if n == 1 { "1 item".to_owned() } else { format!("{n} items") };
        let message = match (cut, crow_only) {
            (false, false) => format!("Copied {what}"),
            (false, true) => format!("Copied {what} (Crow only)"),
            (true, false) => format!("Cut {what}: paste to move"),
            (true, true) => format!("Cut {what}: paste to move (Crow only)"),
        };
        operations::message(message, false, cx);
        self.notify_views(cx);
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
                    crate::feedback::refuse(crate::feedback::RefuseTarget::Field(input), window, cx);
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
        // A 100k-file clipboard is ~10 MB; scoring the folder or shaping its
        // missing-path label with that blocked the main thread for 30 s
        // (findings #25). The field keeps the text; the listing uses its head.
        let query: String = query.chars().take(palette::MAX_QUERY_CHARS).collect();
        let query = query.as_str();
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
                            files: Vec::new(),
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
                    (Some(children), false) => {
                        palette::path_listing(&state.parent, &state.partial, children, &state.files, &home)
                    }
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
            let mut files = Vec::new();
            let listed = crow_fs::list_dir(&target, hidden, |batch| {
                for entry in batch {
                    if entry.is_dir && !entry.is_package() {
                        children.push(palette::PathEntry::new(entry.path, entry.name));
                    } else {
                        files.push(palette::PathEntry::new(entry.path, entry.name));
                    }
                }
            });
            (children, files, listed.is_err())
        });
        cx.spawn(async move |this, cx| {
            let (children, files, missing) = task.await;
            this.update(cx, |this, cx| {
                let applied = match &mut this.palette {
                    Some(palette) => match &mut palette.path {
                        Some(state) if state.parent == parent && state.generation == generation => {
                            state.children = Some(children);
                            state.files = files;
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
        if let Some(Item::Folder { label, source, .. }) = palette.listing.items.get(palette.cursor) {
            let text = match label.as_str() {
                _ if *source == Source::File => label.clone(),
                "/" => "/".to_owned(),
                "~" => "~/".to_owned(),
                _ => format!("{label}/"),
            };
            let input = palette.input.clone();
            input.update(cx, |input, cx| input.set_text(text, window, cx));
            return;
        }
        if palette.path.is_none() {
            crate::feedback::refuse(crate::feedback::RefuseTarget::Field(&palette.input), window, cx);
            return;
        }
        let input = palette.input.clone();
        let query = palette.input.read(cx).text().to_owned();
        let task = self.complete_typed_path(query.clone(), cx);
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
            // A file row reveals the file in its folder, in a new tab with Cmd-Return.
            Some(Item::Folder { path, source: Source::File, .. }) => {
                if new_tab && let Some(parent) = path.parent() {
                    self.open_tab_at(parent.to_path_buf(), window, cx);
                }
                self.pane().update(cx, |pane, cx| pane.reveal(path, cx));
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
            let cleaned = crow_core::clean_pasted_path(&typed).unwrap_or_else(|| typed.clone());
            let target = crow_core::resolve_typed(&cleaned, &cwd, &home);
            target.exists().then(|| (target.is_dir(), target, typed)) // io-ok: inside background_spawn
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
                    None => crate::feedback::refuse(crate::feedback::RefuseTarget::None, window, cx),
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
            CommandId::EditPath => self.edit_path(&EditPath, window, cx),
            CommandId::ViewAsList => self.view_as_list(&ViewAsList, window, cx),
            CommandId::ViewAsColumns => self.view_as_columns(&ViewAsColumns, window, cx),
            CommandId::Undo => self.undo(None, cx),
            CommandId::Redo => self.redo(cx),
            CommandId::AddToSidebar => self.add_to_sidebar(&AddToSidebar, window, cx),
            CommandId::WhyThisView => self.why_this_view(&WhyThisView, window, cx),
            CommandId::UseViewAsDefaults => self.use_view_as_defaults(&UseViewAsDefaults, window, cx),
            CommandId::ResetFolderView => self.reset_folder_view(&ResetFolderView, window, cx),
            CommandId::ShowThemePicker => self.show_theme_picker(&ShowThemePicker, window, cx),
            CommandId::OpenSettings => crate::settings_window::open(cx),
            CommandId::EditSettings => crate::ui::edit_settings_file(cx),
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
            cx.global::<Ui>().feedback.mode,
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
        let Item::Folder { path, label, name, source, last_visit, matched, .. } = &listing.items[i] else {
            return div().into_any_element();
        };
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
        let reveal = (*source == Source::File).then(|| "show in folder".to_owned());
        let right = note.map(str::to_owned).or(age).or(reveal);
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
            .child(
                div()
                    .size(px(22.))
                    .flex_none()
                    .children(icon(path, *source != Source::File, 22).map(|image| img(image).size(px(22.)))),
            )
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
        let count = picker.choice.entries.len();
        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        let mut last_variant = None;
        for (i, entry) in picker.choice.entries.iter().enumerate() {
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
            let label = picker.choice.label(i);
            let selected = i == picker.choice.cursor;
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
            cx.global::<Ui>().feedback.mode,
            cx.listener(|this, _, window, cx| this.cancel_picker(window, cx)),
        ))
    }
}

impl Render for CrowWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A deferred focus assignment (a new item's rename, the filter handing off
        // to the list) must not take focus from an overlay opened in between: the
        // keys the overlay blocks would land in a field behind it. This is the same
        // rule the fuzzer's focus invariant checks.
        if !self.refocus_overlay {
            let surfaces = self.overlay_focus_surfaces(cx);
            if !surfaces.is_empty() && !surfaces.iter().any(|(_, handle)| Some(handle) == window.focused(cx).as_ref()) {
                self.refocus_overlay = true;
                cx.defer_in(window, |this, window, cx| {
                    this.refocus_overlay = false;
                    let surfaces = this.overlay_focus_surfaces(cx);
                    if !surfaces.is_empty()
                        && !surfaces.iter().any(|(_, handle)| Some(handle) == window.focused(cx).as_ref())
                    {
                        window.focus(&surfaces[0].1, cx);
                        cx.notify();
                    }
                });
            }
        }
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
                || cx.global::<Ui>().settings.tips_muted()
                || crate::tips::owner(cx) != Some(cx.entity_id()))
        {
            self.tip = None;
            self.pane().update(cx, |pane, cx| pane.set_tip_inset(false, cx));
            crate::tips::closed(cx.entity_id(), cx);
        }
        let tip = match self.tip {
            Some(id) => crate::tips::key_text(id, cx).map(|key| {
                crate::tips::render_tip(
                    id,
                    key,
                    &theme,
                    f32::from(window.viewport_size().width),
                    cx.global::<Ui>().feedback.mode,
                    cx.listener(|this, _, _, cx| this.close_tip(cx)),
                    cx.listener(|this, _, _, cx| this.mute_tips(cx)),
                )
            }),
            None => None,
        };
        let sheet = self.render_sheet(window, cx);
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
            .on_action(cx.listener(Self::edit_path))
            .on_action(cx.listener(|this, _: &PathHover, _, cx| {
                this.path_hovered = true;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &PathUnhover, _, cx| {
                this.path_hovered = false;
                cx.notify();
            }))
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
                } else if this.search.is_none() && this.pane().read(cx).focus_handle().is_focused(window) {
                    // Nothing else wants Escape. A Vim mark waiting for its letter is a
                    // sequence in progress, so Escape cancels it before it deselects,
                    // as Vim's Esc does. Otherwise clear the list's highlight, as a
                    // click on empty space does. Filter and rename fields have their own
                    // deeper Escape bindings, and search is handled above.
                    let pane = this.pane().clone();
                    // A type-to-select run owns Escape first: dismiss its query, keep the
                    // selection, as Finder does.
                    let consumed = pane.update(cx, |pane, cx| pane.cancel_type_select(cx));
                    if !consumed && !pane.update(cx, |pane, cx| pane.cancel_pending(cx)) {
                        pane.update(cx, |pane, cx| {
                            pane.clear_selection(cx);
                        });
                        // Escape also ends the cut mark; the clipboard itself stays,
                        // so Cmd-V still moves the cut items.
                        if let Some(clipboard) = &mut cx.global_mut::<Operations>().clipboard {
                            clipboard.marked = false;
                        }
                        this.notify_views(cx);
                    }
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

#[cfg(any(test, feature = "fuzz"))]
impl CrowWindow {
    /// The selectors a fuzzer can click in this window, in render order. Views
    /// cap their row lists; the runner drops selectors that are not laid out.
    pub fn debug_selectors(&self, cx: &App) -> Vec<String> {
        let mut out =
            vec!["toolbar".to_owned(), "toolbar-go-to".to_owned(), "toolbar-search".to_owned(), "path-bar".to_owned()];
        out.extend(self.pane().read(cx).debug_selectors(cx));
        out.extend(self.sidebar.read(cx).debug_selectors(cx));
        if cx.global::<Ui>().settings.show_preview {
            out.push("preview".to_owned());
        }
        if let Some(search) = &self.search {
            out.extend(search.read(cx).debug_selectors(cx));
        }
        if self.palette.is_some() {
            out.push("palette-commands-token".to_owned());
            out.push("palette-empty".to_owned());
        }
        if self.why.is_some() {
            out.extend(["why-open-file", "why-reset", "why-defaults", "why-close"].map(str::to_owned));
        }
        if self.sheet.is_some() {
            out.extend(["sheet-panel", "sheet-scrim", "sheet-confirm", "sheet-cancel"].map(str::to_owned));
        }
        if self.picker.is_some() {
            out.push("theme-list".to_owned());
        }
        if self.tip.is_some() {
            out.extend(["tip", "tip-got-it"].map(str::to_owned));
        }
        out
    }

    /// Whether a text field has focus, so the fuzzer types instead of pressing keys.
    pub fn text_focused(&self, window: &Window, cx: &App) -> bool {
        if let Some(palette) = &self.palette {
            return palette.input.read(cx).focus_handle(cx).is_focused(window);
        }
        if let Some(edit) = &self.path_edit {
            return edit.input.read(cx).focus_handle(cx).is_focused(window);
        }
        if let Some(search) = &self.search {
            return search.read(cx).input_focused(window, cx);
        }
        self.pane().read(cx).text_focused(window, cx)
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    /// The folder the window shows, for the fuzz runner's jail check.
    #[cfg(feature = "fuzz")]
    pub fn current_dir(&self, cx: &App) -> PathBuf {
        self.pane().read(cx).current_dir(cx)
    }

    /// Bring the window back to the fuzz tree after a typed path or palette row
    /// left it.
    #[cfg(feature = "fuzz")]
    pub fn navigate_to(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        let pane = self.pane().clone();
        pane.update(cx, |pane, cx| pane.navigate(dir, cx));
    }

    /// The active pane's listing, for the fuzz runner's deep check. The runner
    /// compares it to `read_dir` on a background thread.
    #[cfg(feature = "fuzz")]
    pub fn listing_snapshot(&self, cx: &App, root: Option<&std::path::Path>) -> Vec<(PathBuf, Vec<String>, bool)> {
        self.pane().read(cx).listing_snapshot(cx, root)
    }

    /// Bounds for a generated selector, from the views' layout state. Fixed
    /// chrome selectors go through `debug_bounds` in the runner instead.
    pub fn debug_selector_bounds(&self, selector: &str, cx: &App) -> Option<gpui::Bounds<gpui::Pixels>> {
        if let Some(index) = selector.strip_prefix("sidebar-").and_then(|index| index.parse().ok()) {
            return self.sidebar.read(cx).debug_row_bounds(index, cx);
        }
        self.pane().read(cx).debug_selector_bounds(selector, cx)
    }

    /// Whether any tab is still settling (loading, building rows, reconciling) or
    /// an operation is running; `fuzz::settle` waits on this.
    pub fn is_busy(&self, cx: &App) -> bool {
        self.tabs.iter().any(|(_, pane)| pane.read(cx).is_settling(cx))
            || cx.global::<crate::operations::Operations>().running.is_some()
    }

    /// The H1 invariants. Cursor/selection rows, focus and modals are checked
    /// after every step; `deep` adds the listing = disk check, which belongs
    /// after a settle (the watcher may still be delivering a change).
    /// The runner adds the panic and error-log checks around each step.
    /// "The rename field exists only while renaming" needs no check: the field
    /// lives inside `Pane::renaming`.
    pub fn check_invariants(
        &self,
        window: &Window,
        cx: &App,
        deep: bool,
        root: Option<&std::path::Path>,
    ) -> Vec<crate::fuzz::Violation> {
        self.check_invariants_with_focus(window, cx, deep, root, true)
    }

    /// The H1 invariants. `focus_required` is false for an inactive window: it
    /// has no focused element by design while another window is key, and the
    /// live runner checks every browser window.
    pub fn check_invariants_with_focus(
        &self,
        window: &Window,
        cx: &App,
        deep: bool,
        root: Option<&std::path::Path>,
        focus_required: bool,
    ) -> Vec<crate::fuzz::Violation> {
        use crate::fuzz::Violation;
        let mut out = Vec::new();
        // First: the runner reports only the first violation, and a refused
        // operation outside the fuzz tree matters more than what follows from it.
        for path in &cx.global::<crate::operations::Operations>().refused {
            out.push(Violation::new("jail", format!("an operation tried to touch {}", path.display())));
        }
        // A listing is only expected to match disk once loading and operations
        // have finished; the runner's Settle step waits for that.
        let busy = self.is_busy(cx);
        let active = self.tabs.active_index();
        for (index, (_, pane)) in self.tabs.iter().enumerate() {
            let pane = pane.read(cx);
            for violation in pane.invariant_violations(cx) {
                out.push(Violation::new(violation.check, format!("tab {index}: {}", violation.detail)));
            }
            // Only the active tab's folders are shown; an inactive tab's listing
            // may lag behind a change made in another tab.
            if deep && !busy && index == active {
                for violation in pane.listing_violations(cx, root) {
                    out.push(Violation::new(violation.check, format!("tab {index}: {}", violation.detail)));
                }
            }
        }

        match window.focused(cx) {
            // Only the active window's focus is user-visible. GPUI keeps a
            // deactivated window's `focus` handle, so an inactive window can
            // still name a surface that has since closed (H4 batch 4 C2).
            Some(_) if !focus_required => {}
            Some(focused) => {
                // The topmost focus-owning overlay decides where focus must be. Tips do
                // not take focus, and the native context menu is not a GPUI overlay.
                let mut allowed = self.overlay_focus_surfaces(cx);
                if allowed.is_empty() {
                    let pane = self.pane().read(cx);
                    let sidebar = self.sidebar.read(cx);
                    allowed.push(("pane", pane.focus_handle().clone()));
                    allowed.push(("sidebar", sidebar.debug_focus_handle(cx)));
                    for text in pane.text_focus_handles(cx) {
                        allowed.push(("pane text field", text));
                    }
                    // The bottom bar's path field (⌘L) is a focus surface of its own.
                    if let Some(edit) = &self.path_edit {
                        allowed.push(("path field", edit.input.read(cx).focus_handle(cx)));
                    }
                }
                if !allowed.iter().any(|(_, handle)| handle == &focused) {
                    let surface = allowed.first().map_or("expected", |(name, _)| name);
                    out.push(Violation::new(
                        "focus",
                        format!(
                            "focus is outside the {surface} surface (active={}, {focused:?})",
                            window.is_window_active()
                        ),
                    ));
                }
            }
            None => {
                if let Some(violation) = missing_focus_violation(focus_required) {
                    out.push(violation);
                    return out;
                }
            }
        }

        if self.palette.is_some() && self.picker.is_some() {
            out.push(Violation::new("overlay", "the palette and the theme picker are both open"));
        }

        out.extend(crate::fuzz::a11y_violations(window, "a11y"));
        out
    }
}

/// A window with no focused element is only a violation when focus is required:
/// an inactive window has none by design while another window is key.
#[cfg(any(test, feature = "fuzz"))]
fn missing_focus_violation(focus_required: bool) -> Option<crate::fuzz::Violation> {
    focus_required.then(|| crate::fuzz::Violation::new("focus", "nothing is focused"))
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

    /// The first line of each palette row, aligned with `palette_rows`.
    fn palette_row_names(&self) -> Vec<String> {
        let Some(palette) = self.palette.as_ref() else { return Vec::new() };
        palette
            .listing
            .rows
            .iter()
            .map(|row| match row {
                Row::Header(text) => format!("# {text}"),
                Row::Status(text) => text.to_string(),
                Row::Item(i) => match &palette.listing.items[*i] {
                    Item::Folder { name, .. } => name.clone(),
                    Item::Command { command, .. } => command.label.to_owned(),
                },
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
