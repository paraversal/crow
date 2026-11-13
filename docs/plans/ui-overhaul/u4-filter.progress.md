# u4-filter progress
Branch: `ui-u4-filter` · PR: none · Last updated: 2026-11-13

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 Match ranges from `Folder` | done (this commit) | `name_match` + `match_ranges`; 3 folder tests and `filter_exposes_match_ranges_per_row`; 111 app tests green. |
| I2 Chromeless `TextInput` that scrolls to the caret | done (this commit) | Existing `bare()` used instead of adding `chromeless()`. Added `scroll_x`, clipping and shifted click/IME mapping; applies to every field. Test `scroll_for_caret_keeps_the_caret_visible`; 112 app tests green. |
| I3 Keep list keys out of the field | todo | Spike (2026-11-13): rename does not leak keys today, so `!Filter` only (Q7 answered by measurement). |
| I4 Field inside the focused column and the list frame | todo | |
| I5 Highlighted names | todo | U6 is not merged; U4 adds `style::highlight_text`. |
| I6 Footer and ⌘F escalation | todo | U3 is merged; `CrowWindow::open_search(SearchStart)` exists. |
| I7 No-matches state (D) | todo | |

## Questions
- Q1 (asked 2026-11-13): folder name in the footer → answered: the focused folder's name in both places (footer and D copy).
- Q2 (asked 2026-11-13): field below or above the table header → answered: below the header, directly above the rows.
- Q3 (asked 2026-11-13): D's third line when hidden files are shown → answered: "Esc clears" alone.
- Q4 (asked 2026-11-13): ring when the list has focus but the filter is open → answered: transparent border, `/` stays accent.
- Q5 (asked 2026-11-13): a filtered bench (`--bench-filter`) → answered: add it, document it and record numbers.
- Q6 (asked 2026-11-13): U9 hook `PaneEvent::FilterNoMatches` now or later → answered: leave it to U9.
- Q7 (asked 2026-11-13): rename key leak; gate on `!TextInput` too? → measured 2026-11-13 with a throwaway test `spike_rename_keys`: while renaming, `space` inserts a space and Vim `j` inserts `j` in the field (both pass with the current `.when(self.renaming.is_none(), …)` action gating). No leak; keep `!Filter` only.
- Q8 (asked 2026-11-13): highlight colour on the selected row → answered: follow U6 (`fg` + SEMIBOLD on the active selection, accent + SEMIBOLD elsewhere).
- Q9 (asked 2026-11-13): `Keycap::Outline`'s border is `surface-raised`, the same colour as D's button fill, so it would be invisible on that button → answered: add a `Keycap::border(role)` override; the button passes `fg-subtle`.

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
I3: add `!Filter` to the list/column contexts and the `Pane::key_down` early return; land the keymap unit test before moving UI.

### Answers applied (2026-11-13)
- Q1 focused folder name; Q2 below the header; Q3 "Esc clears" alone; Q4 transparent ring; Q5 `--bench-filter`; Q6 no U9 hook; Q8 follow U6 highlight; Q9 `Keycap::border(role)` override.

- 2026-11-13: I2 done. `scroll_x` is computed in `TextElement::prepaint` and stored in `paint`; the line, caret and selection are clipped to the field, and `bounds_for_range`/`character_index_for_point` account for the shift. Applies to Go to Folder, rename, search, palette and the filter.
