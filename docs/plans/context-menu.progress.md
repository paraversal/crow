# context-menu progress
Branch: context-menu · PR: none · Last updated: 2026-11-14 · State: In critique

## Status
| Item | State | Notes |
| --- | --- | --- |
| C1 Click on empty space deselects | done | `press_empty` in pane.rs; rows set `row_pressed` so the list behind them knows. 5 GPUI mouse tests (`click_*` in window.rs) |
| C2 Context menu model | done (a0aef4b) | `crow-app/src/context_menu.rs`: slots from `COMMANDS`, enabled rules, single-key shortcuts; unit tests + `context_menu_lines_use_the_menu_bar_names_and_keys` |
| C3 Opening the menu | done (a0aef4b), popup not seen running | `crow-macos/src/context_menu.rs` (NSMenu popUpContextMenu from a main-queue turn); `Pane::context_press`; 5 GPUI tests through `choose_context_item`. The native popup itself is a hand check |
| C4 New commands | done (a0aef4b) | Copy Path ⌥⌘C (Edit menu), Show in Finder (File menu, no key); `copy_path_puts_the_selected_paths_on_the_clipboard` |
| C5 Scope and Vim | done | File list only (Q3). Vim: shortcuts shown are the single-key Finder ones; the native menu owns keys while open |

## Questions
- Q1 (asked 2026-11-14): native NSMenu or GPUI-drawn menu? → answer: "Native NSMenu (Recommended)"
- Q2 (asked 2026-11-14): which items; add Copy Path / Show in Finder? → answer: "As proposed (Recommended)" (Copy Path ⌥⌘C, Show in Finder no key; no Get Info, no Open With)
- Q3 (asked 2026-11-14): sidebar and search results in scope? keyboard opening? → answer: "File list only (Recommended)" (sidebar and search results are a follow-up; no key to open the menu)
- Q4 (asked 2026-11-14): column view empty click closes columns to the right? → answer: "Same as Finder (Recommended)"

## Decisions
- The menu opens from an `NSOperationQueue.mainQueue` turn (same reason and helper as Quick Look: a nested run loop inside a GPUI handler panics on GPUI's `App` borrow); the choice comes back through a oneshot channel and runs as the command's action on the focused list (`window.dispatch_action`) — agent, following `quick_look.rs`.
- Disabled, not hidden: Paste with nothing on the pasteboard, Rename with more than one item selected, Add to Sidebar with no folder selected — agent (plan C2 named Paste and Add to Sidebar; Rename-for-one is mine, Finder offers batch rename instead).
- Copy Path and Show in Finder with nothing selected act on the folder shown (Finder's ⌥⌘C does the same; Show in Finder opens the folder itself) — agent.
- Show in Finder sits after Find… in `COMMANDS`: "find" scores the same for "Find…" and "Show in Finder" and ties keep `COMMANDS` order, so `>find` would otherwise run Show in Finder (`palette_find_opens_search` caught it). Alternative not taken: a shorter-label tie-break in `command_listing`, which changes ranking everywhere — agent; critic may prefer the tie-break.
- Shortcuts in the context menu are the last single-keystroke binding that wins in the list's scope, so the Vim layer's `y y` never hides ⌘C (a native menu can't show chords) — agent.
- C1 detects "no row" with a flag set in `Pane::press` (rows handle mouse-down before the list) instead of pointer math, so the side gutters count as empty space and the drag/focus listeners on rows are untouched — agent.
- Worktree at `target/wt-context-menu` (vendor symlinked) because the main checkout is on `housekeeping` with uncommitted work from others — agent.

## Hand checks
- [ ] Context menu (the native popup could not be driven here: posting global mouse events would have hit the user's own running Crow): steps in `docs/testing.md` › Pane › Context menu. Especially: the menu opens at the pointer, no panic or freeze, Esc closes it, a chosen item runs once, Ctrl-click does not also start a drag.
- [ ] Empty click: list view, select a few items, click below the last row → nothing selected, ↓ selects the first row. Column view, three columns open, click the empty part of the first column → the others close, nothing selected. While renaming, click empty space (or another row) → the rename is applied and the field closes.

## Found along the way
- Search results and the sidebar still have no context menu (Q3: follow-up). Search rows have their own `SearchResults::press` (`search.rs:822`).
- Finder's rubber-band selection (drag on empty space) does not exist; a drag from empty space now just deselects.

## Log
- 2026-11-14 C2–C5 done; README and testing.md updated; fmt/clippy/tests green (crow-app 334). Tried to drive the popup in a debug build with posted CGEvents; stopped because the user's own Crow was running and global events could reach it.
- 2026-11-14 C1 done: list and column view, fmt/clippy/tests green.
- 2026-11-14 User answered Q1–Q4, all as recommended.
- 2026-11-14 Plan written from a read of pane.rs, folder.rs, actions.rs, palette.rs, crow-macos.

## Next
Critique (`critique` skill): hand-check the native menu in the bundle, then open the PR.

## Critique
Critic session: 2026-11-14 · Base: main @ 05deb55 · Head reviewed: e0f8786

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | Clicking empty space (or another row) while a rename was open left the field unfocused and the list's actions off the focus path: the rename never committed and the pane was stuck. Plan C1 says a rename in progress commits. | `pane.rs` `press`/`press_empty`/`context_press`; seen in the bundle | plan / UX | fix | fixed (commit below) |
| C2 | The hand check said a running copy's progress keeps updating while the native menu is open; AppKit's tracking loop pauses GPUI frames (plan Q1). Seen: the selection clear did not render until the menu closed. | `docs/testing.md` | plan / docs | fix | fixed (commit below) |
| C3 | The palette comment said Show in Finder sits "under Add to Sidebar"; it is after Eject. | `palette.rs` | code sense | fix | fixed (commit below) |
| C4 | Open shows ⌘↓ in the context menu and key hints but ⌘O in the native menu bar (GPUI takes the first binding, the hints the last). Pre-existing; the context menu follows the hints and Finder. | `actions.rs:126`, `context_menu.rs` `shortcut` | UX / consistency | note | left (pre-existing; the menu bar is the outlier) |
| C5 | AppKit injects an "AutoFill" submenu into the context menu when the filter field had focus. | seen in the bundle | UX | note | left (system behavior, not ours) |
| C6 | After an empty click commits a rename, the operation's completion re-selects the renamed item (`show_results` → `select_when_present`), so "nothing selected" is transient; clicking a row during a rename does the same. | `pane.rs:669` | UX | note | left (operation result; the plan asks for the commit, not the selection after) |

Checks re-run: fmt ✓ · clippy ✓ · tests ✓ (crow-app 336, workspace all green) · bench not needed: no listing, scroll or first-paint path changed (mouse handlers and a menu model only).
Looked at in the app: native Items and Folder menus (open at the pointer, names/keys, disabled Paste and Add to Sidebar, enabled Paste after ⌘C), Copy Path, Cut, Show in Finder, Esc, empty click in list and column view, rename commit on empty click; dark theme, 1200×760. Not looked at: light theme (system is dark; the menu is native and follows the system), VoiceOver, the Vim layer in the running app (the shortcut test covers it).
Verdict: The branch does what the plan asks and the native menu feels right; the rename fix removes a stuck state the new empty click exposed. Better than main.
