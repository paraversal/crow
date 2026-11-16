# M7 H7 — Accessibility hardening progress
Branch: `m7-h7-a11y` · PR: none · Job: m7-h7-a11y-bf16 · Last updated: 2026-11-16 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 GPUI patch: disabled state | done (pending commit) | `patches/gpui-aria-disabled.patch`: `.aria_disabled(bool)` → `Node::set_disabled`/`clear_disabled`, `disabled` in the debug a11y JSON, builder unit test. Both ", unavailable" suffixes replaced (`window.rs` toolbar Back/Forward, `search.rs` Spotlight match segments). Also applied to the shared Settings controls (`controls::button`, `link_button`, `switch`, `segmented`, `tile_choice`, `dropdown`). Crow test `interactive_elements_have_roles_and_labels` asserts the Back button is `disabled: true`, label "Back". |
| I2 Fuzzer a11y invariant | done (pending commit) | `patches/gpui-a11y-interactive-registry.patch`: every painted `Div` with a click listener or focus handle is recorded (role, label, disabled, selector, bounds) under debug + test-support; `Window::debug_interactive_elements()` and `Window::debug_bounds(&str)`. `fuzz::a11y_violations` checks role + non-empty label; `CrowWindow::check_invariants` and the Settings check both call it. `fuzz_smoke` (3 seeds × 30) passes. |
| I2b Fuzz the Settings window | done (pending commit) | `fuzz::settings_state_for` (Settings keymap, control selectors clipped to the viewport, no `cmd-w`, Escape only with an open menu), `StepGen::next_settings` (keys/type/click/settle/clock), `SettingsWindow::{debug_selectors,text_focused,menu_open,check_invariants}`. `settings_run` + `fuzz_settings_smoke` (2 seeds × 30) checks focus-on-control, the a11y registry, error logs, and `Ui.settings` = `settings.toml` after a settle. Clicks resolve (menus open and entries are clicked) and the registry is non-empty. |
| I3 VoiceOver pass | done (AX-tree pass; listening hand checks below) | Dumped the bundle's AX tree per area with a throwaway Swift script (`/tmp/axdump`). Fixed: Settings section headings had no label (now `aria_label(title)`); column view announced the active folder twice (the focusable `file-list` wrapper now `Role::Group` / "Columns" in column view). Found but not fixed: the initially focused Settings control is not marked `AXFocused` (Space proves GPUI focus is on Theme; the GPUI/AppKit activation path clears the adapter's focus — needs a GPUI patch, asked the user); list/sidebar/search/palette rows map to `AXStaticText` because AccessKit maps `ListBoxOption` that way (larger row-semantics change). Areas recorded below. |
| Finish | todo | |

## Questions
- Q1 (asked 2026-11-16): patch GPUI for `aria_disabled`, or keep the ", unavailable" label suffix? → **patch**. Checked before asking: `accesskit_macos-0.26.3/src/node.rs:634` maps `is_disabled` to `AXEnabled` false, so the macOS adapter does expose `Disabled`.
- Q2 (asked 2026-11-16): how does the invariant enumerate elements with `on_click`/focus handles? GPUI's test platform only records `debug_selector` bounds and the a11y tree (which drops role-less nodes); neither lists every clickable/focusable element. → **a second small GPUI patch** (debug + test-support only) recording every element with a click listener or focus handle at paint time; `Window::debug_interactive_elements()`.
- Q3 (asked 2026-11-16): I3 method — AX-tree dump of the bundle vs listening to VoiceOver? → **AX-tree dump** (throwaway Swift script), fix clear defects, list the listening checks as hand checks.

## Decisions
- Branch from `main` @ 9282594, as the plan names it; H1 is merged (`cf85599`). H5 is still an open PR and H6 is unmerged, so the branch has neither; the path bar is out of I3's scope (plan: "H6 if merged").
- Q1: `patches/gpui-aria-disabled.patch` holds the `aria_disabled` setter, `Node::set_disabled`, and `disabled` in the debug a11y JSON. The builder unit test lives in the patch (the test platform never activates a11y, so a Crow test can't read the tree); the Crow tests use the Q2 registry instead.
- Q2: `patches/gpui-a11y-interactive-registry.patch` records (role, label, disabled, selector, bounds) for every `Div` with a click listener or focus handle, in the same debug/test-support configuration as `debug_selector`. Custom elements other than `Div` are not enumerated; Crow's interactive elements are all `Div`s (text inputs render one).

## Hand checks
- [ ] I1 VoiceOver: a disabled toolbar Back/Forward button reads "dimmed".
- [ ] I3 VoiceOver listening pass (the AX tree is recorded; speech itself was not heard): sidebar, list, column view, filter, search, palette, rename, conflict sheet, context menu, tips, bottom bar, Settings. Use `scripts/bundle.sh --open`, VoiceOver on (Cmd-F5), and walk the same areas.

## I3 AX-tree record (bundle, temp config, fixture with 200-file folder)
- **Sidebar**: `AXList "Sidebar"`, headings Favorites/Recent/Locations, rows `AXStaticText` with names, current folder has `help="current folder"`, selected row `SELECTED`.
- **List**: `AXList "fixture, N items"`, header `AXCell` Name/Date Modified/Size/Kind, rows `AXStaticText "name, date, size, kind"`, selected/focused row marked; folder rows have an `Expand <name>` button.
- **Column view**: after the fix, the focusable wrapper is `AXGroup "Columns"`; each column is `AXList "<folder>, N items"`; rows and selection as in list view.
- **Filter**: `AXTextField "Filter fixture" value="no" FOCUSED`, group "Filter fixture", status "1 of 4 shown"; list label "fixture, 1 of 4 items".
- **Search**: `AXSearchField "Search fixture" FOCUSED`, Match group All/Names/Contents, Search scope group with three radios, `AXCheckBox "Skip dev folders"`, results `AXList "Search results, N items"`, status "N results" / "1 of N results selected".
- **Palette**: `AXDialog "Go to or Command Palette"`, `AXTextField "Go to folder" FOCUSED`, `AXList "Places"`, rows `"name, path, section"` with `SELECTED`.
- **Rename**: the row contains `AXTextField "Name" value="alpha" FOCUSED`.
- **Conflict sheet**: `AXDialog "An item named “notes.txt” already exists in “dest”"`, radios Keep Both (selected)/Replace/Skip, buttons Keep Both for All/Skip All/Cancel/Continue.
- **Context menu**: `AXMenu` with Open, Quick Look, Rename, Duplicate, Copy, Cut, Paste (disabled), Copy Path, Show in Finder, Add to Sidebar, Move to Trash; native separators are unlabeled disabled menu items.
- **Tips**: not reached in the fresh bundle (using `/`, ⌘F, ⌘K retires each tip before it can show); code has `Role::Status` + `aria_label("Tip: …")` and a labelled "Got it" button, and the tip tests cover the card.
- **Bottom bar**: `AXGroup "Status bar"`, `AXGroup "Path: …"` with a "Go to <segment>" button per segment, key-hint group, `AXApplicationStatus "N items in folder"`.
- **Settings**: headings now `Heading "Appearance"/"Behaviour"/"Motion & sound"/"New windows"/"Files"`; Theme/Font `AXPopUpButton` with values; Font size/Width/Height `AXIncrementor` with values; switches `AXCheckBox` with values; radios with selected state; file buttons labelled. Initial focus: GPUI focus is on Theme (Space opens the menu) but no node has `AXFocused` until the user tabs; see the finding below.
- **Path bar**: H6 is not merged, so it was not part of this pass (plan: "H6 if merged").

## Found along the way
- **Initial Settings focus is not announced** (GPUI/AppKit): opening Settings focuses Theme in GPUI (Space opens its menu), but the AX tree marks only the window `AXFocused`; after Tab/Shift-Tab the moved control is marked. The activation path (`windowDidBecomeKey` → `update_view_focus_state`) resets the adapter's focus; a Crow-side re-focus after `activate_window` did not change it and was reverted. A fix belongs in the GPUI macOS adapter (new `patches/gpui-*.patch`); asked the user.
- **Rows are `AXStaticText`**: AccessKit maps `Role::ListBoxOption` to `AXStaticText`, so list, sidebar, search and palette rows read as text with `AXSelected`/click actions, not as rows/cells. Changing to `Role::Row`/`Cell` is a larger semantics change; not done.
- **Disabled stepper fields**: when `settings.toml` is broken, the stepper wrapper dims and Tab skips the field, but the inner `SpinButton` has no `aria_disabled` (TextInput has no disabled flag). Not reached by Tab; noted, not fixed.

## Log
- 2026-11-16 Read the plan, `AGENTS.md`, `docs/plans/m7/README.md`, the two "unavailable" sites (`window.rs:560`, `search.rs:1118`), GPUI's a11y builder and `accesskit_macos`'s disabled mapping; created the branch and this file.

## Next
Ask Q1–Q3, then start I1.
