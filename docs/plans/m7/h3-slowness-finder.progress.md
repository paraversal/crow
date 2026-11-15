# M7 H3 — Slowness finder progress
Branch: `m7-h3-slowness` · PR: none · Last updated: 2026-11-16 · State: Ready for critique

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 `crow --fuzz` | done | CLI, temp/external tree, isolated config/state/Trash, RecordingEffects, keys+typing, 16 ms heartbeat, spans, freeze/hang watchdog, report, exit code. 10-minute runs below; bench parity holds. |
| I2 Replay a slow step | done | `--until <index>` settles, pauses the window, prints the tree and the `xctrace` command; `docs/testing.md` documents it. Smoke-tested. |
| I3 First runs | done | 100k seed 404: 607.6 s, 75 slow / 3 freezes (worst 5.2 s, #18); seed 303 was killed at 542 s (#14). Edge + churn seed 304: 650 s, 146 slow / 4 freezes / 6 listing violations, 58.7 s HANG (#15/#16). Repo copy seed 305: 600 s, 3 slow / 1 focus violation (#17). All three 10-minute runs completed. |

## Questions
- Q1 (asked 2026-11-15): budget numbers — report over 50 ms, fail over 250 ms? → **yes, 50/250** (user).
- Q2 (asked 2026-11-15): run `crow --fuzz` nightly in CI? → **no, local only**; the nightly workflow keeps H1's `fuzz_long` (user).
- Q3 (asked 2026-11-15): does H3 drive mouse clicks? → **keys and typing only** for the first version; clicks later (user).

## Decisions
- Branch from `main` @ c5698a6 (H1 #30, H2 #31 and H4 batch 1 #32 merged). The plan's base condition (H1 merged, H2 too if it is) is met. — plan.
- H3 is the next M7 piece: the index's suggested order is H1 → H2 → H4 (first batch) → H3. — user + index.
- Q1/Q2/Q3 above. — user.
- **Measurement is a 16 ms main-thread heartbeat, not `on_next_frame`.** First smoke with the frame callback recorded only 3 callbacks over 39 steps on the 100k fixture and then reported a bogus 973 ms `Resize` freeze; GPUI throttles `on_next_frame` for an inactive/occluded window. A foreground task awaiting `background_executor().timer(16 ms)` measures the main thread directly, which is the plan's stated alternative. — measured + plan.
- Clicks use `window.dispatch_keystroke`; the generator's selector list is cleared so its click rolls fall back to keys (Q3). Right-click/click steps from H1 scripts are no-ops here.
- `Step::AdvanceClock` is a no-op in the real app; real timers run on the 16 ms step pace.
- **The target's own root is never deleted or replaced.** H1's `DeleteShownFolder`/`ReplaceShownFolder` would remove the 100k fixture root on an early step and end the run. H3 applies them only to descendants. H1's test platform still covers a deleted root. — measured on seed 101 + harness safety.
- I3's "the repo itself" run uses a copy (rsync/git archive to a temp dir), not the working tree: `--dir-is-scratch` authorizes mutation and would touch the user's uncommitted files. The 100k fixture is regenerable and is fuzzed in place. — user's safety + review.
- `--dir-is-scratch` is required before any given folder is used; even then, file operations stay jailed to it, and config/state/Trash live in a separate temp folder. — plan + AGENTS no user Trash.
- Fuzz mode installs `RecordingEffects` and `FolderTrash`, and sets `Operations::jail` + `Ui::home` to the tree. `ui::init_at` takes explicit config/state/home; `FolderTrash` is now built with the `fuzz` feature too. — plan + H1 Q12.
- Spans are at `debug` in `listing apply`, `rebuild rows`, `preview load`, `search batch`, `theme reload`, `icon fetch`; fuzz logging defaults to `info,crow=debug` (the binary crate is `crow`, not `crow_app`). `RUST_LOG` still overrides. — plan.
- `docs/testing.md` documents the runner, budget, report and `xctrace` replay; CI adds `cargo clippy -p crow-app --features fuzz --all-targets -- -D warnings`. — plan Finish.
- **Freezes are recorded, not fatal; hangs quit.** `>250 ms` sets a sticky failed outcome and prints `FREEZE`, but the 10-minute run continues; `>30 s` prints `HANG` and exits 1 so a truly wedged step cannot swallow the run. A 100k `Resize` took 5.7 s and still finished, so the threshold is above the freeze. — measured (5 s was too low; a 58.7 s palette hang still trips it).
- **Failures exit 1 explicitly.** GPUI's macOS `quit()` calls `NSApplication terminate:` and exits 0 without returning to `main`; `crow --fuzz` calls `std::process::exit(1)` after flushing for freezes, violations and hangs. Success still uses the normal quit (0). — measured (the first 10-minute attempt printed FAIL but exited 0).
- **H3 drops HideApp/HideOthers/Minimize/Quit keys.** Driving a hidden window is not a user path and stalled a 100k run for 22 s (findings #11); H1's test platform has no hide implementation. — measured + harness.
- **Invariant cost is bounded.** Checks run every 25 steps, not every step; a `Settle`'s deep snapshot is outside the heartbeat attribution; selections ≤16 use `has_row_path`, larger ones build `row_paths` + a set (a 100k select-all must not be 10^10 comparisons). — measured + harness.
- `--minutes` default 10; `--until` overrides it. `--fuzz` needs a real GUI session; CI only runs the fuzz-feature clippy. — plan + Q2.

## Hand checks
- [ ] `cargo run --release -p crow-app --features fuzz -- --fuzz --minutes 1` opens a real window on a temp edge tree, never launches an app, never plays a sound, leaves the user's Trash and pasteboard alone, and exits 0 when no heartbeat is over 250 ms.
- [ ] `--fuzz <dir>` without `--dir-is-scratch` refuses to run (never a user folder). Verified from the command line: it prints the refusal and exits 1.
- [ ] `--fuzz --seed N --until M` stops before step M with the window still open, and `xcrun xctrace record --template 'Time Profiler' --attach crow` can attach. Verified that it pauses and prints the tree + attach command; the actual `xctrace` attach is a hand check.
- [ ] Run a clean 1-minute `--fuzz --tree none` and confirm the user's `~/.config/crow`, `~/.local/state/crow`, `~/Library/Logs/Crow` history and Trash are unchanged except the expected log lines.

## Found along the way
- Seed 101 on the 100k fixture, step 38: invariant `focus — focus is outside the why surface`. It did not recur after the 16 ms check delay on a fresh fixture; likely the deferred-focus timing H1's `run_until_parked` avoids. No findings row yet.
- Seed 303 on the 100k fixture, step 262: `Type("0م🙂")` held the main thread 406.9 ms after toggling hidden files on a 100k folder; spans: none (no app span overlapped). Recorded for I3/H4 triage.
- Seed 202 on the 100k fixture, step 106: `Keys("left")` froze 22 s after `cmd-h` (HideApp) + `enter`; findings #11. The runner now drops HideApp/HideOthers.
- Seed 303 on the 100k fixture: 59 freezes in 180 s (worst 5.7 s `Resize`), then a second attempt ran 542 s and was SIGKILLed (exit 137) with 1107 slow / 101 freezes (findings #12/#14). The kill looks like memory growth from repeated operations, not a harness loop; seed 404 completed the 10-minute run (#18).
- Seed 304 on the small edge tree + churn: 650 s (meets the 10-minute run), 146 slow / 4 freezes / 6 listing violations, then 19 s palette refreshes and a 58.7 s `Type` hang (#15/#16). The listing mismatches are deep checks under churn; H4 owns triage.
- Seed 305 on a `git archive HEAD` copy of the repo: 600.44 s (meets 10 minutes), 3 slow steps (worst 91.3 ms), one `focus — focus is outside the why surface` at step 4550; exit 1 (#17).
- Seed 404 on a fresh 100k fixture completed 607.6 s (10 minutes), 75 slow / 3 freezes (worst 5.2 s `cmd-v`; #18); no violation. The seed-303 SIGKILL (#14) did not repeat, so the 10-minute 100k requirement is met.
- The first release smoke (frame-callback version) reported a 973 ms `Resize` and only 3 frame callbacks; the measurement, not the app, was wrong (see Decisions). Re-run after the heartbeat change.
- `--dir-is-scratch` on the fixture deletes subfolders the fuzzer navigates into and creates/replaces files; the root stays by the rule above. The fixture is disposable (`--make-fixture`).

## Log
- 2026-11-15: read the plan, M7 index, H1/H2 progress, findings, `docs/testing.md`, the fuzz runner and the app entry points; created the branch and this file.
- 2026-11-15: asked Q1–Q3; answers 50/250, local only, keys only.
- 2026-11-15: implemented `fuzz_live`: CLI, temp H1/edge tree, isolated config/state/Trash, sidebar confined to the tree, jail + home, RecordingEffects, keys/typing dispatch via `update_window`, `--until` pause, 16 ms heartbeat, span layer, report/exit code. `ui::init_at`, `FolderTrash` under `fuzz`, `Step::kind`, `state_for`, window/pane snapshot helpers and the six debug spans added. Debug + release smoke runs work; `cargo fmt`, both clippy lines and workspace tests pass.
- 2026-11-15: fixed a double-lease panic: dispatching keys inside `WindowHandle::update` drew the window while `CrowWindow` was leased. Input now goes through `AppContext::update_window` (and state reads through `handle.update` with `state_for` taking `&CrowWindow`). — measured by the first smoke.
- 2026-11-15: 100k smoke with `on_next_frame` recorded only 3 callbacks/39 steps; switched to the heartbeat.
- 2026-11-16: I3 complete: 100k seed 404 607.6 s (75 slow / 3 freezes, #18); edge + churn seed 304 650 s (146 slow / 4 freezes / 6 listing violations, then a 58.7 s HANG, #15/#16); repo copy seed 305 600.4 s (3 slow, one picker focus violation, #17). Seed 303's 542 s SIGKILL is #14; seed 404 met the 10-minute 100k requirement.
- 2026-11-16: Final checks: `cargo fmt --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo clippy -p crow-app --features fuzz --all-targets -- -D warnings`; `cargo test --workspace` (crow-app 411 passed, 1 ignored; other crates green); `cargo test -p crow-app --features fuzz` (416 passed, 1 ignored). Bench on the 100k fixture: branch warm first paint 8 ms, p99 8.68/8.73 ms, avg 8.40 ms; `main` @ c5698a6 warm 8 ms, p99 8.69/8.78 ms, avg 8.41/8.44 ms. No regression from the debug spans.
- 2026-11-16 (critique pass): settle the window (churn paused) before step 0, as H1's runner does, so startup is not attributed to the first step. `cargo test -p crow-app --features fuzz fuzz_live` and a short (0.02-minute) temp-tree smoke pass; the smoke exited 0 with one 101 ms `cmd-,` slow step.

## Next
Ready for critique. The critic should look at the heartbeat/freeze/hang semantics, the `--until` pause, and whether the hidden-window and 100k SIGKILL findings are app or harness.
