# F6 — Folder sizes

**When:** M8, piece F6. Carry it out with the `execute-plan` skill and ASK the questions at the end before the items that depend on them. This plan was written before M7; re-check the line numbers and the 100k budget against `main` when starting. F6 touches the Size column and the bottom bar, so do not run it concurrently with F1 (row rendering) or F3 (right column). F3's info pane shows a folder's size when F6 has one.

## Why
The Size column shows `--` for every folder (`crates/crow-core/src/extensions.rs:119`), so a list sorted by size puts folders in an arbitrary order, and the user can't see where space goes without a separate tool. Finder can "Calculate all sizes" in list view, but it walks again every time and stays slow on large trees.

## What doesn't work, and why
Two ideas from the first discussion (2026-11-14) don't hold up:
- **A folder's modification date won't tell us its size changed.** On APFS a directory's mtime changes only when an entry *directly* inside it is added, removed or renamed. Editing a file in it, or anything deeper, leaves it unchanged. A cache checked against folder mtimes would go stale silently.
- **A shallow pre-index (`tree -L 2` of `/`, `-L 3` of `~`) gives no sizes.** A folder's size is the sum of everything below it at every depth, so an index of `/` means walking the whole disk: millions of entries, TCC prompts for protected folders, and work for folders the user never opens.

## Target
- Folder sizes are measured **lazily, for the folders on screen**, in the background, nearest the top of the view first. A folder shows `--` until its size is known, then the size fades in like other U10 transients.
- Sizes are **cached on disk** and kept correct with **FSEvents history**. macOS records file-system changes with event IDs and can replay everything since a stored ID, even across restarts and while Crow wasn't running. On launch Crow replays since its last ID and marks the changed folders and their ancestors stale. While running, it does the same live.
- Counting matches Finder: each hard-linked file once, symlinks not followed, other volumes not crossed, packages counted as one item, and cloud placeholder files never downloaded.
- A setting turns it on. With it off, Crow behaves as today, and there's a one-off "Calculate Folder Sizes" command.

## Inspiration: asimeow
[`paraversal/asimeow`](https://github.com/paraversal/asimeow) recently got the same kind of measuring, for Time Machine exclusions. It's a different shape of problem (a few dozen known folders per run, not whatever is on screen, and no live updates), but these parts carry over:
- `src/scan.rs` `folder_size`: logical sizes, hard links counted once by `(dev, ino)` when `nlink > 1`, symlinks not followed.
- `src/cache.rs`: a versioned JSON cache in `~/Library/Caches/asimeow/` (Time Machine skips it, and losing it only costs one re-measure), keyed by path, storing the inode so a recreated folder isn't mistaken for the old one.
- `_docs/performance.md`: profile before optimizing. The walk itself was cheap there; the time went into subprocesses.

What doesn't carry over: asimeow treats an entry as fresh when the folder's inode and top-level mtime match and it's younger than a time limit. That's acceptable for a report, but for the reason above Crow needs FSEvents instead.

## Current state
- Listing uses `getattrlistbulk(2)` (`crates/crow-fs/src/listing.rs:1`, `:113`, `:215`) and asks for `ATTR_FILE_TOTALSIZE` only. Folders get `size = 0`.
- Sorting by size compares `Entry::size` (`crates/crow-fs/src/sort.rs:129`), so folders sort as zero.
- Watching uses the `notify` crate per shown folder (`crates/crow-fs/src/watch.rs:21`). `notify` doesn't expose FSEvents event IDs or history replay.
- State files live in `crow_core::state_dir()` (`~/.local/state/crow`, `crates/crow-core/src/config.rs:13`).
- The bottom bar's selection size counts files only (`Folder::selection_bytes`).

## Items

### S1 Spike: measure before choosing (no visible change)
- Time a full walk of `~` and of `target/fixture-100k` three ways: `std::fs::read_dir` + `symlink_metadata`, `getattrlistbulk` (recursive) with `ATTR_FILE_TOTALSIZE`, and the same with `ATTR_FILE_ALLOCSIZE`. Record folders/s, files/s and wall time, warm and cold. `sample` the fastest one to see where time goes.
- Check that the walk never downloads dataless (iCloud) files and never triggers a TCC prompt for a folder the user hasn't opened in Crow. Write down what happens in `~/Library/Mail`, `~/Desktop` and `~/Documents` with and without access granted.
- **Done when:** the numbers and findings are in the progress file. Q1 and Q5 can be answered from them.

### S2 The sizer (`crow-fs`)
- `crow_fs::sizes::measure(root, cancel) -> Measured { bytes, items, folders, unreadable }` built on the `getattrlistbulk` walk from S1. Count each hard link once (`ATTR_FILE_LINKCOUNT` + file id), don't follow symlinks, don't cross devices (`ATTR_CMN_DEVID`), count a package's contents in its own size. Never open file contents.
- Cancellable between batches (a scrolled-away folder stops costing anything).
- **Done when:** unit tests cover nested folders, hard links, a symlink loop, a mount point boundary (or a fake device id), an unreadable subfolder (counted in `unreadable`, not an error) and cancellation.

### S3 The cache
- `crow_fs::sizes::Cache`: path → `{ bytes, items, inode, measured_at, event_id }`, versioned JSON (asimeow's shape). Loaded and saved off the main thread, with a size bound (least recently shown entries go first). Location per Q4.
- A child's cached size helps its parent: once every child folder of `P` is fresh, `P`'s size is its own files plus their sizes, with no walk.
- **Done when:** round-trip, version mismatch (ignored), inode change (stale) and the bound have tests.

### S4 FSEvents history (`crow-macos`)
- A small binding: `FSEventStreamCreate` with `sinceWhen` = the stored event ID, folder-level events (no per-file flag needed), on a dispatch queue, latency around 1 s. Each event marks its folder and all its ancestors stale in the cache.
- Handle `MustScanSubDirs` (mark the subtree stale), `HistoryDone`, `RootChanged`, and a changed event-store UUID (`FSEventsCopyUUIDForDevice`): the history is gone, so drop that volume's entries.
- Persist the last event ID with the cache.
- **Done when:** a test (behind a macOS cfg) writes a file deep under a temp folder and sees the folder and its ancestors marked stale. A restart test replays an event that happened while the stream was off.

### S5 Scheduling (`crow-app`)
- A global size service: panes report the folder rows they show (the `uniform_list` range). Requests go nearest-the-top first, deduplicated, on two background workers at a low QoS. Rows that scroll away are cancelled. Never on the main thread.
- A fresh cached size shows at once. A stale one shows dimmed until it's re-measured. A missing one shows `--`.
- **Done when:** tests drive the service with a fake sizer: order, dedupe, cancel on scroll, stale-then-fresh.

### S6 Showing sizes
- List view Size column, column view's preview pane, the bottom bar's selection size (folders included once known), and the VoiceOver label of the Size cell ("12.3 MB" or "size not calculated").
- New values fade in (U10 `STATUS`, Low/Full). Off sets them at once.
- **Done when:** keystroke tests check the column text before and after a measure, and the hand checks cover the look.

### S7 Sorting by size
- Folders sort by their known size. Unknown sizes sort per Q3.
- **Done when:** sort tests cover known, unknown and stale sizes.

### S8 Setting and command
- `folder_sizes = "off" | "visible"` in `settings.toml` (documented default, live reload, `edit_toml`). View ▸ Calculate Folder Sizes (and the palette) measures the current folder's rows once when the setting is off.
- **Done when:** parse test, keystroke test for the command, README settings table updated.

### S9 Docs, tests, bench
- `docs/testing.md` hand checks, and a bench run of `target/fixture-100k` with the setting on: warm first paint and scroll p99 within the AGENTS.md budgets, compared with `main`.

## Risks and things to measure first
- **R1 TCC prompts.** Walking under `~` can reach Desktop, Documents, Downloads, Mail and other protected folders. A size walk must not be what triggers a permission prompt. S1 measures it; the fix may be skipping protected folders Crow hasn't been granted (shown as `--`).
- **R2 Huge or slow trees.** `/`, a Time Machine volume, `node_modules` forests, network shares. Cancellation (S2) and the per-volume rule (Q6) cover it. Never measure `/` as a whole unless asked.
- **R3 Firmlinks.** `/` and `/System/Volumes/Data` overlap. Measuring `/` must not count the data volume twice.
- **R4 APFS clones** share blocks but each reports its full size. Finder double-counts them too. Accept it and note it.
- **R5 FSEvents history can be purged** (disk full, a new volume UUID). S4's UUID check falls back to re-measuring.

## Questions to ASK
- **Q1** Logical size (what Finder's list shows, and what files already show) or allocated size on disk? Recommended **logical**, for consistency with file rows.
- **Q2** Default for `folder_sizes`: `"off"` like Finder, or `"visible"`? Recommended **off** until the bench and R1 are settled, then decide again.
- **Q3** Sorting while sizes arrive: re-sort as each size lands (Finder does; rows jump, against spec principle 3 "Predictable"), or keep the order until the user re-sorts or all visible sizes are in? Recommended **re-sort once, when the visible sizes are all in**.
- **Q4** Cache location: `~/Library/Caches/crow/` (Time Machine skips it, like asimeow) or Crow's state dir? Recommended **Caches**.
- **Q5** FSEvents scope: one stream for the whole boot volume, or one per folder Crow has measured? Answer from S1.
- **Q6** Network and removable volumes: measure, or always `--`? Recommended **`--`, with the one-off command still allowed**.

## Hand checks
- Home in list view with the setting on: sizes fill in top first, fade in, and match Finder's Get Info (logical) within rounding.
- Edit a file deep in a measured folder: that folder's size and its parents' sizes update within a couple of seconds. Quit Crow, change something, relaunch: the change shows.
- Scroll a 100k folder of folders quickly: no stutter, and no work continues for rows that scrolled away (Activity Monitor CPU).
- No permission prompt appears just from viewing `~`.
- An iCloud Drive folder with files not downloaded: they stay not downloaded.

## Finish

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- Bench `cargo run --release -p crow-app -- --bench target/fixture-100k` with the setting on and off, compared with `main`; warm first paint and scroll p99 must stay within the AGENTS.md budgets.
- `./scripts/bundle.sh`; hand-check the list in "Hand checks" above in light/dark, 640 px and wide.
- Update README's settings table and `docs/testing.md`.
- Commit per item, keep the progress file current, stop at "Ready for critique". Don't merge.
