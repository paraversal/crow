# M9 P3C — Gallery view progress

Branch: m9-p3c-gallery · PR: none · Last updated: 2026-11-19 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 Mode wiring | done | ViewMode::Gallery through views.toml, Settings default view, Why This View, View menu, palette, toolbar and Cmd-4 (Finder + Vim); Cmd-4 in sheets masked. Gallery reuses Folder's direct projection and shared selection; tests cover mode/filter/cursor round trip and Vim. |
| 2 Filename rail | done | Virtualized 48 pt rail with thumbnails, aligned/narrow metadata tiers, cursor ring, cloud badge, tooltips, drag/drop, context menu, filter/rename and arrow/Home/End navigation. Tests cover mode round trips, keys, pair layout and mounted rows. |
| 3 Preview and folder contents | done | File previews share `preview::load_with_plugins` with the Preview pane. Bounded `crow_fs::list_sample` (12 shown/256 examined) with loaded-listing reuse, P8 gate, sort/hidden policy and distinct loading/empty/partial/error/cloud states; tests cover text previews and all folder states. |
| 4 Comparison | done | Two selected entries render in rail order with independent second preview/surface and per-cell samples; 1→2→3→2→1 and out-of-order stale replies are tested, including side-by-side and stacked layouts. |
| Verification and hand-off | in progress | fmt, clippy and the full workspace suite pass. Bench, screenshots and bundle checks are recorded below; VoiceOver and a real iCloud folder remain for the critic. |

## Questions
- none

## Decisions
- Branch `m9-p3c-gallery` from `main` @ 3990a45 after PR #74 merged. — plan sequencing (P3C after P3B), repo convention.
- Gallery's second comparison cell owns a separate plugin surface and preview slot; the cursor's cell reuses `Pane.preview`, so the right pane and one gallery cell share a load while the other cell's timeout cannot cancel it.
- `crow_fs::list_sample` is the bounded primitive: it stops the directory read at the visible-row or examined-entry cap and counts hidden entries before filtering.
- The rail is fixed at 48 pt in every width tier; only the metadata layout changes (aligned size/date when wide, one muted line when narrow), so resize cannot move a row.

## Hand checks
- [x] Rail by keyboard at 1200 pt: Up/Down, Left/Right aliases, Home/End, Shift extension and Cmd-click; List ↔ Icons ↔ Gallery preserve filter, cursor and selection (automated keystroke tests).
- [x] Wide window pair: two labeled image cells side by side, equal size, correct names/sizes (`/tmp/crow-gallery-pair.png`).
- [x] 640 pt: rail readable and the pair stacks vertically; toolbar fits with only the title ellipsized (`/tmp/crow-gallery-640.png`). With Info open at 640 pt the three-pane layout leaves the center preview at zero width; the rail stays readable (`/tmp/crow-gallery-640-info.png`). The center preview works with the right pane closed, as the plan says.
- [x] Folder cell at 1200 pt: heading, immediate child and footer; file code preview shows numbered lines (`/tmp/crow-gallery-window.png`, `/tmp/crow-gallery-file.png`).
- [x] Image preview fills the allotted cell at the available Quick Look resolution (`/tmp/crow-gallery-image.png`).
- [ ] VoiceOver: row name, size, modified date, selection, each preview label.
- [ ] A real iCloud-backed folder placeholder and a native drag of selected files onto a rail folder.
- [ ] Light theme and a search open/close pass in the bundle.
- [x] Bench on `main` @ 3990a45: list cold 131 ms, warm 5 ms, p99 8.55 ms.
- [x] Bench branch @ b02ec2a + working tree: list 100k cold 95 ms, warm 4 ms, p99 8.49 ms; gallery 100k cold 133 ms, warm 3 ms, p99 8.49 ms, max 47.11 ms; gallery 300 images cold 117 ms, warm 2 ms, p99 8.56 ms, max 27.25 ms.
- [x] Bounds: rail rows are uniform 48 pt (about 15 mounted at 710 pt); thumbnails are 20 pt with two Quick Look jobs, a 96-request queue and a 512-entry/48 MiB cache; folder samples read at most 12 rows and 256 dirents; at most two large previews are outstanding (cursor + comparison cell).

## Found along the way
- `cargo test -p crow-app` can fail `shipped_fixture_command_runs_through_the_real_service` when run in parallel; it passes alone. Not touched by this branch.
- At 640 pt with the right pane open, the center pane has no width left for the Gallery preview. The rail remains usable; this is the existing three-pane squeeze, not a separate layout break.

## Log
- 2026-11-19 Branch `m9-p3c-gallery`; plan and P3B progress read; initial progress file.
- 2026-11-19 Mode wiring, toolbar/palette/menu/Settings, Cmd-4, Gallery rail/preview scaffolding, shared preview loader and bounded folder sample in place.
- 2026-11-19 Gallery rail, folder states, comparison cells and stale-reply handling complete. Tests: 729 app + 137 core + 74 fs + 81 search and remaining crates pass, 1 ignored; fmt and clippy clean; bundle built and inspected; bench recorded.

## Next
Update the progress state to Ready for critique after a final bundle pass and commit; list the VoiceOver/iCloud/light-theme/drag hand checks for the critic.
