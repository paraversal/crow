# B4 — Cap and performance progress
Branch: m8.5-b4-cap · PR: none · Last updated: 2026-11-18 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 Control: cancel, or stop reading contents | done (this commit) | `Control` (cancel + stop-contents, `cancel_flag` for Spotlight's `mdfind`), `Summary.contents_stopped`; every caller ported. Engine tests: `stop_contents_keeps_names_streaming`, `contents_stopped_before_the_walk_reads_no_contents`, both cancellation tests; corpus test updated. `cargo test -p crow-search` 35 + 2 pass. |
| 2 Two budgets | todo | |
| 3 Footer | todo | |
| 4 Selection stays put under late strong names | todo | |
| 5 Bench and budget | todo | |
| 6 Docs | todo | |

## Questions
- none yet.

## Decisions
- Branch cut from `main` at `d7ffb5d` (B3 merged) — plan's base — plan.
- Keep the plan's two constants at 20,000 strong and 20,000 other rows — plan.

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

## Next
Item 2: two budgets, explicit `weak_count`, per-hit strong/other decisions in `add_hits`, and the GPUI cap tests.
