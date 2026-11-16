# M7 H5 — No disk I/O on the main thread progress
Branch: `m7-h5-main-thread-io` · PR: none · Job: m7-h5-main-thread-io-cc79 · Last updated: 2026-11-16 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 `pane.rs` `begin_rename` | done (4dfaf68b3b) | `is_dir` now comes from the `Entry`; test `renaming_a_folder_with_a_dot_selects_the_whole_name` |
| I1 `window.rs` `add_to_sidebar` | done (9e6a481618) | uses `Folder::selected_entries` (shared `SelectionEntries`); test `add_to_sidebar_adds_only_the_selected_folders` |
| I1 `sidebar.rs` `folders_of_paths` | done (80b045edec) | `add_dropped_folders` stats in `background_spawn`, then adds on the main thread; existing header/edge drop tests pass |
| I1 `operations.rs` `same_volume` | done (91bb168576) | `drop_request` runs in `background_spawn` from both drop handlers; `same_volume` itself unchanged (unknown → copy) |
| I1 `ui.rs` checked | done | `ui.rs:719-729`, `main.rs:142` and `window.rs:2380` are inside `background_spawn`; `main.rs:251` and `ui.rs:227` are startup; `operations.rs:440` is `FolderTrash` (test/fuzz); `pane.rs:3132` is the fuzz invariant |
| I2 guard | done (28aaf4b986) | `src/io_guard.rs`: scans `crates/crow-app/src`, requires `// io-ok: <reason>`; proved it fails on an unmarked `is_dir()` |
| I3 slow-disk hand check | in progress | disk image + SMB; record in `docs/testing.md` |

## Questions
- Q1 (asked 2026-11-16): guard — (a) CI grep test or (b) debug assertion in `crow_fs`? → answer: **(a) grep test**.
- Q2 (asked 2026-11-16): are `crow-macos` icon/kind/Finder-name lookups during render in scope? → answer: **no** (X6: icons ≤ 24 ms cold; `display_components` cached per path).
- Q3 (asked 2026-11-16): `same_volume` unknown → copy or move? → answer: **copy**.
- Q4 (asked 2026-11-16): the plan said cache device ids because `same_volume` must answer during the drag; code shows it runs only at drop time and Crow draws no move/copy badge. → answer: **(A) resolve the drop in the background** (spawn `drop_request` in the two drop handlers).

## Decisions
- Branch from `main` @ 9282594, as the plan names it — plan.
- H5 is next in the M7 order (H4 batch 2 merged in #37) — plan.
- Q4 A: no `Request::Drop` variant, no dev-id plumbing through `Entry`/`Folder`/`DraggedSelection`; the two drop handlers spawn `drop_request` and emit the request when it returns. The drop highlight clears synchronously; the operation starts one executor hop later — user, 2026-11-16.
- The I2 guard's allowlist is a `// io-ok: <reason>` marker on the call's line or the line above, not a file:line table: line numbers drift, the marker travels with the code. Test modules (from `mod tests {` to EOF) and `fuzz_live.rs` are skipped — user approved (a), mechanism chosen by code convention.

## Hand checks
- [ ] I3: create and attach a disk image (`hdiutil create -size 200m -fs APFS -volname CrowTest /tmp/crow.dmg`), browse it, sleep/eject it while shown, then drop files onto it and rename in it: no beachball, no blocking stat on the main thread.
- [ ] I3: an SMB share, pull the network while it is shown, click around: no beachball.

## Found along the way
- `sidebar.rs:326-338` `components()` calls `crow_macos::display_components` (Finder names) during render; cached per path. Q2: out of scope.
- `pane.rs:3132` `read_dir` in `listing_violations` runs on the main thread, but only in the fuzzer's invariant check (test/`fuzz`); the docs already call that main-thread work deliberate.
- `main.rs` fixture/bench writers (`--make-fixture`, `--make-search-fixture`, `--bench-search`) stat and write on the main thread before any window opens; startup exception, marked `io-ok: CLI fixture/bench`.

## Log
- 2026-11-16 Read the plan, `AGENTS.md`, `docs/testing.md`, the M7 index. Re-scanned `main` @ 9282594: the four plan sites remain; `ui.rs`, `main.rs:142` and `window.rs:2379` are already in `background_spawn`; `operations.rs:438` is `FolderTrash` (test/fuzz only). Asked Q1–Q4.
- 2026-11-16 Answers: (a) grep test, Q2 no, Q3 copy, Q4 A. Implemented the four sites, one commit each; added the guard and proved it fails on an unmarked call. `cargo test -p crow-app`: 421 passed before the guard; the guard and the two new tests pass after; fmt and clippy clean.
- 2026-11-16 Moved the Add to Sidebar test next to the sidebar tests so the rename and Add commits each carry their own test.

## Next
I3: build the bundle, create and attach the disk image, browse it, sleep/eject it while shown, drop and rename on it; record the steps and results in `docs/testing.md`. Then the Finish checks (fmt/clippy/tests, bench vs `main`, bundle, hand checks).
