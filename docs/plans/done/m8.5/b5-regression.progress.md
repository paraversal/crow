# B5 — Regression and finish progress
Branch: m8.5-b5-regression · PR: none · Last updated: 2026-11-18 · State: In critique

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
After B5 is hand-tested and merged, start [M9 P1](../../m9/README.md), the public Rhai plugin API. The M9 index records the decisions to confirm before implementation.

## Critique

Critic session: 2026-11-18 · Base: `main` at `26bf08e` · Original head reviewed: `a607555`

### Literal findings

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | A regular file or symlink could be replaced by a FIFO after classification, stalling the content open. | `crates/crow-search/src/engine.rs` | code | fix | Open without blocking and check the opened descriptor before reading. The FIFO test also exercises this race window directly. |
| C2 | The oracle's extra fixture entries could exceed its size and depth limits, and the failure listing printed the link names as their targets. | `crates/crow-search/src/oracle_tests.rs` | plan | fix | Reserve room for the extras, bound the depth and print the actual targets. |
| C3 | The app corpus matrix used view setters, bypassing the match controls and the window's hidden-file action. | `crates/crow-app/src/window/tests.rs` | tests | fix | Drive the controls in all 24 runs, still comparing paths, counts and skips against the engine. |
| C4 | The roadmap credited the B4 PR with all five merges. The index still described a package dependency superseded by the user's B3 decision. The cap instructions attributed a new query's result to the previous walk. | `docs/ROADMAP.md`, M8.5 index, `docs/testing.md` | docs | fix | Record the correct B5 closeout, reference B3 Q1 and distinguish the hand check from the same-walk automated check. |
| C5 | One outgoing progress-note commit contained a local account name. | B5 progress history | publication | fix | Redacted the two affected unpushed commits and verified the content-only change. The scan's oracle home path is a synthetic test fixture. |

### Semantic findings

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C6 | In Cmd-K, typing `ΟΣΑ` after `ΟΣ` could lose `οσα.txt`. Greek final sigma changes form during lowercasing, invalidating the cached prefix. | `crates/crow-app/src/palette.rs` | correctness | fix | Check the lowercase prefix before reusing candidates. Reproduced first and added a regression test. |
| C7 | At 640 px, a long folder name left the skip switch at x=621–784 and mostly outside the window. | Search scope bar | UI | fix | Let the folder label truncate first. Keep the short scope labels and the skip switch reachable. |
| C8 | A settled miss still shows the existing Return to results hint. After content reads stop at the cap, the footer's omitted count describes only hits observed so far. | Bottom bar, search footer | UX | note | Both are minor copy follow-ups. Documented here with no change to the agreed cap behavior. |

### Verification

- `cargo fmt --check`, clippy with warnings denied, and the workspace suite passed with 825 tests passed and one ignored. The workspace tests use normal macOS access for metadata and directory-watch events.
- The final bundle's trace found the synthetic `.backup_excludes` with Show Hidden Files off and reported all four skip reasons. Search Everything found the name inside `.backup` and left only the unreadable folder in the skip list.
- The GPUI scope test clicks the skip switch at 1280 and 640 px. At 640 px it also checks the full set of enabled exclusion groups.
- The executor's five-minute fuzz log shows seed `1795006983320860000`, churn off, and a pass after 313.68 s. The critique reran smoke in the workspace suite.
- Search benchmark, median of three interleaved runs on `target/fixture-search-200k`. Times are first hit / total in ms. `back` and `backup` have no hits in this fixture.

| Query | `main` | B5 with fixes |
| --- | ---: | ---: |
| `needle` | 1.1 / 1491.7 | 1.1 / 1480.4 |
| `back` | — / 1437.2 | — / 1416.5 |
| `backup` | — / 1426.3 | — / 1411.7 |
| `file-01-02 --names` | 3.6 / 186.8 | 2.4 / 183.6 |

All four rows meet B4's first-hit and total budgets. The signed bundles built successfully. Desktop automation timed out, so visual review used the executor's screenshots and GPUI rendered bounds. Hand-test This Mac against the live Spotlight index, the grouped skipped list with a mouse, and the VoiceOver labels and footer announcement before merging.

Verdict: B1–B4 address the reported empty search, make hidden names findable and give misses a path forward. B5 adds independent walk coverage and verifies that the UI carries engine results and skips through to the panel. The remaining checks need the bundled app.
