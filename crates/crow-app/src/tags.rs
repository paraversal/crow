//! Finder tags at the app boundary: the used-tag inventory in the sidebar and
//! the metadata reader the search engine and folder filter use. Discovery runs
//! off the main thread; tag destinations query Spotlight through `Effects`.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crow_core::normalize_tag;
use gpui::{App, Global, SharedString};

use crate::ui::Ui;

/// Stop after this many indexed paths when collecting used tag names.
pub const INVENTORY_PATH_CAP: usize = 4096;
/// A sidebar longer than this is not an inventory, it is a dump.
pub const INVENTORY_TAG_CAP: usize = 256;

/// The tags the sidebar can show: configured pinned tags first, in file order,
/// then discovered used tags in a stable case-insensitive order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagIndex {
    pub pinned: Vec<String>,
    pub used: Vec<String>,
    pub partial: bool,
    pub error: Option<SharedString>,
}

impl TagIndex {
    /// Every row, deduplicated by NFD identity, pinned order preserved.
    pub fn all(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        for tag in self.pinned.iter().chain(self.used.iter()) {
            let key = normalize_tag(tag);
            if !seen.contains(&key) {
                seen.push(key);
                out.push(tag.clone());
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.pinned.is_empty() && self.used.is_empty()
    }

    pub fn set_pinned(&mut self, tags: Vec<String>) {
        if self.pinned == tags {
            return;
        }
        self.pinned = tags;
    }

    fn apply(&mut self, discovery: TagDiscovery) {
        if self.used == discovery.tags && self.partial == discovery.partial && self.error.is_none() {
            return;
        }
        self.used = discovery.tags;
        self.partial = discovery.partial;
        self.error = discovery.error.map(SharedString::from);
    }

    /// The section title labels the limitation the APIs impose: Spotlight only
    /// indexes visible items, so hidden and unindexed tagged files cannot appear.
    pub fn section_title(&self) -> &'static str {
        if self.error.is_some() {
            "Tags (index unavailable)"
        } else if self.partial {
            "Tags (indexed, partial)"
        } else {
            "Tags (indexed)"
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagDiscovery {
    pub tags: Vec<String>,
    pub partial: bool,
    pub error: Option<String>,
}

/// The inventory seam: production asks Spotlight, tests answer directly.
pub trait TagInventory: Send + Sync {
    fn discover(&self, cancel: &AtomicBool) -> TagDiscovery;
}

pub struct SystemInventory;

impl TagInventory for SystemInventory {
    fn discover(&self, cancel: &AtomicBool) -> TagDiscovery {
        let mut names: BTreeMap<String, String> = BTreeMap::new();
        let mut paths = 0usize;
        let mut partial = false;
        let mut sink = |batch: Vec<crow_macos::SpotlightHit>| {
            for hit in batch {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                paths += 1;
                if paths > INVENTORY_PATH_CAP {
                    partial = true;
                    cancel.store(true, Ordering::Relaxed);
                    return;
                }
                if let Ok(tags) = crow_macos::tags_checked(&hit.path) {
                    for tag in tags {
                        names.entry(normalize_tag(&tag)).or_insert(tag);
                        if names.len() > INVENTORY_TAG_CAP {
                            partial = true;
                            cancel.store(true, Ordering::Relaxed);
                            return;
                        }
                    }
                }
            }
        };
        let result = crow_macos::spotlight_expression("kMDItemUserTags == \"*\"", &mut sink, cancel);
        let error = result.err();
        let mut tags: Vec<String> = names.into_values().collect();
        tags.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b)));
        TagDiscovery { tags, partial: partial || error.is_some(), error }
    }
}

/// Read tags for one path. Used by the search engine and folder tag filters.
pub struct SystemTags;

impl crow_search::TagReader for SystemTags {
    fn read_tags(&self, path: &Path) -> Result<Vec<String>, String> {
        crow_macos::tags_checked(path)
    }
}

pub fn reader() -> Arc<dyn crow_search::TagReader> {
    static READER: std::sync::LazyLock<Arc<SystemTags>> = std::sync::LazyLock::new(|| Arc::new(SystemTags));
    READER.clone()
}

/// The shortest time between two inventory scans. A scan walks Spotlight and
/// reads tags per hit, so a window gaining focus must not start one each time.
pub const REFRESH_COOLDOWN: Duration = Duration::from_secs(30);

/// The scan's own state: one scan at a time, never sooner than `cooldown`
/// after the last one started.
struct Refresh {
    inventory: Arc<dyn TagInventory>,
    cooldown: Duration,
    started: Option<Instant>,
    in_flight: bool,
}

impl Global for Refresh {}

/// Start the first inventory scan. Pinned tags are already in `Ui`; discovered
/// tags may take a moment and update the sidebar when they arrive.
pub fn init(cx: &mut App) {
    install(cx, Arc::new(SystemInventory), REFRESH_COOLDOWN);
    refresh_if_stale(cx);
}

fn install(cx: &mut App, inventory: Arc<dyn TagInventory>, cooldown: Duration) {
    cx.set_global(Refresh { inventory, cooldown, started: None, in_flight: false });
}

/// Rescan for tags created in Finder since the last scan, when the cooldown has
/// passed and none is running. Called when a window comes to the front and when
/// the tag picker opens; a no-op where the app has no inventory (tests).
pub fn refresh_if_stale(cx: &mut App) {
    let now = Instant::now();
    if !cx.has_global::<Refresh>() {
        return;
    }
    let refresh = cx.global_mut::<Refresh>();
    if refresh.in_flight || refresh.started.is_some_and(|started| now.duration_since(started) < refresh.cooldown) {
        return;
    }
    refresh.in_flight = true;
    refresh.started = Some(now);
    let inventory = refresh.inventory.clone();
    cx.spawn(async move |cx| {
        let cancel = AtomicBool::new(false);
        let discovery = cx.background_executor().spawn(async move { inventory.discover(&cancel) }).await;
        cx.update(|cx| {
            cx.global_mut::<Refresh>().in_flight = false;
            // `apply` replaces the discovered names; pinned tags live apart from them.
            cx.global_mut::<Ui>().tags.apply(discovery);
            cx.refresh_windows();
        });
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(Vec<String>);

    impl TagInventory for Fake {
        fn discover(&self, _: &AtomicBool) -> TagDiscovery {
            TagDiscovery { tags: self.0.clone(), partial: false, error: None }
        }
    }

    #[test]
    fn pinned_tags_come_first_and_duplicates_use_nfd_identity() {
        let mut index = TagIndex::default();
        index.set_pinned(vec!["Work".into(), "Café".into()]);
        index.apply(TagDiscovery { tags: vec!["Cafe\u{0301}".into(), "review".into()], partial: false, error: None });
        assert_eq!(index.all(), ["Work", "Café", "review"]);
    }

    #[test]
    fn a_discovery_error_is_visible_and_keeps_pinned_tags() {
        let mut index = TagIndex::default();
        index.set_pinned(vec!["Work".into()]);
        index.apply(TagDiscovery { tags: Vec::new(), partial: true, error: Some("Spotlight failed".into()) });
        assert_eq!(index.all(), ["Work"]);
        assert!(index.partial);
        assert_eq!(index.error.as_deref(), Some("Spotlight failed"));
    }

    #[test]
    fn discovery_goes_through_the_injected_provider() {
        let index = Fake(vec!["Work".into()]).discover(&AtomicBool::new(false));
        assert_eq!(index.tags, ["Work"]);
    }

    /// Answers each scan with the next scripted list and counts the scans.
    struct Scripted {
        answers: std::sync::Mutex<Vec<Vec<String>>>,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl TagInventory for Scripted {
        fn discover(&self, _: &AtomicBool) -> TagDiscovery {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let mut answers = self.answers.lock().unwrap();
            let tags = if answers.len() > 1 { answers.remove(0) } else { answers[0].clone() };
            TagDiscovery { tags, partial: false, error: None }
        }
    }

    #[gpui::test]
    fn a_new_finder_tag_appears_after_the_cooldown_and_pinned_tags_stay(cx: &mut gpui::TestAppContext) {
        cx.update(crate::ui::init_for_tests);
        let inventory = Arc::new(Scripted {
            answers: std::sync::Mutex::new(vec![vec!["review".into()], vec!["review".into(), "Fresh".into()]]),
            calls: Default::default(),
        });
        cx.update(|cx| {
            cx.global_mut::<Ui>().tags.set_pinned(vec!["Work".into()]);
            install(cx, inventory.clone(), Duration::from_millis(60));
            refresh_if_stale(cx);
        });
        cx.run_until_parked();
        assert_eq!(cx.read(|cx| cx.global::<Ui>().tags.all()), ["Work", "review"]);

        // Inside the cooldown: no second scan, however often a window activates.
        cx.update(|cx| {
            for _ in 0..5 {
                refresh_if_stale(cx);
            }
        });
        cx.run_until_parked();
        assert_eq!(inventory.calls.load(Ordering::Relaxed), 1, "the cooldown holds");

        std::thread::sleep(Duration::from_millis(80));
        cx.update(|cx| {
            // Two activations at once still start one scan.
            refresh_if_stale(cx);
            refresh_if_stale(cx);
        });
        cx.run_until_parked();
        assert_eq!(inventory.calls.load(Ordering::Relaxed), 2, "one scan after the cooldown");
        assert_eq!(cx.read(|cx| cx.global::<Ui>().tags.all()), ["Work", "review", "Fresh"], "the new tag merged");
    }
}
