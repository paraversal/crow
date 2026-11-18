# M9 P9C — Extract palette and window feature modules — progress

Branch: codex/m9-p9c-window-modules · PR: none · Last updated: 2026-11-18 · State: in progress
Base: `9d074d6` (M9 P9B)

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 Palette view | done (this commit) | `Palette`, `PaletteMode`, `PathState`, `PaletteDir`, geometry helpers, open/refresh/completion/selection handlers and rows now live in `window/palette_view.rs`; `run_command` and `complete_typed_path` stayed in the parent; palette tests (73) pass |
| 2 Operation UI | done (this commit) | `run_request`, `start`, undo/redo, Empty Trash, sheet dispatch/content, Activity and quit coordination moved to `window/operation_ui.rs`; `Sheet`, `close_sheet`, onboarding handlers and focus surfaces stayed in the parent; focused activity/sheet/quit/trash/undo/first-launch tests pass |
| 3 Path edit | done (this commit) | `PathEdit` and edit/close/confirm/completion handlers moved to `window/path_edit.rs`; `complete_typed_path`, the bottom bar and its hover state stayed in the parent; path_edit/path_bar/completion tests pass |
| 4 Verification | in progress | focused palette/operation/path tests between moves, workspace checks, bundle comparison with `main`, palette refresh and 100k bench |

## Questions

None yet.

## Decisions

- Both the plan document and the M8 → `done/`, M9 plan additions in the working tree are the user's uncommitted docs reorganisation. This branch leaves them untouched and commits only the progress file and code it describes.
- The shared `Sheet` enum stays in `window.rs` (it owns the onboarding variant and focus surface), while sheet dispatch and operation content move to `operation_ui.rs`.
- `close_sheet`, `choose_tips`, `move_onboarding_focus` and `Sheet::Onboarding` construction stay in the parent; onboarding is not an operation concern.
- `run_command` stays in the parent with the action owner so built-in and plugin command routes can converge in P1B; palette rows call it through the parent method.
- `complete_typed_path` stays in the parent because both the palette and the path field call it.

## Hand checks

- [ ] Cmd-K on the large fixture (`target/fixture-100k`): open, type `file-0`/`file-00`/`file-000`; no visible lag, rows and count update (light and dark).
- [ ] Cmd-K path completion: type a folder path, Tab completes the highlighted folder and typed path like Go to Folder; Esc and a scrim click close it and restore focus.
- [ ] Cmd-K command mode: `>` token, running a command, recently used section; Vim layer j/k move across headers.
- [ ] File operation sheet: conflict sheet (Keep Both/Replace/Skip, all-row buttons, Continue/Cancel), permanent delete sheet, Empty Trash sheet.
- [ ] Activity: Alt-Cmd-L, recent entries newest first, per-entry Undo.
- [ ] Path field: Cmd-L, Tab completion, Return navigates/reveals, Escape restores segments and focus; a missing path beeps and leaves the field open with the bar's message.
- [ ] Quit while an operation runs: sheet appears, Keep Running keeps it, Stop and Quit cancels then quits.
- [ ] Compare each flow against a `main` bundle built in a separate worktree on the same folder.

## Found along the way

- `window::tests::watcher_batches_apply_one_at_a_time` failed once during a loaded workspace run (`crates/crow-app/src/window/tests.rs:11423`, the final row-merge assert) and passed alone and on the next full workspace run. It is a timing-sensitive 100×5 ms wait in an untouched folder-watcher test; recorded here rather than changed, outside this plan's scope.

## Log

- 2026-11-18: Read P9C, the M9 index/finish contract and the current `window.rs` layout (4 242 lines + 11 663 test lines). Created the branch from `9d074d6`; started this progress file.
- 2026-11-18: Moved the palette state, geometry helpers, open/refresh/completion/selection handlers, render rows and the palette-only test accessors to `window/palette_view.rs`. `run_command` stayed in the parent with the action owner; `open_palette` and `refresh_palette` are `pub(super)` for `go_to_folder` and the hidden-setting reload. `cargo check`/`clippy -p crow-app --all-targets` clean; `cargo test -p crow-app palette` 73 passed.
- 2026-11-18: Moved `run_request`, `start`, undo/redo, Empty Trash, sheet dispatch and content, Activity and quit coordination to `window/operation_ui.rs` (module `pub(crate)` for the menu's Quit handler). `Sheet`, `Sheet::focus`, `close_sheet`, `choose_tips`, `move_onboarding_focus` and the onboarding creation stayed in the parent; `operation_ui` calls them. Focused `activity`/`sheet`/`quit`/`trash`/`undo`/`first_launch` tests pass; workspace clippy and tests green.
- 2026-11-18: Moved `PathEdit` and the edit/close/confirm/Tab-completion/preview handlers to `window/path_edit.rs`. `complete_typed_path` stayed in the parent for the palette and path field to share; the bar rendering and `path_hovered` stayed with the bottom bar. `path_edit`, `path_bar` and `completion` filters pass. Workspace checks green after one flaky `watcher_batches_apply_one_at_a_time` failure (recorded under Found along the way).

## Next

Verification: build and compare bundles with `main`, run the palette refresh/100k bench on both, then finish the hand-check list.
