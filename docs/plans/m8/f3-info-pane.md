# F3 — Info pane (⌘I)

Audience: an implementing agent. Follow the `execute-plan` skill; read `AGENTS.md` and the M8 index first.
Base: branch `m8-f3-info` from `main`. Progress file: `docs/plans/m8/f3-info-pane.progress.md`.
PR title: "M8 F3: info pane (⌘I)".

## Why

⌘I does nothing. Finder's Get Info is a daily action: size, kind, dates, where, tags, permissions. Crow's preview pane shows content but almost no metadata. The user chose a right-side inspector pane, not a separate Get Info window (2026-11-17).

## Current state

- **The right column is the preview.** `pane.rs:76` `PREVIEW_COLUMN_WIDTH = 300`; the preview column is `min_w(PREVIEW_COLUMN_WIDTH)` at `:2600`; `Pane::preview` (`:660`) returns the fetched `Preview` for the cursor path. `show_preview` (`settings.rs`) toggles the column; View ▸ Hide Preview (⇧⌘P) and the Settings window control it.
- **What an `Entry` knows:** `crow-fs/src/listing.rs:9-20`: name, path, `is_dir`, `hidden`, size, modified, created, added. No permissions, owner, group or inode.
- **What `crow-macos` already exposes:** `kind_name`, `tags` (`metadata.rs:62`), `thumbnail_png`, Finder display names, volumes. There is no permissions/owner helper yet.
- **No ⌘I binding.** `actions.rs` has no `GetInfo`/`ShowInfo` action; the context menu deliberately omitted Get Info (`done/context-menu.progress.md` Q2).
- **The settings window** has a `ShowPreview` switch (`settings_window.rs:1146`, `:1269`).

## Items

### 1 The surface (f)

**Change:** add `crates/crow-app/src/info_pane.rs` with an `InfoPane` entity. It shares the right column with preview as two tabs: **Preview** and **Info**. ⌘I toggles the column and selects the Info tab; ⇧⌘P keeps toggling the whole column; clicking a tab switches. In list view the column is the existing right strip; in column view it is the preview column. The pane follows the cursor/selection; a multi-selection shows a summary. No new window, no separate titlebar.

**Tests:** ⌘I opens the column on Info; a second ⌘I closes it (or returns to the previous tab — ASK); ⇧⌘P still hides the column; clicking Preview shows the old preview; the tab choice survives a folder change; the pane is not shown when `show_preview = false` unless ⌘I explicitly opens it (ASK).

**Layout:** the tab strip is 28 px, inside the existing column; no layout shift when switching tabs. Use theme roles only; the tab strip reuses `chrome`/`style` patterns.

### 2 Fields (f)

**Change:** show, in this order, with a label column and a value column:
- **Name** (full name, wraps; the row's display name may be truncated),
- **Kind** (`crow_macos::kind_name`; "Folder" for a folder; package names),
- **Size** (files: `Entry::size`; folders: the F6 size when known, otherwise "Calculating…" or `--`),
- **Where** (parent path, clickable to navigate; `~` for home),
- **Created**, **Modified**, **Added** (the existing `format_time`; full date and time, not the list's short form),
- **Tags** (`crow_macos::tags`, chips or a comma list),
- **Permissions** and **Owner/Group** (ASK; see item 3),
- **Items** for a folder (child count when the child column is loaded, else `--`; do not walk the disk on the main thread).

Multi-selection: **N items**, total size (files; folders per F6), common kind if all the same, common parent if all the same, and the shared tags. Do not show per-item rows for a 100k selection; cap the list and say "and N more" only if the user asks (ASK).

**Tests:** a file, a folder, a package, a symlink, a multi-selection, and a selection with no common parent; the values match `stat`/Finder for a fixture; a size that arrives from F6 updates the pane without a reload.

### 3 Metadata loading (f)

**Change:** load permissions/owner/group and any expensive fields off the main thread. Add `crow_macos::permissions(path) -> Option<Permissions>` (mode bits, owner, group) using `NSFileManager`/`stat`; keep it in `crow-macos`, not `crow-app`. The pane shows the cheap fields at once and fills the expensive ones when they arrive; a stale result for a previous selection is dropped. No disk I/O on the main thread (AGENTS.md).

**Tests:** a unit test for the mode formatting (e.g. `drwxr-xr-x`, `-rw-r--r--`); a pane test with a fake loader; a selection change while loading does not show the old item's permissions.

### 4 Reaching it (f)

**Change:** add `ShowInfo` (⌘I) to `actions.rs`, the View menu, and `palette::COMMANDS` with a stable key. The context menu's Get Info line (if the user wants it) uses the same action. Update the Settings window's preview description to mention the tabs. README keys and settings.

**Tests:** ⌘I from the list, column view, search results and the sidebar (search results and sidebar show the item's info; ASK whether that is in scope); the palette command; the menu item; VoiceOver reads the pane's role and labels.

## Questions to ASK

1. **Second ⌘I closes the column or returns to Preview?** Recommended: closes the column if Info is showing (a toggle), otherwise opens Info. Finder's ⌘I opens a window, so there is no exact convention.
2. **When `show_preview = false`, does ⌘I open the column?** Recommended: yes, explicitly; the setting controls the default, not an explicit ⌘I.
3. **Permissions and owner/group in v1?** Recommended: yes if `NSFileManager` gives them cheaply; otherwise show mode bits from `stat` and defer owner/group.
4. **Multi-selection: summary only, or a capped list?** Recommended: summary only (count, total size, common kind/parent/tags); a 100k selection must not build 100k rows.
5. **Search results and sidebar in scope?** Recommended: yes for search results (the item exists); the sidebar's rows are folders, so the same pane works. If it grows, limit F3 to the file list and note the rest.
6. **Does the Info tab replace the preview or sit beside it?** The user chose "shares the right column with preview (tabs)" on 2026-11-17; confirm the tab strip design.

## Finish

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- Bench `--bench target/fixture-100k`: the pane must not slow first paint or scrolling; compare with `main`.
- `./scripts/bundle.sh`; hand-check a file, a folder, a package, a multi-selection, a 100k selection, light/dark, 640 px and wide, VoiceOver, and a selection change while metadata loads.
- Update README (⌘I, the tab), `docs/testing.md`, and the Settings window copy.
- Commit, push, "Ready for critique". Don't merge.
