# M9 P9C — Extract palette and window feature modules — progress

Branch: codex/m9-p9c-window-modules · PR: none · Last updated: 2026-11-18 · State: **Ready for critique**
Base: `9d074d6` (M9 P9B)

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 Palette view | done (this commit) | `Palette`, `PaletteMode`, `PathState`, `PaletteDir`, geometry helpers, open/refresh/completion/selection handlers and rows now live in `window/palette_view.rs`; `run_command` and `complete_typed_path` stayed in the parent; palette tests (73) pass |
| 2 Operation UI | done (this commit) | `run_request`, `start`, undo/redo, Empty Trash, sheet dispatch/content, Activity and quit coordination moved to `window/operation_ui.rs`; `Sheet`, `close_sheet`, onboarding handlers and focus surfaces stayed in the parent; focused activity/sheet/quit/trash/undo/first-launch tests pass |
| 3 Path edit | done (this commit) | `PathEdit` and edit/close/confirm/completion handlers moved to `window/path_edit.rs`; `complete_typed_path`, the bottom bar and its hover state stayed in the parent; path_edit/path_bar/completion tests pass |
| 4 Verification | done (this commit) | workspace fmt/clippy/tests green after each move; branch and `main` bundles driven on `target/fixture-100k`; palette open/refresh medians within noise of `main`; 100k first paint/scroll at parity; first run excluded while macOS accessibility activated (67 s stall) |

## Questions

None yet.

## Decisions

- Both the plan document and the M8 → `done/`, M9 plan additions in the working tree are the user's uncommitted docs reorganisation. This branch leaves them untouched and commits only the progress file and code it describes.
- The shared `Sheet` enum stays in `window.rs` (it owns the onboarding variant and focus surface), while sheet dispatch and operation content move to `operation_ui.rs`.
- `close_sheet`, `choose_tips`, `move_onboarding_focus` and `Sheet::Onboarding` construction stay in the parent; onboarding is not an operation concern.
- `run_command` stays in the parent with the action owner so built-in and plugin command routes can converge in P1B; palette rows call it through the parent method.
- `complete_typed_path` stays in the parent because both the palette and the path field call it.

## Hand checks

- [x] Cmd-K on the large fixture (`target/fixture-100k`): branch and `main` bundles, `file-0` typed; identical rows, paths, count ("No places · 10 files") and keycaps. Dark FireBox; light theme left to critique.
- [x] Cmd-K path completion: Shift-Cmd-G `~/Files/projects/file-crow/target/fi`, Tab completed `fixture-100k/` and listed "In fixture-100k"; Esc closed the sheet. Scrim click left to critique (covered by `tests.rs`).
- [ ] Cmd-K command mode: `>` token, running a command, recently used section; Vim layer j/k move across headers.
- [x] Conflict sheet: copy `a.txt` from one scratch folder into another with the same name; sheet showed "An item named “a.txt” already exists…", Keep Both/Replace/Skip, all-row buttons, Cancel/Continue; Esc cancelled.
- [ ] Permanent delete and Empty Trash sheets (tests only, not driven by hand).
- [x] Activity: Option-Cmd-L showed Rename/New Folder/Copy entries newest first, each with Undo; Undo buttons not clicked.
- [ ] Path field: Cmd-L, Tab completion, Return navigates/reveals, Escape restores segments and focus; a missing path beeps and leaves the field open with the bar's message.
- [ ] Quit while an operation runs: sheet appears, Keep Running keeps it, Stop and Quit cancels then quits.
- [x] Compare palette and benches against a `main` bundle built in a separate worktree on the same fixture; the remaining flows were covered by the same tests on both revisions, not side-by-side visually.

## Verification

Workspace `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`: green after each of the three moves. One unrelated timing flake is under Found along the way. Focused filters: `palette` 73, `activity` 1, `sheet` 7, `quit` 3, `trash` 24, `undo` 4, `first_launch` 6, `path_edit` 2 + `path_bar` 11 + `completion` 1.

Bench (same machine/fixture, FireBox, 120 Hz; `target/fixture-100k`; `--bench` after the first accessibility-activation run, which stalled 67 s and was discarded):

| Run | cold first paint | warm first paint | warm load | avg frame | p99 | max |
| --- | --- | --- | --- | --- | --- | --- |
| branch `--bench` | 87 ms | 10 ms | 266 ms | 8.36 ms | 8.60 ms | 20.05 ms |
| `main` `--bench` | 117 ms | 10 ms | 269 ms | 8.35 ms | 8.56 ms | 19.40 ms |
| branch `--bench-filter e` | 116 ms | 8 ms | 262 ms | 8.36 ms | 8.53 ms | 20.20 ms |
| `main` `--bench-filter e` | 117 ms | 8 ms | 261 ms | 8.36 ms | 8.53 ms | 19.79 ms |

Palette on the 100k fixture (throwaway GPUI test: command dispatch, then `refresh_palette` with the palette's folder listing loaded; branch median of 3, `main` median of 5; same absolute fixture path for both):

| Query | branch | `main` |
| --- | --- | --- |
| Cmd-K dispatch | 231.25 ms | 229.72 ms |
| `file-0` | 19.97 ms | 19.97 ms |
| `file-00` | 11.95 ms | 12.00 ms |
| `file-000` | 4.24 ms | 4.32 ms |

The first query is ~20 ms on both revisions; `docs/testing.md`'s 12–15 ms figure comes from the app's `palette refresh` log in a hand check, so the harness numbers are only compared with each other. Parity is exact; the move introduced no measurable change. The throwaway test was removed before the closeout commit.

## Found along the way

- `window::tests::watcher_batches_apply_one_at_a_time` failed once during a loaded workspace run (`crates/crow-app/src/window/tests.rs:11423`, the final row-merge assert) and passed alone and on the next full workspace run. It is a timing-sensitive 100×5 ms wait in an untouched folder-watcher test; recorded here rather than changed, outside this plan's scope.

## Log

- 2026-11-18: Read P9C, the M9 index/finish contract and the current `window.rs` layout (4 242 lines + 11 663 test lines). Created the branch from `9d074d6`; started this progress file.
- 2026-11-18: Moved the palette state, geometry helpers, open/refresh/completion/selection handlers, render rows and the palette-only test accessors to `window/palette_view.rs`. `run_command` stayed in the parent with the action owner; `open_palette` and `refresh_palette` are `pub(super)` for `go_to_folder` and the hidden-setting reload. `cargo check`/`clippy -p crow-app --all-targets` clean; `cargo test -p crow-app palette` 73 passed.
- 2026-11-18: Moved `run_request`, `start`, undo/redo, Empty Trash, sheet dispatch and content, Activity and quit coordination to `window/operation_ui.rs` (module `pub(crate)` for the menu's Quit handler). `Sheet`, `Sheet::focus`, `close_sheet`, `choose_tips`, `move_onboarding_focus` and the onboarding creation stayed in the parent; `operation_ui` calls them. Focused `activity`/`sheet`/`quit`/`trash`/`undo`/`first_launch` tests pass; workspace clippy and tests green.
- 2026-11-18: Moved `PathEdit` and the edit/close/confirm/Tab-completion/preview handlers to `window/path_edit.rs`. `complete_typed_path` stayed in the parent for the palette and path field to share; the bar rendering and `path_hovered` stayed with the bottom bar. `path_edit`, `path_bar` and `completion` filters pass. Workspace checks green after one flaky `watcher_batches_apply_one_at_a_time` failure (recorded under Found along the way).
- 2026-11-18: Verification. `window.rs` went from 4 242 to 2 360 lines; the three child modules are 1 119 + 606 + 193 lines and `window/tests.rs` gained four import lines. New-module string literals are a subset of the old `window.rs` literals (no UI copy changed). Built branch and `main` bundles in separate worktrees, drove the palette, path completion, conflict sheet and Activity on `target/fixture-100k`, and ran `--bench`/`--bench-filter` and the throwaway palette refresh measurement on both; numbers are in Verification. Workspace checks green; temporary test, worktree and scratch folders removed.

## Next

Ready for critique. The critic rebuilds both bundles, finishes the unchecked hand checks (light/dark, Vim command palette, scrim click, path field, permanent delete/Empty Trash, quit while running, VoiceOver) and opens the PR.
