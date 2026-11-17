# F6 — Folder sizes progress
Branch: m8-f6-folder-sizes · PR: [#54](https://github.com/paraversal/file-crow/pull/54) · Last updated: 2026-11-17 · State: PR open

## Status
| Item | State | Notes |
| --- | --- | --- |
| S1 Spike: measure before choosing | done | numbers below; walk is syscall-bound; logical vs allocated differ 0.5–5%; 0 TCC errors from the shell, 11 dataless files never downloaded |
| S2 The sizer (`crow-fs`) | done | `crow_fs::sizes::{measure, Measured, Cancel}`; 12 tests (nested, hard links, symlink loop, fake device boundary, unreadable subfolder/root, cancellation, packages, bulk/portable agreement); totals match S1 |
| S3 The cache | done | `crow_fs::sizes::Cache` (versioned JSON, inode validation at load, `measured_at` bound, `mark_stale`/`mark_subtree_stale`); `crow_core::cache_dir()` = `~/Library/Caches/crow`; 11 tests. Parent combining deferred (critique Q8); the dead `combined_size`/`touch` API is removed. |
| S4 FSEvents history (`crow-macos`) | done | `crow_macos::fsevents::{watch, Event, EventFlags, current_event_id, volume_uuid}`; one boot-volume stream, folder-level, IDs, `WatchRoot`; 3 macOS tests (live stale, restart replay, UUID). `HistoryDone` is ignored; `RootChanged` advances the id and restarts the stream. |
| S5 Scheduling (`crow-app`) | done | `folder_sizes::{FolderSizes, SizeService, Scheduler}`: visible-range sources, two low-QoS workers, dedupe/order/cancel, cache load/save (bounded debounce), FSEvents consumer, UUID check; 7 scheduler/worker tests. The load validates cached inodes off the main thread. |
| S6 Showing sizes | done | Size column cells (dim stale, fade fresh, `size not calculated` spoken), preview folder summary, bottom-bar selection bytes, Info pane Size row; keystroke test `a_folder_size_appears_after_the_measure`. A symlink to a folder shows `--`, like the Info pane. |
| S7 Sorting by size | done | `apply_folder_sizes` patches folder rows before `sort_entries`; `Pane::sizes_changed` re-sorts once when every visible size is in (Q3); test `size_sort_uses_known_unknown_and_stale_folders` covers known and unknown; stale keeps its known bytes but the test does not mark one stale (noted). |
| S8 Setting and command | done | `folder_sizes = "off" \| "visible"` (default off), live reload, Settings window segmented control, View ▸ Calculate Folder Sizes + palette; parse test and `palette_calculate_folder_sizes_measures_when_off`; README/testing updated. The command is disabled in the menu and hidden in the palette when the setting is `"visible"` (critique Q9). |
| S9 Docs, tests, bench | done | fmt/clippy/workspace tests pass; README/testing updated; bundle built. Bench `target/fixture-100k`: off p99 8.42/8.57 ms, visible 8.57/8.63 ms, warm first paint 6–13 ms (budget 50 ms; p99 budget 25 ms). Hand checks on a fixture and a `/tmp` folder passed; home TCC, iCloud, a 100k folder of folders and VoiceOver remain for the user (see Hand checks). |

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
- Q7 (critique, 2026-11-17): validate cached inodes at load, per visible row, or rely on FSEvents? → answer: **at load** (user).
- Q8 (critique, 2026-11-17): wire S3's parent combining now, defer and delete the dead API, or keep it? → answer: **defer; delete `combined_size`/`touch`** (user).
- Q9 (critique, 2026-11-17): Calculate Folder Sizes while `folder_sizes = "visible"`: disable/hide, measure only visible, or keep every row? → answer: **disable the menu item and hide the palette command** (user).
- Q10 (critique, 2026-11-17): FSEvents `HistoryDone`/`RootChanged`: handle, leave, or drop? → answer: **ignore `HistoryDone`; restart the stream on `RootChanged`** (user).

## Decisions
- Branch `m8-f6-folder-sizes` from `main` @ cb49ae4 — M8 index.
- Stop at "Ready for critique" without pushing; the critic opens the PR — AGENTS.md (wins over the plan's Finish wording).
- `measure` returns `Option<Measured>`: `None` means cancelled or the root itself could not be read; a subfolder that cannot be read is `unreadable` and the rest is measured. The plan wrote `-> Measured`; `Option` is needed so a cancelled walk is never cached or shown (plan's S2 + cancellation).
- `Measured.items` counts every entry below the root (files, symlinks and folders; a package counts once, its contents not as items); `folders` is the directory subset. This makes "packages counted as one item" testable (plan's Target + S2).
- A package's contents count toward its parent's bytes but not its items/folders; measuring the package itself counts its contents normally (plan's S2).
- Hard links count once for bytes by `(dev, file id)`; each directory entry still counts as an item (Finder's item count) (plan's S2).
- `listing::bulk` now exposes `list_raw(dir, times, on_batch)`; the listing asks for times, the sizer does not. One packed-buffer parser, no second copy (code convention; S2).
- S2 re-ran the S1 walk shape on the fixture/repo/home and matched the spike's totals; the fixture's absolute time moved from 86 ms (S1) to ~230 ms on both the old spike code and the sizer, so that is cache/environment, not the refactor (see Log).
- `Cache` stores `bytes`, `items`, `inode`, `measured_at`, `event_id` and `stale` (plan's S3 + S4). The inode is checked against the disk in the background load (`validate_inodes`), so a recreated folder is stale before it is seeded. The `folders`/`unreadable`/`hardlinks`/`used_at` fields and `combined_size`/`touch` were removed when parent combining was deferred (critique Q8); the bound now drops the least recently *measured* entry.
- The cache save debounce is capped at 30 s: FSEvents on the boot volume can signal continuously, and "wait for 2 s of quiet" never fired on a busy machine (critique C1). The test `a_steady_stream_of_signals_saves_at_the_cap_not_never` covers it.
- A measurement records the FSEvents id at the *start* of the walk. If the id moves before the walk finishes, the result is marked stale, because the walk may have passed that folder before the change (critique C2).
- The cache file is `crow_core::cache_dir().join("folder-sizes.json")`; `$CROW_CACHE_DIR` overrides it for tests (Q4 + repo config convention).
- FSEvents uses `objc2-core-services` (already in the lock through `notify`) with `dispatch2`; one stream on `/`, folder-level events (no `FileEvents`), `WatchRoot`, 1 s latency in the app (100 ms in tests). The callback sends `Vec<Event>` over an `mpsc` channel; the `Arc<Callback>` context makes drop safe while a callback is queued (plan's S4).
- The UUID check is a binding (`volume_uuid`) plus `Cache::reset_history`; the app service compares them at startup (S5). `reset_history` clears the stored UUID, so the caller can store the new one; otherwise a changed volume reset the cache again on every launch (critique C3).
- The service is not initialized in `ui::init_for_tests`: its worker threads wake the GPUI test scheduler from a non-test thread. Tests that need sizes call `folder_sizes::init_for_tests` with a fake sizer and `measure_now`; the pane tests draw twice (S5/S6).
- Unknown folder sizes sort as 0; a stale size keeps its known bytes and is dimmed. The list re-sorts once when every visible size is in (Q3); scrolling in more rows repeats it when their sizes arrive.
- The one-off command's sizes show in the Size column even when `folder_sizes = "off"` (that is the command's point); the preview, Info pane and bottom bar stay off until the setting is `"visible"` (plan's S8 + "off behaves as today"). With the setting `"visible"`, the View menu item is disabled and the palette command is hidden (Q9).
- A symlink to a folder shows `--` in the Size column and "size not calculated" to VoiceOver, matching the Info pane; Crow does not follow the link (critique C4).
- The Settings window gets a Folder sizes segmented control (`Off` / `Visible`); the plan only asked for the file, but every setting has a control there and the variant would otherwise be dead code (code convention).

## Hand checks
- [x] Home in list view with the setting on: the bundle opened `/Users/jules`; sizes filled in top first (`.bun` 737.3 MB, `.cache` 1.6 GB, `.cargo` 1.5 GB, `Documents` 88.5 GB, `Downloads` 6.1 GB). `Library`/`Files`/`Movies` were still `--` when the app was quit (the walk was in progress, not an error). The list Preview and the Info tab showed the same size (`.bun`: `Folder · 737.3 MB`). The fixture's logical sums match the files exactly (7.3 MB = 7,340,032 bytes); home was not compared with Finder one by one.
- [x] Edit a file deep in a measured folder: appended 1 MB to `nested/inner/three.bin`; `nested` went 5.2 MB → 6.3 MB within 3 s. Quit the app, append another 1 MB, relaunch: `nested` showed 7.3 MB (cache saved on quit; FSEvents replayed the off change).
- [ ] Scroll a 100k folder of folders quickly: not run; `target/fixture-100k` is flat files. The scheduler's order, dedupe and cancel-on-scroll tests cover the logic; a real 100k-folders scroll is for the user.
- [x] No permission prompt appeared from viewing `~` (bundle over `/Users/jules`). The app read Documents, Downloads and Desktop without a prompt; `~/Library` was still `--` while walking, which is the expected in-progress state or an unreadable folder.
- [ ] An iCloud Drive folder with files not downloaded: not run in the app. S1's shell walk found 11 dataless files and downloaded none; the app's metadata-only walk is the same shape, but the hand check remains.
- [ ] VoiceOver: not run; System Events does not expose the GPUI window. The row and Size cell assertions cover "size not calculated"; a real VoiceOver pass is for the user.
- Light/dark and sizes: Solarized Light and Gruvbox dark, 640 px and ~1000 px. The 640 px list hides the Size column (the existing narrow rule) and the layout does not break; the wide list, Preview tab and Info tab show the sizes.

## Critique
Critic session: 2026-11-17 · Base: `main` @ cb49ae4 · Head reviewed: afb9d2a + fixes 9f8088c/b9d2b34

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | The cache was never saved while the app ran: the 2 s debounce reset on every FSEvents signal, and a boot-volume stream signals continuously. The saver stayed parked and the file only changed on quit. | `folder_sizes.rs` `spawn_saver` | code | fix | fixed (9f8088c): bounded 30 s cap, `a_steady_stream_of_signals_saves_at_the_cap_not_never` |
| C2 | A change during a walk was cached as fresh: the worker stored the event id at completion, so a folder the walker had already passed kept the old bytes. | `folder_sizes.rs` `worker` | code | fix | fixed (9f8088c): the start id is stored and the result is marked stale if the id moved; `an_event_during_a_measure_marks_the_result_stale` |
| C3 | `Cache::reset_history` kept the old volume UUID, so after an event-store UUID change the cache reset on every launch. | `sizes/cache.rs:206` | code | fix | fixed (9f8088c): the UUID is cleared for the caller; `reset_history_forgets_the_uuid...` |
| C4 | A symlink to a folder showed `0 bytes` in the Size column while the Info pane showed `--`; the folder branch fell through to the link's `size = 0`. | `extensions.rs:153`; screenshots `/tmp/crow-hand-link3.png` (0 bytes) and `/tmp/crow-hand-fixed.png` (`--`) | UX/UI | fix | fixed (9f8088c): `--` and "size not calculated"; `a_symlink_to_a_folder_has_no_measured_size` |
| C5 | Cached inodes were stored but never checked, and `fresh`/`combined_size`/`touch` were unused. A folder recreated at the same path could keep the old size; the plan's S3 inode rule was not wired. | `sizes/cache.rs`, `folder_sizes.rs` `start` | code / plan | ask | Q7 → validate at load; Q8 → defer parent combining and delete the dead API; fixed (9f8088c), `validate_inodes`, `validation_marks_a_replaced_or_missing_folder_stale` |
| C6 | View ▸ Calculate Folder Sizes stayed enabled with `folder_sizes = "visible"` and measured every folder row, duplicating the service. | `menus.rs`, `palette.rs`, `window.rs` | plan / UX | ask | Q9 → disable the menu item and hide the palette command; fixed (9f8088c, b9d2b34), menu/palette tests |
| C7 | FSEvents `HistoryDone` and `RootChanged` were decoded but unhandled; the plan's S4 names them. | `folder_sizes.rs` `on_events` | plan | ask | Q10 → ignore HistoryDone, advance past RootChanged and restart the stream; fixed (9f8088c) |
| C8 | The list view's Preview tab still said `Folder · --` for a measured folder while the row, the Info tab and the column-view summary showed the size. | `preview.rs` `contents`, `window.rs` `render_preview_pane` | UX / UI | ask | Q11 → show the size in the preview line and its VoiceOver label; fixed (9f8088c), screenshots `/tmp/crow-hand-preview.png`, `/tmp/crow-home.png`; `a_folder_preview_speaks_the_measured_size` |
| C9 | Every FSEvents batch notified and signalled a save even when no cached entry changed; on a busy boot volume that is a frame and a bounded write every 30 s. | `folder_sizes.rs` `on_events` | code / perf | note | left: the 30 s cap bounds it; returning "changed" from `mark_stale`/`mark_subtree_stale` would remove the idle work |
| C10 | `Scheduler.sizes` is unbounded for the session while the cache is pruned to 20k; a pruned entry can stay fresh in the scheduler until its row leaves. | `folder_sizes.rs` `Scheduler` | code | note | left: only long sessions with many folders; the cache remains the persisted bound |
| C11 | The S7 test name claims stale sizes but the test's map has no stale flag; known and unknown are covered. | `folder.rs:2444` | tests | note | left: stale keeping its bytes is in the Decision; a test would need a seeded service |
| C12 | The Info pane's Items row stays `—` even when the cache has the measured `items` count. | `info_pane.rs:437` | UX | note | left: F3 promised only the Size row; the cache field is available for a later piece |
| C13 | With the setting off, the one-off command's sizes show only in the Size column; the preview, Info pane and bottom bar stay off. | `pane.rs`, `info_pane.rs` | UX | note | left: the agent's Decision and the plan's "off behaves as today"; with the setting visible the command is now disabled |
| C14 | `Cache::prune` is O(n log n) per insert once at 20k, and the bound is least-recently-measured rather than least-recently-shown (`touch` was removed with the deferred combining). | `sizes/cache.rs:140` | code | note | left: the cache is a re-measurable optimization; no measured UI cost |
| C15 | The executing agent's S9 said the app created no window; in this session it did, so the bench and hand checks were run. | `docs/plans/m8/f6-folder-sizes.progress.md` S9 | record | note | corrected: bench off/visible and the hand checks above |
| C16 | With `folder_sizes = "visible"`, a flat folder with no folders still starts the boot-volume FSEvents stream, which then processes every event for the session. | `folder_sizes.rs` `ensure_watcher` | code | note | left: the setting is on, and stopping/restarting per folder would miss history for folders measured later |
| C17 | The sizer's non-macOS `device()` returns 0, so `measure` would count nothing on Linux; Crow is macOS-only and CI runs on macOS. | `sizes.rs` `device` | code | note | left |

Checks re-run: `cargo fmt --check` ✓ · `cargo clippy --workspace --all-targets -- -D warnings` ✓ · `cargo test --workspace` ✓ (536 app, 1 ignored; 101 core; 64 fs; 11 macos; 20 search; 6 theme) · bench `target/fixture-100k`: off p99 8.59/8.59 ms, visible 8.56/8.57 ms, warm first paint 9/10 ms; budgets 50 ms warm and 25 ms p99.
Looked at: the app in Gruvbox dark and Solarized Light, 640 px and ~1000 px, on `/tmp/crow-hand` and the bundle over `/Users/jules`; the Size column (0 bytes, `--`, 7.3 MB), the list Preview and Info tab, the live append (5.2 → 6.3 MB in 3 s), quit/relaunch (7.3 MB), top-first fill on home, and the no-prompt home walk. Could not look at: iCloud not-downloaded files, a 100k folder of folders, and VoiceOver (System Events does not expose the GPUI window); the home `Library`/`Files`/`Movies` were still `--` when the app was quit.
Verdict: better. The feature fills sizes lazily, survives a restart, and no longer contradicts itself between the row, preview and Info tab. The four bugs (save starvation, walk/event race, UUID reset, symlink `0 bytes`) were real; the remaining notes are small. Parent combining is deferred by the user's Q8 answer.

## Found along the way
- F6 critique: cache-save starvation, a walk/event race, the UUID reset loop, symlink-to-folder `0 bytes`, the list preview's `--`, unused inode/combining checks, the one-off command staying enabled when visible, and the unhandled FSEvents flags; see Critique.
- The app's window opened in the critic's session; the executing agent's "no window" was session-specific.
- CI on PR #54: the two GPUI hover patches both edit the test import line and insert a test before `test_focus_handle_release`, so whichever applied second failed on a fresh fetch (CI at `window.rs:8436`). They are combined into `patches/gpui-hover-and-mouse-exit.patch`; a clean `./scripts/fetch-gpui.sh` at `6ceb6fe` applies the set.

## Log
- 2026-11-17: branch `m8-f6-folder-sizes` from `main` @ cb49ae4; read the plan, M8 index and F3's progress file. Started S1.
- 2026-11-17: S1 done. Three walks timed on the fixture, the repo and `~`; `sample` shows `getattrlistbulk` + one `open` per folder. Cold not measurable without root; TCC from the shell is clean but the app's identity differs; 11 dataless iCloud files stayed dataless. Asked Q1–Q6; answers recorded above.
- 2026-11-17: S2 done. Refactored `listing::bulk` to a shared `list_raw` (extra `dev`/`file id`/`link count`; times optional) and added `sizes.rs`. `cargo fmt --check`, `clippy -p crow-fs --all-targets -D warnings`, `cargo test -p crow-fs` (55) pass. The sizer's totals on the fixture/repo/home match S1. Note: the fixture's absolute walk time is now ~230 ms for both the old spike code and the sizer; the S1 86 ms was a cache state, not a code difference.
- 2026-11-17: S3 done. Added `sizes::Cache` (serde JSON, version 1, atomic save, inode check, LRU bound, stale marking, child combining) and `crow_core::cache_dir()`. `cargo test -p crow-fs` is 66 tests.
- 2026-11-17: S4 done. Added `crow_macos::fsevents` with `objc2-core-services`/`dispatch2`; one boot-volume stream, folder-level events, IDs, `WatchRoot`, `volume_uuid`. Three tests pass repeatedly: a live write marks the folder and ancestors stale, a restart replays a write made while off, and the boot volume has a UUID. `cargo test -p crow-macos` is 11 tests.
- 2026-11-17: S5–S8 done. Added the app service (visible sources, two low-QoS workers, cache load/save, FSEvents consumer), the Size column/preview/Info/bottom-bar display, size sorting with the once-only re-sort, and the setting + View ▸ Calculate Folder Sizes. `cargo test -p crow-app` is 528 tests; the settings window has the new control.
- 2026-11-17: S9 checks. `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass. README settings table and `docs/testing.md` updated. Bundle built. The app bench could not run: `target/release/crow --bench target/fixture-100k` starts ("starting theme") but creates no window, so it never reaches a frame; the same happens with the size service disabled, with the bundle (`open -n`), and with the installed Crow.app. The listing bench (no GUI) is 4.3 ms first batch, 184 ms for 100k, 5 ms sorted. The app bench and hand checks are left for the critic/user when the GUI session can open a window.
- 2026-11-17: critique. The window opened in this session; ran the bench off/visible and the hand checks (fixture, home, light/dark, 640/~1000 px). Fixed C1–C8 (code 9f8088c, b9d2b34): bounded cache save, walk/event race, UUID reset, symlink `--`, inode validation + combining deferred, command disabled/hidden, FSEvents flags, list preview size. Tests 536 app / 101 core / 64 fs / 11 macos / 20 search / 6 theme. The home `Library`/`Files`/`Movies` rows were still `--` when quit; iCloud, 100k-folders and VoiceOver remain.
- 2026-11-17: PR [#54](https://github.com/paraversal/file-crow/pull/54) opened from `m8-f6-folder-sizes`; the critic's progress record is this commit.
- 2026-11-17: CI fix. The two GPUI hover patches conflicted on a fresh fetch; combined into `patches/gpui-hover-and-mouse-exit.patch`, verified with a clean `./scripts/fetch-gpui.sh` at `6ceb6fe` (temp tree and the repo) and `cargo check -p crow-app`.

## Next
PR open: [#54](https://github.com/paraversal/file-crow/pull/54). The next piece in the M8 index's order after F6 is **F7 — housekeeping** (`docs/plans/m8/f7-housekeeping-4.md`); request for the executing agent: `execute plan F7`.
