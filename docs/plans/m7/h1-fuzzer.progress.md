# H1 action fuzzer progress
Branch: `m7-h1-fuzzer` · PR: none · Last updated: 2026-11-15 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 Side-effect seam (`effects`) | todo | Call sites found: `pane.rs:937` (`open`), `pane.rs:1316` + `search.rs:873` (Quick Look), `pane.rs:1513` + `sidebar.rs:717` (native menu), `pane.rs:1561/1563` + `sidebar.rs:751` (reveal), `window.rs:1764` (`open -t` views.toml), `window.rs:1973` (pasteboard write), `operations.rs:91/94` (pasteboard read), `bottom_bar.rs:491` (config file open, currently `#[cfg(test)]` recorder), `ui.rs:632/642` (config open, config folder), `menus.rs:93` (About), `ui.rs:396` (Dock icon), `sidebar.rs:784` (eject), `search.rs:348` (mdfind), `feedback.rs` (sounds), `folder.rs:1149` (Quick Look follow), `pane.rs:1862` + `search.rs:1773` (native drag). |
| I2 Steps and generator | todo | Blocked on Q1/Q2/Q3. |
| I3 Invariants | todo | Blocked on Q2/Q4. |
| I4 Runner, shrinking, replay | todo | Blocked on Q1/Q3. |
| I5 First run | todo | Needs I1–I4. |

## Questions
- Q1 (asked 2026-11-15): shrinker hand-rolled or `proptest`? → pending.
- Q2 (asked 2026-11-15): `#[cfg(test)]` only or a `fuzz` cargo feature? → pending.
- Q3 (asked 2026-11-15): multiple windows and tabs in the fuzzer? → pending.
- Q4 (asked 2026-11-15): should invariant 6 cover search results? → pending.
- Q5 (asked 2026-11-15): move `window.rs`'s test module out while adding `fuzz.rs`? → pending.
- Q6 (asked 2026-11-15, index): is the M7 order right, or should H6 come earlier? → pending.
- Q7 (asked 2026-11-15, index): nightly fuzz workflow in CI? → pending.

## Decisions
- Branch from `main` @ `50dd5a5` (M6.5 landed), not the plan's `56db832`; the M7 index says every piece branches from a `main` that has M6.5. — plan/index convention.
- Tests never run on the AppKit main thread (`crow-macos/src/lib.rs:37`), so most `crow_macos` effects are already no-ops in tests; `std::process::Command` (`pane.rs:937`, `window.rs:1764`) and `mdfind` (`search.rs:348`) are the ones that really escape. The seam covers all of them anyway.

## Hand checks
- [ ] Effects, in the bundle: Return on a file opens it; Space Quick Looks; Show in Finder reveals; Copy Path pastes; the context menu opens and its items work; sounds play (if on). Same as before H1.
- [ ] Fuzzer: `CROW_FUZZ_MINUTES=2 cargo test -p crow-app fuzz_long -- --ignored --nocapture` launches no app, plays no sound, leaves the user's Trash and pasteboard alone.

## Found along the way
- `bottom_bar::open_config` already has a `#[cfg(test)]` recorder (`OPENED`); I1 replaces it with the global seam.

## Log
- 2026-11-15: read the plan, M7 index, testing docs, repo skill; surveyed every `Command::new`/`crow_macos::` call site; created the branch and this file.

## Next
Ask Q1–Q7, then start I1 (the seam does not depend on any answer).
