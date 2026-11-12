//! Cmd-F's search panel: names and contents from `crow-search` in This Folder or
//! Home, or Spotlight names everywhere. Results stream in; the summary counts
//! skipped folders and lists them with the rule that skipped each one.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use crow_core::{Selection, abbreviate_home};
use crow_fs::ops::Request;
use crow_search::{Hit, HitKind, Query, RuleSet, Summary};
use crow_theme::Rgb;
use futures::StreamExt;
use futures::channel::mpsc::unbounded;
use gpui::{
    App, Context, Entity, EventEmitter, ExternalDragPayload, FileDragPaths, FocusHandle, Focusable, Image, IntoElement,
    MouseButton, MouseDownEvent, Render, Role, ScrollStrategy, SharedString, Subscription, UniformListScrollHandle,
    Window, div, img, prelude::*, px, uniform_list,
};

use crate::actions::{
    CloseSearch, CopyFiles, CutFiles, DeletePermanently, MoveDown, MoveToFirst, MoveToLast, MoveToTrash, MoveUp,
    OpenSelection, QuickLook, SearchEverything, SearchReveal, SelectAll,
};
use crate::chrome::{Keycap, KeycapWeight, with_action_tooltip, with_tooltip};
use crate::icons::{self, Glyph};
use crate::keys::{self, KeyScope};
use crate::pane::DraggedSelection;
use crate::style::{active_selection, color, color_alpha, format_time, icon, inactive_selection};
use crate::text_input::{TextInput, TextInputEvent};
use crate::ui::Ui;

const MAX_RESULTS: usize = 20_000;
const ROW_HEIGHT: f32 = 40.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Folder,
    Home,
    Everywhere,
}

impl Scope {
    /// What the field's placeholder says for this scope.
    fn label(&self, folder: &str) -> SharedString {
        match self {
            Scope::Folder => format!("Search {folder}").into(),
            Scope::Home => "Search Home".into(),
            // U6 renames Everywhere to "This Mac"; this placeholder follows for free.
            Scope::Everywhere => "Search Everywhere".into(),
        }
    }
}

/// Which part of each file the query matches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Match {
    /// Names and contents: M6's behavior.
    #[default]
    All,
    /// Names only; no file is read.
    Names,
    /// Contents only; name-only hits are dropped.
    Contents,
}

fn query_parts(text: &str, matching: Match) -> (Option<String>, Option<String>) {
    let text = text.trim().to_owned();
    match matching {
        Match::All => (Some(text.clone()), Some(text)),
        Match::Names => (Some(text), None),
        Match::Contents => (None, Some(text)),
    }
}

/// Spotlight indexes names only, so the match control never applies there.
fn effective_match_for(scope: Scope, matching: Match) -> Match {
    if scope == Scope::Everywhere { Match::Names } else { matching }
}

pub enum SearchEvent {
    Close,
    FocusResults,
    Open { path: PathBuf, is_dir: bool },
    Reveal(PathBuf),
    Run(Request),
    Copy { paths: Vec<PathBuf>, cut: bool },
}

impl EventEmitter<SearchEvent> for SearchView {}

pub struct SearchView {
    folder: PathBuf,
    /// The folder's own name, for the field's placeholder.
    folder_label: String,
    home: PathBuf,
    show_hidden: bool,
    scope: Scope,
    matching: Match,
    everything: bool,
    input: Entity<TextInput>,
    focus: FocusHandle,
    results: Vec<Row>,
    by_path: HashMap<PathBuf, usize>,
    skipped: Vec<(PathBuf, String)>,
    error: Option<SharedString>,
    searching: bool,
    capped: bool,
    cancel: Arc<AtomicBool>,
    generation: u64,
    exclude_generation: u64,
    selection: Selection<PathBuf>,
    cursor: usize,
    scroll: UniformListScrollHandle,
    show_skipped: bool,
    _subscription: Subscription,
    _ui_subscription: Subscription,
    /// Repaint the band's focus ring when the field or the results take focus.
    _focus_subscriptions: Vec<Subscription>,
}

struct Row {
    path: PathBuf,
    name: String,
    folder: String,
    is_dir: bool,
    is_package: bool,
    kind: SharedString,
    modified: Option<SystemTime>,
    score: u32,
    /// The earliest matching line, for content hits.
    content: Option<(u64, String, Vec<Range<usize>>)>,
}

enum Update {
    Hits(Vec<Hit>),
    Done(Summary),
    Spotlight(Vec<crow_macos::SpotlightHit>),
    SpotlightDone(Result<(), String>),
}

impl Drop for SearchView {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl SearchView {
    pub fn new(
        folder: PathBuf,
        home: PathBuf,
        show_hidden: bool,
        query: String,
        everything: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let folder_label = crow_macos::display_components(&folder)
            .pop()
            .map_or_else(|| folder.display().to_string(), |(name, _)| name);
        let placeholder = Scope::Folder.label(&folder_label);
        let input = cx.new(|cx| TextInput::new(query.clone(), placeholder, cx).bare().with_role(Role::SearchInput));
        let focus = cx.focus_handle();
        let input_focus = input.read(cx).focus_handle(cx);
        let focus_subscriptions = vec![
            cx.on_focus(&input_focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&input_focus, window, |_, _, cx| cx.notify()),
        ];
        let subscription = cx.subscribe(&input, |this, input, event, cx| match event {
            TextInputEvent::Changed => {
                let query = input.read(cx).text().to_owned();
                this.start_search(&query, cx);
            }
            TextInputEvent::Confirmed => this.open_selected(cx),
            TextInputEvent::Cancelled => cx.emit(SearchEvent::Close),
            TextInputEvent::CompletionRequested => cx.emit(SearchEvent::FocusResults),
        });
        let exclude_generation = cx.global::<Ui>().exclude_generation;
        let ui_subscription = cx.observe_global::<Ui>(move |this, cx| {
            let generation = cx.global::<Ui>().exclude_generation;
            if this.exclude_generation != generation {
                this.exclude_generation = generation;
                let query = this.input.read(cx).text().to_owned();
                if !query.is_empty() {
                    this.start_search(&query, cx);
                }
            }
        });
        let mut search = Self {
            folder,
            folder_label,
            home,
            show_hidden,
            scope: Scope::Folder,
            matching: Match::All,
            everything,
            input,
            focus,
            results: Vec::new(),
            by_path: HashMap::new(),
            skipped: Vec::new(),
            error: None,
            searching: false,
            capped: false,
            cancel: Arc::new(AtomicBool::new(false)),
            generation: 0,
            exclude_generation,
            selection: Selection::default(),
            cursor: 0,
            scroll: UniformListScrollHandle::new(),
            show_skipped: false,
            _subscription: subscription,
            _ui_subscription: ui_subscription,
            _focus_subscriptions: focus_subscriptions,
        };
        if !query.is_empty() {
            search.start_search(&query, cx);
        }
        search
    }

    pub fn focus_input(&self, window: &mut Window, cx: &mut App) {
        let focus = self.input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    }

    /// Focus the field and select the whole query, so Cmd-F then typing starts a new search.
    pub fn select_all_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_input(window, cx);
        let len = self.input.read(cx).text().len();
        let input = self.input.clone();
        input.update(cx, |input, cx| input.select_range(0..len, window, cx));
    }

    pub fn focus_results(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    pub fn cancel(&mut self, _: &mut App) {
        self.cancel.store(true, Ordering::Relaxed);
        self.generation += 1;
    }

    pub fn set_show_hidden(&mut self, show: bool, cx: &mut Context<Self>) {
        if self.show_hidden == show {
            return;
        }
        self.show_hidden = show;
        let query = self.input.read(cx).text().to_owned();
        if !query.is_empty() {
            self.start_search(&query, cx);
        }
    }

    fn start_search(&mut self, query: &str, cx: &mut Context<Self>) {
        self.cancel.store(true, Ordering::Relaxed);
        self.generation += 1;
        let generation = self.generation;
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();

        self.results.clear();
        self.by_path.clear();
        self.skipped.clear();
        self.error = None;
        self.capped = false;
        self.show_skipped = false;
        self.selection.clear();
        self.cursor = 0;
        let query = query.trim();
        if query.is_empty() {
            self.searching = false;
            cx.notify();
            return;
        }
        self.searching = true;

        let (tx, mut rx) = unbounded();
        if self.scope == Scope::Everywhere {
            let query = query.to_owned();
            cx.background_spawn(async move {
                let tx = tx;
                let result = crow_macos::spotlight_names(
                    &query,
                    |batch| {
                        let _ = tx.unbounded_send(Update::Spotlight(batch));
                    },
                    &cancel,
                );
                let _ = tx.unbounded_send(Update::SpotlightDone(result));
            })
            .detach();
        } else {
            let exclude = cx.global::<Ui>().exclude.clone();
            let rules = match RuleSet::from_config(&exclude, &self.home) {
                Ok(rules) => rules,
                Err(error) => {
                    self.error = Some(error.into());
                    self.searching = false;
                    cx.notify();
                    return;
                }
            };
            let root = match self.scope {
                Scope::Folder => self.folder.clone(),
                Scope::Home | Scope::Everywhere => self.home.clone(),
            };
            let (names, contents) = query_parts(query, self.effective_match());
            let query = Query { names, contents, show_hidden: self.show_hidden, rules, everything: self.everything };
            cx.background_spawn(async move {
                let tx = tx;
                let summary = crow_search::search(
                    &root,
                    &query,
                    |batch| {
                        let _ = tx.unbounded_send(Update::Hits(batch));
                    },
                    &cancel,
                );
                let _ = tx.unbounded_send(Update::Done(summary));
            })
            .detach();
        }

        cx.spawn(async move |this, cx| {
            while let Some(update) = rx.next().await {
                let keep = this.update(cx, |this, cx| {
                    if this.generation == generation {
                        this.apply(update, cx);
                    }
                });
                if keep.is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn apply(&mut self, update: Update, cx: &mut Context<Self>) {
        match update {
            Update::Hits(hits) => self.add_hits(hits, cx),
            Update::Done(summary) => {
                self.skipped = summary.skipped;
                if let Some(error) = summary.error {
                    self.error = Some(error.into());
                }
                self.searching = false;
                cx.notify();
            }
            Update::Spotlight(hits) => self.add_spotlight(hits, cx),
            Update::SpotlightDone(result) => {
                if let Err(error) = result {
                    self.error = Some(error.into());
                }
                self.searching = false;
                cx.notify();
            }
        }
    }

    fn add_hits(&mut self, hits: Vec<Hit>, cx: &mut Context<Self>) {
        let mut changed = false;
        for hit in hits {
            if self.results.len() >= MAX_RESULTS {
                self.capped = true;
                self.cancel.store(true, Ordering::Relaxed);
                break;
            }
            if let Some(&index) = self.by_path.get(&hit.path) {
                let row = &mut self.results[index];
                if hit.score > row.score {
                    row.score = hit.score;
                }
                if let HitKind::Content { line, text, ranges } = hit.kind
                    && row.content.as_ref().is_none_or(|(best, _, _)| line < *best)
                {
                    row.content = Some((line, text, ranges));
                }
            } else {
                let row = Row::from_hit(hit, &self.home);
                self.by_path.insert(row.path.clone(), self.results.len());
                self.results.push(row);
                changed = true;
            }
        }
        if self.selection.is_empty()
            && let Some(first) = self.results.first()
        {
            self.selection.select_only(first.path.clone());
            self.cursor = 0;
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    fn add_spotlight(&mut self, hits: Vec<crow_macos::SpotlightHit>, cx: &mut Context<Self>) {
        let mut changed = false;
        for hit in hits {
            if self.results.len() >= MAX_RESULTS || self.by_path.contains_key(&hit.path) {
                if self.results.len() >= MAX_RESULTS {
                    self.capped = true;
                }
                continue;
            }
            let row = Row::from_spotlight(hit, &self.home);
            self.by_path.insert(row.path.clone(), self.results.len());
            self.results.push(row);
            changed = true;
        }
        if self.selection.is_empty()
            && let Some(first) = self.results.first()
        {
            self.selection.select_only(first.path.clone());
            self.cursor = 0;
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    fn query(&self, cx: &App) -> String {
        self.input.read(cx).text().to_owned()
    }

    #[cfg(test)]
    pub fn result_paths(&self) -> Vec<PathBuf> {
        self.results.iter().map(|row| row.path.clone()).collect()
    }

    #[cfg(test)]
    pub fn result_content(&self, path: &Path) -> Option<(u64, String)> {
        self.results
            .iter()
            .find(|row| row.path == path)
            .and_then(|row| row.content.as_ref().map(|(line, text, _)| (*line, text.clone())))
    }

    #[cfg(test)]
    pub fn is_searching(&self) -> bool {
        self.searching
    }

    #[cfg(test)]
    pub fn skipped_count(&self) -> usize {
        self.skipped.len()
    }

    pub fn everything(&self) -> bool {
        self.everything
    }

    #[cfg(test)]
    pub fn query_text(&self, cx: &App) -> String {
        self.query(cx)
    }

    #[cfg(test)]
    pub fn input_focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }

    #[cfg(test)]
    pub fn scope(&self) -> Scope {
        self.scope
    }

    fn set_scope(&mut self, scope: Scope, cx: &mut Context<Self>) {
        if self.scope == scope {
            return;
        }
        self.scope = scope;
        let placeholder = scope.label(&self.folder_label);
        let input = self.input.clone();
        input.update(cx, |input, cx| input.set_placeholder(placeholder, cx));
        let query = self.query(cx);
        self.start_search(&query, cx);
    }

    pub fn effective_match(&self) -> Match {
        effective_match_for(self.scope, self.matching)
    }

    /// Store the user's choice and rerun the current query. Spotlight ignores it.
    pub fn set_match(&mut self, matching: Match, cx: &mut Context<Self>) {
        if self.scope == Scope::Everywhere || self.matching == matching {
            return;
        }
        self.matching = matching;
        let query = self.query(cx);
        self.start_search(&query, cx);
    }

    #[cfg(test)]
    pub fn matching(&self) -> Match {
        self.matching
    }

    fn select_only(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.results.get(ix) else { return };
        self.selection.select_only(row.path.clone());
        self.cursor = ix;
        window.focus(&self.focus, cx);
        self.scroll.scroll_to_item(ix, ScrollStrategy::Top);
        cx.notify();
    }

    fn press(&mut self, ix: usize, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let Some(row) = self.results.get(ix) else { return };
        if event.modifiers.platform {
            self.selection.toggle(row.path.clone());
            if self.selection.cursor().is_none() {
                self.cursor = ix;
            }
        } else if event.modifiers.shift {
            self.extend_to(ix);
        } else {
            self.selection.select_only(row.path.clone());
            self.cursor = ix;
        }
        self.scroll.scroll_to_item(ix, ScrollStrategy::Top);
        cx.notify();
    }

    fn extend_to(&mut self, target: usize) {
        if target >= self.results.len() {
            return;
        }
        let paths: Vec<PathBuf> = self.results.iter().map(|row| row.path.clone()).collect();
        let position = |path: &PathBuf| paths.iter().position(|candidate| candidate == path);
        let at = |index: usize| paths[index].clone();
        self.selection.extend_to(target, position, at);
        self.cursor = target;
    }

    fn selected_paths(&self) -> Vec<PathBuf> {
        self.results.iter().filter(|row| self.selection.contains(&row.path)).map(|row| row.path.clone()).collect()
    }

    fn open_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(row) = self.results.get(self.cursor) {
            cx.emit(SearchEvent::Open { path: row.path.clone(), is_dir: row.is_dir && !row.is_package });
        }
    }

    fn reveal_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(row) = self.results.get(self.cursor) {
            cx.emit(SearchEvent::Reveal(row.path.clone()));
        }
    }

    fn quick_look_selected(&mut self, cx: &App) {
        if let Some(row) = self.results.get(self.cursor) {
            crow_macos::toggle_quick_look(&row.path);
            let _ = cx;
        }
    }

    fn copy_selected(&mut self, cut: bool, cx: &mut Context<Self>) {
        let paths = self.selected_paths();
        if !paths.is_empty() {
            cx.emit(SearchEvent::Copy { paths, cut });
        }
    }

    fn trash_selected(&mut self, permanent: bool, cx: &mut Context<Self>) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        let request = if permanent { Request::DeletePermanently { paths } } else { Request::Trash { paths } };
        cx.emit(SearchEvent::Run(request));
    }

    // ----- actions -----

    fn close_action(&mut self, _: &CloseSearch, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(SearchEvent::Close);
    }

    pub fn set_everything(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.everything == on {
            return;
        }
        self.everything = on;
        let query = self.query(cx);
        if !query.trim().is_empty() {
            self.start_search(&query, cx);
        }
    }

    fn search_everything(&mut self, _: &SearchEverything, _: &mut Window, cx: &mut Context<Self>) {
        self.set_everything(!self.everything, cx);
    }

    fn move_up(&mut self, _: &MoveUp, window: &mut Window, cx: &mut Context<Self>) {
        if self.cursor > 0 {
            self.select_only(self.cursor - 1, window, cx);
        } else if !self.results.is_empty() {
            self.select_only(0, window, cx);
        }
    }

    fn move_down(&mut self, _: &MoveDown, window: &mut Window, cx: &mut Context<Self>) {
        if self.cursor + 1 < self.results.len() {
            self.select_only(self.cursor + 1, window, cx);
        }
    }

    fn move_to_first(&mut self, _: &MoveToFirst, window: &mut Window, cx: &mut Context<Self>) {
        if !self.results.is_empty() {
            self.select_only(0, window, cx);
        }
    }

    fn move_to_last(&mut self, _: &MoveToLast, window: &mut Window, cx: &mut Context<Self>) {
        if !self.results.is_empty() {
            self.select_only(self.results.len() - 1, window, cx);
        }
    }

    fn open(&mut self, _: &OpenSelection, _: &mut Window, cx: &mut Context<Self>) {
        self.open_selected(cx);
    }

    fn reveal(&mut self, _: &SearchReveal, _: &mut Window, cx: &mut Context<Self>) {
        self.reveal_selected(cx);
    }

    fn quick_look(&mut self, _: &QuickLook, _: &mut Window, cx: &mut Context<Self>) {
        self.quick_look_selected(cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        let paths = self.results.iter().map(|row| row.path.clone());
        self.selection.select_all(paths);
        cx.notify();
    }

    fn copy(&mut self, _: &CopyFiles, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_selected(false, cx);
    }

    fn cut(&mut self, _: &CutFiles, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_selected(true, cx);
    }

    fn move_to_trash(&mut self, _: &MoveToTrash, _: &mut Window, cx: &mut Context<Self>) {
        self.trash_selected(false, cx);
    }

    fn delete_permanently(&mut self, _: &DeletePermanently, _: &mut Window, cx: &mut Context<Self>) {
        self.trash_selected(true, cx);
    }

    // ----- rendering -----

    /// The window's band while search is open: the query field and Done.
    /// The window places this between Back/Forward and the window edge.
    pub fn render_band(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.global::<Ui>().theme.clone();
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let narrow = window.viewport_size().width < px(640.);
        let ring = if focused { color_alpha(theme.role("accent"), 0x8c) } else { color(theme.role("surface")) };
        let field = div()
            .id("search-field")
            .debug_selector(|| "search-field".into())
            // keys::context_stack (KeyScope::Search) mirrors this.
            .key_context("Search")
            .on_action(cx.listener(Self::search_everything))
            .on_action(cx.listener(Self::close_action))
            .flex_1()
            .min_w(px(160.))
            .h(px(32.))
            .flex()
            .items_center()
            .gap(px(8.))
            .pl(px(10.))
            .pr(px(4.))
            .rounded(px(8.))
            .bg(color(theme.role("surface")))
            .border_2()
            .border_color(ring)
            .text_size(px(14.))
            .text_color(color(theme.role("fg")))
            .child(icons::glyph(Glyph::Search, 15., "fg-muted", &theme))
            .child(div().flex_1().min_w_0().child(self.input.clone()))
            .child(self.render_match_control(focused, cx));
        let done = div()
            .id("search-done")
            .debug_selector(|| "search-done".into())
            .role(Role::Button)
            .aria_label("Done, close search, Escape")
            .aria_keyshortcuts("Escape")
            .flex_none()
            .h(px(30.))
            .px(px(10.))
            .rounded(px(8.))
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(px(13.))
            .text_color(color(theme.role("fg")))
            .hover(move |style| style.bg(color(theme.role("surface"))))
            .on_click(cx.listener(|_, _, _, cx| cx.emit(SearchEvent::Close)))
            .child("Done")
            .when(!narrow, |button| button.child(Keycap::text("esc", KeycapWeight::Outline)));
        div().flex_1().min_w_0().flex().items_center().gap(px(10.)).child(field).child(done)
    }

    fn render_match_control(&self, focused: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.global::<Ui>().theme.clone();
        let spotlight = self.scope == Scope::Everywhere;
        let selected = self.effective_match();
        let fg = color(theme.role("fg"));
        let muted = color(theme.role("fg-muted"));
        let hover_fg = fg;
        let button = |matching: Match, label: &'static str, tooltip: &'static str| {
            let (id, aria) = match matching {
                Match::All => ("search-match-all", "All"),
                Match::Names => ("search-match-names", "Names"),
                Match::Contents => ("search-match-contents", "Contents"),
            };
            let aria = if spotlight { format!("{aria}, unavailable") } else { aria.to_owned() };
            let is_selected = selected == matching;
            let segment = div()
                .id(id)
                .debug_selector(move || id.into())
                .role(Role::RadioButton)
                .aria_label(aria)
                .aria_selected(is_selected)
                .h(px(22.))
                .px(px(9.))
                .rounded(px(4.))
                .flex()
                .items_center()
                .text_size(px(12.))
                .text_color(if is_selected { fg } else { muted })
                .when(is_selected, |segment| segment.bg(color(theme.role("surface-raised"))))
                .when(!is_selected && !spotlight, move |segment| segment.hover(move |style| style.text_color(hover_fg)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    if spotlight {
                        return;
                    }
                    this.set_match(matching, cx);
                    if focused {
                        this.focus_input(window, cx);
                    }
                }))
                .child(label);
            let tooltip = if spotlight { "Spotlight searches names only" } else { tooltip };
            with_tooltip(segment, tooltip, None)
        };
        div()
            .id("search-match")
            .role(Role::RadioGroup)
            .aria_label("Match")
            .flex()
            .flex_none()
            .gap(px(2.))
            .p(px(2.))
            .rounded(px(6.))
            .bg(color(theme.role("bg")))
            .when(spotlight, |control| control.opacity(0.4))
            .child(button(Match::All, "All", "Names and contents"))
            .child(button(Match::Names, "Names", "Names only: no file is read"))
            .child(button(Match::Contents, "Contents", "Contents only"))
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        let muted = color(theme.role("fg-muted"));
        let everything = self.everything;
        div()
            .px_3()
            .py_2()
            .flex()
            .flex_col()
            .gap_2()
            .border_b_1()
            .border_color(color(theme.role("border")))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(self.scope_button(Scope::Folder, "This Folder", cx))
                    .child(self.scope_button(Scope::Home, "Home", cx))
                    .child(self.scope_button(Scope::Everywhere, "Everywhere", cx))
                    .child(div().flex_1())
                    .child(with_action_tooltip(
                        div()
                            .id("search-everything")
                            .role(Role::Button)
                            .aria_label("Search everything, all rules off")
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .text_size(px(12.))
                            .text_color(color(if everything { theme.role("on-accent") } else { theme.role("fg") }))
                            .when(everything, |button| button.bg(color(theme.role("accent"))))
                            .when(!everything, |button| button.bg(color_alpha(theme.role("surface-raised"), 0xaa)))
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.search_everything(&SearchEverything, window, cx)
                                }),
                            )
                            .child("Search everything"),
                        "Search everything, all rules off",
                        &SearchEverything,
                        KeyScope::Search,
                        cx,
                    )),
            )
            .child(div().text_size(px(11.)).text_color(muted).child(
                match keys::text(&SearchEverything, KeyScope::Search, cx) {
                    Some(keys) => {
                        format!("{keys} turns every exclusion set off · contents are literal; /pattern/ is a regex")
                    }
                    None => "Contents are literal; /pattern/ is a regex".to_owned(),
                },
            ))
    }

    fn scope_button(&self, scope: Scope, label: &'static str, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.global::<Ui>().theme.clone();
        let selected = self.scope == scope;
        div()
            .id(label)
            .debug_selector(move || format!("search-scope-{label}"))
            .role(Role::Button)
            .aria_label(label)
            .aria_selected(selected)
            .px_2()
            .py(px(3.))
            .rounded_md()
            .text_size(px(12.))
            .text_color(color(if selected { theme.role("on-accent") } else { theme.role("fg") }))
            .when(selected, |button| button.bg(color(theme.role("accent"))))
            .when(!selected, |button| button.bg(color_alpha(theme.role("surface-raised"), 0xaa)))
            .on_click(cx.listener(move |this, _, _, cx| this.set_scope(scope, cx)))
            .child(label)
    }

    fn render_summary(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        let muted = color(theme.role("fg-muted"));
        let error = color(theme.role("error"));
        let count = self.results.len();
        let skipped = self.skipped.len();
        let mut summary = if self.error.is_some() {
            "Search failed".to_owned()
        } else if self.searching {
            format!("Searching… {count} results")
        } else if count == 0 {
            "No results".to_owned()
        } else {
            format!("{count} results")
        };
        if self.capped {
            summary.push_str(&format!(" (first {MAX_RESULTS})"));
        }
        let scope = self.scope;
        let mut row = div()
            .px_3()
            .py_2()
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(12.))
            .text_color(muted)
            .child(div().flex_1().child(summary));
        if scope == Scope::Everywhere {
            row = row.child("Spotlight searches indexed volumes only");
        } else if skipped > 0 {
            row = row.child(
                div()
                    .id("search-skipped-toggle")
                    .role(Role::Button)
                    .aria_label(format!("{skipped} folders skipped, show them"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_skipped = !this.show_skipped;
                        cx.notify();
                    }))
                    .child(format!("{skipped} folders skipped (rules)")),
            );
        }
        if self.everything {
            row = row.child("All rules off");
        }
        if let Some(error_text) = &self.error {
            row = row.child(div().text_color(error).child(error_text.clone()));
        }
        row
    }

    fn render_results(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        let muted = color(theme.role("fg-muted"));
        let border = color(theme.role("border"));
        let count = self.results.len();
        if count == 0 {
            let message = if self.query(cx).trim().is_empty() {
                "Type to search names and contents"
            } else if self.searching {
                "Searching…"
            } else {
                "No results"
            };
            return div()
                .flex_1()
                .min_h_0()
                .p_4()
                .text_size(px(13.))
                .text_color(muted)
                .child(message)
                .into_any_element();
        }
        let scroll = self.scroll.clone();
        let header = div()
            .h(px(26.))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(11.))
            .text_color(muted)
            .border_b_1()
            .border_color(border)
            .child(div().flex_1().child("Name"))
            .child(div().w(px(190.)).flex_none().child("Folder"))
            .child(div().w(px(110.)).flex_none().child("Kind"))
            .child(div().w(px(140.)).flex_none().child("Modified"));
        div()
            .id("search-results-list")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            // keys::context_stack (KeyScope::SearchResults) mirrors this.
            .key_context("SearchResults")
            .role(Role::ListBox)
            .aria_label(format!("Search results, {count} items"))
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::move_up))
            .on_action(cx.listener(Self::move_down))
            .on_action(cx.listener(Self::move_to_first))
            .on_action(cx.listener(Self::move_to_last))
            .on_action(cx.listener(Self::open))
            .on_action(cx.listener(Self::reveal))
            .on_action(cx.listener(Self::quick_look))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::move_to_trash))
            .on_action(cx.listener(Self::delete_permanently))
            .on_action(cx.listener(Self::close_action))
            .child(header)
            .child(
                uniform_list("search-results", count, cx.processor(Self::render_rows)).track_scroll(&scroll).flex_1(),
            )
            .into_any_element()
    }

    fn render_rows(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<impl IntoElement + use<>> {
        let ui = cx.global::<Ui>();
        let theme = &ui.theme;
        let (fg, muted, subtle, accent) = (
            color(theme.role("fg")),
            color(theme.role("fg-muted")),
            color(theme.role("fg-subtle")),
            theme.role("accent"),
        );
        let selection_bg =
            if self.focus.is_focused(window) { active_selection(theme) } else { inactive_selection(theme) };
        let count = self.results.len();
        let cursor = self.cursor;
        range
            .filter_map(|i| {
                let row = self.results.get(i)?;
                let selected = self.selection.contains(&row.path);
                let name_room = px(300.);
                let name = crate::style::truncate_middle(&row.name, name_room, px(13.), window);
                let content = row.content.as_ref().map(|(line, text, ranges)| {
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_size(px(11.))
                        .text_color(muted)
                        .child(format!("{line}:"))
                        .child(highlighted(text, ranges, accent))
                });
                let entries: Arc<Vec<(PathBuf, bool)>> = if selected {
                    Arc::new(
                        self.results
                            .iter()
                            .filter(|candidate| self.selection.contains(&candidate.path))
                            .map(|candidate| (candidate.path.clone(), candidate.is_dir))
                            .collect(),
                    )
                } else {
                    Arc::new(vec![(row.path.clone(), row.is_dir)])
                };
                let label: SharedString =
                    if entries.len() > 1 { format!("{} items", entries.len()).into() } else { row.name.clone().into() };
                let drag_icon = icon(&row.path, row.is_dir, 22);
                div()
                    .id(("search-row", i))
                    .role(Role::ListBoxOption)
                    .aria_label(format!(
                        "{}, in {}, {}, modified {}",
                        row.name,
                        row.folder,
                        row.kind,
                        format_time(row.modified)
                    ))
                    .aria_position_in_set(i + 1)
                    .aria_size_of_set(count)
                    .aria_selected(selected)
                    .when(cursor == i, |row| row.aria_active_descendant())
                    .h(px(ROW_HEIGHT))
                    .px_2()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(selected, |row| row.bg(selection_bg))
                    .debug_selector(|| format!("search-row-{i}"))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| this.press(i, event, window, cx)),
                    )
                    .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                        if event.click_count() == 2 {
                            this.cursor = i;
                            this.open_selected(cx);
                        }
                    }))
                    .on_drag(DraggedSelection { entries: entries.clone() }, {
                        let drag_icon = drag_icon.clone();
                        let label = label.clone();
                        move |_, offset, _, cx| {
                            cx.new(|_| SearchDrag { icon: drag_icon.clone(), label: label.clone(), cursor: offset })
                        }
                    })
                    .external_drag_payload::<DraggedSelection>({
                        let label = label.clone();
                        move |selection, window, cx| {
                            let entries = selection.entries.as_slice();
                            if let Some((path, is_dir)) = entries.first()
                                && let Some(png) = crow_macos::icon_png(path, *is_dir, 22)
                                && crow_macos::begin_drag(
                                    window,
                                    &crow_macos::ComboDrag {
                                        paths: entries,
                                        icon_png: png.as_slice(),
                                        label: label.as_ref(),
                                        width: 180.0,
                                        height: 24.0,
                                        pill_width: 150.0,
                                        cursor_from_left: 0.0,
                                        cursor_from_top: 12.0,
                                    },
                                )
                            {
                                cx.stop_active_drag(window);
                                return None;
                            }
                            Some(ExternalDragPayload::Files(FileDragPaths::new(entries.iter().cloned())))
                        }
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .size(px(16.))
                                    .flex_none()
                                    .children(icon(&row.path, row.is_dir, 16).map(|image| img(image).size(px(16.)))),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .text_color(fg)
                                            .child(name),
                                    )
                                    .children(content),
                            ),
                    )
                    .child(
                        div()
                            .w(px(190.))
                            .flex_none()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(11.))
                            .text_color(subtle)
                            .child(row.folder.clone()),
                    )
                    .child(
                        div()
                            .w(px(110.))
                            .flex_none()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(11.))
                            .text_color(muted)
                            .child(row.kind.clone()),
                    )
                    .child(
                        div()
                            .w(px(140.))
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(muted)
                            .child(format_time(row.modified)),
                    )
                    .into_any_element()
                    .into()
            })
            .collect()
    }

    fn render_skipped(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        if !self.show_skipped || self.skipped.is_empty() {
            return None;
        }
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        let muted = color(theme.role("fg-muted"));
        let rows = self.skipped.iter().enumerate().map(|(i, (path, rule))| {
            div()
                .id(("search-skipped-row", i))
                .role(Role::ListItem)
                .aria_label(format!("{}, {}", path.display(), rule))
                .px_2()
                .py_1()
                .flex()
                .flex_col()
                .child(div().text_size(px(12.)).child(path.display().to_string()))
                .child(div().text_size(px(11.)).text_color(muted).child(rule.clone()))
        });
        Some(
            div()
                .id("search-skipped")
                .role(Role::Dialog)
                .aria_label("Skipped folders")
                .mx_3()
                .mb_2()
                .max_h(px(220.))
                .overflow_y_scroll()
                .rounded_md()
                .border_1()
                .border_color(color(theme.role("border")))
                .bg(color(theme.role("surface")))
                .child(
                    div()
                        .id("search-skipped-close")
                        .role(Role::Button)
                        .aria_label("Close skipped folders")
                        .px_2()
                        .py_1()
                        .text_size(px(12.))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_skipped = false;
                            cx.notify();
                        }))
                        .child("Close"),
                )
                .children(rows),
        )
    }
}

impl Render for SearchView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        div()
            .id("search-view")
            .role(Role::Group)
            .aria_label("Search")
            // keys::context_stack (KeyScope::Search) mirrors this.
            .key_context("Search")
            .on_action(cx.listener(Self::search_everything))
            .on_action(cx.listener(Self::close_action))
            .size_full()
            .flex()
            .flex_col()
            .bg(color(theme.role("bg")))
            .text_color(color(theme.role("fg")))
            .child(self.render_header(cx))
            .child(self.render_summary(cx))
            .child(self.render_results(cx))
            .children(self.render_skipped(cx))
    }
}

impl Row {
    fn from_hit(hit: Hit, home: &Path) -> Row {
        let content = match hit.kind {
            HitKind::Content { line, text, ranges } => Some((line, text, ranges)),
            HitKind::Name => None,
        };
        Self::new(hit.path, hit.is_dir, hit.modified, hit.score, content, home)
    }

    fn from_spotlight(hit: crow_macos::SpotlightHit, home: &Path) -> Row {
        Self::new(hit.path, hit.is_dir, hit.modified, 0, None, home)
    }

    fn new(
        path: PathBuf,
        is_dir: bool,
        modified: Option<SystemTime>,
        score: u32,
        content: Option<(u64, String, Vec<Range<usize>>)>,
        home: &Path,
    ) -> Row {
        let name = path.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let folder = path.parent().map_or_else(String::new, |parent| abbreviate_home(parent, home));
        let ext = Path::new(&name).extension().and_then(|ext| ext.to_str()).unwrap_or("");
        let is_package = is_dir && matches!(ext, "app" | "bundle" | "framework" | "photoslibrary");
        let kind: SharedString = crow_macos::kind_name(ext, is_dir).to_string().into();
        Row { path, name, folder, is_dir, is_package, kind, modified, score, content }
    }
}

/// The matching line with each regex range tinted.
fn highlighted(text: &str, ranges: &[Range<usize>], accent: Rgb) -> impl IntoElement {
    let mut row = div().flex().items_center();
    let mut start = 0;
    for range in ranges {
        let end = range.end.min(text.len());
        let from = range.start.min(end);
        if from > start {
            row = row.child(text[start..from].to_owned());
        }
        if from < end {
            row = row.child(div().rounded_sm().bg(color_alpha(accent, 0x66)).child(text[from..end].to_owned()));
        }
        start = end;
    }
    if start < text.len() {
        row = row.child(text[start..].to_owned());
    }
    row
}

struct SearchDrag {
    icon: Option<Arc<Image>>,
    label: SharedString,
    cursor: gpui::Point<gpui::Pixels>,
}

impl Render for SearchDrag {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = &cx.global::<Ui>().theme;
        div()
            .ml(px(f32::from(self.cursor.x).min(40.0)))
            .flex()
            .items_center()
            .gap_2()
            .when_some(self.icon.clone(), |element, icon| element.child(img(icon).size(px(22.))))
            .child(
                div()
                    .h(px(20.))
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded_full()
                    .bg(color(theme.role("surface-raised")))
                    .text_color(color(theme.role("fg")))
                    .text_size(px(12.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(self.label.clone()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_parts_follow_the_match_mode() {
        assert_eq!(query_parts("needle", Match::All), (Some("needle".to_owned()), Some("needle".to_owned())));
        assert_eq!(query_parts("needle", Match::Names), (Some("needle".to_owned()), None));
        assert_eq!(query_parts("needle", Match::Contents), (None, Some("needle".to_owned())));
    }

    #[test]
    fn spotlight_forces_names() {
        assert_eq!(effective_match_for(Scope::Everywhere, Match::Contents), Match::Names);
        assert_eq!(effective_match_for(Scope::Everywhere, Match::All), Match::Names);
        assert_eq!(effective_match_for(Scope::Folder, Match::Contents), Match::Contents);
        assert_eq!(effective_match_for(Scope::Folder, Match::All), Match::All);
    }
}
