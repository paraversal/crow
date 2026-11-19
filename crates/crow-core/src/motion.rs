//! Motion and sound feedback: the timing table, the look of each effect at one
//! moment, and which sound an event plays. UI-free, so the app and the motion
//! lab (`crow --motion-lab`) share one source of truth.

use std::time::{Duration, Instant};

use serde::Deserialize;

/// The `motion` setting. `Auto` follows macOS Reduce Motion.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MotionMode {
    #[default]
    Auto,
    Off,
    Low,
    Full,
}

/// What `Auto` resolves to right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effective {
    Off,
    Low,
    Full,
}

impl MotionMode {
    /// The value in `settings.toml`.
    pub fn key(self) -> &'static str {
        match self {
            MotionMode::Auto => "auto",
            MotionMode::Off => "off",
            MotionMode::Low => "low",
            MotionMode::Full => "full",
        }
    }

    pub fn resolve(self, reduce_motion: bool) -> Effective {
        match self {
            MotionMode::Auto if reduce_motion => Effective::Low,
            MotionMode::Auto => Effective::Full,
            MotionMode::Off => Effective::Off,
            MotionMode::Low => Effective::Low,
            MotionMode::Full => Effective::Full,
        }
    }
}

impl Effective {
    pub fn is_off(self) -> bool {
        self == Effective::Off
    }
}

/// The `sound` setting. A separate setting from motion (user, 2026-11-14).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SoundSet {
    Off,
    /// Only the sounds Finder itself makes: trash, empty trash, the alert.
    Finder,
    /// Finder's sounds plus soft system ticks for move, new folder, undo, copy.
    #[default]
    Crow,
}

impl SoundSet {
    /// The value in `settings.toml`.
    pub fn key(self) -> &'static str {
        match self {
            SoundSet::Off => "off",
            SoundSet::Finder => "finder",
            SoundSet::Crow => "crow",
        }
    }
}

/// What happened, for the sound table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Trash,
    EmptyTrash,
    Move,
    NewFolder,
    Undo,
    Refuse,
    CopyDone,
}

/// Where a sound comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SoundId {
    /// A named system sound from `/System/Library/Sounds` (`NSSound soundNamed:`).
    Named(&'static str),
    /// A file under [`SYSTEM_SOUNDS_DIR`], e.g. `finder/move to trash.aif`.
    System(&'static str),
}

/// Finder's own sounds live here (read at run time; Crow ships no audio files).
pub const SYSTEM_SOUNDS_DIR: &str =
    "/System/Library/Components/CoreAudio.component/Contents/SharedSupport/SystemSounds";

/// A sound, optionally cut after `keep` with a short fade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clip {
    pub keep: Duration,
    pub fade: Duration,
}

/// Finder's trash sound is 2.2 s; keep the toss, fade out the paper tail (user, 2026-11-14).
pub const TRASH_CLIP: Clip = Clip { keep: Duration::from_millis(450), fade: Duration::from_millis(150) };

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cue {
    Play { sound: SoundId, volume: f32, clip: Option<Clip> },
    Beep,
}

/// The sound `event` plays in `set`, or `None` for silence. The alert beep is
/// macOS's own sound and volume.
pub fn cue(set: SoundSet, event: Event) -> Option<Cue> {
    match (set, event) {
        (SoundSet::Off, _) => None,
        (_, Event::Trash) => {
            Some(Cue::Play { sound: SoundId::System("finder/move to trash.aif"), volume: 0.6, clip: Some(TRASH_CLIP) })
        }
        (_, Event::EmptyTrash) => {
            Some(Cue::Play { sound: SoundId::System("finder/empty trash.aif"), volume: 0.6, clip: None })
        }
        (_, Event::Refuse) => Some(Cue::Beep),
        (SoundSet::Finder, _) => None,
        (SoundSet::Crow, Event::Move) => Some(Cue::Play { sound: SoundId::Named("Purr"), volume: 0.35, clip: None }),
        (SoundSet::Crow, Event::NewFolder) => Some(Cue::Play { sound: SoundId::Named("Pop"), volume: 0.3, clip: None }),
        (SoundSet::Crow, Event::Undo) => Some(Cue::Play { sound: SoundId::Named("Bottle"), volume: 0.3, clip: None }),
        (SoundSet::Crow, Event::CopyDone) => {
            Some(Cue::Play { sound: SoundId::Named("Glass"), volume: 0.35, clip: None })
        }
    }
}

/// Every sound `set` can play, for preloading.
pub fn sounds(set: SoundSet) -> Vec<SoundId> {
    let mut out = Vec::new();
    for event in
        [Event::Trash, Event::EmptyTrash, Event::Refuse, Event::Move, Event::NewFolder, Event::Undo, Event::CopyDone]
    {
        if let Some(Cue::Play { sound, .. }) = cue(set, event) {
            out.push(sound);
        }
    }
    out
}

/// Every sound any set can play, for the lab's set switch.
pub fn all_sounds() -> Vec<SoundId> {
    let mut out = Vec::new();
    for set in [SoundSet::Finder, SoundSet::Crow] {
        for sound in sounds(set) {
            if !out.contains(&sound) {
                out.push(sound);
            }
        }
    }
    out
}

/// What is happening to a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fx {
    Idle,
    Trash,
    MoveOut,
    Receive,
    Arrive,
    Refuse,
    Reveal,
}

impl Fx {
    /// Whether the effect ends with the row gone.
    pub fn leaving(self) -> bool {
        matches!(self, Fx::Trash | Fx::MoveOut)
    }
}

/// How a row looks at one moment of an effect. The resting look is the default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub height: f32,
    pub opacity: f32,
    pub dx: f32,
    pub trash: f32,
    pub trash_size: f32,
    pub tint: f32,
    pub tint_kind: Tint,
    pub badge: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tint {
    #[default]
    Accent,
    Error,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            height: 1.0,
            opacity: 1.0,
            dx: 0.0,
            trash: 0.0,
            trash_size: 16.0,
            tint: 0.0,
            tint_kind: Tint::Accent,
            badge: 0.0,
        }
    }
}

/// Base durations in ms. The lab multiplies them by five in slow motion.
pub mod ms {
    pub const TRASH_SWALLOW: u64 = 180;
    pub const TRASH_CLOSE: u64 = 140;
    pub const MOVE_FLY: u64 = 260;
    pub const MOVE_CLOSE: u64 = 140;
    pub const RECEIVE: u64 = 360;
    pub const ARRIVE_OPEN: u64 = 120;
    pub const ARRIVE_IN: u64 = 180;
    pub const ARRIVE_TINT: u64 = 420;
    pub const REFUSE: u64 = 320;
    pub const TIP_FULL: u64 = 220;
    pub const LOW_OUT: u64 = 140;
    pub const LOW_IN: u64 = 160;
    pub const LOW_TINT: u64 = 420;
    pub const STATUS: u64 = 140;
    pub const FIELD: u64 = 160;
    pub const COPY: u64 = 1600;
    pub const COPY_FADE: u64 = 320;
    pub const DISCLOSE: u64 = 180;
    pub const PANEL_TAB: u64 = 155;
    pub const VIEW_CHANGE: u64 = 245;
    pub const CHILD_COLUMN: u64 = 145;
    pub const NAVIGATION_LOW: u64 = 120;
    pub const REVEAL: u64 = 300;
}

pub fn ease_in(t: f32) -> f32 {
    t * t * t
}

pub fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

pub fn ease_in_out(t: f32) -> f32 {
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}

/// Eased progress of an in-flight transition. Low uses the shorter fade time.
pub fn navigation_progress(mode: Effective, started: Instant, now: Instant, full_ms: u64) -> Option<f32> {
    let ms = match mode {
        Effective::Off => return None,
        Effective::Low => ms::NAVIGATION_LOW,
        Effective::Full => full_ms,
    };
    let elapsed = now.saturating_duration_since(started);
    let duration = Duration::from_millis(ms);
    (elapsed < duration).then(|| ease_out(elapsed.as_secs_f32() / duration.as_secs_f32()))
}

/// How row effect `fx` looks in phase `phase` at progress `t` (already eased per phase).
pub fn look(mode: Effective, fx: Fx, phase: usize, t: f32) -> Look {
    let mut look = Look::default();
    match (mode, fx, phase) {
        (_, Fx::Idle, _) | (Effective::Off, _, _) => {}
        // Full trash: the icon turns into a trash, the name slides into it, the trash gulps, the gap closes.
        (Effective::Full, Fx::Trash, 0) => {
            let e = ease_in(t);
            look.trash = (t * 2.5).min(1.0);
            look.dx = -22.0 * e;
            look.opacity = 1.0 - e;
        }
        (Effective::Full, Fx::Trash, _) => {
            let e = ease_out(t);
            look.trash = 1.0 - e;
            look.trash_size = 16.0 + 3.0 * (t * std::f32::consts::PI).sin();
            look.opacity = 0.0;
            look.height = 1.0 - e;
        }
        // Full move: the ghost flies; the row stays as an empty slot, then closes.
        (Effective::Full, Fx::MoveOut, 0) => look.opacity = 0.0,
        (Effective::Full, Fx::MoveOut, _) => {
            look.opacity = 0.0;
            look.height = 1.0 - ease_out(t);
        }
        // Full receive: wait for the ghost, then a short accent pulse and a +1 badge.
        (Effective::Full, Fx::Receive, 0) => {}
        (Effective::Full, Fx::Receive, _) => {
            look.tint = 1.0 - ease_in(t);
            look.badge = 1.0 - ease_in(t);
        }
        // Full arrive: the gap opens, the row slides in, a tint fades.
        (Effective::Full, Fx::Arrive, 0) => {
            look.height = ease_out(t);
            look.opacity = 0.0;
            look.tint = 1.0;
        }
        (Effective::Full, Fx::Arrive, 1) => {
            let e = ease_out(t);
            look.opacity = e;
            look.dx = -12.0 * (1.0 - e);
            look.tint = 1.0;
        }
        (Effective::Full, Fx::Arrive, _) => look.tint = 1.0 - ease_in_out(t),
        // Full refuse: a damped shake with an error tint.
        (Effective::Full, Fx::Refuse, _) => {
            look.dx = 5.0 * (t * 3.0 * std::f32::consts::TAU).sin() * (1.0 - t);
            look.tint = 1.0 - t;
            look.tint_kind = Tint::Error;
        }
        // Low: opacity and tint only; nothing moves, gaps snap.
        (Effective::Low, Fx::Trash | Fx::MoveOut, _) => look.opacity = 1.0 - ease_out(t),
        (Effective::Low, Fx::Receive, _) => {
            look.tint = 1.0 - ease_in(t);
            look.badge = 1.0 - ease_in(t);
        }
        (Effective::Low, Fx::Arrive, _) => {
            look.opacity = (t * ms::LOW_TINT as f32 / ms::LOW_IN as f32).min(1.0);
            look.tint = 1.0 - ease_in(t);
        }
        (Effective::Low, Fx::Refuse, _) => {
            look.tint = 1.0 - ease_in(t);
            look.tint_kind = Tint::Error;
        }
        (Effective::Full | Effective::Low, Fx::Reveal, _) => look.tint = 1.0 - ease_out(t),
    }
    look
}

/// How `fx` looks `now - started` into `phases` (the effect's durations).
pub fn look_in(phases: &[Duration], mode: Effective, fx: Fx, started: Instant, now: Instant) -> Look {
    if phases.is_empty() {
        return Look::default();
    }
    let mut remaining = now.saturating_duration_since(started);
    for (phase, duration) in phases.iter().enumerate() {
        if remaining < *duration || phase == phases.len() - 1 {
            let t = if duration.is_zero() { 1.0 } else { (remaining.as_secs_f32() / duration.as_secs_f32()).min(1.0) };
            return look(mode, fx, phase, t);
        }
        remaining -= *duration;
    }
    look(mode, fx, phases.len() - 1, 1.0)
}

/// The phases of `fx` in `mode`, in order. `off` and `Idle` have none.
pub fn phases(mode: Effective, fx: Fx) -> Vec<Duration> {
    let d = |base: u64| Duration::from_millis(base);
    match (mode, fx) {
        (Effective::Off, _) | (_, Fx::Idle) => vec![],
        (Effective::Full, Fx::Trash) => vec![d(ms::TRASH_SWALLOW), d(ms::TRASH_CLOSE)],
        (Effective::Full, Fx::MoveOut) => vec![d(ms::MOVE_FLY), d(ms::MOVE_CLOSE)],
        (Effective::Full, Fx::Receive) => vec![d(ms::MOVE_FLY), d(ms::RECEIVE)],
        (Effective::Full, Fx::Arrive) => vec![d(ms::ARRIVE_OPEN), d(ms::ARRIVE_IN), d(ms::ARRIVE_TINT)],
        (Effective::Full, Fx::Refuse) => vec![d(ms::REFUSE)],
        (Effective::Full | Effective::Low, Fx::Reveal) => vec![d(ms::REVEAL)],
        (Effective::Low, Fx::Trash | Fx::MoveOut) => vec![d(ms::LOW_OUT)],
        (Effective::Low, Fx::Receive | Fx::Arrive | Fx::Refuse) => vec![d(ms::LOW_TINT)],
    }
}

/// A copy's tint pulse: 1 at the start, fading to 0 over `COPY_FADE`. Off has
/// none; Low keeps the tint (it is color, not movement).
pub fn copy_flash(mode: Effective, started: Instant, now: Instant) -> f32 {
    if mode == Effective::Off {
        return 0.0;
    }
    let duration = Duration::from_millis(ms::COPY_FADE);
    let elapsed = now.saturating_duration_since(started);
    if elapsed >= duration {
        return 0.0;
    }
    1.0 - ease_in(elapsed.as_secs_f32() / duration.as_secs_f32())
}

// ----- disclosure: a folder opening or closing inline (list view) -----

/// At most this many rows slide when a folder opens or closes; the rest are off
/// screen anyway, and a 500-row slide would only blur past.
pub const DISCLOSE_MAX_ROWS: usize = 30;

/// How long a folder's rows take to open or close: Full slides, Low fades the
/// new rows in, Off changes in one frame.
pub fn disclose_duration(mode: Effective) -> Duration {
    match mode {
        Effective::Off => Duration::ZERO,
        Effective::Low => Duration::from_millis(ms::LOW_IN),
        Effective::Full => Duration::from_millis(ms::DISCLOSE),
    }
}

/// Full: how far, in rows, the rows below a disclosure sit from their place at
/// progress `t`. Opening: they start `rows` up (where the children now are) and
/// slide down; closing: they start `rows` down (where they were) and slide up.
pub fn disclose_shift(opening: bool, rows: usize, t: f32) -> f32 {
    let distance = rows.min(DISCLOSE_MAX_ROWS) as f32 * (1.0 - ease_out(t.clamp(0.0, 1.0)));
    if opening { -distance } else { distance }
}

/// The opacity of an opening folder's `k`th child row (1 = right under the
/// folder) at progress `t`. Full: each row shows as the gap opens past it, so
/// the rows below never slide over a visible child. Low: all fade in together.
pub fn disclose_opacity(mode: Effective, k: usize, rows: usize, t: f32) -> f32 {
    let e = ease_out(t.clamp(0.0, 1.0));
    let sliding = rows.min(DISCLOSE_MAX_ROWS);
    match mode {
        Effective::Off => 1.0,
        Effective::Full if k <= sliding => (sliding as f32 * e - (k as f32 - 1.0)).clamp(0.0, 1.0),
        _ => e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_flash_fades_and_off_has_none() {
        let start = Instant::now();
        assert_eq!(copy_flash(Effective::Off, start, start), 0.0);
        assert_eq!(copy_flash(Effective::Full, start, start), 1.0);
        let mid = start + Duration::from_millis(ms::COPY_FADE / 2);
        let value = copy_flash(Effective::Full, start, mid);
        assert!(value > 0.0 && value < 1.0, "mid-flash: {value}");
        assert_eq!(copy_flash(Effective::Full, start, start + Duration::from_millis(ms::COPY_FADE)), 0.0);
        assert_eq!(copy_flash(Effective::Low, start, start), 1.0, "Low keeps the tint");
    }

    #[test]
    fn disclosure_slides_in_full_and_fades_in_low() {
        assert_eq!(disclose_duration(Effective::Off), Duration::ZERO);
        assert_eq!(disclose_duration(Effective::Low), Duration::from_millis(ms::LOW_IN));
        assert_eq!(disclose_duration(Effective::Full), Duration::from_millis(ms::DISCLOSE));

        assert_eq!(disclose_shift(true, 3, 0.0), -3.0, "opening: rows below start where the children land");
        assert_eq!(disclose_shift(true, 3, 1.0), 0.0);
        assert_eq!(disclose_shift(false, 3, 0.0), 3.0, "closing: rows below start where they were");
        assert_eq!(disclose_shift(false, 3, 1.0), 0.0);
        assert_eq!(disclose_shift(true, 500, 0.0), -(DISCLOSE_MAX_ROWS as f32), "a huge folder slides at most 30 rows");

        assert_eq!(disclose_opacity(Effective::Full, 1, 3, 0.0), 0.0, "nothing shows before the gap opens");
        assert_eq!(disclose_opacity(Effective::Full, 3, 3, 1.0), 1.0);
        let early = disclose_opacity(Effective::Full, 1, 3, 0.2);
        let late = disclose_opacity(Effective::Full, 3, 3, 0.2);
        assert!(early > late, "the row under the folder shows first: {early} vs {late}");
        assert_eq!(disclose_opacity(Effective::Low, 3, 3, 0.5), ease_out(0.5), "Low fades every row together");
        assert_eq!(disclose_opacity(Effective::Off, 1, 3, 0.0), 1.0);
    }

    const MODES: [Effective; 3] = [Effective::Off, Effective::Low, Effective::Full];
    const FX: [Fx; 7] = [Fx::Idle, Fx::Trash, Fx::MoveOut, Fx::Receive, Fx::Arrive, Fx::Refuse, Fx::Reveal];

    fn resting(look: &Look) -> bool {
        look.height == 1.0 && look.opacity == 1.0 && look.dx == 0.0 && look.tint == 0.0
    }

    #[test]
    fn auto_follows_reduce_motion() {
        assert_eq!(MotionMode::Auto.resolve(false), Effective::Full);
        assert_eq!(MotionMode::Auto.resolve(true), Effective::Low);
        assert_eq!(MotionMode::Off.resolve(false), Effective::Off);
        assert_eq!(MotionMode::Low.resolve(false), Effective::Low);
        assert_eq!(MotionMode::Full.resolve(true), Effective::Full);
    }

    #[test]
    fn off_has_no_phases() {
        for fx in FX {
            assert!(phases(Effective::Off, fx).is_empty(), "{fx:?}");
        }
    }

    #[test]
    fn every_effect_ends_at_rest_or_gone() {
        for mode in MODES {
            for fx in FX {
                let phases = phases(mode, fx);
                if phases.is_empty() {
                    assert_eq!(look(mode, fx, 0, 0.0), Look::default(), "{mode:?} {fx:?} starts at rest");
                    continue;
                }
                let last = phases.len() - 1;
                let end = look(mode, fx, last, 1.0);
                if fx.leaving() {
                    assert_eq!(end.opacity, 0.0, "{mode:?} {fx:?} ends invisible");
                    // Full closes the gap itself; Low snaps it closed at once.
                    let expected_height = if mode == Effective::Full { 0.0 } else { 1.0 };
                    assert_eq!(end.height, expected_height, "{mode:?} {fx:?} ends at its final height");
                } else {
                    assert!(resting(&end), "{mode:?} {fx:?} ends at rest: {end:?}");
                    assert_eq!(end.badge, 0.0, "{mode:?} {fx:?} badge gone");
                    assert_eq!(end.trash, 0.0, "{mode:?} {fx:?} trash gone");
                }
            }
        }
    }

    #[test]
    fn low_never_moves_or_resizes() {
        for fx in FX {
            for phase in 0..phases(Effective::Low, fx).len() {
                for step in 0..=20 {
                    let t = step as f32 / 20.0;
                    let look = look(Effective::Low, fx, phase, t);
                    assert_eq!(look.dx, 0.0, "{fx:?} phase {phase} t {t}");
                    assert_eq!(look.height, 1.0, "{fx:?} phase {phase} t {t}");
                    assert_eq!(look.trash, 0.0, "{fx:?} phase {phase} t {t}");
                    assert_eq!(look.trash_size, 16.0, "{fx:?} phase {phase} t {t}");
                }
            }
        }
    }

    #[test]
    fn cue_table_matches_the_plan() {
        let trash =
            Some(Cue::Play { sound: SoundId::System("finder/move to trash.aif"), volume: 0.6, clip: Some(TRASH_CLIP) });
        let empty = Some(Cue::Play { sound: SoundId::System("finder/empty trash.aif"), volume: 0.6, clip: None });
        let named = |name, volume| Some(Cue::Play { sound: SoundId::Named(name), volume, clip: None });
        for set in [SoundSet::Finder, SoundSet::Crow] {
            assert_eq!(cue(set, Event::Trash), trash, "{set:?}");
            assert_eq!(cue(set, Event::EmptyTrash), empty, "{set:?}");
            assert_eq!(cue(set, Event::Refuse), Some(Cue::Beep), "{set:?}");
        }
        assert_eq!(cue(SoundSet::Finder, Event::Move), None);
        assert_eq!(cue(SoundSet::Finder, Event::NewFolder), None);
        assert_eq!(cue(SoundSet::Finder, Event::Undo), None);
        assert_eq!(cue(SoundSet::Finder, Event::CopyDone), None);
        assert_eq!(cue(SoundSet::Crow, Event::Move), named("Purr", 0.35));
        assert_eq!(cue(SoundSet::Crow, Event::NewFolder), named("Pop", 0.3));
        assert_eq!(cue(SoundSet::Crow, Event::Undo), named("Bottle", 0.3));
        assert_eq!(cue(SoundSet::Crow, Event::CopyDone), named("Glass", 0.35));
        for event in [
            Event::Trash,
            Event::EmptyTrash,
            Event::Move,
            Event::NewFolder,
            Event::Undo,
            Event::Refuse,
            Event::CopyDone,
        ] {
            assert_eq!(cue(SoundSet::Off, event), None, "{event:?}");
        }
    }

    #[test]
    fn preload_lists_cover_every_cue() {
        for set in [SoundSet::Finder, SoundSet::Crow] {
            let listed = sounds(set);
            for event in [Event::Trash, Event::EmptyTrash, Event::Move, Event::NewFolder, Event::Undo, Event::CopyDone]
            {
                if let Some(Cue::Play { sound, .. }) = cue(set, event) {
                    assert!(listed.contains(&sound), "{set:?} preloads {sound:?} for {event:?}");
                }
            }
        }
        assert!(sounds(SoundSet::Off).is_empty());
        assert_eq!(all_sounds().len(), 6, "trash, empty trash, Purr, Pop, Bottle, Glass");
    }
}
