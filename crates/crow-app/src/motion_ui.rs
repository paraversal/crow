//! Shared pieces for views that animate: the exit holder and the glide spring.
//! Durations, easings and looks live in `crow_core::motion`.

use std::time::Instant;

use crow_core::motion::{self, Effective};
use gpui::{SpringAnimation, SpringConfig};

/// The spring every highlight glide uses (the list cursor, the sidebar fill,
/// the palette row): critically damped, so a new target never overshoots.
pub fn glide_config() -> SpringConfig {
    SpringConfig::new(2500.0, 2.0 * 2500.0f32.sqrt(), 1.0)
}

/// The spring a progress bar eases with: critically damped and slow enough to
/// span `ms::PROGRESS`, so a burst of updates never overshoots or jitters.
pub fn progress_config() -> SpringConfig {
    let stiffness = 300.0f32;
    SpringConfig::new(stiffness, 2.0 * stiffness.sqrt(), 1.0)
}

/// A glide to `target` (a position or size) with the shared spring.
pub fn glide<T: gpui::SpringTarget>(target: T) -> SpringAnimation<T> {
    SpringAnimation::new(glide_config()).to(target)
}

pub fn ease_progress(fraction: f32) -> SpringAnimation<f32> {
    SpringAnimation::new(progress_config()).to(fraction)
}

/// A big preview that has just loaded fades in over `PREVIEW_FADE`. Only a
/// loaded preview is wrapped, and its animation is keyed by the item: a preview
/// loads after a pause (the loader waits for the cursor to rest), so arrowing
/// fast shows placeholders that never fade, and each landing preview does.
pub fn fade_when_loaded(element: gpui::Div, loaded: bool, key: &std::path::Path, mode: Effective) -> gpui::AnyElement {
    use gpui::{Animation, AnimationExt as _, IntoElement as _, Styled as _};
    if !loaded || mode == Effective::Off {
        return element.into_any_element();
    }
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    element
        .with_animation(
            gpui::ElementId::from(("preview-fade", hasher.finish() as usize)),
            Animation::new(std::time::Duration::from_millis(motion::ms::PREVIEW_FADE)),
            |el, t| el.opacity(motion::ease_out(t)),
        )
        .into_any_element()
}

/// A value a view keeps after the logical state has changed, so its last
/// picture can fade out. The view renders it with input off (no listeners worth
/// keeping, nothing focusable) and drops it when `progress` returns `None`.
/// Focus and the logical state move on at once; only the picture lingers.
pub struct Leaving<T> {
    pub value: T,
    started: Instant,
    full_ms: u64,
    low_ms: u64,
}

impl<T> Leaving<T> {
    pub fn new(value: T, now: Instant, full_ms: u64, low_ms: u64) -> Self {
        Leaving { value, started: now, full_ms, low_ms }
    }

    /// `1 → 0` fade as it leaves: `Some(opacity)` while live, `None` when done or Off.
    pub fn remaining(&self, mode: Effective, now: Instant) -> Option<f32> {
        motion::progress(mode, self.started, now, self.full_ms, self.low_ms).map(|t| 1.0 - t)
    }
}

/// Move `slot`'s value into `exit` as a `Leaving` (or drop it when Off) and
/// leave `slot` empty. Returns whether something was open.
pub fn retire<T>(
    slot: &mut Option<T>,
    exit: &mut Option<Leaving<T>>,
    mode: Effective,
    full_ms: u64,
    low_ms: u64,
) -> bool {
    let Some(value) = slot.take() else { return false };
    *exit = (mode != Effective::Off).then(|| Leaving::new(value, crate::clock::now(), full_ms, low_ms));
    true
}

/// A `Leaving` value lent back to its slot for one render, so the view's
/// ordinary render code draws the closed sheet's last picture.
pub struct Loan {
    shell: Leaving<()>,
    /// 1 → 0 while it fades out.
    pub fade: f32,
}

/// If `slot` is empty and `exit` still has a live picture, put its value in
/// `slot` and return the loan; call `reclaim` after rendering. An open `slot`
/// (the sheet reopened) or a finished exit clears `exit`.
pub fn lend<T>(slot: &mut Option<T>, exit: &mut Option<Leaving<T>>, mode: Effective, now: Instant) -> Option<Loan> {
    if slot.is_some() {
        *exit = None;
        return None;
    }
    let leaving = exit.take()?;
    let fade = leaving.remaining(mode, now)?;
    let Leaving { value, started, full_ms, low_ms } = leaving;
    *slot = Some(value);
    Some(Loan { shell: Leaving { value: (), started, full_ms, low_ms }, fade })
}

/// Take the value back out of `slot` after the render.
pub fn reclaim<T>(slot: &mut Option<T>, exit: &mut Option<Leaving<T>>, loan: Loan) {
    let Loan { shell: Leaving { started, full_ms, low_ms, .. }, .. } = loan;
    if let Some(value) = slot.take() {
        *exit = Some(Leaving { value, started, full_ms, low_ms });
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn a_leaving_value_fades_from_one_to_zero_then_ends() {
        let start = Instant::now();
        let leaving = Leaving::new((), start, 110, 110);
        let first = leaving.remaining(Effective::Full, start).unwrap();
        let mid = leaving.remaining(Effective::Full, start + Duration::from_millis(55)).unwrap();
        assert_eq!(first, 1.0);
        assert!(mid > 0.0 && mid < 1.0);
        assert_eq!(leaving.remaining(Effective::Full, start + Duration::from_millis(110)), None);
        assert_eq!(leaving.remaining(Effective::Off, start), None);
    }

    #[test]
    fn a_loan_puts_the_value_back_in_its_slot_for_one_render() {
        let (mut slot, mut exit) = (Some(3), None);
        retire(&mut slot, &mut exit, Effective::Full, 110, 110);
        let loan = lend(&mut slot, &mut exit, Effective::Full, Instant::now()).expect("still fading");
        assert_eq!(slot, Some(3), "rendered as if open");
        assert!(loan.fade > 0.9);
        reclaim(&mut slot, &mut exit, loan);
        assert!(slot.is_none() && exit.is_some());
        // Reopened meanwhile: the old picture is dropped, never lent.
        slot = Some(9);
        assert!(lend(&mut slot, &mut exit, Effective::Full, Instant::now()).is_none());
        assert!(exit.is_none() && slot == Some(9));
        // Expired: dropped.
        let (mut slot, mut exit) = (Some(3), None);
        retire(&mut slot, &mut exit, Effective::Full, 110, 110);
        let later = Instant::now() + std::time::Duration::from_millis(500);
        assert!(lend(&mut slot, &mut exit, Effective::Full, later).is_none());
        assert!(slot.is_none() && exit.is_none());
    }

    #[test]
    fn retire_keeps_the_value_unless_off() {
        let (mut slot, mut exit) = (Some(7), None);
        assert!(retire(&mut slot, &mut exit, Effective::Full, 110, 110));
        assert!(slot.is_none() && exit.as_ref().is_some_and(|leaving| leaving.value == 7));
        let (mut slot, mut exit) = (Some(7), None);
        assert!(retire(&mut slot, &mut exit, Effective::Off, 110, 110));
        assert!(exit.is_none(), "Off drops it at once");
        assert!(!retire(&mut slot, &mut exit, Effective::Full, 110, 110), "nothing open");
    }
}
