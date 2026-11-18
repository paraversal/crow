# B5 — Regression and finish progress
Branch: m8.5-b5-regression · PR: none · Last updated: 2026-11-18 · State: Ready for critique

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 Oracle on random trees | done | `crates/crow-search/src/oracle_tests.rs` walks seeded trees with a plain `read_dir` oracle; agrees with the engine on 25 seeds × 4 queries × hidden × everything × match mode (1,200 comparisons; 300 seeds took 57 s, so the plan's "lower the seed count and record it" applies). 4.7 s in debug. Coverage asserts every skip reason and both hit kinds are reached. |
| 2 App-level corpus matrix | done | `search_corpus_matrix` drives the panel through `backup`/`needle` × All/Names/Contents × hidden on/off × everything on/off (24 runs) and compares the path set, done-footer text and skipped count with `crow_search::search` directly. Passes in 3.6 s. |
| 3 Fuzzer | done | First 120-step smoke run (`fuzz_smoke` + `fuzz_smoke_edge_trees`) produced no search key or band selector, so the plan's "already reaches Cmd-F…" assumption did not hold. Added a 5% mode-key branch (`cmd-f`, `cmd-k`, `cmd-shift-g`, `/`) in `StepGen::next`. The next 90-step smoke reached search: seed 2 step 21 `cmd-shift-h` opened it and step 22 clicked `search-idle`; it did not sample `cmd-f`, `⇧⌘A` or `⇧⌘M` itself. Final smoke passes (63.6 s). The 5-minute `fuzz_long` is item 4's Finish run. |
| 4 Bench and docs | done | Docs done (testing oracle/matrix/finish table, README M9 status, M8.5 marked merged and moved to `done/`, links updated). Bench rerun on `main` (`26bf08e`) and the branch: all four rows inside budget (needle 1514.2 vs 1561.7 ms, back 1501.2 vs 1499.9, backup 1473.8 vs 1524.5, names 188.9 vs 190.9; first hit ≤ 2.5 ms). `cargo fmt --check`, clippy and `cargo test --workspace` pass at `fc04736`; `fuzz_long` 5 minutes passed (seed 1795006983320860000, churn off, 313.68 s, no violation). Hand checks done except This Mac, the grouped skipped list and VoiceOver, listed below. |

## Questions
- none yet.

## Decisions
- Branch cut from `main` at `26bf08e` (B4 merged) — plan's base.
- Oracle seeds lowered from 300 to 25: 300 seeds took 57 s in debug, 25 take 4.7 s, inside the plan's ~5 s target — the plan allows this and asks for the number.
- Added a 5% `OPEN_MODE_KEYS` branch (`cmd-f`, `cmd-k`, `cmd-shift-g`, `/`) to the fuzzer: the first 120-step smoke run never opened search — plan.
- Folder move updates `docs/ROADMAP.md`, `docs/plans/inbox.md` and `docs/plans/m8/README.md` links — the move needs them to resolve.

## Bench

`target/fixture-search-200k` (200,000 files), three runs each, median. Commands:
`crow --bench-search target/fixture-search-200k QUERY [--names]`. `main` is `26bf08e`, built in
`../file-crow-b5-main` with the shared target dir; the branch binary was built from this tree. Both
release.

| query | flags | main first hit / total | branch first hit / total | budget |
| --- | --- | --- | --- | --- |
| `needle` | (all) | 1.1 ms / 1561.7 ms | 1.1 ms / 1514.2 ms | pass |
| `back` | (all) | no hit / 1499.9 ms | no hit / 1501.2 ms | pass |
| `backup` | (all) | no hit / 1524.5 ms | no hit / 1473.8 ms | pass |
| `file-01-02` | `--names` | 2.9 ms / 190.9 ms | 2.5 ms / 188.9 ms | pass |

First hit ≤ `main` + 5 ms; total ≤ `main` × 1.10. All pass. `back` and `backup` have no hits because
the fixture's files contain `needle` only, so their numbers are a full walk. B5 changes no engine code.

## Hand checks
- [x] The user's report: Cmd-F in `~`, Folder, All, hidden on: `.back` shows `.backup-excludes` at ~3 s (`m85-backu-home-firebox.png` is the later `.backu` state); `.backu` shows both copies by 10 s (`target/shots/m85-backu-home-firebox.png`), with the footer done at 9 results / 943 skipped. Hidden off: the same (`m85-backu-home-hidden-off-firebox.png`, 8 results, 954 skipped). `--search-trace ~ .backu --hidden --target ~/.backup-excludes` reports `name hit at 4910ms (batch 71)`. This machine's real file is `~/.backup-excludes` (hyphen; B1 noted it).
- [x] Dot names in Cmd-K: hidden off, Cmd-K `zshrc` lists `Files in <home>` with `.zshrc` (`m85-cmdk-zshrc-firebox.png`); `~/ssh` lists `~/.ssh` (`m85-cmdk-ssh-firebox.png`).
- [x] Scopes: a `backup-excludes` miss in `crates/crow-app` offers Search Home ⇧⌘H (`m85-scope-miss-firebox.png`); ⇧⌘H finds both copies (`m85-scope-home-firebox.png`). This Mac was not driven.
- [x] Match modes: Names `needle` misses and offers "Search names and contents" (`m85-match-names-miss-firebox.png`); Contents finds the three corpus files (`m85-match-contents-firebox.png`). All is the default and showed results in the other runs.
- [~] Skips: the footer showed "954 folders skipped ›" and the list is covered by B3's hand check; the grouped list itself was not reopened in this run.
- [x] Cap: `target/fixture-search-200k`, `needle`: "20000 results · 226 more not shown · contents stopped at 20000" (`m85-cap-firebox.png`); typing `file-01-02-00001` makes it the top row (`m85-cap-exact-firebox.png`).
- [ ] VoiceOver: hidden rows read "…, hidden"; the empty-state buttons read with their keys; the Folder segment reads its path; the footer reads once. Not driven.
- [x] Light and dark, 640 px and wide: FireBox and `solarized-light` at 1280×800 (`m85-backu-home-hidden-off-solarized-light.png`); 640×800 (`m85-backu-home-hidden-off-640-firebox.png`) keeps the footer on one line and the hidden rows dimmed. VoiceOver, This Mac and the grouped skipped list remain for the user's hand test.

## Found along the way
- The 78k-file palette test took 738 ms in CI, exceeding its 650 ms debug cutoff. The follow-up keeps all five queries and checks result counts, the top hit and its highlight. `docs/testing.md` gives the release command for comparing the same workload against `main`.
- `docs/ROADMAP.md` still says M8 is in progress and its M8 section says F1 only; M8 is merged on `main` (through #55) but its index/progress files were not moved. Left alone; the M8.5 links were updated.

## Log
- 2026-11-18: read the plan, the M8.5 index, B1's corpus, the engine and app search code. Created the branch and this file.
- 2026-11-18: oracle and app matrix done. The oracle agrees; it walks 25 seeded trees, 1,200 comparisons, 4.7 s. The matrix compares 24 panel runs with the engine, 3.6 s.
- 2026-11-18: fuzzer check. The first 120-step smoke had no search step, so the generator gained a 5% mode-key branch; the next run opened search via ⇧⌘H and clicked `search-idle`. Final smoke passes.
- 2026-11-18: docs done: testing oracle/matrix/finish table, README status, M8.5 marked merged and moved to `done/`, roadmap/inbox/M8 links updated.
- 2026-11-18: bench done on `main` (`../file-crow-b5-main`, shared target) and the branch; all four rows inside budget, branch ≤ main on total and first hit. Full check gate passed (`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, 81 s).
- 2026-11-18: hand checks on the branch bundle: `.back`/`.backu` with hidden on and off, Cmd-K dot names, Search Home from a miss, Names miss and Contents hits, the 200k cap and a late exact name, FireBox/Solarized Light, 1280 and 640. `--search-trace ~ .backu --hidden` put the name hit at 4.9 s (batch 71). This machine's file is `~/.backup-excludes`.
- 2026-11-18: `fuzz_long` 5 minutes: seed 1795006983320860000, churn off, 313.68 s, passed with no violation. Log `/tmp/crow-fuzz-long-b5.log`.
- 2026-11-18: palette CI follow-up passed `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` outside the sandbox (823 passed, 1 ignored).

## Next
Ready for critique. The critic opens the PR; This Mac, the grouped skipped list and VoiceOver remain for the user's hand test.
