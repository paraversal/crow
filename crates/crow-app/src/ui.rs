//! App-wide UI state: settings and theme, reloaded live for every window.

use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use crow_core::{Settings, load_settings};
use crow_theme::Theme;
use gpui::{App, Global, SharedString};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::style::variant_of;

pub struct Ui {
    pub settings: Settings,
    pub theme: Theme,
    /// Shown in every window's status line until the file is fixed.
    pub settings_error: Option<SharedString>,
}

impl Global for Ui {}

struct Watchers {
    theme_rx: Receiver<()>,
    settings_rx: Receiver<()>,
    _theme: Option<RecommendedWatcher>,
    _settings: Option<RecommendedWatcher>,
}

impl Global for Watchers {}

/// Install the `Ui` global and start watching the theme index and `~/.config/crow`.
pub fn init(cx: &mut App) {
    let (settings, settings_error) = match load_settings(&crow_core::config_dir()) {
        Ok(s) => (s, None),
        Err(e) => {
            tracing::warn!("settings: {e}");
            (Settings::default(), Some(e.to_string().into()))
        }
    };
    let theme = crow_theme::current(settings.theme_id(), variant_of(cx.window_appearance()));
    tracing::info!(theme = %theme.name, "starting");
    cx.set_global(Ui { settings, theme, settings_error });

    let (theme_tx, theme_rx) = mpsc::channel();
    let theme_watcher = crow_theme::watch(theme_tx).inspect_err(|e| tracing::warn!("theme watch: {e}")).ok();
    let (settings_tx, settings_rx) = mpsc::channel();
    let settings_watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            settings_tx.send(()).ok();
        }
    })
    .and_then(|mut w| w.watch(&crow_core::config_dir(), RecursiveMode::NonRecursive).map(|()| w))
    .inspect_err(|e| tracing::warn!("settings watch: {e}"))
    .ok();
    cx.set_global(Watchers { theme_rx, settings_rx, _theme: theme_watcher, _settings: settings_watcher });

    // Polling keeps every reload on the main thread; 100 ms doubles as the debounce.
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_millis(100)).await;
            cx.update(poll);
        }
    })
    .detach();
}

/// Default settings and the embedded theme, with no files read or watched.
#[cfg(test)]
pub fn init_for_tests(cx: &mut App) {
    cx.set_global(Ui { settings: Settings::default(), theme: crow_theme::fallback(), settings_error: None });
}

fn poll(cx: &mut App) {
    let watchers = cx.global::<Watchers>();
    let settings_changed = watchers.settings_rx.try_iter().count() > 0;
    let theme_changed = watchers.theme_rx.try_iter().count() > 0;
    if settings_changed {
        reload_settings(cx);
    }
    if settings_changed || theme_changed {
        reload_theme(cx);
    }
}

fn reload_settings(cx: &mut App) {
    match load_settings(&crow_core::config_dir()) {
        Ok(settings) => {
            let ui = cx.global_mut::<Ui>();
            if ui.settings != settings || ui.settings_error.is_some() {
                tracing::info!("settings reloaded");
                ui.settings = settings;
                ui.settings_error = None;
                cx.refresh_windows();
            }
        }
        // A half-saved or broken file: keep the last good settings, say why.
        Err(e) => {
            tracing::warn!("settings: {e}");
            cx.global_mut::<Ui>().settings_error = Some(e.to_string().into());
            cx.refresh_windows();
        }
    }
}

/// Re-resolve the theme (after an index change, a settings change or an
/// appearance flip). A broken theme file keeps the current theme.
pub fn reload_theme(cx: &mut App) {
    let variant = variant_of(cx.window_appearance());
    let (dir, state) = (crow_theme::themes_dir(), crow_theme::state_dir());
    let saved = cx.global::<Ui>().settings.theme_id().map(str::to_owned);
    let loaded = crow_theme::resolve_theme_id(saved.as_deref(), variant, &dir, &state)
        .ok_or_else(|| "no themes in the index".to_owned())
        .and_then(|id| crow_theme::load_theme(&dir, &id));
    match loaded {
        Ok(theme) => {
            if theme.name != cx.global::<Ui>().theme.name {
                tracing::info!(theme = %theme.name, "theme changed");
            }
            cx.global_mut::<Ui>().theme = theme;
            cx.refresh_windows();
        }
        Err(e) => tracing::warn!("theme reload skipped: {e}"),
    }
}
