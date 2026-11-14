# housekeeping progress
Branch: `housekeeping` · Last updated: 2026-11-14 · State: Ready for critique

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
- Expand a folder in list view; drag over its children (no-op), a sibling, a collapsed subfolder, and the empty area below (root); the outline/fill must match the drop.
