# F2 — Typing and keys

Audience: an implementing agent. Follow the `execute-plan` skill; read `AGENTS.md` and the M8 index first.
Base: branch `m8-f2-typing` from `main`. Progress file: `docs/plans/m8/f2-typing-and-keys.progress.md`.
PR title: "M8 F2: visible type-to-select, Enter setting, context menus".

## Why

Typing a name in a folder jumps the selection, but nothing on screen says what was typed. Enter means Rename in the file list and Open in the sidebar and search results; the user wants to choose. Search results and the sidebar still have no context menu. The palette's home row displays the account name but matches only `~`.

## Current state

- **Type-to-select.** `crow_core::TypeSelect` (`crates/crow-core/src/selection.rs:123`) builds a prefix and finds the first matching name; `Folder::type_char` (`crates/crow-app/src/folder.rs:1785`) applies it. `TypeSelect::is_active` exists (`:150`), but the buffer is private and there is no prefix getter. The `/` filter is separate: `StartFilter`, the filter field shows its query.
- **Enter.** `crates/crow-app/src/actions.rs:134` binds `enter` to `Rename` in the `LIST` scope. `cmd-o` (`:132`) and `cmd-down` (`:133`) are `OpenSelection`. The sidebar (`:168`) and search results (`:222`) bind Enter to `OpenSelection`. `KeymapLayer::Vim` adds its own bindings.
- **Context menus.** `crates/crow-app/src/context_menu.rs` builds slots for the file list only; `done/context-menu.progress.md` Q3 deferred search results and the sidebar. Search rows have their own `SearchResults::press` (`crates/crow-app/src/search.rs:822`).
- **Palette home row.** `crates/crow-app/src/palette.rs:712` scores the path label `~`; the row's name is the account short name (`U7` C6). Typing `jul` shows "No folders match 'jul'".

## Items

### 1 Show the type-to-select query (ui)

**Change:** add `TypeSelect::prefix() -> &str` and `Folder::type_select_prefix()`. While a typing run is active (`TypeSelect::is_active`), show the prefix somewhere the user can see it: recommended the bottom bar's left/status slot, as plain text (no new permanent chrome), clearing when the 1 s timeout expires, the folder changes, Escape is pressed, or a name edit starts. The `/` filter keeps its own field; do not merge the two states.

**Tests:** type `no` in a folder → the bottom bar shows `no`; after `TypeSelect::TIMEOUT` it clears; Escape clears it; typing in the filter field does not show it; the Vim layer does not type-to-select (letters are commands).

**ASK:** the user chose "show the prefix" over live filtering on 2026-11-17. If the bottom bar is too quiet, a small overlay above the list is the fallback; do not build both.

### 2 Enter behavior setting (s)

**Change:** add `enter = "rename" | "open"` to `settings.toml`, documented default `"rename"` (Finder). Add an `EnterRow` action and bind `enter` in the `LIST` scope to it; its handler reads `Ui.settings.enter` and dispatches `Rename` or `OpenSelection`. Sidebar and search results keep `OpenSelection`; the Vim layer keeps its own bindings. Add a segmented control to the Settings window under Behaviour, with a one-line description. Live reload; `edit_toml` keeps comments.

**Tests:** with `"rename"`, Enter opens the rename field; with `"open"`, Enter opens the row; the sidebar and search results still open in both modes; the Vim layer is unchanged; the settings window writes only the `enter` line; a reload does not flip the control.

**Edge cases:** a multi-selection with Enter = Open opens every selected item? Finder opens all selected items; recommended yes, `OpenSelection` already handles the selection. With Enter = Rename and more than one selected, Rename is disabled (existing behavior): ring the bell or do nothing, as today.

### 3 Context menus for search results and the sidebar (f)

**Change:** reuse `context_menu.rs` slots for search result rows and sidebar rows. Search results: Open, Reveal in Folder, Quick Look, Copy, Copy Path, Move to Trash (disabled rules as in the file list). Sidebar: Open, Show in Finder, Copy Path, Add to Sidebar / Remove from Sidebar where applicable, Eject for volumes. Use the native `NSMenu` path already in `crow-macos`; keyboard opening is not required (the context-menu critique Q3 said no key).

**Tests:** `choose_context_item` drives each menu; disabled items are disabled; the chosen action runs once; the file list menu is unchanged.

**ASK:** the exact item set, especially Add to Sidebar/Remove and Eject. Recommended: a small Finder-like set, no new commands.

### 4 Palette home row matching (ui)

**Change:** score both the displayed name and the path label for `FolderCandidate`/`Item::Folder`, so `jul` finds the home row and `~` still does. Keep the path label in the second line. This is the U7 C6 follow-up.

**Tests:** typing `jul` shows the home row; typing `~` still shows it; the match ranges highlight the part that matched (name or path); other folder rows are unchanged.

**ASK:** include in F2 or leave as a note? Recommended: include; it is a small matcher change and the row already displays the name.

## Questions to ASK

1. **Type-to-select display: bottom bar or overlay?** Recommended bottom bar, left of the hints; no new permanent chrome.
2. **Enter default:** the user chose Rename on 2026-11-17. Confirm the setting name `enter` and the two values `"rename"` / `"open"`.
3. **Context menu item set:** confirm the search and sidebar lists above; no new commands.
4. **Palette home-row matching:** include in F2 (recommended) or defer.

## Finish

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- Bench not needed unless the palette matcher change touches the 100k path; if it does, compare with `main`.
- `./scripts/bundle.sh`; hand-check typing in a folder, both Enter modes, both context menus, the palette home row, light/dark, 640 px, Vim on/off, VoiceOver for the new menu items and the bottom-bar query.
- Update `docs/testing.md` and the README keys/settings tables.
- Commit, push, "Ready for critique". Don't merge.
