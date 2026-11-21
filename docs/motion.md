# Motion

How Crow moves, and where each effect lives. The timing table below is checked against
`crow_core::motion::ms` by a test, so a changed constant fails until this page says the
same. Measure with `crow --bench target/fixture-100k` (frame times) and
`CROW_BENCH_TRASH=N` (the trash effect on a 100k folder, see [testing.md](testing.md)).

## Principles

- **Predictable.** Motion never moves what the user did not act on. An effect stays in
  its own box or follows a path the eye already follows (a row closing its gap, a column
  opening its width). Nothing slides under the pointer while the user aims.
- **Never blocks input.** Effects are cosmetic: state changes at once and only the picture
  lingers (`motion_ui::Leaving`). Keys and clicks during an effect act on the end state.
- **One source of truth.** Durations, easings and looks live in `crow_core::motion`; views
  only read them.

## Modes

`motion` in `settings.toml` is `auto`, `off`, `low` or `full`; `auto` is Full unless macOS
Reduce Motion is on, then Low.

| Mode | What moves |
| --- | --- |
| Off | The end state in one frame. |
| Low | Opacity and colour only. Nothing changes position, size or height; gaps snap. |
| Full | Everything below. |

## Timings

Milliseconds, Full unless a surface says otherwise.

| Constant | ms |
| --- | --- |
| `TRASH_SWALLOW` | 200 |
| `TRASH_CLOSE` | 130 |
| `MOVE_FLY` | 260 |
| `MOVE_CLOSE` | 140 |
| `RECEIVE` | 360 |
| `ARRIVE_OPEN` | 120 |
| `ARRIVE_IN` | 180 |
| `ARRIVE_TINT` | 420 |
| `REFUSE` | 320 |
| `TIP_FULL` | 220 |
| `LOW_OUT` | 140 |
| `LOW_IN` | 160 |
| `LOW_TINT` | 420 |
| `STATUS` | 140 |
| `FIELD` | 160 |
| `COPY` | 1600 |
| `COPY_FADE` | 320 |
| `DISCLOSE` | 180 |
| `PANEL_TAB` | 155 |
| `VIEW_CHANGE` | 245 |
| `CHILD_COLUMN` | 145 |
| `NAVIGATION_LOW` | 120 |
| `REVEAL` | 300 |
| `SHEET_IN_LOW` | 120 |
| `SHEET_IN` | 140 |
| `SHEET_OUT` | 110 |
| `SCRIM` | 120 |
| `PANE_SLIDE` | 180 |
| `SECTION` | 160 |
| `TAB` | 150 |
| `SEGMENT` | 155 |
| `SEARCH_BAND` | 200 |
| `FILTER_ROW` | 140 |
| `REORDER` | 220 |
| `PROGRESS` | 250 |
| `PREVIEW_FADE` | 120 |
| `TOOLTIP` | 100 |
| `FIELD_IN` | 100 |
| `SPRING_BLINK` | 70 |
| `SPRING_DIP` | 110 |

## Surfaces

| Surface | Full | Low |
| --- | --- | --- |
| Trash, move, receive, arrive, refuse (rows) | Trash: name slides into the trash icon (eased), the icon gulps while visible, the gap closes from the swallow's last third. Others: ghost flight, pulse, slide, shake | Fades and tints |
| List, column and sidebar selection | Spring glide of the fill | Sidebar fades |
| View switch (⌘1–4) | Matching items glide | Crossfade |
| Sort change (header click, View ▸ Sort By) | Visible rows glide (`REORDER`), new rows fade in; skipped over 200 visible rows | List fades |
| Icon size | Tiles glide to the new grid | Snaps |
| Sheets, palette, pickers: open | Fade in, scrim fades in (`SCRIM`) | Same |
| Sheets, palette, pickers: close | Fade out and sink 4 px (`SHEET_OUT`), scrim fades out; input is already released | Fade out |
| Palette highlight | Spring glide between rows; typing snaps it | Snaps |
| Right column (⇧⌘P, ⌘I) | Width eases (`PANE_SLIDE`), content fades after half | Width snaps, content fades |
| Sidebar sections | Rows grow and fade in, closing rows fade while the gap shrinks, chevrons crossfade | Rows fade in; collapse is instant |
| Window tabs | New tab grows, closed tab shrinks away, active fill glides | New tab fades; close is instant |
| Toolbar view switcher | Selected segment glides | Snaps |
| Search band (⌘F) | Field widens across the band, then controls fade in; reverses on Done | Swap and fade |
| `/` filter row | Row opens its height; closing shrinks a picture of the field | Fade in; close is instant |
| Hidden files (⌘⇧.) | List crossfades (rows are swapped by a relist) | Same |
| Favorites reorder | Moved rows glide from their old slot | Snaps |
| Copy progress | Bottom bar line eases toward each value (spring, no overshoot); a finished Activity row shrinks away | Steps; instant |
| Big preview (Preview column, Gallery) | Fades in when it lands after a pause; placeholders never fade | Same |
| Settings switches and segments | Knob slides, tint and segment fill glide | Tint fades |
| Tooltips | Fade in over `TOOLTIP`, out at once | Same |
| Text fields | Outline fades in over `FIELD_IN`; text and caret are immediate | Same |
| Theme change | Whole-window OKLab crossfade, 500 ms | 150 ms |
| Spring-loaded folder | Double blink | One dip |

## No motion on purpose

Typing, the caret, moving the cursor through a list (only the fill glides), scrolling,
hover fills, column resize, divider drags, search results streaming in, context menus
(native) and Quick Look (native).

## Measuring

`crow --motion-lab` fires every row effect in the three modes at ×5 slow motion. For frame
times use `CROW_BENCH_TRASH=N target/release/crow --bench target/fixture-100k`: it reports
frames over 1.5× the idle median as hitches. Compare against `main` before calling a
change a regression.
