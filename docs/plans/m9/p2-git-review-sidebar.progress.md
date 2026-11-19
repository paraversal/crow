# M9 — Git review and resizable side panes progress

Branch `codex/m9-git-review-sidebar` · PR none · Base `aab54a0` (P2 merged as #67) · 2026-11-19 · State: Ready for critique

| Item | State | Notes |
| --- | --- | --- |
| 1 Resize side panes | done | Drag either divider, double-click to reset, or use the focused divider’s arrows and Return. List/Search share the right pane width. Columns retains its automatic fill until resized. GPUI tests drive both handles. |
| 2 Git details on favorite roots | done | Branch, last-fetched upstream divergence, changed count and conflicts. Wider sidebars request bounded +/− totals using the existing file diff cache. Tested repository roots and alias subfolders. |
| 3 Review Git Changes | done | File menu and palette command, pinned repository and ordinary folder badges, keyboard scope and navigation, staged/unstaged/untracked grouping, disabled deleted paths. |
| 4 Git diff in the right pane | done | Git Diff tab with bounded staged and unstaged text hunks. The Git oracle checks separate index/worktree hunks. Worker output is tied to the active file and snapshot. |
| 5 List column picker | done | View command and header context gesture open the same picker. Keyboard test toggles and saves the folder layout. The default action saves both the folder and default in one config edit. |

## Decisions

- The user requested a separate P2 PR first. Follow-up work begins at its published head in a separate worktree.
- Keep the read-only P2 Git contract. Report the existing local tracking refs for remote status.

## Verification

- Formatting and workspace Clippy passed. Workspace tests passed with 995 successful and one ignored. New tests cover mouse and keyboard resizing, the review sheet's scope and Escape keys, the picker saving to views.toml, and distinct staged and unstaged Git hunks.
- Built Crow.app. In a 1280 pt dark window, two real temporary Git fixtures showed branch, changed-path count and upstream divergence on the pins. Checked the staged and unstaged hunks in the live right pane, scoped changes from the File menu, and the changes from both pinned repositories in one sheet.
- On the standard 100,001-entry clean worktree at 120 Hz, two complete follow-up runs reported 3–6 ms warm first paint and 9.05–9.26 ms scroll p99. The complete steady P2 run reported 6 ms and 9.54 ms. Another P2 run changed refresh rate, and a retry timed out. This confirms the follow-up meets the budgets, with limited controlled baseline samples.

## Hand checks

- [x] Dark theme and wide window: pinned branch and divergence, staged and unstaged hunks, and both repositories in the live review sheet.
- [x] GPUI interaction checks: both dividers resize, left divider resets, the right width carries from List to Columns, and the List picker saves a changed layout.
- [ ] Live Solarized Light and 640 pt window: move both dividers, click folder and repository badges, and open Search with the right pane shown.
- [ ] VoiceOver and keyboard-only walkthrough in the live app. Check the changed-path rows, pane dividers, review scope and diff controls.

## Next

Independent critique of the follow-up branch. P2 is already merged.
