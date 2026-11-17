# M7 H8 — leftovers progress
Branch: m7-h8-leftovers · PR: #47 · Last updated: 2026-11-17 · State: PR open

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
- 2026-11-17: `cargo fmt --check` ✓ · `cargo clippy --workspace --all-targets -- -D warnings` ✓ · `cargo test --workspace` ✓ (449 app, 94 core, 44 fs, 6 macos, 20 search, 6 theme). Bench not run: no listing, scroll or first-paint path changed (palette name field and hint text only). `./scripts/bundle.sh` ✓; hand checks above. README's Hints and tips line now names the sidebar's groups and to-list keys.

## Next
PR open: https://github.com/paraversal/file-crow/pull/47. The user hand-tests and squash-merges after CI. H8 was the last M7 piece; after the merge, run the M7 finish checks (30-minute fuzz on three seeds, `crow --fuzz` on the 100k fixture, README status line). Next piece after M7: M8 F6 folder sizes (`docs/plans/m8/f6-folder-sizes.md`) unless the user picks the M8 polish pieces first.

## Critique
Critic session: 2026-11-17 · Base: `main` @ 3a069fc · Head reviewed: 0b19ced (agent) + 4a0ae4be3d (critique)

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | The `binding_for` doc claimed the same rule as GPUI's `DispatchTree::highest_precedence_binding_for_action`, but the branch adds a predicate check GPUI does not make (GPUI only checks that the binding's keystrokes resolve to the action). The check is what keeps the search results' Return from standing in for the sidebar's →. | `keys.rs:78` | code | fix | fixed (4a0ae4be3d) |
| C2 | The L4 hand check says the context menu's `shortcut` "takes the last binding (⌘↓)". The file list's menu does (driven: Open ⌘↓); the sidebar's menu is built with `KeyScope::Sidebar` and shows L with Vim on (↩ without). Both keys work; pre-existing. | `context_menu.rs:197`, sidebar right-click | UX / record | note | left (pre-existing; the record now names the surface) |
| C3 | `context_menu::shortcut` still has the scope-shadowing pattern C1 fixed in `binding_for`: it can pick another scope's binding when the keystrokes resolve to the same action. Visible keys do not change today (list menu ⌘↓, sidebar menu L/↩). | `context_menu.rs:197` | code | note | left (the helper needs single-keystroke bindings; the sidebar menu's keys are valid, and changing them needs a user decision) |
| C4 | `palette_row_names` duplicates `palette_rows` (test-only; one field apart). | `window.rs:3622,3639` | code sense | note | left (12 lines; a shared helper would touch many call sites) |
| C5 | `HintKind::Merged`'s one-binding fallback is unreachable with the shipped keymaps and untested. | `bottom_bar.rs:443` | code | note | left (harmless; it keeps the hint if a future keymap unbinds one key) |
| C6 | The palette's home row now displays "jules" but the fuzzy matcher still scores the path label "~": typing `jul` shows "No folders match 'jul'" (bundle). Pre-existing matcher behavior; the new name makes it noticeable. | `palette.rs:712`, bundle | UX | note | left (L3 is display-only; matching the name line would change folder matching for every row and needs a user decision) |
| C7 | The "to list" hint merges Tab (`SwitchFocus`) and → (`OpenSelection`); pressing → from the sidebar opens the selected folder and focuses the list, not just moves focus. The plan/Q1 asked for "→ to the list"; the behavior is pre-existing. | `bottom_bar.rs:232`, bundle | UX | note | left (plan wording; the key does land focus in the list) |

Checks re-run: `cargo fmt --check` ✓ · `cargo clippy --workspace --all-targets -- -D warnings` ✓ · `cargo test --workspace` ✓ 619 passed, 1 ignored (449 app, 94 core, 44 fs, 6 macos, 20 search, 6 theme); the four new tests pass. Bench not run: no listing, scroll or first-paint path changed (the palette's name field and the sidebar hint text are not on the bench path; the plan says bench not needed).
Looked at: branch bundle (`target/bundle/Crow.app`, Vim on, temp config) on `/tmp/crow-h8-check` in FireBox (dark) and Solarized Light at 1200×760 and 640×760: sidebar hints for a favorite, a Recent row and the volume; the 640 px drop order (groups and to-list stay, reorder/remove drop); the palette's home row (name `jules`, path `~`) in both themes; typing `jk` into the palette, filter, rename and search; File ▸ Open ⌘O; the list context menu Open ⌘↓; the sidebar context menu Open L. Not looked at: VoiceOver (the new text is in existing roles/labels; no new roles), the 100k fixture (no listing path changed), the context menu without Vim (traced: the list menu stays ⌘↓, the sidebar menu shows ↩).
Verdict: Better. All five leftovers are addressed; L1's hint order and L5's typing are visible improvements, L2 is correct at the boundaries, and L3 reads correctly in both themes. The only code beyond the plan is the `binding_for` predicate fix, which the → hint needs. Notes C2, C3 and C6 are pre-existing or outside L3's display scope.
