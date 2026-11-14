# U10 Feedback: motion and sound

Audience: an implementing agent. Follow `.claude/skills/execute-plan/SKILL.md`; read `AGENTS.md` and `docs/plans/ui-overhaul/README.md` first.
Needs: U9 **merged to `main`** (its tip gets an entrance here), and the lab commit on branch `motion-trial` (see I1). ASK before stacking on an unmerged branch.
Branch: `ui-u10-feedback` from `main`. Progress file: `docs/plans/ui-overhaul/u10-feedback.progress.md`.
PR title: "UI U10: Motion and sound feedback". Don't merge.
Design reference: the motion lab, `crow --motion-lab` (branch `motion-trial`, `crates/crow-app/src/motion_lab.rs`). The user tried it on 2026-11-14 and approved its motions and sounds with two changes, both already in the lab (see Decided). The lab is the reference for every look and duration below; where this plan and the lab disagree, the plan wins; where both are silent, ASK.

## Why
Spec principle 9 ("Fun: polish and motion where they help understanding", `docs/Crow — Product & Technical Spec.md:21`). Today a trashed file just vanishes when the folder watcher next reports, a moved file vanishes from one place and appears in another with nothing linking the two, a refused operation only rings the bell or prints a red line, and nothing in Crow makes a sound except the bell. The README of the overhaul parked this as "a motion pass … after U10" (user request, 2026-11-13); on 2026-11-14 the user moved it before the finish pass, so the finish pass (now U11) checks and documents the window once, with motion in it.

## Decided
- User, 2026-11-14: "three animation modes: off, low, full". Sound is a separate setting from motion.
- User, 2026-11-14, after trying the lab: liked the motions and sounds; two changes: Finder's "move to trash" sound has "way too long" a paper tail → play only its start (lab: 450 ms, then a 150 ms fade, `TRASH_CLIP` in `motion_lab.rs`); the move sound "should not be the error sound" → Purr instead of Tink.
- User, 2026-11-14: a motion test suite (the lab) in its own worktree first; this plan turns it into the app.
- Index ground rules: theme roles only; every interactive element has a role and an aria label; every new key behavior gets a keystroke test; config options flat, typed and documented so a future settings page can list them (index Q7).
- AGENTS.md: no disk I/O on the main thread (sounds load at startup, the allowed exception, or in the background); performance budgets (100k folder: warm first paint < 50 ms, scroll p99 < 1.5 × frame time) hold in every mode.

## Modes (what each one means)
| Mode | Rule | Examples |
| --- | --- | --- |
| `off` | Nothing animates. State changes in one frame. Progress still advances (it is information), at most 4 updates/s. | Trash: the row is gone. Refuse: the error line, no shake. |
| `low` | Opacity and color only, ≤ 160 ms out / ≤ 420 ms for a fading tint. Nothing changes position or size; gaps close at once. | Trash: row fades out. Move: row fades, target folder flashes. Refuse: red tint fades. |
| `full` | Movement and size, 120–360 ms per phase, ease-out to land, ease-in to leave. | Trash: icon becomes a trash, name slides into it, gap closes. Move: the row flies to the folder. Refuse: damped shake. |
Setting `motion = "auto"` (default) means `low` while macOS Reduce Motion is on (`crow_macos::reduce_motion`, `crates/crow-macos/src/volumes.rs:153`), else `full`, re-checked on the 100 ms poll (`ui.rs` `init`), so flipping the system switch applies live.

## Sounds
| Event | `finder` set | `crow` set | Volume |
| --- | --- | --- | --- |
| Trash done | Finder `finder/move to trash.aif`, clipped (450 ms + 150 ms fade) | same | 0.6 |
| Empty Trash (if Crow gains it; today it has none) | Finder `finder/empty trash.aif` | same | 0.6 |
| Refused / error | user's alert sound (`NSBeep`) | same | system |
| Move done | none | `Purr` | 0.35 |
| New folder / new file | none | `Pop` | 0.3 |
| Undo / redo done | none | `Bottle` | 0.3 |
| Copy done (only after a visible progress, i.e. > 1 s) | none | `Glass` | 0.35 |
Finder sound paths are under `/System/Library/Components/CoreAudio.component/Contents/SharedSupport/SystemSounds/`; named sounds are `/System/Library/Sounds/*.aiff` through `NSSound soundNamed:`. Crow ships no audio files (Apple's sounds are not ours to redistribute; they are read from the system at run time). Setting `sound = "finder" | "crow" | "off"`, default per Q1. When macOS "Play user interface sound effects" is off (`crow_macos::ui_sounds_enabled`), Crow plays nothing but the alert beep (which macOS governs itself).

## Durations (ms; the lab's `ms` table)
| Effect | Full | Low |
| --- | --- | --- |
| Trash: swallow, then close | 180 + 140 | fade out 140 |
| Move: fly, then close; target pulse after the fly | 260 + 140; pulse 360 | fade out 140; target tint 420 |
| Arrive (new folder, undo, paste, copy result): open, slide in, tint fade | 120 + 180 + 420 | fade in 160 within a 420 tint |
| Refuse: shake 3 periods, 5 px, damped, red tint | 320 | red tint 420 |
| Tip entrance (U9): rise 8 px + fade | 220 | fade 160 |
| Bottom-bar message change | fade 140 | fade 140 |
| Progress done: bar fades | 320 | 320 |
| Cursor glide (Q2): critically damped spring, k = 900 | ~150 to settle | none |
| Sheet / palette open: fade + 0.98→1 scale (Q3) | 140 | fade 120 |
| Theme change fade (exists, `ui.rs:18-19`) | 500 | 150 |
Easing: ease-out cubic to land, ease-in cubic to leave, ease-in-out cubic for flights (`motion_lab.rs` `ease_*`).

## Current state
Read on `main` at `9e109cb`. U9 changes window.rs and bottom_bar.rs; re-read at I1 and record shifted line numbers in the progress file.

**Operations.** Every file operation is a `crow_fs::ops::Request` (`crates/crow-fs/src/ops.rs:31-60`). The pane emits `PaneEvent::Run(request)` for new folder/file (`crates/crow-app/src/pane.rs:1000-1004`), duplicate (`:1010`), trash (`move_to_trash`, `:1014-1019`), delete (`:1024`) and drops (`drop_into`, `:1058-1062`); the sidebar emits drops at `sidebar.rs:663`; search trashes at `search.rs:889`; paste builds Move/Copy at `window.rs:1046`. `CrowWindow::run_request` (`window.rs:1053-1082`) plans in the background, asks on conflicts, then `start` (`:1084-1091`) calls `operations::execute` (`operations.rs:138-185`), whose `on_done` calls `Pane::show_results` (`pane.rs:489-501`): select the results in the focused folder when present (`folder.rs:382`), reload the others (`folder.rs:388`). `operations::finish` (`operations.rs:187-216`) logs, sets the message, refreshes volumes. Undo/redo: `operations::undo`/`redo` (`:218-279`) → `show_results` via `window.rs:1094-1106`. Errors: `operations::message(text, true, cx)` (`:93`); bells: `window.play_system_bell()` (`window.rs:1481`, `:1924`, `:2093`, `:2211`; `text_input.rs:312-511`).

**How rows leave and arrive.** A row disappears only when the folder's listing changes: the watcher feeds `apply_folder_changes` (`folder.rs:671-707`, polled every 100 ms, `:128-134`), which stats off the main thread and calls `replace_entries` (`:480`); `show_results` reloads. So the row is gone some 0–200 ms after the operation finishes, at a time Crow does not choose. Rows are drawn by GPUI `uniform_list` (list `pane.rs:1960`, columns `:1767`), every row `ROW_HEIGHT = 24` (`:49`); `render_table_rows` (`:1284`) and `render_column_rows` (`:1584`) build them. A `uniform_list` row cannot change height: the lab's "collapse the row's height" cannot be done in place (see Risks R1).

**Bottom bar.** `bottom_bar.rs`: progress at `:723-756` (2 px accent bar), message slot from `Operations.message`. **Sheets.** `chrome::sheet`/`sheet_panel` (`chrome.rs:268-300`). **Theme fade.** `ui.rs:18-19` (500 / 150 ms), `set_theme` (`:340-361`) picks by `crow_macos::reduce_motion()`, `run_fade` (`:363-386`) steps every 16 ms. **Settings.** `crow_core::Settings` (`crates/crow-core/src/settings.rs:96-122`), documented default file at the top of the same file (`hints` at `:21-25`), `HintsMode` (`:71-80`) is the pattern for a string enum.

**GPUI.** `AnimationExt::with_animation(s)` and `with_spring` (`vendor/zed/crates/gpui/src/elements/animation.rs:82-160`), `SpringConfig` (`spring.rs:13`). Animations restart when their element id changes and keep the end state when done. GPUI's own `App::set_reduce_motion` (`app.rs:1418`, default false, never set by the platform) freezes *every* `with_animation`, so Crow sets it true only in `off` (Low still needs fades). `executor.advance_clock` (`executor.rs:215`) drives time in tests; verify in I2 that animation elements read that clock (`animation.rs` uses `scheduler::Instant`).

**The lab** (`motion_lab.rs` on `motion-trial`, ~1100 lines): modes, sound sets, slow motion ×5, all events above on a fake folder, `crow_macos::sound` (`play`, `preload`, `set_volume`, `stop`, `beep`, `ui_sounds_enabled`, NSSound cached per thread), a `trash-2.svg` glyph (`Glyph::Trash`). Its `Look` struct and `look(mode, fx, phase, t)` are the visual spec in code.

## Target
Every event above, in the real list and column views, looks as in the lab in each mode, with the sounds of the chosen set; the lab stays as a dev tool (`crow --motion-lab`, like `--bench`) and reads the same timing table and look functions as the app, so tuning happens in one place.

## Items

### I1 Bring the lab and the sound module over (no visible change)
- Cherry-pick the lab commit from `motion-trial` (or copy `motion_lab.rs`, `crow-macos/src/sound.rs`, the `NSSound`/`NSGraphics`/`NSUserDefaults` features, `trash-2.svg`, `Glyph::Trash`, the `--motion-lab` flag). Document the flag in the `main.rs` header and README (dev tools).
- Move the pure parts to `crow-core` (new `motion.rs`): `MotionMode { Auto, Off, Low, Full }` + `resolve(reduce_motion) -> Effective { Off, Low, Full }`, `SoundSet { Off, Finder, Crow }`, `Fx`, `Look`, `look(mode, fx, phase, t)`, `phases(mode, fx) -> Vec<Duration>`, the `ms` table, the easings, `cue(set, event)` (returning a sound *name/relative path*, not an `NSSound`). Unit tests: every (mode, fx) ends at the resting look (height 1, opacity 1, no tint, dx 0) except leaving effects (opacity 0 or height 0); `off` has no phases; Low never sets `dx`, `height < 1` or a size change; cue table matches the Sounds table.
- The lab uses the crow-core functions. **Done when:** lab looks and sounds as before (hand check), `cargo test -p crow-core` covers the table.

### I2 Settings and the effective mode
- `motion = "auto" | "off" | "low" | "full"` and `sound = …` (default Q1) in `Settings`, documented in the default file next to `hints`, live reload, errors in the status line (existing mechanism). A `Feedback` view of the effective mode on the `Ui` global, recomputed on reload and on the 100 ms poll (Reduce Motion). `cx.set_reduce_motion(effective == Off)`.
- Theme fade (`ui.rs:346`) follows the effective mode: Off → instant, Low → 150, Full → 500.
- Spike first: an animation test under `advance_clock` (one row, one fade) to confirm tests can read a mid-animation look; if they cannot, test `look()` purely and hand-check the rest; record it in Decisions.
- Sounds: `preload` the set's sounds at startup (the startup-load exception) and again in the background on a set change; `play` on the main thread only. **Measure first (R3):** does NSSound follow the macOS *alert* volume like Finder's own sounds, or the output volume? Report and ASK if it does not.
- **Done when:** settings parse tests (pattern of `hints_values_parse`), config default file updated, theme fade follows the mode.

### I3 Trash
- Start the effect when `start` runs a `Trash` plan (not at the key press: a conflict or delete sheet may come first). Rows of the plan's paths that are visible in the focused folder play the leave effect; sound on success (`operations::finish`); on error: Refuse on the rows that remain, beep, the existing error message.
- Full needs the R1 decision: recommended a **departure overlay**: when the effect starts, Folder records `Departure { path, row_index, started }` for the visible rows; the row renders its swallow phase from it while it is still listed; when the listing drops it, the rows below it render with `top = ROW × (1 − ease_out(t))` for the close phase (FLIP), so `uniform_list` heights never change. The swallow must not be cut short by an early watcher update: if the entry leaves before the swallow ends, draw the remaining swallow as a ghost row over the list at its last bounds.
- Many items: animate at most the first 20 visible rows; the rest just go. One sound per operation.
- Keystroke tests: ⌘⌫ in each mode leaves the listing correct and the cursor where it is today (effects must not delay selection or the next key). **Done when:** the three modes match the lab in list and column views; trash of 1, 3 and 500 items; bench unchanged.

### I4 Move into a folder (drop, sidebar drop, ⌘V of a cut)
- Drop on a folder row in the list/columns: Full: the dragged item shrinks and fades into the target row (no flight from the source: the drag already carried it), the target pulses with a "+1"-style count change if the column shows counts (Q4), the source row closes as in I3. Low: source fades, target tint. Sound after success (`crow` set).
- Drop on a sidebar favorite: the sidebar row pulses (same tint as the list's receive).
- Keyboard move with no visible target (⌘X, ⌘⌥V in another folder): the pasted rows Arrive (I5) in the destination; the source is no longer on screen.
- The lab's flight between two rows of one list happens only when both rows are visible in one folder (a keyboard "move to folder" Crow does not have yet; leave it out unless Q4 says otherwise).

### I5 Arrive: new folder/file, paste/copy results, duplicate, undo/redo
- `show_results` already knows the result paths; when they appear in the listing (`select_when_present`), those rows play Arrive. In Full the open phase is the FLIP inverse of I3: the rows below the new row render with `top = −ROW × (1 − ease_out(t))`, so they start in their old slot and slide down one row while the new row fades and slides in. ASK if it looks wrong in column view.
- New folder: Arrive, then the rename field opens (today's `rename_when_present`); the field must not wait for the animation (keys typed at once go into it).
- Sounds: new → `Pop`; undo/redo → `Bottle`; copy → `Glass` only after a progress bar was shown.

### I6 Refuse and errors
- Operation errors on visible rows: Refuse look on those rows + beep. Bells with no row (Go to, palette, text fields): Full shakes the field (same curve), Low tints its ring red, Off just beeps. All `play_system_bell` call sites go through one helper `feedback::refuse(target, window, cx)` so the beep stays exactly once.
- Bottom-bar error token: appears with the message fade.

### I7 Small entrances
- U9 tip: rise + fade (Full), fade (Low), instant (Off). Bottom-bar message change: fade. Progress done: the bar fades instead of vanishing.
- Sheets and palette: Q3. Tooltips: no motion (they already wait for a delay; adding a fade makes them feel slower).

### I8 Cursor glide (only if Q2 = yes)
- Full only: the active selection fill is drawn once per list as an absolutely positioned element under the rows, springing to the cursor row's y (scroll offset included); the rows themselves draw no fill. Holding ↓ must never lag behind the key repeat by more than one row: if it does, increase stiffness rather than adding special cases, then ASK.
- Bench with glide on: scroll p99 must stay within budget (`--bench target/fixture-100k`, compare with `main`).

### I9 Docs, tests, bench
- README: the two settings, what the modes do, the dev lab. `docs/testing.md`: hand checks below and the lab. Bench in all three modes (set `motion` in a temp config) and record numbers.

## Seams with other U plans
- **U9.** The tip's entrance (I7) is the "fade-in" U9 left to "the later animation plan" (`../done/u9-tips.md:195`).
- **U11 (finish).** Checks the whole window with motion on, writes the final docs and bench; it must not tune durations (that happens in the lab).
- **M7 (hardening)** and later: an in-app settings page lists `motion` and `sound` (index Q7).

## Risks and things to measure first
- **R1 `uniform_list` rows have one height.** The lab collapses a row's height; in the app the close must be a FLIP offset of the rows below (I3). Spike it in list and column view before I3, including a removal while scrolled and a removal at the bottom edge.
- **R2 Watcher timing.** The listing drops the entry 0–200 ms after the op ends (100 ms poll + stat). Measure the real spread for trash and move; the swallow (180 ms) must look whole whatever the timing (ghost fallback, I3).
- **R3 Volume.** NSSound may play at output volume, louder than Finder's sounds at a low alert volume. Measure with the alert slider low. Root-cause option: `AudioServicesPlaySystemSoundWithCompletion` (follows the alert volume and the UI-sounds switch) — but it cannot fade the trash tail; that would need a trimmed copy written to the cache dir at first launch. ASK with the measurement before building either.
- **R4 Performance.** Animations request frames only while running; check with the bench that an idle window renders no extra frames in Full (no `repeat()` anywhere, springs settle).
- **R5 Many rows.** Cap effects (I3) so trashing 10 000 items costs nothing extra.

## Questions to ASK
- Q1 Default sound set? Recommended **`finder`** (only sounds Finder makes: trash, empty trash, alert), so Crow is quiet out of the box and `crow` is one setting away. Alternative: `crow` (the user liked the full set in the lab). Unblocks I2.
- Q2 Cursor glide in Full? Recommended **yes, Full only**, after the I8 lag check (it was on in the lab and the user did not object). Alternative: leave it out. Unblocks I8.
- Q3 Sheets and the palette: fade + slight scale in Full, fade in Low? Recommended **yes** (140 / 120 ms); not in the lab yet, so add it to the lab first and let the user try it. Unblocks I7's sheet part.
- Q4 A "+1" on the target folder when a move lands: the list has no item counts per folder today. Recommended **pulse only, no count** in the app. Unblocks I4.
- Q5 Should `off` also silence sounds? Recommended **no**: motion and sound are separate settings (user, 2026-11-14).

## Hand checks
In the bundled app (`./scripts/bundle.sh`), FireBox dark and Solarized Light, each in `motion = off`, `low`, `full`, sound `finder` and `crow`:
- Trash one file, three files, a folder of 500 files (⌘⌫) in list and column view → lab look; one sound; cursor lands where it does today.
- Drag a file onto a folder row; onto a sidebar favorite → target pulse; Purr in `crow`.
- ⇧⌘N → new folder arrives, rename field takes typing immediately.
- ⌘Z after a trash → the file arrives; Bottle in `crow`.
- Trash a locked file (`chflags uchg`) → Refuse shake (Full) / tint (Low) / beep only (Off).
- Copy a 2 GB file → progress, bar fades at the end, Glass in `crow`.
- Toggle System Settings ▸ Accessibility ▸ Reduce Motion with `motion = "auto"` → switches Full ↔ Low without relaunch.
- System Settings ▸ Sound: "Play user interface sound effects" off → no sounds except the alert; alert volume low → Crow's sounds are as quiet as Finder's (R3).
- `crow --motion-lab` still runs and matches the app.

## Finish
Progress file "Ready for critique", branch committed locally; the critique agent opens the PR (AGENTS.md). Update the index (`README.md` table row U10) and move this plan to `done/` after merge.
