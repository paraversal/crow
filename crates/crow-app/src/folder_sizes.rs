//! Lazy folder sizes: the visible-row scheduler, two low-QoS workers, the disk
//! cache and the FSEvents history that keeps it honest.
//!
//! Panes report the folder rows their `uniform_list` shows; the scheduler keeps
//! the union of every pane's ranges, cancels rows that scrolled away and asks for
//! the rest nearest-the-top first. Measured sizes go into the cache and back to
//! the main thread through a channel; nothing here measures on the main thread.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crow_core::{FolderSizesMode, KnownSize};
use crow_fs::sizes::{Cache, Cancel, MAX_ENTRIES, Measured};
use crow_macos::{Event, FsEvents};
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui::{App, AppContext as _, Context, Entity, Global};

use crate::ui::Ui;

/// FSEvents coalesces for a second; the app does not need faster.
pub const FSEVENTS_LATENCY: Duration = Duration::from_secs(1);
/// Wait this long after the last change before writing the cache.
pub const SAVE_DEBOUNCE: Duration = Duration::from_secs(2);
/// A steady stream of FSEvents (any activity on the boot volume signals a save)
/// must not postpone the write forever: save at least this often.
pub const SAVE_MAX_DELAY: Duration = Duration::from_secs(30);
/// A pane that stops reporting (a closed window) stops keeping its rows alive.
const SOURCE_TTL: Duration = Duration::from_secs(5);
const WORKERS: usize = 2;

/// A pane's rows: `(source id, 0)` for its list, `(source id, k + 1)` for column k.
pub type SourceKey = (u64, usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SizeView {
    pub bytes: u64,
    pub stale: bool,
    pub version: u64,
}

impl From<SizeView> for KnownSize {
    fn from(size: SizeView) -> Self {
        KnownSize { bytes: size.bytes, stale: size.stale, version: size.version }
    }
}

struct Request {
    path: PathBuf,
    cancel: Cancel,
    generation: u64,
}

struct Outcome {
    path: PathBuf,
    generation: u64,
    measured: Option<Measured>,
}

pub(crate) trait Sizer: Send + Sync {
    fn measure(&self, path: &Path, cancel: &Cancel) -> Option<Measured>;
}

struct RealSizer;

impl Sizer for RealSizer {
    fn measure(&self, path: &Path, cancel: &Cancel) -> Option<Measured> {
        crow_fs::sizes::measure(path, cancel)
    }
}

struct Source {
    paths: Vec<PathBuf>,
    seen: Instant,
}

#[derive(Default)]
struct Scheduler {
    sources: HashMap<SourceKey, Source>,
    visible: Vec<PathBuf>,
    visible_set: HashSet<PathBuf>,
    cancels: HashMap<PathBuf, Cancel>,
    inflight: HashMap<PathBuf, u64>,
    /// Paths whose walk failed (unreadable, or a symlink): do not retry until the
    /// row leaves the screen or an event touches it.
    failed: HashSet<PathBuf>,
    sizes: HashMap<PathBuf, SizeView>,
    next_generation: u64,
    version: u64,
}

impl Scheduler {
    fn seed(&mut self, cache: &Cache) {
        self.sizes.clear();
        for (path, entry) in cache.entries() {
            self.sizes.insert(path.to_path_buf(), SizeView { bytes: entry.bytes, stale: entry.stale, version: 0 });
        }
    }

    fn observe(&mut self, source: SourceKey, paths: &[PathBuf], cache: &Cache) -> Vec<Request> {
        self.sources.insert(source, Source { paths: paths.to_vec(), seen: Instant::now() });
        self.sources.retain(|_, source| source.seen.elapsed() < SOURCE_TTL);
        let mut ordered: Vec<&Source> = self.sources.values().collect();
        ordered.sort_by_key(|source| std::cmp::Reverse(source.seen));
        let mut seen = HashSet::new();
        let mut visible = Vec::new();
        for source in ordered {
            for path in &source.paths {
                if seen.insert(path.clone()) {
                    visible.push(path.clone());
                }
            }
        }
        let visible_set: HashSet<PathBuf> = visible.iter().cloned().collect();
        for path in self.visible_set.difference(&visible_set) {
            if let Some(cancel) = self.cancels.remove(path) {
                cancel.cancel();
            }
            self.inflight.remove(path);
            self.failed.remove(path);
        }
        self.visible_set = visible_set;
        let mut requests = Vec::new();
        for path in &visible {
            if let Some(request) = self.request(path, cache) {
                requests.push(request);
            }
        }
        self.visible = visible;
        requests
    }

    /// Queue one path unless it is fresh, in flight or known to fail.
    fn request(&mut self, path: &Path, cache: &Cache) -> Option<Request> {
        if let Some(entry) = cache.get(path) {
            if let Some(view) = self.sizes.get_mut(path) {
                view.bytes = entry.bytes;
                view.stale = entry.stale;
            } else {
                self.sizes.insert(path.to_path_buf(), SizeView { bytes: entry.bytes, stale: entry.stale, version: 0 });
            }
        }
        if self.sizes.get(path).is_some_and(|size| !size.stale) {
            return None;
        }
        if self.inflight.contains_key(path) || self.failed.contains(path) {
            return None;
        }
        let cancel = Cancel::new();
        self.next_generation += 1;
        let generation = self.next_generation;
        self.cancels.insert(path.to_path_buf(), cancel.clone());
        self.inflight.insert(path.to_path_buf(), generation);
        Some(Request { path: path.to_path_buf(), cancel, generation })
    }

    fn complete(&mut self, outcome: Outcome) -> bool {
        if self.inflight.get(&outcome.path) != Some(&outcome.generation) {
            return false;
        }
        self.inflight.remove(&outcome.path);
        self.cancels.remove(&outcome.path);
        match outcome.measured {
            Some(measured) => {
                self.failed.remove(&outcome.path);
                self.version += 1;
                self.sizes
                    .insert(outcome.path, SizeView { bytes: measured.bytes, stale: false, version: self.version });
                true
            }
            None => {
                if self.visible_set.contains(&outcome.path) {
                    self.failed.insert(outcome.path);
                }
                false
            }
        }
    }

    /// The cache already marked the event's folder and ancestors; sync the views
    /// and let the next `observe` re-request the visible stale rows.
    fn mark_stale(&mut self, path: &Path, cache: &Cache) {
        for (entry_path, size) in &mut self.sizes {
            if let Some(entry) = cache.get(entry_path) {
                size.stale = entry.stale;
            }
        }
        self.failed.retain(|failed| !failed.starts_with(path) && !path.starts_with(failed));
    }

    fn clear(&mut self) {
        for cancel in self.cancels.values() {
            cancel.cancel();
        }
        self.cancels.clear();
        self.inflight.clear();
        self.failed.clear();
        self.sizes.clear();
    }

    /// The setting turned off: stop every request and forget the visible rows,
    /// keeping the measured sizes for when it turns on again.
    fn clear_visible(&mut self) {
        for cancel in self.cancels.values() {
            cancel.cancel();
        }
        self.cancels.clear();
        self.inflight.clear();
        self.failed.clear();
        self.visible.clear();
        self.visible_set.clear();
        self.sources.clear();
    }

    fn size(&self, path: &Path) -> Option<SizeView> {
        self.sizes.get(path).copied()
    }

    /// Every visible row has a fresh size (or a walk that failed, which will not
    /// produce one). Q3: the list re-sorts once, here.
    fn visible_complete(&self) -> bool {
        self.visible
            .iter()
            .all(|path| self.sizes.get(path).is_some_and(|size| !size.stale) || self.failed.contains(path))
    }
}

/// The app-wide folder-size service. Panes hold its entity; the window and the
/// Info pane observe it so a size that arrives repaints them.
pub struct FolderSizes {
    scheduler: Scheduler,
    cache: Arc<RwLock<Cache>>,
    workers: Option<[Sender<Request>; WORKERS]>,
    next_worker: usize,
    results: Option<UnboundedReceiver<Outcome>>,
    events: Option<UnboundedReceiver<Vec<Event>>>,
    events_tx: Option<UnboundedSender<Vec<Event>>>,
    fsevents: Option<FsEvents>,
    save: Option<Sender<()>>,
    cache_path: PathBuf,
    watcher: bool,
    loaded: bool,
    generation: u64,
    next_source: Cell<u64>,
    /// Paths the one-off command measured while the setting is off; the Size
    /// column shows these even though automatic measuring is off.
    manual: HashSet<PathBuf>,
    /// Tests measure synchronously on the main thread with this fake sizer.
    #[cfg(test)]
    test_sizer: Option<Arc<dyn Sizer>>,
    /// Requests a test's render queued; `measure_now` runs them.
    #[cfg(test)]
    pending: Vec<Request>,
}

#[derive(Clone)]
pub struct SizeService(pub Entity<FolderSizes>);

impl Global for SizeService {}

/// `cache_dir` holds `folder-sizes.json`; `watcher` starts the FSEvents stream
/// (off in tests, which drive the scheduler directly).
pub fn init_at(cx: &mut App, cache_dir: PathBuf, watcher: bool) {
    let cache = Arc::new(RwLock::new(Cache::default()));
    let (results_tx, results_rx) = unbounded();
    let (events_tx, events_rx) = unbounded();
    let (save_tx, save_rx) = channel();
    let workers = spawn_workers(cache.clone(), results_tx, Arc::new(RealSizer));
    spawn_saver(cache.clone(), cache_dir.join("folder-sizes.json"), save_rx);
    let entity = cx.new(|_| FolderSizes {
        scheduler: Scheduler::default(),
        cache,
        workers: Some(workers),
        next_worker: 0,
        results: Some(results_rx),
        events: Some(events_rx),
        events_tx: Some(events_tx),
        fsevents: None,
        save: Some(save_tx),
        cache_path: cache_dir.join("folder-sizes.json"),
        watcher,
        loaded: false,
        generation: 0,
        next_source: Cell::new(0),
        manual: HashSet::new(),
        #[cfg(test)]
        test_sizer: None,
        #[cfg(test)]
        pending: Vec::new(),
    });
    entity.update(cx, |service, cx| service.start(cx));
    cx.set_global(SizeService(entity));
}

/// A service for tests: no threads, no FSEvents, and `sizer` runs on the main
/// thread when `observe` asks for a row. The pane tests draw twice to see the
/// value, which is what a real result's notify would do.
#[cfg(test)]
pub fn init_for_tests(cx: &mut App, sizer: Arc<dyn Sizer>) {
    let entity = cx.new(|_| FolderSizes {
        scheduler: Scheduler::default(),
        cache: Arc::new(RwLock::new(Cache::default())),
        workers: None,
        next_worker: 0,
        results: None,
        events: None,
        events_tx: None,
        fsevents: None,
        save: None,
        cache_path: PathBuf::new(),
        watcher: false,
        loaded: true,
        generation: 0,
        next_source: Cell::new(0),
        manual: HashSet::new(),
        test_sizer: Some(sizer),
        pending: Vec::new(),
    });
    cx.set_global(SizeService(entity));
}

impl FolderSizes {
    pub fn source(&self) -> u64 {
        let id = self.next_source.get();
        self.next_source.set(id + 1);
        id
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn size(&self, path: &Path) -> Option<SizeView> {
        self.scheduler.size(path)
    }

    /// Whether every visible row's size is in (or its walk failed). The pane
    /// re-sorts by size when this turns true.
    pub fn visible_complete(&self) -> bool {
        self.scheduler.visible_complete()
    }

    /// A path the one-off command measured while automatic sizes are off.
    pub fn manual(&self, path: &Path) -> bool {
        self.manual.contains(path)
    }

    /// The setting turned off: cancel the visible requests and stop reporting.
    pub fn pause(&mut self) {
        self.scheduler.clear_visible();
        // Off keeps no stream open: it watches `/` and wakes the process for every
        // change on the disk. `observe` starts it again when sizes come back.
        self.fsevents = None;
    }

    /// View ▸ Calculate Folder Sizes: measure `paths` once, regardless of the
    /// setting, and remember them so the Size column shows the result.
    pub fn measure_once(&mut self, paths: &[PathBuf], _cx: &mut Context<Self>) {
        let cache = self.cache.read().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut requests = Vec::new();
        for path in paths {
            self.manual.insert(path.clone());
            self.scheduler.failed.remove(path);
            if let Some(request) = self.scheduler.request(path, &cache) {
                requests.push(request);
            }
        }
        drop(cache);
        #[cfg(test)]
        if self.test_sizer.is_some() {
            self.pending.extend(requests);
            return;
        }
        for request in requests {
            self.dispatch(request);
        }
    }

    /// Called from a pane's row renderer with the rows it shows. Enqueues only;
    /// never notifies, so it cannot make the render loop.
    pub fn observe(&mut self, source: SourceKey, paths: &[PathBuf], cx: &mut Context<Self>) {
        self.ensure_watcher(cx);
        let cache = self.cache.read().unwrap_or_else(|poisoned| poisoned.into_inner());
        let requests = self.scheduler.observe(source, paths, &cache);
        drop(cache);
        #[cfg(test)]
        if self.test_sizer.is_some() {
            self.pending.extend(requests);
            return;
        }
        for request in requests {
            self.dispatch(request);
        }
    }

    /// Run the requests a test's render queued. Tests call this between draws so
    /// the before/after states are deterministic.
    #[cfg(test)]
    pub fn measure_now(&mut self, cx: &mut Context<Self>) {
        let Some(sizer) = self.test_sizer.clone() else { return };
        let pending = std::mem::take(&mut self.pending);
        let mut changed = false;
        for request in pending {
            if request.cancel.is_cancelled() {
                continue;
            }
            let measured = sizer.measure(&request.path, &request.cancel);
            let outcome = Outcome { path: request.path, generation: request.generation, measured };
            if self.scheduler.complete(outcome) {
                changed = true;
            }
        }
        if changed {
            self.generation += 1;
            cx.notify();
        }
    }

    fn dispatch(&mut self, request: Request) {
        let Some(workers) = &mut self.workers else { return };
        let worker = self.next_worker;
        self.next_worker = (self.next_worker + 1) % WORKERS;
        workers[worker].send(request).ok();
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let mut results = self.results.take().expect("started once");
        cx.spawn(async move |this, cx| {
            while let Some(outcome) = results.next().await {
                this.update(cx, |service, cx| service.on_outcome(outcome, cx)).ok();
            }
        })
        .detach();

        let mut events = self.events.take().expect("started once");
        cx.spawn(async move |this, cx| {
            while let Some(batch) = events.next().await {
                this.update(cx, |service, cx| service.on_events(batch, cx)).ok();
            }
        })
        .detach();

        let path = self.cache_path.clone();
        cx.spawn(async move |this, cx| {
            let (loaded, uuid) = cx
                .background_spawn(async move {
                    let mut cache = Cache::load(&path);
                    // io-ok: inside background_spawn; a recreated folder must not show the old inode's size.
                    cache.validate_inodes();
                    // io-ok: inside background_spawn
                    let dev = std::fs::metadata("/").map(|meta| device(&meta)).unwrap_or(0);
                    (cache, crow_macos::volume_uuid(dev))
                })
                .await;
            this.update(cx, |service, cx| {
                let mut loaded = loaded;
                if let (Some(stored), Some(current)) = (loaded.volume_uuid(), uuid.as_deref())
                    && stored != current
                {
                    loaded.reset_history();
                }
                if loaded.volume_uuid().is_none() {
                    loaded.set_volume_uuid(uuid.clone());
                }
                {
                    let mut cache = service.cache.write().unwrap_or_else(|poisoned| poisoned.into_inner());
                    *cache = loaded;
                    service.scheduler.seed(&cache);
                }
                service.loaded = true;
                service.ensure_watcher(cx);
                service.generation += 1;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Start the FSEvents stream once the cache is loaded and the setting asks
    /// for visible sizes. Off keeps the stream off (and the bench deterministic).
    fn ensure_watcher(&mut self, cx: &mut Context<Self>) {
        if !self.watcher || !self.loaded || self.fsevents.is_some() {
            return;
        }
        if cx.global::<Ui>().settings.folder_sizes != FolderSizesMode::Visible {
            return;
        }
        let since = {
            let cache = self.cache.read().unwrap_or_else(|poisoned| poisoned.into_inner());
            let id = cache.last_event_id();
            if id == 0 { crow_macos::current_event_id() } else { id }
        };
        let Some(tx) = self.events_tx.clone() else { return };
        self.fsevents = crow_macos::watch(since, FSEVENTS_LATENCY, move |events| {
            tx.unbounded_send(events).ok();
        });
    }

    fn on_outcome(&mut self, outcome: Outcome, cx: &mut Context<Self>) {
        if self.scheduler.complete(outcome) {
            self.generation += 1;
            if let Some(save) = &self.save {
                save.send(()).ok();
            }
            cx.notify();
        }
    }

    fn on_events(&mut self, events: Vec<Event>, cx: &mut Context<Self>) {
        let mut changed = false;
        let mut restart = false;
        {
            let mut cache = self.cache.write().unwrap_or_else(|poisoned| poisoned.into_inner());
            for event in events {
                if event.flags.dropped() {
                    cache.reset_history();
                    cache.set_last_event_id(crow_macos::current_event_id());
                    self.scheduler.clear();
                    changed = true;
                    continue;
                }
                if event.flags.root_changed() {
                    // The root the stream watches is gone or renamed; skip past
                    // this event before starting again so it cannot replay into a
                    // restart loop.
                    let id = cache.last_event_id().max(event.id);
                    cache.set_last_event_id(id);
                    restart = true;
                    changed = true;
                    continue;
                }
                if event.flags.history_done() {
                    // The marker only says the replay is complete; it is not a
                    // changed folder and must not make anything stale.
                    continue;
                }
                if event.flags.must_scan_subdirs() {
                    cache.mark_subtree_stale(&event.path);
                } else {
                    cache.mark_stale(&event.path);
                }
                cache.set_last_event_id(event.id);
                self.scheduler.mark_stale(&event.path, &cache);
                changed = true;
            }
        }
        if restart {
            self.fsevents = None;
            self.ensure_watcher(cx);
        }
        if changed {
            self.generation += 1;
            if let Some(save) = &self.save {
                save.send(()).ok();
            }
            cx.notify();
        }
    }
}

fn spawn_workers(
    cache: Arc<RwLock<Cache>>,
    results: UnboundedSender<Outcome>,
    sizer: Arc<dyn Sizer>,
) -> [Sender<Request>; WORKERS] {
    let mut senders = Vec::with_capacity(WORKERS);
    for index in 0..WORKERS {
        let (tx, rx) = channel();
        let cache = cache.clone();
        let results = results.clone();
        let sizer = sizer.clone();
        std::thread::Builder::new()
            .name(format!("crow-sizes-{index}"))
            .spawn(move || {
                set_low_qos();
                worker(rx, cache, results, sizer);
            })
            .ok();
        senders.push(tx);
    }
    [senders.remove(0), senders.remove(0)]
}

fn worker(rx: Receiver<Request>, cache: Arc<RwLock<Cache>>, results: UnboundedSender<Outcome>, sizer: Arc<dyn Sizer>) {
    while let Ok(request) = rx.recv() {
        if request.cancel.is_cancelled() {
            results.unbounded_send(Outcome { path: request.path, generation: request.generation, measured: None }).ok();
            continue;
        }
        let start_event = cache.read().unwrap_or_else(|poisoned| poisoned.into_inner()).last_event_id();
        // io-ok: size worker thread
        let inode = std::fs::metadata(&request.path).map(|meta| inode(&meta)).unwrap_or(0);
        let measured = sizer.measure(&request.path, &request.cancel);
        if let Some(measured) = &measured {
            let mut cache = cache.write().unwrap_or_else(|poisoned| poisoned.into_inner());
            let current_event = cache.last_event_id();
            cache.insert(&request.path, inode, measured, start_event, now());
            if current_event != start_event {
                // FSEvents saw a change while the walk ran; the walk may have
                // passed that folder before the change, so the bytes are stale.
                cache.mark_stale(&request.path);
            }
            cache.prune(MAX_ENTRIES);
        }
        results.unbounded_send(Outcome { path: request.path, generation: request.generation, measured }).ok();
    }
}

fn spawn_saver(cache: Arc<RwLock<Cache>>, path: PathBuf, rx: Receiver<()>) {
    std::thread::Builder::new()
        .name("crow-sizes-save".into())
        .spawn(move || {
            loop {
                match rx.recv() {
                    Ok(()) => {
                        let connected = wait_for_save(&rx, SAVE_DEBOUNCE, SAVE_MAX_DELAY);
                        save(&cache, &path);
                        if !connected {
                            break;
                        }
                    }
                    Err(_) => {
                        save(&cache, &path);
                        break;
                    }
                }
            }
        })
        .ok();
}

/// Wait for a quiet `debounce` window, but never past `max` from the first
/// signal: FSEvents on the boot volume can keep signalling forever, and a cache
/// that is only written on quit loses every measurement if the app crashes.
/// Returns false when the sender is gone.
fn wait_for_save(rx: &Receiver<()>, debounce: Duration, max: Duration) -> bool {
    wait_for_save_with(|timeout| rx.recv_timeout(timeout), Instant::now, debounce, max)
}

fn wait_for_save_with(
    mut receive: impl FnMut(Duration) -> Result<(), RecvTimeoutError>,
    now: impl Fn() -> Instant,
    debounce: Duration,
    max: Duration,
) -> bool {
    let deadline = now() + max;
    loop {
        let left = deadline.saturating_duration_since(now());
        if left.is_zero() {
            return true;
        }
        match receive(left.min(debounce)) {
            Ok(()) => {}
            Err(RecvTimeoutError::Timeout) => return true,
            Err(RecvTimeoutError::Disconnected) => return false,
        }
    }
}

fn save(cache: &RwLock<Cache>, path: &Path) {
    let snapshot = cache.read().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
    snapshot.save(path).ok();
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs())
}

// io-ok: type only
fn inode(meta: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.ino()
}

// io-ok: type only
fn device(meta: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.dev()
}

fn set_low_qos() {
    #[cfg(target_os = "macos")]
    // SAFETY: sets the current thread's QoS class; no pointers.
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::mpsc;

    fn cache_with(path: &str, bytes: u64, stale: bool) -> Cache {
        let mut cache = Cache::default();
        cache.insert(Path::new(path), 1, &Measured { bytes, ..Default::default() }, 0, 1);
        if stale {
            cache.mark_stale(Path::new(path));
        }
        cache
    }

    struct NoSizer;

    impl Sizer for NoSizer {
        fn measure(&self, _: &Path, _: &Cancel) -> Option<Measured> {
            None
        }
    }

    #[gpui::test]
    fn turning_sizes_off_closes_the_disk_wide_stream(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| init_for_tests(cx, Arc::new(NoSizer)));
        let service = cx.update(|cx| cx.global::<SizeService>().0.clone());
        service.update(cx, |service, _| {
            service.fsevents = crow_macos::watch(crow_macos::current_event_id(), FSEVENTS_LATENCY, |_| {});
            assert!(service.fsevents.is_some(), "the stream starts while sizes are visible");
            service.pause();
            assert!(service.fsevents.is_none(), "off closes it");
        });
    }

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn requests_are_nearest_the_top_and_deduplicated() {
        let mut scheduler = Scheduler::default();
        let cache = Cache::default();
        let requests = scheduler.observe((0, 0), &paths(&["/a", "/b", "/a", "/c"]), &cache);
        let asked: Vec<&Path> = requests.iter().map(|request| request.path.as_path()).collect();
        assert_eq!(asked, [Path::new("/a"), Path::new("/b"), Path::new("/c")]);
        assert!(scheduler.observe((0, 0), &paths(&["/a", "/b", "/c"]), &cache).is_empty(), "in flight");
    }

    #[test]
    fn scrolling_away_cancels_and_ignores_the_result() {
        let mut scheduler = Scheduler::default();
        let cache = Cache::default();
        let requests = scheduler.observe((0, 0), &paths(&["/a", "/b"]), &cache);
        let a = requests.iter().find(|request| request.path == Path::new("/a")).unwrap();
        assert!(!a.cancel.is_cancelled());
        scheduler.observe((0, 0), &paths(&["/b"]), &cache);
        assert!(a.cancel.is_cancelled());
        assert!(!scheduler.complete(Outcome {
            path: PathBuf::from("/a"),
            generation: a.generation,
            measured: Some(Measured { bytes: 10, ..Default::default() }),
        }));
        assert_eq!(scheduler.size(Path::new("/a")), None);
    }

    #[test]
    fn a_fresh_cache_entry_shows_at_once_and_a_stale_one_is_remeasured() {
        let mut scheduler = Scheduler::default();
        let cache = cache_with("/a", 42, false);
        assert!(scheduler.observe((0, 0), &paths(&["/a"]), &cache).is_empty());
        assert_eq!(scheduler.size(Path::new("/a")), Some(SizeView { bytes: 42, stale: false, version: 0 }));
        let stale = cache_with("/a", 42, true);
        let requests = scheduler.observe((0, 0), &paths(&["/a"]), &stale);
        assert_eq!(requests.len(), 1);
        assert!(scheduler.complete(Outcome {
            path: PathBuf::from("/a"),
            generation: requests[0].generation,
            measured: Some(Measured { bytes: 50, ..Default::default() }),
        }));
        assert_eq!(scheduler.size(Path::new("/a")), Some(SizeView { bytes: 50, stale: false, version: 1 }));
    }

    #[test]
    fn a_failed_walk_is_not_retried_until_the_row_leaves() {
        let mut scheduler = Scheduler::default();
        let cache = Cache::default();
        let requests = scheduler.observe((0, 0), &paths(&["/locked"]), &cache);
        assert_eq!(requests.len(), 1);
        assert!(!scheduler.complete(Outcome {
            path: PathBuf::from("/locked"),
            generation: requests[0].generation,
            measured: None,
        }));
        assert!(scheduler.observe((0, 0), &paths(&["/locked"]), &cache).is_empty());
        scheduler.observe((0, 0), &[], &cache);
        assert_eq!(scheduler.observe((0, 0), &paths(&["/locked"]), &cache).len(), 1, "scrolling back retries");
    }

    #[test]
    fn two_panes_share_the_visible_union() {
        let mut scheduler = Scheduler::default();
        let cache = Cache::default();
        scheduler.observe((0, 0), &paths(&["/a", "/b"]), &cache);
        let column = scheduler.observe((0, 1), &paths(&["/c"]), &cache);
        assert_eq!(column.len(), 1);
        assert_eq!(scheduler.visible, paths(&["/c", "/a", "/b"]), "the newest source is first");
        scheduler.observe((0, 0), &paths(&["/b"]), &cache);
        assert_eq!(scheduler.visible, paths(&["/b", "/c"]));
    }

    struct FakeSizer {
        seen: Mutex<Vec<PathBuf>>,
        bytes: u64,
    }

    impl Sizer for FakeSizer {
        fn measure(&self, path: &Path, _: &Cancel) -> Option<Measured> {
            self.seen.lock().unwrap().push(path.to_path_buf());
            Some(Measured { bytes: self.bytes, items: 1, ..Default::default() })
        }
    }

    #[test]
    fn a_worker_measures_in_order_and_fills_the_cache() {
        let cache = Arc::new(RwLock::new(Cache::default()));
        let (results_tx, mut results_rx) = unbounded();
        let fake = Arc::new(FakeSizer { seen: Mutex::new(Vec::new()), bytes: 7 });
        let (tx, rx) = mpsc::channel();
        let worker_cache = cache.clone();
        let worker_fake = fake.clone();
        std::thread::spawn(move || worker(rx, worker_cache, results_tx, worker_fake));
        for path in ["/a", "/b"] {
            tx.send(Request { path: PathBuf::from(path), cancel: Cancel::new(), generation: 1 }).unwrap();
        }
        let first = futures::executor::block_on(results_rx.recv()).unwrap();
        let second = futures::executor::block_on(results_rx.recv()).unwrap();
        assert_eq!(first.path, PathBuf::from("/a"));
        assert_eq!(second.path, PathBuf::from("/b"));
        assert_eq!(*fake.seen.lock().unwrap(), paths(&["/a", "/b"]));
        assert_eq!(cache.read().unwrap().get(Path::new("/a")).unwrap().bytes, 7);
    }

    #[test]
    fn a_cancelled_request_is_not_measured() {
        let cache = Arc::new(RwLock::new(Cache::default()));
        let (results_tx, mut results_rx) = unbounded();
        let fake = Arc::new(FakeSizer { seen: Mutex::new(Vec::new()), bytes: 7 });
        let (tx, rx) = mpsc::channel();
        let worker_cache = cache.clone();
        let worker_fake = fake.clone();
        std::thread::spawn(move || worker(rx, worker_cache, results_tx, worker_fake));
        let cancel = Cancel::new();
        cancel.cancel();
        tx.send(Request { path: PathBuf::from("/a"), cancel, generation: 1 }).unwrap();
        let outcome = futures::executor::block_on(results_rx.recv()).unwrap();
        assert!(outcome.measured.is_none());
        assert!(fake.seen.lock().unwrap().is_empty());
    }

    #[test]
    fn a_quiet_channel_saves_after_the_debounce() {
        let (tx, rx) = mpsc::channel();
        tx.send(()).unwrap();
        let started = Instant::now();
        assert!(wait_for_save(&rx, Duration::from_millis(30), Duration::from_secs(1)));
        assert!(started.elapsed() >= Duration::from_millis(25), "waited for quiet: {:?}", started.elapsed());
    }

    #[test]
    fn a_steady_stream_of_signals_saves_at_the_cap_not_never() {
        let started = Instant::now();
        let elapsed = std::cell::Cell::new(Duration::ZERO);
        let cap = Duration::from_millis(100);
        let signal_interval = Duration::from_millis(7);

        let receive = |timeout: Duration| {
            assert!(elapsed.get() < cap, "waited for another signal after the cap");
            elapsed.set(elapsed.get() + timeout.min(signal_interval));
            if timeout < signal_interval { Err(RecvTimeoutError::Timeout) } else { Ok(()) }
        };

        assert!(wait_for_save_with(receive, || started + elapsed.get(), Duration::from_millis(30), cap));
        assert_eq!(elapsed.get(), cap, "saved at the cap while signals were still arriving");
    }

    #[test]
    fn a_closed_channel_stops_the_saver() {
        let (tx, rx) = mpsc::channel::<()>();
        drop(tx);
        assert!(!wait_for_save(&rx, Duration::from_millis(30), Duration::from_secs(1)));
    }

    #[test]
    fn an_event_during_a_measure_marks_the_result_stale() {
        struct EventSizer(Arc<RwLock<Cache>>);
        impl Sizer for EventSizer {
            fn measure(&self, _: &Path, _: &Cancel) -> Option<Measured> {
                self.0.write().unwrap().set_last_event_id(11);
                Some(Measured { bytes: 7, ..Default::default() })
            }
        }

        let cache = Arc::new(RwLock::new(Cache::default()));
        cache.write().unwrap().set_last_event_id(10);
        let (results_tx, mut results_rx) = unbounded();
        let (tx, rx) = mpsc::channel();
        let worker_cache = cache.clone();
        let sizer = Arc::new(EventSizer(cache.clone()));
        std::thread::spawn(move || worker(rx, worker_cache, results_tx, sizer));
        tx.send(Request { path: PathBuf::from("/a"), cancel: Cancel::new(), generation: 1 }).unwrap();
        let outcome = futures::executor::block_on(results_rx.recv()).unwrap();
        assert!(outcome.measured.is_some());
        let cache = cache.read().unwrap();
        let entry = cache.get(Path::new("/a")).expect("cached");
        assert!(entry.stale, "the event invalidated the measure");
        assert_eq!(entry.event_id, 10, "the walk's start id, not the completion id");
    }
}
