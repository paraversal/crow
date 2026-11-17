//! The first-launch sheet's state: whether it is still owed and the file the
//! answer goes to. The sheet itself lives in `window.rs`.

use std::path::PathBuf;

use crow_core::{ONBOARDING_FILE, OnboardingState};
use gpui::{App, AppContext, Global, Task};

use crate::ui::Ui;

pub struct Onboarding {
    state: OnboardingState,
    file: PathBuf,
    /// The first window of the launch still owes the sheet.
    pending: bool,
    /// The pending save; a new one waits for it so the file never goes backwards.
    save: Option<Task<()>>,
}

impl Global for Onboarding {}

/// Load `onboarding.json` from the state dir. A startup read, like Tips'.
pub fn init(cx: &mut App) {
    let file = cx.global::<Ui>().state_dir.join(ONBOARDING_FILE);
    let state = OnboardingState::load(&file);
    let pending = !state.is_answered();
    cx.set_global(Onboarding { state, file, pending, save: None });
}

/// True once, for the first window of the launch. False when the question is
/// answered or no state is installed (tests that don't ask for the sheet).
pub fn take_pending(cx: &mut App) -> bool {
    let Some(onboarding) = cx.try_global::<Onboarding>() else { return false };
    if !onboarding.pending {
        return false;
    }
    cx.global_mut::<Onboarding>().pending = false;
    true
}

/// Record the answer and save it off the main thread. The caller writes `tips`.
pub fn answer(show_tips: bool, cx: &mut App) {
    let (changed, state, file, previous) = {
        let onboarding = cx.global_mut::<Onboarding>();
        let changed = onboarding.state.answer(show_tips);
        (changed, onboarding.state.clone(), onboarding.file.clone(), onboarding.save.take())
    };
    if !changed {
        return;
    }
    let task = cx.background_spawn(async move {
        if let Some(previous) = previous {
            previous.await;
        }
        if let Err(e) = state.save(&file) {
            tracing::warn!("{}: {e}", file.display());
        }
    });
    cx.global_mut::<Onboarding>().save = Some(task);
}

/// The answer already recorded, if any (tests read it).
#[cfg(test)]
pub fn answered(cx: &App) -> Option<bool> {
    cx.global::<Onboarding>().state.tips_choice()
}
