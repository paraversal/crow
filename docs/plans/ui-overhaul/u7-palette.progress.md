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
| I1 Match ranges from the fuzzy matcher | in progress | Implementation shipped by U6; add byte-range + score tests |
| I2 Folder model | done | `Source`, `name_start`, `last_visit`, `Listing`/`Row`, `folder_listing`, `relative_age`; tests in palette.rs + `Recent::last_visit` |
| I3 Folder mode on the Sheet | todo | |
| I4 Command mode | todo | |
| I5 ⇥ completion and path mode | todo | |
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
- None yet.

## Log
- 2026-11-14: read plan, AGENTS.md, index; verified U1/U2/U6 on `main`; created branch and this file.
- 2026-11-14: I1 done (U6 already implemented `indices`; added byte-range + score tests). I2 done: palette model replaced `items()` with `listing()`; window holds a `Listing` but still draws the old 26 px rows. Found the section-order/ranking conflict (asked; user chose flat typed queries).

## Next
I3: folder mode on the Sheet (Wide, 50 px input row, sections, 44 px two-line rows, footer keycaps).
