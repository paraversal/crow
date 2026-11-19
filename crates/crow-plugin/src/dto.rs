//! Wire types shared by the host, the worker and (for display) the app.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crow_core::{
    CommandIntent, ContributionValues, Descriptor, EntryData, EntryHandle, ExtensionId, ExtensionKind, HookEvent,
    PluginId, Snapshot,
};

use crate::manifest::{FsAccess, PackageErrorKind, Permissions};
use crate::package::PackageSnapshot;

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

/// A capped text excerpt the Rust host read before enqueueing the call. The
/// script sees it as data on the matching entry; it never gets a path or a
/// read function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExcerptInput {
    pub handle: EntryHandle,
    pub text: String,
}

/// Repository facts a host may attach to one call. Only the bundled provider
/// that owns the extension may fill this; the script sees it as plain data.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostData {
    pub repository: Option<RepositoryData>,
    pub entries: Vec<EntryHostData>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepositoryData {
    pub branch: Option<String>,
    /// A short commit id for a detached HEAD.
    pub head: Option<String>,
    pub detached: bool,
    pub unborn: bool,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub changed: u32,
    /// The first line of HEAD's commit message, when the caller knows it.
    pub last_commit: Option<String>,
    /// Status could not be read safely (for example a cloud-backed worktree).
    pub unavailable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryHostData {
    pub handle: EntryHandle,
    /// The one badge the host chose, following its display precedence.
    pub badge: String,
    /// The spoken description for that badge, e.g. "conflicted".
    pub label: String,
    pub index: Option<String>,
    pub worktree: Option<String>,
    pub ignored: bool,
    /// Staged plus unstaged text line edits; absent means unknown.
    pub lines_changed: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct RequestInput {
    pub snapshot: Snapshot,
    /// At most one host-read excerpt for an entry in this snapshot.
    pub excerpt: Option<ExcerptInput>,
    /// Provider facts for the extension that owns this call, if any.
    pub host: Option<HostData>,
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
        if let Some(excerpt) = &self.excerpt {
            if !handles.contains(&excerpt.handle) {
                return Err(format!("excerpt handle {} is not in the snapshot", excerpt.handle.0));
            }
            if excerpt.text.len() > limits.max_read_bytes || excerpt.text.contains('\0') {
                return Err(format!("excerpt for handle {} is not bounded text", excerpt.handle.0));
            }
        }
        if let Some(host) = &self.host {
            let bounded = |text: Option<&str>, what: &str| {
                let text = text.unwrap_or_default();
                if text.len() > limits.max_display_bytes || text.chars().any(char::is_control) {
                    return Err(format!("{what} is not bounded plain text"));
                }
                Ok(text.len())
            };
            let mut host_bytes = 0usize;
            if let Some(repository) = &host.repository {
                host_bytes = bounded(repository.branch.as_deref(), "repository branch")?
                    + bounded(repository.head.as_deref(), "repository head")?
                    + bounded(repository.last_commit.as_deref(), "last commit")?;
            }
            if host.entries.len() > limits.max_entries_per_call {
                return Err(format!("host data has more than {} entries", limits.max_entries_per_call));
            }
            let mut host_handles = std::collections::HashSet::new();
            for entry in &host.entries {
                if !handles.contains(&entry.handle) {
                    return Err(format!("host data handle {} is not in the snapshot", entry.handle.0));
                }
                if !host_handles.insert(entry.handle) {
                    return Err(format!("duplicate host data handle {}", entry.handle.0));
                }
                if entry.badge.len() > limits.max_badge_bytes || entry.badge.chars().any(char::is_control) {
                    return Err(format!("badge for handle {} is not bounded plain text", entry.handle.0));
                }
                host_bytes = host_bytes
                    .saturating_add(entry.badge.len())
                    .saturating_add(bounded(Some(&entry.label), "entry label")?)
                    .saturating_add(bounded(entry.index.as_deref(), "index status")?)
                    .saturating_add(bounded(entry.worktree.as_deref(), "worktree status")?)
                    .saturating_add(32);
            }
            if host_bytes > limits.max_host_bytes {
                return Err(format!("host data is larger than {} bytes", limits.max_host_bytes));
            }
        }
        Ok(())
    }
}

/// Per-call cancellation. The host sets it when a selection, context or plugin
/// generation is superseded; `on_progress` observes it between Rhai operations.
pub type CancelToken = Arc<AtomicBool>;

/// The caller's handle on one admitted-or-pending request. The stamp is fixed
/// when the handle submits it; cancellation is visible to the service and the
/// app immediately, even if a terminal event is already queued.
#[derive(Clone, Debug)]
pub struct RequestTicket {
    stamp: RequestStamp,
    cancel: CancelToken,
}

impl RequestTicket {
    pub(crate) fn new(stamp: RequestStamp, cancel: CancelToken) -> Self {
        Self { stamp, cancel }
    }

    pub fn stamp(&self) -> RequestStamp {
        self.stamp
    }

    pub fn cancel(&self) {
        self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Read-only cancellation state. The app uses it to reject a terminal reply
    /// that raced a superseded binding.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitError {
    Busy,
    Stopped,
}

/// Why a request ended without a contribution. Every failure category P1B has
/// to distinguish is typed, never inferred from a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestErrorKind {
    /// The worker's explicit command or hook lane was full.
    Busy,
    /// The ticket was cancelled, including a superseded hook.
    Cancelled,
    /// The caller used a generation that is no longer current.
    StaleGeneration,
    /// No Running record or no registered extension with that ID.
    Unavailable,
    /// Malformed input, or an Evaluate/Notify call for the wrong kind.
    InvalidInput,
    /// The input needed a capability the grant does not authorize.
    Capability,
    /// The worker failed the call or was withdrawn while it ran.
    WorkerFailed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestError {
    pub kind: RequestErrorKind,
    pub message: String,
}

impl RequestError {
    pub fn new(kind: RequestErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }

    pub fn cancelled() -> Self {
        Self::new(RequestErrorKind::Cancelled, "cancelled")
    }
}

/// The one terminal result of an admitted request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestResult {
    Values(ContributionValues),
    Intent(CommandIntent),
    Refresh,
    Unit,
    Error(RequestError),
}

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

/// One data-only icon pack published by a Running package. The snapshot keeps
/// the exact approved bytes alive; P1B discards held packs when the generation
/// is withdrawn, and P3A validates the TOML and glyph references.
#[derive(Clone, Debug)]
pub struct RegisteredIconPack {
    id: ExtensionId,
    generation: u64,
    asset: String,
    snapshot: Arc<PackageSnapshot>,
}

impl RegisteredIconPack {
    pub(crate) fn new(id: ExtensionId, generation: u64, asset: String, snapshot: Arc<PackageSnapshot>) -> Self {
        Self { id, generation, asset, snapshot }
    }

    pub fn id(&self) -> &ExtensionId {
        &self.id
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn asset(&self) -> &str {
        &self.asset
    }

    pub fn pack_bytes(&self) -> Option<Arc<[u8]>> {
        self.snapshot.files.get(&self.asset).cloned()
    }

    /// A glyph reference is readable only when the manifest declared that exact
    /// bundle-relative asset. The returned bytes belong to the approved
    /// snapshot, never to a later disk read.
    pub fn declared_asset(&self, name: &str) -> Option<Arc<[u8]>> {
        self.snapshot
            .manifest
            .assets
            .iter()
            .any(|asset| asset == name)
            .then(|| self.snapshot.files.get(name).cloned())
            .flatten()
    }
}

#[derive(Clone, Debug)]
pub struct PluginEntry {
    /// Raw immediate child directory name; the row identity Settings stores.
    pub bundle: OsString,
    /// Display-only lossy name.
    pub dir_name: String,
    pub id: Option<PluginId>,
    pub name: String,
    pub version: Option<String>,
    pub provides: Vec<ExtensionKind>,
    pub state: PluginState,
    pub detail: Option<String>,
    pub digest: Option<String>,
    pub requested_fs: FsAccess,
    /// The approved grant currently allows host-read excerpts.
    pub read_allowed: bool,
    pub descriptors: Vec<Descriptor>,
    /// Derived from the Running record; empty outside Running.
    pub icon_packs: Vec<RegisteredIconPack>,
    pub generation: u64,
    /// True when the package is embedded in the app rather than installed.
    pub bundled: bool,
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

/// A host mutation the app can correlate with its own suppression. The result
/// is the host's outcome, not the queued send result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostMutation {
    Approve,
    Disable,
    Enable,
    Reload,
    Retry,
}

#[derive(Clone, Debug)]
pub enum HostEvent {
    Inventory(Inventory),
    Status {
        plugin: Option<PluginId>,
        name: String,
        message: String,
        error: bool,
    },
    /// Exactly one terminal event for a ticket accepted at ingress while the
    /// service is alive. A stopped service closes the receiver instead.
    RequestFinished {
        extension: ExtensionId,
        stamp: RequestStamp,
        result: RequestResult,
    },
    /// The host finished one approved mutation. Disable suppression waits for
    /// this rather than for an unrelated inventory event.
    MutationFinished {
        plugin: PluginId,
        mutation: HostMutation,
        result: Result<(), String>,
    },
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::Limits;

    fn input(host: Option<HostData>) -> RequestInput {
        RequestInput {
            snapshot: Snapshot {
                scope: "/tmp".into(),
                selection: None,
                entries: vec![EntryData {
                    handle: EntryHandle(1),
                    name: "a.txt".into(),
                    is_dir: false,
                    size: 1,
                    kind: "Text".into(),
                }],
            },
            excerpt: None,
            host,
        }
    }

    fn entry(handle: u32) -> EntryHostData {
        EntryHostData {
            handle: EntryHandle(handle),
            badge: "M".into(),
            label: "modified".into(),
            index: None,
            worktree: Some("modified".into()),
            ignored: false,
            lines_changed: Some(2),
        }
    }

    #[test]
    fn host_data_accepts_a_matching_entry_and_rejects_bad_handles() {
        let limits = Limits::default();
        let valid = HostData {
            repository: Some(RepositoryData {
                branch: Some("main".into()),
                head: None,
                detached: false,
                unborn: false,
                ahead: Some(1),
                behind: None,
                changed: 2,
                last_commit: None,
                unavailable: false,
            }),
            entries: vec![entry(1)],
        };
        assert!(input(Some(valid)).validate(&limits).is_ok());

        let unknown = HostData { repository: None, entries: vec![entry(2)] };
        assert!(input(Some(unknown)).validate(&limits).is_err());

        let duplicate = HostData { repository: None, entries: vec![entry(1), entry(1)] };
        assert!(input(Some(duplicate)).validate(&limits).is_err());

        let mut long_badge = entry(1);
        long_badge.badge = "M".repeat(limits.max_badge_bytes + 1);
        assert!(input(Some(HostData { repository: None, entries: vec![long_badge] })).validate(&limits).is_err());

        let mut control = entry(1);
        control.label = "bad\nlabel".into();
        assert!(input(Some(HostData { repository: None, entries: vec![control] })).validate(&limits).is_err());
    }
}
