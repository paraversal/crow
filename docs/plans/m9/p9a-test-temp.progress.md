# P9A — Own temporary fixtures for their full lifetime progress
Branch: `codex/m9-p9a-test-temp` · PR: [#62](https://github.com/paraversal/file-crow/pull/62) · Base: `083d40c` (M9 P1A, merged) · Last updated: 2026-11-18 · State: PR open

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 owning guards, test config global | done | `tempfile 3.27.0` is a workspace dev dependency. `ui::init_for_tests` stores every config root in a test-only `TestRoots` GPUI global and returns the config path; a second init appends, so a live root is never deleted. Tests cover repeated init, app-drop cleanup after a window, and cleanup on unwind. The theme index moved from process-wide `THEMES_DIR` env mutation into `Ui.themes_dir`/`themes_state_dir`, with per-test fixtures. |
| 2 convert hand-rolled fixtures | done | `crow-core` (settings, exclude, onboarding, paths, tips, usage, extensions), `crow-fs` (listing, ops, sizes, cache, snapshot, testing, trash, watch), `crow-macos` (fsevents, metadata), `crow-search` (engine, oracle, corpus integration), `crow-theme`, and `crow-app` (window fixtures/corpus/fuzz trees, settings, info, trash, UI). `CleanTree`/`TestParent` restore mode-000 modes before deleting. Window fixtures are children of an owned `/tmp/crow-test-*` parent, which also owns the sibling `.trash-*`. |
| 3 fuzz_live ownership | done | `prepare` has a `ScratchGuard` that removes a half-built support/tree on error; `TreeSetup`/`cleanup` keep the existing normal/failure contract. A supplied `--dir-is-scratch` tree is never guarded. The forced-kill boundary is documented in the file and `docs/testing.md`; no prefix sweep. |
| 4 leak check script | done | `scripts/check-test-temp.sh`: private `mktemp` parent, tests under a child `TMPDIR`, pre/post inventory of the canonical `/private/tmp` for `crow-*` and `.trash-*` (a bare `find /tmp` does not descend the symlink), test output untouched, exits with the test status or 1 on leftovers. Final run: 20.5 s, 74,669 pre-existing `/tmp` fixtures, zero new. |
| Acceptance | done | `./scripts/check-test-temp.sh` passes; full workspace tests pass after it; `fuzz_smoke` and `deleting_the_shown_folder_keeps_the_invariants` pass. Live `crow --fuzz` paused/failed/killed behavior is unchanged and not exercised (GUI). |
| CI: P1A cancel test flake (out of P9A) | reported (P1A fix §2) | main CI run `44346035656` failed only `crow-plugin` `host::tests::ticket_cancel_during_evaluation_is_cancelled_and_keeps_the_plugin_running`: got `WorkerFailed("operation budget exhausted")`, expected `Cancelled`. `m9-p1a-fix.md` §2 owns authoritative cancellation; not patched in the P9A lane. |

## Questions
- Q1 (asked 2026-11-18, resolved by the audit): the request said "I think the first part is already done". No P9A progress file, branch or guard code existed on `main` at `083d40c`; `ui::init_for_tests` leaked one root per call. Implemented item 1 from scratch.
- Q2 (asked 2026-11-18, not blocking): CI on `main` failed in the P1A cancellation test. Recorded, not fixed here; [P1A fix §2](m9-p1a-fix.md) rewrites cancellation. The PR run on `codex/m9-p1a-plugin-runtime` was still queued when checked.

## Decisions
- Base is merged `main` `083d40c`; branch `codex/m9-p9a-test-temp` per the M9 index. The user's uncommitted M8→done / M9 docs reorganization is left untouched and uncommitted (not mine).
- Spike (2026-11-18): GPUI's test teardown runs `cx.quit()` then drops the `TestAppContext`. `App` fields drop in declaration order, so view entities, windows and executors drop before `globals_by_type`; a nested `TestAppContext` with an opened window showed the view's `Drop` running before the global's `Drop`, and the global saw the window root still present. A detached job that is never drained can outlive `App` through the test scheduler's executor cycle, but no Crow test job reads the config root; the observed boundary is recorded here.
- Palette/window fixtures stay under `/tmp` (not `$TMPDIR`): palette scoring sees whole paths and a random `$TMPDIR` prefix caused the known CI flake. They are owned children of `/tmp/crow-test-*`; the leak check inventories only new `/tmp` entries and never sweeps.
- The theme picker tests used a process-wide `THEMES_DIR` mutation and a never-cleaned process fixture. `Ui` now carries `themes_dir`/`themes_state_dir` (production defaults still come from the env), so each test installs its own index without touching process env.

## Hand checks
- [x] `./scripts/check-test-temp.sh` → 20.5 s, full workspace clean; 74,669 pre-existing `/tmp` Crow fixtures untouched and zero new in the private `TMPDIR` or `/tmp`.
- [x] `cargo test -p crow-app --bin crow window::tests::fuzz_smoke` and `window::tests::deleting_the_shown_folder_keeps_the_invariants` → pass.
- [ ] Live `crow --fuzz` paused/failed run: keeps `crow-fuzz-*` for replay and a forced kill still bypasses destructors (GUI run; contract unchanged).

## Found along the way
- `main` CI `44346035656` (push of P1A `083d40c`) failed in `crow-plugin/src/host.rs:1885` `ticket_cancel_during_evaluation_is_cancelled_and_keeps_the_plugin_running`: the LOOP script exhausted Rhai's operation budget before the 5 ms cancel was observed (`WorkerFailed` vs `Cancelled`). Outside P9A; cancellation ownership is [P1A fix §2](m9-p1a-fix.md). Recorded in the checklist at the user's request.
- Pre-existing global temp litter measured before this work: 176,543 `crow-*` entries in the user's `TMPDIR` and 38,521 under `/tmp`. Left untouched for the separately authorized cleanup the plan names; the new check uses a fresh parent and only reports entries created during its run.

## Log
- 2026-11-18 Branch created; audit of `temp_dir(`/`TempDir`/`remove_dir_all`/`/tmp` across crates and scripts.
- 2026-11-18 Checked `gh run list` after the user reported CI: `main` failed on the P1A cancel test above. Spike confirmed global/window drop order.
- 2026-11-18 Item 1: `TestRoots` global + workspace `tempfile`, config lifetime tests, per-test theme index on `Ui`.
- 2026-11-18 Item 2: converted core/fs/macos/search/theme/app fixtures; added owned parent guards for `/tmp` fixtures and their trash siblings; mode-restoring cleanup for corpus/fuzz/unreadable trees.
- 2026-11-18 Item 3: `ScratchGuard` in `fuzz_live::prepare`; documented the forced-kill boundary.
- 2026-11-18 Item 4: added `scripts/check-test-temp.sh` and the `docs/testing.md` section. Final check: 20.5 s, 74,669 pre-existing `/tmp` fixtures, zero new. Found and fixed that `find /tmp` skips the symlink; the script now canonicalizes to `/private/tmp`. `cargo fmt --check`, workspace clippy, fuzz-feature clippy and fuzz-bin check pass.

## Next
- Commit the work and set **Ready for critique**. No push, no PR; the critic opens it.

## Critique
Critic session: 2026-11-18 · Base: `083d40c` · Head reviewed: `de61b16`

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | A run that fails after `prepare` leaves the tool-created support folder and tree. Repro: `TMPDIR=$(mktemp -d) cargo run -p crow-app --features fuzz -- --fuzz --script /nonexistent` left `crow-fuzz-support-*` and `crow-fuzz-tree-*` (with the unreadable edge kinds); window-open failures and an app exit with no outcome had the same hole. | `crates/crow-app/src/fuzz_live.rs:160-220`, `:243-254`, `:319-326` | plan / code | fix | fixed (9397dab) |
| C2 | `Ui.themes_state_dir` is not used by the write path: Apply to All Apps calls `crow_theme::set_global(theme)`, which recomputes the process `state_dir()`. In a test with a fixture index that writes the developer's `~/.local/state/themes/current.json`; the progress's "per-test theme index" claim only covered reads. | `crates/crow-theme/src/lib.rs:324`, `crates/crow-app/src/ui.rs:769` | plan / code | fix | fixed (8c1822e) |
| C3 | The "cleanup on unwind" test checked `tempfile::TempDir` directly, not the `TestRoots` config global (the source of the 69,708 leaked config roots). Replaced it with a nested GPUI app that panics and asserts the global removes the root. The fixture doc comments also still said the paths were canonicalized after `TestRoot` stopped canonicalizing. | `crates/crow-app/src/ui.rs:1045`, `crates/crow-app/src/window/tests.rs:115,124` | code / honesty | fix | fixed (8c1822e) |
| C4 | `TestRoot`/`CorpusRoot`/`FuzzRoot` and `TestParent` both remove the same trees, and the three crate-local `test_support` modules duplicate a small `TempDir` wrapper. Harmless; a shared test-support crate would add a dependency for ~60 lines. | `crates/crow-app/src/window/tests.rs:41-135`, `crates/crow-*/src/lib.rs` | code sense | note | left (boring duplication, test-only) |
| C5 | The plan's "existing replay still reproduces deterministically" has no recorded replay in the progress; only `fuzz_smoke` (three fixed seeds) was run. The live paused/failed/killed run remains a GUI hand check. | progress Acceptance / Hand checks | honesty | note | left (recorded for the user) |

Checks re-run: `cargo fmt --check` ✓ · `cargo clippy --workspace --all-targets -- -D warnings` ✓ · `./scripts/check-test-temp.sh` ✓ (full workspace, 20.15 s, clean in the private `TMPDIR` and `/tmp`) · new fuzz-feature unit tests ✓ · new config-unwind and apply-everywhere tests ✓ · `TMPDIR=$(mktemp -d) cargo run -p crow-app --features fuzz -- --fuzz --script /nonexistent` now leaves nothing ✓
Looked at: the diff, both new tests, the fuzz error paths, `scripts/check-test-temp.sh`, `docs/testing.md`, and the fixture lifecycle in the test harness. UI: no user-facing surface changed, so no light/dark, size or VoiceOver pass was needed; the theme picker's Apply to All Apps is covered by the new test.
Verdict: better. The suite and the 100k-scale fixtures now clean up after themselves, the check script is reproducible, and the two remaining leak paths (fuzz startup errors, theme state writes) are closed.
