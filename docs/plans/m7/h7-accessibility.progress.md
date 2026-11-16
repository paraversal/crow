# M7 H7 — Accessibility hardening progress
Branch: `m7-h7-a11y` · PR: none · Job: m7-h7-a11y-bf16 · Last updated: 2026-11-16 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 GPUI patch: disabled state | done (pending commit) | `patches/gpui-aria-disabled.patch`: `.aria_disabled(bool)` → `Node::set_disabled`/`clear_disabled`, `disabled` in the debug a11y JSON, builder unit test. Both ", unavailable" suffixes replaced (`window.rs` toolbar Back/Forward, `search.rs` Spotlight match segments). Also applied to the shared Settings controls (`controls::button`, `link_button`, `switch`, `segmented`, `tile_choice`, `dropdown`). Crow test `interactive_elements_have_roles_and_labels` asserts the Back button is `disabled: true`, label "Back". |
| I2 Fuzzer a11y invariant | done (pending commit) | `patches/gpui-a11y-interactive-registry.patch`: every painted `Div` with a click listener or focus handle is recorded (role, label, disabled, selector, bounds) under debug + test-support; `Window::debug_interactive_elements()` and `Window::debug_bounds(&str)`. `fuzz::a11y_violations` checks role + non-empty label; `CrowWindow::check_invariants` and the Settings check both call it. `fuzz_smoke` (3 seeds × 30) passes. |
| I2b Fuzz the Settings window | done (pending commit) | `fuzz::settings_state_for` (Settings keymap, control selectors clipped to the viewport, no `cmd-w`, Escape only with an open menu), `StepGen::next_settings` (keys/type/click/settle/clock), `SettingsWindow::{debug_selectors,text_focused,menu_open,check_invariants}`. `settings_run` + `fuzz_settings_smoke` (2 seeds × 30) checks focus-on-control, the a11y registry, error logs, and `Ui.settings` = `settings.toml` after a settle. Clicks resolve (menus open and entries are clicked) and the registry is non-empty. |
| I3 VoiceOver pass | todo | |
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
- [ ] I3 VoiceOver listening pass (areas listed in the plan).

## Found along the way
- (none yet)

## Log
- 2026-11-16 Read the plan, `AGENTS.md`, `docs/plans/m7/README.md`, the two "unavailable" sites (`window.rs:560`, `search.rs:1118`), GPUI's a11y builder and `accesskit_macos`'s disabled mapping; created the branch and this file.

## Next
Ask Q1–Q3, then start I1.
