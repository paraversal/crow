# B5 — Regression and finish progress
Branch: m8.5-b5-regression · PR: none · Last updated: 2026-11-18 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 Oracle on random trees | todo | |
| 2 App-level corpus matrix | todo | |
| 3 Fuzzer | todo | |
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
