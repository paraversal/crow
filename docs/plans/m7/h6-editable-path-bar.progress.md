# M7 H6 — Editable path bar progress
Branch: m7-h6-path-bar · PR: none · Job: m7-h6-path-bar-6b6b · Last updated: 2026-11-16 · State: In critique

## Status
| Item | State | Notes |
| --- | --- | --- |
| Q1 build it or ⌘K is the editor | done | User: build it |
| Q2 key for edit mode | done | User: ⌘L; no clash in `actions.rs`/`text_input.rs` |
| Q3 single or double click on empty path space | done | User: double-click |
| I1 Edit mode | done | ⌘L/double-click, field, ⇥ completion, Return, refusal, Esc/blur; starts on the shown path (C-Q1); tests added |
| I2 Accessibility and layout | done | Role::TextInput + aria "Path"; path slot keeps its height (test) |
| I3 Tests | done | 6 path-bar tests + palette and search-seed tests pass |

## Questions
- Q1 (asked 2026-11-16): Build the editable path bar, or close UI-overhaul Q10 as "⌘K / ⇧⌘G is the path editor"? → answer: build it (user).
- Q2 (asked 2026-11-16): Which key enters edit mode, ⌘L or ⇧⌘G? → answer: ⌘L (user).
- Q3 (asked 2026-11-16): Double-click empty path space to edit, or single click? → answer: double-click (user).
- C-Q1 (critique, asked 2026-11-16): Should ⌘L seed the path the bar shows (the cursor entry, or the selected search result) or the pane's folder? The bar follows the cursor, so with `.git` selected it showed `…/file-crow/.git` while the field opened on `~/Files/projects/file-crow`. → answer: the path the bar shows (user).

## Decisions
- Branch from `main` (H5 and H7 merged) — plan — plan.
- ⌘L is also a menu item (Go ▸ Edit Path) and a palette command, so the feature is reachable by key, menu and palette (AGENTS.md principle 5) — code convention.
- The field edits the path the bar shows: the selected search result, else the cursor entry, else the folder. Return navigates a folder or reveals a file; during search it closes search — user, critique C-Q1 (2026-11-16).
- Escape restores focus to where it was; a click elsewhere and focus loss leave focus where the user put it — plan.
- The refusal message uses the same wording as a failed load: `Can't open “<typed>”: <io error>`, and it is cleared when the field closes if nothing newer replaced it — code convention.
- No bench: the change is in the bottom bar only; no listing, scrolling or first paint path — plan.

## Hand checks
- [x] ⌘L opens the field on the path the bar shows (the cursor's, or the selected search result's), all selected, with hints `↩ go · ⇥ complete · esc cancel`; the bar's height does not change (critic, bundle, dark theme: the bar showed `…/file-crow/.git`, the field `~/Files/projects/file-crow/.git` (`h6-edit-shown.png`); search `notes` selected `notes.txt`, the field `/private/tmp/crow-h6-tree/notes.txt` (`h6-search-seed.png`)).
- [x] Typing a path and Tab completes it from disk (`~/Files/projects/file-cr` → `…/file-crow/`).
- [x] Return closes the field and keeps the folder; a missing path beeps (code path covered by the test), keeps the field and shows `Can't open “…”` in the bar; Escape closes the field and clears that error.
- [ ] Double-click on empty path space in the running app (the keystroke test covers it; AppleScript's synthetic clicks missed the target; the critic retried with the window activated and the two clicks still did not register as a double-click).
- [x] Dark and Solarized Light themes side by side; a very long path (the field scrolls); a 255-byte name; a 640 px window (critic, bundle shots: `target/shots/h6-edit-dark3.png`, `h6-light.png`, `h6-longpath.png`, `h6-255.png`, `h6-640.png`).
- [x] VoiceOver reads the field as "Path" and the bar's hints: the AX tree exposes `AXTextField`, `AXTitle = Path`, `AXValue = <path>`, `AXFocused = 1`, and the parent group `AXGroup`/`AXTitle = Edit path` (critic, AX probe). VoiceOver's speech itself was not heard.

## Found along the way
- Focus-out listeners only fire for an active window in GPUI's test platform; the click-away test activates the window first, as the app's window is active.

## Log
- 2026-11-16: Read plan, AGENTS.md, testing docs, bottom bar, text input, palette completion, `reveal_typed_path`. Created branch and progress file. Asked Q1–Q3; user chose build it, ⌘L, double-click.
- 2026-11-16: Added `EditPath`, ⌘L binding, `KeyScope::PathBar`, palette/menu command, `PathEdit` state, field rendering, completion reuse, Return/Esc/blur behavior, hints, layout fit. Five path-bar tests and a palette test pass.
- 2026-11-16: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` all pass (429 crow-app tests, including `fuzz_smoke`). Built `target/bundle/Crow.app` and drove ⌘L, typing, Tab, Return, a missing path and Escape in it.
- 2026-11-16 (critique): Read the plan, progress and diff; re-ran fmt, clippy, the workspace tests and `fuzz_smoke`. Found the fuzzer's focus invariant and `text_focused` missing the path field; fixed in dc3358 with `path_edit_keeps_the_focus_invariant`.
- 2026-11-16 (critique): Drove the bundle in FireBox dark and Solarized Light, 640×500 and 1280×800, a 288-byte path and a 255-byte name; Tab, Return, refusal, Escape; read the AX tree. The synthetic double-click could not be driven; the test covers it.
- 2026-11-16 (critique): C-Q1 answered "the path the bar shows"; `bar_target` now feeds `bar_state` and `edit_path`, with the folder, file and search-result seeds tested; README and the Decision updated (d514243).

## Critique
Job: review-h6-path-bar-7516 · Critic session: 2026-11-16 · Base: main @ d20dc9f · Head reviewed: a71dee9

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | The fuzzer's focus invariant rejected the path field (`focus is outside the pane surface`), and `text_focused` missed it, so a long run would fail as soon as ⌘L appeared and never type into the field | `crates/crow-app/src/window.rs:3449` (`check_invariants`), `:3359` (`text_focused`) | rules (H1 fuzzer) / code | fix | fixed (dc3358, `path_edit_keeps_the_focus_invariant`) |
| C2 | ⌘L seeded the pane's folder, not the path the bar shows: with `.git` selected the bar showed `…/file-crow/.git`, the field `~/Files/projects/file-crow` (screenshot `h6-edit-dark3.png`); the plan says the segments are replaced with "the full path". User chose the shown path | `crates/crow-app/src/window.rs:1113` (`bar_target`), `:541` (`edit_path`) | UX / plan | ask | C-Q1 → shown (user); fixed (d514243, `path_edit_starts_on_the_shown_search_result`; screenshots `h6-edit-shown.png`, `h6-search-seed.png`) |
| C3 | The field's wrapper adds `Role::Group` + aria "Edit path" on top of the field's own `Role::TextInput` + "Path"; VoiceOver gets an extra "Edit path, group" stop. The filter field uses the same group pattern, so it is consistent, not wrong | `crates/crow-app/src/bottom_bar.rs:600` | UI / a11y | note | left (consistent with the filter field) |
| C4 | After a refusal, editing the text leaves the old `Can't open “…”` in the bar until Return or Escape; the message describes the previous text | `crates/crow-app/src/window.rs:545` (`TextInputEvent::Changed` arm) | UX | note | left (bar messages stay until replaced; closing clears it) |
| C5 | The double-click on empty path space could not be driven: two synthetic `click at` events on the activated window did not register as a double-click; the keystroke test covers the behavior | `target/shots/h6-dblclick.png` | UX / hand check | note | left (hand check for the user) |
| C6 | The AX tree exposes the field as `AXTextField`, `AXTitle = Path`, `AXValue = <path>`, `AXFocused = 1`; the parent group is `AXGroup`, `AXTitle = Edit path`. VoiceOver speech itself was not heard | AX probe (`/tmp/crow-ax-field.swift`) | a11y | note | verified role/label; speech is a hand check |
| C7 | `PATH_FIELD_MIN`'s doc calls it a minimum, but the field is `flex_1 min_w_0` and can shrink below 180 px; the constant is only the fit's requested width | `crates/crow-app/src/bottom_bar.rs:288` | code sense | note | left (doc wording; behavior is fine) |
| C8 | The plan's I3 example "type `~/`, ⇥ completes" is covered by the general Tab test with a temp path, not by a `~` case; `complete_typed`'s `~` branch is untested | `crates/crow-app/src/window/tests.rs:4880` | plan / tests | note | left (same code path; `resolve_typed`'s `~` is tested) |

Checks re-run: `cargo fmt --check` ✓ · `cargo clippy --workspace --all-targets -- -D warnings` ✓ · `cargo test --workspace` ✓ (the full suite after both fixes; 16 path tests) · `cargo test -p crow-app fuzz_smoke` ✓ · `CROW_FUZZ_SEED=7 CROW_FUZZ_STEPS=1200 CROW_FUZZ_SHRINK_SECS=0 cargo test -p crow-app fuzz_long -- --ignored --nocapture` ✓ (108 s; run to prove C1's fix; over the 30 s guidance, noted) · bench not needed (bottom bar only; plan says so).

Looked at: bundle shots in FireBox dark and Solarized Light, 640×500 and 1280×800, a 288-byte path and a 255-byte name; ⌘L, typing, Tab completion, Return to a folder, a missing path's beep/error, Escape; the shown-path seed (`h6-edit-shown.png`) and the search-result seed (`h6-search-seed.png`); the bar's height, hints and counts; the AX tree. Could not see: the double-click (C5), VoiceOver speech (C6), the beep's sound (the test asserts the call).

Verdict: better. The field fits the bar, the interactions work, the fuzzer gap is fixed, and the field now starts on the path the bar shows (C-Q1), so entering edit mode no longer changes the path.

## Next
PR open (critic, per AGENTS.md); user hand-tests the double-click and VoiceOver speech.
