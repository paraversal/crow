# u10-feedback progress
Branch: `ui-u10-feedback` · PR: none · Last updated: 2026-11-14 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 Bring the lab and the sound module over | todo | |
| I2 Settings and the effective mode | todo | needs Q1 |
| I3 Trash | todo | needs R1 spike |
| I4 Move into a folder | todo | needs Q4 |
| I5 Arrive | todo | |
| I6 Refuse and errors | todo | |
| I7 Small entrances | todo | needs Q3 |
| I8 Cursor glide | todo | needs Q2 |
| I9 Docs, tests, bench | todo | |

## Questions
- Q1 (asked 2026-11-14): default sound set → pending.
- Q2 (asked 2026-11-14): cursor glide in Full → pending.
- Q3 (asked 2026-11-14): sheets and palette fade/scale → pending.
- Q4 (asked 2026-11-14): "+1" count on a move's target → pending.
- Q5 (asked 2026-11-14): does `off` silence sounds → pending.

## Decisions
- Branch from `main` at `92d671e` (U9 merged at `0718a7c`; plan commit `92d671e` on `main`) — plan says so — plan.

## Hand checks
- [ ] Trash 1 / 3 / 500 items in list and column view, each mode, sound `finder` and `crow` → lab look, one sound, cursor unchanged.
- [ ] Drag onto a folder row and a sidebar favorite → target pulse; Purr in `crow`.
- [ ] ⇧⌘N → new folder arrives, rename field takes typing immediately.
- [ ] ⌘Z after a trash → file arrives; Bottle in `crow`.
- [ ] Trash a locked file (`chflags uchg`) → Refuse shake (Full) / tint (Low) / beep only (Off).
- [ ] Copy a 2 GB file → progress, bar fades at the end, Glass in `crow`.
- [ ] Reduce Motion with `motion = "auto"` → Full ↔ Low live.
- [ ] "Play user interface sound effects" off → only the alert; alert volume low → Crow's sounds as quiet as Finder's (R3).
- [ ] `crow --motion-lab` still runs and matches the app.

## Found along the way
- (none yet)

## Log
- 2026-11-14: branch created from `main`; progress file started. Read the plan, `AGENTS.md`, the overhaul index, the lab on `motion-trial` (`7d6f32a`), and the current operation/row/folder code on `main`.

## Next
Ask Q1–Q5, then start I1 (cherry-pick the lab, move the pure parts to `crow-core::motion`).
