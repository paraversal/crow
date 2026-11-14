# u10-feedback progress
Branch: `ui-u10-feedback` · PR: none · Last updated: 2026-11-14 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 Bring the lab and the sound module over | done (2e6c76d + next) | lab cherry-picked; pure parts in `crow-core::motion` with 6 tests; `feedback::sound` maps `SoundId` → `NSSound`; README documents `--motion-lab`. Lab smoke-runs (window opens, no panic); look/sound hand check pending. |
| I2 Settings and the effective mode | done | `motion`/`sound` settings + docs, `Ui.feedback`, poll + reload, `set_reduce_motion` in Off, theme fade by mode; parse and fade tests. R3 open (see Decisions). |
| I3 Trash | done | R1 departure overlay: `Folder` keeps `RowEffect`s (max 20 visible rows), rows render the swallow, a ghost finishes it if the listing drops the row early, and rows below FLIP up in Full. Sound on success, Refuse on error. Tests: 1 item in off/low/full, 3, 500, columns. Bench: Full p99 9.60 ms, Off 9.07, Low 9.73 (warm 8 ms each). |
| I4 Move into a folder | in progress | Q4 = pulse only |
| I5 Arrive | todo | |
| I6 Refuse and errors | todo | |
| I7 Small entrances | todo | needs Q3 |
| I8 Cursor glide | todo | needs Q2 |
| I9 Docs, tests, bench | todo | |

## Questions
- Q1 (asked 2026-11-14): default sound set → **`crow`** (user).
- Q2 (asked 2026-11-14): cursor glide in Full → **yes, Full only** (user).
- Q3 (asked 2026-11-14): sheets and palette fade/scale → **yes**; add to the lab first, user tries it (user).
- Q4 (asked 2026-11-14): "+1" count on a move's target → **pulse only, no count** (user).
- Q5 (asked 2026-11-14): does `off` silence sounds → **no**, separate settings (user).

## Decisions
- Branch from `main` at `92d671e` (U9 merged at `0718a7c`; plan commit `92d671e` on `main`) — plan says so — plan.
- Q1 default `sound = "crow"` — user.
- Q2 cursor glide in Full — user.
- Q3 sheets/palette fade+scale, lab first — user.
- Q4 target pulse only, no count — user.
- Q5 `motion = "off"` does not silence sound — user.
- Spike (I2): GPUI animations read the test clock. A probe view with `with_animation` (200 ms) recorded `t = 0.5` after `advance_clock(100 ms)` + `simulate_next_frame`. So app tests can read a mid-animation look by recording in the animator; `look()` is tested purely. Spike code reverted.
- R1 (I3): implemented the recommended departure overlay. `uniform_list` heights never change; a row plays the swallow in place, a ghost at its last bounds finishes an early removal, and rows below get `top = ROW × (1 − ease_out(t))` from the moment the listing drops the row (not before the swallow ends). At most 20 visible rows animate.
- R2 (I3): the common path is `show_results` → `reload`, so the row leaves within a few ms and the ghost covers the remaining swallow; the watcher (100 ms poll + stat) is the fallback. Effects live `phases + 250 ms` so a late removal's close is not cut. No spread measured on a real trash; hand check.
- R3 (I2): NSSound's volume is relative to the output device volume, not the macOS alert volume (Apple/community docs: sound effects must be played on `kAudioHardwarePropertyDefaultSystemOutputDevice` to follow the Sound Effects output; `NSSound setVolume:` is subordinate to system volume). Not measured on this machine. Keeping NSSound for now (the lab the user approved used it); hand check with the alert slider low. Root cause would be `AudioServicesPlaySystemSoundWithCompletion` + a trimmed copy of Finder's trash sound in the cache dir (cannot fade the tail), which needs a user decision.
- Sound preload: the current set at startup (startup exception) and the new set when `sound` changes, on the main thread (NSSound caches per thread; a few small system files).

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
- 2026-11-14: I1–I2 committed. I3: `RowEffect`/ghost/FLIP in `folder.rs` and `pane.rs`; trash sound and error beep through `operations::finish`; 10 trash tests pass; bench in all three modes within budget.

## Next
I4: move into a folder (drop on a row, sidebar drop, ⌘V of a cut) with MoveOut/Receive looks and the Purr after the flight.
