# U7 fix — the user's list after U7

Audience: an implementing agent. Follow `.claude/skills/execute-plan/SKILL.md`; read `AGENTS.md` and `docs/plans/ui-overhaul/README.md` first.
Base: branch `ui-u7-fix` from `main` **after U7 (PR https://github.com/paraversal/file-crow/pull/16) is merged**: X1 changes U7's palette code. ASK before starting if PR #16 is still open.
Progress file: `docs/plans/ui-overhaul/u7-fix.progress.md`.
PR title: "UI U7 fix: palette spacing, caret, search ↓, New File, hide preview, freezes, stale delete". Don't merge.

## Where this list comes from (2026-11-14)
The user's notes after using the app during U7, tagged f (feature), p (performance), ui, b (bug), d (direction). The items that make sense to do now are here. Two went elsewhere:
- **d: a fuzzing script for UI robustness** (erratic input, long names, huge folders, what slows things down) is too big for a fix list: `docs/plans/after-ui-overhaul.md` D1.
- **Edge-case layout tests** (long names, deep paths, huge folders), the "could be tests" half of that note: `u10-finish.md` I4.

X6 and X7 start with finding the cause. If the fix turns out large, ASK whether to move the item to `after-ui-overhaul.md` rather than growing this PR.

The plan's line numbers are as of `ui-u7-palette` @ 1ce2044; re-find them on `main`.

## Items

### X1 Palette: more room under section headers (ui)
**Observed (user):** "not enough padding between 'Recent' (first heading) and the first item".
**Where:** `crates/crow-app/src/window.rs`, `PALETTE_HEADER_ROW` (26) and the header in `render_palette_rows` (`.h(26).pt(8)`, 12 px text). Text plus top padding fill the row, so the label sits about 2 px above the first row. Board B (`img/palette-b.png`) shows about 10 px between "Recent" and the first row's fill.
**Change:** header 34 px: `pt(10)`, text 12 px, `pb(8)`; keep it a single `PALETTE_HEADER_ROW` constant so `palette_row_y` and `palette_content_height` stay right. Compare with board B, and with the first header just under the input row (the list's 6 px top padding plus `pt`). If board B and 34 px disagree visibly, take the board and record it.
**Tests:** extend `palette_rows_are_inset_from_the_sheet` (or add `palette_header_leaves_room_above_the_first_row`): `debug_bounds("palette-header-0")` bottom to `palette-row-0` top ≥ 6 px, and the cursor still scrolls into view (`palette_cursor_skips_headers` stays green).
**Done when:** the gap matches board B by eye in the bundle (dark and Solarized Light).

### X2 Text fields: blinking caret, more room on the left (ui)
**Observed (user):** "writing cursors are not pulsing (they should be) and have too little left-padding".
**Where:** `crates/crow-app/src/text_input.rs`. The caret is a 2 px quad at `x(cursor)` from `bounds.left()`, painted every frame, never hidden (`prepaint` ≈ 816–826). Bare fields (`.bare()`: palette `window.rs`, filter `pane.rs:725`, search `search.rs:199`) have no padding at all. The chrome style (Go to Folder, rename) has `px_2`. With an empty field the caret overlaps the placeholder's first letter (seen in the palette: "|Go to a folder").
**Change:**
1. **Blink.** Visible on focus and after every edit or caret move; then 500 ms on / 500 ms off while focused; not drawn when unfocused. Implement in `TextInput` with a `cx.spawn` timer loop and an epoch counter (each edit or move bumps the epoch and restarts the "on" phase; a stale timer sees a changed epoch and stops). Don't call `cx.notify()` for the whole window: notify only the input entity. First check `vendor/zed/crates/editor` for a `BlinkManager`; reuse its approach, not the crate. Optional: if `crow_macos` can read `NSTextInsertionPointBlinkPeriodOn/Off` from user defaults cheaply, use them; else 500/500 (ASK only if it's not trivial).
2. **Left room.** ASK the user which fields they mean, with a screenshot of each bare field (palette, filter, search) and the chrome field (Go to Folder). Recommended: a 2 px text inset inside every field, so the caret sits before the text instead of on it, plus whatever the user names for the field edge. Keep `scroll_for_caret`'s 2 px margin right after the change (its unit test).
**Edge cases:** IME marked text keeps the caret visible (no blinking while composing); a selection hides the caret as today; the Vim layer's palette `j`/`k` moves don't touch the caret.
**Tests:** `text_input` unit or GPUI test with the test executor: focus → caret visible; `advance_clock(600 ms)` → hidden; type a letter → visible at once; unfocus → hidden. Read the visibility through a `#[cfg(test)]` accessor.
**Done when:** the caret blinks in every field and stops when the field loses focus; the user confirms the left room in the bundle.

### X3 Search field: ↓ moves into the results (ui)
**Observed (user):** "arrow down → go into results list".
**Where:** `search.rs` field subscription (`CompletionRequested => FocusResults`, ≈ 220); `actions.rs` has `down` → `MoveDown` for `SearchResults`, `Filter` and others but nothing for the search field. `window.rs` `SearchEvent::FocusResults` (≈ 1820) focuses the list, or the sidebar when there are no results.
**Change:** ↓ in the search field does what Tab does today, landing on the selected row (row 0 unless the user moved). With no results ↓ does nothing (no bell, and unlike Tab it doesn't jump to the sidebar). ASK: should ↑ on the first row go back into the field? Recommended: yes (Spotlight and Mail do this). Bind it in a context only the search field has; check `keys::context_stack` and the Vim layer (`j` must still type a letter in the field).
**Tests:** `search_down_from_field_focuses_results`: search a fixture, `down` → results focused, cursor row 0, `down` again → row 1. Variant with no results: focus stays in the field. If ↑ is approved: `search_up_from_first_row_returns_to_field`.
**Done when:** tests pass; ↓/↑ feel right in the bundle.

### X4 New File (f)
**User:** "add new file action (Finder doesn't have this!)".
**Where:** `crates/crow-fs/src/ops.rs` (`Request::NewFolder`, 32 / 268 / 578, and its undo), `ops_tests.rs:294`; `pane.rs` `new_folder` (958) and how a new folder enters inline rename; `palette.rs` `CommandId` / `COMMANDS` (with a stable `key()`, e.g. `"new-file"`) and its `MenuSlot` in File after New Folder; `actions.rs` keymap.
**Change:** `Request::NewFile { parent }` goes through `crow_fs::ops` like New Folder: unique name (" 2", " 3" as New Folder does), created with `create_new` (never overwrite), undo moves it to the Trash, then the same inline rename as New Folder. It appears in the File menu, the palette and the keymap from the registry.
**ASK:**
1. **Key.** Recommended ⌥⌘N: free in Crow, and Finder only uses it for New Smart Folder, which Crow doesn't have. Alternative: no key, menu and palette only.
2. **Name.** Recommended "untitled" with no extension, rename selecting the whole name. Alternative: "untitled.txt" with only the stem selected.
**Tests:** `ops_tests.rs`: create, name clash numbering, undo to Trash. `window.rs`: `new_file_creates_and_renames` (key → file exists, rename field open with the name selected, type a name and Return → renamed); the palette lists "New File".
**Done when:** tests pass; README's key list and `docs/testing.md` mention it.

### X5 Hide the preview (f)
**User:** "have ability to disable preview on the right side".
**Where:** list view preview pane (`window.rs` render: `sidebar | center | preview`); column view preview column (`pane.rs`, `PREVIEW_COLUMN_WIDTH`, `render_columns`); `settings.rs` (`DEFAULT_SETTINGS`, `Settings`); `palette.rs` registry (View menu); `toggle_state` (U7) so the palette shows an on/off chip.
**Change:** View ▸ Show Preview / Hide Preview with a toggle chip in the palette. Hidden means no preview pane in list view and no preview column in column view (the last column is the selected folder's children); Quick Look still works.
**ASK:**
1. **Key.** Recommended ⇧⌘P, Finder's own Show/Hide Preview.
2. **Where the state lives.** Recommended: a `show_preview = true` setting in `settings.toml` (documented default, live reload), and the menu toggle writes it through `crow_core::edit_toml` so every window and the next launch follow, like a Finder preference. Alternative: per window, not saved.
**Seam:** U10 I5 (the preview fills the rest of the window) must respect this; a note is added there.
**Tests:** keystroke test toggles it and checks the preview element is gone in list and column view (`debug_bounds` returns None) and back; the settings default test covers the key; live reload of `show_preview = false` hides it.
**Done when:** tests pass; README and `docs/testing.md` updated; bench within noise (column rendering changes).

### X6 UI freezes: searching "f" in ~, first visit of large folders (p/b, measure first)
**Observed (user):** "searching 'f' in ~ freezes the app (and also in general, many UI freezes where there shouldn't be any, especially when opening larger directories for the first time)". Spec principle 2: "No frame ever waits on the disk".
**Not the cause any more:** U6-fix F6 already fixed `insert_rows` (worst batch 55 µs, `u6-fix.progress.md`).
**Hypotheses (check in this order; none confirmed):**
- H1 **Finder icons on the main thread.** `style::icon` → `crow_macos::icon_png` (`crates/crow-macos/src/icons.rs:35–45`) calls `NSWorkspace iconForFile` (disk I/O for bundles and custom icons), renders and PNG-encodes, then `Image::from_bytes` decodes again, all during render. The cache key is per extension, but **per path** for bundles, volumes, home children and folders with an extension (`cache_key`, 21–31). A first visit to a folder with many types or bundles (`/Applications`, `~/Library`), or search results across ~, pays this on the main thread row by row.
- H2 Search: main-thread work per streamed batch other than `add_hits` (preview loading for the moving selection, the bottom-bar count, Spotlight results).
- H3 Listing: anything per entry on the main thread after the background listing lands (sort, kind lookup, `rebuild_rows`).
**Measure:** in a release bundle with a temp config, capture with Instruments Time Profiler (`xcrun xctrace record --template 'Time Profiler' --attach crow`) while (a) searching `f` in a large home folder and (b) opening `/Applications` and `~/Library` cold. Or add a temporary frame-time logger that names the slowest spans. Record the worst frames and their stacks in the progress file **before** changing code.
**Change (if H1):** render icons off the main thread. `icon_png` needs AppKit; check whether `iconForFile` and `NSBitmapImageRep` are safe on a background thread (AppKit docs: NSImage drawing off-main is allowed with care). If they are, run them in `background_spawn` and show a generic folder/document glyph until the icon arrives. If they aren't, ASK. Either way, drop the PNG encode-decode round trip if GPUI can take the bitmap directly.
**Then:** run the bench (first paint and scroll p99 vs `main`) and add a cold-icon case if H1 was the cause.
**Done when:** the cause is written down with evidence; searching `f` in ~ and a cold `/Applications` show no frame over ~50 ms (measured); or the user has moved the fix to `after-ui-overhaul.md`.

### X7 Deleted file stays in the list (b, find the cause)
**Observed (user):** after deleting, the file sometimes stays in the list; deleting it again says it doesn't exist. Not reproduced a second time.
**How the list updates:** nothing refreshes the list when an operation finishes (`operations.rs` `finish`, ≈ 187). The folder model waits for the watcher: `crow_fs::watch_folder` (`watch.rs`) → `FolderChange::Paths` → `Folder::apply_folder_changes` (`crates/crow-app/src/folder.rs:656–685`) → `crow_fs::apply_changes` in the background → `replace_entries`.
**Hypotheses (none confirmed):**
- H1 **Lost update between two change batches.** `apply_folder_changes` clones `self.entries` and applies the batch in the background. If a second batch arrives before the first result lands, the second runs on the *old* entries (still holding the deleted file, and its own `changed` set doesn't name it). Whichever lands last wins, so the deleted entry can come back. The `generation` guard only covers reloads. Deleting a file often causes a second event right after (`.DS_Store`, the Trash, a second selected file), so the window is real.
- H2 **Path form mismatch.** `apply_changes` keeps only `p.parent() == Some(dir)` (`crates/crow-fs/src/snapshot.rs:39`). If FSEvents reports `/private/var/...` for `/var/...`, or a differently normalized (NFD/NFC) name, the change is dropped silently.
**Reproduce first:** H1: a GPUI test that sends two `FolderChange::Paths` batches back to back with the first background apply held back (test executor, or a `#[cfg(test)]` hook that delays it), then checks the deleted path is gone. H2: a `crow-fs` test with a symlinked dir (`/tmp` → `/private/tmp` style) and an NFD name, deleting through the other form.
**Change:** for H1, keep at most one apply in flight and merge queued paths into the next run, which starts from the result of the previous one; or stat in the background but merge on the main thread against the current entries. For H2, compare canonical parents (canonicalize `dir` once when the watch starts) and normalize names the way the listing does. Keep the watcher as the only source (no extra full reloads after every operation) unless the user agrees to one.
**Done when:** the confirmed cause has a failing-then-passing test. If neither hypothesis reproduces: add `debug`-level logging of dropped or ignored watcher paths, write down what was tried, and move the item to `after-ui-overhaul.md` (D1 fuzzing is the natural place to catch it again).

## Finish
- `cargo fmt --check` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo test --workspace`.
- Bench vs `main` (X5 touches column rendering, X6 maybe icons).
- `./scripts/bundle.sh`; hand checks below, recorded in the progress file.
- README (keys: ⌥⌘N, ⇧⌘P; `show_preview`), `docs/testing.md` rows.
- Push `ui-u7-fix`, open the PR (each item: cause or approach, how verified; X6 measurements), don't merge. Progress file state: "Ready for critique" per `execute-plan`.

## Hand checks
| Area | Check |
| --- | --- |
| Palette headers | ⌘K: clear room between "Recent" and its first row, same under Favorites and In; dark and Solarized Light. |
| Caret | Palette, filter, search, Go to Folder, rename: the caret blinks, stays solid while typing, stops when the field loses focus, and doesn't sit on the first letter. |
| Search ↓ | ⌘F, type, ↓: the first result is selected and focused; ↓ moves on; (↑ on the first row returns to the field, if approved). |
| New File | ⌥⌘N in a folder: "untitled" appears in rename; Return keeps it; ⌘Z moves it to the Trash. |
| Preview | ⇧⌘P hides the preview in list and column view and shows it again; a new window follows the setting. |
| Freezes | Search `f` in ~ and open `/Applications` for the first time after launch: no visible stall. |
| Delete | Trash several files quickly one after another and a multiple selection: every row disappears. |
