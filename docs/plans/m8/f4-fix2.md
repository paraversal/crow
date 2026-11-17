# M8 F4 fix 2: hover out, motion, the Tab hint

Status: Awaiting approval · Branch: `m8-f4-path-bar` (F4, unmerged) · 2026-11-17

## Why

The user tested the V2 path bar: the hover pill sticks when the pointer leaves the Crow window; there are no animations (the fade/expand were deferred); the field's "Tab complete" hint does nothing visible in the opened-field state (the completion exists; the field opens with the full path selected). This fix list addresses the three before F4 merges.

## Target

The V2 bar as built in `f4-fix.md` (38 px bar, 32 px hover pill, pencil at the path's end, 32 px field across the left section). The three findings above are the only changes.

## Questions (user)

**Q1 — The hover fix: patch GPUI or render the pill from a local hover bool?**
The cause is in GPUI: `PlatformInput::MouseExited` (`vendor/zed/crates/gpui/src/window.rs:6112-6115`) updates the modifiers but not `mouse_position`; both `.hover` and `.group_hover` styles are computed at paint time from `mouse_hit_test`, built from that stale position (`window.rs:4134`, `6451`; `div.rs:3569-3599`). `hitbox.is_hovered()` therefore stays true after the pointer leaves.
- **A (recommended): patch GPUI.** On `MouseExited`, after the exit dispatch (so the `on_mouse_exit` listeners still see the old hit test), move `mouse_position` outside the window and clear `mouse_hit_test`, then notify; regenerate `patches/gpui-mouse-exit-hover.patch` through `scripts/fetch-gpui.sh`. Fixes every hover in the app. Cost: a new vendored patch, which `AGENTS.md` says needs a check-in; a GPUI test.
- **B: render the pill from a local hover bool.** `on_hover` (the listener does get the `MouseExited` callback, `div.rs:3310`) sets a `hovered` bool; the pill's background/border/pencil use it instead of `group_hover`. No patch, but only the pill clears; every other `.hover`/`.group_hover` (segments, rows, tabs) still sticks.

**Q2 — The animations' shape and timing?**
- **A (recommended):** the pill's background, border and pencil fade over 120 ms `ease_out` on hover in/out (the segments stay opaque); on edit the field expands from the pill's width to the left section over 160 ms `ease_out`, the hints fading in; both skip when the effective motion is Off (`crow_core::motion::Effective::Off`, the `text_input.rs:238` pattern). The `with_animation` pattern is already in the bar (`bottom_bar.rs:864`, `917`).
- **B:** name your durations/easing; the shape above stays.

**Q3 — The "Tab complete" hint: make Tab visible or remove the hint?**
Completion exists: `text_input.rs:96` binds Tab→Complete, `window.rs:602` calls `complete_path_edit` → `crow_core::complete_typed` (`paths.rs:34`). The user saw nothing because ⌘L opens the field with the full path selected (`window.rs:598`) and the last component is already complete, so Tab returns the same text.
- **A (recommended): make Tab visibly useful.** When the typed path's last component is complete, Tab cycles the parent's matching entries (the field's text changes; the hint stays). Cost: completion state (the prefix, the matches, the index) and its tests.
- **B: remove the hint.** The completion stays (Tab still completes a partial path), but the field no longer claims it. Cost: the user loses the hint; the completion is undocumented.

## Items

### 1 The hover clears when the pointer leaves the window

**Cause:** `MouseExited` updates only the modifiers; `mouse_position` stays at the last in-window point, and the paint-time `.hover`/`.group_hover` styles read `mouse_hit_test` built from it, so `is_hovered()` keeps returning true.

**Change (Q1 A):** in `vendor/zed/crates/gpui/src/window.rs:6112-6115`, after the `MouseExited` event is dispatched, move `mouse_position` outside the window (e.g. `Point::new(px(-1.), px(-1.))`) and clear `mouse_hit_test`, then notify; regenerate the patch. If Q1 is B, the pill's own `hovered` bool, set by `on_hover`, replaces the pill's `group_hover`.

**Tests:** a GPUI test (the vendored `window.rs` tests) that hit-tests a hitbox, sends `MouseExited`, and asserts the hitbox is no longer hovered; `cargo test -p gpui`. The Crow hand check: the pill and the segments clear when the pointer leaves the window and when the app loses focus.

**Done when:** no hover style survives the pointer leaving the window; the test passes; the patch file is in `patches/`.

### 2 The pill fades and the field expands

**Cause:** the V2's pill and field are static; the fade/expand were deferred.

**Change (Q2 A):** add `HOVER` (120 ms) and `FIELD` (160 ms) to `crow_core::motion::ms`; in `bottom_bar.rs`, wrap the pill in `with_animation` with an id that includes the hover state, fading the pill's `surface`/`border` and the pencil's `muted`→`fg`; wrap the field in `with_animation` with an id that includes the editing state, setting its width from the pill's width to the left section's width and the hints' opacity from 0 to 1; both return the final state when the effective motion is Off.

**Tests:** the existing bar tests keep the final layout (the field's width, no shift at rest); the animations are visual, so the hand checks cover them. No new test for the interpolation itself.

**Done when:** the pill fades in and out; the field expands from the pill to the left section; both are instant with motion Off; light/dark and 640 px look right.

### 3 The "Tab complete" hint

**Cause:** the completion exists (`text_input.rs:96`, `window.rs:602`, `crow_core::complete_typed`), but ⌘L opens the field with the full path selected and the last component complete, so Tab returns the same text and the hint looks dead.

**Change (Q3 A):** in `complete_path_edit`/`complete_typed`, when the typed path's last component is already complete, Tab cycles the parent's matching entries (the field's text changes each press); the hint stays. If Q3 is B, remove `COMPLETE` from `PATH_EDIT_HINTS`; the completion stays.

**Tests:** for A, `complete_typed` cycles the matches from a complete path and the field's Tab changes the text; for B, the `hint_set` test asserts the field's hints no longer include "complete".

**Done when:** A: Tab visibly changes the field from a complete path; B: the hint is gone and Tab still completes a partial path.

## Finish

Run `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo test -p gpui` if the patch lands, the bench on `target/fixture-100k` against `main` (the bar's render path changes), and `./scripts/bundle.sh`. Update `README.md` and `docs/testing.md` for the hover-out, the motion and the removed hint. Commit on `m8-f4-path-bar`, update `docs/plans/m8/f4-fix2.progress.md`, stop at "Ready for test". Don't merge; no PR until the user has tested.

**Hand checks:** hover the path, then move the pointer out of the window (the pill clears); the same on a segment, a row and a tab; the pill fades in/out; click the path (the field expands from the pill; the hints fade in); motion Off (instant); 640 px and wide; Firebox and Solarized Light; ⌘L and double-click; the field's hints show no "Tab complete"; VoiceOver; search mode.

## Answers (user, date)

1. **Q1:** —
2. **Q2:** —
3. **Q3:** —
