# context-menu progress
Branch: context-menu · PR: none · Last updated: 2026-11-14 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| C1 Click on empty space deselects | done | `press_empty` in pane.rs; rows set `row_pressed` so the list behind them knows. 5 GPUI mouse tests (`click_*` in window.rs) |
| C2 Context menu model | todo | |
| C3 Opening the menu | todo | |
| C4 New commands | todo | |
| C5 Scope and Vim | todo | |

## Questions
- Q1 (asked 2026-11-14): native NSMenu or GPUI-drawn menu? → answer: "Native NSMenu (Recommended)"
- Q2 (asked 2026-11-14): which items; add Copy Path / Show in Finder? → answer: "As proposed (Recommended)" (Copy Path ⌥⌘C, Show in Finder no key; no Get Info, no Open With)
- Q3 (asked 2026-11-14): sidebar and search results in scope? keyboard opening? → answer: "File list only (Recommended)" (sidebar and search results are a follow-up; no key to open the menu)
- Q4 (asked 2026-11-14): column view empty click closes columns to the right? → answer: "Same as Finder (Recommended)"

## Decisions
- C1 detects "no row" with a flag set in `Pane::press` (rows handle mouse-down before the list) instead of pointer math, so the side gutters count as empty space and the drag/focus listeners on rows are untouched — agent.
- Worktree at `target/wt-context-menu` (vendor symlinked) because the main checkout is on `housekeeping` with uncommitted work from others — agent.

## Hand checks
- [ ] Empty click: list view, select a few items, click below the last row → nothing selected, ↓ selects the first row. Column view, three columns open, click the empty part of the first column → the others close, nothing selected.

## Found along the way

## Log
- 2026-11-14 C1 done: list and column view, fmt/clippy/tests green.
- 2026-11-14 User answered Q1–Q4, all as recommended.
- 2026-11-14 Plan written from a read of pane.rs, folder.rs, actions.rs, palette.rs, crow-macos.

## Next
Spike: native NSMenu popup from a GPUI mouse handler (deferred to the main queue), check GPUI keeps running and the chosen item comes back.
