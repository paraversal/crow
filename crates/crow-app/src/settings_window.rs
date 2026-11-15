//! The Settings window: one scrolling page of controls bound to `settings.toml`.
//! The file stays the source of truth: every control reads `Ui.settings` on
//! render, and every change writes only its key through `ui::set_setting`.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use crow_core::motion::{Event, MotionMode, SoundSet};
use crow_core::{AppIcon, HintsMode, KeymapLayer};
use crow_theme::Theme;
use gpui::{
    App, Bounds, Context, Entity, FocusHandle, Focusable, FontWeight, Image, ImageFormat, IntoElement, KeyBinding,
    KeyDownEvent, Render, Role, ScrollHandle, SharedString, Subscription, TitlebarOptions, Window, WindowBounds,
    WindowOptions, actions, div, prelude::*, px, size,
};

use crate::actions::OpenSettings;
use crate::controls::{self, ControlMeta};
use crate::style::{color, error_soft, ui_font};
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme_choice::{self, ThemeChoice};
use crate::ui::{self, SettingChange, Ui};

actions!(
    settings_window,
    [
        /// Space or Return on the focused control.
        Activate,
        /// Tab.
        FocusNext,
        /// Shift-Tab.
        FocusPrev,
        /// Shift-↑ on a stepper.
        IncrementTen,
        /// Shift-↓ on a stepper.
        DecrementTen,
        /// ↑ on the focused control.
        Up,
        /// ↓ on the focused control.
        Down,
        /// ← on the focused control.
        Left,
        /// → on the focused control.
        Right,
        /// Escape or ⌘W.
        CloseSettings
    ]
);

/// The settings window's keys. Text fields keep their own keys (Tab and Return
/// arrive as `TextInputEvent`s); ↑ ↓ are free in a field, so a stepper uses them.
pub fn keymap() -> Vec<KeyBinding> {
    let settings = Some("Settings");
    vec![
        KeyBinding::new("space", Activate, Some("Settings && !TextInput")),
        KeyBinding::new("enter", Activate, Some("Settings && !TextInput")),
        KeyBinding::new("left", Left, settings),
        KeyBinding::new("right", Right, settings),
        KeyBinding::new("up", Up, settings),
        KeyBinding::new("down", Down, settings),
        KeyBinding::new("shift-up", IncrementTen, settings),
        KeyBinding::new("shift-down", DecrementTen, settings),
        KeyBinding::new("tab", FocusNext, settings),
        KeyBinding::new("shift-tab", FocusPrev, settings),
        KeyBinding::new("escape", CloseSettings, settings),
        KeyBinding::new("cmd-w", CloseSettings, settings),
        KeyBinding::new("cmd-,", OpenSettings, settings),
    ]
}

/// Open the one Settings window, or bring it to the front and focus its first
/// control. Called by ⌘, and Crow ▸ Settings….
pub fn open(cx: &mut App) {
    if let Some(handle) = cx.windows().into_iter().find_map(|handle| handle.downcast::<SettingsWindow>()) {
        handle
            .update(cx, |this, window, cx| {
                window.activate_window();
                this.focus_first(window, cx);
            })
            .ok();
        return;
    }
    let bounds = Bounds::centered(None, size(px(560.), px(760.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some("Settings".into()),
            appears_transparent: false,
            traffic_light_position: None,
        }),
        is_resizable: false,
        window_min_size: Some(size(px(560.), px(400.))),
        ..Default::default()
    };
    match cx.open_window(options, |window, cx| {
        let view = cx.new(|cx| SettingsWindow::new(window, cx));
        let focus = view.read(cx).first_focus(cx);
        window.focus(&focus, cx);
        view
    }) {
        Ok(handle) => {
            handle.update(cx, |_, window, _| window.activate_window()).ok();
        }
        Err(e) => tracing::error!("open settings window: {e}"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ControlId {
    Theme,
    AppIcon,
    FontFamily,
    FontSize,
    Keymap,
    Hints,
    ShowHidden,
    ShowPreview,
    ViewPerFolder,
    PaletteRecent,
    Motion,
    Sound,
    PlaySound,
    WindowWidth,
    WindowHeight,
    EditSettings,
    ShowConfig,
    OpenSidebarToml,
    OpenViewsToml,
    OpenExcludeToml,
    EditSettingsBanner,
}

impl ControlId {
    /// Reading order: the order Tab visits and the order the page draws.
    const ALL: [ControlId; 21] = [
        ControlId::Theme,
        ControlId::AppIcon,
        ControlId::FontFamily,
        ControlId::FontSize,
        ControlId::Keymap,
        ControlId::Hints,
        ControlId::ShowHidden,
        ControlId::ShowPreview,
        ControlId::ViewPerFolder,
        ControlId::PaletteRecent,
        ControlId::Motion,
        ControlId::Sound,
        ControlId::PlaySound,
        ControlId::WindowWidth,
        ControlId::WindowHeight,
        ControlId::EditSettings,
        ControlId::ShowConfig,
        ControlId::OpenSidebarToml,
        ControlId::OpenViewsToml,
        ControlId::OpenExcludeToml,
        ControlId::EditSettingsBanner,
    ];

    fn key(self) -> &'static str {
        match self {
            ControlId::Theme => "settings-theme",
            ControlId::AppIcon => "settings-app-icon",
            ControlId::FontFamily => "settings-font-family",
            ControlId::FontSize => "settings-font-size",
            ControlId::Keymap => "settings-keymap",
            ControlId::Hints => "settings-hints",
            ControlId::ShowHidden => "settings-show-hidden",
            ControlId::ShowPreview => "settings-show-preview",
            ControlId::ViewPerFolder => "settings-view-per-folder",
            ControlId::PaletteRecent => "settings-palette-recent",
            ControlId::Motion => "settings-motion",
            ControlId::Sound => "settings-sound",
            ControlId::PlaySound => "settings-play-sound",
            ControlId::WindowWidth => "settings-window-width",
            ControlId::WindowHeight => "settings-window-height",
            ControlId::EditSettings => "settings-edit-file",
            ControlId::ShowConfig => "settings-show-config",
            ControlId::OpenSidebarToml => "settings-open-sidebar",
            ControlId::OpenViewsToml => "settings-open-views",
            ControlId::OpenExcludeToml => "settings-open-exclude",
            ControlId::EditSettingsBanner => "settings-edit-file-banner",
        }
    }

    fn label(self) -> &'static str {
        match self {
            ControlId::Theme => "Theme",
            ControlId::AppIcon => "App icon",
            ControlId::FontFamily => "Font",
            ControlId::FontSize => "Font size",
            ControlId::Keymap => "Keymap",
            ControlId::Hints => "Key hints",
            ControlId::ShowHidden => "Show hidden files",
            ControlId::ShowPreview => "Show preview",
            ControlId::ViewPerFolder => "Folders keep their own view",
            ControlId::PaletteRecent => "Remember recent commands in ⌘K",
            ControlId::Motion => "Motion",
            ControlId::Sound => "Sounds",
            ControlId::PlaySound => "Play",
            ControlId::WindowWidth => "Width",
            ControlId::WindowHeight => "Height",
            ControlId::EditSettings => "Edit settings.toml…",
            ControlId::ShowConfig => "Show config folder in Finder",
            ControlId::OpenSidebarToml => "sidebar.toml",
            ControlId::OpenViewsToml => "views.toml",
            ControlId::OpenExcludeToml => "exclude.toml",
            ControlId::EditSettingsBanner => "Edit settings.toml…",
        }
    }

    #[allow(dead_code)] // the accessibility test and M7 H7's fuzzer walk it
    fn role(self) -> Role {
        match self {
            ControlId::Theme | ControlId::FontFamily => Role::ListBox,
            ControlId::AppIcon | ControlId::Keymap | ControlId::Hints | ControlId::Motion | ControlId::Sound => {
                Role::RadioGroup
            }
            ControlId::ShowHidden | ControlId::ShowPreview | ControlId::ViewPerFolder | ControlId::PaletteRecent => {
                Role::Switch
            }
            ControlId::FontSize | ControlId::WindowWidth | ControlId::WindowHeight => Role::SpinButton,
            ControlId::PlaySound
            | ControlId::EditSettings
            | ControlId::ShowConfig
            | ControlId::OpenSidebarToml
            | ControlId::OpenViewsToml
            | ControlId::OpenExcludeToml
            | ControlId::EditSettingsBanner => Role::Button,
        }
    }

    fn description(self, cx: &App) -> Option<SharedString> {
        let text = match self {
            ControlId::Theme => {
                "The theme index in ~/.config/themes. Moving through the list previews it; leaving keeps it."
            }
            ControlId::AppIcon => "The Dock switches now; Finder after the next scripts/bundle.sh.",
            ControlId::FontFamily => "Installed families; System follows macOS.",
            ControlId::FontSize => "10 to 24 points. ↑↓ changes it.",
            ControlId::Keymap => "Vim adds hjkl, gg/G, yy/p and marks on top of the Finder keys.",
            ControlId::Hints => "Auto retires idle hints after you use their key three times.",
            ControlId::ShowHidden => "List dotfiles and other hidden items.",
            ControlId::ShowPreview => "The preview pane in list view and the column in column view.",
            ControlId::ViewPerFolder => "Each folder opens in its saved view; off keeps the window's view.",
            ControlId::PaletteRecent => "List the commands you ran from ⌘K under Recently used.",
            ControlId::Motion => {
                let settings = &cx.global::<Ui>().settings;
                return Some(match settings.motion {
                    MotionMode::Auto => {
                        let reduce = crow_macos::reduce_motion();
                        let effective = settings.motion.resolve(reduce);
                        format!(
                            "Auto now: {}; Reduce Motion is {}.",
                            match effective {
                                crow_core::motion::Effective::Off => "Off",
                                crow_core::motion::Effective::Low => "Low",
                                crow_core::motion::Effective::Full => "Full",
                            },
                            if reduce { "on" } else { "off" }
                        )
                        .into()
                    }
                    _ => "Off changes in one frame; Low fades; Full also moves.".into(),
                });
            }
            ControlId::Sound => "Crow adds soft ticks to Finder's sounds; Off is silent.",
            ControlId::PlaySound => "Play the trash sound in the chosen set.",
            ControlId::WindowWidth | ControlId::WindowHeight => "600 to 4000 points. ↑↓ changes it.",
            ControlId::EditSettings => "Open settings.toml in its default app.",
            ControlId::ShowConfig => "Open ~/.config/crow in Finder.",
            ControlId::OpenSidebarToml
            | ControlId::OpenViewsToml
            | ControlId::OpenExcludeToml
            | ControlId::EditSettingsBanner => return None,
        };
        Some(text.into())
    }

    /// The Files row stays usable while settings.toml is broken; everything that
    /// would write is disabled.
    fn enabled(self, cx: &App) -> bool {
        match self {
            ControlId::EditSettings
            | ControlId::EditSettingsBanner
            | ControlId::ShowConfig
            | ControlId::OpenSidebarToml
            | ControlId::OpenViewsToml
            | ControlId::OpenExcludeToml => true,
            _ => cx.global::<Ui>().settings_error().is_none(),
        }
    }
}

#[derive(Default)]
struct TypeAhead {
    buffer: String,
    at: Option<Instant>,
}

pub struct SettingsWindow {
    focus: FocusHandle,
    handles: HashMap<ControlId, FocusHandle>,
    font_size: Entity<TextInput>,
    window_width: Entity<TextInput>,
    window_height: Entity<TextInput>,
    theme: ThemeChoice,
    theme_scroll: ScrollHandle,
    fonts: Vec<String>,
    font_cursor: usize,
    font_scroll: ScrollHandle,
    theme_type_ahead: TypeAhead,
    font_type_ahead: TypeAhead,
    page_scroll: ScrollHandle,
    /// Each control's direct-child index in the page, for keyboard scrolling.
    control_items: HashMap<ControlId, usize>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = cx.global::<Ui>().settings.clone();
        let font_size = cx.new(|cx| {
            TextInput::new(number_text(f64::from(settings.font_size)), "", cx)
                .with_id("settings-font-size")
                .with_role(Role::SpinButton)
                .with_aria_label("Font size")
                .with_numeric_range(f64::from(settings.font_size), 10.0, 24.0, 1.0)
        });
        let window_width = cx.new(|cx| {
            TextInput::new(settings.window.width.to_string(), "", cx)
                .with_id("settings-window-width")
                .with_role(Role::SpinButton)
                .with_aria_label("Width")
                .with_numeric_range(f64::from(settings.window.width), 600.0, 4000.0, 1.0)
        });
        let window_height = cx.new(|cx| {
            TextInput::new(settings.window.height.to_string(), "", cx)
                .with_id("settings-window-height")
                .with_role(Role::SpinButton)
                .with_aria_label("Height")
                .with_numeric_range(f64::from(settings.window.height), 600.0, 4000.0, 1.0)
        });

        let mut subscriptions = Vec::new();
        for (id, input) in [
            (ControlId::FontSize, &font_size),
            (ControlId::WindowWidth, &window_width),
            (ControlId::WindowHeight, &window_height),
        ] {
            subscriptions.push(cx.subscribe_in(input, window, move |this, _, event, window, cx| {
                this.on_stepper_event(id, event, window, cx);
            }));
        }

        let mut handles = HashMap::new();
        for (index, id) in ControlId::ALL.iter().copied().enumerate() {
            let handle = match id {
                ControlId::FontSize => font_size.read(cx).focus_handle(cx),
                ControlId::WindowWidth => window_width.read(cx).focus_handle(cx),
                ControlId::WindowHeight => window_height.read(cx).focus_handle(cx),
                _ => cx.focus_handle(),
            };
            let tab_index = if id == ControlId::EditSettingsBanner { -1 } else { index as isize };
            handles.insert(id, handle.tab_stop(true).tab_index(tab_index));
        }

        let entity = cx.entity();
        for id in [
            ControlId::Theme,
            ControlId::FontFamily,
            ControlId::FontSize,
            ControlId::WindowWidth,
            ControlId::WindowHeight,
        ] {
            let handle = handles[&id].clone();
            let entity = entity.clone();
            subscriptions.push(window.on_focus_out(&handle, cx, move |_, window, cx| {
                entity.update(cx, |this, cx| this.on_focus_lost(id, window, cx));
            }));
        }

        let mut fonts = vec!["System".to_owned()];
        fonts.extend(crow_macos::available_font_families());
        if !settings.font_family.is_empty() && !fonts.iter().any(|name| name == &settings.font_family) {
            fonts.push(settings.font_family.clone());
        }
        let font_cursor = font_index(&fonts, &settings.font_family);
        let theme = ThemeChoice::new(Vec::new(), &settings.theme);
        let task = theme_choice::load(cx);
        cx.spawn_in(window, async move |this, cx| {
            let infos = task.await;
            this.update_in(cx, |this, _, cx| {
                let current = cx.global::<Ui>().settings.theme.clone();
                this.theme = ThemeChoice::new(infos, &current);
                cx.notify();
            })
            .ok();
        })
        .detach();

        Self {
            focus: cx.focus_handle(),
            handles,
            font_size,
            window_width,
            window_height,
            theme,
            theme_scroll: ScrollHandle::new(),
            fonts,
            font_cursor,
            font_scroll: ScrollHandle::new(),
            theme_type_ahead: TypeAhead::default(),
            font_type_ahead: TypeAhead::default(),
            page_scroll: ScrollHandle::new(),
            control_items: HashMap::new(),
            _subscriptions: subscriptions,
        }
    }

    fn handle(&self, id: ControlId) -> FocusHandle {
        self.handles[&id].clone()
    }

    fn style<'a>(&'a self, id: ControlId, window: &Window, theme: &'a Theme) -> controls::ControlStyle<'a> {
        controls::ControlStyle {
            focus: self.handles.get(&id).expect("control handle"),
            focused: self.is_focused(id, window),
            theme,
        }
    }

    fn is_focused(&self, id: ControlId, window: &Window) -> bool {
        self.handles.get(&id).is_some_and(|handle| handle.is_focused(window))
    }

    fn focused_control(&self, window: &Window) -> Option<ControlId> {
        ControlId::ALL.iter().copied().find(|id| self.is_focused(*id, window))
    }

    fn meta(&self, id: ControlId, cx: &App) -> ControlMeta {
        ControlMeta { id: id.key(), label: id.label(), description: id.description(cx), enabled: id.enabled(cx) }
    }

    fn first_focus(&self, cx: &App) -> FocusHandle {
        let id = if ControlId::Theme.enabled(cx) { ControlId::Theme } else { ControlId::EditSettingsBanner };
        self.handle(id)
    }

    fn focus_first(&self, window: &mut Window, cx: &mut App) {
        let focus = self.first_focus(cx);
        window.focus(&focus, cx);
        self.scroll_to_focused(window);
    }

    fn scroll_to_focused(&self, window: &Window) {
        if let Some(id) = self.focused_control(window)
            && let Some(index) = self.control_items.get(&id)
        {
            self.page_scroll.scroll_to_item(*index);
        }
    }

    fn input(&self, id: ControlId) -> Entity<TextInput> {
        match id {
            ControlId::FontSize => self.font_size.clone(),
            ControlId::WindowWidth => self.window_width.clone(),
            ControlId::WindowHeight => self.window_height.clone(),
            _ => unreachable!("not a stepper"),
        }
    }

    fn setting_value(&self, id: ControlId, cx: &App) -> f64 {
        let settings = &cx.global::<Ui>().settings;
        match id {
            ControlId::FontSize => f64::from(settings.font_size),
            ControlId::WindowWidth => f64::from(settings.window.width),
            ControlId::WindowHeight => f64::from(settings.window.height),
            _ => 0.0,
        }
    }

    fn stepper_value(&self, id: ControlId, cx: &App) -> f64 {
        self.input(id)
            .read(cx)
            .text()
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .unwrap_or_else(|| self.setting_value(id, cx))
    }

    fn sync_stepper(&self, id: ControlId, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_focused(id, window) {
            return;
        }
        let value = self.setting_value(id, cx);
        let text = number_text(value);
        let input = self.input(id);
        if input.read(cx).text() != text {
            input.update(cx, |input, cx| input.set_text(text, window, cx));
        }
        input.update(cx, |input, cx| input.set_numeric_value(value, cx));
    }

    fn set_stepper(&mut self, id: ControlId, value: f64, window: &mut Window, cx: &mut Context<Self>) {
        let (min, max) = stepper_range(id);
        let value = value.clamp(min, max).round();
        match id {
            ControlId::FontSize => ui::set_setting(SettingChange::FontSize(value as f32), cx),
            ControlId::WindowWidth => ui::set_setting(SettingChange::WindowWidth(value as u32), cx),
            ControlId::WindowHeight => ui::set_setting(SettingChange::WindowHeight(value as u32), cx),
            _ => return,
        }
        self.sync_stepper(id, window, cx);
        cx.notify();
    }

    fn commit_stepper(&mut self, id: ControlId, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.stepper_value(id, cx);
        self.set_stepper(id, value, window, cx);
    }

    fn adjust_stepper(&mut self, id: ControlId, delta: f64, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.stepper_value(id, cx) + delta;
        self.set_stepper(id, value, window, cx);
    }

    fn on_stepper_event(&mut self, id: ControlId, event: &TextInputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            TextInputEvent::Confirmed => self.commit_stepper(id, window, cx),
            TextInputEvent::Cancelled => self.close(window, cx),
            TextInputEvent::CompletionRequested => {
                window.focus_next(cx);
                cx.notify();
            }
            TextInputEvent::Changed | TextInputEvent::BackspaceAtStart => {}
        }
    }

    fn on_focus_lost(&mut self, id: ControlId, window: &mut Window, cx: &mut Context<Self>) {
        match id {
            ControlId::Theme => self.commit_theme(window, cx),
            ControlId::FontFamily => self.commit_font(window, cx),
            ControlId::FontSize | ControlId::WindowWidth | ControlId::WindowHeight => {
                self.commit_stepper(id, window, cx);
            }
            _ => {}
        }
    }

    fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.focused_control(window) else { return };
        if !id.enabled(cx) {
            return;
        }
        match id {
            ControlId::Theme => self.commit_theme(window, cx),
            ControlId::FontFamily => self.commit_font(window, cx),
            ControlId::ShowHidden | ControlId::ShowPreview | ControlId::ViewPerFolder | ControlId::PaletteRecent => {
                self.toggle_bool(id, cx)
            }
            ControlId::PlaySound => crate::feedback::play(Event::Trash, cx),
            ControlId::EditSettings | ControlId::EditSettingsBanner => ui::edit_settings_file(cx),
            ControlId::ShowConfig => ui::open_config_folder(cx),
            ControlId::OpenSidebarToml => ui::open_config_file("sidebar.toml", cx),
            ControlId::OpenViewsToml => ui::open_config_file("views.toml", cx),
            ControlId::OpenExcludeToml => ui::open_config_file("exclude.toml", cx),
            ControlId::FontSize | ControlId::WindowWidth | ControlId::WindowHeight => {
                self.commit_stepper(id, window, cx);
            }
            ControlId::AppIcon | ControlId::Keymap | ControlId::Hints | ControlId::Motion | ControlId::Sound => {}
        }
    }

    fn move_horizontal(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.focused_control(window) else { return };
        match id {
            ControlId::Theme => self.move_theme(delta, cx),
            ControlId::FontFamily => self.move_font(delta, window, cx),
            ControlId::AppIcon => self.move_app_icon(delta, window, cx),
            ControlId::Keymap | ControlId::Hints | ControlId::Motion | ControlId::Sound => {
                self.move_segment(id, delta, window, cx);
            }
            _ => {}
        }
    }

    fn move_vertical(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.focused_control(window) else { return };
        match id {
            ControlId::Theme => self.move_theme(delta, cx),
            ControlId::FontFamily => self.move_font(delta, window, cx),
            ControlId::FontSize | ControlId::WindowWidth | ControlId::WindowHeight => {
                // ↑ is -1 in reading order but +1 for a stepper.
                self.adjust_stepper(id, -(delta as f64), window, cx);
            }
            _ => {}
        }
    }

    fn adjust_focused(&mut self, amount: f64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.focused_control(window) else { return };
        if matches!(id, ControlId::FontSize | ControlId::WindowWidth | ControlId::WindowHeight) {
            self.adjust_stepper(id, amount, window, cx);
        }
    }

    fn move_segment(&mut self, id: ControlId, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let (current, count) = match id {
            ControlId::Keymap => (keymap_index(cx.global::<Ui>().settings.keymap), 2),
            ControlId::Hints => (hints_index(cx.global::<Ui>().settings.hints), 3),
            ControlId::Motion => (motion_index(cx.global::<Ui>().settings.motion), 4),
            ControlId::Sound => (sound_index(cx.global::<Ui>().settings.sound), 3),
            _ => return,
        };
        let next = (current as isize + delta).clamp(0, count - 1) as usize;
        if next != current {
            self.select_segment(id, next, window, cx);
        }
    }

    fn select_segment(&mut self, id: ControlId, index: usize, _window: &mut Window, cx: &mut Context<Self>) {
        let change = match id {
            ControlId::Keymap => SettingChange::Keymap(if index == 0 { KeymapLayer::Finder } else { KeymapLayer::Vim }),
            ControlId::Hints => SettingChange::Hints(match index {
                0 => HintsMode::Auto,
                1 => HintsMode::Always,
                _ => HintsMode::Off,
            }),
            ControlId::Motion => SettingChange::Motion(match index {
                0 => MotionMode::Auto,
                1 => MotionMode::Off,
                2 => MotionMode::Low,
                _ => MotionMode::Full,
            }),
            ControlId::Sound => SettingChange::Sound(match index {
                0 => SoundSet::Crow,
                1 => SoundSet::Finder,
                _ => SoundSet::Off,
            }),
            _ => return,
        };
        ui::set_setting(change, cx);
        cx.notify();
    }

    fn toggle_bool(&mut self, id: ControlId, cx: &mut Context<Self>) {
        let settings = &cx.global::<Ui>().settings;
        let change = match id {
            ControlId::ShowHidden => SettingChange::ShowHiddenFiles(!settings.show_hidden_files),
            ControlId::ShowPreview => SettingChange::ShowPreview(!settings.show_preview),
            ControlId::ViewPerFolder => SettingChange::ViewPerFolder(!settings.view_per_folder),
            ControlId::PaletteRecent => SettingChange::PaletteRecentCommands(!settings.palette_recent_commands),
            _ => return,
        };
        ui::set_setting(change, cx);
        cx.notify();
    }

    fn move_app_icon(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let current = app_icon_index(cx.global::<Ui>().settings.app_icon);
        let next = (current as isize + delta).clamp(0, 3) as usize;
        if next != current {
            self.select_app_icon(next, window, cx);
        }
    }

    fn select_app_icon(&mut self, index: usize, _window: &mut Window, cx: &mut Context<Self>) {
        let icon = match index {
            0 => AppIcon::Refined,
            1 => AppIcon::Sun,
            2 => AppIcon::Graphite,
            _ => AppIcon::Bone,
        };
        ui::set_setting(SettingChange::AppIcon(icon), cx);
        cx.notify();
    }

    fn move_theme(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.theme.move_cursor(delta) {
            theme_choice::preview(&self.theme.current_id(), cx);
            self.theme_scroll.scroll_to_item(self.theme.cursor);
            cx.notify();
        }
    }

    fn select_theme(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.theme.select(index) {
            self.commit_theme(window, cx);
            self.theme_scroll.scroll_to_item(self.theme.cursor);
            cx.notify();
        }
    }

    fn commit_theme(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let id = self.theme.current_id();
        if cx.global::<Ui>().settings.theme == id {
            cx.global_mut::<Ui>().preview_theme = None;
        } else {
            ui::choose_theme(&id, cx);
        }
    }

    fn theme_type_ahead(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let labels: Vec<String> = (0..self.theme.entries.len()).map(|i| self.theme.label(i)).collect();
        if let Some(index) = type_ahead(&mut self.theme_type_ahead, &event.keystroke, &labels)
            && self.theme.select(index)
        {
            theme_choice::preview(&self.theme.current_id(), cx);
            self.theme_scroll.scroll_to_item(self.theme.cursor);
            cx.notify();
        }
    }

    fn move_font(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let last = self.fonts.len().saturating_sub(1);
        let next = self.font_cursor.saturating_add_signed(delta).min(last);
        if next != self.font_cursor {
            self.select_font(next, window, cx);
        }
    }

    fn select_font(&mut self, index: usize, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.fonts.get(index).cloned() else { return };
        self.font_cursor = index;
        let family = if name == "System" { String::new() } else { name };
        ui::set_setting(SettingChange::FontFamily(family), cx);
        self.font_scroll.scroll_to_item(index);
        cx.notify();
    }

    fn commit_font(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let family = &cx.global::<Ui>().settings.font_family;
        self.font_cursor = font_index(&self.fonts, family);
    }

    fn font_type_ahead(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = type_ahead(&mut self.font_type_ahead, &event.keystroke, &self.fonts) {
            self.select_font(index, window, cx);
        }
    }

    /// Commit a typed stepper value or a previewed theme before focus leaves
    /// it. Tab and Shift-Tab call this explicitly; the focus-out listeners cover
    /// a click elsewhere.
    fn commit_focused(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.focused_control(window) {
            Some(ControlId::Theme) => self.commit_theme(window, cx),
            Some(ControlId::FontSize) => self.commit_stepper(ControlId::FontSize, window, cx),
            Some(ControlId::WindowWidth) => self.commit_stepper(ControlId::WindowWidth, window, cx),
            Some(ControlId::WindowHeight) => self.commit_stepper(ControlId::WindowHeight, window, cx),
            _ => {}
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_focused(window, cx);
        window.remove_window();
    }
}

/// The state a control announces, for the accessibility invariant and the
/// keystroke tests. The renderer reads the same `Ui.settings` and cursors.
#[cfg(test)]
#[derive(Debug, PartialEq)]
struct ControlState {
    role: Role,
    label: String,
    enabled: bool,
    selected: Option<usize>,
    toggled: Option<bool>,
    value: Option<f64>,
}

#[cfg(test)]
impl SettingsWindow {
    fn control_state(&self, id: ControlId, cx: &App) -> ControlState {
        let settings = &cx.global::<Ui>().settings;
        ControlState {
            role: id.role(),
            label: id.label().to_owned(),
            enabled: id.enabled(cx),
            selected: match id {
                ControlId::Theme => Some(self.theme.cursor),
                ControlId::AppIcon => Some(app_icon_index(settings.app_icon)),
                ControlId::FontFamily => Some(self.font_cursor),
                ControlId::Keymap => Some(keymap_index(settings.keymap)),
                ControlId::Hints => Some(hints_index(settings.hints)),
                ControlId::Motion => Some(motion_index(settings.motion)),
                ControlId::Sound => Some(sound_index(settings.sound)),
                _ => None,
            },
            toggled: match id {
                ControlId::ShowHidden => Some(settings.show_hidden_files),
                ControlId::ShowPreview => Some(settings.show_preview),
                ControlId::ViewPerFolder => Some(settings.view_per_folder),
                ControlId::PaletteRecent => Some(settings.palette_recent_commands),
                _ => None,
            },
            value: match id {
                ControlId::FontSize => Some(f64::from(settings.font_size)),
                ControlId::WindowWidth => Some(f64::from(settings.window.width)),
                ControlId::WindowHeight => Some(f64::from(settings.window.height)),
                _ => None,
            },
        }
    }

    fn set_theme_entries_for_tests(&mut self, infos: Vec<crow_theme::ThemeInfo>, current: &str) {
        self.theme = ThemeChoice::new(infos, current);
    }

    fn set_fonts_for_tests(&mut self, fonts: Vec<String>, current: &str) {
        self.fonts = fonts;
        self.font_cursor = font_index(&self.fonts, current);
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        let font = ui_font(&ui.settings.font_family);
        let font_size = ui.settings.font_size;
        let settings = ui.settings.clone();
        let broken = ui.settings_error();

        if !self.is_focused(ControlId::Theme, window) {
            self.theme.sync_to(&settings.theme);
        }
        if !self.is_focused(ControlId::FontFamily, window) {
            self.font_cursor = font_index(&self.fonts, &settings.font_family);
        }
        for id in [ControlId::FontSize, ControlId::WindowWidth, ControlId::WindowHeight] {
            self.sync_stepper(id, window, cx);
        }
        for id in ControlId::ALL {
            let tab_stop = if id == ControlId::EditSettingsBanner { broken.is_some() } else { id.enabled(cx) };
            self.handle(id).tab_stop(tab_stop);
        }

        let theme_labels: Vec<String> = (0..self.theme.entries.len()).map(|i| self.theme.label(i)).collect();
        let theme_select = select_callback(cx, |this, index, window, cx| this.select_theme(index, window, cx));
        let theme_row = controls::choice_list(
            self.meta(ControlId::Theme, cx),
            &theme_labels,
            self.theme.cursor,
            self.style(ControlId::Theme, window, &theme),
            &self.theme_scroll,
            theme_select,
        )
        .on_key_down(cx.listener(|this, event, _, cx| this.theme_type_ahead(event, cx)));

        let tiles = [AppIcon::Refined, AppIcon::Sun, AppIcon::Graphite, AppIcon::Bone]
            .map(|icon| controls::Tile { label: app_icon_label(icon), image: app_icon_image(icon) });
        let app_icon_select = select_callback(cx, |this, index, window, cx| this.select_app_icon(index, window, cx));
        let app_icon_row = controls::tile_choice(
            self.meta(ControlId::AppIcon, cx),
            &tiles,
            app_icon_index(settings.app_icon),
            self.style(ControlId::AppIcon, window, &theme),
            app_icon_select,
        );

        let font_select = select_callback(cx, |this, index, window, cx| this.select_font(index, window, cx));
        let font_row = controls::choice_list(
            self.meta(ControlId::FontFamily, cx),
            &self.fonts,
            self.font_cursor,
            self.style(ControlId::FontFamily, window, &theme),
            &self.font_scroll,
            font_select,
        )
        .on_key_down(cx.listener(|this, event, window, cx| this.font_type_ahead(event, window, cx)));

        let font_size_row = controls::stepper(
            self.meta(ControlId::FontSize, cx),
            &self.font_size,
            self.style(ControlId::FontSize, window, &theme),
        );

        let keymap_select =
            select_callback(cx, |this, index, window, cx| this.select_segment(ControlId::Keymap, index, window, cx));
        let keymap_row = controls::segmented(
            self.meta(ControlId::Keymap, cx),
            &["Finder", "Vim"],
            keymap_index(settings.keymap),
            self.style(ControlId::Keymap, window, &theme),
            keymap_select,
            None,
        );

        let hints_select =
            select_callback(cx, |this, index, window, cx| this.select_segment(ControlId::Hints, index, window, cx));
        let hints_row = controls::segmented(
            self.meta(ControlId::Hints, cx),
            &["Auto", "Always", "Off"],
            hints_index(settings.hints),
            self.style(ControlId::Hints, window, &theme),
            hints_select,
            None,
        );

        let show_hidden = switch_row(self, ControlId::ShowHidden, settings.show_hidden_files, window, &theme, cx);
        let show_preview = switch_row(self, ControlId::ShowPreview, settings.show_preview, window, &theme, cx);
        let view_per_folder = switch_row(self, ControlId::ViewPerFolder, settings.view_per_folder, window, &theme, cx);
        let palette_recent =
            switch_row(self, ControlId::PaletteRecent, settings.palette_recent_commands, window, &theme, cx);

        let motion_select =
            select_callback(cx, |this, index, window, cx| this.select_segment(ControlId::Motion, index, window, cx));
        let motion_row = controls::segmented(
            self.meta(ControlId::Motion, cx),
            &["Auto", "Off", "Low", "Full"],
            motion_index(settings.motion),
            self.style(ControlId::Motion, window, &theme),
            motion_select,
            None,
        );

        let sound_select =
            select_callback(cx, |this, index, window, cx| this.select_segment(ControlId::Sound, index, window, cx));
        let play = controls::button(
            ControlId::PlaySound.key(),
            "Play",
            false,
            ControlId::PlaySound.enabled(cx),
            self.style(ControlId::PlaySound, window, &theme),
            callback(cx, |_, _, cx| crate::feedback::play(Event::Trash, cx)),
        );
        let sound_row = controls::segmented(
            self.meta(ControlId::Sound, cx),
            &["Crow", "Finder", "Off"],
            sound_index(settings.sound),
            self.style(ControlId::Sound, window, &theme),
            sound_select,
            Some(play.into_any_element()),
        );

        let width_row = controls::stepper(
            self.meta(ControlId::WindowWidth, cx),
            &self.window_width,
            self.style(ControlId::WindowWidth, window, &theme),
        );
        let height_row = controls::stepper(
            self.meta(ControlId::WindowHeight, cx),
            &self.window_height,
            self.style(ControlId::WindowHeight, window, &theme),
        );

        let edit = controls::button(
            ControlId::EditSettings.key(),
            "Edit settings.toml…",
            false,
            true,
            self.style(ControlId::EditSettings, window, &theme),
            callback(cx, |_, _, cx| ui::edit_settings_file(cx)),
        );
        let show_config = controls::button(
            ControlId::ShowConfig.key(),
            "Show config folder in Finder",
            false,
            true,
            self.style(ControlId::ShowConfig, window, &theme),
            callback(cx, |_, _, cx| ui::open_config_folder(cx)),
        );
        let files_line = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap(px(6.))
            .text_size(px(11.))
            .text_color(color(theme.role("fg-muted")))
            .child("Sidebar groups, folder views and search rules stay in")
            .child(link(self, ControlId::OpenSidebarToml, "sidebar.toml", window, &theme, cx))
            .child(link(self, ControlId::OpenViewsToml, "views.toml", window, &theme, cx))
            .child(link(self, ControlId::OpenExcludeToml, "exclude.toml", window, &theme, cx));

        let banner = broken.map(|error| {
            let edit = controls::button(
                "settings-error-edit",
                "Edit settings.toml…",
                false,
                true,
                self.style(ControlId::EditSettingsBanner, window, &theme),
                callback(cx, |_, _, cx| ui::edit_settings_file(cx)),
            );
            div()
                .id("settings-error")
                .role(Role::Alert)
                .aria_label(error.clone())
                .flex()
                .items_center()
                .gap(px(10.))
                .mx(px(20.))
                .mt(px(14.))
                .p(px(10.))
                .rounded_md()
                .bg(error_soft(&theme))
                .text_color(color(theme.role("error")))
                .child(div().flex_1().text_size(px(12.)).child(error))
                .child(edit)
        });

        let mut page_items: Vec<gpui::AnyElement> = Vec::new();
        let mut control_items: HashMap<ControlId, usize> = HashMap::new();
        add_section(
            &mut page_items,
            &mut control_items,
            "Appearance",
            &theme,
            vec![
                (ControlId::Theme, theme_row.into_any_element()),
                (ControlId::AppIcon, app_icon_row.into_any_element()),
                (ControlId::FontFamily, font_row.into_any_element()),
                (ControlId::FontSize, font_size_row.into_any_element()),
            ],
        );
        add_section(
            &mut page_items,
            &mut control_items,
            "Behaviour",
            &theme,
            vec![
                (ControlId::Keymap, keymap_row.into_any_element()),
                (ControlId::Hints, hints_row.into_any_element()),
                (ControlId::ShowHidden, show_hidden.into_any_element()),
                (ControlId::ShowPreview, show_preview.into_any_element()),
                (ControlId::ViewPerFolder, view_per_folder.into_any_element()),
                (ControlId::PaletteRecent, palette_recent.into_any_element()),
            ],
        );
        add_section(
            &mut page_items,
            &mut control_items,
            "Motion & sound",
            &theme,
            vec![(ControlId::Motion, motion_row.into_any_element()), (ControlId::Sound, sound_row.into_any_element())],
        );
        let sound_index = control_items[&ControlId::Sound];
        control_items.insert(ControlId::PlaySound, sound_index);
        add_section(
            &mut page_items,
            &mut control_items,
            "New windows",
            &theme,
            vec![
                (ControlId::WindowWidth, width_row.into_any_element()),
                (ControlId::WindowHeight, height_row.into_any_element()),
            ],
        );
        add_section(
            &mut page_items,
            &mut control_items,
            "Files",
            &theme,
            vec![
                (
                    ControlId::EditSettings,
                    div()
                        .flex()
                        .items_center()
                        .flex_wrap()
                        .gap(px(8.))
                        .child(edit)
                        .child(show_config)
                        .into_any_element(),
                ),
                (ControlId::OpenSidebarToml, files_line.into_any_element()),
            ],
        );
        let files_index = control_items[&ControlId::EditSettings];
        control_items.insert(ControlId::ShowConfig, files_index);
        let links_index = control_items[&ControlId::OpenSidebarToml];
        control_items.insert(ControlId::OpenViewsToml, links_index);
        control_items.insert(ControlId::OpenExcludeToml, links_index);
        self.control_items = control_items;

        div()
            .id("settings-window")
            .role(Role::Group)
            .aria_label("Settings")
            .key_context("Settings")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Activate, window, cx| this.activate(window, cx)))
            .on_action(cx.listener(|this, _: &Left, window, cx| this.move_horizontal(-1, window, cx)))
            .on_action(cx.listener(|this, _: &Right, window, cx| this.move_horizontal(1, window, cx)))
            .on_action(cx.listener(|this, _: &Up, window, cx| this.move_vertical(-1, window, cx)))
            .on_action(cx.listener(|this, _: &Down, window, cx| this.move_vertical(1, window, cx)))
            .on_action(cx.listener(|this, _: &IncrementTen, window, cx| this.adjust_focused(10.0, window, cx)))
            .on_action(cx.listener(|this, _: &DecrementTen, window, cx| this.adjust_focused(-10.0, window, cx)))
            .on_action(cx.listener(|this, _: &FocusNext, window, cx| {
                this.commit_focused(window, cx);
                window.focus_next(cx);
                this.scroll_to_focused(window);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &FocusPrev, window, cx| {
                this.commit_focused(window, cx);
                window.focus_prev(cx);
                this.scroll_to_focused(window);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CloseSettings, window, cx| this.close(window, cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                window.activate_window();
                this.focus_first(window, cx);
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(color(theme.role("bg")))
            .text_color(color(theme.role("fg")))
            .font_family(font)
            .text_size(px(font_size))
            .children(banner)
            .child(
                div()
                    .id("settings-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.page_scroll)
                    .p(px(20.))
                    .flex()
                    .flex_col()
                    .children(page_items),
            )
    }
}

fn section_heading(title: &'static str, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(title)
        .pt(px(16.))
        .pb(px(2.))
        .role(Role::Heading)
        .aria_level(2)
        .text_size(px(15.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color(theme.role("fg")))
        .child(title)
}

/// Add a section heading and its rows to the page's flat child list. The index
/// recorded for each control is what `ScrollHandle::scroll_to_item` takes.
fn add_section(
    page: &mut Vec<gpui::AnyElement>,
    indices: &mut HashMap<ControlId, usize>,
    title: &'static str,
    theme: &Theme,
    rows: Vec<(ControlId, gpui::AnyElement)>,
) {
    page.push(section_heading(title, theme).into_any_element());
    for (id, row) in rows {
        indices.insert(id, page.len());
        page.push(div().pb(px(12.)).child(row).into_any_element());
    }
}

fn switch_row(
    this: &SettingsWindow,
    id: ControlId,
    on: bool,
    window: &Window,
    theme: &Theme,
    cx: &mut Context<SettingsWindow>,
) -> gpui::Stateful<gpui::Div> {
    let toggle = callback(cx, move |this, _, cx| this.toggle_bool(id, cx));
    controls::switch(this.meta(id, cx), on, this.style(id, window, theme), toggle)
}

fn link(
    this: &SettingsWindow,
    id: ControlId,
    label: &'static str,
    window: &Window,
    theme: &Theme,
    cx: &mut Context<SettingsWindow>,
) -> gpui::Stateful<gpui::Div> {
    let name = match id {
        ControlId::OpenSidebarToml => "sidebar.toml",
        ControlId::OpenViewsToml => "views.toml",
        ControlId::OpenExcludeToml => "exclude.toml",
        _ => unreachable!(),
    };
    let activate = callback(cx, move |_, _, cx| ui::open_config_file(name, cx));
    controls::link_button(id.key(), label, id.enabled(cx), this.style(id, window, theme), activate)
}

fn callback<T: 'static>(
    cx: &mut Context<T>,
    f: impl Fn(&mut T, &mut Window, &mut Context<T>) + 'static,
) -> controls::Activate {
    let entity = cx.entity();
    Rc::new(move |window, cx| {
        entity.update(cx, |this, cx| f(this, window, cx));
    })
}

fn select_callback<T: 'static>(
    cx: &mut Context<T>,
    f: impl Fn(&mut T, usize, &mut Window, &mut Context<T>) + 'static,
) -> controls::Select {
    let entity = cx.entity();
    Rc::new(move |index, window, cx| {
        entity.update(cx, |this, cx| f(this, index, window, cx));
    })
}

fn type_ahead(state: &mut TypeAhead, keystroke: &gpui::Keystroke, labels: &[String]) -> Option<usize> {
    if keystroke.modifiers.control || keystroke.modifiers.alt || keystroke.modifiers.platform {
        return None;
    }
    let text = keystroke.key_char.as_deref()?;
    if text.is_empty() || text.chars().any(char::is_control) {
        return None;
    }
    let now = Instant::now();
    if state.at.is_none_or(|at| now.duration_since(at) > Duration::from_millis(800)) {
        state.buffer.clear();
    }
    state.buffer.push_str(text);
    state.at = Some(now);
    let needle = state.buffer.to_lowercase();
    let found = labels.iter().position(|label| label.to_lowercase().starts_with(&needle));
    if found.is_none() {
        state.buffer.clear();
    }
    found
}

fn number_text(value: f64) -> String {
    format!("{}", value.round() as i64)
}

fn stepper_range(id: ControlId) -> (f64, f64) {
    match id {
        ControlId::FontSize => (10.0, 24.0),
        _ => (600.0, 4000.0),
    }
}

fn font_index(fonts: &[String], family: &str) -> usize {
    fonts.iter().position(|name| (name == "System" && family.is_empty()) || name == family).unwrap_or(0)
}

fn keymap_index(value: KeymapLayer) -> usize {
    match value {
        KeymapLayer::Finder => 0,
        KeymapLayer::Vim => 1,
    }
}

fn hints_index(value: HintsMode) -> usize {
    match value {
        HintsMode::Auto => 0,
        HintsMode::Always => 1,
        HintsMode::Off => 2,
    }
}

fn motion_index(value: MotionMode) -> usize {
    match value {
        MotionMode::Auto => 0,
        MotionMode::Off => 1,
        MotionMode::Low => 2,
        MotionMode::Full => 3,
    }
}

fn sound_index(value: SoundSet) -> usize {
    match value {
        SoundSet::Crow => 0,
        SoundSet::Finder => 1,
        SoundSet::Off => 2,
    }
}

fn app_icon_index(value: AppIcon) -> usize {
    match value {
        AppIcon::Refined => 0,
        AppIcon::Sun => 1,
        AppIcon::Graphite => 2,
        AppIcon::Bone => 3,
    }
}

fn app_icon_label(value: AppIcon) -> &'static str {
    match value {
        AppIcon::Refined => "Refined",
        AppIcon::Sun => "Sun",
        AppIcon::Graphite => "Graphite",
        AppIcon::Bone => "Bone",
    }
}

fn app_icon_image(icon: AppIcon) -> Arc<Image> {
    static ICONS: LazyLock<[Arc<Image>; 4]> = LazyLock::new(|| {
        [AppIcon::Refined, AppIcon::Sun, AppIcon::Graphite, AppIcon::Bone]
            .map(|icon| Arc::new(Image::from_bytes(ImageFormat::Png, ui::app_icon_png(icon).to_vec())))
    });
    ICONS[app_icon_index(icon)].clone()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crow_theme::{ThemeInfo, Variant};
    use gpui::{TestAppContext, VisualTestContext, WindowHandle};

    use super::*;

    fn fixture(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("crow-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create fixture");
        dir
    }

    fn setup(cx: &mut TestAppContext) {
        let trash = std::env::temp_dir().join(format!("crow-settings-trash-{}", std::process::id()));
        std::fs::create_dir_all(&trash).ok();
        cx.update(|cx| {
            crate::ui::init_for_tests(cx);
            crate::operations::init(cx, std::sync::Arc::new(crate::operations::FolderTrash(trash)));
            cx.bind_keys(crate::actions::finder_keymap());
            cx.bind_keys(crate::actions::vim_keymap());
            cx.bind_keys(crate::text_input::keymap());
            cx.bind_keys(keymap());
            crate::menus::init(cx);
        });
    }

    fn open_settings(cx: &mut TestAppContext) -> (Entity<SettingsWindow>, VisualTestContext) {
        setup(cx);
        let window: WindowHandle<SettingsWindow> = cx.update(|cx| {
            let bounds = Bounds::centered(None, size(px(560.), px(760.)), cx);
            let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() };
            cx.open_window(options, |window, cx| cx.new(|cx| SettingsWindow::new(window, cx)))
                .expect("open settings window")
        });
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        let view = window.root(&mut vcx).expect("root view");
        vcx.run_until_parked();
        cx.run_until_parked();
        let focus = view.read_with(&vcx, |view, cx| view.first_focus(cx));
        vcx.update(|window, cx| window.focus(&focus, cx));
        draw(&mut vcx);
        (view, vcx)
    }

    fn draw(vcx: &mut VisualTestContext) {
        vcx.update(|window, cx| {
            let clear = window.draw(cx);
            clear.clear(cx);
        });
        vcx.cx.run_until_parked();
        vcx.run_until_parked();
    }

    fn focus_control(view: &Entity<SettingsWindow>, vcx: &mut VisualTestContext, target: ControlId) {
        for _ in 0..ControlId::ALL.len() * 2 {
            let focused = vcx.update(|window, cx| view.read(cx).focused_control(window));
            if focused == Some(target) {
                return;
            }
            vcx.simulate_keystrokes("tab");
        }
        panic!("never focused {target:?}");
    }

    fn state(view: &Entity<SettingsWindow>, vcx: &VisualTestContext, id: ControlId) -> ControlState {
        view.read_with(vcx, |view, cx| view.control_state(id, cx))
    }

    fn settings_text(vcx: &VisualTestContext) -> String {
        let dir = vcx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
        std::fs::read_to_string(dir.join("settings.toml")).expect("settings file")
    }

    fn wait_for(vcx: &VisualTestContext, needle: &str) {
        for _ in 0..100 {
            vcx.run_until_parked();
            if settings_text(vcx).contains(needle) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("never wrote {needle:?}: {}", settings_text(vcx));
    }

    #[gpui::test]
    fn cmd_comma_opens_one_settings_window(cx: &mut TestAppContext) {
        setup(cx);
        let root = fixture("open");
        let browser = cx.update(|cx| crate::window::open(root.clone(), false, None, cx)).expect("browser window");
        let mut vcx = VisualTestContext::from_window(browser.into(), cx);
        vcx.run_until_parked();

        vcx.simulate_keystrokes("cmd-,");
        cx.run_until_parked();
        let settings: Vec<_> =
            cx.update(|cx| cx.windows().into_iter().filter_map(|handle| handle.downcast::<SettingsWindow>()).collect());
        assert_eq!(settings.len(), 1, "⌘, opens one Settings window");
        assert!(
            settings[0]
                .update(cx, |view, window, _| view.is_focused(ControlId::Theme, window))
                .expect("settings window"),
            "the first control is focused"
        );

        vcx.simulate_keystrokes("cmd-,");
        cx.run_until_parked();
        let settings: Vec<_> =
            cx.update(|cx| cx.windows().into_iter().filter_map(|handle| handle.downcast::<SettingsWindow>()).collect());
        assert_eq!(settings.len(), 1, "a second ⌘, focuses the one window");
        assert_eq!(cx.update(|cx| cx.windows().len()), 2, "browser + settings");
        std::fs::remove_dir_all(root).ok();
    }

    #[gpui::test]
    fn cmd_w_closes_the_settings_window(cx: &mut TestAppContext) {
        let (_view, mut vcx) = open_settings(cx);
        vcx.simulate_keystrokes("cmd-w");
        cx.run_until_parked();
        let count = cx.update(|cx| {
            cx.windows().into_iter().filter(|handle| handle.downcast::<SettingsWindow>().is_some()).count()
        });
        assert_eq!(count, 0);
    }

    #[gpui::test]
    fn escape_closes_the_settings_window(cx: &mut TestAppContext) {
        let (_view, mut vcx) = open_settings(cx);
        vcx.simulate_keystrokes("escape");
        cx.run_until_parked();
        let count = cx.update(|cx| {
            cx.windows().into_iter().filter(|handle| handle.downcast::<SettingsWindow>().is_some()).count()
        });
        assert_eq!(count, 0);
    }

    #[gpui::test]
    fn switch_toggles_with_space(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        focus_control(&view, &mut vcx, ControlId::ShowHidden);
        assert_eq!(state(&view, &vcx, ControlId::ShowHidden).toggled, Some(true));
        vcx.simulate_keystrokes("space");
        assert_eq!(state(&view, &vcx, ControlId::ShowHidden).toggled, Some(false));
        assert!(!vcx.read_global::<Ui, _>(|ui, _| ui.settings.show_hidden_files));
        wait_for(&vcx, "show_hidden_files = false");
    }

    #[gpui::test]
    fn segmented_moves_with_arrows(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        focus_control(&view, &mut vcx, ControlId::Keymap);
        vcx.simulate_keystrokes("right");
        assert_eq!(state(&view, &vcx, ControlId::Keymap).selected, Some(1));
        assert_eq!(vcx.read_global::<Ui, _>(|ui, _| ui.settings.keymap), KeymapLayer::Vim);
        wait_for(&vcx, "keymap = \"vim\"");
        vcx.simulate_keystrokes("left");
        assert_eq!(state(&view, &vcx, ControlId::Keymap).selected, Some(0));
        wait_for(&vcx, "keymap = \"finder\"");
    }

    #[gpui::test]
    fn tile_choice_moves_with_arrows(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        focus_control(&view, &mut vcx, ControlId::AppIcon);
        vcx.simulate_keystrokes("right");
        assert_eq!(state(&view, &vcx, ControlId::AppIcon).selected, Some(1));
        assert_eq!(vcx.read_global::<Ui, _>(|ui, _| ui.settings.app_icon), AppIcon::Sun);
        wait_for(&vcx, "app_icon = \"sun\"");
    }

    #[gpui::test]
    fn stepper_arrows_and_typed_digits(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        focus_control(&view, &mut vcx, ControlId::FontSize);
        vcx.simulate_keystrokes("up");
        assert_eq!(state(&view, &vcx, ControlId::FontSize).value, Some(14.0));
        wait_for(&vcx, "font_size = 14.0");
        vcx.simulate_keystrokes("shift-down");
        assert_eq!(state(&view, &vcx, ControlId::FontSize).value, Some(10.0), "14 - 10 clamps to 10");
        wait_for(&vcx, "font_size = 10.0");
        vcx.simulate_keystrokes("cmd-a");
        vcx.simulate_input("20");
        vcx.simulate_keystrokes("enter");
        assert_eq!(state(&view, &vcx, ControlId::FontSize).value, Some(20.0));
        wait_for(&vcx, "font_size = 20.0");
    }

    #[gpui::test]
    fn theme_list_previews_and_commits_on_leaving(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        view.update(&mut vcx, |view, cx| {
            view.set_theme_entries_for_tests(
                vec![
                    ThemeInfo { id: "dark-one".into(), name: "Dark One".into(), variant: Variant::Dark },
                    ThemeInfo { id: "light-one".into(), name: "Light One".into(), variant: Variant::Light },
                ],
                "system",
            );
            cx.notify();
        });
        vcx.run_until_parked();
        focus_control(&view, &mut vcx, ControlId::Theme);
        vcx.simulate_keystrokes("down");
        assert_eq!(state(&view, &vcx, ControlId::Theme).selected, Some(1), "the cursor moved");
        assert_eq!(
            vcx.read_global::<Ui, _>(|ui, _| ui.preview_theme.clone()),
            Some("dark-one".to_owned()),
            "moving previews"
        );
        assert_eq!(vcx.read_global::<Ui, _>(|ui, _| ui.settings.theme.clone()), "system", "not committed yet");
        vcx.simulate_keystrokes("tab");
        draw(&mut vcx);
        assert_eq!(vcx.read_global::<Ui, _>(|ui, _| ui.settings.theme.clone()), "dark-one", "leaving commits");
        wait_for(&vcx, "theme = \"dark-one\"");
    }

    #[gpui::test]
    fn font_list_selects_and_types(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        view.update(&mut vcx, |view, cx| {
            view.set_fonts_for_tests(vec!["System".into(), "Menlo".into(), "Zed Mono".into()], "");
            cx.notify();
        });
        vcx.run_until_parked();
        focus_control(&view, &mut vcx, ControlId::FontFamily);
        vcx.simulate_keystrokes("down");
        assert_eq!(state(&view, &vcx, ControlId::FontFamily).selected, Some(1));
        assert_eq!(vcx.read_global::<Ui, _>(|ui, _| ui.settings.font_family.clone()), "Menlo");
        wait_for(&vcx, "font_family = \"Menlo\"");
        vcx.simulate_input("zed");
        assert_eq!(state(&view, &vcx, ControlId::FontFamily).selected, Some(2));
        assert_eq!(vcx.read_global::<Ui, _>(|ui, _| ui.settings.font_family.clone()), "Zed Mono");
        wait_for(&vcx, "font_family = \"Zed Mono\"");
    }

    #[gpui::test]
    fn focus_ring_does_not_resize_a_control(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        draw(&mut vcx);
        let before = vcx.debug_bounds("settings-theme-control").expect("theme control bounds");
        focus_control(&view, &mut vcx, ControlId::AppIcon);
        draw(&mut vcx);
        let after = vcx.debug_bounds("settings-theme-control").expect("theme control bounds");
        assert_eq!(before, after, "the focus ring is inset and never resizes the control");
    }

    #[gpui::test]
    fn tabbing_to_a_lower_control_scrolls_the_page(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        draw(&mut vcx);
        let before = view.read_with(&vcx, |view, _| view.page_scroll.offset().y);
        focus_control(&view, &mut vcx, ControlId::EditSettings);
        draw(&mut vcx);
        let after = view.read_with(&vcx, |view, _| view.page_scroll.offset().y);
        assert!(after < before, "the page scrolls to the focused control: {before:?} -> {after:?}");
    }

    #[gpui::test]
    fn every_control_has_a_role_and_label(cx: &mut TestAppContext) {
        let (view, vcx) = open_settings(cx);
        for id in ControlId::ALL {
            let state = state(&view, &vcx, id);
            assert!(!state.label.is_empty(), "{id:?} has a label");
            assert_eq!(state.role, id.role(), "{id:?} has its role");
        }
    }

    #[gpui::test]
    fn broken_file_disables_controls_and_fixing_clears_it(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        let dir = vcx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
        std::fs::write(dir.join("settings.toml"), "motion = \"slow\"\n").expect("break file");
        vcx.update(|_, cx| ui::reload_config(cx));
        vcx.run_until_parked();
        assert!(vcx.read_global::<Ui, _>(|ui, _| ui.settings_error().is_some()));
        assert!(!state(&view, &vcx, ControlId::Theme).enabled, "writing controls are disabled");
        assert!(state(&view, &vcx, ControlId::EditSettings).enabled, "the file button stays usable");
        assert!(state(&view, &vcx, ControlId::EditSettingsBanner).enabled, "the banner button is usable");
        assert_eq!(
            view.read_with(&vcx, |view, cx| view.first_focus(cx)),
            view.read_with(&vcx, |view, _| view.handle(ControlId::EditSettingsBanner)),
            "focus starts at the banner's Edit button while the file is broken"
        );
        std::fs::write(dir.join("settings.toml"), "motion = \"auto\"\n").expect("fix file");
        vcx.update(|_, cx| ui::reload_config(cx));
        vcx.run_until_parked();
        assert!(vcx.read_global::<Ui, _>(|ui, _| ui.settings_error().is_none()));
        assert!(state(&view, &vcx, ControlId::Theme).enabled);
    }

    #[gpui::test]
    fn external_edits_update_the_control(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        let dir = vcx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
        std::fs::write(dir.join("settings.toml"), "keymap = \"vim\"\n").expect("edit file");
        vcx.update(|_, cx| ui::reload_config(cx));
        vcx.run_until_parked();
        assert_eq!(state(&view, &vcx, ControlId::Keymap).selected, Some(1), "Vim shows");
        std::fs::write(dir.join("settings.toml"), "\n").expect("delete line");
        vcx.update(|_, cx| ui::reload_config(cx));
        vcx.run_until_parked();
        assert_eq!(state(&view, &vcx, ControlId::Keymap).selected, Some(0), "Finder shows again");
    }

    #[gpui::test]
    fn reload_after_a_write_does_not_flip_the_control(cx: &mut TestAppContext) {
        let (view, mut vcx) = open_settings(cx);
        focus_control(&view, &mut vcx, ControlId::Keymap);
        vcx.simulate_keystrokes("right");
        wait_for(&vcx, "keymap = \"vim\"");
        vcx.update(|_, cx| ui::reload_config(cx));
        vcx.run_until_parked();
        assert_eq!(state(&view, &vcx, ControlId::Keymap).selected, Some(1));
        assert_eq!(vcx.read_global::<Ui, _>(|ui, _| ui.settings.keymap), KeymapLayer::Vim);
    }
}
