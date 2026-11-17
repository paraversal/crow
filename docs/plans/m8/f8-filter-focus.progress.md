# F8 — One focus in the filter progress

Branch: `m8-filter-focus` · PR: none · Last updated: 2026-11-17 · State: In critique

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 One focus in the `/` filter | done | Keymap: `down` in `Filter` → `FilterFocusResults`, `up` in `ListFiltered` → `FilterFocusField`; the field's ↑ is unbound. `Pane::filter_focus_results` / `filter_focus_field`; the list's active fill now follows `self.focus` only. Tests: keymap + seven window tests (↓ enters on the current match, second ↓ moves, ↑ returns from row 0, ↑ moves on later rows, inactive fill while the field is focused, no matches stays in the field, Vim `k` at row 0 stays in the list); column view covered. fmt, clippy and workspace tests pass; bundle hand checks done. |

## Questions
- None. The search field's ↓/↑ model is the precedent; the user asked for one focused element at a time.

## Decisions
- ↓ enters the matches on the current cursor row (first match if none), like the search field's `SearchFocusResults`; a second ↓ moves down. — user's request + search precedent.
- ↑ in the field does nothing; ↑ on the first row returns to the field. — search precedent.
- Enter in the field keeps focusing the matches (current behavior); it is not changed to open the selection. — smallest change; the user asked about arrows.
- The list's selection draws inactive while the field is focused. — one focus must look like one focus; the field ring already goes transparent when the list is focused.
- Vim `k` at the first filtered row does not return to the field; only the arrow does, as in search. — search precedent.

## Hand checks
- [x] `/`, type `a`, ↓: the list has focus and alpha is selected; the field has no ring or caret. ↓ again moves to beta. ↑ from alpha returns to the field; typing appends to the query. (bundle, Gruvbox dark, list view; screenshots `/tmp/crow-f8-filter.png`, `-down1`, `-down2`, `-up1`, `-up2`.)
- [x] Column view: the same on the focused column; the second column follows alpha. (bundle, 1280 px; screenshot `/tmp/crow-f8-columns.png`.)
- [x] 640 px: the column view with the filter still lays out; the list owns the focus and the field ring is gone. (screenshot `/tmp/crow-f8-640.png`.)
- [x] No matches: `zzz`, ↓ does nothing and the field keeps focus. (screenshot `/tmp/crow-f8-nomatch.png`.)
- [x] Vim on and off; light and dark; an empty folder. Critic: Vim on with Solarized Light (`target/shots/f8-down-solarized-solarized-light.png`), Finder keys with FireBox (`target/shots/f8-down-firebox.png`); the field's ring is gone and the cursor row uses the accent fill in both. 640 px with FireBox (`/tmp/f8-640-critic.png`). Empty folder `/tmp/crow-empty-f8`: `/`, `a`, ↓ leaves the field focused, "0 of 0" (`target/shots/f8-empty-firebox.png`).

## Found along the way
- None yet.

## Log
- 2026-11-17: plan and progress written; branch `m8-filter-focus` from `main` (`42ba5f4`).
- 2026-11-17: implemented. `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass. `cargo test -p crow-app filter` 41/41. Bundle hand check next.
- 2026-11-17: added the Vim `k` pin (keymap + `filter_vim_k_at_the_first_row_stays_in_the_list`); filter suite 35/35, clippy and workspace tests pass. Committed locally; state is Ready for critique.

## Critique
Critic session: 2026-11-17 · Base: `main` @ `42ba5f4` · Head reviewed: `f2852ca`

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | The keymap test asserts ↑ in the field is not `MoveUp`, but not that it is unbound; a future binding could move the selection behind the field. The predicate `LIST_FILTERED = "FileList && Filtered && !Filter"` keeps it unbound today, and the row-0 window test covers the common path. | `crates/crow-app/src/actions.rs:361-370` | code / tests | note | left: the code is correct; the existing window test covers the field's ↑ at row 0. |
| C2 | `filter_focus_field` with no cursor falls through to `move_up`, which selects the last row (`Folder::step(-1)` from `None`). The list can be focused with no cursor after a click on empty space. | `crates/crow-app/src/pane.rs:1144-1152`, `crates/crow-app/src/folder.rs:1901` | code / UX | note | left: "otherwise move up" is the plan's wording, and selecting the last row from nothing matches the list's existing ↑ behavior. |

Checks re-run: `cargo fmt --check` ✓ · `cargo clippy --workspace --all-targets -- -D warnings` ✓ · `cargo test --workspace` ✓ (486 crow-app, 96 crow-core, 44 crow-fs, 6 crow-macos, 20 crow-search, 6 crow-theme; 1 ignored) · bench not needed: no row rendering or first-paint change.
Looked at: list and column view; FireBox dark and Solarized Light; Vim on and off; 640 px; no matches and an empty folder; field ring vs cursor fill. VoiceOver: no new role or label in this change.
Verdict: better. One surface owns the keys and the fill follows it; the search field's model is now consistent in the filter, with no layout change.

## Next
Ready for critique. The critic runs the `critique` skill, checks the hand checks above (especially Vim on/off and light/dark), and opens the PR.
