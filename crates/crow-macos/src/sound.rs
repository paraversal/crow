//! Short feedback sounds through `NSSound`.
//!
//! Sounds are loaded once (`preload`, at startup) and kept per thread, so a
//! later `play` does no disk I/O. Playing a sound that is still playing
//! restarts it rather than stacking a second copy.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use objc2::AllocAnyThread;
use objc2::rc::Retained;
use objc2_app_kit::{NSBeep, NSSound};
use objc2_foundation::{NSString, NSUserDefaults};

use crate::main_thread;

/// Where a sound comes from.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Sound {
    /// A named system sound from `/System/Library/Sounds` (e.g. "Tink", "Pop").
    Named(&'static str),
    /// An audio file on disk (e.g. Finder's own sounds inside CoreAudio.component).
    File(PathBuf),
}

thread_local! {
    static LOADED: RefCell<HashMap<Sound, Option<Retained<NSSound>>>> = RefCell::new(HashMap::new());
}

fn load(sound: &Sound) -> Option<Retained<NSSound>> {
    match sound {
        Sound::Named(name) => NSSound::soundNamed(&NSString::from_str(name)),
        Sound::File(path) => {
            NSSound::initWithContentsOfFile_byReference(NSSound::alloc(), &crate::ns_string(path), false)
        }
    }
}

/// Load `sounds` now so `play` never touches the disk. Call at startup.
pub fn preload(sounds: &[Sound]) {
    if main_thread().is_none() {
        return;
    }
    LOADED.with(|loaded| {
        let mut loaded = loaded.borrow_mut();
        for sound in sounds {
            if !loaded.contains_key(sound) {
                let ns = load(sound);
                loaded.insert(sound.clone(), ns);
            }
        }
    });
}

/// Play `sound` at `volume` (0–1). Loads it on first use if it was not preloaded.
pub fn play(sound: &Sound, volume: f32) {
    if main_thread().is_none() {
        return;
    }
    LOADED.with(|loaded| {
        let mut loaded = loaded.borrow_mut();
        let ns = loaded.entry(sound.clone()).or_insert_with(|| load(sound));
        if let Some(ns) = ns {
            if ns.isPlaying() {
                ns.stop();
            }
            ns.setVolume(volume.clamp(0.0, 1.0));
            ns.play();
        }
    });
}

/// The user's alert sound (System Settings ▸ Sound ▸ Alert sound).
pub fn beep() {
    if main_thread().is_some() {
        NSBeep();
    }
}

/// System Settings ▸ Sound ▸ "Play user interface sound effects". On when unset.
pub fn ui_sounds_enabled() -> bool {
    let defaults = NSUserDefaults::standardUserDefaults();
    let key = NSString::from_str("com.apple.sound.uiaudio.enabled");
    match defaults.objectForKey(&key) {
        Some(_) => defaults.boolForKey(&key),
        None => true,
    }
}

/// Change the volume of `sound` while it plays (for fades).
pub fn set_volume(sound: &Sound, volume: f32) {
    with_loaded(sound, |ns| ns.setVolume(volume.clamp(0.0, 1.0)));
}

/// Stop `sound` if it is playing.
pub fn stop(sound: &Sound) {
    with_loaded(sound, |ns| {
        ns.stop();
    });
}

fn with_loaded(sound: &Sound, f: impl FnOnce(&NSSound)) {
    if main_thread().is_none() {
        return;
    }
    LOADED.with(|loaded| {
        if let Some(Some(ns)) = loaded.borrow().get(sound) {
            f(ns);
        }
    });
}
