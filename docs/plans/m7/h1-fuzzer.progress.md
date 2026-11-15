# H1 action fuzzer progress
Branch: `m7-h1-fuzzer` · PR: none · Last updated: 2026-11-15 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 Side-effect seam (`effects`) | done (`4f8f83e`) | `crates/crow-app/src/effects.rs`: `Effects` (`Send + Sync`), `SystemEffects`, `RecordingEffects`, `EffectHost` global. Every escaping call site routed. `main.rs` installs `SystemEffects`; `ui::init_for_tests` installs `RecordingEffects`. Done-when grep hits only `effects.rs`. 10 effect assertions added/extended. |
| I2 Steps and generator | done | `crates/crow-app/src/fuzz.rs`: `Step`, `StepGen`, `GenState`, `state()` (live keymap + context stack + selectors). `CrowWindow`/`Pane`/`Sidebar`/`SearchView` grew `cfg(any(test, feature = "fuzz"))` selector and text-focus accessors. Caps: 4 tabs, 2 windows; `cmd-q` and last-tab `cmd-w` never generated. Tests: same seed same steps, every generated keystroke parses, forbidden keys never generated, live window yields bindings/selectors. |
| I3 Invariants | todo | Blocked on Q2/Q4, answered. |
| I4 Runner, shrinking, replay | todo | Blocked on Q1/Q3, answered. |
| I5 First run | todo | Needs I1–I4. |

## Questions
- Q1 (asked 2026-11-15): shrinker hand-rolled or `proptest`? → answer: **hand-rolled** delta debugging; `rand` + `rand_chacha`.
- Q2 (asked 2026-11-15): `#[cfg(test)]` only or a `fuzz` cargo feature? → answer: **`fuzz` feature**; code under `#[cfg(any(test, feature = "fuzz"))]`.
- Q3 (asked 2026-11-15): multiple windows and tabs in the fuzzer? → answer: **tabs (cap 4) and a second window (cap 2)**, invariants per window.
- Q4 (asked 2026-11-15): should invariant 6 cover search results? → answer: **no**; snapshot by design. Check vanished results only.
- Q5 (asked 2026-11-15): move `window.rs`'s test module out while adding `fuzz.rs`? → answer: **yes**, pure-move commit first (`f5a3edb`).
- Q6 (asked 2026-11-15, index): is the M7 order right, or should H6 come earlier? → answer: **as planned**.
- Q7 (asked 2026-11-15, index): nightly fuzz workflow in CI? → answer: **yes, but every couple of days, adjustable**; user asked for a good decision. Decision: every 3 days at 04:00 UTC, plus `workflow_dispatch` with an optional seed; `CROW_FUZZ_MINUTES` defaults to 20.

## Decisions
- Branch from `main` @ `50dd5a5` (M6.5 landed), not the plan's `56db832`; the M7 index says every piece branches from a `main` that has M6.5. — plan/index convention.
- Tests never run on the AppKit main thread (`crow-macos/src/lib.rs:37`), so most `crow_macos` effects were already no-ops in tests; `std::process::Command` (`pane.rs`, `window.rs`) and `mdfind` (`search.rs`) really escaped. The seam covers all of them.
- `RecordingEffects::show_context_menu` answers only when a test/fuzzer scripts a choice (`set_context_menu_choice`); with no script it returns false, so the existing `choose_context_item` / `choose_menu_item` tests keep driving the menu themselves. — preserves 8 existing tests and gives the fuzzer a scripted answer.
- The plan's "Return on a file records `open`" is wrong for Crow's keymap: `enter` is Rename; `OpenSelection` is `cmd-down` / `cmd-o` (`actions.rs:128-130`). The effect test uses `cmd-down`. — code convention, recorded as a plan deviation.
- Added `.debug_selector` to the Why sheet's local button helper (`window.rs`), so `why-open-file` is clickable in tests and by the fuzzer.
- `rand`/`rand_chacha` are optional normal dependencies enabled by the `fuzz` feature, and also dev-dependencies for tests. Dev-only would not compile H3's `crow --fuzz` release binary. — Q2 requires the generator outside tests.
- The generator filters forbidden keys again in `StepGen::keys`, not only in `state()`, so a caller cannot smuggle `cmd-q`/last-tab `cmd-w` in.
- `cmd-enter` (palette "open in tab") is capped with `cmd-t` at 4 tabs.

## Hand checks
- [ ] Effects, in the bundle: Return on a file opens it; Space Quick Looks; Show in Finder reveals; Copy Path pastes; the context menu opens and its items work; sounds play (if on). Same as before H1.
- [ ] Fuzzer: `CROW_FUZZ_MINUTES=2 cargo test -p crow-app fuzz_long -- --ignored --nocapture` launches no app, plays no sound, leaves the user's Trash and pasteboard alone.

## Found along the way
- `bottom_bar::open_config` had a `#[cfg(test)]` recorder (`OPENED`); replaced by the global seam.
- The Why sheet's buttons had ids but no debug selectors; added them.
- The Settings window is a separate `SettingsWindow` entity; the `CrowWindow` fuzzer accessors cannot see it. I4 will decide whether to drive it as a second window or leave it to its own tests.

## Log
- 2026-11-15: read the plan, M7 index, testing docs, repo skill; surveyed every `Command::new`/`crow_macos::` call site; created the branch and this file.
- 2026-11-15: I1. Added `effects.rs`, the `fuzz` feature, installed `SystemEffects` in `main` and `RecordingEffects` in `init_for_tests`, routed every escaping call. `cargo test -p crow-app` 377 passed; clippy and fmt clean.
- 2026-11-15: moved `window.rs`'s tests into `window/tests.rs` (pure move; `include_str!` path and indentation only). 377 tests still pass.
- 2026-11-15: I2. Added `fuzz.rs` and the selector/text-focus accessors; generator tests pass; clippy clean.

## Next
I3: `CrowWindow::check_invariants` with one deliberately broken unit test per check.
