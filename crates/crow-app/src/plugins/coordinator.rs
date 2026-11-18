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
    ContributionValues, Descriptor, EntryData, EntryHandle, ExtensionId, ExtensionKind, HookEvent, PluginId, Snapshot,
    Tint,
};
use crow_plugin::{
    HostMutation, Inventory, PluginHandle, PluginState, RegisteredIconPack, RequestInput, RequestResult, RequestStamp,
    RequestTicket, SubmitError,
};

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

    fn insert(&mut self, key: RowKey, value: RowValue) {
        if let Some(old) = self.values.remove(&key) {
            self.bytes = self.bytes.saturating_sub(old.value.bytes());
        }
        self.tick = self.tick.wrapping_add(1);
        self.bytes = self.bytes.saturating_add(value.bytes());
        self.values.insert(key, CachedRow { value, touch: self.tick });
        self.evict();
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

enum TicketKind {
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
}

/// The coordinator. One instance lives behind the app's `Plugins` global.
pub struct PluginSurfaces {
    snapshot: std::sync::Arc<PublishedSurfaces>,
    rows: RowCache,
    surfaces: HashMap<SurfaceId, SurfaceState>,
    pending_rows: HashMap<SurfaceId, PendingRows>,
    tickets: HashMap<RequestStamp, PassiveTicket>,
    hooks: HashMap<HookSlot, PendingHook>,
    suppressed: HashSet<PluginId>,
    retry_at: Option<Instant>,
    counters: PluginCounters,
}

impl Default for PluginSurfaces {
    fn default() -> Self {
        Self {
            snapshot: std::sync::Arc::new(PublishedSurfaces::default()),
            rows: RowCache::default(),
            surfaces: HashMap::new(),
            pending_rows: HashMap::new(),
            tickets: HashMap::new(),
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
    pub fn publish_inventory(&mut self, inventory: &Inventory) {
        let mut next = PublishedSurfaces::default();
        for entry in inventory.plugins.iter().filter(|entry| entry.state == PluginState::Running) {
            for descriptor in &entry.descriptors {
                next.extensions.insert(
                    descriptor.id.clone(),
                    PublishedExtension { generation: entry.generation, descriptor: descriptor.clone() },
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
            && self
                .snapshot
                .extensions
                .iter()
                .all(|(id, old)| next.extensions.get(id).is_some_and(|new| new.descriptor == old.descriptor));
        let same_packs = next.packs.len() == self.snapshot.packs.len()
            && self
                .snapshot
                .packs
                .iter()
                .all(|(id, old)| next.packs.get(id).is_some_and(|new| new.generation() == old.generation()));
        if same_extensions && same_packs {
            // Identical approved reload: keep the generation and every ready value.
            return;
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
        }
        self.suppressed.retain(|plugin| {
            inventory.plugins.iter().any(|entry| {
                entry.id.as_ref() == Some(plugin) && matches!(entry.state, PluginState::Starting | PluginState::Running)
            })
        });
        self.snapshot = std::sync::Arc::new(next);
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
        self.suppressed.remove(plugin);
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
    ) {
        self.pending_rows.insert(
            surface,
            PendingRows { scope, scope_version, scope_display, context_revision, descriptors, rows },
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
        if rows.is_empty() || descriptors.is_empty() {
            return;
        }
        let Some(handle) = handle else { return };
        let mut rows = rows;
        rows.sort_by_key(|row| !row.selected);
        let mut seen = HashSet::new();
        rows.retain(|row| seen.insert(row.path.clone()));
        rows.truncate(MAX_BATCH);
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
            let mut entries = Vec::with_capacity(rows.len());
            let mut refs = Vec::with_capacity(rows.len());
            let mut selection_handle = None;
            for (index, row) in rows.iter().enumerate() {
                let handle = EntryHandle(index as u32 + 1);
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
                refs.push((handle, row.path.clone(), row.version));
            }
            let input = RequestInput {
                snapshot: Snapshot { scope: bounded(scope_display), selection: selection_handle, entries },
                excerpt: None,
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
        self.tickets.retain(|_, ticket| {
            let keep = !matches!(&ticket.kind, TicketKind::Rows { surface: s, .. } if *s == surface);
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
            descriptors,
            icon_packs: vec![],
            generation,
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
            }],
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
}
