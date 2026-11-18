# M9 P9C — Extract palette and window feature modules — progress

Branch: codex/m9-p9c-window-modules · PR: none · Last updated: 2026-11-18 · State: **In critique**
Base: `9d074d6` (M9 P9B)

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 Palette view | done (`e3c738a`) | `Palette`, `PaletteMode`, `PathState`, `PaletteDir`, geometry helpers, open/refresh/completion/selection handlers and rows now live in `window/palette_view.rs`; `run_command` and `complete_typed_path` stayed in the parent; palette tests (73) pass |
| 2 Operation UI | done (`84b3a3f`) | `run_request`, `start`, undo/redo, Empty Trash, sheet dispatch/content, Activity and quit coordination moved to `window/operation_ui.rs`; `Sheet`, `close_sheet`, onboarding handlers and focus surfaces stayed in the parent; focused activity/sheet/quit/trash/undo/first-launch tests pass |
| 3 Path edit | done (`79d49dc`) | `PathEdit` and edit/close/confirm/completion handlers moved to `window/path_edit.rs`; `complete_typed_path`, the bottom bar and its hover state stayed in the parent; path_edit/path_bar/completion tests pass |
| 4 Verification | done (`3de3053`) | workspace fmt/clippy/tests green after each move; branch and `main` bundles driven on `target/fixture-100k`; palette open/refresh medians within noise of `main`; 100k first paint/scroll at parity; first run excluded while macOS accessibility activated (67 s stall) |

## Questions

None yet.

## Decisions

- Both the plan document and the M8 → `done/`, M9 plan additions in the working tree are the user's uncommitted docs reorganisation. This branch leaves them untouched and commits only the progress file and code it describes.
- The shared `Sheet` enum stays in `window.rs` (it owns the onboarding variant and focus surface), while sheet dispatch and operation content move to `operation_ui.rs`.
- `close_sheet`, `choose_tips`, `move_onboarding_focus` and `Sheet::Onboarding` construction stay in the parent; onboarding is not an operation concern.
- `run_command` stays in the parent with the action owner so built-in and plugin command routes can converge in P1B; palette rows call it through the parent method.
- `complete_typed_path` stays in the parent because both the palette and the path field call it.
- The palette-only test accessors (`palette_rows`, `palette_row_names`, `palette_row_hidden`, `Palette::row_keys`) moved with the listing they read, so the parent stops importing test-only `Item`/`Row`. Their names and call sites are unchanged.
- `operation_ui` is `pub(crate)` only so `menus.rs` can route the menu's Quit action to `operation_ui::quit`; every other item is private or `pub(super)`.
- Onboarding state, answer recording (`choose_tips`) and focus cycling stay in the parent. Its sheet arm is rendered in `operation_ui` with the other sheet arms because all share the same chrome, focus ring and key context.

## Hand checks

- [x] Cmd-K on the large fixture (`target/fixture-100k`): branch and `main` bundles, `file-0` typed; identical rows, paths, count ("No places · 10 files") and keycaps. Dark FireBox; light theme (Gruvbox Light Hard) checked by critique.
- [x] Cmd-K path completion: Shift-Cmd-G `~/Files/projects/file-crow/target/fi`, Tab completed `fixture-100k/` and listed "In fixture-100k"; Esc closed the sheet. Scrim click covered by `tests.rs`.
- [x] Cmd-K command mode: `>` token, recently used section, and the Theme… command ran from the row. Vim layer: Copy's keycap reads `yy`; `j`/`k` type in the palette field by design (`vim_letters_type_in_every_text_field`), arrows cross headers (`palette_cursor_skips_headers`).
- [x] Conflict sheet: copy `a.txt` from one scratch folder into another with the same name; sheet showed "An item named “a.txt” already exists…", Keep Both/Replace/Skip, all-row buttons, Cancel/Continue; Esc cancelled. Scrim click covered by `clicking_the_scrim_closes_every_overlay`.
- [x] Permanent delete sheet: Cmd-Option-Delete on a scratch file showed "Delete “x.txt” permanently?", Cancel and Delete; Esc cancelled. Empty Trash's empty path said "The Trash is already empty."; the non-empty sheet is test-only.
- [x] Activity: Option-Cmd-L showed Rename/New Folder/Copy entries newest first, each with Undo; Cmd-Z undid the New Folder. Activity's own Undo buttons were not clicked.
- [x] Path field: Cmd-L selected the path; Tab completed the common `fixture-` prefix (several `fixture-*` folders exist); Return navigated to `fixture-f3` and back; Escape restored segments and list focus; a missing path beeped and left the field open with the bar's error.
- [x] Quit while an operation runs: the sheet appeared; Keep Running kept a 3,000-file copy and all files landed; Stop and Quit cut off a 100,001-item copy (70,005 files present) and the app exited.
- [ ] VoiceOver on the moved palette, path field and sheets: no roles, labels or strings changed (new-module string literals are a subset of the old `window.rs` literals), so not driven. Left for the user's hand test.
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
- 2026-11-18: Critique. Compared every moved function, type and constant with the base token-for-token: only `render_palette`'s signature differs (rustfmt line wrapping); no body, string or call changed. Ran workspace checks and the 100k bench, drove the branch bundle at 1200×760 and 640×760 in FireBox and Gruvbox Light Hard, and finished the hand checks. Corrected C1/C2 in this file, noted C3; opening the PR.

## Critique
Critic session: 2026-11-18 · Base: `main` @ `9d074d6` · Head reviewed: `2459007`

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | The hand check said "Vim layer j/k move across headers", but `j`/`k` deliberately type in every text field, including the palette (`vim_letters_type_in_every_text_field`, `crates/crow-app/src/window/tests.rs:4761-4772`); the palette moves with arrows. The unchecked line would never pass as written. | `docs/plans/m9/p9c-window-modules.progress.md` hand checks | plan / record | fix | fixed (critique commit): split the check into the observed command-mode and Vim behavior |
| C2 | The Status rows said "done (this commit)" in the final commit, which only records decisions; the actual moves are `e3c738a`, `84b3a3f`, `79d49dc` and verification `3de3053`. | Status table | record | fix | fixed (critique commit): exact commit references |
| C3 | `operation_ui` is `pub(crate)` to let `menus.rs` reach `quit`; the module path is crate-wide although only `quit` is used outside the window. A private module plus `pub(crate) use operation_ui::quit` would narrow it and keep `window::quit`. Effective API is still one function, so not worth churn now. | `crates/crow-app/src/window.rs:2355`, `crates/crow-app/src/menus.rs:88` | code sense | note | left (effective surface is `quit` only; recorded for the next extraction) |

Checks re-run: `cargo fmt --check` ✓ · `cargo clippy --workspace --all-targets -- -D warnings` ✓ · `cargo test --workspace` ✓ (all binaries, no failures; focused window selectors 88 listed) · `git diff --check` ✓ · branch `--bench target/fixture-100k`: warm first paint 5 ms, avg 8.42 ms, p99 8.60 ms, max 42.02 ms (budget warm < 50 ms, p99 < 1.5 × 8.33 ms frame ✓; compare main from Verification: warm 10 ms, p99 8.56 ms)
Looked at: branch bundle on `target/fixture-100k` at 1200×760 and 640×760, FireBox and Gruvbox Light Hard; palette folder/command modes, path completion and error, conflict, permanent delete, Activity/undo, quit sheet with Keep Running and Stop and Quit. Not looked at: VoiceOver (no labels/roles changed; strings identical) and a non-empty Empty Trash sheet (test-only).
Verdict: better. The moved code is token-identical, the parent keeps window/render/action wiring, tests and hand flows still hit the same code, and the 100k budget is unchanged. The only fixes were record accuracy; no behavior changed.

## Next

Ready for critique. The critic rebuilds both bundles, finishes the unchecked hand checks (light/dark, Vim command palette, scrim click, path field, permanent delete/Empty Trash, quit while running, VoiceOver) and opens the PR.
