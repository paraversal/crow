# u2-menu-bar progress
Branch: `ui-u2-menu-bar` · PR: https://github.com/paraversal/file-crow/pull/9 · Last updated: 2026-11-13

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 Verify GPUI/AppKit menu behavior (spike) | done (2026-11-12) | Findings in Decisions; temporary menu changes and the `start_filter` counter reverted. |
| I2 One command registry with menu slots | done (120fae7) | `Command` gained `menu: Option<MenuSlot>` and `palette`; `MenuName`/`MenuSlot` added; `ShowSearch`, `SearchEverything`, `ShowCommandPalette`, `SearchReveal` added; all Target rows have slots. Tests: `palette::tests::*`, `palette_find_opens_search`. |
| I3 Menu builder from the registry | done (this commit) | New `menus.rs`; `menus::action`, `app_menus`, `init`; `main.rs` no longer builds menus. Tests: `every_menu_command_is_in_the_menus`, `menus_are_in_bar_order`, `menu_keys_are_the_finder_keys`, `menu_items_dispatch`. |
| I4 Find Everything with no search open (Q8) | done (this commit) | `SearchStart`, `open_search`, `SearchView::set_everything`, `Crow` ⇧⌘A binding/listener. Tests: `find_everything_from_the_list_opens_search_with_rules_off`, `find_everything_does_nothing_over_the_palette`, `find_everything_from_the_sidebar_with_search_open`, `open_search_with_a_query_runs_it`; existing `search_everything_turns_the_rules_off` still passes. |
| I5 App, Window and Help menus | done (this commit) | Q3/Q4 approved; About/Settings/Services/Hide trio, Window with Minimize/Zoom and the tab items, Help with "Crow Keyboard Shortcuts". Tests: `menu_keys_are_the_finder_keys`, `menu_items_are_enabled_by_focus`, `menu_items_dispatch`, `window_menu_tab_items_switch_tabs`, `help_opens_palette_in_command_mode`. |
| I6 Docs | done (this commit) | README keys paragraph; `docs/testing.md` automated paragraph and six hand-check rows. |

## Questions
- Q1 (asked 2026-11-12): Reveal Result ⌘R under Go? → answer: **yes**, Go section 1, menu-only.
- Q2 (asked 2026-11-12): plain `/`, Space, ↵ key equivalents → answer: **keep all three native**. The rename field was the only place they were stolen: it is drawn inside the list, so the list's handlers stayed on the focus path. Fix that focus path, patch GPUI's `key_to_menu_equivalent` so `enter` shows ↩, and add the invariant test. Fall back to proxy actions only if moving the rename field proved hard.
- Q3 (asked 2026-11-12): Crow menu contents and new bindings ⌘H, ⌥⌘H, ⌘, → answer: **full recommended set** (About, Settings… ⌘,, Services, Hide ⌘H / Hide Others ⌥⌘H / Show All, Quit).
- Q4 (asked 2026-11-12): Window and Help menus and ⌘M → answer: **build both**, with tab items moved to Window.
- Q5 (asked 2026-11-12): ⇧⌘A with search open and focus outside it → answer: **flip the open search's rules and focus its field**.
- Q6 (asked 2026-11-12): menu titles vs palette labels → answer: **keep both**.
- Q7 (asked 2026-11-12): Vim layer keys in menus → answer: **Finder keys only**, documented.
- Q8 (asked 2026-11-12): commands left out of menus → answer: **add Open (File §0) and Eject (File §2)**, leave the rest palette-only.
- Q9 (asked 2026-11-12): New Window location → answer: **current folder when a window is focused, Home only when no window**.
- Q10 (asked 2026-11-12): check marks / dynamic titles → answer: **no check marks, static titles**.
- Q11 (asked 2026-11-12): Edit ▸ Cut/Copy/Paste/Select All in text fields → answer: **TextInput handles the Crow actions too**.

## Decisions
- Registry shape follows the merged U1 code, not the plan's older snippet: `Command { id, label, action: &'static dyn Action, scope: KeyScope }`. U2 adds `menu: Option<MenuSlot>` and `palette: bool` and keeps `action`/`scope`; there is no `keys` field (U1 derives keys from the live keymap).
- PR #7 and U1 (PR #8) are merged on `main`; branch `ui-u2-menu-bar` starts at `c666799`.
- Q2 implemented: **all** list action listeners are attached only while `renaming.is_none()`, so plain keys and AppKit menu equivalents pass to the rename field. `TextInput` now also listens for `CopyFiles`, `CutFiles`, `PasteFiles` and `crow::SelectAll` and maps them to its own text operations. GPUI is patched by `patches/gpui-enter-menu-equivalent.patch` (map `"enter"` to `0x0d`; AppKit shows ↩). Test `plain_keys_stay_in_every_text_field` types `/` and Space into rename, filter, Go to Folder, search and palette fields and checks list actions are off the rename field's path.
- The registry is ordered by menu (Crow, File, Edit, View, Go, Window, then palette-only), so `>` now lists commands in that order. Menu titles and palette labels stay separate (Q6).
- Window tab items are `MenuName::Window`, section 0. I5's `app_menus` puts Minimize/Zoom and a separator before them, which is why the registry's Window items start at 0 and no leading separator is emitted.
- `OpenSettings` calls `crow_macos::open_in_default_app` directly on the main thread. The plan said `background_spawn`, but `NSWorkspace` is AppKit and must run on the main thread; no disk read happens here. `show_about_panel` and `open_in_default_app` are new `crow-macos` helpers.
- `NewWindow` has both a `Crow` root listener (current folder) and the global no-window fallback (Home), so the menu and ⌘N agree (Q9).
- Review fixes (2026-11-13, after the orchestrator's review; ⌘Z choice from the user: "cmd z should undo text input while a text field is focused"):
  - `TextInput` has its own undo history and handles `crow::Undo`/`crow::Redo`, so ⌘Z/⇧⌘Z and Edit ▸ Undo/Redo act on the text in every field and never reach the file Undo. A run of typing at the cursor or deleting next to it is one step; paste, cut and Tab completion are their own steps; an IME composition is one step. With nothing left, it beeps instead of passing through. Test: `cmd_z_in_a_text_field_undoes_typing_not_files`.
  - Settings… reports a failed open in the status line (`crow_macos::open_in_default_app` now returns whether it opened).
  - `menus::action` (a second id → action table) is gone; menus use `Command.action` directly.
  - `plain_keys_stay_in_every_text_field` now checks that Filter, Quick Look, Rename and Move to Trash are off the focus path in all five fields, and that Return ends a rename.
  - The earlier hand-check note "rename field → only Cut/Copy/Paste/Select All enabled" was wrong: the window root's actions (Undo, Redo, Back …) stayed enabled. Undo/Redo now belong to the field.
- Help ▸ Crow Keyboard Shortcuts opens the palette with `>` prefilled via `open_palette(">")`; `ShowCommandPalette` is hidden from the palette itself.
- I1 findings (seen by hand on 2026-11-12, macOS 27.0.1 build 26A434, temporary menus built with `scripts/bundle.sh`):
  1. **Keys shown.** Filter This Folder showed `/` (no Command modifier). Quick Look showed a blank space. Rename showed the literal string `ENTER`. Other keys came from the first Finder-layer binding (`⌘F`, `⇧⌘A`, `⌘K`, `⇧⌘G`, `⌘[` …). After the GPUI patch, Rename shows ↩ and the other keys are unchanged.
  2. **Text fields.** With the rename field focused, `/` fired Filter This Folder and Space fired Quick Look before the fix. In the filter and Go to Folder fields the keys typed normally. After gating the list handlers, `/` and Space stay in the rename field.
  3. **List.** One `/` keypress called `start_filter` exactly once (temporary `eprintln!` counter).
  4. **Help.** A menu titled exactly `Help` gets AppKit's search field; typing `find` listed `Find…` and `Find Everything…`.
  5. **AppKit additions.** Edit gained `AutoFill`, `Start Dictation…` and `Emoji & Symbols`; View gained `Enter Full Screen`; Window gained `Fill`, `Center`, `Move & Resize`, `Full Screen Tile`, `Remove Window from Set` and the window list. No tab-bar items appeared.
  6. **Enabled state.** List focus: list actions enabled. Sidebar: list actions disabled, Find/Back/View enabled. Search results: Quick Look, Move to Trash, Copy and Select All enabled; Paste, Filter and Rename disabled. Palette: list actions disabled, Find/Back/View enabled.

## Hand checks
- [x] Menu bar: File, Edit, View, Go, Window and Help each show their keys; Rename shows ↩, Filter This Folder `/`, Quick Look Space. (2026-11-13, macOS 27.0.1)
- [x] Menus follow focus: list → list actions enabled; Tab to sidebar → list actions grey, Go ▸ Back stays; search results → Copy/Move to Trash/Quick Look enabled, Paste/Filter disabled; rename field → only Cut/Copy/Paste/Select All enabled. (2026-11-13)
- [x] Menus act: Go ▸ Enclosing Folder from alpha → root; Go ▸ Back → alpha; Edit ▸ Filter This Folder opens the filter; Edit ▸ Find… opens search; View ▸ Show Activity shows the Activity list; File ▸ New Tab/New Window work (New Window opens the current folder). (2026-11-13)
- [x] Find Everything: ⇧⌘A from root opens search with "Search everything, all rules off"; typing `needle` finds `node_modules/pkg/needle.txt`. (2026-11-13)
- [x] Typing is not stolen: rename field accepts `/` and Space (`/ `); filter field accepts `a/b `; Go to Folder/search/palette are covered by `plain_keys_stay_in_every_text_field`. (2026-11-13)
- [x] Help search: Help, type `trash` → Menu Items: Move to Trash. (2026-11-13)
- [x] App menu: About Crow opens the standard panel; Hide Crow hides the app and it un-hides. (2026-11-13)
- [ ] Settings… and Hide Others/Show All: not exercised by hand (Settings opens `settings.toml` in the external default app); code path reviewed.
- [ ] (user) Undo in text fields: trash a file, rename another, type, ⌘Z → typing undone, file stays trashed; ⇧⌘Z redoes; same in filter/search/Go to Folder; ⌘Z in the list restores the file.
- [ ] (user) The checks above marked [x] were run by the agent through System Events; repeat the menu ones by hand before merging.

## Found along the way
- The first hand-check attempt sent `keystroke "down" using command down` (the letters d-o-w-n), which fired Open, Close Tab and New Window in sequence and made the app look like it crashed. The app was fine; use `key code 125` for the arrow. No product change needed.
- `Eject` is disabled with list focus because its listener is on the sidebar; that is the intended scope, not a menu bug.

## Log
- 2026-11-12: Created `ui-u2-menu-bar`, recorded the plan and current code shape. Read `palette.rs`, `actions.rs`, `main.rs`, `window.rs` search/palette/listener sections, `search.rs` and `text_input.rs`.
- 2026-11-12: I2: rewrote `COMMANDS` in menu-bar order, added menu slots for every Target row plus Q1/Q4/Q8, added the four new command ids, and wired `ShowSearch`/`SearchEverything` in `run_command`. Palette tests pass; `palette_find_opens_search` passes. Recorded the new empty-`>` order.
- 2026-11-12: Q2 fix: gated all list listeners on `!renaming`, taught `TextInput` the Crow clipboard/Select All actions, added the GPUI ↩ patch, and added `plain_keys_stay_in_every_text_field` (passes).
- 2026-11-12: I1 spike. Built temporary menus and inspected them through System Events/AX. All temporary code reverted; findings above.
- 2026-11-13: I3/I4/I5: added `menus.rs`, `SearchStart`/`open_search`/`set_everything`, the Crow ⇧⌘A binding/listener, app/Window/Help menus and their handlers; added menu, focus, dispatch, tab, Help and Find Everything tests. I6 docs updated.
- 2026-11-13: Ran the bundle hand checks above. `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` all pass. No bench: menus are built once from a static table and no per-frame rendering changed.

- 2026-11-13: Review fixes (see Decisions): text undo in `TextInput`, Settings error in the status line, `menus::action` removed, stronger text-field test. fmt, clippy and `cargo test --workspace` pass.

## Next
PR #9 is open; the user hand-tests and merges. Do not merge.
