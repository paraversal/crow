# F4 fix 2 progress

Branch: `m8-f4-path-bar` (F4, unmerged) · PR: none · Last updated: 2026-11-17 · State: in progress

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 Hover clears on window exit | done | GPUI patch + test; test passes; patch re-applies in `fetch-gpui.sh` |
| 2 The pill fades, the field expands | done | pill/pencil fade 120 ms; field expands 160 ms; motion Off instant |
| 3 The "Tab complete" hint | planned | Q3: remove vs implement |

## Questions (user)

- **Q1:** the hover fix — patch GPUI's `MouseExited` to move the mouse position out and clear the hit test (all hovers; a vendored patch) or render the pill from a local `on_hover` bool (the pill only)?
- **Q2:** the animations — 120 ms pill fade, 160 ms field expand, `ease_out`, skipped with motion Off?
- **Q3:** the "Tab complete" hint — the completion exists; the field opens with the full path selected. The user chose the inline completion preview (the Finder ⌘⇧G pattern).

## Answers (user, 2026-11-17)

- **Q1:** A, the GPUI patch.
- **Q2:** A, 120/160 ms, `ease_out`, instant with motion Off.
- **Q3:** C, the inline completion preview: the muted suffix, Tab applies, no suffix when nothing completes, the hint stays.

## Decisions

- The GPUI test runs from a throwaway workspace at `/tmp/gpui-ws` (the vendored Zed workspace lists hundreds of unchecked-out members): the 22 GPUI crates + `util_macros`, `reqwest_client`, `http_client_tls`, `tooling/perf`, `assets/fonts`; the `gpui_platform` dev-dep gets `runtime_shaders` (no Metal tool on this machine); `default-members` and `[patch.crates-io]` dropped. Command: `cargo test -p gpui --lib mouse_exit` from `/tmp/gpui-ws`. The plan's `cargo test -p gpui` is the same test, adapted to the sparse vendor.
- The patch is `patches/gpui-mouse-exit-hover.patch`; `scripts/fetch-gpui.sh` re-applied every patch in glob order without conflict.

## Log

- 2026-11-17: the user tested the V2: the hover sticks when the pointer leaves the window; no animations; the "Tab complete" hint looks dead. Plan written; awaiting approval.
- 2026-11-17: corrected the plan: the completion exists (`text_input.rs:96` → `complete_path_edit`); the field opens with the full path selected, so Tab returns the same text. The hover patch must move `mouse_position` out, not just clear the hit test.
- 2026-11-17: the user answered Q1 A, Q2 A, Q3 C (the inline completion preview); the plan amended; awaiting the OK to build.
- 2026-11-17: item 3 spells out the case-insensitive suffix (`~/des` → `~/Desktop/`, no `strip_prefix`) and the caret-at-end gate, with tests.
- 2026-11-17: item 1: `MouseExited` now moves `mouse_position` out and clears `mouse_hit_test` after the exit dispatch; `window::tests::mouse_exit_clears_the_hover_hit_test` passes (the test div needs `.hover(|s| s)` to get a hitbox); patch regenerated and verified through `fetch-gpui.sh`; fmt, clippy, workspace tests pass.
- 2026-11-17: item 2: `PathHover`/`PathUnhover` carry the bar's hover into `CrowWindow`; the pill and pencil fade over 120 ms `ease_out`; the field expands over 160 ms via `max_w` from 0 to the full width (not the pill's width — the plan's wording; the user's test decides); motion Off skips all three.

## Next

Item 3: the inline completion preview.
