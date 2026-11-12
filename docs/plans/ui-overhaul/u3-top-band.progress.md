# u3-top-band progress
Branch: `ui-u3-top-band` · PR: none · Last updated: 2026-11-13

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 History peeks and band layout B | done (this commit) | Peek helpers, their test (`history_peeks_without_moving`) and the tooltip test (`back_button_tooltip_names_the_folder`) came with U1; added `toolbar_view_buttons_switch_views`. Band is 52 px, toolbar role, 28×28 nav with hover/pressed, 15 px flexing ellipsis title, 32×26 view buttons, tooltips with live keys. |
| I2 TextInput bare style/role/label | todo | |
| I3 SearchView match mode and entry point | todo | U2 (merged, 07f49cd) already added `CrowWindow::open_search(SearchStart { folder, query, everything })` + test `open_search_with_a_query_runs_it`; only the SearchView side remains. |
| I4 Band D: query field in the band | todo | |
| I5 Go to button and B search field | todo | |
| I6 Match control drives the query | todo | |
| I7 Tabs while searching | blocked (Q1) | |

## Questions
- Q1 (asked 2026-11-13): tab switch/open/close while searching — close search first (recommended), or per-tab search? → answer: pending
- Q2 (asked 2026-11-13): ⌘F with search open — focus + select all (recommended) or focus only (today)? → answer: pending
- Q3 (asked 2026-11-13): match mode memory — reset to All each open (recommended), session, or `settings.toml`? → answer: pending
- Q4 (asked 2026-11-13): narrow windows — `window_min_size` 640×400 + truncation (recommended), or no minimum? → answer: pending
- Q5 (asked 2026-11-13): match-control keys — none (recommended) or ⌥⌘1/2/3 in Search? → answer: pending
- Q6 (asked 2026-11-13): VoiceOver disabled state — aria label ", unavailable" (recommended) or GPUI `aria_disabled` patch? → answer: pending
- Q7 (asked 2026-11-13): band left edge — keep 86 px (recommended) or push nav to ≈176 px? → answer: pending
- Q8 (asked 2026-11-13): board D's scope token / "names + contents" caption — leave out (recommended) or include? → answer: pending
- Q9 (asked 2026-11-13): `SearchResults && vim` j/k never fire (`search.rs:720` lacks `vim`) — fix in U6 (recommended) or in U3? → answer: pending

## Decisions
- Use U1's merged names: `History::back_target`/`forward_target` replace the plan's `peek_back`/`peek_forward`; `Pane::back_target`/`forward_target` already exist (`pane.rs:329`).
- Use U2's merged `CrowWindow::open_search`/`SearchStart` for I3; no window-side addition needed.
- U1 icons/tooltips/keycaps are on `main`: `icons::{Glyph, glyph, Assets}`, `chrome::{Keycap, KeycapWeight, with_tooltip, with_action_tooltip}`, `keys::{binding_for, text, spoken_for}`.
- Traffic lights: measured with `screencapture` + pixel scan. At y=13 the green button centered 20.25 pt below the window top; at y=19 it centers 26.75 pt in the 52 pt band (band center 26). `traffic_light_position` is now `(14, 19)`.

## Hand checks
- [ ] Top band: hover Back, Forward, view buttons, Go to, search field → tooltips with keys; faded Back at history start; traffic lights centred; light and dark.
- [ ] Search field: click opens with cursor in field; accent ring; Done esc; Tab to results drops the ring; ⌘F restores it; Esc/Done returns focus to the list.
- [ ] All / Names / Contents: Names drops content rows; Contents drops name-only rows; Names in Home is fast; Everywhere shows Names faded.
- [ ] Narrow window: title truncates, then field shrinks, then Go to loses its label; in search mode field shrinks, match control and Done stay.
- [ ] VoiceOver: band reads as toolbar; field announces "Search <folder>, Command F"; search mode field is a search field; Done reads "Done, close search, Escape".

## Found along the way
- `SearchResults` never adds `vim` to its key context (`search.rs:720`), so the `SearchResults && vim` j/k bindings never fire. Outside U3; Q9 asks where to fix.

## Log
- 2026-11-13: answers: Q1 close search on tab change; Q2 focus + select all; Q3 reset to All; Q4 min 640×400; Q5 no match keys; Q6 aria \", unavailable\"; Q7 left edge 86 px; Q8 leave board-D tokens out; Q9 U6 fixes SearchResults vim.
- 2026-11-13: I1 done. Note: the first traffic-light measurement hit a stale running Crow (old 44 px band); killed it and re-measured the new build. `cargo clippy -p crow-app --all-targets` clean, `cargo test -p crow-app` 90 passed.
- 2026-11-13: read AGENTS.md, execute-plan skill, index README, U3 plan, U1 progress, `docs/testing.md`; confirmed U1 and U2 merged on `main`; created branch and this file; asked Q1–Q9.

## Next
I2: TextInput bare style, role/label setters, placeholder paint; then I3.
