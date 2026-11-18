//! Seeded action generator for the fuzzer.
//!
//! Steps are drawn from the live keymap and the elements on screen, so a seed
//! reproduces the same sequence on every platform. The runner (tests) applies
//! them to a real GPUI window; H3 uses the same generator in `crow --fuzz`.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

/// One action the runner applies. `Click`/`RightClick` name a `debug_selector`;
/// `RightClick`'s second field is the menu slot chosen (`None` dismisses it,
/// as does a slot past the end of that menu).
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Keys(String),
    Type(String),
    Click(String),
    RightClick(String, Option<usize>),
    Resize(f32, f32),
    Settle,
    AdvanceClock(u64),
    /// Delete the folder the active pane shows (inside the fuzz tree).
    DeleteShownFolder,
    /// Delete and recreate the folder the active pane shows with different contents.
    ReplaceShownFolder,
}

impl Step {
    /// The variant name, for the slowness report's table.
    #[cfg(feature = "fuzz")]
    pub fn kind(&self) -> &'static str {
        match self {
            Step::Keys(_) => "Keys",
            Step::Type(_) => "Type",
            Step::Click(_) => "Click",
            Step::RightClick(..) => "RightClick",
            Step::Resize(..) => "Resize",
            Step::Settle => "Settle",
            Step::AdvanceClock(_) => "AdvanceClock",
            Step::DeleteShownFolder => "DeleteShownFolder",
            Step::ReplaceShownFolder => "ReplaceShownFolder",
        }
    }
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
    /// `Ui.home` is the developer's real home folder (`init_for_tests`), so
    /// ⇧⌘H would walk it; the live fuzzer's scratch root is safe.
    pub home_is_real: bool,
}

/// The plain navigation keys every window understands, whatever the layer.
const NAV_KEYS: &[&str] = &["up", "down", "left", "right", "escape", "tab", "enter", "space", "backspace"];

/// Vim motions and marks, when the Vim layer is on.
const VIM_KEYS: &[&str] = &["h", "j", "k", "l", "g g", "shift-g", "m a", "' a"];

/// Short strings that exercise paths, text edges and the caret.
const TYPE_ALPHABET: &[&str] =
    &["a", "Z", "0", "9", "/", "~", ".", " ", "-", "_", "'", "\"", "🙂", "e\u{301}", "ש", "م"];

/// Keys that open a mode the rest of the run can then use. Picked more often
/// than a uniform keymap draw: search and the palette were never reached in a
/// 120-step smoke run before B5, so their surfaces went unfuzzed.
const OPEN_MODE_KEYS: &[&str] = &["cmd-f", "cmd-k", "cmd-shift-g", "/"];

/// One more than the longest context menu (the sidebar favorite's, 17 slots),
/// so every item can be drawn.
const MENU_SLOTS: usize = 18;

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
            0..=39 => self.keys(state),
            40..=44 => self.mode_key(state),
            45..=59 if state.text_focused => self.type_text(),
            45..=59 => self.keys(state),
            60..=74 if !state.selectors.is_empty() => self.click(state, false),
            60..=74 => self.keys(state),
            75..=79 if !state.selectors.is_empty() => self.click(state, true),
            75..=79 => self.keys(state),
            80..=84 => Step::Settle,
            85..=89 => Step::AdvanceClock(self.rng.random_range(50..=2000)),
            90..=91 => Step::DeleteShownFolder,
            92..=93 => Step::ReplaceShownFolder,
            _ => {
                let (width, height) = SIZES[self.rng.random_range(0..SIZES.len())];
                Step::Resize(width, height)
            }
        }
    }

    /// Steps for the Settings window: keys, typing, clicks and settles only.
    /// It has no rows to right-click, folders to churn or size to resize.
    pub fn next_settings(&mut self, state: &GenState) -> Step {
        let roll = self.rng.random_range(0..100);
        match roll {
            0..=49 => self.keys(state),
            50..=64 if state.text_focused => self.type_text(),
            50..=64 => self.keys(state),
            65..=84 if !state.selectors.is_empty() => self.click(state, false),
            65..=84 => self.keys(state),
            85..=94 => Step::Settle,
            _ => Step::AdvanceClock(self.rng.random_range(50..=2000)),
        }
    }

    /// A key that opens search, the palette, Go to Folder or the filter, when
    /// one resolves in this focus context; otherwise a normal key.
    fn mode_key(&mut self, state: &GenState) -> Step {
        let pool: Vec<&String> =
            state.active_keys.iter().filter(|key| OPEN_MODE_KEYS.contains(&key.as_str())).collect();
        if pool.is_empty() {
            return self.keys(state);
        }
        Step::Keys(pool[self.rng.random_range(0..pool.len())].clone())
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
        let pool: Vec<&String> =
            source.iter().filter(|key| allowed(key, state.tabs, state.windows, state.home_is_real)).collect();
        if pool.is_empty() {
            return Step::Keys("escape".to_owned());
        }
        Step::Keys(pool[self.rng.random_range(0..pool.len())].clone())
    }

    fn type_text(&mut self) -> Step {
        let mut text = String::new();
        for _ in 0..self.rng.random_range(1..=3) {
            text.push_str(TYPE_ALPHABET[self.rng.random_range(0..TYPE_ALPHABET.len())]);
        }
        Step::Type(text)
    }

    fn click(&mut self, state: &GenState, right: bool) -> Step {
        let selector = state.selectors[self.rng.random_range(0..state.selectors.len())].clone();
        if !right {
            return Step::Click(selector);
        }
        let choice = if self.rng.random_bool(0.25) { None } else { Some(self.rng.random_range(0..MENU_SLOTS)) };
        Step::RightClick(selector, choice)
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

/// A replayable sequence of steps. `print` gives Rust that pastes into a test.
#[derive(Clone, Debug, PartialEq)]
pub struct Script(pub Vec<Step>);

impl Script {
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn print(&self) -> String {
        let steps: Vec<String> = self.0.iter().map(step_source).collect();
        format!("crate::script![{}]", steps.join(", "))
    }
}

/// `script![Keys("cmd-k".into()), Step::Settle]`, for pasted regression tests.
#[macro_export]
macro_rules! script {
    ($($step:expr),* $(,)?) => {
        $crate::fuzz::Script(vec![$($step),*])
    };
}

fn step_source(step: &Step) -> String {
    match step {
        Step::Keys(keys) => format!("Step::Keys({keys:?}.into())"),
        Step::Type(text) => format!("Step::Type({text:?}.into())"),
        Step::Click(selector) => format!("Step::Click({selector:?}.into())"),
        Step::RightClick(selector, choice) => format!("Step::RightClick({selector:?}.into(), {choice:?})"),
        Step::Resize(width, height) => format!("Step::Resize({width:?}, {height:?})"),
        Step::Settle => "Step::Settle".to_owned(),
        Step::AdvanceClock(ms) => format!("Step::AdvanceClock({ms})"),
        Step::DeleteShownFolder => "Step::DeleteShownFolder".to_owned(),
        Step::ReplaceShownFolder => "Step::ReplaceShownFolder".to_owned(),
    }
}

/// One failing run: the seed and step, the violation, and the shrunk script.
#[derive(Clone, Debug)]
pub struct Failure {
    pub seed: u64,
    pub step: usize,
    pub violation: Violation,
    /// The shortest failing sequence the shrinker found.
    pub script: Script,
    /// The sequence as generated, for the findings table's step count.
    pub original: Script,
    /// The edge tree's label (`none` for the ordinary fuzz tree).
    pub tree: String,
    /// Whether churn ran during the failing run.
    pub churn: bool,
    /// Whether the shrunk script still fails the same check with churn off.
    pub reproduces_without_churn: bool,
}

impl Failure {
    pub fn print(&self) -> String {
        format!(
            "seed {} step {}: {} — {}\ntree {}; churn {}; shrunk script reproduces without churn: {}\noriginal {} steps, shrunk to {}:\n// paste into window/tests.rs with `use crate::fuzz::Step;`\n{}",
            self.seed,
            self.step,
            self.violation.check,
            self.violation.detail,
            self.tree,
            if self.churn { "ran" } else { "off" },
            if self.reproduces_without_churn { "yes" } else { "no" },
            self.original.len(),
            self.script.len(),
            self.script.print()
        )
    }

    /// The row for the milestone's `findings.md` (M7's is `docs/plans/done/m7/findings.md`); the agent fills the `#` and status.
    pub fn finding_row(&self) -> String {
        format!(
            "| | H2 | {} / {} / {} | {}: {} ({} steps; without churn: {}) | `{}` | open |",
            self.seed,
            self.tree,
            if self.churn { "churn" } else { "still" },
            self.violation.check,
            self.violation.detail,
            self.original.len(),
            if self.reproduces_without_churn { "yes" } else { "no" },
            self.script.print()
        )
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
            // io-ok: tracing event metadata, not a file
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
fn allowed(key: &str, tabs: usize, windows: usize, home_is_real: bool) -> bool {
    match key {
        // Quitting and minimizing are not fuzzable: the test platform has no
        // implementation for the latter, and the former ends the run.
        "cmd-q" | "cmd-m" | "ctrl-cmd-space" => false,
        // ⇧⌘H searches Home. Under `ui::init_for_tests` that is the developer's
        // real home, which a test must not walk; `crow --fuzz` uses a scratch
        // root. The key is the only fuzzer route: the Home segment is not in
        // `SearchView::debug_selectors`, and the palette needs `>` for commands.
        "cmd-shift-h" if home_is_real => false,
        "cmd-w" if tabs <= 1 => false,
        "cmd-t" | "cmd-enter" if tabs >= MAX_TABS => false,
        "cmd-n" if windows >= MAX_WINDOWS => false,
        _ => true,
    }
}

/// Live bindings and caps for the window in front of the generator.
#[cfg(any(test, feature = "fuzz"))]
fn base_state(
    window: &gpui::Window,
    cx: &gpui::App,
    selectors: Vec<String>,
    text_focused: bool,
    tabs: usize,
    home_is_real: bool,
) -> GenState {
    let stack = window.context_stack();
    let os = cx.platform_info().os;
    let vim = cx.global::<crate::ui::Ui>().settings.keymap == crow_core::KeymapLayer::Vim;
    let windows = cx.windows().len();
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let mut active = Vec::new();
    let mut all = Vec::new();
    for binding in keymap.bindings() {
        let keys = binding.keystrokes().iter().map(|key| key.inner().unparse(os)).collect::<Vec<_>>().join(" ");
        if !allowed(&keys, tabs, windows, home_is_real) {
            continue;
        }
        all.push(keys.clone());
        let (resolved, _) = keymap.bindings_for_input(binding.keystrokes(), &stack);
        if resolved.first().is_some_and(|found| found.action().partial_eq(binding.action())) {
            active.push(keys);
        }
    }
    let mut state =
        GenState { active_keys: active, all_keys: all, text_focused, selectors, tabs, windows, vim, home_is_real };
    state.add_plain_keys();
    state
}

/// The generator's view of `view`: live bindings, focus, selectors and caps.
/// The fuzz runner has a real `Window`; the test runner wraps this in its context.
#[cfg(any(test, feature = "fuzz"))]
pub fn state_for(view: &crate::window::CrowWindow, window: &gpui::Window, cx: &gpui::App) -> GenState {
    let home_is_real = cx.global::<crate::ui::Ui>().home == crow_core::home();
    base_state(window, cx, view.debug_selectors(cx), view.text_focused(window, cx), view.tab_count(), home_is_real)
}

/// The Settings window as a fuzz surface: its controls' selectors, the keys its
/// own keymap resolves, and no window-closing keys. `cmd-w` would end the
/// surface, and Escape closes the window unless a drop-down is open.
///
/// Test-only: it clips selectors with `Window::debug_bounds`, which needs GPUI's
/// `test-support`; the live fuzzer builds without it.
#[cfg(test)]
pub fn settings_state_for(
    view: &crate::settings_window::SettingsWindow,
    window: &gpui::Window,
    cx: &gpui::App,
) -> GenState {
    let height = window.viewport_size().height;
    let selectors = view
        .debug_selectors()
        .into_iter()
        .filter(|selector| {
            window.debug_bounds(selector).is_some_and(|bounds| bounds.top() < height && bounds.bottom() > gpui::px(0.))
        })
        .collect();
    let home_is_real = cx.global::<crate::ui::Ui>().home == crow_core::home();
    let mut state = base_state(window, cx, selectors, view.text_focused(window), 2, home_is_real);
    state.active_keys.retain(|key| key != "cmd-w");
    state.all_keys.retain(|key| key != "cmd-w");
    if !view.menu_open() {
        state.active_keys.retain(|key| key != "escape");
        state.all_keys.retain(|key| key != "escape");
    }
    state
}

/// Every painted element with a click listener or focus handle must expose a
/// role and a non-empty label. Empty in the live fuzzer, which builds without
/// GPUI's `test-support` and so doesn't record the list.
#[cfg(any(test, feature = "fuzz"))]
pub fn a11y_violations(window: &gpui::Window, check: &'static str) -> Vec<Violation> {
    #[cfg(test)]
    {
        let Some(elements) = window.debug_interactive_elements() else { return Vec::new() };
        let mut out = Vec::new();
        for element in elements {
            let name = element.selector.as_deref().unwrap_or("an element");
            if element.role.is_none() {
                out.push(Violation::new(check, format!("{name} is interactive but has no role")));
            }
            if element.label.as_ref().is_none_or(|label| label.trim().is_empty()) {
                out.push(Violation::new(check, format!("{name} is interactive but has no aria label")));
            }
        }
        out
    }
    #[cfg(not(test))]
    {
        let _ = (window, check);
        Vec::new()
    }
}

#[cfg(test)]
pub fn state(view: &gpui::Entity<crate::window::CrowWindow>, cx: &mut gpui::VisualTestContext) -> GenState {
    cx.update(|window, cx| state_for(view.read(cx), window, cx))
}

/// Run until parked, then wait (bounded) for every tab, operation and watcher
/// poll to quiet. The folder poll runs every 100 ms on the test clock, so each
/// turn advances it; two quiet turns cover a change delivered by the watcher.
#[cfg(test)]
pub fn settle(view: &gpui::Entity<crate::window::CrowWindow>, cx: &mut gpui::VisualTestContext) {
    let mut quiet = 0;
    for _ in 0..40 {
        cx.executor().advance_clock(std::time::Duration::from_millis(150));
        cx.run_until_parked();
        if view.read_with(cx, |window, cx| window.is_busy(cx)) {
            quiet = 0;
        } else {
            quiet += 1;
            if quiet >= 2 {
                return;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("folders never settled");
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
            home_is_real: true,
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
            active_keys: vec!["cmd-q".into(), "cmd-w".into(), "cmd-t".into(), "cmd-n".into(), "cmd-shift-h".into()],
            all_keys: vec!["cmd-q".into(), "cmd-w".into(), "cmd-t".into(), "cmd-n".into(), "cmd-shift-h".into()],
            tabs: 1,
            windows: 1,
            home_is_real: true,
            ..GenState::default()
        };
        let mut generator = StepGen::new(1);
        for _ in 0..500 {
            if let Step::Keys(keys) = generator.next(&state) {
                assert!(keys != "cmd-q", "never quits");
                assert!(keys != "cmd-w", "never closes the last tab");
                assert!(keys != "cmd-shift-h", "never walks the real home");
            }
        }
    }

    #[test]
    fn search_home_is_skipped_only_with_a_real_home() {
        assert!(!allowed("cmd-shift-h", 1, 1, true));
        assert!(allowed("cmd-shift-h", 1, 1, false), "the live fuzzer's scratch root is safe");
    }

    #[test]
    fn shown_folder_steps_print() {
        let script = Script(vec![Step::DeleteShownFolder, Step::ReplaceShownFolder]);
        assert_eq!(script.print(), "crate::script![Step::DeleteShownFolder, Step::ReplaceShownFolder]");
    }

    #[test]
    fn shown_folder_steps_are_rare() {
        let state = state();
        let mut generator = StepGen::new(9);
        let (mut delete, mut replace) = (0, 0);
        for _ in 0..10_000 {
            match generator.next(&state) {
                Step::DeleteShownFolder => delete += 1,
                Step::ReplaceShownFolder => replace += 1,
                _ => {}
            }
        }
        assert!(delete > 0 && replace > 0, "{delete} {replace}");
        assert!(delete + replace < 1_000, "low weight: {delete} {replace}");
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
