# B4 — Cap and performance progress
Branch: m8.5-b4-cap · PR: none · Last updated: 2026-11-18 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 Control: cancel, or stop reading contents | done (this commit) | `Control` (cancel + stop-contents, `cancel_flag` for Spotlight's `mdfind`), `Summary.contents_stopped`; every caller ported. Engine tests: `stop_contents_keeps_names_streaming`, `contents_stopped_before_the_walk_reads_no_contents`, both cancellation tests; corpus test updated. `cargo test -p crow-search` 35 + 2 pass. |
| 2 Two budgets | done (this commit) | `MAX_STRONG_ROWS`/`MAX_OTHER_ROWS`; `add_hits` ranks before building a row, strong overflow cancels, other overflow counts `dropped` and stops contents. Stops at exactly 20,000 other rows (the plan's test expects `contents_stopped` after `MAX_OTHER_ROWS` content hits), and cancels in Contents mode. `weak_count` maintained by `insert_rows`. GPUI tests: `strong_name_after_the_cap_is_kept`, `weak_hits_after_the_cap_are_counted`, `strong_names_cap_cancels`, `contents_only_cap_cancels`; all pass. |
| 3 Footer | done (this commit) | `FooterState` now carries `names_capped`, `contents_stopped`, `dropped`; the four branches in the plan's order; warning colour; aria label unchanged. `footer_summary_copy` covers every branch with exact strings. |
| 4 Selection stays put under late strong names | done (this commit) | No code change needed. `late_strong_name_keeps_the_cursor_row`: 30 content rows, Down twice (cursor 2 on `content-02.txt`), then 5 strong names → same path, cursor 7. Passes. |
| 5 Bench and budget | done (this commit) | `--bench-search DIR [QUERY] [--names\|--contents]` prints `first_hit`, `first_name_hit`, `total`, `hits`, `scanned`, `skipped`, `contents_stopped`; engine bench, no app cap. Medians recorded under Bench; all four rows inside budget. |
| 6 Docs | done (this commit) | `docs/testing.md`: two budgets and the four footer states (Search result cap hand check), bench flags/output and the budget in the performance table, log fields; stale "First 20000 files" gone. |

## Questions
- none yet.

## Decisions
- Branch cut from `main` at `d7ffb5d` (B3 merged) — plan's base — plan.
- Keep the plan's two constants at 20,000 strong and 20,000 other rows — plan.

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
- [ ] `target/fixture-search-200k`, Cmd-F `needle` (All): footer reaches "… more not shown · contents stopped at 20000"; typing `file-01-02-00001` without leaving search makes that exact file the top row.
- [ ] Same folder, Names `file`: footer "First 20000 name matches · refine the search"; typing more narrows.
- [ ] While results stream, arrow down a few rows: selection does not jump when batches arrive.
- [ ] Light/dark, 640 px: footer ellipsizes, doesn't wrap; VoiceOver reads the footer once when the search settles.

## Found along the way
- none yet.

## Log
- 2026-11-18: read the plan, M8.5 index decisions 9, AGENTS.md and the current engine/search code. Created the branch and this file.
- 2026-11-18: item 1 done. `Control` shares two relaxed flags; the walker and receive loop check cancel, the walker and `ContentSink` check stop-contents. `Summary.contents_stopped` reports it. Spotlight takes `Control::cancel_flag()` because `mdfind` takes a bare `&AtomicBool`. Test tree note: the plan's streaming test keeps 50 content + 50 name files; the deterministic "stopped before the walk" test proves no content reads, since a 256-hit batch can legitimately carry every content hit from the 50-file tree before the sink gets to call stop.

- 2026-11-18: items 2 and 3 done together: the budgets decide what the footer reports, so one commit carries both. `add_hits` computes `is_strong_name` once and passes it to `Row::from_hit`. While filling the other budget it stops contents on the row that fills it, not after a rejected 20,001st hit — the plan's `strong_name_after_the_cap_is_kept` test expects `contents_stopped()` after exactly `MAX_OTHER_ROWS` content hits. `Weak rows` are tracked explicitly. Tests pass (`cap_` filter 6 passed; `footer_summary_copy`). fmt/clippy clean.

- 2026-11-18: item 4 done. The inserted rows only move the cursor's row right; `follow_selection` re-indexes by path. Test passes; no production change.

- 2026-11-18: items 5 and 6 done. Bench harness extended, main and branch medians recorded (all within budget), docs updated. Shared-target collision between the two worktrees forced the branch release build into a cloned target dir; the recorded binaries were checked to come from each tree's own `crow-search` (`main` had no `Control`, branch did).

## Next
Finish: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`; then hand checks and Ready for critique.
