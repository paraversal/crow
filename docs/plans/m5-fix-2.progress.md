# m5-fix-2 progress
Branch: m5-fix · PR: https://github.com/paraversal/file-crow/pull/6 · Last updated: 2026-11-12

## Status
| Item | State | Notes |
| --- | --- | --- |
| D1 Drag feedback: flicker while moving, laggy drag image | in progress | Split from reordering (D2). Measure first: event rate, frames, target changes, main-thread time. |
| D2 One insertion target between sidebar items | todo | Confirmed: “above B” and “below A” are two targets; group-title drops stay. |
| D3 Larger chevron disclosure | todo | 16 pt semibold `›`/`⌄`, 20 px gutter, larger hit area. |
| D4 Remove Option-click expand-all | todo | Delete `expand_all` cascade; Option-click = normal expand. |

## Questions
- Q1 (asked 2026-11-12): m5-fix-2 questions (flicker symptom, origin, which image, drag mechanism, Option-click, group-title drops, branch). → answers (from the user):
  - Sidebar reordering flicker = the double “between A and B” target; the bar jumps with fine movement. The folder/file drag highlight is a separate rendering problem: the correct target is chosen, but its highlight blinks while the pointer moves (steady while resting), for both Crow→Crow and Finder→Crow.
  - Both the file icon from Crow’s list and the favorite label lag.
  - Approved: switch in-app dragging to GPUI’s internal drag with native promotion when leaving the window.
  - Option-click after removing expand-all = normal expand/collapse, like a click.
  - Keep dropping a favorite on a group title (moves it to that group’s start), with one line under the title.
  - Continue on `m5-fix`, update PR #6.
- Q2 (asked 2026-11-12): explain the flicker and group-title questions. → answers as above.
- Out of scope (future, user): dropping an item on the sidebar should choose between “add to favorites” and “move into that folder” by cursor position, with Finder-like “+” tooltip feedback.

## Decisions
- Two separate drag issues: D2 is the favorite-insertion model; D1 is rendering/feedback of would-be targets. Do not conflate them — user.
- Same branch and PR — user.
- In-app drag image: use GPUI’s internal drag and promote to a native file drag when the pointer leaves the window — user approved if per-update work alone does not fix the lag.
- No vendored code changes unless measured to be required; prefer app-side work.

## Log
- 2026-11-12: Plan `docs/plans/m5-fix-2.md` written and approved (lgtm), answers recorded. Progress file started.

## Next
D1 measure: instrument FileDrop event rate, frame times, target changes and main-thread time during a Crow→Crow and a Finder→Crow drag; record the cause before changing code.
