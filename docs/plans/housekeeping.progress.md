# housekeeping progress
Branch: `housekeeping` · Last updated: 2026-11-14 · State: PR open · PR: https://github.com/paraversal/file-crow/pull/24

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1. Crow stays running with no windows | done | Removed the main app's quit-on-last-close hook; `Application::on_reopen` opens the launch folder (or activates the front window). `may_close`/`AfterStop` deleted; closing the last window leaves a running operation alone. |
| 2. Promote an in-window drag on deactivation | done | GPUI `Window::promote_active_drag_to_platform`; `CrowWindow` observes window activation and promotes when it loses key. Test: `deactivating_the_window_promotes_an_active_drag`. |
| 3. Escape cancels an in-window drag | done | GPUI `AnyDrag.origin`, `Window::cancel_active_drag`, 200 ms `DragReturn` fly-back; `ui::install_drag_cancel` intercepts Escape before action bindings. Test: `escape_cancels_a_drag_and_returns_the_combo`. |
| 4. Expanded-folder drop targets | done | `Folder::row_ends`/`row_parents` (O(1) parent/end), `Pane::DropTarget`, row handlers on nested rows, outline around an expanded target block, fill for a collapsed target. Four tests. |
| 5. Checks | done | fmt, clippy `-D warnings`, full workspace tests, 100k bench. Hand checks below. |

## Decisions
- Reopen uses the directory Crow was launched on (`start`), not the last visited folder.
- Closing the last window does not stop a running operation; ⌘Q with no window cancels it cleanly, waits for `Operations.running` to clear, then quits. With a window, the existing "Stop and Quit" sheet is unchanged.
- Native promotion happens in `observe_window_activation` when the window stops being key. A drag without an external payload source (sidebar reorder) stays a GPUI drag.
- `parent_row` is now precomputed in `rebuild_rows`; the old backward scan was O(rows) and `render` recomputes the drop target every frame.
- The expanded outline is a plain absolute div (no id/handlers), so it does not intercept drops; the row handlers still win.
- GPUI changes are one patch: `patches/gpui-drag-lifecycle.patch` (`app.rs`, `elements/div.rs`, `window.rs` only).

## Verification
- `cargo fmt --check` ✓
- `cargo clippy --workspace --all-targets -- -D warnings` ✓
- `cargo test --workspace` ✓ (317 app, 87 core, 30 fs, 4 macos, 18 search, 6 theme)
- Bench 100k (`target/fixture-100k`): p99 8.61 ms, warm first paint 8 ms; U11's `main` baseline p99 8.61 ms. No regression.
- Patch round-trip: `git -C vendor/zed apply --reverse --check` and forward apply both succeed.

## Hand checks still open (macOS)
- Close the last window: Crow stays in the Dock; Dock click and ⌘N open a window; ⌘Q quits.
- Start a drag, bring another app/window forward without leaving Crow's bounds; the combo must become a native drag.
- Escape during an in-window drag and during a native-promoted drag: both fly back.
- Expand a folder in list view and drag one of its children: over its siblings nothing is outlined (no-op); over a sibling folder that row is outlined; over a top-level file or the empty space below, the whole list is outlined; over the expanded folder from outside, the folder's whole block is outlined. No target ever shows a fill.
- Sidebar: drag a file over a favorite folder and over the Favorites title: both get the same outline, and the current-folder fill stays visible under it.
- Same in column view (⌘3): over a folder row that row is outlined; over a file row or a column's empty space, that column is outlined; dragging an item over its own column outlines nothing.
- Escape mid-drag, then let go of the mouse without pressing anything else: the combo flies back and disappears on its own (no second Escape).
- Close every window, start a long copy first if you can (or drag a big folder to another volume, then ⌘W): ⌘Q opens a window at the launch folder with the Stop and Quit sheet; Esc keeps the copy running.
- Bench: `cargo run --release -p crow-app -- --bench target/fixture-100k` against `main` (the critic's run stalled with the display not drawing, so no numbers from the critique).

## Critique
Critic session: 2026-11-14 · Base: main @ a219393 · Head reviewed: a64da95

No plan document exists for this branch (only this progress file); the items above were checked against their own descriptions and the user's bug reports during the critique.

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | The "where does a drop on this row land" rule was written twice (row handlers and `drop_target_for_row`) | `pane.rs` | code sense | fix | fixed (a0366e7): `Folder::drop_header` |
| C2 | `row_ends`/`row_parents` were only tested one level deep | `folder.rs` rebuild_rows | code | fix | fixed (9039eb5): two-level test, fails when the subtree-end rule is broken |
| C3 | After Escape the combo stayed on screen at its origin until another key redrew the window: the frame that saw the flight finished still drew it and requested no further frame. The test hid it by forcing a draw every frame (user report, bug 2) | GPUI patch, `Window::prepaint` | code / UX | fix | fixed (0cfadd6), test now fails on the old patch |
| C4 | No mark for a drop into the folder being shown, in list (bug 1) or column view (bug 3) | `pane.rs` | UX | ask | Q1 → outline the list / the column (6d9cefd) |
| C5 | No-op targets were outlined (a child dragged over its siblings outlined its own folder) | `pane.rs` | UX | ask | Q2 → mark only drops that act (6d9cefd) |
| C6 | Folder-row drop fill read like the selection fill (bug 4) | `pane.rs` | UI | ask | Q3 → outline only, list and column (6d9cefd) |
| C7 | ⌘Q with no window and a running operation cancelled it silently (agent decision) | `window.rs` quit | UX | ask | Q4 → open a window at the launch folder and ask (cc1a8b9) |
| C8 | `deactivating_the_window_promotes_an_active_drag` builds its own probe view with its own observer, so it tests the GPUI patch, not `CrowWindow`'s subscription; a CrowWindow test can't see the handoff because the test platform's `start_external_drag` returns false and its setter is `pub(crate)` | `window.rs` tests | code | note | left: needs one more GPUI patch line; covered by a hand check |
| C9 | The sidebar still marked drop targets with the old fill, now different from list and column | `sidebar.rs` | UI | ask | user: same outline here too (see the commit after cc1a8b9); the fill helper is gone, `style::drop_outline` is shared |
| C10 | ⌘N with no window opens `$HOME`, the Dock and ⌘Q open the launch folder; same folder unless Crow was started with a path argument | `menus.rs:89` | UX | note | left |

User decisions (2026-11-14): Q1 outline the whole list or column for drops into the shown folder · Q2 mark a target only when the drop would act · Q3 one accent outline for every target, no fill · Q4 ⌘Q with no window opens one and asks.
New GPUI API: `App::active_drag_value::<T>()` (3 lines, in `gpui-drag-lifecycle.patch`), to read the dragged paths for Q2.

Checks re-run: fmt ✓ · clippy ✓ · tests ✓ (320 app, 87 core, 30 fs, 4 macos, 18 search, 6 theme) · bench not measured: the bench window stalled (one frame of 377 s, display not drawing), left as a hand check
Looked at in the app: nothing. The changes are drag-in-progress states (outlines, fly-back, native promotion) that couldn't be driven here; drop targets and outlines were checked through GPUI tests that read rendered bounds. Everything visual is in the hand checks above.
Verdict: better. Crow behaves like a resident Mac app, drags can be cancelled and promoted, and every drop target now looks the same and only appears when the drop would do something. The open risks are visual and timing details only a hand test can confirm.
