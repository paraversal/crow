//! The app's plugin service: one background host, its inventory global, and the
//! main-thread drain that moves worker replies onto the UI. No disk I/O and no
//! waiting for a plugin happens on the UI thread.

use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use crow_plugin::{HostEvent, HostMutation, Inventory, Limits, PluginHandle, PluginService};
use gpui::{App, Global};

use crate::ui::Ui;

mod coordinator;
mod demand;
pub use demand::PluginRows;
#[allow(unused_imports)]
pub use coordinator::{
    HOOK_DEBOUNCE, MAX_BATCH, MAX_PASSIVE_PER_PLUGIN, MAX_PASSIVE_REQUESTS, MAX_ROW_BYTES, MAX_ROW_VALUES,
    PluginCounters, PluginSurfaces, PublishedExtension, PublishedSurfaces, RowInput, RowValue, ScopeKey, SurfaceId,
    new_surface,
};

#[derive(Default)]
pub struct Plugins {
    pub inventory: Inventory,
    pub handle: Option<PluginHandle>,
    pub surfaces: PluginSurfaces,
    events: Option<Receiver<HostEvent>>,
    _service: Option<PluginService>,
}

impl Global for Plugins {}

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
    if let Some(inventory) = newest_inventory {
        let plugins = cx.global_mut::<Plugins>();
        plugins.surfaces.publish_inventory(inventory);
        plugins.inventory = inventory.clone();
        refresh = true;
    }
    let handle = cx.try_global::<Plugins>().and_then(|plugins| plugins.handle.clone());
    let mut messages: Vec<(String, bool)> = Vec::new();
    for event in events {
        match event {
            HostEvent::Inventory(_) => {}
            HostEvent::Status { message, error, .. } => messages.push((message, error)),
            HostEvent::RequestFinished { extension, stamp, result } => {
                let changed = cx.global_mut::<Plugins>().surfaces.finish(&extension, stamp, result, now);
                refresh |= changed;
            }
            HostEvent::MutationFinished { plugin, mutation, result } => {
                let plugins = cx.global_mut::<Plugins>();
                let running = plugins.surfaces.mutation_finished(&plugin, mutation, &result);
                refresh |= running || mutation == HostMutation::Disable;
                if let Err(error) = result {
                    messages.push((error, true));
                }
            }
        }
    }
    for (message, error) in messages {
        status(message, error, cx);
        refresh = true;
    }
    cx.global_mut::<Plugins>().surfaces.drain_rows(handle.as_ref(), now);
    let hooks = cx.global_mut::<Plugins>().surfaces.flush_hooks(handle.as_ref(), now);
    refresh |= hooks;
    if refresh {
        cx.refresh_windows();
    }
}

/// Settings successfully queued Disable. The package stops contributing until
/// the host reports the correlated mutation, so a queued reply cannot leak.
/// All current row-contribution descriptors, in lexical fully qualified order.
/// Column and decoration renderers filter this by their configured surface.
pub fn row_descriptors(cx: &App) -> Vec<crow_core::ExtensionId> {
    let Some(plugins) = cx.try_global::<Plugins>() else { return Vec::new() };
    let snapshot = plugins.surfaces.snapshot();
    snapshot
        .of_kind(crow_core::ExtensionKind::Column)
        .chain(snapshot.of_kind(crow_core::ExtensionKind::Decoration))
        .map(|(id, _)| id.clone())
        .collect()
}

pub fn notify_operation_done(cx: &mut App) {
    if cx.try_global::<Plugins>().is_some() {
        cx.global_mut::<Plugins>().surfaces.notify_operation_done(Instant::now());
    }
}

pub fn suppress_disable(id: crow_core::PluginId, cx: &mut App) {
    if cx.try_global::<Plugins>().is_some() {
        cx.global_mut::<Plugins>().surfaces.suppress_disable(&id);
        cx.refresh_windows();
    }
}

fn status(message: String, error: bool, cx: &mut App) {
    if cx.try_global::<crate::operations::Operations>().is_some() {
        crate::operations::message(message, error, cx);
    } else {
        tracing::warn!(%message, error, "plugin status");
    }
}
