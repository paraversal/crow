//! Motion and sound feedback for the app: maps `crow_core::motion` cues to the
//! system sounds `crow-macos` plays, and plays them on the main thread.

use std::path::PathBuf;

use crow_core::motion::{self, SoundId};
use crow_macos::Sound;

/// The `NSSound` for a cue's `SoundId`.
pub fn sound(id: SoundId) -> Sound {
    match id {
        SoundId::Named(name) => Sound::Named(name),
        SoundId::System(relative) => Sound::File(PathBuf::from(motion::SYSTEM_SOUNDS_DIR).join(relative)),
    }
}
