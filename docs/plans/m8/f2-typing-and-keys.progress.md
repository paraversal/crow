# F2 — Typing and keys progress

Branch: `m8-f2-typing` · PR: [#50](https://github.com/paraversal/file-crow/pull/50) · Last updated: 2026-11-17 · State: PR open

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 Show the type-to-select query | done | `TypeSelect::prefix`/`clear`; `Folder::type_select_prefix`/`cancel_type_select`; the bar's transient slot shows the prefix left of the hints. The run expires on a generation-checked `TIMEOUT` task (`FolderEvent::TypeSelect`), not the 100 ms poll. Escape dismisses it and keeps the selection; starting a filter, a rename or another column clears it. Tests: core prefix/clear; bar shows `no`, Escape clears and keeps the row, the timeout clears, the filter field and Vim never show it. |
| 2 Enter behavior setting | done | `enter = "rename" \| "open"` (default rename), `EnterRow` reads it, `f2` always renames. Settings ▸ Behaviour ▸ What Return does (Rename/Open). Tests: default renames, open mode opens, F2 renames in both, sidebar and search still open, the control writes only its line and survives a reload; the keymap test now expects F2. Critique Q2: Rename with more than one selected now refuses with a beep and `Select one item to rename` (info, retires), matching the context menu. |
| 3 Context menus for search results and the sidebar | done | The sidebar menu already existed; verified, not rebuilt. Search results gained `context_menu::search_entries` (Open, Reveal in Folder, Quick Look, Copy, Cut, Copy Path, Move to Trash) and `SearchView` right-click/Ctrl-click, selection-first, action dispatch. `CopyPath` gained a search handler and ⌥⌘C in `SearchResults` (critique Q1). Test: titles, Copy Path shortcut and text, Ctrl-click, Move to Trash once, the key. |
| 4 Palette home row matching | done | The home row's name is scored when it is not a suffix of the label; `name_matched` carries its ranges so `jul` highlights in the name and `~` in the path. Test: both queries, both highlight lines. |
| 5 Finish: checks, bundle, docs | done | fmt, clippy and `cargo test --workspace` pass; bench clean run warm 7 ms / p99 8.60 ms (budget met); `./scripts/bundle.sh` built; README and `docs/testing.md` updated. Visual checks below; the critic drove the search menu by mouse and read the AX tree for VoiceOver. |

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
- [x] Right-click a search result: the automated test drives the titles, Copy Path, Ctrl-click and Move to Trash. The critic drove the mouse path in the bundle: right-click opened the menu at the row, Copy Path put the row's path on the clipboard, and the menu closed with the row still selected. The file list and sidebar menus are unchanged (existing tests).
- [x] Palette: `jul` shows the home row (`jules`, path `~`) with `jul` highlighted in the name (bundle); `~` still shows it with the path highlighted (test); other rows rank as before (test).
- [x] Light and dark (Solarized Light, Gruvbox dark), 640 px and wide, Vim on and off.
- [x] VoiceOver for the new menu items and the bottom-bar query: the AX tree exposes `AXGroup | Type to select: no` and each native menu item by title. The query is a group, so VoiceOver reads it when navigating to it, not on every keystroke.

## Found along the way

- An external `sidebar.toml` reload can leave the sidebar's cached rows stale until a notify (F1 C5). Outside F2; noted for F7.
- Search results for equal-score name hits can swap order between runs (`notes.txt`/`notes2.txt` in the fixture). The search engine decides the order, not F2; noted for M8.5.

## Log

- 2026-11-17: Read AGENTS.md, the M8 index, the F2 plan and F1's progress; read `selection.rs`, `folder.rs`, `pane.rs`, `bottom_bar.rs`, `context_menu.rs`, `sidebar.rs`, `search.rs`, `palette.rs`, `settings.rs`, `settings_window.rs`, `ui.rs`, `actions.rs`, `menus.rs`. Created the branch and this progress file. Asked Q1–Q5; answers recorded.
- 2026-11-17: Item 1. `TypeSelect` gained `prefix`/`clear`; `Folder` gained `type_select_prefix`, `cancel_type_select` and a per-run generation timer that clears the buffer and emits `FolderEvent::TypeSelect`. The bar's transient slot draws the prefix (aria "Type to select: …") only while the list owns the keys; `fit_to_window` gives it room. Escape consumes the run before the Vim mark and the selection; the filter, rename and a column focus change clear it. Tests pass.
- 2026-11-17: Item 2. `EnterAction` in `settings.toml`; `EnterRow` in `LIST`; `f2` → `Rename`; the pane dispatches by the setting. Settings control "What Return does", description names F2. `keys::is_function_key` displays F2 instead of f2. README and `docs/testing.md` updated. Tests: window modes, sidebar/search, settings write/reload, keymap string.
- 2026-11-17: Item 3. Search-result menu: `TitledSlot` (renamed from `SidebarSlot`, now shared), `SEARCH` slots, `search_entries` with the SearchResults scope. `SearchView.menu`, `context_press` (selects an unselected row, keeps a selection), right-click and Ctrl-click handlers, `menu_closed` dispatches the command action, `copy_path` handler. Test drives titles, Copy Path, Ctrl-click and Move to Trash.
- 2026-11-17: Item 4. `Item::Folder.name_matched`; `folder_listing` scores the name only when `name_start` is None; `name_ranges` prefers it. Test for `jul` and `~`.
- 2026-11-17: Finish. `cargo fmt --check` and clippy pass; `cargo test --workspace` passes (478 app, 96 core, 44 fs, 6 macos, 20 search, 6 theme, `fuzz_smoke` included). Bench on `target/fixture-100k`: clean run warm first paint 7 ms, load 240 ms, p50 8.33 ms, p95 8.49 ms, p99 8.60 ms, max 19.57 ms (599 frames, 120 fps); the first run was contaminated by the window being occluded (30 fps, one 6-minute App Nap frame) and is discarded. F1's `main` numbers were warm 10 ms / p99 8.47 ms, so this is within run-to-run noise and the budgets. `./scripts/bundle.sh` built and signed; the bundle was driven with a temp config and fixture for the visual checks above. The palette matcher scores the name only for the home row, so the per-candidate work on a 100k folder is unchanged.

## Next

PR #50 is open for the user to hand-test and merge. Next piece in the M8 order: **F5** (tips and first launch); the executing agent request is `execute plan F5`.

## Critique

Critic session: 2026-11-17 · Base: `main` @ `40c5339` · Head reviewed: `41b18da` (+ `d4fd652`)

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | Right-clicking a row already in the selection moved the row cursor but not the selection cursor or the preview; Open/Reveal/Quick Look acted on one row while the bar and preview named another | `crates/crow-app/src/search.rs:891` | code / UX | fix | fixed (`d4fd652`) |
| C2 | The search menu's Copy Path shows no shortcut and has no key in the results scope, while the file list's Copy Path shows ⌥⌘C | `crates/crow-app/src/context_menu.rs:197`, `crates/crow-app/src/search.rs:1131` | UX / consistency | ask | Q1 → add; fixed (`8fe226c`) |
| C3 | `TypeSelect::prefix()` returns the buffer after the run timed out but before the timer task clears it; the bar gates on `is_active`, so it is not visible today, but a future caller could show a stale prefix | `crates/crow-core/src/selection.rs:156` | code sense | note | left (gated by `type_select_active`) |
| C4 | The prefix is bare text at the left of the hints, the same color as the path; at a glance `no` reads like a hint label. The plan specified plain text in the bottom bar (Q1), so this is a presentation note, not a plan miss | `crates/crow-app/src/bottom_bar.rs:695` | UX / UI | note | left (as specified) |
| C5 | The plan's edge case says Rename is disabled with more than one selected; the code renames the cursor row. The agent recorded the plan's claim as false and left the behavior; `EnterRow` now makes Return the main path into it | `crates/crow-app/src/pane.rs:1315` | plan / UX | ask | Q2 → disable; fixed (`8fe226c`): beep + `Select one item to rename` (info, retires) |
| C6 | Search result order for equal-score name hits flipped between two runs (`notes.txt` first, then `notes2.txt` first); the search engine, not F2, decides the order | search results | UX | note | left (M8.5) |
| C7 | The type-select query is an `AXGroup` with the label "Type to select: …", not a live region; VoiceOver reads it when navigating, not on each keystroke | `crates/crow-app/src/bottom_bar.rs:700` | accessibility | note | left (as specified) |

Checks re-run: `cargo fmt --check` ✓ · `cargo clippy --workspace --all-targets -- -D warnings` ✓ · `cargo test --workspace` ✓ (481 app, 96 core, 44 fs, 6 macos, 20 search, 6 theme; `fuzz_smoke` included) · `./scripts/bundle.sh` ✓ · bench not re-run: the fixes touch the search view and the rename guard, not the 100k listing path (the agent's run: warm 7 ms, p99 8.60 ms vs F1's `main` 10 ms / 8.47 ms).

Looked at: the branch and `main` bundles on `/tmp/crow-f2-fixture`; the type-to-select prefix at 1280 and 640 px in FireBox and Solarized Light; the search-result menu by mouse (right-click, Copy Path to the clipboard, Escape) and the active row/preview after it closes; the search menu's Copy Path now shows ⌥⌘C; two rows selected + Return shows `Select one item to rename` and opens no field; the File menu (Rename F2); the Settings window (What Return does); the palette `jul`/`~` on both branches (`main` says "No folders match 'jul'"); the AX tree for the query and the menu items. Could not: hear VoiceOver's speech (AX tree only); the preview update while the native menu is open is blocked by its modal loop and lands on close.

Verdict: Better. The four items do what the plan asks, the new tests cover the paths, the search menu has one active row for the bar, preview and actions, and a multi-selection Rename now says why it refuses. Both critique questions were answered and fixed; nothing blocks the PR.
- 2026-11-17: Critique. Read the whole diff against `main` (`40c5339`), re-ran fmt/clippy/`cargo test --workspace`, built the branch and `main` bundles and drove both on `/tmp/crow-f2-fixture`. Fixed C1: a right-click on a selected search result now sets the selection cursor and reloads the preview, so the bar, preview and single-row actions name the same row (`d4fd652`, test). Asked Q1 (Copy Path key) and Q2 (multi-selection Rename); answers: add ⌥⌘C in search results, and disable Rename with more than one selected. Fixed both (`8fe226c`): `cmd-alt-c` in `SearchResults`; `Pane::rename` refuses with a beep and the info message `Select one item to rename`. Tests: the menu entry's shortcut, the key copying the cursor row, and the refusal message. Checks pass (481 app, 96 core, 44 fs, 6 macos, 20 search, 6 theme); bundle rebuilt and the two states checked in it.
- 2026-11-17: Pushed `m8-f2-typing` and opened PR #50; the progress file is at PR open. Next: the user hand-tests and merges; then F5.
