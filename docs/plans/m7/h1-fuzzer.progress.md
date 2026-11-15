# H1 action fuzzer progress
Branch: `m7-h1-fuzzer` · PR: none · Last updated: 2026-11-15 · State: In critique

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 Side-effect seam (`effects`) | done (`4f8f83e`) | `crates/crow-app/src/effects.rs`: `Effects` (`Send + Sync`), `SystemEffects`, `RecordingEffects`, `EffectHost` global. Every escaping call site routed. `main.rs` installs `SystemEffects`; `ui::init_for_tests` installs `RecordingEffects`. Done-when grep hits only `effects.rs`. 10 effect assertions added/extended. |
| I2 Steps and generator | done | `crates/crow-app/src/fuzz.rs`: `Step`, `StepGen`, `GenState`, `state()` (live keymap + context stack + selectors). `CrowWindow`/`Pane`/`Sidebar`/`SearchView` grew `cfg(any(test, feature = "fuzz"))` selector and text-focus accessors. Caps: 4 tabs, 2 windows; `cmd-q` and last-tab `cmd-w` never generated. Tests: same seed same steps, every generated keystroke parses, forbidden keys never generated, live window yields bindings/selectors. |
| I3 Invariants | done | `CrowWindow::check_invariants(&self, window, cx, deep, root)`: cursor/selection rows, focus inside the top overlay (or pane/sidebar/text field), palette/picker exclusivity, rename state, and (deep) listing = `read_dir` honoring hidden files. `fuzz::Violation`; `fuzz::logs` counts error-level lines per thread. Deliberate-break tests for cursor, selection, listing, stray focus, overlay focus, and the log hook. |
| I4 Runner, shrinking, replay | done | Runner in `window/tests.rs` (needs the test helpers); `fuzz::{Script, Failure, script!}`, delta-debugging shrinker, `fuzz_smoke` / `fuzz_long`, `.github/workflows/fuzz.yml` every 3 days. Test-platform exclusions: `cmd-m` (no minimize) and `ctrl-cmd-space` (no character palette). Deep listing check only after `Settle`/at the end, only for the active tab and only under the fuzz tree; `fuzz::settle` advances the test clock and waits for two quiet turns. Each seed/shrink trial resets the tree, writes a tree-only sidebar, and closes every window before opening a fresh one. Replay skips `cmd-w` on a one-tab window. |
| I5 First run | done | `fuzz_smoke` seeds 1–3 × 30 steps on a 40-file tree: 3.56 s. Findings #1–#3 fixed in H1 (user approved), #4 open for H4. First long run: 36.8 min, 6 seeds × 3000 steps, no violation (the shell backend killed the process at its 40-min cap, not the fuzzer). A second run added 5 seeds × 1000 steps, no violation, before the shell backend timed out. |

## Questions
- Q9 (critique, asked 2026-11-15): should fuzzed right-clicks pick a random menu item (so Trash, Duplicate, Open… get fuzzed) instead of always dismissing? → **yes**; `RightClick(selector, Option<slot>)`, `363a8df`.
- Q10 (critique, asked 2026-11-15): keep the 25-key `NoAction` list in `Sheet`, add a guard test, or replace it with one root fix? → **keep it and add a guard test**; `sheets_block_every_window_key` (`d5ba74d`), which fails when the list is missing a key (checked by deleting the ⌘K block).
- Q11 (critique, asked 2026-11-15): the M7 index says H1 covers the Settings window; do it in H1 or move it to H2/H7? → **move to H7**; index and `h7-accessibility.md` I2b updated (`e2179e1`).
- Q12 (critique, asked 2026-11-15): confine the fuzzer's file operations to its tree (a guard in `operations`, and `Ui.home` = the tree in fuzz runs)? → pending
- Q8 (asked 2026-11-15): `fuzz_smoke` seed 1 finds a real focus bug (theme picker stays open, Tab moves focus behind it). Fix in H1 or record for H4 and ignore seed 1? → answer: **fix now**. Fixed in H1 with `tab_stays_in_the_theme_picker`; the same modal-focus bug in the Why sheet (arrows/Tab) got `why_sheet_keeps_focus_on_arrows_and_tab`.
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
- `cmd-m` (Minimize) and `ctrl-cmd-space` (Show Character Palette) are excluded: GPUI's test platform panics with "not implemented". They are real app actions, covered by hand checks.
- The deep listing check runs only after `Step::Settle` and once at the end of a run; checking it after every step false-positived on an in-flight trash whose watcher had not delivered yet. `fuzz::settle` advances the test clock (the folder poll is a 100 ms `background_executor` timer) and waits for two quiet turns.
- The deep listing check only covers the active tab (only it is shown) and folders under the fuzz tree. Inactive tabs may lag by design; real directories outside the tree are out of H1's scope.
- The fuzzer writes a tree-only `sidebar.toml` (favorites `alpha`/`beta`, no Recent/Locations) so a click cannot navigate to the user's real folders. Typed paths can still leave the tree; the listing check simply skips them.
- Search open allows the sidebar focus: the documented Tab cycle is field → results → sidebar → field. This means H4 #4 (right-click on the sidebar while search is open) is not distinguishable by the invariant; it is recorded for H4.
- `fuzz_smoke` uses 30 steps per seed on a 40-file tree, not the plan's 300 steps: 300 steps took 33 s (real rendering plus occasional 4–6 s undo/relayout steps), over the plan's ~10 s budget. The 3-seed smoke now takes 3.74 s; `fuzz_long` keeps 3000 steps on the 200-file tree.
- `fuzz::settle` advances the test clock because the folder poll is a 100 ms `background_executor` timer, not real time. This was the cause of the early listing false positives.
- The first long run found one invariant false positive: an inline child row (`alpha/inner`) was selected but not in `entries`. The selection check now uses `row_paths()` (own and inline child rows); test `inline_child_selection_is_in_the_listing`.
- The repo's `AGENTS.md` and the project `execute-plan` skill say the executing agent commits locally and stops at "Ready for critique"; the critique agent opens the PR. The plan's Finish says "push `m7-h1-fuzzer`"; the repo rule wins, so this branch is not pushed here.
- Invariant 3 checks that the focused handle is one of the known focus-owning surfaces (pane, sidebar, pane text field, or the top overlay). GPUI's test platform disables the accessibility tree (`TestAppContext::new` sets `accessibility_force_disabled`; the test platform's `a11y_init` is a no-op), so role/aria-label presence cannot be read generically; the existing per-surface tests cover it. H7 can strengthen this when it enables a11y.
- Invariant 5's "one overlay of each kind" is structural (`Option` fields); the meaningful checks are palette/picker exclusivity and rename state vs `rename_text`.
- Invariant 6 compares the full `entries` listing to `read_dir` (hidden setting honored). The name filter is a view over `entries` and has its own tests; replicating its matcher here would duplicate it.

## Hand checks
- [ ] Effects, in the bundle: Return on a file opens it; Space Quick Looks; Show in Finder reveals; Copy Path pastes; the context menu opens and its items work; sounds play (if on). Same as before H1.
- [ ] Sheets (critique): with the theme picker (⌥⌘T), the Why sheet (⌘J) and a conflict sheet open, Tab, the arrows, ⌘K, ⌘T, ⌘W and ⌘[ do nothing behind the sheet; Return/Esc still confirm/cancel; the picker's ↑/↓ still move.
- [ ] Fuzzer: `CROW_FUZZ_MINUTES=2 cargo test -p crow-app fuzz_long -- --ignored --nocapture` launches no app, plays no sound, leaves the user's Trash and pasteboard alone.

## Found along the way
- **H4 #4** (open): `cmd-shift-a` opens search; right-clicking a sidebar row moves focus to the sidebar while search stays open. Shrunk: `crate::script![Step::Keys("cmd-shift-a".into()), Step::RightClick("sidebar-5".into(), None)]`. Not fixed in H1; needs a product decision (close search on sidebar click vs keep focus in the field).
- **H1 finding #1** (fixed here, user approved): `alt-cmd-t` opens the theme picker; `tab` (`SwitchFocus` in the `Crow` context) moved focus behind it. `ThemePicker` now binds Tab/Shift-Tab to `NoAction`; test `tab_stays_in_the_theme_picker`.
- **H1 finding #2** (fixed here): the Why sheet let arrows/Tab move focus behind it. `Sheet` now binds Tab/Shift-Tab/arrows to `NoAction`; the picker's later up/down bindings still win. Test `why_sheet_keeps_focus_on_arrows_and_tab`.
- The first shrink of finding #2 was invalid (`right`, `down`) because `close_all_windows` left the old window alive; it now loops until `cx.windows()` is empty. The corrected script is `cmd-j`, `right`.
- `bottom_bar::open_config` had a `#[cfg(test)]` recorder (`OPENED`); replaced by the global seam.
- The Why sheet's buttons had ids but no debug selectors; added them.
- The Settings window is a separate `SettingsWindow` entity; the fuzzer opens it with ⌘, but does not drive or check it. Its own tests cover it; H1 leaves it there.

## Critique
Critic session: 2026-11-15 · Base: main @ 50dd5a5 · Head reviewed: 7fa42be

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | The `fuzz` feature (Q2, for H3's `crow --fuzz`) did not compile outside tests: the widened `Pane` impl called test-only `Folder::names`/`selected_names`, the focus invariant called test-only search focus handles | `pane.rs:3087`, `search.rs:781` | rules / code | fix | fixed (`34b6b39`); dead code allowed under the feature until H3 adds the entry point |
| C2 | The shrinker counted any failure or panic as "still fails", so a shrink could slip to a different bug | `window/tests.rs` `shrink` | code | fix | fixed (`25bf5a6`): same check only |
| C3 | Under the fuzzer a right-click never closed its menu: `RecordingEffects` answers only when scripted, the runner never scripted, so `Pane::context_menu`/`Sidebar::menu` stayed set (the sidebar row keeps its outline) — a state the app can't reach | `effects.rs` `show_context_menu`, `sidebar.rs:992` | code | fix | fixed (`25bf5a6`); then Q9 → each right-click picks a slot (`363a8df`) |
| C4 | Rename invariant compared `is_renaming()` with `rename_text().is_some()`, both reads of `self.renaming`; it could never fire | `window.rs` `check_invariants` | code | fix | fixed (`25bf5a6`): removed, documented as structural |
| C5 | A failure printed only after shrinking (up to 300 replays of up to 3000 steps); the CI job's 45-min timeout could kill it before the summary got the failing step | `fuzz_run` | code | fix | fixed (`25bf5a6`): printed before shrinking |
| C6 | findings #4's shrunk script names `sidebar-5`, which the fuzz sidebar (2 favorites) doesn't have; replaying it finds nothing | `findings.md` | record | fix | row annotated (`25bf5a6`) |
| C7 | 25 `NoAction` bindings in `Sheet` mirror every `Crow`-context key; complete today (only `cmd-m` and Esc differ) but any new `Crow` binding leaks behind sheets again, and it also blocks ⌘N/⌘, while a sheet is open. Found and fixed beyond Q8's scope (Q8 approved the picker Tab fix) | `actions.rs:149-156, 227-254` | code sense / UX | ask | Q10 → kept the list, added `sheets_block_every_window_key` (`d5ba74d`) |
| C8 | The M7 index says H1 "covers the browser windows and the Settings window"; H1 doesn't drive or check the Settings window | `m7/README.md` table, Found along the way | plan | ask | Q11 → moved to H7 I2b (`e2179e1`) |
| C9 | `cmd-n` opens a second window on the current folder (the tree); the runner only drives the first window (`VisualTestContext` targets it), so the second is opened and checked but never fuzzed. Documented in `testing.md` | runner | plan | note | left: Q3 met literally; H2/H3 can drive both |
| C10 | `fuzz::logs` and `fuzz::settle` are `cfg(test)`; H3's binary will need its own settle and log hook, and `listing_violations` reads disk on the calling thread (main thread in H3) | `fuzz.rs`, `pane.rs` | code | note | left for H3 |
| C11 | Overlay exclusivity (palette + picker) has no deliberate-break test, unlike the other checks (plan I3 asks for one per check) | `window/tests.rs` | plan | note | left: needs a test-only way to open both; small H4 item |
| C12 | `check_all_windows` retries a listing mismatch with real `sleep`s (25 × 20 ms); bounded, so a stuck listing still fires, but not recorded as a workaround | `window/tests.rs` | rules | note | left: FSEvents runs on real time |
| C14 | Nothing confines the fuzzer's file operations to its tree. Today they stay there because `keep_in_tree` pulls the pane back after each step and search's Home scope segment has no fuzz selector. But `Ui.home` is the real home, so a future selector (`search-scope-home`) would let ⌘⌫ / ⌘⌥⌫ on a search result move or delete a real home file | `ui.rs` `init_for_tests`, `search.rs:372`, `keep_in_tree` | code / safety | ask | Q12 |
| C13 | `fuzz_smoke` runs 3 × 30 steps, not the plan's 300 (33 s vs ~10 s budget); recorded as a Decision without asking | Decisions | plan | note | left; the suite now takes ~20 s for crow-app, so 300 steps would double it |

Checks re-run: fmt ✓ · clippy ✓ (also `--features fuzz` ✓ after C1) · tests ✓ (crow-app 392 + 1 ignored; workspace all green) · `fuzz_long` after the fixes: 2 min, 2 seeds × 1000 steps, no violation · no render or listing path changed (effects calls only)
Looked at in the app: not driven. The user-visible changes are the sheet key blocks (covered by `tab_stays_in_the_theme_picker`, `why_sheet_keeps_focus_on_arrows_and_tab`) and the effects seam (unchanged behavior, needs a human: Quick Look, open, sounds, Finder). Both are in Hand checks.
Verdict: pending answers to Q9–Q11.

## Log
- 2026-11-15: read the plan, M7 index, testing docs, repo skill; surveyed every `Command::new`/`crow_macos::` call site; created the branch and this file.
- 2026-11-15: I1. Added `effects.rs`, the `fuzz` feature, installed `SystemEffects` in `main` and `RecordingEffects` in `init_for_tests`, routed every escaping call. `cargo test -p crow-app` 377 passed; clippy and fmt clean.
- 2026-11-15: moved `window.rs`'s tests into `window/tests.rs` (pure move; `include_str!` path and indentation only). 377 tests still pass.
- 2026-11-15: I2. Added `fuzz.rs` and the selector/text-focus accessors; generator tests pass; clippy clean.
- 2026-11-15: I3. Added `check_invariants`, pane/folder invariant helpers, the error-log hook, and 7 deliberate-break tests; clippy clean.
- 2026-11-15: I4. Added the runner, shrinker, replay, `fuzz_smoke`/`fuzz_long`, the every-three-days workflow, the hermetic tree/sidebar/clock handling, and the docs. Fixed the three modal focus leaks the fuzzer found (picker Tab, Why arrows/Tab, sheet window keys).
- 2026-11-15: I5. `fuzz_smoke` 3.56 s; 36.8-minute long run over 6 seeds (3000 steps each) with no violation; a second run added 5 seeds (1000 steps) with no violation. One invariant false positive (inline child selection) fixed with `row_paths()` and a regression test.
- 2026-11-15: Finish. `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` pass; `./scripts/bundle.sh` builds `target/bundle/Crow.app`.

## Next
Critique the branch, then open the PR (the repo's critique step).
