# u5-bottom-bar progress
Branch: `ui-u5-bottom-bar` · PR: https://github.com/paraversal/file-crow/pull/13 · Last updated: 2026-11-13 · State: PR open (not merged)

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 `hints` setting and usage model | done | `HintsMode`, `Settings.hints`, `DEFAULT_SETTINGS` block, `crow-core/src/usage.rs` (`Usage`, `RETIRE_AFTER`, `USAGE_CAP`, `idle_hint_visible`); 5 unit tests; `cargo test -p crow-core` 53 green. |
| I2 Key-use recorder | done | `crates/crow-app/src/usage.rs`: `UsageState` global (`usage.json` in the state dir), `record`/`record_key`/`uses`/`count_key`; called from `Pane::start_filter` (only when it opens the field), `show_search`, `show_palette`. Saves are chained so the file never goes backwards. Tests `pressing_slash_counts_filter_uses` (3 keys, file persisted), `palette_and_dispatch_count_too`, `refocusing_an_open_filter_does_not_count`; 133 app tests green. |
| I3 Three slots and counts | done | New `bottom_bar.rs` (`BarState`, `Primary`, `BarError`, `Count`, `Path`, count helpers, render); `CrowWindow::bar_state`/`render_bottom_bar`. Accessors: `Folder::selected_bytes` (cached by generation), `Pane::{shown_count, selected_bytes, is_filtering, filter_focus, is_renaming}`, `SearchView::{result_count, selected_count, selected_entry, input_focused}`, `Sidebar::cursor_group`, `Ui::config_error_file`. 30 px bar, `path-bar` is `Role::Group` "Status bar"; path keeps its ids/selectors. Tests `counts_name_the_focused_folder`, `counts_show_filter_and_selection`, `one_item_is_singular`, `message_does_not_hide_the_count`, `error_and_running_operation_show_together`; existing path-bar test unchanged and green. 139 app tests. |
| I4 Hints by focus and retiring | done | `bottom_bar::{BarFocus, Hint, Spec, hint_set, hints, merged_keys}`; `CrowWindow::bar_focus` classifies List / FilterField / ListFiltered / Sidebar / SearchResults / SearchField / Overlay (overlay or a rename). Keys from `keys::binding_for` with new `KeyScope::{ListFiltered, ColumnsFiltered}`. Idle hints use `idle_hint_visible` + `usage::uses`; focus hints never retire; `off` hides all; an error token drops every hint. Sidebar hints follow the cursor row (Q6): favorite reorder/remove, recent remove, disk ⌘E, always ⇥ to list. `Keycap::text_role` added for focus/idle colors. 8 tests (`hints_follow_focus`, vim context, retiring, reload, always, off, error, overlay). 147 app tests. |
| I5 Progress and Cancel | done | `RunningView` (label, detail, fraction, percent) + `crow_fs::human_size_pair`; 2 px accent line id `bar-progress`; `Role::ProgressIndicator` group id `bar-progress-text` with aria value 0–100; Cancel button id `bar-cancel` ("Cancel" + live Esc cap, tooltip, aria `Cancel operation, Escape`). `operations::running_label` maps Copy/Move/Duplicate/Delete to -ing. Undo/redo still only message. Tests: `trash_progress_counts_items`, `bytes_progress_uses_one_unit`, `zero_total_shows_no_detail`, `human_size_pair` in crow-fs, `cancel_button_sets_the_cancel_flag`, `escape_cancels_from_the_list`. 152 app tests. |
| I6 Error token with Open | done | `error_token` pill (20 px, radius 10, `style::error_soft`, `alert-circle` Lucide SVG, text ellipsis at 420 px, Group aria "Error: …", tooltip = full text); Open button id `bar-error-open` (16 px, error fill, bg text, semibold) only for config errors. `crow_macos::open_path` (default app, then `open -t`), `bottom_bar::open_config` in a background task with failure as an operation message; tests record the path instead. Q9 line numbers: `crow_core::config::{parse_error, file_message}` used by settings, views, sidebar and exclude loaders. Q2: `operations::message` (info clears after 5 s via a generation-guarded timer), `operations::clear_message` on navigation; errors stay. Q11 applied (Apply to All Apps is an operation error). Tests: `config_error_token_opens_the_broken_file`, `fixing_the_file_clears_the_token`, `pane_error_has_no_open_button`, `info_messages_expire_but_errors_stay`, `navigating_clears_an_operation_error`; core line-number assertions. 157 app tests. |
| I7 Narrow window drops hints first | done | `bottom_bar::{fit_hints, fit_hints_to_window}`: measures path segments, hint keycap + label, and counts with `shape_line` at 12 px; `bar = viewport − 24`; drops hints from the right before the path truncates. Unit test `fit_hints_drops_from_the_end`; window test `narrow_window_drops_hints_before_the_path` (560 → no hints, path drawn; 1900 → all three; the fixture's long temp path needs > 1200). |
| R1 Path folds its middle | done | Review: the path was clipped on the right, hiding the current folder. `bottom_bar::fit` keeps the first and last segments and folds the middle into a "…" button (`path-folded`, goes to the deepest hidden folder, tooltip lists them); only the last segment can ellipsize. Unit `fit_shares_the_bar`; window test `narrow_window_drops_hints_before_the_path` sweeps 1200→380 px. |
| R2 Hints vs path | done | Hints stay while the path can still show first … parent › last; they go before the parent folds. |
| R3 Transient slot capped | done | `BarState.transient_max` leaves the path its ends; progress label, messages and the error text ellipsize. Test `error_and_progress_leave_the_path_its_ends`. |
| R4 Small errors retire | done | "No mark x" is an info message; the clipboard message now expires too. Test `copy_and_missing_mark_messages_retire`. |
| R5 VoiceOver | done | The transient slot is `Role::Group` while it shows hints and `Role::Status` only for progress, messages and errors. Needs the VoiceOver hand check. |
| R6 Bar design | done (user) | While filtering the bar keeps the folder's count (the field says "n of N"); the idle hint set is `/ filter` only (⌘F and ⌘K are on the toolbar). |
| R7 Close guard | done (user) | Closing the last window (⌘W on the last tab, red button) or quitting (⌘Q, menu, palette) during an operation shows a `StopOperation` sheet: Esc keeps it, Return cancels, waits for it to wind down, then closes or quits. Other windows close freely. Tests `closing_the_last_window_mid_operation_asks_first`, `closing_with_other_windows_open_just_closes`, `quitting_mid_operation_asks_first`. |
| R8 Word moves | done (user) | `text_input` `WordLeft/Right`, `SelectWordLeft/Right`, `DeleteWordLeft/Right` on ⌥←/→, ⌥⇧←/→, ⌥⌫/⌥⌦. Unit `word_boundaries_follow_mac_fields`, keystroke `option_arrows_move_by_word_in_text_fields`. |
| R9 List width | done (user) | `TableLayout::fitted`: Name takes what the others leave, then the others give way from the right to their minimums; Name's divider trades width with the next column. Past every minimum whole columns hide from the right (width 0) so no cell is cut at the pane's edge; a lone Name fits the pane exactly. Widths come from the last frame's list bounds; a width change asks for one more frame. Tests in `crow-core/table.rs` and `list_name_column_follows_the_window_width`. |
| R10 Cleanup | done | Stale `allow(dead_code)` removed; `Primary::None`, `Pane::shown_count` and the old `fit_hints` gone; `Ui::config_error` is test-only. Deferred requests noted in U6 (search ranking, name highlights, scope key), U8 (⌥↑↓ groups) and the index (motion pass). |

## Questions
All 13 answered by the user on 2026-11-13; every recommended answer was chosen.
- Q1 → scope: no hints while an error token shows.
- Q2 → timed: info messages clear after 5 s; op errors become the error token and clear when the next operation starts or on the next navigation.
- Q3 → counts table as tabled.
- Q4 → selected folders add nothing; a folders-only selection shows no size.
- Q5 → search field hints "↵ open · ⇥ results · esc done".
- Q6 → drop sidebar hints that do nothing on the cursor row; "⌘E eject" on a disk.
- Q7 → no hints while an overlay is open.
- Q8 → search mode shows the selected result's path.
- Q9 → file name plus "line N" where the TOML parser gives a span.
- Q10 → default app for `.toml`, falling back to `open -t`.
- Q11 → "Apply to All Apps" failures show as an operation error message, not a settings error.
- Q12 → bar height 30 px.
- Q13 → every use of the action counts (key, menu, palette). Implemented as handler calls in the three actions; save tasks are chained in `UsageState.save` so two quick uses cannot write out of order.

## Decisions
- Version 30 px bar (Q12), message lifetime 5 s (Q2), search-result path (Q8), error Open fallback (Q10) — user, 2026-11-13.
- `crow_core::idle_hint_visible(mode, uses)` kept as the plan names it; no extra settings wrapper.
- Q11 applied early: "Apply to All Apps" now sets an `Operations.message` error instead of `Ui.errors[SETTINGS]`, so the token has no file and no Open.
- Q8 applied in `bar_state`: search mode shows the selected result's path.
- `Path` data lives in `BarState` so `bottom_bar::render` needs no access to `CrowWindow`'s private fields.
- Counts helpers are pure (`pane_count`, `search_count`, `sidebar_count`) so the grammar is testable without a window.
- Hints resolve keys through U1's `KeyScope` lookup, not the live dispatch tree: two new scopes (`ListFiltered`, `ColumnsFiltered`) mirror U4's `FileList [ColumnView] Filtered` contexts. This avoids the plan's "first frame after a focus change shows the old context" risk.
- Reorder's merged cap comes out as `⌥⌘↑↓` (the live formatter's Apple order), not the board text `⌘⌥↑↓`; every other cap uses the same order.
- Return shows as `↩` (U1's `keys::function_key`), not `↵` in the plan's test sketch.
- Q9 changed core error strings to "<path> line N: <message>"; the bar strips the config dir, so tests assert "views.toml line 2" rather than "views.toml:".
- I7 measures with the same `shape_line` approach as `style::truncate_middle_parts`; the window test uses 1900 px for "all hints" because the fixture's temp path is longer than the plan's assumed 1200.
- Q2 implemented with one timer per info message and a generation counter, so a newer message is never cleared by an older timer; `PaneEvent::Entered` clears any message.
- `crow_macos::open_path` is only exercised by hand (the binary would launch an app); the window test checks the recorded path.
- Cancel's hover sets `color_alpha(fg, 0x14)` as the background instead of layering it over `surface-raised`; GPUI hover styles replace the fill, so the exact composite differs by one layer.
- `Keycap::text_role` is a small addition to U1's keycap (the plan's focus `fg` / idle `fg-subtle` text needs it).

## Bench (100k fixture, same machine, release)
| Run | warm first paint | p50 | p95 | p99 |
| --- | --- | --- | --- | --- |
| U5 branch 1 | 10 ms | 8.33 ms | 8.74 ms | 8.87 ms |
| U5 branch 2 | 11 ms | 8.33 ms | 8.72 ms | 9.06 ms |
| U5 branch 3 | 10 ms | 8.33 ms | 8.71 ms | 8.97 ms |
| `main` 1 | 10 ms | 8.33 ms | 8.68 ms | 8.89 ms |
| `main` 2 | 10 ms | 8.34 ms | 8.71 ms | 8.95 ms |
| `main` 3 | 11 ms | 8.34 ms | 8.67 ms | 8.86 ms |

Within run-to-run noise. `./scripts/bundle.sh` builds `target/bundle/Crow.app` (Lucide license present).

## Hand checks
- [ ] Bottom bar hints: fresh state dir (`CROW_STATE_DIR=$(mktemp -d)`): idle hints `/ filter · ⌘F search · ⌘K go to`. Press `/` and Esc three times: "filter" disappears, the others stay; relaunch: still gone. Focus the sidebar, a filter, search results: the hint set changes each time. `hints = "always"` brings them back; `"off"` hides all. Vim layer: same, and `m` shows "Mark this folder…".
- [ ] Bottom bar progress: copy a large folder to another disk: accent line on the bar's top edge, "Copying … · n %", Cancel esc; the path and count stay. Esc or Cancel stops it.
- [ ] Bottom bar error: break `views.toml` while an operation runs: progress and the red token show together, hints are gone; Open opens `views.toml`; fixing it removes the token.
- [ ] Bottom bar counts: column view: hover the count, the tooltip names the focused column's folder. Filter: "n of N shown". Select several files: "n selected · size".
- [ ] Narrow window: shrink the window: hints disappear right to left before the path is cut.
- [ ] VoiceOver: Cmd-F5: the count reads "18 items in me"; an error reads "Error: …" once; Open and Cancel read as buttons.

## Found along the way
- `copy_paths` wrote `Operations.message` directly, so "Copied n items" never expired; it now goes through `operations::message` (review, 2026-11-13).
- Closing the last window quits Crow (`main.rs` `on_window_closed`), which cut a running copy off partway; now guarded (review fix R7).

## Log
- 2026-11-13: Review fixes R1–R10 (see Status). Bench after the fixes: warm first paint 11 ms, p50 8.33, p95 8.59, p99 9.05 ms (within noise of the runs above). README, `docs/testing.md` and the U6/U8 plans updated; `cargo fmt --check`, clippy and `cargo test --workspace` green.
- 2026-11-13: Finish. README (settings row, usage.json, a bottom-bar paragraph in Keys) and `docs/testing.md` (automated list, U5 bench numbers, new hand rows, "Progress and cancel" now describes the bar) updated. Bench vs `main` within noise. `cargo fmt --check`, clippy and `cargo test --workspace` green; bundle built. PR opened.
- 2026-11-13: I7 done. `fit_hints_to_window` returns early with no hints, so an error/overlay frame costs nothing here. `cargo test --workspace` green (159 app tests), clippy clean.
- 2026-11-13: I6 done. Added the Lucide `alert-circle` icon (24 px grid, existing ISC notice covers it). Message writes now go through `operations::message`; direct `Operations.message` writes remain only inside `Operations` itself. `cargo test --workspace` green, clippy clean.
- 2026-11-13: I5 done. Progress detail is bytes-based when the plan has bytes, items-based for trash/delete; cancel keeps Esc's existing path. `cargo test --workspace` green, clippy clean.
- 2026-11-13: I4 done. Hint labels/keys read from `BarState`; `KeyScope` gains filtered variants. `cargo test --workspace` green, clippy clean.
- 2026-11-13: I3 done. The error token is plain `error`-colored text until I6 gives it the pill and Open; progress is text until I5. `cargo test --workspace` green, clippy clean.
- 2026-11-13: I2 done. `Usage::record` returns false at the cap, so a capped count writes nothing. `cargo test --workspace` green (133 app tests), clippy clean.
- 2026-11-13: I1 done. `usage.json` is pretty JSON (`{ "counts": { … } }`); a corrupt file loads empty. `cargo test -p crow-core` 53 passed.
- 2026-11-13: asked Q1–Q13; all recommended answers accepted.
- 2026-11-13: read `AGENTS.md`, the index README, the plan, U1/U4 progress. Confirmed U1 (#8) and U4 (#12) are merged on `main`. Created the branch and this file. Preparing the 13 plan questions.

## Next
None: PR open for review and hand testing (now including the review fixes). Do not merge.

