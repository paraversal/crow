# P9A — Own temporary fixtures for their full lifetime progress
Branch: `codex/m9-p9a-test-temp` · PR: none · Base: `083d40c` (M9 P1A, merged) · Last updated: 2026-11-18 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 owning guards, test config global | todo | `ui::init_for_tests` still creates `crow-app-config-<pid>-<n>` per call and never removes it; no prior P9A work on this branch |
| 2 convert hand-rolled fixtures | todo | audit below |
| 3 fuzz_live ownership | todo | audit below |
| 4 leak check script | todo | no `scripts/` check yet |
| Acceptance | todo | full workspace run, zero leftovers, replay parity |
| CI: P1A cancel test flake (out of P9A) | deferred to P1A fix §2 | main CI run `44346035656` failed only `crow-plugin` `host::tests::ticket_cancel_during_evaluation_is_cancelled_and_keeps_the_plugin_running`: got `WorkerFailed("operation budget exhausted")`, expected `Cancelled`. Reproduced from the log, not locally yet. `m9-p1a-fix.md` §2 already owns authoritative cancellation; do not patch it in the P9A lane. |

## Questions
- Q1 (asked 2026-11-18, not blocking): the request says "I think the first part is already done". No P9A progress file, branch or guard code exists on `main` at `083d40c`; `ui::init_for_tests` still leaks one root per call. Proceeding with item 1 from scratch unless the user points at the prior work.
- Q2 (asked 2026-11-18, not blocking): CI on `main` failed in the P1A cancellation test (status row above). Fix now on this branch, or leave it to the P1A fix lane that rewrites cancellation? Recommended: leave it to P1A fix §2; P9A only records it. The pending PR run on `codex/m9-p1a-plugin-runtime` may show the same.

## Decisions
- Base is merged `main` `083d40c`; branch `codex/m9-p9a-test-temp` per the M9 index. The user's uncommitted M8→done / M9 docs reorganization is left untouched and uncommitted (not mine).
- Spike (2026-11-18): GPUI's test teardown runs `cx.quit()` then drops the `TestAppContext`. `App` fields drop in declaration order, so view entities, windows and executors drop before `globals_by_type`; a nested `TestAppContext` with an opened window and a detached pending job showed the view's `Drop` running before the global's `Drop`, and the global saw the window root still present. Therefore a test-only GPUI global is a sound owner for the config root. A detached job that is never drained can outlive `App` via the test scheduler's executor cycle (the macro calls `dispatcher.drain_tasks()` after `drop(cx)`), but no Crow test job reads the config root; the observed boundary is recorded here.

## Hand checks
- [ ] Later: none expected beyond running the leak check and one existing replay.

## Found along the way
- `main` CI `44346035656` (push of P1A `083d40c`) failed in `crow-plugin/src/host.rs:1885` `ticket_cancel_during_evaluation_is_cancelled_and_keeps_the_plugin_running`: the LOOP script exhausted Rhai's operation budget before the 5 ms cancel was observed (`WorkerFailed` vs `Cancelled`). Outside P9A; cancellation ownership is [P1A fix §2](m9-p1a-fix.md). Recorded in the checklist at the user's request.

## Log
- 2026-11-18 Branch created; initial audit of `temp_dir(`/`TempDir`/`remove_dir_all`/`/tmp` across crates and scripts.
- 2026-11-18 Checked `gh run list` after the user reported CI: `main` failed on the P1A cancel test above; the PR run was still queued. Spike confirmed the global/window/job drop order.

## Next
- Implement the `crow-app` config guard (`TestRoots` global + `tempfile` workspace dev dependency).
