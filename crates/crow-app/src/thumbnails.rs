//! Bounded Quick Look thumbnails for the icon grid (shared with Gallery).
//!
//! The grid reports the keys it is showing; the service dedupes them, keeps at
//! most a small fixed number of requests in flight and caches the PNGs as GPUI
//! images. Everything that touches the disk or Quick Look runs off the main
//! thread. A reply is keyed by raw path, listing revision and pixel size, so a
//! renamed or replaced file never paints under its old thumbnail.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crow_fs::{ReadDecision, ReadGate, ReadHint};
use gpui::{App, AppContext as _, Context, Entity, Global, Image, ImageFormat};

/// Two Quick Look requests at once: each can wait up to two seconds.
const MAX_CONCURRENT: usize = 2;
/// Visible rows plus a little overscan; a fast scroll cannot grow the queue.
const MAX_QUEUE: usize = 96;
const MAX_ENTRIES: usize = 512;
const MAX_BYTES: usize = 48 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ThumbnailKey {
    pub path: PathBuf,
    /// `Folder::plugin_entry_version`: path, size, kind and modification time.
    pub revision: u64,
    pub points: u32,
}

impl ThumbnailKey {
    pub fn new(path: impl Into<PathBuf>, revision: u64, points: u32) -> Self {
        Self { path: path.into(), revision, points }
    }
}

/// One tile asking for a thumbnail. The gate is checked off the main thread,
/// before Quick Look is allowed to read the file.
pub struct ThumbnailRequest {
    pub key: ThumbnailKey,
    pub gate: Arc<dyn ReadGate>,
}

struct Job {
    key: ThumbnailKey,
    gate: Arc<dyn ReadGate>,
}

struct Cached {
    image: Option<Arc<Image>>,
    bytes: usize,
    used: u64,
}

#[derive(Default)]
struct Cache {
    entries: HashMap<ThumbnailKey, Cached>,
    total_bytes: usize,
    tick: u64,
}

impl Cache {
    fn image(&mut self, key: &ThumbnailKey) -> Option<Arc<Image>> {
        let entry = self.entries.get_mut(key)?;
        self.tick += 1;
        entry.used = self.tick;
        entry.image.clone()
    }

    fn insert(&mut self, key: ThumbnailKey, png: Option<Vec<u8>>) {
        self.tick += 1;
        let bytes = png.as_ref().map_or(0, Vec::len);
        let image = png.map(|png| Arc::new(Image::from_bytes(ImageFormat::Png, png)));
        if let Some(old) = self.entries.insert(key, Cached { image, bytes, used: self.tick }) {
            self.total_bytes = self.total_bytes.saturating_sub(old.bytes);
        }
        self.total_bytes += bytes;
        while self.entries.len() > MAX_ENTRIES || self.total_bytes > MAX_BYTES {
            let Some(oldest) = self.entries.iter().min_by_key(|(_, entry)| entry.used).map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(old) = self.entries.remove(&oldest) {
                self.total_bytes = self.total_bytes.saturating_sub(old.bytes);
            }
        }
    }
}

/// The platform call, behind a trait so tests never start Quick Look.
pub trait Generator: Send + Sync {
    fn thumbnail(&self, path: &Path, points: u32) -> Option<Vec<u8>>;
}

struct QuickLook;

impl Generator for QuickLook {
    fn thumbnail(&self, path: &Path, points: u32) -> Option<Vec<u8>> {
        // io-ok: the thumbnail service calls this from a background task.
        crow_macos::thumbnail_png(path, f64::from(points))
    }
}

/// A test generator that declines every request.
#[cfg(test)]
pub struct NoThumbnails;

#[cfg(test)]
impl Generator for NoThumbnails {
    fn thumbnail(&self, _: &Path, _: u32) -> Option<Vec<u8>> {
        None
    }
}

pub struct Thumbnails {
    cache: RefCell<Cache>,
    pending: HashSet<ThumbnailKey>,
    queue: VecDeque<Job>,
    running: usize,
    generator: Arc<dyn Generator>,
}

impl Thumbnails {
    fn new(generator: Arc<dyn Generator>) -> Self {
        Self {
            cache: RefCell::new(Cache::default()),
            pending: HashSet::new(),
            queue: VecDeque::new(),
            running: 0,
            generator,
        }
    }
}

impl Thumbnails {
    /// The cached image for a visible tile, if Quick Look already answered.
    pub fn image(&self, key: &ThumbnailKey) -> Option<Arc<Image>> {
        self.cache.borrow_mut().image(key)
    }

    /// Whether this revision already has an answer, including a miss. A miss is
    /// cached so a file Quick Look cannot render is not requested every frame.
    pub fn is_cached(&self, key: &ThumbnailKey) -> bool {
        self.cache.borrow().entries.contains_key(key)
    }

    /// Report the tiles on screen. Requests that left the screen are dropped
    /// from the queue; at most two jobs run and a bounded queue waits.
    pub fn request(&mut self, requests: Vec<ThumbnailRequest>, cx: &mut Context<Self>) {
        let wanted: HashSet<ThumbnailKey> = requests.iter().map(|request| request.key.clone()).collect();
        let dropped: Vec<ThumbnailKey> =
            self.queue.iter().filter(|job| !wanted.contains(&job.key)).map(|job| job.key.clone()).collect();
        self.queue.retain(|job| wanted.contains(&job.key));
        for key in dropped {
            self.pending.remove(&key);
        }
        for request in requests {
            if self.pending.contains(&request.key) {
                continue;
            }
            if self.queue.len() >= MAX_QUEUE {
                break;
            }
            self.pending.insert(request.key.clone());
            self.queue.push_back(Job { key: request.key, gate: request.gate });
        }
        self.pump(cx);
    }

    fn pump(&mut self, cx: &mut Context<Self>) {
        while self.running < MAX_CONCURRENT {
            let Some(job) = self.queue.pop_front() else { break };
            self.running += 1;
            let Job { key, gate } = job;
            let task_key = key.clone();
            let generator = self.generator.clone();
            cx.spawn(async move |this, cx| {
                let path = task_key.path.clone();
                let points = task_key.points;
                let png = cx
                    .background_spawn(async move {
                        if gate.check(&path, ReadHint::for_path(&path)) != ReadDecision::Readable {
                            return None;
                        }
                        generator.thumbnail(&path, points)
                    })
                    .await;
                this.update(cx, |this, cx| {
                    this.running = this.running.saturating_sub(1);
                    this.pending.remove(&task_key);
                    this.cache.borrow_mut().insert(task_key, png);
                    this.pump(cx);
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }
}

pub struct ThumbnailService(pub Entity<Thumbnails>);

impl Global for ThumbnailService {}

/// Install the app-wide cache once.
pub fn init(cx: &mut App) {
    install(cx, Arc::new(QuickLook));
}

/// Tests install a generator that declines, so no Quick Look runs in the suite.
#[cfg(test)]
pub fn init_for_tests(cx: &mut App) {
    install(cx, Arc::new(NoThumbnails));
}

fn install(cx: &mut App, generator: Arc<dyn Generator>) {
    if cx.try_global::<ThumbnailService>().is_some() {
        return;
    }
    let thumbnails = cx.new(|_| Thumbnails::new(generator));
    cx.set_global(ThumbnailService(thumbnails));
}

pub fn service(cx: &App) -> Option<Entity<Thumbnails>> {
    cx.try_global::<ThumbnailService>().map(|service| service.0.clone())
}

/// The shared gate for callers that have no cloud service (tests).
pub fn readable_gate(cx: &App) -> Arc<dyn ReadGate> {
    cx.try_global::<crate::cloud_status::CloudStatusService>()
        .map(|service| service.0.read(cx).gate())
        .unwrap_or_else(|| Arc::new(crow_fs::AllowAll))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str, revision: u64) -> ThumbnailKey {
        ThumbnailKey::new(Path::new(name), revision, 56)
    }

    #[test]
    fn cache_keeps_the_newest_entries_and_counts_bytes() {
        let mut cache = Cache::default();
        for i in 0..MAX_ENTRIES + 4 {
            cache.insert(key(&format!("{i}.txt"), 1), Some(vec![0; 16]));
        }
        assert!(cache.entries.len() <= MAX_ENTRIES);
        assert!(cache.total_bytes <= MAX_BYTES);
        assert!(cache.entries.contains_key(&key(&format!("{}.txt", MAX_ENTRIES + 3), 1)), "newest survives");
        assert!(!cache.entries.contains_key(&key("0.txt", 1)), "oldest is evicted");
    }

    #[test]
    fn a_new_revision_does_not_reuse_the_old_image() {
        let mut cache = Cache::default();
        cache.insert(key("a.txt", 1), Some(vec![1, 2, 3]));
        assert!(cache.image(&key("a.txt", 1)).is_some());
        assert!(cache.image(&key("a.txt", 2)).is_none());
    }

    struct Fake;

    impl Generator for Fake {
        fn thumbnail(&self, _: &std::path::Path, _: u32) -> Option<Vec<u8>> {
            Some(vec![1, 2, 3, 4])
        }
    }

    #[gpui::test]
    fn a_visible_request_is_deduped_and_cached(cx: &mut gpui::TestAppContext) {
        let service = cx.update(|cx| cx.new(|_| Thumbnails::new(Arc::new(Fake))));
        let wanted = key("a.txt", 7);
        let request = || ThumbnailRequest { key: wanted.clone(), gate: Arc::new(crow_fs::AllowAll) };
        service.update(cx, |service, cx| service.request(vec![request(), request()], cx));
        for _ in 0..200 {
            cx.run_until_parked();
            if cx.update(|cx| service.read(cx).image(&wanted)).is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(cx.update(|cx| service.read(cx).image(&wanted)).is_some(), "the request lands in the cache");
    }
}
