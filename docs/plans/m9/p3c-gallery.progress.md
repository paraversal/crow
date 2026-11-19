# M9 P3C — Gallery view progress

Branch: m9-p3c-gallery · PR: none · Last updated: 2026-11-19 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 Mode wiring | todo | Gallery in views/settings/menu/palette/toolbar, Cmd-4, shared projection/selection, keep filter/cursor/selection across List/Icons/Gallery. |
| 2 Filename rail | todo | Spike row bounds, trackpad scroll, resize anchoring, right pane; virtualized rail with nav and actions. |
| 3 Preview and folder contents | todo | Reuse Preview tab providers; bounded folder sample; stale-result rejection. |
| 4 Comparison | todo | Two-cell compare with independent state; 1→2→3→2→1 switching. |
| Verification and hand-off | todo | fmt, clippy, workspace tests, bundle, screenshots, bench, hand checks. |

## Questions
- none

## Decisions
- Branch `m9-p3c-gallery` from `main` @ 3990a45 after PR #74 merged. — plan sequencing (P3C after P3B), repo convention.

## Hand checks
- [ ] Rail by mouse and keyboard, resize with Info open and closed, drag to a folder, two-cell layout light/dark at 640 pt and wide.
- [ ] VoiceOver: row name, size, modified date, selection, each preview label.
- [ ] Bench List on `main` and branch; Gallery on branch with 100k and mixed-media fixtures. Record first paint, scroll, mounted-row bounds, queued thumbnails, folder sampling, outstanding previews.

## Found along the way
- none

## Log
- 2026-11-19 Branch `m9-p3c-gallery`; plan and P3B progress read; initial progress file.

## Next
Read the Gallery-relevant code: view modes and settings, list/icons row construction, selection, preview pane and providers, keymap/menu/palette wiring.
