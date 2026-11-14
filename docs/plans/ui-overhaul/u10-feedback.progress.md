# u10-feedback progress
Branch: `ui-u10-feedback` · PR: none · Last updated: 2026-11-14 · State: In critique

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 Bring the lab and the sound module over | done (2e6c76d + next) | lab cherry-picked; pure parts in `crow-core::motion` with 6 tests; `feedback::sound` maps `SoundId` → `NSSound`; README documents `--motion-lab`. Lab smoke-runs (window opens, no panic); look/sound hand check pending. |
| I2 Settings and the effective mode | done | `motion`/`sound` settings + docs, `Ui.feedback`, poll + reload, `set_reduce_motion` in Off, theme fade by mode; parse and fade tests. R3 open (see Decisions). |
| I3 Trash | done | R1 departure overlay: `Folder` keeps `RowEffect`s (max 20 visible rows), rows render the swallow, a ghost finishes it if the listing drops the row early, and rows below FLIP up in Full. Sound on success, Refuse on error. Tests: 1 item in off/low/full, 3, 500, columns. Bench: Full p99 9.60 ms, Off 9.07, Low 9.73 (warm 8 ms each). |
| I4 Move into a folder | done | `Move` starts MoveOut on visible sources, Receive on the target folder row (list and columns) and a `Sidebar::pulse` on the target favorite. No flight or count (Q4). Purr on success through `operations::finish`. Tests: drop on a row in Full, source/target effects, favorite pulse. |
| I5 Arrive | done | Results selected by `show_results` play Arrive when they appear; Full's open phase slides the rows below down from their old slot (`-ROW × (1 − ease_out(t))`). New folder/file, duplicate, paste/copy, undo/redo all go through it. Rename starts immediately. Sounds: Pop, Bottle, Glass (>1 s). Tests: new folder Arrive + typing, undo Arrive. |
| I6 Refuse and errors | done | Every bell site goes through `feedback::refuse` (or `TextInput::refuse`): one beep, then Full shakes the field / Low tints its ring red / Off just beeps. Operation errors Refuse the rows that remain (trash, move, copy). The bar's message/error transient fades in on change. |
| I7 Small entrances | done (scale open Q6) | Tip rises 8 px + fades (Full) / fades (Low); bar message and error fade; a finished progress bar fades over 320 ms; sheets and the palette fade in (140/120 ms) and the lab has a mock sheet (`p`). GPUI has no element scale, so 0.98→1 is not implemented; Q6. |
| I8 Cursor glide | done | Full only: one absolutely positioned fill per list/column under the rows, spring k = 2500 (the lab's 900 lagged 1.67 rows at a 30 ms repeat; 2500 passes the one-row test). Rows draw no active fill while gliding. Tests: placement, lag; bench Full p99 9.73–9.83 ms (one busy-machine run 34 ms, three clean reruns 9.7–9.8). |
| I9 Docs, tests, bench | done | README: `motion`/`sound` in the settings table, a paragraph on the modes and the lab. `docs/testing.md`: U10 hand checks and the lab; bench numbers in the performance table. Final bench: Full 9.73–9.83 ms, Off 9.83, Low 9.97, warm 8–10 ms. |

## Questions
- Q1 (asked 2026-11-14): default sound set → **`crow`** (user).
- Q2 (asked 2026-11-14): cursor glide in Full → **yes, Full only** (user).
- Q3 (asked 2026-11-14): sheets and palette fade/scale → **yes**; add to the lab first, user tries it (user).
- Q4 (asked 2026-11-14): "+1" count on a move's target → **pulse only, no count** (user).
- Q5 (asked 2026-11-14): does `off` silence sounds → **no**, separate settings (user).
- Q6 (asked 2026-11-14): sheets/palette 0.98→1 scale → **fade only** (user; GPUI has no element transform).
- Q7 (asked 2026-11-14): R3 NSSound volume vs alert volume → **keep NSSound** (user); hand check at low alert volume.
- Q8 (asked 2026-11-14): glide stiffness → **k = 2500** (user; 900 lagged 1.67 rows).

## Decisions
- Branch from `main` at `92d671e` (U9 merged at `0718a7c`; plan commit `92d671e` on `main`) — plan says so — plan.
- Q1 default `sound = "crow"` — user.
- Q2 cursor glide in Full — user.
- Q3 sheets/palette fade+scale, lab first — user.
- Q4 target pulse only, no count — user.
- Q5 `motion = "off"` does not silence sound — user.
- Q6 sheets/palette: fade only, no 0.98→1 scale — GPUI 0.2.2 has no element transform; user chose the fade.
- Q7 keep NSSound for sounds — the user accepted the output-volume behavior (hand check with the alert slider low); AudioServices + a cache copy would be the root-cause change.
- Q8 cursor glide k = 2500 — measured: 900 lagged 1.67 rows at a 30 ms repeat, 2500 passes the one-row test; user chose 2500.
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
- 2026-11-14: I4: a `Move` starts MoveOut on the visible sources and Receive on the target row; the sidebar pulses the target favorite. No flight or count (Q4). Purr on success. 3 new tests.
- 2026-11-14: I5: Arrive starts when `select_when_present` results appear; `row_shift` also handles the open phase (rows below start one row up and slide down). Rename opens at once. Pop/Bottle/Glass wired in `operations`. 2 new tests.
- 2026-11-14: I6: `feedback::refuse`/`TextInput::refuse` replace every `play_system_bell`; the field shakes (Full) or tints red (Low); operation errors refuse the remaining rows; the bar transient fades on a new message or error.
- 2026-11-14: I7: tip rise/fade, bar message/error fade, progress-done fade (320 ms), sheet/palette fade (140/120 ms) and a mock sheet in the lab (`p`). GPUI has no element scale: the 0.98→1 part is Q6.
- 2026-11-14: I8: cursor glide in Full, one fill per list/column under the rows, spring k = 2500 (900 lagged 1.67 rows; 2500 passes the one-row test). Bench Full p99 9.73–9.83 ms.
- 2026-11-14: I9: README and `docs/testing.md` updated; final bench in all three modes. All items done; branch ready for critique.

## Next
Critique the branch (`critique` skill), then open the PR. Hand checks in `docs/testing.md` (Motion and sound) are for the user in the bundled app.

## Critique
Critic session: 2026-11-14 · Base: main @ 5818fb3 (branch from 92d671e) · Head reviewed: 135797d

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | After trashing or moving the selected item (list and columns) the cursor jumped to the first row; Finder leaves nothing selected. Cause: `auto_select` reselected the first row on every reload that emptied the selection, not only on first load. Pre-existing on `main`, reported by the user; the U10 trash tests pinned the jump ("where it does today"). | `crates/crow-app/src/folder.rs` `replace_entries` | UX | fix | fixed (2a283c4): fires once on first full load; tests now expect no selection, ↓ starts at the top; new column move test. All fail without the fix. |
| C2 | In Full with the list focused, every selected row dropped its own fill and only the glide fill under the cursor showed: ⌘A highlighted one row. | `crates/crow-app/src/pane.rs` `render_table_rows`, `render_column_rows` | code / UI | fix | fixed (c0ed0a3): only the cursor row hands its fill to the glide; unit test. |
| C3 | The default `settings.toml` says "a move flies"; there is no flight (Q4, plan I4). | `crates/crow-core/src/settings.rs:31` | rules (docs) | fix | fixed (14ad807) |
| C4 | No expand/collapse motion for inline folders (list view). Not in the plan; user asked. | `folder.rs` `sync_children`, `pane.rs` `chevron_glyph` | UX | ask | Q9 |
| C5 | The sidebar's current-location fill jumps rather than glides. Not in the plan; user asked. | `sidebar.rs` render | UX | ask | Q10 |
| C6 | `refresh_feedback` reads Reduce Motion and the UI-sounds default on every 100 ms poll. In-memory defaults reads; cheap, no change. | `crates/crow-app/src/ui.rs` `poll` | code sense | note | left |

Checks re-run: fmt ✓ · clippy ✓ · tests ✓ (437 after fixes) · bench inconclusive: the display alternated 120/30 Hz during the runs. Under the same 30 Hz state, branch p99 35.7–35.9 ms vs main 35.2–35.5; one clean branch run p99 9.75 ms, warm paint 7 ms (agent: 9.73–9.83).
Looked at in the app: not driven (no screen control in this session). Visual and sound checks stay with the user's Hand checks, plus: ⌘A in Full shows every row selected (C2); trash/move the selection in columns leaves no highlight, ↓ selects the first item (C1).
Verdict: pending Q9/Q10.
