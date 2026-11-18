//! The app-owned plugin surface coordinator.
//!
//! Views publish bounded demand and read ready values; this module owns the
//! generation snapshot, the request tickets, the row cache and hook delivery.
//! It never runs a script: all evaluation stays on the plugin service's workers.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crow_core::{
    CommandIntent, ContributionValues, Descriptor, EntryData, EntryHandle, ExtensionId, ExtensionKind, HookEvent,
    PluginId, SectionDestination, SectionRow, Snapshot, Tint,
};
use crow_plugin::{
    HostMutation, Inventory, PluginHandle, PluginState, RegisteredIconPack, RequestInput, RequestResult, RequestStamp,
    RequestTicket, SubmitError,
};
use gpui::WindowId;

/// One view-owned demand source. Ids are never reused; a closed owner stops
/// observing and the coordinator drops its pending work.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SurfaceId(pub u64);

static NEXT_SURFACE: AtomicU64 = AtomicU64::new(1);

pub fn new_surface() -> SurfaceId {
    SurfaceId(NEXT_SURFACE.fetch_add(1, Ordering::Relaxed))
}

/// The raw identity of a listing or search session. Paths are compared raw.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ScopeKey {
    Folder(PathBuf),
    /// Search sessions are distinct even when they cover the same origin.
    #[allow(dead_code)]
    Search {
        session: u64,
        origin: PathBuf,
    },
}

impl ScopeKey {
    pub fn path(&self) -> &PathBuf {
        match self {
            Self::Folder(path) | Self::Search { origin: path, .. } => path,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishedExtension {
    pub generation: u64,
    pub package: String,
    pub read_allowed: bool,
    pub descriptor: Descriptor,
}

/// The immutable generation snapshot views render from.
#[derive(Clone, Debug, Default)]
pub struct PublishedSurfaces {
    pub extensions: BTreeMap<ExtensionId, PublishedExtension>,
    pub packs: BTreeMap<ExtensionId, RegisteredIconPack>,
}

#[allow(dead_code)]
impl PublishedSurfaces {
    pub fn extension(&self, id: &ExtensionId) -> Option<&PublishedExtension> {
        self.extensions.get(id)
    }

    pub fn of_kind(&self, kind: ExtensionKind) -> impl Iterator<Item = (&ExtensionId, &PublishedExtension)> {
        self.extensions.iter().filter(move |(_, extension)| extension.descriptor.kind == kind)
    }

    pub fn has_kind(&self, kind: ExtensionKind) -> bool {
        self.extensions.values().any(|extension| extension.descriptor.kind == kind)
    }

    pub fn has_any(&self, kinds: &[ExtensionKind]) -> bool {
        self.extensions.values().any(|extension| kinds.contains(&extension.descriptor.kind))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowValue {
    pub text: String,
    pub spoken: Option<String>,
    pub tint: Option<Tint>,
    pub dim: bool,
}

impl RowValue {
    fn empty() -> Self {
        Self { text: String::new(), spoken: None, tint: None, dim: false }
    }

    fn bytes(&self) -> usize {
        self.text.len() + self.spoken.as_ref().map_or(0, String::len) + 32
    }
}

/// One visible row offered to a row-contribution descriptor.
#[derive(Clone, Debug)]
pub struct RowInput {
    pub path: PathBuf,
    /// Changes when the listing or a watcher replaces or rewrites this entry.
    pub version: u64,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub kind: String,
    /// The focused surface's current selection; only a one-item selection is sent.
    pub selected: bool,
    /// Provider facts for the bundled `git` package, if any.
    pub git: Option<crow_plugin::EntryHostData>,
}

/// What a command activation captured. It must still match when the reply or
/// the reveal check returns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandCapture {
    pub surface: SurfaceId,
    pub scope: ScopeKey,
    pub selection: Vec<(PathBuf, bool)>,
}

/// A checked command effect, already resolved from the activation's handles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginEffect {
    Message(String),
    CopyText(String),
    Reveal { path: PathBuf, identity: (u64, u64) },
}

struct CommandTicket {
    stamp: RequestStamp,
    ticket: TicketProbe,
    extension: ExtensionId,
    window: WindowId,
    capture: CommandCapture,
    entries: HashMap<EntryHandle, (PathBuf, Option<(u64, u64)>)>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RowKey {
    extension: ExtensionId,
    generation: u64,
    scope: ScopeKey,
    path: PathBuf,
    version: u64,
}

struct CachedRow {
    value: RowValue,
    touch: u64,
}

#[derive(Default)]
struct RowCache {
    values: HashMap<RowKey, CachedRow>,
    bytes: usize,
    tick: u64,
}

impl RowCache {
    fn get(&mut self, key: &RowKey) -> Option<RowValue> {
        self.tick = self.tick.wrapping_add(1);
        let tick = self.tick;
        let row = self.values.get_mut(key)?;
        row.touch = tick;
        Some(row.value.clone())
    }

    fn peek(&self, key: &RowKey) -> Option<&RowValue> {
        self.values.get(key).map(|row| &row.value)
    }

    /// Refresh recency from the bounded visible-range pass. Render-time peeks
    /// stay read-only, but a row the user is looking at counts as used.
    fn touch(&mut self, key: &RowKey) {
        self.tick = self.tick.wrapping_add(1);
        let tick = self.tick;
        if let Some(row) = self.values.get_mut(key) {
            row.touch = tick;
        }
    }

    fn insert(&mut self, key: RowKey, value: RowValue) {
        if let Some(old) = self.values.remove(&key) {
            self.bytes = self.bytes.saturating_sub(old.value.bytes());
        }
        self.tick = self.tick.wrapping_add(1);
        self.bytes = self.bytes.saturating_add(value.bytes());
        self.values.insert(key, CachedRow { value, touch: self.tick });
        self.evict();
    }

    fn remove_path_any(&mut self, path: &Path) {
        let keys: Vec<RowKey> = self.values.keys().filter(|key| key.path == path).cloned().collect();
        for key in keys {
            if let Some(old) = self.values.remove(&key) {
                self.bytes = self.bytes.saturating_sub(old.value.bytes());
            }
        }
    }

    fn evict(&mut self) {
        while self.values.len() > MAX_ROW_VALUES || self.bytes > MAX_ROW_BYTES {
            let Some((key, _)) =
                self.values.iter().min_by_key(|(_, row)| row.touch).map(|(key, row)| (key.clone(), row.touch))
            else {
                break;
            };
            if let Some(row) = self.values.remove(&key) {
                self.bytes = self.bytes.saturating_sub(row.value.bytes());
            }
        }
    }

    fn remove_extension(&mut self, extension: &ExtensionId) {
        let keys: Vec<RowKey> = self.values.keys().filter(|key| &key.extension == extension).cloned().collect();
        for key in keys {
            if let Some(row) = self.values.remove(&key) {
                self.bytes = self.bytes.saturating_sub(row.value.bytes());
            }
        }
    }

    fn remove_plugin(&mut self, plugin: &PluginId) {
        let keys: Vec<RowKey> = self.values.keys().filter(|key| &key.extension.plugin == plugin).cloned().collect();
        for key in keys {
            if let Some(row) = self.values.remove(&key) {
                self.bytes = self.bytes.saturating_sub(row.value.bytes());
            }
        }
    }

    fn remove_path(&mut self, scope: &ScopeKey, path: &Path) {
        let keys: Vec<RowKey> =
            self.values.keys().filter(|key| &key.scope == scope && key.path.as_path() == path).cloned().collect();
        for key in keys {
            if let Some(row) = self.values.remove(&key) {
                self.bytes = self.bytes.saturating_sub(row.value.bytes());
            }
        }
    }

    fn remove_scope(&mut self, scope: &ScopeKey) {
        let keys: Vec<RowKey> = self.values.keys().filter(|key| &key.scope == scope).cloned().collect();
        for key in keys {
            if let Some(row) = self.values.remove(&key) {
                self.bytes = self.bytes.saturating_sub(row.value.bytes());
            }
        }
    }

    fn len(&self) -> usize {
        self.values.len()
    }

    fn bytes(&self) -> usize {
        self.bytes
    }
}

struct SurfaceState {
    scope: ScopeKey,
    scope_version: u64,
    versions: HashMap<PathBuf, u64>,
    seen: Instant,
}

struct StatusBinding {
    scope: ScopeKey,
    revision: u64,
    values: BTreeMap<ExtensionId, String>,
    seen: HashSet<ExtensionId>,
}

#[derive(Clone, Debug)]
pub struct SidebarDir {
    pub path: PathBuf,
    pub name: String,
    pub version: u64,
    /// No-follow directory identity captured off the main thread. `None` keeps
    /// the row out of a request until the host has admitted it.
    pub identity: Option<(u64, u64)>,
}

struct SidebarBinding {
    scope: ScopeKey,
    revision: u64,
    values: BTreeMap<ExtensionId, Vec<SectionRow>>,
    seen: HashSet<ExtensionId>,
}

struct InfoBinding {
    scope: ScopeKey,
    revision: u64,
    values: BTreeMap<ExtensionId, Vec<crow_core::InfoRow>>,
    seen: HashSet<ExtensionId>,
}

enum TicketKind {
    Preview {
        surface: SurfaceId,
        sender: futures::channel::oneshot::Sender<RequestResult>,
    },
    Sidebar {
        surface: SurfaceId,
        scope: ScopeKey,
        revision: u64,
        descriptor: ExtensionId,
        entries: HashMap<EntryHandle, (PathBuf, (u64, u64))>,
    },
    Info {
        surface: SurfaceId,
        scope: ScopeKey,
        revision: u64,
        descriptor: ExtensionId,
    },
    Status {
        surface: SurfaceId,
        scope: ScopeKey,
        revision: u64,
        descriptor: ExtensionId,
    },
    Rows {
        surface: SurfaceId,
        scope: ScopeKey,
        scope_version: u64,
        descriptor: ExtensionId,
        entries: Vec<(EntryHandle, PathBuf, u64)>,
    },
    Hook {
        scope: Option<ScopeKey>,
    },
}

enum TicketProbe {
    Real(RequestTicket),
    #[cfg(test)]
    Flag(std::sync::Arc<std::sync::atomic::AtomicBool>),
}

impl TicketProbe {
    fn is_cancelled(&self) -> bool {
        match self {
            Self::Real(ticket) => ticket.is_cancelled(),
            #[cfg(test)]
            Self::Flag(flag) => flag.load(Ordering::Relaxed),
        }
    }

    fn cancel(&self) {
        match self {
            Self::Real(ticket) => ticket.cancel(),
            #[cfg(test)]
            Self::Flag(flag) => flag.store(true, Ordering::Relaxed),
        }
    }
}

struct PassiveTicket {
    stamp: RequestStamp,
    ticket: TicketProbe,
    extension: ExtensionId,
    kind: TicketKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum HookSlot {
    Scope(SurfaceId),
    Selection(SurfaceId),
    Files(PathBuf),
    Operation,
}

struct PendingHook {
    due: Instant,
    event: HookEvent,
    scope: Option<ScopeKey>,
}

#[derive(Default, Clone, Debug)]
pub struct PluginCounters {
    pub submitted_rows: u64,
    pub submitted_hooks: u64,
    pub coalesced_rows: u64,
    pub coalesced_hooks: u64,
    pub rejected_tickets: u64,
    pub cache_high_water: usize,
}

struct PendingRows {
    scope: ScopeKey,
    scope_version: u64,
    scope_display: String,
    context_revision: u64,
    descriptors: Vec<ExtensionId>,
    rows: Vec<RowInput>,
    repository: Option<crow_plugin::RepositoryData>,
}

/// The coordinator. One instance lives behind the app's `Plugins` global.
pub struct PluginSurfaces {
    snapshot: std::sync::Arc<PublishedSurfaces>,
    snapshot_generation: u64,
    rows: RowCache,
    surfaces: HashMap<SurfaceId, SurfaceState>,
    pending_rows: HashMap<SurfaceId, PendingRows>,
    tickets: HashMap<RequestStamp, PassiveTicket>,
    commands: HashMap<RequestStamp, CommandTicket>,
    status: HashMap<SurfaceId, StatusBinding>,
    info: HashMap<SurfaceId, InfoBinding>,
    sidebar: HashMap<SurfaceId, SidebarBinding>,
    hooks: HashMap<HookSlot, PendingHook>,
    suppressed: HashSet<PluginId>,
    retry_at: Option<Instant>,
    counters: PluginCounters,
}

impl Default for PluginSurfaces {
    fn default() -> Self {
        Self {
            snapshot: std::sync::Arc::new(PublishedSurfaces::default()),
            snapshot_generation: 0,
            rows: RowCache::default(),
            surfaces: HashMap::new(),
            pending_rows: HashMap::new(),
            tickets: HashMap::new(),
            commands: HashMap::new(),
            status: HashMap::new(),
            info: HashMap::new(),
            sidebar: HashMap::new(),
            hooks: HashMap::new(),
            suppressed: HashSet::new(),
            retry_at: None,
            counters: PluginCounters::default(),
        }
    }
}

#[allow(dead_code)]
impl PluginSurfaces {
    pub fn snapshot(&self) -> std::sync::Arc<PublishedSurfaces> {
        self.snapshot.clone()
    }

    /// Increments whenever the published generation snapshot changes.
    pub fn generation(&self) -> u64 {
        self.snapshot_generation
    }

    pub fn row_value_peek(
        &self,
        extension: &ExtensionId,
        scope: &ScopeKey,
        path: &Path,
        version: u64,
    ) -> Option<RowValue> {
        let published = self.snapshot.extension(extension)?;
        self.rows
            .peek(&RowKey {
                extension: extension.clone(),
                generation: published.generation,
                scope: scope.clone(),
                path: path.to_path_buf(),
                version,
            })
            .cloned()
    }

    pub fn counters(&self) -> &PluginCounters {
        &self.counters
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn row_bytes(&self) -> usize {
        self.rows.bytes()
    }

    pub fn pending_tickets(&self) -> usize {
        self.tickets.len()
    }

    /// Publish the newest inventory. A generation that disappeared or changed
    /// is dropped before any terminal reply from the same poll drain is read.
    pub fn publish_inventory(&mut self, inventory: &Inventory) -> bool {
        let mut next = PublishedSurfaces::default();
        for entry in inventory.plugins.iter().filter(|entry| entry.state == PluginState::Running) {
            for descriptor in &entry.descriptors {
                next.extensions.insert(
                    descriptor.id.clone(),
                    PublishedExtension {
                        generation: entry.generation,
                        package: entry.title().to_owned(),
                        read_allowed: entry.read_allowed,
                        descriptor: descriptor.clone(),
                    },
                );
            }
            for pack in &entry.icon_packs {
                next.packs.insert(pack.id().clone(), pack.clone());
            }
        }
        let invalidated: Vec<ExtensionId> = self
            .snapshot
            .extensions
            .iter()
            .filter(|(id, old)| next.extensions.get(id).is_none_or(|new| new.generation != old.generation))
            .map(|(id, _)| id.clone())
            .collect();
        let same_extensions = invalidated.is_empty()
            && next.extensions.len() == self.snapshot.extensions.len()
            && self.snapshot.extensions.iter().all(|(id, old)| {
                next.extensions
                    .get(id)
                    .is_some_and(|new| new.descriptor == old.descriptor && new.package == old.package)
            });
        let same_packs = next.packs.len() == self.snapshot.packs.len()
            && self
                .snapshot
                .packs
                .iter()
                .all(|(id, old)| next.packs.get(id).is_some_and(|new| new.generation() == old.generation()));
        if same_extensions && same_packs {
            // Identical approved reload: keep the generation and every ready value.
            return false;
        }
        for id in &invalidated {
            self.rows.remove_extension(id);
            self.tickets.retain(|_, ticket| {
                if &ticket.extension == id {
                    ticket.ticket.cancel();
                    false
                } else {
                    true
                }
            });
            self.commands.retain(|_, command| {
                if &command.extension == id {
                    command.ticket.cancel();
                    false
                } else {
                    true
                }
            });
        }
        self.suppressed.retain(|plugin| {
            inventory.plugins.iter().any(|entry| {
                entry.id.as_ref() == Some(plugin) && matches!(entry.state, PluginState::Starting | PluginState::Running)
            })
        });
        self.status.clear();
        self.info.clear();
        self.sidebar.clear();
        self.snapshot = std::sync::Arc::new(next);
        self.snapshot_generation = self.snapshot_generation.wrapping_add(1);
        true
    }

    /// Settings queued a Disable. Drop the package's contributions now and keep
    /// them suppressed until the host reports the correlated completion.
    pub fn suppress_disable(&mut self, plugin: &PluginId) {
        self.suppressed.insert(plugin.clone());
        self.rows.remove_plugin(plugin);
        self.tickets.retain(|_, ticket| {
            if &ticket.extension.plugin == plugin {
                ticket.ticket.cancel();
                false
            } else {
                true
            }
        });
        self.commands.retain(|_, command| {
            if &command.extension.plugin == plugin {
                command.ticket.cancel();
                false
            } else {
                true
            }
        });
        self.status.clear();
        self.info.clear();
        self.sidebar.clear();
        // Views must drop the package's columns and accepted previews now, before
        // the host's inventory round trip.
        self.snapshot_generation = self.snapshot_generation.wrapping_add(1);
    }

    pub fn mutation_finished(
        &mut self,
        plugin: &PluginId,
        mutation: HostMutation,
        _result: &Result<(), String>,
    ) -> bool {
        if mutation != HostMutation::Disable {
            return false;
        }
        if self.suppressed.remove(plugin) {
            // Disable failed and the authoritative inventory still allows the
            // package, or it succeeded: re-resolve views either way.
            self.snapshot_generation = self.snapshot_generation.wrapping_add(1);
        }
        self.snapshot.extensions.values().any(|extension| &extension.descriptor.id.plugin == plugin)
    }

    pub fn is_suppressed(&self, plugin: &PluginId) -> bool {
        self.suppressed.contains(plugin)
    }

    fn current_extension(&self, extension: &ExtensionId, generation: u64) -> Option<&PublishedExtension> {
        let published = self.snapshot.extension(extension)?;
        (published.generation == generation && !self.suppressed.contains(&extension.plugin)).then_some(published)
    }

    fn in_flight_for(&self, extension: &ExtensionId) -> usize {
        self.tickets.values().filter(|ticket| &ticket.extension == extension).count()
    }

    fn can_submit(&self, extension: &ExtensionId) -> bool {
        self.tickets.len() < MAX_PASSIVE_REQUESTS && self.in_flight_for(extension) < MAX_PASSIVE_PER_PLUGIN
    }

    /// Ask each Status descriptor once per active scope/revision. The display
    /// uses the first nonempty value in fully qualified ID order.
    #[allow(clippy::too_many_arguments)]
    pub fn request_status(
        &mut self,
        handle: Option<&PluginHandle>,
        surface: SurfaceId,
        scope: ScopeKey,
        revision: u64,
        display: &str,
        descriptors: &[ExtensionId],
        host: Option<crow_plugin::HostData>,
    ) {
        let Some(handle) = handle else { return };
        let current =
            self.status.get(&surface).is_some_and(|binding| binding.scope == scope && binding.revision == revision);
        if !current {
            self.tickets.retain(|_, ticket| {
                let keep = !matches!(&ticket.kind, TicketKind::Status { surface: s, .. } if *s == surface);
                if !keep {
                    ticket.ticket.cancel();
                }
                keep
            });
            self.status.insert(
                surface,
                StatusBinding { scope: scope.clone(), revision, values: BTreeMap::new(), seen: HashSet::new() },
            );
        }
        let Some(binding) = self.status.get(&surface) else { return };
        let needed: Vec<ExtensionId> = descriptors
            .iter()
            .filter(|id| !binding.seen.contains(*id))
            .filter(|id| !(binding.values.contains_key(*id)))
            .cloned()
            .collect();
        for descriptor in needed {
            let Some(published) = self.snapshot.extension(&descriptor).cloned() else { continue };
            if published.descriptor.kind != ExtensionKind::Status
                || self.suppressed.contains(&descriptor.plugin)
                || !self.can_submit(&descriptor)
            {
                continue;
            }
            let input = RequestInput {
                snapshot: Snapshot { scope: bounded(display), selection: None, entries: Vec::new() },
                excerpt: None,
                host: host.clone(),
            };
            match handle.try_evaluate(descriptor.clone(), published.generation, revision, input) {
                Ok(ticket) => {
                    // Mark seen when the request is admitted, not when it answers:
                    // a render loop must not submit the same status again while
                    // the first reply is in flight.
                    if let Some(binding) = self.status.get_mut(&surface) {
                        binding.seen.insert(descriptor.clone());
                    }
                    self.counters.submitted_rows += 1;
                    self.tickets.insert(
                        ticket.stamp(),
                        PassiveTicket {
                            stamp: ticket.stamp(),
                            ticket: TicketProbe::Real(ticket),
                            extension: descriptor.clone(),
                            kind: TicketKind::Status { surface, scope: scope.clone(), revision, descriptor },
                        },
                    );
                }
                Err(SubmitError::Busy) => break,
                Err(SubmitError::Stopped) => break,
            }
        }
    }

    pub fn status_item(&self, surface: SurfaceId) -> Option<(String, String)> {
        let binding = self.status.get(&surface)?;
        for (id, text) in &binding.values {
            if text.is_empty() {
                continue;
            }
            let title = self
                .snapshot
                .extension(id)
                .map_or_else(|| id.to_string(), |published| published.descriptor.title.clone());
            return Some((text.clone(), title));
        }
        None
    }

    /// Ask each Info descriptor once for the visible single selection.
    #[allow(clippy::too_many_arguments)]
    pub fn request_info(
        &mut self,
        handle: Option<&PluginHandle>,
        surface: SurfaceId,
        scope: ScopeKey,
        revision: u64,
        display: &str,
        descriptors: &[ExtensionId],
        entry: EntryData,
        host: Option<crow_plugin::HostData>,
    ) {
        let Some(handle) = handle else { return };
        let current =
            self.info.get(&surface).is_some_and(|binding| binding.scope == scope && binding.revision == revision);
        if !current {
            self.tickets.retain(|_, ticket| {
                let keep = !matches!(&ticket.kind, TicketKind::Info { surface: s, .. } if *s == surface);
                if !keep {
                    ticket.ticket.cancel();
                }
                keep
            });
            self.info.insert(
                surface,
                InfoBinding { scope: scope.clone(), revision, values: BTreeMap::new(), seen: HashSet::new() },
            );
        }
        let Some(binding) = self.info.get(&surface) else { return };
        let needed: Vec<ExtensionId> = descriptors.iter().filter(|id| !binding.seen.contains(*id)).cloned().collect();
        for descriptor in needed {
            let Some(published) = self.snapshot.extension(&descriptor).cloned() else { continue };
            if published.descriptor.kind != ExtensionKind::Info
                || self.suppressed.contains(&descriptor.plugin)
                || !self.can_submit(&descriptor)
            {
                continue;
            }
            let input = RequestInput {
                snapshot: Snapshot { scope: bounded(display), selection: None, entries: vec![entry.clone()] },
                excerpt: None,
                host: host.clone(),
            };
            match handle.try_evaluate(descriptor.clone(), published.generation, revision, input) {
                Ok(ticket) => {
                    if let Some(binding) = self.info.get_mut(&surface) {
                        binding.seen.insert(descriptor.clone());
                    }
                    self.counters.submitted_rows += 1;
                    self.tickets.insert(
                        ticket.stamp(),
                        PassiveTicket {
                            stamp: ticket.stamp(),
                            ticket: TicketProbe::Real(ticket),
                            extension: descriptor.clone(),
                            kind: TicketKind::Info { surface, scope: scope.clone(), revision, descriptor },
                        },
                    );
                }
                Err(SubmitError::Busy | SubmitError::Stopped) => break,
            }
        }
    }

    pub fn info_rows(&self, surface: SurfaceId) -> Vec<crow_core::InfoRow> {
        let Some(binding) = self.info.get(&surface) else { return Vec::new() };
        binding.values.values().flat_map(|rows| rows.iter().take(16)).take(16).cloned().collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn request_sidebar(
        &mut self,
        handle: Option<&PluginHandle>,
        surface: SurfaceId,
        descriptor: &ExtensionId,
        scope: ScopeKey,
        revision: u64,
        display: &str,
        dirs: &[SidebarDir],
    ) {
        let Some(handle) = handle else { return };
        let current =
            self.sidebar.get(&surface).is_some_and(|binding| binding.scope == scope && binding.revision == revision);
        if !current {
            self.tickets.retain(|_, ticket| {
                let keep = !matches!(&ticket.kind, TicketKind::Sidebar { surface: s, .. } if *s == surface);
                if !keep {
                    ticket.ticket.cancel();
                }
                keep
            });
            self.sidebar.insert(
                surface,
                SidebarBinding { scope: scope.clone(), revision, values: BTreeMap::new(), seen: HashSet::new() },
            );
        }
        let Some(binding) = self.sidebar.get(&surface) else { return };
        if binding.seen.contains(descriptor) || !self.can_submit(descriptor) {
            return;
        }
        let Some(published) = self.snapshot.extension(descriptor).cloned() else { return };
        if published.descriptor.kind != ExtensionKind::Sidebar || self.suppressed.contains(&descriptor.plugin) {
            return;
        }
        let mut entries = Vec::with_capacity(dirs.len().min(MAX_BATCH));
        let mut table = HashMap::new();
        for (index, dir) in dirs.iter().filter(|dir| dir.identity.is_some()).take(MAX_BATCH).enumerate() {
            let Some(identity) = dir.identity else { continue };
            let entry_handle = EntryHandle(index as u32 + 1);
            entries.push(EntryData {
                handle: entry_handle,
                name: bounded(&dir.name),
                is_dir: true,
                size: 0,
                kind: String::new(),
            });
            table.insert(entry_handle, (dir.path.clone(), identity));
            let _ = dir.version;
        }
        let input = RequestInput {
            snapshot: Snapshot { scope: bounded(display), selection: None, entries },
            excerpt: None,
            host: None,
        };
        match handle.try_evaluate(descriptor.clone(), published.generation, revision, input) {
            Ok(ticket) => {
                if let Some(binding) = self.sidebar.get_mut(&surface) {
                    binding.seen.insert(descriptor.clone());
                }
                self.counters.submitted_rows += 1;
                self.tickets.insert(
                    ticket.stamp(),
                    PassiveTicket {
                        stamp: ticket.stamp(),
                        ticket: TicketProbe::Real(ticket),
                        extension: descriptor.clone(),
                        kind: TicketKind::Sidebar {
                            surface,
                            scope,
                            revision,
                            descriptor: descriptor.clone(),
                            entries: table,
                        },
                    },
                );
            }
            Err(SubmitError::Busy | SubmitError::Stopped) => {}
        }
    }

    /// Submit one preview provider. The caller races the receiver against the
    /// total plugin deadline and falls through to the next candidate.
    pub fn request_preview(
        &mut self,
        handle: &PluginHandle,
        surface: SurfaceId,
        descriptor: &ExtensionId,
        generation: u64,
        revision: u64,
        input: RequestInput,
    ) -> Option<futures::channel::oneshot::Receiver<RequestResult>> {
        let published = self.snapshot.extension(descriptor)?.clone();
        if published.generation != generation
            || published.descriptor.kind != ExtensionKind::Preview
            || self.suppressed.contains(&descriptor.plugin)
            || !self.can_submit(descriptor)
        {
            return None;
        }
        let (sender, receiver) = futures::channel::oneshot::channel();
        match handle.try_evaluate(descriptor.clone(), published.generation, revision, input) {
            Ok(ticket) => {
                self.counters.submitted_rows += 1;
                self.tickets.insert(
                    ticket.stamp(),
                    PassiveTicket {
                        stamp: ticket.stamp(),
                        ticket: TicketProbe::Real(ticket),
                        extension: descriptor.clone(),
                        kind: TicketKind::Preview { surface, sender },
                    },
                );
                Some(receiver)
            }
            Err(SubmitError::Busy | SubmitError::Stopped) => None,
        }
    }

    pub fn sidebar_rows(&self, surface: SurfaceId) -> Vec<SectionRow> {
        self.sidebar.get(&surface).and_then(|binding| binding.values.values().next()).cloned().unwrap_or_default()
    }

    /// Record the latest visible-row demand for a surface. The coordinator
    /// drains it on the next poll, so a row renderer never submits a call.
    #[allow(clippy::too_many_arguments)]
    pub fn queue_rows(
        &mut self,
        surface: SurfaceId,
        scope: ScopeKey,
        scope_version: u64,
        scope_display: String,
        context_revision: u64,
        descriptors: Vec<ExtensionId>,
        rows: Vec<RowInput>,
        repository: Option<crow_plugin::RepositoryData>,
    ) {
        self.pending_rows.insert(
            surface,
            PendingRows { scope, scope_version, scope_display, context_revision, descriptors, rows, repository },
        );
    }

    /// Submit the latest queued demand. Replacing (not appending) per surface
    /// keeps scrolling from creating a backlog.
    pub fn drain_rows(&mut self, handle: Option<&PluginHandle>, now: Instant) {
        let pending = std::mem::take(&mut self.pending_rows);
        for (surface, demand) in pending {
            self.observe_rows(
                handle,
                surface,
                demand.scope,
                demand.scope_version,
                &demand.scope_display,
                demand.context_revision,
                &demand.descriptors,
                demand.rows,
                demand.repository,
                now,
            );
        }
    }

    /// Publish one bounded visible-row observation. The view reports paths and
    /// versions; the coordinator dedupes, batches and schedules misses.
    #[allow(clippy::too_many_arguments)]
    pub fn observe_rows(
        &mut self,
        handle: Option<&PluginHandle>,
        surface: SurfaceId,
        scope: ScopeKey,
        scope_version: u64,
        scope_display: &str,
        context_revision: u64,
        descriptors: &[ExtensionId],
        rows: Vec<RowInput>,
        repository: Option<crow_plugin::RepositoryData>,
        now: Instant,
    ) {
        let state = self.surfaces.entry(surface).or_insert_with(|| SurfaceState {
            scope: scope.clone(),
            scope_version,
            versions: HashMap::new(),
            seen: now,
        });
        if state.scope != scope || state.scope_version != scope_version {
            state.scope = scope.clone();
            state.scope_version = scope_version;
            state.versions.clear();
        }
        state.seen = now;
        for row in &rows {
            state.versions.insert(row.path.clone(), row.version);
        }
        // The bounded row pass is the viewport signal: a value still on screen
        // is recent even though render-time reads must not schedule work.
        for descriptor in descriptors {
            let Some(generation) = self.snapshot.extension(descriptor).map(|published| published.generation) else {
                continue;
            };
            for row in &rows {
                self.rows.touch(&RowKey {
                    extension: descriptor.clone(),
                    generation,
                    scope: scope.clone(),
                    path: row.path.clone(),
                    version: row.version,
                });
            }
        }
        if rows.is_empty() || descriptors.is_empty() {
            return;
        }
        let Some(handle) = handle else { return };
        let mut rows = rows;
        rows.sort_by_key(|row| !row.selected);
        let mut seen = HashSet::new();
        rows.retain(|row| seen.insert(row.path.clone()));
        let selection: Option<PathBuf> = rows.iter().filter(|row| row.selected).map(|row| row.path.clone()).next();

        for descriptor in descriptors {
            let Some(published) = self.snapshot.extension(descriptor) else { continue };
            let generation = published.generation;
            let kind = published.descriptor.kind;
            if self.current_extension(descriptor, generation).is_none() {
                continue;
            }
            if !matches!(kind, ExtensionKind::Column | ExtensionKind::Decoration) {
                continue;
            }
            if !self.can_submit(descriptor) {
                self.counters.coalesced_rows += 1;
                continue;
            }
            let already = self.tickets.values().any(|ticket| {
                &ticket.extension == descriptor
                    && matches!(&ticket.kind, TicketKind::Rows { surface: s, scope: q, .. } if *s == surface && q == &scope)
            });
            if already {
                self.counters.coalesced_rows += 1;
                continue;
            }
            let mut entries = Vec::with_capacity(rows.len().min(MAX_BATCH));
            let mut refs = Vec::with_capacity(rows.len().min(MAX_BATCH));
            let mut host_entries = Vec::new();
            let mut selection_handle = None;
            for row in &rows {
                if refs.len() >= MAX_BATCH {
                    break;
                }
                // A clean Git row has no contribution; asking the worker for it
                // would turn a folder scroll into hundreds of empty round trips.
                if crate::git::is_git_plugin(&descriptor.plugin) && row.git.is_none() {
                    continue;
                }
                // Demand is the misses only. A cached value, hit or empty,
                // already serves this generation, scope, path and version.
                let key = RowKey {
                    extension: descriptor.clone(),
                    generation,
                    scope: scope.clone(),
                    path: row.path.clone(),
                    version: row.version,
                };
                if self.rows.peek(&key).is_some() {
                    continue;
                }
                let handle = EntryHandle(refs.len() as u32 + 1);
                if selection.as_ref() == Some(&row.path) {
                    selection_handle = Some(handle);
                }
                entries.push(EntryData {
                    handle,
                    name: bounded(&row.name),
                    is_dir: row.is_dir,
                    size: row.size,
                    kind: bounded(&row.kind),
                });
                if let Some(git) = &row.git {
                    let mut git = git.clone();
                    git.handle = handle;
                    host_entries.push(git);
                }
                refs.push((handle, row.path.clone(), row.version));
            }
            if entries.is_empty() {
                continue;
            }
            let host = crate::git::is_git_plugin(&descriptor.plugin)
                .then(|| crow_plugin::HostData { repository: repository.clone(), entries: host_entries.clone() });
            let input = RequestInput {
                snapshot: Snapshot { scope: bounded(scope_display), selection: selection_handle, entries },
                excerpt: None,
                host,
            };
            match handle.try_evaluate(descriptor.clone(), generation, context_revision, input) {
                Ok(ticket) => {
                    self.counters.submitted_rows += 1;
                    self.tickets.insert(
                        ticket.stamp(),
                        PassiveTicket {
                            stamp: ticket.stamp(),
                            ticket: TicketProbe::Real(ticket),
                            extension: descriptor.clone(),
                            kind: TicketKind::Rows {
                                surface,
                                scope: scope.clone(),
                                scope_version,
                                descriptor: descriptor.clone(),
                                entries: refs,
                            },
                        },
                    );
                    self.retry_at = None;
                }
                Err(SubmitError::Busy) => {
                    self.counters.coalesced_rows += 1;
                    self.retry_at.get_or_insert(now + HOOK_DEBOUNCE);
                }
                Err(SubmitError::Stopped) => break,
            }
        }
    }

    /// Track one explicit command. The capture and the entry table stay with
    /// the ticket until its terminal reply.
    pub fn track_command(
        &mut self,
        ticket: RequestTicket,
        extension: ExtensionId,
        window: WindowId,
        capture: CommandCapture,
        entries: HashMap<EntryHandle, (PathBuf, Option<(u64, u64)>)>,
    ) {
        self.commands.insert(
            ticket.stamp(),
            CommandTicket {
                stamp: ticket.stamp(),
                ticket: TicketProbe::Real(ticket),
                extension,
                window,
                capture,
                entries,
            },
        );
    }

    /// Consume a terminal command reply. Returns the window and the checked
    /// effect only when the generation is still current.
    pub fn finish_command(
        &mut self,
        extension: &ExtensionId,
        stamp: RequestStamp,
        result: RequestResult,
    ) -> Option<(WindowId, ExtensionId, PluginEffect, CommandCapture)> {
        let ticket = self.commands.remove(&stamp)?;
        if &ticket.extension != extension
            || ticket.stamp != stamp
            || ticket.ticket.is_cancelled()
            || self.current_extension(extension, stamp.generation).is_none()
        {
            self.counters.rejected_tickets += 1;
            return None;
        }
        let effect = match result {
            RequestResult::Intent(CommandIntent::Message(text)) => PluginEffect::Message(text),
            RequestResult::Intent(CommandIntent::CopyText(text)) => PluginEffect::CopyText(text),
            RequestResult::Intent(CommandIntent::Reveal(handle)) => {
                let (path, identity) = ticket.entries.get(&handle)?.clone();
                PluginEffect::Reveal { path, identity: identity? }
            }
            RequestResult::Unit | RequestResult::Error(_) | RequestResult::Refresh => return None,
            RequestResult::Values(_) => return None,
        };
        Some((ticket.window, extension.clone(), effect, ticket.capture))
    }

    /// Cancel every outstanding preview request for one surface. A new preview
    /// binding or a deadline calls this so the old candidate chain stops.
    pub fn cancel_previews(&mut self, surface: SurfaceId) {
        self.tickets.retain(|_, ticket| {
            let keep = !matches!(&ticket.kind, TicketKind::Preview { surface: s, .. } if *s == surface);
            if !keep {
                ticket.ticket.cancel();
            }
            keep
        });
    }

    pub fn cancel_commands_for_surface(&mut self, surface: SurfaceId) {
        self.commands.retain(|_, command| {
            if command.capture.surface == surface {
                command.ticket.cancel();
                false
            } else {
                true
            }
        });
    }

    /// A terminal reply. Returns true when a ready value changed.
    pub fn finish(
        &mut self,
        extension: &ExtensionId,
        stamp: RequestStamp,
        result: RequestResult,
        now: Instant,
    ) -> bool {
        let Some(ticket) = self.tickets.remove(&stamp) else {
            self.counters.rejected_tickets += 1;
            return false;
        };
        if &ticket.extension != extension || ticket.stamp != stamp || ticket.ticket.is_cancelled() {
            self.counters.rejected_tickets += 1;
            return false;
        }
        if self.current_extension(extension, stamp.generation).is_none() {
            self.counters.rejected_tickets += 1;
            return false;
        }
        match (ticket.kind, result) {
            (TicketKind::Preview { sender, .. }, result) => {
                let _ = sender.send(result);
                false
            }
            (
                TicketKind::Sidebar { surface, scope, revision, descriptor, entries },
                result @ (RequestResult::Values(_) | RequestResult::Unit),
            ) => {
                let rows = match result {
                    RequestResult::Unit => Vec::new(),
                    RequestResult::Values(ContributionValues::Sidebar(rows)) => rows,
                    _ => {
                        self.counters.rejected_tickets += 1;
                        return false;
                    }
                };
                let rows: Vec<SectionRow> = rows
                    .into_iter()
                    .map(|row| {
                        let destination = row
                            .handle
                            .and_then(|handle| entries.get(&handle))
                            .map(|(path, identity)| SectionDestination::Folder {
                                path: path.clone(),
                                identity: *identity,
                            })
                            .unwrap_or(SectionDestination::DisplayOnly);
                        SectionRow { id: row.id, text: row.title, meta: row.meta, badge: row.badge, destination }
                    })
                    .collect();
                let Some(binding) = self.sidebar.get_mut(&surface) else { return false };
                if binding.scope != scope || binding.revision != revision {
                    self.counters.rejected_tickets += 1;
                    return false;
                }
                binding.seen.insert(descriptor.clone());
                let changed = binding.values.get(&descriptor) != Some(&rows);
                binding.values.insert(descriptor, rows);
                changed
            }
            (
                TicketKind::Info { surface, scope, revision, descriptor },
                result @ (RequestResult::Values(_) | RequestResult::Unit),
            ) => {
                let rows = match result {
                    RequestResult::Unit => Vec::new(),
                    RequestResult::Values(ContributionValues::Info(rows)) => rows,
                    _ => {
                        self.counters.rejected_tickets += 1;
                        return false;
                    }
                };
                let Some(binding) = self.info.get_mut(&surface) else { return false };
                if binding.scope != scope || binding.revision != revision {
                    self.counters.rejected_tickets += 1;
                    return false;
                }
                binding.seen.insert(descriptor.clone());
                let changed = binding.values.get(&descriptor) != Some(&rows);
                binding.values.insert(descriptor, rows);
                changed
            }
            (
                TicketKind::Status { surface, scope, revision, descriptor },
                result @ (RequestResult::Values(_) | RequestResult::Unit),
            ) => {
                let text = match result {
                    RequestResult::Unit => String::new(),
                    RequestResult::Values(ContributionValues::Status(items)) => {
                        items.into_iter().map(|item| item.text).find(|text| !text.is_empty()).unwrap_or_default()
                    }
                    _ => {
                        self.counters.rejected_tickets += 1;
                        return false;
                    }
                };
                let Some(binding) = self.status.get_mut(&surface) else { return false };
                if binding.scope != scope || binding.revision != revision {
                    self.counters.rejected_tickets += 1;
                    return false;
                }
                binding.seen.insert(descriptor.clone());
                if binding.values.get(&descriptor) == Some(&text) {
                    return false;
                }
                binding.values.insert(descriptor, text);
                true
            }
            (
                TicketKind::Rows { surface, scope, scope_version, descriptor, entries },
                result @ (RequestResult::Values(_) | RequestResult::Unit),
            ) => {
                let kind = self.kind_of(&descriptor);
                let by_handle = match result {
                    RequestResult::Unit => HashMap::new(),
                    RequestResult::Values(values) => {
                        let values = match values {
                            ContributionValues::Column(values) if kind == Some(ExtensionKind::Column) => values,
                            ContributionValues::Decoration(values) if kind == Some(ExtensionKind::Decoration) => values,
                            _ => {
                                self.counters.rejected_tickets += 1;
                                return false;
                            }
                        };
                        values.into_iter().map(|value| (value.handle, value)).collect()
                    }
                    _ => unreachable!(),
                };
                let Some(state) = self.surfaces.get_mut(&surface) else { return false };
                if state.scope != scope || state.scope_version != scope_version {
                    self.counters.rejected_tickets += 1;
                    return false;
                }
                state.seen = now;
                let generation = stamp.generation;
                let mut changed = false;
                for (handle, path, version) in entries {
                    if state.versions.get(&path).copied() != Some(version) {
                        continue;
                    }
                    let key = RowKey { extension: descriptor.clone(), generation, scope: scope.clone(), path, version };
                    let value = by_handle.get(&handle).map_or_else(RowValue::empty, |value| RowValue {
                        text: value.text.clone(),
                        spoken: value.spoken.clone(),
                        tint: value.tint,
                        dim: value.dim,
                    });
                    if self.rows.peek(&key) != Some(&value) {
                        self.rows.insert(key, value);
                        changed = true;
                    }
                }
                self.counters.cache_high_water = self.counters.cache_high_water.max(self.rows.len());
                changed
            }
            (TicketKind::Hook { scope }, reply) => {
                if matches!(reply, RequestResult::Refresh) {
                    match scope {
                        Some(scope) => self.rows.remove_scope(&scope),
                        None => self.rows.remove_extension(extension),
                    }
                    return true;
                }
                false
            }
            _ => {
                self.counters.rejected_tickets += 1;
                false
            }
        }
    }

    fn kind_of(&self, extension: &ExtensionId) -> Option<ExtensionKind> {
        self.snapshot.extension(extension).map(|published| published.descriptor.kind)
    }

    /// Ready value for one row; missing values are cached as empty rows.
    pub fn row_value(
        &mut self,
        extension: &ExtensionId,
        scope: &ScopeKey,
        path: &Path,
        version: u64,
    ) -> Option<RowValue> {
        let generation = self.snapshot.extension(extension)?.generation;
        let key = RowKey {
            extension: extension.clone(),
            generation,
            scope: scope.clone(),
            path: path.to_path_buf(),
            version,
        };
        self.rows.get(&key)
    }

    pub fn has_row(&self, extension: &ExtensionId, scope: &ScopeKey, path: &Path, version: u64) -> bool {
        let Some(published) = self.snapshot.extension(extension) else { return false };
        self.rows
            .peek(&RowKey {
                extension: extension.clone(),
                generation: published.generation,
                scope: scope.clone(),
                path: path.to_path_buf(),
                version,
            })
            .is_some()
    }

    pub fn invalidate_path(&mut self, scope: &ScopeKey, path: &Path) {
        self.rows.remove_path(scope, path);
        for state in self.surfaces.values_mut() {
            if &state.scope == scope {
                state.versions.remove(path);
            }
        }
    }

    /// A provider snapshot or line count changed. Drop this plugin's ready
    /// values and let the next visible pass ask again.
    pub fn invalidate_plugin(&mut self, plugin: &PluginId) {
        self.rows.remove_plugin(plugin);
        for binding in self.status.values_mut() {
            binding.values.retain(|id, _| &id.plugin != plugin);
            binding.seen.retain(|id| &id.plugin != plugin);
        }
        for binding in self.info.values_mut() {
            binding.values.retain(|id, _| &id.plugin != plugin);
            binding.seen.retain(|id| &id.plugin != plugin);
        }
        for binding in self.sidebar.values_mut() {
            binding.values.retain(|id, _| &id.plugin != plugin);
            binding.seen.retain(|id| &id.plugin != plugin);
        }
    }

    pub fn invalidate_path_everywhere(&mut self, path: &Path) {
        self.rows.remove_path_any(path);
    }

    pub fn bump_scope(&mut self, surface: SurfaceId, scope: ScopeKey, scope_version: u64, _now: Instant) {
        self.rows.remove_scope(&scope);
        if let Some(state) = self.surfaces.get_mut(&surface)
            && state.scope == scope
        {
            state.scope_version = scope_version;
            state.versions.clear();
        }
    }

    pub fn drop_surface(&mut self, surface: SurfaceId) {
        self.surfaces.remove(&surface);
        self.pending_rows.remove(&surface);
        self.status.remove(&surface);
        self.info.remove(&surface);
        self.sidebar.remove(&surface);
        self.cancel_commands_for_surface(surface);
        self.tickets.retain(|_, ticket| {
            let keep = !matches!(
                &ticket.kind,
                TicketKind::Rows { surface: s, .. }
                    | TicketKind::Status { surface: s, .. }
                    | TicketKind::Info { surface: s, .. }
                    | TicketKind::Sidebar { surface: s, .. }
                    | TicketKind::Preview { surface: s, .. } if *s == surface
            );
            if !keep {
                ticket.ticket.cancel();
            }
            keep
        });
    }

    pub fn notify_scope_changed(&mut self, surface: SurfaceId, scope: String, now: Instant) {
        self.queue_hook(HookSlot::Scope(surface), HookEvent::ScopeChanged { scope: scope.clone() }, None, now);
        self.queue_hook(HookSlot::Selection(surface), HookEvent::SelectionChanged { scope }, None, now);
    }

    pub fn notify_selection_changed(&mut self, surface: SurfaceId, scope: String, now: Instant) {
        self.queue_hook(HookSlot::Selection(surface), HookEvent::SelectionChanged { scope }, None, now);
    }

    pub fn notify_files_changed(&mut self, scope: ScopeKey, display: String, now: Instant) {
        self.invalidate_scope(&scope);
        self.queue_hook(
            HookSlot::Files(scope.path().clone()),
            HookEvent::FilesChanged { scope: display },
            Some(scope),
            now,
        );
    }

    pub fn notify_operation_done(&mut self, now: Instant) {
        self.queue_hook(HookSlot::Operation, HookEvent::OperationDone, None, now);
    }

    fn queue_hook(&mut self, slot: HookSlot, event: HookEvent, scope: Option<ScopeKey>, now: Instant) {
        self.hooks.insert(slot, PendingHook { due: now + HOOK_DEBOUNCE, event, scope });
    }

    fn invalidate_scope(&mut self, scope: &ScopeKey) {
        self.rows.remove_scope(scope);
        for state in self.surfaces.values_mut() {
            if &state.scope == scope {
                state.versions.clear();
            }
        }
    }

    /// Send due hooks. Runs from the 100 ms app poll, so one poll interval is
    /// the debounce window; a Busy lane retries once on the next tick.
    pub fn flush_hooks(&mut self, handle: Option<&PluginHandle>, now: Instant) -> bool {
        let Some(handle) = handle else {
            self.hooks.clear();
            return false;
        };
        if self.retry_at.is_some_and(|due| now < due) {
            return false;
        }
        let due: Vec<HookSlot> =
            self.hooks.iter().filter(|(_, hook)| hook.due <= now).map(|(slot, _)| slot.clone()).collect();
        if due.is_empty() {
            return false;
        }
        let hooks: Vec<&Descriptor> =
            self.snapshot.of_kind(ExtensionKind::Hook).map(|(_, extension)| &extension.descriptor).collect();
        let mut sent = false;
        for slot in due {
            let Some((event, event_scope)) = self.hooks.get(&slot).map(|hook| (hook.event.clone(), hook.scope.clone()))
            else {
                continue;
            };
            let mut busy = false;
            let mut slot_sent = false;
            for descriptor in &hooks {
                if self.suppressed.contains(&descriptor.id.plugin) {
                    continue;
                }
                if !self.can_submit(&descriptor.id) {
                    busy = true;
                    continue;
                }
                let generation = self.snapshot.extension(&descriptor.id).map_or(0, |published| published.generation);
                let revision = stamp_revision(event.scope(), &slot);
                match handle.try_notify(descriptor.id.clone(), generation, revision, event.clone()) {
                    Ok(ticket) => {
                        self.counters.submitted_hooks += 1;
                        self.tickets.insert(
                            ticket.stamp(),
                            PassiveTicket {
                                stamp: ticket.stamp(),
                                ticket: TicketProbe::Real(ticket),
                                extension: descriptor.id.clone(),
                                kind: TicketKind::Hook { scope: event_scope.clone() },
                            },
                        );
                        sent = true;
                        slot_sent = true;
                    }
                    Err(SubmitError::Busy) => busy = true,
                    Err(SubmitError::Stopped) => {
                        self.hooks.clear();
                        return sent;
                    }
                }
            }
            if busy && !slot_sent {
                if let Some(hook) = self.hooks.get_mut(&slot) {
                    hook.due = now + HOOK_DEBOUNCE;
                }
                self.retry_at = Some(now + HOOK_DEBOUNCE);
                self.counters.coalesced_hooks += 1;
            } else {
                self.hooks.remove(&slot);
            }
        }
        sent
    }

    #[cfg(test)]
    pub fn queue_hook_for_test(&mut self, event: HookEvent, now: Instant) {
        self.queue_hook(HookSlot::Operation, event, None, now);
    }

    #[cfg(test)]
    pub fn track_ticket_for_test(
        &mut self,
        extension: ExtensionId,
        stamp: RequestStamp,
        cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
        scope: ScopeKey,
    ) {
        self.tickets.insert(
            stamp,
            PassiveTicket {
                stamp,
                ticket: TicketProbe::Flag(cancelled),
                extension,
                kind: TicketKind::Hook { scope: Some(scope) },
            },
        );
    }

    #[cfg(test)]
    pub fn publish_rows_for_test(&mut self, published: PublishedSurfaces) {
        self.snapshot = std::sync::Arc::new(published);
    }

    #[cfg(test)]
    pub fn set_sidebar_rows_for_test(&mut self, surface: SurfaceId, descriptor: ExtensionId, rows: Vec<SectionRow>) {
        let binding = self.sidebar.entry(surface).or_insert_with(|| SidebarBinding {
            scope: ScopeKey::Folder(PathBuf::new()),
            revision: 0,
            values: BTreeMap::new(),
            seen: HashSet::new(),
        });
        binding.values.insert(descriptor, rows);
    }

    #[cfg(test)]
    pub fn queued_scope_hooks(&self) -> usize {
        self.hooks.keys().filter(|slot| matches!(slot, HookSlot::Scope(_))).count()
    }

    #[cfg(test)]
    pub fn clear_hooks_for_test(&mut self) {
        self.hooks.clear();
    }

    #[cfg(test)]
    pub fn track_surface_for_test(&mut self, surface: SurfaceId) {
        self.surfaces.entry(surface).or_insert_with(|| SurfaceState {
            scope: ScopeKey::Folder(PathBuf::new()),
            scope_version: 0,
            versions: HashMap::new(),
            seen: Instant::now(),
        });
    }

    #[cfg(test)]
    pub fn has_tracked_surface_for_test(&self, surface: SurfaceId) -> bool {
        self.surfaces.contains_key(&surface)
            || self.status.contains_key(&surface)
            || self.info.contains_key(&surface)
            || self.sidebar.contains_key(&surface)
    }

    #[cfg(test)]
    pub fn tracked_surface_count(&self) -> usize {
        let mut ids: HashSet<SurfaceId> = self.surfaces.keys().copied().collect();
        ids.extend(self.status.keys().copied());
        ids.extend(self.info.keys().copied());
        ids.extend(self.sidebar.keys().copied());
        ids.len()
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn track_rows_for_test(
        &mut self,
        extension: ExtensionId,
        stamp: RequestStamp,
        cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
        surface: SurfaceId,
        scope: ScopeKey,
        scope_version: u64,
        entries: Vec<(EntryHandle, PathBuf, u64)>,
    ) {
        self.tickets.insert(
            stamp,
            PassiveTicket {
                stamp,
                ticket: TicketProbe::Flag(cancelled),
                extension: extension.clone(),
                kind: TicketKind::Rows { surface, scope, scope_version, descriptor: extension, entries },
            },
        );
    }

    #[cfg(test)]
    pub fn pending_hooks(&self) -> usize {
        self.hooks.len()
    }

    #[cfg(test)]
    pub fn track_command_for_test(
        &mut self,
        stamp: RequestStamp,
        extension: ExtensionId,
        capture: CommandCapture,
        entries: HashMap<EntryHandle, (PathBuf, Option<(u64, u64)>)>,
        cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        self.commands.insert(
            stamp,
            CommandTicket {
                stamp,
                ticket: TicketProbe::Flag(cancelled),
                extension,
                window: gpui::WindowId::from(1),
                capture,
                entries,
            },
        );
    }
}

pub const MAX_ROW_VALUES: usize = 4_096;
pub const MAX_ROW_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PASSIVE_PER_PLUGIN: usize = 2;
pub const MAX_PASSIVE_REQUESTS: usize = 16;
pub const MAX_BATCH: usize = 128;
pub const HOOK_DEBOUNCE: Duration = Duration::from_millis(100);

fn bounded(text: &str) -> String {
    let text = crate::style::display_text(text);
    let mut end = text.len().min(crow_plugin::Limits::default().max_display_bytes);
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

fn stamp_revision(scope: Option<&str>, slot: &HookSlot) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    scope.unwrap_or("").hash(&mut hasher);
    slot.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use crow_core::{DescriptorSource, EntryValue, ExtensionKind, LocalId};
    use crow_plugin::{FsAccess, PluginEntry};

    use super::*;

    fn plugin() -> PluginId {
        PluginId::parse("example.test").unwrap()
    }

    fn descriptor(kind: ExtensionKind, local: &str) -> Descriptor {
        Descriptor::new(
            kind,
            ExtensionId::new(plugin(), LocalId::parse(local).unwrap()),
            "X".into(),
            DescriptorSource::Function("run".into()),
        )
    }

    fn entry(generation: u64, descriptors: Vec<Descriptor>, state: PluginState) -> PluginEntry {
        PluginEntry {
            bundle: OsString::from("example.test"),
            dir_name: "example.test".into(),
            id: Some(plugin()),
            name: "Example".into(),
            version: Some("1".into()),
            provides: descriptors.iter().map(|descriptor| descriptor.kind).collect(),
            state,
            detail: None,
            digest: Some("a".repeat(64)),
            requested_fs: FsAccess::None,
            read_allowed: false,
            descriptors,
            icon_packs: vec![],
            generation,
            bundled: false,
        }
    }

    fn inventory(entries: Vec<PluginEntry>) -> Inventory {
        Inventory { plugins: entries, grants_error: None, scan_error: None }
    }

    fn running(generation: u64, descriptors: Vec<Descriptor>) -> Inventory {
        inventory(vec![entry(generation, descriptors, PluginState::Running)])
    }

    fn stamp(generation: u64, request_id: u64) -> RequestStamp {
        RequestStamp { generation, request_id, context_revision: 3 }
    }

    fn value(handle: u32, text: &str) -> EntryValue {
        EntryValue { handle: EntryHandle(handle), text: text.into(), spoken: None, tint: None, dim: false }
    }

    #[allow(clippy::too_many_arguments)]
    fn start_row(
        surfaces: &mut PluginSurfaces,
        inventory: Inventory,
        column: &Descriptor,
        surface: SurfaceId,
        scope: &ScopeKey,
        path: &Path,
        version: u64,
        stamp: RequestStamp,
    ) {
        surfaces.publish_inventory(&inventory);
        surfaces.observe_rows(
            None,
            surface,
            scope.clone(),
            9,
            "/tmp",
            3,
            &[],
            vec![RowInput {
                path: path.to_path_buf(),
                version,
                name: "a.txt".into(),
                is_dir: false,
                size: 1,
                kind: "Text".into(),
                selected: false,
                git: None,
            }],
            None,
            Instant::now(),
        );
        surfaces.track_rows_for_test(
            column.id.clone(),
            stamp,
            Arc::new(AtomicBool::new(false)),
            surface,
            scope.clone(),
            9,
            vec![(EntryHandle(1), path.to_path_buf(), version)],
        );
    }

    #[test]
    fn identical_inventory_keeps_ready_values_and_a_changed_generation_drops_them() {
        let column = descriptor(ExtensionKind::Column, "lines");
        let scope = ScopeKey::Folder(PathBuf::from("/tmp"));
        let path = PathBuf::from("/tmp/a.txt");
        let surface = new_surface();
        let mut surfaces = PluginSurfaces::default();
        start_row(&mut surfaces, running(5, vec![column.clone()]), &column, surface, &scope, &path, 1, stamp(5, 1));
        assert!(surfaces.finish(
            &column.id,
            stamp(5, 1),
            RequestResult::Values(ContributionValues::Column(vec![value(1, "3")])),
            Instant::now(),
        ));
        assert_eq!(surfaces.row_value(&column.id, &scope, &path, 1).unwrap().text, "3");

        surfaces.publish_inventory(&running(5, vec![column.clone()]));
        assert_eq!(surfaces.row_value(&column.id, &scope, &path, 1).unwrap().text, "3");

        surfaces.publish_inventory(&running(6, vec![column.clone()]));
        assert!(surfaces.row_value(&column.id, &scope, &path, 1).is_none());
        assert!(!surfaces.finish(
            &column.id,
            stamp(5, 1),
            RequestResult::Values(ContributionValues::Column(vec![value(1, "9")])),
            Instant::now(),
        ));
    }

    #[test]
    fn cancelled_duplicate_and_stale_scope_replies_are_ignored() {
        let column = descriptor(ExtensionKind::Column, "lines");
        let scope = ScopeKey::Folder(PathBuf::from("/tmp"));
        let path = PathBuf::from("/tmp/a.txt");
        let surface = new_surface();
        let mut surfaces = PluginSurfaces::default();
        let cancelled = Arc::new(AtomicBool::new(true));
        start_row(&mut surfaces, running(5, vec![column.clone()]), &column, surface, &scope, &path, 1, stamp(5, 1));
        surfaces.track_rows_for_test(
            column.id.clone(),
            stamp(5, 2),
            cancelled,
            surface,
            scope.clone(),
            9,
            vec![(EntryHandle(1), path.clone(), 1)],
        );
        assert!(!surfaces.finish(
            &column.id,
            stamp(5, 2),
            RequestResult::Values(ContributionValues::Column(vec![value(1, "cancelled")])),
            Instant::now(),
        ));
        assert!(surfaces.finish(
            &column.id,
            stamp(5, 1),
            RequestResult::Values(ContributionValues::Column(vec![value(1, "ok")])),
            Instant::now(),
        ));
        assert!(!surfaces.finish(
            &column.id,
            stamp(5, 1),
            RequestResult::Values(ContributionValues::Column(vec![value(1, "dup")])),
            Instant::now(),
        ));
        assert_eq!(surfaces.row_value(&column.id, &scope, &path, 1).unwrap().text, "ok");

        surfaces.track_rows_for_test(
            column.id.clone(),
            stamp(5, 3),
            Arc::new(AtomicBool::new(false)),
            surface,
            scope.clone(),
            9,
            vec![(EntryHandle(1), path.clone(), 1)],
        );
        surfaces.observe_rows(
            None,
            surface,
            ScopeKey::Folder(PathBuf::from("/other")),
            10,
            "/other",
            4,
            &[],
            vec![],
            None,
            Instant::now(),
        );
        assert!(!surfaces.finish(
            &column.id,
            stamp(5, 3),
            RequestResult::Values(ContributionValues::Column(vec![value(1, "stale")])),
            Instant::now(),
        ));
    }

    #[test]
    fn a_same_path_replacement_invalidates_the_old_reply() {
        let column = descriptor(ExtensionKind::Column, "lines");
        let scope = ScopeKey::Folder(PathBuf::from("/tmp"));
        let path = PathBuf::from("/tmp/a.txt");
        let surface = new_surface();
        let mut surfaces = PluginSurfaces::default();
        start_row(&mut surfaces, running(5, vec![column.clone()]), &column, surface, &scope, &path, 1, stamp(5, 1));
        surfaces.track_rows_for_test(
            column.id.clone(),
            stamp(5, 2),
            Arc::new(AtomicBool::new(false)),
            surface,
            scope.clone(),
            9,
            vec![(EntryHandle(1), path.clone(), 2)],
        );
        surfaces.invalidate_path(&scope, &path);
        assert!(!surfaces.finish(
            &column.id,
            stamp(5, 2),
            RequestResult::Values(ContributionValues::Column(vec![value(1, "new")])),
            Instant::now(),
        ));
    }

    #[test]
    fn disable_suppression_survives_an_unrelated_inventory_and_clears_on_the_host_completion() {
        let column = descriptor(ExtensionKind::Column, "lines");
        let scope = ScopeKey::Folder(PathBuf::from("/tmp"));
        let path = PathBuf::from("/tmp/a.txt");
        let surface = new_surface();
        let mut surfaces = PluginSurfaces::default();
        let enabled = running(5, vec![column.clone()]);
        start_row(&mut surfaces, enabled.clone(), &column, surface, &scope, &path, 1, stamp(5, 1));
        assert!(surfaces.finish(
            &column.id,
            stamp(5, 1),
            RequestResult::Values(ContributionValues::Column(vec![value(1, "3")])),
            Instant::now(),
        ));
        surfaces.suppress_disable(&plugin());
        assert!(surfaces.row_value(&column.id, &scope, &path, 1).is_none());
        surfaces.publish_inventory(&enabled);
        assert!(surfaces.is_suppressed(&plugin()), "another inventory must not clear the disable");
        assert!(surfaces.mutation_finished(&plugin(), HostMutation::Disable, &Err("denied".into())));

        surfaces.publish_inventory(&running(5, vec![column.clone()]));
        surfaces.suppress_disable(&plugin());
        surfaces.publish_inventory(&inventory(vec![entry(5, vec![column.clone()], PluginState::Disabled)]));
        assert!(!surfaces.mutation_finished(&plugin(), HostMutation::Disable, &Ok(())));
        assert!(!surfaces.is_suppressed(&plugin()));
        assert!(surfaces.snapshot().extension(&column.id).is_none());
    }

    #[test]
    fn disable_suppression_bumps_the_view_generation_for_columns_and_previews() {
        let column = descriptor(ExtensionKind::Column, "lines");
        let mut surfaces = PluginSurfaces::default();
        surfaces.publish_inventory(&running(5, vec![column]));
        let before = surfaces.generation();
        surfaces.suppress_disable(&plugin());
        let suppressed = surfaces.generation();
        assert!(suppressed > before, "views must re-resolve when Disable is queued");
        surfaces.mutation_finished(&plugin(), HostMutation::Disable, &Ok(()));
        assert!(surfaces.generation() > suppressed, "views must re-resolve when the host completes");
    }

    #[test]
    fn owner_close_and_drop_clear_pending_tickets() {
        let column = descriptor(ExtensionKind::Column, "lines");
        let scope = ScopeKey::Folder(PathBuf::from("/tmp"));
        let surface = new_surface();
        let mut surfaces = PluginSurfaces::default();
        surfaces.publish_inventory(&running(5, vec![column.clone()]));
        surfaces.track_rows_for_test(
            column.id.clone(),
            stamp(5, 1),
            Arc::new(AtomicBool::new(false)),
            surface,
            scope.clone(),
            9,
            vec![],
        );
        assert_eq!(surfaces.pending_tickets(), 1);
        surfaces.drop_surface(surface);
        assert_eq!(surfaces.pending_tickets(), 0);
    }

    #[test]
    fn repeated_demand_coalesces_and_missing_service_does_not_queue() {
        let column = descriptor(ExtensionKind::Column, "lines");
        let scope = ScopeKey::Folder(PathBuf::from("/tmp"));
        let surface = new_surface();
        let mut surfaces = PluginSurfaces::default();
        surfaces.publish_inventory(&running(5, vec![column.clone()]));
        let rows = vec![RowInput {
            path: PathBuf::from("/tmp/a.txt"),
            version: 1,
            name: "a.txt".into(),
            is_dir: false,
            size: 1,
            kind: "Text".into(),
            selected: false,
            git: None,
        }];
        surfaces.observe_rows(
            None,
            surface,
            scope.clone(),
            9,
            "/tmp",
            3,
            std::slice::from_ref(&column.id),
            rows,
            None,
            Instant::now(),
        );
        assert_eq!(surfaces.pending_tickets(), 0, "no service handle means no submission");
        assert_eq!(surfaces.counters().submitted_rows, 0);
    }

    #[test]
    fn hook_signals_coalesce_per_owner_and_operation() {
        let mut surfaces = PluginSurfaces::default();
        let now = Instant::now();
        let surface = new_surface();
        surfaces.notify_scope_changed(surface, "/tmp".into(), now);
        surfaces.notify_scope_changed(surface, "/tmp".into(), now);
        surfaces.notify_selection_changed(surface, "/tmp".into(), now);
        surfaces.notify_operation_done(now);
        surfaces.notify_operation_done(now);
        assert_eq!(surfaces.pending_hooks(), 3);
        assert!(!surfaces.flush_hooks(None, now + HOOK_DEBOUNCE));
        assert_eq!(surfaces.pending_hooks(), 0, "a closed service clears pending hooks");
    }

    fn command_capture(scope: &ScopeKey, path: &Path) -> CommandCapture {
        CommandCapture { surface: new_surface(), scope: scope.clone(), selection: vec![(path.to_path_buf(), false)] }
    }

    #[test]
    fn command_effects_resolve_handles_and_drop_stale_generations() {
        let command = descriptor(ExtensionKind::Command, "relative");
        let scope = ScopeKey::Folder(PathBuf::from("/tmp"));
        let path = PathBuf::from("/tmp/a.txt");
        let capture = command_capture(&scope, &path);
        let mut surfaces = PluginSurfaces::default();
        surfaces.publish_inventory(&running(5, vec![command.clone()]));
        let mut entries = HashMap::new();
        entries.insert(EntryHandle(1), (path.clone(), Some((3, 4))));
        entries.insert(EntryHandle(2), (PathBuf::from("/tmp/b.txt"), None));

        surfaces.track_command_for_test(
            stamp(5, 1),
            command.id.clone(),
            capture.clone(),
            entries.clone(),
            Arc::new(AtomicBool::new(false)),
        );
        let (window, _, effect, got) = surfaces
            .finish_command(&command.id, stamp(5, 1), RequestResult::Intent(CommandIntent::Reveal(EntryHandle(1))))
            .unwrap();
        assert_eq!(window, gpui::WindowId::from(1));
        assert_eq!(effect, PluginEffect::Reveal { path: path.clone(), identity: (3, 4) });
        assert_eq!(got, capture);

        surfaces.track_command_for_test(
            stamp(5, 2),
            command.id.clone(),
            capture.clone(),
            entries.clone(),
            Arc::new(AtomicBool::new(true)),
        );
        assert!(
            surfaces
                .finish_command(&command.id, stamp(5, 2), RequestResult::Intent(CommandIntent::Reveal(EntryHandle(1))))
                .is_none()
        );

        surfaces.track_command_for_test(
            stamp(5, 3),
            command.id.clone(),
            capture.clone(),
            entries.clone(),
            Arc::new(AtomicBool::new(false)),
        );
        surfaces.publish_inventory(&running(6, vec![command.clone()]));
        assert!(
            surfaces
                .finish_command(&command.id, stamp(5, 3), RequestResult::Intent(CommandIntent::Message("x".into())))
                .is_none()
        );

        surfaces.publish_inventory(&running(5, vec![command.clone()]));
        surfaces.track_command_for_test(
            stamp(5, 4),
            command.id.clone(),
            capture.clone(),
            entries.clone(),
            Arc::new(AtomicBool::new(false)),
        );
        assert!(
            surfaces
                .finish_command(&command.id, stamp(5, 4), RequestResult::Intent(CommandIntent::Reveal(EntryHandle(9))))
                .is_none()
        );
        assert!(
            surfaces
                .finish_command(&command.id, stamp(5, 4), RequestResult::Intent(CommandIntent::Message("dup".into())))
                .is_none()
        );
    }

    #[test]
    fn refresh_invalidates_only_its_captured_scope() {
        let column = descriptor(ExtensionKind::Column, "lines");
        let hook = descriptor(ExtensionKind::Hook, "hook");
        let scope = ScopeKey::Folder(PathBuf::from("/tmp"));
        let path = PathBuf::from("/tmp/a.txt");
        let surface = new_surface();
        let mut surfaces = PluginSurfaces::default();
        start_row(&mut surfaces, running(5, vec![column.clone()]), &column, surface, &scope, &path, 1, stamp(5, 1));
        surfaces.publish_inventory(&inventory(vec![entry(
            5,
            vec![column.clone(), hook.clone()],
            PluginState::Running,
        )]));
        assert!(surfaces.finish(
            &column.id,
            stamp(5, 1),
            RequestResult::Values(ContributionValues::Column(vec![value(1, "3")])),
            Instant::now(),
        ));
        surfaces.track_ticket_for_test(hook.id.clone(), stamp(5, 2), Arc::new(AtomicBool::new(false)), scope.clone());
        assert!(surfaces.finish(&hook.id, stamp(5, 2), RequestResult::Refresh, Instant::now()));
        assert!(surfaces.row_value(&column.id, &scope, &path, 1).is_none());
    }

    #[test]
    fn a_visible_row_survives_cache_pressure() {
        let extension = descriptor(ExtensionKind::Column, "lines").id;
        let scope = ScopeKey::Folder(PathBuf::from("/tmp"));
        let key = |i: usize| RowKey {
            extension: extension.clone(),
            generation: 1,
            scope: scope.clone(),
            path: PathBuf::from(format!("/tmp/{i}")),
            version: 1,
        };
        let value = || RowValue { text: "x".into(), spoken: None, tint: None, dim: false };
        let mut cache = RowCache::default();
        for i in 0..MAX_ROW_VALUES - 1 {
            cache.insert(key(i), value());
        }
        cache.touch(&key(0));
        for i in MAX_ROW_VALUES - 1..MAX_ROW_VALUES + 2 {
            cache.insert(key(i), value());
        }
        assert!(cache.peek(&key(0)).is_some(), "the visible row is recent");
        assert!(cache.peek(&key(1)).is_none(), "the cold row is evicted first");
    }
}
