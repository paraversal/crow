//! Logs for shortcuts that reach Crow but run nothing, and for key-window changes, so an
//! intermittent "shortcuts stop working" leaves a trace in crow.log (target `input`).
//! Only Command and Control chords are logged, never plain typing.

use gpui::{App, KeystrokeEvent, Window};

/// Watch every keystroke Crow receives; call once at startup.
pub fn init(cx: &mut App) {
    cx.observe_keystrokes(log_unmatched_shortcut).detach();
}

fn is_shortcut(event: &KeystrokeEvent) -> bool {
    let modifiers = &event.keystroke.modifiers;
    modifiers.platform || modifiers.control
}

fn log_unmatched_shortcut(event: &KeystrokeEvent, window: &mut Window, _cx: &mut App) {
    if event.action.is_some() || !is_shortcut(event) {
        return;
    }
    let contexts = event.context_stack.iter().map(|context| format!("{context:?}")).collect::<Vec<_>>().join(" > ");
    tracing::warn!(
        target: "input",
        "shortcut matched no binding: keys={} window_active={} contexts=[{contexts}] macos: {}",
        event.keystroke.unparse(gpui::OperatingSystem::Mac),
        window.is_window_active(),
        snapshot(),
    );
}

/// Log a window becoming or ceasing to be key, with what macOS reports at that moment.
pub fn log_activation(window: &Window) {
    tracing::info!(
        target: "input",
        "window {}: macos: {}",
        if window.is_window_active() { "became key" } else { "resigned key" },
        snapshot(),
    );
}

fn snapshot() -> String {
    crow_macos::input_snapshot().map_or_else(|| "unavailable".into(), |snapshot| snapshot.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(keys: &'static str) -> KeystrokeEvent {
        KeystrokeEvent {
            keystroke: gpui::Keystroke::parse(keys).unwrap(),
            input_preference: gpui::InputPreference::KeyBindings,
            action: None,
            context_stack: Vec::new(),
        }
    }

    #[test]
    fn only_command_and_control_chords_count_as_shortcuts() {
        assert!(is_shortcut(&event("cmd-k")));
        assert!(is_shortcut(&event("ctrl-w")));
        assert!(!is_shortcut(&event("k")), "plain typing is never logged");
        assert!(!is_shortcut(&event("shift-k")));
        assert!(!is_shortcut(&event("alt-k")));
    }
}
