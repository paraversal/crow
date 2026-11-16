# M7 H7 — Accessibility hardening progress
Branch: `m7-h7-a11y` · PR: none (critic opens it) · Job: m7-h7-a11y-bf16 · Last updated: 2026-11-16 · State: In critique

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 GPUI patch: disabled state | done (`d3f732b`) | `patches/gpui-aria-disabled.patch`: `.aria_disabled(bool)` → `Node::set_disabled`/`clear_disabled`, `disabled` in the debug a11y JSON, builder unit test. Both ", unavailable" suffixes replaced (`window.rs` toolbar Back/Forward, `search.rs` Spotlight match segments). Also applied to the shared Settings controls (`controls::button`, `link_button`, `switch`, `segmented`, `tile_choice`, `dropdown`). Crow test `interactive_elements_have_roles_and_labels` asserts the Back button is `disabled: true`, label "Back". |
| I2 Fuzzer a11y invariant | done (`d3f732b`) | `patches/gpui-a11y-interactive-registry.patch`: every painted `Div` with a click listener or focus handle is recorded (role, label, disabled, selector, bounds) under debug + test-support; `Window::debug_interactive_elements()` and `Window::debug_bounds(&str)`. `fuzz::a11y_violations` checks role + non-empty label; `CrowWindow::check_invariants` and the Settings check both call it. `fuzz_smoke` (3 seeds × 30) passes. |
| I2b Fuzz the Settings window | done (`d3f732b`) | `fuzz::settings_state_for` (Settings keymap, control selectors clipped to the viewport, no `cmd-w`, Escape only with an open menu), `StepGen::next_settings` (keys/type/click/settle/clock), `SettingsWindow::{debug_selectors,text_focused,menu_open,check_invariants}`. `settings_run` + `fuzz_settings_smoke` (2 seeds × 30) checks focus-on-control, the a11y registry, error logs, and `Ui.settings` = `settings.toml` after a settle. Clicks resolve (menus open and entries are clicked) and the registry is non-empty. |
| I3 VoiceOver pass | done (AX-tree pass; listening hand checks below) | Dumped the bundle's AX tree per area with a throwaway Swift script (`/tmp/axdump`). Fixed: Settings section headings had no label (now `aria_label(title)`); column view announced the active folder twice (the focusable `file-list` wrapper now `Role::Group` / "Columns" in column view). Found but not fixed: the initially focused Settings control is not marked `AXFocused` (Space proves GPUI focus is on Theme; the GPUI/AppKit activation path clears the adapter's focus — needs a GPUI patch, asked the user); list/sidebar/search/palette rows map to `AXStaticText` because AccessKit maps `ListBoxOption` that way (larger row-semantics change). Areas recorded below. |
| Finish | done | `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (421 + 93 + 44 + 5 + 20 + 6 pass, `fuzz_long` ignored). `rm -rf vendor/zed && ./scripts/fetch-gpui.sh` fetched `6ceb6fe` and applied all five patches; `cargo check -p crow-app --all-targets` and the a11y/fuzz tests pass after the clean fetch. CI runs on the PR the critic opens. |

## Questions
- Q1 (asked 2026-11-16): patch GPUI for `aria_disabled`, or keep the ", unavailable" label suffix? → **patch**. Checked before asking: `accesskit_macos-0.26.3/src/node.rs:634` maps `is_disabled` to `AXEnabled` false, so the macOS adapter does expose `Disabled`.
- Q2 (asked 2026-11-16): how does the invariant enumerate elements with `on_click`/focus handles? GPUI's test platform only records `debug_selector` bounds and the a11y tree (which drops role-less nodes); neither lists every clickable/focusable element. → **a second small GPUI patch** (debug + test-support only) recording every element with a click listener or focus handle at paint time; `Window::debug_interactive_elements()`.
- Q3 (asked 2026-11-16): I3 method — AX-tree dump of the bundle vs listening to VoiceOver? → **AX-tree dump** (throwaway Swift script), fix clear defects, list the listening checks as hand checks.
- Q4 (asked 2026-11-16, in the final report): the initially focused Settings control is not marked `AXFocused` until the user tabs (GPUI/AppKit activation clears the adapter's focus; Space proves GPUI focus is on Theme). Patch GPUI's `windowDidBecomeKey` to request a frame on first activation too, or leave it for a later piece? → waiting.

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
- 2026-11-16 I1–I2b committed as `d3f732b`; I3 fixes as `e5fd11f`. Clean GPUI fetch and full checks pass. The AX pass used a temp config (`CROW_CONFIG_DIR`) and a fixture with a 200-file folder; the bundle's AX tree was dumped per area, and the two label defects were fixed and re-checked in the rebuilt bundle.

## Next
Critique: review the branch, answer Q4 (GPUI focus patch or later), open the PR.

## Critique
Job: review-m7-h7-a11y-4f51 · Critic session: 2026-11-16 · Base: `main` @ `9282594` · Head reviewed: `b4dfe94` + `cb563d7`

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | `crow --fuzz` no longer compiled: `a11y_violations` and `settings_state_for` called GPUI's test-support-only `debug_interactive_elements`/`debug_bounds` in every `--features fuzz` build. CI's `--all-targets` clippy unified test-support and hid it; `cargo check -p crow-app --features fuzz --bin crow` failed with E0599. | `fuzz.rs:396,414` | code | fix | fixed (`cb563d7`): the check is test-only and empty in the live fuzzer, as its doc said; CI now checks `--bin crow` without `--all-targets` |
| C2 | `settings_run` compared `Ui.settings` to `settings.toml` immediately after the last step. Writes are spawned (`ui::write_config`), so a pending write could flake; every other step settles first. | `window/tests.rs` `settings_run` | code | fix | fixed (`cb563d7`): the final check runs through `Step::Settle` |
| C3 | `docs/testing.md` still said the fuzzer does not drive the Settings window and did not list `fuzz_settings_smoke`. | `docs/testing.md:83,147` | docs | fix | fixed (`cb563d7`) |
| C4 | The Finish row said the clean fetch applied "all four patches"; the tree has five (three on `main` + the two new ones). | progress Finish | record | fix | fixed here |
| C5 | The Status table said "done (pending commit)" for I1–I2b after `d3f732b`. | progress Status | record | fix | fixed here |
| C6 | Initial Settings focus is not announced: the bundle's AX tree marks the window `AXFocused` but no control; Space opens Theme's menu, so GPUI focus is there. | `settings_window.rs` focus/activation; AX dump | UX / a11y | ask | Q4 still open; recommendation: later piece (a third GPUI patch) |
| C7 | The `aria_disabled` patch's unit test (`test_aria_disabled_writes_the_node_flag`) never runs in CI: `vendor/zed` is excluded from the workspace. | `patches/gpui-aria-disabled.patch` | code sense | note | left: the Crow registry test and the AX hand check cover the behavior |
| C8 | PR #39 (H5) CI failed on `palette_count_says_places`: `folder_listing` scores the full path label, and the runner's random temp root made "alpha" a subsequence of the beta path. Pre-existing on `main`; not H5's or H7's code. | `palette.rs:788`; run `43877870113` | flake | ask | Q2: fix on H5's PR (deterministic fixture base) or rerun |
| C9 | `fuzz_settings_smoke` uses 2 seeds where H1's smoke uses 3; the test takes 1.2 s. | `window/tests.rs` | code sense | note | left: the budget comment is honest; a third seed is cheap if wanted |

Checks re-run: `cargo fmt --check` ✓ · `cargo clippy --workspace --all-targets -- -D warnings` ✓ · `cargo clippy -p crow-app --features fuzz --all-targets -- -D warnings` ✓ · `cargo check -p crow-app --features fuzz --bin crow` ✓ (E0599 before C1) · `cargo test --workspace` ✓ (421 app + 93 core + 44 fs + 5 macOS + 20 search + 6 theme; `fuzz_long` ignored) · `fuzz_settings_smoke` ✓ 1.19 s · both new patches pass `git apply --reverse --check` against the fetched `6ceb6fe` · bundle AX dump ✓.

Looked at: `target/bundle/Crow.app` with a temp config and a 3-file fixture. AX tree: Back/Forward are `AXButton … enabled=0` (VoiceOver reads "dimmed"), column view is `AXGroup "Columns"` with one `AXList` per column, Settings headings are labelled and the controls read their values; no control has `AXFocused` on first open (C6). The a11y invariant was also run across base, palette, filter, search, why, theme picker, activity, rename, context menu and Settings with a throwaway test: no violations. Could not listen to VoiceOver itself; the AX flags are what it reads. Light/dark and 640 px were not re-checked: the diff changes roles and labels only, no styles.

Verdict: better. Disabled controls now expose AccessKit's `Disabled` instead of a label suffix, the fuzzer refuses unlabelled click/focus elements, and the Settings window is a driven surface. The `--features fuzz` build regression is fixed; the initial-focus defect remains open as Q4.

## Next
Q4 (GPUI focus patch now or later) and Q2 (the H5 CI flake) are with the user. Then push and open the PR.
