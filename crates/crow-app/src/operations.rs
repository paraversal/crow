//! App-wide file operations: one serialized mutation coordinator, the session
//! transfer queue, the undo/redo stacks, the activity log and the file clipboard.
//! The engine itself (planning, safety rules, undo) lives in `crow_fs::ops`.
//!
//! Only the head job holds an engine [`Job`]. A queued request keeps its request
//! and a weak origin; planning runs when the request reaches the head, so work
//! from another job cannot invalidate an early conflict preview.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use crow_core::TabId;
use crow_fs::ops::{
    self, Conflict, ConflictKey, Job, JobEvent, JobId, JobOptions, Outcome, Plan, Progress, Record, Resolution, Trash,
};
use gpui::{App, Global, SharedString, WindowHandle, WindowId, prelude::*};

use crate::window::CrowWindow;

/// What dropping `paths` on the folder `into` does, like Finder: a move within a
/// volume, a copy across volumes; `copy` (Option held) forces a copy. Items already
/// in `into`, and folders dropped onto themselves or into their own contents, are
/// left out; `None` when nothing is left.
pub fn drop_request(paths: &[PathBuf], into: PathBuf, copy: bool) -> Option<ops::Request> {
    let paths: Vec<PathBuf> = paths.iter().filter(|p| drop_moves(p, &into)).cloned().collect();
    if paths.is_empty() {
        return None;
    }
    let same_volume = paths.iter().all(|p| same_volume(p, &into));
    let paths: Arc<[PathBuf]> = paths.into();
    Some(if same_volume && !copy { ops::Request::Move { paths, into } } else { ops::Request::Copy { paths, into } })
}

/// Whether dropping `paths` into `into` would do anything: not every item is already
/// there, and none is `into` itself or a folder above it.
pub fn drop_would_act(paths: &[PathBuf], into: &Path) -> bool {
    paths.iter().any(|path| drop_moves(path, into))
}

fn drop_moves(path: &Path, into: &Path) -> bool {
    path.parent() != Some(into) && !into.starts_with(path)
}

fn same_volume(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    // io-ok: only called from drop_request, which the drop handlers spawn in the background
    match (a.symlink_metadata(), b.metadata()) {
        (Ok(x), Ok(y)) => x.dev() == y.dev(),
        _ => false,
    }
}

/// A stable id for one accepted request, from submission to its terminal entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RequestId(pub u64);

/// Where a request came from. Every callback through this origin is weak: a
/// closed window or a tab that has moved on is simply left alone.
#[derive(Clone)]
pub struct Origin {
    pub window: WindowHandle<CrowWindow>,
    pub tab: TabId,
    pub dir: PathBuf,
    pub surface: crate::plugins::SurfaceId,
}

impl Origin {
    /// Run `f` in the origin window only while its tab still shows the folder it
    /// showed when the request was submitted.
    fn with_current(
        &self,
        cx: &mut App,
        f: impl FnOnce(&mut CrowWindow, &mut gpui::Window, &mut gpui::Context<CrowWindow>),
    ) {
        self.window
            .update(cx, |this, window, cx| {
                if this.origin_is_current(self, cx) {
                    f(this, window, cx);
                }
            })
            .ok();
    }
}

/// The head's state. Terminal outcomes live in [`Operations::log`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobPhase {
    Queued,
    Planning,
    Running,
    PauseRequested,
    Paused,
    AwaitingConflict,
    Cancelling,
}

impl JobPhase {
    /// Queued work, planning and an active transfer are still moving; a paused or
    /// conflict-waiting head is stable until the user acts.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn is_settling(self) -> bool {
        matches!(self, JobPhase::Planning | JobPhase::Running | JobPhase::PauseRequested | JobPhase::Cancelling)
    }
}

pub struct QueuedJob {
    pub id: RequestId,
    pub request: ops::Request,
    pub label: String,
    pub origin: Option<Origin>,
}

pub struct ActiveJob {
    pub id: RequestId,
    pub request: ops::Request,
    pub label: String,
    pub origin: Option<Origin>,
    pub phase: JobPhase,
    pub progress: Progress,
    pub started: Instant,
    pub pause_supported: bool,
    pub conflict: Option<Conflict>,
    engine_id: Option<JobId>,
    engine: Option<Job>,
    plan_cancel: Arc<AtomicBool>,
}

impl ActiveJob {
    fn new(queued: QueuedJob) -> Self {
        Self {
            id: queued.id,
            label: queued.label,
            request: queued.request,
            origin: queued.origin,
            phase: JobPhase::Queued,
            progress: Progress::default(),
            started: Instant::now(),
            pause_supported: false,
            conflict: None,
            engine_id: None,
            engine: None,
            plan_cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    fn view(&self, queued: usize) -> Running {
        Running {
            label: self.label.clone(),
            progress: self.progress.clone(),
            queued,
            planning: self.phase == JobPhase::Planning,
            pausing: self.phase == JobPhase::PauseRequested,
            paused: self.phase == JobPhase::Paused,
            waiting: self.phase == JobPhase::AwaitingConflict,
        }
    }
}

/// One window-independent conflict question. `host` is the one window currently
/// showing it; dismissing the sheet clears it without answering the job.
#[derive(Clone)]
pub struct Prompt {
    pub job: RequestId,
    pub conflict: Conflict,
    pub host: Option<WindowId>,
}

impl Prompt {
    pub fn key(&self) -> ConflictKey {
        self.conflict.key
    }
}

/// The view the bottom bar and Activity draw for the active job.
pub struct Running {
    pub label: String,
    pub progress: Progress,
    pub queued: usize,
    pub planning: bool,
    pub pausing: bool,
    pub paused: bool,
    pub waiting: bool,
}

/// A non-transfer mutation occupying the single engine slot. These cannot be
/// cancelled (undo is a short disk pass); the queue waits for them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Maintenance {
    Undo,
    Redo,
    EmptyTrash,
}

#[derive(Clone)]
pub struct LogEntry {
    pub record: Arc<Record>,
    pub at: SystemTime,
    pub error: Option<String>,
    pub cancelled: bool,
    pub undone: bool,
}

/// What a confirmed stop should do after the terminal cleanup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopIntent {
    Quit,
    Close,
}

/// What `Cmd-C` / `Cmd-X` put on the clipboard. The paths also go on the system
/// pasteboard; `change_count` tells whether it still holds them (or someone copied since).
pub struct Clipboard {
    pub paths: Arc<[PathBuf]>,
    pub cut: bool,
    pub change_count: Option<isize>,
    /// When a copy's tint pulse started; `None` for a cut.
    pub flash: Option<Instant>,
    /// The cut rows still show their mark. Cleared by Escape, undo, a paste or a
    /// new copy/cut.
    pub marked: bool,
    /// The paths as a set: a visible row is one lookup, never a 100k scan.
    set: HashSet<PathBuf>,
}

impl Clipboard {
    pub fn new(paths: Arc<[PathBuf]>, cut: bool, change_count: Option<isize>) -> Self {
        let set = paths.iter().cloned().collect();
        Self { paths, cut, change_count, flash: (!cut).then(Instant::now), marked: cut, set }
    }

    /// Whether `path` is on the clipboard.
    pub fn contains(&self, path: &Path) -> bool {
        self.set.contains(path)
    }
}

/// What a paste would bring in, and whether it moves them: our clipboard if the
/// pasteboard still holds what we put there; otherwise whatever another app (Finder) copied.
pub fn pasteboard_paths(cx: &App) -> (Arc<[PathBuf]>, bool) {
    let effects = crate::effects::effects(cx);
    let ours =
        cx.global::<Operations>().clipboard.as_ref().filter(|c| c.change_count == effects.pasteboard_change_count());
    match ours {
        Some(c) => (c.paths.clone(), c.cut),
        None => (effects.read_file_urls().into(), false),
    }
}

pub struct Operations {
    trash: Arc<dyn Trash>,
    pub(crate) undo_stack: Vec<usize>,
    redo_stack: Vec<Arc<Record>>,
    pub log: Vec<LogEntry>,
    /// The request at the head of the session queue, if any.
    pub active: Option<ActiveJob>,
    /// Transfers waiting for the head to finish, in submission order.
    pub queue: VecDeque<QueuedJob>,
    /// The one conflict question, independent of any window.
    pub prompt: Option<Prompt>,
    /// Undo, redo or emptying the Trash owns the engine slot with the head.
    pub maintenance: Option<Maintenance>,
    /// A confirmed stop: no further queued request starts.
    pub stopping: bool,
    next_request: u64,
    pub clipboard: Option<Clipboard>,
    /// The latest result or error, for the status line.
    pub message: Option<(SharedString, bool)>,
    /// Bumped by every message, so an old timer cannot clear a newer one and the
    /// bar can fade in a new one.
    pub message_generation: u64,
    /// The fraction a finished operation's bar shows while it fades, and its id.
    pub progress_done: Option<(f32, u64)>,
    progress_generation: u64,
    /// Fuzz runs only: every path an operation touches must be under this
    /// folder. Anything else is refused and recorded in `refused`.
    #[cfg(any(test, feature = "fuzz"))]
    pub jail: Option<PathBuf>,
    #[cfg(any(test, feature = "fuzz"))]
    pub refused: Vec<PathBuf>,
    /// Tests hold planning so a queue can be inspected deterministically.
    #[cfg(test)]
    pub test_hold_planning: bool,
}

impl Operations {
    /// The fuzz tree every path must stay under; production has no jail.
    pub fn jail(&self) -> Option<&std::path::Path> {
        #[cfg(any(test, feature = "fuzz"))]
        {
            self.jail.as_deref()
        }
        #[cfg(not(any(test, feature = "fuzz")))]
        {
            None
        }
    }

    /// Nothing is planning, running, queued or maintaining the engine.
    pub fn is_idle(&self) -> bool {
        self.active.is_none() && self.queue.is_empty() && self.maintenance.is_none()
    }

    /// Any work a quit or a last-window close must not cut off silently.
    pub fn has_pending(&self) -> bool {
        !self.is_idle() || self.prompt.is_some()
    }

    pub fn has_active(&self) -> bool {
        self.active.is_some()
    }

    /// Whether a UI settle should keep waiting: a moving job or maintenance work.
    #[cfg(any(test, feature = "fuzz"))]
    pub fn is_settling(&self) -> bool {
        self.maintenance.is_some() || self.active.as_ref().is_some_and(|active| active.phase.is_settling())
    }

    pub fn queued_count(&self) -> usize {
        self.queue.len()
    }

    pub fn running(&self) -> Option<Running> {
        self.active.as_ref().map(|active| active.view(self.queue.len()))
    }

    pub fn active_id(&self) -> Option<RequestId> {
        self.active.as_ref().map(|active| active.id)
    }

    /// The paths Cmd-C/Cmd-X put on the clipboard, for the fuzz runner's bulk tag.
    #[cfg(feature = "fuzz")]
    pub fn clipboard_paths(&self) -> usize {
        self.clipboard.as_ref().map_or(0, |clipboard| clipboard.paths.len())
    }

    /// The paths the next undo would touch.
    #[cfg(feature = "fuzz")]
    pub fn undo_paths(&self) -> usize {
        self.undo_stack.last().and_then(|index| self.log.get(*index)).map_or(0, |entry| entry.record.steps.len())
    }

    /// The paths the next redo would touch.
    #[cfg(feature = "fuzz")]
    pub fn redo_paths(&self) -> usize {
        self.redo_stack.last().map_or(0, |record| record.steps.len())
    }
}

/// Info messages clear after this long; errors stay until something replaces them.
const MESSAGE_LIFETIME: Duration = Duration::from_secs(5);

impl Global for Operations {}

/// Show a message in the bar. Info messages retire after five seconds (Q2); errors
/// stay until the next operation, navigation or a fixed config file.
pub fn message(text: impl Into<SharedString>, is_error: bool, cx: &mut App) {
    // Errors name the file at fault; GPUI's text shaping rejects a newline in a name.
    let text: SharedString = text.into();
    let text: SharedString = crate::style::display_text(&text).into_owned().into();
    let generation = {
        let ops = cx.global_mut::<Operations>();
        ops.message_generation += 1;
        ops.message = Some((text, is_error));
        ops.message_generation
    };
    if is_error {
        return;
    }
    cx.spawn(async move |cx| {
        cx.background_executor().timer(MESSAGE_LIFETIME).await;
        cx.update(|cx| {
            let ops = cx.global_mut::<Operations>();
            if ops.message_generation == generation {
                ops.message = None;
                cx.refresh_windows();
            }
        });
    })
    .detach();
}

/// Drop the message now; a pending info timer becomes a no-op.
pub fn clear_message(cx: &mut App) {
    let ops = cx.global_mut::<Operations>();
    ops.message_generation += 1;
    ops.message = None;
}

pub fn init(cx: &mut App, trash: Arc<dyn Trash>) {
    cx.set_global(Operations {
        trash,
        undo_stack: Vec::new(),
        redo_stack: Vec::new(),
        log: Vec::new(),
        active: None,
        queue: VecDeque::new(),
        prompt: None,
        maintenance: None,
        stopping: false,
        next_request: 0,
        clipboard: None,
        message: None,
        message_generation: 0,
        progress_done: None,
        progress_generation: 0,
        #[cfg(any(test, feature = "fuzz"))]
        jail: None,
        #[cfg(any(test, feature = "fuzz"))]
        refused: Vec::new(),
        #[cfg(test)]
        test_hold_planning: false,
    });
    cx.on_window_closed(|cx, id| window_closed(id, cx)).detach();
}

/// Every path `plan` reads or writes: the request's own and each item's.
#[cfg(any(test, feature = "fuzz"))]
fn plan_paths(plan: &Plan) -> Vec<&Path> {
    use ops::Request;
    let mut out: Vec<&Path> = match &plan.request {
        Request::NewFolder { parent } | Request::NewFile { parent } => vec![parent],
        Request::Rename { path, .. } => vec![path],
        Request::Duplicate { paths } | Request::Trash { paths } | Request::DeletePermanently { paths } => {
            paths.iter().map(PathBuf::as_path).collect()
        }
        Request::Copy { paths, into } | Request::Move { paths, into } => {
            paths.iter().map(PathBuf::as_path).chain(std::iter::once(into.as_path())).collect()
        }
    };
    for item in &plan.items {
        out.extend(item.source.iter().map(PathBuf::as_path));
        out.push(item.target.as_path());
    }
    out
}

/// The first path of `plan` outside the fuzz jail, if one is set.
#[cfg(any(test, feature = "fuzz"))]
fn outside_jail(plan: &Plan, cx: &App) -> Option<PathBuf> {
    let jail = cx.global::<Operations>().jail.clone()?;
    plan_paths(plan).into_iter().find(|path| !path.starts_with(&jail)).map(Path::to_path_buf)
}

/// The plan's label in the progressive form the bar shows: "Copy 3 items" → "Copying 3 items".
pub fn running_label(label: &str) -> String {
    let (verb, rest) = label.split_once(' ').unwrap_or((label, ""));
    let ing = match verb {
        "Copy" => "Copying",
        "Move" => "Moving",
        "Duplicate" => "Duplicating",
        "Delete" => "Deleting",
        _ => verb,
    };
    let label = if rest.is_empty() { ing.to_owned() } else { format!("{ing} {rest}") };
    // A rename label names its files; a newline there reaches GPUI's text shaping.
    crate::style::display_text(&label).into_owned()
}

/// A label for a request that has not reached the planning head yet.
pub fn request_label(request: &ops::Request) -> String {
    fn count(n: usize) -> String {
        if n == 1 { "1 item".to_owned() } else { format!("{n} items") }
    }
    fn dir_name(path: &Path) -> String {
        path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into())
    }
    match request {
        ops::Request::NewFolder { .. } => "New Folder".to_owned(),
        ops::Request::NewFile { .. } => "New File".to_owned(),
        ops::Request::Rename { path, new_name } => {
            let name = path.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
            format!("Rename “{name}” to “{new_name}”")
        }
        ops::Request::Duplicate { paths } => format!("Duplicate {}", count(paths.len())),
        ops::Request::Copy { paths, into } => format!("Copy {} to {}", count(paths.len()), dir_name(into)),
        ops::Request::Move { paths, into } => format!("Move {} to {}", count(paths.len()), dir_name(into)),
        ops::Request::Trash { paths } => format!("Move {} to Trash", count(paths.len())),
        ops::Request::DeletePermanently { paths } => format!("Delete {} permanently", count(paths.len())),
    }
}

fn is_transfer(request: &ops::Request) -> bool {
    matches!(request, ops::Request::Copy { .. } | ops::Request::Move { .. } | ops::Request::Duplicate { .. })
}

/// Accept `request` into the session queue. Transfers append in submission
/// order; any other mutation is refused while the coordinator is busy, so the
/// existing one-at-a-time undo ordering is preserved.
pub fn enqueue(request: ops::Request, origin: Option<Origin>, cx: &mut App) {
    let ops = cx.global::<Operations>();
    if ops.stopping {
        return;
    }
    if !is_transfer(&request) && !ops.is_idle() {
        message("Another operation is still running", true, cx);
        cx.refresh_windows();
        return;
    }
    let id = {
        let ops = cx.global_mut::<Operations>();
        ops.next_request += 1;
        let id = RequestId(ops.next_request);
        ops.queue.push_back(QueuedJob { id, label: request_label(&request), request, origin });
        id
    };
    tracing::info!(request = %id.0, "request queued");
    cx.refresh_windows();
    start_next(cx);
}

/// Start the head's planning pass. Only one plan or engine job runs at a time.
fn start_next(cx: &mut App) {
    let (id, request, cancel) = {
        let ops = cx.global_mut::<Operations>();
        if ops.stopping || ops.active.is_some() || ops.maintenance.is_some() {
            return;
        }
        let Some(queued) = ops.queue.pop_front() else { return };
        let mut active = ActiveJob::new(queued);
        active.phase = JobPhase::Planning;
        let id = active.id;
        let request = active.request.clone();
        let cancel = active.plan_cancel.clone();
        ops.active = Some(active);
        (id, request, cancel)
    };
    cx.refresh_windows();
    #[cfg(test)]
    if cx.global::<Operations>().test_hold_planning {
        return;
    }
    cx.spawn(async move |cx| {
        let planned = cx.background_spawn(async move { ops::plan_cancellable(request, &cancel) }).await;
        cx.update(|cx| finish_plan(id, planned, cx));
    })
    .detach();
}

/// Tests only: release a head held in Planning.
#[cfg(test)]
pub fn release_planning(cx: &mut App) {
    let (id, request, cancel) = {
        let ops = cx.global_mut::<Operations>();
        ops.test_hold_planning = false;
        let Some(active) = &ops.active else { return };
        (active.id, active.request.clone(), active.plan_cancel.clone())
    };
    cx.spawn(async move |cx| {
        let planned = cx.background_spawn(async move { ops::plan_cancellable(request, &cancel) }).await;
        cx.update(|cx| finish_plan(id, planned, cx));
    })
    .detach();
}

fn finish_plan(id: RequestId, planned: Result<Plan, ops::OpError>, cx: &mut App) {
    let Some(active) = cx.global::<Operations>().active.as_ref().filter(|active| active.id == id) else { return };
    if active.phase == JobPhase::Cancelling {
        let label = active.label.clone();
        finish_without_job(id, label, None, true, cx);
        return;
    }
    match planned {
        Err(error) => {
            let label = cx.global::<Operations>().active.as_ref().map_or_else(String::new, |a| a.label.clone());
            let cancelled =
                cx.global::<Operations>().active.as_ref().is_some_and(|a| a.plan_cancel.load(Ordering::Relaxed));
            finish_without_job(id, label, (!cancelled).then_some(error.to_string()), cancelled, cx);
        }
        Ok(plan) => {
            #[cfg(any(test, feature = "fuzz"))]
            if let Some(path) = outside_jail(&plan, cx) {
                cx.global_mut::<Operations>().refused.push(path.clone());
                let error = format!("Refused outside the fuzz tree: {}", path.display());
                finish_without_job(id, plan.label.clone(), Some(error), false, cx);
                return;
            }
            if plan.items.is_empty() {
                // Nothing to do (every item was already there).
                if let Some(active) = cx.global_mut::<Operations>().active.take_if(|a| a.id == id) {
                    tracing::info!(request = %active.id.0, "request did nothing");
                }
                start_next(cx);
                cx.refresh_windows();
                return;
            }
            {
                let ops = cx.global_mut::<Operations>();
                let Some(active) = ops.active.as_mut().filter(|active| active.id == id) else { return };
                active.label = plan.label.clone();
                active.progress.total_items = plan.items.len();
                active.progress.total_bytes = plan.total_bytes;
            }
            launch_job(id, plan, cx);
        }
    }
}

fn launch_job(id: RequestId, plan: Plan, cx: &mut App) {
    let (trash, origin, request, total_items, total_bytes) = {
        let ops = cx.global::<Operations>();
        let Some(active) = ops.active.as_ref() else { return };
        (
            ops.trash.clone(),
            active.origin.clone(),
            active.request.clone(),
            active.progress.total_items,
            active.progress.total_bytes,
        )
    };
    let options = JobOptions::default();
    // The worker is a plain std thread; its wakeups must not reach GPUI's
    // scheduler (the test scheduler asserts on off-thread scheduling). Drain a
    // std channel from a UI task instead; one timer runs only while a job does.
    let (tx, rx) = std::sync::mpsc::channel::<JobEvent>();
    let spawned = Job::spawn(plan, options, trash, move |event| {
        tx.send(event).ok();
    });
    let job = match spawned {
        Ok(job) => job,
        Err(error) => {
            let label = cx.global::<Operations>().active.as_ref().map_or_else(String::new, |a| a.label.clone());
            finish_without_job(id, label, Some(error.to_string()), false, cx);
            return;
        }
    };
    let engine_id = job.id();
    {
        let ops = cx.global_mut::<Operations>();
        let Some(active) = ops.active.as_mut().filter(|active| active.id == id) else { return };
        active.engine_id = Some(engine_id);
        active.engine = Some(job);
        active.phase = JobPhase::Running;
        active.pause_supported = active.engine.as_ref().is_some_and(Job::pause_supported);
        active.progress.done_items = 0;
        active.progress.done_bytes = 0;
        active.progress.total_items = total_items;
        active.progress.total_bytes = total_bytes;
        ops.message_generation += 1;
        ops.message = None;
    }
    tracing::info!(request = %id.0, engine = %engine_id, "transfer started");
    if let Some(origin) = origin {
        origin.with_current(cx, |this, _window, cx| this.job_started(&request, cx));
    }
    cx.refresh_windows();
    cx.spawn(async move |cx| {
        loop {
            let mut disconnected = false;
            loop {
                match rx.try_recv() {
                    Ok(event) => {
                        let terminal = matches!(event, JobEvent::Terminal { .. });
                        cx.update(|cx| handle_event(id, event, cx));
                        if terminal {
                            return;
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
            if disconnected {
                // The worker ended without its Terminal event (a panic, say).
                // Finish the head so the queue and quit cannot wait forever.
                cx.update(|cx| worker_disconnected(id, cx));
                return;
            }
            cx.background_executor().timer(Duration::from_millis(16)).await;
        }
    })
    .detach();
}

fn worker_disconnected(id: RequestId, cx: &mut App) {
    let Some(active) = cx.global::<Operations>().active.as_ref().filter(|active| active.id == id) else { return };
    let label = active.label.clone();
    let outcome = Arc::new(Outcome {
        record: Record { label, steps: Vec::new() },
        error: Some(ops::OpError::Io(
            PathBuf::from("copy job"),
            Arc::new(std::io::Error::other("the copy worker stopped without reporting")),
        )),
        cancelled: false,
    });
    finish_job(id, outcome, cx);
}

fn handle_event(id: RequestId, event: JobEvent, cx: &mut App) {
    enum FollowUp {
        None,
        Prompt,
        Terminal(Arc<Outcome>),
    }
    let mut follow_up = FollowUp::None;
    {
        let ops = cx.global_mut::<Operations>();
        let Some(active) = ops.active.as_mut().filter(|active| active.id == id) else { return };
        let engine_matches = active.engine_id;
        match event {
            JobEvent::Started { job_id, total_items, total_bytes, pause_supported }
                if engine_matches == Some(job_id) =>
            {
                if total_items > 0 {
                    active.progress.total_items = total_items;
                }
                if total_bytes > 0 {
                    active.progress.total_bytes = total_bytes;
                }
                active.pause_supported = pause_supported;
            }
            JobEvent::Progress { job_id, progress } if engine_matches == Some(job_id) => {
                if active.phase != JobPhase::AwaitingConflict {
                    active.progress = progress;
                }
            }
            JobEvent::PauseAcknowledged { job_id, progress } if engine_matches == Some(job_id) => {
                if active.phase == JobPhase::PauseRequested {
                    active.progress = progress;
                    active.phase = JobPhase::Paused;
                }
            }
            JobEvent::Conflict(conflict)
                if engine_matches == Some(conflict.key.job_id) && active.phase != JobPhase::Cancelling =>
            {
                if let Some(choice) = conflict.auto {
                    // An apply-to-all policy answered this one; the engine moves on.
                    tracing::debug!(job = %conflict.key.job_id, ?choice, "conflict answered by policy");
                } else {
                    active.phase = JobPhase::AwaitingConflict;
                    active.conflict = Some(conflict.clone());
                    ops.prompt = Some(Prompt { job: id, conflict, host: None });
                    follow_up = FollowUp::Prompt;
                }
            }
            JobEvent::Terminal { job_id, outcome } if engine_matches == Some(job_id) => {
                active.engine.take();
                follow_up = FollowUp::Terminal(outcome);
            }
            _ => {}
        }
    }
    match follow_up {
        FollowUp::None => cx.refresh_windows(),
        FollowUp::Prompt => {
            present_prompt(cx);
            cx.refresh_windows();
        }
        FollowUp::Terminal(outcome) => finish_job(id, outcome, cx),
    }
}

fn finish_job(id: RequestId, outcome: Arc<Outcome>, cx: &mut App) {
    let Some(mut active) = cx.global_mut::<Operations>().active.take_if(|active| active.id == id) else { return };
    let Outcome { record, error, cancelled } = (*outcome).clone();
    let record = Arc::new(record);
    let shown = active.started.elapsed() >= Duration::from_secs(1);
    let fraction = fraction_of(&active.progress);
    let label = record.label.clone();
    let event = crate::feedback::event_for(&active.request);
    let results = record.results();
    let rename = matches!(active.request, ops::Request::NewFolder { .. } | ops::Request::NewFile { .. });
    let prompt_cleared = cx.global::<Operations>().prompt.as_ref().is_some_and(|prompt| prompt.job == id);
    if prompt_cleared {
        cx.global_mut::<Operations>().prompt = None;
        dismiss_prompt_sheets(None, cx);
    }
    let fade = {
        let ops = cx.global_mut::<Operations>();
        tracing::info!(request = %id.0, steps = record.steps.len(), cancelled, "request finished");
        ops.log.push(LogEntry {
            record: record.clone(),
            at: SystemTime::now(),
            error: error.as_ref().map(ToString::to_string),
            cancelled,
            undone: false,
        });
        if record.is_undoable() {
            ops.undo_stack.push(ops.log.len() - 1);
        }
        ops.redo_stack.clear();
        shown.then(|| {
            ops.progress_generation += 1;
            let generation = ops.progress_generation;
            ops.progress_done = Some((fraction, generation));
            generation
        })
    };
    if let Some(generation) = fade {
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_millis(crow_core::motion::ms::COPY_FADE)).await;
            cx.update(|cx| {
                let ops = cx.global_mut::<Operations>();
                if ops.progress_generation == generation {
                    ops.progress_done = None;
                    cx.refresh_windows();
                }
            });
        })
        .detach();
    }
    match (&error, cancelled) {
        (Some(e), _) => {
            crate::feedback::play(crow_core::motion::Event::Refuse, cx);
            message(e.to_string(), true, cx);
        }
        (None, true) => message(format!("{label}: cancelled"), false, cx),
        (None, false) => {
            if let Some(event) = event
                && !record.steps.is_empty()
                && (event != crow_core::motion::Event::CopyDone || shown)
            {
                crate::feedback::play(event, cx);
            }
            clear_message(cx);
        }
    }
    if let Some(origin) = active.origin.take() {
        let failed = error.is_some();
        origin.with_current(cx, |this, _window, cx| {
            this.job_finished(&active.request, results, rename, failed, cx);
        });
    }
    crate::plugins::notify_operation_done(cx);
    crate::ui::refresh_volumes(cx);
    start_next(cx);
    cx.refresh_windows();
}

/// Finish a request that never had an engine job: a plan error, a jail
/// refusal, a spawn failure or a cancellation before the first item.
fn finish_without_job(id: RequestId, label: String, error: Option<String>, cancelled: bool, cx: &mut App) {
    let Some(_active) = cx.global_mut::<Operations>().active.take_if(|active| active.id == id) else { return };
    let prompt_cleared = cx.global::<Operations>().prompt.as_ref().is_some_and(|prompt| prompt.job == id);
    if prompt_cleared {
        cx.global_mut::<Operations>().prompt = None;
        dismiss_prompt_sheets(None, cx);
    }
    if error.is_some() || cancelled {
        let record = Arc::new(Record { label: label.clone(), steps: Vec::new() });
        let ops = cx.global_mut::<Operations>();
        ops.log.push(LogEntry { record, at: SystemTime::now(), error: error.clone(), cancelled, undone: false });
    }
    match error {
        Some(error) => {
            crate::feedback::play(crow_core::motion::Event::Refuse, cx);
            message(error, true, cx);
        }
        None if cancelled => message(format!("{label}: cancelled"), false, cx),
        None => {}
    }
    crate::plugins::notify_operation_done(cx);
    start_next(cx);
    cx.refresh_windows();
}

fn fraction_of(progress: &Progress) -> f32 {
    let (done, total) = if progress.total_bytes > 0 {
        (progress.done_bytes, progress.total_bytes)
    } else {
        (progress.done_items as u64, progress.total_items as u64)
    };
    if total == 0 { 0.0 } else { (done as f32 / total as f32).clamp(0.0, 1.0) }
}

/// Request a pause for the active transfer. The bar shows "Pausing…" until the
/// worker acknowledges at a real safe point.
pub fn pause_active(cx: &mut App) {
    let ops = cx.global_mut::<Operations>();
    let Some(active) = ops.active.as_mut() else { return };
    if active.phase != JobPhase::Running || !active.pause_supported {
        return;
    }
    #[cfg(test)]
    let paused = active.engine.as_ref().is_none_or(Job::pause);
    #[cfg(not(test))]
    let paused = active.engine.as_ref().is_some_and(Job::pause);
    if paused {
        active.phase = JobPhase::PauseRequested;
        cx.refresh_windows();
    }
}

/// Resume a paused transfer, or clear a pause that has not been acknowledged yet.
pub fn resume_active(cx: &mut App) {
    let ops = cx.global_mut::<Operations>();
    let Some(active) = ops.active.as_mut() else { return };
    if !matches!(active.phase, JobPhase::Paused | JobPhase::PauseRequested) {
        return;
    }
    if let Some(job) = &active.engine {
        job.resume();
    }
    active.phase = JobPhase::Running;
    cx.refresh_windows();
}

/// Stop the active request. A queued request never starts; a running one keeps
/// its slot until its terminal event has been logged.
pub fn cancel_active(cx: &mut App) {
    let (id, label, force) = {
        let ops = cx.global_mut::<Operations>();
        let Some(active) = ops.active.as_mut() else { return };
        let force = active.engine.is_none() && !matches!(active.phase, JobPhase::Planning | JobPhase::Cancelling);
        active.phase = JobPhase::Cancelling;
        active.conflict = None;
        if ops.prompt.as_ref().is_some_and(|prompt| prompt.job == active.id) {
            ops.prompt = None;
        }
        if let Some(job) = &active.engine {
            job.cancel();
        } else {
            active.plan_cancel.store(true, Ordering::Relaxed);
        }
        (active.id, active.label.clone(), force)
    };
    dismiss_prompt_sheets(None, cx);
    tracing::info!(request = %id.0, "request cancelled");
    if force {
        // No worker exists (a synthesized test head or a head that never spawned
        // its engine), so there is nothing to wait for.
        finish_without_job(id, label, None, true, cx);
    }
    cx.refresh_windows();
}

/// Cancel one queued request (not the head). Its Activity entry becomes a
/// cancelled outcome immediately; nothing started, so there is no Undo.
pub fn cancel_queued(id: RequestId, cx: &mut App) {
    let removed = {
        let ops = cx.global_mut::<Operations>();
        ops.queue.iter().position(|queued| queued.id == id).and_then(|index| ops.queue.remove(index))
    };
    let Some(queued) = removed else { return };
    tracing::info!(request = %queued.id.0, "queued request cancelled");
    {
        let ops = cx.global_mut::<Operations>();
        ops.log.push(LogEntry {
            record: Arc::new(Record { label: queued.label.clone(), steps: Vec::new() }),
            at: SystemTime::now(),
            error: None,
            cancelled: true,
            undone: false,
        });
    }
    message(format!("{}: cancelled", queued.label), false, cx);
    cx.refresh_windows();
}

/// Answer the current conflict. A stale job or attempt key is ignored; only the
/// one prompt's key is accepted.
pub fn resolve_prompt(job: RequestId, key: ConflictKey, choice: Resolution, apply_to_all: bool, cx: &mut App) -> bool {
    let accepted = {
        let ops = cx.global_mut::<Operations>();
        let Some(active) = ops.active.as_mut().filter(|active| active.id == job) else { return false };
        let current = ops.prompt.as_ref().is_some_and(|prompt| prompt.job == job && prompt.key() == key);
        if !current || active.conflict.as_ref().is_none_or(|conflict| conflict.key != key) {
            return false;
        }
        #[cfg(test)]
        let accepted = active.engine.as_ref().is_none_or(|engine| engine.resolve(key, choice, apply_to_all));
        #[cfg(not(test))]
        let accepted = active.engine.as_ref().is_some_and(|engine| engine.resolve(key, choice, apply_to_all));
        if accepted {
            ops.prompt = None;
            if active.phase == JobPhase::AwaitingConflict {
                active.phase = JobPhase::Running;
            }
            active.conflict = None;
        }
        accepted
    };
    if accepted {
        // The prompt is settled; any sheet still showing this or a stale attempt
        // for it goes away.
        dismiss_prompt_sheets(None, cx);
        cx.refresh_windows();
    }
    accepted
}

/// Show the prompt in a suitable window: the origin first, then the active or
/// any other Crow window without a sheet. If none is free, Activity keeps it.
pub fn present_prompt(cx: &mut App) {
    let Some(prompt) = cx.global::<Operations>().prompt.clone() else { return };
    if let Some(host) = prompt.host
        && let Some(handle) = cx
            .windows()
            .into_iter()
            .find(|handle| handle.window_id() == host)
            .and_then(|handle| handle.downcast::<CrowWindow>())
        && handle.update(cx, |this, window, cx| this.refresh_conflict(window, cx)).unwrap_or(false)
    {
        return;
    }
    let mut candidates: Vec<WindowHandle<CrowWindow>> =
        cx.windows().into_iter().filter_map(|handle| handle.downcast::<CrowWindow>()).collect();
    let origin = cx.global::<Operations>().active.as_ref().and_then(|active| active.origin.clone());
    let active_window = cx.active_window().map(|handle| handle.window_id());
    candidates.sort_by_key(|handle| {
        let is_origin = origin.as_ref().is_some_and(|origin| origin.window.window_id() == handle.window_id());
        let is_active = active_window == Some(handle.window_id());
        (!is_origin, !is_active)
    });
    for handle in candidates {
        if handle.update(cx, |this, window, cx| this.show_conflict(window, cx)).unwrap_or(false) {
            return;
        }
    }
}

/// Bring the prompt to `handle` from Activity, closing a sheet it was shown in.
pub fn present_prompt_in(handle: WindowHandle<CrowWindow>, cx: &mut App) {
    let host = cx.global::<Operations>().prompt.as_ref().and_then(|prompt| prompt.host);
    if let Some(host) = host
        && host != handle.window_id()
        && let Some(old) = cx
            .windows()
            .into_iter()
            .find(|window| window.window_id() == host)
            .and_then(|window| window.downcast::<CrowWindow>())
    {
        old.update(cx, |this, window, cx| this.dismiss_conflict_sheet(None, window, cx)).ok();
        if let Some(prompt) = cx.global_mut::<Operations>().prompt.as_mut() {
            prompt.host = None;
        }
    }
    if handle.update(cx, |this, window, cx| this.show_conflict(window, cx)).unwrap_or(false) {
        cx.refresh_windows();
    } else {
        present_prompt(cx);
    }
}

/// Iterate Crow windows and drop a conflict sheet. `key` narrows it to one
/// attempt; `None` drops any conflict sheet.
pub fn dismiss_prompt_sheets(key: Option<ConflictKey>, cx: &mut App) {
    for handle in cx.windows().into_iter().filter_map(|handle| handle.downcast::<CrowWindow>()) {
        handle.update(cx, |this, window, cx| this.dismiss_conflict_sheet(key, window, cx)).ok();
    }
}

fn window_closed(closed: WindowId, cx: &mut App) {
    let mut rehost = false;
    if let Some(prompt) = cx.global_mut::<Operations>().prompt.as_mut()
        && prompt.host == Some(closed)
    {
        prompt.host = None;
        rehost = true;
    }
    if rehost {
        present_prompt(cx);
    }
}

/// Whether `id` is Crow's only browser window. Settings and other auxiliary
/// windows do not count; closing the last browser window quits Crow.
pub fn is_last_crow_window(id: WindowId, cx: &App) -> bool {
    let ids: Vec<WindowId> = cx
        .windows()
        .into_iter()
        .filter_map(|handle| handle.downcast::<CrowWindow>())
        .map(|handle| handle.window_id())
        .collect();
    ids.len() == 1 && ids[0] == id
}

/// Cancel the queue and the head for a confirmed close or quit. Queued jobs
/// leave immediately as cancelled; the head keeps the slot until its terminal
/// event so a committed prefix can still be undone.
pub fn cancel_all(cx: &mut App) {
    let mut cancelled = Vec::new();
    {
        let ops = cx.global_mut::<Operations>();
        ops.stopping = true;
        while let Some(queued) = ops.queue.pop_front() {
            cancelled.push((queued.id, queued.label));
        }
    }
    for (id, label) in cancelled {
        tracing::info!(request = %id.0, "queued request cancelled for stop");
        let ops = cx.global_mut::<Operations>();
        ops.log.push(LogEntry {
            record: Arc::new(Record { label: label.clone(), steps: Vec::new() }),
            at: SystemTime::now(),
            error: None,
            cancelled: true,
            undone: false,
        });
        message(format!("{label}: cancelled"), false, cx);
    }
    cancel_active(cx);
}

/// The head's label for a stop sheet, falling back to the queue.
pub fn stop_label(cx: &App) -> String {
    let ops = cx.global::<Operations>();
    if let Some(active) = &ops.active {
        return running_label(&active.label);
    }
    match ops.queue.len() {
        0 => "An operation".to_owned(),
        1 => running_label(&ops.queue[0].label),
        n => format!("{n} operations"),
    }
}

/// Undo log entry `index` (the newest by default). Undo checks the disk first and
/// refuses rather than overwrite anything that changed since.
pub fn undo(index: Option<usize>, cx: &mut App, on_done: impl FnOnce(Vec<PathBuf>, &mut App) + 'static) {
    let (index, record, trash) = {
        let ops = cx.global::<Operations>();
        if ops.stopping || !ops.is_idle() {
            message("Another operation is still running", true, cx);
            cx.refresh_windows();
            return;
        }
        let Some(index) = index.or_else(|| ops.undo_stack.last().copied()) else { return };
        let Some(entry) = ops.log.get(index).filter(|e| !e.undone && e.record.is_undoable()) else { return };
        (index, entry.record.clone(), ops.trash.clone())
    };
    cx.global_mut::<Operations>().maintenance = Some(Maintenance::Undo);
    cx.spawn(async move |cx| {
        let result = cx.background_spawn(async move { ops::undo(&record, trash.as_ref()) }).await;
        cx.update(|cx| {
            let ops = cx.global_mut::<Operations>();
            ops.maintenance = None;
            match result {
                Ok(reversed) => {
                    let label = reversed.label.clone();
                    ops.undo_stack.retain(|&i| i != index);
                    if let Some(entry) = ops.log.get_mut(index) {
                        entry.undone = true;
                    }
                    let restored = reversed.results();
                    ops.redo_stack.push(Arc::new(reversed));
                    message(label, false, cx);
                    crate::feedback::play(crow_core::motion::Event::Undo, cx);
                    crate::plugins::notify_operation_done(cx);
                    crate::ui::refresh_volumes(cx);
                    on_done(restored, cx);
                }
                Err(e) => {
                    message(e.to_string(), true, cx);
                    crate::plugins::notify_operation_done(cx);
                }
            }
            start_next(cx);
            cx.refresh_windows();
        });
    })
    .detach();
}

/// Redo the most recently undone operation.
pub fn redo(cx: &mut App, on_done: impl FnOnce(Vec<PathBuf>, &mut App) + 'static) {
    let (reversed, trash) = {
        let ops = cx.global::<Operations>();
        if ops.stopping || !ops.is_idle() {
            message("Another operation is still running", true, cx);
            cx.refresh_windows();
            return;
        }
        let Some(reversed) = ops.redo_stack.last().cloned() else { return };
        (reversed, ops.trash.clone())
    };
    cx.global_mut::<Operations>().maintenance = Some(Maintenance::Redo);
    cx.spawn(async move |cx| {
        let result = cx.background_spawn(async move { ops::undo(&reversed, trash.as_ref()) }).await;
        cx.update(|cx| {
            let ops = cx.global_mut::<Operations>();
            ops.maintenance = None;
            match result {
                Ok(redone) => {
                    let label = redone.label.trim_start_matches("Undo Undo ").to_owned();
                    let results = redone.results();
                    ops.redo_stack.pop();
                    ops.log.push(LogEntry {
                        record: Arc::new(Record { label: label.clone(), steps: redone.steps }),
                        at: SystemTime::now(),
                        error: None,
                        cancelled: false,
                        undone: false,
                    });
                    ops.undo_stack.push(ops.log.len() - 1);
                    message(format!("Redo {label}"), false, cx);
                    crate::feedback::play(crow_core::motion::Event::Undo, cx);
                    crate::plugins::notify_operation_done(cx);
                    crate::ui::refresh_volumes(cx);
                    on_done(results, cx);
                }
                Err(e) => {
                    message(e.to_string(), true, cx);
                    crate::plugins::notify_operation_done(cx);
                }
            }
            start_next(cx);
            cx.refresh_windows();
        });
    })
    .detach();
}

/// Claim the engine slot for a non-transfer mutation. Returns false with the
/// usual message when something else owns it.
pub fn begin_maintenance(kind: Maintenance, cx: &mut App) -> bool {
    let ops = cx.global_mut::<Operations>();
    if ops.stopping || !ops.is_idle() {
        message("Another operation is still running", true, cx);
        cx.refresh_windows();
        return false;
    }
    ops.maintenance = Some(kind);
    true
}

/// Release the engine slot after maintenance and let the queue advance.
pub fn end_maintenance(cx: &mut App) {
    cx.global_mut::<Operations>().maintenance = None;
    start_next(cx);
    cx.refresh_windows();
}

/// Tests only: install a synthesized head with a stable engine id so event
/// injection can drive the coordinator without a worker thread.
#[cfg(test)]
pub fn test_active_in(cx: &mut App, request: ops::Request, phase: JobPhase, progress: Progress) -> (RequestId, JobId) {
    let ops = cx.global_mut::<Operations>();
    ops.next_request += 1;
    let id = RequestId(ops.next_request);
    let engine_id = JobId(1_000_000 + id.0);
    let mut active = ActiveJob::new(QueuedJob { id, label: request_label(&request), request, origin: None });
    active.phase = phase;
    active.progress = progress;
    active.pause_supported = true;
    active.engine_id = Some(engine_id);
    ops.active = Some(active);
    (id, engine_id)
}

/// Tests only: deliver one engine event to the synthesized head.
#[cfg(test)]
pub fn test_deliver(event: JobEvent, cx: &mut App) {
    let Some(id) = cx.global::<Operations>().active_id() else { return };
    handle_event(id, event, cx);
}

/// Tests only: clear coordinator state between injected scenarios.
#[cfg(test)]
pub fn test_clear(cx: &mut App) {
    let ops = cx.global_mut::<Operations>();
    ops.active = None;
    ops.queue.clear();
    ops.prompt = None;
    ops.maintenance = None;
    ops.stopping = false;
}

/// A Trash that is a plain folder, so tests never touch the real one.
#[cfg(any(test, feature = "fuzz"))]
pub struct FolderTrash(pub PathBuf);

#[cfg(any(test, feature = "fuzz"))]
impl Trash for FolderTrash {
    fn trash(&self, path: &std::path::Path) -> std::io::Result<PathBuf> {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let name = path.file_name().ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
        let dest = self.0.join(format!("{}-{}", N.fetch_add(1, Ordering::Relaxed), name.to_string_lossy()));
        // io-ok: test/fuzz Trash, not the app's main thread
        std::fs::rename(path, &dest)?;
        Ok(dest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_label_uses_the_ing_form() {
        // The labels `crow_fs::ops::plan` makes.
        let cases = [
            ("Copy 3 items to Backup", "Copying 3 items to Backup"),
            ("Move 1 item to Trash", "Moving 1 item to Trash"),
            ("Duplicate 2 items", "Duplicating 2 items"),
            ("Delete 2 items permanently", "Deleting 2 items permanently"),
            ("New Folder", "New Folder"),
            ("Rename “a” to “b”", "Rename “a” to “b”"),
            ("Rename “line\nbreak.txt” to “b”", "Rename “line break.txt” to “b”"),
        ];
        for (label, expected) in cases {
            assert_eq!(running_label(label), expected, "{label}");
        }
    }

    #[test]
    fn request_labels_match_the_planned_shape() {
        let paths: Arc<[PathBuf]> = vec![PathBuf::from("/a/x")].into();
        assert_eq!(
            request_label(&ops::Request::Copy { paths: paths.clone(), into: PathBuf::from("/b") }),
            "Copy 1 item to b"
        );
        assert_eq!(request_label(&ops::Request::Trash { paths }), "Move 1 item to Trash");
    }

    #[test]
    fn a_huge_clipboard_looks_up_by_set() {
        // A visible row is one set lookup, never a scan of the 100k paths.
        let paths: Arc<[PathBuf]> = (0..100_000).map(|i| PathBuf::from(format!("/tmp/f{i}"))).collect();
        let clipboard = Clipboard::new(paths, false, None);
        assert!(clipboard.contains(Path::new("/tmp/f99999")));
        assert!(!clipboard.contains(Path::new("/tmp/nope")));
        assert!(clipboard.flash.is_some(), "a copy starts a flash");
        assert!(!clipboard.marked, "a copy is not marked");
        let cut = Clipboard::new(vec![PathBuf::from("/tmp/a")].into(), true, None);
        assert!(cut.flash.is_none(), "a cut does not flash");
        assert!(cut.marked, "a cut is marked");
    }
}
