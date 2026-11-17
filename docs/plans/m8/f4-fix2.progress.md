# F4 fix 2 progress

Branch: `m8-f4-path-bar` (F4, unmerged) · PR: none · Last updated: 2026-11-17 · State: Awaiting approval

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 Hover clears on window exit | planned | Q1: the GPUI patch vs the bar's own state |
| 2 The pill fades, the field expands | planned | Q2: the durations/easing |
| 3 The "Tab complete" hint | planned | Q3: remove vs implement |

## Questions (user)

- **Q1:** the hover fix — patch GPUI's `MouseExited` to move the mouse position out and clear the hit test (all hovers; a vendored patch) or render the pill from a local `on_hover` bool (the pill only)?
- **Q2:** the animations — 120 ms pill fade, 160 ms field expand, `ease_out`, skipped with motion Off?
- **Q3:** the "Tab complete" hint — the completion exists; the field opens with the full path selected. The user chose the inline completion preview (the Finder ⌘⇧G pattern).

## Answers (user, 2026-11-17)

- **Q1:** A, the GPUI patch.
- **Q2:** A, 120/160 ms, `ease_out`, instant with motion Off.
- **Q3:** C, the inline completion preview: the muted suffix, Tab applies, no suffix when nothing completes, the hint stays.

## Log

- 2026-11-17: the user tested the V2: the hover sticks when the pointer leaves the window; no animations; the "Tab complete" hint looks dead. Plan written; awaiting approval.
- 2026-11-17: corrected the plan: the completion exists (`text_input.rs:96` → `complete_path_edit`); the field opens with the full path selected, so Tab returns the same text. The hover patch must move `mouse_position` out, not just clear the hit test.
- 2026-11-17: the user answered Q1 A, Q2 A, Q3 C (the inline completion preview); the plan amended; awaiting the OK to build.
- 2026-11-17: item 3 spells out the case-insensitive suffix (`~/des` → `~/Desktop/`, no `strip_prefix`) and the caret-at-end gate, with tests.

## Next

The user's OK on the amended plan (Q3 C); then build the fix list and stop for the test.
