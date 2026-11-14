# M7 H3 — Slowness finder (`crow --fuzz`)

Audience: an implementing agent. Follow `.claude/skills/execute-plan/SKILL.md`; read `AGENTS.md`, `docs/plans/m7/README.md`, `h1-fuzzer.md` and `h2-edge-trees-and-churn.md` first.
Base: branch `m7-h3-slowness` from `main` **after H1 is merged** (H2 too if it is; ASK otherwise). Progress file: `docs/plans/m7/h3-slowness-finder.progress.md`.
PR title: "M7 H3: slowness finder". Don't merge.

## Why
The test platform has no real frames or real disk timing, so H1 can't see a freeze. Spec principle 2: no frame ever waits on the disk. `done/u7-fix.md` X6 found a 3–5 s main-thread block only after the user hit it. H3 runs H1's generator in the real app with a frame-time budget, so slow paths show up with a reproducible step.

## Current state
- `--bench <dir>` (`crates/crow-app/src/bench.rs`, run from `main.rs`) measures first paint and scroll p99 on the 100k fixture.
- X6 used a temporary 50 ms heartbeat on the main thread and `xctrace`; nothing permanent remains.
- H1's generator and invariants exist behind the `fuzz` feature (or under another name; check H1's progress file).

## Items

### I1 `crow --fuzz`
**Change:** `cargo run --release -p crow-app --features fuzz -- --fuzz <dir> [--seed N] [--minutes M] [--tree <edge kinds>] [--churn]`: opens a real window on `<dir>` (a temp copy of an H2 tree by default, never a user folder unless `--dir-is-scratch` is passed), installs `RecordingEffects` (no apps launched), dispatches H1 steps through the real window (keystrokes via `Window::dispatch_keystroke`, clicks via the same selector map), at a realistic pace (one step per frame, or per 16 ms), and checks H1's invariants every N steps.
**Frame budget:** record every frame's duration (GPUI's frame callback / `on_next_frame` timestamps, or a 16 ms main-thread heartbeat measuring its lateness). A frame over 50 ms is a **slow step**: record seed, step index, the step, the frame time, and the last 5 steps.
**Spans:** wrap the suspects in `tracing` spans at `debug` (listing apply, `rebuild_rows`, icon fetch, search batch, preview load, theme reload); on a slow step print the spans that overlapped it. Keep the spans (cheap when off); remove any ad hoc timers.
**Report:** at the end, a table sorted by worst frame: step kind, count over budget, worst ms, seed+index to replay. Exit code 1 if any frame exceeds 250 ms (a freeze), 0 otherwise.

### I2 Replay a slow step
`crow --fuzz … --seed N --until <index>` replays to just before the slow step and pauses, so it can be profiled: `xcrun xctrace record --template 'Time Profiler' --attach crow`. Document it in `docs/testing.md`.

### I3 First runs
On the 100k fixture, an H2 edge tree with churn, and the repo itself: 10 minutes each, release build. Slow steps → `findings.md` with frame time, spans and the replay command. Don't fix here (H4), unless the cause is the fuzz harness itself.

## Finish
- fmt · clippy · tests (with and without `--features fuzz`: `cargo clippy -p crow-app --features fuzz --all-targets -- -D warnings`). CI gets the `--features fuzz` clippy line so the feature doesn't rot.
- Bench vs `main`: the spans must not move first paint or scroll p99.
- Commit, push, "Ready for critique".

## Questions to ASK
1. **Budget numbers:** report over 50 ms, fail over 250 ms? Recommended: yes; AGENTS.md's scroll budget (p99 < 1.5 × frame) stays the bench's job, not H3's.
2. **Should `crow --fuzz` also run nightly in CI?** A real window needs a logged-in GUI session; GitHub's macOS runners have one but are slow and noisy. Recommended: no, local only; nightly CI runs H1's `fuzz_long`.
3. **Does H3 drive mouse clicks?** Recommended: keys and typing only at first (stable in a real window); clicks once keys are clean.
