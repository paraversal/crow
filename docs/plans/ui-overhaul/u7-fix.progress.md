# u7-fix progress
Branch: `ui-u7-fix` · PR: none · Last updated: 2026-11-14 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| X1 Palette: more room under section headers | done | header 34 px, `pt(10)/pb(8)`, label line height 16; test `palette_header_leaves_room_above_the_first_row` (8 px gap). Only the eye check in the bundle is left. |
| X2 Text fields: blinking caret, more room on the left | done | 2 px text inset, caret just before the first glyph, x-height-centered text, 500/500 blink; test `caret_blinks_in_text_fields`. Bundle check of the vertical look pending. |
| X3 Search field: ↓ moves into the results | done | ↓ in the field focuses and selects the cursor row, nothing without results; ↑ on row 0 returns to the field. Tests `search_down_from_field_focuses_results`, `search_down_without_results_stays_in_the_field`, `search_up_from_first_row_returns_to_field` plus a keymap unit test. |
| X4 New File | done | `Request::NewFile` in `crow_fs::ops` (unique "untitled", `create_new`, undo trashes it), File menu, palette (`new-file`), ⌥⌘N; inline rename. Ops test `new_file_creates_an_empty_file_with_a_free_name`, window test `new_file_creates_and_renames`. README and `docs/testing.md` updated. |
| X5 Hide the preview | done | `Settings.show_preview` (documented default, live reload), View ▸ Show/Hide Preview with a title that follows the state, palette toggle chip, ⇧⌘P. Hides the list preview, the column preview and the search preview. Tests `preview_toggle_hides_it_in_list_and_columns`, `show_preview_false_in_settings_hides_it`, `preview_menu_names_the_action`; README and `docs/testing.md` updated. |
| X6 UI freezes | in progress | cause found: search batches. Pre-fix `add_hits` took 40-75 ms per batch, once 203 ms for a 2.9M-hit batch; 327 batches ≥3 ms during one `f` search in ~. Icons (H1) are minor (max 24 ms, 5 calls). Fix so far: engine batches capped at 256 hits, hits dropped after the result cap, and the receive loop yields after 8 ms. Final measurement and the content-hit volume fix are being checked. |
| X7 Deleted file stays in the list | done | H1 confirmed: two change batches in flight replayed the older listing. Fixed by stat'ing in the background and merging against the current entries on the main thread (`crow_fs::stat_changes` / `apply_stats`). H2 (path form) handled for symlinked ancestors; test `changes_through_a_symlinked_ancestor_still_match`. Test `a_late_change_batch_does_not_bring_back_a_deleted_file` fails on the old pipeline. |

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
- X3 bindings: a GPUI key predicate (`Search && TextInput`) only matches tags of one context level, so ↓ is bound on `Search` (the band's level) instead of combining `Search` and `TextInput`. The results' plain ↓ keeps winning there because `SearchResults` matches one level deeper.
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
- 2026-11-14 X6 measurements (release bundle, temp config, automation via System Events): searching `f` in ~ produced 327 `add_hits` batches ≥3 ms (worst 203 ms, batch 2,939,988 hits — a single file's matching lines). A 50 ms heartbeat showed a 3.0-4.9 s main-thread block while the stream drained. Icons: only 5 calls ≥2 ms (max 24 ms, cold), so H1 is not the cause; H3 not implicated. Engine now splits batches at 256 hits (`crow-search` test `batches_are_bounded_even_for_a_file_with_many_matches`), the view drops queued batches once capped, and the receive loop yields after 8 ms of work. Post-fix `add_hits` max 8-22 ms; a long block remains when queued hits are dropped, pointing at the sheer number of content hits per file (one per matching line) — the fix is being narrowed down with a `sample` of the running app.
- 2026-11-14 X7 implemented: `apply_folder_changes` lied about the cause — it cloned entries and diffed in the background; a second batch could land with a pre-deletion snapshot and put the file back. Now `stat_changes` runs in the background, `apply_stats` merges on the main thread against current entries; duplicate paths in a burst are deduped (this briefly showed two rows in `filter_follows_live_changes`). Symlinked watcher paths are rewritten into the folder's form. Workspace tests green (236 app, 29 fs).
- 2026-11-14 X5 implemented and tested (235 crow-app tests, whole workspace green; clippy clean). Menus are rebuilt on toggle and on live settings reload so the View item flips between Show and Hide Preview. Bench still to run.
- 2026-11-14 X4 implemented and tested (232 crow-app tests, crow-fs green); menu order and palette key-table tests updated. No extension warning exists in Crow's rename, so the user's "don't warn" wish needs no code.
- 2026-11-14 X3 implemented and tested; 41 search tests and the keymap tests pass.
- 2026-11-14 X2 implemented: 2 px text inset (caret, selection, IME bounds and hit-testing follow), the text is centered on its x-height band, and the caret blinks 500/500 on a per-field executor-clock loop that only notifies the input entity. Focus changes are reconciled in `paint` (focus listeners registered during render never fired in tests; the loop + paint check is simpler and has no listener lifetime). `caret_blinks_in_text_fields` covers focus, off, on, typing, blur; all 227 crow-app tests pass. Visual check in the bundle still to do.

## Next
X6: confirm what the remaining multi-second block is (sample output), then finish the fix and measure again.
