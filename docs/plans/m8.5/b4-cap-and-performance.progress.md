# B4 — Cap and performance progress
Branch: m8.5-b4-cap · PR: none · Last updated: 2026-11-18 · State: In critique

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 Control: cancel, or stop reading contents | done (da7f796) | `Control` (cancel + stop-contents, `cancel_flag` for Spotlight's `mdfind`), `Summary.contents_stopped`; every caller ported. Engine tests: `stop_contents_keeps_names_streaming`, `contents_stopped_before_the_walk_reads_no_contents`, both cancellation tests; corpus test updated. `cargo test -p crow-search` 35 + 2 pass. |
| 2 Two budgets | done (decae08) | `MAX_STRONG_ROWS`/`MAX_OTHER_ROWS`; `add_hits` ranks before building a row, strong overflow cancels, other overflow counts `dropped` and stops contents. Stops at exactly 20,000 other rows (the plan's test expects `contents_stopped` after `MAX_OTHER_ROWS` content hits), and cancels in Contents mode. `weak_count` maintained by `insert_rows`. GPUI tests: `strong_name_after_the_cap_is_kept`, `weak_hits_after_the_cap_are_counted`, `strong_names_cap_cancels`, `contents_only_cap_cancels`; all pass. |
| 3 Footer | done (decae08) | `FooterState` now carries `names_capped`, `contents_stopped`, `dropped`; the four branches in the plan's order; warning colour; aria label unchanged. `footer_summary_copy` covers every branch with exact strings. |
| 4 Selection stays put under late strong names | done (92929c0) | No code change needed. `late_strong_name_keeps_the_cursor_row`: 30 content rows, Down twice (cursor 2 on `content-02.txt`), then 5 strong names → same path, cursor 7. Passes. |
| 5 Bench and budget | done (a3eb65c, 77991d9) | `--bench-search DIR [QUERY] [--names\|--contents]` prints `first_hit`, `first_name_hit`, `total`, `hits`, `scanned`, `skipped`, `contents_stopped`; engine bench, no app cap. Medians recorded under Bench; all four rows inside budget. |
| 6 Docs | done (77991d9) | `docs/testing.md`: two budgets and the four footer states (Search result cap hand check), bench flags/output and the budget in the performance table, log fields; stale "First 20000 files" gone. |

## Questions
- Q1 (2026-11-18, critique C5): Names-only `f0` reports "contents stopped" although Names never reads contents. → User chose `omit_names_suffix`: omit that suffix in Names mode only; keep counts, budgets and All/Contents wording unchanged.

## Decisions
- Branch cut from `main` at `d7ffb5d` (B3 merged) — plan's base — plan.
- Keep the plan's two constants at 20,000 strong and 20,000 other rows — plan.
- Critique Q1: Names-only weak-cap footer omits "contents stopped at 20000"; All and Contents retain it. This deliberately refines item 3's unconditional suffix to match the selected mode — user approved 2026-11-18.

## Bench

`target/fixture-search-200k` (200,000 files), three runs each, median. Commands:
`crow --bench-search target/fixture-search-200k QUERY [--names|--contents]`.
`main` is `d7ffb5d` in `../file-crow-b4-main`, built `--release`; this branch is built `--release`.

| query | flags | main first hit / total | branch first hit / total | budget |
| --- | --- | --- | --- | --- |
| `needle` | (all) | 1 ms / 1603.0 ms | 1.0 ms / 1568.7 ms | pass |
| `back` | (all) | 0 ms / 1449.0 ms | no hit / 1461.9 ms | pass |
| `backup` | (all) | 0 ms / 1457.0 ms | no hit / 1448.5 ms | pass |
| `file-01-02` | `--names` | 5 ms / 210.0 ms | 2.2 ms / 184.5 ms | pass |

First hit ≤ `main` + 5 ms; total ≤ `main` × 1.10. All pass. `back` and `backup` have no hits
because the fixture's files contain `needle` only; their numbers are a full walk. The fixture is
unchanged from the docs' 200k fixture (200,544 entries scanned).
`main`'s `--bench-search` had no match flags yet, so the main worktree got a temporary local patch
adding `--names` to its bench (engine untouched; the worktree is discarded after measuring). Without
it, the names row has nothing comparable on `main`.

## Hand checks
- [x] `target/fixture-search-200k`, Cmd-F `needle` (All): footer reaches "20000 results · 265 more not shown · contents stopped at 20000" (`target/shots/search-cap-all-firebox.png`). Typing `file-01-02-00001` makes that exact file the only/top row (`search-cap-exact-firebox.png`).
- [x] Same folder, Names `file`: footer "First 20000 name matches · refine the search" in warning (`search-cap-names-firebox.png`); the match control is on Names.
- [x] While results stream, arrow down three rows: when the walk finishes the selected row is still `file-21-08-00311.txt`, not the top row (`search-cap-select-firebox.png`; bottom bar says 1 of 20000).
- [x] 640 px, dark and light: the footer stays on one line, warning-coloured, and does not wrap (`search-cap-640-firebox.png`, `search-cap-640-solarized-light.png`); this string fits, so the ellipsis is not exercised. VoiceOver was not driven; the footer's `Role::Status` and aria label are unchanged from the old `capped` footer.

## Found along the way
- none yet.

## Log
- 2026-11-18: read the plan, M8.5 index decisions 9, AGENTS.md and the current engine/search code. Created the branch and this file.
- 2026-11-18: item 1 done. `Control` shares two relaxed flags; the walker and receive loop check cancel, the walker and `ContentSink` check stop-contents. `Summary.contents_stopped` reports it. Spotlight takes `Control::cancel_flag()` because `mdfind` takes a bare `&AtomicBool`. Test tree note: the plan's streaming test keeps 50 content + 50 name files; the deterministic "stopped before the walk" test proves no content reads, since a 256-hit batch can legitimately carry every content hit from the 50-file tree before the sink gets to call stop.

- 2026-11-18: items 2 and 3 done together: the budgets decide what the footer reports, so one commit carries both. `add_hits` computes `is_strong_name` once and passes it to `Row::from_hit`. While filling the other budget it stops contents on the row that fills it, not after a rejected 20,001st hit — the plan's `strong_name_after_the_cap_is_kept` test expects `contents_stopped()` after exactly `MAX_OTHER_ROWS` content hits. `Weak rows` are tracked explicitly. Tests pass (`cap_` filter 6 passed; `footer_summary_copy`). fmt/clippy clean.

- 2026-11-18: item 4 done. The inserted rows only move the cursor's row right; `follow_selection` re-indexes by path. Test passes; no production change.

- 2026-11-18: items 5 and 6 done. Bench harness extended, main and branch medians recorded (all within budget), docs updated. Shared-target collision between the two worktrees forced the branch release build into a cloned target dir; the recorded binaries were checked to come from each tree's own `crow-search` (`main` had no `Control`, branch did).

- 2026-11-18: finish. `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace` green (73.6 s). Bundle built with `./scripts/bundle.sh`; hand checks captured under `target/shots/`. `agent-job` could not update: its `meta.json` says "Extra data" (the job still appeared at start); progress-file writes are the source of truth.

## Next
Commit the reviewed fixes and open the B4 PR. After user hand tests and squash-merge, next is `execute plan B5` (`docs/plans/m8.5/b5-regression.md`).

## Critique
Critic session: 2026-11-18 · Base: main @ d7ffb5d · Head reviewed: 575d65e

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | `matched` is never called for nonmatching buffers, so stopping contents left active nonmatching files reading through EOF. | `crates/crow-search/src/engine.rs:608-653` | code / literal | fix | Fixed in this critique: stack-only controlled reader; deterministic matching/nonmatching read and buffer-reuse regressions pass in the workspace suite. |
| C2 | Rejected-only batches changed `dropped` or the strong-cap state without notifying GPUI; the visible footer could remain stale until another accepted row or completion. | `crates/crow-app/src/search.rs:618-630` | UX / semantic | fix | Fixed in this critique: notify for cap-only state changes. Existing rejection test passes, extended with a strong hit after rejections in the same batch. |
| C3 | Each hit batch cloned the folder path solely to avoid a mutable borrow, introducing an unnecessary hot-path allocation. | `crates/crow-app/src/search.rs:602-672` | code sense / literal | fix | Fixed in this critique: borrow the folder only when constructing an accepted row. Existing cap/selection tests pass. |
| C4 | At 640 px the existing scope bar clips the right-hand skip switch on both main and the branch. | `target/shots/b4-critique-main-narrow-light-all.png`, `b4-critique-final-narrow-light-all.png` | UI / semantic | note | Left: unchanged scope-bar layout, outside B4's footer/cap change. |
| C5 | Names-only weak matches report "contents stopped at 20000" even though no contents were searched. Reproduced with Names `f0` in the 100k fixture. The implementation follows the plan, but the copy contradicts the selected mode (principle 4, Legible). | `crates/crow-app/src/search.rs:2610-2617`; `target/shots/b4-critique-final-weak-all.png` vs `b4-critique-main-weak-all.png` | UX / semantic | ask | Q1 → user approved omitting the suffix in Names only; fixed in this critique. Explicit Names/Contents footer cases and final workspace suite pass. Actual 1280 px dark and 640 px light windows show only results/dropped counts (`b4-critique-approved-weak-*`). |

Checks re-run: `cargo fmt --check` ✓; `cargo clippy --workspace --all-targets -- -D warnings` ✓; `cargo test --workspace` ✓ (821 passed, 1 ignored, 12 suites); signed release bundle ✓; CLI usage/unknown flag/extra argument/missing directory errors ✓; isolated Names/Contents smoke ✓ (one hit each, no first-name time in Contents). Final interleaved search benchmarks ✓, all four rows within budget. Listing/scroll bench not needed: no listing, scrolling or first-paint code changed.
Looked at: main and branch bundles built with `scripts/bundle.sh`, both 1280×800 and 640×800; FireBox dark/Finder and Solarized Light/Vim; broad All/Names/Contents, exact-name narrowing, result cursor, Escape, empty folder and `target/fixture-100k`. Final narrow footer is warning-coloured and single-line in both themes; these strings fit, so the ellipsis was not exercised. Screenshot evidence is under `target/shots/b4-critique-*`. System Events exposes only the window group and native window buttons: VoiceOver speech/announcement frequency remains a hand check.
Verdict: better after the fixes: a filled content budget no longer discards later strong names, and the footer explains the limit without adding chrome or contradicting Names mode. No open Fix or Ask findings; one unchanged out-of-scope UI note and the listed human hand checks remain.

Review notes: B4's canonical menu-bar job cannot load its malformed JSON metadata (`Extra data`); a separate review job is receiving phase notes. Rust-analyzer exited on its first references request, so source and compiler checks are the fallback. No new public API is changed by the critique fixes.

Verification log: initial fmt/clippy passed. The first workspace run stopped on a new review-only assertion that tried to read `Role::Status` from GPUI's interactive-element registry; status text is not an interactive element. Removed that invalid assertion (the original count assertions remain, and same-batch late-name coverage is stronger), then restarted the required gates. A CLI smoke initially assumed the existing corpus had a `needle` name; it correctly reported zero name hits. Replaced that smoke input with an isolated deterministic file rather than changing production behavior.

### Critique benchmark

Three interleaved runs per binary/query on the unchanged 200k fixture, after the final engine fix and workspace gates. Main `d7ffb5d` built with `scripts/bundle.sh` in a detached temporary worktree with its own release target; its only temporary source patch adds the CLI Names flag, not engine changes. Branch used the final signed bundle. Zero-hit first latency is "none", not main's old printed `0ms`.

| Query | Main first / total (ms) | Branch first / total (ms) | Budget |
| --- | --- | --- | --- |
| `needle` All | 2 / 1576 | 1.1 / 1518.8 | pass |
| `back` All | none / 1462 | none / 1462.3 | pass |
| `backup` All | none / 1479 | none / 1470.5 | pass |
| `file-01-02` Names | 4 / 185 | 3.9 / 183.8 | pass |

Matching hit counts: 200000, 0, 0, 18799; 200544 entries scanned in every run. Raw local evidence: `/tmp/crow-b4-critique-bench-raw.txt`.

### Remaining hand checks

- [ ] VoiceOver (Cmd-F5): search `needle` in `target/fixture-search-200k`; after completion, the settled footer should be announced once, not once per arriving batch. Repeat Names `f0` in `target/fixture-100k`: counts and dropped count are announced without a claim about contents.
- [ ] Human final pass on the signed bundle: All `needle` → exact file name, Names `file`, and Names `f0`; light/dark, 640 px and wide. The automated visual pass covers these surfaces, but cannot establish spoken timing.

Final review verification: after Q1, fmt/clippy clean and workspace tests green again (821 passed, 1 ignored, 12 suites); signed bundle rebuilt. Actual Names-only weak-cap footer inspected in dark/wide and light/640 px. Benchmark engine/CLI unchanged by Q1, so the recorded final-engine medians remain applicable. Temporary baseline worktree and smoke-driver scripts removed; local screenshots and raw benchmark evidence retained. Publication scan of the actual `origin/main..HEAD` range and selective added lines is clean; an earlier scan without an upstream included already-published history, not just this branch.
