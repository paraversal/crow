//! Package lifecycle: discovery, grants, one worker per enabled package,
//! generation checks and the inventory the app renders. All I/O here runs on
//! the service thread, never on the UI thread.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};

use crow_core::{Descriptor, ExtensionId, PluginId};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};

use crate::dto::{
    Call, CallError, CallErrorKind, HostEvent, Inventory, PluginEntry, PluginState, Reply, RequestInput, RequestStamp,
    WorkerMessage,
};
use crate::grants::{self, Grants};
use crate::limits::Limits;
use crate::manifest::FsAccess;
use crate::package::{self, PackageCandidate, PackageSnapshot};
use crate::worker::{EnqueueError, Worker};

pub(crate) enum Inbox {
    Command(HostCommand),
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

#[derive(Clone)]
struct RuntimeState {
    state: PluginState,
    detail: Option<String>,
    descriptors: Vec<Descriptor>,
    digest: Option<String>,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self { state: PluginState::PendingApproval, detail: None, descriptors: Vec::new(), digest: None }
    }
}

struct WorkerSlot {
    generation: u64,
    digest: String,
    worker: Worker,
}

pub struct PluginHost {
    plugins_dir: PathBuf,
    grants_path: PathBuf,
    limits: Limits,
    candidates: Vec<PackageCandidate>,
    grants: Grants,
    grants_error: Option<String>,
    scan_error: Option<String>,
    states: HashMap<PluginId, RuntimeState>,
    workers: HashMap<PluginId, WorkerSlot>,
    reaping: Vec<JoinHandle<()>>,
    generations: HashMap<PluginId, u64>,
    next_generation: u64,
    next_request: u64,
    events: UnboundedSender<HostEvent>,
    inbox: Sender<Inbox>,
    reported: HashMap<String, String>,
    reported_grants: Option<String>,
    reported_scan: Option<String>,
}

impl PluginHost {
    pub(crate) fn new(
        config_dir: &Path,
        limits: Limits,
        events: UnboundedSender<HostEvent>,
        inbox: Sender<Inbox>,
    ) -> Self {
        Self {
            plugins_dir: config_dir.join("plugins"),
            grants_path: grants::grants_path(config_dir),
            limits,
            candidates: Vec::new(),
            grants: Grants::default(),
            grants_error: None,
            scan_error: None,
            states: HashMap::new(),
            workers: HashMap::new(),
            reaping: Vec::new(),
            generations: HashMap::new(),
            next_generation: 0,
            next_request: 0,
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

    /// Re-read grants and bundles, invalidate changed workers, and start every
    /// currently enabled package whose exact snapshot is approved.
    pub fn refresh(&mut self) {
        self.reap();
        let grants_result = (|| {
            grants::ensure_grants_file(&self.grants_path)?;
            grants::load_grants(&self.grants_path)
        })();
        match grants_result {
            Ok(grants) => {
                self.grants = grants;
                self.grants_error = None;
            }
            Err(error) => {
                self.grants = Grants::default();
                self.grants_error = Some(error);
                self.stop_all_workers();
            }
        }

        let discovery = package::discover(&self.plugins_dir, &self.limits);
        self.scan_error = discovery.error;
        let candidates = discovery.candidates;

        let new_ids: HashSet<PluginId> = candidates
            .iter()
            .filter(|candidate| candidate.snapshot.is_ok())
            .filter_map(|candidate| candidate.id.clone())
            .collect();
        let stale: Vec<PluginId> = self
            .workers
            .iter()
            .filter(|(id, slot)| {
                candidates
                    .iter()
                    .find(|candidate| candidate.id.as_ref() == Some(*id))
                    .and_then(|candidate| candidate.snapshot.as_ref().ok())
                    .is_none_or(|snapshot| snapshot.digest != slot.digest)
                    || !new_ids.contains(*id)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in stale {
            self.stop_worker(&id);
        }

        self.candidates = candidates;
        for candidate in self.candidates.clone() {
            self.reconcile_candidate(&candidate);
        }
        for id in self.states.keys().cloned().collect::<Vec<_>>() {
            if !new_ids.contains(&id) {
                self.states.remove(&id);
            }
        }
        self.reap();
        self.publish();
    }

    fn reconcile_candidate(&mut self, candidate: &PackageCandidate) {
        let Some(id) = candidate.id.clone() else {
            return;
        };
        let snapshot = match &candidate.snapshot {
            Ok(snapshot) => snapshot,
            Err(error) => {
                if self.workers.contains_key(&id) {
                    self.stop_worker(&id);
                }
                self.set_state(&id, PluginState::from(&error.kind), Some(error.message.clone()), Vec::new(), None);
                return;
            }
        };
        let digest = snapshot.digest.clone();
        let previous = self.states.get(&id).cloned().filter(|state| state.digest.as_deref() == Some(&digest));

        if let Some(error) = self.grants_error.clone() {
            if self.workers.contains_key(&id) {
                self.stop_worker(&id);
            }
            self.set_state(&id, PluginState::Disabled, Some(error), Vec::new(), Some(digest));
            return;
        }

        let Some(grant) = self.grants.get(&id).cloned() else {
            if self.workers.contains_key(&id) {
                self.stop_worker(&id);
            }
            self.set_state(&id, PluginState::PendingApproval, None, Vec::new(), Some(digest));
            return;
        };
        if !grant.enabled {
            if self.workers.contains_key(&id) {
                self.stop_worker(&id);
            }
            self.set_state(&id, PluginState::Disabled, None, Vec::new(), Some(digest));
            return;
        }
        if !grant.authorizes(snapshot) {
            if self.workers.contains_key(&id) {
                self.stop_worker(&id);
            }
            let detail = if grant.digest != digest {
                Some("package changed since it was approved".to_owned())
            } else {
                Some("requested capabilities changed since approval".to_owned())
            };
            self.set_state(&id, PluginState::PendingApproval, detail, Vec::new(), Some(digest));
            return;
        }

        if let Some(previous) = &previous {
            if matches!(previous.state, PluginState::Paused | PluginState::Failed) {
                return;
            }
            if matches!(previous.state, PluginState::Running | PluginState::Starting)
                && self.workers.get(&id).is_some_and(|slot| slot.digest == digest)
            {
                return;
            }
        }
        if self.workers.get(&id).is_some_and(|slot| slot.digest == digest) {
            return;
        }
        self.start_worker(&id, snapshot.clone(), grant.fs);
    }

    fn start_worker(&mut self, id: &PluginId, snapshot: std::sync::Arc<PackageSnapshot>, fs: FsAccess) {
        let generation = self.bump_generation(id);
        let digest = snapshot.digest.clone();
        match Worker::spawn(
            id.clone(),
            generation,
            snapshot,
            fs.permits_read(),
            self.limits.clone(),
            self.inbox.clone(),
        ) {
            Ok(worker) => {
                let mailbox = worker.mailbox.clone();
                self.workers.insert(id.clone(), WorkerSlot { generation, digest: digest.clone(), worker });
                self.set_state(id, PluginState::Starting, None, Vec::new(), Some(digest));
                if mailbox.enqueue_command(Call::Register).is_err() {
                    self.worker_failed(id, generation, CallError::new(CallErrorKind::Script, "worker mailbox is full"));
                }
            }
            Err(error) => {
                self.set_state(id, PluginState::Failed, Some(error), Vec::new(), Some(digest));
            }
        }
    }

    pub fn approve(&mut self, id: &PluginId, expected_digest: &str) -> Result<(), String> {
        let candidate = self.candidate(id).cloned().ok_or_else(|| format!("no package \"{id}\""))?;
        let snapshot = candidate
            .snapshot
            .as_ref()
            .map_err(|error| format!("{}: {}", candidate_name(&candidate), error.message))?
            .clone();
        if snapshot.digest != expected_digest {
            return Err("the package changed since it was read; approve the new snapshot".into());
        }
        let current = package::load_package_at(&self.plugins_dir, &candidate.dir_name, &self.limits)
            .map_err(|error| format!("{}: {}", candidate_name(&candidate), error.message))?;
        if current.digest != expected_digest {
            self.refresh();
            return Err("the package changed on disk; review the new snapshot before approving".into());
        }
        grants::approve(&self.grants_path, id, &snapshot).map_err(|error| format!("plugins.toml: {error}"))?;
        self.grants = grants::load_grants(&self.grants_path).unwrap_or_default();
        self.grants_error = None;
        if self.states.get(id).is_some_and(|state| {
            state.state == PluginState::Running && state.digest.as_deref() == Some(expected_digest)
        }) && self.workers.get(id).is_some_and(|slot| slot.digest == expected_digest)
        {
            self.publish();
            return Ok(());
        }
        if self.workers.contains_key(id) {
            self.stop_worker(id);
        }
        self.start_worker(id, snapshot, self.grants.get(id).map_or(FsAccess::None, |grant| grant.fs));
        self.publish();
        Ok(())
    }

    pub fn disable(&mut self, id: &PluginId) -> Result<(), String> {
        if self.grants.get(id).is_some() {
            grants::set_enabled(&self.grants_path, id, false).map_err(|error| format!("plugins.toml: {error}"))?;
            self.grants = grants::load_grants(&self.grants_path).unwrap_or_default();
        }
        if self.workers.contains_key(id) {
            self.stop_worker(id);
        }
        let state = if self.grants.get(id).is_some() { PluginState::Disabled } else { PluginState::PendingApproval };
        let digest = self
            .candidates
            .iter()
            .find(|c| c.id.as_ref() == Some(id))
            .and_then(|c| c.snapshot.as_ref().ok().map(|snapshot| snapshot.digest.clone()));
        self.set_state(id, state, None, Vec::new(), digest);
        self.publish();
        Ok(())
    }

    pub fn enable(&mut self, id: &PluginId) -> Result<(), String> {
        let Some(grant) = self.grants.get(id) else {
            return Err("approve the package before enabling it".into());
        };
        if !grant.enabled {
            grants::set_enabled(&self.grants_path, id, true).map_err(|error| format!("plugins.toml: {error}"))?;
            self.grants = grants::load_grants(&self.grants_path).unwrap_or_default();
        }
        self.refresh();
        Ok(())
    }

    pub fn reload(&mut self, _id: &PluginId) {
        self.refresh();
    }

    pub fn retry(&mut self, id: &PluginId) -> Result<(), String> {
        let state = self.states.get(id).cloned().unwrap_or_default();
        if !matches!(state.state, PluginState::Paused | PluginState::Failed) {
            return Err("the package is not paused or failed".into());
        }
        let candidate = self.candidate(id).cloned().ok_or_else(|| format!("no package \"{id}\""))?;
        let snapshot = candidate
            .snapshot
            .as_ref()
            .map_err(|error| format!("{}: {}", candidate_name(&candidate), error.message))?
            .clone();
        let grant = self
            .grants
            .get(id)
            .filter(|grant| grant.authorizes(&snapshot))
            .ok_or_else(|| "the current snapshot is not approved".to_owned())?
            .clone();
        if self.workers.contains_key(id) {
            self.stop_worker(id);
        }
        self.start_worker(id, snapshot, grant.fs);
        self.publish();
        Ok(())
    }

    pub fn evaluate(
        &mut self,
        extension: ExtensionId,
        input: RequestInput,
        context_revision: u64,
    ) -> Result<RequestStamp, String> {
        input.validate(&self.limits)?;
        let Some(slot) = self.workers.get(&extension.plugin) else {
            return Err(format!("plugin \"{}\" is not running", extension.plugin));
        };
        if !self.states.get(&extension.plugin).is_some_and(|state| state.state == PluginState::Running) {
            return Err(format!("plugin \"{}\" is not running", extension.plugin));
        }
        self.next_request += 1;
        let stamp = RequestStamp { generation: slot.generation, request_id: self.next_request, context_revision };
        slot.worker
            .mailbox
            .enqueue_command(Call::Evaluate { extension, input, stamp })
            .map_err(|EnqueueError::Busy| "plugin is busy".to_owned())?;
        Ok(stamp)
    }

    pub fn notify(&mut self, extension: ExtensionId, event: crow_core::HookEvent, context_revision: u64) {
        let Some(slot) = self.workers.get(&extension.plugin) else {
            return;
        };
        if !self.states.get(&extension.plugin).is_some_and(|state| state.state == PluginState::Running) {
            return;
        }
        self.next_request += 1;
        let stamp = RequestStamp { generation: slot.generation, request_id: self.next_request, context_revision };
        slot.worker.mailbox.enqueue_refresh(Call::Notify { extension, event, stamp });
    }

    pub(crate) fn handle_inbox(&mut self, message: Inbox) {
        match message {
            Inbox::Command(command) => self.command(command),
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

    fn worker_message(&mut self, message: WorkerMessage) {
        match message {
            WorkerMessage::Registered { plugin, generation, result } => {
                if !self.is_current(&plugin, generation) {
                    return;
                }
                match result {
                    Ok(descriptors) => {
                        let Some(candidate) = self.candidate(&plugin) else {
                            self.worker_failed(&plugin, generation, CallError::invalid("package disappeared"));
                            return;
                        };
                        let Ok(snapshot) = &candidate.snapshot else {
                            self.worker_failed(&plugin, generation, CallError::invalid("package is invalid"));
                            return;
                        };
                        if let Err(error) = crate::registry::validate_published(
                            &plugin,
                            &snapshot.manifest.provides,
                            &descriptors,
                            &self.limits,
                        ) {
                            self.worker_failed(&plugin, generation, CallError::invalid(error));
                            return;
                        }
                        let digest = snapshot.digest.clone();
                        self.set_state(&plugin, PluginState::Running, None, descriptors, Some(digest));
                        self.publish();
                    }
                    Err(error) => self.worker_failed(&plugin, generation, error),
                }
            }
            WorkerMessage::Replied { plugin, generation, extension, stamp, reply } => {
                if !self.is_current(&plugin, generation) {
                    return;
                }
                if !self.states.get(&plugin).is_some_and(|state| state.state == PluginState::Running) {
                    return;
                }
                match reply {
                    Reply::Intent(intent) => {
                        self.send(HostEvent::CommandIntent { plugin, stamp, intent });
                    }
                    Reply::Refresh => {
                        self.send(HostEvent::Refresh { plugin, stamp });
                    }
                    Reply::Values(values) => {
                        self.send(HostEvent::Contribution { extension, stamp, values });
                    }
                    Reply::Unit | Reply::Registered(_) => {}
                }
            }
            WorkerMessage::Failed { plugin, generation, error } => {
                if !self.is_current(&plugin, generation) {
                    return;
                }
                self.worker_failed(&plugin, generation, error);
            }
        }
        self.reap();
    }

    fn worker_failed(&mut self, id: &PluginId, _generation: u64, error: CallError) {
        if error.kind == CallErrorKind::Cancelled {
            self.stop_worker(id);
            return;
        }
        let digest = self.states.get(id).and_then(|state| state.digest.clone());
        self.stop_worker(id);
        let state = if error.kind == CallErrorKind::Budget { PluginState::Paused } else { PluginState::Failed };
        self.set_state(id, state, Some(error.message), Vec::new(), digest);
        self.publish();
    }

    fn is_current(&self, id: &PluginId, generation: u64) -> bool {
        self.workers.get(id).is_some_and(|slot| slot.generation == generation)
            && self.generations.get(id).copied() == Some(generation)
    }

    fn stop_worker(&mut self, id: &PluginId) {
        if let Some(mut slot) = self.workers.remove(id) {
            slot.worker.stop();
            if let Some(join) = slot.worker.take_join() {
                self.reaping.push(join);
            }
        }
        if let Some(state) = self.states.get_mut(id) {
            state.descriptors.clear();
        }
        self.bump_generation(id);
    }

    fn stop_all_workers(&mut self) {
        for id in self.workers.keys().cloned().collect::<Vec<_>>() {
            self.stop_worker(&id);
        }
        for state in self.states.values_mut() {
            state.descriptors.clear();
        }
    }

    fn bump_generation(&mut self, id: &PluginId) -> u64 {
        self.next_generation += 1;
        self.generations.insert(id.clone(), self.next_generation);
        self.next_generation
    }

    fn generation(&self, id: &PluginId) -> u64 {
        self.generations.get(id).copied().unwrap_or(0)
    }

    fn set_state(
        &mut self,
        id: &PluginId,
        state: PluginState,
        detail: Option<String>,
        descriptors: Vec<Descriptor>,
        digest: Option<String>,
    ) {
        self.states.insert(id.clone(), RuntimeState { state, detail, descriptors, digest });
    }

    fn candidate(&self, id: &PluginId) -> Option<&PackageCandidate> {
        self.candidates.iter().find(|candidate| candidate.id.as_ref() == Some(id))
    }

    fn name_for(&self, id: &PluginId) -> String {
        self.candidate(id).map_or_else(|| id.to_string(), candidate_name)
    }

    fn status(&self, id: &PluginId, message: impl Into<String>, error: bool) {
        self.send(HostEvent::Status {
            plugin: Some(id.clone()),
            name: self.name_for(id),
            message: message.into(),
            error,
        });
    }

    fn send(&self, event: HostEvent) {
        let _ = self.events.unbounded_send(event);
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
            let (state, detail, descriptors) = match (&candidate.snapshot, &candidate.id) {
                (Err(error), _) => (PluginState::from(&error.kind), Some(error.message.clone()), Vec::new()),
                (Ok(snapshot), Some(id)) => {
                    let runtime = self.states.get(id).cloned().unwrap_or_else(|| {
                        if let Some(error) = &self.grants_error {
                            RuntimeState {
                                state: PluginState::Disabled,
                                detail: Some(error.clone()),
                                ..RuntimeState::default()
                            }
                        } else if self.grants.get(id).is_some_and(|grant| grant.enabled) {
                            RuntimeState {
                                state: PluginState::PendingApproval,
                                digest: Some(snapshot.digest.clone()),
                                ..RuntimeState::default()
                            }
                        } else if self.grants.get(id).is_some() {
                            RuntimeState { state: PluginState::Disabled, ..RuntimeState::default() }
                        } else {
                            RuntimeState {
                                state: PluginState::PendingApproval,
                                digest: Some(snapshot.digest.clone()),
                                ..RuntimeState::default()
                            }
                        }
                    });
                    (runtime.state, runtime.detail, runtime.descriptors)
                }
                (Ok(_), None) => (PluginState::PendingApproval, None, Vec::new()),
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
                generation: candidate.id.as_ref().map_or(0, |id| self.generation(id)),
            });
        }
        Inventory { plugins, grants_error: self.grants_error.clone(), scan_error: self.scan_error.clone() }
    }

    fn publish(&mut self) {
        let inventory = self.build_inventory();
        let mut statuses = Vec::new();
        for entry in &inventory.plugins {
            if entry.state.is_error() {
                if let Some(detail) = &entry.detail {
                    let key = entry.dir_name.clone();
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
                self.reported.remove(&entry.dir_name);
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
            self.send(HostEvent::Status { plugin, name, message, error: true });
        }
        self.send(HostEvent::Inventory(inventory));
    }

    pub fn shutdown(&mut self) {
        self.stop_all_workers();
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

/// The app-owned service. Commands are enqueued without waiting; events arrive
/// on the futures channel the UI already knows how to await.
pub struct PluginService {
    commands: Sender<Inbox>,
    _join: JoinHandle<()>,
}

impl PluginService {
    pub fn start(config_dir: PathBuf, limits: Limits) -> Result<(Self, UnboundedReceiver<HostEvent>), String> {
        let (command_tx, command_rx) = mpsc::channel::<Inbox>();
        let (event_tx, event_rx) = unbounded::<HostEvent>();
        let inbox = command_tx.clone();
        let join = thread::Builder::new()
            .name("crow-plugins".into())
            .spawn(move || {
                let mut host = PluginHost::new(&config_dir, limits, event_tx, inbox);
                host.refresh();
                while let Ok(message) = command_rx.recv() {
                    match message {
                        Inbox::Command(HostCommand::Shutdown) => break,
                        other => host.handle_inbox(other),
                    }
                }
                host.shutdown();
            })
            .map_err(|error| format!("could not start the plugin service: {error}"))?;
        Ok((Self { commands: command_tx, _join: join }, event_rx))
    }

    pub fn handle(&self) -> PluginHandle {
        PluginHandle { commands: self.commands.clone() }
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
}

impl PluginHandle {
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

    pub fn shutdown(&self) {
        let _ = self.commands.send(Inbox::Command(HostCommand::Shutdown));
    }

    fn send(&self, command: HostCommand) -> Result<(), String> {
        self.commands.send(Inbox::Command(command)).map_err(|_| "the plugin service stopped".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Permissions;
    use crow_core::{EntryData, EntryHandle, ExtensionKind, HookEvent, Snapshot};
    use std::sync::mpsc::Receiver;
    use std::time::{Duration, Instant};

    struct Harness {
        host: PluginHost,
        inbox: Receiver<Inbox>,
        events: UnboundedReceiver<HostEvent>,
        last: Inventory,
        seen: Vec<HostEvent>,
        _dir: tempfile::TempDir,
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
            let (event_tx, events) = unbounded::<HostEvent>();
            let mut host = PluginHost::new(dir.path(), test_limits(), event_tx, inbox_tx);
            host.refresh();
            let mut harness = Self { host, inbox, events, last: Inventory::default(), seen: Vec::new(), _dir: dir };
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
                while let Ok(event) = self.events.try_recv() {
                    if let HostEvent::Inventory(inventory) = &event {
                        self.last = inventory.clone();
                    }
                    self.seen.push(event);
                }
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
                reads: std::collections::BTreeMap::new(),
            }
        }

        fn evaluate(&mut self, id: &str, local: &str) -> RequestStamp {
            let plugin = PluginId::parse(id).unwrap();
            let extension = ExtensionId::new(plugin.clone(), crow_core::LocalId::parse(local).unwrap());
            let stamp = self.host.evaluate(extension, Self::input(), 7).unwrap();
            self.pump_until(|harness| {
                harness.seen.iter().any(|event| matches!(
                event,
                HostEvent::CommandIntent { plugin: p, stamp: s, .. } if p == &plugin && s.request_id == stamp.request_id
            ))
            });
            stamp
        }
    }

    fn test_limits() -> Limits {
        Limits { call_deadline: Duration::from_millis(250), ..Limits::default() }
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
fn run(ctx) { #{ action: "copy-text", text: ctx.scope + "/" + ctx.entries[0].name } }
"#;

    const LOOP: &str = r#"
fn register() { [ #{ kind: "command", id: "copy", title: "Copy", function: "run" } ] }
fn run(ctx) { while true { } }
"#;

    #[test]
    fn approve_starts_one_worker_and_publishes_its_registry() {
        let mut harness = Harness::new(&[("example.a", COMMAND)]);
        assert_eq!(harness.entry("example.a").state, PluginState::PendingApproval);
        assert_eq!(harness.entry("example.a").descriptors.len(), 0);
        harness.approve("example.a");
        let entry = harness.entry("example.a");
        assert_eq!(entry.state, PluginState::Running);
        assert_eq!(entry.descriptors.len(), 1);
        assert_eq!(entry.descriptors[0].id.to_string(), "example.a.copy");
        assert_eq!(entry.descriptors[0].kind, ExtensionKind::Command);

        harness.evaluate("example.a", "copy");
        assert!(harness.seen.iter().any(|event| matches!(
            event,
            HostEvent::CommandIntent { intent: crow_core::CommandIntent::CopyText(text), .. } if text == "/tmp/a.txt"
        )));
    }

    #[test]
    fn changing_bytes_withdraws_the_old_generation_before_review() {
        let mut harness = Harness::new(&[("example.a", COMMAND)]);
        harness.approve("example.a");
        let old_generation = harness.entry("example.a").generation;
        let bundle = harness._dir.path().join("plugins/example-a/main.rhai");
        std::fs::write(&bundle, LOOP).unwrap();
        harness.host.refresh();
        harness.pump_until(|harness| harness.entry("example.a").state == PluginState::PendingApproval);
        let entry = harness.entry("example.a");
        assert_eq!(entry.descriptors.len(), 0, "contributions are withdrawn");
        assert!(entry.generation > old_generation, "old replies cannot match the new generation");
        assert!(entry.digest.is_some(), "the new snapshot is displayed for approval");
        assert!(entry.detail.as_deref().unwrap().contains("changed"));
    }

    #[test]
    fn malformed_grants_withdraw_and_a_fixed_file_reactivates() {
        let mut harness = Harness::new(&[("example.a", COMMAND)]);
        harness.approve("example.a");
        let grants_path = harness._dir.path().join("plugins.toml");
        let good = std::fs::read_to_string(&grants_path).unwrap();
        std::fs::write(&grants_path, "[plugin.\"example.a\"]\nenabled = true\ndigest = \"short\"\nfs = \"none\"\n")
            .unwrap();
        harness.host.refresh();
        harness.pump_until(|harness| harness.inventory().grants_error.is_some());
        let entry = harness.entry("example.a");
        assert_eq!(entry.state, PluginState::Disabled);
        assert_eq!(entry.descriptors.len(), 0);

        std::fs::write(&grants_path, good).unwrap();
        harness.host.refresh();
        harness.pump_until(|harness| {
            harness.inventory().grants_error.is_none() && harness.entry("example.a").state == PluginState::Running
        });
        assert_eq!(harness.entry("example.a").descriptors.len(), 1);
    }

    #[test]
    fn read_requested_but_not_granted_is_pending_not_running() {
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
        let (inbox_tx, inbox) = mpsc::channel::<Inbox>();
        let (event_tx, events) = unbounded::<HostEvent>();
        let mut host = PluginHost::new(dir.path(), test_limits(), event_tx, inbox_tx);
        host.refresh();
        let grants_path = dir.path().join("plugins.toml");
        let mut harness = Harness { host, inbox, events, last: Inventory::default(), seen: Vec::new(), _dir: dir };
        harness.pump_until(|harness| harness.entry("example.read").digest.is_some());
        let digest = harness.entry("example.read").digest.clone().unwrap();
        std::fs::write(
            grants_path,
            format!("[plugin.\"example.read\"]\nenabled = true\ndigest = \"{digest}\"\nfs = \"none\"\n"),
        )
        .unwrap();
        harness.host.refresh();
        harness.pump_until(|harness| harness.entry("example.read").state == PluginState::PendingApproval);
        assert!(harness.entry("example.read").detail.as_deref().unwrap().contains("capabilities"));
        assert_eq!(harness.entry("example.read").descriptors.len(), 0);
    }

    #[test]
    fn unsupported_capability_is_an_inventory_state_and_starts_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = dir.path().join("plugins");
        std::fs::create_dir(&plugins).unwrap();
        let bundle = plugins.join("spawn");
        std::fs::create_dir(&bundle).unwrap();
        std::fs::write(
            bundle.join("plugin.toml"),
            r#"
id = "example.spawn"
name = "Spawn"
version = "0.1.0"
api = 1
entry = "main.rhai"
provides = ["command"]
permissions = { spawn = ["git"], net = false }
"#,
        )
        .unwrap();
        std::fs::write(bundle.join("main.rhai"), COMMAND).unwrap();
        let (inbox_tx, inbox) = mpsc::channel::<Inbox>();
        let (event_tx, events) = unbounded::<HostEvent>();
        let mut host = PluginHost::new(dir.path(), test_limits(), event_tx, inbox_tx);
        host.refresh();
        let mut harness = Harness { host, inbox, events, last: Inventory::default(), seen: Vec::new(), _dir: dir };
        harness.pump_until(|harness| harness.inventory().plugins.len() == 1);
        assert_eq!(harness.inventory().plugins[0].state, PluginState::UnsupportedCapability);
        assert_eq!(harness.inventory().plugins[0].descriptors.len(), 0);
    }

    #[test]
    fn oversized_input_is_rejected_before_enqueue_and_reports_busy_when_full() {
        let mut harness = Harness::new(&[("example.a", LOOP)]);
        harness.approve("example.a");
        let plugin = PluginId::parse("example.a").unwrap();
        let extension = ExtensionId::new(plugin, crow_core::LocalId::parse("copy").unwrap());
        let mut input = Harness::input();
        input.snapshot.entries = (0..=harness.host.limits.max_entries_per_call)
            .map(|index| EntryData {
                handle: EntryHandle(index as u32),
                name: format!("{index}.txt"),
                is_dir: false,
                size: 0,
                kind: "Text".into(),
            })
            .collect();
        let error = harness.host.evaluate(extension.clone(), input, 1).unwrap_err();
        assert!(error.contains("more than"), "{error}");

        let mut limits = test_limits();
        limits.command_mailbox = 1;
        let mailbox = crate::worker::Mailbox::new(&limits);
        let call = Call::Evaluate {
            extension,
            input: Harness::input(),
            stamp: RequestStamp { generation: 1, request_id: 1, context_revision: 1 },
        };
        mailbox.enqueue_command(call.clone()).unwrap();
        assert_eq!(mailbox.enqueue_command(call), Err(EnqueueError::Busy));
    }

    #[test]
    fn budget_pauses_a_while_b_still_replies() {
        let mut harness = Harness::new(&[("example.a", LOOP), ("example.b", COMMAND)]);
        harness.approve("example.a");
        harness.approve("example.b");
        let a = PluginId::parse("example.a").unwrap();
        let b = PluginId::parse("example.b").unwrap();
        let a_ext = ExtensionId::new(a.clone(), crow_core::LocalId::parse("copy").unwrap());
        let b_ext = ExtensionId::new(b.clone(), crow_core::LocalId::parse("copy").unwrap());
        let a_stamp = harness.host.evaluate(a_ext, Harness::input(), 1).unwrap();
        let b_stamp = harness.host.evaluate(b_ext, Harness::input(), 1).unwrap();
        harness.pump_until(|harness| {
            harness.entry("example.a").state == PluginState::Paused
                && harness.entry("example.b").state == PluginState::Running
                && harness.seen.iter().any(|event| matches!(
                    event,
                    HostEvent::CommandIntent { plugin, stamp, .. } if plugin == &b && stamp.request_id == b_stamp.request_id
                ))
        });
        assert_eq!(harness.entry("example.a").descriptors.len(), 0);
        assert!(harness.seen.iter().any(|event| matches!(
            event,
            HostEvent::CommandIntent { plugin, stamp, .. } if plugin == &b && stamp.request_id == b_stamp.request_id
        )));
        let _ = a_stamp;
    }

    #[test]
    fn disable_drops_a_queued_reply_and_retry_uses_a_new_generation() {
        let mut harness = Harness::new(&[("example.a", LOOP)]);
        harness.approve("example.a");
        let a = PluginId::parse("example.a").unwrap();
        let extension = ExtensionId::new(a.clone(), crow_core::LocalId::parse("copy").unwrap());
        let stamp = harness.host.evaluate(extension, Harness::input(), 3).unwrap();
        harness.host.disable(&a).unwrap();
        harness.pump_until(|harness| harness.entry("example.a").state == PluginState::Disabled);
        std::thread::sleep(Duration::from_millis(60));
        harness.pump_until(|_| true);
        assert!(!harness.seen.iter().any(|event| matches!(
            event,
            HostEvent::CommandIntent { plugin, stamp: event_stamp, .. }
                if plugin == &a && event_stamp.request_id == stamp.request_id
        )));

        harness.host.retry(&a).unwrap_err();
        harness.host.enable(&a).unwrap();
        harness.pump_until(|harness| harness.entry("example.a").state == PluginState::Running);
        assert!(harness.entry("example.a").generation > 0);
    }

    #[test]
    fn a_swapped_asset_between_discovery_and_approval_cannot_execute() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = dir.path().join("plugins");
        std::fs::create_dir(&plugins).unwrap();
        write_bundle(&plugins, "example.a", "A", COMMAND, Permissions::default());
        let (inbox_tx, inbox) = mpsc::channel::<Inbox>();
        let (event_tx, events) = unbounded::<HostEvent>();
        let mut host = PluginHost::new(dir.path(), test_limits(), event_tx, inbox_tx);
        host.refresh();
        let mut harness = Harness { host, inbox, events, last: Inventory::default(), seen: Vec::new(), _dir: dir };
        harness.pump_until(|harness| harness.entry("example.a").digest.is_some());
        let digest = harness.entry("example.a").digest.clone().unwrap();
        std::fs::write(harness._dir.path().join("plugins/example-a/main.rhai"), LOOP).unwrap();
        let error = harness.host.approve(&PluginId::parse("example.a").unwrap(), &digest).unwrap_err();
        assert!(error.contains("changed"), "{error}");
        harness.pump_until(|harness| harness.entry("example.a").state == PluginState::PendingApproval);
        assert_eq!(harness.entry("example.a").descriptors.len(), 0);
    }

    #[test]
    fn hook_refresh_and_contribution_events_are_forwarded() {
        let script = r#"
fn register() {
    [
        #{ kind: "command", id: "copy", title: "Copy", function: "run" },
        #{ kind: "hook", id: "changed", title: "Changed", function: "hook" }
    ]
}
fn run(ctx) { #{ action: "copy-text", text: "x" } }
fn hook(ctx) { #{ refresh: true } }
"#;
        let mut harness = Harness::new(&[("example.a", script)]);
        harness.approve("example.a");
        let plugin = PluginId::parse("example.a").unwrap();
        let hook = ExtensionId::new(plugin.clone(), crow_core::LocalId::parse("changed").unwrap());
        harness.host.notify(hook, HookEvent::FilesChanged { scope: "/tmp".into() }, 4);
        harness.pump_until(|harness| {
            harness.seen.iter().any(|event| {
                matches!(
                    event,
                    HostEvent::Refresh { plugin: p, .. } if p == &plugin
                )
            })
        });
    }
}
