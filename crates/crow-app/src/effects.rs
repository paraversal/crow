//! Every side effect that escapes Crow's process, behind one seam.
//!
//! Production installs [`SystemEffects`], which calls the AppKit and `open(1)`
//! code unchanged. Tests and the fuzzer install [`RecordingEffects`], which
//! records each call and answers from a script, so no app launches, no sound
//! plays and the user's pasteboard and Trash are never touched.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crow_macos::{ComboDrag, ContextMenuEntry, QuickLookKey, Sound, SpotlightHit};
use gpui::{App, Global, Window};

/// One method per escaping effect. `Send + Sync` so background tasks (opening a
/// config file, ejecting a disk, Spotlight) can hold the same seam.
pub trait Effects: Send + Sync {
    /// Open `path` in the app that owns it, like a double-click (`open path`).
    fn open_path(&self, path: &Path) -> Result<(), String>;
    /// Open `path` in the default text editor (`open -t path`).
    fn open_text(&self, path: &Path) -> Result<(), String>;
    /// Open a config file: its default app, then a text editor (`open -t`).
    fn open_config(&self, path: &Path) -> Result<(), String>;
    /// Open `path` with its default app through AppKit; false when nothing opened it.
    fn open_in_default_app(&self, path: &Path) -> bool;
    /// Show the folder `path` itself in Finder; false when Finder did not open.
    fn open_folder_in_finder(&self, path: &Path) -> bool;
    /// Show `paths` selected in Finder windows.
    fn reveal_in_finder(&self, paths: &[PathBuf]);

    /// Toggle Quick Look on `path`.
    fn toggle_quick_look(&self, path: &Path);
    /// Follow the cursor in an open Quick Look panel.
    fn update_quick_look(&self, path: &Path);
    /// Route the panel's arrow keys back into the window.
    fn set_quick_look_key_handler(&self, handler: Box<dyn Fn(QuickLookKey) + 'static>);

    /// Put `paths` on the general pasteboard as file URLs; the new change count.
    fn write_file_urls(&self, paths: &[PathBuf]) -> Option<isize>;
    /// The file URLs another app put on the pasteboard.
    fn read_file_urls(&self) -> Vec<PathBuf>;
    /// The general pasteboard's change count, to tell our own contents from another app's.
    fn pasteboard_change_count(&self) -> Option<isize>;

    /// Load `sounds` now, so the first play does no disk I/O.
    fn preload_sounds(&self, sounds: &[Sound]);
    fn play_sound(&self, sound: &Sound, volume: f32);
    fn stop_sound(&self, sound: &Sound);
    fn set_sound_volume(&self, sound: &Sound, volume: f32);
    fn beep(&self);

    /// Show a native context menu; `done` receives the chosen slot, or none.
    /// False when no menu opened (not on the main thread, or no window).
    fn show_context_menu(
        &self,
        window: &mut Window,
        entries: Vec<ContextMenuEntry>,
        done: Box<dyn FnOnce(Option<usize>)>,
    ) -> bool;
    fn show_about_panel(&self);
    fn set_app_icon(&self, png: &[u8]);
    /// Start a native drag of `drag`'s items; false to let GPUI's own drag run.
    fn begin_drag(&self, window: &mut Window, drag: &ComboDrag<'_>) -> bool;
    /// Move the window from an app-owned titlebar drag (`app_owns_titlebar_drag`).
    fn start_window_move(&self, window: &mut Window);
    /// The titlebar's double-click action, as the system's preference chooses it.
    fn titlebar_double_click(&self, window: &mut Window);

    /// Eject the volume at `path`.
    fn eject(&self, path: &Path) -> Result<(), String>;
    /// Run Spotlight (`mdfind`) for `query`, streaming hits into `sink`.
    fn spotlight_names(
        &self,
        query: &str,
        sink: Box<dyn FnMut(Vec<SpotlightHit>) + Send>,
        cancel: &AtomicBool,
    ) -> Result<(), String>;

    #[cfg(any(test, feature = "fuzz"))]
    fn as_any(&self) -> &dyn std::any::Any;
}

/// The real thing: AppKit, `open(1)`, `mdfind` and `diskutil`, exactly as before.
pub struct SystemEffects;

impl Effects for SystemEffects {
    fn open_path(&self, path: &Path) -> Result<(), String> {
        std::process::Command::new("open").arg(path).spawn().map(|_| ()).map_err(|e| e.to_string())
    }

    fn open_text(&self, path: &Path) -> Result<(), String> {
        std::process::Command::new("open").arg("-t").arg(path).spawn().map(|_| ()).map_err(|e| e.to_string())
    }

    fn open_config(&self, path: &Path) -> Result<(), String> {
        crow_macos::open_path(path, false).or_else(|_| crow_macos::open_path(path, true)).map_err(|e| e.to_string())
    }

    fn open_in_default_app(&self, path: &Path) -> bool {
        crow_macos::open_in_default_app(path)
    }

    fn open_folder_in_finder(&self, path: &Path) -> bool {
        crow_macos::open_folder_in_finder(path)
    }

    fn reveal_in_finder(&self, paths: &[PathBuf]) {
        crow_macos::reveal_in_finder(paths);
    }

    fn toggle_quick_look(&self, path: &Path) {
        crow_macos::toggle_quick_look(path);
    }

    fn update_quick_look(&self, path: &Path) {
        crow_macos::update_quick_look(path);
    }

    fn set_quick_look_key_handler(&self, handler: Box<dyn Fn(QuickLookKey) + 'static>) {
        crow_macos::set_quick_look_key_handler(handler);
    }

    fn write_file_urls(&self, paths: &[PathBuf]) -> Option<isize> {
        crow_macos::write_file_urls(paths)
    }

    fn read_file_urls(&self) -> Vec<PathBuf> {
        crow_macos::read_file_urls()
    }

    fn pasteboard_change_count(&self) -> Option<isize> {
        crow_macos::pasteboard_change_count()
    }

    fn preload_sounds(&self, sounds: &[Sound]) {
        crow_macos::preload(sounds);
    }

    fn play_sound(&self, sound: &Sound, volume: f32) {
        crow_macos::play(sound, volume);
    }

    fn stop_sound(&self, sound: &Sound) {
        crow_macos::stop(sound);
    }

    fn set_sound_volume(&self, sound: &Sound, volume: f32) {
        crow_macos::set_volume(sound, volume);
    }

    fn beep(&self) {
        crow_macos::beep();
    }

    fn show_context_menu(
        &self,
        window: &mut Window,
        entries: Vec<ContextMenuEntry>,
        done: Box<dyn FnOnce(Option<usize>)>,
    ) -> bool {
        crow_macos::show_context_menu(window, entries, done)
    }

    fn show_about_panel(&self) {
        crow_macos::show_about_panel();
    }

    fn set_app_icon(&self, png: &[u8]) {
        crow_macos::set_app_icon(png);
    }

    fn begin_drag(&self, window: &mut Window, drag: &ComboDrag<'_>) -> bool {
        crow_macos::begin_drag(window, drag)
    }

    fn start_window_move(&self, window: &mut Window) {
        window.start_window_move();
    }

    fn titlebar_double_click(&self, window: &mut Window) {
        window.titlebar_double_click();
    }

    fn eject(&self, path: &Path) -> Result<(), String> {
        crow_macos::eject(path).map_err(|e| e.to_string())
    }

    fn spotlight_names(
        &self,
        query: &str,
        sink: Box<dyn FnMut(Vec<SpotlightHit>) + Send>,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        crow_macos::spotlight_names(query, sink, cancel)
    }

    #[cfg(any(test, feature = "fuzz"))]
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The installed seam. One global, so every view and background task sees it.
pub struct EffectHost(pub Arc<dyn Effects>);

impl Global for EffectHost {}

pub fn install(cx: &mut App, effects: Arc<dyn Effects>) {
    cx.set_global(EffectHost(effects));
}

/// The installed seam. Cloning the `Arc` is cheap; call sites keep it for the
/// length of one effect.
pub fn effects(cx: &App) -> Arc<dyn Effects> {
    cx.global::<EffectHost>().0.clone()
}

/// Put `text` on the general pasteboard (Copy Path, a text field's copy and cut).
/// With a recorder installed it stays in the recorder, so a fuzz run in a real
/// window never replaces the user's clipboard.
pub fn write_text(text: String, cx: &mut App) {
    #[cfg(any(test, feature = "fuzz"))]
    if let Some(recorder) = recording(cx) {
        recorder.write_text(text);
        return;
    }
    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
}

/// The general pasteboard's text (a text field's paste); the recorder's own
/// text when one is installed.
pub fn read_text(cx: &mut App) -> Option<String> {
    #[cfg(any(test, feature = "fuzz"))]
    if let Some(recorder) = recording(cx) {
        return recorder.read_text();
    }
    cx.read_from_clipboard().and_then(|item| item.text())
}

/// The installed recorder, when tests or the fuzzer installed one.
#[cfg(any(test, feature = "fuzz"))]
pub fn recording(cx: &App) -> Option<&RecordingEffects> {
    cx.global::<EffectHost>().0.as_any().downcast_ref::<RecordingEffects>()
}

/// What a recorded effect was asked to do. Paths and labels only: never the
/// pasteboard contents or icon bytes.
#[cfg(any(test, feature = "fuzz"))]
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    OpenPath(PathBuf),
    OpenText(PathBuf),
    OpenConfig(PathBuf),
    OpenInDefaultApp(PathBuf),
    OpenFolderInFinder(PathBuf),
    RevealInFinder(Vec<PathBuf>),
    ToggleQuickLook(PathBuf),
    UpdateQuickLook(PathBuf),
    SetQuickLookKeyHandler,
    WriteFileUrls(Vec<PathBuf>),
    ReadFileUrls,
    WriteText,
    ReadText,
    PasteboardChangeCount,
    PreloadSounds(Vec<Sound>),
    PlaySound(Sound, f32),
    StopSound(Sound),
    SetSoundVolume(Sound, f32),
    Beep,
    ContextMenu(Vec<ContextMenuEntry>),
    AboutPanel,
    AppIcon(usize),
    BeginDrag(Vec<PathBuf>),
    StartWindowMove,
    TitlebarDoubleClick,
    Eject(PathBuf),
    Spotlight(String),
}

/// Answers the scripted questions; everything else succeeds without doing anything.
#[cfg(any(test, feature = "fuzz"))]
#[derive(Default)]
struct Script {
    /// `None` leaves the menu unopened (tests drive `choose_*` themselves);
    /// `Some(choice)` answers at once, as the native menu would.
    context_menu_choice: Option<Option<usize>>,
    file_urls: Vec<PathBuf>,
    pasteboard_change_count: Option<isize>,
    write_change_count: Option<isize>,
    open_error: Option<String>,
    begin_drag: bool,
    /// The recorder's stand-in for the pasteboard's text.
    text: Option<String>,
}

/// Records every effect and answers from [`Script`]. Installed by
/// `ui::init_for_tests`; the fuzzer scripts a context-menu choice when it wants
/// the item's action to run.
#[cfg(any(test, feature = "fuzz"))]
pub struct RecordingEffects {
    calls: std::sync::Mutex<Vec<Call>>,
    script: std::sync::Mutex<Script>,
    /// Keep the paths of `write_file_urls` calls. Tests assert them; the live
    /// fuzzer never reads them, and a 100k copy would clone every path.
    record_file_urls: bool,
}

#[cfg(any(test, feature = "fuzz"))]
impl Default for RecordingEffects {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "fuzz"))]
#[allow(dead_code)] // the fuzzer scripts answers; not every test uses each setter yet
impl RecordingEffects {
    pub fn new() -> Self {
        RecordingEffects {
            calls: std::sync::Mutex::new(Vec::new()),
            script: std::sync::Mutex::new(Script::default()),
            record_file_urls: true,
        }
    }

    /// A recorder that only counts clipboard writes, for the live fuzzer: a
    /// 100k-file copy would otherwise clone every path into the call log.
    pub fn without_file_urls() -> Self {
        RecordingEffects { record_file_urls: false, ..Self::new() }
    }

    /// Every call since the last `take_calls`, in order.
    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("the effect recorder is not poisoned").clone()
    }

    /// The calls since the last take, clearing them.
    pub fn take_calls(&self) -> Vec<Call> {
        std::mem::take(&mut *self.calls.lock().expect("the effect recorder is not poisoned"))
    }

    pub fn clear(&self) {
        self.calls.lock().expect("the effect recorder is not poisoned").clear();
    }

    /// Answer the next context menu with `choice` (`None` closes it without acting).
    pub fn set_context_menu_choice(&self, choice: Option<usize>) {
        self.script.lock().expect("the effect script is not poisoned").context_menu_choice = Some(choice);
    }

    /// Stop answering context menus; tests that drive `choose_*` need this.
    pub fn clear_context_menu_choice(&self) {
        self.script.lock().expect("the effect script is not poisoned").context_menu_choice = None;
    }

    /// What another app has on the pasteboard.
    pub fn set_file_urls(&self, paths: Vec<PathBuf>) {
        self.script.lock().expect("the effect script is not poisoned").file_urls = paths;
    }

    /// The pasteboard's change count; `None` means "not readable here" (as in tests today).
    pub fn set_pasteboard_change_count(&self, count: Option<isize>) {
        self.script.lock().expect("the effect script is not poisoned").pasteboard_change_count = count;
    }

    /// The count `write_file_urls` reports; `None` keeps tests' own-clipboard semantics.
    pub fn set_write_change_count(&self, count: Option<isize>) {
        self.script.lock().expect("the effect script is not poisoned").write_change_count = count;
    }

    /// Make `open_path` fail with `error`.
    pub fn set_open_error(&self, error: Option<String>) {
        self.script.lock().expect("the effect script is not poisoned").open_error = error;
    }

    /// Whether a native drag should claim the drag (false lets GPUI's own run).
    pub fn set_begin_drag(&self, enabled: bool) {
        self.script.lock().expect("the effect script is not poisoned").begin_drag = enabled;
    }

    /// The text Crow last put on the (recorded) pasteboard.
    pub fn text(&self) -> Option<String> {
        self.script.lock().expect("the effect script is not poisoned").text.clone()
    }

    fn write_text(&self, text: String) {
        self.record(Call::WriteText);
        self.script.lock().expect("the effect script is not poisoned").text = Some(text);
    }

    fn read_text(&self) -> Option<String> {
        self.record(Call::ReadText);
        self.text()
    }

    fn record(&self, call: Call) {
        self.calls.lock().expect("the effect recorder is not poisoned").push(call);
    }

    fn open_result(&self, call: Call) -> Result<(), String> {
        self.record(call);
        match self.script.lock().expect("the effect script is not poisoned").open_error.clone() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

#[cfg(any(test, feature = "fuzz"))]
impl Effects for RecordingEffects {
    fn open_path(&self, path: &Path) -> Result<(), String> {
        self.open_result(Call::OpenPath(path.to_path_buf()))
    }

    fn open_text(&self, path: &Path) -> Result<(), String> {
        self.open_result(Call::OpenText(path.to_path_buf()))
    }

    fn open_config(&self, path: &Path) -> Result<(), String> {
        self.open_result(Call::OpenConfig(path.to_path_buf()))
    }

    fn open_in_default_app(&self, path: &Path) -> bool {
        self.record(Call::OpenInDefaultApp(path.to_path_buf()));
        true
    }

    fn open_folder_in_finder(&self, path: &Path) -> bool {
        self.record(Call::OpenFolderInFinder(path.to_path_buf()));
        true
    }

    fn reveal_in_finder(&self, paths: &[PathBuf]) {
        self.record(Call::RevealInFinder(paths.to_vec()));
    }

    fn toggle_quick_look(&self, path: &Path) {
        self.record(Call::ToggleQuickLook(path.to_path_buf()));
    }

    fn update_quick_look(&self, path: &Path) {
        self.record(Call::UpdateQuickLook(path.to_path_buf()));
    }

    fn set_quick_look_key_handler(&self, _handler: Box<dyn Fn(QuickLookKey) + 'static>) {
        self.record(Call::SetQuickLookKeyHandler);
    }

    fn write_file_urls(&self, paths: &[PathBuf]) -> Option<isize> {
        if self.record_file_urls {
            self.record(Call::WriteFileUrls(paths.to_vec()));
        }
        self.script.lock().expect("the effect script is not poisoned").write_change_count
    }

    fn read_file_urls(&self) -> Vec<PathBuf> {
        self.record(Call::ReadFileUrls);
        self.script.lock().expect("the effect script is not poisoned").file_urls.clone()
    }

    fn pasteboard_change_count(&self) -> Option<isize> {
        self.record(Call::PasteboardChangeCount);
        self.script.lock().expect("the effect script is not poisoned").pasteboard_change_count
    }

    fn preload_sounds(&self, sounds: &[Sound]) {
        self.record(Call::PreloadSounds(sounds.to_vec()));
    }

    fn play_sound(&self, sound: &Sound, volume: f32) {
        self.record(Call::PlaySound(sound.clone(), volume));
    }

    fn stop_sound(&self, sound: &Sound) {
        self.record(Call::StopSound(sound.clone()));
    }

    fn set_sound_volume(&self, sound: &Sound, volume: f32) {
        self.record(Call::SetSoundVolume(sound.clone(), volume));
    }

    fn beep(&self) {
        self.record(Call::Beep);
    }

    fn show_context_menu(
        &self,
        _window: &mut Window,
        entries: Vec<ContextMenuEntry>,
        done: Box<dyn FnOnce(Option<usize>)>,
    ) -> bool {
        self.record(Call::ContextMenu(entries));
        let scripted = self.script.lock().expect("the effect script is not poisoned").context_menu_choice;
        match scripted {
            Some(choice) => {
                done(choice);
                true
            }
            // No script: nothing opens, so a test can call `choose_context_item` itself.
            None => false,
        }
    }

    fn show_about_panel(&self) {
        self.record(Call::AboutPanel);
    }

    fn set_app_icon(&self, png: &[u8]) {
        self.record(Call::AppIcon(png.len()));
    }

    fn begin_drag(&self, _window: &mut Window, drag: &ComboDrag<'_>) -> bool {
        self.record(Call::BeginDrag(drag.paths.iter().map(|(path, _)| path.clone()).collect()));
        self.script.lock().expect("the effect script is not poisoned").begin_drag
    }

    fn start_window_move(&self, _window: &mut Window) {
        self.record(Call::StartWindowMove);
    }

    fn titlebar_double_click(&self, _window: &mut Window) {
        self.record(Call::TitlebarDoubleClick);
    }

    fn eject(&self, path: &Path) -> Result<(), String> {
        self.record(Call::Eject(path.to_path_buf()));
        Ok(())
    }

    fn spotlight_names(
        &self,
        query: &str,
        _sink: Box<dyn FnMut(Vec<SpotlightHit>) + Send>,
        _cancel: &AtomicBool,
    ) -> Result<(), String> {
        self.record(Call::Spotlight(query.to_owned()));
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
