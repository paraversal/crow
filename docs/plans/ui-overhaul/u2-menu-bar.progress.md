# u2-menu-bar progress
Branch: `ui-u2-menu-bar` · PR: none · Last updated: 2026-11-12

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 Verify GPUI/AppKit menu behavior (spike) | done (2026-11-12) | Findings in Decisions; temporary menu changes and the `start_filter` counter reverted. |
| I2 One command registry with menu slots | done (120fae7) | `Command` gained `menu: Option<MenuSlot>` and `palette`; `MenuName`/`MenuSlot` added; `ShowSearch`, `SearchEverything`, `ShowCommandPalette`, `SearchReveal` added; all Target rows have slots. Tests: `palette::tests::*`, `palette_find_opens_search`. |
| I3 Menu builder from the registry | todo | New `menus.rs`; move global handlers and `set_menus` out of `main.rs`. |
| I4 Find Everything with no search open (Q8) | todo | Add `SearchStart`, `open_search`, Crow-context ⇧⌘A and `set_everything`. |
| I5 App, Window and Help menus | todo | Q3/Q4 approved the full recommended sets; build About/Settings/Hide, Window, Help. |
| I6 Docs | todo | README and `docs/testing.md`. |

## Questions
- Q1 (asked 2026-11-12): Reveal Result ⌘R under Go? → answer: **yes**, Go section 1, menu-only.
- Q2 (asked 2026-11-12): plain `/`, Space, ↵ key equivalents → answer: **keep all three native** ("Other"). The rename field is the only place they are stolen: it is drawn inside the list, so the list's StartFilter/QuickLook/Rename handlers stay on the focus path. Fix that focus path (draw the rename input outside the list's action element, or attach the list handlers only when not renaming), so AppKit passes disabled items' keys through. Patch GPUI's `key_to_menu_equivalent` (vendor patch) to map `enter` to `\r` so Rename shows ↩. Add the invariant test: with each text input focused, `/`, Space and Return land in the field. Fall back to proxy actions only if moving the rename field proves hard.
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
- Q2 implemented: **all** list action listeners are attached only while `renaming.is_none()`, so plain keys and AppKit menu equivalents pass to the rename field; `TextInput` now also listens for `CopyFiles`, `CutFiles`, `PasteFiles` and `crow::SelectAll` and maps them to its own text operations; GPUI is patched by `patches/gpui-enter-menu-equivalent.patch` (map `"enter"` to `0x0d`, AppKit shows ↩). The general test `plain_keys_stay_in_every_text_field` types `/` and Space into rename, filter, Go to Folder, search and palette fields and checks list actions are off the rename field's path.
- I1 findings (seen by hand on 2026-11-12, macOS 27.0.1 build 26A434, temporary menus built with `scripts/bundle.sh`):
  1. **Keys shown.** Filter This Folder shows `/` (AX `AXMenuItemCmdChar="/"`, no Command modifier). Quick Look shows a blank space (`" "`). Rename shows the literal string `ENTER` (uppercase, no Command modifier), not a Return glyph. Other keys come from the first Finder-layer binding, as expected (`⌘F`, `⇧⌘A`, `⌘K`, `⇧⌘G`, `⌘[` …).
  2. **Text fields.** With the rename field focused, typing `/` fired Filter This Folder (the rename field closed and the filter opened), and typing Space fired Quick Look (a Quick Look window appeared). Space and `/` never landed in the rename field. In the `/` filter field, `a/b ` typed normally; in the Go to Folder field, `/` and Space typed normally (the list actions are disabled there because their listener is not on the focus path).
  3. **List.** In the list, one `/` keypress called `start_filter` exactly once (temporary `eprintln!` counter, app launched from the terminal: one line). GPUI handles the keystroke before AppKit's menu equivalent.
  4. **Help.** A menu titled exactly `Help` gets AppKit's search field as its first item (an `AXMenuItem` containing an `AXTextField` and a results table). Typing `find` listed Menu Items: `Find…`, `Find Everything…` (plus system help topics).
  5. **AppKit additions.** Edit gained `AutoFill`, `Start Dictation…` (🎤) and `Emoji & Symbols` (⌃⌘Space); View gained `Enter Full Screen` (⌃⌘F); Window gained `Fill`, `Center`, `Move & Resize`, `Full Screen Tile`, `Remove Window from Set`, then the window list. No `Show Tab Bar` / `Merge All Windows` tab items appeared. The Help menu had no extra command items.
  6. **Enabled state.** List focus: New Folder, Rename, Quick Look, Filter This Folder, Move to Trash, Cut, Copy, Paste, Select All, Find…, Back and Show Hidden Files enabled; Find Everything… disabled until I4 adds the `Crow` listener. Sidebar: all list actions disabled; Find…, Back and View enabled. Search field: list actions disabled; Find… and Find Everything… enabled (Search listener). Search results: Quick Look, Move to Trash, Copy and Select All enabled; Paste, Cut, Filter, Rename and New Folder disabled; Find…, Back and View enabled. Palette: list actions disabled; Find…, Back and View enabled; Find Everything… disabled (pre-I4).

## Log
- 2026-11-12: Created `ui-u2-menu-bar`, recorded the plan and current code shape. Read `palette.rs`, `actions.rs`, `main.rs`, `window.rs` search/palette/listener sections, `search.rs` and `text_input.rs`.
- 2026-11-12: I2: rewrote `COMMANDS` in menu-bar order, added menu slots for every Target row plus Q1/Q4/Q8, added the four new command ids, and wired `ShowSearch` (and temporarily `SearchEverything`) in `run_command`. Palette tests pass; new `palette_find_opens_search` passes. Recorded the new empty-`>` order: Crow, File, Edit, View, Go, Window, then palette-only Switch Focus, Remove from Sidebar, Move Favorite Up/Down.
- 2026-11-12: Q2 fix: gated all list listeners on `!renaming`, taught `TextInput` the Crow clipboard/Select All actions, added the GPUI ↩ patch, and added `plain_keys_stay_in_every_text_field` (passes). Menu hand check still pending until the real menus exist.
- 2026-11-12: I1 spike. Built temporary menus (File: Rename/Quick Look/Filter; Edit: Find/Find Everything/Cut/Copy/Paste/Select All; Go: Back; Window; Help) with `scripts/bundle.sh`, inspected them through System Events/AX, typed into the rename, filter and Go to Folder fields, and counted `start_filter` calls in the list. All temporary code reverted; findings above.

## Next
I2: add `MenuName`, `MenuSlot`, `Command.menu`/`palette`, the three new command ids and the Target menu slots to `palette.rs`; then I3 builds the menu. After the real menus exist, hand-check the menu state with the rename field focused.
