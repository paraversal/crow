# u4-filter progress
Branch: `ui-u4-filter` · PR: none · Last updated: 2026-11-13

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 Match ranges from `Folder` | todo | |
| I2 Chromeless `TextInput` that scrolls to the caret | todo | Merged name is `TextInput::bare()`, not `chromeless()`. |
| I3 Keep list keys out of the field | todo | Spike (2026-11-13): rename does not leak keys today, so `!Filter` only (Q7 answered by measurement). |
| I4 Field inside the focused column and the list frame | todo | |
| I5 Highlighted names | todo | U6 is not merged; U4 adds `style::highlight_text`. |
| I6 Footer and ⌘F escalation | todo | U3 is merged; `CrowWindow::open_search(SearchStart)` exists. |
| I7 No-matches state (D) | todo | |

## Questions
- Q1 (asked 2026-11-13): folder name in the footer → pending
- Q2 (asked 2026-11-13): field below or above the table header → pending
- Q3 (asked 2026-11-13): D's third line when hidden files are shown → pending
- Q4 (asked 2026-11-13): ring when the list has focus but the filter is open → pending
- Q5 (asked 2026-11-13): a filtered bench (`--bench-filter`) → pending
- Q6 (asked 2026-11-13): U9 hook `PaneEvent::FilterNoMatches` now or later → pending
- Q7 (asked 2026-11-13): rename key leak; gate on `!TextInput` too? → measured 2026-11-13 with a throwaway test `spike_rename_keys`: while renaming, `space` inserts a space and Vim `j` inserts `j` in the field (both pass with the current `.when(self.renaming.is_none(), …)` action gating). No leak; keep `!Filter` only.
- Q8 (asked 2026-11-13): highlight colour on the selected row → pending
- Q9 (new, asked 2026-11-13): `Keycap::Outline`'s border is `surface-raised`, the same colour as D's button fill, so it would be invisible on that button → pending

## Decisions
- Merged names used: `TextInput::bare()` replaces the plan's `chromeless()`; U3's `CrowWindow::open_search`/`SearchStart` and `SearchView::query_text`/`scope`/`input_focus_handle` exist and are reused. U1's `keys`, `chrome`, `icons`, `style` helpers are on `main`.
- Plan line references are stale (U3 moved code); line numbers in this file refer to the tree at branch start.

## Hand checks
- [ ] Field in the focused column; other columns don't move.
- [ ] List view field below the header.
- [ ] Highlighted match on selected/unselected rows in a light and a dark theme.
- [ ] 60-character query stays inside a 220 px field with the caret visible.
- [ ] ⌘F from field and footer, and the no-match button, open search with the same text in the focused folder.
- [ ] D state in both themes; hidden-files note.
- [ ] VoiceOver: "Filter <folder>", count, no-match line; check whether AXStatus changes are announced.
- [ ] IME (Japanese) still works in a text field (I2 touches every TextInput).

## Found along the way
- (none yet)

## Log
- 2026-11-13: read AGENTS.md, execute-plan, index README, U4 plan, U3 progress; confirmed U1/U2/U3 are merged on `main` (U3's PR #10 was squash-merged as `a539089`) and U6 is not started. Created branch and this file. Measured Q7 with a throwaway test, then removed it. Asked Q1–Q6, Q8–Q9.

## Next
Wait for answers to Q1–Q6, Q8–Q9; meanwhile start I1 (match ranges) and I2 (TextInput scroll), which do not depend on them.
