//! One open folder: its listing, sort, selection and live updates. The list view
//! shows one; the column view shows a chain of them.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Receiver;
#[cfg(test)]
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crow_core::motion::{self, Effective, Fx, Look};
use crow_core::{Selection, TypeSelect};
use crow_fs::{Entry, FolderChange, FolderWatcher, Sort};
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{
    Context, Entity, EventEmitter, ScrollStrategy, SharedString, Subscription, Task, UniformListScrollHandle,
    prelude::*,
};

use crate::folder_sizes::FolderSizes;
use crate::info_pane::{SelectionSummary, TAG_SCAN_CAP};
use crate::style::kind;

/// Show what has been read so far at most this often while a big folder loads.
const PARTIAL_EVERY: Duration = Duration::from_millis(50);
/// More changes than this at once: relisting is cheaper than diffing.
const MAX_DIFF: usize = 500;
/// Wait this long for the watcher stream to go quiet, then re-stat the paths
/// recent batches added (FSEvents can coalesce away a follow-up delete).
const RECONCILE_DELAY: Duration = Duration::from_millis(300);

pub enum FolderEvent {
    /// The cursor moved to another item (or to none).
    CursorMoved,
    /// A listing finished.
    Loaded,
    /// The visible rows changed: a listing, an expansion or a child folder.
    Changed,
    /// A type-to-select run started or expired; the bottom bar shows its prefix.
    TypeSelect,
}

/// The deepest folder list view expands inline; stops symlink cycles.
const MAX_DEPTH: usize = 32;

/// The selected rows as (path, is a folder), shared with drag values.
pub(crate) type SelectionEntries = Arc<Vec<(PathBuf, bool)>>;

/// The cached selection: the generation it was built for, and the entries.
type SelectionCache = RefCell<Option<(u64, SelectionEntries)>>;

/// One visible row: an entry of this folder, or one of an expanded folder under it.
enum FlatRow {
    /// Index into `entries`.
    Own(usize),
    /// An entry of a child folder and its depth under this one.
    Child(Entry, usize),
}

/// The open children's rows, by path. Arc clones, so an apply can snapshot the
/// tree for a background flatten without copying a 100k listing.
type ChildRows = HashMap<PathBuf, (Arc<Vec<FlatRow>>, Arc<Vec<Entry>>)>;

/// A flattened listing: the rows and the per-row tables the list view reads.
struct Rows {
    rows: Arc<Vec<FlatRow>>,
    row_ends: Arc<Vec<usize>>,
    row_parents: Arc<Vec<Option<usize>>>,
    match_ranges: Arc<Vec<(u32, u32)>>,
}

/// Which part of an apply's tail to run once its rows are in place.
#[derive(Clone, Copy, Debug)]
enum ApplyKind {
    /// A staged load's partial: no prune, no first-item select.
    Partial,
    /// A full listing; `prune` is false when a watcher batch named its removals.
    Full { prune: bool },
}

/// A feedback effect playing on one row (trash, move out, receive, arrive, refuse).
/// The folder keeps it until the effect's phases end, so the row can render its look
/// and the rows below can close the gap after the listing drops it.
pub struct RowEffect {
    pub path: PathBuf,
    pub fx: Fx,
    /// The mode's phases for this effect, computed once at start.
    phases: Vec<Duration>,
    /// The row's index when the effect started, for the ghost and the FLIP shift.
    pub row_index: usize,
    pub started: Instant,
    /// The row's entry, for a ghost after the listing drops it.
    pub entry: Entry,
    /// The listing dropped this row while the effect was running.
    pub removed: bool,
    /// When the listing dropped it, so the close starts then, not before.
    removed_at: Option<Instant>,
}

impl RowEffect {
    fn new(path: PathBuf, fx: Fx, mode: Effective, row_index: usize, entry: Entry) -> Self {
        RowEffect {
            path,
            fx,
            phases: motion::phases(mode, fx),
            row_index,
            started: Instant::now(),
            entry,
            removed: false,
            removed_at: None,
        }
    }

    /// How the row looks at `now`: each phase in turn, then the last one at its end.
    pub fn look(&self, mode: Effective, now: Instant) -> Look {
        motion::look_in(&self.phases, mode, self.fx, self.started, now)
    }

    /// Still in the first phase: the row is drawn where it is, not closing yet.
    pub fn entering(&self, now: Instant) -> bool {
        self.phases.first().is_some_and(|first| now.saturating_duration_since(self.started) < *first)
    }

    /// The first phase's length (the swallow or fade out).
    fn first_phase(&self) -> Duration {
        self.phases.first().copied().unwrap_or_default()
    }

    /// The second phase's length (the close), or zero when there is none.
    fn close_phase(&self) -> Duration {
        self.phases.get(1).copied().unwrap_or_default()
    }

    /// When the close starts: after the swallow, or when the listing dropped the
    /// row if that came later (the ghost covers an early removal).
    fn close_start(&self) -> Instant {
        let after_swallow = self.started + self.first_phase();
        match self.removed_at {
            Some(removed) if removed > after_swallow => removed,
            _ => after_swallow,
        }
    }
}

/// A folder row opening or closing inline (list view, Full and Low motion).
struct Disclosure {
    path: PathBuf,
    opening: bool,
    mode: Effective,
    /// When the triangle was toggled: the chevron turns from here.
    toggled: Instant,
    /// When the child rows appeared (opening) or left (closing): rows move from here.
    /// An opening waits for the child listing, which loads in the background.
    started: Option<Instant>,
    /// The folder's row index then, and how many rows it showed or hid.
    parent_ix: usize,
    rows: usize,
}

impl Disclosure {
    fn duration(&self) -> Duration {
        motion::disclose_duration(self.mode)
    }

    /// Progress of the rows' motion at `now`, while it runs.
    fn progress(&self, now: Instant) -> Option<f32> {
        let started = self.started?;
        let elapsed = now.saturating_duration_since(started);
        let duration = self.duration();
        (elapsed < duration).then(|| elapsed.as_secs_f32() / duration.as_secs_f32())
    }

    fn running(&self, now: Instant) -> bool {
        self.progress(now).is_some() || now.saturating_duration_since(self.toggled) < self.duration()
    }
}

/// An opening whose child rows never showed up (an empty folder) is dropped after this.
const DISCLOSE_WAIT: Duration = Duration::from_secs(2);

/// At most this many visible rows animate per operation; the rest just go.
const MAX_EFFECTS: usize = 20;
/// A removed row's close may start after the effect's own phases (the watcher can
/// lag); keep the effect this much longer so the FLIP is never cut short.
const WATCH_GRACE: Duration = Duration::from_millis(250);

/// A folder opened inline (list view): its listing and the events that change it.
struct ChildFolder {
    folder: Entity<Folder>,
    _subscription: Subscription,
}

enum Load {
    Partial(Vec<Entry>),
    Done(std::io::Result<(Vec<Entry>, f64)>),
}

pub struct Folder {
    dir: PathBuf,
    entries: Arc<Vec<Entry>>,
    load_ms: f64,
    first_paint_ms: Option<f64>,
    loading: bool,
    /// Increments per load, so results from an earlier load are dropped.
    generation: u64,
    sort: Sort,
    selection: Selection<PathBuf>,
    /// Row of the selection's cursor in `entries`, kept in step with it.
    cursor_ix: Option<usize>,
    /// The cursor's row and path before an apply. If the apply dropped it, the
    /// row now at that index (or the new last row) becomes the cursor, as in Finder.
    cursor_before_apply: Option<(usize, PathBuf)>,
    /// After going up or back, Finder selects the folder you came from.
    select_after_load: Option<PathBuf>,
    /// Items an operation produced: selected as soon as they appear in the listing.
    select_when_present: Vec<PathBuf>,
    /// The mode their Arrive effect plays in.
    arrival_mode: Effective,
    /// Select the first item once loaded. Off for column-view children, which show
    /// a folder's contents without selecting anything until you move into them.
    auto_select: bool,
    type_select: TypeSelect,
    /// Bumped per typed letter, so only the last run's timer clears the prefix.
    type_select_gen: u64,
    show_hidden: bool,
    /// Name filter (list/column view): shown rows are this folder's matches, flat.
    filter: Option<String>,
    /// First match in each shown row's name, as byte offsets. Aligned with `rows`
    /// while a filter is set; `(u32::MAX, u32::MAX)` where the row has no range.
    match_ranges: Arc<Vec<(u32, u32)>>,
    /// List view: folders shown inline, by path.
    children: HashMap<PathBuf, ChildFolder>,
    expanded: HashSet<PathBuf>,
    /// The rows list view draws: this folder's entries with expanded children flattened in.
    rows: Arc<Vec<FlatRow>>,
    /// For each row, one past the last row of its subtree.
    row_ends: Arc<Vec<usize>>,
    /// For each nested row, the row of the expanded folder that contains it.
    row_parents: Arc<Vec<Option<usize>>>,
    /// Bumped whenever the selection changes (not on a reorder or a reload).
    selection_generation: u64,
    /// Bumped by every row build, so a background flatten that lost the race is
    /// dropped instead of overwriting newer rows.
    rows_generation: u64,
    /// Background row builds in flight. Until the last one lands, the entries are
    /// newer than the rows; checks and tests wait for it (`is_settling`).
    rows_pending: usize,
    /// The load's full listing has arrived; the next full apply ends `loading`.
    /// A sync rebuild during the stream must not, or the window reopens.
    load_done: bool,
    /// Feedback effects on visible rows (trash, refuse), cleared when their phases end.
    effects: Vec<RowEffect>,
    /// Bumped per effect batch, so an older timer cannot clear a newer effect.
    effect_generation: u64,
    /// Folders opening or closing inline (list view).
    disclosures: Vec<Disclosure>,
    /// The selected rows for dragging, rebuilt only after one of those changes.
    selection_entries: SelectionCache,
    /// The selected files' bytes and folders among them, kept current by every
    /// selection change: the bottom bar asks every frame, and rescanning a 100k
    /// listing per apply was 6% of the main thread's busy time (H4 batch 6).
    selection_totals: (u64, usize),
    /// The Info tab's multi-selection summary, rebuilt only when the selection changes.
    selection_summary: RefCell<Option<(u64, SelectionSummary)>>,
    /// The selected folders' known bytes, keyed by (selection generation, size generation).
    selection_folder_bytes: RefCell<Option<(u64, u64, u64)>>,
    scroll: UniformListScrollHandle,
    changes_rx: Receiver<FolderChange>,
    /// A sender kept only for tests, which queue watcher batches without the disk.
    #[cfg(test)]
    changes_tx: Sender<FolderChange>,
    /// Tests hold an apply in flight to reproduce out-of-order batches.
    #[cfg(test)]
    apply_delay: Duration,
    /// Tests count spawned watcher batches.
    #[cfg(test)]
    apply_batches: usize,
    /// A watcher batch is being stat'ed and applied; new changes wait for it.
    applying: bool,
    /// Paths that arrived while `applying`; the next batch takes them.
    pending: Vec<PathBuf>,
    /// A rescan arrived while `applying`.
    pending_rescan: bool,
    /// Tests hold a re-sort in flight to reproduce a stale result landing late.
    #[cfg(test)]
    sort_delay: Duration,
    _watcher: Option<FolderWatcher>,
    error: Option<SharedString>,
    /// Paths a watcher batch added; re-stat them once the stream is quiet,
    /// because FSEvents may coalesce away the delete that follows a rename.
    reconcile: Vec<PathBuf>,
    /// Bumped per batch, so only the last batch's quiet timer reconciles.
    reconcile_gen: u64,
    /// True while a reconcile timer or stat is in flight (Settle waits for it).
    reconciling: bool,
    _poll: Task<()>,
}

impl EventEmitter<FolderEvent> for Folder {}

impl Folder {
    pub fn new(dir: PathBuf, show_hidden: bool, sort: Sort, auto_select: bool, cx: &mut Context<Self>) -> Self {
        let (changes_tx, changes_rx) = std::sync::mpsc::channel();
        #[cfg(test)]
        let test_tx = changes_tx.clone();
        // 100 ms between checks doubles as the debounce for bursts of file changes.
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                if this.update(cx, |this, cx| this.apply_folder_changes(cx)).is_err() {
                    break;
                }
            }
        });
        let watcher = crow_fs::watch_folder(&dir, changes_tx)
            .inspect_err(|e| tracing::warn!("watch {}: {e}", dir.display()))
            .ok();
        let mut folder = Self {
            dir,
            entries: Arc::new(Vec::new()),
            load_ms: 0.0,
            first_paint_ms: None,
            loading: false,
            generation: 0,
            sort,
            selection: Selection::default(),
            cursor_ix: None,
            cursor_before_apply: None,
            select_after_load: None,
            select_when_present: Vec::new(),
            arrival_mode: Effective::Off,
            auto_select,
            type_select: TypeSelect::default(),
            type_select_gen: 0,
            show_hidden,
            filter: None,
            match_ranges: Arc::new(Vec::new()),
            children: HashMap::new(),
            expanded: HashSet::new(),
            rows: Arc::new(Vec::new()),
            row_ends: Arc::new(Vec::new()),
            row_parents: Arc::new(Vec::new()),
            selection_generation: 0,
            rows_generation: 0,
            rows_pending: 0,
            load_done: false,
            effects: Vec::new(),
            effect_generation: 0,
            disclosures: Vec::new(),
            selection_entries: RefCell::new(None),
            selection_totals: (0, 0),
            selection_summary: RefCell::new(None),
            selection_folder_bytes: RefCell::new(None),
            scroll: UniformListScrollHandle::new(),
            changes_rx,
            #[cfg(test)]
            changes_tx: test_tx,
            #[cfg(test)]
            apply_delay: Duration::ZERO,
            #[cfg(test)]
            apply_batches: 0,
            applying: false,
            pending: Vec::new(),
            pending_rescan: false,
            #[cfg(test)]
            sort_delay: Duration::ZERO,
            _watcher: watcher,
            error: None,
            reconcile: Vec::new(),
            reconcile_gen: 0,
            reconciling: false,
            _poll: poll,
        };
        folder.load(cx);
        folder
    }

    // ----- read access -----

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn entries(&self) -> &Arc<Vec<Entry>> {
        &self.entries
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    /// True while the listing or its rows are still being built: a check that
    /// reads rows must wait, because an apply swaps the entries before its rows.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn is_settling(&self) -> bool {
        self.loading || self.rows_pending > 0 || self.reconciling
    }

    /// True while any folder open inside this one is still settling.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn children_settling(&self, cx: &gpui::App) -> bool {
        self.children.values().any(|child| {
            let child = child.folder.read(cx);
            child.is_settling() || child.children_settling(cx)
        })
    }

    /// True while this folder or any folder open inside it is still listing,
    /// or a watcher batch is waiting to reconcile its additions.
    pub fn is_loading_deep(&self, cx: &gpui::App) -> bool {
        self.is_loading()
            || self.reconciling
            || self.children.values().any(|child| child.folder.read(cx).is_loading_deep(cx))
    }

    pub fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    pub fn load_ms(&self) -> f64 {
        self.load_ms
    }

    pub fn first_paint_ms(&self) -> Option<f64> {
        self.first_paint_ms
    }

    pub fn scroll(&self) -> &UniformListScrollHandle {
        &self.scroll
    }

    pub fn cursor_ix(&self) -> Option<usize> {
        self.cursor_ix
    }

    pub fn cursor_entry(&self) -> Option<&Entry> {
        self.row(self.cursor_ix?).map(|(entry, _)| entry)
    }

    /// Rows list view draws: this folder's entries with expanded children flattened in.
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// The entry at visible row `ix` and how deep it is indented under this folder.
    pub fn row(&self, ix: usize) -> Option<(&Entry, usize)> {
        match self.rows.get(ix)? {
            FlatRow::Own(i) => self.entries.get(*i).map(|entry| (entry, 0)),
            FlatRow::Child(entry, depth) => Some((entry, *depth)),
        }
    }

    /// Whether `path` is open inline, wherever in the tree it sits.
    pub fn is_expanded(&self, path: &Path, cx: &gpui::App) -> bool {
        self.expanded.contains(path) || self.children.values().any(|child| child.folder.read(cx).is_expanded(path, cx))
    }

    /// The open child folder that lists `path`, if it is nested under one of them.
    fn owner_of(&self, path: &Path, cx: &gpui::App) -> Option<Entity<Folder>> {
        let mut found: Option<Entity<Folder>> = None;
        for child in self.children.values() {
            let folder = child.folder.read(cx);
            if path.starts_with(folder.dir()) {
                found = Some(child.folder.clone());
                break;
            }
        }
        found
    }

    /// The expanded folder row that contains row `ix`, if it is nested.
    pub fn parent_row(&self, ix: usize) -> Option<usize> {
        self.row_parents.get(ix).copied().flatten()
    }

    /// The folder row a drop on row `ix` lands in: the row itself when it is a folder,
    /// the expanded folder that lists it when it is a nested file, `None` for a
    /// top-level file (the drop goes to this folder).
    pub fn drop_header(&self, ix: usize) -> Option<usize> {
        let (entry, depth) = self.row(ix)?;
        if entry.is_dir && !entry.is_package() {
            Some(ix)
        } else if depth > 0 {
            self.parent_row(ix)
        } else {
            None
        }
    }

    /// The row after the last row of the subtree rooted at `ix` (exclusive).
    pub fn subtree_end(&self, ix: usize) -> usize {
        self.row_ends.get(ix).copied().unwrap_or(self.row_count())
    }

    fn row_path<'a>(row: &'a FlatRow, entries: &'a [Entry]) -> Option<&'a Path> {
        match row {
            FlatRow::Own(i) => entries.get(*i).map(|entry| entry.path.as_path()),
            FlatRow::Child(entry, _) => Some(entry.path.as_path()),
        }
    }

    fn row_entry<'a>(row: &'a FlatRow, entries: &'a [Entry]) -> Option<&'a Entry> {
        match row {
            FlatRow::Own(i) => entries.get(*i),
            FlatRow::Child(entry, _) => Some(entry),
        }
    }

    fn index_of_path(&self, path: &Path) -> Option<usize> {
        self.rows.iter().position(|row| Self::row_path(row, &self.entries).is_some_and(|p| p == path))
    }

    /// The visible row showing `path`, including an inline child row.
    pub fn row_of_path(&self, path: &Path) -> Option<usize> {
        self.index_of_path(path)
    }

    /// The rows still index the current entries. An apply swaps the entries
    /// before its rows land; a key action in that window must not index the new
    /// listing with the old rows' indices.
    fn rows_match_entries(&self) -> bool {
        self.rows.iter().all(|row| match row {
            FlatRow::Own(i) => *i < self.entries.len(),
            FlatRow::Child(..) => true,
        })
    }

    pub fn is_selected(&self, path: &Path) -> bool {
        self.selection.contains(&path.to_path_buf())
    }

    pub fn selected_count(&self) -> usize {
        self.selection.len()
    }

    /// The selected entries, in display order.
    /// The selected rows as (path, is a folder), in display order; used for dragging.
    /// Cached: scanning a 100k folder per frame would show up in the scroll bench.
    pub fn selected_entries(&self) -> SelectionEntries {
        if let Some((generation, entries)) = &*self.selection_entries.borrow()
            && *generation == self.selection_generation
        {
            return entries.clone();
        }
        let entries = Arc::new(
            self.rows
                .iter()
                .filter_map(|row| match row {
                    FlatRow::Own(i) => self.entries.get(*i),
                    FlatRow::Child(entry, _) => Some(entry),
                })
                .filter(|entry| self.selection.contains(&entry.path))
                .map(|entry| (entry.path.clone(), entry.is_dir))
                .collect::<Vec<(PathBuf, bool)>>(),
        );
        *self.selection_entries.borrow_mut() = Some((self.selection_generation, entries.clone()));
        entries
    }

    /// The selected files' bytes and how many selected items are folders.
    /// Folders add no bytes: their size needs a disk walk. Kept current by every
    /// selection change; no listing scan.
    pub fn selected_bytes(&self) -> (u64, usize) {
        self.selection_totals
    }

    /// The selected files' bytes plus the known sizes of selected folders, and
    /// the folder count. Cached by selection and size generation, so a 100k
    /// selection is scanned once per change, not once per frame.
    pub fn selected_bytes_with_sizes(&self, sizes: &FolderSizes) -> (u64, usize) {
        let (files, folders) = self.selection_totals;
        if folders == 0 {
            return (files, 0);
        }
        let generation = sizes.generation();
        if let Some((selection, size_generation, bytes)) = *self.selection_folder_bytes.borrow()
            && selection == self.selection_generation
            && size_generation == generation
        {
            return (files + bytes, folders);
        }
        let bytes = self
            .selected_entries()
            .iter()
            .filter(|(_, is_dir)| *is_dir)
            .filter_map(|(path, _)| sizes.size(path))
            .map(|size| size.bytes)
            .sum();
        *self.selection_folder_bytes.borrow_mut() = Some((self.selection_generation, generation, bytes));
        (files + bytes, folders)
    }

    /// Bumped whenever the selection changes; the Info pane keys its load on it.
    pub fn selection_generation(&self) -> u64 {
        self.selection_generation
    }

    /// The multi-selection summary: count, exact file bytes, folder count, a common
    /// kind and parent, and up to `TAG_SCAN_CAP` paths for the shared-tag read.
    /// Cached by selection generation; one scan per selection change, no path clones
    /// beyond the capped tag list.
    pub fn selection_summary(&self) -> SelectionSummary {
        if let Some((generation, summary)) = &*self.selection_summary.borrow()
            && *generation == self.selection_generation
        {
            return summary.clone();
        }
        let (bytes, folders) = self.selection_totals;
        let mut count = 0;
        let mut common_parent: Option<PathBuf> = None;
        let mut parent_set = false;
        let mut parent_mixed = false;
        let mut common_kind: Option<(String, bool)> = None;
        let mut kind_mixed = false;
        let mut tag_paths = Vec::new();
        for row in self.rows.iter() {
            let entry = match row {
                FlatRow::Own(i) => self.entries.get(*i),
                FlatRow::Child(entry, _) => Some(entry),
            };
            let Some(entry) = entry else { continue };
            if !self.selection.contains(&entry.path) {
                continue;
            }
            count += 1;
            let parent = entry.path.parent().map(Path::to_path_buf);
            if !parent_set {
                common_parent = parent;
                parent_set = true;
            } else if !parent_mixed && common_parent != parent {
                parent_mixed = true;
            }
            if !kind_mixed {
                let is_folder = entry.is_dir && !entry.is_package();
                match &common_kind {
                    None => {
                        common_kind =
                            Some(if is_folder { (String::new(), true) } else { (entry.ext().to_lowercase(), false) });
                    }
                    Some((ext, first_is_folder)) => {
                        if *first_is_folder != is_folder || (!is_folder && !entry.ext().eq_ignore_ascii_case(ext)) {
                            kind_mixed = true;
                        }
                    }
                }
            }
            if tag_paths.len() < TAG_SCAN_CAP {
                tag_paths.push(entry.path.clone());
            }
        }
        let summary = SelectionSummary {
            count,
            bytes: Some(bytes),
            folders,
            kind: if kind_mixed {
                None
            } else {
                common_kind.map(|(ext, is_folder)| crow_macos::kind_name(&ext, is_folder))
            },
            parent: if parent_mixed { None } else { common_parent },
            tag_paths: tag_paths.into(),
            tags_truncated: count > TAG_SCAN_CAP,
        };
        *self.selection_summary.borrow_mut() = Some((self.selection_generation, summary.clone()));
        summary
    }

    /// A row's contribution to `selected_bytes`: file bytes, and 1 for a folder.
    fn entry_totals(entry: &Entry) -> (u64, usize) {
        if entry.is_dir { (0, 1) } else { (entry.size, 0) }
    }

    fn add_totals(totals: &mut (u64, usize), entry: &Entry) {
        let (bytes, folders) = Self::entry_totals(entry);
        totals.0 += bytes;
        totals.1 += folders;
    }

    fn subtract_totals(totals: &mut (u64, usize), entry: &Entry) {
        let (bytes, folders) = Self::entry_totals(entry);
        totals.0 = totals.0.saturating_sub(bytes);
        totals.1 = totals.1.saturating_sub(folders);
    }

    /// Whatever changed, the drag snapshot is stale.
    fn selection_changed(&mut self) {
        self.selection_generation += 1;
    }

    pub fn selected_paths(&self) -> Vec<PathBuf> {
        self.rows
            .iter()
            .filter_map(|row| match row {
                FlatRow::Own(i) => self.entries.get(*i),
                FlatRow::Child(entry, _) => Some(entry),
            })
            .filter(|entry| self.selection.contains(&entry.path))
            .map(|entry| entry.path.clone())
            .collect()
    }

    pub fn type_select_active(&self) -> bool {
        self.type_select.is_active(Instant::now())
    }

    /// What was typed in the current run (lowercased), whether or not it is still active.
    pub fn type_select_prefix(&self) -> &str {
        self.type_select.prefix()
    }

    /// End the typing run, if any. Returns whether one was active.
    pub fn cancel_type_select(&mut self, cx: &mut Context<Self>) -> bool {
        let active = self.type_select_active();
        if active {
            self.type_select.clear();
            self.type_select_gen += 1;
            cx.emit(FolderEvent::TypeSelect);
            cx.notify();
        }
        active
    }

    /// The active name filter, if any.
    pub fn filter(&self) -> Option<&str> {
        self.filter.as_deref()
    }

    /// The first filter match in shown row `ix`'s name, as byte offsets into it.
    pub fn row_match(&self, ix: usize) -> Option<Range<usize>> {
        let (start, end) = *self.match_ranges.get(ix)?;
        (start != u32::MAX).then_some(start as usize..end as usize)
    }

    /// Filter this folder's own rows (case-insensitive substring). Open inline
    /// folders are ignored while a filter is active; clearing it restores them.
    pub fn set_filter(&mut self, filter: Option<String>, cx: &mut Context<Self>) {
        let filter = filter.filter(|query| !query.is_empty());
        if self.filter == filter {
            return;
        }
        self.filter = filter;
        self.rebuild_rows(cx);
        self.prune_selection();
        self.settle_cursor(cx);
        cx.emit(FolderEvent::Changed);
        cx.notify();
    }

    // ----- changes -----

    /// Moving into a column: select its first item, now or as soon as it has loaded.
    pub fn select_first(&mut self, cx: &mut Context<Self>) {
        self.auto_select = true;
        if self.cursor_ix.is_none() && !self.entries.is_empty() {
            self.select_only(0, cx);
        }
    }

    /// Select `paths` once they show up (an operation's results), then forget them.
    /// They play Arrive in `mode` when they appear.
    pub fn select_when_present(&mut self, paths: Vec<PathBuf>, mode: Effective, cx: &mut Context<Self>) {
        self.select_when_present = paths;
        self.arrival_mode = mode;
        self.reload(cx);
    }

    // ----- feedback effects -----

    /// Start `fx` on the visible rows among `paths` (at most 20), for `mode`'s
    /// duration. `off` starts nothing: the state changes in one frame.
    pub fn start_effect(&mut self, paths: &[PathBuf], fx: Fx, mode: Effective, cx: &mut Context<Self>) {
        if motion::phases(mode, fx).is_empty() {
            return;
        }
        let wanted: HashSet<&Path> = paths.iter().map(PathBuf::as_path).collect();
        let mut started = 0;
        for ix in 0..self.row_count() {
            if started >= MAX_EFFECTS {
                break;
            }
            let Some(entry) = self.row(ix).map(|(entry, _)| entry.clone()) else { continue };
            if !wanted.contains(entry.path.as_path()) {
                continue;
            }
            self.effects.retain(|effect| effect.path != entry.path);
            self.effects.push(RowEffect::new(entry.path.clone(), fx, mode, ix, entry));
            started += 1;
        }
        if started == 0 {
            return;
        }
        self.schedule_effect_clear(cx);
    }

    /// Clear the effects once the longest one has finished (plus the watcher
    /// grace). A later, shorter effect must not cut an earlier, longer one.
    fn schedule_effect_clear(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        let wait = self
            .effects
            .iter()
            .map(|effect| {
                let total: Duration = effect.phases.iter().sum();
                (effect.started + total + WATCH_GRACE).saturating_duration_since(now)
            })
            .max()
            .unwrap_or_default();
        self.effect_generation += 1;
        let generation = self.effect_generation;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, cx| {
                if this.effect_generation == generation {
                    this.effects.clear();
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Start a leaving effect for each row a watcher batch (or a permanent
    /// delete) removed without an operation effect, so the rows below close the
    /// gap. Off starts nothing: the state changes in one frame.
    fn start_removed_effects(&mut self, stats: &[(PathBuf, Option<Entry>)], cx: &mut Context<Self>) {
        // A rename arrives as a removal and an addition in the same batch: the
        // row is still there under its new name, so it is not a removal to animate.
        if stats.iter().any(|(_, entry)| entry.is_some()) {
            return;
        }
        let mode = cx.try_global::<crate::ui::Ui>().map_or(Effective::Off, |ui| ui.feedback.mode);
        if motion::phases(mode, Fx::Trash).is_empty() {
            return;
        }
        // The operation may already animate the first visible rows; the cap is
        // for the rows that move, so count the leaving effects already running.
        let mut started = self.effects.iter().filter(|effect| effect.fx.leaving()).count();
        for (path, entry) in stats {
            if entry.is_some() || started >= MAX_EFFECTS {
                continue;
            }
            if self.effects.iter().any(|effect| effect.path == *path) {
                continue;
            }
            let Some(ix) = self.index_of_path(path) else { continue };
            let Some((entry, _)) = self.row(ix) else { continue };
            let mut effect = RowEffect::new(path.clone(), Fx::Trash, mode, ix, entry.clone());
            effect.removed = true;
            effect.removed_at = Some(effect.started);
            self.effects.push(effect);
            started += 1;
        }
        if started > 0 {
            self.schedule_effect_clear(cx);
        }
    }

    /// How the row at `path` looks right now, if an effect is playing on it.
    pub fn effect_look(&self, path: &Path, mode: Effective, now: Instant) -> Option<Look> {
        self.effects.iter().find(|effect| effect.path == path).map(|effect| effect.look(mode, now))
    }

    pub fn effects(&self) -> &[RowEffect] {
        &self.effects
    }

    /// How far the rows below an effect have moved (Full motion's FLIP). A leaving
    /// row's close slides the rows below up; an arriving row's open slides them down
    /// from their old slot. `ix` is the row's index in the listing as it is now.
    pub fn row_shift(&self, ix: usize, mode: Effective, now: Instant, row_height: f32) -> f32 {
        if mode != Effective::Full {
            return 0.0;
        }
        let mut shift = self.disclosure_shift(ix, now) * row_height;
        // Arriving rows: the rows below start one row up and slide down into place.
        for effect in &self.effects {
            if effect.fx != Fx::Arrive || effect.removed || effect.row_index >= ix {
                continue;
            }
            let open = effect.first_phase();
            let elapsed = now.saturating_duration_since(effect.started);
            if elapsed < open {
                let t = if open.is_zero() { 1.0 } else { elapsed.as_secs_f32() / open.as_secs_f32() };
                shift -= row_height * (1.0 - motion::ease_out(t));
            }
        }
        let mut removed: Vec<&RowEffect> =
            self.effects.iter().filter(|effect| effect.removed && effect.fx.leaving()).collect();
        if removed.is_empty() {
            return shift;
        }
        removed.sort_by_key(|effect| effect.row_index);
        // The row's index before the removals: each removed row at or above it adds one.
        let mut original = ix;
        for effect in &removed {
            if effect.row_index <= original {
                original += 1;
            }
        }
        for effect in removed {
            if effect.row_index >= original {
                continue;
            }
            let close_start = effect.close_start();
            if now < close_start {
                continue;
            }
            let close = effect.close_phase();
            let t = if close.is_zero() {
                1.0
            } else {
                (now.saturating_duration_since(close_start).as_secs_f32() / close.as_secs_f32()).min(1.0)
            };
            shift += row_height * (1.0 - motion::ease_out(t));
        }
        shift
    }

    // ----- disclosure motion (list view) -----

    /// The rows nested under row `ix`: how many follow it at a greater depth.
    fn descendant_rows(&self, ix: usize) -> usize {
        let Some((_, depth)) = self.row(ix) else { return 0 };
        (ix + 1..self.row_count()).take_while(|row| self.row(*row).is_some_and(|(_, d)| d > depth)).count()
    }

    /// Note that `path` is about to open or close, in the current motion mode.
    /// A closing's rows leave now; an opening's arrive with the child listing.
    fn record_disclosure(&mut self, path: PathBuf, opening: bool, cx: &gpui::App) {
        let mode = cx.try_global::<crate::ui::Ui>().map_or(Effective::Off, |ui| ui.feedback.mode);
        let now = Instant::now();
        self.disclosures.retain(|d| {
            d.path != path
                && (d.running(now) || d.started.is_none() && now.saturating_duration_since(d.toggled) < DISCLOSE_WAIT)
        });
        if mode == Effective::Off {
            return;
        }
        let parent_ix = self.index_of_path(&path).unwrap_or(0);
        let (started, rows) = if opening { (None, 0) } else { (Some(now), self.descendant_rows(parent_ix)) };
        self.disclosures.push(Disclosure { path, opening, mode, toggled: now, started, parent_ix, rows });
    }

    /// Start the openings whose child rows have just appeared.
    fn start_disclosures(&mut self) {
        let now = Instant::now();
        for ix in 0..self.disclosures.len() {
            let d = &self.disclosures[ix];
            if !d.opening || d.started.is_some() || now.saturating_duration_since(d.toggled) >= DISCLOSE_WAIT {
                continue;
            }
            let Some(parent_ix) = self.index_of_path(&d.path) else { continue };
            let rows = self.descendant_rows(parent_ix);
            if rows == 0 {
                continue;
            }
            let d = &mut self.disclosures[ix];
            d.started = Some(now);
            d.parent_ix = parent_ix;
            d.rows = rows;
        }
    }

    /// For tests: row `ix`'s shift (rows), opacity and `path`'s chevron turn, `after`
    /// the disclosure of `path` started moving (or was toggled, for the chevron).
    #[cfg(test)]
    pub fn disclosure_probe(&self, path: &Path, ix: usize, after: Duration) -> Option<(f32, f32, Option<f32>)> {
        let d = self.disclosures.iter().rev().find(|d| d.path == path)?;
        let rows_at = d.started? + after;
        let chevron_at = d.toggled + after;
        Some((
            self.disclosure_shift(ix, rows_at),
            self.disclosure_opacity(ix, rows_at),
            self.chevron_turn(path, chevron_at),
        ))
    }

    /// Whether any row effect or disclosure is still moving: the pane asks for frames.
    pub fn animating(&self, now: Instant) -> bool {
        !self.effects.is_empty()
            || self.disclosures.iter().any(|d| {
                d.running(now)
                    || d.opening && d.started.is_none() && now.saturating_duration_since(d.toggled) < DISCLOSE_WAIT
            })
    }

    /// Full: how far row `ix` sits from its place, in rows, because a folder above
    /// it is opening or closing.
    fn disclosure_shift(&self, ix: usize, now: Instant) -> f32 {
        let mut shift = 0.0;
        for d in &self.disclosures {
            if d.mode != Effective::Full {
                continue;
            }
            let Some(t) = d.progress(now) else { continue };
            let below = if d.opening { d.parent_ix + d.rows } else { d.parent_ix };
            if ix > below {
                shift += motion::disclose_shift(d.opening, d.rows, t);
            }
        }
        shift
    }

    /// The opacity of row `ix` while the folder above it opens (1 otherwise).
    pub fn disclosure_opacity(&self, ix: usize, now: Instant) -> f32 {
        let mut opacity = 1.0;
        for d in &self.disclosures {
            if !d.opening || ix <= d.parent_ix || ix > d.parent_ix + d.rows {
                continue;
            }
            if let Some(t) = d.progress(now) {
                opacity *= motion::disclose_opacity(d.mode, ix - d.parent_ix, d.rows, t);
            }
        }
        opacity
    }

    /// Full: how far the chevron for `path` has turned towards open (0 closed, 1
    /// open) while it turns; `None` when it is at rest.
    pub fn chevron_turn(&self, path: &Path, now: Instant) -> Option<f32> {
        let d = self.disclosures.iter().rev().find(|d| d.path == path && d.mode == Effective::Full)?;
        let elapsed = now.saturating_duration_since(d.toggled);
        let duration = d.duration();
        if elapsed >= duration {
            return None;
        }
        let t = motion::ease_out(elapsed.as_secs_f32() / duration.as_secs_f32());
        Some(if d.opening { t } else { 1.0 - t })
    }

    /// List the folder again now rather than waiting for the watcher.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
    }

    pub fn select_after_load(&mut self, path: PathBuf) {
        self.select_after_load = Some(path);
    }

    pub fn set_show_hidden(&mut self, show: bool, cx: &mut Context<Self>) {
        if self.show_hidden != show {
            self.show_hidden = show;
            self.load(cx);
            let children: Vec<Entity<Folder>> = self.children.values().map(|c| c.folder.clone()).collect();
            for child in children {
                child.update(cx, |child, cx| child.set_show_hidden(show, cx));
            }
        }
    }

    pub fn set_sort(&mut self, sort: Sort, cx: &mut Context<Self>) {
        self.set_sort_with_sizes(sort, None, cx);
    }

    /// `sizes` patches folder rows with the size service's bytes before sorting.
    /// The pane passes it when the sort key is Size; the children are re-sorted
    /// by the pane when every visible size is in.
    pub fn set_sort_with_sizes(
        &mut self,
        sort: Sort,
        sizes: Option<Arc<HashMap<PathBuf, u64>>>,
        cx: &mut Context<Self>,
    ) {
        if self.sort == sort && sizes.is_none() {
            return;
        }
        self.sort = sort;
        self.sort_children(sort, cx);
        if self.loading {
            // The in-flight load sorts with the order captured when it started;
            // list again so its result arrives in this order.
            self.load(cx);
        } else {
            self.resort_with_sizes(sizes, 0, cx);
        }
    }

    fn sort_children(&mut self, sort: Sort, cx: &mut Context<Self>) {
        let children: Vec<Entity<Folder>> = self.children.values().map(|c| c.folder.clone()).collect();
        for child in children {
            child.update(cx, |child, cx| child.set_sort(sort, cx));
        }
    }

    /// Re-sort off the main thread; `sizes` patches folder rows first. The sort
    /// runs on the entries captured here; if the listing is replaced while it
    /// runs, its result is stale — sort the newer listing instead (or list again
    /// after two conflicts).
    pub(crate) fn resort_with_sizes(
        &mut self,
        sizes: Option<Arc<HashMap<PathBuf, u64>>>,
        attempt: u8,
        cx: &mut Context<Self>,
    ) {
        let sort = self.sort;
        let entries = self.entries.clone();
        let to_sort = entries.clone();
        let sizes_for_task = sizes.clone();
        #[cfg(test)]
        let delay = self.sort_delay;
        cx.spawn(async move |this, cx| {
            let sorted = cx
                .background_spawn(async move {
                    let mut entries = to_sort.as_ref().clone();
                    if let Some(sizes) = &sizes_for_task {
                        apply_folder_sizes(&mut entries, sizes);
                    }
                    crow_fs::sort_entries(&mut entries, &sort, &kind);
                    entries
                })
                .await;
            #[cfg(test)]
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            this.update(cx, |this, cx| {
                if this.sort != sort {
                    return;
                }
                if Arc::ptr_eq(&this.entries, &entries) {
                    this.replace_entries_after(sorted, Some(&[]), cx);
                } else if attempt < 2 {
                    this.resort_with_sizes(sizes, attempt + 1, cx);
                } else {
                    this.load(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// List the folder in the background. Big folders appear in stages: what has
    /// been read so far is shown (sorted) every 50 ms until done.
    fn load(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        // A fresh listing supersedes any pending reconcile of watcher additions.
        self.reconcile_gen += 1;
        self.reconcile.clear();
        self.reconciling = false;
        self.loading = true;
        self.load_done = false;
        let (generation, dir, hidden, sort) = (self.generation, self.dir.clone(), self.show_hidden, self.sort);
        let (tx, mut rx) = unbounded();
        let started = Instant::now();
        cx.background_spawn(async move { list_in_stages(&dir, hidden, sort, started, &tx) }).detach();
        cx.spawn(async move |this, cx| {
            while let Some(mut update) = rx.next().await {
                // A busy main thread queues several staged partials; only the
                // latest is worth a 100k apply (two in one frame were 38+38 ms).
                while let Ok(next) = rx.try_recv() {
                    update = next;
                }
                let current = this.update(cx, |this, cx| {
                    if this.generation != generation {
                        return false;
                    }
                    if this.first_paint_ms.is_none() {
                        this.first_paint_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
                    }
                    match update {
                        Load::Partial(entries) => this.apply_partial(entries, cx),
                        Load::Done(Ok((entries, ms))) => {
                            // `finish_apply` clears `loading` when the rows are in place.
                            this.load_done = true;
                            this.load_ms = ms;
                            tracing::debug!(items = entries.len(), ms, dir = %this.dir.display(), "loaded");
                            this.replace_entries(entries, cx);
                            cx.emit(FolderEvent::Loaded);
                        }
                        Load::Done(Err(e)) => {
                            this.loading = false;
                            this.load_done = false;
                            tracing::warn!("open {}: {e}", this.dir.display());
                            this.error = Some(format!("Can't open “{}”: {e}", this.dir.display()).into());
                            cx.emit(FolderEvent::Loaded);
                            cx.notify();
                        }
                    }
                    true
                });
                if !matches!(current, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    /// Swap in a new listing, keeping the selection on the same items.
    fn replace_entries(&mut self, entries: Vec<Entry>, cx: &mut Context<Self>) {
        self.replace_entries_after(entries, None, cx);
    }

    /// Show what a staged load has read so far. A partial only adds entries:
    /// pruning the selection, closing inline folders or reconciling effect
    /// state here would act on rows the load has not reached yet. The full
    /// listing does all of that when it lands.
    fn apply_partial(&mut self, entries: Vec<Entry>, cx: &mut Context<Self>) {
        self.apply_partial_inner(entries, cx, false);
    }

    fn apply_partial_inner(&mut self, entries: Vec<Entry>, cx: &mut Context<Self>, sync: bool) {
        let _span = tracing::debug_span!("listing partial").entered();
        self.replace_entries_arc(entries, cx);
        if sync {
            self.rebuild_rows(cx);
            self.finish_apply(ApplyKind::Partial, cx);
        } else {
            self.rebuild_rows_async(cx, ApplyKind::Partial);
        }
    }

    /// Select what a load or an operation asked for, and keep the selection's
    /// totals in step with it. Both the partial and the full apply call this.
    /// Returns whether it selected something (an operation result, not a removal).
    fn apply_pending_selection(&mut self, cx: &mut Context<Self>) -> bool {
        let mut selected = false;
        if let Some(target) = self.select_after_load.clone()
            && let Some(entry) = self.entries.iter().find(|entry| entry.path == target)
        {
            self.select_after_load = None;
            self.selection_totals = Self::entry_totals(entry);
            self.selection.select_only(target);
            self.selection_changed();
            selected = true;
        }
        if !self.select_when_present.is_empty() && self.all_present(&self.select_when_present) {
            let wanted = std::mem::take(&mut self.select_when_present);
            self.selection.clear();
            let mut totals = (0u64, 0usize);
            for path in &wanted {
                self.selection.toggle(path.clone());
            }
            // One pass over the listing, not one lookup per wanted path.
            let wanted_set: HashSet<&Path> = wanted.iter().map(PathBuf::as_path).collect();
            for entry in self.entries.iter() {
                if wanted_set.contains(entry.path.as_path()) {
                    Self::add_totals(&mut totals, entry);
                }
            }
            self.selection_totals = totals;
            self.selection_changed();
            selected = true;
            let mode = self.arrival_mode;
            self.start_effect(&wanted, Fx::Arrive, mode, cx);
        }
        selected
    }

    /// Whether every path is in the listing. A large result set (a 100k
    /// duplicate or paste) made the old per-path scan O(n²) and could hold the
    /// main thread for 30 s on one apply (findings #26).
    fn all_present(&self, paths: &[PathBuf]) -> bool {
        if paths.len() <= 16 {
            paths.iter().all(|path| self.entries.iter().any(|entry| &entry.path == path))
        } else {
            let present: HashSet<&Path> = self.entries.iter().map(|entry| entry.path.as_path()).collect();
            paths.iter().all(|path| present.contains(path.as_path()))
        }
    }

    /// Swap the listing and drop the old one off the main thread when it is
    /// large: freeing a 100k-entry listing is ~15 ms of the apply.
    fn replace_entries_arc(&mut self, entries: Vec<Entry>, cx: &mut Context<Self>) {
        let old = std::mem::replace(&mut self.entries, Arc::new(entries));
        Self::drop_listing(old, cx);
    }

    /// Free a listing off the main thread when it is large. Dropping a 100k
    /// listing on the main thread is a visible hitch on navigation, tab close
    /// and collapse, where the whole folder entity is dropped.
    fn drop_listing(entries: Arc<Vec<Entry>>, cx: &mut Context<Self>) {
        if entries.len() >= 1_024 {
            cx.background_spawn(async move {
                drop(entries);
            })
            .detach();
        }
    }

    /// Move this folder's listing (and its open children's) to the background
    /// before the entity is dropped. The pane calls this when it navigates away,
    /// closes a tab or collapses an inline folder.
    pub fn release_listing(&mut self, cx: &mut Context<Self>) {
        let children: Vec<Entity<Folder>> = self.children.values().map(|child| child.folder.clone()).collect();
        for child in children {
            child.update(cx, |child, cx| child.release_listing(cx));
        }
        let entries = std::mem::replace(&mut self.entries, Arc::new(Vec::new()));
        Self::drop_listing(entries, cx);
    }

    /// A watcher batch: only the paths it reports can have left the listing, so
    /// prune just those instead of hashing every row (a 100k select-all paid
    /// ~300 ms per batch). `stats` is `apply_stats`'s input.
    fn replace_entries_after(
        &mut self,
        entries: Vec<Entry>,
        stats: Option<&[(PathBuf, Option<Entry>)]>,
        cx: &mut Context<Self>,
    ) {
        self.replace_entries_after_inner(entries, stats, cx, false);
    }

    fn replace_entries_after_inner(
        &mut self,
        entries: Vec<Entry>,
        stats: Option<&[(PathBuf, Option<Entry>)]>,
        cx: &mut Context<Self>,
        sync: bool,
    ) {
        let _span = tracing::debug_span!("listing apply").entered();
        // Remember the cursor's row before the listing changes: if the apply drops
        // it, the row now at that index becomes the cursor (Finder's next row).
        self.cursor_before_apply =
            self.cursor_ix.and_then(|ix| self.row(ix).map(|(entry, _)| (ix, entry.path.clone())));
        // A watcher batch names the paths it removed; drop the selected ones before
        // the swap, while the old entries (and their sizes) are still here.
        if let Some(stats) = stats {
            let removed: Vec<&PathBuf> = stats
                .iter()
                .filter(|(path, entry)| entry.is_none() && self.selection.contains(path))
                .map(|(path, _)| path)
                .collect();
            if !removed.is_empty() {
                if removed.len() <= 16 {
                    // The usual batch: find each path and stop at the first hit.
                    for path in &removed {
                        if let Some(entry) = self.entries.iter().find(|entry| &entry.path == *path) {
                            Self::subtract_totals(&mut self.selection_totals, entry);
                        }
                    }
                } else {
                    // A bulk delete: one pass over the old listing, not one per path.
                    let gone: HashSet<&PathBuf> = removed.iter().copied().collect();
                    for entry in self.entries.iter() {
                        if gone.contains(&entry.path) {
                            Self::subtract_totals(&mut self.selection_totals, entry);
                        }
                    }
                }
                for path in &removed {
                    self.selection.remove(path);
                }
                self.selection_changed();
                // The cursor may have named one of them. Settle it now, not when the
                // new rows land: until then the old row would draw as the cursor
                // while the selection no longer holds it (the fuzzer's cursor check).
                self.settle_cursor(cx);
            }
            // A changed entry's size feeds the bottom bar's total too: adjust only
            // the batch's selected paths, not the whole 100k listing.
            let changed: Vec<(&PathBuf, &Entry)> = stats
                .iter()
                .filter(|(path, entry)| entry.is_some() && self.selection.contains(path))
                .filter_map(|(path, entry)| entry.as_ref().map(|entry| (path, entry)))
                .collect();
            if !changed.is_empty() {
                if changed.len() <= 16 {
                    for &(path, new) in &changed {
                        if let Some(old) = self.entries.iter().find(|entry| &entry.path == path) {
                            Self::subtract_totals(&mut self.selection_totals, old);
                        }
                        Self::add_totals(&mut self.selection_totals, new);
                    }
                } else {
                    // A bulk change: one pass over the old listing, not one per path.
                    let changed_paths: HashSet<&Path> = changed.iter().map(|(path, _)| path.as_path()).collect();
                    for entry in self.entries.iter() {
                        if changed_paths.contains(entry.path.as_path()) {
                            Self::subtract_totals(&mut self.selection_totals, entry);
                        }
                    }
                    for &(_, new) in &changed {
                        Self::add_totals(&mut self.selection_totals, new);
                    }
                }
            }
        }
        if let Some(stats) = stats {
            self.start_removed_effects(stats, cx);
        }
        self.replace_entries_arc(entries, cx);
        if !self.effects.is_empty() {
            // Only the effect paths matter (at most MAX_EFFECTS); hashing every
            // listing path cloned 100k paths on every apply with an effect running.
            let present: HashSet<PathBuf> = {
                let effect_paths: HashSet<&Path> = self.effects.iter().map(|effect| effect.path.as_path()).collect();
                let mut present = HashSet::with_capacity(effect_paths.len());
                for entry in self.entries.iter() {
                    if effect_paths.contains(entry.path.as_path()) {
                        present.insert(entry.path.clone());
                        if present.len() == effect_paths.len() {
                            break;
                        }
                    }
                }
                present
            };
            let mut restored = Vec::new();
            for effect in &mut self.effects {
                if present.contains(&effect.path) {
                    // Undo put the row back: stop its leaving effect rather than hide it.
                    if effect.removed {
                        restored.push(effect.path.clone());
                    }
                } else {
                    if !effect.removed {
                        effect.removed = true;
                        effect.removed_at = Some(Instant::now());
                    }
                }
            }
            self.effects.retain(|effect| !restored.contains(&effect.path));
        }
        self.sync_children(cx);
        if sync {
            self.rebuild_rows(cx);
            self.finish_apply(ApplyKind::Full { prune: stats.is_none() }, cx);
        } else {
            self.rebuild_rows_async(cx, ApplyKind::Full { prune: stats.is_none() });
        }
    }

    /// Open and close the inline child folders the expansion set asks for.
    fn sync_children(&mut self, cx: &mut Context<Self>) {
        let wanted: Vec<PathBuf> = self
            .entries
            .iter()
            .filter(|entry| entry.is_dir && !entry.is_package() && self.expanded.contains(&entry.path))
            .map(|entry| entry.path.clone())
            .collect();
        self.expanded.retain(|path| wanted.contains(path));
        // A collapsed inline folder may hold a large listing; free it off the
        // main thread before the entity drops.
        let removed: Vec<Entity<Folder>> = self
            .children
            .iter()
            .filter(|(path, _)| !wanted.contains(*path))
            .map(|(_, child)| child.folder.clone())
            .collect();
        for child in removed {
            child.update(cx, |child, cx| child.release_listing(cx));
        }
        self.children.retain(|path, _| wanted.contains(path));
        for path in wanted {
            if self.children.contains_key(&path) {
                continue;
            }
            let (hidden, sort) = (self.show_hidden, self.sort);
            let child = cx.new(|cx| Folder::new(path.clone(), hidden, sort, false, cx));
            let subscription = cx.subscribe(&child, |this, _, event, cx| {
                if matches!(event, FolderEvent::Loaded | FolderEvent::Changed) {
                    this.child_changed(cx);
                }
            });
            self.expanded.insert(path.clone());
            self.children.insert(path, ChildFolder { folder: child, _subscription: subscription });
        }
    }

    /// Flatten this listing and every open child folder into the visible rows.
    /// With a filter, the rows are this folder's matching entries only (flat).
    fn rebuild_rows(&mut self, cx: &gpui::App) {
        let _span = tracing::debug_span!("rebuild rows").entered();
        let children = self.child_rows(cx);
        let lower;
        let filter = match &self.filter {
            Some(query) => {
                lower = query.to_lowercase();
                Some((query.as_str(), lower.as_str()))
            }
            None => None,
        };
        let built = build_rows(&self.entries, &children, filter);
        self.assign_rows(built);
    }

    /// The open child folders' rows, by path. Arc clones: the flattening can then
    /// run off the main thread without reading entities.
    fn child_rows(&self, cx: &gpui::App) -> ChildRows {
        self.children
            .iter()
            .map(|(path, child)| {
                let child = child.folder.read(cx);
                (path.clone(), (child.rows.clone(), child.entries.clone()))
            })
            .collect()
    }

    fn assign_rows(&mut self, built: Rows) {
        self.rows = built.rows;
        self.row_ends = built.row_ends;
        self.row_parents = built.row_parents;
        self.match_ranges = built.match_ranges;
        self.rows_generation += 1;
    }

    /// Build the rows from a snapshot in the background, then finish the apply on
    /// the main thread. A 100k flatten plus its filter was 17–37 ms of the apply.
    fn rebuild_rows_async(&mut self, cx: &mut Context<Self>, kind: ApplyKind) {
        self.rows_pending += 1;
        let snapshot = self.entries.clone();
        let entries = self.entries.clone();
        let children = self.child_rows(cx);
        let filter = self.filter.clone();
        let generation = self.generation;
        let rows_generation = self.rows_generation;
        cx.spawn(async move |this, cx| {
            let built = cx
                .background_spawn(async move {
                    let lower = filter.as_ref().map(|query| query.to_lowercase());
                    let filter =
                        filter.as_ref().zip(lower.as_ref()).map(|(query, lower)| (query.as_str(), lower.as_str()));
                    build_rows(&snapshot, &children, filter)
                })
                .await;
            this.update(cx, |this, cx| {
                this.rows_pending = this.rows_pending.saturating_sub(1);
                // A newer listing, or a synchronous rebuild (an inline folder opened
                // or closed), owns the rows now; a stale flatten must not overwrite it.
                if this.generation != generation || !Arc::ptr_eq(&this.entries, &entries) {
                    return;
                }
                if this.rows_generation == rows_generation {
                    this.assign_rows(built);
                }
                this.finish_apply(kind, cx);
            })
            .ok();
        })
        .detach();
    }

    /// The rest of an apply, after its rows are in place.
    fn finish_apply(&mut self, kind: ApplyKind, cx: &mut Context<Self>) {
        let cursor_before = match kind {
            ApplyKind::Full { .. } => self.cursor_before_apply.take(),
            ApplyKind::Partial => None,
        };
        if let ApplyKind::Full { .. } = kind
            && self.load_done
        {
            // The listing is settled only now: until the rows land, the entries are
            // newer than the rows, and a check (or a key) that reads one through the
            // other sees a mismatch. A sync rebuild during the stream keeps it true.
            self.load_done = false;
            self.loading = false;
        }
        let pending_selected = self.apply_pending_selection(cx);
        if let ApplyKind::Full { prune } = kind {
            // Only the first full load selects the first item. After that a removal
            // selects the row now at the cursor's old index
            // (`select_next_after_removal`), never the top of the folder.
            if self.auto_select && !self.loading {
                self.auto_select = false;
                if self.selection.is_empty()
                    && let Some(first) = self.entries.first()
                {
                    self.selection_totals = Self::entry_totals(first);
                    self.selection.select_only(first.path.clone());
                    self.selection_changed();
                }
            }
            if prune {
                self.prune_selection();
            }
        }
        if !pending_selected && let Some((old_ix, old_path)) = cursor_before {
            self.select_next_after_removal(old_ix, old_path, cx);
        }
        self.settle_cursor(cx);
        // The drag snapshot is keyed by the selection, not by the rows: a reorder
        // does not invalidate it, and a 100k select-all no longer rescans per apply.
        cx.emit(FolderEvent::Changed);
        cx.notify();
    }

    /// Drop selected paths that are no longer visible anywhere in the tree, and
    /// sum what stays: the totals are rebuilt here so no other pass has to scan.
    fn prune_selection(&mut self) {
        let before = self.selection.len();
        let before_totals = self.selection_totals;
        if before == 0 {
            self.selection_totals = (0, 0);
            *self.selection_summary.borrow_mut() = None;
            return;
        }
        let entries = self.entries.clone();
        let rows = self.rows.clone();
        let mut totals = (0u64, 0usize);
        if before <= 16 {
            // The usual selection is one row. Walking the rows once and looking
            // each up in the small selection avoids hashing a 100k-row set on
            // every watcher batch (measured at ~180 ms per apply).
            let mut wanted: HashSet<PathBuf> = self.selection.iter().cloned().collect();
            let mut keep: HashSet<PathBuf> = HashSet::with_capacity(wanted.len());
            for row in rows.iter() {
                let Some(entry) = Self::row_entry(row, &entries) else { continue };
                if wanted.remove(&entry.path) {
                    keep.insert(entry.path.clone());
                    Self::add_totals(&mut totals, entry);
                    if wanted.is_empty() {
                        break;
                    }
                }
            }
            self.selection.retain(|path| keep.contains(path));
        } else {
            let mut present: HashSet<&Path> = HashSet::with_capacity(rows.len());
            for row in rows.iter() {
                let Some(entry) = Self::row_entry(row, &entries) else { continue };
                present.insert(entry.path.as_path());
                if self.selection.contains(&entry.path) {
                    Self::add_totals(&mut totals, entry);
                }
            }
            self.selection.retain(|path| present.contains(path.as_path()));
        }
        self.selection_totals = totals;
        if self.selection_totals != before_totals {
            // A selected file's size changed: the Info tab's cached summary is stale.
            *self.selection_summary.borrow_mut() = None;
        }
        if self.selection.len() != before {
            self.selection_changed();
        }
    }

    /// After a removal dropped the cursor row, put the cursor on the row now at
    /// its old index (or the new last row), as Finder does. Other selected rows
    /// stay selected; the new cursor row joins them, because the cursor row must
    /// be selected (the pane's invariant).
    fn select_next_after_removal(&mut self, old_ix: usize, old_path: PathBuf, cx: &mut Context<Self>) {
        if self.index_of_path(&old_path).is_some() {
            // A reorder (sort, filter, reload), not a removal.
            return;
        }
        let Some(last) = self.row_count().checked_sub(1) else { return };
        let ix = old_ix.min(last);
        let Some((entry, _)) = self.row(ix) else { return };
        let path = entry.path.clone();
        let totals = Self::entry_totals(entry);
        if self.selection.is_empty() {
            self.selection.select_only(path);
            self.selection_totals = totals;
            self.selection_changed();
        } else if self.selection.contains(&path) {
            self.selection.set_cursor(path);
        } else {
            self.selection.toggle(path);
            self.selection_totals.0 += totals.0;
            self.selection_totals.1 += totals.1;
            self.selection_changed();
        }
        self.cursor_ix = Some(ix);
        self.scroll.scroll_to_item(ix, ScrollStrategy::Center);
        cx.emit(FolderEvent::CursorMoved);
    }

    /// Point the cursor row at the selection's cursor; scroll if it moved.
    fn settle_cursor(&mut self, cx: &mut Context<Self>) {
        let previous = self.cursor_entry().map(|entry| entry.path.clone());
        self.cursor_ix = self.selection.cursor().and_then(|cursor| self.index_of_path(cursor));
        let now = self.cursor_entry().map(|entry| entry.path.clone());
        if now != previous {
            if let Some(ix) = self.cursor_ix {
                self.scroll.scroll_to_item(ix, ScrollStrategy::Center);
            }
            cx.emit(FolderEvent::CursorMoved);
        }
    }

    /// A child folder's listing or expansion changed: redraw the rows it contributes.
    fn child_changed(&mut self, cx: &mut Context<Self>) {
        self.rebuild_rows(cx);
        self.start_disclosures();
        self.prune_selection();
        self.settle_cursor(cx);
        cx.emit(FolderEvent::Changed);
        cx.notify();
    }

    /// List view: show `path`'s contents inline. A row nested under an open
    /// folder is opened by the folder that lists it.
    pub fn expand(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !self.is_expanded(&path, cx) {
            self.record_disclosure(path.clone(), true, cx);
        }
        self.expand_inner(path, cx);
        self.start_disclosures();
    }

    fn expand_inner(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !self.entries.iter().any(|entry| entry.path == path && entry.is_dir && !entry.is_package()) {
            if let Some(owner) = self.owner_of(&path, cx) {
                owner.update(cx, |owner, cx| owner.expand_inner(path, cx));
            }
            return;
        }
        self.expanded.insert(path);
        self.sync_children(cx);
        self.rebuild_rows(cx);
        self.prune_selection();
        self.settle_cursor(cx);
        cx.emit(FolderEvent::Changed);
        cx.notify();
    }

    /// List view: hide `path`'s contents, and everything below it.
    pub fn collapse(&mut self, path: &Path, cx: &mut Context<Self>) {
        if self.is_expanded(path, cx) {
            self.record_disclosure(path.to_path_buf(), false, cx);
        }
        self.collapse_inner(path, cx);
    }

    fn collapse_inner(&mut self, path: &Path, cx: &mut Context<Self>) {
        if !self.expanded.remove(path) {
            if let Some(owner) = self.owner_of(path, cx) {
                let path = path.to_path_buf();
                owner.update(cx, |owner, cx| owner.collapse_inner(&path, cx));
            }
            return;
        }
        // Free the child's listing before the entity drops; a 100k folder
        // collapsed on the main thread is a visible hitch.
        if let Some(child) = self.children.remove(path) {
            child.folder.update(cx, |child, cx| child.release_listing(cx));
        }
        self.sync_children(cx);
        self.rebuild_rows(cx);
        self.prune_selection();
        self.settle_cursor(cx);
        cx.emit(FolderEvent::Changed);
        cx.notify();
    }

    /// The disclosure triangle: expand or collapse.
    pub fn toggle_expansion(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.is_expanded(&path, cx) {
            self.collapse(&path, cx);
        } else {
            self.expand(path, cx);
        }
    }

    fn apply_folder_changes(&mut self, cx: &mut Context<Self>) {
        let mut paths = std::mem::take(&mut self.pending);
        let mut rescan = std::mem::take(&mut self.pending_rescan);
        for change in self.changes_rx.try_iter() {
            match change {
                FolderChange::Paths(p) => paths.extend(p),
                FolderChange::Rescan => rescan = true,
            }
        }
        if paths.is_empty() && !rescan {
            return;
        }
        // One batch at a time: the poll used to spawn a second 100k apply while
        // the first was in flight, and both could land in the same frame (a
        // 152 ms step on the 100k fixture). Changes wait for the next batch.
        if self.applying || self.loading {
            self.pending.extend(paths);
            self.pending_rescan |= rescan;
            return;
        }
        if rescan || paths.len() > MAX_DIFF {
            self.load(cx);
            return;
        }
        self.applying = true;
        #[cfg(test)]
        {
            self.apply_batches += 1;
        }
        let (dir, hidden, sort, generation) = (self.dir.clone(), self.show_hidden, self.sort, self.generation);
        #[cfg(test)]
        let delay = self.apply_delay;
        cx.spawn(async move |this, cx| {
            // Stat and merge off the main thread; the merge is against the listing
            // captured here, and the landing checks it is still current. A batch
            // that lands late therefore cannot replay the older listing it was
            // computed from (which is how deleted files came back).
            let entries = this.update(cx, |this, _| this.entries.clone()).ok();
            let Some(entries) = entries else { return };
            let snapshot = entries.clone();
            let (stats, updated) = cx
                .background_spawn(async move {
                    let stats = crow_fs::stat_changes(&dir, &paths, hidden);
                    let updated = crow_fs::apply_stats(&snapshot, &stats, &sort, &kind);
                    (stats, updated)
                })
                .await;
            #[cfg(test)]
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            this.update(cx, |this, cx| {
                this.applying = false;
                if this.generation == generation && this.sort == sort {
                    // The paths this batch added: the reconcile re-stats only those.
                    // One pass over the listing, not one per changed path: a burst
                    // of 500 adds in a 100k folder took ~800 ms on the main thread.
                    let mut added: HashSet<PathBuf> =
                        stats.iter().filter(|(_, entry)| entry.is_some()).map(|(path, _)| path.clone()).collect();
                    if !added.is_empty() {
                        for existing in this.entries.iter() {
                            added.remove(&existing.path);
                        }
                    }
                    let added: Vec<PathBuf> = added.into_iter().collect();
                    // A load or re-sort may have replaced the listing while the
                    // background merge ran; merge into the current one then.
                    let updated = if Arc::ptr_eq(&this.entries, &entries) {
                        updated
                    } else {
                        crow_fs::apply_stats(&this.entries, &stats, &this.sort, &kind)
                    };
                    this.replace_entries_after(updated, Some(&stats), cx);
                    if !this.reconcile.is_empty() || !added.is_empty() {
                        this.schedule_reconcile(added, cx);
                    }
                }
                // Changes that arrived meanwhile go next, still one at a time.
                this.apply_folder_changes(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Re-stat the paths recent watcher batches added, once the stream has been
    /// quiet for `RECONCILE_DELAY`. FSEvents coalesces events and can lose the
    /// delete that follows a rename, which would leave a row forever; the stat
    /// is off the main thread and only touches the paths that were added.
    fn schedule_reconcile(&mut self, added: Vec<PathBuf>, cx: &mut Context<Self>) {
        self.reconcile.extend(added);
        self.reconcile_gen += 1;
        self.reconciling = true;
        let generation = self.reconcile_gen;
        let (dir, hidden, sort) = (self.dir.clone(), self.show_hidden, self.sort);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(RECONCILE_DELAY).await;
            let paths = this
                .update(cx, |this, _| {
                    if this.reconcile_gen != generation {
                        return None;
                    }
                    Some(std::mem::take(&mut this.reconcile))
                })
                .ok()
                .flatten();
            let Some(paths) = paths else { return };
            if paths.is_empty() {
                this.update(cx, |this, _| {
                    if this.reconcile_gen == generation {
                        this.reconciling = false;
                    }
                })
                .ok();
                return;
            }
            let pending_paths = paths.clone();
            let entries = this.update(cx, |this, _| this.entries.clone()).ok();
            let Some(entries) = entries else { return };
            let snapshot = entries.clone();
            let (stats, updated) = cx
                .background_spawn(async move {
                    let stats = crow_fs::stat_changes(&dir, &paths, hidden);
                    // Every added path still existing means no apply; skip the
                    // merge then, since most reconciles find nothing gone.
                    let updated = stats
                        .iter()
                        .any(|(_, entry)| entry.is_none())
                        .then(|| crow_fs::apply_stats(&snapshot, &stats, &sort, &kind));
                    (stats, updated)
                })
                .await;
            this.update(cx, |this, cx| {
                if this.reconcile_gen != generation {
                    return;
                }
                this.reconciling = false;
                if this.sort != sort {
                    return;
                }
                // One apply at a time: a watcher batch or a load may be in
                // flight, and a second 100k apply in the same frame was 38+38 ms.
                // Re-stat these paths in the next batch.
                if this.applying || this.loading {
                    this.pending.extend(pending_paths);
                    return;
                }
                // Every added path still exists: the listing already has them, so
                // skip the rebuild (a second 100k-row apply per create).
                let Some(updated) = updated else { return };
                let updated = if Arc::ptr_eq(&this.entries, &entries) {
                    updated
                } else {
                    crow_fs::apply_stats(&this.entries, &stats, &this.sort, &kind)
                };
                this.replace_entries_after(updated, Some(&stats), cx);
            })
            .ok();
        })
        .detach();
    }

    // ----- selection -----

    fn set_cursor(&mut self, ix: usize, cx: &mut Context<Self>) {
        let moved = self.cursor_ix != Some(ix);
        self.cursor_ix = Some(ix);
        // Non-strict: scrolls only when the row is out of view, like Finder.
        self.scroll.scroll_to_item(ix, ScrollStrategy::Top);
        if let Some(path) = self.row(ix).map(|(entry, _)| entry.path.clone()) {
            crate::effects::effects(cx).update_quick_look(&path);
        }
        if moved {
            cx.emit(FolderEvent::CursorMoved);
        }
        cx.notify();
    }

    pub fn select_only(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some((path, totals)) = self.row(ix).map(|(entry, _)| (entry.path.clone(), Self::entry_totals(entry)))
        else {
            return;
        };
        self.selection.select_only(path);
        self.selection_totals = totals;
        self.selection_changed();
        self.set_cursor(ix, cx);
    }

    /// Click inside an existing selection: keep it (so it can be dragged), move the cursor.
    pub fn click(&mut self, ix: usize, cx: &mut Context<Self>) {
        match self.row(ix) {
            Some((entry, _)) if self.selection.contains(&entry.path) => self.set_cursor(ix, cx),
            Some(_) => self.select_only(ix, cx),
            None => {}
        }
    }

    pub fn toggle(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some((path, totals)) = self.row(ix).map(|(entry, _)| (entry.path.clone(), Self::entry_totals(entry)))
        else {
            return;
        };
        let was_selected = self.selection.contains(&path);
        self.selection.toggle(path);
        if was_selected {
            self.selection_totals.0 = self.selection_totals.0.saturating_sub(totals.0);
            self.selection_totals.1 = self.selection_totals.1.saturating_sub(totals.1);
        } else {
            self.selection_totals.0 += totals.0;
            self.selection_totals.1 += totals.1;
        }
        self.selection_changed();
        match self.selection.cursor().and_then(|cursor| self.index_of_path(cursor)) {
            Some(ix) => self.set_cursor(ix, cx),
            None => {
                self.cursor_ix = None;
                cx.emit(FolderEvent::CursorMoved);
                cx.notify();
            }
        }
    }

    pub fn extend_to(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.rows.len() || !self.rows_match_entries() {
            return;
        }
        let entries = self.entries.clone();
        let rows = self.rows.clone();
        let at = |i: usize| match &rows[i] {
            FlatRow::Own(j) => entries[*j].path.clone(),
            FlatRow::Child(entry, _) => entry.path.clone(),
        };
        let position = |path: &PathBuf| {
            rows.iter().position(|row| match row {
                FlatRow::Own(j) => &entries[*j].path == path,
                FlatRow::Child(entry, _) => &entry.path == path,
            })
        };
        let (from, to) = self.selection.extend_to(ix, position, at);
        let mut totals = (0u64, 0usize);
        for i in from..=to {
            if let Some(entry) = self.row(i).map(|(entry, _)| entry) {
                Self::add_totals(&mut totals, entry);
            }
        }
        self.selection_totals = totals;
        self.selection_changed();
        self.set_cursor(ix, cx);
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        if !self.rows_match_entries() {
            return;
        }
        let entries = self.entries.clone();
        let mut totals = (0u64, 0usize);
        // Iterate the rows in place: a clone here is a 100k-element copy on the
        // main thread, and `selection` is a disjoint field.
        let paths = self.rows.iter().map(|row| {
            let entry = match row {
                FlatRow::Own(i) => &entries[*i],
                FlatRow::Child(entry, _) => entry,
            };
            Self::add_totals(&mut totals, entry);
            entry.path.clone()
        });
        self.selection.select_all(paths);
        self.selection_totals = totals;
        self.selection_changed();
        cx.notify();
    }

    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.selection.clear();
        self.selection_totals = (0, 0);
        self.selection_changed();
        self.cursor_ix = None;
        cx.emit(FolderEvent::CursorMoved);
        cx.notify();
    }

    /// The row `delta` away from the cursor, clamped; from nothing, the first or last row.
    pub fn step(&self, delta: isize) -> usize {
        let last = self.rows.len().saturating_sub(1);
        match self.cursor_ix {
            Some(ix) => ix.saturating_add_signed(delta).min(last),
            None if delta < 0 => last,
            None => 0,
        }
    }

    /// Finder's type-to-select within this folder.
    pub fn type_char(&mut self, c: char, cx: &mut Context<Self>) {
        let prefix = self.type_select.push(c, Instant::now()).to_owned();
        // The run expires on its own; each letter's timer checks its generation,
        // so only the last run's timer clears the prefix.
        self.type_select_gen += 1;
        let generation = self.type_select_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TypeSelect::TIMEOUT).await;
            this.update(cx, |this, cx| {
                if this.type_select_gen == generation {
                    this.type_select.clear();
                    cx.emit(FolderEvent::TypeSelect);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        let entries = self.entries.clone();
        let names = self.rows.iter().filter_map(|row| match row {
            FlatRow::Own(i) => entries.get(*i).map(|entry| entry.name.as_str()),
            FlatRow::Child(entry, _) => Some(entry.name.as_str()),
        });
        if let Some(ix) = TypeSelect::find(&prefix, names) {
            self.select_only(ix, cx);
        }
        cx.emit(FolderEvent::TypeSelect);
    }

    /// Bench helper: jump the view without touching the selection.
    pub fn scroll_strict(&self, ix: usize) {
        self.scroll.scroll_to_item_strict(ix, ScrollStrategy::Top);
    }

    /// Feed a watcher batch without touching the disk (tests).
    #[cfg(test)]
    pub(crate) fn queue_changes(&self, paths: Vec<PathBuf>) {
        self.changes_tx.send(FolderChange::Paths(paths)).ok();
    }

    /// Hold apply results in flight for `delay` (tests reproduce out-of-order batches).
    #[cfg(test)]
    pub(crate) fn set_apply_delay(&mut self, delay: Duration) {
        self.apply_delay = delay;
    }

    /// How many watcher batches this folder has spawned (tests).
    #[cfg(test)]
    pub(crate) fn apply_batches(&self) -> usize {
        self.apply_batches
    }

    /// Hold re-sorts in flight for `delay` (tests reproduce a stale sort landing late).
    #[cfg(test)]
    pub(crate) fn set_sort_delay(&mut self, delay: Duration) {
        self.sort_delay = delay;
    }

    #[cfg(test)]
    pub fn names(&self) -> Vec<String> {
        self.rows
            .iter()
            .filter_map(|row| match row {
                FlatRow::Own(i) => self.entries.get(*i).map(|entry| entry.name.clone()),
                FlatRow::Child(entry, _) => Some(entry.name.clone()),
            })
            .collect()
    }

    #[cfg(test)]
    pub fn selected_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .rows
            .iter()
            .filter_map(|row| match row {
                FlatRow::Own(i) => self.entries.get(*i),
                FlatRow::Child(entry, _) => Some(entry),
            })
            .filter(|entry| self.selection.contains(&entry.path))
            .map(|entry| entry.name.clone())
            .collect();
        names.sort();
        names
    }

    /// The listing generation, for the fuzzer's cursor invariant message.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn generation_for_test(&self) -> u64 {
        self.generation
    }

    /// The row generation, for the fuzzer's cursor invariant message.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn rows_generation_for_test(&self) -> u64 {
        self.rows_generation
    }

    /// The selection's cursor path, for the fuzzer's cursor invariant.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn selection_cursor_path(&self) -> Option<PathBuf> {
        self.selection.cursor().cloned()
    }

    /// The selected paths, for the fuzzer's selection invariant.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn selection_paths(&self) -> Vec<PathBuf> {
        self.selection.iter().cloned().collect()
    }

    /// Every row's path, including inline child rows; the selection may name one.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn row_paths(&self) -> Vec<PathBuf> {
        self.rows
            .iter()
            .filter_map(|row| match row {
                FlatRow::Own(i) => self.entries.get(*i).map(|entry| entry.path.clone()),
                FlatRow::Child(entry, _) => Some(entry.path.clone()),
            })
            .collect()
    }

    /// Whether `path` is one of the visible rows, including an inline child.
    /// No allocation: the fuzz runner uses it for a small selection.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn has_row_path(&self, path: &std::path::Path) -> bool {
        self.rows.iter().any(|row| match row {
            FlatRow::Own(i) => self.entries.get(*i).is_some_and(|entry| entry.path == path),
            FlatRow::Child(entry, _) => entry.path == path,
        })
    }

    /// Break the cursor on purpose, to prove the invariant fires.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn set_cursor_ix_for_test(&mut self, ix: Option<usize>) {
        self.cursor_ix = ix;
    }

    /// Time a synthetic listing apply without a load. `stats` is a watcher
    /// batch; `None` is a full reload, which prunes against every row.
    #[cfg(test)]
    pub fn replace_entries_for_test(
        &mut self,
        entries: Vec<Entry>,
        stats: Option<&[(PathBuf, Option<Entry>)]>,
        cx: &mut Context<Self>,
    ) {
        // The apply's row build normally runs on the background executor; a test
        // wants the finished rows to assert on.
        self.replace_entries_after_inner(entries, stats, cx, true);
    }

    /// Apply a staged listing's partial, as the load loop does.
    #[cfg(test)]
    pub fn apply_partial_for_test(&mut self, entries: Vec<Entry>, cx: &mut Context<Self>) {
        self.apply_partial_inner(entries, cx, true);
    }

    /// The open inline child at `path`, so a test can hold it across a collapse.
    #[cfg(test)]
    pub fn child_for_test(&self, path: &Path) -> Option<Entity<Folder>> {
        self.children.get(path).map(|child| child.folder.clone())
    }

    /// Set the operation results to select without starting a reload.
    #[cfg(test)]
    pub fn select_when_present_for_test(&mut self, paths: Vec<PathBuf>, mode: Effective) {
        self.select_when_present = paths;
        self.arrival_mode = mode;
    }

    /// Select a path that is not in the listing, to prove the invariant fires.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn select_path_for_test(&mut self, path: PathBuf) {
        self.selection.select_only(path);
    }

    /// Drop a listed entry without touching disk, to prove the listing check fires.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn drop_entry_for_test(&mut self, name: &str, cx: &gpui::App) {
        let entries: Vec<Entry> = self.entries.iter().filter(|entry| entry.name != name).cloned().collect();
        self.entries = Arc::new(entries);
        self.rebuild_rows(cx);
    }
}

/// Case-insensitive substring, with an ASCII fast path so filtering 100k rows
/// does not allocate a lowercase copy of every name.
fn name_matches(name: &str, query: &str, lower_query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    if name.is_ascii() {
        // An ASCII name can only contain an ASCII query after lowercasing.
        // A non-ASCII query that lowercases to ASCII (the Kelvin sign) still
        // matches; one that stays non-ASCII cannot occur in an ASCII name.
        if !lower_query.is_ascii() {
            return false;
        }
        let needle = lower_query.as_bytes();
        name.as_bytes().windows(needle.len()).any(|window| window.eq_ignore_ascii_case(needle))
    } else {
        name.to_lowercase().contains(lower_query)
    }
}

/// Where `query` first matches `name`, as byte offsets into `name`. `None` means no
/// match, or (for the rare non-ASCII cases where char-wise lowercasing differs from
/// `str::to_lowercase`, e.g. the Greek final sigma) a match the predicate found but
/// that cannot be mapped back; the row then shows without a highlight.
fn name_match(name: &str, query: &str, lower_query: &str) -> Option<Range<usize>> {
    if query.is_empty() {
        return Some(0..0);
    }
    if name.is_ascii() {
        // See `name_matches`: a non-ASCII lowered query cannot occur in an
        // ASCII name, and the slow path would lowercase the name for nothing.
        if !lower_query.is_ascii() {
            return None;
        }
        let needle = lower_query.as_bytes();
        return name
            .as_bytes()
            .windows(needle.len())
            .position(|window| window.eq_ignore_ascii_case(needle))
            .map(|start| start..start + needle.len());
    }
    // Lowercase char by char, remembering where each lowered char came from, so the
    // found range maps back to the original name's byte offsets.
    let mut lowered = String::with_capacity(name.len());
    // (lowered start, original start, original end) for each lowered char. One
    // original char can lower to several, e.g. İ -> i + combining dot.
    let mut starts: Vec<(usize, usize, usize)> = Vec::with_capacity(name.len());
    for (byte, ch) in name.char_indices() {
        let end = byte + ch.len_utf8();
        for lower in ch.to_lowercase() {
            starts.push((lowered.len(), byte, end));
            lowered.push(lower);
        }
    }
    let start_lowered = lowered.find(lower_query)?;
    let end_lowered = start_lowered + lower_query.len();
    // A range inside one original char's expansion (or spanning one) widens to
    // whole chars, which keeps both ends on char boundaries.
    let start = starts.iter().rev().find(|(at, _, _)| *at <= start_lowered).map(|(_, byte, _)| *byte)?;
    let end = if end_lowered == lowered.len() {
        name.len()
    } else {
        starts.iter().rev().find(|(at, _, _)| *at < end_lowered).map(|(_, _, byte)| *byte)?
    };
    (start < end && name.is_char_boundary(start) && name.is_char_boundary(end)).then_some(start..end)
}

/// Runs on a background thread: list `dir`, sending sorted partial listings at most every
/// 50 ms, then the full sorted listing.
fn list_in_stages(dir: &Path, hidden: bool, sort: Sort, started: Instant, tx: &UnboundedSender<Load>) {
    let mut all: Vec<Entry> = Vec::new();
    let mut last_partial = started;
    let result = crow_fs::list_dir(dir, hidden, |batch| {
        all.extend(batch);
        if last_partial.elapsed() >= PARTIAL_EVERY || (last_partial == started && !all.is_empty()) {
            last_partial = Instant::now();
            let mut partial = all.clone();
            crow_fs::sort_entries(&mut partial, &sort, &kind);
            // The folder may have been closed; nobody listening is fine.
            tx.unbounded_send(Load::Partial(partial)).ok();
        }
    });
    let done = result.map(|()| {
        crow_fs::sort_entries(&mut all, &sort, &kind);
        (all, started.elapsed().as_secs_f64() * 1000.0)
    });
    tx.unbounded_send(Load::Done(done)).ok();
}

/// Patch folder rows with the size service's known bytes; an unknown folder is
/// 0, so it sorts before known folders in ascending order (Q3's "once" order).
fn apply_folder_sizes(entries: &mut [Entry], sizes: &HashMap<PathBuf, u64>) {
    for entry in entries {
        if entry.is_dir && !entry.is_symlink {
            entry.size = sizes.get(&entry.path).copied().unwrap_or(0);
        }
    }
}

/// Flatten `entries` and the open children into the rows the list view draws.
/// Free of entities so an apply can run it on the background executor.
fn build_rows(entries: &[Entry], children: &ChildRows, filter: Option<(&str, &str)>) -> Rows {
    if let Some((filter, lower)) = filter {
        let mut rows = Vec::new();
        let mut ranges = Vec::new();
        for (i, entry) in entries.iter().enumerate() {
            let range = name_match(&entry.name, filter, lower);
            // The predicate keeps today's shown set. `name_match` only adds the
            // highlight, and can fail where full-string lowercasing differs.
            let shown = if entry.name.is_ascii() && filter.is_ascii() {
                range.is_some()
            } else {
                name_matches(&entry.name, filter, lower)
            };
            if !shown {
                continue;
            }
            rows.push(FlatRow::Own(i));
            ranges.push(range.map_or((u32::MAX, u32::MAX), |r| (r.start as u32, r.end as u32)));
        }
        return Rows {
            row_ends: Arc::new((1..=rows.len()).collect()),
            row_parents: Arc::new(vec![None; rows.len()]),
            rows: Arc::new(rows),
            match_ranges: Arc::new(ranges),
        };
    }
    let mut rows = Vec::with_capacity(entries.len());
    for (i, entry) in entries.iter().enumerate() {
        rows.push(FlatRow::Own(i));
        if !entry.is_dir || entry.is_package() {
            continue;
        }
        let Some((child_rows, child_entries)) = children.get(&entry.path) else { continue };
        for row in child_rows.iter() {
            match row {
                FlatRow::Own(j) => {
                    if let Some(entry) = child_entries.get(*j) {
                        rows.push(FlatRow::Child(entry.clone(), 1));
                    }
                }
                FlatRow::Child(entry, depth) if *depth < MAX_DEPTH => {
                    rows.push(FlatRow::Child(entry.clone(), depth + 1));
                }
                FlatRow::Child(..) => {}
            }
        }
    }
    let mut ends = vec![rows.len(); rows.len()];
    let mut parents = vec![None; rows.len()];
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let mut last_at_depth: Vec<usize> = Vec::new();
    for (ix, row) in rows.iter().enumerate() {
        let depth = match row {
            FlatRow::Own(_) => 0,
            FlatRow::Child(_, depth) => *depth,
        };
        if depth > 0 {
            parents[ix] = last_at_depth.get(depth - 1).copied();
        }
        if last_at_depth.len() > depth {
            last_at_depth.truncate(depth);
        }
        last_at_depth.push(ix);
        while let Some(&(top, top_depth)) = stack.last() {
            if depth <= top_depth {
                ends[top] = ix;
                stack.pop();
            } else {
                break;
            }
        }
        stack.push((ix, depth));
    }
    Rows {
        rows: Arc::new(rows),
        row_ends: Arc::new(ends),
        row_parents: Arc::new(parents),
        match_ranges: Arc::new(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, size: u64, is_dir: bool) -> Entry {
        Entry {
            name: name.to_owned(),
            path: PathBuf::from("/").join(name),
            is_dir,
            is_symlink: false,
            hidden: false,
            size,
            modified: None,
            created: None,
            added: None,
        }
    }

    #[test]
    fn size_sort_uses_known_unknown_and_stale_folders() {
        let mut entries = vec![
            entry("a", 0, true),
            entry("b", 0, true),
            entry("c", 0, true),
            entry("d", 0, true),
            entry("e", 5, false),
        ];
        // a, b and d are known; c is unknown (0); d's value is stale but still known.
        let sizes = HashMap::from([(PathBuf::from("/a"), 100), (PathBuf::from("/b"), 10), (PathBuf::from("/d"), 50)]);
        apply_folder_sizes(&mut entries, &sizes);
        let sort = Sort { key: crow_fs::SortKey::Size, ascending: true, folders_first: true };
        crow_fs::sort_entries(&mut entries, &sort, &kind);
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["c", "b", "d", "a", "e"], "unknown 0, 10, 50, 100, then the file");
    }

    #[test]
    fn name_match_finds_the_first_ascii_occurrence() {
        assert_eq!(name_match("View_vi", "VI", "vi"), Some(0..2));
        assert_eq!(name_match("alphabet", "ph", "ph"), Some(2..4));
        assert_eq!(name_match("alpha", "zz", "zz"), None);
        assert_eq!(name_match("alpha", "ALPHA", "alpha"), Some(0..5));
        assert_eq!(name_match("alpha", "alpha beta", "alpha beta"), None);
    }

    #[test]
    fn name_match_maps_non_ascii_to_original_bytes() {
        assert_eq!(name_match("Ärger.txt", "rg", "rg"), Some(2..4));
        assert_eq!(name_match("Ärger.txt", "ÄR", "är"), Some(0..3));
        // A query that full-string lowercasing also misses.
        assert_eq!(name_match("Grüße", "ÜSS", "üss"), None);
        assert!(!name_matches("Grüße", "ÜSS", "üss"));
        // The char-wise map can't express the final sigma; the row still shows.
        assert!(name_matches("ΣΊΣΥΦΟΣ", "ς", "ς"));
        assert_eq!(name_match("ΣΊΣΥΦΟΣ", "ς", "ς"), None);
        // Combining marks and expansions: the range stays on char boundaries.
        assert_eq!(name_match("e\u{301}clair", "e", "e"), Some(0..1));
        assert_eq!(name_match("İstanbul", "i", "i"), Some(0..2));
    }

    #[test]
    fn name_match_agrees_with_name_matches() {
        let corpus = ["alpha", "Ärger.txt", "Grüße", "ΣΊΣΥΦΟΣ", "İstanbul", "e\u{301}clair", "café", "STRASSE"];
        let queries = ["a", "ä", "rg", "ss", "σ", "ς", "i", "e", "café", "ÜSS", "zzz"];
        for name in corpus {
            for query in queries {
                let lower = query.to_lowercase();
                if name_matches(name, query, &lower) {
                    // The known gap: final sigma (and other context-sensitive lowercasing).
                    let known = name == "ΣΊΣΥΦΟΣ" && query == "ς";
                    assert!(
                        name_match(name, query, &lower).is_some() || known,
                        "{name:?} shown for {query:?} without a range"
                    );
                } else {
                    assert_eq!(name_match(name, query, &lower), None, "{name:?} highlighted for {query:?}");
                }
            }
        }
    }

    #[test]
    fn a_non_ascii_query_skips_ascii_names_without_lowercasing() {
        // The old path lowercased every name (and built the char map) for a
        // non-ASCII query; 100k names made a filter keystroke a ~100 ms
        // main-thread stall (findings #22).
        let names: Vec<String> = (0..100_000).map(|i| format!("file-{i:06}.txt")).collect();
        let slow_match = |name: &str| {
            let mut lowered = String::with_capacity(name.len());
            let mut starts: Vec<(usize, usize, usize)> = Vec::with_capacity(name.len());
            for (byte, ch) in name.char_indices() {
                let end = byte + ch.len_utf8();
                for lower in ch.to_lowercase() {
                    starts.push((lowered.len(), byte, end));
                    lowered.push(lower);
                }
            }
            lowered.find("م")
        };
        let started = Instant::now();
        for name in &names {
            assert!(!name_matches(name, "م", "م"));
            assert_eq!(name_match(name, "م", "م"), None);
        }
        let fast = started.elapsed();
        let started = Instant::now();
        for name in &names {
            assert_eq!(slow_match(name), None);
        }
        let slow = started.elapsed();
        assert!(fast * 2 < slow, "the ASCII path took {fast:?}, the old match path {slow:?}");
        // A non-ASCII query that lowercases to ASCII still matches.
        assert!(name_matches("k", "\u{212A}", "k"), "the Kelvin sign lowercases to k");
        assert_eq!(name_match("k", "\u{212A}", "k"), Some(0..1));
    }
}
