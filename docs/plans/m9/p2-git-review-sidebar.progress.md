# M9 — Git review and resizable side panes progress

Branch `codex/m9-git-review-sidebar` · PR none · Base `90e7456` (P2 PR #67) · 2026-11-19 · State: in progress

| Item | State | Notes |
| --- | --- | --- |
| 1 Resize side panes | done | Drag either divider, double-click to reset, or use the focused divider’s arrows and Return. List/Search share the right pane width. Columns retains its automatic fill until resized. GPUI tests drive both handles. |
| 2 Git details on favorite roots | done | Branch, last-fetched upstream divergence, changed count and conflicts. Wider sidebars request bounded +/− totals using the existing file diff cache. Tested repository roots and alias subfolders. |
| 3 Review Git Changes | done | File menu and palette command, pinned repository and ordinary folder badges, keyboard scope and navigation, staged/unstaged/untracked grouping, disabled deleted paths. |
| 4 Git diff in the right pane | done | Git Diff tab with bounded staged and unstaged text hunks. The Git oracle checks separate index/worktree hunks. Worker output is tied to the active file and snapshot. |
| 5 List column picker | in progress | Keep configuration edits comment-preserving. |

## Decisions

- The user requested a separate P2 PR first. Follow-up work begins at its published head in a separate worktree.
- Keep the read-only P2 Git contract. Report the existing local tracking refs for remote status.

## Next

Add the List column picker and verify end-to-end configuration writes.
