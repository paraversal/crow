//! Cmd-K: the palette sheet's state, listing refresh, rows and geometry.
//!
//! The static command inventory and matching live in `palette.rs`. Command
//! execution stays with the action owner (`CrowWindow::run_command` in
//! `window.rs`) so built-in and plugin routes can converge there.

use super::*;
use crate::palette::{Item, Row, Source};

/// Which half of the palette is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PaletteMode {
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
pub(super) struct Palette {
    pub(super) input: Entity<TextInput>,
    pub(super) mode: PaletteMode,
    pub(super) listing: palette::Listing,
    /// The focused folder's name, for the "In <name>" header.
    here: String,
    /// Path mode's parent listing, while the query is a path.
    pub(super) path: Option<PathState>,
    pub(super) cursor: usize,
    folders: Vec<palette::FolderCandidate>,
    /// The current folder's full listing (hidden entries too), for typed queries.
    pub(super) here_dir: Option<PaletteDir>,
    /// The last typed query and the file indices it matched, for incremental narrowing.
    file_narrow: Option<(String, Vec<u32>)>,
    scroll: ScrollHandle,
    /// The file surface that opened the palette, so a plugin command can
    /// restore its focus and revalidate the same scope/selection.
    plugin_context: Option<crate::plugins::FileCommandContext>,
    _subscription: Subscription,
}

#[cfg(test)]
impl Palette {
    /// The shortcut shown on row `i`, from the live keymap; the layer test reads it.
    pub(super) fn row_keys(&self, i: usize, cx: &App) -> Option<SharedString> {
        match self.listing.items.get(i)? {
            Item::Command { command, .. } => keys::text(command.action, command.scope, cx),
            Item::PluginCommand { .. } | Item::Folder { .. } => None,
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
pub(super) const PALETTE_LIST_PAD: f32 = 6.;
const PALETTE_FOLDER_ROW: f32 = 44.;
const PALETTE_COMMAND_ROW: f32 = 40.;
const PALETTE_PLUGIN_ROW: f32 = 46.;
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
        Item::PluginCommand { .. } => PALETTE_PLUGIN_ROW,
        Item::Folder { .. } => PALETTE_FOLDER_ROW,
    }
}

/// Path mode's listing of one parent, cached while the query stays under it.
pub(super) struct PathState {
    parent: PathBuf,
    partial: String,
    /// `None` while the background listing runs.
    children: Option<Vec<palette::PathEntry>>,
    /// The parent's files (and packages), listed with `children`.
    files: Vec<palette::PathEntry>,
    /// The parent is not a folder, or cannot be read.
    missing: bool,
    /// Dropped listings carry an older number.
    generation: u64,
}

/// One folder for the palette, listed off the main thread with hidden entries.
pub(super) struct PaletteDir {
    folders: Vec<palette::PathEntry>,
    files: Vec<palette::PathEntry>,
    missing: bool,
}

/// List a folder for the palette. Hidden entries are always included; the
/// listing decides whether they match, so a setting change never needs a relist.
fn list_palette_dir(parent: PathBuf, cx: &App) -> Task<PaletteDir> {
    cx.background_spawn(async move {
        let mut folders = Vec::new();
        let mut files = Vec::new();
        let listed = crow_fs::list_dir(&parent, true, |batch| {
            for entry in batch {
                let hidden = entry.hidden;
                if entry.is_dir && !entry.is_package() {
                    folders.push(palette::PathEntry::new(entry.path, entry.name, hidden));
                } else {
                    files.push(palette::PathEntry::new(entry.path, entry.name, hidden));
                }
            }
        });
        PaletteDir { folders, files, missing: listed.is_err() }
    })
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
    pub(super) fn show_palette(&mut self, _: &ShowCommandPalette, window: &mut Window, cx: &mut Context<Self>) {
        crate::usage::record(&ShowCommandPalette, cx);
        // Another overlay owns the keys; open_palette would refuse below.
        if self.search_blocked() {
            return;
        }
        crate::tips::key_used(TipId::GoTo, cx);
        self.open_palette("", window, cx);
    }

    /// Help ▸ Crow Keyboard Shortcuts starts the palette in command mode.
    pub(super) fn keyboard_shortcuts(&mut self, _: &KeyboardShortcuts, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette(">", window, cx);
    }

    pub(super) fn open_palette(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
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
            PaletteMode::Folders => {
                let search_includes_hidden = cx.global::<Ui>().settings.search_includes_hidden;
                let hidden = palette::hidden_match(&text, self.show_hidden, search_includes_hidden);
                palette::folder_listing(&text, &folders, &here, &[], &home, hidden, None)
            }
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
        let plugin_context = crate::plugins::current_command_context(cx);
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
            here_dir: None,
            file_narrow: None,
            scroll: ScrollHandle::new(),
            plugin_context,
            _subscription: subscription,
        });
        // The current folder's files and hidden subfolders arrive in the
        // background; typed queries use them, and the empty listing is already up.
        let task = list_palette_dir(cwd.clone(), cx);
        cx.spawn(async move |this, cx| {
            let dir = task.await;
            this.update(cx, |this, cx| {
                // The palette is per use; a result for a folder we left is stale.
                if this.pane().read(cx).current_dir(cx) != cwd {
                    return;
                }
                let home = cx.global::<Ui>().home.clone();
                {
                    let Some(palette) = this.palette.as_mut() else { return };
                    let hidden: Vec<palette::FolderCandidate> = dir
                        .folders
                        .iter()
                        .filter(|entry| entry.hidden)
                        .filter(|entry| !palette.folders.iter().any(|candidate| candidate.path == entry.path))
                        .map(|entry| palette::hidden_folder_candidate(&entry.path, entry.hidden, &home))
                        .collect();
                    palette.folders.extend(hidden);
                    palette.here_dir = Some(dir);
                    palette.file_narrow = None;
                }
                let query =
                    this.palette.as_ref().map(|palette| palette.input.read(cx).text().to_owned()).unwrap_or_default();
                this.refresh_palette(&query, cx);
            })
            .ok();
        })
        .detach();
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
        let mut listing = palette::command_listing(
            query,
            &recent,
            ui.settings.palette_recent_commands,
            ui.settings.folder_sizes == crow_core::FolderSizesMode::Visible,
        );
        let selection = self.palette.as_ref().and_then(|palette| palette.plugin_context.as_ref());
        let commands: Vec<_> = crate::plugins::command_items(cx)
            .into_iter()
            .filter(|command| {
                selection
                    .is_some_and(|context| crate::plugins::command_eligible(&command.extension, &context.selection, cx))
            })
            .collect();
        let matched = palette::plugin_command_matches(query, &commands);
        if !matched.is_empty() {
            listing.rows.push(Row::Header("Plugins".into()));
            for (item, _) in matched {
                listing.rows.push(Row::Item(listing.items.len()));
                listing.items.push(item);
            }
            listing.count = if query.trim().is_empty() {
                format!("{} commands", listing.items.len()).into()
            } else {
                format!("{} of {} commands", listing.items.len(), listing.items.len()).into()
            };
        }
        listing
    }

    pub(super) fn refresh_palette(&mut self, query: &str, cx: &mut Context<Self>) {
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
        let search_includes_hidden = cx.global::<Ui>().settings.search_includes_hidden;
        let command_listing = (self.palette.as_ref().map(|palette| palette.mode) == Some(PaletteMode::Commands))
            .then(|| self.palette_command_listing(query, cx));
        let mut spawn: Option<(PathBuf, u64)> = None;
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
                let relist = match &mut palette.path {
                    Some(state) if state.parent == parent => {
                        state.partial.clone_from(&partial);
                        false
                    }
                    slot => {
                        *slot = Some(PathState {
                            parent: parent.clone(),
                            partial: partial.clone(),
                            children: None,
                            files: Vec::new(),
                            missing: false,
                            generation: 0,
                        });
                        true
                    }
                };
                let state = palette.path.as_mut().expect("set above");
                if relist {
                    state.generation += 1;
                    spawn = Some((parent.clone(), state.generation));
                }
                let hidden = palette::hidden_match(&partial, show_hidden, search_includes_hidden);
                let state = palette.path.as_ref().expect("set above");
                palette.listing = match (&state.children, state.missing) {
                    (None, _) => palette::loading_path_listing(&state.parent),
                    (Some(_), true) => palette::missing_path_listing(&state.parent, &home),
                    (Some(children), false) => {
                        palette::path_listing(&state.parent, &state.partial, children, &state.files, &home, hidden)
                    }
                };
            } else {
                palette.path = None;
                let hidden = palette::hidden_match(query, show_hidden, search_includes_hidden);
                let narrowed = if query.is_empty() {
                    None
                } else {
                    palette.here_dir.as_ref().map(|dir| {
                        let previous = palette.file_narrow.as_ref().map(|(old, at)| (old.as_str(), at.as_slice()));
                        palette::narrow_files(query, &dir.files, previous)
                    })
                };
                let files: &[palette::PathEntry] = palette.here_dir.as_ref().map_or(&[], |dir| dir.files.as_slice());
                palette.listing = palette::folder_listing(
                    query,
                    &palette.folders,
                    &palette.here,
                    files,
                    &home,
                    hidden,
                    narrowed.as_deref(),
                );
                palette.file_narrow = narrowed.map(|at| (query.to_owned(), at));
            }
        }
        if let Some((parent, generation)) = spawn {
            self.spawn_path_listing(parent, generation, cx);
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
    /// has moved to another parent meanwhile.
    fn spawn_path_listing(&mut self, parent: PathBuf, generation: u64, cx: &mut Context<Self>) {
        let task = list_palette_dir(parent.clone(), cx);
        cx.spawn(async move |this, cx| {
            let dir = task.await;
            this.update(cx, |this, cx| {
                let applied = match &mut this.palette {
                    Some(palette) => match &mut palette.path {
                        Some(state) if state.parent == parent && state.generation == generation => {
                            state.children = Some(dir.folders);
                            state.files = dir.files;
                            state.missing = dir.missing;
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
    pub(super) fn toggle_state(&self, id: palette::CommandId, cx: &App) -> Option<bool> {
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
        let plugin_context = palette.plugin_context.clone();
        self.palette = None;
        self.focus_center(window, cx);
        match item {
            Some(Item::Command { command, .. }) => {
                self.run_command(command.id, window, cx);
                ui::record_command(command.id, cx);
            }
            Some(Item::PluginCommand { command, .. }) => {
                // Restore the source surface and its captured scope/selection
                // before the action's own availability check runs.
                if let Some(search) = &self.search {
                    search.update(cx, |search, cx| {
                        search.focus_results(window, cx);
                    });
                } else {
                    let focus = self.pane().read(cx).focus_handle().clone();
                    window.focus(&focus, cx);
                }
                if let Some(context) = plugin_context {
                    crate::plugins::restore_command_context(context, cx);
                }
                window.dispatch_action(
                    Box::new(crate::actions::PluginCommand {
                        extension: command.extension,
                        generation: command.generation,
                    }),
                    cx,
                );
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

    pub(super) fn render_palette(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
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
                        Item::PluginCommand { .. } => {
                            self.render_palette_plugin_row(&palette.listing, *i, selected, &theme, cx)
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
        let Item::Folder { path, label, name, source, hidden, last_visit, matched, .. } = &listing.items[i] else {
            return div().into_any_element();
        };
        let hidden = *hidden;
        let (name_text, name_ranges) = (name.clone(), listing.items[i].name_ranges());
        let (path_text, path_ranges) =
            truncate_middle_with_ranges(label, matched, px(palette_path_width(window)), px(12.), window);
        let section = palette::section_header(*source, here);
        let age = if *source == Source::Recent {
            last_visit.as_ref().map(|then| palette::relative_age(now, *then))
        } else {
            None
        };
        let name_aria = if hidden { format!("{name}, hidden") } else { name.clone() };
        let aria = match &age {
            Some(age) => format!("{name_aria}, {label}, {section}, visited {age}"),
            None => format!("{name_aria}, {label}, {section}"),
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
                    .opacity(content_opacity(hidden, false))
                    .children(icon(path, *source != Source::File, 22).map(|image| img(image).size(px(22.)))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .child(
                        div()
                            .text_size(px(14.))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .opacity(content_opacity(hidden, false))
                            .child(highlight_text(name_text, &name_ranges, theme, selected)),
                    )
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

    fn render_palette_plugin_row(
        &self,
        listing: &palette::Listing,
        i: usize,
        selected: bool,
        theme: &crow_theme::Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Item::PluginCommand { command, matched } = &listing.items[i] else { return div().into_any_element() };
        let muted = color(theme.role("fg-muted"));
        let border = color(theme.role("border"));
        let hover = color(theme.role("surface-raised"));
        div()
            .id(("palette-plugin-row", i))
            .debug_selector(|| format!("palette-plugin-row-{i}"))
            .role(Role::ListBoxOption)
            .aria_label(format!("{}, plugin {}", command.title, command.package))
            .aria_position_in_set(i + 1)
            .aria_size_of_set(listing.items.len())
            .aria_selected(selected)
            .when(selected, |row| row.aria_active_descendant())
            .h(px(PALETTE_PLUGIN_ROW))
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
                    .child("Plugin"),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(div().text_size(px(14.)).whitespace_nowrap().child(highlight_text(
                        &command.title,
                        matched,
                        theme,
                        selected,
                    )))
                    .child(
                        div().text_size(px(12.)).text_color(muted).whitespace_nowrap().child(command.package.clone()),
                    ),
            )
            .child(
                div()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(border)
                    .px(px(7.))
                    .h(px(18.))
                    .flex()
                    .items_center()
                    .text_size(px(12.))
                    .text_color(muted)
                    .child("plugin"),
            )
            .into_any_element()
    }
}

#[cfg(test)]
impl CrowWindow {
    /// The palette's rows for tests: "# Header", a folder's abbreviated path or a
    /// command's label.
    pub(super) fn palette_rows(&self) -> Vec<String> {
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
                    Item::PluginCommand { command, .. } => command.title.clone(),
                },
            })
            .collect()
    }

    /// The first line of each palette row, aligned with `palette_rows`.
    pub(super) fn palette_row_names(&self) -> Vec<String> {
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
                    Item::PluginCommand { command, .. } => command.title.clone(),
                },
            })
            .collect()
    }

    /// Whether palette row `row` is a hidden folder (None for headers and commands).
    pub(super) fn palette_row_hidden(&self, row: usize) -> Option<bool> {
        let palette = self.palette.as_ref()?;
        let Row::Item(index) = palette.listing.rows.get(row)? else { return None };
        match palette.listing.items.get(*index)? {
            Item::Folder { hidden, .. } => Some(*hidden),
            Item::Command { .. } | Item::PluginCommand { .. } => None,
        }
    }
}
