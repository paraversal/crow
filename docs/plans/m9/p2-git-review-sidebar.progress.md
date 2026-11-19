# M9 — Git review and resizable side panes progress

Branch `codex/m9-git-review-sidebar` · PR none · Base `90e7456` (P2 PR #67) · 2026-11-19 · State: in progress

| Item | State | Notes |
| --- | --- | --- |
| 1 Resize side panes | done | Drag either divider, double-click to reset, or use the focused divider’s arrows and Return. List/Search share the right pane width. Columns retains its automatic fill until resized. GPUI tests drive both handles. |
| 2 Git details on favorite roots | done | Branch, last-fetched upstream divergence, changed count and conflicts. Wider sidebars request bounded +/− totals using the existing file diff cache. Tested repository roots and alias subfolders. |
| 3 Review Git Changes | in progress | Add repository, folder, and pinned repository scopes. |
| 4 Git diff in the right pane | todo | Reuse filter-free line preparation. |
| 5 List column picker | todo | Keep configuration edits comment-preserving. |

## Decisions

- The user requested a separate P2 PR first. Follow-up work begins at its published head in a separate worktree.
- Keep the read-only P2 Git contract. Report the existing local tracking refs for remote status.

## Next

Wire the Review Git Changes sheet into the menu, palette, and folder badges.
