# m5-fix progress
Branch: m5-fix (based on `m5-sidebar-settings-themes`; PR #5 still open) · PR: none · Last updated: 2026-11-12

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 Quick Look crashes | in progress | Cause identified: QL panel spins a nested run loop inside a GPUI `App` borrow; the 100 ms poll task re-enters `App::spawn` → `AppCell::borrow_mut` panic → SIGABRT. Fix direction: never call QL panel APIs while `App` is borrowed + give the panel a controller. |
| I2 Path bar segments don't navigate | todo | Cause: `render_path_bar` has no click handlers at all. |
| I3 Column view Left at leftmost column | todo | Cause: `pane.rs` `move_left` returns when `focused == 0`. |
| I4 List view expandable folders | todo | Last item; largest. |
| I5 Favorite drag-down reorder | todo | Not diagnosed yet. |
| I6 Sidebar row flicker during folder drag | todo | Not diagnosed yet. |

## Questions
- Q1 (asked 2026-11-12, batched): branch base; I2 last-segment click; I3 at `/` and view-mode persistence; I4 key table, persistence, extras. → answers (from the user):
  - Branch base: branch off `m5-sidebar-settings-themes`, PR stacked on that branch.
  - I2: clicking a file segment opens the file; clicking a folder segment navigates as a new history step; if already there, no-op.
  - I3: Left on the leftmost column navigates one folder up — pretend `n-1` was the leftmost column and push it in from the left. At `/` nothing happens. Column view stays on regardless of the parent's `views.toml` mode.
  - I4: Finder-style keys (file: Left = up one directory, Right = nothing; nested row: Left = select parent row, then Left again collapses). Expansion is session/window scoped, collapses on navigation away. Option-click a triangle expands all descendants (no Option-arrow). Vim `h`/`l` follow the table. Selection may span levels and operations act on all selected rows.

## Decisions
- Branch `m5-fix` from `m5-sidebar-settings-themes`, not `main`, because M5 is unmerged (#5) and every fix touches M5 code — user decided.
- Progress file committed before any code — execute-plan skill.

## Log
- 2026-11-12: Branch created, progress file written. Investigated I1 crash reports (`~/Library/Logs/DiagnosticReports/crow-2026-11-12-174339.ips`, `crow-2026-11-12-171729.ips`): SIGABRT; panic originates in `AppCell::borrow_mut` (`crates/../vendor/zed/crates/gpui/src/app.rs:155-156`) called from `App::spawn`; unified log at the same second prints `-[QLPreviewPanel <private>] called while the panel has no controller.` twice. QL plugin loading runs a nested run loop inside the action handler; the `ui::init` 100 ms poll task then re-enters GPUI while `App` is already borrowed.

## Next
I1: reproduce from the terminal (stderr shows the panic message), then implement: QL calls dispatched outside the GPUI `App` borrow, and a `QLPreviewPanelController` so the panel has a controller.
