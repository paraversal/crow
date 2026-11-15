//! Seeded action generator for the fuzzer.
//!
//! Steps are drawn from the live keymap and the elements on screen, so a seed
//! reproduces the same sequence on every platform. The runner (tests) applies
//! them to a real GPUI window; H3 uses the same generator in `crow --fuzz`.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

/// One action the runner applies. `Click`/`RightClick` name a `debug_selector`.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Keys(String),
    Type(String),
    Click(String),
    RightClick(String),
    Resize(f32, f32),
    Settle,
    AdvanceClock(u64),
}

/// What the generator knows about the window right now.
#[derive(Clone, Debug, Default)]
pub struct GenState {
    /// Keystrokes whose binding resolves in the current focus context.
    pub active_keys: Vec<String>,
    /// Every binding allowed by the caps, whether or not it fires here.
    pub all_keys: Vec<String>,
    /// A text field (filter, rename, search, palette) has focus.
    pub text_focused: bool,
    /// Laid-out selectors the runner may click.
    pub selectors: Vec<String>,
    pub tabs: usize,
    pub windows: usize,
    pub vim: bool,
}

/// The plain navigation keys every window understands, whatever the layer.
const NAV_KEYS: &[&str] = &["up", "down", "left", "right", "escape", "tab", "enter", "space", "backspace"];

/// Vim motions and marks, when the Vim layer is on.
const VIM_KEYS: &[&str] = &["h", "j", "k", "l", "g g", "shift-g", "m a", "' a"];

/// Short strings that exercise paths, text edges and the caret.
const TYPE_ALPHABET: &[&str] =
    &["a", "Z", "0", "9", "/", "~", ".", " ", "-", "_", "'", "\"", "🙂", "👩🏽‍💻", "e\u{301}", "ש", "م"];

const SIZES: &[(f32, f32)] = &[(700., 500.), (1280., 800.), (1920., 1100.)];

/// The caps from H1 Q3: four tabs, two windows.
const MAX_TABS: usize = 4;
const MAX_WINDOWS: usize = 2;

/// Draws steps from a seeded RNG. The same seed and state give the same steps.
pub struct StepGen {
    rng: ChaCha8Rng,
}

impl StepGen {
    pub fn new(seed: u64) -> Self {
        StepGen { rng: ChaCha8Rng::seed_from_u64(seed) }
    }

    pub fn next(&mut self, state: &GenState) -> Step {
        let roll = self.rng.random_range(0..100);
        match roll {
            0..=44 => self.keys(state),
            45..=59 if state.text_focused => self.type_text(),
            45..=59 => self.keys(state),
            60..=74 if !state.selectors.is_empty() => self.click(state, false),
            60..=74 => self.keys(state),
            75..=79 if !state.selectors.is_empty() => self.click(state, true),
            75..=79 => self.keys(state),
            80..=84 => Step::Settle,
            85..=89 => Step::AdvanceClock(self.rng.random_range(50..=2000)),
            _ => {
                let (width, height) = SIZES[self.rng.random_range(0..SIZES.len())];
                Step::Resize(width, height)
            }
        }
    }

    fn keys(&mut self, state: &GenState) -> Step {
        let active = self.rng.random_bool(0.8);
        let source = if active && !state.active_keys.is_empty() {
            &state.active_keys
        } else if !state.all_keys.is_empty() {
            &state.all_keys
        } else {
            return Step::Keys("escape".to_owned());
        };
        let pool: Vec<&String> = source.iter().filter(|key| allowed(key, state.tabs, state.windows)).collect();
        if pool.is_empty() {
            return Step::Keys("escape".to_owned());
        }
        Step::Keys(pool[self.rng.random_range(0..pool.len())].clone())
    }

    fn type_text(&mut self) -> Step {
        let mut text = String::new();
        for _ in 0..self.rng.random_range(1..=6) {
            text.push_str(TYPE_ALPHABET[self.rng.random_range(0..TYPE_ALPHABET.len())]);
        }
        Step::Type(text)
    }

    fn click(&mut self, state: &GenState, right: bool) -> Step {
        let selector = state.selectors[self.rng.random_range(0..state.selectors.len())].clone();
        if right { Step::RightClick(selector) } else { Step::Click(selector) }
    }
}

impl GenState {
    /// Add the plain navigation keys (and Vim's, when that layer is on).
    pub fn add_plain_keys(&mut self) {
        for key in NAV_KEYS {
            self.push_key(key);
        }
        if self.vim {
            for key in VIM_KEYS {
                self.push_key(key);
            }
        }
    }

    fn push_key(&mut self, key: &str) {
        if !self.all_keys.iter().any(|known| known == key) {
            self.all_keys.push(key.to_owned());
        }
        if !self.active_keys.iter().any(|known| known == key) {
            self.active_keys.push(key.to_owned());
        }
    }
}

/// One failed invariant, with the check's name and the state that broke it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    pub check: &'static str,
    pub detail: String,
}

impl Violation {
    pub fn new(check: &'static str, detail: impl Into<String>) -> Self {
        Violation { check, detail: detail.into() }
    }
}

/// Error-level log lines since the last step. Warnings are allowed.
#[cfg(test)]
pub mod logs {
    use std::cell::Cell;

    use tracing::Level;
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::{Context, SubscriberExt};

    thread_local! {
        static ERRORS: Cell<usize> = const { Cell::new(0) };
    }

    struct Counter;

    impl<S: tracing::Subscriber> Layer<S> for Counter {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
            if event.metadata().level() == &Level::ERROR {
                ERRORS.with(|count| count.set(count.get() + 1));
            }
        }
    }

    /// Install the counter once. A subscriber installed by someone else wins;
    /// then the check sees no errors (recorded in the progress file).
    pub fn install() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let subscriber = tracing_subscriber::registry().with(Counter);
            let _ = tracing::subscriber::set_global_default(subscriber);
        });
    }

    pub fn errors() -> usize {
        ERRORS.with(Cell::get)
    }

    pub fn reset() {
        ERRORS.with(|count| count.set(0));
    }
}

/// Whether a binding may be generated under the current caps. Quitting and
/// closing the last tab are never generated; new tabs and windows stop at the caps.
fn allowed(key: &str, tabs: usize, windows: usize) -> bool {
    match key {
        "cmd-q" => false,
        "cmd-w" if tabs <= 1 => false,
        "cmd-t" | "cmd-enter" if tabs >= MAX_TABS => false,
        "cmd-n" if windows >= MAX_WINDOWS => false,
        _ => true,
    }
}

/// The generator's view of `view`: live bindings, focus, selectors and caps.
#[cfg(test)]
pub fn state(view: &gpui::Entity<crate::window::CrowWindow>, cx: &mut gpui::VisualTestContext) -> GenState {
    let stack = cx.update(|window, _| window.context_stack());
    let os = cx.update(|_, cx| cx.platform_info().os);
    let text_focused = cx.update(|window, cx| view.read(cx).text_focused(window, cx));
    let selectors = view.read_with(cx, |window, cx| window.debug_selectors(cx));
    let (tabs, vim) = view.read_with(cx, |window, cx| {
        (window.tab_count(), cx.global::<crate::ui::Ui>().settings.keymap == crow_core::KeymapLayer::Vim)
    });
    let windows = cx.update(|_, cx| cx.windows().len());
    let (active_keys, all_keys) = cx.update(|_, cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let mut active = Vec::new();
        let mut all = Vec::new();
        for binding in keymap.bindings() {
            let keys = binding.keystrokes().iter().map(|key| key.inner().unparse(os)).collect::<Vec<_>>().join(" ");
            if !allowed(&keys, tabs, windows) {
                continue;
            }
            all.push(keys.clone());
            let (resolved, _) = keymap.bindings_for_input(binding.keystrokes(), &stack);
            if resolved.first().is_some_and(|found| found.action().partial_eq(binding.action())) {
                active.push(keys);
            }
        }
        (active, all)
    });
    let mut state = GenState { active_keys, all_keys, text_focused, selectors, tabs, windows, vim };
    state.add_plain_keys();
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> GenState {
        GenState {
            active_keys: vec!["down".into(), "space".into(), "cmd-k".into()],
            all_keys: vec!["down".into(), "space".into(), "cmd-k".into(), "cmd-q".into()],
            text_focused: true,
            selectors: vec!["row-alpha".into(), "row-notes.txt".into()],
            tabs: 1,
            windows: 1,
            vim: false,
        }
    }

    #[test]
    fn same_seed_same_steps() {
        let state = state();
        let mut first = StepGen::new(42);
        let mut second = StepGen::new(42);
        let a: Vec<_> = (0..300).map(|_| first.next(&state)).collect();
        let b: Vec<_> = (0..300).map(|_| second.next(&state)).collect();
        assert_eq!(a, b);
    }

    #[test]
    fn generated_keys_parse() {
        let state = state();
        let mut generator = StepGen::new(7);
        for _ in 0..500 {
            if let Step::Keys(keys) = generator.next(&state) {
                for part in keys.split_whitespace() {
                    gpui::Keystroke::parse_with_os(part, gpui::OperatingSystem::Mac)
                        .unwrap_or_else(|error| panic!("{part:?}: {error}"));
                }
            }
        }
    }

    #[test]
    fn forbidden_keys_are_never_generated() {
        let state = GenState {
            active_keys: vec!["cmd-q".into(), "cmd-w".into(), "cmd-t".into(), "cmd-n".into()],
            all_keys: vec!["cmd-q".into(), "cmd-w".into(), "cmd-t".into(), "cmd-n".into()],
            tabs: 1,
            windows: 1,
            ..GenState::default()
        };
        let mut generator = StepGen::new(1);
        for _ in 0..500 {
            if let Step::Keys(keys) = generator.next(&state) {
                assert!(keys != "cmd-q", "never quits");
                assert!(keys != "cmd-w", "never closes the last tab");
            }
        }
    }

    #[test]
    fn error_logs_are_counted() {
        logs::install();
        logs::reset();
        tracing::error!("fuzz test error");
        assert_eq!(logs::errors(), 1);
        tracing::warn!("fuzz test warning");
        assert_eq!(logs::errors(), 1, "warnings are allowed");
    }
}
