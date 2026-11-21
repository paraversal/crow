//! Motion and sound lab (`crow --motion-lab`): a fake folder where every
//! feedback event can be triggered and judged in the three motion modes
//! (Off, Low, Full) and two sound sets (Finder, Crow). A trial for the
//! feedback plan, not a shipping view: nothing here touches the disk.

use std::path::PathBuf;
use std::time::Duration;

use crow_core::motion::{
    self, Clip, Cue, Effective, Event, Fx, Look, MotionMode, SoundSet, Tint, ease_in, ease_in_out, ease_out, ms,
};
use crow_macos::Sound;
use crow_theme::Theme;
use gpui::{
    Animation, AnimationExt, App, Bounds, Context, Div, FocusHandle, Focusable, IntoElement, KeyDownEvent, Render,
    Role, SharedString, SpringAnimation, SpringConfig, TitlebarOptions, Window, WindowBounds, WindowOptions, div, img,
    point, prelude::*, px, size,
};

use crate::feedback;
use crate::icons::{Glyph, glyph};
use crate::style::{active_selection, color, color_alpha, error_soft, icon, ui_font};
use crate::ui::Ui;

const ROW: f32 = 24.;

#[derive(Clone)]
struct Row {
    id: u64,
    name: SharedString,
    dir: bool,
    locked: bool,
    items: usize,
    fx: Fx,
    /// Bumped for every new effect, so the element id (and its animation) restarts.
    generation: u64,
    /// For `MoveOut`: the folder it goes into.
    into: Option<u64>,
}

impl Row {
    fn leaving(&self) -> bool {
        self.fx.leaving()
    }
}

/// A row in flight to a folder (Full mode), drawn over the list.
struct Ghost {
    generation: u64,
    name: SharedString,
    dir: bool,
    from: f32,
    to: f32,
}

struct Undo {
    index: usize,
    row: Row,
    into: Option<u64>,
}

pub fn open(cx: &mut App) {
    let sounds: Vec<Sound> = motion::all_sounds().into_iter().map(feedback::sound).collect();
    crate::effects::effects(cx).preload_sounds(&sounds);
    // The lab chooses motion itself; GPUI's own switch would freeze every animation.
    cx.set_reduce_motion(false);
    let bounds = Bounds::centered(None, size(px(720.), px(600.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some("Crow motion lab".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(14.), px(19.))),
        }),
        ..Default::default()
    };
    match cx.open_window(options, |window, cx| {
        let lab = cx.new(MotionLab::new);
        window.focus(&lab.focus_handle(cx), cx);
        lab
    }) {
        Ok(_) => {}
        Err(e) => tracing::error!("open motion lab: {e}"),
    }
}

pub struct MotionLab {
    focus: FocusHandle,
    mode: Effective,
    sounds: SoundSet,
    slow: bool,
    glide: bool,
    rows: Vec<Row>,
    cursor: u64,
    next_id: u64,
    generation: u64,
    undo: Vec<Undo>,
    ghosts: Vec<Ghost>,
    trash_count: usize,
    trash_generation: u64,
    status: SharedString,
    status_generation: u64,
    tip: Option<u64>,
    /// Whether the mock sheet (Q3) is open.
    sheet: bool,
    /// When the sheet last closed (it fades out for `SHEET_OUT`), and the demo progress bar.
    sheet_closed: Option<std::time::Instant>,
    progress: f32,
    /// Copy progress: its generation and whether it finished.
    copy: Option<(u64, bool)>,
    ui_sounds: bool,
    /// Bumped per sound played, so a clip's fade never cuts a newer sound.
    sound_generation: u64,
}

impl Focusable for MotionLab {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl MotionLab {
    fn new(cx: &mut Context<Self>) -> Self {
        let names: &[(&str, bool, bool)] = &[
            ("Backup", true, false),
            ("Photos", true, false),
            ("Projects", true, false),
            ("archive.zip", false, false),
            ("budget.numbers", false, false),
            ("IMG_4021.heic", false, false),
            ("invoice-2026-11.pdf", false, false),
            ("Locked.pdf", false, true),
            ("notes.md", false, false),
            ("readme.txt", false, false),
            ("talk.key", false, false),
        ];
        let rows: Vec<Row> = names
            .iter()
            .enumerate()
            .map(|(i, (name, dir, locked))| Row {
                id: i as u64,
                name: (*name).into(),
                dir: *dir,
                locked: *locked,
                items: if *dir { 3 + i * 4 } else { 0 },
                fx: Fx::Idle,
                generation: 0,
                into: None,
            })
            .collect();
        let mode = MotionMode::Auto.resolve(crow_macos::reduce_motion());
        MotionLab {
            focus: cx.focus_handle(),
            mode,
            sounds: SoundSet::Finder,
            slow: false,
            glide: true,
            cursor: rows[3].id,
            next_id: rows.len() as u64,
            rows,
            generation: 1,
            undo: Vec::new(),
            ghosts: Vec::new(),
            trash_count: 0,
            trash_generation: 0,
            status: "Try ⌘⌫, m, ⌘Z, ⇧⌘N, c, t, e on the list".into(),
            status_generation: 0,
            tip: None,
            sheet: false,
            sheet_closed: None,
            progress: 0.0,
            copy: None,
            ui_sounds: crow_macos::ui_sounds_enabled(),
            sound_generation: 0,
        }
    }

    fn dur(&self, base: u64) -> Duration {
        self.scaled(Duration::from_millis(base))
    }

    /// Slow motion ×5, for the lab's slow switch.
    fn scaled(&self, base: Duration) -> Duration {
        if self.slow { base * 5 } else { base }
    }

    fn next_generation(&mut self) -> u64 {
        self.generation += 1;
        self.generation
    }

    fn play(&mut self, event: Event, cx: &mut Context<Self>) {
        let effects = crate::effects::effects(cx);
        match motion::cue(self.sounds, event) {
            Some(Cue::Play { sound, volume, clip }) => {
                let sound = feedback::sound(sound);
                effects.play_sound(&sound, volume);
                self.sound_generation += 1;
                if let Some(clip) = clip {
                    self.clip(sound, volume, clip, self.sound_generation, cx);
                }
            }
            Some(Cue::Beep) => effects.beep(),
            None => {}
        }
    }

    /// After `clip.keep`, ramp the volume to zero over `clip.fade` and stop, unless
    /// a newer sound started meanwhile (it owns the volume then).
    fn clip(&self, sound: Sound, volume: f32, clip: Clip, generation: u64, cx: &mut Context<Self>) {
        let effects = crate::effects::effects(cx);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(clip.keep).await;
            const STEPS: u32 = 8;
            for step in 1..=STEPS {
                let current = this.update(cx, |this, _| this.sound_generation == generation).unwrap_or(false);
                if !current {
                    return;
                }
                if step == STEPS {
                    effects.stop_sound(&sound);
                } else {
                    effects.set_sound_volume(&sound, volume * (1.0 - step as f32 / STEPS as f32));
                }
                cx.background_executor().timer(clip.fade / STEPS).await;
            }
        })
        .detach();
    }

    fn say(&mut self, text: impl Into<SharedString>) {
        self.status = text.into();
        self.status_generation += 1;
    }

    fn after(&self, delay: Duration, cx: &mut Context<Self>, f: impl FnOnce(&mut Self, &mut Context<Self>) + 'static) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update(cx, |this, cx| {
                f(this, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn index_of(&self, id: u64) -> Option<usize> {
        self.rows.iter().position(|r| r.id == id)
    }

    /// The y of `index` once every leaving row has closed.
    fn settled_y(&self, index: usize) -> f32 {
        self.rows[..index].iter().filter(|r| !r.leaving()).count() as f32 * ROW
    }

    fn move_cursor(&mut self, delta: i32) {
        let live: Vec<u64> = self.rows.iter().filter(|r| !r.leaving()).map(|r| r.id).collect();
        let Some(at) = live.iter().position(|&id| id == self.cursor) else {
            if let Some(&first) = live.first() {
                self.cursor = first;
            }
            return;
        };
        let to = (at as i32 + delta).clamp(0, live.len() as i32 - 1) as usize;
        self.cursor = live[to];
    }

    /// The cursor leaves a row that is going away: next row below, else above.
    fn step_off(&mut self, id: u64) {
        let Some(i) = self.index_of(id) else { return };
        let below = self.rows[i + 1..].iter().find(|r| !r.leaving()).map(|r| r.id);
        let above = self.rows[..i].iter().rev().find(|r| !r.leaving()).map(|r| r.id);
        if let Some(next) = below.or(above) {
            self.cursor = next;
        }
    }

    /// Start `fx` on row `id`; after `total`, settle it (or remove it when it leaves).
    fn start_fx(&mut self, id: u64, fx: Fx, total: Duration, cx: &mut Context<Self>) {
        let generation = self.next_generation();
        let Some(i) = self.index_of(id) else { return };
        self.rows[i].fx = fx;
        self.rows[i].generation = generation;
        self.after(total, cx, move |this, _| this.finish(id, generation));
    }

    fn finish(&mut self, id: u64, generation: u64) {
        let Some(i) = self.index_of(id) else { return };
        if self.rows[i].generation != generation {
            return;
        }
        if self.rows[i].leaving() {
            let mut row = self.rows.remove(i);
            let into = row.into.take();
            row.fx = Fx::Idle;
            self.undo.push(Undo { index: i, row, into });
        } else {
            self.rows[i].fx = Fx::Idle;
        }
    }

    fn trash(&mut self, cx: &mut Context<Self>) {
        let Some(i) = self.index_of(self.cursor) else { return };
        if self.rows[i].leaving() {
            return;
        }
        if self.rows[i].locked {
            return self.refuse(cx);
        }
        let id = self.cursor;
        let name = self.rows[i].name.clone();
        self.step_off(id);
        self.trash_count += 1;
        self.play(Event::Trash, cx);
        self.say(format!("Moved “{name}” to the Trash · ⌘Z puts it back"));
        let total = match self.mode {
            Effective::Off => Duration::ZERO,
            Effective::Low => self.dur(ms::LOW_OUT),
            Effective::Full => self.dur(ms::TRASH_SWALLOW + ms::TRASH_CLOSE),
        };
        self.start_fx(id, Fx::Trash, total, cx);
        if self.mode == Effective::Off {
            let generation = self.rows[i].generation;
            self.finish(id, generation);
        }
    }

    fn move_into_folder(&mut self, cx: &mut Context<Self>) {
        let Some(i) = self.index_of(self.cursor) else { return };
        if self.rows[i].leaving() {
            return;
        }
        let target = self.rows[i + 1..]
            .iter()
            .chain(self.rows[..i].iter().rev())
            .find(|r| r.dir && !r.leaving() && r.id != self.cursor)
            .map(|r| r.id);
        let Some(target) = target else { return self.refuse(cx) };
        if self.rows[i].locked {
            return self.refuse(cx);
        }
        let id = self.cursor;
        let t = self.index_of(target).unwrap_or(0);
        let (from, to) = (self.settled_y(i), self.settled_y(t));
        let row = &mut self.rows[i];
        row.into = Some(target);
        let (name, dir) = (row.name.clone(), row.dir);
        self.rows[t].items += 1;
        let target_name = self.rows[t].name.clone();
        self.step_off(id);
        self.say(format!("Moved “{name}” into {target_name} · ⌘Z puts it back"));

        match self.mode {
            Effective::Off => {
                self.play(Event::Move, cx);
                self.start_fx(id, Fx::MoveOut, Duration::ZERO, cx);
                let generation = self.generation;
                self.finish(id, generation);
            }
            Effective::Low => {
                self.play(Event::Move, cx);
                self.start_fx(id, Fx::MoveOut, self.dur(ms::LOW_OUT), cx);
                self.start_fx(target, Fx::Receive, self.dur(ms::LOW_TINT), cx);
            }
            Effective::Full => {
                let fly = self.dur(ms::MOVE_FLY);
                self.start_fx(id, Fx::MoveOut, self.dur(ms::MOVE_FLY + ms::MOVE_CLOSE), cx);
                self.start_fx(target, Fx::Receive, self.dur(ms::MOVE_FLY + ms::RECEIVE), cx);
                let generation = self.next_generation();
                self.ghosts.push(Ghost { generation, name, dir, from, to });
                self.after(fly, cx, move |this, cx| {
                    this.ghosts.retain(|g| g.generation != generation);
                    this.play(Event::Move, cx);
                });
            }
        }
    }

    fn arrive(&mut self, index: usize, row: Row, cx: &mut Context<Self>) {
        let id = row.id;
        let index = index.min(self.rows.len());
        self.rows.insert(index, row);
        self.cursor = id;
        let total = match self.mode {
            Effective::Off => Duration::ZERO,
            Effective::Low => self.dur(ms::LOW_TINT),
            Effective::Full => self.dur(ms::ARRIVE_OPEN + ms::ARRIVE_IN + ms::ARRIVE_TINT),
        };
        if self.mode == Effective::Off {
            self.rows[index].fx = Fx::Idle;
        } else {
            self.start_fx(id, Fx::Arrive, total, cx);
        }
    }

    fn new_folder(&mut self, cx: &mut Context<Self>) {
        let taken = |name: &str| self.rows.iter().any(|r| r.name.as_ref() == name);
        let mut name = "untitled folder".to_owned();
        let mut n = 2;
        while taken(&name) {
            name = format!("untitled folder {n}");
            n += 1;
        }
        let index = self.index_of(self.cursor).map_or(0, |i| i + 1);
        let row = Row {
            id: self.next_id,
            name: name.clone().into(),
            dir: true,
            locked: false,
            items: 0,
            fx: Fx::Idle,
            generation: 0,
            into: None,
        };
        self.next_id += 1;
        self.play(Event::NewFolder, cx);
        self.say(format!("New folder “{name}”"));
        self.arrive(index, row, cx);
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        let Some(Undo { index, row, into }) = self.undo.pop() else {
            self.say("Nothing to undo");
            return;
        };
        match into.and_then(|t| self.index_of(t)) {
            Some(t) => self.rows[t].items = self.rows[t].items.saturating_sub(1),
            None => self.trash_count = self.trash_count.saturating_sub(1),
        }
        self.play(Event::Undo, cx);
        self.say(format!("Put back “{}”", row.name));
        self.arrive(index, row, cx);
    }

    fn refuse(&mut self, cx: &mut Context<Self>) {
        let Some(i) = self.index_of(self.cursor) else { return };
        let name = self.rows[i].name.clone();
        self.play(Event::Refuse, cx);
        self.say(format!("“{name}” is locked"));
        let total = match self.mode {
            Effective::Off => return,
            Effective::Low => self.dur(ms::LOW_TINT),
            Effective::Full => self.dur(ms::REFUSE),
        };
        self.start_fx(self.cursor, Fx::Refuse, total, cx);
    }

    fn empty_trash(&mut self, cx: &mut Context<Self>) {
        if self.trash_count == 0 {
            self.say("The Trash is already empty");
            return;
        }
        self.play(Event::EmptyTrash, cx);
        self.say(format!("Emptied the Trash ({} items)", self.trash_count));
        self.trash_count = 0;
        self.trash_generation += 1;
        self.undo.retain(|u| u.into.is_some());
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        if self.copy.is_some_and(|(_, done)| !done) {
            return;
        }
        let generation = self.next_generation();
        self.copy = Some((generation, false));
        self.say("Copying 3 items to Backup…");
        self.after(Duration::from_millis(ms::COPY), cx, move |this, cx| {
            if this.copy.map(|(g, _)| g) != Some(generation) {
                return;
            }
            this.copy = Some((generation, true));
            this.play(Event::CopyDone, cx);
            this.say("Copied 3 items to Backup");
            let fade = match this.mode {
                Effective::Off => Duration::ZERO,
                Effective::Low | Effective::Full => this.dur(ms::COPY_FADE),
            };
            this.after(fade, cx, move |this, _| {
                if this.copy.map(|(g, _)| g) == Some(generation) {
                    this.copy = None;
                }
            });
        });
    }

    fn toggle_tip(&mut self) {
        self.tip = match self.tip {
            Some(_) => None,
            None => Some(self.next_generation()),
        };
    }

    /// Open or close the mock sheet; closing starts its fade-out.
    fn set_sheet(&mut self, open: bool) {
        if self.sheet && !open {
            self.sheet_closed = Some(std::time::Instant::now());
        }
        self.sheet = open;
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let ks = &event.keystroke;
        let m = &ks.modifiers;
        let plain = !m.platform && !m.control && !m.alt && !m.shift;
        match ks.key.as_str() {
            "up" if plain => self.move_cursor(-1),
            "down" if plain => self.move_cursor(1),
            "k" if plain => self.move_cursor(-1),
            "j" if plain => self.move_cursor(1),
            "backspace" if m.platform && m.shift => self.empty_trash(cx),
            "backspace" if m.platform => self.trash(cx),
            "z" if m.platform => self.undo(cx),
            "n" if m.platform && m.shift => self.new_folder(cx),
            "m" if plain => self.move_into_folder(cx),
            "e" if plain => self.refuse(cx),
            "c" if plain => self.copy(cx),
            "t" if plain => self.toggle_tip(),
            "1" if plain => self.mode = Effective::Off,
            "2" if plain => self.mode = Effective::Low,
            "3" if plain => self.mode = Effective::Full,
            "s" if plain => {
                self.sounds = match self.sounds {
                    SoundSet::Off => SoundSet::Finder,
                    SoundSet::Finder => SoundSet::Crow,
                    SoundSet::Crow => SoundSet::Off,
                }
            }
            "x" if plain => self.slow = !self.slow,
            "g" if plain => self.glide = !self.glide,
            "p" if plain => self.set_sheet(!self.sheet),
            "escape" => self.set_sheet(false),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn phases(&self, fx: Fx) -> Vec<Animation> {
        motion::phases(self.mode, fx).into_iter().map(|d| Animation::new(self.scaled(d))).collect()
    }

    fn render_row(&self, row: &Row, theme: &Theme, glide: bool) -> gpui::AnyElement {
        let selected = row.id == self.cursor && !row.leaving();
        let spec = RowSpec {
            name: row.name.clone(),
            dir: row.dir,
            locked: row.locked,
            items: row.items,
            selected: selected && !glide,
            theme: theme.clone(),
        };
        let phases = self.phases(row.fx);
        if phases.is_empty() {
            return spec.render(Look::default()).into_any_element();
        }
        let (mode, fx) = (self.mode, row.fx);
        div()
            .with_animations(("row", row.id * 100_000 + row.generation), phases, move |_, phase, t| {
                spec.render(motion::look(mode, fx, phase, t))
            })
            .into_any_element()
    }
}

#[derive(Clone)]
struct RowSpec {
    name: SharedString,
    dir: bool,
    locked: bool,
    items: usize,
    selected: bool,
    theme: Theme,
}

impl RowSpec {
    fn render(&self, look: Look) -> Div {
        let theme = &self.theme;
        let path = if self.dir { std::env::temp_dir() } else { PathBuf::from("/lab").join(self.name.as_ref()) };
        let file_icon = icon(&path, self.dir, 16).map(|i| img(i).size(px(16.)));
        let tint = match look.tint_kind {
            Tint::Accent => color_alpha(theme.role("accent"), (look.tint * 0x50 as f32) as u8),
            Tint::Error => {
                let mut c = error_soft(theme);
                c.a *= look.tint * 2.0;
                c
            }
        };
        let bg = if self.selected { active_selection(theme) } else { color_alpha(theme.role("bg"), 0) };
        let trash_size = look.trash_size;
        div().h(px(ROW * look.height)).overflow_hidden().flex_none().rounded(px(5.)).bg(bg).child(
            div()
                .size_full()
                .rounded(px(5.))
                .bg(tint)
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(8.))
                .child(
                    // Icon slot: the file icon, with the trash drawn over it.
                    div()
                        .relative()
                        .size(px(16.))
                        .flex_none()
                        .child(div().opacity(1.0 - look.trash).children(file_icon))
                        .child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .opacity(look.trash)
                                .child(glyph(Glyph::Trash, trash_size, "fg-muted", theme)),
                        ),
                )
                .child(
                    div()
                        .relative()
                        .left(px(look.dx))
                        .opacity(look.opacity)
                        .flex()
                        .flex_1()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            div()
                                .flex_1()
                                .text_color(color(theme.role("fg")))
                                .whitespace_nowrap()
                                .child(self.name.clone()),
                        )
                        .when(self.locked, |el| {
                            el.child(
                                div().text_size(px(11.)).text_color(color(theme.role("fg-subtle"))).child("locked"),
                            )
                        })
                        .when(self.dir, |el| {
                            el.child(
                                div()
                                    .flex()
                                    .gap(px(4.))
                                    .items_center()
                                    .text_size(px(11.))
                                    .text_color(color(theme.role("fg-subtle")))
                                    .when(look.badge > 0.0, |el| {
                                        el.child(
                                            div()
                                                .opacity(look.badge)
                                                .text_color(color(theme.role("accent")))
                                                .child("+1"),
                                        )
                                    })
                                    .child(format!("{} items", self.items)),
                            )
                        }),
                ),
        )
    }
}

fn chip(label: &'static str, on: bool, theme: &Theme) -> Div {
    div()
        .px(px(8.))
        .h(px(22.))
        .flex()
        .items_center()
        .rounded(px(5.))
        .text_size(px(12.))
        .when(on, |el| el.bg(color(theme.role("surface-raised"))).text_color(color(theme.role("fg"))))
        .when(!on, |el| el.text_color(color(theme.role("fg-muted"))))
        .child(label)
}

fn group(theme: &Theme) -> Div {
    div().flex().items_center().p(px(2.)).gap(px(2.)).rounded(px(7.)).bg(color(theme.role("surface")))
}

fn caption(text: &'static str, theme: &Theme) -> Div {
    div().text_size(px(11.)).text_color(color(theme.role("fg-subtle"))).child(text)
}

impl Render for MotionLab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        let font = ui_font(&ui.settings.font_family);
        let glide = self.mode == Effective::Full && self.glide;

        let mode_chip = |this: &Self, mode: Effective, label: &'static str, cx: &mut Context<Self>| {
            chip(label, this.mode == mode, &theme)
                .id(label)
                .role(Role::RadioButton)
                .aria_label(SharedString::from(format!("Motion {label}")))
                .aria_toggled(toggled(this.mode == mode))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.mode = mode;
                    cx.notify();
                }))
        };
        let sound_chip = |this: &Self, set: SoundSet, label: &'static str, cx: &mut Context<Self>| {
            chip(label, this.sounds == set, &theme)
                .id(label)
                .role(Role::RadioButton)
                .aria_label(SharedString::from(format!("Sound {label}")))
                .aria_toggled(toggled(this.sounds == set))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.sounds = set;
                    cx.notify();
                }))
        };

        let header = div()
            .h(px(52.))
            .pl(px(88.))
            .pr(px(14.))
            .flex()
            .items_center()
            .gap(px(14.))
            .border_b_1()
            .border_color(color(theme.role("border")))
            .child(caption("Motion 1 2 3", &theme))
            .child(
                group(&theme)
                    .child(mode_chip(self, Effective::Off, "Off", cx))
                    .child(mode_chip(self, Effective::Low, "Low", cx))
                    .child(mode_chip(self, Effective::Full, "Full", cx)),
            )
            .child(caption("Sound s", &theme))
            .child(
                group(&theme)
                    .child(sound_chip(self, SoundSet::Off, "Silent", cx))
                    .child(sound_chip(self, SoundSet::Finder, "Finder", cx))
                    .child(sound_chip(self, SoundSet::Crow, "Crow", cx)),
            )
            .child(
                chip("Slow ×5  x", self.slow, &theme)
                    .id("slow")
                    .role(Role::CheckBox)
                    .aria_label("Slow motion")
                    .aria_toggled(toggled(self.slow))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.slow = !this.slow;
                        cx.notify();
                    })),
            )
            .child(
                chip("Glide  g", self.glide, &theme)
                    .id("glide")
                    .role(Role::CheckBox)
                    .aria_label("Cursor glide (Full only)")
                    .aria_toggled(toggled(self.glide))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.glide = !this.glide;
                        cx.notify();
                    })),
            );

        let event_button =
            |label: &'static str, keys: &'static str, f: fn(&mut Self, &mut Context<Self>), cx: &mut Context<Self>| {
                div()
                    .id(label)
                    .role(Role::Button)
                    .aria_label(label)
                    .px(px(8.))
                    .h(px(26.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(color(theme.role("border")))
                    .text_size(px(12.))
                    .text_color(color(theme.role("fg")))
                    .hover(|s| s.bg(color(theme.role("surface"))))
                    .child(label)
                    .child(div().text_color(color(theme.role("fg-subtle"))).child(keys))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        f(this, cx);
                        cx.notify();
                    }))
            };
        let events = div()
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .px(px(14.))
            .py(px(10.))
            .child(event_button("Trash", "⌘⌫", |this, cx| this.trash(cx), cx))
            .child(event_button("Move into folder", "m", |this, cx| this.move_into_folder(cx), cx))
            .child(event_button("Undo", "⌘Z", |this, cx| this.undo(cx), cx))
            .child(event_button("New folder", "⇧⌘N", |this, cx| this.new_folder(cx), cx))
            .child(event_button("Refuse", "e", |this, cx| this.refuse(cx), cx))
            .child(event_button("Copy", "c", |this, cx| this.copy(cx), cx))
            .child(event_button("Tip", "t", |this, _| this.toggle_tip(), cx))
            .child(event_button("Empty Trash", "⇧⌘⌫", |this, cx| this.empty_trash(cx), cx))
            .child(event_button("Sheet", "p", |this, _| this.set_sheet(!this.sheet), cx))
            .child(event_button(
                "Progress +25%",
                "",
                |this, _| this.progress = if this.progress >= 1.0 { 0.0 } else { this.progress + 0.25 },
                cx,
            ));

        // The list: a glide highlight under the rows (Full), the rows, and flying ghosts on top.
        let cursor_y = self.index_of(self.cursor).map_or(0.0, |i| self.settled_y(i));
        let stiffness: f32 = if self.slow { 900.0 / 25.0 } else { 900.0 };
        // Critically damped: no overshoot.
        let damping = 2.0 * stiffness.sqrt();
        let mut list = div().relative().flex().flex_col().px(px(8.)).py(px(4.));
        if glide {
            list = list.child(
                div()
                    .absolute()
                    .left(px(8.))
                    .right(px(8.))
                    .h(px(ROW))
                    .rounded(px(5.))
                    .bg(active_selection(&theme))
                    .with_spring(
                        "glide",
                        SpringAnimation::new(SpringConfig::new(stiffness, damping, 1.0)).to(px(cursor_y + 4.0)),
                        |el, y| el.top(y),
                    ),
            );
        }
        for row in &self.rows {
            list = list.child(self.render_row(row, &theme, glide));
        }
        for ghost in &self.ghosts {
            let spec = RowSpec {
                name: ghost.name.clone(),
                dir: ghost.dir,
                locked: false,
                items: 0,
                selected: false,
                theme: theme.clone(),
            };
            let (from, to) = (ghost.from, ghost.to);
            let surface = color(theme.role("surface-raised"));
            list = list.child(div().absolute().left(px(8.)).right(px(8.)).with_animation(
                ("ghost", ghost.generation),
                Animation::new(self.dur(ms::MOVE_FLY)),
                move |el, t| {
                    let e = ease_in_out(t);
                    let y = from + (to - from) * e + 4.0;
                    // Shrinks a little and fades as it lands.
                    let inset = 40.0 * e;
                    el.top(px(y)).child(
                        div()
                            .ml(px(inset))
                            .mr(px(inset * 3.0))
                            .rounded(px(5.))
                            .bg(surface)
                            .shadow_md()
                            .opacity(1.0 - 0.6 * e)
                            .child(spec.render(Look::default())),
                    )
                },
            ));
        }

        let tip = self.tip.map(|generation| {
            let strip = div()
                .mx(px(14.))
                .mb(px(8.))
                .h(px(32.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(7.))
                .bg(color(theme.role("surface")))
                .border_1()
                .border_color(color(theme.role("border")))
                .text_size(px(12.))
                .text_color(color(theme.role("fg")))
                .child(div().text_color(color(theme.role("accent"))).child("⌘Z"))
                .child(div().flex_1().child("puts back what you just trashed"))
                .child(div().text_color(color(theme.role("fg-muted"))).child("Got it"));
            match self.mode {
                Effective::Off => strip.into_any_element(),
                Effective::Low => strip
                    .with_animation(("tip", generation), Animation::new(self.dur(ms::LOW_IN)), |el, t| {
                        el.opacity(ease_out(t))
                    })
                    .into_any_element(),
                Effective::Full => strip
                    .with_animation(("tip", generation), Animation::new(self.dur(ms::TIP_FULL)), |el, t| {
                        let e = ease_out(t);
                        el.opacity(e).relative().top(px(8.0 * (1.0 - e)))
                    })
                    .into_any_element(),
            }
        });

        // Bottom bar: progress on the top edge, status, Trash count.
        let progress = self.copy.map(|(generation, done)| {
            let bar = div().absolute().top_0().left_0().h(px(2.)).bg(color(theme.role("accent")));
            if done {
                let bar = bar.w_full();
                match self.mode {
                    Effective::Off => bar.into_any_element(),
                    _ => bar
                        .with_animation(("copy-done", generation), Animation::new(self.dur(ms::COPY_FADE)), |el, t| {
                            el.opacity(1.0 - ease_in(t))
                        })
                        .into_any_element(),
                }
            } else {
                // Progress is information, not decoration: it moves in every mode
                // (Off steps it four times a second instead of every frame).
                let mut animation = Animation::new(Duration::from_millis(ms::COPY));
                if self.mode == Effective::Off {
                    animation = animation.with_max_fps(4.0);
                }
                bar.with_animation(("copy", generation), animation, |el, t| el.w(gpui::relative(t))).into_any_element()
            }
        });
        let status_text = div().child(self.status.clone());
        let status: gpui::AnyElement = match self.mode {
            Effective::Off => status_text.into_any_element(),
            _ => status_text
                .with_animation(("status", self.status_generation), Animation::new(self.dur(ms::STATUS)), |el, t| {
                    el.opacity(0.3 + 0.7 * ease_out(t))
                })
                .into_any_element(),
        };
        let trash_label =
            if self.trash_count == 0 { "Trash empty".to_owned() } else { format!("Trash · {}", self.trash_count) };
        let bottom = div()
            .relative()
            .h(px(30.))
            .px(px(14.))
            .flex()
            .items_center()
            .gap(px(10.))
            .border_t_1()
            .border_color(color(theme.role("border")))
            .text_size(px(12.))
            .text_color(color(theme.role("fg-muted")))
            .children(progress)
            .child(div().flex_1().child(status))
            .when(!self.ui_sounds, |el| {
                el.child(
                    div()
                        .text_color(color(theme.role("warning")))
                        .child("macOS UI sounds are off; the lab plays anyway"),
                )
            })
            .child(div().pl(px(10.)).border_l_1().border_color(color(theme.role("border"))).child(trash_label));

        let leaving = self
            .sheet_closed
            .filter(|_| !self.sheet)
            .and_then(|closed| {
                crow_core::motion::progress(self.mode, closed, std::time::Instant::now(), ms::SHEET_OUT, ms::SHEET_OUT)
            })
            .map(|t| 1.0 - t);
        if leaving.is_some() {
            window.request_animation_frame();
        }
        let sheet = (self.sheet || leaving.is_some()).then(|| {
            let panel = crate::chrome::sheet_panel("lab-sheet", crate::chrome::SheetWidth::Narrow, &theme)
                .p_4()
                .flex()
                .flex_col()
                .gap_2()
                .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child("A sheet"))
                .child("Esc or a click outside closes it. This is the Q3 entrance.");
            crate::chrome::sheet(
                panel,
                &theme,
                self.mode,
                leaving,
                cx.listener(|this, _, _, cx| {
                    this.set_sheet(false);
                    cx.notify();
                }),
            )
        });

        div()
            .id("motion-lab")
            .role(Role::Application)
            .aria_label("Crow motion lab")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(color(theme.role("bg")))
            .text_color(color(theme.role("fg")))
            .font_family(font)
            .text_size(px(13.))
            .child(header)
            .child(events)
            .child({
                let bar = div().h(px(2.)).bg(color(theme.role("accent")));
                let bar = if self.mode == Effective::Full {
                    bar.with_spring("lab-progress", crate::motion_ui::ease_progress(self.progress), |el, fraction| {
                        el.w(gpui::relative(fraction))
                    })
                    .into_any_element()
                } else {
                    bar.w(gpui::relative(self.progress)).into_any_element()
                };
                div().w_full().h(px(2.)).child(bar)
            })
            .child(
                div().id("lab-list").role(Role::List).aria_label("Lab folder").flex_1().overflow_y_scroll().child(list),
            )
            .children(tip)
            .child(bottom)
            .children(sheet)
    }
}

fn toggled(on: bool) -> gpui::Toggled {
    if on { gpui::Toggled::True } else { gpui::Toggled::False }
}
