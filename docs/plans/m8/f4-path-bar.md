# F4 — Path bar

Audience: an implementing agent. Follow the `execute-plan` skill; read `AGENTS.md` and the M8 index first.
Base: branch `m8-f4-path-bar` from `main`. Progress file: `docs/plans/m8/f4-path-bar.progress.md`.
PR title: "M8 F4: draggable path segments, discoverable editing".

## Why

The bottom path bar is a breadcrumb of segments, and it is editable: ⌘L opens the path field, and a double-click on empty space does the same (`bottom_bar.rs:681`). Nothing on screen says so. The segments also cannot be dragged, so the bar is a dead end when you want the folder in another app. The user wants a Windows Explorer-style bar eventually; M8 makes the current bar discoverable and draggable, and records the Windows style for M9 (user, 2026-11-17).

## Current state

- **The bar's data.** `bottom_bar.rs:108` `Path { parts: Vec<(String, PathBuf)>, last_is_dir, spoken, hidden }`; `BarState.path` and `BarState.editing` (`:41-44`). `CrowWindow::bar_state` builds it (`window.rs:1141`).
- **Segments.** `bottom_bar.rs:600-680`: each part is a `div` with `Role::Button`, a Finder icon, the name, a `›` separator, a click that navigates or opens, and a tooltip. The folded `…` is `folded_segment` (`:552`). The empty space's double-click dispatches `EditPath` (`:681`).
- **Editing.** `window.rs:151` `path_edit: Option<PathEdit>`; `EditPath` opens the field (`:595`), `confirm_path_edit` (`:623`), `complete_path_edit` (`:679`), `close_path_edit` (`:603`). `BarFocus::PathEdit` (`:1032`) maps to `KeyScope::PathBar` (`:1123`).
- **No drag payload on segments.** Rows use `DraggedSelection` and `external_drag_payload` (`pane.rs:1987`, `:2315`); the path bar has no `on_drag`/`external_drag_payload`.
- **The Windows-style option is not built.** The user chose: design it now, build it as an M9 setting (2026-11-17).

## Items

### 1 Drag folders from the path bar (ui)

**Change:** make every path segment draggable as a folder URL. Dragging a segment out to Finder or another app must drag that folder (not the text, not the whole selection). Dragging within Crow can drop onto a pane/sidebar row like a folder drag; if that is not wanted, restrict it to external drags and record why. Do not start a drag while the path field is editing.

**Implementation:** reuse the existing drag plumbing. A segment can use `DraggedSelection` with one entry `(path, true)` and an `external_drag_payload` that writes a file URL; or add a small `DraggedFolder(PathBuf)` type if a one-folder payload is cleaner. Keep the click behavior: a click still navigates; a drag does not also navigate. The folded `…` segment should drag the deepest folder it stands for (the same target its click uses), or be non-draggable — ASK.

**Tests:** a GPUI drag test starts a drag on a segment and asserts the payload path; a click still navigates; a drag does not navigate; the path field has no drag; the folded segment behaves as chosen.

### 2 Discoverable edit affordance (ui)

**Change:** make it obvious that the bar can be edited without adding permanent chrome. Recommended: on hover over the path area, show a small pencil glyph at the right edge (or a subtle `surface-raised` background on the empty space) with the tooltip "Edit path (⌘L)"; clicking the empty space once opens the field (currently only a double-click does), and ⌘L stays. Do not change the bar's height or the segments' positions; the affordance is an overlay or a hover-only child.

**Tests:** a hover test shows the affordance; a single click on empty space dispatches `EditPath`; a click on a segment still navigates (does not edit); ⌘L still works; the bar's layout is unchanged at 640 px and wide.

**ASK:** pencil glyph vs a hover-only background vs both. Recommended: a pencil on hover plus the existing double-click; a single click on empty space is the Windows-like behavior and is worth adding.

### 3 Windows Explorer-style option (d)

**Design now, build in M9.** Record the shape in this plan and the M9 index:
- `path_bar = "crow" | "explorer"` in `settings.toml`, default `"crow"`.
- Explorer mode: the breadcrumb is always visibly a control (segments are buttons with a hover fill), clicking the empty area turns the whole bar into an editable field with a clear border and a "›" affordance; the field selects the whole path on open; Escape returns to the breadcrumb. It is the same data as today, so no new path model.
- The setting is documented, live-reloaded, and added to the Settings window (a segmented control under Appearance or Behaviour).
- M8 only records this; do not build it here. The user chose "M9 setting" on 2026-11-17.

**Tests (when built):** both modes render; the setting writes only its line; the Explorer mode's click-to-edit and Escape; the path still navigates.

## Questions to ASK

1. **Folded `…` segment drag:** drag the deepest folder it stands for (recommended, matches its click) or make it non-draggable?
2. **Edit affordance:** pencil on hover (recommended), hover-only background, or both?
3. **Single click on empty space edits the path?** Recommended: yes, Windows-like; keep the double-click and ⌘L.
4. **Internal drops from a path segment:** allow dropping the folder onto a pane/sidebar row (recommended: yes, it is the same as dragging the folder) or external only?

## Finish

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- Bench not needed unless the bar's render path changes; if it does, compare with `main`.
- `./scripts/bundle.sh`; hand-check dragging a segment to Finder, a click, a single-click edit, ⌘L, the folded segment, 640 px and wide, light/dark, VoiceOver for the new affordance.
- Update README keys and `docs/testing.md`.
- Commit, push, "Ready for critique". Don't merge.
