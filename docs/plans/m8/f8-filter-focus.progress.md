# F8 — One focus in the filter progress

Branch: `m8-filter-focus` · PR: none · Last updated: 2026-11-17 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 One focus in the `/` filter | in progress | Plan written; implementation starts now. |

## Questions
- None. The search field's ↓/↑ model is the precedent; the user asked for one focused element at a time.

## Decisions
- ↓ enters the matches on the current cursor row (first match if none), like the search field's `SearchFocusResults`; a second ↓ moves down. — user's request + search precedent.
- ↑ in the field does nothing; ↑ on the first row returns to the field. — search precedent.
- Enter in the field keeps focusing the matches (current behavior); it is not changed to open the selection. — smallest change; the user asked about arrows.
- The list's selection draws inactive while the field is focused. — one focus must look like one focus; the field ring already goes transparent when the list is focused.
- Vim `k` at the first filtered row does not return to the field; only the arrow does, as in search. — search precedent.

## Hand checks
- [ ] `/`, type `a`, ↓: the list has focus and alpha is selected; the field has no ring or caret. ↓ again moves to beta. ↑ from alpha returns to the field; typing appends to the query.
- [ ] Column view: the same on the focused column.
- [ ] Vim on and off; light and dark; 640 px and a wide window.
- [ ] No matches: ↓ does nothing, the field keeps focus. Empty folder: the field opens and ↓ does nothing.

## Found along the way
- None yet.

## Log
- 2026-11-17: plan and progress written; branch `m8-filter-focus` from `main` (`42ba5f4`).

## Next
Implement the keymap, `Pane` handlers, render predicates and tests.
