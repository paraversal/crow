# u4-filter progress
Branch: `ui-u4-filter` · PR: none · Last updated: 2026-11-13

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 Match ranges from `Folder` | done (this commit) | `name_match` + `match_ranges`; 3 folder tests and `filter_exposes_match_ranges_per_row`; 111 app tests green. |
| I2 Chromeless `TextInput` that scrolls to the caret | done (this commit) | Existing `bare()` used instead of adding `chromeless()`. Added `scroll_x`, clipping and shifted click/IME mapping; applies to every field. Test `scroll_for_caret_keeps_the_caret_visible`; 112 app tests green. |
| I3 Keep list keys out of the field | done (this commit) | `LIST`, `LIST_FILTERED`, `COLUMNS`, `LIST_NOT_COLUMNS` plus `!Filter` in the Vim layer; `Pane::key_down` returns while the field has focus; `KeyScope::Filter` now includes the list contexts. Unit test `filter_field_context_disables_list_keys`; 113 app tests green. |
| I4 Field inside the focused column and the list frame | done (this commit) | New `pane_filter.rs` (submodule of `pane`); old bar and `render_filter` deleted; field in the focused column/list; `column_row_at` uses `row_count`; column list count now `row_count`. Tests `filter_field_opens_in_the_focused_column`, `filter_field_opens_above_the_list_rows`, `filter_vim_letters_stay_in_the_column_field`, `filter_field_takes_space_slash_and_list_keys_as_text`, `filter_closes_when_the_focused_column_changes`, `filter_follows_live_changes`; list actions also leave the focus path while the field is focused. 121 app tests green. |
| I5 Highlighted names | done (this commit) | `Truncated`, `truncate_middle_parts`, `map_range`; `style::highlight_text` (U6 reuses it). Both row renderers highlight the first match, through the middle truncation. Tests `map_range_through_truncation`, `highlight_text_clamps_ranges`. |
| I6 Footer and ⌘F escalation | done (this commit) | `Pane::take_filter`; `show_search` reads the focused folder, takes the query and calls U3's `open_search`; `search_blocked` keeps other overlays from losing the filter. `SearchView::folder` test accessor. Tests `cmd_f_while_filtering_searches_the_focused_folder_with_the_query`, `cmd_f_from_the_list_while_filtering_also_escalates`, `filter_footer_click_opens_search`, `filter_footer_hides_for_an_empty_query`. |
| I7 No-matches state (D) | done (this commit) | The block from I4 is gated by `shows_no_matches`; note copy by `hidden_note`. Tests `filter_with_no_matches_offers_search`, `filter_no_match_note_follows_hidden_files`, `no_matches_waits_for_a_settled_listing` (loading/error cases are unit-tested: a small fixture settles too fast to hold a loading window), `hidden_note_names_the_blocker_when_hidden_files_are_off`. |

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
Finish: README + `docs/testing.md`, `--bench-filter`, bench vs `main`, `./scripts/bundle.sh` hand checks, push and open the PR.

### Answers applied (2026-11-13)
- Q1 focused folder name; Q2 below the header; Q3 "Esc clears" alone; Q4 transparent ring; Q5 `--bench-filter`; Q6 no U9 hook; Q8 follow U6 highlight; Q9 `Keycap::border(role)` override.

- 2026-11-13: I2 done. `scroll_x` is computed in `TextElement::prepaint` and stored in `paint`; the line, caret and selection are clipped to the field, and `bounds_for_range`/`character_index_for_point` account for the shift. Applies to Go to Folder, rename, search, palette and the filter.

- 2026-11-13: I3 done. All `FileList`/`ColumnView` bindings now carry `!Filter`; `Filter` up/down and `TextInput` keys are unchanged. `KeyScope::Filter`'s stack mirrors the field inside a filtered list. Decision: the field's `esc` keycap will use `text_input::Cancel`, not `ClearFilter`: `ClearFilter` is gated by `!Filter`, and in the field `Cancel` is the action that actually fires (`ClearFilter` remains what the key does from the list).

- 2026-11-13: I4+I5 done in one commit (they share the row renderers). New module is `pane::pane_filter` (a child module) rather than `main.rs`: it accesses `Pane`'s private state without widening ~10 fields to `pub(crate)`. Field press propagation stops at the field; while the field has focus the list's action listeners are off the focus path (otherwise AppKit's menu equivalent keeps "Filter This Folder" enabled and steals `/`). The field's Esc keycap uses `text_input::Cancel` (the action that fires there; `ClearFilter` is gated by `!Filter`). Column list counts use `row_count`, fixing the old count mismatch while filtering.

- 2026-11-13: I6 done. ⌘F from the field or the list, and a click on the footer or the D button, all dispatch `ShowSearch`; `show_search` guards overlays before taking the filter, so no text is lost when search cannot open.

- 2026-11-13: I7 done. The loading window could not be held with a small fixture (it settles before the first draw), so that half of the condition is a unit test and the window test covers D appearing after the listing settles.
