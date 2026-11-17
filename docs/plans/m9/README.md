# M9 — Phase 2: plugins, views, and operations (index)

Phase 1 is done (M6, the UI overhaul, M6.5, M7). M8 is the Finder-parity pass; M8.5 is the search-reliability pass between M8 and M9. M9 is Phase 2: the spec's "Later" table plus the infrastructure follow-ups that do not belong in a UI milestone.

This is a roadmap, not a set of plans. Each piece gets its own plan in this folder when it starts, with its own "Questions to ASK", branch, progress file and critique. Do not start a piece until the pieces it needs are merged to `main` (ASK before stacking on an unmerged branch).

Source: the spec's MVP scope / "Later" table (`docs/Crow — Product & Technical Spec.md:288-318`), the M7 index's "Not in M7", and the U11 follow-ups. Folder sizes is **not** here; it is M8 F6.

## The pieces

| Piece | What | Needs | Branch |
| --- | --- | --- | --- |
| P1 Public Rhai plugin API | The internal plugin host becomes a public, documented API: `plugin.toml` manifest, permissions, worker isolation, operation budget, the extension points the built-ins use | — | `m9-p1-plugin-api` |
| P2 Git plugin / Projects | The first public plugin: status letters, ignored-file dimming, a lines-changed column, branch/ahead-behind in the status bar, the Projects sidebar section. No stage/commit/push | P1 | `m9-p2-git-plugin` |
| P3 Icon packs and views | Data-only icon packs; icon and gallery views with the same selection, preview, drag and keyboard rules as list/column | P1 for packs; views can start after M8 F1 | `m9-p3-icons-views` |
| P4 Saved queries and Tags | Live searches as virtual folders; Finder tags in the sidebar and as a filter | M6 search; `crow-macos::tags` | `m9-p4-queries-tags` |
| P5 Copy queue | Pause, resume and a conflict dialog for long copies; one queue, visible in the bottom bar/activity | M4 operations; M8 F1 for row feedback | `m9-p5-copy-queue` |
| P6 Batch rename with preview | Rename many items with a preview, numbering, find/replace; undoable | M4 operations | `m9-p6-batch-rename` |
| P7 Drop in and spring-loaded folders | Accept drops from other apps everywhere; spring-open a folder after a hover while dragging | M8 F4 for path drag; existing drag plumbing | `m9-p7-drop-in` |
| P8 iCloud download status | Show not-downloaded/downloading/downloaded for iCloud items; never trigger a download just by browsing | `crow-macos` iCloud keys | `m9-p8-icloud` |
| P9 Scale and maintenance | Cross-run fuzz/test dashboard; test temp-dir hygiene; split `window.rs` (9.4k lines); move the palette view out of `window.rs` (~700 lines); the Windows Explorer path-bar setting from M8 F4.3; Solarized Light contrast (theme index, not Crow code) | — | one branch per item |

**P9 item: Explorer-style path bar (designed in M8 F4.3, built here).** `path_bar = "crow" | "explorer"` in `settings.toml`, default `"crow"`. Explorer mode: the breadcrumb is always visibly a control (segments are buttons with a hover fill); clicking the empty area turns the whole bar into an editable field with a clear border and a "›" affordance; the field selects the whole path on open; Escape returns to the breadcrumb. It is the same `Path` data as today, so no new path model. The setting is documented, live-reloaded, and added to the Settings window (a segmented control under Appearance or Behaviour). Tests when built: both modes render; the setting writes only its line; Explorer mode's click-to-edit and Escape; the path still navigates.

P9's test temp-dir item is the 2026-11-17 finding: a full test run leaves one `crow-*` folder per test in the system temp folder (`ui::init_for_tests` alone made 69,708 `crow-app-config-*` folders; the whole set was 124,961 folders and 5.8 GB). The same helpers land in `/tmp` when a runner unsets `TMPDIR`, which is where the user first saw them. The fix is a per-test root that is removed when the test process ends, plus a check that a clean run leaves none behind. It is small and independent; it can start before the rest of P9.

**Suggested order:** P1 → P2 → P3 → P4 → P5 → P6 → P7 → P8; P9 in parallel, one item at a time. P1 is first because P2 and P3 need it. P5/P6/P7 all touch `crow_fs::ops` and `window.rs`; never two writers at once. P9's `window.rs` split should wait until the M8 pieces that touch `window.rs` are merged, or be done first in a quiet window.

**M9 is done when:** every piece is merged and hand-checked; the spec's "Later" table has no open row except the "Not planned" ones; README's status line says Phase 2 is done (or names the next milestone); the fuzz dashboard has a recorded baseline; a clean `cargo test --workspace` leaves no `crow-*` folders in the system temp folder.

## Questions to ASK

Index-level questions for the user, asked by whoever starts P1:

1. **Which Phase 2 feature first?** Recommended P1 (plugin API) then P2 (git plugin), as the spec says the git plugin is the first public plugin. P3/P4 can run in parallel after P1 if there are two writers.
2. **One M9 or split it?** Recommended: keep one roadmap now; split into M9 (plugins/views) and M10 (operations) when P5 starts, if the pieces are still large.
3. **Plugin API language:** Rhai per the spec, or another embedded language? Recommended Rhai; the spec's manifest and permission model assume it.
4. **Icon/gallery views:** both, or icon first? Recommended icon view first, gallery second; both reuse the list's selection/preview rules.
5. **Copy queue UI:** a Finder-like queue in the activity list, or a separate window? Recommended the activity list, with pause/resume and a conflict sheet.
6. **Batch rename rules:** numbering, find/replace, regex, date tokens? Recommended numbering + find/replace + a preview; regex only if asked.
7. **iCloud status:** show it in the list's Kind/status cell and the info pane? Recommended both, read-only, never download.
8. **P9 order:** test temp-dir hygiene first (small, stops the 5.8 GB of litter), then the fuzz dashboard (it feeds every later piece), then the `window.rs` split, then the palette move, then the path-bar setting. Recommended.

Each piece gets its own plan with these answered where they apply.

## Not in M9

- Remote file systems (SFTP, S3), desktop icons, Open/Save panels, free icon placement, Mac App Store: the spec's "Not planned" list.
- Staging, committing, pushing: stays with the `git` CLI and tools like lazygit.
- Folder sizes: M8 F6.
- Search reliability: M8.5.
- Rubber-band selection: M8 index Q2; if deferred, it lands in P9 or a small M8 fix list.
- The Windows Explorer-style path bar: designed in M8 F4.3, built in P9.
