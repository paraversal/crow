# u1-shared-parts progress
Branch: `ui-u1-shared-parts` · PR: https://github.com/paraversal/file-crow/pull/8 · Last updated: 2026-11-12 · State: PR open (not merged)

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 Keys from the live keymap | done (78e03f1) | `keys.rs`; palette rows/footer, delete sheet, picker, search header read the live keymap; tests `keys::tests::*`, `command_palette_keys_follow_the_layer` pass; grep has only the sort arrow. |
| I2 Keycap element | done (78e03f1) | three weights, three builders; unit + render smoke tests pass. Hand check FireBox/Paper pending. |
| I3 Tooltip | done (78e03f1) | all Target-table elements have tooltips; `History`/`Pane` peeks added; `history_peeks_without_moving` and `back_button_tooltip_names_the_folder` (incl. hover after 600 ms) pass. |
| I4 Sheet overlay | done (6ded2f7) | five overlays use `chrome::sheet`; `clicking_the_scrim_closes_every_overlay` and `clicking_inside_a_sheet_keeps_it_open` pass. Light/dark scrim and window drag while a sheet is open need a hand check. |
| I5 Icons | done (6ded2f7) | Lucide 1.72.0: chevron-left, chevron-right, list (columns-3 for Columns), search, arrow-right, x, chevron-down; ISC notice at `crates/crow-app/assets/icons/LICENSE`, copied to `Crow.app/Contents/Resources/Lucide-LICENSE.txt` by `scripts/bundle.sh`; embedded via `include_bytes!`; title-bar glyphs 16 px in unchanged 26×26 / 28×22 buttons; `every_glyph_resolves` and `assets_list_icons` pass. Bundle hand check pending. |
| I6 Selection tints | done (6ded2f7) | named tint helpers in `style.rs`; unfocused search selection is now `selection`; `tints_follow_roles` passes; grep shows 0x55 only in `style.rs`; `cursor_ring` is an inset shadow (crisp-ring hand check pending). |

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
- 2026-11-12: I4–I6 done. Sheets keep their ids; Wide is 620, so Why and the palette widen. Scrim click is mouse-down and the panel stops propagation (verified by clicking `sheet-cancel`). `cargo test --workspace` green (71 app tests), clippy clean.
- 2026-11-12: Bench fixture exists at `target/fixture-100k`; bench vs `main` still to run.

## Bench (100k fixture, same machine)
| Run | warm first paint | p50 | p95 | p99 |
| --- | --- | --- | --- | --- |
| U1 branch | 10 ms | 8.33 ms | 8.66 ms | 8.79 ms |
| `main` | 10 ms | 8.33 ms | 8.70 ms | 8.93 ms |

Within run-to-run noise; recorded in `docs/testing.md` next to M6.

## Hand checks done
- Bundle (`scripts/bundle.sh`) built; `Lucide-LICENSE.txt` is in `Contents/Resources/`.
- Title bar: back/forward and list/columns are drawn stroke icons, crisp on Retina, in FireBox.
- Go to Folder sheet: scrim darkens the window (sampled pixels outside the panel darken; `bg`-on-`bg` areas stay equal, as expected for a dark theme), panel 12 px radius, input focused, traffic lights still visible.
- Window drag with the sheet open: the mouse-down first cancels the sheet (outside-click semantics); the drag then moves the window, so dragging is not broken and no scrim-below-title-band change is needed.
- Back tooltip over the disabled button: "Back ⌘[" with the key in `fg-muted`, as the boards show.
- Not hand-checked yet: dark/light scrim pair on a light theme, tooltips for every other Target-table element, the Vim palette keys in the bundle, `cursor_ring` crispness (U8 uses it).

## Next
None: all six items done and verified; PR #8 is open for review and hand testing. Remaining items above are hand checks for the user. Do not merge.
