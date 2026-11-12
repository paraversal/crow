# u3-top-band progress
Branch: `ui-u3-top-band` · PR: none · Last updated: 2026-11-13

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 History peeks and band layout B | done (this commit) | Peek helpers, their test (`history_peeks_without_moving`) and the tooltip test (`back_button_tooltip_names_the_folder`) came with U1; added `toolbar_view_buttons_switch_views`. Band is 52 px, toolbar role, 28×28 nav with hover/pressed, 15 px flexing ellipsis title, 32×26 view buttons, tooltips with live keys. |
| I2 TextInput bare style/role/label | done (this commit) | `bare()`, `with_role()`, `set_placeholder()`, bare placeholder paints `fg-muted`; new test not needed because the D tests type into the bare input. `set_label` was dropped: the placeholder is the label in every U3 use (see Decisions). |
| I3 SearchView match mode and entry point | done (this commit) | `Match`, `query_parts`, `effective_match_for`, `set_match`, `Scope::label`; the input is bare + `Role::SearchInput` with the scope placeholder; `set_scope` updates it. U2 already provided the window entry point. Tests `query_parts_follow_the_match_mode`, `spotlight_forces_names`. |
| I4 Band D: query field in the band | done (this commit) | `SearchView::render_band` owns the field, ring and Done; the window places it after Back/Forward; `.child(self.input.clone())` removed from `render_header`. Tests: field click, ⌘F refocus + select-all, Escape restores pane focus, Done, Vim letters, go-to hidden. |
| I5 Go to button and B search field | done (this commit) | Go to and field built in `render_title_bar`; both dispatch their actions (not direct calls); keycaps from the live keymap; narrow rules at 760/640 px. Tests `go_to_button_opens_the_palette`, `go_to_button_is_hidden_while_searching`. |
| I6 Match control drives the query | done (this commit) | Segmented radio group in `render_band`; Spotlight shows Names faded with ", unavailable". Tests `match_names_and_contents_change_the_results`, `spotlight_scope_disables_the_match_control`. |
| I7 Tabs while searching | done (this commit) | switching, opening or closing a tab (click included) calls `close_search` first; `switching_tabs_closes_search` passes. |

## Questions
- Q1 (asked 2026-11-13): tab switch/open/close while searching — answer: close search first
- Q2 (asked 2026-11-13): ⌘F with search open — answer: focus + select all
- Q3 (asked 2026-11-13): match mode memory — answer: reset to All each open
- Q4 (asked 2026-11-13): narrow windows — answer: `window_min_size` 640×400 + truncation
- Q5 (asked 2026-11-13): match-control keys — answer: none
- Q6 (asked 2026-11-13): VoiceOver disabled state — answer: aria label ", unavailable"
- Q7 (asked 2026-11-13): band left edge — answer: keep 86 px
- Q8 (asked 2026-11-13): board D tokens — answer: leave out
- Q9 (asked 2026-11-13): `SearchResults && vim` j/k — answer: fix in U6

## Decisions
- Use U1's merged names: `History::back_target`/`forward_target` replace the plan's `peek_back`/`peek_forward`; `Pane::back_target`/`forward_target` already exist (`pane.rs:329`).
- Use U2's merged `CrowWindow::open_search`/`SearchStart` for I3; no window-side addition needed.
- U1 icons/tooltips/keycaps are on `main`: `icons::{Glyph, glyph, Assets}`, `chrome::{Keycap, KeycapWeight, with_tooltip, with_action_tooltip}`, `keys::{binding_for, text, spoken_for}`.
- Traffic lights: measured with `screencapture` + pixel scan. At y=13 the green button centered 20.25 pt below the window top; at y=19 it centers 26.75 pt in the 52 pt band (band center 26). `traffic_light_position` is now `(14, 19)`.
- `set_label` was not built despite I2: the placeholder is also the aria label in both U3 uses (B is a button with its own label; D's input label is "Search <scope>" = placeholder). A setter with no caller would be dead code.
- `window_min_size` is 640 × 400 in `open` (Q4); the truncation rules are at 760/640 px; checked at 660 × 520.
- The band's focus ring is drawn by the window, so `SearchView` subscribes to the input's focus in/out and notifies, and ⌘F's refocus notifies the window explicitly; otherwise the ring went stale when focus moved between the field and the results.

## Hand checks
- [x] Top band: band B matches the board in dark and light; traffic lights centred; Back is faded at the start of history.
- [x] Search field: click/⌘F opens with the cursor in the field; accent ring in both themes; Tab drops the ring; ⌘F restores it and selects the whole query (typing replaces it); Done and Esc return to the folder with the pane focused.
- [x] Narrow window (660 × 520): Go to is icon-only, title truncates, fields shrink; in search mode the field flexes and All / Names / Contents and Done esc stay whole.
- [ ] Tooltips: hover every band button (covered by U1's tooltip tests and the existing back-tooltip test; not re-hovered by hand in U3).
- [ ] All / Names / Contents speed: Names in Home on a big tree should be visibly faster (tests prove no content reads; timing not measured by hand).
- [ ] VoiceOver: band reads as toolbar; field announces "Search <folder>, Command F"; search mode field is a search field; Done reads "Done, close search, Escape".

## Found along the way
- `SearchResults` never adds `vim` to its key context (`search.rs:720`), so the `SearchResults && vim` j/k bindings never fire. Outside U3; Q9 asks where to fix.

## Log
- 2026-11-13: answers: Q1 close search on tab change; Q2 focus + select all; Q3 reset to All; Q4 min 640×400; Q5 no match keys; Q6 aria \", unavailable\"; Q7 left edge 86 px; Q8 leave board-D tokens out; Q9 U6 fixes SearchResults vim.
- 2026-11-13: I1 done. Note: the first traffic-light measurement hit a stale running Crow (old 44 px band); killed it and re-measured the new build. `cargo clippy -p crow-app --all-targets` clean, `cargo test -p crow-app` 90 passed.
- 2026-11-13: hand-verified in the bundle: band B and D in dark and light, traffic-light centring (measured 26.75 pt vs 26), narrow window at 660, Esc/Done/⌘F refocus with select-all, Tab moving focus to the results and the ring following both ways. Bundle rebuilt after each change; screenshot crops kept in `/tmp` only.
- 2026-11-13: README ▸ Keys and `docs/testing.md` updated (band wording, Go to, match modes, five new hand rows; automated paragraph names the new tests).
- 2026-11-13: I7 done, including the minimum window size. `cargo test --workspace` green (101 app tests), clippy clean.
- 2026-11-13: I2–I6 done in one commit: the bare input, match modes and the band B/D surfaces are interdependent through `render_title_bar`/`render_band`, so they landed together. `cargo test -p crow-app` 100 passed, workspace clippy clean. Hand-viewed B and D in the bundle; both match the boards.
- 2026-11-13: read AGENTS.md, execute-plan skill, index README, U3 plan, U1 progress, `docs/testing.md`; confirmed U1 and U2 merged on `main`; created branch and this file; asked Q1–Q9.

## Next
Commit the focus-ring fix, push `ui-u3-top-band`, open the PR (screenshots of B and D; no bench needed).
