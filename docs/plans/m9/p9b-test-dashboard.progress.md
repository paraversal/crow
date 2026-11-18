# P9B — Cross-run test and fuzz reports progress
Branch: `codex/m9-p9b-test-dashboard` · PR: none · Base: `fbdb8c1` (M9 P9A, merged) · Last updated: 2026-11-18 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 collector: preserve output, exit, metadata | done | `scripts/test-report.py`: no shell, merged streaming to terminal + `run.log`, atomic `run.json`, per-binary cargo counts and full failing test output, exit/signal preserved, `empty` vs `passed`, relative logs/replays, recursive summary with malformed/unknown schema skipped. |
| 1 fuzz events in `fuzz_long` and `crow --fuzz` | done | `src/fuzz_events.rs` emits opt-in JSON lines (`CROW_TEST_EVENTS=1`). Headless seeds carry tree/churn/requested/executed, failure invariant/index and a Rust replay artifact. Live carries the same seed events, frame summary (heartbeats, 50 ms threshold, over-budget, worst), failure `--script`/`--until` and a kept replay artifact. |
| 2 JSON report, Markdown summary, CI upload | in progress | Report/summary done; CI wrappers and artifact upload written, not yet CI-run. |
| 3 fixtures: pass, failure, lost, malformed, unknown schema | done | `scripts/test_report_test.py`: 13 cases (pass, test failure, signaled child, empty, seed pass/live frames, GPUI failure after start, start without end, malformed/unknown events, seed failure+end pairing, env non-copy, two revisions, GPU absence vs zero). |
| 4 baseline, smoke, aggregation and replay examples | todo | |
| Acceptance: two revisions in one table; failure keeps exit/log/replay | in progress | Fixtures cover both. Real failing subprocess: exit 7 preserved, `end-of-log` in `run.log`, report `failed`. Real headless fuzz: seed 1, 2 steps, passed, one seed event pair. |

## Questions
- none yet.

## Decisions
- Base is merged `main` `fbdb8c1`; branch `codex/m9-p9b-test-dashboard` per the M9 index. The user's uncommitted M8→done / M9 docs reorganization is left untouched and uncommitted (not mine).
- Event protocol: opt-in `CROW_TEST_EVENTS=1`, one compact JSON object per line prefixed `CROW_EVENT `, schema 1. The collector sets it for `fuzz` and `fuzz-live` modes only; human output and replay syntax stay unchanged.
- Report result values: `passed`, `failed`, `interrupted`, `empty`, `incomplete`, `paused`, `collector-error`. A zero exit with no test result or seed evidence is `empty`; a collector error is a separate `collector_error` field so the child's `exit_code` stays actual.
- Live frame numbers are broken out in a second summary table. A missing `frame-summary` event renders `—`, never `0`.

## Hand checks
- [ ] Live `crow --fuzz` report: run a short authorized or synthetic live report, confirm seed start/end, frame summary and failure replay path.
- [ ] CI artifact upload and step summary on a failing run (cannot exercise from this machine without pushing).

## Found along the way
- none yet.

## Log
- 2026-11-18 Branch created; read the plan, M9 index, CI workflows, `docs/testing.md`, `fuzz_long`, `fuzz.rs` and `fuzz_live.rs`.
- 2026-11-18 Implemented `scripts/test-report.py` and 13 fixture tests; all pass. Wrapped `sh -c '… exit 7'`: collector returned 7, `run.log` ended `end-of-log`, report was `failed` with `failure_output`.
- 2026-11-18 Added `fuzz_events.rs`; `fuzz_long` emits seed start/end/failure with Rust replay; live emits seed start/end/failure, frame summary, freeze/invariant failures, and HANG. Both configurations compile; clippy clean on `crow-app` tests and `--features fuzz`.
- 2026-11-18 Wrapped `ci.yml` Tests and `fuzz.yml` Fuzz with the collector; added always-on summary and artifact upload (`upload-artifact` v4.6.2).

## Next
Run fmt, full workspace clippy/tests, commit, then record the first baseline and write the reporting section into `docs/testing.md`.
