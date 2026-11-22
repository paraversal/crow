//! Cmd-F's search panel: names and contents from `crow-search` in This Folder or
//! Home, or Spotlight names on This Mac. Results stream in; the footer counts
//! results and skipped folders, and lists the rule that skipped each one.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use crow_core::{
    ColumnWidth, Exclude, KeymapLayer, ResizeDrag, SavedExclusions, SavedKind, SavedMatch, SavedQuery, SavedQueryId,
    SavedScope, SearchMatch, Selection, SetName, TableLayout, TagId,
};
use crow_fs::ops::Request;
use crow_fs::{AllowAll, CloudStatus, Entry, ReadGate};
use crow_search::{Control, Hit, HitKind, Query, RuleSet, Skip, SkipReason, Summary};
use futures::StreamExt;
use futures::channel::mpsc::unbounded;
use gpui::{
    AnyElement, App, Bounds, Context, Entity, EventEmitter, ExternalDragPayload, FileDragPaths, FocusHandle, Focusable,
    FontWeight, Image, IntoElement, MouseButton, MouseDownEvent, Pixels, Point, Render, Role, ScrollStrategy,
    SharedString, Subscription, Toggled, UniformListDecoration, UniformListScrollHandle, Window, div, img, prelude::*,
    px, relative, uniform_list,
};

use crate::actions::{
    CloseSearch, CopyFiles, CopyPath, CutFiles, CycleMatch, DeletePermanently, ExtendDown, ExtendToFirst, ExtendToLast,
    ExtendUp, MoveDown, MoveToFirst, MoveToLast, MoveToTrash, MoveUp, OpenSelection, QuickLook, Rename,
    SearchEverything, SearchFocusField, SearchFocusResults, SearchHome, SearchReveal, SelectAll,
};
use crate::chrome::{Keycap, KeycapWeight, with_action_tooltip, with_tooltip};
use crate::cloud_status::{self, CloudStatusService};
use crate::extensions::Ext;
use crate::icons::{self, Glyph};
use crate::info_pane::{InfoKey, InfoTarget, SelectionSummary, SelectionSummaryBuilder};
use crate::keys::{self, KeyScope};
use crate::operations::Operations;
use crate::pane::{DraggedSelection, island_edges};
use crate::preview::{self, Preview};
use crate::style::{
    active_selection, cloud_icon, color, color_alpha, cut_outline, display_text, field_ring, format_time,
    format_time_short, highlight_text, inactive_selection, map_range, truncate_middle_parts,
};
use crate::text_input::{TextInput, TextInputEvent};
use crate::trash::PRIVACY_SETTINGS_URL;
use crate::ui::Ui;

/// Rows for strong name matches (the name contains the query contiguously). When
/// these fill, the search is cancelled: the query is too broad to be useful.
pub(crate) const MAX_STRONG_ROWS: usize = 20_000;
/// How long a renamed row outlives a refresh that did not report it. Spotlight
/// answers a renamed item with nothing for a couple of seconds (measured 2.4 s).
const RENAME_PIN: Duration = Duration::from_secs(10);
/// Rows for everything else: content-only and weak (scattered) fuzzy name hits.
/// When these fill, contents stop being read; names keep streaming.
pub(crate) const MAX_OTHER_ROWS: usize = 20_000;
/// The results' side padding and the gap between their cells, in points.
const RESULT_INSET: f32 = 12.0;
const RESULT_GAP: f32 = 8.0;

/// The last widths a search's columns were dragged to, so the next search in this
/// session opens with them, as a list keeps its widths from folder to folder.
#[derive(Clone)]
struct SearchColumns(TableLayout);

impl gpui::Global for SearchColumns {}

/// Name, Folder and Modified at their default widths (Name's is the target it keeps
/// before the others give way).
fn default_search_table() -> TableLayout {
    TableLayout::new(vec![
        ColumnWidth { id: "name".into(), width: 220.0, min_width: 140.0 },
        ColumnWidth { id: "folder".into(), width: 180.0, min_width: 80.0 },
        ColumnWidth { id: "modified".into(), width: 110.0, min_width: 70.0 },
    ])
}
const ROW_HEIGHT: f32 = 46.0;

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

/// One way to widen a search that found nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Widen {
    /// Search everything: `n` skipped folders that it would walk.
    Everything(usize),
    /// Scope is a folder other than Home.
    Home,
    /// Match is Names or Contents (not All).
    AllMatch,
}

/// The ways to widen a search that found nothing, most specific first.
fn widen_options(scope: Scope, folder_is_home: bool, matching: Match, liftable: usize, everything: bool) -> Vec<Widen> {
    let mut options = Vec::new();
    if liftable > 0 && !everything {
        options.push(Widen::Everything(liftable));
    }
    if scope == Scope::Folder && !folder_is_home {
        options.push(Widen::Home);
    }
    if matching != Match::All && scope != Scope::ThisMac {
        options.push(Widen::AllMatch);
    }
    options
}

pub enum SearchEvent {
    Close,
    FocusResults,
    Open {
        path: PathBuf,
        is_dir: bool,
    },
    Reveal(PathBuf),
    Rename(PathBuf),
    Run(Request),
    Copy {
        paths: Arc<[PathBuf]>,
        cut: bool,
    },
    /// The band's Save/Update button.
    Save,
    /// Rename the selected results with one rule and one preview.
    BatchRename(Arc<[crow_core::batch_rename::RenameInput]>),
}

impl EventEmitter<SearchEvent> for SearchView {}

pub struct SearchView {
    /// The saved record this session shows; `None` is the transient Cmd-F overlay
    /// or a Tag destination.
    saved: Option<SavedQuery>,
    /// A Tag destination: a fixed tag predicate over Spotlight's index.
    tag: Option<TagId>,
    /// A unique owner id, so a reply can name its tab/session.
    session_id: u64,
    /// Bumped when the persisted spec is replaced by an external edit.
    spec_generation: u64,
    /// The clock captured once at the start of the current run; the age cutoff and
    /// the Spotlight client-side filter both use it.
    run_started: SystemTime,
    /// The saved-queries generation this session last applied.
    saved_generation: u64,
    saved_removed: bool,
    /// The display title: the saved name, or the folder's name for the overlay.
    title: String,
    /// The predicates resolved from the saved record, evaluated by the engine.
    predicates: crow_search::Predicates,
    /// Predicate-only results: no text, a metadata hit kind.
    metadata_only: bool,
    /// The engine's metadata result budget ended the scan.
    metadata_capped: bool,
    /// Paths seen by the current run, for refresh pruning.
    seen: std::collections::HashSet<PathBuf>,
    /// Rows a batch rename or its Undo just moved, kept until a run reports them.
    pinned: HashMap<PathBuf, Instant>,
    /// A refresh keeps the old snapshot until a complete generation proves removals.
    refreshing: bool,
    refresh_names: usize,
    refresh_other: usize,
    /// The last accepted snapshot is older than the file system or a failed run.
    stale: bool,
    /// The session is the active visible destination (watchers rerun only then).
    active: bool,
    /// A watcher or config change arrived while inactive.
    dirty: bool,
    /// The recursive watcher for a saved destination's bounded root.
    watch: Option<crow_macos::FsEvents>,
    watch_root: Option<PathBuf>,
    watch_debounce_running: bool,
    folder: PathBuf,
    /// The folder's own name, for the field's placeholder.
    folder_label: String,
    home: PathBuf,
    show_hidden: bool,
    /// The `search_includes_hidden` setting: visit dot entries while Show Hidden
    /// Files is off. Changing it reruns an open query.
    search_includes_hidden: bool,
    scope: Scope,
    matching: Match,
    everything: bool,
    /// "Skip dev folders"; None hides the switch when no set is enabled.
    skip_label: Option<String>,
    input: Entity<TextInput>,
    focus: FocusHandle,
    results: Vec<Row>,
    by_path: HashMap<PathBuf, usize>,
    skipped: Vec<Skip>,
    error: Option<SharedString>,
    searching: bool,
    /// The strong-name budget filled: the walk was cancelled.
    names_capped: bool,
    /// The other budget filled and the engine stopped reading contents.
    contents_stopped: bool,
    /// New rows the other budget refused.
    dropped: usize,
    /// When `start_search` began the current search; for the log and first-hit ms.
    started: Instant,
    /// Milliseconds from `started` to the first accepted hit.
    first_hit_ms: Option<u64>,
    control: Control,
    generation: u64,
    run_id: RunId,
    exclude_generation: u64,
    /// The compiled rules for `exclude_generation`; the glob compile is ~14 ms.
    rules: Option<(u64, Arc<RuleSet>)>,
    selection: Selection<PathBuf>,
    /// Bumped on every selection change; the Info pane keys its load on it.
    selection_generation: u64,
    /// The Info tab's multi-selection summary, rebuilt only when the selection changes.
    summary_cache: RefCell<Option<(u64, SelectionSummary)>>,
    cursor: usize,
    /// The user picked a row, so streaming batches stop re-selecting the best hit.
    user_moved: bool,
    /// Every accepted hit, including extra content lines for the same row.
    hit_count: usize,
    /// How many leading rows are name matches with a contiguous query match.
    name_count: usize,
    /// How many rows after the name block are content-only hits.
    content_count: usize,
    /// How many rows at the tail are weak fuzzy name hits.
    weak_count: usize,
    /// Content matches unavailable because their file is a cloud placeholder.
    cloud_skipped: u64,
    /// Candidates whose tags could not be read for a tag predicate.
    tag_errors: u64,
    /// This view's id in the cloud-status service's visible-row sources.
    cloud_source: u64,
    /// This view's plugin demand source.
    plugin_surface: crate::plugins::SurfaceId,
    /// The selected row's preview, loaded off the main thread.
    preview: Option<LoadedPreview>,
    preview_generation: u64,
    preview_plugin_generation: u64,
    preview_views_generation: u64,
    preview_cancel: Arc<AtomicBool>,
    scroll: UniformListScrollHandle,
    /// The results list's drawn bounds, captured at prepaint for drop hit testing.
    list_bounds: std::rc::Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The folder result under a drag, outlined like a folder row in a listing.
    drop_hover: Option<PathBuf>,
    /// Name, Folder and Modified widths, shared by the header and every row.
    table: TableLayout,
    /// A header divider being dragged.
    resize: Option<ResizeDrag>,
    show_skipped: bool,
    /// The row whose context menu is open, while it is.
    menu: Option<usize>,
    /// Plugin commands appended to the open context menu.
    menu_plugins: Vec<crate::actions::PluginCommand>,
    _subscription: Subscription,
    _ui_subscription: Subscription,
    _cloud_subscription: Option<Subscription>,
    /// Repaint the band's focus ring when the field or the results take focus.
    _focus_subscriptions: Vec<Subscription>,
    /// Focus subscriptions need a window; attach once on the first render, so a
    /// saved-query destination can be built from a navigation action.
    focus_attached: bool,
    /// The retained metadata row order once the engine finishes a predicate-only run.
    metadata_order_dirty: bool,
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
    /// A dot name or the Finder hidden flag; drawn dimmed, spoken as "hidden".
    hidden: bool,
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
    Home,
    Everything,
}

/// Names every batch and completion. The session id separates tabs, the location
/// id keeps a removed/renamed record distinct, and the two generations reject an
/// obsolete run inside one session.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RunId {
    session: u64,
    location: Option<SavedQueryId>,
    spec_generation: u64,
    run_generation: u64,
}

enum Update {
    Hits(Vec<Hit>, RunId),
    Done(Summary, RunId),
    Spotlight(Vec<crow_macos::SpotlightHit>, RunId),
    SpotlightDone(Result<(), String>, RunId),
}

impl Update {
    fn run_id(&self) -> &RunId {
        match self {
            Update::Hits(_, run) | Update::Done(_, run) | Update::Spotlight(_, run) | Update::SpotlightDone(_, run) => {
                run
            }
        }
    }
}

impl Drop for SearchView {
    fn drop(&mut self) {
        self.control.cancel();
        self.preview_cancel.store(true, Ordering::Relaxed);
    }
}

/// Search's visible-range demand for decoration values, mirroring the list's
/// `PluginRows` decoration. It only queues demand; the coordinator submits it.
struct PluginSearchRows {
    view: Entity<SearchView>,
}

impl UniformListDecoration for PluginSearchRows {
    fn compute(
        &self,
        visible_range: Range<usize>,
        _bounds: Bounds<Pixels>,
        _scroll_offset: Point<Pixels>,
        _item_height: Pixels,
        _item_count: usize,
        _window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let descriptors = crate::plugins::decoration_ids(cx);
        if descriptors.is_empty() {
            return div().into_any_element();
        }
        let (surface, scope, scope_version, display, rows) = {
            let view = self.view.read(cx);
            (
                view.plugin_surface,
                view.plugin_scope_key(),
                view.generation,
                view.scope_label(),
                view.plugin_rows(visible_range),
            )
        };
        // Search may span repositories (Home, This Mac); each result belongs to
        // its own parent folder's worktree, and decorations need no single
        // repository value.
        let rows: Vec<crate::plugins::RowInput> = rows
            .into_iter()
            .map(|mut row| {
                if let Some(parent) = row.path.parent() {
                    row.git = crate::git::entry_host(cx, parent, &row.path, row.is_dir, false);
                }
                row
            })
            .collect();
        if !rows.is_empty() {
            cx.global_mut::<crate::plugins::Plugins>().surfaces.queue_rows(
                surface,
                scope,
                scope_version,
                display,
                0,
                descriptors,
                rows,
            );
        }
        div().into_any_element()
    }
}

fn next_session_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

pub(crate) fn spotlight_expression_for(predicates: &crow_search::Predicates, text: &str) -> Option<String> {
    let content_type = match predicates.kind.as_ref() {
        Some(crow_search::Kind::Folder) => Some("public.folder"),
        Some(crow_search::Kind::File) => Some("public.data"),
        Some(crow_search::Kind::Image) => Some("public.image"),
        Some(crow_search::Kind::Pdf) => Some("com.adobe.pdf"),
        Some(crow_search::Kind::Audio) => Some("public.audio"),
        Some(crow_search::Kind::Video) => Some("public.movie"),
        Some(crow_search::Kind::Archive) => Some("public.archive"),
        Some(crow_search::Kind::Text) => Some("public.text"),
        Some(crow_search::Kind::Extension(_)) | None => None,
    };
    let extension = match predicates.kind.as_ref() {
        Some(crow_search::Kind::Extension(extension)) => Some(extension.clone()),
        _ => None,
    };
    crow_macos::SpotlightPredicate {
        content_type,
        extension,
        modified_within_secs: predicates.modified_within.map(|window| window.as_secs()),
        tags: predicates.tags.clone(),
        name_contains: (!text.is_empty()).then(|| text.to_owned()),
    }
    .expression()
}

pub(crate) fn predicates_for_saved(saved: &SavedQuery) -> crow_search::Predicates {
    let kind = saved.kind.as_ref().map(|kind| match kind {
        SavedKind::Folder => crow_search::Kind::Folder,
        SavedKind::File => crow_search::Kind::File,
        SavedKind::Image => crow_search::Kind::Image,
        SavedKind::Pdf => crow_search::Kind::Pdf,
        SavedKind::Audio => crow_search::Kind::Audio,
        SavedKind::Video => crow_search::Kind::Video,
        SavedKind::Archive => crow_search::Kind::Archive,
        SavedKind::Text => crow_search::Kind::Text,
        SavedKind::Extension(extension) => crow_search::Kind::Extension(extension.clone()),
    });
    crow_search::Predicates { kind, modified_within: saved.modified_within().ok().flatten(), tags: saved.tags.clone() }
}

impl SearchView {
    /// The transient Cmd-F overlay: the panel's scope, match and hidden policy
    /// start from the window and settings, and no saved record owns it.
    pub fn new(
        folder: PathBuf,
        home: PathBuf,
        show_hidden: bool,
        query: String,
        everything: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::build(folder, home, show_hidden, query, everything, None, None, cx)
    }

    /// A saved-query destination owned by a tab. Scope, match, hidden and
    /// exclusion policy come from the record; the input starts with its text.
    pub fn saved(query: SavedQuery, home: PathBuf, _show_hidden: bool, cx: &mut Context<Self>) -> Self {
        let folder = match query.scope {
            SavedScope::Folder => query.root_path(&home).ok().flatten().unwrap_or_else(|| home.clone()),
            SavedScope::Home | SavedScope::ThisMac => home.clone(),
        };
        let query_text = query.text.clone();
        let everything = query.exclusions == SavedExclusions::Everything;
        Self::build(folder, home, query.enter_hidden_folders, query_text, everything, Some(query), None, cx)
    }

    /// A Finder tag destination: every indexed item with the tag, names through
    /// Spotlight. The text field can narrow that set; the tag stays fixed.
    pub fn tag(tag: TagId, home: PathBuf, show_hidden: bool, cx: &mut Context<Self>) -> Self {
        Self::build(home.clone(), home, show_hidden, String::new(), true, None, Some(tag), cx)
    }

    #[allow(clippy::too_many_arguments)] // the saved/tag constructors share one setup path
    fn build(
        folder: PathBuf,
        home: PathBuf,
        show_hidden: bool,
        query: String,
        everything: bool,
        saved: Option<SavedQuery>,
        tag: Option<TagId>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.on_release(Self::clear_preview).detach();
        let folder_label = crate::style::display_name(&folder);
        let placeholder = if let Some(tag) = &tag {
            format!("Search {}", tag.as_str())
        } else {
            match &saved {
                Some(saved) => format!("Search {}", saved.name),
                None => Scope::Folder.label(&folder_label).to_string(),
            }
        };
        let input = cx.new(|cx| TextInput::new(query.clone(), placeholder, cx).bare().with_role(Role::SearchInput));
        let focus = cx.focus_handle();
        let subscription = cx.subscribe(&input, |this, input, event, cx| match event {
            TextInputEvent::Changed => {
                let query = input.read(cx).text().to_owned();
                this.start_search(&query, cx);
            }
            // Return moves into the results, on the first one; Return there opens it.
            TextInputEvent::Confirmed => cx.emit(SearchEvent::FocusResults),
            TextInputEvent::Cancelled => {
                if this.show_skipped {
                    this.show_skipped = false;
                    cx.notify();
                } else if this.saved.is_some() || this.tag.is_some() {
                    cx.emit(SearchEvent::FocusResults);
                } else {
                    cx.emit(SearchEvent::Close);
                }
            }
            TextInputEvent::CompletionRequested => cx.emit(SearchEvent::FocusResults),
            TextInputEvent::BackspaceAtStart => {}
        });
        let exclude_generation = cx.global::<Ui>().exclude_generation;
        let skip_label_value = skip_label(&cx.global::<Ui>().exclude);
        let search_includes_hidden = if tag.is_some() {
            true
        } else {
            saved.as_ref().map_or(cx.global::<Ui>().settings.search_includes_hidden, |saved| saved.include_hidden)
        };
        let matching = if tag.is_some() {
            Match::Names
        } else {
            match saved.as_ref().map_or(cx.global::<Ui>().settings.search_match, |saved| match saved.matching {
                SavedMatch::All => SearchMatch::All,
                SavedMatch::Names => SearchMatch::Names,
                SavedMatch::Contents => SearchMatch::Contents,
            }) {
                SearchMatch::All => Match::All,
                SearchMatch::Names => Match::Names,
                SearchMatch::Contents => Match::Contents,
            }
        };
        let scope = if tag.is_some() {
            Scope::ThisMac
        } else {
            saved.as_ref().map_or(Scope::Folder, |saved| match saved.scope {
                SavedScope::Folder => Scope::Folder,
                SavedScope::Home => Scope::Home,
                SavedScope::ThisMac => Scope::ThisMac,
            })
        };
        let saved_generation = cx.global::<Ui>().saved_queries_generation;
        let ui_subscription = cx.observe_global::<Ui>(move |this, cx| {
            let exclude_generation = cx.global::<Ui>().exclude_generation;
            if this.exclude_generation != exclude_generation {
                this.exclude_generation = exclude_generation;
                this.skip_label = skip_label(&cx.global::<Ui>().exclude);
                if !this.everything {
                    this.refresh_now(cx);
                }
            }
            if this.saved.is_some() {
                let generation = cx.global::<Ui>().saved_queries_generation;
                if this.saved_generation != generation {
                    this.saved_generation = generation;
                    this.reload_saved_spec(cx);
                }
            }
        });
        let (cloud_source, cloud_subscription) = match cx.try_global::<CloudStatusService>() {
            Some(service) => {
                let entity = service.0.clone();
                let source = cloud_status::register_source(&entity, cx);
                let subscription = cx.observe(&entity, |search, _, cx| search.cloud_changed(cx));
                (source, Some(subscription))
            }
            None => (0, None),
        };
        let predicates = if let Some(tag) = &tag {
            crow_search::Predicates { tags: vec![tag.as_str().to_owned()], ..Default::default() }
        } else {
            saved.as_ref().map(predicates_for_saved).unwrap_or_default()
        };
        let everything = if tag.is_some() { true } else { everything };
        let title = if let Some(tag) = &tag {
            tag.as_str().to_owned()
        } else {
            saved.as_ref().map_or(folder_label.clone(), |saved| saved.name.clone())
        };
        let mut search = Self {
            saved: saved.clone(),
            tag,
            session_id: next_session_id(),
            spec_generation: 0,
            run_started: SystemTime::now(),
            saved_generation,
            saved_removed: false,
            predicates,
            metadata_only: false,
            metadata_capped: false,
            seen: std::collections::HashSet::new(),
            pinned: HashMap::new(),
            refreshing: false,
            refresh_names: 0,
            refresh_other: 0,
            stale: false,
            active: true,
            dirty: false,
            watch: None,
            watch_root: None,
            watch_debounce_running: false,
            title,
            folder,
            folder_label,
            home,
            show_hidden: saved.as_ref().map_or(show_hidden, |saved| saved.enter_hidden_folders),
            search_includes_hidden,
            scope,
            matching,
            everything,
            skip_label: skip_label_value,
            input,
            focus,
            results: Vec::new(),
            by_path: HashMap::new(),
            skipped: Vec::new(),
            error: None,
            searching: false,
            names_capped: false,
            contents_stopped: false,
            dropped: 0,
            started: Instant::now(),
            first_hit_ms: None,
            control: Control::new(),
            generation: 0,
            run_id: RunId { session: 0, location: None, spec_generation: 0, run_generation: 0 },
            exclude_generation,
            rules: None,
            selection: Selection::default(),
            selection_generation: 0,
            summary_cache: RefCell::new(None),
            cursor: 0,
            user_moved: false,
            hit_count: 0,
            name_count: 0,
            content_count: 0,
            weak_count: 0,
            cloud_skipped: 0,
            tag_errors: 0,
            cloud_source,
            plugin_surface: crate::plugins::new_surface(),
            preview: None,
            preview_generation: 0,
            preview_plugin_generation: 0,
            preview_views_generation: 0,
            preview_cancel: Arc::new(AtomicBool::new(false)),
            scroll: UniformListScrollHandle::new(),
            list_bounds: std::rc::Rc::new(Cell::new(None)),
            drop_hover: None,
            table: cx.try_global::<SearchColumns>().map_or_else(default_search_table, |columns| columns.0.clone()),
            resize: None,
            show_skipped: false,
            menu: None,
            menu_plugins: Vec::new(),
            _subscription: subscription,
            _ui_subscription: ui_subscription,
            _cloud_subscription: cloud_subscription,
            _focus_subscriptions: Vec::new(),
            focus_attached: false,
            metadata_order_dirty: false,
        };
        if !query.trim().is_empty() || ((saved.is_some() || search.tag.is_some()) && !search.predicates.is_empty()) {
            search.start_search(&query, cx);
        }
        if search.saved.is_some() || search.tag.is_some() {
            search.spawn_clock(cx);
        }
        search
    }

    /// A bounded clock for rolling-date queries and locations without a
    /// recursive watcher. Inactive tabs skip ticks.
    fn spawn_clock(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let mut ticks = 0u64;
            loop {
                cx.background_executor().timer(Duration::from_secs(60)).await;
                ticks += 1;
                let keep = this.update(cx, |this, cx| {
                    if !this.active || (this.saved.is_none() && this.tag.is_none()) {
                        return;
                    }
                    let rolling = this.predicates.modified_within.is_some();
                    let backstop = (this.scope != Scope::Folder || this.watch.is_none()) && ticks.is_multiple_of(5);
                    if rolling || backstop {
                        this.dirty = true;
                        this.refresh_dirty(cx);
                    }
                });
                if keep.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    #[cfg(test)]
    pub fn empty_message_for_test(&self, cx: &App) -> String {
        self.empty_message(cx)
    }

    #[cfg(test)]
    pub fn pinned_count_for_test(&self) -> usize {
        self.pinned.len()
    }

    /// Age every renamed-row pin past its limit, as if `RENAME_PIN` had passed.
    #[cfg(test)]
    pub fn expire_pins_for_test(&mut self) {
        let old = Instant::now().checked_sub(RENAME_PIN + Duration::from_secs(1)).expect("an old instant");
        for at in self.pinned.values_mut() {
            *at = old;
        }
    }

    /// A watcher notification in tests (and the real debounced path): mark the
    /// snapshot dirty; the active tab reruns it.
    #[cfg(test)]
    pub fn mark_dirty(&mut self, cx: &mut Context<Self>) {
        self.dirty = true;
        self.refresh_dirty(cx);
    }

    /// A watcher tick during a scan schedules one follow-up after it settles.
    fn refresh_dirty(&mut self, cx: &mut Context<Self>) {
        if self.active && self.dirty && !self.searching && !self.watch_debounce_running {
            self.dirty = false;
            self.refresh_now(cx);
        }
    }

    fn watch_root(&self) -> Option<PathBuf> {
        self.saved.as_ref()?;
        match self.scope {
            Scope::Folder => Some(self.folder.clone()),
            Scope::Home | Scope::ThisMac => None,
        }
    }

    /// Install the watcher before the scan starts, or replace it when the root
    /// moved. FSEvents replays everything since `since`, so a change that lands
    /// during the first scan still marks the session dirty.
    fn ensure_watch(&mut self, cx: &mut Context<Self>) {
        // The GPUI test scheduler rejects a wake from FSEvents' dispatch queue.
        // Tests drive `mark_dirty` directly instead.
        if cx.background_executor().is_test() {
            return;
        }
        let Some(root) = self.watch_root() else {
            self.watch = None;
            self.watch_root = None;
            return;
        };
        if self.watch.is_some() && self.watch_root.as_ref() == Some(&root) {
            return;
        }
        self.watch = None;
        self.watch_root = Some(root.clone());
        let since = crow_macos::current_event_id();
        let (tx, mut rx) = unbounded();
        let Some(root_name) = root.to_str() else { return };
        let Some(watch) = crow_macos::watch_under(root_name, since, Duration::from_millis(250), move |events| {
            let _ = tx.unbounded_send(events);
        }) else {
            return;
        };
        self.watch = Some(watch);
        cx.spawn(async move |this, cx| {
            while let Some(events) = rx.next().await {
                if events.is_empty() {
                    continue;
                }
                let _ = this.update(cx, |this, cx| {
                    if this.watch_root.as_ref() != Some(&root) {
                        return;
                    }
                    this.dirty = true;
                    if this.watch_debounce_running {
                        return;
                    }
                    this.watch_debounce_running = true;
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(Duration::from_millis(300)).await;
                        let _ = this.update(cx, |this, cx| {
                            this.watch_debounce_running = false;
                            this.refresh_dirty(cx);
                        });
                    })
                    .detach();
                });
            }
        })
        .detach();
    }

    pub fn plugin_surface(&self) -> crate::plugins::SurfaceId {
        self.plugin_surface
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
        self.control.cancel();
        self.generation += 1;
        self.searching = false;
        self.refreshing = false;
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

    /// The `search_includes_hidden` setting changed: rerun the open query, if any.
    pub fn set_search_includes_hidden(&mut self, include: bool, cx: &mut Context<Self>) {
        if self.search_includes_hidden == include {
            return;
        }
        self.search_includes_hidden = include;
        let query = self.input.read(cx).text().to_owned();
        if !query.is_empty() {
            self.start_search(&query, cx);
        }
    }

    fn plugin_scope_key(&self) -> crate::plugins::ScopeKey {
        crate::plugins::ScopeKey::Search { session: self.generation, origin: self.folder.clone() }
    }

    /// The row version shared by demand, rendering and the coordinator cache.
    fn plugin_row_version(&self, row: &Row) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.generation.hash(&mut hasher);
        row.path.hash(&mut hasher);
        row.is_dir.hash(&mut hasher);
        row.hidden.hash(&mut hasher);
        row.modified.hash(&mut hasher);
        hasher.finish()
    }

    /// One visible-range demand for plugin decorations. Search results have no
    /// size or kind column, so those EntryData fields stay empty.
    fn plugin_rows(&self, range: Range<usize>) -> Vec<crate::plugins::RowInput> {
        range
            .filter_map(|i| self.results.get(i))
            .map(|row| crate::plugins::RowInput {
                path: row.path.clone(),
                version: self.plugin_row_version(row),
                name: row.name.clone(),
                is_dir: row.is_dir,
                size: 0,
                kind: if row.is_dir { "Folder".into() } else { "File".into() },
                selected: self.selection.contains(&row.path),
                git: None,
            })
            .collect()
    }

    pub fn notify_plugin_scope(&self, cx: &mut Context<Self>) {
        if cx.try_global::<crate::plugins::Plugins>().is_some() {
            cx.global_mut::<crate::plugins::Plugins>().surfaces.notify_scope_changed(
                self.plugin_surface,
                self.scope_label(),
                Instant::now(),
            );
        }
    }

    fn request_plugin_status(&self, cx: &mut Context<Self>) {
        if cx.try_global::<crate::plugins::Plugins>().is_some() {
            crate::plugins::request_status(
                cx,
                self.plugin_surface,
                self.plugin_scope_key(),
                self.generation,
                self.scope_label(),
            );
        }
    }

    fn update_plugin_command_context(&self, window: &mut Window, cx: &mut Context<Self>) {
        if cx.try_global::<crate::plugins::Plugins>().is_none() {
            return;
        }
        if window.is_window_active() && self.focus.is_focused(window) {
            let selection = self
                .results
                .iter()
                .filter(|row| self.selection.contains(&row.path))
                .map(|row| (row.path.clone(), row.is_dir))
                .collect();
            let context = crate::plugins::FileCommandContext {
                window: window.window_handle().window_id(),
                surface: self.plugin_surface,
                scope: self.plugin_scope_key(),
                selection,
            };
            crate::plugins::set_command_context(window, Some(context), cx);
        } else {
            crate::plugins::set_command_context(window, None, cx);
        }
    }

    fn bump_selection(&mut self, cx: &mut Context<Self>) {
        self.selection_generation += 1;
        if cx.try_global::<crate::plugins::Plugins>().is_some() {
            cx.global_mut::<crate::plugins::Plugins>().surfaces.notify_selection_changed(
                self.plugin_surface,
                self.scope_label(),
                Instant::now(),
            );
        }
    }

    fn start_search(&mut self, query: &str, cx: &mut Context<Self>) {
        self.start_search_mode(query, false, cx);
    }

    /// Rerun the current draft without clearing the accepted snapshot: a watcher,
    /// an external edit, an exclusion reload or a rolling date tick.
    pub fn refresh_now(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).text().to_owned();
        if query.trim().is_empty() && self.predicates.is_empty() {
            return;
        }
        self.start_search_mode(&query, self.saved.is_some() || self.tag.is_some(), cx);
    }

    /// The saved record changed in `saved-queries.toml` (or this session just
    /// became visible after being hidden). Replace the spec and rerun, keeping a
    /// surviving selected path.
    fn reload_saved_spec(&mut self, cx: &mut Context<Self>) {
        if self.tag.is_some() {
            return;
        }
        let Some(id) = self.saved_query_id() else { return };
        let Some(saved) = cx.global::<Ui>().saved_queries.get(&id).cloned() else {
            self.cancel(cx);
            self.saved_removed = true;
            self.error = Some("This saved search was removed.".into());
            self.watch = None;
            self.watch_root = None;
            cx.notify();
            return;
        };
        if self.saved.as_ref() == Some(&saved) && !self.saved_removed {
            return;
        }
        self.saved_removed = false;
        self.apply_saved_spec(&saved, cx);
        self.refresh_now(cx);
        cx.notify();
    }

    fn apply_saved_spec(&mut self, saved: &SavedQuery, cx: &mut Context<Self>) {
        self.saved = Some(saved.clone());
        self.title = saved.name.clone();
        self.predicates = predicates_for_saved(saved);
        self.scope = match saved.scope {
            SavedScope::Folder => Scope::Folder,
            SavedScope::Home => Scope::Home,
            SavedScope::ThisMac => Scope::ThisMac,
        };
        self.matching = match saved.matching {
            SavedMatch::All => Match::All,
            SavedMatch::Names => Match::Names,
            SavedMatch::Contents => Match::Contents,
        };
        self.exclusions_changed(saved.exclusions == SavedExclusions::Everything);
        self.show_hidden = saved.enter_hidden_folders;
        self.search_includes_hidden = saved.include_hidden;
        self.folder = match saved.scope {
            SavedScope::Folder => saved.root_path(&self.home).ok().flatten().unwrap_or_else(|| self.home.clone()),
            SavedScope::Home | SavedScope::ThisMac => self.home.clone(),
        };
        self.folder_label = crate::style::display_name(&self.folder);
        self.spec_generation += 1;
        self.input.update(cx, |input, cx| {
            input.set_text_silent(saved.text.clone(), cx);
            input.set_placeholder(self.scope.label(&self.folder_label), cx);
        });
    }

    fn exclusions_changed(&mut self, everything: bool) {
        if self.everything != everything {
            self.everything = everything;
            // Compile again for the next configured run; Everything ignores the rules.
            self.rules = None;
        }
    }

    pub fn saved_query_id(&self) -> Option<SavedQueryId> {
        self.saved.as_ref().map(|saved| saved.id.clone())
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn is_destination(&self) -> bool {
        self.saved.is_some() || self.tag.is_some()
    }

    /// The tag predicate the form should prefill when saving from this session.
    pub fn tag_values(&self) -> &[String] {
        &self.predicates.tags
    }

    /// Deliver a batch from an older run. The window tests use this instead of
    /// sleeping: a stale folder/title/spec generation must be ignored.
    #[cfg(test)]
    pub fn inject_stale_spotlight_for_test(&mut self, hits: Vec<crow_macos::SpotlightHit>, cx: &mut Context<Self>) {
        let mut stale = self.run_id.clone();
        stale.location = None;
        stale.spec_generation = stale.spec_generation.wrapping_sub(1);
        stale.run_generation = stale.run_generation.wrapping_sub(1);
        self.apply(Update::Spotlight(hits, stale), cx);
    }

    /// The field text, including a destination's temporary draft.
    pub fn draft_text(&self, cx: &App) -> String {
        self.input.read(cx).text().to_owned()
    }

    pub fn scope_value(&self) -> SavedScope {
        match self.scope {
            Scope::Folder => SavedScope::Folder,
            Scope::Home => SavedScope::Home,
            Scope::ThisMac => SavedScope::ThisMac,
        }
    }

    pub fn match_value(&self) -> SavedMatch {
        match self.matching {
            Match::All => SavedMatch::All,
            Match::Names => SavedMatch::Names,
            Match::Contents => SavedMatch::Contents,
        }
    }

    pub fn hidden_flags(&self) -> (bool, bool) {
        (self.search_includes_hidden, self.show_hidden)
    }

    pub fn exclusions_value(&self) -> SavedExclusions {
        if self.everything { SavedExclusions::Everything } else { SavedExclusions::Configured }
    }

    pub fn predicate_values(&self) -> (Option<SavedKind>, Option<u32>) {
        (
            self.saved.as_ref().and_then(|saved| saved.kind.clone()),
            self.saved.as_ref().and_then(|saved| saved.modified_within_days),
        )
    }

    fn publish_count(&self, cx: &mut Context<Self>) {
        let Some(id) = self.saved_query_id() else { return };
        let partial = self.metadata_capped || self.names_capped || self.contents_stopped || self.stale;
        cx.global_mut::<Ui>().saved_query_counts.set(
            id,
            self.results.len() as u64,
            partial || self.refreshing,
            self.searching,
        );
        cx.refresh_windows();
    }

    pub fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.active == active {
            return;
        }
        self.active = active;
        if active {
            self.dirty = false;
            self.refresh_now(cx);
        } else {
            self.cancel(cx);
            self.watch = None;
            self.watch_root = None;
        }
    }

    pub fn leave_destination(&mut self, cx: &mut Context<Self>) {
        self.set_active(false, cx);
        if let Some(saved) = self.saved.clone() {
            self.apply_saved_spec(&saved, cx);
        }
    }

    fn start_search_mode(&mut self, query: &str, refresh: bool, cx: &mut Context<Self>) {
        if !self.active || self.saved_removed {
            return;
        }
        self.cancel(cx);
        let generation = self.generation;
        let control = Control::new();
        self.control = control.clone();
        let query = query.trim().to_owned();
        let metadata_only = query.is_empty() && !self.predicates.is_empty();
        let idle = query.is_empty() && !metadata_only;
        self.metadata_only = metadata_only;
        self.run_id = RunId {
            session: self.session_id,
            location: self.saved_query_id(),
            spec_generation: self.spec_generation,
            run_generation: generation,
        };
        let run_id = self.run_id.clone();
        self.skipped.clear();
        self.error = None;
        self.names_capped = false;
        self.contents_stopped = false;
        self.metadata_capped = false;
        self.dropped = 0;
        self.started = Instant::now();
        self.run_started = SystemTime::now();
        self.first_hit_ms = None;
        self.show_skipped = false;
        self.cloud_skipped = 0;
        self.tag_errors = 0;
        self.seen.clear();
        self.refresh_names = 0;
        self.refresh_other = 0;
        self.stale = false;
        if refresh && !idle {
            self.refreshing = true;
            // Keep the selected real path across a refresh. If it disappears, the
            // completion picks the nearest surviving row.
            if self.selection.cursor().is_some() {
                self.user_moved = true;
            }
            self.hit_count = self.results.len();
        } else {
            self.refreshing = false;
            self.results.clear();
            self.by_path.clear();
            self.selection.clear();
            self.bump_selection(cx);
            self.cursor = 0;
            self.user_moved = false;
            self.hit_count = 0;
            self.name_count = 0;
            self.content_count = 0;
            self.weak_count = 0;
            self.clear_preview(cx);
            self.preview_generation += 1;
        }
        if idle {
            self.publish_count(cx);
            cx.notify();
            return;
        }
        self.searching = true;
        self.ensure_watch(cx);
        self.notify_plugin_scope(cx);
        self.publish_count(cx);
        let root = match self.scope {
            Scope::Folder => &self.folder,
            Scope::Home | Scope::ThisMac => &self.home,
        };
        // The query text stays out of the log: queries can be private.
        tracing::debug!(
            target: "crow::search",
            root = %root.display(),
            scope = ?self.scope,
            matching = ?self.matching,
            effective_match = ?self.effective_match(),
            show_hidden = self.show_hidden,
            everything = self.everything,
            metadata_only,
            skip_label = ?self.skip_label,
            query_len = query.len(),
            "search started"
        );

        let (tx, mut rx) = unbounded();
        if self.scope == Scope::ThisMac {
            let query_text = query.clone();
            let predicates = self.predicates.clone();
            // A tag predicate turns the text into one raw expression: `mdfind -name`
            // cannot be combined with metadata attributes. Without tags, keep the
            // established `-name` stream.
            let expression = if query_text.is_empty() || !predicates.tags.is_empty() {
                spotlight_expression_for(&predicates, &query_text)
            } else {
                None
            };
            let effects = crate::effects::effects(cx);
            let cancel = control.cancel_flag();
            let run = run_id.clone();
            cx.background_spawn(async move {
                let hits = tx.clone();
                let hits_for_done = tx.clone();
                let run_for_hits = run.clone();
                let result = match expression {
                    Some(expression) => effects.spotlight_expression(
                        &expression,
                        Box::new(move |batch| {
                            let _ = hits.unbounded_send(Update::Spotlight(batch, run_for_hits.clone()));
                        }),
                        &cancel,
                    ),
                    None => effects.spotlight_names(
                        &query_text,
                        Box::new(move |batch| {
                            let _ = hits.unbounded_send(Update::Spotlight(batch, run_for_hits.clone()));
                        }),
                        &cancel,
                    ),
                };
                let _ = hits_for_done.unbounded_send(Update::SpotlightDone(result, run));
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
            let (names, contents) = query_parts(&query, self.effective_match());
            let read_gate = cx.try_global::<CloudStatusService>().map(|service| service.0.read(cx).gate());
            let query = Query {
                names,
                contents,
                enter_hidden_folders: self.show_hidden,
                include_hidden: self.search_includes_hidden,
                rules,
                everything: self.everything,
                read_gate,
                predicates: self.predicates.clone(),
                tag_reader: Some(crate::tags::reader()),
            };
            let run_for_hits = run_id.clone();
            let run_for_done = run_id.clone();
            cx.background_spawn(async move {
                let tx = tx;
                let summary = crow_search::search(
                    &root,
                    &query,
                    |batch| {
                        let _ = tx.unbounded_send(Update::Hits(batch, run_for_hits.clone()));
                    },
                    &control,
                );
                let _ = tx.unbounded_send(Update::Done(summary, run_for_done));
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
                    this.apply(update, cx);
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
        if update.run_id() != &self.run_id || update.run_id().run_generation != self.generation {
            return;
        }
        let _span = tracing::debug_span!("search batch").entered();
        match update {
            // Once the strong-name cap is hit the walk is cancelled; queued batches
            // are dropped rather than merged, so the UI is never busy draining them.
            // The other budget keeps names streaming, so its batches still apply.
            Update::Hits(_, _) if self.names_capped => {}
            Update::Hits(hits, _) => self.add_hits(hits, cx),
            Update::Done(summary, _) => self.finish(summary, cx),
            Update::Spotlight(_, _) if self.names_capped => {}
            Update::Spotlight(hits, _) => self.add_spotlight(hits, cx),
            Update::SpotlightDone(result, _) => {
                if let Err(error) = result {
                    self.error = Some(error.into());
                }
                let complete = self.error.is_none() && !self.names_capped;
                self.finish_refresh(complete, cx);
                tracing::debug!(
                    target: "crow::search",
                    elapsed_ms = self.started.elapsed().as_millis() as u64,
                    first_hit_ms = ?self.first_hit_ms,
                    rows = self.results.len(),
                    hits = self.hit_count,
                    names_capped = self.names_capped,
                    error = ?self.error,
                    "spotlight search finished"
                );
                self.searching = false;
                self.publish_count(cx);
                self.refresh_dirty(cx);
                cx.notify();
            }
        }
    }

    fn finish(&mut self, summary: Summary, cx: &mut Context<Self>) {
        let scanned = summary.scanned;
        self.cloud_skipped = summary.cloud_skipped;
        self.tag_errors = summary.tag_errors;
        self.metadata_capped = summary.capped;
        let skipped = summary.skipped.len();
        // Every reason is listed: rules, hidden folders, packages and
        // unreadable folders are the four ways a search can miss something.
        self.skipped = summary.skipped;
        if let Some(error) = summary.error {
            self.error = Some(error.into());
        }
        let unreadable = self.skipped.iter().any(|skip| matches!(skip.reason, SkipReason::Unreadable(_)));
        self.contents_stopped |= summary.contents_stopped;
        let complete = !summary.cancelled
            && !summary.capped
            && !unreadable
            && self.error.is_none()
            && !self.names_capped
            && !self.contents_stopped;
        self.finish_refresh(complete, cx);
        tracing::debug!(
            target: "crow::search",
            elapsed_ms = self.started.elapsed().as_millis() as u64,
            first_hit_ms = ?self.first_hit_ms,
            scanned,
            skipped,
            rows = self.results.len(),
            hits = self.hit_count,
            names_capped = self.names_capped,
            contents_stopped = self.contents_stopped,
            metadata_capped = self.metadata_capped,
            complete,
            dropped = self.dropped,
            error = ?self.error,
            "search finished"
        );
        self.searching = false;
        self.publish_count(cx);
        self.refresh_dirty(cx);
        cx.notify();
    }

    /// A refresh may remove an old row only when a complete generation proved the
    /// walk saw everything. A cancelled, failed or capped run keeps the old rows
    /// marked stale instead of silently losing changes.
    fn finish_refresh(&mut self, complete: bool, cx: &mut Context<Self>) {
        if self.refreshing {
            if complete || self.metadata_capped || self.names_capped || self.contents_stopped {
                self.prune_unseen();
            } else {
                self.stale = true;
                let limit = if self.metadata_only { MAX_STRONG_ROWS } else { MAX_STRONG_ROWS + MAX_OTHER_ROWS };
                self.results.truncate(limit);
            }
            if self.metadata_only {
                self.metadata_order_dirty = true;
            } else if self.scope != Scope::ThisMac {
                self.results.sort_by_key(|row| {
                    let rank = if row.strong_name {
                        0
                    } else if row.name_match {
                        2
                    } else {
                        1
                    };
                    (rank, std::cmp::Reverse(if row.strong_name { row.score } else { 0 }))
                });
            }
            self.reindex_rows();
            self.refreshing = false;
        }
        if self.metadata_only {
            self.sort_metadata_results();
        }
        let selected = self.selection.len();
        let cursor = self.selection.cursor().cloned();
        self.selection.retain(|path| self.by_path.contains_key(path));
        if let Some(&index) = self.selection.cursor().and_then(|path| self.by_path.get(path)) {
            self.cursor = index;
        } else if self.results.is_empty() {
            self.cursor = 0;
        } else {
            self.cursor = self.cursor.min(self.results.len() - 1);
            self.selection.select_only(self.results[self.cursor].path.clone());
        }
        if self.selection.len() != selected || self.selection.cursor() != cursor.as_ref() {
            self.bump_selection(cx);
            self.clear_preview(cx);
            self.load_preview(cx);
        }
    }

    /// Drop rows the completed generation did not report, rebuilding the tables.
    fn prune_unseen(&mut self) {
        // A run that reports a pinned row ends its pin; an old one expires.
        let now = Instant::now();
        self.pinned.retain(|path, at| !self.seen.contains(path) && now.duration_since(*at) < RENAME_PIN);
        if !self.pinned.is_empty() {
            let pinned = &self.pinned;
            let seen = std::mem::take(&mut self.seen);
            self.results.retain(|row| seen.contains(&row.path) || pinned.contains_key(&row.path));
            self.seen = seen;
            return;
        }
        if self.seen.len() == self.results.len() && self.results.iter().all(|row| self.seen.contains(&row.path)) {
            return;
        }
        let seen = std::mem::take(&mut self.seen);
        self.results.retain(|row| seen.contains(&row.path));
    }

    fn reindex_rows(&mut self) {
        self.by_path.clear();
        for (index, row) in self.results.iter().enumerate() {
            self.by_path.insert(row.path.clone(), index);
        }
        if self.metadata_only {
            self.name_count = 0;
            self.content_count = 0;
            self.weak_count = self.results.len();
        } else {
            self.name_count = self.results.iter().filter(|row| row.strong_name).count();
            self.content_count = self.results.iter().skip(self.name_count).filter(|row| !row.name_match).count();
            self.weak_count = self.results.len() - self.name_count - self.content_count;
        }
        self.hit_count = self.results.len();
    }

    /// Predicate-only results are ordered by natural name, then raw path, whatever
    /// order the parallel walk or a refresh delivered them in.
    fn sort_metadata_results(&mut self) {
        if !self.metadata_order_dirty {
            return;
        }
        self.metadata_order_dirty = false;
        self.results.sort_by(|a, b| crow_core::natural_order(&a.name, &b.name).then_with(|| a.path.cmp(&b.path)));
        self.reindex_rows();
        if let Some(path) = self.selection.cursor().cloned()
            && let Some(&index) = self.by_path.get(&path)
        {
            self.cursor = index;
        }
    }

    fn add_hits(&mut self, hits: Vec<Hit>, cx: &mut Context<Self>) {
        if self.refreshing {
            self.add_refresh_hits(hits, cx);
            return;
        }
        let query = self.query(cx);
        let staging = crow_fs::ops::owned_staging_paths();
        let mut changed = false;
        let mut fresh: Vec<Row> = Vec::new();
        let mut fresh_at: HashMap<PathBuf, usize> = HashMap::new();
        let mut fresh_strong = 0usize;
        let mut fresh_others = 0usize;
        for hit in hits {
            if staging.iter().any(|container| hit.path.starts_with(container)) {
                continue;
            }
            // Compute the rank once: the budgets and the row both need it.
            let strong_name = match &hit.kind {
                HitKind::Name { ranges } => is_strong_name(&hit.path, ranges, &query),
                HitKind::Metadata { .. } => self.metadata_only,
                HitKind::Content { .. } => false,
            };
            let known = self.by_path.contains_key(&hit.path) || fresh_at.contains_key(&hit.path);
            if !known {
                if strong_name {
                    if self.name_count + fresh_strong >= MAX_STRONG_ROWS {
                        changed = true;
                        self.names_capped = true;
                        self.metadata_capped |= self.metadata_only;
                        self.control.cancel();
                        break;
                    }
                    fresh_strong += 1;
                } else {
                    let other = self.content_count + self.weak_count + fresh_others;
                    if other >= MAX_OTHER_ROWS {
                        changed = true;
                        self.dropped += 1;
                        self.stop_contents_at_cap();
                        // A strong name later in this batch must still get in.
                        continue;
                    }
                    fresh_others += 1;
                    if other + 1 == MAX_OTHER_ROWS {
                        self.stop_contents_at_cap();
                    }
                }
            }
            if self.first_hit_ms.is_none() {
                self.first_hit_ms = Some(self.started.elapsed().as_millis() as u64);
            }
            // A content hit carries the file's other matching lines as a count.
            let more = match &hit.kind {
                HitKind::Content { more, .. } => *more,
                HitKind::Name { .. } | HitKind::Metadata { .. } => 0,
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
                let root = (self.scope == Scope::Folder).then_some(self.folder.as_path());
                let row = Row::from_hit(hit, &self.home, root, strong_name);
                fresh_at.insert(row.path.clone(), fresh.len());
                fresh.push(row);
                changed = true;
            }
        }
        if !fresh.is_empty() {
            if self.metadata_only {
                // The engine sends predicate-only hits in natural-name order; a
                // refresh can mix old rows, so the final order is settled once at
                // completion instead of sorting the growing list every batch. The
                // index is refreshed here so the auto-selected first row is real.
                self.results.extend(fresh);
                self.metadata_order_dirty = true;
                self.reindex_rows();
            } else {
                self.insert_rows(fresh);
            }
        }
        let before = self.selection.cursor().cloned();
        self.follow_selection(cx);
        if before != self.selection.cursor().cloned() {
            self.load_preview(cx);
        }
        if changed {
            self.publish_count(cx);
            cx.notify();
        }
    }

    /// Update retained paths in place while the replacement generation streams.
    /// Only paths accepted in this generation consume its row budgets.
    fn add_refresh_hits(&mut self, hits: Vec<Hit>, cx: &mut Context<Self>) {
        let query = self.query(cx);
        let root = (self.scope == Scope::Folder).then_some(self.folder.as_path());
        let staging = crow_fs::ops::owned_staging_paths();
        for hit in hits {
            if staging.iter().any(|container| hit.path.starts_with(container)) {
                continue;
            }
            if self.seen.contains(&hit.path) {
                let row = &mut self.results[self.by_path[&hit.path]];
                if let HitKind::Content { line, text, ranges, .. } = hit.kind
                    && row.content.as_ref().is_none_or(|(best, _, _)| line < *best)
                {
                    row.set_content((line, text, ranges));
                }
                continue;
            }
            let strong = match &hit.kind {
                HitKind::Name { ranges } => is_strong_name(&hit.path, ranges, &query),
                HitKind::Metadata { .. } => true,
                HitKind::Content { .. } => false,
            };
            if strong {
                if self.refresh_names == MAX_STRONG_ROWS {
                    self.names_capped = true;
                    self.control.cancel();
                    break;
                }
                self.refresh_names += 1;
            } else {
                if self.refresh_other == MAX_OTHER_ROWS {
                    self.dropped += 1;
                    continue;
                }
                self.refresh_other += 1;
                if self.refresh_other == MAX_OTHER_ROWS && !self.contents_stopped {
                    self.contents_stopped = true;
                    self.control.stop_contents();
                    if self.matching == Match::Contents {
                        self.control.cancel();
                    }
                }
            }
            self.seen.insert(hit.path.clone());
            let row = Row::from_hit(hit, &self.home, root, strong);
            if let Some(&index) = self.by_path.get(&row.path) {
                self.results[index] = row;
            } else {
                self.by_path.insert(row.path.clone(), self.results.len());
                self.results.push(row);
            }
        }
        self.publish_count(cx);
        cx.notify();
    }

    /// The other budget filled: stop the engine's content reads. In Contents
    /// mode nothing later can be shown, so cancel the walk too.
    fn stop_contents_at_cap(&mut self) {
        if self.contents_stopped {
            return;
        }
        self.contents_stopped = true;
        self.control.stop_contents();
        if self.effective_match() == Match::Contents {
            self.control.cancel();
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
            let weak_added = weak.len();
            self.results.extend(weak);
            self.weak_count += weak_added;
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
        self.weak_count = old_weak.len() + weak.len();
        let weak_count = self.weak_count;
        self.results = merged.into_iter().chain(tail).chain(content).chain(old_weak).chain(weak).collect();
        debug_assert_eq!(self.results.len(), self.name_count + self.content_count + weak_count);
        // Rows only move to the right, so re-index the moved suffix and index the appended rows.
        let first = first_insert.unwrap_or(if content_added { old_content_end } else { old_total });
        for (offset, row) in self.results[first..].iter().enumerate() {
            self.by_path.insert(row.path.clone(), first + offset);
        }
    }

    /// Until the user picks a row, keep the best row selected as batches arrive.
    /// After that, keep the keyboard cursor on the same path when rows above it move.
    fn follow_selection(&mut self, cx: &mut Context<Self>) {
        if !self.user_moved {
            if let Some(first) = self.results.first() {
                self.selection.select_only(first.path.clone());
                self.bump_selection(cx);
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
        // A tag expression already made Spotlight enforce every selected tag;
        // the client-side pass only has to prove kind and age. Without tags, the
        // hit's tags were never read, and the predicate does not ask for them.
        let known_tags = (!self.predicates.tags.is_empty()).then_some(self.predicates.tags.as_slice());
        let staging = crow_fs::ops::owned_staging_paths();
        let mut changed = false;
        for hit in hits {
            if staging.iter().any(|container| hit.path.starts_with(container)) {
                continue;
            }
            if !self.predicates.is_empty()
                && !self.predicates.matches_with_tags(&hit.path, hit.is_dir, hit.modified, self.run_started, known_tags)
            {
                continue;
            }
            if !self.seen.insert(hit.path.clone()) {
                continue;
            }
            if self.seen.len() > MAX_STRONG_ROWS {
                self.seen.remove(&hit.path);
                self.control.cancel();
                self.names_capped = true;
                self.metadata_capped |= self.metadata_only;
                break;
            }
            if self.first_hit_ms.is_none() {
                self.first_hit_ms = Some(self.started.elapsed().as_millis() as u64);
            }
            self.hit_count += 1;
            let row = Row::from_spotlight(hit, &query, &self.home);
            if let Some(&index) = self.by_path.get(&row.path) {
                self.results[index] = row;
            } else {
                self.by_path.insert(row.path.clone(), self.results.len());
                self.results.push(row);
            }
            changed = true;
        }
        self.name_count = self.results.len();
        self.content_count = 0;
        self.weak_count = 0;
        if self.metadata_only {
            self.metadata_order_dirty = true;
        }
        let before = self.selection.cursor().cloned();
        self.follow_selection(cx);
        if before != self.selection.cursor().cloned() {
            self.load_preview(cx);
        }
        if changed {
            self.publish_count(cx);
            cx.notify();
        }
    }

    fn clear_preview(&mut self, cx: &mut App) {
        if let Some(mut loaded) = self.preview.take() {
            loaded.preview.release_image(cx);
        }
    }

    /// Load the cursor row's preview off the main thread, unless it is loaded.
    fn load_preview(&mut self, cx: &mut Context<Self>) {
        let surface = self.plugin_surface;
        let Some(row) = self.results.get(self.cursor) else {
            self.clear_preview(cx);
            self.preview_generation += 1;
            self.preview_cancel.store(true, Ordering::Relaxed);
            crate::plugins::cancel_previews(surface, cx);
            return;
        };
        let path = row.path.clone();
        let line = row.content.as_ref().map(|(line, _, _)| *line);
        let is_dir = row.is_dir;
        if self.preview.as_ref().is_some_and(|loaded| loaded.entry.path == path) {
            return;
        }
        self.clear_preview(cx);
        self.preview_generation += 1;
        let generation = self.preview_generation;
        self.preview_cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.preview_cancel = cancel.clone();
        crate::plugins::cancel_previews(surface, cx);
        let extensions = cx.global::<Ext>().0.clone();
        let (gate, cloud) = match cx.try_global::<CloudStatusService>() {
            Some(service) => {
                let service = service.0.read(cx);
                (service.gate(), service.status(&path))
            }
            None => (Arc::new(AllowAll) as Arc<dyn ReadGate>, None),
        };
        let requested = path.clone();
        let configured = cx.global::<Ui>().views.resolve(&self.folder).preview_providers.0.clone();
        let initial_candidates = crate::plugins::preview_candidates(cx, &configured, &path, is_dir, false);
        let revision = generation;
        cx.spawn(async move |this, cx| {
            let entry = cx.background_spawn(async move { crow_fs::stat_entry(&requested).ok().flatten() }).await;
            let Some(entry) = entry else { return };
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let candidates = if entry.is_symlink { Vec::new() } else { initial_candidates };
            let Some(preview) = preview::load_with_plugins(
                cx,
                surface,
                candidates,
                entry.clone(),
                extensions,
                gate,
                cloud,
                line,
                revision,
                preview::ImageSize::Pane,
                cancel.clone(),
            )
            .await
            else {
                return;
            };
            let loaded = Some(LoadedPreview { entry, preview });
            this.update(cx, |this, cx| {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                if this.preview_generation == generation
                    && this.results.get(this.cursor).is_some_and(|row| row.path == path)
                {
                    let mut loaded = loaded;
                    if let Some(preview) = loaded.as_mut()
                        && this.cloud_status(&path, cx).is_some_and(|status| preview.preview.update_cloud(status))
                    {
                        this.load_preview(cx);
                    } else {
                        this.preview = loaded;
                    }
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The cloud-status service changed: re-arm a blocked preview when its
    /// selected row becomes positively local/downloaded.
    fn cloud_changed(&mut self, cx: &mut Context<Self>) {
        if let Some(row) = self.results.get(self.cursor)
            && let Some(status) = self.cloud_status(&row.path, cx)
            && let Some(loaded) = self.preview.as_mut().filter(|loaded| loaded.entry.path == row.path)
            && loaded.preview.update_cloud(status)
        {
            self.clear_preview(cx);
            self.load_preview(cx);
        }
        cx.notify();
    }

    fn cloud_status(&self, path: &Path, cx: &gpui::App) -> Option<CloudStatus> {
        cx.try_global::<CloudStatusService>().and_then(|service| service.0.read(cx).status(path))
    }

    /// The preview pane's parts, when a result is selected.
    pub fn preview_parts(&self) -> Option<(Entry, Option<Preview>)> {
        self.preview.as_ref().map(|loaded| (loaded.entry.clone(), Some(loaded.preview.clone())))
    }

    /// What the Info tab shows for the results: the cursor result, or a summary of
    /// a multi-selection. Sizes for a multi-selection are read off the main thread.
    pub fn info_target(&self) -> (Option<InfoTarget>, Option<InfoKey>) {
        if self.selection.len() > 1 {
            let summary = self.selection_summary();
            let key = summary.key(self.selection_generation);
            return (Some(InfoTarget::Many(summary)), Some(key));
        }
        let Some((entry, _)) = self.preview_parts() else { return (None, None) };
        let key = InfoKey::Single(entry.path.clone());
        (Some(InfoTarget::Single { entry, items: None }), Some(key))
    }

    fn selection_summary(&self) -> SelectionSummary {
        if let Some((generation, summary)) = &*self.summary_cache.borrow()
            && *generation == self.selection_generation
        {
            return summary.clone();
        }
        let mut builder = SelectionSummaryBuilder::default();
        let mut folders = 0;
        for row in &self.results {
            if !self.selection.contains(&row.path) {
                continue;
            }
            if row.is_dir {
                folders += 1;
            }
            let ext = Path::new(&row.name).extension().and_then(|e| e.to_str()).unwrap_or("");
            builder.add(&row.path, ext, row.is_dir && !row.is_package);
        }
        let summary = builder.finish(None, folders);
        *self.summary_cache.borrow_mut() = Some((self.selection_generation, summary.clone()));
        summary
    }

    fn query(&self, cx: &App) -> String {
        self.input.read(cx).text().to_owned()
    }

    /// The empty state names what ran: the text, else the tag or saved query
    /// (a tag or metadata-only query has no text).
    fn empty_message(&self, cx: &App) -> String {
        let text = self.query(cx);
        let scope = self.scope_label();
        if !text.trim().is_empty() {
            return format!("No results for \"{text}\" in {scope}");
        }
        if let Some(tag) = &self.tag {
            return format!("No results for tag \"{}\" in {scope}", tag.as_str());
        }
        if self.saved.is_some() {
            return format!("No results for \"{}\" in {scope}", self.title);
        }
        format!("No results in {scope}")
    }

    #[cfg(test)]
    pub fn scroll_for_test(&self) -> &UniformListScrollHandle {
        &self.scroll
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
        !self.query(cx).trim().is_empty() || !self.predicates.is_empty()
    }

    /// How many results the list holds so far.
    pub fn result_count(&self) -> usize {
        self.results.len()
    }

    #[cfg(test)]
    pub fn names_capped(&self) -> bool {
        self.names_capped
    }

    #[cfg(test)]
    pub fn contents_stopped(&self) -> bool {
        self.contents_stopped
    }

    #[cfg(test)]
    pub fn dropped(&self) -> usize {
        self.dropped
    }

    #[cfg(test)]
    pub fn control_cancelled(&self) -> bool {
        self.control.is_cancelled()
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
    pub fn skipped_paths(&self) -> Vec<PathBuf> {
        self.skipped.iter().map(|skip| skip.path.clone()).collect()
    }

    /// The skipped list's groups, as the renderer builds them.
    #[cfg(test)]
    pub fn skipped_groups(&self) -> Vec<(&'static str, Vec<PathBuf>)> {
        skip_groups(&self.skipped)
            .into_iter()
            .map(|(heading, _, skips)| (heading, skips.into_iter().map(|skip| skip.path.clone()).collect()))
            .collect()
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

    /// Search uses the origin folder's `views.toml` icon source, else the
    /// global pack. Re-resolved per render, so a config edit applies live.
    pub fn icon_source(&self, cx: &gpui::App) -> String {
        crate::icon_packs::effective_icon_source(cx.global::<Ui>(), &self.folder)
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
        if self.tag.is_some() || self.scope == scope {
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
        if self.tag.is_some() || self.scope == Scope::ThisMac || self.matching == matching {
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
        self.bump_selection(cx);
        self.cursor = ix;
        self.user_moved = true;
        window.focus(&self.focus, cx);
        self.scroll.scroll_to_item(ix, ScrollStrategy::Top);
        self.load_preview(cx);
        self.follow_quick_look(cx);
        cx.notify();
    }

    fn press(&mut self, ix: usize, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let Some(row) = self.results.get(ix) else { return };
        if event.modifiers.control {
            self.context_press(ix, window, cx);
            return;
        }
        self.user_moved = true;
        // AppKit wants the mouse-down event when a GPUI drag later leaves the window.
        crow_macos::remember_mouse_down();
        if event.modifiers.platform {
            self.selection.toggle(row.path.clone());
            // The cursor follows the model, or Return, Quick Look and the arrows
            // would act on the row the toggle left.
            let cursor = self.selection.cursor().cloned();
            if let Some(index) = cursor.and_then(|path| self.results.iter().position(|row| row.path == path)) {
                self.cursor = index;
            }
        } else if event.modifiers.shift {
            self.extend_to(ix, cx);
        } else if self.selection.contains(&row.path) {
            // A press inside the selection keeps it, so the whole set can be dragged.
            self.selection.set_cursor(row.path.clone());
            self.cursor = ix;
        } else {
            self.selection.select_only(row.path.clone());
            self.cursor = ix;
        }
        self.bump_selection(cx);
        self.scroll.scroll_to_item(ix, ScrollStrategy::Top);
        self.load_preview(cx);
        cx.notify();
    }

    /// A right-click or Ctrl-click on result `ix`. An unselected row is selected
    /// first; a row in the selection keeps it, as in the list, and becomes the
    /// active row, so the bar, the preview and the single-row actions all name it.
    fn context_press(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let Some(row) = self.results.get(ix) else { return };
        if !self.selection.contains(&row.path) {
            self.selection.select_only(row.path.clone());
            self.scroll.scroll_to_item(ix, ScrollStrategy::Top);
        } else {
            // The selection stays, so Copy, Cut and Trash still take all of it.
            self.selection.set_cursor(row.path.clone());
        }
        self.bump_selection(cx);
        // Single-row actions (Open, Reveal, Quick Look) act on the cursor.
        self.cursor = ix;
        self.load_preview(cx);
        self.user_moved = true;
        cx.notify();
        self.open_menu(ix, window, cx);
    }

    fn open_menu(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let mut entries = crate::context_menu::search_entries(cx);
        let selected: Vec<(PathBuf, bool)> = self
            .results
            .iter()
            .filter(|row| self.selection.contains(&row.path))
            .map(|row| (row.path.clone(), row.is_dir))
            .collect();
        let plugins = crate::plugins::context_menu_commands(cx, &selected);
        self.menu_plugins = plugins.iter().map(|(_, action)| action.clone()).collect();
        if !plugins.is_empty() {
            entries.push(crow_macos::ContextMenuEntry::Separator);
            entries.extend(plugins.into_iter().map(|(title, _)| crow_macos::ContextMenuEntry::Item {
                title,
                shortcut: None,
                enabled: true,
            }));
        }
        self.menu = Some(ix);
        cx.notify();
        let (sender, receiver) = futures::channel::oneshot::channel();
        // Off the main thread (tests) the recorder answers at once; in the app the
        // native menu calls back when it closes.
        if !crate::effects::effects(cx).show_context_menu(
            window,
            entries,
            Box::new(move |chosen| {
                sender.send(chosen).ok();
            }),
        ) {
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            let chosen = receiver.await.ok().flatten();
            this.update_in(cx, |this, window, cx| this.menu_closed(chosen, window, cx)).ok();
        })
        .detach();
    }

    /// The menu closed with the line at `chosen` picked (an index into its slots), or none.
    fn menu_closed(&mut self, chosen: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.take().is_none() {
            self.menu_plugins.clear();
            return;
        }
        cx.notify();
        let builtins = crate::context_menu::search_slots().len();
        let plugin =
            chosen.and_then(|ix| ix.checked_sub(builtins + 1)).and_then(|ix| self.menu_plugins.get(ix)).cloned();
        self.menu_plugins.clear();
        // The results have focus, so the action runs exactly as its key would.
        window.focus(&self.focus, cx);
        if let Some(action) = plugin {
            window.dispatch_action(Box::new(action), cx);
            return;
        }
        let Some((id, _)) = chosen.and_then(|ix| crate::context_menu::search_slots().get(ix).copied().flatten()) else {
            return;
        };
        window.dispatch_action(crate::context_menu::command(id).action.boxed_clone(), cx);
    }

    #[cfg(test)]
    pub fn menu(&self) -> Option<usize> {
        self.menu
    }

    #[cfg(test)]
    pub fn choose_menu_item(&mut self, chosen: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.menu_closed(chosen, window, cx);
    }

    fn extend_to(&mut self, target: usize, cx: &mut Context<Self>) {
        if target >= self.results.len() {
            return;
        }
        let paths: Vec<PathBuf> = self.results.iter().map(|row| row.path.clone()).collect();
        let position = |path: &PathBuf| paths.iter().position(|candidate| candidate == path);
        let at = |index: usize| paths[index].clone();
        self.selection.extend_to(target, position, at);
        self.bump_selection(cx);
        self.cursor = target;
        self.user_moved = true;
    }

    fn selected_paths(&self) -> Vec<PathBuf> {
        self.results.iter().filter(|row| self.selection.contains(&row.path)).map(|row| row.path.clone()).collect()
    }

    /// Keep result rows, selection and cursor on the same items after a rename.
    pub(crate) fn apply_rename_map(
        &mut self,
        map: &std::collections::HashMap<PathBuf, PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if map.is_empty() {
            return;
        }
        let mut changed = false;
        let now = Instant::now();
        for row in &mut self.results {
            if let Some(new) = map.get(&row.path) {
                self.pinned.remove(&row.path);
                self.pinned.insert(new.clone(), now);
                row.path = new.clone();
                row.name = new.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                changed = true;
            }
        }
        if changed {
            self.by_path = self.results.iter().enumerate().map(|(index, row)| (row.path.clone(), index)).collect();
            self.selection.remap(map);
            self.bump_selection(cx);
            cx.notify();
        }
    }

    fn selected_rename_inputs(&self) -> Vec<crow_core::batch_rename::RenameInput> {
        self.results
            .iter()
            .filter(|row| self.selection.contains(&row.path))
            .map(|row| crow_core::batch_rename::RenameInput {
                source: row.path.clone(),
                name: row.name.clone(),
                is_dir: row.is_dir,
            })
            .collect()
    }

    fn rename(&mut self, _: &Rename, window: &mut Window, cx: &mut Context<Self>) {
        let entries = self.selected_rename_inputs();
        if let [entry] = entries.as_slice() {
            cx.emit(SearchEvent::Rename(entry.source.clone()));
            return;
        }
        if entries.is_empty() {
            crate::feedback::refuse(crate::feedback::RefuseTarget::None, window, cx);
            crate::operations::message("Select a result to rename", false, cx);
            cx.notify();
            return;
        }
        cx.emit(SearchEvent::BatchRename(entries.into()));
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

    fn quick_look_selected(&mut self, window: &Window, cx: &mut App) {
        if let Some(row) = self.results.get(self.cursor) {
            crate::pane::install_quick_look_keys(window, cx);
            crate::effects::effects(cx).toggle_quick_look(&row.path);
        }
    }

    /// An open Quick Look panel follows the cursor row, as it does in a folder.
    fn follow_quick_look(&self, cx: &App) {
        if let Some(row) = self.results.get(self.cursor) {
            crate::effects::effects(cx).update_quick_look(&row.path);
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
        } else if self.saved.is_none() {
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
        if !query.trim().is_empty() || !self.predicates.is_empty() {
            self.start_search(&query, cx);
        }
    }

    fn search_everything(&mut self, _: &SearchEverything, _: &mut Window, cx: &mut Context<Self>) {
        if self.tag.is_some() {
            return;
        }
        self.set_everything(!self.everything, cx);
    }

    /// Search Home, keeping the field or results focused.
    pub fn search_home(&mut self, cx: &mut Context<Self>) {
        self.set_scope(Scope::Home, cx);
    }

    fn search_home_action(&mut self, _: &SearchHome, _: &mut Window, cx: &mut Context<Self>) {
        self.search_home(cx);
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

    /// Shift-↑: extend the selection one row up from the anchor.
    fn extend_up(&mut self, _: &ExtendUp, window: &mut Window, cx: &mut Context<Self>) {
        self.extend_by(-1, window, cx);
    }

    fn extend_down(&mut self, _: &ExtendDown, window: &mut Window, cx: &mut Context<Self>) {
        self.extend_by(1, window, cx);
    }

    /// ⇧⌥↑/↓: add every result from the cursor through the first (or last), keeping
    /// what is selected and the anchor.
    fn extend_through(&mut self, to_last: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.results.is_empty() {
            return;
        }
        let last = self.results.len() - 1;
        let from = self.selection.cursor().is_some().then_some(self.cursor);
        let from = from.unwrap_or(if to_last { 0 } else { last }).min(last);
        let (lo, hi, target) = if to_last { (from, last, last) } else { (0, from, 0) };
        let keys: Vec<_> = self.results[lo..=hi].iter().map(|row| row.path.clone()).collect();
        let start = self.results[from].path.clone();
        let end = self.results[target].path.clone();
        self.selection.extend_through(keys, start, end);
        self.cursor = target;
        self.bump_selection(cx);
        self.user_moved = true;
        window.focus(&self.focus, cx);
        self.scroll.scroll_to_item(target, ScrollStrategy::Top);
        self.load_preview(cx);
        self.follow_quick_look(cx);
        cx.notify();
    }

    fn extend_to_first(&mut self, _: &ExtendToFirst, window: &mut Window, cx: &mut Context<Self>) {
        self.extend_through(false, window, cx);
    }

    fn extend_to_last(&mut self, _: &ExtendToLast, window: &mut Window, cx: &mut Context<Self>) {
        self.extend_through(true, window, cx);
    }

    fn extend_by(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.cursor.checked_add_signed(delta).filter(|target| *target < self.results.len()) else {
            return;
        };
        self.extend_to(target, cx);
        window.focus(&self.focus, cx);
        self.scroll.scroll_to_item(target, ScrollStrategy::Top);
        self.load_preview(cx);
        self.follow_quick_look(cx);
        cx.notify();
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

    fn quick_look(&mut self, _: &QuickLook, window: &mut Window, cx: &mut Context<Self>) {
        self.quick_look_selected(window, cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        let paths = self.results.iter().map(|row| row.path.clone());
        self.selection.select_all(paths);
        self.bump_selection(cx);
        self.user_moved = true;
        cx.notify();
    }

    fn copy(&mut self, _: &CopyFiles, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_selected(false, cx);
    }

    fn cut(&mut self, _: &CutFiles, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_selected(true, cx);
    }

    /// The search menu's Copy Path: the selected results, or the cursor's row.
    fn copy_path(&mut self, _: &CopyPath, _: &mut Window, cx: &mut Context<Self>) {
        let mut paths = self.selected_paths();
        if paths.is_empty()
            && let Some(row) = self.results.get(self.cursor)
        {
            paths.push(row.path.clone());
        }
        if paths.is_empty() {
            return;
        }
        let text = paths.iter().map(|path| path.to_string_lossy()).collect::<Vec<_>>().join("\n");
        crate::effects::write_text(text, cx);
        let what = if paths.len() == 1 { "path".to_owned() } else { format!("{} paths", paths.len()) };
        crate::operations::message(format!("Copied {what}"), false, cx);
        cx.notify();
    }

    fn move_to_trash(&mut self, _: &MoveToTrash, _: &mut Window, cx: &mut Context<Self>) {
        self.trash_selected(false, cx);
    }

    fn delete_permanently(&mut self, _: &DeletePermanently, _: &mut Window, cx: &mut Context<Self>) {
        self.trash_selected(true, cx);
    }

    pub fn run_command(&mut self, command: crate::palette::CommandId, window: &mut Window, cx: &mut Context<Self>) {
        use crate::palette::CommandId;
        match command {
            CommandId::Open => self.open_selected(cx),
            CommandId::Rename => self.rename(&Rename, window, cx),
            CommandId::SearchReveal => self.reveal_selected(cx),
            CommandId::QuickLook => self.quick_look_selected(window, cx),
            CommandId::Copy => self.copy_selected(false, cx),
            CommandId::Cut => self.copy_selected(true, cx),
            CommandId::CopyPath => self.copy_path(&CopyPath, window, cx),
            CommandId::MoveToTrash => self.trash_selected(false, cx),
            CommandId::DeletePermanently => self.trash_selected(true, cx),
            CommandId::SelectAll => self.select_all(&SelectAll, window, cx),
            CommandId::ExtendSelectionToStart => self.extend_to_first(&ExtendToFirst, window, cx),
            CommandId::ExtendSelectionToEnd => self.extend_to_last(&ExtendToLast, window, cx),
            _ => {}
        }
    }

    // ----- rendering -----

    /// The window's band while search is open: the query field and Done.
    /// The window places this between Back/Forward and the window edge.
    /// The toolbar's band. `look` is set while it opens (`crow_core::motion::band_look`).
    pub fn render_band(
        &mut self,
        look: Option<crow_core::motion::BandLook>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = cx.global::<Ui>().theme.clone();
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let ring = if focused { field_ring(&theme) } else { color(theme.role("surface")) };
        let field = div()
            .id("search-field")
            .debug_selector(|| "search-field".into())
            // keys::context_stack (KeyScope::Search) mirrors this.
            .key_context("Search")
            .on_action(cx.listener(Self::search_everything))
            .on_action(cx.listener(Self::search_home_action))
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
            .child(div().opacity(look.map_or(1.0, |look| look.controls)).child(self.render_match_control(focused, cx)));
        // Widening: the field starts at the toolbar field's 240 px and takes a share of
        // the band's free space. A flex factor under 1 hands out only that share, so
        // the width runs linearly to the settled one without a clamp that ends the
        // motion early or a width guess that overshoots.
        let field = match look {
            Some(look) if look.widen < 1.0 => {
                let mut field = field.flex_basis(px(240.)).flex_shrink(1.);
                field.style().flex_grow = Some(look.widen);
                field
            }
            _ => field,
        };
        let title_color = color(theme.role("fg-muted"));
        let save_surface = color(theme.role("surface"));
        let band_fg = color(theme.role("fg"));
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
        let mut band = div().flex_1().min_w_0().flex().items_center().gap(px(10.)).child(field);
        let controls = look.map_or(1.0, |look| look.controls);
        let save_label = if self.saved.is_some() { "Update…" } else { "Save…" };
        band = band.child(
            div()
                .id("search-save")
                .debug_selector(|| "search-save".into())
                .role(Role::Button)
                .aria_label(save_label)
                .flex_none()
                .h(px(30.))
                .px(px(10.))
                .rounded(px(8.))
                .flex()
                .items_center()
                .text_size(px(13.))
                .text_color(band_fg)
                .hover(move |style| style.bg(save_surface))
                .on_click(cx.listener(|_, _, _, cx| cx.emit(SearchEvent::Save)))
                .opacity(controls)
                .child(save_label),
        );
        if self.saved.is_some() || self.tag.is_some() {
            // The destination title stays visible; Done would imply closing the tab.
            band = band.child(
                div()
                    .id("saved-query-title")
                    .debug_selector(|| "saved-query-title".into())
                    .flex_none()
                    .max_w(px(260.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(13.))
                    .text_color(title_color)
                    .child(self.title.clone()),
            );
        } else {
            band = band.child(done.opacity(controls));
        }
        band.opacity(look.map_or(1.0, |look| look.fade))
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
            .min_w_0()
            .gap(px(2.))
            .p(px(2.))
            .rounded(px(7.))
            .bg(color(theme.role("surface")))
            .child(self.scope_segment(Scope::Folder, self.folder_label.clone().into(), cx))
            .child(self.scope_segment(Scope::Home, "Home".into(), cx))
            .child(self.scope_segment(Scope::ThisMac, "This Mac".into(), cx));
        let right: Option<gpui::AnyElement> = if self.scope == Scope::ThisMac {
            Some(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(muted)
                    .child("Spotlight: indexed volumes only")
                    .into_any_element(),
            )
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
            // With several exclusion sets enabled, the switch label must also
            // give up space so the three scope buttons remain reachable.
            .max_w(relative(0.4))
            .px(px(8.))
            .flex()
            .flex_none()
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
        let folder_path = (scope == Scope::Folder).then(|| crate::style::display_path(&self.folder, &self.home));
        let aria = match scope {
            Scope::Folder => format!("Search in {label}, {}", folder_path.as_deref().unwrap_or_default()),
            Scope::Home => "Search in Home".to_owned(),
            Scope::ThisMac => "Search This Mac with Spotlight, names only".to_owned(),
        };
        let hover = color_alpha(theme.role("surface-raised"), 0x80);
        let pressed = color(theme.role("surface-raised"));
        let segment = div()
            .id(id)
            .debug_selector(move || id.into())
            .role(Role::RadioButton)
            .aria_label(aria)
            .aria_selected(selected)
            .h(px(22.))
            .px(px(9.))
            .flex()
            .when(scope == Scope::Folder, |segment| segment.min_w_0())
            .when(scope != Scope::Folder, |segment| segment.flex_none())
            .items_center()
            .rounded(px(5.))
            .whitespace_nowrap()
            .text_color(color(if selected { theme.role("fg") } else { theme.role("fg-muted") }))
            .when(selected, |segment| segment.bg(pressed))
            .when(!selected, move |segment| {
                segment.hover(move |style| style.bg(hover)).active(move |style| style.bg(pressed))
            })
            .on_click(cx.listener(move |this, _, _, cx| this.set_scope(scope, cx)))
            .when(scope == Scope::Folder, |segment| {
                segment.child(icons::glyph(Glyph::Folder, 12., "fg-muted", &theme)).gap(px(4.))
            })
            .child(div().min_w_0().max_w(px(160.)).overflow_hidden().whitespace_nowrap().text_ellipsis().child(label));
        // The bar stays quiet; the full path and the Home key are one hover away.
        if scope == Scope::Folder {
            with_tooltip(segment, folder_path.unwrap_or_default(), None)
        } else if scope == Scope::Home {
            with_action_tooltip(segment, "Search Home", &SearchHome, KeyScope::Search, cx)
        } else {
            segment
        }
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
        } else if self.names_capped || self.contents_stopped || self.metadata_capped || self.stale {
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
        let liftable = self.skipped.iter().any(|skip| skip.reason.lifted_by_everything());
        let hint = (self.scope != Scope::ThisMac && (liftable || self.everything)).then(|| {
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
            matching: self.effective_match(),
            hits: self.hit_count,
            files: self.results.len(),
            names_capped: self.names_capped,
            contents_stopped: self.contents_stopped,
            metadata_capped: self.metadata_capped,
            stale: self.stale,
            dropped: self.dropped,
            cloud_skipped: self.cloud_skipped,
            tag_errors: self.tag_errors,
            error: self.error.as_deref(),
        })
    }

    /// Idle, error, empty or results, depending on the query and what has arrived.
    fn render_body(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        if self.query(cx).trim().is_empty() && self.predicates.is_empty() {
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

    /// The empty state's widen buttons: what this miss could reach.
    fn widen_options(&self) -> Vec<Widen> {
        let liftable = self.skipped.iter().filter(|skip| skip.reason.lifted_by_everything()).count();
        widen_options(
            self.scope,
            self.scope == Scope::Folder && self.folder == self.home,
            self.effective_match(),
            liftable,
            self.everything,
        )
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
        let home_spoken =
            keys::spoken_for(&SearchHome, KeyScope::Search, cx).unwrap_or_else(|| "Shift Command H".to_owned());
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
            .child(self.render_example(
                3,
                div().child("Home").into_any_element(),
                "your whole home folder",
                ExampleAction::Home,
                format!("Search Home, {home_spoken}"),
                cx,
            ))
            .when(self.skip_label.is_some(), |examples| {
                examples.child(self.render_example(
                    4,
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
                            ExampleAction::Home => this.search_home(cx),
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
        let message = self.empty_message(cx);
        let spoken_everything =
            keys::spoken_for(&SearchEverything, KeyScope::Search, cx).unwrap_or_else(|| "Shift Command A".to_owned());
        let spoken_home =
            keys::spoken_for(&SearchHome, KeyScope::Search, cx).unwrap_or_else(|| "Shift Command H".to_owned());
        let buttons: Vec<gpui::AnyElement> = self
            .widen_options()
            .into_iter()
            .map(|option| {
                let (kind, label, aria) = match option {
                    Widen::Everything(n) => (
                        "everything",
                        format!("{}. Search everything", skipped_was_text(n)),
                        format!("{}. Search everything, {spoken_everything}", skipped_was_text(n)),
                    ),
                    Widen::Home => ("home", "Search Home".to_owned(), format!("Search Home, {spoken_home}")),
                    Widen::AllMatch => {
                        ("all-match", "Search names and contents".to_owned(), "Search names and contents".to_owned())
                    }
                };
                let keycap = match option {
                    Widen::Everything(_) => {
                        Keycap::for_action(&SearchEverything, KeyScope::Search, KeycapWeight::Outline, cx)
                    }
                    Widen::Home => Keycap::for_action(&SearchHome, KeyScope::Search, KeycapWeight::Outline, cx),
                    Widen::AllMatch => None,
                };
                div()
                    .id(kind)
                    .debug_selector(move || format!("search-empty-{kind}"))
                    .role(Role::Button)
                    .aria_label(aria)
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_color(accent)
                    .on_click(cx.listener(move |this, _, window, cx| match option {
                        Widen::Everything(_) => this.search_everything(&SearchEverything, window, cx),
                        Widen::Home => this.search_home(cx),
                        Widen::AllMatch => this.set_match(Match::All, cx),
                    }))
                    .child(label)
                    .children(keycap)
                    .into_any_element()
            })
            .collect();
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
            .children(buttons)
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
        let has_decorations = !crate::plugins::decoration_ids(cx).is_empty();
        let widths = self.column_widths();
        // Columns hidden for a narrow panel have width 0; the last shown one has no divider.
        let last = widths.iter().rposition(|w| *w > 0.0).unwrap_or(0);
        let mut header = div()
            .id("search-header")
            .debug_selector(|| "search-header".into())
            .role(Role::Group)
            .aria_label("Search result columns")
            .h(px(26.))
            .flex_none()
            .px(px(RESULT_INSET))
            .flex()
            .items_center()
            .gap(px(RESULT_GAP))
            .text_size(px(11.))
            .text_color(muted)
            .border_b_1()
            .border_color(border);
        for (i, title) in ["Name", "Folder", "Modified"].into_iter().enumerate().filter(|(i, _)| widths[*i] > 0.0) {
            let cell = div()
                .id(("search-column", i))
                .debug_selector(move || format!("search-column-{i}"))
                .role(Role::ColumnHeader)
                .aria_label(title)
                .relative()
                .h_full()
                .flex()
                .items_center()
                .when(i == 0, |cell| cell.flex_1().min_w_0())
                .when(i > 0, |cell| cell.w(px(widths[i])).flex_none())
                .child(div().overflow_hidden().whitespace_nowrap().text_ellipsis().child(title))
                // The divider sits in the gap after the cell; its grab area is the whole gap.
                .when(i != last, |cell| {
                    cell.child(
                        div()
                            .id(("search-resize", i))
                            .debug_selector(move || format!("search-resize-{i}"))
                            .role(Role::Splitter)
                            .aria_label(format!("Resize {title} column"))
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .right(px(-RESULT_GAP))
                            .w(px(RESULT_GAP))
                            .flex()
                            .justify_center()
                            .cursor(gpui::CursorStyle::ResizeLeftRight)
                            .child(div().w(px(1.)).h_full().bg(border))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                    let available = this.available_width();
                                    this.resize = this.table.begin_resize(i, f32::from(event.position.x), available);
                                    cx.stop_propagation();
                                }),
                            ),
                    )
                });
            header = header.child(cell);
        }
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
            .on_action(cx.listener(Self::extend_up))
            .on_action(cx.listener(Self::extend_down))
            .on_action(cx.listener(Self::focus_field_from_results))
            .on_action(cx.listener(Self::move_to_first))
            .on_action(cx.listener(Self::extend_to_first))
            .on_action(cx.listener(Self::extend_to_last))
            .on_action(cx.listener(Self::move_to_last))
            .on_action(cx.listener(Self::open))
            .on_action(cx.listener(Self::rename))
            .on_action(cx.listener(Self::reveal))
            .on_action(cx.listener(Self::quick_look))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy_path))
            .on_action(cx.listener(Self::move_to_trash))
            .on_action(cx.listener(Self::delete_permanently))
            .on_action(cx.listener(Self::close_action))
            .when(self.resize.is_some(), |list| {
                list.child(crate::pane::column_resize_tracker(
                    cx.weak_entity(),
                    Self::drag_column,
                    Self::end_column_drag,
                ))
            })
            .child(header)
            .child({
                let list = uniform_list("search-results", count, cx.processor(Self::render_rows))
                    .track_scroll(&scroll)
                    .flex_1();
                let list = if has_decorations {
                    list.with_decoration(PluginSearchRows { view: cx.entity() }).into_any_element()
                } else {
                    list.into_any_element()
                };
                let bounds = self.list_bounds.clone();
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .on_children_prepainted(move |children, _, _| {
                        bounds.set(children.first().copied());
                    })
                    .child(list)
            })
            .into_any_element()
    }

    /// The Name, Folder and Modified widths for the results' last drawn width: Name
    /// takes the rest, and Modified then Folder give way when the panel is narrow.
    fn drag_column(&mut self, x: f32, cx: &mut Context<Self>) {
        let Some(drag) = self.resize else { return };
        self.table.drag_to(&drag, x);
        cx.set_global(SearchColumns(self.table.clone()));
        cx.notify();
    }

    fn end_column_drag(&mut self, cx: &mut Context<Self>) {
        if self.resize.take().is_some() {
            cx.notify();
        }
    }

    fn column_widths(&self) -> Vec<f32> {
        self.table.fitted(self.available_width())
    }

    /// The width the three columns share: the results' last drawn width less the
    /// side padding and the two gaps.
    fn available_width(&self) -> f32 {
        let chrome = 2.0 * RESULT_INSET + 2.0 * RESULT_GAP;
        self.list_bounds.get().map_or(self.table.total_width(), |bounds| f32::from(bounds.size.width) - chrome)
    }

    #[cfg(test)]
    pub fn column_width_for_test(&self, id: &str) -> Option<f32> {
        self.table.width_of(id)
    }

    /// The real folder under `pointer`, from the same fixed-row geometry the list
    /// drew. A file, package or skipped row is unavailable.
    pub(crate) fn set_drop_hover(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.drop_hover != path {
            self.drop_hover = path;
            cx.notify();
        }
    }

    pub(crate) fn drop_hit(&self, pointer: Point<Pixels>, _cx: &App) -> Option<crate::drag_drop::DropHit> {
        let bounds = self.list_bounds.get()?;
        if !bounds.contains(&pointer) {
            return None;
        }
        let offset = f32::from(self.scroll.0.borrow().base_handle.offset().y);
        // The offset is negative once scrolled down.
        let row = (f32::from(pointer.y - bounds.top()) - offset) / ROW_HEIGHT;
        if row < 0.0 {
            return None;
        }
        let row = self.results.get(row.floor() as usize)?;
        (row.is_dir && !crow_fs::is_package_path(&row.path))
            .then(|| crate::drag_drop::DropHit::folder(row.path.clone(), self.generation, true))
    }

    fn render_rows(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<impl IntoElement + use<>> {
        let (theme, mode) = {
            let ui = cx.global::<Ui>();
            (ui.theme.clone(), ui.feedback.mode)
        };
        let theme = &theme;
        let (fg, muted, subtle) =
            (color(theme.role("fg")), color(theme.role("fg-muted")), color(theme.role("fg-subtle")));
        let focused = self.focus.is_focused(window);
        let selection_bg = if focused { active_selection(theme) } else { inactive_selection(theme) };
        let count = self.results.len();
        let cursor = self.cursor;
        let cloud_visible: Vec<PathBuf> =
            range.clone().filter_map(|i| self.results.get(i)).map(|row| row.path.clone()).collect();
        if let Some(service) = cx.try_global::<CloudStatusService>() {
            let service = service.0.clone();
            service.update(cx, |service, cx| service.observe((self.cloud_source, 0), &cloud_visible, cx));
        }
        let decoration_ids = crate::plugins::decoration_ids(cx);
        let has_decorations = !decoration_ids.is_empty();
        let plugin_scope = self.plugin_scope_key();
        let now = Instant::now();
        let icons = crate::icon_packs::Resolver::for_source(cx, &self.icon_source(cx));
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
        let widths = self.column_widths();
        // Name's text room: its cell less the icon and the gap after it.
        let name_room = px((widths[0] - 16.0 - RESULT_GAP).max(0.0));
        range
            .filter_map(|i| {
                let row = self.results.get(i)?;
                let version = self.plugin_row_version(row);
                let presentation =
                    crate::plugins::row_presentation(cx, &plugin_scope, &row.path, version, &decoration_ids);
                let plugin_spoken = if presentation.spoken.is_empty() {
                    String::new()
                } else {
                    format!(", {}", presentation.spoken.join(", "))
                };
                let cloud = self.cloud_status(&row.path, cx);
                let cloud_spoken =
                    cloud.and_then(CloudStatus::spoken_suffix).map(|suffix| format!(", {suffix}")).unwrap_or_default();
                let on_clipboard = clipboard.is_some_and(|clipboard| clipboard.contains(&row.path));
                let cut_edge = cut_edges.get(i - first).copied().flatten();
                let flash_tint = (on_clipboard && flash > 0.0)
                    .then(|| color_alpha(theme.role("accent"), (flash * 0x40 as f32) as u8));
                let selected = self.selection.contains(&row.path);
                let on_selection = selected && focused;
                let truncated = truncate_middle_parts(&row.name, name_room, px(13.), window);
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
                let drag_icon = cloud_icon(cloud, &row.path, row.is_dir, 22);
                let drop_opacity = (self.drop_hover.as_deref() == Some(row.path.as_path()))
                    .then(|| crate::drag_drop::cue_opacity(cx, &row.path));
                div()
                    .id(("search-row", i))
                    .role(Role::ListBoxOption)
                    .aria_label(format!("{}{plugin_spoken}{cloud_spoken}", row.label()))
                    .aria_position_in_set(i + 1)
                    .aria_size_of_set(count)
                    .aria_selected(selected)
                    .when(cursor == i, |row| row.aria_active_descendant())
                    .h(px(ROW_HEIGHT))
                    .w_full()
                    .px(px(RESULT_INSET))
                    .rounded_md()
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(RESULT_GAP))
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
                    .when_some(drop_opacity, |row, opacity| {
                        row.child(crate::style::drop_outline(theme).inset_0().opacity(opacity))
                    })
                    .debug_selector(|| format!("search-row-{i}"))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| this.press(i, event, window, cx)),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, _: &MouseDownEvent, window, cx| this.context_press(i, window, cx)),
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
                                && let Some(png) = crate::style::drag_icon_png(cx, path, *is_dir, 22)
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
                                    .opacity(crate::style::row_opacity(row.hidden, presentation.dim, false))
                                    .child(crate::preview::path_icon(
                                        &icons,
                                        &row.path,
                                        row.is_dir,
                                        row.is_package,
                                        cloud,
                                        theme,
                                        16.,
                                    )),
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
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .overflow_hidden()
                                                    .whitespace_nowrap()
                                                    .text_ellipsis()
                                                    .text_size(px(13.))
                                                    .text_color(if selected {
                                                        fg
                                                    } else {
                                                        presentation
                                                            .tint
                                                            .map_or(fg, |tint| color(theme.role(tint.key())))
                                                    })
                                                    .opacity(crate::style::row_opacity(
                                                        row.hidden,
                                                        presentation.dim,
                                                        false,
                                                    ))
                                                    .when(presentation.dim, |name| name.italic())
                                                    .child(name),
                                            )
                                            .children(cloud_status::badge(cloud, theme))
                                            .when(has_decorations, |cell| {
                                                cell.child(
                                                    div()
                                                        .w(px(32.))
                                                        .flex_none()
                                                        .overflow_hidden()
                                                        .whitespace_nowrap()
                                                        .text_ellipsis()
                                                        .text_size(px(11.))
                                                        .text_color(muted)
                                                        .child(presentation.badge.clone().unwrap_or_default()),
                                                )
                                            }),
                                    )
                                    .children(content),
                            ),
                    )
                    .when(widths[1] > 0.0, |cells| {
                        cells.child(
                            div()
                                .w(px(widths[1]))
                                .flex_none()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(px(12.))
                                .text_color(muted)
                                .debug_selector(move || format!("search-folder-{i}"))
                                .child(row.folder.clone()),
                        )
                    })
                    .when(widths[2] > 0.0, |cells| {
                        cells.child(
                            div()
                                .w(px(widths[2]))
                                .flex_none()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(px(12.))
                                .text_color(muted)
                                .debug_selector(move || format!("search-modified-{i}"))
                                .child(format_time_short(row.modified)),
                        )
                    })
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
        let accent = color(theme.role("accent"));
        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        let mut index = 0usize;
        for (heading, kind, skips) in skip_groups(&self.skipped) {
            rows.push(
                div()
                    .id(format!("search-skipped-header-{kind}"))
                    .debug_selector(move || format!("search-skipped-header-{kind}"))
                    .role(Role::Heading)
                    .aria_level(2)
                    .aria_label(heading)
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(muted)
                    .child(heading)
                    .into_any_element(),
            );
            let mut privacy = false;
            for skip in skips {
                if matches!(&skip.reason, SkipReason::Unreadable(text) if text == "no permission") {
                    privacy = true;
                }
                let shown = self.skipped_display(&skip.path);
                let reason = skip_reason_text(&skip.reason);
                rows.push(
                    div()
                        .id(("search-skipped-row", index))
                        .debug_selector(move || format!("search-skipped-row-{index}"))
                        .role(Role::ListItem)
                        .aria_label(format!("{shown}, {reason}"))
                        .px_3()
                        .py_1()
                        .flex()
                        .flex_col()
                        .child(div().text_size(px(12.)).child(shown))
                        .child(div().text_size(px(11.)).text_color(muted).child(reason))
                        .into_any_element(),
                );
                index += 1;
            }
            if privacy {
                rows.push(
                    div()
                        .id("search-skipped-privacy")
                        .debug_selector(|| "search-skipped-privacy".into())
                        .role(Role::Button)
                        .aria_label("Open Privacy & Security in System Settings")
                        .px_3()
                        .py_1()
                        .text_size(px(12.))
                        .text_color(accent)
                        .hover(|style| style.underline())
                        .on_click(cx.listener(|_, _, _, cx| cx.open_url(PRIVACY_SETTINGS_URL)))
                        .child("Open Privacy & Security…")
                        .into_any_element(),
                );
            }
        }
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
                        .child(div().flex_1().child("Skipped folders"))
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
        if !self.focus_attached {
            self.focus_attached = true;
            let input_focus = self.input.read(cx).focus_handle(cx);
            self._focus_subscriptions = vec![
                cx.on_focus(&input_focus, window, |_, _, cx| cx.notify()),
                cx.on_blur(&input_focus, window, |_, _, cx| cx.notify()),
            ];
        }
        let plugin_generation =
            cx.try_global::<crate::plugins::Plugins>().map_or(0, |plugins| plugins.surfaces.generation());
        let views_generation = cx.global::<Ui>().views_generation;
        if self.preview_plugin_generation != plugin_generation || self.preview_views_generation != views_generation {
            self.preview_plugin_generation = plugin_generation;
            self.preview_views_generation = views_generation;
            self.clear_preview(cx);
            self.load_preview(cx);
        }
        self.request_plugin_status(cx);
        self.update_plugin_command_context(window, cx);
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        let idle = self.query(cx).trim().is_empty() && self.predicates.is_empty();
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
            .on_action(cx.listener(Self::search_home_action))
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
    matching: Match,
    /// Every accepted hit, including extra content lines for one file.
    hits: usize,
    /// Result rows: one per path.
    files: usize,
    names_capped: bool,
    contents_stopped: bool,
    metadata_capped: bool,
    stale: bool,
    dropped: usize,
    cloud_skipped: u64,
    tag_errors: u64,
    error: Option<&'a str>,
}

/// " · 3 cloud files not downloaded", when content coverage was reduced by
/// placeholders. Name and metadata hits still count, so this is not an error.
fn cloud_text(n: u64) -> String {
    match n {
        0 => String::new(),
        1 => " · 1 cloud file not downloaded".to_owned(),
        n => format!(" · {n} cloud files not downloaded"),
    }
}

/// "3 results" / "3 results in 1 file", without the searching prefix.
fn results_text(hits: usize, files: usize) -> String {
    let results = if hits == 1 { "1 result".to_owned() } else { format!("{hits} results") };
    if hits > files {
        let files = if files == 1 { "1 file".to_owned() } else { format!("{files} files") };
        format!("{results} in {files}")
    } else {
        results
    }
}

fn tag_text(n: u64) -> String {
    if n == 0 { String::new() } else { format!(" · {n} with unreadable tags") }
}

fn footer_summary(state: FooterState<'_>) -> String {
    if let Some(error) = state.error {
        return error.lines().next().unwrap_or(error).to_owned();
    }
    if state.names_capped {
        return format!("First {MAX_STRONG_ROWS} name matches · refine the search");
    }
    let tags = tag_text(state.tag_errors);
    if state.metadata_capped {
        let results = results_text(state.hits, state.files);
        return format!("{results} · first {} matches; partial results{tags}", crow_search::MAX_METADATA_RESULTS);
    }
    if state.contents_stopped {
        let results = results_text(state.hits, state.files);
        let dropped = state.dropped;
        let cloud = cloud_text(state.cloud_skipped);
        return if state.searching {
            format!("Searching names… {results} · {dropped} more not shown{cloud}{tags}")
        } else if state.matching == Match::Names {
            format!("{results} · {dropped} more not shown{tags}")
        } else {
            format!("{results} · {dropped} more not shown · contents stopped at {MAX_OTHER_ROWS}{cloud}{tags}")
        };
    }
    let prefix = if state.searching { "Searching… " } else { "" };
    let cloud = cloud_text(state.cloud_skipped);
    let stale = if state.stale && !state.searching { " · previous results kept" } else { "" };
    if state.hits == 0 {
        return if state.searching { "Searching…".to_owned() } else { format!("No results{cloud}{stale}{tags}") };
    }
    format!("{prefix}{}{cloud}{stale}{tags}", results_text(state.hits, state.files))
}

/// "1 folder skipped" / "3 folders skipped", for the footer link and its aria label.
pub fn skipped_text(n: usize) -> String {
    if n == 1 { "1 folder skipped".to_owned() } else { format!("{n} folders skipped") }
}

/// The line under a skipped folder: the rule's text, or what the setting means.
/// B3 extends this with packages and unreadable folders.
fn skip_reason_text(reason: &SkipReason) -> String {
    match reason {
        SkipReason::HiddenFolder => "hidden folder (Show Hidden Files is off)".to_owned(),
        SkipReason::Package => "package (Search everything looks inside)".to_owned(),
        other => other.to_string(),
    }
}

/// The group a reason belongs to, for the skipped list's header selector.
fn skip_kind(reason: &SkipReason) -> &'static str {
    match reason {
        SkipReason::Rule(_) => "excluded",
        SkipReason::HiddenFolder => "hidden",
        SkipReason::Package => "packages",
        SkipReason::Unreadable(_) => "unreadable",
    }
}

/// The skipped list in display order: exclusion rules, hidden folders, packages,
/// unreadable folders; paths sorted within each group. Only non-empty groups.
fn skip_groups(skips: &[Skip]) -> Vec<(&'static str, &'static str, Vec<&Skip>)> {
    const GROUPS: &[(&str, &str)] = &[
        ("Excluded", "excluded"),
        ("Hidden folders", "hidden"),
        ("Packages", "packages"),
        ("Couldn't read", "unreadable"),
    ];
    GROUPS
        .iter()
        .filter_map(|&(heading, kind)| {
            let mut group: Vec<&Skip> = skips.iter().filter(|skip| skip_kind(&skip.reason) == kind).collect();
            group.sort_by(|a, b| a.path.cmp(&b.path));
            (!group.is_empty()).then_some((heading, kind, group))
        })
        .collect()
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
        let mut label = display_text(&self.name).into_owned();
        if self.hidden {
            label.push_str(", hidden");
        }
        label.push_str(&format!(", in {}, modified {}", self.folder, format_time(self.modified)));
        if let Some((line, text, _)) = &self.content {
            label.push_str(&format!(", line {line}: {text}"));
        }
        label
    }

    fn from_hit(hit: Hit, home: &Path, folder_root: Option<&Path>, strong_name: bool) -> Row {
        let (content, name_match, name_ranges) = match hit.kind {
            HitKind::Content { line, text, ranges, .. } => (Some((line, text, ranges)), false, Vec::new()),
            HitKind::Name { ranges } => (None, true, ranges),
            HitKind::Metadata { .. } => (None, false, Vec::new()),
        };
        Self::new(
            hit.path,
            hit.is_dir,
            hit.hidden,
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
        // Spotlight carries no Finder hidden flag; a dot name is all it can tell.
        let hidden = name.starts_with('.');
        Self::new(hit.path, hit.is_dir, hidden, hit.modified, 0, true, true, ranges, None, home, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn new(
        path: PathBuf,
        is_dir: bool,
        hidden: bool,
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
        let is_package = is_dir && crow_fs::is_package_name(&name);
        let mut row = Row {
            path,
            name,
            folder,
            is_dir,
            is_package,
            hidden,
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
    fn widen_options_are_ordered_and_gated() {
        assert_eq!(widen_options(Scope::Folder, true, Match::All, 0, false), [], "already Home");
        assert_eq!(widen_options(Scope::Folder, false, Match::All, 3, false), [Widen::Everything(3), Widen::Home]);
        assert_eq!(widen_options(Scope::Folder, false, Match::Names, 0, false), [Widen::Home, Widen::AllMatch]);
        assert_eq!(widen_options(Scope::Home, false, Match::Names, 0, false), [Widen::AllMatch]);
        assert_eq!(widen_options(Scope::ThisMac, false, Match::Names, 2, false), [Widen::Everything(2)]);
        assert_eq!(
            widen_options(Scope::Folder, false, Match::Contents, 2, false),
            [Widen::Everything(2), Widen::Home, Widen::AllMatch]
        );
        assert_eq!(
            widen_options(Scope::Folder, false, Match::Names, 2, true),
            [Widen::Home, Widen::AllMatch],
            "everything is already on"
        );
    }

    #[test]
    fn footer_summary_copy() {
        let state = |searching, hits, files, names_capped, contents_stopped, dropped, error| FooterState {
            searching,
            matching: Match::All,
            hits,
            files,
            names_capped,
            contents_stopped,
            metadata_capped: false,
            stale: false,
            dropped,
            cloud_skipped: 0,
            tag_errors: 0,
            error,
        };
        assert_eq!(footer_summary(state(true, 0, 0, false, false, 0, None)), "Searching…");
        assert_eq!(footer_summary(state(true, 44, 15, false, false, 0, None)), "Searching… 44 results in 15 files");
        assert_eq!(footer_summary(state(false, 44, 15, false, false, 0, None)), "44 results in 15 files");
        assert_eq!(footer_summary(state(false, 3, 3, false, false, 0, None)), "3 results");
        assert_eq!(footer_summary(state(false, 1, 1, false, false, 0, None)), "1 result");
        assert_eq!(footer_summary(state(false, 2, 1, false, false, 0, None)), "2 results in 1 file");
        assert_eq!(footer_summary(state(false, 0, 0, false, false, 0, None)), "No results");
        assert_eq!(
            footer_summary(state(false, 20_000, 20_000, true, false, 0, None)),
            "First 20000 name matches · refine the search"
        );
        assert_eq!(
            footer_summary(state(true, 44, 15, false, true, 3, None)),
            "Searching names… 44 results in 15 files · 3 more not shown"
        );
        assert_eq!(
            footer_summary(state(false, 44, 15, false, true, 3, None)),
            "44 results in 15 files · 3 more not shown · contents stopped at 20000"
        );
        assert_eq!(
            footer_summary(state(false, 1, 1, false, true, 1, None)),
            "1 result · 1 more not shown · contents stopped at 20000"
        );
        let mut names = state(false, MAX_OTHER_ROWS, MAX_OTHER_ROWS, false, true, 80_000, None);
        names.matching = Match::Names;
        assert_eq!(footer_summary(names), "20000 results · 80000 more not shown");
        let mut contents = state(false, MAX_OTHER_ROWS, MAX_OTHER_ROWS, false, true, 1, None);
        contents.matching = Match::Contents;
        assert_eq!(footer_summary(contents), "20000 results · 1 more not shown · contents stopped at 20000");
        // The strong cap is the bigger fact, and an error outranks both.
        assert_eq!(
            footer_summary(state(false, 44, 15, true, true, 3, None)),
            "First 20000 name matches · refine the search"
        );
        assert_eq!(
            footer_summary(state(false, 0, 0, true, true, 3, Some("Invalid pattern: unclosed group"))),
            "Invalid pattern: unclosed group"
        );
        let mut cloud = state(false, 2, 1, false, false, 0, None);
        cloud.cloud_skipped = 3;
        assert_eq!(footer_summary(cloud), "2 results in 1 file · 3 cloud files not downloaded");
        let mut one = state(false, 0, 0, false, false, 0, None);
        one.cloud_skipped = 1;
        assert_eq!(footer_summary(one), "No results · 1 cloud file not downloaded");
    }

    #[test]
    fn skipped_text_is_singular_for_one() {
        assert_eq!(skipped_text(1), "1 folder skipped");
        assert_eq!(skipped_text(3), "3 folders skipped");
        assert_eq!(skipped_was_text(1), "1 folder was skipped");
        assert_eq!(skipped_was_text(3), "3 folders were skipped");
        assert_eq!(skip_reason_text(&SkipReason::HiddenFolder), "hidden folder (Show Hidden Files is off)");
        assert_eq!(skip_reason_text(&SkipReason::Rule("dev: name \"target\"".to_owned())), "dev: name \"target\"");
        assert_eq!(skip_reason_text(&SkipReason::Unreadable("no permission".to_owned())), "no permission");
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
    fn spotlight_tag_expression_combines_literal_name_and_tags() {
        let predicates = crow_search::Predicates {
            kind: None,
            modified_within: None,
            tags: vec!["Work".into(), "say \"hi\"".into()],
        };
        assert_eq!(
            spotlight_expression_for(&predicates, "wo*rk?").as_deref(),
            Some(r#"kMDItemFSName == "*wo\*rk\?*"cd && kMDItemUserTags == "Work" && kMDItemUserTags == "say \"hi\"""#)
        );
        assert_eq!(
            spotlight_expression_for(&predicates, "").as_deref(),
            Some(r#"kMDItemUserTags == "Work" && kMDItemUserTags == "say \"hi\"""#)
        );
    }

    #[test]
    fn spotlight_forces_names() {
        assert_eq!(effective_match_for(Scope::ThisMac, Match::Contents), Match::Names);
        assert_eq!(effective_match_for(Scope::ThisMac, Match::All), Match::Names);
        assert_eq!(effective_match_for(Scope::Folder, Match::Contents), Match::Contents);
        assert_eq!(effective_match_for(Scope::Folder, Match::All), Match::All);
    }
}
