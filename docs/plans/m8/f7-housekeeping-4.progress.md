# f7-housekeeping-4 progress
Branch: m8-f7-housekeeping · PR: none · Last updated: 2026-11-17 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 Open Terminal Here | done (commit pending) | `settings.terminal` (default `Terminal`) and a Settings ▸ Behaviour ▸ Terminal app field; `Effects::open_terminal` via `/usr/bin/open -a`, waiting so failures reach the status line; File menu, row/empty-space menus and palette; a file opens its parent, a multi-selection disables the line; no key. Tests: recorder path, palette, menu enable/disable, failure, Settings write. |
| 2 New File/Folder with a row selected | done (b048682) | user: shown folder. Confirmed `current_dir` is the focused column's folder, so the key/dialog/palette already agree; added New Folder/New File to the row menu (`context_menu::ITEMS`); tests: folder and file selections, row menu, palette; the existing filter test covers the visible new row |
| 3 Window drag | done (f551c0a) | `app_owns_titlebar_drag: true` plus a title-box drag surface routed through the two new `Effects` methods (`start_window_move`, `titlebar_double_click`), so the GPUI test platform never hits its `unimplemented!`; test presses the band, a toolbar button and a double-click. No toolbar control sits inside the surface, so controls no longer move the window |
| 4 Board-only details | done (commit pending) | Line folder/clock SVGs (`Glyph::Folder`/`Clock`) for favorites and Recent, tinted `link` or the favorite's color (the redundant color dot is gone); `style::focused_column` darkens the focused column by 10%; columns are a fixed 196 px; the text preview has numbered, syntax-coloured lines (new dependency-free `syntax.rs`, 80-line page). Tests: icon bounds, column width, preview lines, tokenizer. Scope token deferred. |
| 5 NSSound volume hand check | todo (hand check) | no code; needs the bundle, the alert slider low and the user's ears. Steps in `docs/testing.md` ▸ NSSound volume. |
| 6 Empty Trash | done (commit pending) | ⌘⇧⌫ (Crow), File ▸ Empty Trash…, palette; counts off the main thread first, then the sheet; `crow_fs::trash` removes home + every volume `.Trashes/<uid>`, reports failures, plays `Event::EmptyTrash`; fuzz jailed. Tests: crow-fs removal/symlink/missing/failure, sheet/cancel/empty/partial/text-field/palette, sound. |

## Questions
- Q1 (asked 2026-11-17): Terminal.app or iTerm when installed; does a file selection open its parent folder; does it get a key? → answer: "choose a terminal in settings, default Terminal.app (could be any app theoretically …)"; file selection: "parent"; key: "bindable in settings, we need infrastructure for creating custom keybinds anyway … maybe this could be a future action item?" So: a `terminal` setting, no key in F7.
- Q2 (asked 2026-11-17): New File/Folder parent: shown folder (recommended) or selected folder? → answer: "shown".
- Q3 (asked 2026-11-17): which of the five board-only details to build now? → answer: "line-icons", "tint", "columns-196", "syntax"; scope token not selected, and the user asked what "board" meant (the U11 target images in `docs/plans/ui-overhaul/img/`; the token is `top-band-d.png`) → built the four; scope token deferred.
- Q4 (asked 2026-11-17): 196 px columns as a fixed constant or a view setting? → answer: "fixed".
- Q5 (asked 2026-11-17): Empty Trash: ask first; home only or every volume; which focus contexts; undoable? → answer: "ask", "all", "any", "no".

## Decisions
- No vendor patch for item 3: this vendored GPUI has `app_owns_titlebar_drag` (vendor/zed/crates/gpui/src/platform.rs:2675) and `Window::start_window_move` (window.rs:3372), so the plan's conditional Q3 does not arise. Measured while scoping; hand check will confirm traffic lights and double-click.
- Terminal app comes from a new `settings.toml` key (default `"Terminal"`), because the user wants to choose one; no key binding (custom keybind infrastructure is a future item). Recommended in the inbox.
- The search-field scope token is deferred: the user did not select it; it duplicates the existing scope bar (U3/U6 left it out).
- 196 px is a constant, not a setting (user, 2026-11-17).
- Terminal app is a free-text setting (name or app path), not a list of installed apps: the user wants to choose any app, and `open -a` already accepts both. The path is passed as the app's document; a file selection passes its parent.
- No key for Open Terminal Here. Custom keybind infrastructure (detect, display, persist) is a future inbox item, per the user.
- The favorite color dot is gone; the line folder icon now carries the favorite's `color` (the board shows only the icon).
- Empty Trash lives in `crow_fs::trash`, not the undoable `ops` engine: emptying is permanent by definition and has no undo record. It is called off the main thread.
- Empty Trash counts before the sheet so an empty Trash says so without a pointless confirmation; a partial failure is reported as an error and does not stop the rest.
- The fuzz jail is applied to the Trash folders (a fuzz run must never empty a mounted volume's real Trash).
- The syntax highlighter is line-based and covers Rust, TOML, JSON, JS/TS, Python and shell; no crate, theme roles only, at most 80 lines per preview.

## Hand checks
- [ ] Window drag: drag the band's empty area and each toolbar button (Back, Forward, view buttons, Go to, search field) in a wide and a 640 px window; only the empty band should move the window. Traffic lights stay at (14, 19), double-click zooms, light and dark.
- [ ] NSSound: with the macOS alert slider low and output volume normal, play Finder's trash/empty-trash sounds and Crow's (`sound = "finder"` and `"crow"`); report whether Crow is louder than Finder.
- [ ] Empty Trash: with one disposable item in the real Trash, confirm the sheet, check the Finder menu and the sound; do not empty a Trash holding anything needed. Also try a disk image's `.Trashes/<uid>` and a locked item.
- [ ] Open Terminal Here: palette, row menu and empty-space menu; a folder, a file and a multi-selection; set `terminal = "iTerm"` and a missing app.
- [ ] Board details: compare with `docs/plans/ui-overhaul/img/assembled-filter.png`: sidebar line icons, 196 px columns, darker focused column; select a `.rs` file for the numbered syntax preview. Light and dark, 640 px and wide.
- [ ] Settings ▸ Behaviour ▸ Terminal app: type `iTerm`, Return; reopen and check it persists; clear it and Return gives `Terminal`.

## Found along the way
- (none yet)

## Log
- 2026-11-17: Read the plan, M8 index, AGENTS.md, testing doc, effects, actions, palette, context menu, pane new-item/delete paths, sheets and the GPUI titlebar drag API. Branch created. Questions batched.
- 2026-11-17: Item 3: set `app_owns_titlebar_drag` and put the move/double-click on the title's flex box (the band's empty area), through `Effects` so tests record it. `cargo test -p crow-app band_drag` passes. Real AppKit drag, traffic lights and double-click still need the hand check.
- 2026-11-17: Item 2: the shown-folder parent was already the behavior; added New Folder/New File to the row context menu. `cargo test -p crow-app new_items_go_to_the_shown_folder`, `items_menu_offers_new_folder`, `palette_new_folder_runs` pass.

## Next
Run the before-commit checks, commit items 2 and 3, then start item 1 (Open Terminal Here with the new terminal setting).
