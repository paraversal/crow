# M7 H1 — Action fuzzer

Audience: an implementing agent. Follow `.claude/skills/execute-plan/SKILL.md`; read `AGENTS.md`, `docs/testing.md` and `docs/plans/m7/README.md` first.
Base: branch `m7-h1-fuzzer` from `main`. Progress file: `docs/plans/m7/h1-fuzzer.progress.md`.
PR title: "M7 H1: action fuzzer". Don't merge.

## Why
Each U plan tested its own keys and states. Nothing tests random sequences across overlays (palette, search, Go to Folder, rename, conflict sheet, Why, theme picker, tips), focus contexts, views and tabs. U7-fix X7 (a deleted file coming back) was found by luck. H1 builds a reproducible fuzzer that finds such bugs and hands back a short script that replays them.

## Current state (main @ 56db832)
- GPUI tests: `crates/crow-app/src/window.rs` `mod tests` (≈ line 3093; 274 `#[gpui::test]`s). Helpers: `fixture`, `open`, `open_with`, `open_sized`, `settle`, `wait_for_search`, `pane(&view, &cx)`. Tests use `ui::init_for_tests` (temp config dir; `ui.rs:282`) and `cx.simulate_keystrokes` / `simulate_input` / `debug_bounds`.
- Trash in tests: `operations.rs:380–394` has a `#[cfg(test)] FolderTrash` (moves into a temp folder), so trashing in tests never touches the user's Trash. Keep it that way.
- **Side effects that escape the process** (must never run under the fuzzer):
  - `Pane::open_file` spawns `open <path>` (`pane.rs:935`): Return/⌘↓ on a file launches a real app.
  - Quick Look (`pane.rs:1282`, `crow_macos::toggle_quick_look`), Show in Finder, Copy Path / pasteboard, sound (U10), the native context menu (`context_menu.rs`, an AppKit `NSMenu` with a modal tracking loop), "Open in Terminal" or any other `Command::new`, notifications. Find them all: `rg 'Command::new|crow_macos::' crates/crow-app/src`.
- Keymap: `keys.rs` builds bindings; `cx.key_bindings()` (`keys.rs:79`) lists them live. Commands with menus/palette: `palette::Command` registry.
- No `proptest`/`rand` in the workspace yet.

## Items

### I1 Side-effect seam (`effects`)
**Change:** add `crates/crow-app/src/effects.rs`: a trait `Effects` with one method per escaping effect (`open(path)`, `reveal(paths)`, `quick_look(path)` / `close_quick_look()`, `write_pasteboard(..)`, `play(sound)`, `show_context_menu(model) -> Option<choice>`, plus whatever the `rg` finds), a GPUI global holding `Arc<dyn Effects>`. `SystemEffects` calls today's code unchanged; `RecordingEffects` (tests) records calls and returns a scripted answer (e.g. the context menu returns `None` or a chosen item). `main.rs` installs `SystemEffects`; `ui::init_for_tests` installs `RecordingEffects`. Route every call site through the global.
**Tests:** one existing keystroke test per effect asserts the recorded call (e.g. Return on `notes.txt` records `open(notes.txt)`), replacing any test that now would spawn `open`.
**Done when:** `rg 'Command::new|crow_macos::(toggle_quick_look|reveal|pasteboard|play)' crates/crow-app/src` hits only `effects.rs` (and `main.rs`/bench); behavior in the bundle is unchanged (hand check).

### I2 Steps and generator
**Change:** `crates/crow-app/src/fuzz.rs` (`#[cfg(test)]` module, or a `fuzz` cargo feature if H3 needs it outside tests; see Q2):
- `enum Step { Keys(String), Type(String), Click(String /* debug selector */), RightClick(String), Resize(f32, f32), Settle, AdvanceClock(u64 /* ms */) }`.
- `StepGen` draws steps from a seeded RNG, weighted:
  - **Keys** from the live keymap (`cx.key_bindings()`), filtered to the bindings whose context matches the current focus most of the time (say 80 %), any binding the rest; plus plain navigation keys (arrows, escape, tab, enter, space, backspace) and Vim keys when the Vim layer is on.
  - **Type** only when a text field has focus: short strings from an alphabet that includes `/`, `~`, `.`, spaces, emoji, combining marks and RTL letters.
  - **Click / RightClick** on a selector picked from the elements on screen. Rows, sidebar items, toolbar buttons and path segments already have `debug_selector`s (`row-<name>`, `column-<n>-row-<name>`, `sidebar`, `path-bar`, …); collect visible ones via a `#[cfg(test)]` accessor rather than guessing names.
  - **Resize** among a few sizes (700×500, 1280×800, 1920×1100).
  - **Settle** / **AdvanceClock** occasionally, so timers (announcements, tips, motion, debounce) fire mid-sequence.
- Never generate quitting or closing the last window (`cmd-q`, `cmd-w` on the last tab); allow new tabs/windows up to a cap (Q3).
**Tests:** the same seed produces the same `Vec<Step>` twice; generated keys all parse as keystrokes.

### I3 Invariants
**Change:** `CrowWindow::check_invariants(&self, cx) -> Vec<Violation>` (cfg as Q2), checked after every step:
1. no panic (the runner catches it and reports the step);
2. every pane's cursor and selection index rows that exist; the selection's paths are in the listing;
3. exactly one focused element, and it has a role and an aria label;
4. an overlay open ⇒ focus is inside it; no overlay open ⇒ focus is in a pane, the sidebar or the filter/search field;
5. at most one overlay of each kind; the rename field exists only while a rename is in progress;
6. after `Settle`: each shown folder's listing equals `read_dir` of that folder (names, honoring the hidden-files setting and the filter);
7. no log line at `error` level since the last step (hook `tracing` in tests; warnings allowed).
Add each check with a deliberately broken state in a unit test that proves it fires.

### I4 Runner, shrinking, replay
**Change:** `fuzz::run(seed, steps, tree) -> Result<(), Failure>` opens a window on a temp tree (H1 uses the existing `fixture` shape plus a few hundred files; H2 adds edge trees), applies steps, checks invariants. On failure:
- **Shrink** to a minimal failing sequence: delta debugging (drop halves, then single steps, then simplify `Type` strings), re-running from a fresh window each time. Hand-rolled unless Q1 says `proptest`.
- **Print** the seed, the failing step, the violation, and the shrunk script as Rust you can paste into `window.rs` tests: `fuzz::replay(cx, &tree, script![Keys("cmd-k"), Type("~/"), Keys("tab"), …])`.
- **Record** the finding in `docs/plans/m7/findings.md` (create it: `| # | Seed | Steps | Violation | Shrunk script | Status |`) — the runner prints the row; the agent pastes it.
**Entry points:**
- `fuzz_smoke` (`#[gpui::test]`, runs in `cargo test --workspace`): 3 fixed seeds × 300 steps. Must stay under ~10 s total.
- `fuzz_long` (`#[ignore]`): seed and steps from `CROW_FUZZ_SEED` / `CROW_FUZZ_STEPS` / `CROW_FUZZ_MINUTES`; prints a new seed per run when none is given. `cargo test -p crow-app fuzz_long -- --ignored --nocapture`.
- `.github/workflows/fuzz.yml` (if index Q2 says yes): nightly, `CROW_FUZZ_MINUTES=20`, failure output into the job summary.

### I5 First run
Run `fuzz_long` for at least 30 minutes over several seeds. Every distinct violation goes into `findings.md` with its shrunk script. **Don't fix findings in H1** unless the fix is a one-liner in the fuzzer's own code or a false positive in an invariant; real bugs go to H4. If `fuzz_smoke` fails on main because of a real bug, mark that seed `#[ignore = "H4 #n"]` and ASK.

## Finish
- `cargo fmt --check` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo test --workspace` (time `fuzz_smoke`; record it).
- No bench needed unless I1 touched a render path (it shouldn't).
- `docs/testing.md`: a "Fuzzing" section (how to run, replay, read a finding). README: one line under testing.
- `./scripts/bundle.sh`; hand checks below.
- Commit, push `m7-h1-fuzzer`, progress file "Ready for critique".

## Hand checks
| Area | Check |
| --- | --- |
| Effects | In the bundle: Return on a file opens it; Space Quick Looks; Show in Finder reveals; Copy Path pastes; the context menu opens and its items work; sounds play (if on). Same as before H1. |
| Fuzzer | `cargo test -p crow-app fuzz_long -- --ignored --nocapture` with `CROW_FUZZ_MINUTES=2` launches no app, plays no sound, leaves the user's Trash and pasteboard alone. |

## Questions to ASK
1. **Shrinker: `proptest` or hand-rolled?** Recommended: hand-rolled delta debugging (≈ 100 lines). Steps depend on the UI state they run in, which `proptest`'s value-tree shrinking handles poorly, and it avoids a dependency. Use `rand` + `rand_chacha` (seedable, stable across platforms) for the RNG; add them as dev-dependencies.
2. **`#[cfg(test)]` only, or a `fuzz` cargo feature?** H3 needs the generator and invariants in a release binary. Recommended: a `fuzz` feature on `crow-app`, enabled in dev-dependencies for tests and by `--features fuzz` for H3; never in the bundle.
3. **Multiple windows and tabs in the fuzzer?** Recommended: tabs yes (cap 4), a second window yes (cap 2), checked by the same invariants per window.
4. **Should invariant 6 (listing = disk) also cover search results?** Recommended: no; search is a snapshot by design. Check only that every result path existed when found (skip) and that opening a vanished result shows an error, not a panic.
5. **Move `window.rs`'s test module into `window/tests.rs` (or `tests/`) while adding `fuzz.rs`?** Recommended: yes, a pure move commit first (no edits), since `window.rs` is 9.4k lines; ASK because it churns blame.
