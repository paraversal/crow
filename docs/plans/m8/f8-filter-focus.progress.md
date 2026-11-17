# F8 — One focus in the filter progress

Branch: `m8-filter-focus` · PR: none · Last updated: 2026-11-17 · State: Ready for critique

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 One focus in the `/` filter | done | Keymap: `down` in `Filter` → `FilterFocusResults`, `up` in `ListFiltered` → `FilterFocusField`; the field's ↑ is unbound. `Pane::filter_focus_results` / `filter_focus_field`; the list's active fill now follows `self.focus` only. Tests: keymap + six window tests (↓ enters on the current match, second ↓ moves, ↑ returns from row 0, ↑ moves on later rows, inactive fill while the field is focused, no matches stays in the field); column view covered. fmt, clippy and workspace tests pass; bundle hand checks done. |

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
- [ ] Vim on and off; light and dark; an empty folder. The Vim keymap is covered by `filter_field_context_disables_list_keys` and the existing Vim filter tests; the change uses the existing active/inactive selection roles, so light/dark is a visual check for the critic. The empty folder uses the same `row_count() == 0` path as no matches.

## Found along the way
- None yet.

## Log
- 2026-11-17: plan and progress written; branch `m8-filter-focus` from `main` (`42ba5f4`).
- 2026-11-17: implemented. `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass. `cargo test -p crow-app filter` 41/41. Bundle hand check next.

## Next
Ready for critique. The critic runs the `critique` skill, checks the hand checks above (especially Vim on/off and light/dark), and opens the PR.
