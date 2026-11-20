# Crow code audit through M9

2026-11-20 · Audit base `fb3c58e` (`origin/main` after P7 #82) · Pre-M9 `cb21dcb` · P2 baseline `922c1a0`

This report extends the P2 code audit across the full merged M9 milestone (22 merges from P1A through P7). It measures code size and complexity, checks the repository rules and cross-feature contracts over the merged product, benchmarks M9 against pre-M9 on the same machine, summarizes fuzz coverage and runs, and records the triaged findings feeding the M9 fix batches and backlog.

## Size and complexity

Crow entered M9 with 58,706 Rust lines across 89 files (38,848 non-test lines, 4,415 decision points outside tests). At the M9 audit base (`fb3c58e`) it has 107,901 Rust lines across 140 files (72,883 non-test lines, 35,018 test lines, 9,184 decision points) — an increase of 49,195 lines (+83.8%), of which 15,160 lines (+76.3%) are tests. The release binary grew from 22 MiB (`cb21dcb`) to 35 MiB (`fb3c58e`), driven primarily by `gix` and `rhai`.

| Revision | Rust files | Rust lines | Non-test lines | Test lines | Decision points outside tests |
| --- | ---: | ---: | ---: | ---: | ---: |
| Before M9 (`cb21dcb`) | 89 | 58,706 | 38,848 | 19,858 | 4,415 |
| P1A (`083d40c`) | 100 | 65,853 | 43,478 | 22,375 | 5,080 |
| P9A (`fbdb8c1`) | 100 | 66,228 | 43,544 | 22,684 | 5,082 |
| P9B (`9d074d6`) | 101 | 66,604 | 43,876 | 22,728 | 5,089 |
| P9C (`b035537`) | 104 | 66,644 | 43,912 | 22,732 | 5,089 |
| P8 (`d5cab1e`) | 107 | 68,397 | 45,009 | 23,388 | 5,198 |
| P1B (`7b58509`) | 110 | 74,954 | 49,884 | 25,070 | 5,850 |
| P2 pre-audit (`922c1a0`) | 113 | 78,142 | 52,041 | 26,101 | 6,212 |
| P2 merged (`aab54a0`) | 113 | 78,077 | 51,805 | 26,272 | 6,192 |
| P2 follow-up (`0eeb0e2`) | 116 | 79,418 | 53,023 | 26,395 | 6,364 |
| Nav follow-up (`e7fc40b`) | 117 | 80,237 | 53,669 | 26,568 | 6,441 |
| P3A icon packs (`5d5b9d3`) | 119 | 82,366 | 55,077 | 27,289 | 6,687 |
| List/Preview fit (`925ce2f`) | 119 | 82,399 | 55,081 | 27,318 | 6,687 |
| Settings overhaul (`3219b37`) | 122 | 86,755 | 58,225 | 28,530 | 7,104 |
| P3B icon grid (`3990a45`) | 124 | 89,744 | 60,389 | 29,355 | 7,370 |
| P3C gallery (`6f662cd`) | 125 | 91,923 | 62,111 | 29,812 | 7,578 |
| P4A saved queries (`8fc76ab`) | 131 | 96,157 | 65,626 | 30,531 | 8,121 |
| P4B Finder tags (`08c4f9d`) | 134 | 97,989 | 66,846 | 31,143 | 8,292 |
| P5A copy engine (`c04cde3`) | 135 | 100,226 | 68,127 | 32,099 | 8,500 |
| P5B copy queue (`dc25fae`) | 135 | 102,366 | 69,439 | 32,927 | 8,709 |
| P6 batch rename (`b4d6404`) | 138 | 105,252 | 71,529 | 33,723 | 9,013 |
| P9D Explorer path (`31ca1a2`) | 138 | 105,630 | 71,660 | 33,970 | 9,027 |
| P7 drop & spring (`fb3c58e`) | 140 | 107,901 | 72,883 | 35,018 | 9,184 |

| Crate at M9 audit base (`fb3c58e`) | Files | Rust lines | Non-test lines | Test lines | Non-test decision points | Delta vs P2 (`922c1a0`) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `crow-app` | 66 | 76,147 | 52,294 | 23,853 | 6,259 | +21,276 |
| `crow-core` | 25 | 9,486 | 6,607 | 2,879 | 819 | +3,872 |
| `crow-fs` | 15 | 8,037 | 5,044 | 2,993 | 853 | +3,712 |
| `crow-plugin` | 10 | 6,796 | 4,177 | 2,619 | 665 | −11 |
| `crow-search` | 7 | 3,351 | 1,583 | 1,768 | 175 | +548 |
| `crow-macos` | 14 | 2,138 | 1,863 | 275 | 193 | +221 |
| `crow-plugin-git` | 2 | 1,493 | 943 | 550 | 180 | +141 |
| `crow-theme` | 1 | 453 | 372 | 81 | 40 | 0 |

**Method.** Counts come from [`scripts/code-index.py`](../scripts/code-index.py), calibrated against the P2 audit's baseline (`crow-theme` matches identically at 453/372/81/40; total decision points are within 1.8% across 78k lines due to macro-opaque tokenization). Physical lines include comments and blanks across tracked `crates/**/*.rs`. Test lines cover `tests/`, `*_tests.rs`, `window/tests.rs`, and `#[cfg(test)]` / `#[test]` items. Runnable harnesses such as the motion lab and live fuzzer are in the non-test column. Decision points count `if`, `while`, `for`, `loop`, `match` arms (`arms - 1`) and guards, `&&`, `||`, `?`, and `let … else` inside non-test named functions and methods. Macro bodies are opaque.

### Where M9 added code

| Milestone | Net Rust lines | Main source of change |
| --- | ---: | --- |
| P1A (`083d40c`) | +7,147 | Package loading, grants, workers, the public API and settings |
| P9A (`fbdb8c1`) | +375 | Test fixture ownership and cleanup |
| P9B (`9d074d6`) | +376 | Test and fuzz instrumentation |
| P9C (`b035537`) | +40 | Extracted window modules |
| P8 (`d5cab1e`) | +1,753 | Cloud metadata, per-file read policy and UI |
| P1B (`7b58509`) | +6,557 | Passive contribution coordination, integration and tests |
| P2 pre-audit (`922c1a0`) | +3,188 | Git provider, Projects, source-path fidelity and Git-specific integration |
| P2 audit (`aab54a0`) | −65 | Shared worktree snapshots, batched visible line counts, bounded viewport versions (−236 non-test lines, +171 test lines) |
| P2 follow-up (`0eeb0e2`) | +1,341 | Git review and diff pane, pinned repository status, resizable side panes |
| Nav follow-up (`e7fc40b`) | +819 | List and Columns transitions, reveal feedback, persistent sidebar width, bulk selection scaling tests |
| P3A icon packs (`5d5b9d3`) | +2,129 | Local data-only icon packs, 4-tier resolution, live reload and fallback |
| List/Preview fit (`925ce2f`) | +33 | List column width fitting alongside Preview |
| Settings overhaul (`3219b37`) | +4,356 | 6-category sidebar, live settings search, shared config error banner, preview provider editor |
| P3B icon grid (`3990a45`) | +2,989 | Virtualized sorted icon grid, 2D keyboard navigation, marquee and Cmd-drag additive selection |
| P3C gallery (`6f662cd`) | +2,179 | Gallery view, two-item comparison, shared Quick Look thumbnail and preview pipeline |
| P4A saved queries (`8fc76ab`) | +4,234 | `saved-queries.toml`, live virtual destinations, typed kind/age/tag predicates, background counts |
| P4B Finder tags (`08c4f9d`) | +1,832 | Finder tags in sidebar, `/` local tag filter and picker, saved-query tag predicates |
| P5A copy engine (`c04cde3`) | +2,237 | Resumable copy/move runner, byte-offset chunk resume, typed conflict protocol and replacement staging |
| P5B copy queue (`dc25fae`) | +2,140 | App-wide FIFO transfer queue in Activity, pause/resume, Conflict and Stop sheets |
| P6 batch rename (`b4d6404`) | +2,886 | Find/replace, add text and numbering modes, staged two-phase commits for cycles, single undo group |
| P9D Explorer path (`31ca1a2`) | +378 | Opt-in Explorer-style path bar, resting frame, click-to-edit and Go button |
| P7 drop & spring (`fb3c58e`) | +2,271 | Unified drop target router, 600 ms spring-loaded folder state machine, Back spring and GPUI drag operation mask patch |

### Largest files and split proposals

| File | Total lines | Non-test | Test | Decision points | Split assessment |
| --- | ---: | ---: | ---: | ---: | --- |
| `crates/crow-app/src/window/tests.rs` | 16,286 | 0 | 16,286 | 0 | Split by feature area (`tests/navigation.rs`, `tests/operations.rs`, `tests/search.rs`, `tests/virtual_views.rs`, `tests/drag_drop.rs`) when compile times warrant. |
| `crates/crow-app/src/pane.rs` | 6,418 | 6,254 | 164 | 748 | Split Icons grid (`pane/icons.rs`), List/Columns rendering (`pane/list.rs`, `pane/columns.rs`), and drop hit-testing (`pane/drop.rs`), matching `pane/gallery.rs`. |
| `crates/crow-app/src/settings_window.rs` | 5,328 | 3,447 | 1,881 | 575 | Extract category renderers (`appearance.rs`, `behaviour.rs`, `plugins.rs`) and tests, matching `preview.rs` and `saved_queries.rs`. |
| `crates/crow-app/src/search.rs` | 4,445 | 4,137 | 308 | 502 | Separate result session state / refresh / Spotlight streaming from GPUI rendering. |
| `crates/crow-app/src/window.rs` | 3,595 | 3,593 | 2 | 494 | Extract drag/spring routing (`window/drag.rs`) and Why/theme overlays (`window/overlays.rs`). |
| `crates/crow-app/src/folder.rs` | 3,082 | 2,876 | 206 | 378 | Extract tag-filter scanning and inline-tree row projection from core listing/watcher state. |
| `crates/crow-plugin/src/host.rs` | 2,267 | 1,192 | 1,075 | 160 | Reasonable non-test size; split tests into `host_tests.rs` if it grows further. |
| `crates/crow-app/src/plugins/coordinator.rs` | 2,247 | 1,628 | 619 | 215 | Move UI-free coordinator into `crow-plugin` and split per-surface reply handling (carried P3). |
| `crates/crow-app/src/sidebar.rs` | 2,197 | 2,088 | 109 | 330 | Extract row rendering and context-menu building from model/layout hit-testing. |
| `crates/crow-app/src/palette.rs` | 2,112 | 1,597 | 515 | 162 | Unify `CommandId::key` and `COMMANDS` metadata into one registry table (carried P6). |

### Top 20 non-test functions by decision points

| # | Function | Location | Lines | Decision points |
| ---: | --- | --- | ---: | ---: |
| 1 | `CommandId::key` | `crates/crow-app/src/palette.rs:162` | 76 | 71 |
| 2 | `Sidebar::render` | `crates/crow-app/src/sidebar.rs:1596` | 549 | 59 |
| 3 | `search` | `crates/crow-search/src/engine.rs:376` | 255 | 55 |
| 4 | `build` | `crates/crow-fs/src/testing.rs:256` | 89 | 55 |
| 5 | `CrowWindow::run_command` | `crates/crow-app/src/window.rs:2859` | 85 | 55 |
| 6 | `validate_batch_rename` | `crates/crow-fs/src/ops/batch.rs:291` | 201 | 51 |
| 7 | `Runner::commit_place` | `crates/crow-fs/src/ops/job.rs:765` | 181 | 50 |
| 8 | `validate_registration` | `crates/crow-plugin/src/registry.rs:17` | 146 | 49 |
| 9 | `ControlId::key` | `crates/crow-app/src/settings_window.rs:264` | 53 | 48 |
| 10 | `ControlId::label` | `crates/crow-app/src/settings_window.rs:357` | 53 | 48 |
| 11 | `Pane::render` | `crates/crow-app/src/pane.rs:5755` | 377 | 46 |
| 12 | `ControlId::description` | `crates/crow-app/src/settings_window.rs:461` | 100 | 46 |
| 13 | `MotionLab::key_down` | `crates/crow-app/src/motion_lab.rs:489` | 36 | 46 |
| 14 | `CrowWindow::render` | `crates/crow-app/src/window.rs:3073` | 304 | 43 |
| 15 | `PluginSurfaces::finish` | `crates/crow-app/src/plugins/coordinator.rs:1121` | 175 | 41 |
| 16 | `Pane::render_icon_rows` | `crates/crow-app/src/pane.rs:4363` | 405 | 40 |
| 17 | `render` | `crates/crow-app/src/bottom_bar.rs:769` | 574 | 38 |
| 18 | `Sidebar::looks` | `crates/crow-app/src/sidebar.rs:869` | 142 | 37 |
| 19 | `apply` | `crates/crow-core/src/sidebar.rs:473` | 108 | 37 |
| 20 | `plan_cancellable` | `crates/crow-fs/src/ops.rs:497` | 153 | 35 |

### Dependencies (`Cargo.lock` since `cb21dcb`)

`Cargo.lock` grew from 615 to 697 unique crates (+82: 2 local crates `crow-plugin` and `crow-plugin-git`, 52 `gix-*` crates, 10 `rhai` transitive crates, 17 `gix` transitive crates, and `unicode-normalization`). Crow added no direct duplicate dependency (aside from `anyhow` in `crow-app` for 2 lines of GPUI's `Action::build` signature; already in lockfile). Transitive overlaps come from `gix = "=0.90.0"` (`jiff`/`filetime` alongside `chrono`/`time`, a 3rd `hashbrown` version via `dashmap`, two `defmt` versions via `faster-hex`, and unused `gix-protocol`/`gix-transport`) and `rhai` (`smartstring` + `compact_str` + `thin-vec`, `ahash`).

## What changed

### In the P2 code audit

- **Worktree scans.** Folder lookup resolves a canonical worktree and then consumes one cached status snapshot shared across folders and windows. Two fresh folder scopes in one repository now require one full scan, versus two at the P2 baseline. The GPUI regression opens a second scope through a symlink and checks that it gets the same snapshot and the correct file's Git values. It then stages the file using Git and checks that the watcher updates the index and worktree fields. Snapshot jobs are bounded across polls. The watcher registers its paths together off the UI thread and coalesces changes. A notification arriving during a scan remains queued for the next scan. [Git app service](../crates/crow-app/src/git.rs)
- **Visible line counts.** Each batch opens the repository and builds its filter-free diff state once. It computes the changed sides recorded in the snapshot and publishes a total only when every changed side yields a count. Line work is capped at four concurrent batches. Windows share the cache by repository snapshot and relative path, including aliases of the same file. Regression cases cover stale snapshots, mixed binary/text sides and the size limit. [Provider](../crates/crow-plugin-git/src/lib.rs) · [Git oracle](../crates/crow-plugin-git/tests/oracle.rs)
- **Reply lifetime.** Replacing a Projects scan retires its result stream, including an already queued completion. Git snapshot invalidation cancels old plugin requests and discards their queued demand. Line completion invalidates only matching folder scopes, leaving search and virtual surfaces untouched. [Coordinator](../crates/crow-app/src/plugins/coordinator.rs)
- **Viewport version map.** Visible row tracking retains only the current viewport's entry versions, so scrolling a 100k folder does not accumulate per-row entries across the whole listing. [Folder state](../crates/crow-app/src/folder.rs)
- **Bundled Git isolation.** The bundled Git script is loaded from the embedded source rather than an external disk path, keeping startup deterministic. [Plugin service](../crates/crow-app/src/plugins.rs)

### After P2 (P2 follow-up through P7)

- **Features shipped after P2:** Git review/diff and pinned repository status (#68), navigation motion and persistent sidebar width (#69), data-only icon packs (#71), List/Preview column fitting (#72), categorized searchable Settings overhaul (#73), sorted Icon grid with marquee selection (#74), Gallery view with two-item comparison (#75), Saved Queries as live destinations (#76), Finder Tags in sidebar, `/` filter and saved queries (#77), resumable copy jobs and conflict protocol (#78), app-wide copy queue and Activity (#79), batch rename with staged cycles and grouped undo (#80), opt-in Explorer-style path bar (#81), and native external drop routing with spring-loaded folders (#82).
- **Rules sweep summary:** Zero Theme role violations in M9 UI; zero AppKit/Foundation code outside `crow-macos`; zero `TODO`/`FIXME` comments added in M9; all 9 `patches/gpui-*.patch` apply cleanly in order to upstream `6ceb6fe` (producing a byte-identical `vendor/zed`), and `scripts/fetch-gpui.sh` was updated on this branch to verify the full patch stack against a temporary Git index before resetting so re-runs (and CI `actions/cache` hits) stay idempotent despite overlapping hunks between `gpui-drag-lifecycle.patch` and `gpui-drag-operations.patch`.
- **Cross-feature audit summary:** Auditing the 22 merged pieces together uncovered real cross-feature regressions that per-branch reviews missed: sidebar `drop_hit` section-header offset, scrolled Gallery drop sign inversion, unguarded `pane.dir()` panics on virtual destinations, Command Palette acting on the hidden folder behind ephemeral Cmd-F search, view-switch history corruption on virtual destinations, `save_as_defaults` wiping M9 per-folder keys, iCloud read-policy bypasses in Git and saved-query counts, and macOS TCC `EPERM` on `~/.Trash` swallowed as "Trash is already empty".

## Performance

Measured on Apple M5 Pro (18 cores, 24 GiB RAM), macOS 27.2.1, Built-in Liquid Retina XDR Display (120 Hz, awake), `1280 × 800`, FireBox, motion off, release builds, three rotating runs per condition. Values are medians with observed ranges in parentheses. Budgets: warm first paint < 50 ms, scroll p99 < 12.5 ms (1.5 × 8.33 ms), search first hit ≤ pre-M9 + 5 ms and total ≤ pre-M9 × 1.10.

### GUI listing, filtering, Icons and Gallery

| Workload | Build / config | Warm first paint | Cold first paint | Scroll p50 | Scroll p99 | Max frame | Peak RSS |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| List 100k | pre-M9 (`cb21dcb`) | 3 ms (3–5) | 126 ms (86–129) | 8.33 ms | 8.51 ms (8.49–8.53) | 19.29 ms | 225 MiB |
| List 100k | audit-base (`fb3c58e`), Git off | 3 ms (3–5) | 129 ms (98–134) | 8.33 ms | 8.51 ms (8.51–8.54) | 16.76 ms | 224 MiB |
| List 100k | audit-base (`fb3c58e`), Git on | 3 ms (3–3) | 87 ms (86–127) | 8.33 ms | 8.53 ms (8.52–8.55) | 10.88 ms | 246 MiB |
| Filter `'e'` 100k | pre-M9 (`cb21dcb`) | 3 ms (3–3) | 85 ms (83–93) | 8.33 ms | 8.50 ms (8.49–8.51) | 16.63 ms | 222 MiB |
| Filter `'e'` 100k | audit-base (`fb3c58e`) | 3 ms (3–3) | 84 ms (84–84) | 8.33 ms | 8.55 ms (8.53–8.57) | 17.63 ms | 225 MiB |
| Icons 100k | audit-base (`fb3c58e`) | 3 ms (3–4) | 86 ms (82–89) | 8.33 ms | 8.52 ms (8.50–8.53) | 15.81 ms | 235 MiB |
| Gallery 100k | audit-base (`fb3c58e`) | 3 ms (3–4) | 128 ms (89–137) | 8.33 ms | 8.52 ms (8.50–8.53) | 29.42 ms | 224 MiB |
| Icons 300 images | audit-base (`fb3c58e`) | 1 ms (1–1) | 86 ms (81–118) | 8.33 ms | 8.55 ms (8.54–8.58) | 29.05 ms | 155 MiB |
| Gallery 300 images | audit-base (`fb3c58e`) | 1 ms (1–1) | 111 ms (99–128) | 8.33 ms | 8.53 ms (8.51–8.80) | 29.31 ms | 167 MiB |
| Icons 2,000 images | audit-base (`fb3c58e`) | 3 ms (3–3) | 125 ms (125–126) | 8.33 ms | 8.49 ms (8.49–8.51) | 19.01 ms | 159 MiB |
| Gallery 2,000 images | audit-base (`fb3c58e`) | 3 ms (3–3) | 93 ms (88–93) | 8.33 ms | 8.52 ms (8.48–8.52) | 27.56 ms | 168 MiB |

Every GUI workload holds 120 fps with zero regression versus pre-M9 (`+0.00` to `+0.05` ms p99, warm first paint 1–3 ms). Earlier runs confirmed that when macOS display sleep suspends `CVDisplayLink`, `--bench` stalls at 0% CPU on both pre-M9 and audit-base because `Pane::bench_step` only calls `window.request_animation_frame()` from `Pane::render`.

### Search (200k nested files) and saved queries

| Query (`--bench-search target/fixture-search-200k`) | pre-M9 first hit | audit-base first hit | pre-M9 total | audit-base total | Delta |
| --- | ---: | ---: | ---: | ---: | ---: |
| `needle` (All, 200k hits) | 1.1 ms (1.0–1.9) | 1.1 ms (1.1–1.2) | 1510.2 ms (1458.6–2502.0) | 1621.7 ms (1599.7–1647.7) | +7.4% |
| `backup` (All, 0 hits) | none | none | 1492.0 ms (1440.6–1494.1) | 1565.3 ms (1538.5–1585.4) | +4.9% |
| `file-01-02 --names` (18,799 hits) | 5.7 ms (5.6–6.8) | 6.0 ms (4.4–6.3) | 211.9 ms (200.7–215.4) | 212.8 ms (205.1–213.5) | +0.4% |
| `needle --contents` (200k hits) | 1.1 ms (1.0–1.1) | 1.0 ms (1.0–1.1) | 1503.2 ms (1468.7–1519.6) | 1603.8 ms (1580.9–1630.9) | +6.7% |
| Saved query `"" --kind text --modified-within 30` (20k cap) | — | 521.5 ms (512.7–534.5) | — | 523.3 ms (514.6–536.0) | — |

All four B4 workloads stay within budget (+0.4% to +7.4% total, first hit within +0.3 ms; the ~5–7% content-read delta comes from P8's per-file cloud read-policy check). Metadata-only saved queries buffer up to 20,000 hits to return a deterministic sorted set, so first hit equals walk time.

### Listing, tag filter, batch rename, copy throughput and idle resources

| Workload | Result (medians, 3 runs) | Comparison |
| --- | --- | --- |
| `crow-fs` `list_bench` (100k) | first batch 4.4 ms, list 194.6 ms, name sort 4.9 ms, kind sort 14.1 ms | pre-M9: 4.2 / 204.8 / 5.2 / 14.7 ms |
| `bench_tag_filter_100k` (synthetic reader) | first batch 0.6 ms (0.5–0.6), settled 205.4 ms (204.9–205.6) | P4B recorded: 0.6 / 215.0 ms (real `NSURLTagNamesKey` 100k settle ~3.6 s accepted) |
| Batch rename (10,000 files) | preflight 153.2 ms (153.1–155.1), commit 1491.3 ms (1460.8–1500.0), undo 1343.4 ms (1321.2–1379.5) | — |
| Copy 1 × 4 GiB (same-vol APFS clone) | Crow **0.7 ms** (0.4–1.4) | `cp -c` 3.8 ms (2.7–4.3) · `ditto` 5.3 ms (5.2–5.7) |
| Copy 20k × 1 KiB (same-vol APFS clone) | Crow **1779.7 ms** (1737.1–1867.7); with 20 ms pause+resume **1783.1 ms** (pause ack **0.09 ms**) | `cp -Rc` 1786.3 ms · `ditto` 2061.0 ms |
| Copy 1 × 4 GiB (cross-vol APFS DMG) | Crow **2301.0 ms** (829.2–2781.8); with 20 ms mid-file pause+resume **2912.3 ms** (pause ack **0.80 ms**) | `cp` 2782.4 ms · `ditto` 2755.5 ms |
| Copy 20k × 1 KiB (cross-vol APFS DMG) | Crow **5849.6 ms** (3344.1–6743.2); with 20 ms pause+resume **6119.5 ms** (pause ack **0.02 ms**) | `cp -R` 6621.6 ms · `ditto` 6704.6 ms |
| Idle memory & CPU (100k Git worktree, Git on, saved query & pinned tag, 120 s) | RSS 245.5 → 253.3 MiB (flat after +60 s); 0.0% instantaneous CPU, **0.38% average CPU** (0.46 s CPU time / 120 s) | Near zero idle CPU; no RSS drift |

Scrolling Icons and Gallery through 2,000 images holds peak RSS at 159 MiB and 168 MiB (versus 155 MiB and 167 MiB on 300 images) under the 48 MiB / 512-entry thumbnail cache cap, and closing a tab or switching out of Gallery releases its decoded preview textures (`Preview::release_image`, `gallery::release`, `Pane::release_listings`).

### P2 Git status and line-count benchmarks (retained from P2 audit)

| Release build (100k Git worktree, 120 Hz) | Warm first paint | Scroll p99 | Background Git status scan |
| --- | ---: | ---: | ---: |
| `main` before P2 (`7b58509`) | 6 ms (5–10) | 8.69 ms (8.52–8.73) | — |
| P2 baseline (`922c1a0`), Git enabled | 5 ms (5–6) | 8.51 ms (8.51–8.64) | 243 ms (240–756) |
| P2 audited (`aab54a0`), Git enabled | 5 ms (4–5) | 8.58 ms (8.57–8.59) | 232 ms (228–234) |
| P2 audited (`aab54a0`), Git disabled | 5 ms (4–5) | 8.72 ms (8.60–8.77) | — |

| Line-count batch (64 modified tracked files in 100k, 18 samples) | Median | Range |
| --- | ---: | ---: |
| P2 baseline (`922c1a0`) | 146.78 ms | 145.09–153.48 ms |
| P2 audited (`aab54a0`) | 9.66 ms | 9.39–10.48 ms |

That line-count batch is **15.2× as fast** after opening the repository and building filter-free diff state once per batch.

## Remaining findings

Triaged with the owner on 2026-11-20.

### Blocking M9 Finish (scheduled in M9 audit-fix batches)

**Must fix (10):**

| Area | Evidence | Summary |
| --- | --- | --- |
| Drop / Sidebar | `crates/crow-app/src/sidebar.rs:1163-1170` vs `:1736-1789` | `drop_hit` filters out `Row::SectionHeader` before `.nth(index)` while `render` includes it, shifting Recent/Locations/Projects drops onto the wrong folder. |
| Drop / Gallery | `crates/crow-app/src/pane.rs:2581` | `gallery_row_at` adds negative `shown_offset` instead of subtracting it; scrolled Gallery drops hit the wrong row. |
| Virtual locations / Panic | `crates/crow-app/src/pane.rs:666-668, 1599, 1759, 2181, 2259`; `pane/gallery.rs:266`; `window.rs:2266` | Unguarded `pane.dir()` (`expect("a folder destination")`) crashes on Saved Query / Tag destinations during batch rename, hidden-folder watcher/cloud events, or switching tabs with Why This View open. |
| Search / Palette routing | `crates/crow-app/src/window.rs:2935-2941, 2203, 652, 799` | Checks `pane.folder().is_some()` before `active_search(cx)`, routing Command Palette destructive actions (Trash, Delete Permanently, Rename, Paste, New File/Folder) to the hidden folder behind ephemeral Cmd-F search. |
| Virtual locations / History | `crates/crow-app/src/window.rs:763-777`; `pane.rs:1805-1836, 1999` | `set_mode` (⌘1–⌘4) on Saved Query / Tag pushes the stale hidden folder onto `history` while staying in virtual mode (split-brain UI + corrupted history). |
| Git / iCloud read policy (P2 carried) | `crates/crow-plugin-git/src/lib.rs:263-269, 505-571, 819-826`; `git.rs:402, 612`; `info_pane.rs:294` | Path-prefix guard misses iCloud Desktop/Documents and third-party File Providers; `worktree_status` and `LineDiff::layers` never consult `SF_DATALESS` or `crow_fs::ReadGate`. |
| Saved queries / iCloud read policy | `crates/crow-app/src/saved_query_counts.rs:76` | Background saved-query counter hardcodes `read_gate: None`, bypassing `SystemReadGate` on content-matching queries under cloud roots. |
| Config / `views.toml` | `crates/crow-core/src/views.rs:726-738` | `save_as_defaults` ("Use as Defaults") removes the entire `[folder."…"]` table, silently wiping M9 per-folder `preview_providers`, `icon_source`, `icon_size`, `icon_caption`, and `columns`. |
| Empty Trash / macOS TCC | `crates/crow-fs/src/trash.rs:32-38`; `window/operation_ui.rs:364-368` | `trash::count` swallows `fs::read_dir` `EPERM` as `0`, making ⇧⌘⌫ report `"The Trash is already empty."` without Full Disk Access. Report permission error + Full Disk Access route and add Finder AppleEvent fallback. |
| Live fuzz / Main-thread freezes | `crow --fuzz --seed 20261120 --minutes 10` (step 6913 `Keys("cmd-[")`, 2,079.3 ms; 60 freezes >250 ms; 0 invariant violations) | Real-frame navigation steps (`cmd-[`, `cmd-up`, `cmd-shift-a`) and `icon fetch` (up to 95.6 ms) stall the main thread on the small fuzz tree; needs profiling and off-main teardown/fetch. |

**Should fix (28, grouped by batch; #1 `scripts/fetch-gpui.sh` fixed on this branch for CI):**

- **Drop, spring & virtual parity:** Gallery background drop outline and blank-preview hit target (`pane/gallery.rs:514-555`, `pane.rs:3580`), breadcrumb/PathBar spring blocked by ancestor check (`drag.rs:193`, `window.rs:1404`), hover validity (`drop_would_act`) in sidebar/path bar/search, `go_back` after `spring_commit` using `all_present` (`pane.rs:1421-1428`), virtual spring origin & Cmd-F restore on cancel (`window.rs:1492, 1674-1676`), `SearchView::run_command` missing `Rename` (`search.rs:2636`), `ReviewGitChanges` virtual guard (`window/git_review.rs:136`), unconditional `cx.notify()` on every drag pointer sample (`window.rs:1513`).
- **iCloud, Activity & permission UX:** Drag promotion calling `icon_png` directly instead of cloud-safe `type_icon_png` (`pane.rs:289`, `search.rs:3637`), Empty Trash missing from Activity (`window/operation_ui.rs:381-398, 794-827`), dedicated in-pane permission state for unreadable (`EPERM`/`EACCES`) folders with Retry and Open Full Disk Access (`folder.rs:1660-1667`).
- **Config, reachability, bar & logging:** `OpenSavedQueriesToml` omitted from `settings_window/catalog.rs:318-340`, unified clickable/clearing error slots for `plugins.toml` and `pack.toml`, inline `#` comment preservation (`views.rs:440`, `sidebar.rs:502`, `grants.rs:139`), Projects commands menu/context-menu reachability (`palette.rs:362-384`), 640 px bottom-bar config-error `Open` button and right-pane `Git Diff` tab / Preview `info_row` clipping (`bottom_bar.rs:594-631`, `info_pane.rs:590-635`, `preview.rs:572-581`), `logging.rs:47` panic on unwritable log file.
- **Selection, sheets, a11y & tags:** Saved-search form root role/label & `aria_toggled` (`saved_query_form.rs:430, 458-462`), Search/Saved/Tag mouse-down selection collapse, Cmd-click cursor desync, missing Shift-Arrow `ExtendUp`/`ExtendDown`, and post-batch-rename selection remap (`search.rs:2251-2269, 3391`, `pane.rs:1600`), `Folder::click`/`Selection::select_all`/`set_selection_paths` cursor/anchor sync, Stop/Empty Trash/Delete Tab rings, Conflict Space handler, Tag Picker `Clear` focusability, `CrowWindow` Saved-search form 640×500 scroll clamp & `SettingsWindow` missing `.key_context("Sheet")`, Settings Escape in stepper/terminal inputs, `[`-prefixed tag round-trip (`saved_query_form.rs:46-52`), bounded background refresh for Finder tag discovery (`tags.rs:143-158`).
- **Async freshness, watchers, batch scale & grid motion:** `Pane::release_listings` leaking `tag_sessions` (`pane.rs:1551-1583`), `Sidebar::release_plugins` not cancelling saved-query counts, `Folder` tag-scan & `list_in_stages` cancel flags + overlapping-generation test, `path_edit`/`palette_view`/`refresh_recent`/`refresh_volumes` generation guards, `FolderSizes::pause()` not dropping root `/` FSEvents stream (`folder_sizes.rs:353-355`), batch-rename main-thread preview rebuild per keystroke (`batch_rename.rs:69, 191, 208, 223`), icon-grid preset and debounced window-resize animation (`pane.rs`).
- **Visual polish, deduplication, tooling & fuzz:** `io_guard.rs` covering `crow_macos::` disk calls and `write_atomic`, sidebar Tag/SavedQuery icons clobbered by icon pack (`sidebar.rs:1928-1930`), Icons/Gallery `cut_outline` consistency (`pane.rs:4695`, `pane/gallery.rs`), replacing `window.rs:1902` `home_dir` with `Ui.home`, consolidating icon-fallback/sheet/SelectionSummary duplicates and tightening `coordinator.rs`/`plugins.rs` allows + deprecated `iconForFileType:`, and expanding `fuzz.rs` + `debug_selectors`/`text_focused` to cover M9 surfaces.

### Deferred backlog (recorded in `docs/plans/inbox.md`)

- **Git cache lifetime, watchers & Projects reuse (carried P1):** Session-long `folders`/`repos`/`lines` maps, up to 16 recursive watchers outliving all windows, and Projects re-opening repos independently (`git.rs:24, 102-108, 447-465, 708-710`).
- **Search channel backpressure & sync `displayNameAtPath` (carried P7):** Unbounded hit channels (`crow-search/src/engine.rs:405, 517`) and synchronous `display_components` on render cache miss (`style.rs:245-261`, duplicated in `sidebar.rs:839-850`).
- **Hidden-tab watchers, unfocused caret blink loops & thumbnail queue scoping:** Hidden-tab `FolderWatcher` + 100 ms polls, unfocused 500 ms `TextInput` blink loops, retained `SearchView` 60 s timers, and cross-window singleton thumbnail queue eviction.
- **Structural splits & UI-free placement (carried P3–P6):** Splitting large `crow-app` files (`pane.rs`, `settings_window.rs`, `search.rs`, `window.rs`, `folder.rs`, `coordinator.rs`), `registry.rs::validate_registration`, unifying `palette.rs::CommandId::key`, and moving UI-free modules (`coordinator.rs`, `icon_packs.rs` loader, `operations.rs` drop routing, `saved_query_counts.rs`, `tags.rs`) into lower crates.
- **Remaining Settings UI for TOML-only keys:** `views.toml` (`icon_source`, default `icon_size`/`icon_caption`/`columns`), `sidebar.toml` (`pinned_tags`, `[sections].order`, `[section].show`, `[projects].roots`), `settings.toml` (`[window].sidebar_width`), and atomic first-launch `settings.toml` creation.

## Verification

- **Workspace checks (`fb3c58e`):** `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `./scripts/check-test-temp.sh` (clean), and `python3 scripts/test_report_test.py` (17 passed) all passed. Full `cargo test --workspace` ran three times through `scripts/test-report.py`: **1,268 passed / 0 failed / 2 ignored** each run, zero flakes (29.3 s and 29.8 s warm; vs P9B baseline 905 passed / 1 ignored / 16.1 s).
- **Fuzz runs (`fb3c58e`):**
  - M7 seed replays (`CROW_FUZZ_SHRINK_SECS=0`): row 5 (`1794761100463163000`, 829 steps), row 9 (`1794773462336254000`, 501 steps), and row 21 (`1794851559033176001`, 3,000 steps, `CROW_FUZZ_CHURN=1`, 1,772 s) all **passed**.
  - Headless churn (10 min, `CROW_FUZZ_CHURN=1`, `revision_dirty: false`): **passed** — 3 seeds (`1795180021133772000..002`), 9,000 steps, 658.5 s, 0 failures.
  - Headless edge trees (5 min, `CROW_FUZZ_TREE=long-name,deep-20`, `CROW_FUZZ_CHURN=0`, `revision_dirty: false`): **passed** — 3 seeds (`1795180021861927000..002`), 9,000 steps, 476.1 s, 0 failures.
  - Live real-frame fuzz (10 min, `crow --fuzz --seed 20261120 --minutes 10`, binary built from clean `fb3c58e`): **0 invariant violations**, **failed on `freeze`** — 22,441 steps in 600.3 s, 31,283 heartbeats, 200 over 50 ms, 60 freezes over 250 ms, worst **2,079.3 ms** at step 6913 `Keys("cmd-[")` (tracked in the Must-Fix table above).
- **Bundled visual verification:** Built signed `Crow.app` bundles for both `fb3c58e` and pre-M9 `cb21dcb` and inspected 18 screenshots across FireBox dark and Solarized Light at `1280 × 800` and `640 × 500` (List, Columns/Vim, Icons + local pack + thumbnails, Gallery + 640 px Preview, Explorer path bar + config error, empty folder, locked folder, Git worktree ignored-file steady state, live theme switch recoloring pack glyphs, and 100k folder). Interactive native-drag, Finder Put Back, real iCloud placeholder eviction, and spoken VoiceOver checks remain hand checks for the owner.
