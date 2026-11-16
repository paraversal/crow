# M7 H6 — Editable path bar progress
Branch: m7-h6-path-bar · PR: none · Job: m7-h6-path-bar-6b6b · Last updated: 2026-11-16 · State: Ready for critique

## Status
| Item | State | Notes |
| --- | --- | --- |
| Q1 build it or ⌘K is the editor | done | User: build it |
| Q2 key for edit mode | done | User: ⌘L; no clash in `actions.rs`/`text_input.rs` |
| Q3 single or double click on empty path space | done | User: double-click |
| I1 Edit mode | done | ⌘L/double-click, field, ⇥ completion, Return, refusal, Esc/blur; tests added |
| I2 Accessibility and layout | done | Role::TextInput + aria "Path"; path slot keeps its height (test) |
| I3 Tests | done | 5 path-bar tests + palette command test pass |

## Questions
- Q1 (asked 2026-11-16): Build the editable path bar, or close UI-overhaul Q10 as "⌘K / ⇧⌘G is the path editor"? → answer: build it (user).
- Q2 (asked 2026-11-16): Which key enters edit mode, ⌘L or ⇧⌘G? → answer: ⌘L (user).
- Q3 (asked 2026-11-16): Double-click empty path space to edit, or single click? → answer: double-click (user).

## Decisions
- Branch from `main` (H5 and H7 merged) — plan — plan.
- ⌘L is also a menu item (Go ▸ Edit Path) and a palette command, so the feature is reachable by key, menu and palette (AGENTS.md principle 5) — code convention.
- The field edits the active pane's folder path, not the selected search result's path; Return navigates the pane, which closes search — code convention (the bar's path slot already follows the cursor, but a path editor edits a folder).
- Escape restores focus to where it was; a click elsewhere and focus loss leave focus where the user put it — plan.
- The refusal message uses the same wording as a failed load: `Can't open “<typed>”: <io error>`, and it is cleared when the field closes if nothing newer replaced it — code convention.
- No bench: the change is in the bottom bar only; no listing, scrolling or first paint path — plan.

## Hand checks
- [x] ⌘L opens the field on the folder's path, all selected, with hints `↩ go · ⇥ complete · esc cancel`; the bar's height does not change (bundle, dark theme, `file-crow`).
- [x] Typing a path and Tab completes it from disk (`~/Files/projects/file-cr` → `…/file-crow/`).
- [x] Return closes the field and keeps the folder; a missing path beeps (code path covered by the test), keeps the field and shows `Can't open “…”` in the bar; Escape closes the field and clears that error.
- [ ] Double-click on empty path space in the running app (the keystroke test covers it; AppleScript's synthetic clicks missed the target).
- [ ] Dark and Solarized Light themes side by side; a very long path (the field scrolls); a 255-byte name; a 640 px window.
- [ ] VoiceOver reads the field as "Path" and the bar's hints.

## Found along the way
- Focus-out listeners only fire for an active window in GPUI's test platform; the click-away test activates the window first, as the app's window is active.

## Log
- 2026-11-16: Read plan, AGENTS.md, testing docs, bottom bar, text input, palette completion, `reveal_typed_path`. Created branch and progress file. Asked Q1–Q3; user chose build it, ⌘L, double-click.
- 2026-11-16: Added `EditPath`, ⌘L binding, `KeyScope::PathBar`, palette/menu command, `PathEdit` state, field rendering, completion reuse, Return/Esc/blur behavior, hints, layout fit. Five path-bar tests and a palette test pass.
- 2026-11-16: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` all pass (429 crow-app tests, including `fuzz_smoke`). Built `target/bundle/Crow.app` and drove ⌘L, typing, Tab, Return, a missing path and Escape in it.

## Next
Commit locally; stop at Ready for critique (AGENTS.md: the critic opens the PR).
