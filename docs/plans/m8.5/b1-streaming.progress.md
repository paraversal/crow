# B1 — Streaming fix, corpus and trace progress

Branch: `m8.5-b1-streaming` · PR: none · Last updated: 2026-11-18 · State: In critique

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 Batcher | done (e6d425f) | `FLUSH` 30 ms, first hit at once, full batches at once. Four pure-clock tests pass; `batches_are_bounded_for_many_files` still passes. |
| 2 Name before contents, regular files only | done (e6d425f) | Separate `tx.send` for the name hit; contents only for regular files and symlinks to regular files. Tests: `content_search_skips_fifos` (10 s timeout), `name_hit_arrives_before_its_content_hit`, `a_symlink_to_a_file_gets_a_content_hit`. |
| 3 Typed skips, unreadable folders | done (fe5c5a6) | `Skip`/`SkipReason::{Rule, Unreadable}` exported; `error_path`/`error_text` unwrap `ignore::Error`; app keeps rule skips only. Test `unreadable_folder_is_reported` passes with `"no permission"`. |
| 4 Corpus | done (fd3c139) | `crow_search::corpus::{build, remove, cases}`; `crow --make-search-corpus DIR` builds it. Fixed bytes, modes 0644/0755/000, relative symlinks, a FIFO, 300 `crowd/` weak matches. |
| 5 Named cases | done (fd3c139) | `crates/crow-search/tests/corpus_cases.rs` runs all 20 cases through the real engine with a 10 s deadline each; all pass. The `unreadable` case skips itself when `read_dir` succeeds as root. |
| 6 Trace and log lines | done (6bd82f2) | `--search-trace` runs the real engine and prints batches, totals, skips and the target verdict; `SearchView` has `started`/`first_hit_ms` and logs start/finish under `crow::search`. |
| 7 Docs | done (79c543f) | `docs/testing.md`: the crow-search tests, `--make-search-corpus`, `--bench-search QUERY`, and a "Search diagnostics" section (`--search-trace`, `RUST_LOG=crow::search=debug`). |

## Questions

None; the plan says there are none and the index's decisions are binding.

## Decisions

- The uncommitted `docs/plans/m8.5/README.md` re-cut and the new batch plans were the user's drafts. They are committed with the progress file so the critique sees the plan in history (`2e67d96`); the executing agent did not change their content.
- Items 1 and 2 share one commit (`e6d425f`): both are the streaming change in the same walker, and the tests for the ordering cannot be separated from the batcher.
- `--search-trace` has two fallback verdicts the plan's list does not name: `no skipped ancestor, and the name matches (not visited?)` and `no skipped ancestor, and names were not searched`. The plan's single "the name does not match" line would be false in those cases; the check still uses the engine's own `FuzzyMatcher`, never a copy of the walk.
- The reported `~/.backup_excludes` is absent on this machine; the home folder has `~/.backup-excludes` instead. The corpus uses the plan's exact name; the GUI hand check should use `~/.backup-excludes`.

## Hand checks

- [x] Bench (branch vs `main`, release, same machine, `target/fixture-search-200k`, 200,544 scanned). `needle` (200k hits): branch first hit 2/1 ms, total 1526/1526 ms; `main` first hit 7/4 ms, total 1553/1515 ms. `back` and `backup`: 0 hits on both (the fixture has neither in names or contents), first hit 0 ms, totals 1456–1495 vs 1473–1535 ms. First hit same or better; total within noise.
- [ ] The user's repro, before and after: on `main` and on the branch, Cmd-F in `~`, Folder, All, Show Hidden Files on; `.back`, then `.backu` → `.backup-excludes` appears within a moment on the branch while the footer still says "Searching…". Record the time. The engine trace below shows the streaming half; the GUI comparison is left for the critic/user.
- [x] `crow --search-trace ~ .backu --hidden` (full home, match All): 15 batches in the first 679 ms (`+3ms batch 1: 1 hits`, `+33ms batch 2: 9 hits`, …), then no further `.backu` name or content hits while the walk kept reading contents; the 300 s job cap stopped it. Results are on screen long before the walk ends.
- [~] `--make-search-corpus target/fixture-search-corpus`, Cmd-F `backup` with hidden on and off. Engine-level: the hit sets are identical to `main` (`backup`, match All) — 21 hits with hidden on and 17 with it off; only the summary differs (branch scans `pipe` and reports the unreadable folder as a sixth skip). `main`'s engine **hangs on the corpus FIFO** and had to run with the pipe removed; the branch finishes with the pipe present. The GUI walk is left for the critic/user.

## Found along the way

- `/tmp/crow-search-corpus-*` folders from killed runs of the earlier tests can survive with a mode-000 `unreadable` child; `chmod -R u+rwx` first when removing them. The corpus test removes its own folder on success.
- The 200k search fixture has no `back`/`backup` names or contents, so those bench queries measure the walk only (0 hits). A `--bench-search` fixture with those names would make the name-hit path measurable.

## Log

- 2026-11-18 Plan read, branch created, progress file and plan docs committed (`25edc2f`, `2e67d96`).
- 2026-11-18 Items 1–2: `Batcher` with pure-clock tests; name hit sent before the content read; contents only for regular files and symlinks to regular files; FIFO test with a 10 s deadline. `cargo test -p crow-search`: 27 passed.
- 2026-11-18 Item 3: `Skip`/`SkipReason`, `error_path`/`error_text`, unreadable folder test; app filters to `Rule` for B1. 28 crow-search tests passed.
- 2026-11-18 Items 4–5: corpus builder with 300 crowd files, symlinks, FIFO and mode-000 folder; `--make-search-corpus`; all 20 corpus cases pass through the real engine.
- 2026-11-18 Item 6: `--search-trace` verdicts exercised on the corpus (name hit, content hit, under-skipped `node_modules`, outside root, name mismatch); `SearchView` start/finish logs at `crow::search`; first-hit ms on both search engines.
- 2026-11-18 Item 7 + checks: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` all pass; `scripts/bundle.sh` built `target/bundle/Crow.app`.
- 2026-11-18 Bench and comparisons: see Hand checks. Release builds of both branches; `main` in a throwaway worktree with `vendor/` linked and the shared target dir, removed afterwards.

## Next

Ready for critique. The critic should run the two GUI hand checks (the home repro, and Cmd-F on the corpus hidden on/off) on the bundle.

## Critique

Critic session: 2026-11-18 · Base: `main` @ 111881e · Head reviewed: af1cfdd + the critique fixes

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | `name_hit_arrives_before_its_content_hit` compared positions inside the flattened hit list. On `main` the name and content hits share one batch (`[[true, false]]`), sent after the content read, so the test passed without the fix. It now asserts the name hit's batch comes before the content hit's; the same probe fails on `main`. | `crates/crow-search/src/engine.rs:933` | code (test) | fix | fixed (critique commit 1) |
| C2 | `unreadable_folder_is_reported` restored the folder to 0755 before checking `read_dir`, so the root check always saw `Ok` and the assertions never ran (the test printed its skip line on every run). The check now happens while the folder is 000; the test asserts the `Unreadable("no permission")` skip and no hit. | `crates/crow-search/src/engine.rs:702` | code (test) | fix | fixed (critique commit 1) |
| C3 | `crow --make-search-corpus` failed on a second run (`File exists`): `mkfifo` and the five `symlink` calls were unconditional, despite `build`'s "creating or overwriting it" contract. `build` now removes stale links and the pipe first; `corpus_can_be_rebuilt_over_itself` covers it. | `crates/crow-search/src/corpus.rs:121` | code | fix | fixed (critique commit 1) |
| C4 | `search_trace` loaded `exclude.toml` without the plan's `io-ok: CLI diagnostic, before any window opens` marker; every other read there has one. | `crates/crow-app/src/main.rs:221` | rules / plan | fix | fixed (critique commit 1) |
| C5 | `--search-trace --target` canonicalizes the target, so a symlink target is compared by its resolved path and can print "not found; … the name matches (not visited?)". | `crates/crow-app/src/main.rs:241` | UX (diagnostic) | note | left |
| C6 | `--search-trace` prints `first_hit=0ms` for a search with no hits, which reads as an instant hit. | `crates/crow-app/src/main.rs:274` | UX (diagnostic) | note | left |
| C7 | The 200k bench fixture's names contain neither `back` nor `backup`, so `--bench-search` measures the content path only, not the new per-name send. Measured separately on `target/fixture-100k` with `file`: no regression. | `docs/testing.md` / bench hand check | code sense (coverage) | note | left |

Checks re-run: `cargo fmt --check` ✓ · `cargo clippy --workspace --all-targets -- -D warnings` ✓ · `cargo test --workspace` ✓ (28 crow-search unit tests, both corpus tests) · corpus 20/20 cases ✓ · `--search-trace` on the corpus: 21 hits hidden on / 17 off (same as `main`), the FIFO does not stall, the unreadable folder is a sixth skip; all five verdicts exercised ✓

Budgets: release probes on `target/fixture-search-200k` (`needle`): branch first hit 1–4 ms, total 1719–1906 ms; `main` 4–5 ms, 1695–1757 ms. On `target/fixture-100k` (`file`, names only): branch 50–72 ms first hit, 790–825 ms total; `main` 51–110 ms, 784–837 ms. Within run-to-run noise; first hit same or better.

Looked at: branch bundle (built after af1cfdd), Home (`~`), Cmd-F, Folder, All, hidden on, `.backu`: `.backup-excludes` visible at 0.7 s while the footer says "Searching… 365 results in 85 files"; still streaming at 2.7 s. The machine has `~/.backup-excludes`, not `~/.backup_excludes`. `main` was compared at the engine level (the `[[true, false]]` probe and the trace), not in its GUI. Light/dark, 640 px and VoiceOver were not walked: B1 changes no pixels, roles, labels or copy, and the only UI change is the skip reason type, filtered to `Rule` in B1.

Verdict: right and better. The user's `.backu` case now streams within a second while the walk continues; the FIFO hang and the silent unreadable folder are fixed at the engine level; the corpus and trace make the next four batches testable. The review found no behavior bug in the fix itself — two regression tests did not test their defect and are fixed, and the corpus builder could not be re-run.

