# F4 fix progress

Branch: `m8-f4-path-bar` (F4, unmerged) · PR: none · Last updated: 2026-11-17 · State: Ready for test

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 The chosen shape (V2) | done | 38 px bar; 32 px hover pill around the path and pencil; pencil at the path's end; empty-space and pill-padding clicks edit; the fade/expand deferred |
| 2 Edit field | done | 32 px, 8 px radius, 12 px padding; spans the left section (path + hints) up to the counts' divider; text vertically centered |
| 3 Docs, bench, hand checks | done | README and `docs/testing.md`; bench 156 / 11 / 8.63 ms vs main 143 / 11 / 8.64; `target/bundle/Crow.app` built; the user's hand test pending |

## Decisions

- **V2** (user, 2026-11-17): the hover-revealed pill; 38 px bar, 32 px pill, 32 px field across the left section; 12 px padding; the pencil always at the path's end; the empty-space click edits.
- **Fade/expand deferred:** the pill appears at once and the field replaces it. The motion can be added later with `with_animation`; the static states are what the user tests now.
- The F4 drag work (segments, folded "…", the field having no drag) is unchanged.

## Hand checks

- [ ] Hover the path: the pill appears around the path and pencil; the pencil brightens; the empty space stays background.
- [ ] Click the pill's padding, the empty space, the pencil: the field opens; it spans the path and the hints up to the counts' divider; 12 px padding; the text is vertically centered; all selected.
- [ ] ⌘L and a double-click on the empty space still edit; Tab/Return/Esc; a missing path's error.
- [ ] Click a segment: navigates; a drag does not navigate; drag a segment to Finder, a pane row and a sidebar favorite; the folded "…" clicks and drags its deepest folder.
- [ ] 640 px and wide; Firebox and Solarized Light; VoiceOver (pencil "Edit path", segments "Go to X", field "Path"); search mode (the selected result's path; the pencil edits it).
- [ ] While the field is open, dragging selects text and starts no file drag.

## Log

- 2026-11-17: built the V2 on `m8-f4-path-bar`: the 38 px bar, the 32 px hover pill, the pencil at the path's end, the 32 px field across the left section with the hints inside, the sibling separators. The fade/expand is deferred.
- 2026-11-17: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` pass (500 app + 1 ignored, 99 core, 44 search, 6 fs, 20 macos, 6 theme).
- 2026-11-17: bench on `target/fixture-100k`: cold 156 ms, warm 11 ms, p99 8.63 ms (main 143 / 11 / 8.64; the F4 branch 153 / 11 / 8.59). Within the budgets and run-to-run noise.
- 2026-11-17: `./scripts/bundle.sh` built `target/bundle/Crow.app`.

## Next

The user tests `target/bundle/Crow.app` against the hand checks above. After the test: the critique/PR, or the fade/expand if wanted.
