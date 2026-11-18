# B4 — Cap and performance progress
Branch: m8.5-b4-cap · PR: none · Last updated: 2026-11-18 · State: Ready for critique

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
Ready for critique. The critic opens the PR after review; do not push.
