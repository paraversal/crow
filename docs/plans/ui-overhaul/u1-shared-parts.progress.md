# u1-shared-parts progress
Branch: `ui-u1-shared-parts` · PR: none · Last updated: 2026-11-12

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 Keys from the live keymap | todo | |
| I2 Keycap element | todo | |
| I3 Tooltip | todo | |
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

## Next
Create branch `ui-u1-shared-parts`, commit this progress file, start I1.
