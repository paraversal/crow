//! Package lifecycle: discovery, grants, one worker per enabled package,
//! generation checks and the inventory the app renders. All I/O here runs on
//! the service thread, never on the UI thread.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};

use crow_core::{Descriptor, ExtensionId, HookEvent, PluginId};

use crate::dto::{
    Call, CallError, CallErrorKind, CancelToken, HostEvent, Inventory, PluginEntry, PluginState, Reply, RequestError,
    RequestErrorKind, RequestInput, RequestResult, RequestStamp, RequestTicket, SubmitError, WorkerMessage,
};
use crate::grants::{self, Grants};
use crate::limits::Limits;
use crate::manifest::FsAccess;
use crate::package::{self, PackageCandidate, PackageSnapshot};
use crate::worker::{EnqueueError, RefreshEnqueue, Worker};

pub(crate) enum Inbox {
    Command(HostCommand),
    /// One handle submission is waiting on the service ingress.
    Submission,
    Worker(WorkerMessage),
}

pub(crate) enum HostCommand {
    Refresh,
    Approve { id: PluginId, digest: String },
    Disable(PluginId),
    Enable(PluginId),
    Reload(PluginId),
    Retry(PluginId),
    Shutdown,
}

/// One accepted handle call, before it becomes an admitted request.
pub(crate) enum Submission {
    Evaluate { extension: ExtensionId, input: RequestInput, stamp: RequestStamp, cancel: CancelToken },
    Notify { extension: ExtensionId, event: HookEvent, stamp: RequestStamp, cancel: CancelToken },
}

struct WorkerSlot {
    generation: u64,
    worker: Worker,
}

/// The single authority for one valid plugin ID. Only `Starting` and `Running`
/// can own a worker and a live generation; only `Running` publishes
/// descriptors.
struct RuntimeRecord {
    snapshot: Arc<PackageSnapshot>,
    state: PluginState,
    detail: Option<String>,
    read_allowed: bool,
    worker: Option<WorkerSlot>,
    descriptors: Vec<Descriptor>,
    icon_packs: Vec<crate::dto::RegisteredIconPack>,
}

impl RuntimeRecord {
    fn new(snapshot: Arc<PackageSnapshot>) -> Self {
        Self {
            snapshot,
            state: PluginState::PendingApproval,
            detail: None,
            read_allowed: false,
            worker: None,
            descriptors: Vec::new(),
            icon_packs: Vec::new(),
        }
    }

    fn generation(&self) -> u64 {
        self.worker.as_ref().map_or(0, |slot| slot.generation)
    }
}

struct AdmittedRequest {
    extension: ExtensionId,
    stamp: RequestStamp,
    cancel: CancelToken,
}

enum GrantVerdict {
    Authorized(FsAccess),
    Missing,
    Disabled,
    Mismatch(Option<String>),
}

pub struct PluginHost {
    plugins_dir: PathBuf,
    grants_path: PathBuf,
    limits: Limits,
    candidates: Vec<PackageCandidate>,
    grants: Grants,
    grants_error: Option<String>,
    scan_error: Option<String>,
    runtime: HashMap<PluginId, RuntimeRecord>,
    requests: HashMap<u64, AdmittedRequest>,
    reaping: Vec<JoinHandle<()>>,
    next_generation: u64,
    events: Sender<HostEvent>,
    inbox: Sender<Inbox>,
    reported: HashMap<OsString, String>,
    reported_grants: Option<String>,
    reported_scan: Option<String>,
}

impl PluginHost {
    pub(crate) fn new(config_dir: &Path, limits: Limits, events: Sender<HostEvent>, inbox: Sender<Inbox>) -> Self {
        Self {
            plugins_dir: config_dir.join("plugins"),
            grants_path: grants::grants_path(config_dir),
            limits,
            candidates: Vec::new(),
            grants: Grants::default(),
            grants_error: None,
            scan_error: None,
            runtime: HashMap::new(),
            requests: HashMap::new(),
            reaping: Vec::new(),
            next_generation: 0,
            events,
            inbox,
            reported: HashMap::new(),
            reported_grants: None,
            reported_scan: None,
        }
    }

    pub fn inventory(&self) -> Inventory {
        self.build_inventory()
    }

    /// Re-read grants and bundles, withdraw anything the grants no longer
    /// authorize, and reconcile one runtime record per valid plugin ID.
    pub fn refresh(&mut self) {
        self.reap();
        match self.reload_grants_from_disk() {
            Ok(()) => self.withdraw_revoked(),
            Err(error) => {
                self.grants = Grants::default();
                self.grants_error = Some(error.clone());
                self.withdraw_all_for_grants_error();
            }
        }
        self.scan_candidates();
        self.reconcile();
        self.reap();
        self.publish();
    }

    fn reload_grants_from_disk(&mut self) -> Result<(), String> {
        grants::ensure_grants_file(&self.grants_path)?;
        self.grants = grants::load_grants(&self.grants_path)?;
        self.grants_error = None;
        Ok(())
    }

    fn scan_candidates(&mut self) {
        let discovery = package::discover(&self.plugins_dir, &self.limits);
        self.scan_error = discovery.error;
        self.candidates = discovery.candidates;
    }

    fn grant_verdict(&self, id: &PluginId, snapshot: &PackageSnapshot) -> GrantVerdict {
        match self.grants.get(id) {
            None => GrantVerdict::Missing,
            Some(grant) if !grant.enabled => GrantVerdict::Disabled,
            Some(grant) if grant.authorizes(snapshot) => GrantVerdict::Authorized(grant.fs),
            Some(grant) => GrantVerdict::Mismatch(Some(if grant.digest != snapshot.digest {
                "package changed since it was approved".to_owned()
            } else {
                "requested capabilities changed since approval".to_owned()
            })),
        }
    }

    /// Before the bundle scan: a grant that is gone, disabled or no longer
    /// matches this exact snapshot cannot keep running. This is the one path
    /// that makes Approve B withdraw A after an external grant edit.
    fn withdraw_revoked(&mut self) {
        let ids: Vec<PluginId> = self.runtime.keys().cloned().collect();
        for id in ids {
            let Some(record) = self.runtime.get(&id) else { continue };
            let (state, detail) = match self.grant_verdict(&id, &record.snapshot) {
                GrantVerdict::Authorized(_) => continue,
                GrantVerdict::Missing => (PluginState::PendingApproval, None),
                GrantVerdict::Disabled => (PluginState::Disabled, None),
                GrantVerdict::Mismatch(detail) => (PluginState::PendingApproval, detail),
            };
            self.withdraw_worker(&id);
            if let Some(record) = self.runtime.get_mut(&id) {
                record.state = state;
                record.detail = detail;
            }
        }
    }

    fn withdraw_all_for_grants_error(&mut self) {
        let ids: Vec<PluginId> = self.runtime.keys().cloned().collect();
        for id in ids {
            self.withdraw_worker(&id);
            if let Some(record) = self.runtime.get_mut(&id) {
                record.state = PluginState::Disabled;
                record.detail = self.grants_error.clone();
            }
        }
    }

    fn reconcile(&mut self) {
        let mut valid: HashSet<PluginId> = HashSet::new();
        let candidates = self.candidates.clone();
        for candidate in &candidates {
            let Some(id) = candidate.id.clone() else { continue };
            let Ok(snapshot) = &candidate.snapshot else { continue };
            if !valid.insert(id.clone()) {
                continue;
            }
            self.reconcile_one(&id, snapshot.clone());
        }
        let stale: Vec<PluginId> = self.runtime.keys().filter(|id| !valid.contains(*id)).cloned().collect();
        for id in stale {
            self.remove_record(&id);
        }
    }

    fn reconcile_one(&mut self, id: &PluginId, snapshot: Arc<PackageSnapshot>) {
        let same_snapshot = self.runtime.get(id).is_some_and(|record| record.snapshot.digest == snapshot.digest);
        if !same_snapshot {
            self.withdraw_worker(id);
            let record = self.runtime.entry(id.clone()).or_insert_with(|| RuntimeRecord::new(snapshot.clone()));
            if record.snapshot.digest != snapshot.digest {
                record.snapshot = snapshot.clone();
                record.descriptors.clear();
                record.icon_packs.clear();
                record.state = PluginState::PendingApproval;
                record.detail = None;
            }
        }
        let Some(record) = self.runtime.get(id) else { return };
        let verdict = self.grant_verdict(id, &record.snapshot);
        if self.grants_error.is_some() {
            let error = self.grants_error.clone();
            self.withdraw_worker(id);
            if let Some(record) = self.runtime.get_mut(id) {
                record.state = PluginState::Disabled;
                record.detail = error;
            }
            return;
        }
        match verdict {
            GrantVerdict::Authorized(fs) => {
                let (state, has_worker) = {
                    let record = self.runtime.get(id).unwrap();
                    (record.state, record.worker.is_some())
                };
                if let Some(record) = self.runtime.get_mut(id) {
                    record.read_allowed = fs.permits_read();
                }
                // Paused and Failed stay sticky for this snapshot; Retry is
                // the only path that asks for a fresh generation.
                if matches!(state, PluginState::Paused | PluginState::Failed) {
                    return;
                }
                if matches!(state, PluginState::Starting | PluginState::Running) && has_worker {
                    return;
                }
                let snapshot = self.runtime.get(id).unwrap().snapshot.clone();
                self.start_worker(id, snapshot, fs);
            }
            GrantVerdict::Missing => {
                self.withdraw_worker(id);
                if let Some(record) = self.runtime.get_mut(id) {
                    record.state = PluginState::PendingApproval;
                    record.detail = None;
                }
            }
            GrantVerdict::Disabled => {
                self.withdraw_worker(id);
                if let Some(record) = self.runtime.get_mut(id) {
                    record.state = PluginState::Disabled;
                    record.detail = None;
                }
            }
            GrantVerdict::Mismatch(detail) => {
                self.withdraw_worker(id);
                if let Some(record) = self.runtime.get_mut(id) {
                    record.state = PluginState::PendingApproval;
                    record.detail = detail;
                }
            }
        }
    }

    fn start_worker(&mut self, id: &PluginId, snapshot: Arc<PackageSnapshot>, fs: FsAccess) {
        let generation = self.alloc_generation();
        match Worker::spawn(id.clone(), generation, snapshot, self.limits.clone(), self.inbox.clone()) {
            Ok(mut worker) => {
                let mailbox = worker.mailbox.clone();
                let Some(record) = self.runtime.get_mut(id) else {
                    worker.stop();
                    return;
                };
                record.worker = Some(WorkerSlot { generation, worker });
                record.state = PluginState::Starting;
                record.detail = None;
                record.read_allowed = fs.permits_read();
                record.descriptors.clear();
                record.icon_packs.clear();
                if mailbox.enqueue_command(Call::Register).is_err() {
                    self.worker_failed(id, generation, CallError::script("worker mailbox is full"));
                }
            }
            Err(error) => {
                if let Some(record) = self.runtime.get_mut(id) {
                    record.state = PluginState::Failed;
                    record.detail = Some(error);
                    record.worker = None;
                    record.descriptors.clear();
                    record.icon_packs.clear();
                }
            }
        }
    }

    fn alloc_generation(&mut self) -> u64 {
        self.next_generation += 1;
        self.next_generation
    }

    pub fn approve(&mut self, id: &PluginId, expected_digest: &str) -> Result<(), String> {
        if let Err(error) = self.reload_grants_from_disk() {
            self.grants = Grants::default();
            self.grants_error = Some(error.clone());
            self.withdraw_all_for_grants_error();
            self.publish();
            return Err(format!("plugins.toml: {error}"));
        }
        self.withdraw_revoked();
        self.scan_candidates();
        self.reconcile();

        let matching = self.candidates.iter().find(|candidate| {
            candidate.id.as_ref() == Some(id)
                && candidate.snapshot.as_ref().is_ok_and(|snapshot| snapshot.digest == expected_digest)
        });
        let Some(candidate) = matching else {
            self.publish();
            if let Some(candidate) = self.candidate(id) {
                if let Err(error) = &candidate.snapshot {
                    return Err(format!("{}: {}", candidate_name(candidate), error.message));
                }
                return Err(format!(
                    "{}: the package changed on disk; review the new snapshot before approving",
                    candidate_name(candidate)
                ));
            }
            return Err(format!("no package \"{id}\""));
        };
        let snapshot = candidate.snapshot.as_ref().expect("matching snapshot").clone();
        if let Err(error) = grants::approve(&self.grants_path, id, &snapshot) {
            self.refresh();
            return Err(format!("plugins.toml: {error}"));
        }
        if let Err(error) = self.reload_grants_from_disk() {
            self.refresh();
            return Err(format!("plugins.toml: {error}"));
        }
        self.reconcile();
        self.publish();
        Ok(())
    }

    pub fn disable(&mut self, id: &PluginId) -> Result<(), String> {
        if let Err(error) = self.reload_grants_from_disk() {
            self.refresh();
            return Err(format!("plugins.toml: {error}"));
        }
        if self.grants.get(id).is_some()
            && let Err(error) = grants::set_enabled(&self.grants_path, id, false)
        {
            self.refresh();
            return Err(format!("plugins.toml: {error}"));
        }
        self.refresh();
        Ok(())
    }

    pub fn enable(&mut self, id: &PluginId) -> Result<(), String> {
        if let Err(error) = self.reload_grants_from_disk() {
            self.refresh();
            return Err(format!("plugins.toml: {error}"));
        }
        if self.grants.get(id).is_none() {
            self.refresh();
            return Err("approve the package before enabling it".into());
        }
        if !self.grants.get(id).is_some_and(|grant| grant.enabled)
            && let Err(error) = grants::set_enabled(&self.grants_path, id, true)
        {
            self.refresh();
            return Err(format!("plugins.toml: {error}"));
        }
        self.refresh();
        Ok(())
    }

    pub fn reload(&mut self, _id: &PluginId) {
        self.refresh();
    }

    /// Refresh first, then start a fresh generation only if the same approved
    /// snapshot is still Paused or Failed.
    pub fn retry(&mut self, id: &PluginId) -> Result<(), String> {
        self.refresh();
        let Some(record) = self.runtime.get(id) else {
            return Err("no package for that plugin".into());
        };
        if !matches!(record.state, PluginState::Paused | PluginState::Failed) {
            return Err("the package is not paused or failed".into());
        }
        let snapshot = record.snapshot.clone();
        let fs = if record.read_allowed { FsAccess::Read } else { FsAccess::None };
        self.withdraw_worker(id);
        self.start_worker(id, snapshot, fs);
        self.publish();
        Ok(())
    }

    pub(crate) fn handle_inbox(&mut self, message: Inbox) {
        match message {
            Inbox::Command(command) => self.command(command),
            Inbox::Submission => {}
            Inbox::Worker(message) => self.worker_message(message),
        }
    }

    fn command(&mut self, command: HostCommand) {
        match command {
            HostCommand::Refresh => self.refresh(),
            HostCommand::Shutdown => {}
            HostCommand::Approve { id, digest } => {
                if let Err(error) = self.approve(&id, &digest) {
                    self.status(&id, error, true);
                }
            }
            HostCommand::Disable(id) => {
                if let Err(error) = self.disable(&id) {
                    self.status(&id, error, true);
                }
            }
            HostCommand::Enable(id) => {
                if let Err(error) = self.enable(&id) {
                    self.status(&id, error, true);
                }
            }
            HostCommand::Reload(id) => self.reload(&id),
            HostCommand::Retry(id) => {
                if let Err(error) = self.retry(&id) {
                    self.status(&id, error, true);
                }
            }
        }
        self.reap();
    }

    /// Admit one handle submission. A ticket accepted at ingress always gets
    /// exactly one terminal event, including every admission rejection.
    pub(crate) fn admit(&mut self, submission: Submission) {
        match submission {
            Submission::Evaluate { extension, input, stamp, cancel } => {
                self.admit_evaluate(extension, input, stamp, cancel)
            }
            Submission::Notify { extension, event, stamp, cancel } => {
                self.admit_notify(extension, event, stamp, cancel)
            }
        }
        self.reap();
    }

    fn admit_evaluate(
        &mut self,
        extension: ExtensionId,
        input: RequestInput,
        stamp: RequestStamp,
        cancel: CancelToken,
    ) {
        if cancel.load(Ordering::Relaxed) {
            self.finish_error(&extension, stamp, RequestError::cancelled());
            return;
        }
        let Some(slot) = self.running_slot(&extension.plugin, stamp.generation) else {
            self.finish_admission_error(&extension, stamp, &extension.plugin);
            return;
        };
        let Some(descriptor) = self.running_descriptor(&extension) else {
            self.finish_error(
                &extension,
                stamp,
                RequestError::new(
                    RequestErrorKind::Unavailable,
                    format!("extension \"{extension}\" is not registered"),
                ),
            );
            return;
        };
        if matches!(descriptor.kind, crow_core::ExtensionKind::Hook | crow_core::ExtensionKind::IconPack) {
            self.finish_error(
                &extension,
                stamp,
                RequestError::new(
                    RequestErrorKind::InvalidInput,
                    "use Notify for hooks and registration for icon packs",
                ),
            );
            return;
        }
        if let Err(error) = input.validate(&self.limits) {
            self.finish_error(&extension, stamp, RequestError::new(RequestErrorKind::InvalidInput, error));
            return;
        }
        if input.excerpt.is_some() {
            let read_allowed = self.runtime.get(&extension.plugin).is_some_and(|record| record.read_allowed);
            if !read_allowed {
                self.finish_error(
                    &extension,
                    stamp,
                    RequestError::new(
                        RequestErrorKind::Capability,
                        format!("plugin \"{}\" has no fs read capability", extension.plugin),
                    ),
                );
                return;
            }
        }
        let call = Call::Evaluate { extension: extension.clone(), input, stamp, cancel: cancel.clone() };
        match slot.worker.mailbox.enqueue_command(call) {
            Ok(()) => {
                self.requests.insert(stamp.request_id, AdmittedRequest { extension, stamp, cancel });
            }
            Err(EnqueueError::Busy) => {
                self.finish_error(
                    &extension,
                    stamp,
                    RequestError::new(RequestErrorKind::Busy, "the plugin worker lane is full"),
                );
            }
        }
    }

    fn admit_notify(&mut self, extension: ExtensionId, event: HookEvent, stamp: RequestStamp, cancel: CancelToken) {
        if cancel.load(Ordering::Relaxed) {
            self.finish_error(&extension, stamp, RequestError::cancelled());
            return;
        }
        let Some(slot) = self.running_slot(&extension.plugin, stamp.generation) else {
            self.finish_admission_error(&extension, stamp, &extension.plugin);
            return;
        };
        let Some(descriptor) = self.running_descriptor(&extension) else {
            self.finish_error(
                &extension,
                stamp,
                RequestError::new(
                    RequestErrorKind::Unavailable,
                    format!("extension \"{extension}\" is not registered"),
                ),
            );
            return;
        };
        if descriptor.kind != crow_core::ExtensionKind::Hook {
            self.finish_error(
                &extension,
                stamp,
                RequestError::new(RequestErrorKind::InvalidInput, "only hooks can be notified"),
            );
            return;
        }
        if let Some(scope) = event.scope()
            && (scope.len() > self.limits.max_display_bytes || scope.chars().any(char::is_control))
        {
            self.finish_error(
                &extension,
                stamp,
                RequestError::new(RequestErrorKind::InvalidInput, "hook scope is not bounded plain text"),
            );
            return;
        }
        let call = Call::Notify { extension: extension.clone(), event, stamp, cancel: cancel.clone() };
        match slot.worker.mailbox.enqueue_refresh(call) {
            RefreshEnqueue::Queued => {
                self.requests.insert(stamp.request_id, AdmittedRequest { extension, stamp, cancel });
            }
            RefreshEnqueue::Replaced(displaced) => {
                self.requests.insert(stamp.request_id, AdmittedRequest { extension, stamp, cancel });
                self.finish_displaced(displaced);
            }
            RefreshEnqueue::Full => {
                self.finish_error(
                    &extension,
                    stamp,
                    RequestError::new(RequestErrorKind::Busy, "the plugin hook lane is full"),
                );
            }
            RefreshEnqueue::Closed => {
                self.finish_error(
                    &extension,
                    stamp,
                    RequestError::new(RequestErrorKind::WorkerFailed, "the plugin worker stopped"),
                );
            }
        }
    }

    fn finish_admission_error(&mut self, extension: &ExtensionId, stamp: RequestStamp, id: &PluginId) {
        let (kind, message) = match self.runtime.get(id) {
            None => (RequestErrorKind::Unavailable, format!("plugin \"{id}\" is not installed")),
            Some(record) if record.generation() != stamp.generation => (
                RequestErrorKind::StaleGeneration,
                format!("generation {} is not current for plugin \"{id}\"", stamp.generation),
            ),
            Some(_) => (RequestErrorKind::Unavailable, format!("plugin \"{id}\" is not running")),
        };
        self.finish_error(extension, stamp, RequestError::new(kind, message));
    }

    fn finish_displaced(&mut self, call: Call) {
        let Some(stamp) = call.stamp() else { return };
        let Some(request) = self.requests.remove(&stamp.request_id) else { return };
        request.cancel.store(true, Ordering::Relaxed);
        self.send(HostEvent::RequestFinished {
            extension: request.extension,
            stamp,
            result: RequestResult::Error(RequestError::cancelled()),
        });
    }

    fn finish_error(&mut self, extension: &ExtensionId, stamp: RequestStamp, error: RequestError) {
        self.send(HostEvent::RequestFinished {
            extension: extension.clone(),
            stamp,
            result: RequestResult::Error(error),
        });
    }

    fn running_slot(&self, id: &PluginId, generation: u64) -> Option<&WorkerSlot> {
        let record = self.runtime.get(id)?;
        if record.state != PluginState::Running {
            return None;
        }
        record.worker.as_ref().filter(|slot| slot.generation == generation)
    }

    fn running_descriptor(&self, extension: &ExtensionId) -> Option<&Descriptor> {
        let record = self.runtime.get(&extension.plugin)?;
        if record.state != PluginState::Running {
            return None;
        }
        record.descriptors.iter().find(|descriptor| descriptor.id == *extension)
    }

    fn worker_message(&mut self, message: WorkerMessage) {
        match message {
            WorkerMessage::Registered { plugin, generation, result } => {
                if !self.is_current_worker(&plugin, generation) {
                    return;
                }
                let Some(record) = self.runtime.get(&plugin) else { return };
                if record.state != PluginState::Starting {
                    return;
                }
                match result {
                    Ok(descriptors) => {
                        let snapshot = record.snapshot.clone();
                        if let Err(error) =
                            crate::registry::validate_published(&plugin, &snapshot, &descriptors, &self.limits)
                        {
                            self.worker_failed(&plugin, generation, CallError::invalid(error));
                            return;
                        }
                        let icon_packs = descriptors
                            .iter()
                            .filter_map(|descriptor| {
                                descriptor.asset().map(|asset| {
                                    crate::dto::RegisteredIconPack::new(
                                        descriptor.id.clone(),
                                        generation,
                                        asset.to_owned(),
                                        snapshot.clone(),
                                    )
                                })
                            })
                            .collect();
                        if let Some(record) = self.runtime.get_mut(&plugin) {
                            record.state = PluginState::Running;
                            record.detail = None;
                            record.descriptors = descriptors;
                            record.icon_packs = icon_packs;
                        }
                        self.publish();
                    }
                    Err(error) => self.worker_failed(&plugin, generation, error),
                }
            }
            WorkerMessage::Replied { plugin, generation, extension, stamp, reply } => {
                let matches = self
                    .requests
                    .get(&stamp.request_id)
                    .is_some_and(|request| request.extension == extension && request.stamp == stamp);
                if !matches {
                    return;
                }
                let request = self.requests.remove(&stamp.request_id).expect("checked request");
                let current = self.is_running_generation(&plugin, generation);
                if request.cancel.load(Ordering::Relaxed) || !current {
                    self.send(HostEvent::RequestFinished {
                        extension,
                        stamp,
                        result: RequestResult::Error(RequestError::cancelled()),
                    });
                    return;
                }
                let result = match reply {
                    Reply::Intent(intent) => RequestResult::Intent(intent),
                    Reply::Refresh => RequestResult::Refresh,
                    Reply::Values(values) => RequestResult::Values(values),
                    Reply::Unit => RequestResult::Unit,
                };
                self.send(HostEvent::RequestFinished { extension, stamp, result });
            }
            WorkerMessage::Failed { plugin, generation, request, error } => {
                if let Some(stamp) = request
                    && let Some(record) = self
                        .requests
                        .get(&stamp.request_id)
                        .is_some_and(|record| record.stamp == stamp)
                        .then(|| self.requests.remove(&stamp.request_id))
                        .flatten()
                {
                    if error.kind == CallErrorKind::Cancelled || record.cancel.load(Ordering::Relaxed) {
                        self.send(HostEvent::RequestFinished {
                            extension: record.extension,
                            stamp,
                            result: RequestResult::Error(RequestError::cancelled()),
                        });
                    } else {
                        self.send(HostEvent::RequestFinished {
                            extension: record.extension,
                            stamp,
                            result: RequestResult::Error(RequestError::new(
                                RequestErrorKind::WorkerFailed,
                                error.message.clone(),
                            )),
                        });
                    }
                }
                // A fatal failure for the current generation still stops the
                // worker, even if the associated ticket had been cancelled.
                if error.kind != CallErrorKind::Cancelled {
                    self.worker_failed(&plugin, generation, error);
                }
            }
        }
        self.reap();
    }

    fn worker_failed(&mut self, id: &PluginId, generation: u64, error: CallError) {
        if error.kind == CallErrorKind::Cancelled || !self.is_current_worker(id, generation) {
            return;
        }
        self.withdraw_worker(id);
        let state = if error.kind == CallErrorKind::Budget { PluginState::Paused } else { PluginState::Failed };
        if let Some(record) = self.runtime.get_mut(id) {
            record.state = state;
            record.detail = Some(error.message);
        }
        self.publish();
    }

    fn is_current_worker(&self, id: &PluginId, generation: u64) -> bool {
        self.runtime.get(id).and_then(|record| record.worker.as_ref()).is_some_and(|slot| slot.generation == generation)
    }

    fn is_running_generation(&self, id: &PluginId, generation: u64) -> bool {
        self.runtime.get(id).is_some_and(|record| {
            record.state == PluginState::Running
                && record.worker.as_ref().is_some_and(|slot| slot.generation == generation)
        }) && self.is_current_worker(id, generation)
    }

    /// Cancel and finish every admitted request for this plugin, then stop its
    /// worker and withdraw its published descriptors.
    fn withdraw_worker(&mut self, id: &PluginId) {
        self.finish_requests_for(id);
        if let Some(record) = self.runtime.get_mut(id) {
            if let Some(mut slot) = record.worker.take() {
                slot.worker.stop();
                if let Some(join) = slot.worker.take_join() {
                    self.reaping.push(join);
                }
            }
            record.descriptors.clear();
            record.icon_packs.clear();
        }
    }

    fn finish_requests_for(&mut self, id: &PluginId) {
        let requests: Vec<u64> = self
            .requests
            .iter()
            .filter(|(_, request)| request.extension.plugin == *id)
            .map(|(request, _)| *request)
            .collect();
        for request_id in requests {
            let Some(request) = self.requests.remove(&request_id) else { continue };
            request.cancel.store(true, Ordering::Relaxed);
            self.send(HostEvent::RequestFinished {
                extension: request.extension,
                stamp: request.stamp,
                result: RequestResult::Error(RequestError::cancelled()),
            });
        }
    }

    fn remove_record(&mut self, id: &PluginId) {
        self.withdraw_worker(id);
        self.runtime.remove(id);
    }

    fn candidate(&self, id: &PluginId) -> Option<&PackageCandidate> {
        self.candidates.iter().find(|candidate| candidate.id.as_ref() == Some(id))
    }

    fn name_for(&self, id: &PluginId) -> String {
        self.candidate(id).map_or_else(|| id.to_string(), candidate_name)
    }

    fn status(&self, id: &PluginId, message: impl Into<String>, error: bool) {
        let name = self.name_for(id);
        let message = message.into();
        // Approval errors already name the package; the status line adds the
        // name once, never twice.
        let prefixed = if message.starts_with(&format!("{name}: ")) { message } else { format!("{name}: {message}") };
        self.send(HostEvent::Status { plugin: Some(id.clone()), name, message: prefixed, error });
    }

    fn send(&self, event: HostEvent) {
        let _ = self.events.send(event);
    }

    fn reap(&mut self) {
        let mut still = Vec::new();
        for join in self.reaping.drain(..) {
            if join.is_finished() {
                let _ = join.join();
            } else {
                still.push(join);
            }
        }
        self.reaping = still;
    }

    fn build_inventory(&self) -> Inventory {
        let mut plugins = Vec::new();
        for candidate in &self.candidates {
            let (state, detail, descriptors, icon_packs, generation) = match (&candidate.snapshot, &candidate.id) {
                (Err(error), _) => {
                    (PluginState::from(&error.kind), Some(error.message.clone()), Vec::new(), Vec::new(), 0)
                }
                (Ok(snapshot), Some(id)) => match self.runtime.get(id) {
                    Some(record) if record.snapshot.digest == snapshot.digest => (
                        record.state,
                        record.detail.clone(),
                        record.descriptors.clone(),
                        record.icon_packs.clone(),
                        record.generation(),
                    ),
                    _ => (PluginState::PendingApproval, None, Vec::new(), Vec::new(), 0),
                },
                (Ok(_), None) => (PluginState::PendingApproval, None, Vec::new(), Vec::new(), 0),
            };
            let (provides, digest, requested_fs, version) = match &candidate.snapshot {
                Ok(snapshot) => (
                    snapshot.manifest.provides.clone(),
                    Some(snapshot.digest.clone()),
                    snapshot.manifest.permissions.fs(),
                    Some(snapshot.manifest.version.clone()),
                ),
                Err(_) => (Vec::new(), None, FsAccess::None, candidate.version.clone()),
            };
            plugins.push(PluginEntry {
                bundle: candidate.bundle.clone(),
                dir_name: candidate.dir_name.clone(),
                id: candidate.id.clone(),
                name: candidate.name.clone().unwrap_or_else(|| candidate.dir_name.clone()),
                version,
                provides,
                state,
                detail,
                digest,
                requested_fs,
                descriptors,
                icon_packs,
                generation,
            });
        }
        Inventory { plugins, grants_error: self.grants_error.clone(), scan_error: self.scan_error.clone() }
    }

    fn publish(&mut self) {
        let inventory = self.build_inventory();
        let present: HashSet<OsString> = inventory.plugins.iter().map(|entry| entry.bundle.clone()).collect();
        self.reported.retain(|dir, _| present.contains(dir));
        let mut statuses = Vec::new();
        for entry in &inventory.plugins {
            if entry.state.is_error() {
                if let Some(detail) = &entry.detail {
                    let key = entry.bundle.clone();
                    if self.reported.get(&key) != Some(detail) {
                        statuses.push((
                            entry.id.clone(),
                            entry.title().to_owned(),
                            format!("{}: {detail}", entry.title()),
                        ));
                        self.reported.insert(key, detail.clone());
                    }
                }
            } else {
                self.reported.remove(&entry.bundle);
            }
        }
        if self.reported_grants != inventory.grants_error {
            if let Some(error) = &inventory.grants_error {
                statuses.push((None, "Plugins".into(), format!("plugins.toml: {error}; grants withdrawn")));
            }
            self.reported_grants = inventory.grants_error.clone();
        }
        if self.reported_scan != inventory.scan_error {
            if let Some(error) = &inventory.scan_error {
                statuses.push((None, "Plugins".into(), format!("plugins: {error}")));
            }
            self.reported_scan = inventory.scan_error.clone();
        }
        for (plugin, name, message) in statuses {
            match &plugin {
                Some(id) => tracing::error!(plugin = %id, %name, %message, "plugin error"),
                None => tracing::error!(%name, %message, "plugin error"),
            }
            self.send(HostEvent::Status { plugin, name, message, error: true });
        }
        self.send(HostEvent::Inventory(inventory));
    }

    pub fn shutdown(&mut self) {
        let ids: Vec<PluginId> = self.runtime.keys().cloned().collect();
        for id in ids {
            self.withdraw_worker(&id);
        }
        for join in self.reaping.drain(..) {
            let _ = join.join();
        }
    }
}

impl Drop for PluginHost {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn candidate_name(candidate: &PackageCandidate) -> String {
    candidate.name.clone().unwrap_or_else(|| candidate.dir_name.clone())
}

/// The app-owned service. Commands and worker messages share the control
/// inbox; handle submissions and their wakes share one bounded ingress.
pub struct PluginService {
    commands: Sender<Inbox>,
    ingress: SyncSender<Submission>,
    next_request: Arc<AtomicU64>,
    _join: JoinHandle<()>,
}

impl PluginService {
    pub fn start(config_dir: PathBuf, limits: Limits) -> Result<(Self, Receiver<HostEvent>), String> {
        let (command_tx, command_rx) = mpsc::channel::<Inbox>();
        let (event_tx, event_rx) = mpsc::channel::<HostEvent>();
        let (ingress_tx, ingress_rx) = mpsc::sync_channel::<Submission>(limits.service_mailbox.max(1));
        let inbox = command_tx.clone();
        let next_request = Arc::new(AtomicU64::new(0));
        let join = thread::Builder::new()
            .name("crow-plugins".into())
            .spawn(move || {
                let mut host = PluginHost::new(&config_dir, limits, event_tx, inbox);
                host.refresh();
                while let Ok(message) = command_rx.recv() {
                    match message {
                        Inbox::Command(HostCommand::Shutdown) => break,
                        Inbox::Submission => {
                            if let Ok(submission) = ingress_rx.try_recv() {
                                host.admit(submission);
                            }
                        }
                        other => host.handle_inbox(other),
                    }
                }
                host.shutdown();
            })
            .map_err(|error| format!("could not start the plugin service: {error}"))?;
        Ok((Self { commands: command_tx, ingress: ingress_tx, next_request, _join: join }, event_rx))
    }

    pub fn handle(&self) -> PluginHandle {
        PluginHandle::new(self.commands.clone(), self.ingress.clone(), self.next_request.clone())
    }
}

impl Drop for PluginService {
    fn drop(&mut self) {
        let _ = self.commands.send(Inbox::Command(HostCommand::Shutdown));
    }
}

#[derive(Clone)]
pub struct PluginHandle {
    commands: Sender<Inbox>,
    ingress: SyncSender<Submission>,
    next_request: Arc<AtomicU64>,
}

impl PluginHandle {
    fn new(commands: Sender<Inbox>, ingress: SyncSender<Submission>, next_request: Arc<AtomicU64>) -> Self {
        Self { commands, ingress, next_request }
    }

    pub fn refresh(&self) {
        let _ = self.commands.send(Inbox::Command(HostCommand::Refresh));
    }

    pub fn approve(&self, id: PluginId, digest: String) -> Result<(), String> {
        self.send(HostCommand::Approve { id, digest })
    }

    pub fn disable(&self, id: PluginId) -> Result<(), String> {
        self.send(HostCommand::Disable(id))
    }

    pub fn enable(&self, id: PluginId) -> Result<(), String> {
        self.send(HostCommand::Enable(id))
    }

    pub fn reload(&self, id: PluginId) -> Result<(), String> {
        self.send(HostCommand::Reload(id))
    }

    pub fn retry(&self, id: PluginId) -> Result<(), String> {
        self.send(HostCommand::Retry(id))
    }

    pub fn try_evaluate(
        &self,
        extension: ExtensionId,
        generation: u64,
        context_revision: u64,
        input: RequestInput,
    ) -> Result<RequestTicket, SubmitError> {
        let (ticket, submission) =
            self.submission(extension, generation, context_revision, |extension, stamp, cancel| Submission::Evaluate {
                extension,
                input,
                stamp,
                cancel,
            });
        self.submit(submission)?;
        Ok(ticket)
    }

    pub fn try_notify(
        &self,
        extension: ExtensionId,
        generation: u64,
        context_revision: u64,
        event: HookEvent,
    ) -> Result<RequestTicket, SubmitError> {
        let (ticket, submission) =
            self.submission(extension, generation, context_revision, |extension, stamp, cancel| Submission::Notify {
                extension,
                event,
                stamp,
                cancel,
            });
        self.submit(submission)?;
        Ok(ticket)
    }

    fn submission(
        &self,
        extension: ExtensionId,
        generation: u64,
        context_revision: u64,
        build: impl FnOnce(ExtensionId, RequestStamp, CancelToken) -> Submission,
    ) -> (RequestTicket, Submission) {
        let request_id = self.next_request.fetch_add(1, Ordering::Relaxed) + 1;
        let stamp = RequestStamp { generation, request_id, context_revision };
        let cancel = Arc::new(AtomicBool::new(false));
        let ticket = RequestTicket::new(stamp, cancel.clone());
        (ticket, build(extension, stamp, cancel))
    }

    fn submit(&self, submission: Submission) -> Result<(), SubmitError> {
        match self.ingress.try_send(submission) {
            Ok(()) => self.commands.send(Inbox::Submission).map_err(|_| SubmitError::Stopped),
            Err(TrySendError::Full(_)) => Err(SubmitError::Busy),
            Err(TrySendError::Disconnected(_)) => Err(SubmitError::Stopped),
        }
    }

    pub fn shutdown(&self) {
        let _ = self.commands.send(Inbox::Command(HostCommand::Shutdown));
    }

    fn send(&self, command: HostCommand) -> Result<(), String> {
        self.commands.send(Inbox::Command(command)).map_err(|_| "the plugin service stopped".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc::Receiver;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::dto::{ExcerptInput, RequestInput};
    use crate::manifest::Permissions;
    use crow_core::{EntryData, EntryHandle, HookEvent, Snapshot};

    struct Harness {
        host: PluginHost,
        inbox: Receiver<Inbox>,
        events: Receiver<HostEvent>,
        last: Inventory,
        seen: Vec<HostEvent>,
        next_request: u64,
        dir: tempfile::TempDir,
    }

    impl Harness {
        fn new(scripts: &[(&str, &str)]) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let plugins = dir.path().join("plugins");
            std::fs::create_dir(&plugins).unwrap();
            for (id, script) in scripts {
                let name = id.rsplit('.').next().unwrap().to_owned();
                write_bundle(&plugins, id, &name, script, Permissions::default());
            }
            Self::from_dir(dir)
        }

        fn from_dir(dir: tempfile::TempDir) -> Self {
            let (inbox_tx, inbox) = mpsc::channel::<Inbox>();
            let (event_tx, events) = mpsc::channel::<HostEvent>();
            let mut host = PluginHost::new(dir.path(), test_limits(), event_tx, inbox_tx);
            host.refresh();
            let mut harness =
                Self { host, inbox, events, last: Inventory::default(), seen: Vec::new(), next_request: 0, dir };
            harness.pump_until(|_| true);
            harness
        }

        fn inventory(&self) -> &Inventory {
            &self.last
        }

        fn entry(&self, id: &str) -> &PluginEntry {
            self.last.plugins.iter().find(|entry| entry.id.as_ref().is_some_and(|p| p.as_str() == id)).unwrap()
        }

        fn pump_until(&mut self, until: impl Fn(&Self) -> bool) {
            let started = Instant::now();
            loop {
                while let Ok(message) = self.inbox.try_recv() {
                    self.host.handle_inbox(message);
                }
                self.drain_events();
                if until(self) {
                    return;
                }
                assert!(
                    started.elapsed() < Duration::from_secs(5),
                    "timed out; inventory={:?}; events={:?}",
                    self.last,
                    self.seen
                );
                std::thread::sleep(Duration::from_millis(2));
            }
        }

        fn drain_events(&mut self) {
            while let Ok(event) = self.events.try_recv() {
                if let HostEvent::Inventory(inventory) = &event {
                    self.last = inventory.clone();
                }
                self.seen.push(event);
            }
        }

        fn approve(&mut self, id: &str) {
            let digest = self.entry(id).digest.clone().unwrap();
            self.host.approve(&PluginId::parse(id).unwrap(), &digest).unwrap();
            let plugin = PluginId::parse(id).unwrap();
            self.pump_until(move |harness| {
                harness
                    .last
                    .plugins
                    .iter()
                    .any(|entry| entry.id.as_ref() == Some(&plugin) && entry.state == PluginState::Running)
            });
        }

        fn input() -> RequestInput {
            RequestInput {
                snapshot: Snapshot {
                    scope: "/tmp".into(),
                    selection: Some(EntryHandle(1)),
                    entries: vec![EntryData {
                        handle: EntryHandle(1),
                        name: "a.txt".into(),
                        is_dir: false,
                        size: 3,
                        kind: "Text".into(),
                    }],
                },
                excerpt: None,
            }
        }

        fn submit_evaluate(&mut self, id: &str, local: &str, generation: u64, input: RequestInput) -> RequestTicket {
            let extension = extension(id, local);
            self.next_request += 1;
            let stamp = RequestStamp { generation, request_id: self.next_request, context_revision: 7 };
            let cancel = Arc::new(AtomicBool::new(false));
            let ticket = RequestTicket::new(stamp, cancel.clone());
            self.host.admit(Submission::Evaluate { extension, input, stamp, cancel });
            self.drain_events();
            ticket
        }

        fn submit_notify(&mut self, id: &str, local: &str, generation: u64, event: HookEvent) -> RequestTicket {
            let extension = extension(id, local);
            self.next_request += 1;
            let stamp = RequestStamp { generation, request_id: self.next_request, context_revision: 4 };
            let cancel = Arc::new(AtomicBool::new(false));
            let ticket = RequestTicket::new(stamp, cancel.clone());
            self.host.admit(Submission::Notify { extension, event, stamp, cancel });
            self.drain_events();
            ticket
        }

        fn terminal(&self, request_id: u64) -> Option<&HostEvent> {
            self.seen.iter().find(
                |event| matches!(event, HostEvent::RequestFinished { stamp, .. } if stamp.request_id == request_id),
            )
        }

        fn count_terminal(&self, request_id: u64) -> usize {
            self.seen
                .iter()
                .filter(
                    |event| matches!(event, HostEvent::RequestFinished { stamp, .. } if stamp.request_id == request_id),
                )
                .count()
        }

        fn wait_terminal(&mut self, ticket: &RequestTicket) -> (ExtensionId, RequestStamp, RequestResult) {
            let request_id = ticket.stamp().request_id;
            self.pump_until(|harness| harness.terminal(request_id).is_some());
            match self.terminal(request_id).unwrap() {
                HostEvent::RequestFinished { extension, stamp, result } => (extension.clone(), *stamp, result.clone()),
                _ => unreachable!(),
            }
        }

        /// Pull one matching worker reply out of the inbox without giving it to
        /// the host, so a test can cancel between completion and delivery.
        fn take_reply(&mut self, request_id: u64) -> WorkerMessage {
            let started = Instant::now();
            loop {
                while let Ok(message) = self.inbox.try_recv() {
                    match message {
                        Inbox::Worker(message @ WorkerMessage::Replied { stamp, .. })
                            if stamp.request_id == request_id =>
                        {
                            return message;
                        }
                        other => self.host.handle_inbox(other),
                    }
                }
                assert!(started.elapsed() < Duration::from_secs(5), "worker never replied");
                std::thread::sleep(Duration::from_millis(2));
            }
        }

        fn feed(&mut self, message: WorkerMessage) {
            self.host.handle_inbox(Inbox::Worker(message));
            self.drain_events();
        }

        fn grants(&self) -> String {
            std::fs::read_to_string(self.dir.path().join("plugins.toml")).unwrap()
        }

        fn write_grants(&self, text: &str) {
            std::fs::write(self.dir.path().join("plugins.toml"), text).unwrap();
        }
    }

    fn test_limits() -> Limits {
        Limits { call_deadline: Duration::from_millis(250), ..Limits::default() }
    }

    fn extension(id: &str, local: &str) -> ExtensionId {
        ExtensionId::new(PluginId::parse(id).unwrap(), crow_core::LocalId::parse(local).unwrap())
    }

    fn write_bundle(plugins: &Path, id: &str, name: &str, script: &str, permissions: Permissions) -> PathBuf {
        let dir_name = id.replace('.', "-");
        let dir = plugins.join(&dir_name);
        std::fs::create_dir_all(&dir).unwrap();
        let permissions = match (permissions.fs, permissions.spawn.is_empty(), permissions.net) {
            (Some(FsAccess::Read), true, false) => r#"{ fs = "read", spawn = [], net = false }"#,
            _ => r#"{ fs = "none", spawn = [], net = false }"#,
        };
        let manifest = format!(
            r#"
id = "{id}"
name = "{name}"
version = "0.1.0"
api = 1
entry = "main.rhai"
provides = ["command", "hook"]
permissions = {permissions}
"#
        );
        std::fs::write(dir.join("plugin.toml"), manifest).unwrap();
        std::fs::write(dir.join("main.rhai"), script).unwrap();
        dir
    }

    const COMMAND: &str = r#"
fn register() { [ #{ kind: "command", id: "copy", title: "Copy", function: "run" } ] }
fn run(ctx) { #{ kind: "copy_text", text: ctx.scope + "/" + ctx.entries[0].name } }
"#;

    const COMMAND_AND_HOOK: &str = r#"
fn register() {
    [
        #{ kind: "command", id: "copy", title: "Copy", function: "run" },
        #{ kind: "hook", id: "changed", title: "Changed", function: "hook" }
    ]
}
fn run(ctx) { #{ kind: "copy_text", text: ctx.scope + "/" + ctx.entries[0].name } }
fn hook(ctx) { #{ refresh: true } }
"#;

    const LOOP: &str = r#"
fn register() { [ #{ kind: "command", id: "copy", title: "Copy", function: "run" } ] }
fn run(ctx) { while true { } }
"#;

    const EXCERPT: &str = r#"
fn register() { [ #{ kind: "command", id: "copy", title: "Copy", function: "run" } ] }
fn run(ctx) { #{ kind: "copy_text", text: ctx.entries[0].excerpt } }
"#;

    #[test]
    fn external_grant_edit_withdraws_a_when_approving_b() {
        let mut harness = Harness::new(&[("example.a", COMMAND), ("example.b", COMMAND)]);
        harness.approve("example.a");
        harness.approve("example.b");
        let generation = harness.entry("example.a").generation;
        let ticket = harness.submit_evaluate("example.a", "copy", generation, Harness::input());
        let _ready = harness.take_reply(ticket.stamp().request_id);
        // The reply is now queued in the test harness, not yet given to the host.

        let grants = harness
            .grants()
            .replace("[plugin.\"example.a\"]\nenabled = true", "[plugin.\"example.a\"]\nenabled = false");
        harness.write_grants(&grants);

        let b_digest = harness.entry("example.b").digest.clone().unwrap();
        harness.host.approve(&PluginId::parse("example.b").unwrap(), &b_digest).unwrap();
        harness.pump_until(|harness| harness.entry("example.a").state == PluginState::Disabled);

        let a = harness.entry("example.a");
        assert_eq!(a.descriptors.len(), 0);
        assert_eq!(a.generation, 0);
        assert_eq!(harness.entry("example.b").state, PluginState::Running);
        let (_, _, result) = harness.wait_terminal(&ticket);
        assert_eq!(result, RequestResult::Error(RequestError::cancelled()));

        // The old reply arriving after withdrawal must not become an intent.
        harness.feed(_ready);
        assert!(!harness.seen.iter().any(|event| matches!(
            event,
            HostEvent::RequestFinished { stamp, result: RequestResult::Intent(_), .. }
                if stamp.request_id == ticket.stamp().request_id
        )));
        assert_eq!(harness.terminal(ticket.stamp().request_id).count_matches(), 1);
    }

    trait CountMatches {
        fn count_matches(&self) -> usize;
    }

    impl CountMatches for Option<&HostEvent> {
        fn count_matches(&self) -> usize {
            usize::from(self.is_some())
        }
    }

    #[test]
    fn identical_reload_keeps_the_worker_and_generation() {
        let mut harness = Harness::new(&[("example.a", COMMAND)]);
        harness.approve("example.a");
        let generation = harness.entry("example.a").generation;
        harness.host.reload(&PluginId::parse("example.a").unwrap());
        harness.pump_until(|harness| harness.entry("example.a").state == PluginState::Running);
        assert_eq!(harness.entry("example.a").generation, generation);
        assert_eq!(harness.entry("example.a").descriptors.len(), 1);
        let ticket = harness.submit_evaluate("example.a", "copy", generation, Harness::input());
        let (_, _, result) = harness.wait_terminal(&ticket);
        assert!(
            matches!(result, RequestResult::Intent(crow_core::CommandIntent::CopyText(text)) if text == "/tmp/a.txt")
        );
    }

    #[test]
    fn changed_bytes_require_approval_and_withdraw_queued_work() {
        let mut harness = Harness::new(&[("example.a", COMMAND)]);
        harness.approve("example.a");
        let generation = harness.entry("example.a").generation;
        let ticket = harness.submit_evaluate("example.a", "copy", generation, Harness::input());
        let _ready = harness.take_reply(ticket.stamp().request_id);
        std::fs::write(harness.dir.path().join("plugins/example-a/main.rhai"), LOOP).unwrap();
        harness.host.refresh();
        harness.pump_until(|harness| harness.entry("example.a").state == PluginState::PendingApproval);
        let entry = harness.entry("example.a");
        assert_eq!(entry.descriptors.len(), 0);
        assert_eq!(entry.generation, 0);
        assert!(entry.detail.as_deref().unwrap().contains("changed"));
        let (_, _, result) = harness.wait_terminal(&ticket);
        assert_eq!(result, RequestResult::Error(RequestError::cancelled()));
        harness.feed(_ready);
        assert!(!harness.seen.iter().any(|event| matches!(
            event,
            HostEvent::RequestFinished { stamp, result: RequestResult::Intent(_), .. }
                if stamp.request_id == ticket.stamp().request_id
        )));
    }

    #[test]
    fn approving_a_changed_snapshot_is_refused_named_once_and_grants_nothing() {
        let mut harness = Harness::new(&[("example.a", COMMAND)]);
        let id = PluginId::parse("example.a").unwrap();
        let digest = harness.entry("example.a").digest.clone().unwrap();
        std::fs::write(harness.dir.path().join("plugins/example-a/main.rhai"), LOOP).unwrap();

        harness.host.command(HostCommand::Approve { id, digest });
        harness.pump_until(|harness| harness.entry("example.a").state == PluginState::PendingApproval);
        let status = harness
            .seen
            .iter()
            .rev()
            .find_map(|event| match event {
                HostEvent::Status { message, error: true, .. } => Some(message.clone()),
                _ => None,
            })
            .expect("a status event");
        assert_eq!(
            status, "a: the package changed on disk; review the new snapshot before approving",
            "the status line names the package once"
        );
        assert!(!harness.grants().contains("[plugin.\"example.a\"]"), "{}", harness.grants());
        assert!(harness.entry("example.a").descriptors.is_empty());
    }

    #[test]
    fn action_errors_name_the_plugin_once() {
        let mut harness = Harness::new(&[("example.a", COMMAND)]);
        harness.approve("example.a");
        harness.host.command(HostCommand::Retry(PluginId::parse("example.a").unwrap()));
        harness.pump_until(|harness| {
            harness.seen.iter().any(|event| matches!(event, HostEvent::Status { error: true, .. }))
        });
        let status = harness
            .seen
            .iter()
            .rev()
            .find_map(|event| match event {
                HostEvent::Status { message, error: true, .. } => Some(message.clone()),
                _ => None,
            })
            .expect("a status event");
        assert_eq!(status, "a: the package is not paused or failed");
    }

    #[test]
    fn an_authorized_read_grant_accepts_an_excerpt() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = dir.path().join("plugins");
        std::fs::create_dir(&plugins).unwrap();
        write_bundle(
            &plugins,
            "example.read",
            "Read",
            EXCERPT,
            Permissions { fs: Some(FsAccess::Read), spawn: Vec::new(), net: false },
        );
        let mut harness = Harness::from_dir(dir);
        harness.approve("example.read");
        let generation = harness.entry("example.read").generation;
        let mut input = Harness::input();
        input.excerpt = Some(ExcerptInput { handle: EntryHandle(1), text: "hello".into() });
        let ticket = harness.submit_evaluate("example.read", "copy", generation, input);
        let (_, _, result) = harness.wait_terminal(&ticket);
        assert_eq!(result, RequestResult::Intent(crow_core::CommandIntent::CopyText("hello".into())));
    }

    #[test]
    fn changed_capability_and_malformed_grants_withdraw_running_work() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = dir.path().join("plugins");
        std::fs::create_dir(&plugins).unwrap();
        write_bundle(
            &plugins,
            "example.read",
            "Read",
            COMMAND,
            Permissions { fs: Some(FsAccess::Read), spawn: Vec::new(), net: false },
        );
        let mut harness = Harness::from_dir(dir);
        harness.approve("example.read");
        let generation = harness.entry("example.read").generation;
        let ticket = harness.submit_evaluate("example.read", "copy", generation, Harness::input());
        let _ready = harness.take_reply(ticket.stamp().request_id);

        let grants = harness.grants().replace("fs = \"read\"", "fs = \"none\"");
        harness.write_grants(&grants);
        harness.host.refresh();
        harness.pump_until(|harness| harness.entry("example.read").state == PluginState::PendingApproval);
        assert!(harness.entry("example.read").detail.as_deref().unwrap().contains("capabilities"));
        assert_eq!(harness.entry("example.read").descriptors.len(), 0);
        let (_, _, result) = harness.wait_terminal(&ticket);
        assert_eq!(result, RequestResult::Error(RequestError::cancelled()));

        harness.write_grants("[plugin.\"example.read\"]\nenabled = true\ndigest = \"short\"\nfs = \"none\"\n");
        harness.host.refresh();
        harness.pump_until(|harness| harness.inventory().grants_error.is_some());
        assert_eq!(harness.entry("example.read").state, PluginState::Disabled);
        assert_eq!(harness.entry("example.read").descriptors.len(), 0);

        harness.write_grants(&grants.replace("fs = \"none\"", "fs = \"read\""));
        harness.host.refresh();
        harness.pump_until(|harness| {
            harness.inventory().grants_error.is_none() && harness.entry("example.read").state == PluginState::Running
        });
        assert_eq!(harness.entry("example.read").descriptors.len(), 1);
    }

    #[test]
    fn authorized_retry_gets_a_new_generation() {
        let mut harness = Harness::new(&[("example.a", LOOP)]);
        harness.host.limits.call_deadline = Duration::from_secs(5);
        harness.approve("example.a");
        let generation = harness.entry("example.a").generation;
        harness.submit_evaluate("example.a", "copy", generation, Harness::input());
        harness.pump_until(|harness| harness.entry("example.a").state == PluginState::Paused);
        assert_eq!(harness.entry("example.a").generation, 0);
        harness.host.retry(&PluginId::parse("example.a").unwrap()).unwrap();
        harness.pump_until(|harness| harness.entry("example.a").state == PluginState::Running);
        assert!(harness.entry("example.a").generation > generation);
    }

    #[test]
    fn a_cancelled_ready_reply_finishes_as_cancelled_and_never_becomes_an_intent() {
        let mut harness = Harness::new(&[("example.a", COMMAND)]);
        harness.approve("example.a");
        let generation = harness.entry("example.a").generation;
        let ticket = harness.submit_evaluate("example.a", "copy", generation, Harness::input());
        let ready = harness.take_reply(ticket.stamp().request_id);
        ticket.cancel();
        harness.feed(ready);
        let (got_extension, stamp, result) = harness.wait_terminal(&ticket);
        assert_eq!(got_extension, extension("example.a", "copy"));
        assert_eq!(stamp, ticket.stamp());
        assert_eq!(result, RequestResult::Error(RequestError::cancelled()));
        assert_eq!(harness.entry("example.a").state, PluginState::Running);
        assert_eq!(harness.count_terminal(ticket.stamp().request_id), 1);
        assert!(
            !harness
                .seen
                .iter()
                .any(|event| matches!(event, HostEvent::RequestFinished { result: RequestResult::Intent(_), .. }))
        );
    }

    #[test]
    fn an_already_cancelled_queued_call_does_not_run_the_script() {
        let script = r#"
fn register() { [ #{ kind: "command", id: "copy", title: "Copy", function: "run" } ] }
fn run(ctx) { hold_worker("cancel-queued"); #{ kind: "copy_text", text: "ran" } }
"#;
        let mut harness = Harness::new(&[("example.a", script)]);
        harness.host.limits.call_deadline = Duration::from_secs(5);
        harness.approve("example.a");
        let generation = harness.entry("example.a").generation;
        let first = harness.submit_evaluate("example.a", "copy", generation, Harness::input());
        crate::worker::test_gate::wait_until_entered("cancel-queued");
        let second = harness.submit_evaluate("example.a", "copy", generation, Harness::input());
        second.cancel();
        crate::worker::test_gate::release("cancel-queued");
        let (_, _, first_result) = harness.wait_terminal(&first);
        assert!(matches!(first_result, RequestResult::Intent(_)));
        let (_, _, second_result) = harness.wait_terminal(&second);
        assert_eq!(second_result, RequestResult::Error(RequestError::cancelled()));
        assert_eq!(harness.entry("example.a").state, PluginState::Running);
        assert_eq!(harness.count_terminal(second.stamp().request_id), 1);
    }

    #[test]
    fn disable_before_consuming_a_ready_reply_still_finishes_it_once() {
        let mut harness = Harness::new(&[("example.a", COMMAND)]);
        harness.approve("example.a");
        let generation = harness.entry("example.a").generation;
        let ticket = harness.submit_evaluate("example.a", "copy", generation, Harness::input());
        let ready = harness.take_reply(ticket.stamp().request_id);
        harness.host.disable(&PluginId::parse("example.a").unwrap()).unwrap();
        harness.pump_until(|harness| harness.entry("example.a").state == PluginState::Disabled);
        let (_, _, result) = harness.wait_terminal(&ticket);
        assert_eq!(result, RequestResult::Error(RequestError::cancelled()));
        harness.feed(ready);
        assert_eq!(harness.count_terminal(ticket.stamp().request_id), 1);
        assert!(
            !harness
                .seen
                .iter()
                .any(|event| matches!(event, HostEvent::RequestFinished { result: RequestResult::Intent(_), .. }))
        );
    }

    #[test]
    fn a_duplicate_reply_is_ignored() {
        let mut harness = Harness::new(&[("example.a", COMMAND)]);
        harness.approve("example.a");
        let generation = harness.entry("example.a").generation;
        let ticket = harness.submit_evaluate("example.a", "copy", generation, Harness::input());
        let ready = harness.take_reply(ticket.stamp().request_id);
        harness.feed(ready.clone());
        harness.feed(ready);
        let (_, _, result) = harness.wait_terminal(&ticket);
        assert!(matches!(result, RequestResult::Intent(_)));
        assert_eq!(harness.count_terminal(ticket.stamp().request_id), 1);
    }

    #[test]
    fn a_hook_reply_keeps_its_stamp_and_extension() {
        let mut harness = Harness::new(&[("example.a", COMMAND_AND_HOOK)]);
        harness.approve("example.a");
        let generation = harness.entry("example.a").generation;
        let ticket =
            harness.submit_notify("example.a", "changed", generation, HookEvent::FilesChanged { scope: "/tmp".into() });
        let (got_extension, stamp, result) = harness.wait_terminal(&ticket);
        assert_eq!(got_extension, extension("example.a", "changed"));
        assert_eq!(stamp, ticket.stamp());
        assert_eq!(result, RequestResult::Refresh);
    }

    struct ServiceHarness {
        _service: PluginService,
        handle: PluginHandle,
        events: Receiver<HostEvent>,
        inventory: Inventory,
        _dir: tempfile::TempDir,
    }

    impl ServiceHarness {
        fn new(scripts: &[(&str, &str)], limits: Limits) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let plugins = dir.path().join("plugins");
            std::fs::create_dir(&plugins).unwrap();
            for (id, script) in scripts {
                let name = id.rsplit('.').next().unwrap().to_owned();
                write_bundle(&plugins, id, &name, script, Permissions::default());
            }
            Self::from_dir(dir, scripts.len(), limits)
        }

        fn from_dir(dir: tempfile::TempDir, expected: usize, limits: Limits) -> Self {
            let (service, events) = PluginService::start(dir.path().to_path_buf(), limits).unwrap();
            let handle = service.handle();
            let mut harness = Self { _service: service, handle, events, inventory: Inventory::default(), _dir: dir };
            harness.wait_inventory(|inventory| inventory.plugins.len() == expected);
            harness
        }

        fn wait_inventory(&mut self, until: impl Fn(&Inventory) -> bool) {
            let started = Instant::now();
            loop {
                while let Ok(event) = self.events.try_recv() {
                    if let HostEvent::Inventory(inventory) = event {
                        self.inventory = inventory;
                    }
                }
                if until(&self.inventory) {
                    return;
                }
                assert!(started.elapsed() < Duration::from_secs(5), "inventory timed out: {:?}", self.inventory);
                std::thread::sleep(Duration::from_millis(2));
            }
        }

        fn entry(&self, id: &str) -> &PluginEntry {
            self.inventory.plugins.iter().find(|entry| entry.id.as_ref().is_some_and(|p| p.as_str() == id)).unwrap()
        }

        fn approve(&mut self, id: &str) {
            let digest = self.entry(id).digest.clone().unwrap();
            self.handle.approve(PluginId::parse(id).unwrap(), digest).unwrap();
            let plugin = PluginId::parse(id).unwrap();
            self.wait_inventory(move |inventory| {
                inventory
                    .plugins
                    .iter()
                    .any(|entry| entry.id.as_ref() == Some(&plugin) && entry.state == PluginState::Running)
            });
        }

        fn generation(&self, id: &str) -> u64 {
            self.entry(id).generation
        }

        fn wait_terminal(&mut self, ticket: &RequestTicket) -> RequestResult {
            let stamp = ticket.stamp();
            let started = Instant::now();
            loop {
                while let Ok(event) = self.events.try_recv() {
                    match event {
                        HostEvent::Inventory(inventory) => self.inventory = inventory,
                        HostEvent::RequestFinished { stamp: finished, result, .. } if finished == stamp => {
                            return result;
                        }
                        _ => {}
                    }
                }
                assert!(started.elapsed() < Duration::from_secs(8), "request {} timed out", stamp.request_id);
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }

    #[test]
    fn the_handle_path_evaluates_and_notifies_with_matching_stamps() {
        let mut service = ServiceHarness::new(&[("example.a", COMMAND_AND_HOOK)], test_limits());
        service.approve("example.a");
        let generation = service.generation("example.a");
        let copy = extension("example.a", "copy");
        let ticket = service.handle.try_evaluate(copy.clone(), generation, 11, Harness::input()).unwrap();
        let result = service.wait_terminal(&ticket);
        assert!(
            matches!(result, RequestResult::Intent(crow_core::CommandIntent::CopyText(text)) if text == "/tmp/a.txt")
        );
        assert_eq!(ticket.stamp().generation, generation);
        assert_eq!(ticket.stamp().context_revision, 11);

        let hook = extension("example.a", "changed");
        let ticket = service
            .handle
            .try_notify(hook.clone(), generation, 12, HookEvent::FilesChanged { scope: "/tmp".into() })
            .unwrap();
        let result = service.wait_terminal(&ticket);
        assert_eq!(result, RequestResult::Refresh);
        assert_eq!(ticket.stamp().context_revision, 12);

        let other_handle = service.handle.clone();
        let first = service.handle.try_evaluate(copy.clone(), generation, 13, Harness::input()).unwrap();
        let second = other_handle.try_evaluate(copy, generation, 13, Harness::input()).unwrap();
        assert_ne!(first.stamp().request_id, second.stamp().request_id, "one counter for all clones");
    }

    fn assert_rejection(result: RequestResult, kind: RequestErrorKind, contains: &str) {
        let RequestResult::Error(error) = result else {
            panic!("expected {kind:?}, got {result:?}");
        };
        assert_eq!(error.kind, kind, "{}", error.message);
        assert!(error.message.contains(contains), "{}", error.message);
    }

    #[test]
    fn handles_reject_stale_generations_bad_extensions_bad_input_and_missing_capability() {
        let mut service = ServiceHarness::new(&[("example.a", COMMAND_AND_HOOK)], test_limits());
        service.approve("example.a");
        let generation = service.generation("example.a");
        let copy = extension("example.a", "copy");

        let ticket = service.handle.try_evaluate(copy.clone(), generation + 1, 1, Harness::input()).unwrap();
        assert_rejection(service.wait_terminal(&ticket), RequestErrorKind::StaleGeneration, "not current");

        let ticket = service.handle.try_evaluate(copy.clone(), generation, 1, Harness::input()).unwrap();
        assert!(matches!(service.wait_terminal(&ticket), RequestResult::Intent(_)));

        let missing = extension("example.a", "missing");
        let ticket = service.handle.try_evaluate(missing, generation, 1, Harness::input()).unwrap();
        assert_rejection(service.wait_terminal(&ticket), RequestErrorKind::Unavailable, "is not registered");

        let hook = extension("example.a", "changed");
        let ticket = service.handle.try_evaluate(hook, generation, 1, Harness::input()).unwrap();
        assert_rejection(service.wait_terminal(&ticket), RequestErrorKind::InvalidInput, "use Notify");

        let mut duplicate = Harness::input();
        duplicate.snapshot.entries.push(duplicate.snapshot.entries[0].clone());
        let ticket = service.handle.try_evaluate(copy.clone(), generation, 1, duplicate).unwrap();
        assert_rejection(service.wait_terminal(&ticket), RequestErrorKind::InvalidInput, "duplicate entry handle");

        let mut excerpt = Harness::input();
        excerpt.excerpt = Some(ExcerptInput { handle: EntryHandle(1), text: "hello".into() });
        let ticket = service.handle.try_evaluate(copy, generation, 1, excerpt).unwrap();
        assert_rejection(service.wait_terminal(&ticket), RequestErrorKind::Capability, "no fs read capability");
    }

    #[test]
    fn a_held_worker_reports_busy_while_another_plugin_still_replies() {
        let script = r#"
fn register() { [ #{ kind: "command", id: "copy", title: "Copy", function: "run" } ] }
fn run(ctx) { hold_worker("saturation-a"); #{ kind: "copy_text", text: "a" } }
"#;
        let mut limits = test_limits();
        limits.command_mailbox = 1;
        limits.call_deadline = Duration::from_secs(5);
        let mut service = ServiceHarness::new(&[("example.a", script), ("example.b", COMMAND)], limits);
        service.approve("example.a");
        service.approve("example.b");
        let a = extension("example.a", "copy");
        let b = extension("example.b", "copy");
        let a_generation = service.generation("example.a");
        let b_generation = service.generation("example.b");

        let first = service.handle.try_evaluate(a.clone(), a_generation, 1, Harness::input()).unwrap();
        crate::worker::test_gate::wait_until_entered("saturation-a");
        let second = service.handle.try_evaluate(a.clone(), a_generation, 1, Harness::input()).unwrap();
        let busy = service.handle.try_evaluate(a, a_generation, 1, Harness::input()).unwrap();
        assert_eq!(
            service.wait_terminal(&busy),
            RequestResult::Error(RequestError::new(RequestErrorKind::Busy, "the plugin worker lane is full",))
        );

        let other = service.handle.try_evaluate(b, b_generation, 1, Harness::input()).unwrap();
        assert!(matches!(service.wait_terminal(&other), RequestResult::Intent(_)), "B replies while A is held");

        crate::worker::test_gate::release("saturation-a");
        assert!(matches!(service.wait_terminal(&first), RequestResult::Intent(_)));
        assert!(matches!(service.wait_terminal(&second), RequestResult::Intent(_)));
    }

    #[test]
    fn ticket_cancel_during_evaluation_is_cancelled_and_keeps_the_plugin_running() {
        let mut limits = test_limits();
        limits.call_deadline = Duration::from_secs(5);
        let mut service = ServiceHarness::new(&[("example.a", LOOP)], limits);
        service.approve("example.a");
        let generation = service.generation("example.a");
        let ticket =
            service.handle.try_evaluate(extension("example.a", "copy"), generation, 1, Harness::input()).unwrap();
        std::thread::sleep(Duration::from_millis(5));
        ticket.cancel();
        ticket.cancel();
        assert_eq!(service.wait_terminal(&ticket), RequestResult::Error(RequestError::cancelled()));
        service.wait_inventory(|inventory| {
            inventory.plugins.iter().any(|entry| {
                entry.id.as_ref().is_some_and(|id| id.as_str() == "example.a") && entry.state == PluginState::Running
            })
        });
    }

    #[test]
    fn a_coalesced_hook_finishes_the_displaced_ticket_as_cancelled() {
        let script = r#"
fn register() {
    [
        #{ kind: "command", id: "copy", title: "Copy", function: "run" },
        #{ kind: "hook", id: "changed", title: "Changed", function: "hook" }
    ]
}
fn run(ctx) { hold_worker("coalesce"); #{ kind: "copy_text", text: "a" } }
fn hook(ctx) { #{ refresh: true } }
"#;
        let mut limits = test_limits();
        limits.call_deadline = Duration::from_secs(5);
        let mut service = ServiceHarness::new(&[("example.a", script)], limits);
        service.approve("example.a");
        let generation = service.generation("example.a");
        let hold =
            service.handle.try_evaluate(extension("example.a", "copy"), generation, 1, Harness::input()).unwrap();
        crate::worker::test_gate::wait_until_entered("coalesce");
        let hook = extension("example.a", "changed");
        let first = service
            .handle
            .try_notify(hook.clone(), generation, 9, HookEvent::FilesChanged { scope: "/a".into() })
            .unwrap();
        let second =
            service.handle.try_notify(hook, generation, 9, HookEvent::FilesChanged { scope: "/a".into() }).unwrap();
        assert_eq!(service.wait_terminal(&first), RequestResult::Error(RequestError::cancelled()));
        crate::worker::test_gate::release("coalesce");
        assert!(matches!(service.wait_terminal(&hold), RequestResult::Intent(_)));
        assert_eq!(service.wait_terminal(&second), RequestResult::Refresh);
    }

    #[test]
    fn a_full_hook_lane_finishes_the_extra_hook_as_busy() {
        let script = r#"
fn register() {
    [
        #{ kind: "command", id: "copy", title: "Copy", function: "run" },
        #{ kind: "hook", id: "changed", title: "Changed", function: "hook" }
    ]
}
fn run(ctx) { hold_worker("hook-lane"); #{ kind: "copy_text", text: "a" } }
fn hook(ctx) { #{ refresh: true } }
"#;
        let mut limits = test_limits();
        limits.call_deadline = Duration::from_secs(5);
        limits.refresh_mailbox = 1;
        let mut service = ServiceHarness::new(&[("example.a", script)], limits);
        service.approve("example.a");
        let generation = service.generation("example.a");
        let hold =
            service.handle.try_evaluate(extension("example.a", "copy"), generation, 1, Harness::input()).unwrap();
        crate::worker::test_gate::wait_until_entered("hook-lane");
        let hook = extension("example.a", "changed");
        let first = service
            .handle
            .try_notify(hook.clone(), generation, 1, HookEvent::FilesChanged { scope: "/a".into() })
            .unwrap();
        let extra =
            service.handle.try_notify(hook, generation, 1, HookEvent::FilesChanged { scope: "/b".into() }).unwrap();
        assert_eq!(
            service.wait_terminal(&extra),
            RequestResult::Error(RequestError::new(RequestErrorKind::Busy, "the plugin hook lane is full"))
        );
        crate::worker::test_gate::release("hook-lane");
        assert!(matches!(service.wait_terminal(&hold), RequestResult::Intent(_)));
        assert_eq!(service.wait_terminal(&first), RequestResult::Refresh);
    }

    #[test]
    fn a_full_ingress_returns_submit_busy_without_a_ticket() {
        let (commands, _commands_rx) = mpsc::channel::<Inbox>();
        let (ingress, _ingress_rx) = mpsc::sync_channel::<Submission>(1);
        let handle = PluginHandle::new(commands, ingress, Arc::new(AtomicU64::new(0)));
        let copy = extension("example.a", "copy");
        assert!(handle.try_evaluate(copy.clone(), 1, 1, Harness::input()).is_ok());
        assert_eq!(handle.try_evaluate(copy, 1, 1, Harness::input()).unwrap_err(), SubmitError::Busy);
    }

    #[test]
    fn a_nonregular_bundle_does_not_block_the_service_or_a_second_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = dir.path().join("plugins");
        std::fs::create_dir(&plugins).unwrap();
        write_bundle(&plugins, "example.good", "Good", COMMAND, Permissions::default());
        let blocked = plugins.join("blocked");
        std::fs::create_dir(&blocked).unwrap();
        std::fs::write(
            blocked.join("plugin.toml"),
            r#"
id = "example.blocked"
name = "Blocked"
version = "0.1.0"
api = 1
entry = "main.rhai"
provides = ["command"]
permissions = { fs = "none", spawn = [], net = false }
"#,
        )
        .unwrap();
        let fifo = std::ffi::CString::new(blocked.join("main.rhai").as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0, "{}", std::io::Error::last_os_error());

        let mut service = ServiceHarness::from_dir(dir, 2, test_limits());
        let blocked = service.inventory.plugins.iter().find(|entry| entry.dir_name == "blocked").unwrap();
        assert_eq!(blocked.state, PluginState::InvalidRead);
        assert!(blocked.detail.as_deref().unwrap().contains("main.rhai"));
        service.approve("example.good");
        assert_eq!(service.entry("example.good").state, PluginState::Running);
    }

    #[test]
    fn a_fatal_worker_failure_still_stops_the_plugin_after_its_ticket_was_cancelled() {
        let mut harness = Harness::new(&[("example.a", COMMAND)]);
        harness.approve("example.a");
        let generation = harness.entry("example.a").generation;
        let ticket = harness.submit_evaluate("example.a", "copy", generation, Harness::input());
        let ready = harness.take_reply(ticket.stamp().request_id);
        ticket.cancel();
        harness.feed(ready);
        let (_, _, result) = harness.wait_terminal(&ticket);
        assert_eq!(result, RequestResult::Error(RequestError::cancelled()));
        assert_eq!(harness.entry("example.a").state, PluginState::Running);

        harness.feed(WorkerMessage::Failed {
            plugin: PluginId::parse("example.a").unwrap(),
            generation,
            request: Some(ticket.stamp()),
            error: CallError::script("boom"),
        });
        harness.pump_until(|harness| harness.entry("example.a").state == PluginState::Failed);
        assert_eq!(harness.entry("example.a").descriptors.len(), 0);
        assert_eq!(harness.count_terminal(ticket.stamp().request_id), 1);
    }

    #[test]
    fn an_undeclared_icon_pack_asset_rejects_the_whole_registration() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = dir.path().join("plugins");
        let bundle = plugins.join("theme");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(
            bundle.join("plugin.toml"),
            r#"
id = "example.theme"
name = "Theme"
version = "0.1.0"
api = 1
entry = "main.rhai"
assets = ["icons/pack.toml"]
provides = ["icon-pack", "command"]
permissions = { fs = "none", spawn = [], net = false }
"#,
        )
        .unwrap();
        std::fs::write(
            bundle.join("main.rhai"),
            r#"
fn register() {
    [
        #{ kind: "command", id: "copy", title: "Copy", function: "run" },
        #{ kind: "icon-pack", id: "theme", title: "Theme", asset: "icons/other.toml" }
    ]
}
fn run(ctx) { #{ kind: "copy_text", text: "x" } }
"#,
        )
        .unwrap();
        std::fs::create_dir(bundle.join("icons")).unwrap();
        std::fs::write(bundle.join("icons/pack.toml"), "api = 1").unwrap();

        let mut harness = Harness::from_dir(dir);
        let digest = harness.entry("example.theme").digest.clone().unwrap();
        harness.host.approve(&PluginId::parse("example.theme").unwrap(), &digest).unwrap();
        harness.pump_until(|harness| harness.entry("example.theme").state == PluginState::Failed);
        let entry = harness.entry("example.theme");
        assert!(entry.detail.as_deref().unwrap().contains("not declared"), "{:?}", entry.detail);
        assert!(entry.descriptors.is_empty());
        assert!(entry.icon_packs.is_empty());
    }

    #[test]
    fn an_approved_icon_pack_exposes_the_exact_snapshot_bytes_until_its_bytes_change() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = dir.path().join("plugins");
        let bundle = plugins.join("theme");
        std::fs::create_dir_all(bundle.join("icons")).unwrap();
        std::fs::write(
            bundle.join("plugin.toml"),
            r#"
id = "example.theme"
name = "Theme"
version = "0.1.0"
api = 1
entry = "main.rhai"
assets = ["icons/pack.toml", "icons/rust.svg"]
provides = ["icon-pack"]
permissions = { fs = "none", spawn = [], net = false }
"#,
        )
        .unwrap();
        std::fs::write(
            bundle.join("main.rhai"),
            r#"fn register() { [ #{ kind: "icon-pack", id: "theme", title: "Theme", asset: "icons/pack.toml" } ] }"#,
        )
        .unwrap();
        let pack = b"api = 1\n[extension]\nrs = { icon = \"rust.svg\" }\n";
        let glyph = b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>";
        std::fs::write(bundle.join("icons/pack.toml"), pack).unwrap();
        std::fs::write(bundle.join("icons/rust.svg"), glyph).unwrap();

        let mut service = ServiceHarness::from_dir(dir, 1, test_limits());
        service.approve("example.theme");
        let entry = service.entry("example.theme");
        assert_eq!(entry.icon_packs.len(), 1);
        let registered = &entry.icon_packs[0];
        assert_eq!(registered.id().to_string(), "example.theme.theme");
        assert_eq!(registered.generation(), entry.generation);
        assert_eq!(registered.asset(), "icons/pack.toml");
        assert_eq!(registered.pack_bytes().unwrap().as_ref(), pack);
        assert_eq!(registered.declared_asset("icons/rust.svg").unwrap().as_ref(), glyph);
        assert!(registered.declared_asset("icons/other.svg").is_none());

        std::fs::write(service._dir.path().join("plugins/theme/icons/pack.toml"), b"api = 2\n").unwrap();
        service.handle.refresh();
        service.wait_inventory(|inventory| {
            inventory.plugins.iter().any(|entry| {
                entry.id.as_ref().is_some_and(|id| id.as_str() == "example.theme")
                    && entry.state == PluginState::PendingApproval
                    && entry.icon_packs.is_empty()
                    && entry.generation == 0
            })
        });

        service.approve("example.theme");
        std::fs::write(service._dir.path().join("plugins/theme/icons/rust.svg"), b"<svg/>").unwrap();
        service.handle.refresh();
        service.wait_inventory(|inventory| {
            inventory.plugins.iter().any(|entry| {
                entry.id.as_ref().is_some_and(|id| id.as_str() == "example.theme")
                    && entry.state == PluginState::PendingApproval
                    && entry.descriptors.is_empty()
            })
        });
    }

    #[test]
    fn a_stopped_service_closes_the_event_stream_and_rejects_submissions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("plugins")).unwrap();
        let (service, events) = PluginService::start(dir.path().to_path_buf(), test_limits()).unwrap();
        let handle = service.handle();
        drop(service);
        let started = Instant::now();
        loop {
            match events.recv_timeout(Duration::from_millis(10)) {
                Ok(_) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    assert!(started.elapsed() < Duration::from_secs(5), "event stream stayed open");
                }
            }
        }
        assert_eq!(
            handle.try_evaluate(extension("example.a", "copy"), 1, 1, Harness::input()).unwrap_err(),
            SubmitError::Stopped
        );
    }
}
