//! The app's plugin service: one background host, its inventory global, and the
//! main-thread drain that moves worker replies onto the UI. No disk I/O and no
//! waiting for a plugin happens on the UI thread.

use std::path::PathBuf;
use std::sync::mpsc::Receiver;

use crow_plugin::{HostEvent, Inventory, Limits, PluginHandle, PluginService};
use gpui::{App, Global};

use crate::ui::Ui;

#[derive(Default)]
pub struct Plugins {
    pub inventory: Inventory,
    pub handle: Option<PluginHandle>,
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
                events: Some(events),
                _service: Some(service),
            });
        }
        Err(error) => {
            let inventory = Inventory { scan_error: Some(error.clone()), ..Inventory::default() };
            cx.set_global(Plugins { inventory, handle: None, events: None, _service: None });
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
    for event in events {
        apply(event, cx);
    }
}

fn apply(event: HostEvent, cx: &mut App) {
    match event {
        HostEvent::Inventory(inventory) => {
            cx.global_mut::<Plugins>().inventory = inventory;
            cx.refresh_windows();
        }
        HostEvent::Status { message, error, .. } => status(message, error, cx),
        // P1B applies command intents and consumes contributions. P1A keeps the
        // validated events on the host boundary; commands remain unapplied.
        HostEvent::CommandIntent { .. } | HostEvent::Refresh { .. } | HostEvent::Contribution { .. } => {}
    }
}

fn status(message: String, error: bool, cx: &mut App) {
    if cx.try_global::<crate::operations::Operations>().is_some() {
        crate::operations::message(message, error, cx);
    } else {
        tracing::warn!(%message, error, "plugin status");
    }
}
