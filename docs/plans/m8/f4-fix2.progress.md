# F4 fix 2 progress

Branch: `m8-f4-path-bar` (F4, unmerged) · PR: none · Last updated: 2026-11-17 · State: Awaiting approval

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 Hover clears on window exit | planned | Q1: the GPUI patch vs the bar's own state |
| 2 The pill fades, the field expands | planned | Q2: the durations/easing |
| 3 The "Tab complete" hint | planned | Q3: remove vs implement |

## Questions (user)

- **Q1:** the hover fix — patch GPUI's `MouseExited` to clear the hit test (all hovers; a vendored patch) or the bar's own hover state (the pill only)?
- **Q2:** the animations — 120 ms pill fade, 160 ms field expand, `ease_out`, skipped with motion Off?
- **Q3:** the "Tab complete" hint — remove the hint and the dead `Complete` action, or implement completion?

## Log

- 2026-11-17: the user tested the V2: the hover sticks when the pointer leaves the window; no animations; the "Tab complete" hint is wrong. Plan written; awaiting approval.

## Next

The user's answers to Q1–Q3; then build the fix list and stop for the test.
