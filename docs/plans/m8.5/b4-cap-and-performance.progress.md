# B4 — Cap and performance progress
Branch: m8.5-b4-cap · PR: none · Last updated: 2026-11-18 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 Control: cancel, or stop reading contents | todo | |
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

## Next
Item 1: add `Control` to `crates/crow-search/src/engine.rs`, export it from `lib.rs`, and port every caller.
