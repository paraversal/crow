# m5-fix progress
Branch: m5-fix (based on `m5-sidebar-settings-themes`; PR #5 still open) · PR: none · Last updated: 2026-11-12

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 Quick Look crashes | done (this commit) | Cause: `QLPreviewPanel` loads its preview plugins on a nested run loop, and the panel calls sat inside a GPUI input handler (`handle.update` holds the `App` borrow). GPUI's 100 ms poll task then ran during that loop and `AppCell::borrow_mut` panicked, aborting the process. Fix: every panel call now runs from a fresh main-queue turn (`NSOperationQueue::mainQueue` + `RcBlock`), updates coalesce; a `QLPreviewPanelDelegate` forwards the arrows QL does not use to Crow as `MoveUp/Down/Left/Right` (the panel is key while open), Space stays QL's close key. Verified from the release bundle on a folder, text files, a PDF and a PNG: panel shows each, arrows move the selection, Space closes; no panic, no new crash report. |
| I2 Path bar segments don't navigate | done (this commit) | Cause: `render_path_bar` rendered plain text with no hitboxes or click handlers. Now each segment is a `Role::Button` with a hover background and an aria label; a folder navigates as a history step (clicking the folder you are already in does nothing), a selected file opens with `Pane::open_file`. Also replaced the per-segment `path.is_dir()` stat with the cursor entry's own `is_dir`. Test: `path_bar_segments_navigate_as_history_steps`. |
| I3 Column view Left at leftmost column | done (this commit) | Cause: `move_left` returned when `focused == 0`. Now it pushes the parent in as the new column 0, selects the folder we came from in it and adds a history step; at `/` it does nothing. Also: a pending `views.toml` reload (e.g. the write behind Cmd-3) used to re-apply the new leftmost folder's mode and could flip the pane to list — `apply_view` now leaves the mode alone while a multi-column chain is open. Test: `column_view_left_on_the_leftmost_column_goes_up`. |
| I4 List view expandable folders | todo | Last item; largest. |
| I5 Favorite drag-down reorder | todo | Not diagnosed yet. |
| I6 Sidebar row flicker during folder drag | todo | Not diagnosed yet. |

## Hand checks
- I1 from `target/bundle/Crow.app` (release, `./scripts/bundle.sh`), driven with System Events key codes and screenshots (2026-11-12):
  - folder: the system panel shows the folder icon, name, size and modification date.
  - `a.txt`, `b.txt`: panel shows the text; Down while open moves the selection and the panel follows.
  - `pic.pdf`: panel renders the page; Up/Down while open switch between `pic.pdf` and `pic.png`.
  - `pic.png`: panel shows the image; Space closes it.
  - Process stayed alive; no new entry in `~/Library/Logs/DiagnosticReports` (the 18:28 reports are the deliberate dev-binary repros before the fix). The Finder window that appeared mid-test was the user's own action, not Crow.
- Still a human check for the user: the same list inside a normal session, especially arrow keys while Quick Look is open.

## Questions
- Q1 (asked 2026-11-12, batched): branch base; I2 last-segment click; I3 at `/` and view-mode persistence; I4 key table, persistence, extras. → answers (from the user):
  - Branch base: branch off `m5-sidebar-settings-themes`, PR stacked on that branch.
  - I2: clicking a file segment opens the file; clicking a folder segment navigates as a new history step; if already there, no-op.
  - I3: Left on the leftmost column navigates one folder up — pretend `n-1` was the leftmost column and push it in from the left. At `/` nothing happens. Column view stays on regardless of the parent's `views.toml` mode.
  - I4: Finder-style keys (file: Left = up one directory, Right = nothing; nested row: Left = select parent row, then Left again collapses). Expansion is session/window scoped, collapses on navigation away. Option-click a triangle expands all descendants (no Option-arrow). Vim `h`/`l` follow the table. Selection may span levels and operations act on all selected rows.

## Decisions
- Branch `m5-fix` from `m5-sidebar-settings-themes`, not `main`, because M5 is unmerged (#5) and every fix touches M5 code — user decided.
- Progress file committed before any code — execute-plan skill.
- Quick Look keeps the keyboard: the panel is key, but its delegate forwards the arrows it does not handle to Crow. Doing it the other way (refusing key status) did not work: the panel becomes key anyway.
- In a multi-column chain a `views.toml` reload no longer changes the view mode. Entering a folder can still switch to column view; the running chain is one session, as the user asked for I3.

## Log
- 2026-11-12: Branch created, progress file written. Crash reports investigated (`crow-2026-11-12-174339.ips`, `crow-2026-11-12-171729.ips`): SIGABRT from a Rust panic.
- 2026-11-12: Reproduced from the terminal: `thread 'main' panicked at vendor/zed/crates/gpui/src/app/async_context.rs:66:27: called `Result::unwrap()` on an `Err` value: RefCell already borrowed: attempted mutable App borrow at …; existing mutable borrow group began at vendor/zed/crates/gpui/src/window.rs:2262:22` (the `on_input` handler), followed by `fatal runtime error: failed to initiate panic, error 3, aborting`. The existing borrow is the Space handler; Quick Look's plugin load starts a nested run loop; the `ui::init` 100 ms poll task re-enters during it.
- 2026-11-12: First fix attempt (deferring panel calls to the main queue) removed the crash but the panel took key and arrows did nothing. Probed `QLPreviewPanelDelegate`: it receives keyDown/keyUp NSEvents for arrows (117, 118, 119, 120 key codes) that QL does not handle; Space keyDown is consumed by QL itself (only keyUp reached the delegate).
- 2026-11-12: Final I1 implementation: `crates/crow-macos/src/quick_look.rs` (main-queue dispatch, coalesced updates, delegate, `set_quick_look_key_handler`), `crates/crow-macos/src/lib.rs` exports, `pane.quick_look` registers a handler that dispatches the arrow actions to the window. Bundle hand checks as above.
- 2026-11-12: I2 and I3 implemented with GPUI tests. `column_view_left_on_the_leftmost_column_goes_up` first failed (`mode == List` after Left): Cmd-3's asynchronous config write bumped `views_generation`, and the window observer re-applied the parent's resolved view. Fixed by keeping the mode while a multi-column chain is open.
- 2026-11-12: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` all clean (30 + 38 + 27 + 4 + 6 tests pass).

## Next
I5: find why dragging a favorite downward lands wrong. Re-read `crates/crow-app/src/sidebar.rs` (`on_drag`, `drag_over::<DraggedFavorite>`, `on_drop`, the top-border indicator) and `SidebarEdit::Move` semantics; reproduce with a core test (up, down one, down several, across groups, onto itself) before changing code.
