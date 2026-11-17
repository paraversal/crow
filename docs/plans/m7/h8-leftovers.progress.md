# M7 H8 — leftovers progress
Branch: m7-h8-leftovers · PR: none · Last updated: 2026-11-17 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| L1 Sidebar hints miss ⌥↑/↓ and → (U8 C6) | blocked (Q1) | `bottom_bar.rs` `hint_set` sidebar arm has reorder/remove/eject/to list only; `actions.rs` binds ⌥↑/↓ to PreviousGroup/NextGroup and → to OpenSelection. Ordering needs Q1. |
| L2 `short_bytes` rounds after choosing the unit (U8) | in progress | `sidebar.rs:177`; 999 950 bytes → "1000 KB". Unit test boundaries. |
| L3 Palette home row reads "~" twice (U7) | blocked (Q3) | `palette.rs` `folder_candidates` labels home `~`; row name is `label[name_start..]`; path is the same label. Name source needs Q3. |
| L4 Open: ⌘↓ in the context menu, ⌘O in the menu bar (context-menu C4) | blocked (Q2) | `actions.rs:129-131`: cmd-o first, cmd-down last. Hints/context menu take the last; GPUI's native menu takes the first (`vendor/zed/crates/gpui_macos/src/platform.rs:348`). |
| L5 Vim j/k block typing those letters (U7) | in progress | Palette has `Palette && vim` j/k; rename sits in a `FileList && vim` context with no `!TextInput`. Filter already has `!Filter`; search field has no vim binding. |

## Questions
- Q1 (asked 2026-11-17): L1 hint order when the bar is narrow — recommended keep "⇥ to list" and "⌥↑↓ groups", drop reorder and remove first.
- Q2 (asked 2026-11-17): L4 which Open key wins — recommended ⌘↓ everywhere, keep ⌘O bound but not shown.
- Q3 (asked 2026-11-17): L3 home row name — recommended the account's short name, fall back to "Home".

## Decisions
- Branch `m7-h8-leftovers` from `main` (plan) — plan.

## Hand checks
- [ ] L1: focus the sidebar at 1280 px wide → the hint row names the group jumps (⌥↑/↓) and → to the list; narrow the window → reorder/remove drop before those.
- [ ] L2: a volume with 999 950 bytes free shows "1.0 MB free" (or "1 MB free"), never "1000 KB free".
- [ ] L3: ⌘K with Home in Favorites → the home row's name is the account short name (or "Home") and its path is "~".
- [ ] L4: File ▸ Open and the list's context menu show the same key.
- [ ] L5: Vim layer on; type "jk" into the palette, filter, rename and search → the letters appear, the cursor does not move.

## Found along the way
- (none yet)

## Log
- 2026-11-17: read AGENTS.md, the M7 index, H8, the earlier critique notes (U8 C6/C7, U11 I9, context-menu C4), README and testing.md. Branch + progress file. Re-checked all five items on `main`; all still present.

## Next
Ask Q1–Q3, then implement L2 and L5 (independent), then L1, L3, L4.
