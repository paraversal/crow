//! App-wide UI state: settings, view settings, the sidebar's configuration,
//! recent folders, volumes and the theme, reloaded live for every window.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crow_core::{
    Exclude, Recent, RecentCommands, Settings, SidebarConfig, Views, load_settings, load_sidebar, load_views,
};
use crow_theme::Theme;
use gpui::{App, AppContext, Global, SharedString};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::style::variant_of;

const FADE: Duration = Duration::from_millis(500);
const FADE_REDUCED_MOTION: Duration = Duration::from_millis(150);

pub struct Ui {
    pub settings: Settings,
    /// The colors on screen: the current theme, or a step of a fade towards it.
    pub theme: Theme,
    /// Where the colors are heading; equal to `theme` when no fade is running.
    target: Theme,
    fade: Option<(Theme, Instant, Duration)>,
    /// False in tests, so colors change at once.
    fade_enabled: bool,
    /// A theme shown while the picker's cursor is on it, before it's chosen: an id or "system".
    pub preview_theme: Option<String>,
    pub views: Views,
    /// Counts reloads of `views.toml`, so panes know to apply it again.
    pub views_generation: u64,
    pub sidebar: SidebarConfig,
    /// Exclusions for search, reloaded with the other config files.
    pub exclude: Exclude,
    /// Counts `exclude.toml` reloads that changed it, so a running search can restart.
    pub exclude_generation: u64,
    pub recent: Recent,
    /// Commands run from the palette, newest first (state, not config).
    pub recent_commands: RecentCommands,
    zoxide: Vec<(f64, PathBuf)>,
    /// The folders Recent shows, best first (computed off the main thread).
    pub recent_list: Vec<PathBuf>,
    /// More of the same ranking for the command palette (favorites included).
    pub palette_recents: Vec<PathBuf>,
    pub volumes: Vec<crow_macos::Volume>,
    pub config_dir: PathBuf,
    pub state_dir: PathBuf,
    pub home: PathBuf,
    /// One message per config file that doesn't load; shown in the status line until fixed.
    errors: [Option<SharedString>; 4],
}

impl Global for Ui {}

const SETTINGS: usize = 0;
const VIEWS: usize = 1;
const SIDEBAR: usize = 2;
const EXCLUDE: usize = 3;

impl Ui {
    fn new(config_dir: PathBuf, state_dir: PathBuf, home: PathBuf, theme: Theme, fade_enabled: bool) -> Ui {
        let recent = Recent::load(&state_dir.join("recent.json"));
        let recent_commands = RecentCommands::load(&state_dir.join("palette-commands.json"));
        let mut ui = Ui {
            settings: Settings::default(),
            target: theme.clone(),
            theme,
            fade: None,
            fade_enabled,
            preview_theme: None,
            views: Views::default(),
            views_generation: 0,
            sidebar: SidebarConfig::parse(crow_core::DEFAULT_SIDEBAR).unwrap_or_default(),
            exclude: Exclude::default(),
            exclude_generation: 0,
            recent,
            recent_commands,
            zoxide: Vec::new(),
            recent_list: Vec::new(),
            palette_recents: Vec::new(),
            volumes: Vec::new(),
            config_dir,
            state_dir,
            home,
            errors: Default::default(),
        };
        ui.load_settings();
        ui.load_views();
        ui.load_sidebar();
        ui.load_exclude();
        ui
    }

    /// The first config file problem, if any. Tests read it; the bar uses the file form.
    #[cfg(test)]
    pub fn config_error(&self) -> Option<SharedString> {
        self.errors.iter().flatten().next().cloned()
    }

    /// The first config file problem and the file it names, with the config dir
    /// stripped so the bar can say "views.toml: …".
    pub fn config_error_file(&self) -> Option<(SharedString, PathBuf)> {
        const FILES: [&str; 4] = ["settings.toml", "views.toml", "sidebar.toml", "exclude.toml"];
        let (index, message) = self.errors.iter().enumerate().find_map(|(i, e)| e.as_ref().map(|m| (i, m)))?;
        let dir = self.config_dir.display().to_string();
        let text = message.strip_prefix(&dir).map_or(message.as_str(), |rest| rest.trim_start_matches('/'));
        Some((text.to_owned().into(), self.config_dir.join(FILES[index])))
    }

    fn set_error(&mut self, which: usize, result: Result<(), String>) -> bool {
        let new = result.err().map(|e| {
            tracing::warn!("{e}");
            SharedString::from(e)
        });
        let changed = self.errors[which] != new;
        self.errors[which] = new;
        changed
    }

    /// Reload `settings.toml`; a broken file keeps the last good settings. True if anything changed.
    fn load_settings(&mut self) -> bool {
        match load_settings(&self.config_dir) {
            Ok(settings) => {
                let changed = settings != self.settings;
                self.settings = settings;
                self.set_error(SETTINGS, Ok(())) || changed
            }
            Err(e) => self.set_error(SETTINGS, Err(e.to_string())),
        }
    }

    pub fn load_views(&mut self) -> bool {
        match load_views(&self.config_dir, &self.home) {
            Ok(views) => {
                self.views = views;
                self.views_generation += 1;
                self.set_error(VIEWS, Ok(()));
                true
            }
            Err(e) => self.set_error(VIEWS, Err(e)),
        }
    }

    pub fn load_sidebar(&mut self) -> bool {
        match load_sidebar(&self.config_dir) {
            Ok(sidebar) => {
                let changed = sidebar != self.sidebar;
                self.sidebar = sidebar;
                self.set_error(SIDEBAR, Ok(())) || changed
            }
            Err(e) => self.set_error(SIDEBAR, Err(e)),
        }
    }

    /// Reload `exclude.toml`; a broken file keeps the last good rules.
    pub fn load_exclude(&mut self) -> bool {
        match crow_core::load_exclude(&self.config_dir) {
            Ok(exclude) => {
                let changed = exclude != self.exclude;
                self.exclude = exclude;
                self.set_error(EXCLUDE, Ok(()));
                if changed {
                    self.exclude_generation += 1;
                }
                changed
            }
            Err(e) => self.set_error(EXCLUDE, Err(e)),
        }
    }

    pub fn favorites(&self) -> Vec<(String, Vec<crow_core::Favorite>)> {
        self.sidebar.favorites(&self.home)
    }
}

struct Watchers {
    theme_rx: Receiver<()>,
    config_rx: Receiver<()>,
    mounts_rx: Receiver<()>,
    _theme: Option<RecommendedWatcher>,
    _config: Option<RecommendedWatcher>,
    _mounts: Option<crow_macos::MountWatcher>,
}

impl Global for Watchers {}

/// Install the `Ui` global and start watching the theme index, `~/.config/crow` and mounts.
pub fn init(cx: &mut App) {
    let home = crow_core::home();
    let config_dir = crow_core::config_dir();
    // The config dir is watched, so it has to exist before the first file is written.
    std::fs::create_dir_all(&config_dir).ok();
    let mut ui = Ui::new(config_dir.clone(), crow_core::state_dir(), home, crow_theme::fallback(), true);
    let theme = crow_theme::current(ui.settings.theme_id(), variant_of(cx.window_appearance()));
    tracing::info!(theme = %theme.name, "starting");
    ui.target = theme.clone();
    ui.theme = theme;
    cx.set_global(ui);
    crate::usage::init(cx);

    let (theme_tx, theme_rx) = mpsc::channel();
    let theme_watcher = crow_theme::watch(theme_tx).inspect_err(|e| tracing::warn!("theme watch: {e}")).ok();
    let (config_tx, config_rx) = mpsc::channel();
    let config_watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            config_tx.send(()).ok();
        }
    })
    .and_then(|mut w| w.watch(&config_dir, RecursiveMode::NonRecursive).map(|()| w))
    .inspect_err(|e| tracing::warn!("config watch: {e}"))
    .ok();
    let (mounts_tx, mounts_rx) = mpsc::channel();
    let mounts = crow_macos::watch_mounts(move || {
        mounts_tx.send(()).ok();
    });
    cx.set_global(Watchers {
        theme_rx,
        config_rx,
        mounts_rx,
        _theme: theme_watcher,
        _config: config_watcher,
        _mounts: mounts,
    });

    refresh_volumes(cx);
    load_zoxide(cx);
    // Polling keeps every reload on the main thread; 100 ms doubles as the debounce.
    cx.spawn(async move |cx| {
        let mut ticks = 0u32;
        loop {
            cx.background_executor().timer(Duration::from_millis(100)).await;
            ticks += 1;
            // Free space changes without a notification; a minute is fresh enough.
            let minute = ticks.is_multiple_of(600);
            cx.update(|cx| {
                poll(cx);
                if minute {
                    refresh_volumes(cx);
                }
            });
        }
    })
    .detach();
}

/// Default settings and the embedded theme, with config files in a fresh temp folder
/// and nothing watched.
#[cfg(test)]
pub fn init_for_tests(cx: &mut App) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "crow-app-config-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::remove_dir_all(&root).ok();
    let ui = Ui::new(root.join("config"), root.join("state"), crow_core::home(), crow_theme::fallback(), false);
    cx.set_global(ui);
    crate::extensions::init(cx);
    crate::usage::init(cx);
    root.join("config")
}

fn poll(cx: &mut App) {
    let watchers = cx.global::<Watchers>();
    let config_changed = watchers.config_rx.try_iter().count() > 0;
    let theme_changed = watchers.theme_rx.try_iter().count() > 0;
    let mounts_changed = watchers.mounts_rx.try_iter().count() > 0;
    if config_changed {
        reload_config(cx);
    }
    if config_changed || theme_changed {
        reload_theme(cx);
    }
    if mounts_changed {
        refresh_volumes(cx);
    }
}

/// Reload every config file. Views always count as changed (cheap, and panes only
/// re-apply what differs); the others only when their contents did.
pub fn reload_config(cx: &mut App) {
    let ui = cx.global_mut::<Ui>();
    let settings = ui.load_settings();
    ui.load_views();
    let sidebar = ui.load_sidebar();
    let exclude = ui.load_exclude();
    if settings || sidebar || exclude {
        tracing::info!("config reloaded");
    }
    if sidebar {
        refresh_recent(cx);
    }
    cx.refresh_windows();
}

/// Re-resolve the theme (after an index change, a settings change, an appearance
/// flip or a move in the theme picker). A broken theme file keeps the current theme.
pub fn reload_theme(cx: &mut App) {
    let variant = variant_of(cx.window_appearance());
    let (dir, state) = (crow_theme::themes_dir(), crow_theme::state_dir());
    let ui = cx.global::<Ui>();
    let saved = match &ui.preview_theme {
        Some(id) => Some(id.clone()),
        None => ui.settings.theme_id().map(str::to_owned),
    };
    let loaded = crow_theme::resolve_theme_id(saved.as_deref(), variant, &dir, &state)
        .ok_or_else(|| "no themes in the index".to_owned())
        .and_then(|id| crow_theme::load_theme(&dir, &id));
    match loaded {
        Ok(theme) => set_theme(theme, cx),
        Err(e) => tracing::warn!("theme reload skipped: {e}"),
    }
}

/// Show `theme`, fading from the colors on screen where they differ.
pub fn set_theme(theme: Theme, cx: &mut App) {
    let ui = cx.global_mut::<Ui>();
    if theme.id == ui.target.id && theme.same_colors(&ui.target) {
        return;
    }
    tracing::info!(theme = %theme.name, "theme changed");
    let fading = ui.fade.is_some();
    if ui.fade_enabled && !theme.same_colors(&ui.theme) {
        let duration = if crow_macos::reduce_motion() { FADE_REDUCED_MOTION } else { FADE };
        // A change mid-fade starts from the colors on screen, not the old theme.
        ui.fade = Some((ui.theme.clone(), Instant::now(), duration));
        ui.target = theme;
        if !fading {
            run_fade(cx);
        }
    } else {
        ui.fade = None;
        ui.target = theme.clone();
        ui.theme = theme;
        cx.refresh_windows();
    }
}

fn run_fade(cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            let done = cx.update(|cx| {
                let ui = cx.global_mut::<Ui>();
                let Some((from, start, duration)) = &ui.fade else { return true };
                let t = (start.elapsed().as_secs_f32() / duration.as_secs_f32()).min(1.0);
                // Ease-out cubic: quick to start, gentle to land.
                let eased = 1.0 - (1.0 - t).powi(3);
                ui.theme = from.mix(&ui.target, eased);
                if t >= 1.0 {
                    ui.fade = None;
                    ui.theme = ui.target.clone();
                }
                cx.refresh_windows();
                t >= 1.0
            });
            if done {
                break;
            }
            cx.background_executor().timer(Duration::from_millis(16)).await;
        }
    })
    .detach();
}

/// The theme to apply after the picker: write it to `settings.toml` and show it.
pub fn choose_theme(id: &str, cx: &mut App) {
    let ui = cx.global_mut::<Ui>();
    ui.preview_theme = None;
    ui.settings.theme = id.to_owned();
    let id = id.to_owned();
    write_config(SETTINGS, move |config, _| crow_core::save_setting(config, "theme", &id), cx);
    reload_theme(cx);
}

/// "Apply to all apps": make `theme` the global choice for its variant and follow it here.
pub fn apply_theme_everywhere(theme: &Theme, cx: &mut App) {
    if let Err(e) = crow_theme::set_global(theme) {
        // Not a file problem, so it belongs with the operation messages (no Open button).
        crate::operations::message(format!("Can't apply the theme to all apps: {e}"), true, cx);
        return;
    }
    choose_theme("system", cx);
}

/// Record a visit to `dir` for Recent, and save it.
pub fn record_visit(dir: &Path, cx: &mut App) {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    cx.global_mut::<Ui>().recent.visit(dir, now);
    refresh_recent(cx);
}

/// Remember a command run from the palette and save it to the state dir in the
/// background (the in-memory list updates at once). Does nothing when
/// `palette_recent_commands` is off.
pub fn record_command(id: crate::palette::CommandId, cx: &mut App) {
    let ui = cx.global::<Ui>();
    if !ui.settings.palette_recent_commands {
        return;
    }
    let mut recent = ui.recent_commands.clone();
    // Commands this build no longer has leave the list for good on the next save.
    recent.retain(|key| crate::palette::CommandId::from_key(key).is_some());
    recent.push(id.key(), crate::palette::RECENT_COMMANDS);
    let file = ui.state_dir.join("palette-commands.json");
    cx.global_mut::<Ui>().recent_commands = recent.clone();
    let task = cx.background_spawn(async move {
        if let Err(e) = recent.save(&file) {
            tracing::warn!("{}: {e}", file.display());
        }
    });
    task.detach();
    cx.refresh_windows();
}

/// Re-read `palette-commands.json` into `Ui`, as a relaunch would (tests only).
#[cfg(test)]
pub fn reload_state_for_tests(cx: &mut App) {
    let file = cx.global::<Ui>().state_dir.join("palette-commands.json");
    cx.global_mut::<Ui>().recent_commands = RecentCommands::load(&file);
}

/// Recompute Recent's list off the main thread (it checks each folder still exists)
/// and save the visit history.
pub fn refresh_recent(cx: &mut App) {
    let ui = cx.global::<Ui>();
    let recent = ui.recent.clone();
    let zoxide = ui.zoxide.clone();
    let sidebar_count = ui.sidebar.recent.count;
    // The palette wants more folders than the sidebar shows; keep favorites in this
    // list and filter them out only for the sidebar's rows.
    let count = sidebar_count.max(50);
    let favorites: Vec<PathBuf> = ui.favorites().into_iter().flat_map(|(_, f)| f).map(|f| f.path).collect();
    let file = ui.state_dir.join("recent.json");
    let task = cx.background_spawn(async move {
        if let Err(e) = recent.save(&file) {
            tracing::warn!("{}: {e}", file.display());
        }
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        recent.top(count, now, &zoxide, |p| p != Path::new("/") && p.is_dir())
    });
    cx.spawn(async move |cx| {
        let list = task.await;
        cx.update(|cx| {
            let ui = cx.global_mut::<Ui>();
            let sidebar: Vec<PathBuf> =
                list.iter().filter(|path| !favorites.contains(path)).take(sidebar_count).cloned().collect();
            let changed = ui.recent_list != sidebar || ui.palette_recents != list;
            ui.recent_list = sidebar;
            ui.palette_recents = list;
            if changed {
                cx.refresh_windows();
            }
        });
    })
    .detach();
}

fn load_zoxide(cx: &mut App) {
    let home = cx.global::<Ui>().home.clone();
    let task = cx.background_spawn(async move { crow_core::query_zoxide(&home) });
    cx.spawn(async move |cx| {
        let zoxide = task.await;
        cx.update(|cx| {
            cx.global_mut::<Ui>().zoxide = zoxide;
            refresh_recent(cx);
        });
    })
    .detach();
}

pub fn refresh_volumes(cx: &mut App) {
    let task = cx.background_spawn(async { crow_macos::volumes() });
    cx.spawn(async move |cx| {
        let volumes = task.await;
        cx.update(|cx| {
            let ui = cx.global_mut::<Ui>();
            if ui.volumes != volumes {
                ui.volumes = volumes;
                cx.refresh_windows();
            }
        });
    })
    .detach();
}

/// Write a config file off the main thread, then reload it here. A failure shows
/// in the status line like any other config problem.
fn write_config(which: usize, write: impl FnOnce(&Path, &Path) -> Result<(), String> + Send + 'static, cx: &mut App) {
    let ui = cx.global::<Ui>();
    let (config_dir, home) = (ui.config_dir.clone(), ui.home.clone());
    let task = cx.background_spawn(async move { write(&config_dir, &home) });
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| {
            let ui = cx.global_mut::<Ui>();
            match result {
                Ok(()) => match which {
                    VIEWS => {
                        ui.load_views();
                    }
                    SIDEBAR => {
                        ui.load_sidebar();
                        refresh_recent(cx);
                    }
                    EXCLUDE => {
                        ui.load_exclude();
                    }
                    _ => {
                        ui.load_settings();
                    }
                },
                Err(e) => {
                    ui.set_error(which, Err(e));
                }
            }
            cx.refresh_windows();
        });
    })
    .detach();
}

/// Remember a view change as `dir`'s own setting.
pub fn save_view_change(dir: PathBuf, change: crow_core::ViewChange, cx: &mut App) {
    write_config(VIEWS, move |config, home| crow_core::save_folder_change(config, home, &dir, change), cx);
}

/// Forget `dir`'s own view setting.
pub fn reset_view(dir: PathBuf, cx: &mut App) {
    write_config(VIEWS, move |config, home| crow_core::reset_folder(config, home, &dir), cx);
}

/// Make `dir`'s view the default for every folder.
pub fn use_view_as_defaults(dir: PathBuf, cx: &mut App) {
    let resolved = cx.global::<Ui>().views.resolve(&dir);
    write_config(VIEWS, move |config, home| crow_core::save_as_defaults(config, home, &dir, &resolved), cx);
}

/// Change the favorites in `sidebar.toml`.
pub fn edit_sidebar(edit: crow_core::SidebarEdit, cx: &mut App) {
    write_config(SIDEBAR, move |config, home| crow_core::edit_sidebar(config, home, &edit), cx);
}
