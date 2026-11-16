# M7 H7 — Accessibility hardening progress
Branch: `m7-h7-a11y` · PR: none · Job: m7-h7-a11y-bf16 · Last updated: 2026-11-16 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 GPUI patch: disabled state | todo | |
| I2 Fuzzer a11y invariant | todo | |
| I2b Fuzz the Settings window | todo | |
| I3 VoiceOver pass | todo | |
| Finish | todo | |

## Questions
- Q1 (asked 2026-11-16): patch GPUI for `aria_disabled`, or keep the ", unavailable" label suffix? → waiting. Checked before asking: `accesskit_macos-0.26.3/src/node.rs:634` maps `is_disabled` to `AXEnabled` false, so the macOS adapter does expose `Disabled`.
- Q2 (asked 2026-11-16): how does the invariant enumerate elements with `on_click`/focus handles? GPUI's test platform only records `debug_selector` bounds and the a11y tree (which drops role-less nodes); neither lists every clickable/focusable element. → waiting.
- Q3 (asked 2026-11-16): I3 method — AX-tree dump of the bundle vs listening to VoiceOver? → waiting.

## Decisions
- Branch from `main` @ 9282594, as the plan names it; H1 is merged (`cf85599`). H5 is still an open PR and H6 is unmerged, so the branch has neither; the path bar is out of I3's scope (plan: "H6 if merged").

## Hand checks
- [ ] I1 VoiceOver: a disabled toolbar Back/Forward button reads "dimmed".
- [ ] I3 VoiceOver listening pass (areas listed in the plan).

## Found along the way
- (none yet)

## Log
- 2026-11-16 Read the plan, `AGENTS.md`, `docs/plans/m7/README.md`, the two "unavailable" sites (`window.rs:560`, `search.rs:1118`), GPUI's a11y builder and `accesskit_macos`'s disabled mapping; created the branch and this file.

## Next
Ask Q1–Q3, then start I1.
