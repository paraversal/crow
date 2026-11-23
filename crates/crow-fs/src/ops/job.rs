//! Job execution for the file operation engine: stable job/item/attempt ids,
//! ordered events, pause/cancel gates, staging, late conflicts and Replace.
//!
//! A synchronous `execute` entry point exists for the engine's own tests.

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::batch::{register_staging, unregister_staging};
use super::*;

/// A long-lived id for one accepted job.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct JobId(pub u64);

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "job-{}", self.0)
    }
}

static NEXT_JOB: AtomicU64 = AtomicU64::new(1);

fn next_job_id() -> JobId {
    JobId(NEXT_JOB.fetch_add(1, Ordering::Relaxed))
}

/// Identifies one conflict prompt. A reply for any other key is a no-op.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConflictKey {
    pub job_id: JobId,
    pub item_index: usize,
    pub attempt: u32,
}

/// Why the engine is asking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictReason {
    /// The target was taken when the plan was made.
    Exists,
    /// A free target became occupied before commit.
    Appeared,
    /// The target an earlier plan or decision was based on is now a different object.
    Changed,
}

/// One conflict prompt, scoped to the exact job/item/attempt.
#[derive(Clone, Debug)]
pub struct Conflict {
    pub key: ConflictKey,
    pub source: PathBuf,
    pub target: PathBuf,
    /// No-follow identity observed for this attempt.
    pub observed: FileIdentity,
    /// The identity the plan-time choice or previous conflict decision covered, if any.
    pub previous: Option<FileIdentity>,
    pub reason: ConflictReason,
    pub choices: [Resolution; 3],
    /// The current apply-to-all policy, if the user set one.
    pub policy: Option<Resolution>,
    /// Set when that policy answered this conflict without waiting.
    pub auto: Option<Resolution>,
}

/// Everything the worker reports. Progress is coalesced; transitions are not.
#[derive(Clone, Debug)]
pub enum JobEvent {
    Started { job_id: JobId, total_items: usize, total_bytes: u64, pause_supported: bool },
    Progress { job_id: JobId, progress: Progress },
    PauseAcknowledged { job_id: JobId, progress: Progress },
    Conflict(Conflict),
    Terminal { job_id: JobId, outcome: Arc<Outcome> },
}

/// How to run a job. Existing callers can pre-resolve plan conflicts and keep
/// Keep Both as the default for anything that appears later.
#[derive(Clone)]
pub struct JobOptions {
    /// Plan-time choices, by target path.
    pub resolutions: HashMap<PathBuf, Resolution>,
    /// Used when a plan-time conflict has no entry in `resolutions`.
    pub initial_default: Option<Resolution>,
    /// Used for a conflict that appears after planning. The app leaves this `None`
    /// so every late conflict waits for the current decision.
    pub late_default: Option<Resolution>,
    /// At most one progress event per interval. Zero emits every update.
    pub progress_interval: Duration,
    #[cfg(test)]
    pub(crate) hooks: Option<TestHooks>,
}

impl Default for JobOptions {
    fn default() -> Self {
        Self {
            resolutions: HashMap::new(),
            initial_default: None,
            late_default: None,
            progress_interval: Duration::from_millis(33),
            #[cfg(test)]
            hooks: None,
        }
    }
}

#[cfg(test)]
#[derive(Clone, Default)]
pub(crate) struct TestHooks {
    pub after_stage: Option<Arc<dyn Fn() + Send + Sync>>,
    pub before_commit: Option<Arc<dyn Fn() + Send + Sync>>,
    pub before_replace_trash: Option<Arc<dyn Fn() + Send + Sync>>,
    pub after_replace_trash: Option<Arc<dyn Fn() + Send + Sync>>,
    pub before_source_retire: Option<Arc<dyn Fn() + Send + Sync>>,
    pub on_first_bytes: Option<Arc<dyn Fn() + Send + Sync>>,
    pub after_commit: Option<Arc<dyn Fn() + Send + Sync>>,
    pub before_batch_stage: Option<Arc<dyn Fn(usize) + Send + Sync>>,
    pub before_batch_place: Option<Arc<dyn Fn(usize) + Send + Sync>>,
    pub before_batch_restore: Option<Arc<dyn Fn(usize) + Send + Sync>>,
    /// Test-only: take the non-clone byte-copy path.
    pub no_clone: bool,
    /// Test-only: take the cross-volume move path without a second volume.
    pub force_cross_device: bool,
}

/// A running job. Dropping it does not stop the worker; call [`Job::cancel`] or
/// [`Job::wait`].
pub struct Job {
    id: JobId,
    label: String,
    control: Arc<JobControl>,
    thread: Option<JoinHandle<Arc<Outcome>>>,
}

impl Job {
    /// Start `plan` on its own transfer thread. The event sink runs on that
    /// thread, in order, and must not block on the job's own control methods.
    pub fn spawn<F>(plan: Plan, options: JobOptions, trash: Arc<dyn Trash>, mut events: F) -> io::Result<Self>
    where
        F: FnMut(JobEvent) + Send + 'static,
    {
        let id = next_job_id();
        let pause_supported =
            matches!(&plan.request, Request::Copy { .. } | Request::Move { .. } | Request::Duplicate { .. });
        let control = Arc::new(JobControl::new(id, pause_supported));
        let worker_control = control.clone();
        let label = plan.label.clone();
        let thread = std::thread::Builder::new()
            .name(format!("crow-op-{id}"))
            .spawn(move || run_job(id, &plan, &options, trash.as_ref(), &worker_control, None, &mut events))
            .map_err(|e| io::Error::other(format!("could not start the copy worker: {e}")))?;
        Ok(Self { id, label, control, thread: Some(thread) })
    }

    pub fn id(&self) -> JobId {
        self.id
    }

    pub fn pause_supported(&self) -> bool {
        self.control.pause_supported
    }

    /// Request a pause. The worker acknowledges it at the next real checkpoint;
    /// a clone acknowledges only before or after the clone, or between entries.
    pub fn pause(&self) -> bool {
        self.control.request_pause()
    }

    pub fn resume(&self) {
        self.control.resume();
    }

    pub fn cancel(&self) {
        self.control.cancel();
    }

    /// Answer the current conflict. A stale attempt key is ignored.
    pub fn resolve(&self, key: ConflictKey, choice: Resolution, apply_to_all: bool) -> bool {
        self.control.resolve(key, choice, apply_to_all)
    }

    /// Wait for the worker and return its one terminal outcome.
    pub fn wait(mut self) -> Arc<Outcome> {
        let Some(thread) = self.thread.take() else {
            return Arc::new(Outcome {
                record: Record { label: self.label.clone(), steps: Vec::new() },
                error: Some(OpError::Io(
                    PathBuf::from("copy job"),
                    Arc::new(io::Error::other("the copy worker is missing")),
                )),
                cancelled: false,
            });
        };
        match thread.join() {
            Ok(outcome) => outcome,
            Err(_) => Arc::new(Outcome {
                record: Record { label: self.label.clone(), steps: Vec::new() },
                error: Some(OpError::Io(
                    PathBuf::from("copy job"),
                    Arc::new(io::Error::other("the copy worker panicked")),
                )),
                cancelled: false,
            }),
        }
    }
}

struct ControlState {
    pause_requested: bool,
    paused: bool,
    cancel: bool,
    terminal: bool,
    waiting: Option<ConflictKey>,
    decisions: HashMap<ConflictKey, Resolution>,
    policy: Option<Resolution>,
}

struct JobControl {
    id: JobId,
    pause_supported: bool,
    state: Mutex<ControlState>,
    cv: Condvar,
}

impl JobControl {
    fn new(id: JobId, pause_supported: bool) -> Self {
        Self {
            id,
            pause_supported,
            state: Mutex::new(ControlState {
                pause_requested: false,
                paused: false,
                cancel: false,
                terminal: false,
                waiting: None,
                decisions: HashMap::new(),
                policy: None,
            }),
            cv: Condvar::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ControlState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn request_pause(&self) -> bool {
        let mut state = self.lock();
        if state.terminal || state.cancel || state.paused || !self.pause_supported || state.waiting.is_some() {
            return false;
        }
        state.pause_requested = true;
        true
    }

    fn resume(&self) {
        let mut state = self.lock();
        state.pause_requested = false;
        state.paused = false;
        self.cv.notify_all();
    }

    fn cancel(&self) {
        let mut state = self.lock();
        state.cancel = true;
        self.cv.notify_all();
    }

    fn finish(&self) {
        let mut state = self.lock();
        state.terminal = true;
        state.waiting = None;
        self.cv.notify_all();
    }

    fn is_cancelled(&self) -> bool {
        self.lock().cancel
    }

    fn policy(&self) -> Option<Resolution> {
        self.lock().policy
    }

    fn resolve(&self, key: ConflictKey, choice: Resolution, apply_to_all: bool) -> bool {
        let mut state = self.lock();
        if state.waiting != Some(key) {
            return false;
        }
        if apply_to_all {
            state.policy = Some(choice);
        }
        state.decisions.insert(key, choice);
        self.cv.notify_all();
        true
    }

    /// A safe point: acknowledge a pending pause here, or report cancellation.
    fn checkpoint(&self, snapshot: &Progress, emit: &mut dyn FnMut(JobEvent)) -> Result<(), Stop> {
        let mut state = self.lock();
        if state.cancel {
            return Err(Stop::Cancelled);
        }
        if !state.pause_requested {
            return Ok(());
        }
        state.pause_requested = false;
        state.paused = true;
        drop(state);
        emit(JobEvent::PauseAcknowledged { job_id: self.id, progress: snapshot.clone() });
        let mut state = self.lock();
        while state.paused && !state.cancel {
            state = self.cv.wait(state).unwrap_or_else(|e| e.into_inner());
        }
        if state.cancel {
            state.paused = false;
            return Err(Stop::Cancelled);
        }
        state.paused = false;
        Ok(())
    }

    fn await_conflict(
        &self,
        key: ConflictKey,
        conflict: Conflict,
        emit: &mut dyn FnMut(JobEvent),
    ) -> Result<Resolution, Stop> {
        let mut state = self.lock();
        if state.cancel {
            return Err(Stop::Cancelled);
        }
        state.waiting = Some(key);
        drop(state);
        emit(JobEvent::Conflict(conflict));
        let mut state = self.lock();
        loop {
            if state.cancel {
                state.waiting = None;
                return Err(Stop::Cancelled);
            }
            if let Some(choice) = state.decisions.remove(&key) {
                state.waiting = None;
                return Ok(choice);
            }
            state = self.cv.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }
}

#[derive(Debug)]
enum Stop {
    Cancelled,
    Failed(OpError),
}

impl From<OpError> for Stop {
    fn from(error: OpError) -> Self {
        Self::Failed(error)
    }
}

fn op_io(path: &Path, error: io::Error) -> OpError {
    OpError::Io(path.to_path_buf(), Arc::new(error))
}

fn failed<T>(error: OpError) -> Result<T, Stop> {
    Err(Stop::Failed(error))
}

#[derive(Clone, Debug)]
enum Decision {
    Fresh,
    KeepBoth,
    Replace(FileIdentity),
    Skip,
}

enum ItemResult {
    Done,
    Skipped,
}

enum Commit {
    Placed { target: PathBuf, step_index: usize },
    Skipped,
}

struct Runner<'a> {
    job_id: JobId,
    request: Request,
    items: &'a [PlannedItem],
    options: &'a JobOptions,
    trash: &'a dyn Trash,
    control: &'a JobControl,
    external_cancel: Option<&'a AtomicBool>,
    emit: &'a mut dyn FnMut(JobEvent),
    record: Record,
    progress: Progress,
    last_progress: Option<Instant>,
    item_index: usize,
    item_bytes: u64,
    before_bytes: u64,
    #[cfg(test)]
    first_bytes_fired: bool,
}

impl<'a> Runner<'a> {
    fn new(
        job_id: JobId,
        plan: &'a Plan,
        options: &'a JobOptions,
        trash: &'a dyn Trash,
        control: &'a JobControl,
        external_cancel: Option<&'a AtomicBool>,
        emit: &'a mut dyn FnMut(JobEvent),
    ) -> Self {
        Self {
            job_id,
            request: plan.request.clone(),
            items: &plan.items,
            options,
            trash,
            control,
            external_cancel,
            emit,
            record: Record { label: plan.label.clone(), steps: Vec::new() },
            progress: Progress { total_items: plan.items.len(), total_bytes: plan.total_bytes, ..Progress::default() },
            last_progress: None,
            item_index: 0,
            item_bytes: 0,
            before_bytes: 0,
            #[cfg(test)]
            first_bytes_fired: false,
        }
    }

    fn run(&mut self) -> Result<(), Stop> {
        (self.emit)(JobEvent::Started {
            job_id: self.job_id,
            total_items: self.progress.total_items,
            total_bytes: self.progress.total_bytes,
            pause_supported: self.control.pause_supported,
        });
        let batch = match &self.request {
            Request::BatchRename { moves } => Some(moves.clone()),
            _ => None,
        };
        if let Some(moves) = batch {
            return self.run_batch(&moves);
        }
        for index in 0..self.items.len() {
            let item = self.items[index].clone();
            self.item_index = index;
            self.item_bytes = item.bytes;
            self.checkpoint()?;
            self.before_bytes = self.progress.done_bytes;
            self.progress.current = item.source.clone().unwrap_or_else(|| item.target.clone());
            self.emit_progress(true);
            match self.run_item(&item) {
                Ok(ItemResult::Done | ItemResult::Skipped) => {
                    self.progress.done_bytes =
                        self.before_bytes.saturating_add(item.bytes).min(self.progress.total_bytes);
                    self.progress.done_items = self.progress.done_items.saturating_add(1);
                    self.emit_progress(true);
                }
                Err(stop) => return Err(stop),
            }
            self.checkpoint()?;
        }
        Ok(())
    }

    fn checkpoint(&mut self) -> Result<(), Stop> {
        if let Some(external) = self.external_cancel
            && external.load(Ordering::Relaxed)
        {
            self.control.cancel();
        }
        let progress = self.progress.clone();
        self.control.checkpoint(&progress, self.emit)
    }

    fn run_batch(&mut self, moves: &[BatchRenameEntry]) -> Result<(), Stop> {
        let job_id = self.job_id;
        let hooks = self.batch_hooks();
        let emit = &mut *self.emit;
        let progress = &mut self.progress;
        let control = self.control;
        let external = self.external_cancel;
        let cancel = || control.is_cancelled() || external.is_some_and(|cancel| cancel.load(Ordering::Relaxed));
        let result = execute_batch(
            moves,
            &cancel,
            &mut |done| {
                progress.done_items = done;
                (emit)(JobEvent::Progress { job_id, progress: progress.clone() });
            },
            &hooks,
        );
        match result {
            Ok(committed) => {
                progress.done_items = committed.len();
                (emit)(JobEvent::Progress { job_id, progress: progress.clone() });
                self.record.steps.push(Step::BatchRenamed { moves: committed });
                Ok(())
            }
            Err(OpError::Cancelled) => Err(Stop::Cancelled),
            Err(error) => Err(Stop::Failed(error)),
        }
    }

    fn batch_hooks(&self) -> BatchHooks {
        #[cfg(test)]
        {
            let hooks = self.options.hooks.as_ref();
            BatchHooks {
                before_stage: hooks.and_then(|h| h.before_batch_stage.clone()),
                after_stage: None,
                before_place: hooks.and_then(|h| h.before_batch_place.clone()),
                before_restore: hooks.and_then(|h| h.before_batch_restore.clone()),
            }
        }
        #[cfg(not(test))]
        {
            BatchHooks::default()
        }
    }

    fn emit_progress(&mut self, force: bool) {
        let due = self.options.progress_interval.is_zero()
            || self.last_progress.is_none_or(|last| last.elapsed() >= self.options.progress_interval);
        if force || due {
            self.last_progress = Some(Instant::now());
            (self.emit)(JobEvent::Progress { job_id: self.job_id, progress: self.progress.clone() });
        }
    }

    fn on_bytes(&mut self, copied: u64) -> Result<(), OpError> {
        // copyfile reports some status events without the current file's bytes (for
        // example an xattr after the data). Progress only moves forward; a late
        // callback with a smaller total must not take the bar backwards.
        let done = self.before_bytes.saturating_add(copied.min(self.item_bytes)).min(self.progress.total_bytes);
        self.progress.done_bytes = self.progress.done_bytes.max(done);
        self.emit_progress(false);
        #[cfg(test)]
        if !self.first_bytes_fired && copied > 0 {
            self.first_bytes_fired = true;
            let hook = self.options.hooks.as_ref().and_then(|h| h.on_first_bytes.clone());
            if let Some(hook) = hook {
                hook();
            }
        }
        self.checkpoint().map_err(|stop| match stop {
            Stop::Cancelled => OpError::Cancelled,
            Stop::Failed(error) => error,
        })
    }

    fn run_item(&mut self, item: &PlannedItem) -> Result<ItemResult, Stop> {
        match self.request.clone() {
            Request::NewFolder { .. } => {
                self.checkpoint()?;
                match std::fs::create_dir(&item.target) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                        return failed(OpError::NameTaken(item.target.clone()));
                    }
                    Err(e) => return failed(op_io(&item.target, e)),
                }
                self.record.steps.push(Step::Created(item.target.clone()));
                Ok(ItemResult::Done)
            }
            Request::NewFile { .. } => {
                self.checkpoint()?;
                match std::fs::OpenOptions::new().write(true).create_new(true).open(&item.target) {
                    Ok(_) => {}
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                        return failed(OpError::NameTaken(item.target.clone()));
                    }
                    Err(e) => return failed(op_io(&item.target, e)),
                }
                self.record.steps.push(Step::Created(item.target.clone()));
                Ok(ItemResult::Done)
            }
            Request::Rename { path, .. } => {
                self.checkpoint()?;
                let result = if same_file(&path, &item.target) {
                    std::fs::rename(&path, &item.target)
                } else {
                    rename_no_clobber(&path, &item.target)
                };
                match result {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                        return failed(OpError::NameTaken(item.target.clone()));
                    }
                    Err(e) => return failed(op_io(&path, e)),
                }
                self.record.steps.push(Step::Moved { from: path, to: item.target.clone() });
                Ok(ItemResult::Done)
            }
            Request::Trash { .. } => {
                self.checkpoint()?;
                if !exists(&item.target) {
                    return failed(OpError::NotFound(item.target.clone()));
                }
                let trashed = self.trash.trash(&item.target).map_err(|e| op_io(&item.target, e))?;
                self.record.steps.push(Step::Trashed { original: item.target.clone(), trashed });
                Ok(ItemResult::Done)
            }
            Request::DeletePermanently { .. } => {
                self.checkpoint()?;
                remove_tree(&item.target).map_err(|e| op_io(&item.target, e))?;
                self.record.steps.push(Step::Deleted(item.target.clone()));
                Ok(ItemResult::Done)
            }
            Request::BatchRename { .. } => unreachable!("a batch runs as one commit segment"),
            Request::Duplicate { .. } | Request::Copy { .. } | Request::Move { .. } => self.transfer(item),
        }
    }

    fn transfer(&mut self, item: &PlannedItem) -> Result<ItemResult, Stop> {
        let source = item.source.as_ref().ok_or_else(|| Stop::Failed(OpError::NotFound(item.target.clone())))?;
        self.recheck_source(source, item)?;

        let moving = matches!(self.request, Request::Move { .. });
        let cross_volume_move = moving && (self.force_cross_device() || !same_volume(source, &item.target));
        let stage_first = !moving || cross_volume_move;
        let mut attempt = 0u32;
        let decision = self.initial_decision(item, source, &item.target, &mut attempt)?;
        if matches!(decision, Decision::Skip) {
            return Ok(ItemResult::Skipped);
        }

        if stage_first {
            let stage =
                Stage::create(&item.target, self.job_id, self.item_index).map_err(|e| op_io(&item.target, e))?;
            self.checkpoint()?;
            match copy_tree(source, &stage.item, self.use_clone(), &mut |copied| self.on_bytes(copied)) {
                Ok(()) => {}
                Err(OpError::Cancelled) => return Err(Stop::Cancelled),
                Err(error) => return Err(Stop::Failed(error)),
            }
            self.on_bytes(item.bytes).map_err(|error| match error {
                OpError::Cancelled => Stop::Cancelled,
                error => Stop::Failed(error),
            })?;
            self.recheck_source(source, item)?;
            #[cfg(test)]
            self.fire_hook(|h| h.after_stage.clone());
            self.checkpoint()?;
            let commit = self.commit_place(item, &stage.item, None, item.target.clone(), decision, &mut attempt)?;
            return match commit {
                Commit::Skipped => Ok(ItemResult::Skipped),
                Commit::Placed { target, step_index } if moving => {
                    self.retire_move_source(source, &target, step_index)?;
                    Ok(ItemResult::Done)
                }
                Commit::Placed { .. } => Ok(ItemResult::Done),
            };
        }

        match self.commit_place(item, source, Some(source), item.target.clone(), decision, &mut attempt)? {
            Commit::Skipped => Ok(ItemResult::Skipped),
            Commit::Placed { .. } => Ok(ItemResult::Done),
        }
    }

    fn initial_decision(
        &mut self,
        item: &PlannedItem,
        source: &Path,
        target: &Path,
        attempt: &mut u32,
    ) -> Result<Decision, Stop> {
        let observed = match target_identity(target) {
            Ok(Some(identity)) => identity,
            Ok(None) => return Ok(Decision::Fresh),
            Err(error) => return Err(Stop::Failed(error)),
        };
        let reason = if item.target_identity.is_some() {
            if item.target_identity.as_ref() == Some(&observed) {
                ConflictReason::Exists
            } else {
                ConflictReason::Changed
            }
        } else {
            ConflictReason::Appeared
        };
        let chosen = if item.conflict && reason != ConflictReason::Changed {
            self.options.resolutions.get(target).copied().or(self.options.initial_default)
        } else {
            self.options.late_default
        };
        if let Some(choice) = chosen {
            return Ok(decision_from(choice, Some(observed)));
        }
        let (choice, approved) =
            self.wait_conflict(source, target.to_path_buf(), observed, reason, item.target_identity.clone(), attempt)?;
        Ok(decision_from(choice, approved))
    }

    #[allow(clippy::too_many_arguments)]
    fn wait_conflict(
        &mut self,
        source: &Path,
        target: PathBuf,
        observed: FileIdentity,
        reason: ConflictReason,
        previous: Option<FileIdentity>,
        attempt: &mut u32,
    ) -> Result<(Resolution, Option<FileIdentity>), Stop> {
        let mut observed = observed;
        let mut reason = reason;
        let mut previous = previous;
        loop {
            *attempt = attempt.saturating_add(1);
            let key = ConflictKey { job_id: self.job_id, item_index: self.item_index, attempt: *attempt };
            let policy = self.control.policy();
            let conflict = Conflict {
                key,
                source: source.to_path_buf(),
                target: target.clone(),
                observed: observed.clone(),
                previous: previous.clone(),
                reason,
                choices: [Resolution::KeepBoth, Resolution::Replace, Resolution::Skip],
                policy,
                auto: None,
            };
            let choice = if reason != ConflictReason::Changed
                && let Some(auto) = policy
            {
                (self.emit)(JobEvent::Conflict(Conflict { auto: Some(auto), ..conflict }));
                auto
            } else {
                self.control.await_conflict(key, conflict, self.emit).map_err(|stop| match stop {
                    Stop::Cancelled => Stop::Cancelled,
                    Stop::Failed(error) => Stop::Failed(error),
                })?
            };
            match choice {
                Resolution::Skip | Resolution::KeepBoth => return Ok((choice, Some(observed))),
                Resolution::Replace => {}
            }
            // A replacement must cover the identity at the moment it is approved.
            let now = match target_identity(&target) {
                Ok(Some(identity)) => identity,
                Ok(None) => return Ok((Resolution::Replace, None)),
                Err(error) => return Err(Stop::Failed(error)),
            };
            if now == observed {
                return Ok((Resolution::Replace, Some(now)));
            }
            previous = Some(observed);
            observed = now;
            reason = ConflictReason::Changed;
        }
    }

    fn commit_place(
        &mut self,
        item: &PlannedItem,
        place_from: &Path,
        move_source: Option<&Path>,
        target: PathBuf,
        mut decision: Decision,
        attempt: &mut u32,
    ) -> Result<Commit, Stop> {
        let source = item.source.as_deref().unwrap_or(place_from);
        loop {
            self.checkpoint()?;
            self.recheck_source(source, item)?;
            #[cfg(test)]
            self.fire_hook(|h| h.before_commit.clone());
            match decision.clone() {
                Decision::Skip => return Ok(Commit::Skipped),
                Decision::Fresh => match target_identity(&target) {
                    Ok(None) => match rename_no_clobber(place_from, &target) {
                        Ok(()) => return Ok(self.placed(&target, move_source)),
                        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                            let observed = match target_identity(&target) {
                                Ok(Some(identity)) => identity,
                                Ok(None) => continue,
                                Err(error) => return Err(Stop::Failed(error)),
                            };
                            decision =
                                self.conflict_for(source, &target, observed, ConflictReason::Appeared, None, attempt)?;
                        }
                        Err(e) => return failed(op_io(&target, e)),
                    },
                    Ok(Some(observed)) => {
                        decision =
                            self.conflict_for(source, &target, observed, ConflictReason::Appeared, None, attempt)?;
                    }
                    Err(error) => return Err(Stop::Failed(error)),
                },
                Decision::KeepBoth => {
                    let parent = target.parent().ok_or_else(|| Stop::Failed(OpError::NotFound(target.clone())))?;
                    let name = file_name_os(&item.target)?;
                    let is_dir = item.source_identity.as_ref().is_some_and(|source| source.kind == EntryKind::Dir);
                    // Names already taken do not use up the retry budget; it is for
                    // names that appear while the copy is being placed.
                    let free = keep_both_candidates(parent, name, is_dir).filter(|candidate| !exists(candidate));
                    for candidate in free.take(MAX_COLLISION_RETRIES) {
                        match rename_no_clobber(place_from, &candidate) {
                            Ok(()) => return Ok(self.placed(&candidate, move_source)),
                            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                            Err(e) => return failed(op_io(&candidate, e)),
                        }
                    }
                    return failed(OpError::NameTaken(item.target.clone()));
                }
                Decision::Replace(approved) => {
                    let current = match target_identity(&target) {
                        Ok(Some(identity)) => Some(identity),
                        Ok(None) => None,
                        Err(error) => return Err(Stop::Failed(error)),
                    };
                    match current {
                        Some(current) if current != approved => {
                            decision = self.conflict_for(
                                source,
                                &target,
                                current,
                                ConflictReason::Changed,
                                Some(approved),
                                attempt,
                            )?;
                        }
                        Some(_) => {
                            // NSFileManager's Trash has no expected-identity argument.
                            // Verify immediately before and after; a swap in this
                            // window is detected below and restored no-clobber.
                            #[cfg(test)]
                            self.fire_hook(|h| h.before_replace_trash.clone());
                            let trashed = match self.trash.trash(&target) {
                                Ok(trashed) => Some(trashed),
                                Err(e) if e.kind() == io::ErrorKind::NotFound => None,
                                Err(e) => return failed(op_io(&target, e)),
                            };
                            let Some(trashed) = trashed else {
                                match rename_no_clobber(place_from, &target) {
                                    Ok(()) => return Ok(self.placed(&target, move_source)),
                                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                                        let observed = match target_identity(&target) {
                                            Ok(Some(identity)) => identity,
                                            Ok(None) => continue,
                                            Err(error) => return Err(Stop::Failed(error)),
                                        };
                                        decision = self.conflict_for(
                                            source,
                                            &target,
                                            observed,
                                            ConflictReason::Appeared,
                                            None,
                                            attempt,
                                        )?;
                                        continue;
                                    }
                                    Err(e) => return failed(op_io(&target, e)),
                                }
                            };
                            let verified = identity(&trashed).ok().is_some_and(|id| id.same_after_rename(&approved));
                            if !verified {
                                match rename_no_clobber(&trashed, &target) {
                                    Ok(()) => {
                                        let observed = match target_identity(&target) {
                                            Ok(Some(identity)) => identity,
                                            Ok(None) => continue,
                                            Err(error) => return Err(Stop::Failed(error)),
                                        };
                                        decision = self.conflict_for(
                                            source,
                                            &target,
                                            observed,
                                            ConflictReason::Changed,
                                            Some(approved),
                                            attempt,
                                        )?;
                                        continue;
                                    }
                                    Err(restore_error) => {
                                        self.record
                                            .steps
                                            .push(Step::Trashed { original: target.clone(), trashed: trashed.clone() });
                                        return failed(OpError::RestoreBlocked {
                                            target,
                                            trashed,
                                            error: Arc::new(restore_error),
                                        });
                                    }
                                }
                            }
                            self.record
                                .steps
                                .push(Step::Trashed { original: target.clone(), trashed: trashed.clone() });
                            let trash_index = self.record.steps.len() - 1;
                            #[cfg(test)]
                            self.fire_hook(|h| h.after_replace_trash.clone());
                            match rename_no_clobber(place_from, &target) {
                                Ok(()) => return Ok(self.placed(&target, move_source)),
                                Err(place_error) => match rename_no_clobber(&trashed, &target) {
                                    Ok(()) => {
                                        self.record.steps.remove(trash_index);
                                        return failed(op_io(&target, place_error));
                                    }
                                    Err(restore_error) => {
                                        return failed(OpError::RestoreBlocked {
                                            target,
                                            trashed,
                                            error: Arc::new(restore_error),
                                        });
                                    }
                                },
                            }
                        }
                        None => match rename_no_clobber(place_from, &target) {
                            Ok(()) => return Ok(self.placed(&target, move_source)),
                            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                                let observed = match target_identity(&target) {
                                    Ok(Some(identity)) => identity,
                                    Ok(None) => continue,
                                    Err(error) => return Err(Stop::Failed(error)),
                                };
                                decision = self.conflict_for(
                                    source,
                                    &target,
                                    observed,
                                    ConflictReason::Appeared,
                                    None,
                                    attempt,
                                )?;
                            }
                            Err(e) => return failed(op_io(&target, e)),
                        },
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn conflict_for(
        &mut self,
        source: &Path,
        target: &Path,
        observed: FileIdentity,
        reason: ConflictReason,
        previous: Option<FileIdentity>,
        attempt: &mut u32,
    ) -> Result<Decision, Stop> {
        let (choice, approved) =
            self.wait_conflict(source, target.to_path_buf(), observed, reason, previous, attempt)?;
        Ok(decision_from(choice, approved))
    }

    fn placed(&mut self, target: &Path, move_source: Option<&Path>) -> Commit {
        let step = match move_source {
            Some(source) => Step::Moved { from: source.to_path_buf(), to: target.to_path_buf() },
            None => Step::Created(target.to_path_buf()),
        };
        self.record.steps.push(step);
        #[cfg(test)]
        self.fire_hook(|h| h.after_commit.clone());
        Commit::Placed { target: target.to_path_buf(), step_index: self.record.steps.len() - 1 }
    }

    fn force_cross_device(&self) -> bool {
        #[cfg(test)]
        if let Some(hooks) = &self.options.hooks
            && hooks.force_cross_device
        {
            return true;
        }
        force_cross_device()
    }

    fn use_clone(&self) -> bool {
        #[cfg(test)]
        if let Some(hooks) = &self.options.hooks
            && hooks.no_clone
        {
            return false;
        }
        true
    }

    fn retire_move_source(&mut self, source: &Path, target: &Path, step_index: usize) -> Result<(), Stop> {
        #[cfg(test)]
        self.fire_hook(|h| h.before_source_retire.clone());
        match remove_tree(source) {
            Ok(()) => {
                if let Some(step) = self.record.steps.get_mut(step_index)
                    && let Step::Created(to) = step
                {
                    let to = to.clone();
                    *step = Step::Moved { from: source.to_path_buf(), to };
                }
                Ok(())
            }
            Err(error) => failed(OpError::SourceKept {
                source: source.to_path_buf(),
                target: target.to_path_buf(),
                error: Arc::new(error),
            }),
        }
    }

    fn recheck_source(&self, source: &Path, item: &PlannedItem) -> Result<(), Stop> {
        let Some(planned) = &item.source_identity else { return Ok(()) };
        match identity(source) {
            Ok(now) if now == *planned => Ok(()),
            Ok(_) => failed(OpError::SourceChanged(source.to_path_buf())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => failed(OpError::NotFound(source.to_path_buf())),
            Err(error) => failed(op_io(source, error)),
        }
    }

    #[cfg(test)]
    fn fire_hook(&self, pick: impl FnOnce(&TestHooks) -> Option<Arc<dyn Fn() + Send + Sync>>) {
        let hook = self.options.hooks.as_ref().and_then(pick);
        if let Some(hook) = hook {
            hook();
        }
    }
}

fn decision_from(choice: Resolution, approved: Option<FileIdentity>) -> Decision {
    match choice {
        Resolution::KeepBoth => Decision::KeepBoth,
        Resolution::Skip => Decision::Skip,
        Resolution::Replace => match approved {
            Some(identity) => Decision::Replace(identity),
            None => Decision::Fresh,
        },
    }
}

const MAX_COLLISION_RETRIES: usize = 64;

fn same_volume(source: &Path, target: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let source_dev = source.symlink_metadata().map(|m| m.dev());
    let target_dev = target.parent().and_then(|parent| parent.metadata().ok()).map(|m| m.dev());
    matches!((source_dev, target_dev), (Ok(a), Some(b)) if a == b)
}

#[cfg(test)]
fn force_cross_device() -> bool {
    FORCE_CROSS_DEVICE.load(Ordering::Relaxed)
}

#[cfg(not(test))]
fn force_cross_device() -> bool {
    false
}

#[cfg(test)]
pub(crate) static FORCE_CROSS_DEVICE: AtomicBool = AtomicBool::new(false);

struct Stage {
    container: PathBuf,
    item: PathBuf,
}

impl Stage {
    fn create(target: &Path, job_id: JobId, item_index: usize) -> io::Result<Self> {
        use std::os::unix::fs::DirBuilderExt;
        let parent = target.parent().ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        for n in 0..MAX_COLLISION_RETRIES {
            let container = parent.join(format!(".crow-partial-{}-{}-{item_index}-{n}", std::process::id(), job_id.0));
            let mut builder = std::fs::DirBuilder::new();
            builder.mode(0o700);
            match builder.create(&container) {
                Ok(()) => {
                    register_staging(container.clone());
                    return Ok(Self { item: container.join("item"), container });
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(io::ErrorKind::AlreadyExists, "no private staging name was free"))
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        // Only the container this job created, and only when it is ours: create_dir
        // was exclusive, so a pre-existing path never reaches this.
        unregister_staging(&self.container);
        if let Err(e) = remove_tree(&self.container) {
            warn(&format!("could not remove staging folder {}: {e}", self.container.display()));
        }
    }
}

fn run_job(
    job_id: JobId,
    plan: &Plan,
    options: &JobOptions,
    trash: &dyn Trash,
    control: &JobControl,
    external_cancel: Option<&AtomicBool>,
    emit: &mut dyn FnMut(JobEvent),
) -> Arc<Outcome> {
    let (result, record) = {
        let mut runner = Runner::new(job_id, plan, options, trash, control, external_cancel, emit);
        let result = runner.run();
        (result, runner.record)
    };
    let (error, cancelled) = match result {
        Ok(()) => (None, false),
        Err(Stop::Cancelled) => (None, true),
        Err(Stop::Failed(error)) => (Some(error), control.is_cancelled()),
    };
    let outcome = Arc::new(Outcome { record, error, cancelled });
    control.finish();
    emit(JobEvent::Terminal { job_id, outcome: outcome.clone() });
    outcome
}

/// A synchronous run for the engine's tests: no pause, plan-time choices by
/// path and Keep Both for anything that appears later.
#[cfg(test)]
pub fn execute(
    plan: &Plan,
    resolutions: &HashMap<PathBuf, Resolution>,
    trash: &dyn Trash,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(&Progress),
) -> Outcome {
    let options = JobOptions {
        resolutions: resolutions.clone(),
        initial_default: Some(Resolution::KeepBoth),
        late_default: Some(Resolution::KeepBoth),
        progress_interval: Duration::ZERO,
        #[cfg(test)]
        hooks: None,
    };
    let id = next_job_id();
    let control = JobControl::new(id, false);
    let mut emit = |event: JobEvent| {
        if let JobEvent::Progress { progress: p, .. } = event {
            progress(&p);
        }
    };
    let outcome = run_job(id, plan, &options, trash, &control, Some(cancel), &mut emit);
    match Arc::try_unwrap(outcome) {
        Ok(outcome) => outcome,
        Err(outcome) => (*outcome).clone(),
    }
}

/// Move without replacing anything; across volumes, copy then delete the source.
pub(super) fn move_item(
    from: &Path,
    to: &Path,
    cancel: &AtomicBool,
    on_bytes: &mut dyn FnMut(u64),
) -> Result<(), OpError> {
    let renamed = if force_cross_device() {
        Err(io::Error::from(io::ErrorKind::CrossesDevices))
    } else {
        rename_no_clobber(from, to)
    };
    match renamed {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
            let stage = Stage::create(to, JobId(0), 0).map_err(|e| op_io(to, e))?;
            copy_tree(from, &stage.item, true, &mut |copied| {
                if cancel.load(Ordering::Relaxed) {
                    return Err(OpError::Cancelled);
                }
                on_bytes(copied);
                Ok(())
            })?;
            rename_no_clobber(&stage.item, to).map_err(|e| op_io(to, e))?;
            // The copy is in place; a source that cannot be removed is reported
            // as kept, like a cross-volume move in a job.
            remove_tree(from).map_err(|error| OpError::SourceKept {
                source: from.to_path_buf(),
                target: to.to_path_buf(),
                error: Arc::new(error),
            })
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(OpError::NameTaken(to.to_path_buf())),
        Err(e) => Err(op_io(from, e)),
    }
}

#[cfg(target_os = "macos")]
fn copy_tree(
    source: &Path,
    dest: &Path,
    clone: bool,
    checkpoint: &mut dyn FnMut(u64) -> Result<(), OpError>,
) -> Result<(), OpError> {
    use std::ffi::{CStr, CString, c_char, c_int, c_void};
    use std::os::unix::ffi::OsStrExt;

    struct Context<'a> {
        checkpoint: &'a mut dyn FnMut(u64) -> Result<(), OpError>,
        source: PathBuf,
        failed_path: Option<PathBuf>,
        finished_files: u64,
        stopped: Option<OpError>,
    }

    fn callback_path(path: *const c_char) -> Option<PathBuf> {
        if path.is_null() {
            return None;
        }
        // SAFETY: copyfile passes a valid NUL-terminated path for the callback.
        let path = unsafe { CStr::from_ptr(path) };
        Some(PathBuf::from(std::ffi::OsStr::from_bytes(path.to_bytes())))
    }

    fn source_size(path: *const c_char) -> u64 {
        callback_path(path).and_then(|path| std::fs::symlink_metadata(path).ok()).map_or(0, |m| m.len())
    }

    extern "C" fn callback(
        what: c_int,
        stage: c_int,
        state: libc::copyfile_state_t,
        src: *const c_char,
        _dst: *const c_char,
        ctx: *mut c_void,
    ) -> c_int {
        // SAFETY: ctx is the Boxed Context passed below, alive for the whole copyfile call.
        let ctx = unsafe { &mut *ctx.cast::<Context>() };
        let mut copied: libc::off_t = 0;
        // SAFETY: state is valid during the callback; COPIED writes one off_t.
        unsafe { libc::copyfile_state_get(state, libc::COPYFILE_STATE_COPIED as u32, (&raw mut copied).cast()) };
        let copied = copied.max(0) as u64;
        if stage == libc::COPYFILE_ERR || what == libc::COPYFILE_RECURSE_ERROR {
            ctx.failed_path = callback_path(src);
        }
        if what == libc::COPYFILE_RECURSE_FILE && stage == libc::COPYFILE_FINISH {
            ctx.finished_files = ctx.finished_files.saturating_add(source_size(src));
        }
        let total = if what == libc::COPYFILE_COPY_DATA && stage == libc::COPYFILE_PROGRESS {
            ctx.finished_files.saturating_add(copied)
        } else {
            ctx.finished_files
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (ctx.checkpoint)(total)));
        let error = match result {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(error),
            Err(_) => Some(OpError::Io(ctx.source.clone(), Arc::new(io::Error::other("copy callback panicked")))),
        };
        if let Some(error) = error {
            ctx.stopped = Some(error);
            return libc::COPYFILE_QUIT;
        }
        libc::COPYFILE_CONTINUE
    }

    let (src, dst) = (
        CString::new(source.as_os_str().as_bytes()).map_err(|e| op_io(source, e.into()))?,
        CString::new(dest.as_os_str().as_bytes()).map_err(|e| op_io(dest, e.into()))?,
    );
    let mut ctx = Box::new(Context {
        checkpoint,
        source: source.to_path_buf(),
        failed_path: None,
        finished_files: 0,
        stopped: None,
    });
    let mut flags = libc::COPYFILE_METADATA
        | libc::COPYFILE_DATA
        | libc::COPYFILE_RECURSIVE
        | libc::COPYFILE_NOFOLLOW
        | libc::COPYFILE_EXCL;
    if clone {
        flags |= libc::COPYFILE_CLONE;
    }
    // SAFETY: the state and Boxed context outlive the copyfile call.
    let rc = unsafe {
        let state = libc::copyfile_state_alloc();
        if state.is_null() {
            return Err(op_io(source, io::Error::last_os_error()));
        }
        libc::copyfile_state_set(state, libc::COPYFILE_STATE_STATUS_CB as u32, callback as *const c_void);
        libc::copyfile_state_set(state, libc::COPYFILE_STATE_STATUS_CTX as u32, (&raw mut *ctx).cast());
        let rc = libc::copyfile(src.as_ptr(), dst.as_ptr(), state, flags);
        // errno is read before the state is freed: freeing may overwrite it.
        let error = (rc != 0).then(io::Error::last_os_error);
        libc::copyfile_state_free(state);
        (rc, error)
    };
    let (rc, os_error) = rc;
    if let Some(error) = ctx.stopped.take() {
        return Err(error);
    }
    if rc == 0 {
        Ok(())
    } else {
        // Name the child copyfile failed on, not the tree root.
        let path = ctx.failed_path.take().unwrap_or_else(|| source.to_path_buf());
        Err(op_io(&path, os_error.unwrap_or_else(io::Error::last_os_error)))
    }
}

#[cfg(not(target_os = "macos"))]
fn copy_tree(
    source: &Path,
    dest: &Path,
    _clone: bool,
    checkpoint: &mut dyn FnMut(u64) -> Result<(), OpError>,
) -> Result<(), OpError> {
    use std::io::{Read, Write};

    fn copy_entry(
        source: &Path,
        dest: &Path,
        copied: &mut u64,
        checkpoint: &mut dyn FnMut(u64) -> Result<(), OpError>,
    ) -> Result<(), OpError> {
        let meta = source.symlink_metadata().map_err(io_err(source))?;
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(source).map_err(io_err(source))?;
            std::os::unix::fs::symlink(target, dest).map_err(io_err(dest))?;
            checkpoint(*copied)?;
            return Ok(());
        }
        if meta.is_dir() {
            std::fs::create_dir(dest).map_err(io_err(dest))?;
            for entry in std::fs::read_dir(source).map_err(io_err(source))? {
                let entry = entry.map_err(io_err(source))?;
                copy_entry(&entry.path(), &dest.join(entry.file_name()), copied, checkpoint)?;
            }
            std::fs::set_permissions(dest, meta.permissions()).map_err(io_err(dest))?;
            checkpoint(*copied)?;
            return Ok(());
        }
        if !meta.is_file() {
            return Err(OpError::Unsupported(source.to_path_buf(), "copying this file type"));
        }
        let mut input = std::fs::File::open(source).map_err(io_err(source))?;
        let mut output = std::fs::OpenOptions::new().write(true).create_new(true).open(dest).map_err(io_err(dest))?;
        let mut buffer = vec![0u8; 1 << 20];
        loop {
            let read = input.read(&mut buffer).map_err(io_err(source))?;
            if read == 0 {
                break;
            }
            output.write_all(&buffer[..read]).map_err(io_err(dest))?;
            *copied = copied.saturating_add(read as u64);
            checkpoint(*copied)?;
        }
        output.set_permissions(meta.permissions()).map_err(io_err(dest))?;
        if let (Ok(accessed), Ok(modified)) = (meta.accessed(), meta.modified()) {
            let times = std::fs::FileTimes::new().set_accessed(accessed).set_modified(modified);
            output.set_times(times).map_err(io_err(dest))?;
        }
        checkpoint(*copied)?;
        Ok(())
    }

    let mut copied = 0;
    copy_entry(source, dest, &mut copied, checkpoint)
}
