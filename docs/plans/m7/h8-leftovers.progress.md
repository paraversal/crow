# M7 H8 — leftovers progress
Branch: m7-h8-leftovers · PR: none · Last updated: 2026-11-17 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| L1 Sidebar hints miss ⌥↑/↓ and → (U8 C6) | done | `hint_set` sidebar order is groups, to list, reorder, remove (eject first on volumes). "to list" merges ⇥ and →; `binding_for` now checks a binding's own context predicate (the search results' Return was shadowing the sidebar's →). Tests: `hints_follow_focus`, `vim_layer_hints_use_the_vim_context`, `sidebar_hints_at_1280_show_groups_and_the_right_arrow`. |
| L2 `short_bytes` rounds after choosing the unit (U8) | done | `sidebar.rs:177` now rounds at the shown precision and re-checks the unit; 999 950 bytes → "1.0 MB", 9.96 GB → "10 GB". Unit test `short_sizes_round_before_choosing_the_unit` covers the KB/MB and GB/TB boundaries. |
| L3 Palette home row reads "~" twice (U7) | done | `FolderCandidate`/`Item::Folder` carry a `name`; `folder_name` uses the home folder's display name (the account short name, "Home" if empty) and leaves the path label `~`. `name_start` is now `Option`, so match ranges are empty when the name is not a label suffix. Test `palette_home_row_names_the_account_and_keeps_the_tilde_path`. |
| L4 Open: ⌘↓ in the context menu, ⌘O in the menu bar (context-menu C4) | done (no change) | User chose the Finder split: menu bar ⌘O, hints/context menu ⌘↓. `actions.rs:129-131` already does this; the comment now records why. Evidence: Finder's File ▸ Open is ⌘O (System Events). |
| L5 Vim j/k block typing those letters (U7) | done | Removed the palette's `Palette && vim` j/k bindings; added `!TextInput` to the Vim list/column contexts so a rename field keeps plain letters. Filter (`!Filter`) and search (no vim binding) were already fine. Test `vim_letters_type_in_every_text_field` types "jk" into all four. |

## Questions
- Q1 (asked 2026-11-17): L1 hint order when the bar is narrow — recommended keep "⇥ to list" and "⌥↑↓ groups", drop reorder and remove first. → answer: `groups_first` (groups, then to-list, then reorder/remove; eject stays first on volume rows).
- Q2 (asked 2026-11-17): L4 which Open key wins — recommended ⌘↓ everywhere. → re-asked with Finder evidence: Finder's File ▸ Open is ⌘O (checked with System Events), while its list views open with ⌘↓. Answer: `split` — keep the menu bar on ⌘O and the hints/context menu on ⌘↓; no code change, record the evidence and drop L4.
- Q3 (asked 2026-11-17): L3 home row name — recommended the account's short name, fall back to "Home". → answer: `short_name`.

## Decisions
- Branch `m7-h8-leftovers` from `main` (plan) — plan.
- L4 is not a bug: the menu bar and the list surfaces intentionally show different Open keys, matching Finder's own split (File ▸ Open ⌘O; list ⌘↓). L4 is dropped with no behavior change, and the comment in `actions.rs` now records why. — user (Q2).
- L1 shows ⌥↑↓ groups first, then one merged "⇥→ to list" hint (the two keys that focus the list), then reorder and remove. Eject stays first on volume rows. — user (Q1) + code convention (the existing Reorder hint merges two keys).

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
