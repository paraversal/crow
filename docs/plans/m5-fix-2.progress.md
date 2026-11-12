# m5-fix-2 progress
Branch: m5-fix · PR: https://github.com/paraversal/file-crow/pull/6 · Last updated: 2026-11-12

## Status
| Item | State | Notes |
| --- | --- | --- |
| D1 Drag feedback: flicker while moving, laggy drag image | done (this commit) | Cause: file drags started an `NSDraggingSession` at the threshold, so every update returned as `FileDropEvent` → synthetic MouseMove and the highlight was recomputed per native event; the in-window image was AppKit’s. Fix: list and column rows use GPUI `on_drag` with a shared `DraggedSelection` (Arc of paths + folder flags) and `external_drag_payload`, so the in-window image is GPUI’s and a native session starts only when the pointer leaves. Deleted `crow-macos/src/drag.rs` and the `raw-window-handle` dependency. Verified: a recorded GPUI drag kept the target row lit for 423 consecutive frames with zero toggles (the native path toggled ~100 times in 3 s); `dragging_a_row_onto_a_folder_moves_it`, `dragging_a_row_onto_a_sidebar_favorite_moves_it`, all existing drop tests. Finder→Crow drags still use the platform path and need a hand check. |
| D2 One insertion target between sidebar items | done (this commit) | Replaced the two per-row halves with one slot computed from the pointer against the remembered row bounds: upper half of a favorite = before it, lower half = after it, group title = that group’s start. One absolutely positioned 2 px line is drawn by the row that owns the boundary, so there is never a second line and no row changes height. Group-title drops kept. Tests: `dragging_a_favorite_down_inserts_below_the_row` now checks the single state slot below beta (0,2) and above alpha (0,0); core tests unchanged. |
| D3 Larger chevron disclosure | done (this commit) | 16 pt semibold `›`/`⌄` in a 20 px gutter with a full-row-height hit area and hover color; indent 18 px per level. Same role, aria label and `disclosure-<name>` selector. |
| D4 Remove Option-click expand-all | done (this commit) | Deleted `Folder.expand_all` and the cascade; Option-click now expands/collapses one folder like a click. The `option_clicking_a_triangle_expands_everything_below` test is gone. README/testing docs updated. |

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
- 2026-11-12: D1 measured before changing code. Native drags: `draggingUpdated` ~61 Hz, the target row’s pixels toggled ~100 times in 3 s; a screen recording showed the drag image is AppKit’s and updates only on drag events. With GPUI dragging: target steady for 423 frames, no toggles. First attempt to compute drag data per row called `selected_entries()` per frame (100k-row scan) and dropped the scroll bench to 26 fps; now the selection is generation-cached in `Folder` and shared as an `Arc`, bench back to p99 9.0 ms.
- 2026-11-12: D2–D4 implemented with the plan’s answers. `cargo fmt --check`, clippy `-D warnings`, `cargo test --workspace` clean (36 app tests). Bench: warm first paint 10 ms, scroll p99 9.01 ms at 120 Hz.
- 2026-11-12: Hand checks still needed: drag-out to Finder and Mail (native promotion), a Finder→Crow drag highlight, and the feel of the larger chevron.

## Next
PR #6 updated with the follow-up. The user hand-tests: drag-out to Finder/Mail, a Finder→Crow drag highlight, and the drag/chevron feel. Do not merge.
