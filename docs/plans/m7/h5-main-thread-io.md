# M7 H5 — No disk I/O on the main thread

Audience: an implementing agent. Follow the `execute-plan` skill; read `AGENTS.md` and `docs/plans/m7/README.md` first.
Base: branch `m7-h5-main-thread-io` from `main`. Progress file: `docs/plans/m7/h5-main-thread-io.progress.md`.
PR title: "M7 H5: no disk I/O on the main thread". Don't merge.

## Why
AGENTS.md: "No disk I/O on the main thread". A single `stat` is cheap on APFS but can block for seconds on a sleeping external disk, a network share or a cloud placeholder. A scan of `main` @ 56db832 (`rg 'std::fs::|\.exists\(\)|\.is_dir\(\)|metadata\(' crates/crow-app/src`) still finds a few on the main thread.

## Current state (re-check line numbers on `main`)
| Where | What | Fix direction |
| --- | --- | --- |
| `pane.rs` `begin_rename` (≈ 1178) | `path.is_dir()` to decide whether to select the extension | Use the entry's known kind (the caller has the `Entry`); pass `is_dir` in |
| `window.rs` `add_to_sidebar` (≈ 1587) | `p.is_dir()` over the selection | The pane's entries know which are folders (as `sidebar.rs` `folders_of_selection` already does) |
| `sidebar.rs` `folders_of_paths` (≈ 736) | `is_dir()` over paths dropped from another app | `background_spawn`, then add on the main thread |
| `operations.rs` `same_volume` (≈ 41) | `symlink_metadata` + `metadata` on drop, to choose move vs copy | Must answer while the drag is over the target (the cursor badge). Cache the device id per shown folder (from the listing) and per dragged item (at drag start, in the background); fall back to "copy" with an unknown answer, never a blocking stat |
| `ui.rs` (≈ 522–527) | `is_dir`/`symlink_metadata` | Already inside `background_spawn`; no change. Record it as checked |
| Startup (`logging.rs`, `ui.rs:210`, `main.rs`) | config dir creation | Allowed (startup exception) |
Also check `crow-macos` calls made during render (icons, kinds, Finder names): `done/u7-fix.progress.md` X6 measured icons at ≤ 24 ms cold. Decide with the user (Q2) whether that's in scope.

## Items
### I1 Fix the sites above
One commit per site, each with a test where behavior is observable (e.g. rename of a folder named `a.b` selects the whole name; Add to Sidebar with a mixed selection adds only folders).

### I2 Guard against regressions
**Change:** a debug-only check that panics (tests) or logs at `error` (debug builds) when the main thread does disk I/O. Options, cheapest first: (a) a CI grep test that fails on new `std::fs`/`Path::{exists,is_dir,is_file,metadata,symlink_metadata,canonicalize,read_dir}` in `crates/crow-app/src` outside an allowlist of `background_spawn` closures, tests and startup files; (b) a wrapper `crow_fs::bg` that asserts `!is_main_thread()` in debug builds, with all disk calls going through `crow_fs`. Recommended in Q1.
### I3 Slow-disk hand check
Mount a disk image or SMB share, browse it, sleep the disk / pull the network, and click around: no beachball. Record in `docs/testing.md`.

## Finish
fmt · clippy · tests; bench vs `main` (drag/drop and rename aren't on the bench paths; note it). `./scripts/bundle.sh`; hand checks. Commit, push, "Ready for critique".

## Questions to ASK
1. **Guard: grep test (a) or debug assertion in `crow_fs` (b)?** Recommended: (a) now (small, catches new code at review time); (b) only if (a) proves too noisy.
2. **Are `crow-macos` icon/kind lookups during render in scope?** Recommended: no; X6 measured them as minor. Re-open if H3 shows them in slow steps.
3. **`same_volume` unknown → copy or move?** Recommended: copy (Finder's safe default across volumes; nothing is lost if wrong).
