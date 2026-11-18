# M9 P9C — Extract palette and window feature modules — progress

Branch: codex/m9-p9c-window-modules · PR: none · Last updated: 2026-11-18 · State: in progress
Base: `9d074d6` (M9 P9B)

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 Palette view | in progress | move `Palette`, `PaletteMode`, path listing state, helpers, handlers and rows to `window/palette_view.rs`; keep `palette.rs` matching/commands and `run_command` with the action owner |
| 2 Operation UI | todo | move request/sheet dispatch, conflict and Activity presentation, result application and stop/quit coordination to `window/operation_ui.rs`; keep onboarding and shared modal ownership in the parent |
| 3 Path edit | todo | move `PathEdit` and its handlers to `window/path_edit.rs`; keep the shared `complete_typed_path` helper and bottom-bar rendering in the parent |
| 4 Verification | todo | focused palette/operation/path tests between moves, workspace checks, bundle comparison with `main`, palette refresh and 100k bench |

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

- Nothing yet.

## Log

- 2026-11-18: Read P9C, the M9 index/finish contract and the current `window.rs` layout (4 242 lines + 11 663 test lines). Created the branch from `9d074d6`; started this progress file.

## Next

Move the palette state, geometry helpers, handlers and rows to `window/palette_view.rs`, then run the named palette tests.
