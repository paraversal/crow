# U6 search panel progress
Branch: `ui-u6-search-panel` · PR: https://github.com/paraversal/file-crow/pull/14 · Last updated: 2026-11-13

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 This Mac and no Kind | done | `Scope::ThisMac`, folder-named segment, `Row.kind`/Kind column gone; `search_scope_buttons_switch_scope` |
| I2 Name match ranges | done | `HitKind::Name { ranges }`, `FuzzyMatcher::indices`, `style::highlight_text` + `map_range`; engine and app unit tests; bench below |
| I3 Scope bar | done | 38 px bar, segmented scope, `Skip <sets> folders` switch, Spotlight note; `skip_label` unit test, switch/key/hidden tests |
| I4 Footer, counts, errors, skipped list | done | `hit_count`, `footer_summary`, error text, above-footer skipped list; counts/link/Esc/error/unit tests |
| I5 Rows | done | Name 180/Modified 100, content line under the name, name highlights, aria includes the line; row aria test |
| I6 Idle state with examples | done | heading, four examples, live-key line; four keystroke tests |
| I7 Preview pane in search mode | done | background `stat_entry` + `preview::fetch_at`, shared 300 px chrome, hidden < 900 px; list and column tests |
| I8 Vim j/k in the results list | done | `SearchResults vim` context; j/k cursor test; testing.md claim now true |
| I9 Preview at the hit line | done | `crow_core::text_excerpt` (1 MiB, 5 above, 12 lines), hit tint, head fallback; core + app tests |
| User: names outrank contents | done | name block sorted by fuzzy score, content-only rows after; `search_name_hits_outrank_content_hits` |
| User: highlight name matches | done | in I2/I5 |
| User: key to switch All / Names / Contents | done | ⇧⌘M `CycleMatch` in `Search` context; tooltip shows it; cycle test |

## Questions
All asked 2026-11-13; answers from the user.
- Q1 hit-line excerpt → yes (I9 added).
- Q2 footer wording → "n results in m files" only when n > m; "n results" when equal; singular; keeps "files".
- Q3 row height → one 40 px row.
- Q4 Folder column → relative to the scope root; `~`-abbreviated for Home and This Mac.
- Q5 switch label → names the enabled sets, hidden when none; 3+ list names and truncate.
- Q6 preview → hidden in idle/no results/error and below 900 px in search only.
- Q7 switch in idle → keep visible.
- Q8 everything on → footer reads "Skip folders again".
- Q9 This Mac idle → "Search names on this Mac".
- Q10 Esc with skipped list → close the list first, then search (new key test).
- Q11 capped copy → "First 20000 files · refine the search" in warning.
- Q12 scope keys → none beyond Q13.
- Q13 match-switch key → ⇧⌘M cycles All → Names → Contents.
- Name-hit ordering → name rows above content rows, best fuzzy score first.

## Decisions
- U3 is merged to `main` (#10) with `render_band`, `Match`, `open_search(SearchStart { folder, query, everything })`, `Scope::label`; the plan's "After U3" shape matches the code.
- U1 is merged to `main` (#8): `style::{highlight_text, truncate_middle_parts, map_range, active_selection, inactive_selection, cursor_ring}`, `chrome::{Keycap, with_tooltip, with_action_tooltip}`, `keys::{binding_for, text}` all exist. `highlight_text` already clamps ranges.
- Names outrank contents (user request, 2026-11-13): display all name-match rows above content-only rows; name rows keep walker order, content rows keep arrival order. Insertion happens per batch (name rows before the content block) and `by_path` is rebuilt, so the walker hot path is untouched.
- "Name matches stand out more": name-match rows already carry a highlighted, semibold name and now sort first; content-only rows keep the excerpt under the name. No extra color hierarchy.
- `kind_name` stays for `style::kind`; only `Row.kind`/the Kind column go (I1).
- `FuzzyMatcher::indices` computes score and ranges in one nucleo call. Bench: branch 3944/3747/4430 ms, `main` 4416/4449/4327 ms on `target/fixture-search-200k`; first hit 3–10 ms both. No regression over 10 %.
- Esc with the skipped list open is handled in three places (field `Cancelled`, `CloseSearch`, window `CancelOperation`), so it works from the field, results and sidebar.
- Preview `stat_entry` + `fetch_at` run in one `background_spawn`; stale results are dropped by path, and a load only starts when the selected path changes.

## Hand checks
- [ ] Scope bar in both themes: selected segment, switch knob, hover/pressed → matches `../ui-overhaul/img/search-panel-b.png`
- [ ] Idle state: heading, four examples, key line → matches `../ui-overhaul/img/search-panel-d.png`
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
- 2026-11-13: read the plan, `search.rs`, `engine.rs`, `style.rs`, `chrome.rs`, `keys.rs`, `preview.rs`, `pane.rs`, `window.rs` search code and tests. U3/U1 merged; branch created from `main`.
- 2026-11-13: asked Q1–Q13 in two forms; all answered (see Questions).
- 2026-11-13: implemented I1–I9, the three U5-review requests and Esc-first. `cargo fmt --check`, clippy (`-D warnings`) and `cargo test --workspace` green (190 app tests, 57 core, 27 search, 11 fs, 4 macos, 6 theme).
- 2026-11-13: README and `docs/testing.md` updated (scope bar, footer, idle, preview, This Mac, Cmd-Shift-M, hand rows).
- 2026-11-13: ran the app (`target/release/crow target/u6-ui`) and checked the real surface: idle examples and live key line, name-first results with the name match highlighted, content lines under names, footer counts, 300 px preview with the hit line tinted, folder column `"."` at the root, and preview hidden at an 850 px window. Rows needed `.w_full()` so columns line up with the header; fixed and re-checked.

## Next
PR #14 is open and CI is running. A human needs the unchecked hand checks (light theme, VoiceOver switch/footer, the switch click, footer hint with rules off) before the squash merge.
