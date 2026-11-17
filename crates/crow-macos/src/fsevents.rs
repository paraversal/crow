//! FSEvents history: folder-level events with IDs, replayed since a stored ID.
//!
//! The stream runs on its own dispatch queue and sends batches of events to a
//! channel. Nothing here touches the UI or the disk beyond FSEvents itself.

use std::ffi::{CStr, OsStr, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::Arc;
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
    // SAFETY: `info` is the `Arc<Callback>` pointer passed to FSEventStreamCreate.
    // The stream holds one reference until `FsEvents::drop`; this temporary clone
    // keeps the callback alive even if drop runs while it is on the queue.
    unsafe { Arc::increment_strong_count(info.cast::<Callback>()) };
    let callback = unsafe { Arc::from_raw(info.cast::<Callback>()) };
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
}

/// A running stream; dropping it stops delivery.
pub struct FsEvents {
    stream: objc2_core_services::FSEventStreamRef,
    _queue: DispatchRetained<DispatchQueue>,
    context: *mut c_void,
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
            drop(Arc::from_raw(self.context.cast::<Callback>()));
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
    let callback = Arc::new(Callback { on_events: Box::new(on_events) });
    let context = Arc::into_raw(callback) as *mut c_void;
    let mut stream_context = objc2_core_services::FSEventStreamContext {
        version: 0,
        info: context,
        retain: None,
        release: None,
        copyDescription: None,
    };
    let root = CFString::from_str("/");
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
    if stream.is_null() {
        // SAFETY: the stream did not take the context.
        unsafe { drop(Arc::from_raw(context.cast::<Callback>())) };
        return None;
    }
    let queue = DispatchQueue::new("crow.folder-sizes", None);
    // SAFETY: the stream is valid and the queue is retained by this struct.
    unsafe { FSEventStreamSetDispatchQueue(stream, Some(&queue)) };
    // SAFETY: the stream is valid and has a dispatch queue.
    if !unsafe { FSEventStreamStart(stream) } {
        // SAFETY: the stream is valid; it never started.
        unsafe {
            FSEventStreamInvalidate(stream);
            FSEventStreamRelease(stream);
            drop(Arc::from_raw(context.cast::<Callback>()));
        }
        return None;
    }
    Some(FsEvents { stream, _queue: queue, context })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crow_fs::sizes::{Cache, Measured};
    use std::path::Path;
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Instant;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("crow-macos-fsevents-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir.canonicalize().expect("canonical")
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
    fn a_volume_has_an_event_store_uuid() {
        use std::os::unix::fs::MetadataExt;
        let dev = std::fs::metadata("/").expect("root").dev();
        assert!(volume_uuid(dev).is_some(), "the boot volume has a history UUID");
    }

    #[test]
    fn an_event_marks_the_folder_and_its_ancestors_stale() {
        let root = temp_dir("live");
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
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_restart_replays_an_event_that_happened_while_off() {
        let root = temp_dir("restart");
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
        std::fs::remove_dir_all(&root).ok();
    }
}
