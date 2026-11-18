# B5 — Regression and finish progress
Branch: m8.5-b5-regression · PR: none · Last updated: 2026-11-18 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 Oracle on random trees | done | `crates/crow-search/src/oracle_tests.rs` walks seeded trees with a plain `read_dir` oracle; agrees with the engine on 25 seeds × 4 queries × hidden × everything × match mode (1,200 comparisons; 300 seeds took 57 s, so the plan's "lower the seed count and record it" applies). 4.7 s in debug. Coverage asserts every skip reason and both hit kinds are reached. |
| 2 App-level corpus matrix | done | `search_corpus_matrix` drives the panel through `backup`/`needle` × All/Names/Contents × hidden on/off × everything on/off (24 runs) and compares the path set, done-footer text and skipped count with `crow_search::search` directly. Passes in 3.6 s. |
| 3 Fuzzer | done | First 120-step smoke run (`fuzz_smoke` + `fuzz_smoke_edge_trees`) produced no search key or band selector, so the plan's "already reaches Cmd-F…" assumption did not hold. Added a 5% mode-key branch (`cmd-f`, `cmd-k`, `cmd-shift-g`, `/`) in `StepGen::next`. The next 90-step smoke reached search: seed 2 step 21 `cmd-shift-h` opened it and step 22 clicked `search-idle`; it did not sample `cmd-f`, `⇧⌘A` or `⇧⌘M` itself. Final smoke passes (63.6 s). The 5-minute `fuzz_long` is item 4's Finish run. |
| 4 Bench and docs | todo | |

## Questions
- none yet.

## Decisions
- Branch cut from `main` at `26bf08e` (B4 merged) — plan's base.

## Hand checks
- [ ] The user's report: Cmd-F in `~`, Folder, All, hidden on: `.back` and `.backu` both show `~/.backup_excludes` within a moment. Then hidden off: the same.
- [ ] Dot names in Cmd-K: Hidden off: Cmd-K `zshrc` finds it under "Files in …"; `~/ssh` finds `~/.ssh`.
- [ ] Scopes: a miss in a subfolder offers Search Home; ⇧⌘H finds it. This Mac finds a name outside Home.
- [ ] Match modes: All, Names, Contents on one word; a Names miss offers "Search names and contents".
- [ ] Skips: `~` with hidden off: the skipped list groups Excluded, Hidden folders, Packages, Couldn't read; ⇧⌘A lifts the first three.
- [ ] Cap: `target/fixture-search-200k`, `needle`, then an exact file name: top row.
- [ ] VoiceOver: hidden rows read "…, hidden"; the empty-state buttons read with their keys; the Folder segment reads its path; the footer reads once.
- Light and dark, 640 px and wide, for every row.

## Found along the way
- none yet.

## Log
- 2026-11-18: read the plan, the M8.5 index, B1's corpus, the engine and app search code. Created the branch and this file.

## Next
Write `crates/crow-search/src/oracle_tests.rs`, wire it into `lib.rs`, and run it once to see the first disagreement.
