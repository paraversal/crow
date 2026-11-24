//! App-wide UI state: settings, view settings, the sidebar's configuration,
//! recent folders, volumes and the theme, reloaded live for every window.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crow_core::motion::{Effective, MotionMode, SoundSet};
use crow_core::{
    EnterAction, Exclude, FolderSizesMode, HintsMode, KeymapLayer, PathBarMode, Recent, RecentCommands, SavedQueries,
    SavedQueryId, SearchMatch, Settings, SidebarConfig, TipsMode, Views, load_saved_queries, load_settings,
    load_sidebar, load_views,
};
use crow_theme::Theme;
use futures::{FutureExt, future::Shared};
use gpui::{App, AppContext, Global, SharedString, Task};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::feedback::{self, Feedback};
use crate::style::variant_of;

const FADE: Duration = Duration::from_millis(500);
const FADE_REDUCED_MOTION: Duration = Duration::from_millis(150);

#[derive(Clone)]
enum ConfigSnapshot {
    Settings(Settings),
    Views(Views),
    SavedQueries(SavedQueries),
}

type ConfigWrite = Shared<Task<Result<ConfigSnapshot, String>>>;

/// The sidebar's last-known result count for one saved query. A running count
/// can be partial; inactive counts keep their last value until refreshed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SavedQueryCount {
    pub count: u64,
    pub partial: bool,
    pub searching: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SavedQueryCounts {
    counts: HashMap<SavedQueryId, SavedQueryCount>,
}

impl SavedQueryCounts {
    pub fn set(&mut self, id: SavedQueryId, count: u64, partial: bool, searching: bool) {
        let entry = self.counts.entry(id).or_default();
        entry.count = count;
        entry.partial = partial;
        entry.searching = searching;
    }

    /// The row's right-hand label: "12", "12+" when capped, "12…" while a rerun
    /// is streaming, or "…" before the first result.
    pub fn label(&self, id: &SavedQueryId) -> Option<String> {
        let count = self.counts.get(id)?;
        let mut label = format!("{}{}", count.count, if count.partial { "+" } else { "" });
        if count.searching {
            label.push('…');
        }
        Some(label)
    }

    pub fn retain(&mut self, ids: &[SavedQueryId]) {
        self.counts.retain(|id, _| ids.contains(id));
    }
}

pub struct Ui {
    pub settings: Settings,
    /// The motion mode and sound set in effect, and macOS's UI-sounds switch.
    pub feedback: Feedback,
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
    /// Newest started reload of each kind; a reply from an older one is dropped.
    config_reload_generation: u64,
    /// Bumped when a write of each config file is queued and again when it lands; a
    /// reload whose copy of the epoch is older read the file before that write.
    write_epoch: [u64; KEYS + 1],
    keys_reload_generation: u64,
    theme_reload_generation: u64,
    /// The user's key overrides (`keys.toml`); a broken file keeps the last good set.
    pub keys: crow_core::keymap_edit::KeyOverrides,
    /// Saved searches, reloaded live with the other config. A broken file keeps
    /// the last good set; open destinations rerun against the new specs.
    pub saved_queries: SavedQueries,
    /// Counts `saved-queries.toml` reloads that changed it.
    pub saved_queries_generation: u64,
    /// Sidebar result counts, streamed by the active destination and refreshed
    /// for visible inactive rows.
    pub saved_query_counts: SavedQueryCounts,
    /// Finder tags the sidebar shows: configured pinned first, then discovered.
    pub tags: crate::tags::TagIndex,
    pub recent: Recent,
    /// Commands run from the palette, newest first (state, not config).
    pub recent_commands: RecentCommands,
    zoxide: Vec<(f64, PathBuf)>,
    /// The folders Recent shows, best first (computed off the main thread).
    pub recent_list: Vec<PathBuf>,
    /// Files set aside on the shelf (state folder, `shelf.json`).
    pub shelf: crow_core::Shelf,
    /// More of the same ranking for the command palette (favorites included).
    pub palette_recents: Vec<PathBuf>,
    pub volumes: Vec<crow_macos::Volume>,
    /// Sidebar rows whose path is a symlink, filled by the Recent refresh.
    pub aliases: HashSet<PathBuf>,
    /// Latest background refresh of Recent / volumes; an older reply that lands
    /// after a newer one started is dropped.
    pub recent_refresh: u64,
    pub volumes_refresh: u64,
    pub themes_dir: PathBuf,
    pub themes_state_dir: PathBuf,
    /// The saved theme id last reported as missing, so the notice shows once.
    pub missing_theme_notice: Option<String>,
    /// Whether windows are blurred and the sidebar translucent right now.
    pub translucent: bool,
    /// View ▸ Hide Sidebar: the left column is collapsed in every window. Not saved.
    pub sidebar_hidden: bool,
    /// The last theme load error shown, so it isn't repeated on every reload.
    pub theme_error: Option<String>,
    pub config_dir: PathBuf,
    config_writes: [Option<ConfigWrite>; SAVED_QUERIES + 1],
    config_write_revision: [u64; SAVED_QUERIES + 1],
    pub state_dir: PathBuf,
    pub home: PathBuf,
    /// The Settings category last chosen, read once at startup. The window maps
    /// an unknown id to Appearance.
    pub settings_category: Option<String>,
    /// The focused folder of the browser that last invoked Settings, for the
    /// preview editor's Use Current Folder action.
    pub settings_folder: Option<PathBuf>,
    settings_category_write: Option<Shared<Task<Result<(), String>>>>,
    /// The last queued write of each config or state file, so the next write to the
    /// same file waits for it instead of racing it.
    file_writes: HashMap<&'static str, Shared<Task<Result<(), String>>>>,
    /// One message per config file that doesn't load; shown in the status line until fixed.
    errors: [Option<SharedString>; 6],
}

/// A config file problem as the Settings window lists it, with the config
/// directory stripped so rows read "views.toml line 12: …".
#[derive(Clone, Debug, PartialEq)]
pub struct ConfigIssue {
    pub file: &'static str,
    pub path: PathBuf,
    pub message: SharedString,
    pub line: Option<u32>,
}

/// Every config problem the app knows, in the order Settings lists them: the
/// five files Crow loads itself, then `plugins.toml` from the plugin host and the
/// icon pack `pack.toml` that failed to load. The status bar and Settings both
/// read this one list, so a problem shows in both and clears from both.
pub fn config_issues(cx: &App) -> Vec<ConfigIssue> {
    let ui = cx.global::<Ui>();
    let mut issues = ui.config_issues();
    if let Some(error) =
        cx.try_global::<crate::plugins::Plugins>().and_then(|plugins| plugins.inventory.grants_error.clone())
    {
        let message = ui.without_config_dir(&error);
        issues.push(ConfigIssue {
            file: "plugins.toml",
            path: ui.config_dir.join("plugins.toml"),
            line: error_line(&message),
            message,
        });
    }
    if let Some(packs) = cx.try_global::<crate::icon_packs::IconPacks>()
        && let Some(error) = packs.error()
    {
        let path = packs.error_file().map_or_else(|| ui.config_dir.join("icon-packs"), Path::to_path_buf);
        let message = ui.without_config_dir(error);
        issues.push(ConfigIssue { file: "pack.toml", path, line: error_line(&message), message });
    }
    issues
}

/// The first problem and the file it names, for the status bar's pill.
pub fn config_error_file(cx: &App) -> Option<(SharedString, PathBuf)> {
    let issue = config_issues(cx).into_iter().next()?;
    Some((issue.message, issue.path))
}

const CONFIG_FILES: [&str; 6] =
    ["settings.toml", "views.toml", "sidebar.toml", "exclude.toml", "saved-queries.toml", "keys.toml"];

impl Global for Ui {}

const SETTINGS: usize = 0;
const VIEWS: usize = 1;
const SIDEBAR: usize = 2;
const EXCLUDE: usize = 3;
const SAVED_QUERIES: usize = 4;
const KEYS: usize = 5;

impl Ui {
    fn new(config_dir: PathBuf, state_dir: PathBuf, home: PathBuf, theme: Theme, fade_enabled: bool) -> Ui {
        let recent = Recent::load(&state_dir.join("recent.json"));
        let shelf = crow_core::Shelf::load(&state_dir.join("shelf.json"));
        let recent_commands = RecentCommands::load(&state_dir.join("palette-commands.json"));
        // io-ok: startup: the last Settings category is one small read before the first window
        let settings_category = std::fs::read_to_string(state_dir.join("settings-category"))
            .ok()
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty());
        let mut ui = Ui {
            settings: Settings::default(),
            feedback: Feedback::default(),
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
            config_reload_generation: 0,
            write_epoch: Default::default(),
            keys_reload_generation: 0,
            theme_reload_generation: 0,
            keys: Default::default(),
            recent,
            recent_commands,
            zoxide: Vec::new(),
            recent_list: Vec::new(),
            shelf,
            palette_recents: Vec::new(),
            volumes: Vec::new(),
            aliases: HashSet::new(),
            recent_refresh: 0,
            volumes_refresh: 0,
            themes_dir: crow_theme::themes_dir(),
            themes_state_dir: crow_theme::state_dir(),
            missing_theme_notice: None,
            translucent: false,
            sidebar_hidden: false,
            theme_error: None,
            config_dir,
            config_writes: Default::default(),
            config_write_revision: Default::default(),
            state_dir,
            home,
            settings_category,
            settings_folder: None,
            settings_category_write: None,
            file_writes: HashMap::new(),
            saved_queries: SavedQueries::default(),
            saved_queries_generation: 0,
            saved_query_counts: SavedQueryCounts::default(),
            tags: crate::tags::TagIndex::default(),
            errors: Default::default(),
        };
        // io-ok: startup, before a window exists
        ui.load_settings();
        ui.load_views(); // io-ok: startup
        ui.load_sidebar(); // io-ok: startup
        ui.load_exclude(); // io-ok: startup
        ui.load_keys(); // io-ok: startup
        ui.apply_loaded_saved_queries(load_saved_queries(&ui.config_dir, &ui.home));
        ui.feedback = Feedback::read(&ui.settings);
        ui
    }

    /// The first config file problem, if any. Tests read it; the bar uses the file form.
    #[cfg(test)]
    pub fn config_error(&self) -> Option<SharedString> {
        self.errors.iter().flatten().next().cloned()
    }

    /// How long the running theme fade lasts, for tests.
    #[cfg(test)]
    pub fn fade_duration(&self) -> Option<Duration> {
        self.fade.as_ref().map(|(_, _, duration)| *duration)
    }

    /// One config file's problem, with the config dir stripped so the window
    /// can say "settings.toml line 3: …".
    pub fn file_error(&self, file: &str) -> Option<SharedString> {
        let index = CONFIG_FILES.iter().position(|name| *name == file)?;
        Some(self.without_config_dir(self.errors[index].as_ref()?))
    }

    fn without_config_dir(&self, message: &str) -> SharedString {
        let dir = self.config_dir.display().to_string();
        message.strip_prefix(&dir).map_or(message, |rest| rest.trim_start_matches('/')).to_owned().into()
    }

    /// A broken `settings.toml`, with the config dir stripped so the settings
    /// window can say "settings.toml line 3: …".
    pub fn settings_error(&self) -> Option<SharedString> {
        self.file_error("settings.toml")
    }

    /// A broken `views.toml`; the view controls that write it are disabled.
    pub fn views_error(&self) -> Option<SharedString> {
        self.file_error("views.toml")
    }

    /// Every file-keyed problem, in file order, for the Settings error region.
    pub fn config_issues(&self) -> Vec<ConfigIssue> {
        CONFIG_FILES
            .iter()
            .filter_map(|file| {
                let message = self.file_error(file)?;
                Some(ConfigIssue { file, path: self.config_dir.join(file), line: error_line(&message), message })
            })
            .collect()
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
        if self.config_writes[SETTINGS].is_some() {
            return false;
        }
        // io-ok: startup; reloads read in the background
        self.apply_loaded_settings(load_settings(&self.config_dir).map_err(|error| error.to_string()))
    }

    fn apply_loaded_settings(&mut self, result: Result<Settings, String>) -> bool {
        match result {
            Ok(settings) => {
                let changed = settings != self.settings;
                self.settings = settings;
                self.set_error(SETTINGS, Ok(())) || changed
            }
            Err(error) => self.set_error(SETTINGS, Err(error)),
        }
    }

    pub fn load_views(&mut self) -> bool {
        if self.config_writes[VIEWS].is_some() {
            return false;
        }
        // io-ok: startup; reloads read in the background
        self.apply_loaded_views(load_views(&self.config_dir, &self.home))
    }

    fn apply_loaded_views(&mut self, result: Result<Views, String>) -> bool {
        match result {
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
        self.apply_loaded_sidebar(load_sidebar(&self.config_dir)) // io-ok: startup; reloads read in the background
    }

    fn apply_loaded_sidebar(&mut self, result: Result<crow_core::SidebarConfig, String>) -> bool {
        match result {
            Ok(sidebar) => {
                let changed = sidebar != self.sidebar;
                self.tags.set_pinned(sidebar.pinned_tags.clone());
                self.sidebar = sidebar;
                self.set_error(SIDEBAR, Ok(())) || changed
            }
            Err(e) => self.set_error(SIDEBAR, Err(e)),
        }
    }

    /// Reload `keys.toml`; a broken file keeps the last good keys. True if the keys changed.
    pub fn load_keys(&mut self) -> bool {
        self.apply_loaded_keys(read_keys(&self.config_dir)) // io-ok: startup; reloads read in the background
    }

    fn apply_loaded_keys(&mut self, result: Result<crow_core::keymap_edit::KeyOverrides, String>) -> bool {
        match result {
            Ok(keys) => {
                let changed = keys != self.keys;
                self.keys = keys;
                self.set_error(KEYS, Ok(()));
                changed
            }
            Err(e) => {
                self.set_error(KEYS, Err(e));
                false
            }
        }
    }

    /// Reload `exclude.toml`; a broken file keeps the last good rules.
    pub fn load_exclude(&mut self) -> bool {
        self.apply_loaded_exclude(crow_core::load_exclude(&self.config_dir)) // io-ok: startup; reloads read in the background
    }

    fn apply_loaded_exclude(&mut self, result: Result<crow_core::Exclude, String>) -> bool {
        match result {
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

    /// Reload `saved-queries.toml`; a broken file keeps the last good set.
    fn apply_loaded_saved_queries(&mut self, result: Result<SavedQueries, String>) -> bool {
        match result {
            Ok(queries) => {
                let changed = queries != self.saved_queries;
                let ids: Vec<SavedQueryId> = queries.queries.iter().map(|query| query.id.clone()).collect();
                self.saved_query_counts.retain(&ids);
                self.saved_queries = queries;
                self.set_error(SAVED_QUERIES, Ok(()));
                if changed {
                    self.saved_queries_generation += 1;
                }
                changed
            }
            Err(e) => self.set_error(SAVED_QUERIES, Err(e)),
        }
    }

    pub fn favorites(&self) -> Vec<(String, Vec<crow_core::Favorite>)> {
        self.sidebar.favorites(&self.home)
    }
}

/// The line from a message like "views.toml line 12: unknown field".
fn error_line(message: &str) -> Option<u32> {
    let (_, rest) = message.split_once(" line ")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

struct Watchers {
    theme_rx: Receiver<()>,
    config_rx: Receiver<()>,
    icon_rx: Receiver<()>,
    mounts_rx: Receiver<()>,
    _theme: Option<crow_theme::ThemeWatcher>,
    colors_rx: Receiver<()>,
    _colors: Option<crow_macos::SystemColorWatcher>,
    _config: Option<RecommendedWatcher>,
    _icon: Option<RecommendedWatcher>,
    _mounts: Option<crow_macos::MountWatcher>,
}

impl Global for Watchers {}

/// Install the `Ui` global and start watching the theme index, `~/.config/crow` and mounts.
pub fn init(cx: &mut App) {
    init_at(cx, crow_core::config_dir(), crow_core::state_dir(), crow_core::home(), crow_core::cache_dir());
}

/// `init` with explicit folders, for `crow --fuzz`: its config, state, home and
/// cache stay in the run's temp folders, so it cannot read the user's sidebar or
/// write Recent or folder sizes.
pub fn init_at(cx: &mut App, config_dir: PathBuf, state_dir: PathBuf, home: PathBuf, cache_dir: PathBuf) {
    // The config dir is watched, so it has to exist before the first file is written.
    // io-ok: startup: the config dir must exist before it is watched
    std::fs::create_dir_all(&config_dir).ok();
    // Packs are optional, but the folder exists so the recursive watcher can attach
    // and adding a pack needs no restart.
    let icon_packs_dir = config_dir.join("icon-packs");
    // io-ok: startup: the pack folder must exist before it is watched
    std::fs::create_dir_all(&icon_packs_dir).ok();
    let mut ui = Ui::new(config_dir.clone(), state_dir, home, crow_theme::fallback(), true);
    let theme = crow_theme::current(ui.settings.theme_id(), variant_of(cx.window_appearance()));
    let theme = resolve_system_colors(theme);
    tracing::info!(theme = %theme.name, "starting");
    ui.target = theme.clone();
    ui.theme = theme;
    ui.translucent = translucency_wanted(&ui.settings);
    cx.set_global(ui);
    cx.on_app_quit(|cx| {
        let pending = cx.global::<Ui>().config_writes.clone();
        async move {
            for pending in pending.into_iter().flatten() {
                let _ = pending.await;
            }
        }
    })
    .detach();
    crate::folder_sizes::init_at(cx, cache_dir, true);
    crate::cloud_status::init_at(cx);
    crate::thumbnails::init(cx);
    crate::git::init_at(cx);
    crate::icon_packs::install(cx);
    let feedback = cx.global::<Ui>().feedback;
    cx.set_reduce_motion(feedback.mode == Effective::Off);
    feedback::preload(feedback.sound, cx);
    // After the first frame: decoding the icon is not worth delaying first paint for.
    cx.defer(|cx| crate::effects::effects(cx).set_app_icon(APP_ICON_PNG));
    // A saved theme that no longer exists is said once, which needs the message list.
    cx.defer(reload_theme);
    crate::usage::init(cx);
    crate::tips::init(cx);
    crate::onboarding::init(cx);
    crate::tags::init(cx);

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
    let (icon_tx, icon_rx) = mpsc::channel();
    let icon_watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            icon_tx.send(()).ok();
        }
    })
    .and_then(|mut w| w.watch(&icon_packs_dir, RecursiveMode::Recursive).map(|()| w))
    .inspect_err(|e| tracing::warn!("icon pack watch: {e}"))
    .ok();
    let (mounts_tx, mounts_rx) = mpsc::channel();
    let mounts = crow_macos::watch_mounts(move || {
        mounts_tx.send(()).ok();
    });
    let (colors_tx, colors_rx) = mpsc::channel();
    let colors = crow_macos::watch_system_colors(move || {
        colors_tx.send(()).ok();
    });
    cx.set_global(Watchers {
        colors_rx,
        _colors: colors,
        theme_rx,
        config_rx,
        icon_rx,
        mounts_rx,
        _theme: theme_watcher,
        _config: config_watcher,
        _icon: icon_watcher,
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
    install_drag_cancel(cx);
}

/// Escape ends an in-window drag and flies the combo back, like AppKit's cancel.
fn install_drag_cancel(cx: &mut App) {
    cx.intercept_keystrokes(|event, window, cx| {
        if event.keystroke.key == "escape" && window.cancel_active_drag(cx) {
            // A sprung folder goes back now, not on the next mouse move.
            if let Some(Some(root)) = window.root::<crate::window::CrowWindow>() {
                root.update(cx, |this, cx| this.drag_cancelled(window, cx));
            }
            cx.stop_propagation();
        }
    })
    .detach();
}

/// Owns every root `init_for_tests` hands out, for as long as the test's `App`
/// lives. Keeping the directories here rather than in `Ui` means the config
/// path returned to callers stays a plain path, while GPUI's teardown (after
/// windows, views and executors) removes the files. A second `init_for_tests`
/// in one app appends instead of replacing, so a live window's config is never
/// deleted underneath it.
#[cfg(test)]
#[derive(Default)]
struct TestRoots(Vec<tempfile::TempDir>);

#[cfg(test)]
impl gpui::Global for TestRoots {}

/// Run `work` (file I/O) on the background pool after every earlier write queued
/// under `key` has finished. Rapid edits of one file apply in the order made.
pub fn queue_file_write(
    key: &'static str,
    work: impl FnOnce() -> Result<(), String> + Send + 'static,
    cx: &mut App,
) -> Shared<Task<Result<(), String>>> {
    if key == "keys.toml" {
        cx.global_mut::<Ui>().write_epoch[KEYS] += 1;
    }
    let previous = cx.global_mut::<Ui>().file_writes.remove(key);
    let task = cx
        .background_spawn(async move {
            if let Some(previous) = previous {
                let _ = previous.await;
            }
            work()
        })
        .shared();
    cx.global_mut::<Ui>().file_writes.insert(key, task.clone());
    task
}

/// Like [`queue_file_write`] for saves nobody waits for; a failure is logged.
fn queue_state_save(
    key: &'static str,
    file: PathBuf,
    save: impl FnOnce(&Path) -> Result<(), String> + Send + 'static,
    cx: &mut App,
) {
    let task = queue_file_write(key, move || save(&file), cx);
    cx.spawn(async move |_| {
        if let Err(error) = task.await {
            tracing::warn!("{key}: {error}");
        }
    })
    .detach();
}

/// Change the shelf and save it to the state folder in the background.
pub fn edit_shelf(cx: &mut App, edit: impl FnOnce(&mut crow_core::Shelf)) {
    let ui = cx.global_mut::<Ui>();
    edit(&mut ui.shelf);
    let (shelf, file) = (ui.shelf.clone(), ui.state_dir.join("shelf.json"));
    queue_state_save(
        "shelf.json",
        file,
        move |file| shelf.save(file).map_err(|e| format!("{}: {e}", file.display())),
        cx,
    );
    cx.refresh_windows();
}

/// Default settings and the embedded theme, with config files in a fresh temp folder
/// and nothing watched.
#[cfg(test)]
pub fn init_for_tests(cx: &mut App) -> PathBuf {
    let root = tempfile::Builder::new().prefix("crow-app-config-").tempdir().expect("test config root");
    crate::effects::install(cx, std::sync::Arc::new(crate::effects::RecordingEffects::new()));
    let config_dir = root.path().join("config");
    let state_dir = root.path().join("state");
    // Tests start past the first-launch sheet unless they ask for it: a fresh
    // state dir would otherwise owe it in every window test.
    // io-ok: test-only state
    std::fs::create_dir_all(&state_dir).ok();
    // io-ok: test-only state
    std::fs::write(state_dir.join(crow_core::ONBOARDING_FILE), "{\"tips_choice\": true}\n").ok();
    // No theme index unless a test installs one: never the developer's own `~/.config/themes`.
    let (themes_dir, themes_state_dir) = (root.path().join("no-themes"), root.path().join("no-themes-state"));
    crate::clock::install_for_tests(cx.background_executor().clone());
    let mut ui = Ui::new(config_dir.clone(), state_dir, crow_core::home(), crow_theme::fallback(), false);
    ui.themes_dir = themes_dir;
    ui.themes_state_dir = themes_state_dir;
    cx.set_global(ui);
    cx.default_global::<TestRoots>().0.push(root);
    crate::extensions::init(cx);
    crate::icon_packs::install(cx);
    crate::cloud_status::init_for_tests(cx);
    crate::thumbnails::init_for_tests(cx);
    crate::git::init_for_tests(cx);
    crate::plugins::init_empty(cx);
    crate::usage::init(cx);
    crate::tips::init(cx);
    crate::onboarding::init(cx);
    // Tests set `Ui.tags` directly; scanning the real Spotlight index in every
    // GPUI test would be slow and machine-dependent.
    install_drag_cancel(cx);
    config_dir
}

/// Read `keys.toml` and check every command it names. Disk work: startup or a
/// background task.
fn read_keys(config_dir: &Path) -> Result<crow_core::keymap_edit::KeyOverrides, String> {
    let keys = crow_core::keymap_edit::load_keys(config_dir)?; // io-ok: startup or background task
    for command in keys.finder.keys().chain(keys.vim.keys()) {
        if crate::palette::CommandId::from_key(command).is_none() {
            return Err(format!("keys.toml: unknown command `{command}`"));
        }
    }
    Ok(keys)
}

fn poll(cx: &mut App) {
    let watchers = cx.global::<Watchers>();
    let config_changed = watchers.config_rx.try_iter().count() > 0;
    let icon_packs_changed = watchers.icon_rx.try_iter().count() > 0;
    let index_changed = watchers.theme_rx.try_iter().count() > 0;
    let colors_changed = watchers.colors_rx.try_iter().count() > 0;
    let theme_changed = index_changed || colors_changed;
    let mounts_changed = watchers.mounts_rx.try_iter().count() > 0;
    if config_changed {
        reload_config(cx);
    }
    if icon_packs_changed {
        crate::icon_packs::reload(cx);
    }
    crate::plugins::poll(cx);
    crate::git::poll(cx);
    // Reduce Motion and the UI-sounds switch can change without a config edit.
    refresh_feedback(cx);
    refresh_translucency(cx);
    refresh_clipboard(cx);
    if index_changed && let Some(watcher) = cx.global_mut::<Watchers>()._theme.as_mut() {
        // The index may have appeared or vanished: watch where it is now.
        watcher.refresh();
    }
    // A config edit that changes the theme reloads it once the new settings are applied.
    if theme_changed {
        reload_theme(cx);
    }
    if mounts_changed {
        refresh_volumes(cx);
    }
}

/// Drop Crow's clipboard when another app copied since: the cut rows stop dimming
/// and a paste uses the new pasteboard contents.
fn refresh_clipboard(cx: &mut App) {
    let count = crate::effects::effects(cx).pasteboard_change_count();
    let changed = {
        let ops = cx.global_mut::<crate::operations::Operations>();
        match &ops.clipboard {
            Some(clipboard) if clipboard.change_count != count => {
                ops.clipboard = None;
                true
            }
            _ => false,
        }
    };
    if changed {
        cx.refresh_windows();
    }
}

/// Recompute the effective motion mode and sound set from settings, Reduce Motion
/// and the UI-sounds switch, and apply them. In `off`, GPUI's own switch freezes
/// every `with_animation` (Low still needs fades, so it stays false there); a new
/// sound set is preloaded so its first play does no disk I/O.
pub fn refresh_feedback(cx: &mut App) {
    let (feedback, changed) = {
        let ui = cx.global_mut::<Ui>();
        let feedback = Feedback::read(&ui.settings);
        let changed = feedback != ui.feedback;
        ui.feedback = feedback;
        (feedback, changed)
    };
    if !changed {
        return;
    }
    cx.set_reduce_motion(feedback.mode == Effective::Off);
    feedback::preload(feedback.sound, cx);
    cx.refresh_windows();
}

/// Whether the sidebar should be translucent now: the setting, unless macOS Reduce
/// Transparency is on.
pub fn translucency_wanted(settings: &crow_core::Settings) -> bool {
    #[cfg(test)]
    {
        settings.translucent_sidebar
    }
    #[cfg(not(test))]
    {
        settings.translucent_sidebar && !crow_macos::reduce_transparency()
    }
}

/// Make every window's background match `translucent_sidebar`, if that changed since the last call.
pub fn refresh_translucency(cx: &mut App) {
    let want = translucency_wanted(&cx.global::<Ui>().settings);
    if cx.global::<Ui>().translucent == want {
        return;
    }
    cx.global_mut::<Ui>().translucent = want;
    for handle in cx.windows() {
        handle.update(cx, |_, window, _| window.set_background_appearance(window_background(want))).ok();
    }
    cx.refresh_windows();
}

/// The window background for a translucency choice: blurred only while the sidebar needs it.
pub fn window_background(translucent: bool) -> gpui::WindowBackgroundAppearance {
    if translucent { gpui::WindowBackgroundAppearance::Blurred } else { gpui::WindowBackgroundAppearance::Opaque }
}

/// Every config file as read off the main thread.
struct LoadedConfig {
    settings: Result<Settings, String>,
    views: Result<Views, String>,
    sidebar: Result<SidebarConfig, String>,
    exclude: Result<crow_core::Exclude, String>,
    keys: Result<crow_core::keymap_edit::KeyOverrides, String>,
    /// `Ui::write_epoch` when the read started.
    epochs: [u64; KEYS + 1],
}

/// Reload every config file: read and parse in the background, apply on the main
/// thread. A newer reload makes an older reply stale, and it is dropped. Views
/// always count as changed (cheap, and panes only re-apply what differs); the
/// others only when their contents did.
pub fn reload_config(cx: &mut App) {
    let ui = cx.global_mut::<Ui>();
    ui.config_reload_generation += 1;
    let generation = ui.config_reload_generation;
    let (config, home) = (ui.config_dir.clone(), ui.home.clone());
    let epochs = ui.write_epoch;
    let task = cx.background_spawn(async move {
        LoadedConfig {
            // io-ok: background task
            settings: load_settings(&config).map_err(|error| error.to_string()),
            // io-ok: background task
            views: load_views(&config, &home),
            // io-ok: background task
            sidebar: load_sidebar(&config),
            // io-ok: background task
            exclude: crow_core::load_exclude(&config),
            // io-ok: background task
            keys: read_keys(&config),
            epochs,
        }
    });
    cx.spawn(async move |cx| {
        let loaded = task.await;
        cx.update(|cx| {
            if cx.global::<Ui>().config_reload_generation == generation {
                apply_loaded_config(loaded, cx);
            }
        });
    })
    .detach();
}

fn apply_loaded_config(loaded: LoadedConfig, cx: &mut App) {
    let icon_before = cx.global::<Ui>().settings.icon_pack.clone();
    let ui = cx.global_mut::<Ui>();
    // A file written since the read started, or one with a write still queued, is
    // owned by that write; its own result applies.
    let LoadedConfig {
        settings: new_settings,
        views,
        sidebar: new_sidebar,
        exclude: new_exclude,
        keys: new_keys,
        epochs,
    } = loaded;
    let current = ui.write_epoch;
    let fresh = |which: usize| current[which] == epochs[which];
    let settings = fresh(SETTINGS) && ui.config_writes[SETTINGS].is_none() && ui.apply_loaded_settings(new_settings);
    let icon_changed = ui.settings.icon_pack != icon_before;
    if fresh(VIEWS) && ui.config_writes[VIEWS].is_none() {
        ui.apply_loaded_views(views);
    }
    let sidebar = fresh(SIDEBAR) && ui.apply_loaded_sidebar(new_sidebar);
    let exclude = fresh(EXCLUDE) && ui.apply_loaded_exclude(new_exclude);
    let keys = fresh(KEYS) && ui.apply_loaded_keys(new_keys);
    if settings || sidebar || exclude || keys {
        tracing::info!("config reloaded");
    }
    if sidebar {
        refresh_recent(cx);
    }
    if settings {
        // The theme id may have changed; `reload_theme` reads the settings applied just now.
        reload_theme(cx);
        crate::menus::refresh(cx);
        let show_hidden = cx.global::<Ui>().settings.show_hidden_files;
        crate::window::apply_show_hidden(show_hidden, cx);
        let search_includes_hidden = cx.global::<Ui>().settings.search_includes_hidden;
        crate::window::apply_search_includes_hidden(search_includes_hidden, cx);
    }
    if icon_changed {
        crate::icon_packs::reload(cx);
    }
    if keys {
        apply_keys(cx);
    }
    reload_saved_queries(cx);
    refresh_feedback(cx);
    refresh_translucency(cx);
    crate::plugins::reload(cx);
    cx.refresh_windows();
}

/// Rebind from `keys.toml` and refresh everything that shows a key.
fn apply_keys(cx: &mut App) {
    crate::keymap::install(cx);
    crate::menus::refresh(cx);
    cx.refresh_windows();
}

/// Reload `keys.toml` after Settings wrote it: read in the background, apply on
/// the main thread unless a newer reload started.
pub fn reload_keys(cx: &mut App) {
    let ui = cx.global_mut::<Ui>();
    // The write that called this has landed: a config reload that read keys.toml earlier is stale.
    ui.write_epoch[KEYS] += 1;
    ui.keys_reload_generation += 1;
    let generation = ui.keys_reload_generation;
    let config = ui.config_dir.clone();
    // io-ok: background task
    let task = cx.background_spawn(async move { read_keys(&config) });
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| {
            if cx.global::<Ui>().keys_reload_generation == generation {
                if cx.global_mut::<Ui>().apply_loaded_keys(result) {
                    apply_keys(cx);
                }
                cx.refresh_windows();
            }
        });
    })
    .detach();
}

/// The app icon, `assets/app-icon.png` at the repository root: the Dock shows it
/// while Crow runs, and `scripts/bundle.sh` and `scripts/site.sh` make every other
/// size from the same file.
pub const APP_ICON_PNG: &[u8] = include_bytes!("../../../assets/app-icon.png");

/// One change from the settings window or a menu toggle. `set_setting` applies it
/// to `Ui.settings` at once, then writes only that key to `settings.toml`.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingChange {
    Theme(String),
    Keymap(KeymapLayer),
    Enter(EnterAction),
    FolderTarget(crow_core::FolderTarget),
    ToolbarPins(Vec<String>),
    Hints(HintsMode),
    Tips(TipsMode),
    FolderSizes(FolderSizesMode),
    Motion(MotionMode),
    Sound(SoundSet),
    IconPack(String),
    PathBar(PathBarMode),
    IconStyle(crow_core::IconStyle),
    SidebarStyle(crow_core::SidebarStyle),
    FontFamily(String),
    FontSize(f32),
    Terminal(String),
    ShowHiddenFiles(bool),
    SearchIncludesHidden(bool),
    SearchMatch(SearchMatch),
    ShowPreview(bool),
    PaletteRecentCommands(bool),
    TranslucentSidebar(bool),
    ViewPerFolder(bool),
    WindowWidth(u32),
    WindowHeight(u32),
    SidebarWidth(u32),
}

/// Apply `change` in memory, update everything that follows from it, and write
/// the one key off the main thread. A write failure goes to the status line.
pub fn set_setting(change: SettingChange, cx: &mut App) {
    let changed = {
        let settings = &mut cx.global_mut::<Ui>().settings;
        match &change {
            SettingChange::Theme(value) => replace(&mut settings.theme, value.clone()),
            SettingChange::Keymap(value) => replace(&mut settings.keymap, *value),
            SettingChange::Enter(value) => replace(&mut settings.enter, *value),
            SettingChange::FolderTarget(value) => replace(&mut settings.folder_target, *value),
            SettingChange::ToolbarPins(value) => replace(&mut settings.toolbar_pins, value.clone()),
            SettingChange::Hints(value) => replace(&mut settings.hints, *value),
            SettingChange::Tips(value) => replace(&mut settings.tips, *value),
            SettingChange::FolderSizes(value) => replace(&mut settings.folder_sizes, *value),
            SettingChange::Motion(value) => replace(&mut settings.motion, *value),
            SettingChange::Sound(value) => replace(&mut settings.sound, *value),
            SettingChange::IconPack(value) => replace(&mut settings.icon_pack, value.clone()),
            SettingChange::PathBar(value) => replace(&mut settings.path_bar, *value),
            SettingChange::IconStyle(value) => replace(&mut settings.icon_style, *value),
            SettingChange::SidebarStyle(value) => replace(&mut settings.sidebar_style, *value),
            SettingChange::FontFamily(value) => replace(&mut settings.font_family, value.clone()),
            SettingChange::FontSize(value) => replace(&mut settings.font_size, *value),
            SettingChange::Terminal(value) => replace(&mut settings.terminal, value.clone()),
            SettingChange::ShowHiddenFiles(value) => replace(&mut settings.show_hidden_files, *value),
            SettingChange::SearchIncludesHidden(value) => replace(&mut settings.search_includes_hidden, *value),
            SettingChange::SearchMatch(value) => replace(&mut settings.search_match, *value),
            SettingChange::ShowPreview(value) => replace(&mut settings.show_preview, *value),
            SettingChange::PaletteRecentCommands(value) => replace(&mut settings.palette_recent_commands, *value),
            SettingChange::TranslucentSidebar(value) => replace(&mut settings.translucent_sidebar, *value),
            SettingChange::ViewPerFolder(value) => replace(&mut settings.view_per_folder, *value),
            SettingChange::WindowWidth(value) => replace(&mut settings.window.width, *value),
            SettingChange::WindowHeight(value) => replace(&mut settings.window.height, *value),
            SettingChange::SidebarWidth(value) => replace(&mut settings.window.sidebar_width, *value),
        }
    };
    if !changed {
        return;
    }
    match &change {
        SettingChange::Theme(_) => {
            cx.global_mut::<Ui>().preview_theme = None;
            reload_theme(cx);
        }
        SettingChange::Motion(_) | SettingChange::Sound(_) => refresh_feedback(cx),
        SettingChange::ShowHiddenFiles(show) => crate::window::apply_show_hidden(*show, cx),
        SettingChange::SearchIncludesHidden(include) => crate::window::apply_search_includes_hidden(*include, cx),
        // The View menu's checkmarks read both.
        SettingChange::ShowPreview(_) | SettingChange::FolderSizes(_) => crate::menus::refresh(cx),
        SettingChange::TranslucentSidebar(_) => refresh_translucency(cx),
        _ => {}
    }
    cx.refresh_windows();
    write_setting(change, cx);
}

fn replace<T: PartialEq>(slot: &mut T, value: T) -> bool {
    if *slot == value {
        return false;
    }
    *slot = value;
    true
}

fn write_setting(change: SettingChange, cx: &mut App) {
    match change {
        SettingChange::Theme(value) => {
            write_config(SETTINGS, move |config, _| crow_core::save_setting(config, "theme", &value), cx);
        }
        SettingChange::Keymap(value) => {
            write_config(SETTINGS, move |config, _| crow_core::save_setting(config, "keymap", value.key()), cx);
        }
        SettingChange::Enter(value) => {
            write_config(SETTINGS, move |config, _| crow_core::save_setting(config, "enter", value.key()), cx);
        }
        SettingChange::FolderTarget(value) => {
            write_config(SETTINGS, move |config, _| crow_core::save_setting(config, "folder_target", value.key()), cx);
        }
        SettingChange::ToolbarPins(value) => {
            write_config(
                SETTINGS,
                move |config, _| crow_core::save_string_list_setting(config, "toolbar_pins", &value),
                cx,
            );
        }
        SettingChange::Hints(value) => {
            write_config(SETTINGS, move |config, _| crow_core::save_setting(config, "hints", value.key()), cx);
        }
        SettingChange::Tips(value) => {
            write_config(SETTINGS, move |config, _| crow_core::save_setting(config, "tips", value.key()), cx);
        }
        SettingChange::FolderSizes(value) => {
            write_config(SETTINGS, move |config, _| crow_core::save_setting(config, "folder_sizes", value.key()), cx);
        }
        SettingChange::Motion(value) => {
            write_config(SETTINGS, move |config, _| crow_core::save_setting(config, "motion", value.key()), cx);
        }
        SettingChange::Sound(value) => {
            write_config(SETTINGS, move |config, _| crow_core::save_setting(config, "sound", value.key()), cx);
        }
        SettingChange::IconPack(value) => {
            write_config(SETTINGS, move |config, _| crow_core::save_setting(config, "icon_pack", &value), cx);
        }
        SettingChange::IconStyle(value) => {
            write_config(SETTINGS, move |config, _| crow_core::save_setting(config, "icon_style", value.key()), cx);
        }
        SettingChange::PathBar(value) => {
            write_config(SETTINGS, move |config, _| crow_core::save_setting(config, "path_bar", value.key()), cx);
        }
        SettingChange::SidebarStyle(value) => {
            write_config(SETTINGS, move |config, _| crow_core::save_setting(config, "sidebar_style", value.key()), cx);
        }
        SettingChange::FontFamily(value) => {
            write_config(SETTINGS, move |config, _| crow_core::save_setting(config, "font_family", &value), cx);
        }
        SettingChange::FontSize(value) => {
            write_config(
                SETTINGS,
                move |config, _| crow_core::save_number_setting(config, "font_size", f64::from(value)),
                cx,
            );
        }
        SettingChange::Terminal(value) => {
            write_config(SETTINGS, move |config, _| crow_core::save_setting(config, "terminal", &value), cx);
        }
        SettingChange::ShowHiddenFiles(value) => {
            write_config(
                SETTINGS,
                move |config, _| crow_core::save_bool_setting(config, "show_hidden_files", value),
                cx,
            );
        }
        SettingChange::SearchIncludesHidden(value) => {
            write_config(
                SETTINGS,
                move |config, _| crow_core::save_bool_setting(config, "search_includes_hidden", value),
                cx,
            );
        }
        SettingChange::SearchMatch(value) => {
            write_config(SETTINGS, move |config, _| crow_core::save_setting(config, "search_match", value.key()), cx);
        }
        SettingChange::ShowPreview(value) => {
            write_config(SETTINGS, move |config, _| crow_core::save_bool_setting(config, "show_preview", value), cx);
        }
        SettingChange::TranslucentSidebar(value) => {
            write_config(
                SETTINGS,
                move |config, _| crow_core::save_bool_setting(config, "translucent_sidebar", value),
                cx,
            );
        }
        SettingChange::PaletteRecentCommands(value) => {
            write_config(
                SETTINGS,
                move |config, _| crow_core::save_bool_setting(config, "palette_recent_commands", value),
                cx,
            );
        }
        SettingChange::ViewPerFolder(value) => {
            write_config(SETTINGS, move |config, _| crow_core::save_bool_setting(config, "view_per_folder", value), cx);
        }
        SettingChange::WindowWidth(value) => {
            write_config(
                SETTINGS,
                move |config, _| crow_core::save_table_number(config, "window", "width", i64::from(value)),
                cx,
            );
        }
        SettingChange::WindowHeight(value) => {
            write_config(
                SETTINGS,
                move |config, _| crow_core::save_table_number(config, "window", "height", i64::from(value)),
                cx,
            );
        }
        SettingChange::SidebarWidth(value) => {
            write_config(
                SETTINGS,
                move |config, _| crow_core::save_table_number(config, "window", "sidebar_width", i64::from(value)),
                cx,
            );
        }
    }
}

/// View ▸ Show/Hide Preview: remember the choice for every window and the next launch.
pub fn set_show_preview(show: bool, cx: &mut App) {
    set_setting(SettingChange::ShowPreview(show), cx);
}

/// Remember the chosen Settings category in the state folder. Writes are
/// serialized so rapid category switches leave the last choice on disk.
pub fn save_settings_category(id: &str, cx: &mut App) {
    let (state_dir, previous) = {
        let ui = cx.global_mut::<Ui>();
        if ui.settings_category.as_deref() == Some(id) {
            return;
        }
        ui.settings_category = Some(id.to_owned());
        (ui.state_dir.clone(), ui.settings_category_write.take())
    };
    let id = id.to_owned();
    let task = cx
        .background_spawn(async move {
            if let Some(previous) = previous {
                let _ = previous.await;
            }
            // io-ok: runs inside the `background_spawn` above.
            crow_core::write_atomic(&state_dir.join("settings-category"), &id).map_err(|error| error.to_string())
        })
        .shared();
    cx.global_mut::<Ui>().settings_category_write = Some(task.clone());
    cx.spawn(async move |cx| {
        if let Err(error) = task.await {
            tracing::warn!("settings category: {error}");
            cx.update(|cx| {
                if cx.try_global::<crate::operations::Operations>().is_some() {
                    let message = format!("Couldn't remember the Settings category: {error}");
                    crate::operations::message(message, true, cx);
                    cx.refresh_windows();
                }
            });
        }
    })
    .detach();
}

/// Re-resolve the theme (after an index change, a settings change, an appearance
/// flip or a move in the theme picker). The files are read in the background and
/// the newest reload wins: a reply from an older one is dropped. A broken theme
/// file keeps the current theme.
pub fn reload_theme(cx: &mut App) {
    let variant = variant_of(cx.window_appearance());
    let ui = cx.global_mut::<Ui>();
    ui.theme_reload_generation += 1;
    let generation = ui.theme_reload_generation;
    let (dir, state) = (ui.themes_dir.clone(), ui.themes_state_dir.clone());
    let saved = match &ui.preview_theme {
        Some(id) => Some(id.clone()),
        None => ui.settings.theme_id().map(str::to_owned),
    };
    let task = cx.background_spawn(async move {
        // io-ok: background task
        let id = crow_theme::resolve_theme_id(saved.as_deref(), variant, &dir, &state);
        // io-ok: background task
        let missing = crow_theme::missing_saved(saved.as_deref(), &dir).map(str::to_owned);
        // io-ok: background task
        (missing, crow_theme::load_theme(&dir, &id))
    });
    cx.spawn(async move |cx| {
        let (missing, loaded) = task.await;
        cx.update(|cx| {
            if cx.global::<Ui>().theme_reload_generation == generation {
                apply_loaded_theme(missing, loaded, cx);
            }
        });
    })
    .detach();
}

fn apply_loaded_theme(missing: Option<String>, loaded: Result<Theme, String>, cx: &mut App) {
    let _span = tracing::debug_span!("theme reload").entered();
    match loaded {
        Ok(theme) => {
            let theme = resolve_system_colors(theme);
            // Said once per missing id, not on every reload.
            cx.global_mut::<Ui>().theme_error = None;
            if cx.global::<Ui>().missing_theme_notice != missing && cx.has_global::<crate::operations::Operations>() {
                cx.global_mut::<Ui>().missing_theme_notice = missing.clone();
                if let Some(missing) = missing {
                    crate::operations::message(format!("Theme ‘{missing}’ not found; using {}", theme.name), false, cx);
                    cx.refresh_windows();
                }
            }
            set_theme(theme, cx);
        }
        Err(e) => {
            tracing::warn!("theme reload skipped: {e}");
            // The last good theme stays; the file and line are said once per distinct error.
            if cx.global::<Ui>().theme_error.as_deref() != Some(e.as_str())
                && cx.has_global::<crate::operations::Operations>()
            {
                cx.global_mut::<Ui>().theme_error = Some(e.clone());
                crate::operations::message(e, true, cx);
                cx.refresh_windows();
            }
        }
    }
}

/// Apply the system colours as they look in the theme's own variant, not the OS's.
fn resolve_system_colors(theme: Theme) -> Theme {
    let variant = theme.variant;
    theme.resolve_system(&|reference| system_color(reference, variant))
}

/// A system colour for a theme's `system:*` role (Finder follows the accent colour).
fn system_color(reference: &str, variant: crow_theme::Variant) -> Option<crow_theme::Rgb> {
    #[cfg(test)]
    {
        let _ = (reference, variant);
        None
    }
    #[cfg(not(test))]
    {
        let dark = variant == crow_theme::Variant::Dark;
        let rgb = match reference {
            "system:accent" => crow_macos::accent_color(dark),
            "system:selection" => crow_macos::unfocused_selection_color(dark),
            _ => None,
        };
        rgb.map(crow_theme::Rgb)
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
    // Off: colors change at once. Low: a short fade. Full: the full half second.
    let duration = match ui.feedback.mode {
        Effective::Off => None,
        Effective::Low => Some(FADE_REDUCED_MOTION),
        Effective::Full => Some(FADE),
    };
    if ui.fade_enabled
        && let Some(duration) = duration
        && !theme.same_colors(&ui.theme)
    {
        // A change mid-fade starts from the colors on screen, not the old theme.
        ui.fade = Some((ui.theme.clone(), crate::clock::now(), duration));
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
                let t = (crate::clock::elapsed(*start).as_secs_f32() / duration.as_secs_f32()).min(1.0);
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
    // Clear a stale preview even when the choice is already the saved theme.
    cx.global_mut::<Ui>().preview_theme = None;
    set_setting(SettingChange::Theme(id.to_owned()), cx);
}

/// Open `settings.toml` in its default app, with the same failure message the
/// old ⌘, showed when no app owns `.toml` files.
pub fn edit_settings_file(cx: &mut App) {
    open_config_file("settings.toml", cx);
}

/// Where the third-party notices live: the bundle's Resources, else the source tree.
fn acknowledgements_candidates() -> Vec<PathBuf> {
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("Resources").join("THIRD-PARTY-NOTICES.md")));
    bundled.into_iter().chain([Path::new(env!("CARGO_MANIFEST_DIR")).join("../../THIRD-PARTY-NOTICES.md")]).collect()
}

/// Help ▸ Acknowledgements: show THIRD-PARTY-NOTICES.md in the text editor.
pub fn open_acknowledgements(cx: &mut App) {
    let found = cx.background_spawn(async move {
        acknowledgements_candidates().into_iter().find(|path| path.is_file()) // io-ok: inside background_spawn
    });
    cx.spawn(async move |cx| {
        let opened = match found.await {
            Some(path) => cx.update(|cx| crate::effects::effects(cx).open_text(&path)),
            None => Err("the notices file is missing from this build".to_owned()),
        };
        if let Err(e) = opened {
            cx.update(|cx| {
                crate::operations::message(format!("Couldn't open the acknowledgements: {e}"), true, cx);
                cx.refresh_windows();
            });
        }
    })
    .detach();
}

/// Open `name` from the config folder in its default app.
pub fn open_config_file(name: &str, cx: &mut App) {
    let path = cx.global::<Ui>().config_dir.join(name);
    open_config_path(&path, cx);
}

/// Open one config file, or the folder holding a problem that no one file explains.
pub fn open_config_path(path: &Path, cx: &mut App) {
    if !crate::effects::effects(cx).open_in_default_app(path) {
        let message = format!("Couldn't open {}: no app is set to open .toml files", path.display());
        crate::operations::message(message, true, cx);
        cx.refresh_windows();
    }
}

/// Open `<config>/plugins` in a Crow window, creating it if this is the first visit.
pub fn open_plugins_folder(cx: &mut App) {
    let dir = cx.global::<Ui>().config_dir.join("plugins");
    let create = cx.background_spawn(async move {
        std::fs::create_dir_all(&dir).map(|()| dir) // io-ok: inside background_spawn
    });
    cx.spawn(async move |cx| {
        let dir = match create.await {
            Ok(dir) => dir,
            Err(error) => {
                cx.update(|cx| {
                    crate::operations::message(format!("Couldn't create the plugins folder: {error}"), true, cx);
                    cx.refresh_windows();
                });
                return;
            }
        };
        cx.update(|cx| crate::window::open_path(dir, true, cx));
    })
    .detach();
}

/// Open the config folder in a Crow window.
pub fn open_config_folder(cx: &mut App) {
    let dir = cx.global::<Ui>().config_dir.clone();
    crate::window::open_path(dir, true, cx);
}

/// "Apply to all apps": make the theme `id` the global choice for its variant and
/// follow it here. The theme is loaded and written in the background; the choice
/// follows once it is on disk.
pub fn apply_theme_everywhere(id: String, cx: &mut App) {
    let (dir, state) = {
        let ui = cx.global::<Ui>();
        (ui.themes_dir.clone(), ui.themes_state_dir.clone())
    };
    let task = cx.background_spawn(async move {
        // io-ok: background task
        let theme = crow_theme::load_theme(&dir, &id)?;
        // io-ok: background task
        crow_theme::set_global(&theme, &state).map_err(|e| format!("Can't apply the theme to all apps: {e}"))
    });
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| match result {
            Ok(()) => choose_theme("system", cx),
            // Not a file problem, so it belongs with the operation messages (no Open button).
            Err(e) => crate::operations::message(e, true, cx),
        });
    })
    .detach();
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
    queue_state_save(
        "palette-commands.json",
        file,
        move |file| recent.save(file).map_err(|e| format!("{}: {e}", file.display())),
        cx,
    );
    cx.refresh_windows();
}

/// Re-read `palette-commands.json` into `Ui`, as a relaunch would (tests only).
#[cfg(test)]
pub fn reload_state_for_tests(cx: &mut App) {
    let file = cx.global::<Ui>().state_dir.join("palette-commands.json");
    let category_file = cx.global::<Ui>().state_dir.join("settings-category");
    let category = read_settings_category(category_file);
    let ui = cx.global_mut::<Ui>();
    ui.recent_commands = RecentCommands::load(&file);
    ui.settings_category = category;
}

#[cfg(test)]
fn read_settings_category(path: PathBuf) -> Option<String> {
    // io-ok: test-only simulated relaunch
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// Recompute Recent's list off the main thread (it checks each folder still exists)
/// and save the visit history.
pub fn refresh_recent(cx: &mut App) {
    let ui = cx.global_mut::<Ui>();
    ui.recent_refresh += 1;
    let generation = ui.recent_refresh;
    let ui = cx.global::<Ui>();
    let recent = ui.recent.clone();
    let zoxide = ui.zoxide.clone();
    let sidebar_count = ui.sidebar.recent.count;
    // The palette wants more folders than the sidebar shows; keep favorites in this
    // list and filter them out only for the sidebar's rows.
    let count = sidebar_count.max(50);
    let favorites: Vec<PathBuf> = ui.favorites().into_iter().flat_map(|(_, f)| f).map(|f| f.path).collect();
    let file = ui.state_dir.join("recent.json");
    let alias_candidates = favorites;
    {
        let (saved, file) = (recent.clone(), file.clone());
        queue_state_save(
            "recent.json",
            file,
            move |file| saved.save(file).map_err(|e| format!("{}: {e}", file.display())),
            cx,
        );
    }
    let task = cx.background_spawn(async move {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        // io-ok: inside background_spawn
        let list = recent.top(count, now, &zoxide, |p| p != Path::new("/") && p.is_dir());
        // A sidebar row can only be a symlink (Finder aliases are files, not folders).
        let aliases: HashSet<PathBuf> = alias_candidates
            .iter()
            .chain(list.iter())
            .filter(|path| path.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink())) // io-ok: inside background_spawn
            .cloned()
            .collect();
        // Sidebar Recent leaves out favorites and repeats, also when reached through a
        // symlink (`~/code/crow` when `~/src/crow` is a favorite and `~/code` links to `~/src`).
        let real = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf()); // io-ok: inside background_spawn
        let mut seen: HashSet<PathBuf> = alias_candidates.iter().map(|path| real(path)).collect();
        let sidebar: Vec<PathBuf> =
            list.iter().filter(|path| seen.insert(real(path))).take(sidebar_count).cloned().collect();
        (list, aliases, sidebar)
    });
    cx.spawn(async move |cx| {
        let (list, aliases, sidebar) = task.await;
        cx.update(|cx| {
            let ui = cx.global_mut::<Ui>();
            if ui.recent_refresh != generation {
                return;
            }
            let changed = ui.recent_list != sidebar || ui.palette_recents != list || ui.aliases != aliases;
            ui.recent_list = sidebar;
            ui.palette_recents = list;
            ui.aliases = aliases;
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
    // `init_for_tests` installs no watchers; tests keep `volumes` empty so their
    // sidebar paths are only the ones they configure.
    if cx.try_global::<Watchers>().is_none() {
        return;
    }
    let ui = cx.global_mut::<Ui>();
    ui.volumes_refresh += 1;
    let generation = ui.volumes_refresh;
    let task = cx.background_spawn(async { crow_macos::volumes() });
    cx.spawn(async move |cx| {
        let volumes = task.await;
        cx.update(|cx| {
            let ui = cx.global_mut::<Ui>();
            if ui.volumes_refresh != generation {
                return;
            }
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
    write_config_with_completion(which, write, None, cx);
}

pub type ViewsWriteCompletion = Box<dyn FnOnce(Result<(), String>, &mut App)>;

fn write_config_with_completion(
    which: usize,
    write: impl FnOnce(&Path, &Path) -> Result<(), String> + Send + 'static,
    completion: Option<ViewsWriteCompletion>,
    cx: &mut App,
) {
    // A reload that read the file before this write must not apply its older copy.
    cx.global_mut::<Ui>().write_epoch[which] += 1;
    let ui = cx.global::<Ui>();
    let (config_dir, home) = (ui.config_dir.clone(), ui.home.clone());
    if matches!(which, SETTINGS | VIEWS | SAVED_QUERIES) {
        let ui = cx.global_mut::<Ui>();
        let previous = ui.config_writes[which].take();
        ui.config_write_revision[which] += 1;
        let revision = ui.config_write_revision[which];
        // Each edit reads and replaces the same file. Queue them in input order
        // and publish only the last snapshot so rapid key presses stay responsive.
        let task = cx
            .background_spawn(async move {
                if let Some(previous) = previous {
                    let _ = previous.await;
                }
                write(&config_dir, &home)?;
                match which {
                    // io-ok: background task
                    SETTINGS => load_settings(&config_dir).map(ConfigSnapshot::Settings).map_err(|e| e.to_string()),
                    // io-ok: background task
                    VIEWS => load_views(&config_dir, &home).map(ConfigSnapshot::Views),
                    // io-ok: background task
                    SAVED_QUERIES => load_saved_queries(&config_dir, &home).map(ConfigSnapshot::SavedQueries),
                    _ => unreachable!("queued config file"),
                }
            })
            .shared();
        cx.global_mut::<Ui>().config_writes[which] = Some(task.clone());
        cx.spawn(async move |cx| {
            let result = task.await;
            cx.update(|cx| {
                let outcome = result.as_ref().map(|_| ()).map_err(Clone::clone);
                if cx.global::<Ui>().config_write_revision[which] == revision {
                    let ui = cx.global_mut::<Ui>();
                    ui.config_writes[which] = None;
                    ui.write_epoch[which] += 1;
                    match result {
                        Ok(ConfigSnapshot::Settings(settings)) => ui.apply_loaded_settings(Ok(settings)),
                        Ok(ConfigSnapshot::Views(views)) => ui.apply_loaded_views(Ok(views)),
                        Ok(ConfigSnapshot::SavedQueries(queries)) => ui.apply_loaded_saved_queries(Ok(queries)),
                        Err(error) if completion.is_none() || which == SAVED_QUERIES => ui.set_error(which, Err(error)),
                        Err(_) => false,
                    };
                    cx.refresh_windows();
                }
                if let Some(completion) = completion {
                    completion(outcome, cx);
                }
            });
        })
        .detach();
        return;
    }
    let task = cx.background_spawn(async move {
        write(&config_dir, &home)?;
        // Read back off the main thread, so applying it is not disk work.
        Ok(match which {
            // io-ok: background task
            SIDEBAR => Some(Reread::Sidebar(load_sidebar(&config_dir))),
            // io-ok: background task
            EXCLUDE => Some(Reread::Exclude(Box::new(crow_core::load_exclude(&config_dir)))),
            _ => None,
        })
    });
    cx.spawn(async move |cx| {
        let result: Result<Option<Reread>, String> = task.await;
        cx.update(|cx| {
            let ui = cx.global_mut::<Ui>();
            ui.write_epoch[which] += 1;
            match result {
                Ok(Some(Reread::Sidebar(sidebar))) => {
                    ui.apply_loaded_sidebar(sidebar);
                    refresh_recent(cx);
                }
                Ok(Some(Reread::Exclude(exclude))) => {
                    ui.apply_loaded_exclude(*exclude);
                }
                Ok(None) => unreachable!("queued writes handled above"),
                Err(e) => {
                    ui.set_error(which, Err(e));
                }
            }
            cx.refresh_windows();
        });
    })
    .detach();
}

/// A file read back after Crow wrote it.
enum Reread {
    Sidebar(Result<SidebarConfig, String>),
    Exclude(Box<Result<crow_core::Exclude, String>>),
}

/// Reload the saved-query file without blocking the UI or publishing a read
/// that started before a newer edit.
fn reload_saved_queries(cx: &mut App) {
    let ui = cx.global_mut::<Ui>();
    if ui.config_writes[SAVED_QUERIES].is_some() {
        return;
    }
    ui.config_write_revision[SAVED_QUERIES] += 1;
    let revision = ui.config_write_revision[SAVED_QUERIES];
    let (config, home) = (ui.config_dir.clone(), ui.home.clone());
    // io-ok: background task
    let task = cx.background_spawn(async move { load_saved_queries(&config, &home) });
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| {
            let ui = cx.global_mut::<Ui>();
            if ui.config_write_revision[SAVED_QUERIES] == revision && ui.apply_loaded_saved_queries(result) {
                cx.refresh_windows();
            }
        });
    })
    .detach();
}

pub fn save_saved_query(query: crow_core::SavedQuery, completion: Option<SavedQueryWriteCompletion>, cx: &mut App) {
    write_config_with_completion(
        SAVED_QUERIES,
        move |config, home| crow_core::save_saved_query(config, home, &query),
        completion,
        cx,
    );
}

pub fn remove_saved_query(id: SavedQueryId, completion: Option<SavedQueryWriteCompletion>, cx: &mut App) {
    write_config_with_completion(
        SAVED_QUERIES,
        move |config, _| crow_core::remove_saved_query(config, &id),
        completion,
        cx,
    );
}

pub type SavedQueryWriteCompletion = Box<dyn FnOnce(Result<(), String>, &mut App)>;

/// Change `[defaults]` in views.toml. The queued reload publishes a new views
/// generation, so browser panes apply it like any edit.
pub fn save_defaults_change(change: crow_core::DefaultsChange, completion: Option<ViewsWriteCompletion>, cx: &mut App) {
    write_config_with_completion(
        VIEWS,
        move |config, home| crow_core::save_defaults_change(config, home, change),
        completion,
        cx,
    );
}

/// Set or clear one folder's preview-provider list.
pub fn save_folder_preview_providers(
    dir: PathBuf,
    providers: Option<Vec<String>>,
    completion: ViewsWriteCompletion,
    cx: &mut App,
) {
    write_config_with_completion(
        VIEWS,
        move |config, home| crow_core::save_folder_preview_providers(config, home, &dir, providers),
        Some(completion),
        cx,
    );
}

/// Set or clear one path rule's preview-provider list, proving the rule identity.
pub fn save_rule_preview_providers(
    locator: crow_core::RuleLocator,
    providers: Option<Vec<String>>,
    completion: ViewsWriteCompletion,
    cx: &mut App,
) {
    write_config_with_completion(
        VIEWS,
        move |config, home| crow_core::save_rule_preview_providers(config, home, &locator, providers),
        Some(completion),
        cx,
    );
}

/// Append a preview-only path rule after the existing rules.
pub fn add_preview_rule(
    index: usize,
    pattern: String,
    providers: Vec<String>,
    completion: ViewsWriteCompletion,
    cx: &mut App,
) {
    write_config_with_completion(
        VIEWS,
        move |config, home| crow_core::add_preview_rule(config, home, index, &pattern, providers),
        Some(completion),
        cx,
    );
}

/// Remember a view change as `dir`'s own setting.
pub fn save_view_change(dir: PathBuf, change: crow_core::ViewChange, cx: &mut App) {
    write_config(VIEWS, move |config, home| crow_core::save_folder_change(config, home, &dir, change), cx);
}

pub fn save_default_columns(folder: PathBuf, columns: Vec<String>, cx: &mut App) {
    write_config(VIEWS, move |config, home| crow_core::save_default_columns(config, home, &folder, columns), cx);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crow_theme::Rgb;

    fn other_theme() -> Theme {
        let mut theme = crow_theme::fallback();
        theme.id = "other".into();
        theme.palette.insert("accent".into(), Rgb(0x123456));
        theme
    }

    #[gpui::test]
    fn the_folder_sizes_setting_refreshes_the_view_menu(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            init_for_tests(cx);
            crate::menus::init_for_tests(cx);
            let calculate_disabled = |cx: &App| {
                let menus = cx.get_menus().expect("menus are set");
                menus
                    .iter()
                    .flat_map(|menu| menu.items.iter())
                    .find_map(|item| match item {
                        gpui::OwnedMenuItem::Action { name, disabled, .. } if name == "Calculate Folder Sizes" => {
                            Some(*disabled)
                        }
                        _ => None,
                    })
                    .expect("the View menu lists Calculate Folder Sizes")
            };
            assert!(!calculate_disabled(cx));
            set_setting(SettingChange::FolderSizes(FolderSizesMode::Visible), cx);
            assert!(calculate_disabled(cx), "the menu follows the setting at once");
        });
    }

    #[gpui::test]
    fn writes_to_one_file_run_in_the_order_they_were_queued(cx: &mut gpui::TestAppContext) {
        cx.update(init_for_tests);
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (first, second) = (log.clone(), log.clone());
        let last = cx.update(|cx| {
            let _earlier = queue_file_write(
                "probe.toml",
                move || {
                    std::thread::sleep(Duration::from_millis(60));
                    first.lock().unwrap().push(1);
                    Ok(())
                },
                cx,
            );
            queue_file_write(
                "probe.toml",
                move || {
                    second.lock().unwrap().push(2);
                    Ok(())
                },
                cx,
            )
        });
        let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = finished.clone();
        cx.update(|cx| {
            cx.spawn(async move |_| {
                last.await.expect("the write succeeded");
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
            })
            .detach();
        });
        for _ in 0..200 {
            cx.run_until_parked();
            if finished.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(finished.load(std::sync::atomic::Ordering::SeqCst), "the queued writes finished");
        assert_eq!(*log.lock().unwrap(), vec![1, 2], "the later write waited for the earlier one");
    }

    #[test]
    fn app_icon_is_a_square_png() {
        let png = APP_ICON_PNG;
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"), "assets/app-icon.png is a PNG");
        // IHDR comes first: width and height are big-endian u32s at bytes 16 and 20.
        let side = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().expect("four bytes"));
        assert_eq!(side(16), side(20), "assets/app-icon.png is square");
        assert!(side(16) >= 512, "assets/app-icon.png is at least 512 px (1024 recommended)");
    }

    #[gpui::test]
    fn each_setting_writes_only_its_key(cx: &mut gpui::TestAppContext) {
        let config = cx.update(crate::ui::init_for_tests);
        cx.update(crate::menus::init_for_tests);
        let before = "# mine\n\ntheme = \"system\" # theme\nkeymap = \"finder\"\nhints = \"auto\"\ntips = \"auto\"\nmotion = \"auto\"\nsound = \"crow\"\nicon_pack = \"native\"\nfont_family = \"\"\nfont_size = 13.0\nterminal = \"Terminal\"\nshow_hidden_files = true\nsearch_includes_hidden = true\nsearch_match = \"all\"\nshow_preview = true\npalette_recent_commands = true\nview_per_folder = true\n\n[window]\nwidth = 1200\nheight = 760\n";
        let changes = [
            SettingChange::Theme("gruvbox-dark-hard".into()),
            SettingChange::Keymap(KeymapLayer::Vim),
            SettingChange::Hints(HintsMode::Always),
            SettingChange::Tips(TipsMode::Off),
            SettingChange::Motion(MotionMode::Off),
            SettingChange::Sound(SoundSet::Finder),
            SettingChange::IconPack("example.pack".into()),
            SettingChange::FontFamily("Menlo".into()),
            SettingChange::FontSize(14.0),
            SettingChange::Terminal("iTerm".into()),
            SettingChange::ShowHiddenFiles(false),
            SettingChange::SearchIncludesHidden(false),
            SettingChange::SearchMatch(SearchMatch::Names),
            SettingChange::ShowPreview(false),
            SettingChange::PaletteRecentCommands(false),
            SettingChange::ViewPerFolder(false),
            SettingChange::WindowWidth(900),
            SettingChange::WindowHeight(640),
        ];
        for change in changes {
            std::fs::write(config.join("settings.toml"), before).expect("write settings");
            cx.update(crate::ui::reload_config);
            cx.update(|cx| set_setting(change.clone(), cx));
            cx.run_until_parked();
            let after = std::fs::read_to_string(config.join("settings.toml")).expect("read settings");
            let changed: Vec<_> = before.lines().zip(after.lines()).filter(|(old, new)| old != new).collect();
            assert_eq!(changed.len(), 1, "{change:?} changed {changed:?}\n{after}");
        }
    }

    #[gpui::test]
    fn test_config_roots_live_until_the_app_drops(_outer: &mut gpui::TestAppContext) {
        use std::path::PathBuf;

        struct ViewProbe(Vec<PathBuf>);
        impl gpui::Render for ViewProbe {
            fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
                gpui::div()
            }
        }
        impl Drop for ViewProbe {
            fn drop(&mut self) {
                assert!(
                    self.0.iter().all(|path| path.exists()),
                    "the window outlived a config root: {:?}",
                    self.0.iter().filter(|path| !path.exists()).collect::<Vec<_>>()
                );
            }
        }

        let test_cx = gpui::TestAppContext::build(gpui::TestDispatcher::new(0), Some("config roots"));
        let first = test_cx.update(crate::ui::init_for_tests);
        let second = test_cx.update(crate::ui::init_for_tests);
        assert_ne!(first, second, "each init gets its own root");
        assert!(first.exists(), "a second init keeps the first root alive");
        assert!(second.exists());
        test_cx.update(|cx| {
            assert_eq!(cx.global::<Ui>().config_dir, second, "the live Ui uses the newest root");
            cx.open_window(gpui::WindowOptions::default(), |_, cx| {
                cx.new(|_| ViewProbe(vec![first.clone(), second.clone()]))
            })
            .expect("nested window");
        });
        test_cx.run_until_parked();
        test_cx.update(|cx| cx.quit());
        test_cx.run_until_parked();
        drop(test_cx);
        assert!(!first.exists(), "the app drop removed the first root");
        assert!(!second.exists(), "the app drop removed the second root");
    }

    /// GPUI's test platform ends a test with `quit` and drops the app; a panic
    /// while that app is alive must still drop the config global's roots.
    #[gpui::test]
    fn config_roots_are_removed_when_a_test_unwinds(_outer: &mut gpui::TestAppContext) {
        use std::sync::{Arc, Mutex};

        let root = Arc::new(Mutex::new(None));
        let tracked = root.clone();
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let test_cx = gpui::TestAppContext::build(gpui::TestDispatcher::new(0), Some("config unwind"));
            let config = test_cx.update(crate::ui::init_for_tests);
            let root = config.parent().expect("test root").to_path_buf();
            std::fs::write(config.join("file"), "x").expect("fixture file");
            *tracked.lock().unwrap() = Some(root);
            panic!("deliberate fixture panic");
        }));
        assert!(caught.is_err());
        let root = root.lock().unwrap().clone().expect("the config root");
        assert!(!root.exists(), "the global removed the config root during unwinding");
    }

    #[gpui::test]
    fn theme_fade_follows_the_motion_mode(cx: &mut gpui::TestAppContext) {
        let root = tempfile::Builder::new().prefix("crow-ui-fade-").tempdir().expect("test root");
        let ui = Ui::new(
            root.path().join("config"),
            root.path().join("state"),
            crow_core::home(),
            crow_theme::fallback(),
            true,
        );
        cx.update(|cx| {
            cx.set_global(ui);
            cx.default_global::<TestRoots>().0.push(root);
        });
        for (mode, expected) in [
            (Effective::Off, None),
            (Effective::Low, Some(Duration::from_millis(150))),
            (Effective::Full, Some(Duration::from_millis(500))),
        ] {
            cx.update(|cx| {
                let ui = cx.global_mut::<Ui>();
                ui.feedback.mode = mode;
                ui.fade = None;
                ui.theme = crow_theme::fallback();
                ui.target = crow_theme::fallback();
                set_theme(other_theme(), cx);
                assert_eq!(cx.global::<Ui>().fade_duration(), expected, "{mode:?}");
            });
        }
    }
}
