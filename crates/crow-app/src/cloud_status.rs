//! Cached iCloud status for visible and selected rows, plus the fresh read gate
//! shared with preview, search and plugins.
//!
//! Panes report the rows their lists show; the service batches Foundation
//! lookups off the main thread, bounds its cache, drops late replies by version
//! and polls only visible active downloads. Rendering reads the cache and never
//! calls Foundation.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crow_fs::{CloudStatus, ReadGate, status_age_limit};
#[cfg(test)]
use crow_fs::{ReadDecision, ReadHint};
use crow_theme::Theme;
use gpui::{App, AppContext as _, Context, Div, Entity, Global, Pixels, div, prelude::*, px};

use crate::icons::{Glyph, glyph};

/// A pane's rows: `(source id, 0)` for its list, `(source id, k + 1)` for column k.
pub type CloudSource = (u64, usize);

/// A cache large enough for several visible lists and the selected items, small
/// enough that a walk over a big home folder cannot grow it without bound.
pub const MAX_ENTRIES: usize = 4096;
const BATCH_SIZE: usize = 128;
const BADGE_WIDTH: f32 = 12.;
/// A pane that stops reporting (a closed window) stops keeping its rows alive.
const SOURCE_TTL: Duration = Duration::from_secs(5);
/// Visible downloading rows are re-read at this pace; the value is short enough
/// to show the transition to downloaded, long enough not to churn Foundation.
pub const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// The classification boundary shared with P1B and P3B. Implementations must be
/// thread-safe; `batch` runs on a background executor.
pub trait CloudProvider: Send + Sync {
    fn status(&self, path: &Path) -> CloudStatus;

    fn batch(&self, paths: &[PathBuf]) -> Vec<CloudStatus> {
        paths.iter().map(|path| self.status(path)).collect()
    }
}

pub struct SystemProvider;

impl CloudProvider for SystemProvider {
    fn status(&self, path: &Path) -> CloudStatus {
        // io-ok: the cloud status service probes on a background task.
        crow_macos::cloud_status(path)
    }

    fn batch(&self, paths: &[PathBuf]) -> Vec<CloudStatus> {
        // io-ok: the cloud status service probes on a background task.
        crow_macos::cloud_status_batch(paths)
    }
}

#[derive(Clone, Copy)]
struct Cached {
    status: CloudStatus,
    checked: Instant,
}

struct Source {
    paths: Vec<PathBuf>,
    seen: Instant,
}

struct Request {
    path: PathBuf,
    version: u64,
}

/// The app-wide service. Panes and search observe it; the Info pane reads it.
pub struct CloudStatuses {
    cache: HashMap<PathBuf, Cached>,
    versions: HashMap<PathBuf, u64>,
    next_version: u64,
    pending: HashSet<PathBuf>,
    sources: HashMap<CloudSource, Source>,
    visible: Vec<PathBuf>,
    provider: Arc<dyn CloudProvider>,
    gate: Arc<dyn ReadGate>,
    next_source: Cell<u64>,
    batch_in_flight: bool,
    poll_scheduled: bool,
}

#[derive(Clone)]
pub struct CloudStatusService(pub Entity<CloudStatuses>);

impl Global for CloudStatusService {}

pub fn register_source<T: 'static>(service: &Entity<CloudStatuses>, cx: &mut Context<T>) -> u64 {
    let owner = service.read(cx).source();
    let service = service.downgrade();
    cx.on_release(move |_, cx| {
        service.update(cx, |service, cx| service.forget(owner, cx)).ok();
    })
    .detach();
    owner
}

/// The fresh read gate for a background reader, taken on the main thread
/// before the work is spawned. Without the service nothing is cloud-aware, so
/// ordinary local reads are allowed.
pub fn read_gate(cx: &App) -> Arc<dyn ReadGate> {
    match cx.try_global::<CloudStatusService>() {
        Some(service) => service.0.read(cx).gate(),
        None => Arc::new(crow_fs::AllowAll),
    }
}

pub fn init_at(cx: &mut App) {
    install(cx, Arc::new(SystemProvider), Arc::new(crow_macos::SystemReadGate));
}

/// Test app setup: a controllable provider and gate. `ui::init_for_tests` uses
/// this; a test changes statuses through `set_test_status`.
#[cfg(test)]
pub fn init_for_tests(cx: &mut App) {
    let statuses = Arc::new(std::sync::Mutex::new(HashMap::new()));
    cx.set_global(TestStatuses(statuses.clone()));
    install(cx, Arc::new(TestProvider(statuses.clone())), Arc::new(TestGate(statuses)));
}

fn install(cx: &mut App, provider: Arc<dyn CloudProvider>, gate: Arc<dyn ReadGate>) {
    let entity = cx.new(|_| CloudStatuses {
        cache: HashMap::new(),
        versions: HashMap::new(),
        next_version: 0,
        pending: HashSet::new(),
        sources: HashMap::new(),
        visible: Vec::new(),
        provider,
        gate,
        next_source: Cell::new(0),
        batch_in_flight: false,
        poll_scheduled: false,
    });
    cx.set_global(CloudStatusService(entity));
}

impl CloudStatuses {
    pub fn source(&self) -> u64 {
        let id = self.next_source.get();
        self.next_source.set(id + 1);
        id
    }

    /// The cached status for painting. `None` is unprobed or evicted from the
    /// cache, not `Unknown`.
    pub fn status(&self, path: &Path) -> Option<CloudStatus> {
        self.cache.get(path).map(|entry| entry.status)
    }

    /// The fresh gate for automatic readers. It never trusts this cache.
    pub fn gate(&self) -> Arc<dyn ReadGate> {
        self.gate.clone()
    }

    /// Called from a list renderer with the rows it shows. Enqueues only; never
    /// notifies, so it cannot make the render loop.
    pub fn observe(&mut self, source: CloudSource, paths: &[PathBuf], cx: &mut Context<Self>) {
        if paths.is_empty() {
            self.sources.remove(&source);
        } else {
            self.sources.insert(source, Source { paths: paths.to_vec(), seen: Instant::now() });
        }
        self.sources.retain(|_, source| source.seen.elapsed() < SOURCE_TTL);
        self.update_visible(cx);
    }

    pub fn forget(&mut self, owner: u64, cx: &mut Context<Self>) {
        self.sources.retain(|(source, _), _| *source != owner);
        self.update_visible(cx);
    }

    fn update_visible(&mut self, cx: &mut Context<Self>) {
        let mut ordered: Vec<&Source> = self.sources.values().collect();
        ordered.sort_by_key(|source| std::cmp::Reverse(source.seen));
        let mut seen = HashSet::new();
        self.visible = ordered
            .into_iter()
            .flat_map(|source| &source.paths)
            .filter(|path| seen.insert((*path).clone()))
            .take(MAX_ENTRIES)
            .cloned()
            .collect();
        // Rows that scrolled away stop their queued lookups; a late reply is
        // dropped because its version entry is gone.
        self.pending.retain(|path| seen.contains(path));
        self.versions.retain(|path, _| seen.contains(path));
        self.request_stale(cx);
    }

    /// A watcher batch touched `dir`: forget its cached statuses and re-request
    /// visible rows. A late reply from before the event is dropped by version.
    pub fn invalidate_under(&mut self, dir: &Path, cx: &mut Context<Self>) {
        let mut changed = false;
        self.cache.retain(|path, _| {
            let keep = !path.starts_with(dir);
            changed |= !keep;
            keep
        });
        let touched: Vec<PathBuf> = self.pending.iter().filter(|path| path.starts_with(dir)).cloned().collect();
        for path in touched {
            self.pending.remove(&path);
            self.next_version += 1;
            self.versions.insert(path, self.next_version);
        }
        if changed {
            cx.notify();
        }
        self.request_stale(cx);
    }

    /// Tests change a provider's answer and force the visible rows to be read
    /// again without waiting for a status TTL.
    #[cfg(test)]
    pub fn force_refresh(&mut self, cx: &mut Context<Self>) {
        for path in &self.visible {
            self.cache.remove(path);
        }
        self.request_stale(cx);
    }

    fn request_stale(&mut self, cx: &mut Context<Self>) {
        if self.batch_in_flight {
            return;
        }
        let now = Instant::now();
        let mut requests = Vec::new();
        for path in &self.visible {
            let fresh = self
                .cache
                .get(path)
                .is_some_and(|entry| now.duration_since(entry.checked) < status_age_limit(entry.status));
            if fresh || self.pending.contains(path) {
                continue;
            }
            self.next_version += 1;
            let version = self.next_version;
            self.versions.insert(path.clone(), version);
            self.pending.insert(path.clone());
            requests.push(Request { path: path.clone(), version });
            if requests.len() == BATCH_SIZE {
                break;
            }
        }
        if requests.is_empty() {
            self.ensure_poll(cx);
            return;
        }
        self.batch_in_flight = true;
        let provider = self.provider.clone();
        let paths: Vec<PathBuf> = requests.iter().map(|request| request.path.clone()).collect();
        cx.spawn(async move |this, cx| {
            let statuses = cx.background_spawn(async move { provider.batch(&paths) }).await;
            this.update(cx, |this, cx| this.accept(&requests, statuses, cx)).ok();
        })
        .detach();
    }

    fn accept(&mut self, requests: &[Request], statuses: Vec<CloudStatus>, cx: &mut Context<Self>) {
        self.batch_in_flight = false;
        let changed = self.apply(requests, statuses);
        if changed {
            cx.notify();
        }
        self.request_stale(cx);
    }

    /// Apply one batch after checking the version it was requested with.
    fn apply(&mut self, requests: &[Request], statuses: Vec<CloudStatus>) -> bool {
        let mut changed = false;
        for (request, status) in requests.iter().zip(statuses) {
            if self.versions.get(&request.path) != Some(&request.version) || !self.pending.remove(&request.path) {
                continue;
            }
            if self.cache.get(&request.path).is_none_or(|entry| entry.status != status) {
                changed = true;
            }
            self.cache.insert(request.path.clone(), Cached { status, checked: Instant::now() });
            self.versions.remove(&request.path);
        }
        self.trim_cache();
        changed
    }

    fn trim_cache(&mut self) {
        while self.cache.len() > MAX_ENTRIES {
            let Some(oldest) = self.cache.iter().min_by_key(|(_, entry)| entry.checked).map(|(path, _)| path.clone())
            else {
                break;
            };
            self.cache.remove(&oldest);
        }
    }

    /// Keep re-reading visible downloads (and not-downloaded rows, so an
    /// external Finder download shows up) until none are left.
    fn ensure_poll(&mut self, cx: &mut Context<Self>) {
        if self.poll_scheduled {
            return;
        }
        let has_active = self.visible.iter().any(|path| {
            self.cache
                .get(path)
                .is_some_and(|entry| !matches!(entry.status, CloudStatus::Local | CloudStatus::Downloaded))
        });
        if !has_active {
            return;
        }
        self.poll_scheduled = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(POLL_INTERVAL).await;
            this.update(cx, |this, cx| {
                this.poll_scheduled = false;
                this.request_stale(cx);
            })
            .ok();
        })
        .detach();
    }
}

/// Leave room for the glyph and the row's 4 px gap before truncating a name.
pub fn name_room(status: Option<CloudStatus>, room: Pixels) -> Pixels {
    if status.is_some_and(|status| !status.is_local()) { (room - px(BADGE_WIDTH + 4.)).max(px(0.)) } else { room }
}

/// The row badge: only a non-local, non-downloaded state is visible. Downloaded
/// iCloud files stay unbadged like Finder's; Info carries the explicit status.
pub fn badge(status: Option<CloudStatus>, theme: &Theme) -> Option<Div> {
    let (glyph_name, role, name) = match status? {
        CloudStatus::Local | CloudStatus::Downloaded => return None,
        CloudStatus::NotDownloaded => (Glyph::Cloud, "fg-muted", "not-downloaded"),
        CloudStatus::Downloading => (Glyph::Cloud, "accent", "downloading"),
        CloudStatus::Unknown => (Glyph::CloudOff, "warning", "unknown"),
    };
    Some(
        div()
            .debug_selector(move || format!("cloud-badge-{name}"))
            .w(px(BADGE_WIDTH))
            .h(px(16.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .child(glyph(glyph_name, 12.0, role, theme)),
    )
}

#[cfg(test)]
#[derive(Clone, Default)]
pub struct TestStatuses(Arc<std::sync::Mutex<HashMap<PathBuf, CloudStatus>>>);

#[cfg(test)]
impl Global for TestStatuses {}

#[cfg(test)]
pub fn set_test_status(cx: &mut App, path: impl Into<PathBuf>, status: CloudStatus) {
    if let Some(global) = cx.try_global::<TestStatuses>() {
        global.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(path.into(), status);
    }
}

#[cfg(test)]
struct TestProvider(Arc<std::sync::Mutex<HashMap<PathBuf, CloudStatus>>>);

#[cfg(test)]
impl CloudProvider for TestProvider {
    fn status(&self, path: &Path) -> CloudStatus {
        self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(path).copied().unwrap_or(CloudStatus::Local)
    }
}

#[cfg(test)]
struct TestGate(Arc<std::sync::Mutex<HashMap<PathBuf, CloudStatus>>>);

#[cfg(test)]
impl ReadGate for TestGate {
    fn check(&self, path: &Path, hint: ReadHint) -> ReadDecision {
        if hint.cheap_decision() != ReadDecision::Readable {
            return hint.cheap_decision();
        }
        match self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(path).copied() {
            Some(CloudStatus::Local | CloudStatus::Downloaded) | None => ReadDecision::Readable,
            Some(CloudStatus::NotDownloaded | CloudStatus::Downloading) => ReadDecision::SkipCloud,
            Some(CloudStatus::Unknown) => ReadDecision::SkipAmbiguous,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakeProvider {
        statuses: std::sync::Mutex<HashMap<PathBuf, CloudStatus>>,
        calls: AtomicUsize,
    }

    impl FakeProvider {
        fn new(entries: &[(&str, CloudStatus)]) -> Self {
            let statuses = entries.iter().map(|(path, status)| (PathBuf::from(path), *status)).collect();
            Self { statuses: std::sync::Mutex::new(statuses), calls: AtomicUsize::new(0) }
        }
    }

    impl CloudProvider for FakeProvider {
        fn status(&self, path: &Path) -> CloudStatus {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.statuses.lock().unwrap().get(path).copied().unwrap_or(CloudStatus::Local)
        }
    }

    fn service(provider: Arc<FakeProvider>) -> (CloudStatuses, Arc<dyn ReadGate>) {
        let gate = Arc::new(TestGate(Arc::new(std::sync::Mutex::new(HashMap::new()))));
        let cache = HashMap::new();
        (
            CloudStatuses {
                cache,
                versions: HashMap::new(),
                next_version: 0,
                pending: HashSet::new(),
                sources: HashMap::new(),
                visible: Vec::new(),
                provider,
                gate,
                next_source: Cell::new(0),
                batch_in_flight: false,
                poll_scheduled: false,
            },
            Arc::new(crow_fs::AllowAll),
        )
    }

    #[test]
    fn status_precedence_and_unknown_are_kept() {
        let provider = Arc::new(FakeProvider::new(&[
            ("/a", CloudStatus::Downloading),
            ("/b", CloudStatus::NotDownloaded),
            ("/c", CloudStatus::Unknown),
        ]));
        let (service, _) = service(provider);
        assert_eq!(service.status(Path::new("/a")), None, "unprobed is not unknown");
        assert_eq!(service.provider.status(Path::new("/a")), CloudStatus::Downloading);
        assert_eq!(service.provider.status(Path::new("/b")), CloudStatus::NotDownloaded);
        assert_eq!(service.provider.status(Path::new("/missing")), CloudStatus::Local);
    }

    #[test]
    fn cache_is_bounded() {
        let provider = Arc::new(FakeProvider::new(&[]));
        let (mut service, _) = service(provider);
        let now = Instant::now();
        for i in 0..MAX_ENTRIES + 10 {
            service.cache.insert(PathBuf::from(format!("/f{i}")), Cached { status: CloudStatus::Local, checked: now });
        }
        service.trim_cache();
        assert_eq!(service.cache.len(), MAX_ENTRIES);
    }

    #[gpui::test]
    fn a_fresh_placeholder_keeps_its_poll_scheduled(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let path = PathBuf::from("/placeholder");
            let provider = Arc::new(FakeProvider::new(&[]));
            let (mut service, _) = service(provider);
            service.visible.push(path.clone());
            service.cache.insert(path, Cached { status: CloudStatus::NotDownloaded, checked: Instant::now() });
            cx.new(|cx| {
                service.request_stale(cx);
                assert!(!service.batch_in_flight, "the placeholder is still fresh");
                assert!(service.poll_scheduled, "the next check is scheduled even if there is no work yet");
                service
            });
        });
    }

    #[gpui::test]
    fn scrolling_keeps_only_one_bounded_batch_in_flight(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let provider = Arc::new(FakeProvider::new(&[]));
            let (mut service, _) = service(provider);
            let paths: Vec<PathBuf> = (0..BATCH_SIZE + 1).map(|i| PathBuf::from(format!("/first/{i}"))).collect();
            cx.new(|cx| {
                service.observe((0, 0), &paths, cx);
                assert_eq!(service.pending.len(), BATCH_SIZE);
                service.observe((0, 0), &[PathBuf::from("/second/a")], cx);
                assert!(service.batch_in_flight);
                assert!(service.pending.is_empty(), "offscreen work is discarded and the next batch waits");
                service
            });
        });
    }

    #[test]
    fn late_reply_for_an_invalidated_path_is_dropped() {
        let provider = Arc::new(FakeProvider::new(&[]));
        let (mut service, _) = service(provider);
        let path = PathBuf::from("/dir/a");
        service.visible = vec![path.clone()];
        service.pending.insert(path.clone());
        service.versions.insert(path.clone(), 8);
        let stale = Request { path: path.clone(), version: 7 };
        assert!(!service.apply(&[stale], vec![CloudStatus::NotDownloaded]));
        assert_eq!(service.status(&path), None);
        assert!(service.pending.contains(&path), "the newer request is still in flight");

        let fresh = Request { path: path.clone(), version: 8 };
        assert!(service.apply(&[fresh], vec![CloudStatus::Downloaded]));
        assert_eq!(service.status(&path), Some(CloudStatus::Downloaded));
        assert!(!service.pending.contains(&path));
    }

    #[test]
    fn a_late_reply_for_a_does_not_touch_b() {
        let provider = Arc::new(FakeProvider::new(&[]));
        let (mut service, _) = service(provider);
        let a = PathBuf::from("/cloud/a");
        let b = PathBuf::from("/cloud/b");
        service.versions.insert(b.clone(), 1);
        service.pending.insert(b.clone());
        assert!(service.apply(&[Request { path: b.clone(), version: 1 }], vec![CloudStatus::Downloaded]));
        service.versions.insert(a.clone(), 1);
        service.pending.insert(a.clone());
        assert!(service.apply(&[Request { path: a, version: 1 }], vec![CloudStatus::NotDownloaded]));
        assert_eq!(service.status(&b), Some(CloudStatus::Downloaded), "B keeps its own state");
    }

    #[test]
    fn test_gate_blocks_placeholders_and_allows_downloaded() {
        let statuses = Arc::new(std::sync::Mutex::new(HashMap::new()));
        let gate = TestGate(statuses.clone());
        let path = Path::new("/cloud/note.txt");
        statuses.lock().unwrap().insert(path.to_path_buf(), CloudStatus::NotDownloaded);
        assert_eq!(gate.check(path, ReadHint::default()), ReadDecision::SkipCloud);
        statuses.lock().unwrap().insert(path.to_path_buf(), CloudStatus::Downloading);
        assert_eq!(gate.check(path, ReadHint::default()), ReadDecision::SkipCloud);
        statuses.lock().unwrap().insert(path.to_path_buf(), CloudStatus::Unknown);
        assert_eq!(gate.check(path, ReadHint::default()), ReadDecision::SkipAmbiguous);
        statuses.lock().unwrap().insert(path.to_path_buf(), CloudStatus::Downloaded);
        assert_eq!(gate.check(path, ReadHint::default()), ReadDecision::Readable);
        assert_eq!(gate.check(Path::new("/local/note.txt"), ReadHint::default()), ReadDecision::Readable);
    }
}
