# M7 H5 — No disk I/O on the main thread progress
Branch: `m7-h5-main-thread-io` · PR: none · Job: m7-h5-main-thread-io-cc79 · Last updated: 2026-11-16 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 `pane.rs` `begin_rename` | todo | `path.is_dir()` at `pane.rs:1259`; callers: `rename` action (`pane.rs:1251`, has the `Entry`) and `rename_when_present` (`pane.rs:2740`, entry is in the listing) |
| I1 `window.rs` `add_to_sidebar` | todo | `p.is_dir()` at `window.rs:1643`; `Folder::selected_entries()` already returns `(path, is_dir)` (`folder.rs:442`) |
| I1 `sidebar.rs` `folders_of_paths` | todo | `is_dir()` at `sidebar.rs:869`; callers are the two `on_drop` handlers for headers and row edges (`sidebar.rs:940-944`, `1015-1022`) |
| I1 `operations.rs` `same_volume` | todo | `symlink_metadata`+`metadata` at `operations.rs:39-46`; called only at drop time (`pane.rs:1357`, `sidebar.rs:845`), never during drag-over; GPUI's drag cursor is Move/Copy by source window, not volume (`vendor/zed/crates/gpui_macos/src/window.rs:3809-3840`), so the plan's "cursor badge" premise does not hold. Q4 asks how to resolve it |
| I1 `ui.rs` checked | todo | `ui.rs:522-527` (now `719-729`) is inside `background_spawn`; `main.rs:142` and `window.rs:2379` too. Record as checked |
| I2 guard | todo | Q1: grep test (a) or debug assertion (b) |
| I3 slow-disk hand check | todo | disk image + SMB; record in `docs/testing.md` |

## Questions
- Q1 (asked 2026-11-16): guard — (a) CI grep test or (b) debug assertion in `crow_fs`? Recommended: (a).
- Q2 (asked 2026-11-16): are `crow-macos` icon/kind/Finder-name lookups during render in scope? Recommended: no (X6 measured icons ≤ 24 ms cold; `display_components` is cached per path at `sidebar.rs:326-338`).
- Q3 (asked 2026-11-16): `same_volume` unknown → copy or move? Recommended: copy.
- Q4 (asked 2026-11-16): the plan says cache device ids because `same_volume` must answer during the drag. Code: it runs only at drop time, and Crow draws no move/copy badge. Options: (A) resolve move-vs-copy in the background when the drop is planned (recommended); (B) cache dev ids as written.

## Decisions
- Branch from `main` @ 9282594, as the plan names it — plan.
- H5 is next in the M7 order (H4 batch 2 merged in #37) — plan.

## Hand checks
- [ ] I3: create and attach a disk image (`hdiutil create -size 200m -fs APFS -volname CrowTest /tmp/crow.dmg`), browse it, sleep/eject it while shown, then drop files onto it and rename in it: no beachball, no blocking stat on the main thread.
- [ ] I3: an SMB share, pull the network while it is shown, click around: no beachball.

## Found along the way
- `sidebar.rs:326-338` `components()` calls `crow_macos::display_components` (Finder names) during render; cached per path. Q2 covers whether it is in scope.
- `pane.rs:3113` `read_dir` in `listing_violations` runs on the main thread, but only in the fuzzer's invariant check (test/`fuzz`); the docs already call that main-thread work deliberate.

## Log
- 2026-11-16 Read the plan, `AGENTS.md`, `docs/testing.md`, the M7 index. Re-scanned `main` @ 9282594: the four plan sites remain; `ui.rs`, `main.rs:142` and `window.rs:2379` are already in `background_spawn`; `operations.rs:438` is `FolderTrash` (test/fuzz only). Asked Q1–Q4.

## Next
Ask Q1–Q4 (done in the same message), then fix I1 sites 1–3 while waiting; start with `begin_rename`.
