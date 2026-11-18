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

pub fn start(cx: &mut App, config_dir: PathBuf) {
    match PluginService::start(config_dir, Limits::default()) {
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
    if let Some(inventory) = newest_inventory {
        let changed = {
            let plugins = cx.global_mut::<Plugins>();
            let changed = plugins.surfaces.publish_inventory(inventory);
            plugins.inventory = inventory.clone();
            changed
        };
        menu_projection_changed |= changed;
        refresh = true;
    }
    let handle = cx.try_global::<Plugins>().and_then(|plugins| plugins.handle.clone());
    let mut messages: Vec<(String, bool)> = Vec::new();
    let mut results: Vec<crate::actions::PluginCommandResult> = Vec::new();
    for event in events {
        match event {
            HostEvent::Inventory(_) => {}
            HostEvent::Status { message, error, .. } => messages.push((message, error)),
            HostEvent::RequestFinished { extension, stamp, result } => {
                let command = cx.global_mut::<Plugins>().surfaces.finish_command(&extension, stamp, result.clone());
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
    cx.global_mut::<Plugins>().surfaces.request_status(handle.as_ref(), surface, scope, revision, &display, &ids);
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
    cx.global_mut::<Plugins>().surfaces.request_info(handle.as_ref(), surface, scope, revision, &display, &ids, data);
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
