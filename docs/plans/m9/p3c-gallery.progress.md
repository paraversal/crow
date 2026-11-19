# M9 P3C — Gallery view progress

Branch: m9-p3c-gallery · PR: none · Last updated: 2026-11-19 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 Mode wiring | done | ViewMode::Gallery through views.toml, Settings default view, Why This View, View menu, palette, toolbar and Cmd-4 (Finder + Vim); Cmd-4 in sheets masked. Gallery reuses Folder's direct projection and shared selection; tests cover mode/filter/cursor round trip and Vim. |
| 2 Filename rail | in progress | Virtualized 48 pt rail, thumbnails, size/date tiers, cursor ring, drag/drop, context menu and row navigation are in place; folder sample rendering and remaining hand checks are next. |
| 3 Preview and folder contents | in progress | File previews reuse the Preview provider pipeline through `preview::load_with_plugins`; bounded `crow_fs::list_sample` added with tests. Folder cell sample UI is wired but needs verification. |
| 4 Comparison | in progress | Two selected entries render in rail order, independent second preview/surface and per-cell sample slots; 1→2→3→2→1 transitions not yet tested. |
| Verification and hand-off | todo | fmt, clippy, workspace tests, bundle, screenshots, bench, hand checks. |

## Questions
- none

## Decisions
- Branch `m9-p3c-gallery` from `main` @ 3990a45 after PR #74 merged. — plan sequencing (P3C after P3B), repo convention.
- Gallery's second comparison cell owns a separate plugin surface and preview slot; the cursor's cell reuses `Pane.preview`, so the right pane and one gallery cell share a load while the other cell's timeout cannot cancel it.
- `crow_fs::list_sample` is the bounded primitive: it stops the directory read at the visible-row or examined-entry cap and counts hidden entries before filtering.
- The rail is fixed at 48 pt in every width tier; only the metadata layout changes (aligned size/date when wide, one muted line when narrow), so resize cannot move a row.

## Found along the way
- `cargo test -p crow-app` can fail `shipped_fixture_command_runs_through_the_real_service` when run in parallel; it passes alone. Not touched by this branch.

## Log
- 2026-11-19 Branch `m9-p3c-gallery`; plan and P3B progress read; initial progress file.
- 2026-11-19 Mode wiring, toolbar/palette/menu/Settings, Cmd-4, Gallery rail/preview scaffolding, shared preview loader and bounded folder sample in place. App tests pass except one unrelated parallel flake; fmt and clippy clean.

## Next
Verify the folder-contents cell and sample states (empty, partial, unreadable, cloud), then exercise 1→2→3→2→1 preview transitions and stale replies.
