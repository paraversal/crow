# u5-bottom-bar progress
Branch: `ui-u5-bottom-bar` · PR: none · Last updated: 2026-11-13 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 `hints` setting and usage model | done | `HintsMode`, `Settings.hints`, `DEFAULT_SETTINGS` block, `crow-core/src/usage.rs` (`Usage`, `RETIRE_AFTER`, `USAGE_CAP`, `idle_hint_visible`); 5 unit tests; `cargo test -p crow-core` 53 green. |
| I2 Key-use recorder | todo | depends on Q13 |
| I3 Three slots and counts | todo | depends on Q2, Q3, Q4, Q8 |
| I4 Hints by focus and retiring | todo | depends on Q5, Q6, Q7 |
| I5 Progress and Cancel | todo | |
| I6 Error token with Open | todo | depends on Q9, Q10, Q11 |
| I7 Narrow window drops hints first | todo | |

## Questions
All 13 answered by the user on 2026-11-13; every recommended answer was chosen.
- Q1 → scope: no hints while an error token shows.
- Q2 → timed: info messages clear after 5 s; op errors become the error token and clear when the next operation starts or on the next navigation.
- Q3 → counts table as tabled.
- Q4 → selected folders add nothing; a folders-only selection shows no size.
- Q5 → search field hints "↵ open · ⇥ results · esc done".
- Q6 → drop sidebar hints that do nothing on the cursor row; "⌘E eject" on a disk.
- Q7 → no hints while an overlay is open.
- Q8 → search mode shows the selected result's path.
- Q9 → file name plus "line N" where the TOML parser gives a span.
- Q10 → default app for `.toml`, falling back to `open -t`.
- Q11 → "Apply to All Apps" failures show as an operation error message, not a settings error.
- Q12 → bar height 30 px.
- Q13 → every use of the action counts (key, menu, palette).

## Decisions
- Version 30 px bar (Q12), message lifetime 5 s (Q2), search-result path (Q8), error Open fallback (Q10) — user, 2026-11-13.
- `crow_core::idle_hint_visible(mode, uses)` kept as the plan names it; no extra settings wrapper.

## Hand checks
- [ ] Bottom bar hints: fresh state dir (`CROW_STATE_DIR=$(mktemp -d)`): idle hints `/ filter · ⌘F search · ⌘K go to`. Press `/` and Esc three times: "filter" disappears, the others stay; relaunch: still gone. Focus the sidebar, a filter, search results: the hint set changes each time. `hints = "always"` brings them back; `"off"` hides all. Vim layer: same, and `m` shows "Mark this folder…".
- [ ] Bottom bar progress: copy a large folder to another disk: accent line on the bar's top edge, "Copying … · n %", Cancel esc; the path and count stay. Esc or Cancel stops it.
- [ ] Bottom bar error: break `views.toml` while an operation runs: progress and the red token show together, hints are gone; Open opens `views.toml`; fixing it removes the token.
- [ ] Bottom bar counts: column view: hover the count, the tooltip names the focused column's folder. Filter: "n of N shown". Select several files: "n selected · size".
- [ ] Narrow window: shrink the window: hints disappear right to left before the path is cut.
- [ ] VoiceOver: Cmd-F5: the count reads "18 items in jules"; an error reads "Error: …" once; Open and Cancel read as buttons.

## Found along the way
- Version 30 px bar (Q12), message lifetime 5 s (Q2), search-result path (Q8), error Open fallback (Q10) — user, 2026-11-13.
- `crow_core::idle_hint_visible(mode, uses)` kept as the plan names it; no extra settings wrapper.

## Log
- 2026-11-13: I1 done. `usage.json` is pretty JSON (`{ "counts": { … } }`); a corrupt file loads empty. `cargo test -p crow-core` 53 passed.
- 2026-11-13: asked Q1–Q13; all recommended answers accepted.
- 2026-11-13: read `AGENTS.md`, the index README, the plan, U1/U4 progress. Confirmed U1 (#8) and U4 (#12) are merged on `main`. Created the branch and this file. Preparing the 13 plan questions.

## Next
I2: `crates/crow-app/src/usage.rs` global, `usage::record`/`uses`/`record_key`/`count_key`, handler calls; then the keystroke/palette counting tests.
