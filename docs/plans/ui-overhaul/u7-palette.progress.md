# u7-palette progress
Branch: ui-u7-palette · PR: https://github.com/paraversal/file-crow/pull/16 · Last updated: 2026-11-14 · State: **In critique** (PR #16 open)

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
| I6 Recently used commands | done | `CommandId::key`/`from_key` (stable kebab ids, round-trip test), `crow_core::RecentCommands` (front/dedup/cap 5, load/save, broken → empty), `Settings.palette_recent_commands` (default true, documented), `Ui.recent_commands` loaded at startup + `ui::record_command` (background save, no-op when off), `confirm_palette` records palette runs, "Recently used" (5, not repeated) via `command_listing`; 3 keystroke tests |

## Questions
- Q1–Q9 (asked 2026-11-14, batch of the plan's "Questions to ASK") → **all recommended answers** (from the user):
  - Q1 Recently used only on an empty query. Q2 favorite+recent → Favorites. Q3 limits 8 / all / 20 empty, 20 per section with a query, ≤ 50 total. Q4 "this folder" row after ⇥. Q5 5 recent, not repeated. Q6 command footer `↵ run · ⌫ folders`. Q7 record palette-only runs. Q8 no alias marker in U7 (follow-up). Q9 zoxide-only rows show nothing.

- Q10 (critique, asked 2026-11-14): the empty palette's cursor starts on the folder you are in (C5). Recommended: leave the current folder out of the palette's Recent section. → answer: pending
- Q11 (critique, asked 2026-11-14): `surface-raised` lines vanish in Solarized Light (C6). Recommended: use the `border` role for the palette's dividers, Outline keycaps and chip. → answer: pending
- Q12 (critique, asked 2026-11-14): `>` starts on Quit Crow (C7, pre-existing). Recommended: keep registry order but leave Quit out of the empty-query list (still found by typing). → answer: pending
- Q13 (critique, asked 2026-11-14): move the palette view out of `window.rs` (C8). Recommended: a follow-up, not in U7. → answer: pending

## Decisions
- Plan/decided scope, restated: empty-query-only "Recently used"; `palette_recent_commands` default `true`; ⇥ completion in folder mode only; footer keycaps from the live keymap.
- Reuse U6's `FuzzyMatcher::indices` and `style::highlight_text` instead of adding a second implementation (plan's I1 owner note; shared-names table) — plan.
- **Path mode needs a non-row line** ("Loading…", "No folder at …"): `Row::Status(SharedString)` was added to the plan's `Header | Item`. It renders as a 44 px `Role::Label` line and is never selectable.
- Path-mode children are sorted by name when the partial is empty (plan says "listing order"; `list_dir` is unsorted, so a stable order was needed for tests and the UI).
- `Listing.rows` (not `items`) decides the empty state, so a status-only section still draws its header.
- Command count with a query is "m of n" without the word "commands" (Target), so `palette_gt_becomes_commands_token` asserts the two numbers, not a suffix.
- **Deviates from Target (user approved 2026-11-14):** with a non-empty query, folder rows are one flat list by score (no section headers), like command mode. Fixed sections let a weak subsequence match in an earlier section (the start folder's long path, recorded as Recent at `window.rs:142`) outrank the folder actually named in the query, which broke the unchanged `command_palette_cmd_return_opens_a_new_tab`. Empty queries keep the Recent / Favorites / In sections and Q3 caps; typed queries cap at 50 total.

## Hand checks
- [ ] Palette over scrim; sections Recent / Favorites / In <folder>; twins show different paths; accented matches in name and path; "2 h ago"; ↵ / ⌘↵; Vim j/k across headers.
- [ ] ⇥ on a row → its path + `/`; typing narrows; ↵ on "this folder"; `.` shows hidden folders.
- [ ] `>` becomes the Commands token; Backspace removes it; menu column, on/off chip, keycaps follow the Vim layer.
- [ ] Run two commands, quit, relaunch: `>` shows them under Recently used; `palette_recent_commands = false` hides it live and stops recording.
- [ ] FireBox and Solarized Light: token, highlights, chip, keycaps; narrow window keeps 16 px margins.

Automated layout check (throwaway test, since removed) read the rendered bounds in the test platform:
sheet 620×526 at (458, 52) — top offset 52, max list 440; header 26 px; folder row 44 px;
command row 40 px; Commands token 74×24 inside the 50 px input row (list starts at 52+50+1+6=109).
Not verified by me: the light/dark visual look and real key interactions — `screencapture` needs Screen
Recording permission and timed out (TCC prompt), and scripted ⌘K needs Accessibility, so those stay
with the user.

## Critique
Critic session: 2026-11-14 · Base: main @ 0b4fdaa · Head reviewed: c0da7a6

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | `#[allow(dead_code)]` left on `Item::Folder`, `name_ranges`, `Listing.rows/count`, `relative_age` ("I3 renders it"); all used since I3. `section_caps_apply` comment still says "20 per section" for typed queries | `palette.rs:568–690, 1044` | code | fix | fixed (15a3754) |
| C2 | List padding was vertical only: the cursor row's fill ran into the sheet's edges. Board B and `main` inset it; `PALETTE_PATH_WIDTH` already assumed horizontal padding | `window.rs` list `.py(...)`; screenshot branch vs main | UI | fix | fixed (b8c9907), test `palette_rows_are_inset_from_the_sheet` (fails without the fix) |
| C3 | Path line width was a constant for a 620 px sheet; at the 640 px window minimum the sheet is 608 px, so long paths were clipped, not middle-truncated | `window.rs` `PALETTE_PATH_WIDTH` | UI | fix | fixed (b8c9907) |
| C4 | Footer `> commands` was plain text; board B and its neighbours use an Outline keycap | `window.rs` footer | UI | fix | fixed (b8c9907) |
| C5 | Empty query: the cursor starts on the most frecent Recent folder, usually **the folder you are in** (`~` "just now"), so ⌘K ↵ does nothing. On `main` the first row was the first favorite | `palette.rs` `folder_listing`; `ui.rs` `refresh_recent` keeps the current dir | UX regression | ask | Q10 |
| C6 | Solarized Light (the default light theme) maps `surface-raised` to `surface`, so the input/footer dividers, the Outline keycaps' borders, the toggle chip border and row hover all vanish. `border` (base02) stays visible; the bottom bar and search already use it | `window.rs` palette render; `~/.config/themes/schemes/solarized-light.json` roles | UI | ask | Q11 |
| C7 | `>` with an empty query puts the cursor on **Quit Crow** (registry order starts with the Crow menu), so ⌘K > ↵ quits. Same on `main`, not a regression, but U7 makes `>` a first-class mode | `palette.rs` `COMMANDS`, `command_listing` | UX | ask | Q12 |
| C8 | `window.rs` grew 5738 → 6806 lines; about 700 are the palette view (state, path listing, scroll math, three row renderers with a `too_many_arguments` allow) | `window.rs` | code sense | ask | Q13 |
| C9 | Performance was measured on `folder_listing` (10k candidates, 0.73 ms), not on `refresh_palette` as the plan asks; the gap (path state, a clone of the command list) is small | progress Log | record | note | left: the measured part dominates |
| C10 | The home row reads "~" twice (name and path) | folder row | UI | note | left: plan edge case says so |
| C11 | Vim layer: `j`/`k` move the cursor, so names with j or k (e.g. "projects") can't be typed, which now also blocks ⇥ path completion | `actions.rs` Palette && vim | UX | note | left: plan kept it, PR body mentions it |
| C12 | `palette_rows` (window tests) and `row_text` (palette tests) are the same helper twice | `window.rs`, `palette.rs` tests | code sense | note | left |

Checks re-run: fmt ✓ · clippy ✓ · tests ✓ (337, +1 from C2) · bench not needed: no pane row rendering changed
Looked at in the app (bundled branch and `main`, temp config/state dirs, scripted keys, window-only captures): folder mode empty and "git" (twins, highlights, ages), command mode empty and "hid" (token, chip, keycaps), ⇥ into path mode ("this folder"), Gruvbox dark hard and Solarized Light, 640 × 560 window, Vim layer keycaps ("cw" for Rename). Not looked at: VoiceOver, trackpad scrolling, IME in the bare field, FireBox.
Verdict: clearly better than `main`: twins are told apart, sources and ages read at a glance, keys teach themselves, and ⇥ makes the palette a real navigator. Two things should change before merge: the default cursor on the current folder (C5) and the invisible structure in the default light theme (C6).

## Found along the way
- Go to Folder's Tab completion still runs `read_dir` on the main thread (`window.rs`, the `go_to_folder` subscription). The palette runs its own completion in `background_spawn`; moving Go to Folder off the main thread is the follow-up the plan names.
- Path mode sorts a parent's children by name when there is no partial: `crow_fs::list_dir` yields unsorted batches, and a stable order is what the UI and tests need.

## Log
- 2026-11-14: read plan, AGENTS.md, index; verified U1/U2/U6 on `main`; created branch and this file.
- 2026-11-14: I1 done (U6 already implemented `indices`; added byte-range + score tests). I2 done: palette model replaced `items()` with `listing()`; window holds a `Listing` but still draws the old 26 px rows. Found the section-order/ranking conflict (asked; user chose flat typed queries).
- 2026-11-14: I3 done (sheet, sections, two-line rows, keycap footer). I4 done (Commands token, mode-aware field/footer/count, menu column, toggle chips, keycaps). I5 done (path mode with off-main-thread listing, Tab completion).
- 2026-11-14: I6 done (remembered commands + setting). Performance: `folder_listing` with 10 000 candidates, release, worst of 100 runs (empty, 1–2 letter, specific and no-match queries): **0.73 ms**, under the 4 ms budget. `refresh_palette` logs its elapsed time at `debug`. README and `docs/testing.md` updated.

## Next
Critique: waiting on Q10–Q13 from the user; then carry out the answers, re-run checks, push, update PR #16, State → PR open.
