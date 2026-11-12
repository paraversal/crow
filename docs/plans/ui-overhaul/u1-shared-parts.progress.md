# u1-shared-parts progress
Branch: `ui-u1-shared-parts` · PR: none · Last updated: 2026-11-12

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 Keys from the live keymap | done (this commit) | `keys.rs`; palette rows/footer, delete sheet, picker, search header read the live keymap; tests `keys::tests::*`, `command_palette_keys_follow_the_layer` pass; grep has only the sort arrow. |
| I2 Keycap element | done (this commit) | three weights, three builders; unit + render smoke tests pass. Hand check FireBox/Paper pending. |
| I3 Tooltip | done (this commit) | all Target-table elements have tooltips; `History`/`Pane` peeks added; `history_peeks_without_moving` and `back_button_tooltip_names_the_folder` (incl. hover after 600 ms) pass. |
| I4 Sheet overlay | todo | |
| I5 Icons | todo | |
| I6 Selection tints | todo | |

## Questions
- U1 Q1–Q8 (asked 2026-11-12) → all recommended answers accepted:
  Q1 return glyph ↩; Q2 scrim `bg` 0x8c dark / `fg` 0x40 light; Q3 widths 360/520/620;
  Q4 `Key` fill `bg`; Q5 download Lucide 1.72.0 SVGs + ISC license, copy license into the bundle;
  Q6 unfocused search selection → `selection`; Q7 add `History` peek helpers; Q8 SearchResults vim bug left for U6.

## Decisions
- Icons pinned to Lucide 1.72.0 (latest release at start, 2026-11-12); 8 files, ~2.2 KB total, ISC license.
- Return glyph is ↩ in keys.rs (the single formatting table).

## Log
- 2026-11-12: read `AGENTS.md`, index README, U1 plan; asked the 8 U1 questions and recorded answers.
- 2026-11-12: I1 done. `&'static dyn Action` in the const compiles (no `Box` fallback needed). `caps` treats a multi-key Vim sequence as one glued cap, modifiers split only for single chords.
- 2026-11-12: I2/I3 done. Tests can show tooltips (verified with advance_clock + another mouse move); no asset registration needed here.
- 2026-11-12: `cargo test -p crow-app` 66 passed; clippy clean.

## Next
I4: one sheet frame with a scrim for the five overlays.
