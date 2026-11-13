# u5-bottom-bar progress
Branch: `ui-u5-bottom-bar` · PR: none · Last updated: 2026-11-13 · State: started

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 `hints` setting and usage model | todo | |
| I2 Key-use recorder | todo | depends on Q13 |
| I3 Three slots and counts | todo | depends on Q2, Q3, Q4, Q8 |
| I4 Hints by focus and retiring | todo | depends on Q5, Q6, Q7 |
| I5 Progress and Cancel | todo | |
| I6 Error token with Open | todo | depends on Q9, Q10, Q11 |
| I7 Narrow window drops hints first | todo | |

## Questions
- Q1 (asked 2026-11-13): board E shows "/ filter" next to the error; the plan scope says hints drop while an error shows. Recommended: follow the scope.
- Q2 (asked 2026-11-13): op message lifetime. Recommended: info messages clear after 5 s; op error messages become the error token and clear when the next operation starts or on the next navigation.
- Q3 (asked 2026-11-13): counts grammar as tabled (single selected file's size; "1 of 44 selected" in search). Recommended: yes as tabled.
- Q4 (asked 2026-11-13): folder sizes in a selection. Recommended: files only, no marker; folders-only selection shows no size.
- Q5 (asked 2026-11-13): search-field hints. Recommended: "↵ open · ⇥ results · esc done".
- Q6 (asked 2026-11-13): sidebar hints by row. Recommended: drop hints that do nothing on the cursor row; "⌘E eject" on a disk.
- Q7 (asked 2026-11-13): overlay open. Recommended: no hints.
- Q8 (asked 2026-11-13): path in search mode. Recommended: show the selected result's path.
- Q9 (asked 2026-11-13): error text. Recommended: file name plus "line N" where the TOML parser gives a span.
- Q10 (asked 2026-11-13): Open button app. Recommended: default app for `.toml`, falling back to `open -t`.
- Q11 (asked 2026-11-13): "Apply to All Apps" failures stored as a settings error. Recommended: show them as an operation error message instead.
- Q12 (asked 2026-11-13): bar height. Recommended: 30 px.
- Q13 (asked 2026-11-13): should menu/palette uses count for hint retirement? Recommended: yes, count every use of the action.

## Decisions
- (none yet)

## Hand checks
- [ ] Bottom bar hints: fresh state dir (`CROW_STATE_DIR=$(mktemp -d)`): idle hints `/ filter · ⌘F search · ⌘K go to`. Press `/` and Esc three times: "filter" disappears, the others stay; relaunch: still gone. Focus the sidebar, a filter, search results: the hint set changes each time. `hints = "always"` brings them back; `"off"` hides all. Vim layer: same, and `m` shows "Mark this folder…".
- [ ] Bottom bar progress: copy a large folder to another disk: accent line on the bar's top edge, "Copying … · n %", Cancel esc; the path and count stay. Esc or Cancel stops it.
- [ ] Bottom bar error: break `views.toml` while an operation runs: progress and the red token show together, hints are gone; Open opens `views.toml`; fixing it removes the token.
- [ ] Bottom bar counts: column view: hover the count, the tooltip names the focused column's folder. Filter: "n of N shown". Select several files: "n selected · size".
- [ ] Narrow window: shrink the window: hints disappear right to left before the path is cut.
- [ ] VoiceOver: Cmd-F5: the count reads "18 items in jules"; an error reads "Error: …" once; Open and Cancel read as buttons.

## Found along the way
- (none yet)

## Log
- 2026-11-13: read `AGENTS.md`, the index README, the plan, U1/U4 progress. Confirmed U1 (#8) and U4 (#12) are merged on `main`. Created the branch and this file. Preparing the 13 plan questions.

## Next
Ask Q1–Q13 in one batch, then start I1 (crow-core `HintsMode` + `usage.rs`) while the answers are applied.
