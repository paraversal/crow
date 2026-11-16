# M7 H6 — Editable path bar

Audience: an implementing agent. Follow the `execute-plan` skill; read `AGENTS.md`, `docs/plans/m7/README.md` and `docs/plans/ui-overhaul/README.md` (Q10) first.
Base: branch `m7-h6-path-bar` from `main`. Progress file: `docs/plans/m7/h6-editable-path-bar.progress.md`.
PR title: "M7 H6: editable path bar". Don't merge.

## Why
The UI overhaul left path editing to M7 (index Q10: "Path editing stays with the bottom path bar and is decided in M7"; U5 left segment clicks alone). Since then, ⇧⌘G opens ⌘K with the current path typed (`done/u7-fix.progress.md`), and ⌘K completes paths with ⇥. So the first decision is whether the bar should become editable at all, or whether ⌘K already covers it (Q1).

## Current state
- Bottom bar path slot: `crates/crow-app/src/bottom_bar.rs`, segments `path-segment-<i>` (≈ 600–627), `Role::Button`, tooltip, click navigates (or opens a file segment); folding into "…" by `fit` (≈ 303).
- Text field: `crates/crow-app/src/text_input.rs` (`TextInput`, blinking caret, `.bare()` style).
- Path completion: the palette's ⇥ completion (`palette.rs`, background `read_dir`) and `crow_core::resolve_typed`.

## Items (if Q1 = build it)
### I1 Edit mode
- **Enter:** double-click empty space in the path slot, or a key (Q2). The segments are replaced by a `TextInput` with the full path (`~` abbreviated as in the palette), all selected.
- **Typing:** ⇥ completes the folder name like ⌘K (reuse the palette's completion function, not a copy; move it to a shared place if needed). Return goes to the folder, or reveals a file (as `reveal_typed_path` does). A path that doesn't exist: beep (U10 sound set), field stays open, error in the bar's status slot.
- **Leave:** Escape, clicking elsewhere, or focus loss restore the segments without navigating.
- Hints slot shows `↵ go · ⇥ complete · esc cancel` while editing (U5 hints).
### I2 Accessibility and layout
The field has `Role::TextInput` and aria label "Path"; the bar's layout doesn't jump (same height, the field takes the path slot's width; hints and counts keep their places or drop as `fit` already decides).
### I3 Tests
Keystroke tests: enter edit mode, type `~/`, ⇥ completes, Return navigates; Escape restores; a missing path beeps and keeps the field; clicking a row while editing cancels. Layout test: path slot height unchanged in edit mode.

## Finish
fmt · clippy · tests; bench not needed (bottom bar only; say so). README keys table; `docs/testing.md` row. Bundle, hand checks (dark and Solarized Light, long paths, a 255-byte name). Commit, push, "Ready for critique".

## Questions to ASK
1. **Build it, or close Q10 as "⌘K / ⇧⌘G is the path editor"?** Recommended: build it, small: some users reach for the path itself, and the bar is where Finder (⌥⌘P) and most explorers put it. If the user says ⌘K is enough, the piece becomes: README note + make clicking the path slot's empty space open ⇧⌘G, and nothing else.
2. **Which key enters edit mode?** ⌘L (browsers, many file managers) or ⇧⌘G (Finder, now opens ⌘K)? Recommended: ⌘L for the bar; ⇧⌘G stays with ⌘K. Check the keymap for a clash first (`keys.rs`).
3. **Single click on empty path space edits, or double-click?** Recommended: double-click; a single click there is too easy to hit by accident next to a segment.
