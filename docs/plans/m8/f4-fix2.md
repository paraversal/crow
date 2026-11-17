# M8 F4 fix 2: hover out, motion, the Tab hint

Status: Awaiting approval · Branch: `m8-f4-path-bar` (F4, unmerged) · 2026-11-17

## Why

The user tested the V2 path bar: the hover pill sticks when the pointer leaves the Crow window; there are no animations (the fade/expand were deferred); the field's "Tab complete" hint claims a completion the field doesn't have. This fix list addresses the three before F4 merges.

## Target

The V2 bar as built in `f4-fix.md` (38 px bar, 32 px hover pill, pencil at the path's end, 32 px field across the left section). The three findings above are the only changes.

## Questions (user)

**Q1 — The hover fix: patch GPUI or keep the bar's own state?**
The cause is in GPUI: `Window::mouse_hit_test` is recomputed from the last mouse position (`vendor/zed/crates/gpui/src/window.rs:4134`, `6451`), and the `MouseExited` branch (`window.rs:6112-6115`) updates the modifiers but never clears it. `hitbox.is_hovered()` (`window.rs:818`, `872`) therefore stays true after the pointer leaves, so every `hover`/`group_hover` style stays painted.
- **A (recommended): patch GPUI.** Clear `mouse_hit_test` after the `MouseExited` event is dispatched (so the `on_mouse_exit` listeners still see the old hit test) and notify; regenerate `patches/gpui-mouse-exit-hover.patch` through `scripts/fetch-gpui.sh`. Fixes every hover in the app (the pill, the segments, rows, tabs), not just the bar. Cost: a new vendored patch, which `AGENTS.md` says needs a check-in; a GPUI test.
- **B: the bar's own hover state.** Track hover with `on_mouse_move`/`on_mouse_exit` on the path area and style the pill from it. No patch, but only the pill clears; the segments and every other hover still stick, and the bar re-implements what GPUI's `hover` does.

**Q2 — The animations' shape and timing?**
- **A (recommended):** the pill's background, border and pencil fade over 120 ms `ease_out` on hover in/out (the segments stay opaque); on edit the field expands from the pill's width to the left section over 160 ms `ease_out`, the hints fading in; both skip when the effective motion is Off (`crow_core::motion::Effective::Off`, the `text_input.rs:238` pattern). The `with_animation` pattern is already in the bar (`bottom_bar.rs:864`, `917`).
- **B:** name your durations/easing; the shape above stays.

**Q3 — The "Tab complete" hint: remove or implement?**
- **A (recommended, matches your report):** remove the `COMPLETE` hint from `PATH_EDIT_HINTS` (`bottom_bar.rs:245-247`) and the `Complete` action/binding if the path field is its only user; the field's hints then show only what works (Return, Esc). The `text_input::Complete` action and the help entry (`window.rs:2882`) go with it if nothing else uses them.
- **B: implement completion.** Tab completes the typed path (the common prefix, then the list of matches) in the field. A real feature with its own tests; more than this fix list.

## Items

### 1 The hover clears when the pointer leaves the window

**Cause:** GPUI's `MouseExited` branch leaves `mouse_hit_test` stale; `is_hovered()` keeps returning true, so the pill's `group_hover("path-area")` and every `hover` style stay painted.

**Change (Q1 A):** in `vendor/zed/crates/gpui/src/window.rs:6112-6115`, after the `MouseExited` event is dispatched, set `self.mouse_hit_test = HitTest::default()` and notify; regenerate the patch. If Q1 is B, the bar's own `hovered` state replaces the pill's `group_hover`, cleared by `on_mouse_exit`.

**Tests:** a GPUI test (the vendored `window.rs` tests) that hit-tests a hitbox, sends `MouseExited`, and asserts the hitbox is no longer hovered; `cargo test -p gpui`. The Crow hand check: the pill and the segments clear when the pointer leaves the window and when the app loses focus.

**Done when:** no hover style survives the pointer leaving the window; the test passes; the patch file is in `patches/`.

### 2 The pill fades and the field expands

**Cause:** the V2's pill and field are static; the fade/expand were deferred.

**Change (Q2 A):** add `HOVER` (120 ms) and `FIELD` (160 ms) to `crow_core::motion::ms`; in `bottom_bar.rs`, wrap the pill in `with_animation` with an id that includes the hover state, fading the pill's `surface`/`border` and the pencil's `muted`→`fg`; wrap the field in `with_animation` with an id that includes the editing state, setting its width from the pill's width to the left section's width and the hints' opacity from 0 to 1; both return the final state when the effective motion is Off.

**Tests:** the existing bar tests keep the final layout (the field's width, no shift at rest); the animations are visual, so the hand checks cover them. No new test for the interpolation itself.

**Done when:** the pill fades in and out; the field expands from the pill to the left section; both are instant with motion Off; light/dark and 640 px look right.

### 3 The "Tab complete" hint

**Cause:** `PATH_EDIT_HINTS` includes `COMPLETE` ("Tab complete"), but the path field has no completion; the hint claims a key that does nothing.

**Change (Q3 A):** remove `COMPLETE` from `PATH_EDIT_HINTS` and the `Complete` action/binding and help entry if the path field is their only user; the field's hints show Return and Esc only.

**Tests:** the `hint_set` test asserts the field's hints no longer include "complete"; the field's Tab does nothing (or moves focus as before) and no keymap entry mentions completion.

**Done when:** the field shows no completion hint and no dead completion key.

## Finish

Run `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo test -p gpui` if the patch lands, the bench on `target/fixture-100k` against `main` (the bar's render path changes), and `./scripts/bundle.sh`. Update `README.md` and `docs/testing.md` for the hover-out, the motion and the removed hint. Commit on `m8-f4-path-bar`, update `docs/plans/m8/f4-fix2.progress.md`, stop at "Ready for test". Don't merge; no PR until the user has tested.

**Hand checks:** hover the path, then move the pointer out of the window (the pill clears); the same on a segment, a row and a tab; the pill fades in/out; click the path (the field expands from the pill; the hints fade in); motion Off (instant); 640 px and wide; Firebox and Solarized Light; ⌘L and double-click; the field's hints show no "Tab complete"; VoiceOver; search mode.

## Answers (user, date)

1. **Q1:** —
2. **Q2:** —
3. **Q3:** —
