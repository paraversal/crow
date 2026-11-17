# F7 — Housekeeping 4

Audience: an implementing agent. Follow the `execute-plan` skill; read `AGENTS.md` and the M8 index first.
Base: branch `m8-f7-housekeeping` from `main`. Progress file: `docs/plans/m8/f7-housekeeping-4.progress.md`.
PR title: "M8 F7: Open Terminal Here, new-item placement, window drag, board details".

## Why

The D3 inbox and the U11 follow-ups left a handful of small, independent gaps: no Open Terminal Here, New File/Folder ignores a selected folder, dragging on toolbar controls moves the window, a few board-only details were deferred, and the NSSound volume hand check is still open. None is big enough for its own milestone piece; together they are one fix list.

## Current state

- **Open Terminal.** `Effects` (`crates/crow-app/src/effects.rs:17`) has `open_path`, `open_text`, `open_config`, `open_in_default_app`, `open_folder_in_finder`, `reveal_in_finder`; no terminal. The context menu (`context_menu.rs`) and `palette::COMMANDS` have no terminal command.
- **New File/Folder.** `Pane::new_folder` (`pane.rs:1336`) and `new_file` (`pane.rs:1340`) always use `self.current_dir(cx)` as the parent. The actions are `cmd-shift-n` / `cmd-alt-n` (`actions.rs:136-137`); the Items menu and palette use the same actions. `show_results` starts the rename (`pane.rs:728`).
- **Window drag.** `window.rs:66` builds `WindowOptions` with a transparent titlebar and traffic-light position, but does not set `app_owns_titlebar_drag`, so AppKit's native titlebar drag is active; dragging on toolbar controls moves the window (D3).
- **Board details** (U11 Q3, deferred): line icons per favorite and clock icons in Recent; a syntax-coloured text preview with line numbers; 196 px columns; a darker focused-column tint; a scope token inside the search field.
- **NSSound volume.** U10 R3: `NSSound` follows the output volume, not the macOS alert volume; the user kept NSSound and asked for a hand check with the alert slider low (`done/u10-feedback.progress.md:24`, `:40`). The check is still open.
- **Sidebar background menu** was dropped by the user; do not build it.

## Items

### 1 Open Terminal Here (f)

**Change:** add `Effects::open_terminal(path) -> bool` to the trait, `SystemEffects` and `RecordingEffects`. It opens Terminal at the folder: recommended Terminal.app via `NSWorkspace`/`open -a Terminal`, or iTerm if installed per Q1. Add a context-menu line for a folder (and for a file's parent folder?) and a palette command "Open Terminal Here"; a key is optional and needs the user's OK (every new binding is a user decision). Show a status-line error if it fails.

**Tests:** the recording effect records the path; the context menu item is enabled for a folder and disabled for a multi-selection; the palette command runs; a failure sets the status line.

**ASK:** Terminal.app (recommended: always present) vs iTerm when installed; whether a file selection opens its parent folder; whether it gets a key.

### 2 New File/Folder with a row selected (f)

**Change:** decide the parent. Recommended: always create in the **shown folder**, as today, even when a folder row is selected; Finder's New Folder creates in the shown folder. If the user wants "inside the selected folder", add that only with an explicit decision. Add both commands to the Items menu if they are missing; keep the rename-on-create behavior and the filter-clearing fix (findings #10).

**Tests:** with a folder selected, ⌘⇧N creates in the shown folder (recommended) and starts the rename; with a file selected, the same; the Items menu and palette run the same action; the new row is visible under an active filter.

**ASK:** shown folder (recommended) or selected folder.

### 3 Window drag (b)

**Change:** stop toolbar controls from moving the window. Set `WindowOptions::app_owns_titlebar_drag = true` and add an explicit drag strip (the band's empty area) that calls `window.start_window_drag()`, or the GPUI equivalent. Keep the traffic lights at (14, 19), double-click-to-zoom, and the titlebar's transparent look. Timebox: if this needs a `patches/gpui-*.patch` change, stop and ASK (AGENTS.md: workarounds that need a check-in include new patches).

**Tests:** a GPUI mouse test drags on the band's empty area and asserts the window drag starts; a drag on a toolbar button does not; the traffic lights and double-click still work in the bundle. Hand-test a wide and a 640 px window.

**ASK:** if the root fix needs a vendor patch, present the patch and its cost before building it.

### 4 Board-only details (ui)

Build the deferred U11 Q3 details, one at a time, each with a screenshot against the board:
- **Sidebar line icons** per favorite and **clock icons** in Recent (replace Finder icons only where the board shows a line icon; keep Finder icons elsewhere).
- **Syntax-coloured text preview with line numbers** (the preview already shows text; add a small highlighter and a gutter; do not add a dependency if the existing theme roles can carry it — ASK before a new crate).
- **196 px columns** (today 220–360 by longest name; the board shows 196; make it a constant or a view setting? ASK).
- **Darker focused-column tint** (today `surface` at 0x40; the board is darker; use a named `style` helper, not a raw color).
- **Scope token inside the search field** (the board's "in file-crow ⌄"; U3/U6 deliberately left it out; add only if the user wants it).

**Tests:** a screenshot or render-bounds check per item; light/dark; 640 px; no layout shift; bench if the preview or columns change the 100k path.

**ASK:** which of the five to build now; recommended: line icons and the focused-column tint first (small, visible), syntax preview and 196 px columns next, scope token last (it competes with the scope bar).

### 5 NSSound volume hand check (b)

**Change:** measure with the macOS alert slider low and the output volume normal. If Crow's sounds are louder than Finder's, report the measurement and ASK before changing the sound path (the root-cause option is `AudioServicesPlaySystemSoundWithCompletion` plus a trimmed cache copy, which cannot fade the trash tail). If they match, close the hand check and record it.

**Tests:** none; a hand check with the alert slider low, `sound = "crow"` and `"finder"`.

## Questions to ASK

1. **Terminal:** Terminal.app (recommended) or iTerm when installed? Does a file selection open its parent folder? Does it get a key?
2. **New File/Folder parent:** shown folder (recommended) or selected folder?
3. **Window drag:** if the root fix needs a GPUI patch, is that acceptable? Recommended: only with a measured need and a hand check of traffic lights/double-click.
4. **Board details:** which five to build now (recommended order above)?
5. **196 px columns:** a fixed constant or a setting? Recommended: fixed, matching the board; a setting adds a config key for a one-time design choice.

## Finish

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- Bench if the preview, columns or band change the 100k path; compare with `main`.
- `./scripts/bundle.sh`; hand-check each item in light/dark, 640 px and wide, VoiceOver for the new menu items, and the NSSound check.
- Update README, `docs/testing.md`, and the context-menu/palette lists.
- Commit, push, "Ready for critique". Don't merge.
