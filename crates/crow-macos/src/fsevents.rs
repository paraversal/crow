//! FSEvents history: folder-level events with IDs, replayed since a stored ID.
//!
//! The stream runs on its own dispatch queue and sends batches of events to a
//! channel. Nothing here touches the UI or the disk beyond FSEvents itself.

use std::ffi::{CStr, OsStr, c_void};
use std::os::unix::ffi::OsStrExt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use dispatch2::{DispatchQueue, DispatchRetained};
use objc2_core_foundation::{CFArray, CFString, CFUUID};
use objc2_core_services::{
    ConstFSEventStreamRef, FSEventStreamCreate, FSEventStreamCreateFlags, FSEventStreamEventFlags,
    FSEventStreamEventId, FSEventStreamInvalidate, FSEventStreamRelease, FSEventStreamSetDispatchQueue,
    FSEventStreamStart, FSEventStreamStop, FSEventsCopyUUIDForDevice, FSEventsGetCurrentEventId,
    kFSEventStreamCreateFlagWatchRoot, kFSEventStreamEventFlagEventIdsWrapped, kFSEventStreamEventFlagHistoryDone,
    kFSEventStreamEventFlagKernelDropped, kFSEventStreamEventFlagMustScanSubDirs, kFSEventStreamEventFlagRootChanged,
    kFSEventStreamEventFlagUserDropped,
};

/// One folder-level event: the folder that changed, its flags and its history id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub path: PathBuf,
    pub flags: EventFlags,
    pub id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventFlags(pub u32);

impl EventFlags {
    pub const MUST_SCAN_SUBDIRS: Self = Self(kFSEventStreamEventFlagMustScanSubDirs);
    pub const USER_DROPPED: Self = Self(kFSEventStreamEventFlagUserDropped);
    pub const KERNEL_DROPPED: Self = Self(kFSEventStreamEventFlagKernelDropped);
    pub const IDS_WRAPPED: Self = Self(kFSEventStreamEventFlagEventIdsWrapped);
    pub const HISTORY_DONE: Self = Self(kFSEventStreamEventFlagHistoryDone);
    pub const ROOT_CHANGED: Self = Self(kFSEventStreamEventFlagRootChanged);

    pub fn contains(self, flag: Self) -> bool {
        self.0 & flag.0 != 0
    }

    /// The kernel lost events: the subtree must be scanned.
    pub fn must_scan_subdirs(self) -> bool {
        self.contains(Self::MUST_SCAN_SUBDIRS)
    }

    pub fn history_done(self) -> bool {
        self.contains(Self::HISTORY_DONE)
    }

    pub fn root_changed(self) -> bool {
        self.contains(Self::ROOT_CHANGED)
    }

    /// Events were dropped or the ids wrapped; the history is no longer usable.
    pub fn dropped(self) -> bool {
        self.contains(Self::USER_DROPPED) || self.contains(Self::KERNEL_DROPPED) || self.contains(Self::IDS_WRAPPED)
    }
}

/// The current event id, for a first run or after the history was reset.
pub fn current_event_id() -> u64 {
    // SAFETY: reads a global event id; no pointers involved.
    unsafe { FSEventsGetCurrentEventId() }
}

/// The event-store UUID for `dev`; `None` when FSEvents has no history for it.
pub fn volume_uuid(dev: u64) -> Option<String> {
    let uuid = unsafe { FSEventsCopyUUIDForDevice(dev as i32) }?;
    let string = CFUUID::new_string(None, Some(&uuid))?;
    Some(string.to_string())
}

struct Callback {
    on_events: Box<dyn Fn(Vec<Event>) + Send + Sync>,
    logged_panic: AtomicBool,
}

unsafe extern "C-unwind" fn retain_context(info: *const c_void) -> *const c_void {
    // SAFETY: the creator's Arc is alive when CoreServices retains the context.
    unsafe { Arc::increment_strong_count(info.cast::<Callback>()) };
    info
}

unsafe extern "C-unwind" fn release_context(info: *const c_void) {
    // SAFETY: this balances the retain performed by CoreServices.
    unsafe { Arc::decrement_strong_count(info.cast::<Callback>()) };
}

unsafe extern "C-unwind" fn event_callback(
    _stream: ConstFSEventStreamRef,
    info: *mut c_void,
    num_events: usize,
    event_paths: NonNull<c_void>,
    event_flags: NonNull<FSEventStreamEventFlags>,
    event_ids: NonNull<FSEventStreamEventId>,
) {
    if info.is_null() || num_events == 0 {
        return;
    }
    // SAFETY: CoreServices retains the context through delivery of the callback.
    let callback = unsafe { &*info.cast::<Callback>() };
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: without `UseCFTypes`, `eventPaths` is a `char **` of `num_events`
        // NUL-terminated paths, and the flag/id arrays have the same length.
        let paths = event_paths.as_ptr().cast::<*const i8>();
        let flags = unsafe { std::slice::from_raw_parts(event_flags.as_ptr(), num_events) };
        let ids = unsafe { std::slice::from_raw_parts(event_ids.as_ptr(), num_events) };
        let mut events = Vec::with_capacity(num_events);
        for i in 0..num_events {
            let path = unsafe { *paths.add(i) };
            if path.is_null() {
                continue;
            }
            let path = unsafe { CStr::from_ptr(path) };
            events.push(Event {
                path: PathBuf::from(OsStr::from_bytes(path.to_bytes())),
                flags: EventFlags(flags[i]),
                id: ids[i],
            });
        }
        (callback.on_events)(events);
    }));
    if result.is_err() && !callback.logged_panic.swap(true, Ordering::Relaxed) {
        tracing::error!("FSEvents callback panicked");
    }
}

/// A running stream; dropping it stops delivery.
pub struct FsEvents {
    stream: objc2_core_services::FSEventStreamRef,
    _queue: DispatchRetained<DispatchQueue>,
}

// SAFETY: the stream and its queue are used only through the FSEvents API, which
// is thread-safe for stop/invalidate; the context is an Arc the callback shares.
unsafe impl Send for FsEvents {}

impl Drop for FsEvents {
    fn drop(&mut self) {
        // SAFETY: the stream came from FSEventStreamCreate and is still valid.
        unsafe {
            FSEventStreamStop(self.stream);
            FSEventStreamInvalidate(self.stream);
            FSEventStreamRelease(self.stream);
        }
    }
}

/// Watch the boot volume from `since`, delivering folder-level events after
/// `latency`. `None` when FSEvents cannot create or start the stream.
pub fn watch(
    since: u64,
    latency: Duration,
    on_events: impl Fn(Vec<Event>) + Send + Sync + 'static,
) -> Option<FsEvents> {
    watch_under("/", since, latency, on_events)
}

/// Watch a folder and its descendants. The stream reports changes to the root
/// itself as well as changes inside it.
pub fn watch_under(
    root: &str,
    since: u64,
    latency: Duration,
    on_events: impl Fn(Vec<Event>) + Send + Sync + 'static,
) -> Option<FsEvents> {
    let callback = Arc::new(Callback { on_events: Box::new(on_events), logged_panic: AtomicBool::new(false) });
    let mut stream_context = objc2_core_services::FSEventStreamContext {
        version: 0,
        info: Arc::as_ptr(&callback).cast_mut().cast(),
        retain: Some(retain_context),
        release: Some(release_context),
        copyDescription: None,
    };
    let root = CFString::from_str(root);
    let paths = CFArray::from_objects(&[&*root]);
    // SAFETY: a CFArray<CFString> is a CFArray; the stream only reads it.
    let paths: objc2_core_foundation::CFRetained<CFArray> =
        unsafe { objc2_core_foundation::CFRetained::cast_unchecked(paths) };
    let flags: FSEventStreamCreateFlags = kFSEventStreamCreateFlagWatchRoot;
    // SAFETY: the callback matches the expected signature, the context is valid
    // for the call, and `paths` holds one CFString.
    let stream = unsafe {
        FSEventStreamCreate(
            None,
            Some(event_callback),
            &mut stream_context,
            &paths,
            since,
            latency.as_secs_f64(),
            flags,
        )
    };
    // CoreServices keeps its own reference for the stream context.
    drop(callback);
    if stream.is_null() {
        return None;
    }
    let queue = DispatchQueue::new("crow.fsevents", None);
    // SAFETY: the stream is valid and the queue is retained by this struct.
    unsafe { FSEventStreamSetDispatchQueue(stream, Some(&queue)) };
    // SAFETY: the stream is valid and has a dispatch queue.
    if !unsafe { FSEventStreamStart(stream) } {
        // SAFETY: the stream is valid; it never started.
        unsafe {
            FSEventStreamInvalidate(stream);
            FSEventStreamRelease(stream);
        }
        return None;
    }
    Some(FsEvents { stream, _queue: queue })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crow_fs::sizes::{Cache, Measured};
    use std::path::Path;
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Instant;

    fn temp_dir(name: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::Builder::new().prefix(&format!("crow-macos-fsevents-{name}-")).tempdir().expect("temp dir");
        let path = dir.path().canonicalize().expect("canonical");
        std::fs::create_dir_all(&path).expect("create temp dir");
        (dir, path)
    }

    fn wait_for(tx: &Receiver<Vec<Event>>, path: &Path, timeout: Duration) -> Vec<Event> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let left = deadline.saturating_duration_since(Instant::now());
            let Ok(events) = tx.recv_timeout(left) else { break };
            // Another test's stream watches `/` too; only this temp tree counts.
            if events.iter().any(|event| event.path == path || event.path.starts_with(path)) {
                return events;
            }
        }
        Vec::new()
    }

    #[test]
    fn callback_context_is_dropped_once_after_delivering_events() {
        struct OnDrop(std::sync::mpsc::Sender<()>);

        impl Drop for OnDrop {
            fn drop(&mut self) {
                self.0.send(()).unwrap();
            }
        }

        let (_dir, root) = temp_dir("context");
        let (events_tx, events_rx) = channel();
        let (drop_tx, drop_rx) = channel();
        let on_drop = OnDrop(drop_tx);
        let events =
            watch_under(root.to_str().unwrap(), current_event_id(), Duration::from_millis(50), move |events| {
                let _keep_until_drop = &on_drop;
                events_tx.send(events).unwrap();
            })
            .expect("start stream");
        std::fs::write(root.join("first"), b"event").unwrap();
        assert!(!wait_for(&events_rx, &root, Duration::from_secs(5)).is_empty());
        assert_eq!(drop_rx.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty));
        drop(events);
        drop_rx.recv_timeout(Duration::from_secs(5)).expect("release callback");
        assert_eq!(drop_rx.recv_timeout(Duration::from_secs(5)), Err(std::sync::mpsc::RecvTimeoutError::Disconnected));
    }

    #[test]
    fn watchers_can_be_released_while_files_change() {
        let (_dir, root) = temp_dir("churn");
        std::thread::scope(|scope| {
            let (done, running) = channel::<()>();
            let (started, ready) = channel();
            let root = &root;
            scope.spawn(move || {
                let file = root.join("changing");
                std::fs::write(&file, b"first").unwrap();
                started.send(()).unwrap();
                while running
                    .recv_timeout(Duration::from_millis(1))
                    .is_err_and(|error| error == std::sync::mpsc::RecvTimeoutError::Timeout)
                {
                    std::fs::write(&file, b"changed").unwrap();
                }
            });
            ready.recv().unwrap();
            for _ in 0..200 {
                let watcher = watch_under(root.to_str().unwrap(), current_event_id(), Duration::from_millis(1), |_| {})
                    .expect("start stream");
                drop(watcher);
            }
            drop(done);
        });
    }

    #[test]
    fn a_panicking_callback_does_not_stop_delivery() {
        const CHILD: &str = "CROW_FSEVENTS_PANIC_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "fsevents::tests::a_panicking_callback_does_not_stop_delivery"])
                .env(CHILD, "1")
                .output()
                .expect("run callback test in a subprocess");
            assert!(
                output.status.success(),
                "callback subprocess exited with {}:\n{}\n{}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        let (_dir, root) = temp_dir("panic");
        let first = root.join("first");
        let second = root.join("second");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        let first_event = first.clone();
        let second_event = second.clone();
        let panicked = AtomicBool::new(false);
        let (panicked_tx, panicked_rx) = channel();
        let (delivered_tx, delivered_rx) = channel();
        let _watch =
            watch_under(root.to_str().unwrap(), current_event_id(), Duration::from_millis(50), move |events| {
                if events.iter().any(|event| event.path == first_event) && !panicked.swap(true, Ordering::Relaxed) {
                    panicked_tx.send(()).unwrap();
                    panic!("intentional FSEvents callback panic");
                }
                if events.iter().any(|event| event.path == second_event) {
                    delivered_tx.send(()).unwrap();
                }
            })
            .expect("start stream");
        std::fs::write(first.join("item"), b"first event").unwrap();
        panicked_rx.recv_timeout(Duration::from_secs(5)).expect("first delivery");
        std::fs::write(second.join("item"), b"second event").unwrap();
        delivered_rx.recv_timeout(Duration::from_secs(5)).expect("delivery after the panic");
    }

    #[test]
    fn a_volume_has_an_event_store_uuid() {
        use std::os::unix::fs::MetadataExt;
        let dev = std::fs::metadata("/").expect("root").dev();
        assert!(volume_uuid(dev).is_some(), "the boot volume has a history UUID");
    }

    #[test]
    fn an_event_marks_the_folder_and_its_ancestors_stale() {
        let (_root, root) = temp_dir("live");
        let deep = root.join("a/b");
        std::fs::create_dir_all(&deep).expect("create deep");
        let mut cache = Cache::default();
        for path in [&root, &root.join("a"), &deep] {
            cache.insert(path, 1, &Measured::default(), 0, 0);
        }
        let (tx, rx) = channel();
        let since = current_event_id();
        let _events = watch(since, Duration::from_millis(100), move |events| {
            tx.send(events).ok();
        })
        .expect("start stream");
        std::fs::write(deep.join("file.txt"), b"x").expect("write file");
        let events = wait_for(&rx, &deep, Duration::from_secs(5));
        assert!(!events.is_empty(), "an event for {deep:?}");
        for event in &events {
            cache.mark_stale(&event.path);
        }
        assert!(cache.get(&deep).expect("deep cached").stale, "the changed folder");
        assert!(cache.get(&root.join("a")).expect("a cached").stale, "its parent");
        assert!(cache.get(&root).expect("root cached").stale, "the root");
    }

    #[test]
    fn a_restart_replays_an_event_that_happened_while_off() {
        let (_root, root) = temp_dir("restart");
        let deep = root.join("a/b");
        std::fs::create_dir_all(&deep).expect("create deep");
        let since = current_event_id();
        std::fs::write(deep.join("file.txt"), b"x").expect("write file");
        let (tx, rx) = channel();
        let _events = watch(since, Duration::from_millis(100), move |events| {
            tx.send(events).ok();
        })
        .expect("start stream");
        assert!(!wait_for(&rx, &deep, Duration::from_secs(5)).is_empty(), "the history replays the write");
    }
}
