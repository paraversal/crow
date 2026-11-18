# P9B — Cross-run test and fuzz reports progress
Branch: `codex/m9-p9b-test-dashboard` · PR: none · Base: `fbdb8c1` (M9 P9A, merged) · Last updated: 2026-11-18 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 collector: preserve output, exit, metadata | todo | |
| 1 fuzz events in `fuzz_long` and `crow --fuzz` | todo | |
| 2 JSON report, Markdown summary, CI upload | todo | |
| 3 fixtures: pass, failure, lost, malformed, unknown schema | todo | |
| 4 baseline, smoke, aggregation and replay examples | todo | |
| Acceptance: two revisions in one table; failure keeps exit/log/replay | todo | |

## Questions
- none yet.

## Decisions
- Base is merged `main` `fbdb8c1`; branch `codex/m9-p9b-test-dashboard` per the M9 index. The user's uncommitted M8→done / M9 docs reorganization is left untouched and uncommitted (not mine).
- Event protocol: opt-in `CROW_TEST_EVENTS=1`, one compact JSON object per line prefixed `CROW_EVENT `, schema 1. The collector sets it for `fuzz` and `fuzz-live` modes only; human output and replay syntax stay unchanged.

## Hand checks
- [ ] Live `crow --fuzz` report: run a short authorized or synthetic live report, confirm seed start/end, frame summary and failure replay path.
- [ ] CI artifact upload and step summary on a failing run (cannot exercise from this machine without pushing).

## Found along the way
- none yet.

## Log
- 2026-11-18 Branch created; read the plan, M9 index, CI workflows, `docs/testing.md`, `fuzz_long`, `fuzz.rs` and `fuzz_live.rs`.

## Next
Implement `scripts/test-report.py` (`run`/`summary`) and its fixture tests.
