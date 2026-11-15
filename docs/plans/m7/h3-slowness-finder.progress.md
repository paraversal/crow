# M7 H3 — Slowness finder progress
Branch: `m7-h3-slowness` · PR: none · Last updated: 2026-11-15 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 `crow --fuzz` | todo | CLI, real window, H1 steps, frame budget, spans, report, exit code |
| I2 Replay a slow step | todo | `--until <index>` pauses for `xctrace`; document in `docs/testing.md` |
| I3 First runs | todo | 10 min each: 100k fixture, H2 edge tree with churn, the repo |

## Questions
- Q1 (asked 2026-11-15): budget numbers — report over 50 ms, fail over 250 ms? → waiting
- Q2 (asked 2026-11-15): run `crow --fuzz` nightly in CI? → waiting
- Q3 (asked 2026-11-15): does H3 drive mouse clicks? → waiting

## Decisions
- Branch from `main` @ c5698a6 (H1 #30, H2 #31 and H4 batch 1 #32 merged). The plan's base condition (H1 merged, H2 too if it is) is met. — plan.
- H3 is the next M7 piece: the index's suggested order is H1 → H2 → H4 (first batch) → H3. — user + index.

## Hand checks
- [ ] `cargo run --release -p crow-app --features fuzz -- --fuzz --minutes 1` opens a real window on a temp edge tree, never launches an app, never plays a sound, leaves the user's Trash and pasteboard alone, and exits 0 when no frame is over 250 ms.
- [ ] `--fuzz <dir>` without `--dir-is-scratch` refuses to run (never a user folder).
- [ ] `--fuzz --seed N --until M` stops before step M with the window still open, and `xcrun xctrace record --template 'Time Profiler' --attach crow` can attach.

## Found along the way
- (none yet)

## Log
- 2026-11-15: read the plan, M7 index, H1/H2 progress, findings, `docs/testing.md`, the fuzz runner and the app entry points; created the branch and this file.

## Next
Ask Q1–Q3, then implement I1: CLI parsing, temp tree, isolated config/state, RecordingEffects, real-window step dispatch, frame tracer and slow-step report.
