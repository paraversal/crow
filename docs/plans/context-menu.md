# Context menu and click-to-deselect

Audience: an implementing agent. Follow `.claude/skills/execute-plan/SKILL.md`; read `AGENTS.md` first.
Base: branch `context-menu` from `main` @ 05deb55 (after Housekeeping, PR 24).
Progress file: `docs/plans/context-menu.progress.md`.
PR title: "Context menu and click-to-deselect". Don't merge.

## Where this comes from (2026-11-14)
The user, after Housekeeping: clicking empty space should deselect like Finder, and there is no context menu (New File, New Folder, …) at all. Line numbers below are as of 05deb55.

## What exists
- **Row presses.** List rows: `pane.rs` `render_table_rows` (≈1696) → `Pane::press` (1345): focuses the list, Cmd toggles, Shift extends, else `FolderModel::click` (`folder.rs:1165`). Column rows: `render_column_rows` (≈2009), same `press(k, …)`. Search results have their own `SearchResults::press` (`search.rs:822`); the sidebar has its own rows (`sidebar.rs`). Only the left button is handled anywhere; nothing listens to `MouseButton::Right`.
- **Empty space.** List view: the `uniform_list("entries")` fills the body (`pane.rs` ≈2530), so space below the last row is inside the list, not on a row. Rows are contiguous; "between rows" only means the 4 px top inset and side gutters. The list's `track_focus` already focuses it on any mouse-down (GPUI `div.rs:2872`). Column view: each column is `("column-view", k)` (≈2169) with a `uniform_list(("column-list", k))`.
- **Selection model.** `FolderModel::clear_selection` (`folder.rs:1219`) clears selection and cursor. Column view's ← already uses it on the column it leaves (`pane.rs:1130`).
- **Actions.** `actions.rs` (keymap), `palette.rs` `COMMANDS` (label, action, `KeyScope`, `MenuSlot`; the menu bar is built from it in `menus.rs`). List actions run on the list's focus path (`pane.rs` ≈2590 `on_action` block), so dispatching an action with the list focused runs it exactly as the key would.
- Already there: Open (`OpenSelection`), Quick Look, Rename, Duplicate, Copy (`CopyFiles`), Cut, Paste, Paste Move ("Move Item Here"), Move to Trash, Delete Permanently, New Folder, New File, Add to Sidebar.
- **Not there:** Copy Path, Show in Finder, Get Info, Open With. The spec's extension points include "Context menu item" (`docs/Crow — Product & Technical Spec.md` ≈271), not built.
- **AppKit.** `crow-macos` already defines Objective-C classes with `define_class!` (`drag.rs:45`, `quick_look.rs:14`); `objc2-app-kit` lacks the `NSMenu`/`NSMenuItem` features today. GPUI builds the menu bar from NSMenu (`vendor/zed/crates/gpui_macos/src/platform.rs` ≈278) but exposes no popup-menu API.

## Items

### C1 Click on empty space deselects
**Change:** a left mouse-down in the file list that lands on no row clears the selection of the folder under the pointer and keeps focus on the list (Finder: focus stays, nothing selected, the cursor is gone, so the next ↓ starts from the first row as it does today with no cursor). Applies to list view (below the last row, top inset, side gutters) and column view (empty part of a column). Clicks on rows, the disclosure triangle, the header, the filter field and the rename field behave as now. Cmd- or Shift-click on empty space does nothing (Finder keeps the selection).
**How:** detect "no row" by position (`drag_row_at` / `column_row_at` already map a pointer to a row) in one mouse-down handler on the list body and on each column, rather than having rows stop propagation (the row's own drag start and the focus transfer listen to the same event). Any rename in progress commits as it does for any click elsewhere.
**ASK Q4 (column view):** what an empty-space click in a column does to the columns to its right.
**Tests (`window.rs`):** `click_below_rows_clears_selection` (list: select two, click below the last row → none selected, list still focused, ↓ selects row 0); `click_on_row_still_selects`; `cmd_click_on_empty_space_keeps_selection`; column-view variant per Q4.
**Done when:** tests pass; feels like Finder in the bundle.

### C2 Context menu model
One table, like `COMMANDS`, built from it: each entry names a `CommandId`, so the title, action and shortcut shown come from the registry and match the menu bar, palette and key hints. Two menus:
- **Items** (right-click on a row or a selection): Open · Quick Look ‖ Rename · Duplicate · Copy · Cut · Paste ‖ Copy Path · Show in Finder · Add to Sidebar ‖ Move to Trash — final list per ASK Q2.
- **Folder** (right-click on empty space): New Folder · New File ‖ Paste ‖ Show in Finder (the folder shown) — final list per Q2.
Items that can't act are shown disabled, not hidden (Paste with an empty clipboard; Add to Sidebar for a file). Pure function `context_menu(kind, state) -> Vec<MenuEntry>` with unit tests.

### C3 Opening the menu
- Right-click and Ctrl-click on a row: an unselected row is selected first (Cmd/Shift ignored, like Finder); a row inside the selection keeps the selection. On empty space: clear the selection (Finder does), then the Folder menu.
- The list takes focus first, so the chosen action dispatches on the list's focus path exactly like its key.
- Escape, a click outside, or the window losing focus closes the menu with no effect.
- How it is drawn: **ASK Q1**.
**Tests:** right-click on an unselected row selects it and opens the Items menu; on a selected row of a multi-selection keeps all; on empty space clears and opens the Folder menu; choosing New Folder creates the folder and starts its rename (through `crow_fs::ops` as the key does); Escape closes. With a native menu the popup itself can't run in GPUI tests: test the menu model, the selection change and the dispatch of the chosen `CommandId` through a seam, and hand-check the popup.

### C4 New commands (per Q2)
Each gets a `CommandId` (stable `key()`), a `COMMANDS` entry with a `MenuSlot`, a palette entry and a key if approved, a keystroke test, README key list. Disk work off the main thread.
- **Copy Path** (Finder: ⌥⌘C): absolute paths of the selection, one per line, to the clipboard.
- **Show in Finder**: `NSWorkspace activateFileViewerSelectingURLs` in `crow-macos`.
- **Get Info** (Finder: ⌘I): opens Finder's Get Info window (AppleScript via `NSAppleScript`, needs an Automation permission prompt) or shows Crow's preview pane. Recommended: leave out for now.
- **Open With ▸**: needs `NSWorkspace URLsForApplicationsToOpenURL`, app icons and a submenu. Recommended: leave out (`after-ui-overhaul.md`).

### C5 Scope and Vim (per Q3)
Sidebar and search results have their own row code; whether they get a menu now is Q3. Vim layer: with a drawn menu `j`/`k` move in it like the palette; with a native menu AppKit owns the keys while it is open. Opening the menu from the keyboard (Finder has none; VoiceOver uses AXShowMenu) is part of Q3.

## ASK
- **Q1 Native NSMenu or GPUI-drawn?**
  - *Native* (`NSMenu popUpMenuPositioningItem:atLocation:inView:` in `crow-macos`, a small `define_class!` target for the chosen item): exact macOS look, behavior, type-to-select, VoiceOver, positioning at screen edges, submenus for free; matches the menu bar. Costs: AppKit runs a nested tracking loop while it is open (GPUI frames pause, e.g. a spinner freezes; drags and file events queue); the popup itself can't be driven in GPUI tests (model and dispatch are tested, the popup is a hand check); themes don't apply; new `objc2-app-kit` features `NSMenu`, `NSMenuItem`.
  - *Drawn* (a GPUI overlay like the palette/sheets): themed, fully testable with `simulate_keystrokes`, Vim keys. Costs: we write keyboard navigation, edge flipping, submenus (if Open With comes later), dismissal, AX roles (`Role::Menu`/`MenuItem`); it will feel slightly un-Mac next to the native menu bar.
  - Recommended: native.
- **Q2 Which items?** Recommended: the lists in C2, plus Copy Path (⌥⌘C) and Show in Finder (no key) as new commands; Get Info and Open With left out.
- **Q3 Scope.** Recommended: list and column view only now; sidebar menu (Remove from Sidebar, Eject, Show in Finder) and search results (Reveal, Copy, Trash) as a follow-up; no keyboard shortcut to open the menu.
- **Q4 Column view empty click.** Finder: clicking empty space in a column deselects in that column and closes the columns to its right (the column becomes the focused one; the preview disappears). Recommended: same.

## Finish
fmt, clippy, tests; README (mouse section: right-click, empty click; new keys), `docs/testing.md` hand checks; progress file "Ready for critique".
