# P9B — Cross-run test and fuzz reports progress
Branch: `codex/m9-p9b-test-dashboard` · PR: none · Base: `fbdb8c1` (M9 P9A, merged) · Last updated: 2026-11-18 · State: Ready for critique

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 collector: preserve output, exit, metadata | done | `scripts/test-report.py`: no shell, merged streaming to terminal + `run.log`, atomic `run.json`, per-binary cargo counts and full failing test output, exit/signal preserved, `empty` vs `passed`, relative logs/replays, recursive summary with malformed/unknown schema skipped. |
| 1 fuzz events in `fuzz_long` and `crow --fuzz` | done | `src/fuzz_events.rs` emits opt-in JSON lines (`CROW_TEST_EVENTS=1`). Headless seeds carry tree/churn/requested/executed, failure invariant/index and a Rust replay artifact. Live carries the same seed events, frame summary (heartbeats, 50 ms threshold, over-budget, worst), failure `--script`/`--until` and a kept replay artifact. |
| 2 JSON report, Markdown summary, CI upload | done | `ci.yml` and `fuzz.yml` wrap the existing commands, append the collector summary with `if: always()`, and upload `test-report-workspace` / `test-report-fuzz` with `upload-artifact` v4.6.2. Not run on GitHub from this machine; the wrapper and YAML were reviewed locally. |
| 3 fixtures: pass, failure, lost, malformed, unknown schema | done | `scripts/test_report_test.py`: 13 cases (pass, test failure, signaled child, empty, seed pass/live frames, GPUI failure after start, start without end, malformed/unknown events, seed failure+end pairing, env non-copy, two revisions, GPU absence vs zero). |
| 4 baseline, smoke, aggregation and replay examples | done | `docs/testing.md` documents run/summary, events, aggregation, headless and live replay, fixtures and the first baseline at `1220af2a13ad`; workspace 905 passed / 0 failed / 1 ignored in 16.1 s, smoke 2 passed in 4.0 s, host `Julians-MacBook-Pro.local` arm64. |
| Acceptance: two revisions in one table; failure keeps exit/log/replay | done | Fixtures cover two revisions and all failure shapes. Real failing subprocess: exit 7 preserved, `end-of-log` in `run.log`, report `failed`. Real headless fuzz: seed 1, 2 steps, passed, one seed event pair. Live path exercised by a synthetic event record; no GUI live run. |

## Questions
- none yet.

## Decisions
- Base is merged `main` `fbdb8c1`; branch `codex/m9-p9b-test-dashboard` per the M9 index. The user's uncommitted M8→done / M9 docs reorganization is left untouched and uncommitted (not mine).
- Event protocol: opt-in `CROW_TEST_EVENTS=1`, one compact JSON object per line prefixed `CROW_EVENT `, schema 1. The collector sets it for `fuzz` and `fuzz-live` modes only; human output and replay syntax stay unchanged.
- Report result values: `passed`, `failed`, `interrupted`, `empty`, `incomplete`, `paused`, `collector-error`. A zero exit with no test result or seed evidence is `empty`; a collector error is a separate `collector_error` field so the child's `exit_code` stays actual.
- Live frame numbers are broken out in a second summary table. A missing `frame-summary` event renders `—`, never `0`.

## Hand checks
- [x] Synthetic live record: collector decoded seed start/end, frame summary (10 heartbeats, 50 ms threshold, 0 over, 0.0 ms worst), and an unavailable frame summary rendered `—` rather than `0`.
- [ ] Real live `crow --fuzz` report: run a short GUI session and confirm the same events/frame table. Not run here; the live code compiles under `--features fuzz` and the synthetic decoder path is tested.
- [ ] CI artifact upload and step summary on a failing run: requires a GitHub run; YAML reviewed locally, not executed.

## Found along the way
- none yet.

## Log
- 2026-11-18 Branch created; read the plan, M9 index, CI workflows, `docs/testing.md`, `fuzz_long`, `fuzz.rs` and `fuzz_live.rs`.
- 2026-11-18 Implemented `scripts/test-report.py` and 13 fixture tests; all pass. Wrapped `sh -c '… exit 7'`: collector returned 7, `run.log` ended `end-of-log`, report was `failed` with `failure_output`.
- 2026-11-18 Added `fuzz_events.rs`; `fuzz_long` emits seed start/end/failure with Rust replay; live emits seed start/end/failure, frame summary, freeze/invariant failures, and HANG. Both configurations compile; clippy clean on `crow-app` tests and `--features fuzz`.
- 2026-11-18 Wrapped `ci.yml` Tests and `fuzz.yml` Fuzz with the collector; added always-on summary and artifact upload (`upload-artifact` v4.6.2).
- 2026-11-18 Full gate: `cargo fmt --check`, workspace clippy, fuzz-feature clippy, fuzz-bin check and `cargo test --workspace` all pass. The collector Python fixtures pass (13).
- 2026-11-18 Baseline: workspace `workspace-20261118T170422Z` passed 905/0/1 in 16.1 s; smoke `smoke-20261118T170438Z` passed 2 in 4.0 s, both at `1220af2a13ad`. Reports are under `target/test-reports/`; `docs/testing.md` has the commands, aggregation example, replay shapes and baseline table.
- 2026-11-18 Final live-only change: a generated HANG now carries the kept step script as the replay artifact; a user-supplied `--script` is never copied.

## Next
Commit the docs and progress, then stop at **Ready for critique**. No push, no PR.
