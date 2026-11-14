# u7-fix progress
Branch: `ui-u7-fix` · PR: none · Last updated: 2026-11-14 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| X1 Palette: more room under section headers | done | header 34 px, `pt(10)/pb(8)`, label line height 16; test `palette_header_leaves_room_above_the_first_row` (8 px gap). Only the eye check in the bundle is left. |
| X2 Text fields: blinking caret, more room on the left | done | 2 px text inset, caret just before the first glyph, x-height-centered text, 500/500 blink; test `caret_blinks_in_text_fields`. Bundle check of the vertical look pending. |
| X3 Search field: ↓ moves into the results | todo | ↑ part needs the user (Q2) |
| X4 New File | todo | key and name need the user (Q3, Q4) |
| X5 Hide the preview | todo | key and state location need the user (Q5, Q6) |
| X6 UI freezes | todo | measure first |
| X7 Deleted file stays in the list | todo | reproduce first |

## Questions
- Q1 (X2, asked 2026-11-14): which fields need more room on the left, and how much? → answer: the 2 px text inset in every field, and the text should also be vertically centered (it read slightly low). Implemented both.
- Q2 (X3, asked 2026-11-14): should ↑ on the first result row return to the search field? → answer: yes.
- Q3 (X4, asked 2026-11-14): key for New File → answer: ⌥⌘N.
- Q4 (X4, asked 2026-11-14): new file name → answer: "untitled" without extension, whole name selected; no warning when an extension changes (Crow has no such warning today, so nothing to remove).
- Q5 (X5, asked 2026-11-14): key for Show/Hide Preview → answer: ⇧⌘P.
- Q6 (X5, asked 2026-11-14): where the preview state lives → answer: `show_preview` in settings.toml.
- Q7 (X5, asked 2026-11-14): hide the search preview too? → answer: yes, one switch hides every right-hand preview.

## Decisions
- Base: `main` at a3a8e96, U7 (PR #16) is merged, so the branch starts from it (plan's condition met).
- X1 header: the 12 px label's default line height is 19.5 px, so 34 px with `pt(10)/pb(8)` left only 4.5 px. Added `line_height(16)` so 10 + 16 + 8 = 34 px and the label-to-row gap is 8 px, as on board B. The plan's own numbers assumed a 16 px line.
- X1 test measures the header *label* bounds, not the header box: the header and the first row are adjacent by construction (gap 0 between their boxes), while the plan's test asks for a ≥ 6 px gap. The label bottom to row top is the visible gap.

## Hand checks
- [ ] Palette headers: ⌘K: clear room between "Recent" and its first row, same under Favorites and In; dark and Solarized Light.
- [ ] Caret: palette, filter, search, Go to Folder, rename: the caret blinks, stays solid while typing, stops when the field loses focus, and doesn't sit on the first letter.
- [ ] Search ↓: ⌘F, type, ↓: the first result is selected and focused; ↓ moves on; (↑ on the first row returns to the field, if approved).
- [ ] New File: ⌥⌘N in a folder: "untitled" appears in rename; Return keeps it; ⌘Z moves it to the Trash.
- [ ] Preview: ⇧⌘P hides the preview in list and column view and shows it again; a new window follows the setting.
- [ ] Freezes: search `f` in ~ and open `/Applications` for the first time after launch: no visible stall.
- [ ] Delete: trash several files quickly one after another and a multiple selection: every row disappears.

## Found along the way
- Working tree had user changes before this branch (`.gitignore`, `AGENTS.md`, `u2-menu-bar.md`, `u5-bottom-bar.md`, untracked `package.json`/`package-lock.json`). Left alone; commits touch only this plan's files.

## Log
- 2026-11-14 Plan read; branch `ui-u7-fix` created from `main` (a3a8e96, U7 merged). Progress file added.
- 2026-11-14 X1 implemented and tested (28 palette tests green, `cargo fmt --check` clean).
- 2026-11-14 X2 implemented: 2 px text inset (caret, selection, IME bounds and hit-testing follow), the text is centered on its x-height band, and the caret blinks 500/500 on a per-field executor-clock loop that only notifies the input entity. Focus changes are reconciled in `paint` (focus listeners registered during render never fired in tests; the loop + paint check is simpler and has no listener lifetime). `caret_blinks_in_text_fields` covers focus, off, on, typing, blur; all 227 crow-app tests pass. Visual check in the bundle still to do.

## Next
X3: ↓ in the search field focuses the results, ↑ on row 0 returns to the field.
