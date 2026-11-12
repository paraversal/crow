//! One open folder: its listing, sort, selection and live updates. The list view
//! shows one; the column view shows a chain of them.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crow_core::{Selection, TypeSelect};
use crow_fs::{Entry, FolderChange, FolderWatcher, Sort};
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{Context, EventEmitter, ScrollStrategy, SharedString, Task, UniformListScrollHandle, prelude::*};

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
    scroll: UniformListScrollHandle,
    changes_rx: Receiver<FolderChange>,
    _watcher: Option<FolderWatcher>,
    error: Option<SharedString>,
    _poll: Task<()>,
}

impl EventEmitter<FolderEvent> for Folder {}

impl Folder {
    pub fn new(dir: PathBuf, show_hidden: bool, sort: Sort, auto_select: bool, cx: &mut Context<Self>) -> Self {
        let (changes_tx, changes_rx) = mpsc::channel();
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
            scroll: UniformListScrollHandle::new(),
            changes_rx,
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
        self.cursor_ix.and_then(|i| self.entries.get(i))
    }

    pub fn is_selected(&self, path: &Path) -> bool {
        self.selection.contains(&path.to_path_buf())
    }

    pub fn selected_count(&self) -> usize {
        self.selection.len()
    }

    /// The selected entries, in display order.
    pub fn selected_paths(&self) -> Vec<PathBuf> {
        self.entries.iter().filter(|e| self.selection.contains(&e.path)).map(|e| e.path.clone()).collect()
    }

    pub fn type_select_active(&self) -> bool {
        self.type_select.is_active(Instant::now())
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
        let present: HashSet<&PathBuf> = self.entries.iter().map(|e| &e.path).collect();
        self.selection.retain(|p| present.contains(p));
        if let Some(target) = self.select_after_load.clone()
            && present.contains(&target)
        {
            self.select_after_load = None;
            self.selection.select_only(target);
        }
        if !self.select_when_present.is_empty() && self.select_when_present.iter().all(|p| present.contains(p)) {
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
        let previous = self.cursor_entry().map(|e| e.path.clone());
        self.cursor_ix = self.selection.cursor().and_then(|c| self.entries.iter().position(|e| &e.path == c));
        let now = self.cursor_entry().map(|e| e.path.clone());
        if now != previous {
            if let Some(ix) = self.cursor_ix {
                self.scroll.scroll_to_item(ix, ScrollStrategy::Center);
            }
            cx.emit(FolderEvent::CursorMoved);
        }
        cx.notify();
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
        let (entries, dir, hidden, sort, generation) =
            (self.entries.clone(), self.dir.clone(), self.show_hidden, self.sort, self.generation);
        cx.spawn(async move |this, cx| {
            let updated = cx
                .background_spawn(async move { crow_fs::apply_changes(&entries, &dir, &paths, hidden, &sort, &kind) })
                .await;
            this.update(cx, |this, cx| {
                if this.generation == generation && this.sort == sort {
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
        if let Some(entry) = self.entries.get(ix) {
            crow_macos::update_quick_look(&entry.path);
        }
        if moved {
            cx.emit(FolderEvent::CursorMoved);
        }
        cx.notify();
    }

    pub fn select_only(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(entry) = self.entries.get(ix) else { return };
        self.selection.select_only(entry.path.clone());
        self.set_cursor(ix, cx);
    }

    /// Click inside an existing selection: keep it (so it can be dragged), move the cursor.
    pub fn click(&mut self, ix: usize, cx: &mut Context<Self>) {
        match self.entries.get(ix) {
            Some(e) if self.selection.contains(&e.path) => self.set_cursor(ix, cx),
            Some(_) => self.select_only(ix, cx),
            None => {}
        }
    }

    pub fn toggle(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(entry) = self.entries.get(ix) else { return };
        self.selection.toggle(entry.path.clone());
        match self.selection.cursor().and_then(|c| self.entries.iter().position(|e| &e.path == c)) {
            Some(ix) => self.set_cursor(ix, cx),
            None => {
                self.cursor_ix = None;
                cx.emit(FolderEvent::CursorMoved);
                cx.notify();
            }
        }
    }

    pub fn extend_to(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.entries.len() {
            return;
        }
        let entries = self.entries.clone();
        self.selection.extend_to(ix, |p| entries.iter().position(|e| &e.path == p), |i| entries[i].path.clone());
        self.set_cursor(ix, cx);
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.selection.select_all(self.entries.iter().map(|e| e.path.clone()));
        cx.notify();
    }

    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.selection.clear();
        self.cursor_ix = None;
        cx.emit(FolderEvent::CursorMoved);
        cx.notify();
    }

    /// The row `delta` away from the cursor, clamped; from nothing, the first or last row.
    pub fn step(&self, delta: isize) -> usize {
        let last = self.entries.len().saturating_sub(1);
        match self.cursor_ix {
            Some(ix) => ix.saturating_add_signed(delta).min(last),
            None if delta < 0 => last,
            None => 0,
        }
    }

    /// Finder's type-to-select within this folder.
    pub fn type_char(&mut self, c: char, cx: &mut Context<Self>) {
        let prefix = self.type_select.push(c, Instant::now()).to_owned();
        if let Some(ix) = TypeSelect::find(&prefix, self.entries.iter().map(|e| e.name.as_str())) {
            self.select_only(ix, cx);
        }
    }

    /// Bench helper: jump the view without touching the selection.
    pub fn scroll_strict(&self, ix: usize) {
        self.scroll.scroll_to_item_strict(ix, ScrollStrategy::Top);
    }

    #[cfg(test)]
    pub fn names(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.name.clone()).collect()
    }

    #[cfg(test)]
    pub fn selected_names(&self) -> Vec<String> {
        let mut names: Vec<String> =
            self.entries.iter().filter(|e| self.selection.contains(&e.path)).map(|e| e.name.clone()).collect();
        names.sort();
        names
    }
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
