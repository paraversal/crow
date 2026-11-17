# F6 — Folder sizes progress
Branch: m8-f6-folder-sizes · PR: none · Last updated: 2026-11-17 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| S1 Spike: measure before choosing | done | numbers below; walk is syscall-bound; logical vs allocated differ 0.5–5%; 0 TCC errors from the shell, 11 dataless files never downloaded |
| S2 The sizer (`crow-fs`) | done | `crow_fs::sizes::{measure, Measured, Cancel}`; 12 tests (nested, hard links, symlink loop, fake device boundary, unreadable subfolder/root, cancellation, packages, bulk/portable agreement); totals match S1 |
| S3 The cache | done | `crow_fs::sizes::Cache` (versioned JSON, inode, `used_at` bound, `mark_stale`/`mark_subtree_stale`, `combined_size`); `crow_core::cache_dir()` = `~/Library/Caches/crow`; 11 tests |
| S4 FSEvents history (`crow-macos`) | done | `crow_macos::fsevents::{watch, Event, EventFlags, current_event_id, volume_uuid}`; one boot-volume stream, folder-level, IDs, `WatchRoot`; 3 macOS tests (live stale, restart replay, UUID) |
| S5 Scheduling (`crow-app`) | todo | |
| S6 Showing sizes | todo | |
| S7 Sorting by size | todo | |
| S8 Setting and command | todo | |
| S9 Docs, tests, bench | todo | |

## S1 measurements (2026-11-17, arm64 macOS 27.2.1)
Throwaway `crates/crow-fs/examples/walk_bench.rs` (deleted after the run). All walks: no symlinks followed, hard links counted once by `(dev, inode)`, no device crossing.

| Tree | Method | Wall (2 runs) | Files/s | Folders/s | Logical bytes | Hardlinks skipped |
| --- | --- | --- | --- | --- | --- | --- |
| `target/fixture-100k` (100k files, flat) | `read_dir` + `lstat` | 151 / 147 ms | 660k–678k | — | 0 | 0 |
| | `getattrlistbulk` + TOTALSIZE | 87 / 86 ms | 1.15M | — | 0 | 0 |
| | `getattrlistbulk` + ALLOCSIZE | 86 / 86 ms | 1.16M | — | 0 | 0 |
| repo (449k files, 11.9k folders) | `read_dir` + `lstat` | 2.96 s | 152k | 4.0k | 18.67 GB | 12,007 |
| | `getattrlistbulk` + TOTALSIZE | 0.87 s | 514k | 13.6k | 18.67 GB (exact match) | 12,007 |
| | `getattrlistbulk` + ALLOCSIZE | 0.87 s | 514k | 13.6k | 19.61 GB | 12,007 |
| `~` (1.56M files, 218.7k folders) | `read_dir` + `lstat` | 27.3 / 27.4 s | 57k | 8.0k | 347.05 GB | 36,141 |
| | `getattrlistbulk` + TOTALSIZE | 21.0 / 19.9 s | 75k–79k | 10.4k–11.0k | 347.04–347.06 GB | 36,141 |
| | `getattrlistbulk` + ALLOCSIZE | 19.5 / 19.5 s | 80k | 11.2k | 338.13 GB | 36,141 |

- **Cold** was not measured: dropping the metadata cache needs `sudo purge`, which this session does not have. The two warm runs are stable (readdir 27.334 vs 27.387 s), so the second run is the warm number; the first is the closest available to cold and is within 0.2 %.
- `sample` of the fastest (bulk-alloc, home, 20.3 s): 2608 samples in `getattrlistbulk`, 1082 in `open` (one per folder), 72 `madvise`, 18 `close`; no user-space hotspot. The walk is syscall-bound, as expected.
- **TCC:** the full `~` walk had 0 errors and did not prompt; `~/Library/Mail`, `~/Desktop` and `~/Documents` were included. This shell runs under the terminal's TCC identity, which already has access, so this does not prove Crow.app will. The sizer treats `EPERM` as `unreadable`; the Crow.app hand check is still required.
- **Dataless:** 11 `SF_DATALESS` files under `~/Library/Mobile Documents`; both passes found the same 11 and the walk never opens file contents, so nothing was downloaded.
- **Q1:** logical (TOTALSIZE) matches Finder and the file rows; allocated differs by 0.5 % (repo +5 %, home −2.6 %).
- **Q5:** one stream for the boot volume: it is one history and one callback, catches changes before a folder is measured, and FSEvents does not read contents, so a protected path in an event is not a TCC read.

## Questions
- Q1 (asked 2026-11-17): logical or allocated size? → answer: **logical** (user).
- Q2 (asked 2026-11-17): default `folder_sizes` off or visible? → answer: **off** (user).
- Q3 (asked 2026-11-17, follow-up on a toggle): re-sort as sizes arrive, or once when visible sizes are in? → answer: **once**; no new toggle (user).
- Q4 (asked 2026-11-17): cache in `~/Library/Caches/crow/` or the state dir? → answer: **Caches** (user).
- Q5 (asked 2026-11-17): one FSEvents stream for the boot volume or one per measured folder? → answer: **one boot-volume stream** (user, from S1).
- Q6 (asked 2026-11-17): network/removable volumes measured or always `--`? → answer: **always `--`**, the one-off command may still measure (user).

## Decisions
- Branch `m8-f6-folder-sizes` from `main` @ cb49ae4 — M8 index.
- Stop at "Ready for critique" without pushing; the critic opens the PR — AGENTS.md (wins over the plan's Finish wording).
- `measure` returns `Option<Measured>`: `None` means cancelled or the root itself could not be read; a subfolder that cannot be read is `unreadable` and the rest is measured. The plan wrote `-> Measured`; `Option` is needed so a cancelled walk is never cached or shown (plan's S2 + cancellation).
- `Measured.items` counts every entry below the root (files, symlinks and folders; a package counts once, its contents not as items); `folders` is the directory subset. This makes "packages counted as one item" testable (plan's Target + S2).
- A package's contents count toward its parent's bytes but not its items/folders; measuring the package itself counts its contents normally (plan's S2).
- Hard links count once for bytes by `(dev, file id)`; each directory entry still counts as an item (Finder's item count) (plan's S2).
- `listing::bulk` now exposes `list_raw(dir, times, on_batch)`; the listing asks for times, the sizer does not. One packed-buffer parser, no second copy (code convention; S2).
- S2 re-ran the S1 walk shape on the fixture/repo/home and matched the spike's totals; the fixture's absolute time moved from 86 ms (S1) to ~230 ms on both the old spike code and the sizer, so that is cache/environment, not the refactor (see Log).
- `Cache` stores `folders`, `unreadable`, `hardlinks`, `used_at` and `stale` in addition to the plan's `bytes`, `items`, `inode`, `measured_at`, `event_id`: the first three are needed to combine child sizes honestly, `used_at` is the bound's LRU key, and `stale` is what FSEvents marks (plan's S3 + S4).
- `Cache::combined_size` refuses to use a child with hard links, because a hard link can cross child folders and would be counted twice; the parent's own hard links are refused too (plan's hard-link rule).
- The cache file is `crow_core::cache_dir().join("folder-sizes.json")`; `$CROW_CACHE_DIR` overrides it for tests (Q4 + repo config convention).
- FSEvents uses `objc2-core-services` (already in the lock through `notify`) with `dispatch2`; one stream on `/`, folder-level events (no `FileEvents`), `WatchRoot`, 1 s latency in the app (100 ms in tests). The callback sends `Vec<Event>` over an `mpsc` channel; the `Arc<Callback>` context makes drop safe while a callback is queued (plan's S4).
- The UUID check is a binding (`volume_uuid`) plus `Cache::reset_history`; the app service compares them at startup (S5), because only it owns the cache and the stream (plan's S4).

## Hand checks
- [ ] Home in list view with the setting on: sizes fill in top first, fade in, and match Finder's Get Info (logical) within rounding.
- [ ] Edit a file deep in a measured folder: that folder's size and its parents update within a couple of seconds. Quit Crow, change something, relaunch: the change shows.
- [ ] Scroll a 100k folder of folders quickly: no stutter, and no work continues for rows that scrolled away (Activity Monitor CPU).
- [ ] No permission prompt appears just from viewing `~`.
- [ ] An iCloud Drive folder with files not downloaded: they stay not downloaded.

## Found along the way
- (none yet)

## Log
- 2026-11-17: branch `m8-f6-folder-sizes` from `main` @ cb49ae4; read the plan, M8 index and F3's progress file. Started S1.
- 2026-11-17: S1 done. Three walks timed on the fixture, the repo and `~`; `sample` shows `getattrlistbulk` + one `open` per folder. Cold not measurable without root; TCC from the shell is clean but the app's identity differs; 11 dataless iCloud files stayed dataless. Asked Q1–Q6; answers recorded above.
- 2026-11-17: S2 done. Refactored `listing::bulk` to a shared `list_raw` (extra `dev`/`file id`/`link count`; times optional) and added `sizes.rs`. `cargo fmt --check`, `clippy -p crow-fs --all-targets -D warnings`, `cargo test -p crow-fs` (55) pass. The sizer's totals on the fixture/repo/home match S1. Note: the fixture's absolute walk time is now ~230 ms for both the old spike code and the sizer; the S1 86 ms was a cache state, not a code difference.
- 2026-11-17: S3 done. Added `sizes::Cache` (serde JSON, version 1, atomic save, inode check, LRU bound, stale marking, child combining) and `crow_core::cache_dir()`. `cargo test -p crow-fs` is 66 tests.
- 2026-11-17: S4 done. Added `crow_macos::fsevents` with `objc2-core-services`/`dispatch2`; one boot-volume stream, folder-level events, IDs, `WatchRoot`, `volume_uuid`. Three tests pass repeatedly: a live write marks the folder and ancestors stale, a restart replays a write made while off, and the boot volume has a UUID. `cargo test -p crow-macos` is 11 tests.

## Next
S5: the app's size service (visible ranges, two low-QoS workers, cancel on scroll, cache load/save, FSEvents consumer), with a fake sizer.
