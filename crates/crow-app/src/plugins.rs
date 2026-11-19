//! The app's plugin service: one background host, its inventory global, and the
//! main-thread drain that moves worker replies onto the UI. No disk I/O and no
//! waiting for a plugin happens on the UI thread.

use std::hash::Hash;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::Instant;

use crow_core::{CommandTarget, SectionRow};
use crow_plugin::{HostEvent, HostMutation, Inventory, Limits, PluginHandle, PluginService};
use gpui::{App, AppContext as _, Global, Window, WindowId};

use crate::actions::PluginCommand;

use crate::ui::Ui;

mod columns;
mod coordinator;
mod demand;
pub use columns::PluginColumn;
#[allow(unused_imports)]
pub use coordinator::{
    CommandCapture, HOOK_DEBOUNCE, MAX_BATCH, MAX_PASSIVE_PER_PLUGIN, MAX_PASSIVE_REQUESTS, MAX_ROW_BYTES,
    MAX_ROW_VALUES, PluginCounters, PluginEffect, PluginSurfaces, PublishedExtension, PublishedSurfaces, RowInput,
    RowValue, ScopeKey, SidebarDir, SurfaceId, new_surface,
};
pub use demand::PluginRows;
/// The file surface the menu/palette would act on. Views update it only while
/// their own focus is live; a text field, sidebar, sheet or another window has
/// no plugin command target.
#[derive(Clone, Debug)]
pub struct FileCommandContext {
    pub window: WindowId,
    pub surface: SurfaceId,
    pub scope: ScopeKey,
    /// Selected rows in display order, with `is_dir`.
    pub selection: Vec<(PathBuf, bool)>,
}

#[derive(Default)]
pub struct Plugins {
    pub inventory: Inventory,
    pub handle: Option<PluginHandle>,
    pub surfaces: PluginSurfaces,
    pub command_context: Option<FileCommandContext>,
    command_preparations: usize,
    events: Option<Receiver<HostEvent>>,
    _service: Option<PluginService>,
}

impl Global for Plugins {}

#[cfg(test)]
impl Plugins {}

/// Start the service for the real config dir. Called once at app startup.
pub fn init(cx: &mut App) {
    let config_dir = cx.global::<Ui>().config_dir.clone();
    start(cx, config_dir);
}

/// A global with no service, for tests that only render the inventory.
#[cfg(test)]
pub fn init_empty(cx: &mut App) {
    cx.set_global(Plugins::default());
}

pub fn git_package() -> Result<std::sync::Arc<crow_plugin::package::PackageSnapshot>, String> {
    crow_plugin::package::embedded(
        &[
            ("plugin.toml", include_str!("../assets/plugins/git/plugin.toml")),
            ("main.rhai", include_str!("../assets/plugins/git/main.rhai")),
        ],
        &Limits::default(),
    )
    .map_err(|error| format!("{:?}: {}", error.kind, error.message))
}

/// The bundled Git package is Running in its approved generation.
pub fn bundled_git_running(cx: &App) -> bool {
    let Some(plugins) = cx.try_global::<Plugins>() else { return false };
    plugins.inventory.plugins.iter().any(|entry| {
        entry.bundled
            && entry.state == crow_plugin::PluginState::Running
            && entry.id.as_ref().is_some_and(|id| id.as_str() == crate::git::PLUGIN_ID)
    })
}

pub fn invalidate_git_lines(cx: &mut App, paths: &[PathBuf]) {
    if cx.try_global::<Plugins>().is_some() {
        cx.global_mut::<Plugins>().surfaces.invalidate_git_lines(paths);
    }
}

pub fn invalidate_git(cx: &mut App) {
    if cx.try_global::<Plugins>().is_some()
        && let Ok(id) = crow_core::PluginId::parse(crate::git::PLUGIN_ID)
    {
        cx.global_mut::<Plugins>().surfaces.invalidate_plugin(&id);
    }
}

pub fn refresh_git(cx: &mut App, refresh: crate::git::PluginRefresh, folder_matches: impl Fn(&Path) -> bool) {
    if cx.try_global::<Plugins>().is_some() {
        cx.global_mut::<Plugins>().surfaces.refresh_git(refresh, folder_matches);
    }
}

pub fn start(cx: &mut App, config_dir: PathBuf) {
    start_inner(cx, config_dir, true);
}

/// Tests that pin an exact inventory use only the fixtures they installed.
#[cfg(test)]
pub fn start_for_tests(cx: &mut App, config_dir: PathBuf) {
    start_inner(cx, config_dir, false);
}

fn start_inner(cx: &mut App, config_dir: PathBuf, with_bundled: bool) {
    let bundled = match (with_bundled, git_package()) {
        (true, Ok(package)) => vec![package],
        (true, Err(error)) => {
            tracing::warn!("bundled git package: {error}");
            Vec::new()
        }
        (false, _) => Vec::new(),
    };
    match PluginService::start_with_bundled(config_dir, Limits::default(), bundled) {
        Ok((service, events)) => {
            let handle = service.handle();
            cx.set_global(Plugins {
                inventory: Inventory::default(),
                handle: Some(handle),
                surfaces: PluginSurfaces::default(),
                command_context: None,
                command_preparations: 0,
                events: Some(events),
                _service: Some(service),
            });
        }
        Err(error) => {
            let inventory = Inventory { scan_error: Some(error.clone()), ..Inventory::default() };
            cx.set_global(Plugins {
                inventory,
                handle: None,
                surfaces: PluginSurfaces::default(),
                command_context: None,
                command_preparations: 0,
                events: None,
                _service: None,
            });
            status(format!("Plugins: {error}"), true, cx);
        }
    }
}

/// Config reload asks the same host to re-read grants and bundles; the host
/// invalidates changed workers before any new script runs.
pub fn reload(cx: &mut App) {
    if let Some(handle) = cx.try_global::<Plugins>().and_then(|plugins| plugins.handle.as_ref()) {
        handle.refresh();
    }
}

/// Drain the service's event channel on the main thread. Called by the existing
/// 100 ms config poll; no background thread wakes GPUI directly.
pub fn poll(cx: &mut App) {
    let events: Vec<HostEvent> = {
        let Some(plugins) = cx.try_global::<Plugins>() else {
            return;
        };
        let Some(receiver) = &plugins.events else {
            return;
        };
        receiver.try_iter().collect()
    };
    let now = Instant::now();
    // The newest inventory in the drain is the authority. Publish it before
    // reading any terminal reply from the same drain so a withdrawn generation
    // cannot apply a value or an effect.
    let newest_inventory = events.iter().rev().find_map(|event| match event {
        HostEvent::Inventory(inventory) => Some(inventory),
        _ => None,
    });
    let mut refresh = false;
    let mut menu_projection_changed = false;
    let mut icon_packs_changed = false;
    if let Some(inventory) = newest_inventory {
        let changed = {
            let plugins = cx.global_mut::<Plugins>();
            let changed = plugins.surfaces.publish_inventory(inventory);
            plugins.inventory = inventory.clone();
            changed
        };
        menu_projection_changed |= changed;
        icon_packs_changed = changed;
        refresh = true;
    }
    if icon_packs_changed {
        crate::icon_packs::reload(cx);
    }
    let handle = cx.try_global::<Plugins>().and_then(|plugins| plugins.handle.clone());
    let mut messages: Vec<(String, bool)> = Vec::new();
    let mut results: Vec<crate::actions::PluginCommandResult> = Vec::new();
    for event in events {
        match event {
            HostEvent::Inventory(_) => {}
            HostEvent::Status { message, error, .. } => messages.push((message, error)),
            HostEvent::RequestFinished { extension, stamp, result } => {
                let command = cx.global_mut::<Plugins>().surfaces.finish_command(&extension, stamp, &result);
                if let Some((window, extension, effect, capture)) = command {
                    results.push(crate::actions::PluginCommandResult { window, extension, effect, capture });
                } else {
                    let changed = cx.global_mut::<Plugins>().surfaces.finish(&extension, stamp, result, now);
                    refresh |= changed;
                }
            }
            HostEvent::MutationFinished { plugin, mutation, result } => {
                let plugins = cx.global_mut::<Plugins>();
                let running = plugins.surfaces.mutation_finished(&plugin, mutation, &result);
                refresh |= running || mutation == HostMutation::Disable;
                menu_projection_changed |= mutation == HostMutation::Disable;
                if let Err(error) = result {
                    messages.push((error, true));
                }
            }
        }
    }
    for action in results {
        dispatch_to_window(action, cx);
    }
    for (message, error) in messages {
        status(message, error, cx);
        refresh = true;
    }
    if menu_projection_changed {
        crate::menus::refresh(cx);
    }
    crate::git::sync_enabled(cx);
    cx.global_mut::<Plugins>().surfaces.drain_rows(handle.as_ref(), now);
    let hooks = cx.global_mut::<Plugins>().surfaces.flush_hooks(handle.as_ref(), now);
    refresh |= hooks;
    if refresh {
        cx.refresh_windows();
    }
}

/// All current row-contribution descriptors, in lexical fully qualified order.
/// Column and decoration renderers filter this by their configured surface.
pub fn row_descriptors(cx: &App) -> Vec<crow_core::ExtensionId> {
    let Some(plugins) = cx.try_global::<Plugins>() else { return Vec::new() };
    let snapshot = plugins.surfaces.snapshot();
    snapshot
        .of_kind(crow_core::ExtensionKind::Column)
        .chain(snapshot.of_kind(crow_core::ExtensionKind::Decoration))
        .filter(|(id, _)| !plugins.surfaces.is_suppressed(&id.plugin))
        .map(|(id, _)| id.clone())
        .collect()
}

/// Update the command target from a file surface's render. A surface that lost
/// focus clears only its own window's context.
pub fn set_command_context(window: &Window, context: Option<FileCommandContext>, cx: &mut App) {
    let Some(plugins) = cx.try_global::<Plugins>() else { return };
    let id = window.window_handle().window_id();
    if context.is_some() {
        cx.global_mut::<Plugins>().command_context = context;
    } else if plugins.command_context.as_ref().is_some_and(|current| current.window == id) {
        cx.global_mut::<Plugins>().command_context = None;
    }
}

/// One predicate shared by menu availability and dispatch. The action carries
/// the descriptor and generation the row was built for.
pub fn command_available(action: &PluginCommand, window: &Window, cx: &App) -> bool {
    if !window.is_window_active() {
        return false;
    }
    let Some(plugins) = cx.try_global::<Plugins>() else { return false };
    let Some(context) = plugins.command_context.as_ref() else { return false };
    if context.window != window.window_handle().window_id() {
        return false;
    }
    let snapshot = plugins.surfaces.snapshot();
    let Some(published) = snapshot.extension(&action.extension) else { return false };
    if published.generation != action.generation
        || published.descriptor.kind != crow_core::ExtensionKind::Command
        || plugins.surfaces.is_suppressed(&action.extension.plugin)
    {
        return false;
    }
    command_matches(&published.descriptor, &context.selection)
}

pub fn command_matches(descriptor: &crow_core::Descriptor, selection: &[(PathBuf, bool)]) -> bool {
    match descriptor.applies_to {
        CommandTarget::Scope => true,
        CommandTarget::Selection => {
            if selection.is_empty() || selection.len() > 128 {
                return false;
            }
            match &descriptor.extensions {
                None => true,
                Some(suffixes) => selection.iter().all(|(path, is_dir)| !*is_dir && suffix_matches(path, suffixes)),
            }
        }
    }
}

pub fn current_command_context(cx: &App) -> Option<FileCommandContext> {
    cx.try_global::<Plugins>().and_then(|plugins| plugins.command_context.clone())
}

pub fn restore_command_context(context: FileCommandContext, cx: &mut App) {
    if cx.try_global::<Plugins>().is_some() {
        cx.global_mut::<Plugins>().command_context = Some(context);
    }
}

pub fn capture_current(capture: &CommandCapture, window: &Window, cx: &App) -> bool {
    let Some(context) = cx.try_global::<Plugins>().and_then(|plugins| plugins.command_context.as_ref()) else {
        return false;
    };
    context.window == window.window_handle().window_id()
        && context.surface == capture.surface
        && context.scope == capture.scope
        && context.selection == capture.selection
}

pub fn capture_revision(capture: &CommandCapture) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    capture.surface.hash(&mut hasher);
    capture.scope.hash(&mut hasher);
    capture.selection.hash(&mut hasher);
    std::hash::Hasher::finish(&hasher)
}

pub fn begin_command_preparation(cx: &mut App) -> bool {
    let Some(plugins) = cx.try_global::<Plugins>() else { return false };
    if plugins.command_preparations >= 16 {
        return false;
    }
    cx.global_mut::<Plugins>().command_preparations += 1;
    true
}

pub fn end_command_preparation(cx: &mut App) {
    if let Some(plugins) = cx.try_global::<Plugins>() {
        let remaining = plugins.command_preparations.saturating_sub(1);
        cx.global_mut::<Plugins>().command_preparations = remaining;
    }
}

pub fn plugin_name(extension: &crow_core::ExtensionId, cx: &App) -> String {
    cx.try_global::<Plugins>()
        .and_then(|plugins| plugins.surfaces.snapshot().extension(extension).map(|published| published.package.clone()))
        .unwrap_or_else(|| extension.plugin.to_string())
}

fn suffix_matches(path: &Path, suffixes: &[String]) -> bool {
    let Some(name) = path.file_name() else { return false };
    let bytes = name.as_bytes();
    let Some(dot) = bytes.iter().rposition(|byte| *byte == b'.') else { return false };
    if dot + 1 == bytes.len() {
        return false;
    }
    let suffix = &bytes[dot + 1..];
    suffixes.iter().any(|candidate| {
        candidate.len() == suffix.len()
            && candidate.bytes().zip(suffix).all(|(left, right)| left == right.to_ascii_lowercase())
    })
}

/// Running plugin commands for the palette, lexically ordered by package, title
/// and fully qualified ID.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RowPresentation {
    pub badge: Option<String>,
    pub git_badge: bool,
    pub spoken: Vec<String>,
    pub tint: Option<crow_core::Tint>,
    pub dim: bool,
}

pub fn status_ids(cx: &App) -> Vec<crow_core::ExtensionId> {
    let Some(plugins) = cx.try_global::<Plugins>() else { return Vec::new() };
    let snapshot = plugins.surfaces.snapshot();
    snapshot
        .of_kind(crow_core::ExtensionKind::Status)
        .filter(|(id, _)| !plugins.surfaces.is_suppressed(&id.plugin))
        .map(|(id, _)| id.clone())
        .collect()
}

pub fn request_status(cx: &mut App, surface: SurfaceId, scope: ScopeKey, revision: u64, display: String) {
    let Some(plugins) = cx.try_global::<Plugins>() else { return };
    let handle = plugins.handle.clone();
    let ids = status_ids(cx);
    if ids.is_empty() {
        return;
    }
    let host = crate::git::host_data(cx, scope.path(), Vec::new());
    cx.global_mut::<Plugins>().surfaces.request_status(handle.as_ref(), surface, scope, revision, &display, &ids, host);
}

pub fn status_item(cx: &App, surface: SurfaceId) -> Option<(String, String)> {
    cx.try_global::<Plugins>().and_then(|plugins| plugins.surfaces.status_item(surface))
}

pub fn info_ids(cx: &App) -> Vec<crow_core::ExtensionId> {
    let Some(plugins) = cx.try_global::<Plugins>() else { return Vec::new() };
    let snapshot = plugins.surfaces.snapshot();
    snapshot
        .of_kind(crow_core::ExtensionKind::Info)
        .filter(|(id, _)| !plugins.surfaces.is_suppressed(&id.plugin))
        .map(|(id, _)| id.clone())
        .collect()
}

pub fn request_info(
    cx: &mut App,
    surface: SurfaceId,
    scope: ScopeKey,
    revision: u64,
    display: String,
    entry: &crow_fs::Entry,
) {
    let Some(plugins) = cx.try_global::<Plugins>() else { return };
    let handle = plugins.handle.clone();
    let ids = info_ids(cx);
    if ids.is_empty() {
        return;
    }
    let data = crow_core::EntryData {
        handle: crow_core::EntryHandle(1),
        name: crate::style::display_text(&entry.name).into_owned(),
        is_dir: entry.is_dir,
        size: entry.size,
        kind: crate::style::kind(entry).to_string(),
    };
    let host = crate::git::repository_data(cx, scope.path()).map(|repository| crow_plugin::HostData {
        repository: Some(repository),
        entries: crate::git::entry_host(cx, scope.path(), &entry.path, entry.is_dir, false).into_iter().collect(),
    });
    cx.global_mut::<Plugins>().surfaces.request_info(
        handle.as_ref(),
        surface,
        scope,
        revision,
        &display,
        &ids,
        data,
        host,
    );
}

pub fn info_rows(cx: &App, surface: SurfaceId) -> Vec<crow_core::InfoRow> {
    cx.try_global::<Plugins>().map_or_else(Vec::new, |plugins| plugins.surfaces.info_rows(surface))
}

pub fn has_sidebar(cx: &App) -> bool {
    let Some(plugins) = cx.try_global::<Plugins>() else { return false };
    if !plugins.surfaces.snapshot().has_kind(crow_core::ExtensionKind::Sidebar) {
        return false;
    }
    plugins
        .surfaces
        .snapshot()
        .of_kind(crow_core::ExtensionKind::Sidebar)
        .any(|(id, _)| !plugins.surfaces.is_suppressed(&id.plugin))
}

pub fn sidebar_descriptors(cx: &App) -> Vec<(crow_core::ExtensionId, String)> {
    let Some(plugins) = cx.try_global::<Plugins>() else { return Vec::new() };
    let snapshot = plugins.surfaces.snapshot();
    snapshot
        .of_kind(crow_core::ExtensionKind::Sidebar)
        .filter(|(id, _)| !plugins.surfaces.is_suppressed(&id.plugin))
        .map(|(id, published)| (id.clone(), published.descriptor.title.clone()))
        .collect()
}

pub fn request_sidebar(
    cx: &mut App,
    surface: SurfaceId,
    id: &crow_core::ExtensionId,
    scope: ScopeKey,
    revision: u64,
    display: String,
    dirs: &[SidebarDir],
) {
    let Some(plugins) = cx.try_global::<Plugins>() else { return };
    let handle = plugins.handle.clone();
    cx.global_mut::<Plugins>().surfaces.request_sidebar(handle.as_ref(), surface, id, scope, revision, &display, dirs);
}

pub fn sidebar_rows(cx: &App, surface: SurfaceId) -> Vec<SectionRow> {
    cx.try_global::<Plugins>().map_or_else(Vec::new, |plugins| plugins.surfaces.sidebar_rows(surface))
}

#[derive(Clone, Debug)]
pub struct PreviewCandidate {
    pub id: crow_core::ExtensionId,
    pub generation: u64,
    pub package: String,
    pub read_allowed: bool,
}

/// Configured preview descriptor IDs, in config order, whose suffix filter
/// matches this entry. Folders, symlinks and unregistered IDs are skipped.
pub fn preview_candidates(
    cx: &App,
    configured: &[String],
    path: &Path,
    is_dir: bool,
    is_symlink: bool,
) -> Vec<PreviewCandidate> {
    if is_dir || is_symlink {
        return Vec::new();
    }
    let Some(plugins) = cx.try_global::<Plugins>() else { return Vec::new() };
    let snapshot = plugins.surfaces.snapshot();
    let mut out = Vec::new();
    for raw in configured {
        let Ok(id) = crow_core::ExtensionId::parse(raw) else { continue };
        if out.iter().any(|candidate: &PreviewCandidate| candidate.id == id) {
            continue;
        }
        let Some(published) = snapshot.extension(&id) else { continue };
        if published.descriptor.kind != crow_core::ExtensionKind::Preview
            || plugins.surfaces.is_suppressed(&id.plugin)
            || !published.descriptor.extensions.as_ref().is_none_or(|suffixes| suffix_matches(path, suffixes))
        {
            continue;
        }
        out.push(PreviewCandidate {
            id,
            generation: published.generation,
            package: published.package.clone(),
            read_allowed: published.read_allowed,
        });
    }
    out
}

pub fn cancel_previews(surface: SurfaceId, cx: &mut App) {
    if cx.try_global::<Plugins>().is_some() {
        cx.global_mut::<Plugins>().surfaces.cancel_previews(surface);
    }
}

pub fn try_preview(
    cx: &mut App,
    surface: SurfaceId,
    candidate: &PreviewCandidate,
    entry: &crow_fs::Entry,
    excerpt: Option<crow_plugin::ExcerptInput>,
    revision: u64,
) -> Option<futures::channel::oneshot::Receiver<crow_plugin::RequestResult>> {
    let handle = cx.try_global::<Plugins>().and_then(|plugins| plugins.handle.clone())?;
    let input = crow_plugin::RequestInput {
        snapshot: crow_core::Snapshot {
            scope: entry.path.parent().map_or_else(String::new, |parent| parent.display().to_string()),
            selection: None,
            entries: vec![crow_core::EntryData {
                handle: crow_core::EntryHandle(1),
                name: crate::style::display_text(&entry.name).into_owned(),
                is_dir: entry.is_dir,
                size: entry.size,
                kind: crate::style::kind(entry).to_string(),
            }],
        },
        excerpt,
        host: None,
    };
    cx.global_mut::<Plugins>().surfaces.request_preview(
        &handle,
        surface,
        &candidate.id,
        candidate.generation,
        revision,
        input,
    )
}

pub fn decoration_ids(cx: &App) -> Vec<crow_core::ExtensionId> {
    let Some(plugins) = cx.try_global::<Plugins>() else { return Vec::new() };
    let snapshot = plugins.surfaces.snapshot();
    snapshot
        .of_kind(crow_core::ExtensionKind::Decoration)
        .filter(|(id, _)| !plugins.surfaces.is_suppressed(&id.plugin))
        .map(|(id, _)| id.clone())
        .collect()
}

/// One configured tile caption: the descriptor's title and the returned cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnValue {
    pub title: String,
    pub text: String,
    pub spoken: Option<String>,
}

pub fn column_title(cx: &App, id: &crow_core::ExtensionId) -> Option<String> {
    let plugins = cx.try_global::<Plugins>()?;
    Some(plugins.surfaces.snapshot().extension(id)?.descriptor.title.clone())
}

pub fn column_value(
    cx: &App,
    id: &crow_core::ExtensionId,
    scope: &ScopeKey,
    path: &Path,
    version: u64,
) -> Option<ColumnValue> {
    let plugins = cx.try_global::<Plugins>()?;
    let title = plugins.surfaces.snapshot().extension(id)?.descriptor.title.clone();
    let value = plugins.surfaces.row_value_peek(id, scope, path, version)?;
    (!value.text.is_empty()).then_some(ColumnValue { title, text: value.text, spoken: value.spoken })
}

/// The descriptors an icon grid needs: decorations for badges and dimming,
/// plus the one column configured as its caption. No other list columns.
pub fn icon_row_descriptors(cx: &App, caption: Option<&str>) -> Vec<crow_core::ExtensionId> {
    let mut ids = decoration_ids(cx);
    if let Some(id) = caption.and_then(|caption| crow_core::ExtensionId::parse(caption).ok())
        && !ids.contains(&id)
    {
        ids.push(id);
    }
    ids
}

/// First nonempty badge, first tint, OR'd dim and all spoken descriptions in
/// lexical fully qualified ID order. A badge is its own spoken fallback.
pub fn row_presentation(
    cx: &App,
    scope: &ScopeKey,
    path: &Path,
    version: u64,
    ids: &[crow_core::ExtensionId],
) -> RowPresentation {
    let Some(plugins) = cx.try_global::<Plugins>() else { return RowPresentation::default() };
    let mut out = RowPresentation::default();
    for id in ids {
        let Some(value) = plugins.surfaces.row_value_peek(id, scope, path, version) else { continue };
        if out.badge.is_none() && !value.text.is_empty() {
            out.badge = Some(value.text.clone());
            out.git_badge = crate::git::is_git_plugin(&id.plugin);
        }
        if out.tint.is_none() {
            out.tint = value.tint;
        }
        out.dim |= value.dim;
        let spoken = value
            .spoken
            .filter(|spoken| !spoken.is_empty())
            .or_else(|| (!value.text.is_empty()).then(|| value.text.clone()));
        if let Some(spoken) = spoken {
            out.spoken.push(spoken);
        }
    }
    out
}

pub fn command_eligible(extension: &crow_core::ExtensionId, selection: &[(PathBuf, bool)], cx: &App) -> bool {
    let Some(plugins) = cx.try_global::<Plugins>() else { return false };
    if plugins.surfaces.is_suppressed(&extension.plugin) {
        return false;
    }
    let snapshot = plugins.surfaces.snapshot();
    snapshot.extension(extension).is_some_and(|published| command_matches(&published.descriptor, selection))
}

pub fn command_items(cx: &App) -> Vec<crate::palette::PluginCommandItem> {
    let Some(plugins) = cx.try_global::<Plugins>() else { return Vec::new() };
    let snapshot = plugins.surfaces.snapshot();
    let mut items: Vec<_> = snapshot
        .of_kind(crow_core::ExtensionKind::Command)
        .filter(|(id, _)| !plugins.surfaces.is_suppressed(&id.plugin))
        .map(|(id, published)| crate::palette::PluginCommandItem {
            extension: id.clone(),
            generation: published.generation,
            title: published.descriptor.title.clone(),
            package: published.package.clone(),
        })
        .collect();
    items.sort_by(|left, right| {
        left.package.cmp(&right.package).then(left.title.cmp(&right.title)).then(left.extension.cmp(&right.extension))
    });
    items
}

/// Selection commands that opted into the file context menu, with their menu
/// titles (duplicate titles carry the package name).
pub fn context_menu_commands(cx: &App, selection: &[(PathBuf, bool)]) -> Vec<(String, crate::actions::PluginCommand)> {
    let Some(plugins) = cx.try_global::<Plugins>() else { return Vec::new() };
    let snapshot = plugins.surfaces.snapshot();
    let mut commands: Vec<(String, String, crow_core::ExtensionId, u64, String)> = snapshot
        .of_kind(crow_core::ExtensionKind::Command)
        .filter(|(id, published)| {
            !plugins.surfaces.is_suppressed(&id.plugin)
                && published.descriptor.context_menu
                && published.descriptor.applies_to == CommandTarget::Selection
                && command_matches(&published.descriptor, selection)
        })
        .map(|(id, published)| {
            (
                published.package.clone(),
                published.descriptor.title.clone(),
                id.clone(),
                published.generation,
                published.descriptor.id.to_string(),
            )
        })
        .collect();
    commands.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)).then(left.4.cmp(&right.4)));
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (_, title, _, _, _) in &commands {
        *counts.entry(title.clone()).or_insert(0) += 1;
    }
    commands
        .into_iter()
        .map(|(package, title, extension, generation, _)| {
            let title =
                if counts.get(&title).copied().unwrap_or(0) > 1 { format!("{title} — {package}") } else { title };
            (title, crate::actions::PluginCommand { extension, generation })
        })
        .collect()
}

pub fn notify_operation_done(cx: &mut App) {
    if cx.try_global::<Plugins>().is_some() {
        cx.global_mut::<Plugins>().surfaces.notify_operation_done(Instant::now());
    }
}

/// Settings successfully queued Disable. The package stops contributing until
/// the host reports the correlated mutation, so a queued reply cannot leak.
pub fn suppress_disable(id: crow_core::PluginId, cx: &mut App) {
    if cx.try_global::<Plugins>().is_some() {
        cx.global_mut::<Plugins>().surfaces.suppress_disable(&id);
        crate::menus::refresh(cx);
        cx.refresh_windows();
    }
}

fn dispatch_to_window(action: crate::actions::PluginCommandResult, cx: &mut App) {
    let target = action.window;
    if let Some(handle) = cx.windows().into_iter().find(|handle| handle.window_id() == target) {
        cx.update_window(handle, move |_, window, cx| {
            window.dispatch_action(Box::new(action), cx);
        })
        .ok();
    }
}

fn status(message: String, error: bool, cx: &mut App) {
    if cx.try_global::<crate::operations::Operations>().is_some() {
        crate::operations::message(message, error, cx);
    } else {
        tracing::warn!(%message, error, "plugin status");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crow_core::{Descriptor, DescriptorSource, ExtensionKind, LocalId, PluginId};

    fn command(selection: &[(PathBuf, bool)], extensions: Option<Vec<String>>) -> Descriptor {
        let mut descriptor = Descriptor::new(
            ExtensionKind::Command,
            crow_core::ExtensionId::new(PluginId::parse("example.test").unwrap(), LocalId::parse("c").unwrap()),
            "C".into(),
            DescriptorSource::Function("run".into()),
        );
        descriptor.applies_to = CommandTarget::Selection;
        descriptor.extensions = extensions;
        let _ = selection;
        descriptor
    }

    #[test]
    fn command_eligibility_checks_selection_cap_and_folded_suffixes() {
        let path = PathBuf::from("/tmp/a.TXT");
        let one = [(path.clone(), false)];
        assert!(command_matches(&command(&one, None), &one));
        assert!(!command_matches(&command(&[], None), &[]));
        let mut too_many = Vec::new();
        for i in 0..129 {
            too_many.push((PathBuf::from(format!("/tmp/{i}.txt")), false));
        }
        assert!(!command_matches(&command(&too_many, None), &too_many));
        assert!(command_matches(&command(&one, Some(vec!["txt".into()])), &one));
        assert!(!command_matches(&command(&one, Some(vec!["png".into()])), &one));
        let folder = [(PathBuf::from("/tmp/a.txt"), true)];
        assert!(!command_matches(&command(&folder, Some(vec!["txt".into()])), &folder));
    }

    #[test]
    fn shipped_p1b_fixtures_load_as_packages() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plugins");
        let fixture = crow_plugin::load_package(&root.join("p1b-fixture"), &crow_plugin::Limits::default())
            .expect("valid fixture");
        assert_eq!(fixture.manifest.id.as_str(), "example.p1b-fixture");
        assert!(fixture.manifest.provides.contains(&ExtensionKind::Column));
        assert!(fixture.manifest.provides.contains(&ExtensionKind::Preview));
        assert_eq!(fixture.manifest.permissions.fs(), crow_plugin::FsAccess::Read);
        assert!(fixture.files.contains_key("icons/pack.toml"));
        for invalid in ["p1b-invalid-handle", "p1b-invalid-field", "p1b-budget"] {
            assert!(
                crow_plugin::load_package(&root.join(invalid), &crow_plugin::Limits::default()).is_ok(),
                "{invalid} loads; its defect is exercised at evaluation time"
            );
        }
        assert!(root.join("p1b-fixture/tree/fixture.txt").is_file());
        assert!(root.join("p1b-fixture/tree/fixture-folder").is_dir());
    }

    #[test]
    fn raw_non_utf8_names_still_match_by_suffix() {
        use std::ffi::OsStr;
        let path = Path::new(OsStr::from_bytes(b"bad\xff.PNG"));
        assert!(suffix_matches(path, &["png".into()]));
        assert!(!suffix_matches(path, &["jpg".into()]));
        assert!(!suffix_matches(Path::new("/tmp/README"), &["md".into()]));
    }
}

#[cfg(test)]
mod bundled_tests {
    use super::*;
    use crow_core::{ContributionValues, ExtensionId, PluginId};
    use crow_plugin::{HostData, HostEvent, RepositoryData, RequestInput, RequestResult};
    use std::time::Duration;

    fn wait_inventory(events: &Receiver<HostEvent>) -> crow_plugin::Inventory {
        for _ in 0..200 {
            while let Ok(event) = events.try_recv() {
                if let HostEvent::Inventory(inventory) = event {
                    return inventory;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("no inventory");
    }

    fn start_git() -> (PluginService, Receiver<HostEvent>, crow_plugin::PluginEntry) {
        let package = git_package().expect("embedded package");
        let temp = tempfile::tempdir().unwrap();
        let (service, events) =
            PluginService::start_with_bundled(temp.path().to_path_buf(), Limits::default(), vec![package.clone()])
                .unwrap();
        let handle = service.handle();
        let inventory = wait_inventory(&events);
        assert_eq!(inventory.plugins[0].state, crow_plugin::PluginState::PendingApproval);
        handle.approve(PluginId::parse("git").unwrap(), package.digest.clone()).unwrap();
        for _ in 0..200 {
            let inventory = wait_inventory(&events);
            if let Some(entry) = inventory.plugins.iter().find(|entry| entry.state == crow_plugin::PluginState::Running)
            {
                return (service, events, entry.clone());
            }
        }
        panic!("git package never reached Running");
    }

    fn wait_result(events: &Receiver<HostEvent>, id: &ExtensionId) -> RequestResult {
        for _ in 0..200 {
            while let Ok(event) = events.try_recv() {
                if let HostEvent::RequestFinished { extension, result, .. } = event
                    && &extension == id
                {
                    return result;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("request for {id} did not finish");
    }

    fn entry(handle: u32, badge: &str, label: &str) -> crow_plugin::EntryHostData {
        crow_plugin::EntryHostData {
            handle: crow_core::EntryHandle(handle),
            badge: badge.into(),
            label: label.into(),
            index: None,
            worktree: Some(label.into()),
            ignored: false,
            lines_changed: Some(3),
        }
    }

    #[test]
    fn git_refresh_keeps_ignored_decoration_until_the_current_reply() {
        use crate::git::PluginRefresh;

        let (service, events, plugin) = start_git();
        let handle = service.handle();
        let plugin_id = plugin.id.clone().unwrap();
        let decoration = ExtensionId::parse("git.status").unwrap();
        let mut surfaces = PluginSurfaces::default();
        surfaces.publish_inventory(&Inventory { plugins: vec![plugin], grants_error: None, scan_error: None });

        let surface = new_surface();
        let scope = ScopeKey::Search { session: 1, origin: PathBuf::from("/tmp") };
        let path = PathBuf::from("/tmp/one/ignored.txt");
        let other = PathBuf::from("/tmp/two/ignored.txt");
        let matches_folder = |folder: &Path| folder == Path::new("/tmp/one");
        let row = |path: &Path, ignored: bool| RowInput {
            path: path.to_path_buf(),
            version: 1,
            name: "ignored.txt".into(),
            is_dir: false,
            size: 1,
            kind: "Text".into(),
            selected: false,
            git: ignored.then(|| crow_plugin::EntryHostData { ignored: true, ..entry(1, "", "ignored") }),
        };
        let observe = |surfaces: &mut PluginSurfaces, ignored| {
            surfaces.observe_rows(
                Some(&handle),
                surface,
                scope.clone(),
                1,
                "/tmp",
                1,
                std::slice::from_ref(&decoration),
                vec![row(&path, ignored), row(&other, true)],
                Instant::now(),
            );
        };
        let reply = || loop {
            if let HostEvent::RequestFinished { extension, stamp, result } =
                events.recv_timeout(Duration::from_secs(2)).expect("Git decoration reply")
            {
                assert_eq!(extension, decoration);
                break (stamp, result);
            }
        };
        let dim = |surfaces: &PluginSurfaces, path: &Path| {
            surfaces.row_value_peek(&decoration, &scope, path, 1).is_some_and(|value| value.dim)
        };

        observe(&mut surfaces, true);
        let (stamp, result) = reply();
        assert!(surfaces.finish(&decoration, stamp, result, Instant::now()));
        assert!(dim(&surfaces, &path) && dim(&surfaces, &other));

        surfaces.refresh_git(PluginRefresh::Lines, matches_folder);
        observe(&mut surfaces, true);
        assert_eq!(surfaces.pending_tickets(), 0, "an identical status leaves decoration ready");

        surfaces.refresh_git(PluginRefresh::Status, matches_folder);
        observe(&mut surfaces, false);
        let (stamp, result) = reply();
        assert!(dim(&surfaces, &path) && dim(&surfaces, &other), "the old decoration stays until the reply is applied");
        surfaces.refresh_git(PluginRefresh::Status, matches_folder);
        assert!(!surfaces.finish(&decoration, stamp, result, Instant::now()), "an obsolete reply cannot clear the row");
        assert!(dim(&surfaces, &path));

        observe(&mut surfaces, false);
        let (stamp, result) = reply();
        assert!(surfaces.finish(&decoration, stamp, result, Instant::now()));
        assert!(!dim(&surfaces, &path), "a clean result clears the previously ignored styling");
        assert!(dim(&surfaces, &other), "a different worktree keeps its styling");
        surfaces.suppress_disable(&plugin_id);
        assert!(!dim(&surfaces, &other));
    }

    #[test]
    fn bundled_git_package_maps_decorations_column_and_info() {
        let (_service, events, entry_info) = start_git();
        let handle = _service.handle();
        let host = HostData {
            repository: Some(RepositoryData {
                branch: Some("main".into()),
                head: None,
                detached: false,
                unborn: false,
                ahead: None,
                behind: None,
                changed: 2,
                last_commit: None,
                unavailable: false,
            }),
            entries: vec![entry(7, "M", "modified")],
        };
        let input = RequestInput {
            snapshot: crow_core::Snapshot {
                scope: "/tmp/repo".into(),
                selection: None,
                entries: vec![crow_core::EntryData {
                    handle: crow_core::EntryHandle(7),
                    name: "a.txt".into(),
                    is_dir: false,
                    size: 1,
                    kind: "Text".into(),
                }],
            },
            excerpt: None,
            host: Some(host.clone()),
        };
        handle
            .try_evaluate(ExtensionId::parse("git.status").unwrap(), entry_info.generation, 1, input.clone())
            .unwrap();
        let decoration = wait_result(&events, &ExtensionId::parse("git.status").unwrap());
        let RequestResult::Values(ContributionValues::Decoration(values)) = decoration else {
            panic!("expected decoration, got {decoration:?}");
        };
        assert_eq!(values[0].text, "M");
        assert_eq!(values[0].spoken.as_deref(), Some("modified"));
        assert_eq!(values[0].tint, Some(crow_core::Tint::Success));
        assert!(!values[0].dim);

        handle
            .try_evaluate(ExtensionId::parse("git.lines-changed").unwrap(), entry_info.generation, 2, input.clone())
            .unwrap();
        let column = wait_result(&events, &ExtensionId::parse("git.lines-changed").unwrap());
        let RequestResult::Values(ContributionValues::Column(values)) = column else {
            panic!("expected column, got {column:?}");
        };
        assert_eq!(values[0].text, "3");

        let mut missing = input.clone();
        missing.host.as_mut().unwrap().entries[0].lines_changed = None;
        handle
            .try_evaluate(ExtensionId::parse("git.lines-changed").unwrap(), entry_info.generation, 4, missing)
            .unwrap();
        let column = wait_result(&events, &ExtensionId::parse("git.lines-changed").unwrap());
        let RequestResult::Values(ContributionValues::Column(values)) = column else {
            panic!("expected column");
        };
        assert_eq!(values[0].text, "—", "an unknown count is not zero");

        handle.try_evaluate(ExtensionId::parse("git.details").unwrap(), entry_info.generation, 3, input).unwrap();
        let info = wait_result(&events, &ExtensionId::parse("git.details").unwrap());
        let RequestResult::Values(ContributionValues::Info(rows)) = info else {
            panic!("expected info, got {info:?}");
        };
        assert!(rows.iter().any(|row| row.label == "Status" && row.value == "modified"));
        assert!(rows.iter().any(|row| row.label == "Lines changed" && row.value == "3"));
        assert!(rows.iter().any(|row| row.label == "Branch" && row.value == "main"));
    }

    #[test]
    fn bundled_git_package_registers_and_maps_host_data() {
        let package = git_package().expect("embedded package");
        assert_eq!(package.manifest.id.as_str(), "git");
        let temp = tempfile::tempdir().unwrap();
        let (service, events) =
            PluginService::start_with_bundled(temp.path().to_path_buf(), Limits::default(), vec![package.clone()])
                .unwrap();
        let handle = service.handle();
        let inventory = wait_inventory(&events);
        let entry =
            inventory.plugins.iter().find(|entry| entry.id.as_ref().is_some_and(|id| id.as_str() == "git")).unwrap();
        assert!(entry.bundled);
        assert_eq!(entry.state, crow_plugin::PluginState::PendingApproval);

        handle.approve(PluginId::parse("git").unwrap(), package.digest.clone()).unwrap();
        let mut running = None;
        for _ in 0..200 {
            let inventory = wait_inventory(&events);
            if let Some(entry) =
                inventory.plugins.iter().find(|entry| entry.id.as_ref().is_some_and(|id| id.as_str() == "git"))
                && entry.state == crow_plugin::PluginState::Running
            {
                running = Some(entry.clone());
                break;
            }
        }
        let entry = running.expect("git package reaches Running");
        assert!(entry.descriptors.iter().any(|descriptor| descriptor.kind == crow_core::ExtensionKind::Column));
        let status_id = ExtensionId::parse("git.repository").unwrap();
        let generation = entry.generation;
        let input = RequestInput {
            snapshot: crow_core::Snapshot { scope: "/tmp".into(), selection: None, entries: Vec::new() },
            excerpt: None,
            host: Some(HostData {
                repository: Some(RepositoryData {
                    branch: Some("main".into()),
                    head: None,
                    detached: false,
                    unborn: false,
                    ahead: Some(2),
                    behind: None,
                    changed: 1,
                    last_commit: None,
                    unavailable: false,
                }),
                entries: Vec::new(),
            }),
        };
        handle.try_evaluate(status_id.clone(), generation, 1, input).unwrap();
        for _ in 0..200 {
            while let Ok(event) = events.try_recv() {
                if let HostEvent::RequestFinished { extension, result, .. } = event
                    && extension == status_id
                {
                    match result {
                        RequestResult::Values(ContributionValues::Status(values)) => {
                            assert_eq!(values, vec![crow_core::StatusItem { text: "main ↑2 · 1 changed".into() }]);
                            return;
                        }
                        other => panic!("unexpected result: {other:?}"),
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("status request did not finish");
    }
}
