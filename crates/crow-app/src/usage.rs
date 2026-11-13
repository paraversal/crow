//! Key-use counts for the window: the global that `crow_core::Usage` lives in,
//! how a use is recorded and how hints read it. U9's tips read the same counts.

use std::path::PathBuf;

use crow_core::Usage;
use gpui::{Action, App, AppContext, Global, Task};

use crate::ui::Ui;

pub struct UsageState {
    usage: Usage,
    path: PathBuf,
    /// The pending save; a new one waits for it so the file never goes backwards.
    save: Option<Task<()>>,
}

impl Global for UsageState {}

/// Load `usage.json` from the state dir. A startup read, like Recent's.
pub fn init(cx: &mut App) {
    let path = cx.global::<Ui>().state_dir.join(crow_core::USAGE_FILE);
    let usage = Usage::load(&path);
    cx.set_global(UsageState { usage, path, save: None });
}

/// Record one use of `action`, however it ran: key, menu item or palette.
pub fn record(action: &dyn Action, cx: &mut App) {
    record_key(action.name(), cx);
}

/// Record a use under a non-action key (U9's dismissed tips).
pub fn record_key(key: &str, cx: &mut App) {
    let (usage, path, previous) = {
        let state = cx.global_mut::<UsageState>();
        if !state.usage.record(key) {
            return;
        }
        (state.usage.clone(), state.path.clone(), state.save.take())
    };
    let task = cx.background_spawn(async move {
        if let Some(previous) = previous {
            previous.await;
        }
        if let Err(e) = usage.save(&path) {
            tracing::warn!("{}: {e}", path.display());
        }
    });
    cx.global_mut::<UsageState>().save = Some(task);
}

/// How many times `action` has been used. U9's tips read this too.
pub fn uses(action: &dyn Action, cx: &App) -> u32 {
    count_key(action.name(), cx)
}

pub fn count_key(key: &str, cx: &App) -> u32 {
    cx.global::<UsageState>().usage.count(key)
}

/// Re-read the file, for the persistence test.
#[cfg(test)]
pub fn reload_from_disk(cx: &mut App) {
    let state = cx.global_mut::<UsageState>();
    let path = state.path.clone();
    state.usage = Usage::load(&path);
}
