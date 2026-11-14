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

use crate::style::kind;

/// Show what has been read so far at most this often while a big folder loads.
const PARTIAL_EVERY: Duration = Duration::from_millis(50);
/// More changes than this at once: relisting is cheaper than diffing.
const MAX_DIFF: usize = 500;

pub enum FolderEvent {
    /// The cursor moved to another item (or to none).
    CursorMoved,
    /// A listing finished.
    Loaded,
    /// The visible rows changed: a listing, an expansion or a child folder.
    Changed,
}

/// The deepest folder list view expands inline; stops symlink cycles.
const MAX_DEPTH: usize = 32;

/// The selected rows as (path, is a folder), shared with drag values.
type SelectionEntries = Arc<Vec<(PathBuf, bool)>>;

/// The cached selection: the generation it was built for, and the entries.
type SelectionCache = RefCell<Option<(u64, SelectionEntries)>>;

/// The cached selection's file bytes and folder count, by generation.
type SelectionBytes = RefCell<Option<(u64, (u64, usize))>>;

/// One visible row: an entry of this folder, or one of an expanded folder under it.
enum FlatRow {
    /// Index into `entries`.
    Own(usize),
    /// An entry of a child folder and its depth under this one.
    Child(Entry, usize),
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
    /// After going up or back, Finder selects the folder you came from.
    select_after_load: Option<PathBuf>,
    /// Items an operation produced: selected as soon as they appear in the listing.
    select_when_present: Vec<PathBuf>,
    /// Select the first item once loaded. Off for column-view children, which show
    /// a folder's contents without selecting anything until you move into them.
    auto_select: bool,
    type_select: TypeSelect,
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
    /// Bumped whenever the selection or the visible rows change.
    selection_generation: u64,
    /// Feedback effects on visible rows (trash, refuse), cleared when their phases end.
    effects: Vec<RowEffect>,
    /// Bumped per effect batch, so an older timer cannot clear a newer effect.
    effect_generation: u64,
    /// The selected rows for dragging, rebuilt only after one of those changes.
    selection_entries: SelectionCache,
    /// The selected files' bytes and folders among them, cached the same way.
    selection_bytes: SelectionBytes,
    scroll: UniformListScrollHandle,
    changes_rx: Receiver<FolderChange>,
    /// A sender kept only for tests, which queue watcher batches without the disk.
    #[cfg(test)]
    changes_tx: Sender<FolderChange>,
    /// Tests hold an apply in flight to reproduce out-of-order batches.
    #[cfg(test)]
    apply_delay: Duration,
    _watcher: Option<FolderWatcher>,
    error: Option<SharedString>,
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
            select_after_load: None,
            select_when_present: Vec::new(),
            auto_select,
            type_select: TypeSelect::default(),
            show_hidden,
            filter: None,
            match_ranges: Arc::new(Vec::new()),
            children: HashMap::new(),
            expanded: HashSet::new(),
            rows: Arc::new(Vec::new()),
            selection_generation: 0,
            effects: Vec::new(),
            effect_generation: 0,
            selection_entries: RefCell::new(None),
            selection_bytes: RefCell::new(None),
            scroll: UniformListScrollHandle::new(),
            changes_rx,
            #[cfg(test)]
            changes_tx: test_tx,
            #[cfg(test)]
            apply_delay: Duration::ZERO,
            _watcher: watcher,
            error: None,
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

    /// True while this folder or any folder open inside it is still listing.
    pub fn is_loading_deep(&self, cx: &gpui::App) -> bool {
        self.is_loading() || self.children.values().any(|child| child.folder.read(cx).is_loading_deep(cx))
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
        let (_, depth) = self.row(ix)?;
        (0..ix).rev().find(|row| self.row(*row).is_some_and(|(_, d)| d + 1 == depth))
    }

    fn row_path<'a>(row: &'a FlatRow, entries: &'a [Entry]) -> Option<&'a Path> {
        match row {
            FlatRow::Own(i) => entries.get(*i).map(|entry| entry.path.as_path()),
            FlatRow::Child(entry, _) => Some(entry.path.as_path()),
        }
    }

    fn index_of_path(&self, path: &Path) -> Option<usize> {
        self.rows.iter().position(|row| Self::row_path(row, &self.entries).is_some_and(|p| p == path))
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
    /// Folders add no bytes: their size needs a disk walk. Cached per generation.
    pub fn selected_bytes(&self) -> (u64, usize) {
        if let Some((generation, bytes)) = *self.selection_bytes.borrow()
            && generation == self.selection_generation
        {
            return bytes;
        }
        let (mut bytes, mut folders) = (0u64, 0usize);
        for row in self.rows.iter() {
            let entry = match row {
                FlatRow::Own(i) => self.entries.get(*i),
                FlatRow::Child(entry, _) => Some(entry),
            };
            let Some(entry) = entry.filter(|entry| self.selection.contains(&entry.path)) else { continue };
            if entry.is_dir {
                folders += 1;
            } else {
                bytes += entry.size;
            }
        }
        *self.selection_bytes.borrow_mut() = Some((self.selection_generation, (bytes, folders)));
        (bytes, folders)
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
    pub fn select_when_present(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        self.select_when_present = paths;
        self.reload(cx);
    }

    // ----- feedback effects -----

    /// Start `fx` on the visible rows among `paths` (at most 20), for `mode`'s
    /// duration. `off` starts nothing: the state changes in one frame.
    pub fn start_effect(&mut self, paths: &[PathBuf], fx: Fx, mode: Effective, cx: &mut Context<Self>) {
        let phases = motion::phases(mode, fx);
        let total: Duration = phases.iter().sum();
        if total.is_zero() {
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
        self.effect_generation += 1;
        let generation = self.effect_generation;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(total + WATCH_GRACE).await;
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

    /// How the row at `path` looks right now, if an effect is playing on it.
    pub fn effect_look(&self, path: &Path, mode: Effective, now: Instant) -> Option<Look> {
        self.effects.iter().find(|effect| effect.path == path).map(|effect| effect.look(mode, now))
    }

    pub fn effects(&self) -> &[RowEffect] {
        &self.effects
    }

    /// How far up the rows below a removed row have slid (Full motion's FLIP close).
    /// `ix` is the row's index in the listing as it is now; the shift starts at one
    /// row and eases to zero, so the row begins where it was before the removal.
    pub fn row_shift(&self, ix: usize, mode: Effective, now: Instant, row_height: f32) -> f32 {
        if mode != Effective::Full {
            return 0.0;
        }
        let mut removed: Vec<&RowEffect> =
            self.effects.iter().filter(|effect| effect.removed && effect.fx.leaving()).collect();
        if removed.is_empty() {
            return 0.0;
        }
        removed.sort_by_key(|effect| effect.row_index);
        // The row's index before the removals: each removed row at or above it adds one.
        let mut original = ix;
        for effect in &removed {
            if effect.row_index <= original {
                original += 1;
            }
        }
        let mut shift = 0.0;
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
        if self.sort == sort {
            return;
        }
        self.sort = sort;
        let mut entries = self.entries.as_ref().clone();
        cx.spawn(async move |this, cx| {
            let entries = cx
                .background_spawn(async move {
                    crow_fs::sort_entries(&mut entries, &sort, &kind);
                    entries
                })
                .await;
            this.update(cx, |this, cx| {
                if this.sort == sort {
                    this.replace_entries(entries, cx);
                    let children: Vec<Entity<Folder>> = this.children.values().map(|c| c.folder.clone()).collect();
                    for child in children {
                        child.update(cx, |child, cx| child.set_sort(sort, cx));
                    }
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
        self.loading = true;
        let (generation, dir, hidden, sort) = (self.generation, self.dir.clone(), self.show_hidden, self.sort);
        let (tx, mut rx) = unbounded();
        let started = Instant::now();
        cx.background_spawn(async move { list_in_stages(&dir, hidden, sort, started, &tx) }).detach();
        cx.spawn(async move |this, cx| {
            while let Some(update) = rx.next().await {
                let current = this.update(cx, |this, cx| {
                    if this.generation != generation {
                        return false;
                    }
                    if this.first_paint_ms.is_none() {
                        this.first_paint_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
                    }
                    match update {
                        Load::Partial(entries) => this.replace_entries(entries, cx),
                        Load::Done(Ok((entries, ms))) => {
                            this.loading = false;
                            this.load_ms = ms;
                            tracing::debug!(items = entries.len(), ms, dir = %this.dir.display(), "loaded");
                            this.replace_entries(entries, cx);
                            cx.emit(FolderEvent::Loaded);
                        }
                        Load::Done(Err(e)) => {
                            this.loading = false;
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
        self.entries = Arc::new(entries);
        if !self.effects.is_empty() {
            let present: HashSet<PathBuf> = self.entries.iter().map(|entry| entry.path.clone()).collect();
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
        self.rebuild_rows(cx);
        if let Some(target) = self.select_after_load.clone()
            && self.entries.iter().any(|e| e.path == target)
        {
            self.select_after_load = None;
            self.selection.select_only(target);
        }
        if !self.select_when_present.is_empty()
            && self.select_when_present.iter().all(|p| self.entries.iter().any(|e| &e.path == p))
        {
            let wanted = std::mem::take(&mut self.select_when_present);
            self.selection.clear();
            for path in wanted {
                self.selection.toggle(path);
            }
        }
        if self.auto_select
            && self.selection.is_empty()
            && !self.loading
            && let Some(first) = self.entries.first()
        {
            self.selection.select_only(first.path.clone());
        }
        self.prune_selection();
        self.settle_cursor(cx);
        self.selection_changed();
        cx.emit(FolderEvent::Changed);
        cx.notify();
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
        if let Some(filter) = &self.filter {
            let lower = filter.to_lowercase();
            let mut rows = Vec::new();
            let mut ranges = Vec::new();
            for (i, entry) in self.entries.iter().enumerate() {
                let range = name_match(&entry.name, filter, &lower);
                // The predicate keeps today's shown set. `name_match` only adds the
                // highlight, and can fail where full-string lowercasing differs.
                let shown = if entry.name.is_ascii() && filter.is_ascii() {
                    range.is_some()
                } else {
                    name_matches(&entry.name, filter, &lower)
                };
                if !shown {
                    continue;
                }
                rows.push(FlatRow::Own(i));
                ranges.push(range.map_or((u32::MAX, u32::MAX), |r| (r.start as u32, r.end as u32)));
            }
            self.rows = Arc::new(rows);
            self.match_ranges = Arc::new(ranges);
            return;
        }
        self.match_ranges = Arc::new(Vec::new());
        let mut rows = Vec::with_capacity(self.entries.len());
        for i in 0..self.entries.len() {
            rows.push(FlatRow::Own(i));
            let entry = &self.entries[i];
            if !entry.is_dir || entry.is_package() {
                continue;
            }
            let Some(child) = self.children.get(&entry.path) else { continue };
            let child = child.folder.read(cx);
            for row in child.rows.iter() {
                match row {
                    FlatRow::Own(j) => {
                        if let Some(entry) = child.entries.get(*j) {
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
        self.rows = Arc::new(rows);
    }

    /// Drop selected paths that are no longer visible anywhere in the tree.
    fn prune_selection(&mut self) {
        let entries = self.entries.clone();
        let rows = self.rows.clone();
        let present: HashSet<&Path> = rows.iter().filter_map(|row| Self::row_path(row, &entries)).collect();
        let before = self.selection.len();
        self.selection.retain(|path| present.contains(path.as_path()));
        if self.selection.len() != before {
            self.selection_changed();
        }
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
        self.prune_selection();
        self.settle_cursor(cx);
        cx.emit(FolderEvent::Changed);
        cx.notify();
    }

    /// List view: show `path`'s contents inline. A row nested under an open
    /// folder is opened by the folder that lists it.
    pub fn expand(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !self.entries.iter().any(|entry| entry.path == path && entry.is_dir && !entry.is_package()) {
            if let Some(owner) = self.owner_of(&path, cx) {
                owner.update(cx, |owner, cx| owner.expand(path, cx));
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
        if !self.expanded.remove(path) {
            if let Some(owner) = self.owner_of(path, cx) {
                let path = path.to_path_buf();
                owner.update(cx, |owner, cx| owner.collapse(&path, cx));
            }
            return;
        }
        self.children.remove(path);
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
        let mut paths = Vec::new();
        let mut rescan = false;
        for change in self.changes_rx.try_iter() {
            match change {
                FolderChange::Paths(p) => paths.extend(p),
                FolderChange::Rescan => rescan = true,
            }
        }
        if paths.is_empty() && !rescan {
            return;
        }
        if rescan || self.loading || paths.len() > MAX_DIFF {
            self.load(cx);
            return;
        }
        let (dir, hidden, sort, generation) = (self.dir.clone(), self.show_hidden, self.sort, self.generation);
        #[cfg(test)]
        let delay = self.apply_delay;
        cx.spawn(async move |this, cx| {
            // Stat off the main thread; merge into the listing *as it is when the
            // results land*. A batch that lands late therefore cannot replay the
            // older listing it was computed from (which is how deleted files came back).
            let stats = cx.background_spawn(async move { crow_fs::stat_changes(&dir, &paths, hidden) }).await;
            #[cfg(test)]
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            this.update(cx, |this, cx| {
                if this.generation == generation && this.sort == sort {
                    let updated = crow_fs::apply_stats(&this.entries, &stats, &this.sort, &kind);
                    this.replace_entries(updated, cx);
                }
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
            crow_macos::update_quick_look(&path);
        }
        if moved {
            cx.emit(FolderEvent::CursorMoved);
        }
        cx.notify();
    }

    pub fn select_only(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(path) = self.row(ix).map(|(entry, _)| entry.path.clone()) else { return };
        self.selection.select_only(path);
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
        let Some(path) = self.row(ix).map(|(entry, _)| entry.path.clone()) else { return };
        self.selection.toggle(path);
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
        if ix >= self.rows.len() {
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
        self.selection.extend_to(ix, position, at);
        self.selection_changed();
        self.set_cursor(ix, cx);
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        let entries = self.entries.clone();
        let paths = self.rows.iter().map(|row| match row {
            FlatRow::Own(i) => entries[*i].path.clone(),
            FlatRow::Child(entry, _) => entry.path.clone(),
        });
        self.selection.select_all(paths);
        self.selection_changed();
        cx.notify();
    }

    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.selection.clear();
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
        let entries = self.entries.clone();
        let names = self.rows.iter().map(|row| match row {
            FlatRow::Own(i) => entries[*i].name.as_str(),
            FlatRow::Child(entry, _) => entry.name.as_str(),
        });
        if let Some(ix) = TypeSelect::find(&prefix, names) {
            self.select_only(ix, cx);
        }
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

    #[cfg(test)]
    pub fn names(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|row| match row {
                FlatRow::Own(i) => self.entries[*i].name.clone(),
                FlatRow::Child(entry, _) => entry.name.clone(),
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
}

/// Case-insensitive substring, with an ASCII fast path so filtering 100k rows
/// does not allocate a lowercase copy of every name.
fn name_matches(name: &str, query: &str, lower_query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    if name.is_ascii() && query.is_ascii() {
        let needle = query.as_bytes();
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
    if name.is_ascii() && query.is_ascii() {
        let needle = query.as_bytes();
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
