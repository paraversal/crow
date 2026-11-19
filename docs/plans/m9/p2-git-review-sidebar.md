# M9 — Git review and resizable side panes

Follow up on P2 after [PR #67](https://github.com/paraversal/file-crow/pull/67). Keep Git access read-only. Upstream indicators describe local tracking refs. Pinning a repository must neither fetch remotely nor start unbounded content reads.

1. Give the left sidebar and right Preview/Info pane resize handles. Keep changes local to the window, allow reset and keyboard adjustment, and preserve usable center space at 640 px. Apply the right width to List, Columns, and Search wherever their right pane is shown.
2. Decorate favorite repository roots with branch, changed-path count, and ahead/behind. Reuse background snapshots and bound work across visible pins. Allow a bounded optional line summary with separately reported additions and removals, and use a clear unavailable or partial state for results that cannot be totaled. Pinned folders within a repository remain ordinary favorites.
3. Add Review Git Changes to the palette and menu, and open it from a Git badge for that folder. Show staged, unstaged, and untracked changes. Open the selected surviving path in Crow, and keep deletions visible without offering to reveal a missing file. Include a view of changes across pinned repositories.
4. Add a Git diff to the right pane for one changed file. Use the filter-free provider with separate staged and unstaged views, bounded output, and clear handling for unsupported files. Keep file reads off the UI thread and suppress stale output when selection or plugin state changes.
5. Add a List column picker reachable from the header and View commands. Toggle Lines and other optional columns for the current folder, preserve dormant plugin column IDs, and offer an explicit action to save the layout as the default.

Verify with the workspace gates, GPUI interaction tests, and the 100k fixture against the same main baseline. Use Crow.app for layout checks.
