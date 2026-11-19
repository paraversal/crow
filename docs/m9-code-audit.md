# Crow code audit through M9 P2

2026-11-19 · P2 baseline `922c1a0` · `main` comparison `7b58509`

## Size and complexity

Crow entered M9 with 58,706 Rust lines. By the end of P2 it had 78,142, an increase of 19,436 lines (33.1%). Measured test code accounts for 6,217 lines of that increase. The plugin runtime, app contributions, and Git/Projects milestones account for 16,892 lines of total growth across their commits, including integration and tests.

| Revision | Rust files | Rust lines | Non-test lines | Test lines | Decision points outside tests |
| --- | ---: | ---: | ---: | ---: | ---: |
| Before M9 (`cb21dcb`) | 89 | 58,706 | 38,974 | 19,732 | 4,496 |
| After P1A (`083d40c`) | 100 | 65,853 | 43,601 | 22,252 | 5,167 |
| After P1B, current `main` (`7b58509`) | 110 | 74,954 | 50,037 | 24,917 | 5,958 |
| After P2, before audit | 113 | 78,142 | 52,193 | 25,949 | 6,327 |
| After audit | 113 | 77,989 | 51,914 | 26,075 | 6,296 |

The audit removed 279 non-test Rust lines and 31 decision points, with 126 additional test lines. Dedicated plugin code, comprising `crow-plugin`, `crow-plugin-git`, the app's `plugins` modules and `git.rs`, went from 8,304 to 8,023 non-test lines (−3.4%) and from 1,264 to 1,234 decision points (−2.4%). These figures cover dedicated modules. The window, pane, search and sidebar also integrate plugins. The bundled Git script adds 119 Rhai lines and the plugin fixtures add 94 Rhai lines.

| Crate at P2 baseline | Rust lines | Non-test lines | Test lines | Non-test decision points |
| --- | ---: | ---: | ---: | ---: |
| `crow-app` | 54,871 | 36,965 | 17,906 | 4,221 |
| `crow-plugin` | 6,807 | 4,196 | 2,611 | 679 |
| `crow-core` | 5,614 | 4,000 | 1,614 | 407 |
| `crow-fs` | 4,325 | 2,793 | 1,532 | 480 |
| `crow-search` | 2,803 | 1,268 | 1,535 | 139 |
| `crow-macos` | 1,917 | 1,697 | 220 | 174 |
| `crow-plugin-git` | 1,352 | 902 | 450 | 187 |
| `crow-theme` | 453 | 372 | 81 | 40 |

The index counts physical Rust lines in tracked workspace crates, including comments and blanks. Test lines are files under `tests/`, dedicated `*_tests.rs` and window tests, and inline test modules and functions marked with `cfg(test)` or a test attribute. Runnable harnesses such as the motion lab and live fuzzer are in the non-test column. A Rust syntax-tree walk counts explicit conditionals, loops, match alternatives and guards, short-circuit operators, early-return `?`, and `let … else` in named functions and methods. Macro bodies are opaque. This is a repeatable branch-count proxy. A long table of stable command IDs and a concurrent reply handler can rank next to each other while carrying different maintenance risks.

### Where M9 added code

| Milestone | Net Rust lines | Main source of change |
| --- | ---: | --- |
| P1A | +7,147 | Package loading, grants, workers, the public API and settings |
| P9A | +375 | Test fixture ownership and cleanup |
| P9B | +376 | Test and fuzz instrumentation |
| P9C | +40 | Extracted window modules |
| P8 | +1,753 | Cloud metadata, per-file read policy and UI |
| P1B | +6,557 | Passive contribution coordination, integration and tests |
| P2 | +3,188 | Git provider, Projects, source-path fidelity and Git-specific integration |

## What changed

- **Worktree scans.** Folder lookup resolves a canonical worktree and then consumes one cached status snapshot shared across folders and windows. Two fresh folder scopes in one repository now require one full scan, versus two at the P2 baseline. The GPUI regression opens a second scope through a symlink and checks that it gets the same snapshot and the correct file's Git values. It then stages the file using Git and checks that the watcher updates the index and worktree fields. Snapshot jobs are bounded across polls. The watcher registers its paths together off the UI thread and coalesces changes. A notification arriving during a scan remains queued for the next scan. [Git app service](../crates/crow-app/src/git.rs)
- **Visible line counts.** Each batch opens the repository and builds its filter-free diff state once. It computes the changed sides recorded in the snapshot and publishes a total only when every changed side yields a count. Line work is capped at four concurrent batches. Windows share the cache by repository snapshot and relative path, including aliases of the same file. Regression cases cover stale snapshots, mixed binary/text sides and the size limit. [Provider](../crates/crow-plugin-git/src/lib.rs) · [Git oracle](../crates/crow-plugin-git/tests/oracle.rs)
- **Reply lifetime.** Replacing a Projects scan retires its result stream, including an already queued completion. Git snapshot invalidation cancels old plugin requests and discards their queued demand. Line completion invalidates only matching Git Lines cells. The coordinator records only the current viewport's row versions. A test presenting 128 successive one-row viewports leaves one version entry. [Git app service](../crates/crow-app/src/git.rs) · [Coordinator](../crates/crow-app/src/plugins/coordinator.rs)
- **Plugin isolation and hot-path costs.** Status and Info attach Git facts exclusively to the approved bundled package. Host admission checks that the running package uses an embedded snapshot. Row requests send only per-entry facts, and passive results are handled without cloning the reply for command dispatch. Batching row-cache invalidation reduced the cache walks from one per path to one per batch. [Host](../crates/crow-plugin/src/host.rs) · [Coordinator](../crates/crow-app/src/plugins/coordinator.rs) · [App integration](../crates/crow-app/src/plugins.rs)

## Performance

The standard `crow --bench` run scrolled 599 measured frames in a clean 100,001-entry Git worktree at 1280 × 800, FireBox, 120 Hz and motion off. Three release runs per condition used the same fixture with rotating run order. Times are medians, with observed ranges in parentheses.

| Release build | Warm first paint | Scroll p99 | Background Git status scan |
| --- | ---: | ---: | ---: |
| `main` | 6 ms (5–10) | 8.69 ms (8.52–8.73) | — |
| P2 baseline, Git enabled | 5 ms (5–6) | 8.51 ms (8.51–8.64) | 243 ms (240–756) |
| Audited, Git enabled | 5 ms (4–5) | 8.58 ms (8.57–8.59) | 232 ms (228–234) |
| Audited, Git disabled | 5 ms (4–5) | 8.72 ms (8.60–8.77) | — |

The audited build stayed within the 50 ms first-paint and 12.5 ms scroll budgets. Its median scroll p99 was 0.07 ms higher than the P2 baseline and 0.11 ms lower than `main`. The background single-scan times varied more, with a 756 ms P2 outlier. The deterministic improvement comes from avoiding repeat scans across folders. The release executable grew by about 135 KiB (0.4%) from the P2 baseline.

The line-count benchmark used a local clone of the 100k fixture with 64 tracked files modified in the worktree. It timed opening the repository and counting all 64 rows with an already computed status snapshot, matching the app job boundary. The two release harnesses ran in alternating groups of six, for 18 samples each. Both counted 128 line edits every time.

| Line-count batch | Median | Range |
| --- | ---: | ---: |
| P2 baseline | 146.78 ms | 145.09–153.48 ms |
| Audited | 9.66 ms | 9.39–10.48 ms |

That is **15.2× as fast** for the measured batch. It measures the line-count worker on this fixture, separately from scrolling and full status scans.

## Remaining audit findings

| Area | Evidence | Candidate change |
| --- | --- | --- |
| Git cache lifetime | Folder and repository maps live for the enabled provider session. The watcher follows one worktree, while several windows and search can retain snapshots. Projects discovery also opens repositories separately for summary counts. | Bound retention by active scope and last use. Reuse snapshots for Projects and make worktree watch ownership explicit before adding more providers. |
| Git and cloud storage | The Git guard recognizes known cloud roots. Crow's general read policy also has per-entry placeholder evidence. | Validate worktrees with mixed local and placeholder files against the P8 read policy before allowing Git content reads there. |
| Passive plugin coordinator | The baseline coordinator has 1,606 non-test lines. Its reply handler had 41 decision points, and full row-cache eviction still finds the oldest item by scanning the cache. | Separate row-batch and per-surface reply handling around the existing stamp checks. Measure cache replacement with over 4,096 decorated rows before choosing a different recency index. |
| Plugin registration | `validate_registration` is the largest runtime branch hotspot at 50 baseline decision points. It combines common field validation with contribution-specific rules. | Extract validation by descriptor kind under the existing registration fixtures. Keep rejection and limit semantics pinned. |
| Pane and window | `pane.rs` leads the size index at 4,242 non-test lines. `search.rs` has 3,155, `window.rs` 2,611, and `folder.rs` 2,367. | Separate viewport demand and rendering from folder/window state where they already use different lifetimes. P9C illustrates the benefit to file ownership, with a 40-line cost in the Rust index. |
| Command and settings metadata | The palette's stable-key match has 57 decision points. The stable keys and command metadata are maintained in separate declarations. | Put stable keys in the command registry and use it for lookup, palette and menus. Retain persisted key coverage. |
| Search and display hot paths | Search sends individual name hits through an unbounded internal channel. View-level result caps constrain displayed rows. `display_components` performs Finder display-name lookup on cache misses during rendering. | Measure queue depth under a slow sink and preserve first-hit latency if batching changes. Profile uncached navigation before moving display-name lookup to background work. |

## Verification

`cargo fmt --check`, `cargo clippy --offline --workspace --all-targets -- -D warnings`, and `cargo test --offline --workspace` passed. The workspace ran 990 tests successfully with one ignored, including the smoke fuzzers and the Git CLI oracle. The suite used normal macOS permissions after the sandboxed run failed filesystem watcher and package identification checks.

The audit worked from the existing P2 checkout and left the user's roadmap and plan edits in place. The live run covered the standard wide list benchmark. Interactive VoiceOver, narrow-window and theme checks, and a live two-window keyboard walkthrough remain to be exercised.
