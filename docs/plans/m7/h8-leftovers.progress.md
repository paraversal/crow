# M7 H8 — leftovers progress
Branch: m7-h8-leftovers · PR: none · Last updated: 2026-11-17 · State: Ready for critique

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
- [x] L1: sidebar focused at 1280 px in the bundle (firebox, hints always, `/tmp` start) → bottom bar reads `⌥↑↓ groups · ⇥→ to list · ⌥⌘↑↓ reorder · ⌘⌫ remove · Favorites · 2`. Screenshot `target/shots/h8-tab2.png`. Not hand-checked: the narrow-window drop order; the order is the priority and `fit` drops from the right (`fit_shares_the_bar` unit test), and the critic can narrow the window.
- [x] L2: not hand-checked — it needs a volume with exactly 999 950 bytes free. Unit test `short_sizes_round_before_choosing_the_unit` covers 999 499, 999 949, 999 950 and 1 000 000 bytes, plus the GB→TB equivalents and the 9.96 GB → 10 GB case.
- [x] L3: ⌘K with `~` in Favorites, typed `~` → the row's name is `jules` and its path is `~`. Screenshot `target/shots/h8-l3-home-filtered.png`.
- [x] L4: File ▸ Open shows ⌘O in the bundle (screenshot `target/shots/h8-l4-file-menu.png`). The context menu was not driven (native menu); it is unchanged and its `shortcut` takes the last binding (⌘↓).
- [x] L5: Vim layer in the bundle; typed `jk` into the palette (`h8-l5-palette.png`), the filter (`h8-l5-filter.png`), a rename (`h8-l5-rename.png`) and the search field (`h8-l5-search.png`) → all show `jk`, no cursor move.

## Found along the way
- `keys::binding_for` matched a binding by its keystrokes alone, so the search results' Return shadowed the sidebar's → (and could do the same for other duplicate keys). Fixed in L1 by also checking the binding's own context predicate. The context menu has a similar local `shortcut` helper; it was left alone because its visible keys did not change (noted for the critic).

## Log
- 2026-11-17: read AGENTS.md, the M7 index, H8, the earlier critique notes (U8 C6/C7, U11 I9, context-menu C4), README and testing.md. Branch + progress file. Re-checked all five items on `main`; all still present.
- 2026-11-17: asked Q1–Q3. Q1 groups first; Q2 re-asked after checking Finder (File ▸ Open is ⌘O, not ⌘↓), user chose the Finder split; Q3 short name.
- 2026-11-17: L2 rounding + boundary tests. L5 palette j/k removed, `!TextInput` on the Vim list/column contexts, four-field keystroke test.
- 2026-11-17: L1 merged hint kinds, sidebar order, `binding_for` predicate fix, 1280 px test. L3 name field on palette folders, home row test. L4 no code change; comment records the Finder split.
- 2026-11-17: `cargo fmt --check` ✓ · `cargo clippy --workspace --all-targets -- -D warnings` ✓ · `cargo test --workspace` ✓ (449 app, 94 core, 44 fs, 6 macos, 20 search, 6 theme). Bench not run: no listing, scroll or first-paint path changed (palette name field and hint text only). `./scripts/bundle.sh` ✓; hand checks above.

## Next
Ready for critique. The critic should hand-check the narrow sidebar hint drop order and the context menu's Open key, and decide whether `context_menu::shortcut` should use `keys::binding_for`.
