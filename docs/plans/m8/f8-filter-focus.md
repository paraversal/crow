# F8 — One focus in the `/` filter

**Target:** with the `/` filter open, exactly one surface owns focus. The field owns the keys while it is focused; ↓ (or Return) moves focus to the matches; ↑ on the first row moves focus back to the field. The list's selection is drawn inactive while the field is focused, so the field's caret and the result cursor never look focused at the same time.

**Current state.** `Pane::render_filter_field` binds ↑/↓ to `move_up`/`move_down` (`crates/crow-app/src/pane/pane_filter.rs:63-64`), which select rows but leave focus in the field (`crates/crow-app/src/pane.rs:1199-1205`). The list draws its active selection fill whenever a filter is open (`pane.rs:569`, `:1944`, `:2363`, `:2659`, `:3104`), so the result cursor looks focused while the field's caret blinks. Enter already calls `focus_matches` (`pane.rs:1112`). Search has the model to mirror: `SearchFocusResults` / `SearchFocusField` (`crates/crow-app/src/actions.rs:222-223`, `crates/crow-app/src/search.rs:1072`).

**Change.**

- Add `FilterFocusResults` (↓ in the field) and `FilterFocusField` (↑ on the first filtered row) actions.
- Keymap: `down` in `Filter` → `FilterFocusResults`; remove `up`/`down` → `MoveUp`/`MoveDown` in `Filter`; `up` in `LIST_FILTERED` → `FilterFocusField` (added after the `LIST` bindings so it wins over `LIST`'s `MoveUp`). Vim `j`/`k` stay `MoveDown`/`MoveUp`; `k` at row 0 does not return to the field, as in search.
- `Pane::filter_focus_results`: no matches → nothing (no beep); otherwise `focus_matches` (select row 0 if no cursor, focus the list) and scroll the cursor row into view.
- `Pane::filter_focus_field`: cursor on row 0 → focus the filter input; otherwise move up.
- `render_filter_field`: handle `FilterFocusResults`; drop the `move_up`/`move_down` handlers.
- Active selection predicates: `self.focus.is_focused(window)` only (drop `|| self.filtering.is_some()`) in `selection_fills`, list rows, column rows, the column glide and the list glide.
- Keep Enter (focuses the matches), Escape (clears the filter, focuses the list), click-to-focus and the field's transparent ring when the list is focused.

**Tests.**

- Keymap (`crates/crow-app/src/actions.rs`): `down` in `Filter` → `FilterFocusResults`; `up` in `Filter` is not `MoveUp`; `up` in `ListFiltered` → `FilterFocusField`; `down` in `ListFiltered` is `MoveDown`; `up` in an unfiltered list is `MoveUp`.
- Window (`crates/crow-app/src/window/tests.rs`): `/`, type `a`, ↓ → list focused, cursor on the current match (alpha), `selection_fills` `(1, 0)`; ↓ again → beta; ↑ from alpha → field focused, typing edits the query; ↑ from beta → alpha, list stays focused; Enter in the field still focuses the list; no matches → ↓ does nothing and the field keeps focus; while the field is focused `selection_fills` is `(0, 1)`.
- Column view: the same ↓/↑ on the focused column.
- Update `filter_arrows_and_return_act_on_matching_rows` and `filter_enter_acts_on_the_highlighted_row`.

**Hand checks.** List and column view; Vim on and off; light and dark; 640 px and a wide window; field ring/caret vs list fill; no matches; empty folder.

**Questions.** None. The search field's existing ↓/↑ model is the precedent; the user asked for one focused element at a time.

**Done when.** The tests pass; `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass; the bundle is built and the hand checks above are done.
