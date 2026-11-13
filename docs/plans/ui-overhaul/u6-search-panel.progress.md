# U6 search panel progress
Branch: `ui-u6-search-panel` · PR: none · Last updated: 2026-11-13

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 This Mac and no Kind | todo | U3 merged (#10); `Scope::Everywhere`, `Row.kind`, Kind column still present |
| I2 Name match ranges | todo | `style::highlight_text` + `truncate_middle_parts`/`map_range` already exist (U1) |
| I3 Scope bar | todo | U3's `render_band` owns the field + match control; `render_header` still has pills |
| I4 Footer, counts, errors, skipped list | todo | one row per path, no hit count today |
| I5 Rows | todo | content line inline after name today |
| I6 Idle state with examples | todo | empty state is one line of text today |
| I7 Preview pane in search mode | todo | preview hidden while search open (`window.rs:2130`) |
| I8 Vim j/k in the results list | todo | bindings exist (`actions.rs`), context lacks `vim` |
| I9 Preview at the hit line | blocked (Q1) | add `text_excerpt` only if the user says yes |
| User: names outrank contents | todo | ordering change in `search.rs` `add_hits`, not in the plan |
| User: highlight name matches | todo | covered by I2 |
| User: key to switch All / Names / Contents | blocked (Q13) | no binding exists; needs the user's chord |

## Questions
- Q1 (asked 2026-11-13): preview at the hit line, add `text_excerpt` (I9)? → pending
- Q2 (asked 2026-11-13): "n results in m files" wording (recommended: only when n > m) → pending
- Q3 (asked 2026-11-13): keep one 40 px row height → pending
- Q4 (asked 2026-11-13): Folder column relative to the scope root vs `~`-abbreviated → pending
- Q5 (asked 2026-11-13): switch label with 0 / 3+ enabled sets → pending
- Q6 (asked 2026-11-13): hide preview in idle / no results / error and below 900 px → pending
- Q7 (asked 2026-11-13): keep the switch visible in idle → pending
- Q8 (asked 2026-11-13): footer hint reads "Skip folders again" when everything is on → pending
- Q9 (asked 2026-11-13): This Mac idle heading "Search names on this Mac" → pending
- Q10 (asked 2026-11-13): Esc keeps closing search while the skipped list is open → pending
- Q11 (asked 2026-11-13): capped copy "First 20,000 files · refine the search" → pending
- Q12 (asked 2026-11-13): scope keys (superseded by the user's own request; see Q13) → pending
- Q13 (asked 2026-11-13): which key switches All / Names / Contents, and does it cycle or pick directly? → pending

## Decisions
- U3 is merged to `main` (#10) with `render_band`, `Match`, `open_search(SearchStart { folder, query, everything })`, `Scope::label`; the plan's "After U3" shape matches the code.
- U1 is merged to `main` (#8): `style::{highlight_text, truncate_middle_parts, map_range, active_selection, inactive_selection, cursor_ring}`, `chrome::{Keycap, with_tooltip, with_action_tooltip}`, `keys::{binding_for, text}` all exist. `highlight_text` already clamps ranges.
- Names outrank contents (user request, 2026-11-13): display all name-match rows above content-only rows; name rows keep walker order, content rows keep arrival order. Insertion happens per batch (name rows before the content block) and `by_path` is rebuilt, so the walker hot path is untouched.
- "Name matches stand out more": name-match rows already carry a highlighted, semibold name and now sort first; content-only rows keep the excerpt under the name. No extra color hierarchy.
- `kind_name` stays for `style::kind`; only `Row.kind`/the Kind column go (I1).

## Hand checks
- [ ] Scope bar in both themes: selected segment, switch knob, hover/pressed → matches `img/search-panel-b.png`
- [ ] Idle state: heading, four examples, key line → matches `img/search-panel-d.png`
- [ ] Footer: counts, accent skipped link, keycap; Search everything flips the switch
- [ ] Skipped list above the footer, Close works, Esc still closes search
- [ ] Preview follows the selection in list and column view; hit line highlighted (if Q1 = yes)
- [ ] VoiceOver: switch reads on/off; footer status reads once per finished search
- [ ] Narrow window ≤ 760 px: switch label truncates first, footer text hides, keycap stays
- [ ] Light + dark: match highlights readable on the selected row

## Found along the way
- `docs/testing.md:27–28` claims search keystroke tests cover the Vim layer; none do (I8 fixes the claim).
- `keys::context_stack(KeyScope::SearchResults)` lacks `vim` although `actions.rs` binds j/k in `SearchResults && vim` (I8).

## Log
- 2026-11-13: read the plan, `search.rs`, `engine.rs`, `style.rs`, `chrome.rs`, `keys.rs`, `preview.rs`, `pane.rs`, `window.rs` search code and tests. U3/U1 merged; branch created from `main`. Questions batched for the user.

## Next
Send Q1–Q13 to the user in one form; meanwhile spike `StyledText` + `truncate_middle_parts` + `map_range` clipping and start I1 (pure rename/column removal).
