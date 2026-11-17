# F2 — Typing and keys progress

Branch: `m8-f2-typing` · PR: none · Last updated: 2026-11-17 · State: Ready for critique

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 Show the type-to-select query | done | `TypeSelect::prefix`/`clear`; `Folder::type_select_prefix`/`cancel_type_select`; the bar's transient slot shows the prefix left of the hints. The run expires on a generation-checked `TIMEOUT` task (`FolderEvent::TypeSelect`), not the 100 ms poll. Escape dismisses it and keeps the selection; starting a filter, a rename or another column clears it. Tests: core prefix/clear; bar shows `no`, Escape clears and keeps the row, the timeout clears, the filter field and Vim never show it. |
| 2 Enter behavior setting | done | `enter = "rename" \| "open"` (default rename), `EnterRow` reads it, `f2` always renames. Settings ▸ Behaviour ▸ What Return does (Rename/Open). Tests: default renames, open mode opens, F2 renames in both, sidebar and search still open, the control writes only its line and survives a reload; the keymap test now expects F2. |
| 3 Context menus for search results and the sidebar | done | The sidebar menu already existed; verified, not rebuilt. Search results gained `context_menu::search_entries` (Open, Reveal in Folder, Quick Look, Copy, Cut, Copy Path, Move to Trash) and `SearchView` right-click/Ctrl-click, selection-first, action dispatch. `CopyPath` gained a search handler. Test: titles, Copy Path text, Ctrl-click, Move to Trash once. |
| 4 Palette home row matching | done | The home row's name is scored when it is not a suffix of the label; `name_matched` carries its ranges so `jul` highlights in the name and `~` in the path. Test: both queries, both highlight lines. |
| 5 Finish: checks, bundle, docs | done | fmt, clippy and `cargo test --workspace` pass; bench clean run warm 7 ms / p99 8.60 ms (budget met); `./scripts/bundle.sh` built; README and `docs/testing.md` updated. Visual checks below; VoiceOver and the search menu's mouse path left for the critic. |

## Questions

- Q1 (asked 2026-11-17): Type-to-select display — bottom bar or overlay? → answer: **bottom bar** (user).
- Q2 (asked 2026-11-17): Confirm the setting name and values: `enter = "rename" | "open"`, default `"rename"`. → answer: **yes**, with a more descriptive Settings label ("What Return does"), and **F2 must always rename in both modes** (user). F2 is now bound to `Rename` in `LIST`; the menu/palette show F2 for Rename in both modes, so Q3 dissolved.
- Q3 (asked 2026-11-17): The ↩ display question. Explained: the native menu bar shows an action's first binding, and the palette/context menu show its winning binding; once Return dispatches `EnterRow`, Rename has no key to advertise. F2 → Rename gives it a real, mode-independent key, so Rename shows F2 and the question is moot. → answer: **explain again**; resolved by F2 (user).
- Q4 (asked 2026-11-17): Search-result menu items. → answer: **Open, Reveal in Folder, Quick Look, Copy, Cut, Copy Path, Move to Trash** (user; Cut added to the plan's list).
- Q5 (asked 2026-11-17): Palette home-row matching in F2 or defer? → answer: **include in F2** (user).

## Decisions

- Base `m8-f2-typing` from `main` (`2964848`); F1 is merged. One branch for F2 — repo rule (AGENTS.md, M8 index Q3), not asked.
- The plan's "Current state" is stale for the sidebar: `context_menu.rs` already has `SidebarMenuKind`/`sidebar_entries`, wired in `sidebar.rs` with tests. Item 3 therefore only builds the search-result menu; the sidebar is verified, not rebuilt.
- Escape while a typing run is active clears the prefix and is consumed (the selection stays), matching Finder's dismissal of its type-select overlay. Escape with no run keeps today's behaviour (Vim mark, then clear selection).
- Starting a filter or a rename clears the type-select run, so the bar never shows a stale prefix beside the filter field or the name field.
- The prefix clears on a timer per typing run: `type_char` bumps a generation and spawns one `TypeSelect::TIMEOUT` task; the task clears the buffer and emits `FolderEvent::TypeSelect` only if its generation is still current. This is testable with `advance_clock` (the folder's 100 ms poll checks `std::time::Instant`, which a test cannot advance) and cancels itself when the folder drops.
- F2 → `Rename` in `LIST`, always, in both Enter modes (user). Return dispatches `EnterRow`, which reads `settings.enter`. The sidebar and search results keep `OpenSelection`; Vim keeps `c w`.
- The Settings label is "What Return does" (user asked for a descriptive label), values "Rename" / "Open", description names F2.
- The search-result menu reuses the command registry; `SearchReveal` gets the row title "Reveal in Folder", and `CopyPath` gains a search handler because the pane's handler is not in the search overlay's focus tree.
- Palette matching scores the name only when it is not a suffix of the label (the home row, `name_start: None`). Scoring every name would double the 100k-folder work and change the ranking of ordinary folders whose label already contains the name.
- The plan's edge case says Rename is disabled with more than one selected "as today"; the code has no such check (`Pane::rename` renames the cursor row, and the context menu alone disables Rename for `selected != 1`). Left as the existing key behavior; changing it is a separate product call, not part of F2.

## Hand checks

- [x] Type `no` in a folder: the bottom bar shows `no` left of the hints (Gruvbox dark, Solarized Light, 1200 px and 640 px). Escape clears it and keeps the row; the 1 s timeout clears it (test); typing in the `/` field never shows it (test); Vim `j` moves and shows no prefix (bundle).
- [x] Enter with `enter = "rename"`: the name field opens (test); set `enter = "open"` in Settings: Enter opened `beta.txt` in CotEditor; F2 still opened the name field in the same mode (bundle). Sidebar and search results still open (test); Vim `c w` still renames (test).
- [x] Settings window: "What Return does" shows Rename/Open with the description naming F2; only the `enter` line changes, a reload keeps the control (test); the bundle screenshot shows the control under Behaviour.
- [ ] Right-click a search result: the automated test drives the titles, Copy Path, Ctrl-click and Move to Trash; the mouse path was not driven in the bundle (a synthetic right-click did not register). The file list and sidebar menus are unchanged (existing tests).
- [x] Palette: `jul` shows the home row (`jules`, path `~`) with `jul` highlighted in the name (bundle); `~` still shows it with the path highlighted (test); other rows rank as before (test).
- [x] Light and dark (Solarized Light, Gruvbox dark), 640 px and wide, Vim on and off.
- [ ] VoiceOver for the new menu items and the bottom-bar query (the query has aria "Type to select: …"; the menu items are native). Left for the critic.

## Found along the way

- An external `sidebar.toml` reload can leave the sidebar's cached rows stale until a notify (F1 C5). Outside F2; noted for F7.

## Log

- 2026-11-17: Read AGENTS.md, the M8 index, the F2 plan and F1's progress; read `selection.rs`, `folder.rs`, `pane.rs`, `bottom_bar.rs`, `context_menu.rs`, `sidebar.rs`, `search.rs`, `palette.rs`, `settings.rs`, `settings_window.rs`, `ui.rs`, `actions.rs`, `menus.rs`. Created the branch and this progress file. Asked Q1–Q5; answers recorded.
- 2026-11-17: Item 1. `TypeSelect` gained `prefix`/`clear`; `Folder` gained `type_select_prefix`, `cancel_type_select` and a per-run generation timer that clears the buffer and emits `FolderEvent::TypeSelect`. The bar's transient slot draws the prefix (aria "Type to select: …") only while the list owns the keys; `fit_to_window` gives it room. Escape consumes the run before the Vim mark and the selection; the filter, rename and a column focus change clear it. Tests pass.
- 2026-11-17: Item 2. `EnterAction` in `settings.toml`; `EnterRow` in `LIST`; `f2` → `Rename`; the pane dispatches by the setting. Settings control "What Return does", description names F2. `keys::is_function_key` displays F2 instead of f2. README and `docs/testing.md` updated. Tests: window modes, sidebar/search, settings write/reload, keymap string.
- 2026-11-17: Item 3. Search-result menu: `TitledSlot` (renamed from `SidebarSlot`, now shared), `SEARCH` slots, `search_entries` with the SearchResults scope. `SearchView.menu`, `context_press` (selects an unselected row, keeps a selection), right-click and Ctrl-click handlers, `menu_closed` dispatches the command action, `copy_path` handler. Test drives titles, Copy Path, Ctrl-click and Move to Trash.
- 2026-11-17: Item 4. `Item::Folder.name_matched`; `folder_listing` scores the name only when `name_start` is None; `name_ranges` prefers it. Test for `jul` and `~`.
- 2026-11-17: Finish. `cargo fmt --check` and clippy pass; `cargo test --workspace` passes (478 app, 96 core, 44 fs, 6 macos, 20 search, 6 theme, `fuzz_smoke` included). Bench on `target/fixture-100k`: clean run warm first paint 7 ms, load 240 ms, p50 8.33 ms, p95 8.49 ms, p99 8.60 ms, max 19.57 ms (599 frames, 120 fps); the first run was contaminated by the window being occluded (30 fps, one 6-minute App Nap frame) and is discarded. F1's `main` numbers were warm 10 ms / p99 8.47 ms, so this is within run-to-run noise and the budgets. `./scripts/bundle.sh` built and signed; the bundle was driven with a temp config and fixture for the visual checks above. The palette matcher scores the name only for the home row, so the per-candidate work on a 100k folder is unchanged.

## Next

Critique: build the branch and `main` with `scripts/bundle.sh`, walk the hand checks (especially the search menu by mouse and VoiceOver), fix what is clearly wrong, then open the PR.
