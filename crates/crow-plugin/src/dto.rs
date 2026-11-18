//! Wire types shared by the host, the worker and (for display) the app.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crow_core::{
    CommandIntent, ContributionValues, Descriptor, EntryData, EntryHandle, ExtensionId, ExtensionKind, HookEvent,
    PluginId, Snapshot,
};

use crate::manifest::{FsAccess, PackageErrorKind, Permissions};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RequestStamp {
    pub generation: u64,
    pub request_id: u64,
    pub context_revision: u64,
}

/// A host-owned path for one request handle. The script only ever sees the
/// handle; `identity` is the listing's `(device, inode)` when it has one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryRef {
    pub path: PathBuf,
    pub identity: Option<(u64, u64)>,
}

impl EntryRef {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into(), identity: None }
    }

    pub fn with_identity(path: impl Into<PathBuf>, device: u64, inode: u64) -> Self {
        Self { path: path.into(), identity: Some((device, inode)) }
    }
}

#[derive(Clone, Debug)]
pub struct RequestInput {
    pub snapshot: Snapshot,
    /// Not converted into the script map; `read_excerpt` consults this table.
    pub reads: BTreeMap<EntryHandle, EntryRef>,
}

impl RequestInput {
    pub fn validate(&self, limits: &crate::limits::Limits) -> Result<(), String> {
        if self.snapshot.entries.len() > limits.max_entries_per_call {
            return Err(format!("snapshot has more than {} entries", limits.max_entries_per_call));
        }
        if self.snapshot.scope.len() > limits.max_display_bytes || self.snapshot.scope.chars().any(char::is_control) {
            return Err("scope is not bounded plain text".into());
        }
        let mut handles = std::collections::HashSet::new();
        for entry in &self.snapshot.entries {
            if !handles.insert(entry.handle) {
                return Err(format!("duplicate entry handle {}", entry.handle.0));
            }
            if entry.name.len() > limits.max_display_bytes || entry.name.chars().any(char::is_control) {
                return Err(format!("entry {} name is not bounded plain text", entry.handle.0));
            }
            if entry.kind.len() > limits.max_display_bytes || entry.kind.chars().any(char::is_control) {
                return Err(format!("entry {} kind is not bounded plain text", entry.handle.0));
            }
        }
        if let Some(selection) = self.snapshot.selection
            && !handles.contains(&selection)
        {
            return Err(format!("selection handle {} is not in the snapshot", selection.0));
        }
        Ok(())
    }
}

/// Per-call cancellation. The host sets it when a selection, context or plugin
/// generation is superseded; `on_progress` observes it between Rhai operations.
pub type CancelToken = Arc<AtomicBool>;

#[derive(Clone, Debug)]
pub enum Call {
    Register,
    Evaluate { extension: ExtensionId, input: RequestInput, stamp: RequestStamp, cancel: CancelToken },
    Notify { extension: ExtensionId, event: HookEvent, stamp: RequestStamp, cancel: CancelToken },
}

impl Call {
    pub fn stamp(&self) -> Option<RequestStamp> {
        match self {
            Self::Register => None,
            Self::Evaluate { stamp, .. } | Self::Notify { stamp, .. } => Some(*stamp),
        }
    }

    pub fn cancel_token(&self) -> Option<&CancelToken> {
        match self {
            Self::Register => None,
            Self::Evaluate { cancel, .. } | Self::Notify { cancel, .. } => Some(cancel),
        }
    }

    pub fn cancel(&self) {
        if let Some(token) = self.cancel_token() {
            token.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    pub fn coalesce_key(&self) -> Option<NotifyKey> {
        let Self::Notify { extension, event, stamp, .. } = self else {
            return None;
        };
        Some(NotifyKey {
            extension: extension.clone(),
            kind: event.kind(),
            scope: event.scope().map(str::to_owned),
            context_revision: stamp.context_revision,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NotifyKey {
    pub extension: ExtensionId,
    pub kind: &'static str,
    pub scope: Option<String>,
    pub context_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    Registered(Vec<Descriptor>),
    Values(ContributionValues),
    Intent(CommandIntent),
    Refresh,
    Unit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallErrorKind {
    Script,
    Budget,
    InvalidReply,
    Capability,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallError {
    pub kind: CallErrorKind,
    pub message: String,
}

impl CallError {
    pub fn new(kind: CallErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }

    pub fn script(message: impl Into<String>) -> Self {
        Self::new(CallErrorKind::Script, message)
    }

    pub fn budget(message: impl Into<String>) -> Self {
        Self::new(CallErrorKind::Budget, message)
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(CallErrorKind::InvalidReply, message)
    }

    pub fn capability(message: impl Into<String>) -> Self {
        Self::new(CallErrorKind::Capability, message)
    }

    pub fn cancelled() -> Self {
        Self::new(CallErrorKind::Cancelled, "cancelled")
    }
}

#[derive(Clone, Debug)]
pub enum WorkerMessage {
    Registered { plugin: PluginId, generation: u64, result: Result<Vec<Descriptor>, CallError> },
    Replied { plugin: PluginId, generation: u64, extension: ExtensionId, stamp: RequestStamp, reply: Reply },
    Failed { plugin: PluginId, generation: u64, request: Option<RequestStamp>, error: CallError },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginState {
    PendingApproval,
    Starting,
    Running,
    Paused,
    Failed,
    Disabled,
    InvalidRead,
    InvalidManifest,
    IncompatibleApi,
    UnsupportedCapability,
    ReservedId,
    DuplicateId,
}

impl PluginState {
    pub fn label(self) -> &'static str {
        match self {
            Self::PendingApproval => "Pending approval",
            Self::Starting => "Starting",
            Self::Running => "Running",
            Self::Paused => "Paused",
            Self::Failed => "Failed",
            Self::Disabled => "Disabled",
            Self::InvalidRead => "Unreadable package",
            Self::InvalidManifest => "Invalid manifest",
            Self::IncompatibleApi => "Incompatible API",
            Self::UnsupportedCapability => "Unsupported capability",
            Self::ReservedId => "Reserved plugin ID",
            Self::DuplicateId => "Duplicate plugin ID",
        }
    }

    pub fn is_error(self) -> bool {
        matches!(
            self,
            Self::Paused
                | Self::Failed
                | Self::InvalidRead
                | Self::InvalidManifest
                | Self::IncompatibleApi
                | Self::UnsupportedCapability
                | Self::ReservedId
                | Self::DuplicateId
        )
    }
}

#[derive(Clone, Debug)]
pub struct PluginEntry {
    pub dir_name: String,
    pub id: Option<PluginId>,
    pub name: String,
    pub version: Option<String>,
    pub provides: Vec<ExtensionKind>,
    pub state: PluginState,
    pub detail: Option<String>,
    pub digest: Option<String>,
    pub requested_fs: FsAccess,
    pub descriptors: Vec<Descriptor>,
    pub generation: u64,
}

impl PluginEntry {
    pub fn title(&self) -> &str {
        if self.name.trim().is_empty() { &self.dir_name } else { &self.name }
    }

    pub fn can_approve(&self) -> bool {
        self.id.is_some() && self.state == PluginState::PendingApproval
    }

    pub fn can_disable(&self) -> bool {
        matches!(self.state, PluginState::Starting | PluginState::Running | PluginState::Paused | PluginState::Failed)
    }

    pub fn can_enable(&self) -> bool {
        self.id.is_some() && self.state == PluginState::Disabled
    }

    pub fn can_retry(&self) -> bool {
        matches!(self.state, PluginState::Paused | PluginState::Failed)
    }

    pub fn can_reload(&self) -> bool {
        self.id.is_some() && self.state != PluginState::InvalidRead
    }

    pub fn permissions_summary(&self) -> String {
        match self.requested_fs {
            FsAccess::Read => "Can read bounded excerpts of entries it is given".into(),
            FsAccess::None => "No file access".into(),
        }
    }
}

impl From<&PackageErrorKind> for PluginState {
    fn from(kind: &PackageErrorKind) -> Self {
        match kind {
            PackageErrorKind::Read => Self::InvalidRead,
            PackageErrorKind::Manifest => Self::InvalidManifest,
            PackageErrorKind::UnsupportedCapability => Self::UnsupportedCapability,
            PackageErrorKind::IncompatibleApi => Self::IncompatibleApi,
            PackageErrorKind::ReservedId => Self::ReservedId,
            PackageErrorKind::DuplicateId => Self::DuplicateId,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Inventory {
    pub plugins: Vec<PluginEntry>,
    /// `plugins.toml` was unreadable or invalid; every grant is withdrawn.
    pub grants_error: Option<String>,
    /// `<config>/plugins` itself could not be read.
    pub scan_error: Option<String>,
}

#[derive(Clone, Debug)]
pub enum HostEvent {
    Inventory(Inventory),
    Status { plugin: Option<PluginId>, name: String, message: String, error: bool },
    CommandIntent { plugin: PluginId, stamp: RequestStamp, intent: CommandIntent },
    Refresh { plugin: PluginId, stamp: RequestStamp },
    Contribution { extension: ExtensionId, stamp: RequestStamp, values: ContributionValues },
}

/// The manifest's requested capabilities in a displayable form.
pub fn permissions_summary(permissions: &Permissions) -> String {
    let mut parts = vec![format!("fs={}", permissions.fs().key())];
    if !permissions.spawn.is_empty() {
        parts.push(format!("spawn={}", permissions.spawn.join(",")));
    }
    if permissions.net {
        parts.push("net=true".into());
    }
    parts.join(" ")
}

/// A synthetic snapshot entry, for tests and P1B's host-side input builder.
pub fn entry_data(handle: u32, name: impl Into<String>, is_dir: bool, size: u64, kind: impl Into<String>) -> EntryData {
    EntryData { handle: EntryHandle(handle), name: name.into(), is_dir, size, kind: kind.into() }
}
