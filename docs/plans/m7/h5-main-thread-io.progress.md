# M7 H5 — No disk I/O on the main thread progress
Branch: `m7-h5-main-thread-io` · PR: #39 · Job: m7-h5-main-thread-io-cc79 · Last updated: 2026-11-16 · State: PR open

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 `pane.rs` `begin_rename` | done (4dfaf68b3b) | `is_dir` now comes from the `Entry`; test `renaming_a_folder_with_a_dot_selects_the_whole_name` |
| I1 `window.rs` `add_to_sidebar` | done (9e6a481618) | uses `Folder::selected_entries` (shared `SelectionEntries`); test `add_to_sidebar_adds_only_the_selected_folders` |
| I1 `sidebar.rs` `folders_of_paths` | done (80b045edec) | `add_dropped_folders` stats in `background_spawn`, then adds on the main thread; existing header/edge drop tests pass |
| I1 `operations.rs` `same_volume` | done (91bb168576) | `drop_request` runs in `background_spawn` from both drop handlers; `same_volume` itself unchanged (unknown → copy) |
| I1 `ui.rs` checked | done | `ui.rs:719-729`, `main.rs:142` and `window.rs:2380` are inside `background_spawn`; `main.rs:251` and `ui.rs:227` are startup; `operations.rs:440` is `FolderTrash` (test/fuzz); `pane.rs:3132` is the fuzz invariant |
| I2 guard | done (28aaf4b986) | `src/io_guard.rs`: scans `crates/crow-app/src`, requires `// io-ok: <reason>`; proved it fails on an unmarked `is_dir()` |
| I3 slow-disk hand check | done (docs) | disk image: attach, browse, ⌘⇧N + rename `a.b`, eject while shown → bottom-bar error, no panic, no beachball. SMB/sleeping disk not available; recorded in `docs/testing.md` |

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
- `docs/testing.md`'s removable-volume paragraph now says what the eject actually does (bottom-bar error, rows stay until navigation) instead of "the folder should show as gone" — observed on the branch; the old claim was not true before H5 either.

## Hand checks
- [x] I3 disk image: attach `/tmp/crow-h5.dmg`, browse `/Volumes/CrowTest`, ⌘⇧N + type `a.b` + Return (the folder appears, preview says "a.b Folder"), eject while shown → `Can't open "/Volumes/CrowTest": No such file or directory (os error 2)` in the bottom bar, no panic (process alive), no beachball; the rows stay until navigation. Log: two `WARN crow::folder: open /Volumes/CrowTest/: No such file or directory` lines, no panic.
- [ ] I3 SMB share / sleeping disk: not available on this machine. The changed paths are the same ones the guard covers; a reviewer with a share should pull the network while it is shown and click around.

## Found along the way
- Ejecting a shown volume leaves its rows in place with an error in the bottom bar (`Folder::reload` keeps the last listing when the open fails). Pre-existing, outside H5's scope; `docs/testing.md` now describes it. Finder shows the volume gone; a later piece may want to clear the rows.
- `sidebar.rs:326-338` `components()` calls `crow_macos::display_components` (Finder names) during render; cached per path. Q2: out of scope.
- `pane.rs:3132` `read_dir` in `listing_violations` runs on the main thread, but only in the fuzzer's invariant check (test/`fuzz`); the docs already call that main-thread work deliberate.
- `main.rs` fixture/bench writers (`--make-fixture`, `--make-search-fixture`, `--bench-search`) stat and write on the main thread before any window opens; startup exception, marked `io-ok: CLI fixture/bench`.

## Log
- 2026-11-16 Read the plan, `AGENTS.md`, `docs/testing.md`, the M7 index. Re-scanned `main` @ 9282594: the four plan sites remain; `ui.rs`, `main.rs:142` and `window.rs:2379` are already in `background_spawn`; `operations.rs:438` is `FolderTrash` (test/fuzz only). Asked Q1–Q4.
- 2026-11-16 Answers: (a) grep test, Q2 no, Q3 copy, Q4 A. Implemented the four sites, one commit each; added the guard and proved it fails on an unmarked call. `cargo test -p crow-app`: 421 passed before the guard; the guard and the two new tests pass after; fmt and clippy clean.
- 2026-11-16 Moved the Add to Sidebar test next to the sidebar tests so the rename and Add commits each carry their own test.
- 2026-11-16 Finish checks: `cargo fmt --check` ✓ · `cargo clippy --workspace --all-targets -- -D warnings` ✓ · `cargo test --workspace` ✓ (421 app, 93 core, 44 fs, 5 macos, 20 search, theme). Bench on `target/fixture-100k`, branch vs `main` @ 9282594 in a worktree: branch p99 8.62 ms, avg 8.36, warm first paint 9 ms; main p99 8.71 ms, avg 8.35, warm first paint 11 ms. Cold first paint 135 vs 112 ms (noise; not on the changed path). Drag/drop and rename are not on the bench path; the bench is a no-regression check only.
- 2026-11-16 `./scripts/bundle.sh` built `target/bundle/Crow.app`; hand check on the disk image as above.
- 2026-11-16 Critique: re-ran fmt/clippy/tests; probed the guard with an unmarked `p.is_dir()` (failed as expected); used the branch bundle on `/tmp/crow-h5-check` (rename `a.b`, Add to Sidebar). Found C1 (pre-existing concurrent `sidebar.toml` writes); Q1 answered fix-now. Added `SidebarEdit::AddMany`, used it in `add_to_sidebar` and `add_folders`, added the core test and the `sidebar.toml` assertion; the app now writes all three folders. C2 docs wording fixed.

## Next
PR open: https://github.com/paraversal/file-crow/pull/39. The user hand-tests and squash-merges after CI; do not merge. Next piece in the M7 order: H7 (accessibility, needs H1), then H6 (editable path bar), then H8 (leftovers).

## Critique
Job: review-m7-h5-e5d4 · Critic session: 2026-11-16 · Base: `main` @ 9282594 · Head reviewed: fe575d8

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | Add to Sidebar with several selected folders writes only the last one. In the branch app on `/tmp/crow-h5-check` (folders `a.b`, `alpha`, `beta`, file `notes.txt`), Cmd-A then Ctrl-Cmd-T left only `beta` in `sidebar.toml` and the sidebar; repeated with a clean config, same. Each `ui::edit_sidebar` spawns a concurrent read-modify-write of `sidebar.toml`, so the last write wins. The base has the same loop, so this is pre-existing, not caused by H5; the new test passes 15/15 because the test executor serializes the writes. | `window.rs:1641`, `sidebar.rs:867`, `ui.rs:789` | UX / code | ask → fix | Q1 → fix-now; fixed (389c74454d) |
| C2 | `docs/testing.md` said "H5 keeps every `stat` off the main thread"; Q2 leaves `crow-macos` icon/kind lookups out of scope and the guard is textual, over `crates/crow-app/src` only. | `docs/testing.md:230` | docs | fix | fixed (897acbd824) |
| C4 | `drop_into` and `drop_on` duplicate the spawn-then-emit block. | `pane.rs:1359`, `sidebar.rs:844` | code sense | note | left (two sites, different events; a helper would cost more than it saves) |
| C5 | The guard's patterns miss `File::open`/`fs::read` behind an import; the module doc says it is a guard, not a proof. | `io_guard.rs:13` | code | note | left (matches the plan's pattern list; the doc is honest) |
| C6 | A slow `same_volume` stat delays the drop operation with no progress label in between. | `pane.rs:1362`, `sidebar.rs:846` | UX | note | left (Q4 A approved; the window keeps drawing, which is the goal) |

Checks re-run: `cargo fmt --check` ✓ · `cargo clippy --workspace --all-targets -- -D warnings` ✓ · `cargo test --workspace` ✓ 590 passed, 1 ignored (421 app, 94 core, 44 fs, 5 macos, 20 search, 6 theme); `io_guard::disk_calls_are_marked_background_startup_or_test_only` ✓, and a probe file with an unmarked `p.is_dir()` failed it with `zz_probe.rs:2`. The progress's "421 app" is the passed count (422 run, 1 ignored `fuzz_long`). Bench on `target/fixture-100k`: branch p50 8.39–8.52 ms, warm first paint 10–16 ms; p95/p99 33.7–34.7 ms from 30 fps pacing in this session, so avg/p99 are not comparable; the agent's clean branch/main runs were ~8.6/8.7 ms p99. Drop and rename are not on the bench path; no regression signal.

Looked at: branch bundle in the app (temp config, dark theme, ~1200×760) on `/tmp/crow-h5-check`: rename `a.b` (whole name selected; typing `c` gave `c`), Add to Sidebar with a mixed selection (before: only `beta` landed → C1; after 389c74454d: all three folders in `sidebar.toml` and the sidebar). Not looked at: light theme, 640 px window, VoiceOver, SMB/sleeping disk (unavailable), drag/drop by mouse (no `cliclick`; the GPUI drop tests cover it).

Verdict: The four main-thread stats are gone and the guard works (probed); the change is a clear improvement. The Add to Sidebar multi-add loss was pre-existing but visible on the changed path and the new test did not catch it; Q1 chose fix-now, and the batch write is in (389c74454d).
