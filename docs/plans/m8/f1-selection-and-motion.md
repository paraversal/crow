# F1 — Selection and motion

Audience: an implementing agent. Follow the `execute-plan` skill; read `AGENTS.md` and the M8 index first.
Base: branch `m8-f1-selection` from `main`. Progress file: `docs/plans/m8/f1-selection-and-motion.progress.md`.
PR title: "M8 F1: selection islands, delete focus, removal motion".

## Why

The daily selection interactions still do not match Finder. Deleting a file leaves nothing selected, so the next keystroke has no target. A multi-selection is a stack of separate rounded pills, not one box. A row removed by the file system disappears with no motion even when Full motion is on. Copy and cut look identical. The sidebar's current-folder fill jumps between rows. The tip card covers the last rows.

This piece settles the row rendering that F3 (info pane) and F6 (folder sizes) will touch, so it comes first.

## Current state

- **After a removal, nothing is selected.** `Folder::finish_apply` (`crates/crow-app/src/folder.rs:1324`) deliberately leaves an empty selection empty: the comment at `:1334-1346` says "as in Finder: trashing or moving the selected item leaves nothing selected rather than jumping to the top". Finder actually selects the next row. `prune_selection` (`:1362`) drops gone paths; `settle_cursor` (`:1406`) keeps the cursor on a listed row.
- **Selection is drawn per row.** List view: `pane.rs:1899` (`selection_bg`), `:1967` (`.bg(selection_bg)`). Column view: `:2242`, `:2295`. `draws_selection_fill` (`:2659`) suppresses the cursor row's own fill while the cursor glides in Full motion. Rows are `div`s inside a `uniform_list`, `mx_2`, `rounded_md`, `ROW_HEIGHT` 24.
- **Removal motion exists for operations only.** `RowEffect` and the FLIP shift live at `folder.rs:81-200` and `:640-740`; `window.rs:1432-1439` starts `Fx::Trash`/`Fx::MoveOut` when the operation starts. A watcher batch that removes a row (`replace_entries`, `folder.rs:1160-1200`) marks an existing effect removed but starts no effect for a row that has none, so the gap jumps.
- **Copy/cut state exists, with no visual.** `window.rs:2210` `copy_paths` stores `Operations.clipboard = Some(Clipboard { paths, cut, change_count })` at `:2224`. Rows do not read it.
- **Sidebar fill jumps.** `sidebar.rs:1025` draws `current_location(&theme)` on the current row. U10 deferred the glide.
- **Tip position.** `tips.rs:157` `render_tip`; the card is `absolute().right(12).bottom(bottom_bar::HEIGHT + 8)` (`:179`), so it covers the list's bottom-right rows (U9 deferral).

## Items

### 1 Delete selects the next row (ui)

**Change:** when a removal drops the cursor row, move the cursor to the row now at the same index (the next row); if the removed row was last, select the new last row. If the selection becomes empty, select that row. If other selected rows remain, keep them selected and move only the cursor. Do this in `Folder` after the listing drops the paths, so it covers trash, move, permanent delete and a watcher deletion alike. Do not select the first row of the folder (the old `auto_select` path); only the next/previous row.

**Tests:** trash a middle row → the next name is selected; trash the last row → the previous row is selected; trash a multi-selection → the row after the selection is selected; a watcher `remove` of the cursor row does the same; an external removal of a non-cursor row does not move the cursor.

**Edge cases:** an empty folder after the removal → no selection; a filtered listing → the next *visible* row; an inline folder's child removed → the parent row stays if it is the cursor.

### 2 Finder-style selection islands (ui)

**Change:** compute contiguous runs of selected rows in display order (list and each column view). Draw one rounded fill per island: top rounding on the first row, bottom rounding on the last, square edges between; a 1 px faint rule between rows inside an island (theme role `border` at low alpha, or the existing `inactive_selection` role). Each island gets its own box. Do not change row height, margins or the `uniform_list` item size; the fill must not shift layout.

**Cursor ring:** the `cursor_ring` is an inset shadow (`style::cursor_ring`) and must stay an inset shadow. Recommended: draw it around the whole cursor island, not the single row, so the island reads as one box. If that is awkward with `uniform_list`, keep it on the cursor row and record why. ASK if the visual is ambiguous.

**Tests:** a layout/render-bounds test for one selected row, two adjacent rows, three adjacent rows, and two islands separated by an unselected row; assert the top/bottom radii and the rule count. Check light and dark themes in the bundle; the fill is `active_selection` when the pane is focused and `inactive_selection` otherwise.

**Where:** `pane.rs` `render_table_rows` (`:1899`) and `render_column_rows` (`:2242`). Add a helper `selection_islands(rows, selected) -> Vec<Range<usize>>` next to `draws_selection_fill`; unit-test it in `pane.rs` (UI-free logic could live in `crow-core` if it does not need row types, but the row order is the pane's).

### 3 Complete the row-removal animation (ui)

**Change:** audit the current trash/move FLIP in Full motion first. Then start a leaving effect for a row that disappears from a listing without an operation effect (watcher deletion, permanent delete), using its old row index and entry, so the rows below close the gap. Do **not** animate a sort, a filter change or a folder navigation; those are reorders, not removals. In Low motion the row fades without moving, per the motion table; Off changes state in one frame.

**Tests:** `row_shift` after a watcher removal returns a non-zero shift that reaches zero; a sort with the same entries produces zero shift; a trash still uses `Fx::Trash`; a watcher removal of a row with an existing effect does not start a second effect. Hand check in Full motion on a 20-row folder: delete a middle row, watch the rows close; delete the last row.

### 4 Copy/cut feedback (ui)

**Change:** flash copied rows once (a short tint pulse, using the motion mode's duration; Off = no flash) and dim cut rows until the clipboard changes, a paste completes, undo runs, Escape is pressed, or a new copy/cut replaces them. Read `Operations.clipboard`; do not add a second source of truth. Dimming is a state, not motion: it stays under Off/Low. No layout shift.

**Tests:** copy sets a flash on the selected rows; cut dims them; paste clears the dim; undo clears it; Escape clears it; a new copy replaces the old flash; a 100k copy does not scan the listing per row (the clipboard is app-wide, the visible rows are a set lookup).

### 5 Sidebar current-folder fill glides (ui)

**Change:** animate the current-location fill from the old row to the new row in Full motion (the U10 deferral). Low fades; Off jumps. No layout shift; the row's text and icons do not move. Reuse the motion timing helpers; do not add a second animation engine.

**Tests:** a `Sidebar` probe returns a non-zero offset between rows in Full and zero in Off; the fill lands on the new row; a fast sequence of folder changes does not leave two fills.

### 6 Tip card position (ui)

**Change:** the tip card must not cover the last rows of the list. Recommended: keep it above the bottom bar but add a bottom inset to the list while a tip is on screen, or move the card to the bottom-left over the sidebar. Coordinate with F5 (the tip setting and first-launch popup). No permanent chrome; the tip is transient.

**Tests:** a layout test asserts the list's bottom row is not under the card (or the card is outside the list); hand check at 640 px and wide, light and dark.

## Questions to ASK

1. **Cursor ring on the island or the row?** Recommended: the island, so a multi-selection reads as one box. If `uniform_list` makes that expensive, keep it on the cursor row and record the tradeoff.
2. **Copy flash under `motion = "off"`?** Recommended: no flash under Off (state changes in one frame); cut dimming stays because it is state, not motion.
3. **Does Escape clear the cut dim?** Recommended: yes, along with the existing Escape selection clear (housekeeping 3).
4. **Sidebar glide in this piece?** Recommended: yes; it is small and uses the same motion helpers. If it grows, split it into its own fix list.
5. **Tip position: reserve a bottom inset or move the card?** Recommended: move the card to the bottom-left over the sidebar if it fits; otherwise reserve a bottom inset only while a tip is visible.

## Finish

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- Bench `cargo run --release -p crow-app -- --bench target/fixture-100k` and compare with `main`: this piece touches row rendering, so warm first paint and scroll p99 matter.
- `./scripts/bundle.sh`; hand-check the six items in light and dark, 640 px and wide, an empty folder and the 100k fixture, Vim on and off, and VoiceOver for the selection state (the row roles and `aria_selected` must stay correct).
- Update `docs/testing.md` hand checks for the new selection and motion behavior.
- Commit, push, "Ready for critique". Don't merge.
