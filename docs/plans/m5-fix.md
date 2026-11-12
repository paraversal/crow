# M5-fix — issues from hand-testing M5

Audience: an implementing agent. Follow `.claude/skills/execute-plan/SKILL.md`; read `AGENTS.md` first.
Branch `m5-fix` from `main` (ASK if PR #5 isn't merged yet). One PR. Progress file: `docs/plans/m5-fix.progress.md`.

This document lists **observed problems and the wanted behavior only**. Causes and fixes are not given: find the cause, record it in the progress file, then fix. "Where to look" is a starting point, not a diagnosis.

---

## I1 Quick Look crashes
- **Observed:** Quick Look (Space) on any file or folder crashes Crow.
- **Wanted:** Space previews the item in the system Quick Look panel; arrow keys while it's open follow the selection; Space again closes it. No crash.
- **Where to look:** `crates/crow-macos/src/quick_look.rs`, `crates/crow-app/src/pane.rs` (`quick_look`, cursor moves calling `update_quick_look`).
- **Done when:** works on a file, a folder, an image and a PDF, from `Crow.app` (`./scripts/bundle.sh --open`). Record what was verified by hand.

## I2 Path bar segments don't navigate
- **Observed:** clicking a folder in the bottom path bar does nothing.
- **Wanted:** clicking a segment goes to that folder (a new history step, so Back returns).
- **Where to look:** `crates/crow-app/src/window.rs` `render_path_bar`.
- **ASK:** what clicking the last segment does when it's the selected *file*.
- **Done when:** a keystroke/mouse test clicks a segment and asserts the folder and Back.

## I3 Column view: Left in the leftmost column
- **Observed:** Left arrow in the leftmost column does nothing.
- **Wanted:** it goes up one directory, with the folder you came from selected.
- **Where to look:** `pane.rs` `move_left`.
- **ASK:** at `/` (bell or nothing?); whether column view must stay on when the parent folder's view setting (`views.toml`) says list.
- **Done when:** keystroke test.

## I4 List view: expandable folders
- **Observed (missing feature):** folders in list view can't be expanded inline.
- **Wanted:** Finder-style disclosure triangles; expanded contents are indented under their folder so the hierarchy is clear. Keyboard:

  | Row | Left | Right |
  | --- | --- | --- |
  | folder, collapsed | go up one directory | expand |
  | folder, expanded | collapse | navigate into that folder |

- **ASK before building:** Left/Right on files and on nested rows; whether expansions persist across navigation; Option-click / Option-arrow expand-all; whether Vim `h`/`l` follow this table; whether selection may span levels (multi-select, copy, trash across expanded folders).
- **Constraints:** no disk I/O on the main thread; the 100k-folder budgets in `docs/testing.md` still hold; every new key behavior gets a keystroke test (AGENTS.md).
- **Where to look:** `pane.rs` (`render_table_rows`, `NAME_INSET` is already reserved for the triangle), `folder.rs`, `crates/crow-core/src/selection.rs`, `actions.rs` (Left/Right are currently bound only in `ColumnView`).
- Largest item: do it last.

## I5 Reordering favorites by dragging
- **Observed:** dragging a favorite upward works; dragging it downward doesn't move it correctly. The outline shown while dragging (meant to show where the item will land) looks wrong.
- **Wanted:** drag in either direction lands exactly where the indicator says; the indicator is a clear insertion line that doesn't shift other rows.
- **Where to look:** `crates/crow-app/src/sidebar.rs` (favorite row `on_drag` / `drag_over::<DraggedFavorite>` / `on_drop`), `crates/crow-core/src/sidebar.rs` (`SidebarEdit::Move` index rules).
- **Done when:** core tests cover moves up, down by one, down by several, across groups and onto itself; hand-test row updated.

## I6 Sidebar row flickers during a folder drag
- **Observed:** dragging a folder over a sidebar item makes that item flicker.
- **Wanted:** a steady drop highlight on the row under the pointer, from Finder and from Crow's own list, on favorites, recent, locations and group titles. Drops keep working.
- **Where to look:** `sidebar.rs` rows (`drag_over::<ExternalPaths>` and the other background styles on the same row). Compare with the list's rows in `pane.rs`, which use the same pattern: check whether they flicker too.

---

## Finish
`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, bench (`cargo run --release -p crow-app -- --bench target/fixture-100k`, record in `docs/testing.md`), update `docs/testing.md` hand-test rows and README keys, `./scripts/bundle.sh`, push, open PR "M5-fix: Hand-test fixes" naming each issue's cause. Don't merge: the user hand-tests.
