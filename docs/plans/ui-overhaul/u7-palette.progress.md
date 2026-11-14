# u7-palette progress
Branch: ui-u7-palette · PR: none · Last updated: 2026-11-14

## Prerequisite check (on `main` @ 0b4fdaa)
- U1 merged (#8): `chrome::{Keycap, KeycapWeight::{Outline, Key, Text}}`, `chrome::{sheet, sheet_panel, SheetWidth::{Compact, Narrow, Wide}}`, `keys::{binding_for, caps, chord_text, spoken, text}`, `style::{scrim, active_selection, inactive_selection}`, `icons`. `text_input::bare()` already exists (U1/U3), `with_aria_label` does not.
- U2 merged (#9): `palette::Command { id, label, action, scope, menu: Option<MenuSlot>, palette }`; `COMMANDS` in menu-bar order; `MenuSlot { menu, section, title }`; `MenuName`.
- U6 merged (#14/#15): `crow_search::FuzzyMatcher::indices` (byte ranges, sorted/deduped/merged, grapheme-aware) and `style::highlight_text(text, ranges, theme, on_selection)` already exist. U6's `name_ranges_follow_nucleo_indexing` covers graphemes. **I1 therefore collapses to adding the two tests U6 lacks.**
- Line numbers in the plan are stale after U2–U6; current locations: palette model `crates/crow-app/src/palette.rs` (1–625), window palette code (`struct Palette` 110, `show_palette`/`open_palette` 1743–1792, `refresh_palette` 1794, `move_palette` 1803, `close_palette` 1811, `confirm_palette` 1823, `run_command` 1836, `render_palette` 1881, `render_palette_rows` 1943), tests 4312–4430.

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 Match ranges from the fuzzy matcher | done (f33824e) | U6 shipped `indices`; added byte-range and score tests |
| I2 Folder model | done (4246041) | `Source`, `name_start`, `last_visit`, `Listing`/`Row`, `folder_listing`, `relative_age`; tests in palette.rs + `Recent::last_visit` |
| I3 Folder mode on the Sheet | done (d833a58) | U1 Sheet (Wide, scrim, scrim click), 50 px input row with Go to glyph + count (`Role::Status`), scrolling div of `Role::Group` sections, 44 px two-line rows (name + middle-truncated path, accent matches), Recent ages, keycap footer; `truncate_middle_with_ranges` and `TextInput::with_aria_label` added; `palette_rows` helper + 4 tests |
| I4 Command mode | done | `PaletteMode`, Commands token, `>` strip (idempotent against the queued nested `Changed`), `BackspaceAtStart` + `.report_backspace_at_start()`, `command_listing` with "Recently used"/"All commands" (5, not repeated), 40 px rows with 64 px menu column, toggle chip + `aria_toggled`, `Keycap::Key` from the live keymap, mode-aware placeholder/aria/footer/count; 4 tests |
| I5 ⇥ completion and path mode | done | `path_query`, `path_listing` (this folder first, name matches, 50 cap), `Row::Status` for Loading…/missing parents; `Palette.path` caches a parent listing spawned with `crow_fs::list_dir` off the main thread (generation + parent guard drops stale results, relists when the dot rule changes); Tab completes the highlighted folder or `complete_typed` off the main thread (command mode/no rows: no-op or bell); 4 keystroke tests + 3 unit tests |
| I6 Recently used commands | todo | |

## Questions
- Q1–Q9 (asked 2026-11-14, batch of the plan's "Questions to ASK") → **all recommended answers** (from the user):
  - Q1 Recently used only on an empty query. Q2 favorite+recent → Favorites. Q3 limits 8 / all / 20 empty, 20 per section with a query, ≤ 50 total. Q4 "this folder" row after ⇥. Q5 5 recent, not repeated. Q6 command footer `↵ run · ⌫ folders`. Q7 record palette-only runs. Q8 no alias marker in U7 (follow-up). Q9 zoxide-only rows show nothing.

## Decisions
- Plan/decided scope, restated: empty-query-only "Recently used"; `palette_recent_commands` default `true`; ⇥ completion in folder mode only; footer keycaps from the live keymap.
- Reuse U6's `FuzzyMatcher::indices` and `style::highlight_text` instead of adding a second implementation (plan's I1 owner note; shared-names table) — plan.
- Keep `Palette::row_keys` (used by `command_palette_keys_follow_the_layer`) as the single key-lookup helper.
- **Deviates from Target (user approved 2026-11-14):** with a non-empty query, folder rows are one flat list by score (no section headers), like command mode. Fixed sections let a weak subsequence match in an earlier section (the start folder's long path, recorded as Recent at `window.rs:142`) outrank the folder actually named in the query, which broke the unchanged `command_palette_cmd_return_opens_a_new_tab`. Empty queries keep the Recent / Favorites / In sections and Q3 caps; typed queries cap at 50 total.

## Hand checks
- [ ] Palette over scrim; sections Recent / Favorites / In <folder>; twins show different paths; accented matches in name and path; "2 h ago"; ↵ / ⌘↵; Vim j/k across headers.
- [ ] ⇥ on a row → its path + `/`; typing narrows; ↵ on "this folder"; `.` shows hidden folders.
- [ ] `>` becomes the Commands token; Backspace removes it; menu column, on/off chip, keycaps follow the Vim layer.
- [ ] Run two commands, quit, relaunch: `>` shows them under Recently used; `palette_recent_commands = false` hides it live and stops recording.
- [ ] FireBox and Solarized Light: token, highlights, chip, keycaps; narrow window keeps 16 px margins.

## Found along the way
- Go to Folder's Tab completion still runs `read_dir` on the main thread (`window.rs`, the `go_to_folder` subscription). The palette runs its own completion in `background_spawn`; moving Go to Folder off the main thread is the follow-up the plan names.
- Path mode sorts a parent's children by name when there is no partial: `crow_fs::list_dir` yields unsorted batches, and a stable order is what the UI and tests need.

## Log
- 2026-11-14: read plan, AGENTS.md, index; verified U1/U2/U6 on `main`; created branch and this file.
- 2026-11-14: I1 done (U6 already implemented `indices`; added byte-range + score tests). I2 done: palette model replaced `items()` with `listing()`; window holds a `Listing` but still draws the old 26 px rows. Found the section-order/ranking conflict (asked; user chose flat typed queries).
- 2026-11-14: I3 done (sheet, sections, two-line rows, keycap footer). I4 done (Commands token, mode-aware field/footer/count, menu column, toggle chips, keycaps). I5 done (path mode with off-main-thread listing, Tab completion).

## Next
I6: remembered commands (`palette-commands.json`, `palette_recent_commands`).
