# f7-housekeeping-4 progress
Branch: m8-f7-housekeeping · PR: none · Last updated: 2026-11-17 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 Open Terminal Here | todo | user: terminal chosen by a setting, default Terminal.app; a file selection opens its parent; no key (custom keybinds are a future item) |
| 2 New File/Folder with a row selected | done (b048682) | user: shown folder. Confirmed `current_dir` is the focused column's folder, so the key/dialog/palette already agree; added New Folder/New File to the row menu (`context_menu::ITEMS`); tests: folder and file selections, row menu, palette; the existing filter test covers the visible new row |
| 3 Window drag | done (f551c0a) | `app_owns_titlebar_drag: true` plus a title-box drag surface routed through the two new `Effects` methods (`start_window_move`, `titlebar_double_click`), so the GPUI test platform never hits its `unimplemented!`; test presses the band, a toolbar button and a double-click. No toolbar control sits inside the surface, so controls no longer move the window |
| 4 Board-only details | todo | user: line icons, darker focused tint, 196 px fixed columns, syntax preview; scope token deferred (duplicates the scope bar) |
| 5 NSSound volume hand check | todo | needs the bundle and the user's ears; no code |
| 6 Empty Trash | todo | user: confirm sheet; every mounted volume; any focus except sheets/text; not undoable |

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

## Hand checks
- [ ] Window drag: drag the band's empty area and each toolbar button (Back, Forward, view buttons, Go to, search field) in a wide and a 640 px window; only the empty band should move the window. Traffic lights stay at (14, 19), double-click zooms, light and dark.
- [ ] NSSound: with the macOS alert slider low and output volume normal, play Finder's trash/empty-trash sounds and Crow's (`sound = "finder"` and `"crow"`); report whether Crow is louder than Finder.
- [ ] Empty Trash: with one disposable item in the real Trash, confirm the sheet, check the Finder menu and the sound; do not empty a Trash holding anything needed.

## Found along the way
- (none yet)

## Log
- 2026-11-17: Read the plan, M8 index, AGENTS.md, testing doc, effects, actions, palette, context menu, pane new-item/delete paths, sheets and the GPUI titlebar drag API. Branch created. Questions batched.
- 2026-11-17: Item 3: set `app_owns_titlebar_drag` and put the move/double-click on the title's flex box (the band's empty area), through `Effects` so tests record it. `cargo test -p crow-app band_drag` passes. Real AppKit drag, traffic lights and double-click still need the hand check.
- 2026-11-17: Item 2: the shown-folder parent was already the behavior; added New Folder/New File to the row context menu. `cargo test -p crow-app new_items_go_to_the_shown_folder`, `items_menu_offers_new_folder`, `palette_new_folder_runs` pass.

## Next
Run the before-commit checks, commit items 2 and 3, then start item 1 (Open Terminal Here with the new terminal setting).
