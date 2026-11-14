# context-menu progress
Branch: context-menu · PR: none · Last updated: 2026-11-14 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| C1 Click on empty space deselects | blocked (Q4 for column view) | |
| C2 Context menu model | blocked (Q2) | |
| C3 Opening the menu | blocked (Q1) | |
| C4 New commands | blocked (Q2) | |
| C5 Scope and Vim | blocked (Q3) | |

## Questions
- Q1 (asked 2026-11-14): native NSMenu or GPUI-drawn menu? → open
- Q2 (asked 2026-11-14): which items; add Copy Path / Show in Finder? → open
- Q3 (asked 2026-11-14): sidebar and search results in scope? keyboard opening? → open
- Q4 (asked 2026-11-14): column view empty click closes columns to the right? → open

## Decisions
- Worktree at `target/wt-context-menu` (vendor symlinked) because the main checkout is on `housekeeping` with uncommitted work from others — agent.

## Hand checks

## Found along the way

## Log
- 2026-11-14 Plan written from a read of pane.rs, folder.rs, actions.rs, palette.rs, crow-macos.

## Next
Wait for answers to Q1–Q4.
