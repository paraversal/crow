//! Cmd-F's search panel: names and contents from `crow-search` in This Folder or
//! Home, or Spotlight names on This Mac. Results stream in; the footer counts
//! results and skipped folders, and lists the rule that skipped each one.

use std::collections::{HashMap, VecDeque};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use crow_core::{Exclude, KeymapLayer, Selection, SetName};
use crow_fs::Entry;
use crow_fs::ops::Request;
use crow_search::{Hit, HitKind, Query, RuleSet, Summary};
use futures::StreamExt;
use futures::channel::mpsc::unbounded;
use gpui::{
    App, Context, Entity, EventEmitter, ExternalDragPayload, FileDragPaths, FocusHandle, Focusable, FontWeight, Image,
    IntoElement, MouseButton, MouseDownEvent, Render, Role, ScrollStrategy, SharedString, Subscription, Toggled,
    UniformListScrollHandle, Window, div, img, prelude::*, px, uniform_list,
};

use crate::actions::{
    CloseSearch, CopyFiles, CutFiles, CycleMatch, DeletePermanently, MoveDown, MoveToFirst, MoveToLast, MoveToTrash,
    MoveUp, OpenSelection, QuickLook, SearchEverything, SearchFocusField, SearchFocusResults, SearchReveal, SelectAll,
};
use crate::chrome::{Keycap, KeycapWeight, with_action_tooltip, with_tooltip};
use crate::extensions::Ext;
use crate::icons::{self, Glyph};
use crate::keys::{self, KeyScope};
use crate::operations::Operations;
use crate::pane::{DraggedSelection, island_edges};
use crate::preview::{self, Preview};
use crate::style::{
    active_selection, color, color_alpha, cut_outline, display_text, field_ring, format_time, format_time_short,
    highlight_text, icon, inactive_selection, map_range, truncate_middle_parts,
};
use crate::text_input::{TextInput, TextInputEvent};
use crate::ui::Ui;

const MAX_RESULTS: usize = 20_000;
const ROW_HEIGHT: f32 = 40.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Folder,
    Home,
    ThisMac,
}

impl Scope {
    /// What the field's placeholder says for this scope.
    fn label(&self, folder: &str) -> SharedString {
        match self {
            Scope::Folder => format!("Search {folder}").into(),
            Scope::Home => "Search Home".into(),
            Scope::ThisMac => "Search This Mac".into(),
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
    // start_search has already trimmed it.
    let text = text.to_owned();
    match matching {
        Match::All => (Some(text.clone()), Some(text)),
        Match::Names => (Some(text), None),
        Match::Contents => (None, Some(text)),
    }
}

/// Spotlight indexes names only, so the match control never applies there.
fn effective_match_for(scope: Scope, matching: Match) -> Match {
    if scope == Scope::ThisMac { Match::Names } else { matching }
}

pub enum SearchEvent {
    Close,
    FocusResults,
    Open { path: PathBuf, is_dir: bool },
    Reveal(PathBuf),
    Run(Request),
    Copy { paths: Arc<[PathBuf]>, cut: bool },
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
    /// "Skip dev folders"; None hides the switch when no set is enabled.
    skip_label: Option<String>,
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
    /// The compiled rules for `exclude_generation`; the glob compile is ~14 ms.
    rules: Option<(u64, Arc<RuleSet>)>,
    selection: Selection<PathBuf>,
    cursor: usize,
    /// The user picked a row, so streaming batches stop re-selecting the best hit.
    user_moved: bool,
    /// Every accepted hit, including extra content lines for the same row.
    hit_count: usize,
    /// How many leading rows are name matches with a contiguous query match.
    name_count: usize,
    /// How many rows after the name block are content-only hits.
    content_count: usize,
    /// The selected row's preview, loaded off the main thread.
    preview: Option<LoadedPreview>,
    scroll: UniformListScrollHandle,
    show_skipped: bool,
    _subscription: Subscription,
    _ui_subscription: Subscription,
    /// Repaint the band's focus ring when the field or the results take focus.
    _focus_subscriptions: Vec<Subscription>,
}

struct LoadedPreview {
    entry: Entry,
    preview: Preview,
}

struct Row {
    path: PathBuf,
    name: String,
    folder: String,
    is_dir: bool,
    is_package: bool,
    modified: Option<SystemTime>,
    score: u32,
    /// A hit on the file name, as opposed to contents only.
    name_match: bool,
    /// The name contains the query contiguously, so it ranks above content hits.
    strong_name: bool,
    /// Matched byte ranges in `name`.
    name_ranges: Vec<Range<usize>>,
    /// The earliest matching line, for content hits; the full line and its ranges.
    content: Option<(u64, String, Vec<Range<usize>>)>,
    /// The line as drawn: excerpted around the match, ranges shifted. Built once in `Row::new`.
    content_shown: Option<(u64, String, Vec<Range<usize>>)>,
}

#[derive(Clone, Copy)]
enum ExampleAction {
    Fill(&'static str),
    ThisMac,
    Everything,
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
        let folder_label = crate::style::display_name(&folder);
        let placeholder = Scope::Folder.label(&folder_label);
        let input = cx.new(|cx| TextInput::new(query.clone(), placeholder, cx).bare().with_role(Role::SearchInput));
        let focus = cx.focus_handle();
        let input_focus = input.read(cx).focus_handle(cx);
        let focus_subscriptions = vec![
            cx.on_focus(&input_focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&input_focus, window, |_, _, cx| cx.notify()),
        ];
        let subscription = cx.subscribe_in(&input, window, |this, input, event, window, cx| match event {
            TextInputEvent::Changed => {
                let query = input.read(cx).text().to_owned();
                this.start_search(&query, cx);
            }
            // Return moves into the results, on the first one; Return there opens it.
            TextInputEvent::Confirmed => this.focus_results_from_field(&SearchFocusResults, window, cx),
            TextInputEvent::Cancelled => {
                if this.show_skipped {
                    this.show_skipped = false;
                    cx.notify();
                } else {
                    cx.emit(SearchEvent::Close);
                }
            }
            TextInputEvent::CompletionRequested => cx.emit(SearchEvent::FocusResults),
            TextInputEvent::BackspaceAtStart => {}
        });
        let exclude_generation = cx.global::<Ui>().exclude_generation;
        let skip_label_value = skip_label(&cx.global::<Ui>().exclude);
        let ui_subscription = cx.observe_global::<Ui>(move |this, cx| {
            let generation = cx.global::<Ui>().exclude_generation;
            if this.exclude_generation != generation {
                this.exclude_generation = generation;
                this.skip_label = skip_label(&cx.global::<Ui>().exclude);
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
            skip_label: skip_label_value,
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
            rules: None,
            selection: Selection::default(),
            cursor: 0,
            user_moved: false,
            hit_count: 0,
            name_count: 0,
            content_count: 0,
            preview: None,
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

    /// Focus the results list. Returns false, focusing nothing, when there are no results:
    /// the list isn't rendered then, and focus on it would leave the window without keys.
    pub fn focus_results(&self, window: &mut Window, cx: &mut App) -> bool {
        if self.results.is_empty() {
            return false;
        }
        window.focus(&self.focus, cx);
        true
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
        self.user_moved = false;
        self.hit_count = 0;
        self.name_count = 0;
        self.content_count = 0;
        self.preview = None;
        let query = query.trim();
        if query.is_empty() {
            self.searching = false;
            cx.notify();
            return;
        }
        self.searching = true;

        let (tx, mut rx) = unbounded();
        if self.scope == Scope::ThisMac {
            let query = query.to_owned();
            let effects = crate::effects::effects(cx);
            cx.background_spawn(async move {
                let hits = tx.clone();
                let result = effects.spotlight_names(
                    &query,
                    Box::new(move |batch| {
                        let _ = hits.unbounded_send(Update::Spotlight(batch));
                    }),
                    &cancel,
                );
                let _ = tx.unbounded_send(Update::SpotlightDone(result));
            })
            .detach();
        } else {
            let exclude_generation = cx.global::<Ui>().exclude_generation;
            let cached = match &self.rules {
                Some((generation, rules)) if *generation == exclude_generation => Some(rules.clone()),
                _ => None,
            };
            let rules = match cached {
                Some(rules) => rules,
                None => {
                    let exclude = cx.global::<Ui>().exclude.clone();
                    let rules = match RuleSet::from_config(&exclude, &self.home) {
                        Ok(rules) => Arc::new(rules),
                        Err(error) => {
                            self.error = Some(error.into());
                            self.searching = false;
                            cx.notify();
                            return;
                        }
                    };
                    self.rules = Some((exclude_generation, rules.clone()));
                    rules
                }
            };
            let root = match self.scope {
                Scope::Folder => self.folder.clone(),
                Scope::Home | Scope::ThisMac => self.home.clone(),
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
            // A fast walk can queue thousands of batches. Applying them all in one
            // turn would block drawing for seconds, so give the window a frame every
            // few milliseconds. Tests keep draining in one go (their clock is manual).
            let mut slice = std::time::Instant::now();
            while let Some(update) = rx.next().await {
                let keep = this.update(cx, |this, cx| {
                    if this.generation == generation {
                        this.apply(update, cx);
                    }
                });
                if keep.is_err() {
                    break;
                }
                if !cx.background_executor().is_test() && slice.elapsed().as_millis() >= 8 {
                    cx.background_executor().timer(Duration::from_millis(1)).await;
                    slice = std::time::Instant::now();
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn apply(&mut self, update: Update, cx: &mut Context<Self>) {
        let _span = tracing::debug_span!("search batch").entered();
        match update {
            // Once the result cap is hit the walk is cancelled; queued batches are
            // dropped rather than merged, so the UI is never busy draining them.
            Update::Hits(_) if self.capped => {}
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
        let root = (self.scope == Scope::Folder).then_some(self.folder.as_path());
        let query = self.query(cx);
        let mut changed = false;
        let mut fresh: Vec<Row> = Vec::new();
        let mut fresh_at: HashMap<PathBuf, usize> = HashMap::new();
        for hit in hits {
            let known = self.by_path.contains_key(&hit.path) || fresh_at.contains_key(&hit.path);
            if !known && self.results.len() + fresh.len() >= MAX_RESULTS {
                self.capped = true;
                self.cancel.store(true, Ordering::Relaxed);
                break;
            }
            // A content hit carries the file's other matching lines as a count.
            let more = match &hit.kind {
                HitKind::Content { more, .. } => *more,
                HitKind::Name { .. } => 0,
            };
            self.hit_count += 1 + more as usize;
            if let Some(&index) = self.by_path.get(&hit.path) {
                let row = &mut self.results[index];
                if hit.score > row.score {
                    row.score = hit.score;
                }
                if let HitKind::Content { line, text, ranges, .. } = hit.kind
                    && row.content.as_ref().is_none_or(|(best, _, _)| line < *best)
                {
                    row.set_content((line, text, ranges));
                    changed = true;
                }
            } else if let Some(&index) = fresh_at.get(&hit.path) {
                let row = &mut fresh[index];
                if hit.score > row.score {
                    row.score = hit.score;
                }
                if let HitKind::Content { line, text, ranges, .. } = hit.kind
                    && row.content.as_ref().is_none_or(|(best, _, _)| line < *best)
                {
                    row.set_content((line, text, ranges));
                }
            } else {
                let row = Row::from_hit(hit, &self.home, root, &query);
                fresh_at.insert(row.path.clone(), fresh.len());
                fresh.push(row);
                changed = true;
            }
        }
        if !fresh.is_empty() {
            self.insert_rows(fresh);
        }
        let before = self.selection.cursor().cloned();
        self.follow_selection();
        if before != self.selection.cursor().cloned() {
            self.load_preview(cx);
        }
        if changed {
            cx.notify();
        }
    }

    /// `results` is strong name matches (best score first), then content-only
    /// rows, then weak fuzzy name hits, each in walk order. A batch merges its
    /// strong names into the first block, appends its content hits, then its
    /// weak names.
    fn insert_rows(&mut self, fresh: Vec<Row>) {
        let (mut strong, rest): (Vec<Row>, Vec<Row>) = fresh.into_iter().partition(|row| row.strong_name);
        let (content, weak): (Vec<Row>, Vec<Row>) = rest.into_iter().partition(|row| !row.name_match);
        strong.sort_by_key(|row| std::cmp::Reverse(row.score));
        if strong.is_empty() {
            // Nothing moves in the name block: extend the content block, then the weak tail.
            let content_added = !content.is_empty();
            let at = self.name_count + self.content_count;
            let added = content.len();
            if content_added {
                self.results.splice(at..at, content);
                self.content_count += added;
            }
            let first = if content_added { at } else { self.results.len() };
            self.results.extend(weak);
            for (offset, row) in self.results[first..].iter().enumerate() {
                self.by_path.insert(row.path.clone(), first + offset);
            }
            return;
        }
        let mut all = std::mem::take(&mut self.results);
        let mut tail = all.split_off(self.name_count);
        let old_weak = tail.split_off(self.content_count);
        let old_content_end = all.len() + tail.len();
        let old_total = old_content_end + old_weak.len();
        let mut left: VecDeque<Row> = all.into_iter().collect();
        let mut right: VecDeque<Row> = strong.into();
        let mut merged = Vec::with_capacity(left.len() + right.len());
        let mut first_insert = None;
        loop {
            match (left.front(), right.front()) {
                (Some(l), Some(r)) if l.score >= r.score => merged.push(left.pop_front().expect("left")),
                (Some(_), Some(_)) => {
                    if first_insert.is_none() {
                        first_insert = Some(merged.len());
                    }
                    merged.push(right.pop_front().expect("right"));
                }
                (Some(_), None) => merged.push(left.pop_front().expect("left")),
                (None, Some(_)) => {
                    if first_insert.is_none() {
                        first_insert = Some(merged.len());
                    }
                    merged.push(right.pop_front().expect("right"));
                }
                (None, None) => break,
            }
        }
        self.name_count = merged.len();
        self.content_count = tail.len() + content.len();
        let content_added = !content.is_empty();
        self.results = merged.into_iter().chain(tail).chain(content).chain(old_weak).chain(weak).collect();
        // Rows only move to the right, so re-index the moved suffix and index the appended rows.
        let first = first_insert.unwrap_or(if content_added { old_content_end } else { old_total });
        for (offset, row) in self.results[first..].iter().enumerate() {
            self.by_path.insert(row.path.clone(), first + offset);
        }
    }

    /// Until the user picks a row, keep the best row selected as batches arrive.
    /// After that, keep the keyboard cursor on the same path when rows above it move.
    fn follow_selection(&mut self) {
        if !self.user_moved {
            if let Some(first) = self.results.first() {
                self.selection.select_only(first.path.clone());
                if self.cursor != 0 {
                    self.cursor = 0;
                    self.scroll.scroll_to_item(0, ScrollStrategy::Top);
                }
            }
            return;
        }
        if let Some(path) = self.selection.cursor().cloned()
            && let Some(&index) = self.by_path.get(&path)
            && self.cursor != index
        {
            self.cursor = index;
            self.scroll.scroll_to_item(index, ScrollStrategy::Nearest);
        }
    }

    fn add_spotlight(&mut self, hits: Vec<crow_macos::SpotlightHit>, cx: &mut Context<Self>) {
        let query = self.query(cx);
        let mut changed = false;
        for hit in hits {
            if self.by_path.contains_key(&hit.path) {
                continue;
            }
            if self.results.len() >= MAX_RESULTS {
                self.capped = true;
                break;
            }
            self.hit_count += 1;
            let row = Row::from_spotlight(hit, &query, &self.home);
            self.by_path.insert(row.path.clone(), self.results.len());
            self.results.push(row);
            changed = true;
        }
        self.name_count = self.results.len();
        self.content_count = 0;
        let before = self.selection.cursor().cloned();
        self.follow_selection();
        if before != self.selection.cursor().cloned() {
            self.load_preview(cx);
        }
        if changed {
            cx.notify();
        }
    }

    /// Load the cursor row's preview off the main thread, unless it is loaded.
    fn load_preview(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.results.get(self.cursor) else {
            self.preview = None;
            return;
        };
        let path = row.path.clone();
        let line = row.content.as_ref().map(|(line, _, _)| *line);
        if self.preview.as_ref().is_some_and(|loaded| loaded.entry.path == path) {
            return;
        }
        let extensions = cx.global::<Ext>().0.clone();
        let requested = path.clone();
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_spawn(async move {
                    let entry = crow_fs::stat_entry(&requested).ok().flatten()?;
                    let preview = preview::fetch_at(&entry, &extensions, line);
                    Some(LoadedPreview { entry, preview })
                })
                .await;
            this.update(cx, |this, cx| {
                if this.results.get(this.cursor).is_some_and(|row| row.path == path) {
                    this.preview = loaded;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The preview pane's parts, when a result is selected.
    pub fn preview_parts(&self) -> Option<(Entry, Option<Preview>)> {
        self.preview.as_ref().map(|loaded| (loaded.entry.clone(), Some(loaded.preview.clone())))
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

    /// Hand the view one batch, as the walker would, for deterministic order tests.
    #[cfg(test)]
    pub fn add_test_hits(&mut self, hits: Vec<Hit>, cx: &mut Context<Self>) {
        self.add_hits(hits, cx);
    }

    pub fn is_searching(&self) -> bool {
        self.searching
    }

    /// Whether the field has a query; the bottom bar shows no count in idle.
    pub fn has_query(&self, cx: &App) -> bool {
        !self.query(cx).trim().is_empty()
    }

    /// How many results the list holds so far.
    pub fn result_count(&self) -> usize {
        self.results.len()
    }

    pub fn selected_count(&self) -> usize {
        self.selection.len()
    }

    /// The selected result's path and whether it is a folder.
    pub fn selected_entry(&self) -> Option<(PathBuf, bool)> {
        let path = self.selection.cursor()?;
        let row = self.results.iter().find(|row| &row.path == path)?;
        Some((row.path.clone(), row.is_dir))
    }

    /// Whether the query field has focus.
    #[allow(dead_code)] // U5's hints read it.
    pub fn input_focused(&self, window: &Window, cx: &App) -> bool {
        self.input.read(cx).focus_handle(cx).is_focused(window)
    }

    /// The selectors a fuzzer can click while search covers the pane.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn debug_selectors(&self, cx: &App) -> Vec<String> {
        let mut out = vec![
            "search-field".to_owned(),
            "search-done".to_owned(),
            "search-scope-bar".to_owned(),
            "search-skip-switch".to_owned(),
            "search-footer".to_owned(),
        ];
        if !self.skipped.is_empty() {
            out.push("search-skipped-link".to_owned());
        }
        if self.input.read(cx).text().is_empty() && !self.searching {
            out.push("search-idle".to_owned());
        }
        out
    }

    #[cfg(test)]
    pub fn skipped_count(&self) -> usize {
        self.skipped.len()
    }

    #[cfg(test)]
    pub fn hit_count(&self) -> usize {
        self.hit_count
    }

    #[cfg(test)]
    pub fn row_label(&self, index: usize) -> Option<String> {
        self.results.get(index).map(Row::label)
    }

    #[cfg(test)]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    #[cfg(test)]
    pub fn skipped_open(&self) -> bool {
        self.show_skipped
    }

    #[cfg(test)]
    pub fn footer_summary_text(&self) -> String {
        self.footer_text()
    }

    #[cfg(test)]
    pub fn preview_path(&self) -> Option<PathBuf> {
        self.preview.as_ref().map(|loaded| loaded.entry.path.clone())
    }

    #[cfg(test)]
    pub fn preview_excerpt(&self) -> Option<(u64, String)> {
        self.preview
            .as_ref()
            .and_then(|loaded| loaded.preview.excerpt.as_ref())
            .map(|excerpt| (excerpt.hit, excerpt.text.clone()))
    }

    #[cfg(test)]
    pub fn error_text(&self) -> Option<String> {
        self.error.as_ref().map(ToString::to_string)
    }

    pub fn everything(&self) -> bool {
        self.everything
    }

    #[cfg(test)]
    pub fn query_text(&self, cx: &App) -> String {
        self.query(cx)
    }

    /// The search field's focus handle, for the window's overlay focus rule.
    pub fn input_focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }

    /// The results list's focus handle, for the window's overlay focus rule.
    pub fn results_focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    #[cfg(test)]
    pub fn scope(&self) -> Scope {
        self.scope
    }

    #[cfg(test)]
    pub fn folder(&self) -> &Path {
        &self.folder
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
        if self.scope == Scope::ThisMac || self.matching == matching {
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
        self.user_moved = true;
        window.focus(&self.focus, cx);
        self.scroll.scroll_to_item(ix, ScrollStrategy::Top);
        self.load_preview(cx);
        cx.notify();
    }

    fn press(&mut self, ix: usize, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let Some(row) = self.results.get(ix) else { return };
        self.user_moved = true;
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
        self.load_preview(cx);
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
        self.user_moved = true;
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
            crate::effects::effects(cx).toggle_quick_look(&row.path);
        }
    }

    fn copy_selected(&mut self, cut: bool, cx: &mut Context<Self>) {
        let paths: Arc<[PathBuf]> = self.selected_paths().into();
        if !paths.is_empty() {
            cx.emit(SearchEvent::Copy { paths, cut });
        }
    }

    fn trash_selected(&mut self, permanent: bool, cx: &mut Context<Self>) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        let paths: Arc<[PathBuf]> = paths.into();
        let request = if permanent { Request::DeletePermanently { paths } } else { Request::Trash { paths } };
        cx.emit(SearchEvent::Run(request));
    }

    // ----- actions -----

    fn close_action(&mut self, _: &CloseSearch, _: &mut Window, cx: &mut Context<Self>) {
        if self.show_skipped {
            self.show_skipped = false;
            cx.notify();
        } else {
            cx.emit(SearchEvent::Close);
        }
    }

    /// Esc closes the skipped list before the search. Returns whether it did.
    pub fn close_skipped(&mut self) -> bool {
        if self.show_skipped {
            self.show_skipped = false;
            true
        } else {
            false
        }
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

    /// All → Names → Contents. Spotlight is names-only, so it stays put.
    fn cycle_match(&mut self, _: &CycleMatch, _: &mut Window, cx: &mut Context<Self>) {
        if self.scope == Scope::ThisMac {
            return;
        }
        let next = match self.effective_match() {
            Match::All => Match::Names,
            Match::Names => Match::Contents,
            Match::Contents => Match::All,
        };
        self.set_match(next, cx);
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

    /// ↓ in the field: land on the current cursor row. With no results it does nothing,
    /// unlike Tab, which moves on to the sidebar.
    fn focus_results_from_field(&mut self, _: &SearchFocusResults, window: &mut Window, cx: &mut Context<Self>) {
        if self.results.is_empty() {
            return;
        }
        if self.selection.cursor().is_none() {
            self.select_only(self.cursor.min(self.results.len() - 1), window, cx);
        } else {
            window.focus(&self.focus, cx);
            self.scroll.scroll_to_item(self.cursor, ScrollStrategy::Top);
            cx.notify();
        }
    }

    /// ↑ on the first row goes back to the field; elsewhere it moves up as usual.
    fn focus_field_from_results(&mut self, _: &SearchFocusField, window: &mut Window, cx: &mut Context<Self>) {
        if self.cursor > 0 {
            self.move_up(&MoveUp, window, cx);
        } else {
            self.focus_input(window, cx);
            cx.notify();
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
        self.user_moved = true;
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
        let ring = if focused { field_ring(&theme) } else { color(theme.role("surface")) };
        let field = div()
            .id("search-field")
            .debug_selector(|| "search-field".into())
            // keys::context_stack (KeyScope::Search) mirrors this.
            .key_context("Search")
            .on_action(cx.listener(Self::search_everything))
            .on_action(cx.listener(Self::cycle_match))
            .on_action(cx.listener(Self::close_action))
            .on_action(cx.listener(Self::focus_results_from_field))
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
            .child(Keycap::text("esc", KeycapWeight::Outline));
        div().flex_1().min_w_0().flex().items_center().gap(px(10.)).child(field).child(done)
    }

    fn render_match_control(&self, focused: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.global::<Ui>().theme.clone();
        let spotlight = self.scope == Scope::ThisMac;
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
            let is_selected = selected == matching;
            let segment = div()
                .id(id)
                .debug_selector(move || id.into())
                .role(Role::RadioButton)
                .aria_label(aria)
                .aria_disabled(spotlight)
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
            if spotlight {
                with_tooltip(segment, "Spotlight searches names only", None)
            } else {
                with_action_tooltip(segment, tooltip, &CycleMatch, KeyScope::Search, cx)
            }
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

    fn render_scope_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        let muted = color(theme.role("fg-muted"));
        let segments = div()
            .id("search-scope")
            .role(Role::RadioGroup)
            .aria_label("Search scope")
            .flex()
            .flex_none()
            .gap(px(2.))
            .p(px(2.))
            .rounded(px(7.))
            .bg(color(theme.role("surface")))
            .child(self.scope_segment(Scope::Folder, self.folder_label.clone().into(), cx))
            .child(self.scope_segment(Scope::Home, "Home".into(), cx))
            .child(self.scope_segment(Scope::ThisMac, "This Mac".into(), cx));
        let right: Option<gpui::AnyElement> = if self.scope == Scope::ThisMac {
            Some(div().text_color(muted).child("Spotlight: indexed volumes only").into_any_element())
        } else {
            self.skip_label.as_ref().map(|label| self.render_skip_switch(label, cx).into_any_element())
        };
        div()
            .id("search-scope-bar")
            .debug_selector(|| "search-scope-bar".into())
            .role(Role::Group)
            .aria_label("Search scope")
            .h(px(38.))
            .px_3()
            .flex()
            .items_center()
            .gap(px(10.))
            .border_b_1()
            .border_color(color(theme.role("border")))
            .text_size(px(12.))
            .child(div().flex_none().text_color(muted).child("Search"))
            .child(segments)
            .child(div().flex_1())
            .children(right)
    }

    fn render_skip_switch(&self, label: &str, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.global::<Ui>().theme.clone();
        let on = !self.everything;
        let accent = color(theme.role("accent"));
        let raised = color(theme.role("surface-raised"));
        let knob_color = color(theme.role("bg"));
        let hover = color_alpha(theme.role("surface-raised"), 0x60);
        let shortcut = keys::spoken_for(&SearchEverything, KeyScope::Search, cx);
        let switch = div()
            .id("search-skip-switch")
            .debug_selector(|| "search-skip-switch".into())
            .role(Role::Switch)
            .aria_label(label.to_owned())
            .aria_toggled(if on { Toggled::True } else { Toggled::False })
            .when_some(shortcut.clone(), |switch, keys| switch.aria_keyshortcuts(keys))
            .h(px(24.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(7.))
            .rounded(px(12.))
            .border_1()
            .border_color(raised)
            .text_color(color(if on { theme.role("fg") } else { theme.role("fg-muted") }))
            .hover(move |style| style.bg(hover))
            .on_click(cx.listener(|this, _, window, cx| {
                this.search_everything(&SearchEverything, window, cx);
            }))
            .child(
                div()
                    .relative()
                    .w(px(22.))
                    .h(px(12.))
                    .flex_none()
                    .rounded(px(6.))
                    .bg(if on { accent } else { raised })
                    .child(
                        div()
                            .absolute()
                            .top(px(2.))
                            .left(px(if on { 12. } else { 2. }))
                            .size(px(8.))
                            .rounded_full()
                            .bg(knob_color),
                    ),
            )
            .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(label.to_owned()));
        with_action_tooltip(switch, "Skip folders listed in exclude.toml", &SearchEverything, KeyScope::Search, cx)
    }

    fn scope_segment(&self, scope: Scope, label: SharedString, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.global::<Ui>().theme.clone();
        let selected = self.scope == scope;
        let id = match scope {
            Scope::Folder => "search-scope-folder",
            Scope::Home => "search-scope-home",
            Scope::ThisMac => "search-scope-this-mac",
        };
        let aria = match scope {
            Scope::Folder => format!("Search in {label}"),
            Scope::Home => "Search in Home".to_owned(),
            Scope::ThisMac => "Search This Mac with Spotlight, names only".to_owned(),
        };
        let hover = color_alpha(theme.role("surface-raised"), 0x80);
        let pressed = color(theme.role("surface-raised"));
        div()
            .id(id)
            .debug_selector(move || id.into())
            .role(Role::RadioButton)
            .aria_label(aria)
            .aria_selected(selected)
            .h(px(22.))
            .px(px(9.))
            .flex()
            .items_center()
            .rounded(px(5.))
            .whitespace_nowrap()
            .text_color(color(if selected { theme.role("fg") } else { theme.role("fg-muted") }))
            .when(selected, |segment| segment.bg(pressed))
            .when(!selected, move |segment| {
                segment.hover(move |style| style.bg(hover)).active(move |style| style.bg(pressed))
            })
            .on_click(cx.listener(move |this, _, _, cx| this.set_scope(scope, cx)))
            .child(div().max_w(px(160.)).overflow_hidden().whitespace_nowrap().text_ellipsis().child(label))
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        let (fg, muted, subtle, accent, warning, error) = (
            color(theme.role("fg")),
            color(theme.role("fg-muted")),
            color(theme.role("fg-subtle")),
            color(theme.role("accent")),
            color(theme.role("warning")),
            color(theme.role("error")),
        );
        let summary = self.footer_text();
        let text_color = if self.error.is_some() {
            error
        } else if self.capped {
            warning
        } else {
            fg
        };
        let skipped = self.skipped.len();
        let link = (skipped > 0 && !self.searching && self.error.is_none()).then(|| {
            let chevron = if self.show_skipped { "⌄" } else { "›" };
            div()
                .id("search-skipped-link")
                .debug_selector(|| "search-skipped-link".into())
                .role(Role::Button)
                .aria_label(format!("{}, show list", skipped_text(skipped)))
                .aria_expanded(self.show_skipped)
                .flex_none()
                .text_color(accent)
                .font_weight(FontWeight::MEDIUM)
                .hover(|style| style.underline())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.show_skipped = !this.show_skipped;
                    cx.notify();
                }))
                .child(format!("{} {chevron}", skipped_text(skipped)))
        });
        let hint = (self.scope != Scope::ThisMac && self.skip_label.is_some()).then(|| {
            let hint = if self.everything { "Skip folders again" } else { "Search everything" };
            with_action_tooltip(
                div()
                    .id("search-everything-hint")
                    .debug_selector(|| "search-everything-hint".into())
                    .role(Role::Button)
                    .aria_label(hint)
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .text_color(muted)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.search_everything(&SearchEverything, window, cx);
                    }))
                    .child(hint)
                    .children(Keycap::for_action(
                        &SearchEverything,
                        KeyScope::SearchResults,
                        KeycapWeight::Outline,
                        cx,
                    )),
                hint,
                &SearchEverything,
                KeyScope::SearchResults,
                cx,
            )
        });
        let aria = if self.searching { "Searching".to_owned() } else { summary.clone() };
        div()
            .id("search-footer")
            .debug_selector(|| "search-footer".into())
            .role(Role::Status)
            .aria_label(aria)
            .h(px(32.))
            .px_3()
            .flex()
            .items_center()
            .gap(px(10.))
            .border_t_1()
            .border_color(color(theme.role("border")))
            .text_size(px(12.))
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(text_color)
                    .child(summary),
            )
            .children(link.map(|link| {
                div().flex().items_center().gap(px(10.)).child(div().text_color(subtle).child("·")).child(link)
            }))
            .child(div().flex_1())
            .children(hint)
    }

    /// The footer's left text, also used by tests and VoiceOver.
    fn footer_text(&self) -> String {
        footer_summary(FooterState {
            searching: self.searching,
            hits: self.hit_count,
            files: self.results.len(),
            capped: self.capped,
            error: self.error.as_deref(),
        })
    }

    /// Idle, error, empty or results, depending on the query and what has arrived.
    fn render_body(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        if self.query(cx).trim().is_empty() {
            return self.render_idle(cx);
        }
        if self.results.is_empty() {
            if self.error.is_some() {
                return self.render_error(cx);
            }
            return self.render_empty(cx);
        }
        self.render_results(cx)
    }

    fn scope_label(&self) -> String {
        match self.scope {
            Scope::Folder => self.folder_label.clone(),
            Scope::Home => "Home".to_owned(),
            Scope::ThisMac => "This Mac".to_owned(),
        }
    }

    fn render_idle(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        let (fg, subtle) = (color(theme.role("fg")), color(theme.role("fg-subtle")));
        let heading = match (self.scope, self.effective_match()) {
            (Scope::ThisMac, _) => "Search names on this Mac".to_owned(),
            (_, Match::All) => format!("Search names and contents in {}", self.scope_label()),
            (_, Match::Names) => format!("Search names in {}", self.scope_label()),
            (_, Match::Contents) => format!("Search contents in {}", self.scope_label()),
        };
        let keycap = Keycap::for_action(&SearchEverything, KeyScope::Search, KeycapWeight::Text, cx)
            .map(|keycap| keycap.text_role("fg").into_any_element())
            .unwrap_or_else(|| div().text_color(fg).child("⇧⌘A").into_any_element());
        let spoken =
            keys::spoken_for(&SearchEverything, KeyScope::Search, cx).unwrap_or_else(|| "Shift Command A".to_owned());
        let examples = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(self.render_example(
                0,
                div().child("config").into_any_element(),
                "names and file contents, any case",
                ExampleAction::Fill("config"),
                "Search for config".to_owned(),
                cx,
            ))
            .child(self.render_example(
                1,
                div().child("/fn \\w+_view/").into_any_element(),
                "slashes make it a regular expression",
                ExampleAction::Fill("/fn \\w+_view/"),
                "Search for the regular expression fn backslash w plus underscore view".to_owned(),
                cx,
            ))
            .child(self.render_example(
                2,
                div().child("This Mac").into_any_element(),
                "names only, through Spotlight",
                ExampleAction::ThisMac,
                "Search This Mac".to_owned(),
                cx,
            ))
            .when(self.skip_label.is_some(), |examples| {
                examples.child(self.render_example(
                    3,
                    keycap,
                    "also look inside node_modules, target, .git …",
                    ExampleAction::Everything,
                    format!("Search everything, {spoken}"),
                    cx,
                ))
            });
        let close = keys::text(&CloseSearch, KeyScope::Search, cx).unwrap_or_else(|| "esc".into());
        let reveal = keys::text(&SearchReveal, KeyScope::SearchResults, cx).unwrap_or_else(|| "⌘R".into());
        let quick = keys::text(&QuickLook, KeyScope::SearchResults, cx).unwrap_or_else(|| "Space".into());
        div()
            .id("search-idle")
            .debug_selector(|| "search-idle".into())
            .flex_1()
            .min_h_0()
            .pt(px(34.))
            .px(px(48.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .text_color(fg)
            .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(heading))
            .child(examples)
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(subtle)
                    .child(format!("{close} returns to {} · {reveal} reveals · {quick} previews", self.folder_label)),
            )
            .into_any_element()
    }

    fn render_example(
        &self,
        index: usize,
        face: gpui::AnyElement,
        description: &'static str,
        action: ExampleAction,
        aria: String,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = cx.global::<Ui>().theme.clone();
        let surface = color(theme.role("surface"));
        let raised = color(theme.role("surface-raised"));
        div()
            .flex()
            .items_center()
            .gap(px(14.))
            .child(
                div()
                    .id(("search-example", index))
                    .debug_selector(move || format!("search-example-{index}"))
                    .role(Role::Button)
                    .aria_label(aria)
                    .min_w(px(150.))
                    .h(px(26.))
                    .px_3()
                    .flex()
                    .items_center()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(raised)
                    .bg(surface)
                    .hover(move |style| style.bg(raised))
                    .font_family("Menlo")
                    .text_size(px(12.))
                    .text_color(color(theme.role("fg")))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        match action {
                            ExampleAction::Fill(text) => {
                                let input = this.input.clone();
                                input.update(cx, |input, cx| input.set_text(text, window, cx));
                            }
                            ExampleAction::ThisMac => this.set_scope(Scope::ThisMac, cx),
                            ExampleAction::Everything => {
                                this.search_everything(&SearchEverything, window, cx);
                            }
                        }
                        this.focus_input(window, cx);
                    }))
                    .child(face),
            )
            .child(div().text_color(color(theme.role("fg-muted"))).child(description))
            .into_any_element()
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.global::<Ui>().theme.clone();
        let (muted, accent) = (color(theme.role("fg-muted")), color(theme.role("accent")));
        if self.searching {
            return div()
                .flex_1()
                .min_h_0()
                .p_4()
                .text_size(px(13.))
                .text_color(muted)
                .child("Searching…")
                .into_any_element();
        }
        let skipped = self.skipped.len();
        let message = format!("No results for \"{}\" in {}", self.query(cx), self.scope_label());
        div()
            .id("search-empty")
            .flex_1()
            .min_h_0()
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
            .text_size(px(13.))
            .text_color(muted)
            .child(message)
            .when(skipped > 0 && self.scope != Scope::ThisMac, |body| {
                let hint = if self.everything { "Skip folders again" } else { "Search everything" };
                body.child(
                    div()
                        .id("search-empty-everything")
                        .role(Role::Button)
                        .aria_label(format!("{}. {hint}", skipped_was_text(skipped)))
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_color(accent)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.search_everything(&SearchEverything, window, cx);
                        }))
                        .child(format!("{}. {hint}", skipped_was_text(skipped)))
                        .children(Keycap::for_action(&SearchEverything, KeyScope::Search, KeycapWeight::Outline, cx)),
                )
            })
            .into_any_element()
    }

    fn render_error(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.global::<Ui>().theme.clone();
        let (error, muted) = (color(theme.role("error")), color(theme.role("fg-muted")));
        let message = self.error.as_deref().unwrap_or("Search failed");
        let hint = message.starts_with("Invalid pattern").then(|| {
            div()
                .text_size(px(13.))
                .text_color(muted)
                .child("Slashes make it a regular expression; remove them to search literally.")
        });
        div()
            .id("search-error")
            .flex_1()
            .min_h_0()
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
            .text_size(px(13.))
            .text_color(error)
            .child(format!("Can't search: {message}"))
            .children(hint)
            .into_any_element()
    }

    fn render_results(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        let muted = color(theme.role("fg-muted"));
        let border = color(theme.role("border"));
        let count = self.results.len();
        let scroll = self.scroll.clone();
        let vim = ui.settings.keymap == KeymapLayer::Vim;
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
            .child(div().w(px(180.)).flex_none().child("Folder"))
            .child(div().w(px(110.)).flex_none().child("Modified"));
        div()
            .id("search-results-list")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            // keys::context_stack (KeyScope::SearchResults) mirrors this.
            .key_context(if vim { "SearchResults vim" } else { "SearchResults" })
            .role(Role::ListBox)
            .aria_label(format!("Search results, {count} items"))
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::move_up))
            .on_action(cx.listener(Self::move_down))
            .on_action(cx.listener(Self::focus_field_from_results))
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
        let (fg, muted, subtle) =
            (color(theme.role("fg")), color(theme.role("fg-muted")), color(theme.role("fg-subtle")));
        let focused = self.focus.is_focused(window);
        let selection_bg = if focused { active_selection(theme) } else { inactive_selection(theme) };
        let count = self.results.len();
        let cursor = self.cursor;
        let mode = ui.feedback.mode;
        let now = Instant::now();
        let clipboard = cx.global::<Operations>().clipboard.as_ref();
        let flash = clipboard
            .and_then(|clipboard| clipboard.flash)
            .map(|started| crow_core::motion::copy_flash(mode, started, now))
            .unwrap_or(0.0);
        let cut = clipboard.is_some_and(|clipboard| clipboard.cut && clipboard.marked);
        let first = range.start.saturating_sub(1);
        let last = (range.end + 1).min(count);
        let cut_flags: Vec<bool> = (first..last)
            .map(|i| cut && self.results.get(i).is_some_and(|row| clipboard.is_some_and(|c| c.contains(&row.path))))
            .collect();
        let cut_edges = island_edges(&cut_flags);
        range
            .filter_map(|i| {
                let row = self.results.get(i)?;
                let on_clipboard = clipboard.is_some_and(|clipboard| clipboard.contains(&row.path));
                let cut_edge = cut_edges.get(i - first).copied().flatten();
                let flash_tint = (on_clipboard && flash > 0.0)
                    .then(|| color_alpha(theme.role("accent"), (flash * 0x40 as f32) as u8));
                let selected = self.selection.contains(&row.path);
                let on_selection = selected && focused;
                let truncated = truncate_middle_parts(&row.name, px(300.), px(13.), window);
                let name_ranges: Vec<Range<usize>> = row
                    .name_ranges
                    .iter()
                    .flat_map(|range| map_range(&truncated, range.clone()))
                    .filter(|range| !range.is_empty())
                    .collect();
                let name = highlight_text(truncated.text.clone(), &name_ranges, theme, on_selection);
                // Name matches stay one line; the content line is for content-only hits.
                let content =
                    (!row.name_match).then_some(row.content_shown.as_ref()).flatten().map(|(line, text, ranges)| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .text_size(px(11.))
                            .text_color(muted)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(
                                div()
                                    .w(px(28.))
                                    .flex_none()
                                    .flex()
                                    .justify_end()
                                    .text_color(subtle)
                                    .child(line.to_string()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(highlight_text(text.clone(), ranges, theme, on_selection)),
                            )
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
                let label: SharedString = if entries.len() > 1 {
                    format!("{} items", entries.len()).into()
                } else {
                    display_text(&row.name).into_owned().into()
                };
                let drag_icon = icon(&row.path, row.is_dir, 22);
                div()
                    .id(("search-row", i))
                    .role(Role::ListBoxOption)
                    .aria_label(row.label())
                    .aria_position_in_set(i + 1)
                    .aria_size_of_set(count)
                    .aria_selected(selected)
                    .when(cursor == i, |row| row.aria_active_descendant())
                    .h(px(ROW_HEIGHT))
                    .w_full()
                    .px_2()
                    .rounded_md()
                    .relative()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(selected, |row| row.bg(selection_bg))
                    .when_some(flash_tint, |row, tint| {
                        row.child(
                            div()
                                .absolute()
                                .inset_0()
                                .rounded_md()
                                .bg(tint)
                                .debug_selector(|| format!("search-flash-{i}")),
                        )
                    })
                    .when_some(cut_edge, |row, edge| {
                        row.child(
                            cut_outline(theme, edge.top, edge.bottom)
                                .debug_selector(|| format!("search-cut-outline-{i}")),
                        )
                    })
                    .debug_selector(|| format!("search-row-{i}"))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| this.press(i, event, window, cx)),
                    )
                    .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                        if event.click_count() == 2 {
                            this.cursor = i;
                            this.user_moved = true;
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
                                && crate::effects::effects(cx).begin_drag(
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
                                    .justify_center()
                                    .child(
                                        div()
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .text_size(px(13.))
                                            .text_color(fg)
                                            .child(name),
                                    )
                                    .children(content),
                            ),
                    )
                    .child(
                        div()
                            .w(px(180.))
                            .flex_none()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(12.))
                            .text_color(muted)
                            .child(row.folder.clone()),
                    )
                    .child(
                        div()
                            .w(px(110.))
                            .flex_none()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(12.))
                            .text_color(muted)
                            .child(format_time_short(row.modified)),
                    )
                    .into_any_element()
                    .into()
            })
            .collect()
    }

    /// A skipped folder as the list shows it: relative to the folder scope,
    /// `~`-abbreviated for Home.
    fn skipped_display(&self, path: &Path) -> String {
        match self.scope {
            Scope::Folder => path
                .strip_prefix(&self.folder)
                .ok()
                .map(|relative| {
                    if relative.as_os_str().is_empty() {
                        ".".to_owned()
                    } else {
                        display_text(&relative.to_string_lossy()).into_owned()
                    }
                })
                .unwrap_or_else(|| crate::style::display_path(path, &self.home)),
            Scope::Home | Scope::ThisMac => crate::style::display_path(path, &self.home),
        }
    }

    fn render_skipped(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        if !self.show_skipped || self.skipped.is_empty() {
            return None;
        }
        let theme = cx.global::<Ui>().theme.clone();
        let muted = color(theme.role("fg-muted"));
        let rows = self.skipped.iter().enumerate().map(|(i, (path, rule))| {
            let shown = self.skipped_display(path);
            div()
                .id(("search-skipped-row", i))
                .debug_selector(move || format!("search-skipped-row-{i}"))
                .role(Role::ListItem)
                .aria_label(format!("{shown}, {rule}"))
                .px_3()
                .py_1()
                .flex()
                .flex_col()
                .child(div().text_size(px(12.)).child(shown))
                .child(div().text_size(px(11.)).text_color(muted).child(rule.clone()))
        });
        Some(
            div()
                .id("search-skipped")
                .role(Role::List)
                .aria_label(format!("Skipped folders, {}", self.skipped.len()))
                .w_full()
                .max_h(px(220.))
                .flex()
                .flex_col()
                .border_t_1()
                .border_color(color(theme.role("border")))
                .bg(color(theme.role("surface")))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .px_3()
                        .py_1()
                        .text_size(px(12.))
                        .text_color(muted)
                        .child(div().flex_1().child("Folders skipped by exclude.toml"))
                        .child(
                            div()
                                .id("search-skipped-close")
                                .debug_selector(|| "search-skipped-close".into())
                                .role(Role::Button)
                                .aria_label("Close skipped folders")
                                .px_2()
                                .py_1()
                                .rounded_sm()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.show_skipped = false;
                                    cx.notify();
                                }))
                                .child("Close"),
                        ),
                )
                .child(div().id("search-skipped-rows").flex_1().min_h_0().overflow_y_scroll().children(rows)),
        )
    }
}

impl Render for SearchView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        let idle = self.query(cx).trim().is_empty();
        let now = Instant::now();
        if cx
            .global::<Operations>()
            .clipboard
            .as_ref()
            .and_then(|clipboard| clipboard.flash)
            .is_some_and(|started| crow_core::motion::copy_flash(ui.feedback.mode, started, now) > 0.0)
        {
            window.request_animation_frame();
        }
        div()
            .id("search-view")
            .role(Role::Group)
            .aria_label("Search")
            // keys::context_stack (KeyScope::Search) mirrors this.
            .key_context("Search")
            .on_action(cx.listener(Self::search_everything))
            .on_action(cx.listener(Self::cycle_match))
            .on_action(cx.listener(Self::close_action))
            .size_full()
            .flex()
            .flex_col()
            .bg(color(theme.role("bg")))
            .text_color(color(theme.role("fg")))
            .child(self.render_scope_bar(cx))
            .child(self.render_body(cx))
            .when(!idle, |view| view.children(self.render_skipped(cx)).child(self.render_footer(cx)))
    }
}

/// What the footer says; a pure function so the wording is unit-tested.
struct FooterState<'a> {
    searching: bool,
    /// Every accepted hit, including extra content lines for one file.
    hits: usize,
    /// Result rows: one per path.
    files: usize,
    capped: bool,
    error: Option<&'a str>,
}

fn footer_summary(state: FooterState<'_>) -> String {
    if let Some(error) = state.error {
        return error.lines().next().unwrap_or(error).to_owned();
    }
    if state.capped {
        return format!("First {MAX_RESULTS} files · refine the search");
    }
    let prefix = if state.searching { "Searching… " } else { "" };
    if state.hits == 0 {
        return if state.searching { "Searching…".to_owned() } else { "No results".to_owned() };
    }
    let results = if state.hits == 1 { "1 result".to_owned() } else { format!("{} results", state.hits) };
    if state.hits > state.files {
        let files = if state.files == 1 { "1 file".to_owned() } else { format!("{} files", state.files) };
        format!("{prefix}{results} in {files}")
    } else {
        format!("{prefix}{results}")
    }
}

/// "1 folder skipped" / "3 folders skipped", for the footer link and its aria label.
pub fn skipped_text(n: usize) -> String {
    if n == 1 { "1 folder skipped".to_owned() } else { format!("{n} folders skipped") }
}

/// The same wording as a sentence: "1 folder was skipped" / "3 folders were skipped".
pub fn skipped_was_text(n: usize) -> String {
    if n == 1 { "1 folder was skipped".to_owned() } else { format!("{n} folders were skipped") }
}

/// "Skip dev folders" for the enabled sets, or None when none is enabled.
pub fn skip_label(exclude: &Exclude) -> Option<String> {
    let names: Vec<&str> =
        SetName::ALL.into_iter().filter(|name| exclude.set(*name).enabled).map(SetName::id).collect();
    match names.as_slice() {
        [] => None,
        [one] => Some(format!("Skip {one} folders")),
        [first, second] => Some(format!("Skip {first} and {second} folders")),
        _ => {
            let (last, rest) = names.split_last().expect("not empty");
            Some(format!("Skip {} and {last} folders", rest.join(", ")))
        }
    }
}

impl Row {
    fn label(&self) -> String {
        let mut label =
            format!("{}, in {}, modified {}", display_text(&self.name), self.folder, format_time(self.modified));
        if let Some((line, text, _)) = &self.content {
            label.push_str(&format!(", line {line}: {text}"));
        }
        label
    }

    fn from_hit(hit: Hit, home: &Path, folder_root: Option<&Path>, query: &str) -> Row {
        let (content, name_match, name_ranges) = match hit.kind {
            HitKind::Content { line, text, ranges, .. } => (Some((line, text, ranges)), false, Vec::new()),
            HitKind::Name { ranges } => (None, true, ranges),
        };
        let strong_name = name_match && is_strong_name(&hit.path, &name_ranges, query);
        Self::new(
            hit.path,
            hit.is_dir,
            hit.modified,
            hit.score,
            name_match,
            strong_name,
            name_ranges,
            content,
            home,
            folder_root,
        )
    }

    fn from_spotlight(hit: crow_macos::SpotlightHit, query: &str, home: &Path) -> Row {
        let name = hit.path.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let ranges = ascii_match_ranges(&name, query);
        Self::new(hit.path, hit.is_dir, hit.modified, 0, true, true, ranges, None, home, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn new(
        path: PathBuf,
        is_dir: bool,
        modified: Option<SystemTime>,
        score: u32,
        name_match: bool,
        strong_name: bool,
        name_ranges: Vec<Range<usize>>,
        content: Option<(u64, String, Vec<Range<usize>>)>,
        home: &Path,
        folder_root: Option<&Path>,
    ) -> Row {
        let name = path.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let folder = match folder_root.and_then(|root| path.parent()?.strip_prefix(root).ok()) {
            Some(relative) if relative.as_os_str().is_empty() => ".".to_owned(),
            Some(relative) => relative.to_string_lossy().into_owned(),
            None => path.parent().map_or_else(String::new, |parent| crate::style::display_path(parent, home)),
        };
        let folder = display_text(&folder).into_owned();
        let ext = Path::new(&name).extension().and_then(|ext| ext.to_str()).unwrap_or("");
        let is_package = is_dir && matches!(ext, "app" | "bundle" | "framework" | "photoslibrary");
        let mut row = Row {
            path,
            name,
            folder,
            is_dir,
            is_package,
            modified,
            score,
            name_match,
            strong_name,
            name_ranges,
            content: None,
            content_shown: None,
        };
        if let Some(content) = content {
            row.set_content(content);
        }
        row
    }

    fn set_content(&mut self, (line, text, ranges): (u64, String, Vec<Range<usize>>)) {
        let (shown, shown_ranges) = content_excerpt(&text, &ranges);
        self.content = Some((line, text, ranges));
        self.content_shown = Some((line, shown, shown_ranges));
    }
}

/// Whether a name hit matched the query contiguously, so it ranks above
/// content hits; scattered fuzzy name hits do not. nucleo's own ranges decide
/// (one merged range), so accent folding and NFD names count like the matcher
/// does; a plain case-insensitive substring also counts.
fn is_strong_name(path: &Path, ranges: &[Range<usize>], query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return false;
    }
    if ranges.len() == 1 {
        return true;
    }
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else { return false };
    name.to_lowercase().contains(&query.to_lowercase())
}

/// Every ASCII-case-insensitive occurrence of `query` in `name`, for Spotlight
/// rows (Spotlight returns no match positions).
fn ascii_match_ranges(name: &str, query: &str) -> Vec<Range<usize>> {
    if query.is_empty() || !query.is_ascii() || !name.is_ascii() {
        return Vec::new();
    }
    let name = name.to_ascii_lowercase();
    let query = query.to_ascii_lowercase();
    name.match_indices(&query).map(|(start, _)| start..start + query.len()).collect()
}

/// A long matching line starts near its first match, with a leading ellipsis.
fn content_excerpt(text: &str, ranges: &[Range<usize>]) -> (String, Vec<Range<usize>>) {
    // Drop the line's indent so the text starts right after the line number.
    let indent = text.len() - text.trim_start().len();
    let text = text[indent..].replace('\t', " ");
    let ranges: Vec<Range<usize>> = ranges
        .iter()
        .map(|range| range.start.saturating_sub(indent)..range.end.saturating_sub(indent))
        .filter(|range| !range.is_empty())
        .collect();
    let ranges = ranges.as_slice();
    let first = ranges.first().map_or(0, |range| range.start);
    if first <= 60 {
        return (text, ranges.to_vec());
    }
    let mut offset = first - 20;
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let prefix = "\u{2026}";
    let shifted = ranges
        .iter()
        .map(|range| {
            let start = range.start.saturating_sub(offset) + prefix.len();
            let end = range.end.saturating_sub(offset) + prefix.len();
            start..end.max(start)
        })
        .collect();
    (format!("{prefix}{}", &text[offset..]), shifted)
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
    fn footer_summary_copy() {
        let state = |searching, hits, files, capped, error| FooterState { searching, hits, files, capped, error };
        assert_eq!(footer_summary(state(true, 0, 0, false, None)), "Searching…");
        assert_eq!(footer_summary(state(true, 44, 15, false, None)), "Searching… 44 results in 15 files");
        assert_eq!(footer_summary(state(false, 44, 15, false, None)), "44 results in 15 files");
        assert_eq!(footer_summary(state(false, 3, 3, false, None)), "3 results");
        assert_eq!(footer_summary(state(false, 1, 1, false, None)), "1 result");
        assert_eq!(footer_summary(state(false, 2, 1, false, None)), "2 results in 1 file");
        assert_eq!(footer_summary(state(false, 0, 0, false, None)), "No results");
        assert_eq!(footer_summary(state(false, 20_000, 10, true, None)), "First 20000 files · refine the search");
        assert_eq!(
            footer_summary(state(false, 0, 0, false, Some("Invalid pattern: unclosed group"))),
            "Invalid pattern: unclosed group"
        );
    }

    #[test]
    fn skipped_text_is_singular_for_one() {
        assert_eq!(skipped_text(1), "1 folder skipped");
        assert_eq!(skipped_text(3), "3 folders skipped");
        assert_eq!(skipped_was_text(1), "1 folder was skipped");
        assert_eq!(skipped_was_text(3), "3 folders were skipped");
    }

    #[test]
    fn skip_label_names_enabled_sets() {
        let mut exclude = Exclude::default();
        assert_eq!(skip_label(&exclude).as_deref(), Some("Skip dev folders"));
        exclude.media.enabled = Some(true);
        assert_eq!(skip_label(&exclude).as_deref(), Some("Skip dev and media folders"));
        exclude.system.enabled = Some(true);
        assert_eq!(skip_label(&exclude).as_deref(), Some("Skip dev, media and system folders"));
        for set in [&mut exclude.dev, &mut exclude.media, &mut exclude.system] {
            set.enabled = Some(false);
        }
        assert_eq!(skip_label(&exclude), None);
    }

    #[test]
    fn contiguous_name_matches_are_strong() {
        // The ranges the engine would report for this name and query.
        fn strong(name: &str, query: &str) -> bool {
            let ranges =
                crow_search::FuzzyMatcher::new(query).indices(name).map(|(_, ranges)| ranges).unwrap_or_default();
            is_strong_name(&Path::new("/a").join(name), &ranges, query)
        }
        assert!(strong("configuration.md", "config"));
        assert!(strong("Config.toml", "CONFIG"));
        assert!(!strong("gpui-stop-synthetic-drag-on-file-drag.patch", "config"));
        assert!(!is_strong_name(Path::new("/a/config.toml"), &std::iter::once(0..6).collect::<Vec<_>>(), "  "));
        // Accent folding and decomposed names, as nucleo matches them.
        assert!(strong("R\u{e9}sum\u{e9}.pdf", "resume"));
        assert!(strong("Re\u{301}sume\u{301}.pdf", "resume"));
    }

    #[test]
    fn spotlight_ranges_are_case_insensitive() {
        assert_eq!(ascii_match_ranges("Config.toml", "config"), vec![0..6]);
        assert_eq!(ascii_match_ranges("aConfig.Config", "config"), vec![1..7, 8..14]);
        assert!(ascii_match_ranges("r\u{e9}sum\u{e9}.txt", "sum").is_empty(), "non-ASCII names keep no ranges");
        assert!(ascii_match_ranges("Config.toml", "c\u{f6}nfig").is_empty(), "non-ASCII queries keep no ranges");
    }

    #[test]
    fn content_excerpt_starts_near_the_match() {
        let text = format!("{}needle{}", "x".repeat(200), "y".repeat(50));
        let ranges: Vec<Range<usize>> = std::iter::once(200..206).collect();
        let (shown, shifted) = content_excerpt(&text, &ranges);
        assert!(shown.starts_with('\u{2026}'));
        assert_eq!(&shown[shifted[0].clone()], "needle");
        assert!(shown.len() < text.len());
        let hit: Vec<Range<usize>> = std::iter::once(0..6).collect();
        let (text, ranges) = content_excerpt("needle in beta", &hit);
        assert_eq!(text, "needle in beta");
        let expected: Vec<Range<usize>> = std::iter::once(0..6).collect();
        assert_eq!(ranges, expected);
        // The indent goes; the ranges follow the text.
        let hit: Vec<Range<usize>> = std::iter::once(9..15).collect();
        let (text, ranges) = content_excerpt("    \t    needle()", &hit);
        assert_eq!(text, "needle()");
        assert_eq!(&text[ranges[0].clone()], "needle");
    }

    #[test]
    fn spotlight_forces_names() {
        assert_eq!(effective_match_for(Scope::ThisMac, Match::Contents), Match::Names);
        assert_eq!(effective_match_for(Scope::ThisMac, Match::All), Match::Names);
        assert_eq!(effective_match_for(Scope::Folder, Match::Contents), Match::Contents);
        assert_eq!(effective_match_for(Scope::Folder, Match::All), Match::All);
    }
}
