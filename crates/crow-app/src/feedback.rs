//! Motion and sound feedback for the app: the effective mode, and playing the
//! cues `crow_core::motion` chooses through the system sounds `crow-macos` loads.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use crow_core::Settings;
use crow_core::motion::{self, Clip, Cue, Effective, Event, SoundId, SoundSet};
use crow_fs::ops::Request;
use crow_macos::Sound;
use gpui::App;

use crate::ui::Ui;

/// The mode and sound set in effect, recomputed when settings change and on the
/// 100 ms poll (Reduce Motion and the UI-sounds switch can change under us).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Feedback {
    pub mode: Effective,
    pub sound: SoundSet,
    /// macOS "Play user interface sound effects"; false silences everything but the alert.
    pub ui_sounds: bool,
}

impl Default for Feedback {
    fn default() -> Self {
        Feedback { mode: Effective::Full, sound: SoundSet::Crow, ui_sounds: true }
    }
}

impl Feedback {
    pub fn read(settings: &Settings) -> Self {
        Feedback {
            mode: settings.motion.resolve(crow_macos::reduce_motion()),
            sound: settings.sound,
            ui_sounds: crow_macos::ui_sounds_enabled(),
        }
    }
}

/// The `NSSound` for a cue's `SoundId`.
pub fn sound(id: SoundId) -> Sound {
    match id {
        SoundId::Named(name) => Sound::Named(name),
        SoundId::System(relative) => Sound::File(PathBuf::from(motion::SYSTEM_SOUNDS_DIR).join(relative)),
    }
}

/// Load `set`'s sounds now, so the first play does no disk I/O. Main thread only;
/// a no-op elsewhere (tests, a background task by mistake).
pub fn preload(set: SoundSet) {
    let sounds: Vec<Sound> = motion::sounds(set).into_iter().map(sound).collect();
    crow_macos::preload(&sounds);
}

/// The sound an operation's success plays, if any.
pub fn event_for(request: &Request) -> Option<Event> {
    match request {
        Request::Trash { .. } => Some(Event::Trash),
        Request::Move { .. } => Some(Event::Move),
        Request::NewFolder { .. } | Request::NewFile { .. } => Some(Event::NewFolder),
        Request::Copy { .. } | Request::Duplicate { .. } => Some(Event::CopyDone),
        _ => None,
    }
}

/// What a refused key should shake or tint, besides the beep.
pub enum RefuseTarget<'a> {
    /// No field or row: just the alert.
    None,
    /// A text field: Full shakes it, Low tints its ring red.
    Field(&'a gpui::Entity<crate::text_input::TextInput>),
}

/// Refuse a key or command: exactly one alert beep, then the visual on `target`.
pub fn refuse(target: RefuseTarget, _window: &mut gpui::Window, cx: &mut App) {
    play(Event::Refuse, cx);
    if let RefuseTarget::Field(input) = target {
        input.update(cx, |input, cx| input.refuse_visual(cx));
    }
}

/// Bumped per sound played, so an old clip's fade never cuts a newer sound.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Play `event` in the current sound set, on the main thread.
pub fn play(event: Event, cx: &mut App) {
    let feedback = cx.global::<Ui>().feedback;
    let cue = motion::cue(feedback.sound, event);
    // macOS's UI-sounds switch silences everything but the alert, which macOS
    // itself decides about (and still plays).
    if !feedback.ui_sounds {
        if matches!(cue, Some(Cue::Beep)) {
            crow_macos::beep();
        }
        return;
    }
    match cue {
        Some(Cue::Play { sound: id, volume, clip }) => {
            let sound = sound(id);
            crow_macos::play(&sound, volume);
            let generation = GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
            if let Some(clip) = clip {
                clip_after(sound, volume, clip, generation, cx);
            }
        }
        Some(Cue::Beep) => crow_macos::beep(),
        None => {}
    }
}

/// After `clip.keep`, ramp the volume to zero over `clip.fade` and stop, unless a
/// newer sound started meanwhile (it owns the volume then).
fn clip_after(sound: Sound, volume: f32, clip: Clip, generation: u64, cx: &mut App) {
    cx.spawn(async move |cx| {
        cx.background_executor().timer(clip.keep).await;
        const STEPS: u32 = 8;
        for step in 1..=STEPS {
            if GENERATION.load(Ordering::Relaxed) != generation {
                return;
            }
            if step == STEPS {
                crow_macos::stop(&sound);
            } else {
                crow_macos::set_volume(&sound, volume * (1.0 - step as f32 / STEPS as f32));
            }
            cx.background_executor().timer(clip.fade / STEPS).await;
        }
    })
    .detach();
}
