# housekeeping-3 progress
Branch: `housekeeping-3` · PR: #36 · Last updated: 2026-11-16 · State: PR open

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 ⌘⌫ in text fields | done | `DeleteToStart` action + `cmd-backspace` binding in `text_input.rs`; search field clears, filter field clears, neither trashes. Tests: `cmd_backspace_clears_the_search_field`, `filter_field_takes_space_slash_and_list_keys_as_text`. |
| 2 Escape clears the selection | done | `Pane::clear_selection`; `CancelOperation` falls back to it only when no search is open and the list owns focus. Test: `escape_clears_the_selection_when_nothing_else_is_open` (plain list + filter precedence). |
| 3 Copy Path on empty space | done | `CopyPath` added to `FOLDER`; test extended in `copy_path_puts_the_selected_paths_on_the_clipboard`. |
| 4 Checks | done | fmt, clippy, workspace tests, bundle. Hand checks below. |

## Questions
None; the user approved the three behaviors on 2026-11-16.

## Decisions
- ⌘⌫ follows macOS single-line field behavior (delete to the start, or delete the selection), not “always clear the field” — plan, approved by the user's triage.
- Escape clears only the focused list's selection, and only after rename/filter/search/sheets/palette/theme picker have had their chance. It is scoped to list focus so Escape in the sidebar does not clear the list behind it — implementation detail, consistent with the plan.
- Copy Path goes in the empty-folder menu; the selected-items menu already has it — plan.

## Hand checks
- [ ] Search field: type a query, press ⌘⌫ → field is empty, results reset, no file is trashed.
- [ ] `/` filter: type a query, press ⌘⌫ → query is empty, the highlighted row stays selected, no file is trashed.
- [ ] List: select a row, press Escape → no highlight; press Escape again → nothing else changes.
- [ ] Filter open: select a row, press `/`, type, press Escape → filter closes, selection stays; Escape again clears it.
- [ ] Rename, search, palette, sheet and theme picker still consume Escape first.
- [ ] Empty list space: right-click → Copy Path → the shown folder's path is on the pasteboard.

## Found along the way
- Filter ↓ leaves focus in the field (user report, 2026-11-16): recorded as M7 `findings.md` row 19 for H4 batch 2; not fixed here.

## Log
- 2026-11-16: plan and deferred backlog recorded; branch created from `main`.
- 2026-11-16: implemented all three items; added the search and Escape tests and extended the filter and copy-path tests.
- 2026-11-16: `cargo fmt --check` ✓ · `cargo clippy --workspace --all-targets -- -D warnings` ✓ · `cargo test --workspace` ✓ (578 passed, 1 ignored) · `./scripts/bundle.sh` ✓. Hand checks still open.
- 2026-11-16: critique: fixed Escape with a pending Vim mark; checks re-run (579 passed, 1 ignored); PR #36 opened.

## Next
Hand-check the six items above in the PR build (`target/bundle/Crow.app`); the user merges after CI is green.

## Critique
Critic session: 2026-11-16 · Base: main @ 91a2e42 · Head reviewed: 913e01e

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | With the Vim layer on, Escape during a pending `m` mark cleared the selection but left “Mark this folder: type a letter” in the status bar. Esc should cancel the sequence first, as Vim does. | `crates/crow-app/src/window.rs:3110`, `pane.rs:739` | UX / code | fix | fixed (critique commit): `Pane::cancel_pending` runs before `clear_selection`; test `escape_cancels_a_pending_vim_mark_before_deselecting`; re-checked in the app with the Vim layer on. |
| C2 | Escape after a pending `g` chord still clears the selection. GPUI consumes the pending keystroke before dispatching the Escape action, so the app cannot tell a chord was waiting. | `crates/crow-app/src/actions.rs:276` (`g g`) | UX | note | left: needs a GPUI change (expose the cancelled chord to the action); Finder layer is unaffected. |
| C3 | `Pane::clear_selection` returns `bool` but the window handler ignores it. | `crates/crow-app/src/pane.rs:748` | code sense | note | left: the plan asked for the return value; harmless. |

Checks re-run: fmt ✓ · clippy ✓ · tests ✓ (579 passed, 1 ignored) · bench not needed: no listing, scrolling or first-paint change
Looked at in the app: search field ⌘⌫ (query cleared, results reset); `/` filter ⌘⌫ (query cleared, row stays selected); list Escape (selection and cursor gone, preview cleared); filter Escape twice (filter closes first, selection survives, next Escape clears); rename, search, palette, delete sheet and theme picker all consume Escape before the list; Escape in the sidebar leaves the list selection; column view Escape clears the column and closes the columns to its right; Vim layer `m` + Escape cancels the mark and keeps the selection. Empty-space context menu not driven (right-click automation blocked); its slot and action are covered by `copy_path_puts_the_selected_paths_on_the_clipboard` and the user asked to skip it.

Verdict: better. The three behaviors work as approved, the empty-space menu matches the row menu, and the only surprise found — Escape eating a pending Vim mark — is fixed.
