# F3 — Info pane progress
Branch: m8-f3-info · PR: [#53](https://github.com/paraversal/file-crow/pull/53) · Last updated: 2026-11-17 · State: PR open

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 The surface | done | `InfoPane` entity + 28 px tab strip in list, column and search; ⌘I opens Info / second closes; ⇧⌘P toggles the column and shows Preview; tab choice survives folders. Tests: `cmd_i_opens_info_and_a_second_closes_it`, `info_opens_when_the_default_is_off`, `info_tab_survives_a_folder_change`, `info_shows_in_column_view` |
| 2 Fields | done | Name, Kind, Size, Where (link), Created, Modified, Added, Tags, Permissions, Owner, Group, Items; multi summary (count, bytes, folders, common kind/parent, shared tags, capped at 500). F6 is not merged, so folder sizes and list-view item counts show `--`/`—`. Tests: `info_file_shows_its_size`, `info_package_is_not_a_folder`, `info_multi_selection_is_a_summary`, `info_multi_selection_without_a_common_parent`, `loaded_fields_match_stat`, `a_symlink_reports_its_target_and_a_dangling_one_survives` |
| 3 Metadata loading | done | `crow_macos::permissions` (mode bits, owner, group) + tags/stat in the InfoPane's background loader; generation drops stale results; io guard passes. Tests: `mode_strings_match_ls`, `permissions_read_the_file_and_its_owner`, `a_stale_load_does_not_replace_the_new_selection` |
| 4 Reaching it | done | `ShowInfo` (⌘I) action, View menu, palette key `show-info`, item and search context menus; Settings copy; README and testing.md. Tests: `cmd_i_*`, `info_search_result_target`, `info_sidebar_target_and_where_link`, `palette_show_info_opens_the_info_tab`, `context_menu_lines_use_the_menu_bar_names_and_keys`, `search_result_menu_lists_its_items_and_runs_them` |
| Finish: checks, bench, bundle, README, testing.md | done | fmt/clippy/tests pass; bench within budget vs `main`; `target/bundle/Crow.app` built and the list-view Info tab checked; README/testing.md/settings copy updated |

## Questions
- Q1 (asked 2026-11-17): second ⌘I closes the column or returns to Preview? → answer: closes it if Info is showing (toggle); otherwise opens Info (user)
- Q2 (asked 2026-11-17): with `show_preview = false`, does ⌘I open the column? → answer: yes, explicitly; the setting is the default (user)
- Q3 (asked 2026-11-17): permissions and owner/group in v1? → answer: yes, mode bits and owner/group, loaded off the main thread (user)
- Q4 (asked 2026-11-17): multi-selection summary only, or a capped list? → answer: summary only (user)
- Q5 (asked 2026-11-17): search results and the sidebar in scope? → answer: list, column view, search results and the sidebar (user)
- Q6 (asked 2026-11-17): confirm the tab strip design → answer: text tabs with an accent underline (user; options visualised)
- Q7 (asked 2026-11-17): add Get Info to the context menus? → answer: yes, on file/folder rows (list, column view and search results), not the sidebar (user)

## Decisions
- Info shares the right column with Preview as two tabs; not a separate Get Info window — M8 index (user, 2026-11-17).
- F6 (folder sizes) is not merged; the pane shows `--` for a folder's size and item count until it is, and the F6 field is wired when that piece lands — plan + current `main`.
- Stop at "Ready for critique" without pushing; the critic opens the PR — AGENTS.md (wins over the plan's "push").
- Column visibility is per window: `InfoPane` holds `visible`, initialised from `settings.show_preview` and reset when that setting changes. ⇧⌘P still writes the setting (the default) and toggles the current window; ⌘I toggles only the window, so it can open the column while the default is off — Q2 + plan.
- ⇧⌘P showing the column selects Preview (the menu says "Show Preview"); ⌘I selects Info — plan + menu title.
- Multi-selection tag reads are capped at 500 items; beyond that the Tags row shows `—` rather than claiming a shared set from a partial scan — performance guard, no plan conflict.
- Search multi-selection sizes are computed off the main thread from the same capped scan; the file list's summary uses its cached exact byte total — performance guard.
- A symlink's size and dates follow the target (Finder's Get Info does); a dangling link keeps the link's own mode and size. The link's permissions come from the target when it exists — plan's symlink test.
- The context menu line is "Show Info" (the menu/palette name), not Finder's "Get Info", so the three surfaces cannot drift — Q7 + palette registry.
- The View menu's Show/Hide Preview title follows `settings.show_preview` (the default), not a window's explicit ⌘I override. The menu is app-wide and rebuilt on setting changes; a per-window title would need a menu rebuild on every focus change — noted, not changed.
- F6 is not merged: folder sizes show `--`, and a list-view folder's Items is `—` because no child column is loaded. When F6 lands, its size goes in the same Size row; no pane reload is needed because the target rebuilds from the listing.

## Hand checks
- [x] List view, `target/bundle/Crow.app`: ⌘I opened the column on Info; the tab strip, `.bun` folder fields (Kind Folder, Size `--`, Where `~`, Created/Modified/Added, Tags "No tags", Permissions `drwxr-xr-x`, the account name and group, Items `—`) all rendered; checked with a screen capture (not in the repo).
- [x] Critic, light (`solarized-light`) and dark (`firebox`), 1280 and 640 px: list, column and search Info tabs rendered; the 640 list keeps the sidebar + column and truncates the Name column as the preview always did. Screenshots in `target/shots/f3-info-*` (not committed).
- [x] Critic, a package (`Safari.app`: Kind application, Size `--`, no Items), a symlink (`link.txt`: Size 6 bytes = the target, Permissions `-rw-r--r--`), a dangling link (`dangling`: Size 7 bytes, Permissions `lrwxr-xr-x`), a 5-item selection (5 items, 22 bytes · 2 folders, Mixed, common parent, No tags) and a 100k selection (100000 items, 0 bytes, Mixed, Tags `—`).
- [x] Critic, search result and sidebar targets: the search result's Info shows the file; the sidebar's cursor folder shows its fields and Items when the center lists it; the Where link navigates to the parent.
- [x] Critic, narrow search (640 px): ⌘I/⇧⌘P show the column as an overlay over the results; the results keep their width and do not overlap the tab strip. Test `info_opens_in_a_narrow_search`.
- [ ] VoiceOver on the tab strip, the column label and the Where link: the automated assertions cover the roles and labels (`info-tab-*`, the column label, `info-where` Role::Button); System Events does not expose the GPUI window, so a real VoiceOver pass is left for the user.
- [ ] The native View menu item and the palette chip in the app: the keystroke and palette tests pass; the menu title still follows `settings.show_preview` (see the Decisions).
- [ ] A selection change while metadata loads in the app: the generation test covers the logic; a slow volume would show the "…" fields filling in.

## Critique
Critic session: 2026-11-17 · Base: `main` @ c0ff7bf · Head reviewed: d8398f0

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | The Many key was `{generation, count}`; each folder and search starts its generation at 0, so switching tabs (or search↔pane) with the same generation and count kept the previous selection's tags and bytes. | `info_pane.rs:89`, `pane.rs:738`, `search.rs:687` | code | fix | fixed (11f9212) |
| C2 | At 640 px in search, ⌘I was a silent no-op: the narrow rule hid the column. | `window.rs:3567` | UX | ask | Q1 → explicit override; fixed (d8398f0) |
| C3 | With the override, search's scope row and "Modified" header drew over the tab strip and the results were squeezed to ~133 px (search has no narrow mode). | `window.rs:3569` | UX | ask | Q2 → float the column over the results; fixed (d8398f0) |
| C4 | A search result's Info waits for the preview's stat; the pane shows "No selection" until the preview loads. | `search.rs:690` | UX | note | left (the preview is the only source of the size; the column also waits) |
| C5 | `debug_selectors` gates "preview" on `settings.show_preview`, not `info.visible()`. | `window.rs:3739` | code | note | left (fuzz/test-only) |
| C6 | The View menu's Show/Hide Preview title and the palette chip follow the setting, not a window's explicit ⌘I. | `menus.rs:13`, `window.rs:2839` | UX | note | left (agent's Decision; a per-window title needs a menu rebuild on focus changes) |
| C7 | The Info pane's Items placeholder is `—` while Size is `--`; the plan writes `--` for both. | `info_pane.rs:414` | UI | note | left (Size matches the preview's `--`; `—` matches unknown dates) |
| C8 | The tab strip is not keyboard-focusable; ⌘I/⇧⌘P are the keyboard path. | `info_pane.rs:290` | UX | note | left (roles/labels are set; the keys cover it) |
| C9 | A 100k selection's Tags row says `—` (the shared-tag read is capped at 500). | `info_pane.rs:23` | UX | note | left (agent's Decision; an honest cap) |

Checks re-run: `cargo fmt --check` ✓ · `cargo clippy --workspace --all-targets -- -D warnings` ✓ · `cargo test --workspace` ✓ (518 passed, 1 ignored in crow-app; 99 core, 44 fs, 8 macos, 20 search, 6 theme) · bench `target/fixture-100k`: cold first paint 168 ms, warm 9 ms, avg 8.35 ms, p99 8.52 ms, max 16.74 ms vs `main`'s warm 10 ms, avg 8.35, p99 8.52; warm and p99 are equal, cold varies (92–168 ms across runs) and is not a budget.
Looked at: list, column, search and sidebar Info; light and dark; 1280 and 640 px; file, folder, package, symlink, dangling link, 5-item and 100k selections; the tab strip in all three surfaces; the narrow-search overlay. VoiceOver could not be driven through System Events (the GPUI window exposes only the menu bar there); the role/label tests cover the new elements, and the real VoiceOver pass is a hand check.
Verdict: better. The pane is legible in both themes, the multi-selection summary is honest, and the two critique fixes remove a stale-metadata bug and a silent narrow-search no-op. The remaining notes are small; the View menu title is the only visible inconsistency, and it needs a per-window menu rebuild to fix.

## Bench (2026-11-17, `target/fixture-100k`)
- Branch `m8-f3-info`: cold first paint 151 ms / 121 ms, warm 10 ms / 12 ms, avg 8.36 ms, p99 8.64 ms / 8.57 ms, max 16.12 / 20.25 ms.
- `main` c0ff7bf: cold 92 ms, warm 10 ms, avg 8.35 ms, p99 8.52 ms, max 19.07 ms.
- Budgets: warm first paint < 50 ms, scroll p99 < 1.5 × 16.67 ms (25 ms at 60 Hz; 12.5 ms at 120 Hz). Both pass; the p99 difference is 0.05–0.12 ms (run-to-run noise), warm first paint is equal. Cold first paint varies 92–151 ms between runs and is not a budget.

## Found along the way
- (none yet)

## Log
- 2026-11-17: branch `m8-f3-info` from `main`; read the plan, M8 index, README, testing.md, and the preview/pane/actions/menus/palette/settings code. Asked Q1–Q6.
- 2026-11-17: built `InfoPane`, the tab strip in list/column/search, the fields and multi summary, `crow_macos::permissions`, the background loader with generation checks, `ShowInfo` (⌘I), the View menu, palette and context-menu lines, and the README/testing/settings copy. Added 15 info tests plus the key/context-menu/menu expectations. `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass (516 app tests). Bench vs `main` within budget; bundle built; list-view Info tab checked by screenshot.
- 2026-11-17: critique: fixed the Many key (11f9212) and the narrow-search override/overlay (d8398f0); re-ran fmt/clippy/tests (518 app tests) and the bench; checked light/dark, 640/wide, package/symlink/dangling, 5-item and 100k selections, and the search/sidebar targets; opened the PR.

## Next
PR open: https://github.com/paraversal/file-crow/pull/53. The next piece in the M8 index's order is **F6 — folder sizes** (`docs/plans/m8/f6-folder-sizes.md`); F3 is merged into the branch, so F6 can start. Request for the executing agent: `execute plan F6`.
