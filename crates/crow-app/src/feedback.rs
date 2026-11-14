//! Motion and sound feedback for the app: the effective mode, and playing the
//! cues `crow_core::motion` chooses through the system sounds `crow-macos` loads.

use std::path::PathBuf;

use crow_core::Settings;
use crow_core::motion::{self, Effective, SoundId, SoundSet};
use crow_macos::Sound;

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
