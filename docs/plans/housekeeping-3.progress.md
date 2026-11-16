# housekeeping-3 progress
Branch: `housekeeping-3` · PR: none · Last updated: 2026-11-16 · State: Ready for critique

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

## Next
Hand-check the six items above in `target/bundle/Crow.app`, then run the `critique` skill and open the PR.
