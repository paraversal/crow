# Housekeeping 3 — keyboard and empty-folder menu

Audience: an implementing agent. Follow `.claude/skills/execute-plan/SKILL.md`; read `AGENTS.md` first.
Base: branch `housekeeping-3` from `main`. Progress file: `docs/plans/housekeeping-3.progress.md`.
PR title: "Housekeeping 3: ⌘⌫ in fields, Escape deselects, Copy Path on empty space". Don't merge.

## Why
Three small user-reported gaps approved on 2026-11-16. They are independent of M7's stability work and small enough for one branch; everything else from the triage is in [`after-ui-overhaul.md`](after-ui-overhaul.md) D3.

## Items

### 1 ⌘⌫ deletes to the start of a text field
`crates/crow-app/src/text_input.rs` binds backspace, delete and ⌥-word deletes, but not ⌘⌫. In the search field (`Search` + `TextInput`) no other binding applies, so the key does nothing.

**Change:** add a `DeleteToStart` action, bind `cmd-backspace` in `text_input::keymap`, and delete from the caret to the start (or delete the selection). At offset 0 with no selection, ring the field's bell like backspace does. This is macOS's standard single-line field behavior; with the caret at the end of the search field it clears the whole query.

**Done when:** a GPUI keystroke test types a query in the search field, presses ⌘⌫, and reads an empty query; the file is not trashed. A second test covers the `/` filter field and asserts the file still exists.

### 2 Escape clears the list selection when nothing else is open
`CancelOperation` (`crates/crow-app/src/window.rs` ≈ 3095) stops a running operation, closes the skipped list, or closes search. In a plain list it does nothing, so an item stays highlighted.

**Change:** after those branches, when no search is open, clear the focused pane's selection. Add `Pane::clear_selection` (returns whether anything was selected) so the window handler stays small; it clears the focused folder's selection and reloads the preview. Overlays keep their own Escape handling: rename/filter fields, sheets, palette, theme picker, and search must still win.

**Done when:** a GPUI keystroke test selects a row and presses Escape; the cursor and selection are gone. A second sequence proves precedence: with the filter open, the first Escape closes the filter and keeps the selection, the second clears it. Existing rename/search Escape tests still pass.

### 3 Copy Path in the empty-folder menu
`MenuKind::Folder` already opens on empty list space, but its `FOLDER` slots in `crates/crow-app/src/context_menu.rs` lack `CopyPath`. `Pane::copy_path` already falls back to the shown folder when nothing is selected.

**Change:** add `Some(CommandId::CopyPath)` to `FOLDER` before Show in Finder.

**Done when:** right-clicking empty space and choosing Copy Path writes the shown folder's path to the pasteboard; the existing selected-paths test still passes.

## Finish
- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `./scripts/bundle.sh`; hand-check the three items in the app.
- Commit, push, "Ready for critique". Don't merge.

## Questions to ASK
None: the user approved the three behaviors on 2026-11-16.
