# Crow roadmap

Last updated: 2026-11-17. This page is the index; the plan and progress files under [`plans/`](plans/) are authoritative. Status values: **done**, **in progress**, **planned**, **deferred**.

## Right now

- **M8 F1 is done**: merged in [#48](https://github.com/paraversal/file-crow/pull/48), with the island-rule follow-up in [#49](https://github.com/paraversal/file-crow/pull/49) (commit `2964848`).
- **Next is M8 F2**. The M8 order is F2 → F5 → F4 → F3 → F6 → F7; F6 is independent and can start any time.
- **M8.5** (search reliability) starts after M8. **M9** (Phase 2) starts after M8.5.
- The [inbox](plans/inbox.md) has no untriaged items.

## At a glance

| Phase | Milestone | Status | Items |
| --- | --- | --- | --- |
| Phase 1 | M1–M4: foundation | done | 4 |
| Phase 1 | M5: sidebar, view settings, themes | done | 1 + 14 fixes |
| Phase 1 | M6: filter, command palette, search | done | 7 |
| Phase 1 | UI overhaul U1–U11 | done | 11 + 2 fix lists |
| Phase 1 | M6.5: settings window | done | 7 |
| Phase 1 | M7: hardening | done | 8 + 6 findings batches |
| Phase 1 | Housekeeping 1–3, context menu | done | 5 + 4 + 4 + 5 |
| Post-MVP | M8: Finder parity | in progress | 7 pieces; F1 done, F2–F7 planned |
| Post-MVP | M8.5: search reliability | planned | 7 |
| Phase 2 | M9: plugins, views, operations | planned | 9 |

---

## Phase 1 — MVP (done)

### M1–M4 — foundation

| Item | Status | Where |
| --- | --- | --- |
| M1 Split into crates and build the app shell | done | [#1](https://github.com/paraversal/file-crow/pull/1) |
| M2 Fast listing, sortable list view, selection and Finder-style preview | done | [#2](https://github.com/paraversal/file-crow/pull/2) |
| M3 Column view | done | [#3](https://github.com/paraversal/file-crow/pull/3) |
| M4 File operations with conflicts, progress, undo and the Finder clipboard | done | [#4](https://github.com/paraversal/file-crow/pull/4) |

### M5 — sidebar, view settings, themes

| Item | Status | Where |
| --- | --- | --- |
| M5 Sidebar, view settings and themes | done | [#5](https://github.com/paraversal/file-crow/pull/5) |
| M5-fix I1 Quick Look crashes | done | [#6](https://github.com/paraversal/file-crow/pull/6) |
| M5-fix I2 Path bar segments don't navigate | done | [#6](https://github.com/paraversal/file-crow/pull/6) |
| M5-fix I3 Column view: Left in the leftmost column | done | [#6](https://github.com/paraversal/file-crow/pull/6) |
| M5-fix I4 List view: expandable folders | done | [#6](https://github.com/paraversal/file-crow/pull/6) |
| M5-fix I5 Reordering favorites by dragging | done | [#6](https://github.com/paraversal/file-crow/pull/6) |
| M5-fix I6 Sidebar row flickers during a folder drag | done | [#6](https://github.com/paraversal/file-crow/pull/6) |
| M5-fix-2 D1 Drag feedback: target flickers, dragged file lags | done | [#6](https://github.com/paraversal/file-crow/pull/6) |
| M5-fix-2 D2 One insertion target between sidebar items | done | [#6](https://github.com/paraversal/file-crow/pull/6) |
| M5-fix-2 D3 Larger chevron disclosure | done | [#6](https://github.com/paraversal/file-crow/pull/6) |
| M5-fix-2 D4 Remove Option-click expand-all (freeze) | done | [#6](https://github.com/paraversal/file-crow/pull/6) |
| M5-fix-2 D5 Drag image: Finder-like combo, one image at a time | done | [#6](https://github.com/paraversal/file-crow/pull/6) |
| M5-fix-2 D6 Chevron rotation, not glyph swap | done | [#6](https://github.com/paraversal/file-crow/pull/6) |
| M5-fix-2 D7 Drag handoff: keep the combo, smooth the swap | done | [#6](https://github.com/paraversal/file-crow/pull/6) |
| M5-fix-2 D8 Combo start and attraction easing | done | [#6](https://github.com/paraversal/file-crow/pull/6) |

### M6 — filter, command palette, search

| Item | Status | Where |
| --- | --- | --- |
| S1 `crow-search` engine: rules, fuzzy names, literal/regex contents, streaming, cancel | done | [#7](https://github.com/paraversal/file-crow/pull/7) |
| S2 `exclude.toml`: sets, sentinels, defaults, live reload | done | [#7](https://github.com/paraversal/file-crow/pull/7) |
| S3 `/` filter in the focused pane | done | [#7](https://github.com/paraversal/file-crow/pull/7) |
| S4 Cmd-K command palette and fuzzy go-to | done | [#7](https://github.com/paraversal/file-crow/pull/7) |
| S5 Search panel: names and contents, scope, skipped list | done | [#7](https://github.com/paraversal/file-crow/pull/7) |
| S6 Spotlight whole-disk name search (This Mac) | done | [#7](https://github.com/paraversal/file-crow/pull/7) |
| S7 Finish: docs, tests, bench | done | [#7](https://github.com/paraversal/file-crow/pull/7) |

### UI overhaul U1–U11

Index: [`plans/ui-overhaul/README.md`](plans/ui-overhaul/README.md).

| Item | Status | Where |
| --- | --- | --- |
| U1 Shared parts: keycaps, tooltip, Sheet, SVG icons, selection tints | done | [#8](https://github.com/paraversal/file-crow/pull/8) |
| U2 Menu bar built from the command registry | done | [#9](https://github.com/paraversal/file-crow/pull/9) |
| U3 Top band: search field, Go to, match modes | done | [#10](https://github.com/paraversal/file-crow/pull/10) |
| U4 Filter inside the focused column | done | [#12](https://github.com/paraversal/file-crow/pull/12) |
| U5 Bottom bar: path, hints, progress, errors, counts | done | [#13](https://github.com/paraversal/file-crow/pull/13) |
| U6 Search panel: scope bar, footer, idle examples, preview, This Mac | done | [#14](https://github.com/paraversal/file-crow/pull/14) |
| U6-fix F1–F8: search panel review fixes | done | [#15](https://github.com/paraversal/file-crow/pull/15) |
| U7 Palette: sections, two-line rows, Commands token, Tab completion, recent commands | done | [#16](https://github.com/paraversal/file-crow/pull/16) |
| U7-fix X1–X7: palette spacing, caret, search ↓, New File, hide preview, freezes, stale delete | done | [#18](https://github.com/paraversal/file-crow/pull/18) |
| U8 Sidebar twins, neutral disk bar, one fill per meaning, mark letters | done | [#20](https://github.com/paraversal/file-crow/pull/20) |
| U9 One-time contextual tips | done | [#21](https://github.com/paraversal/file-crow/pull/21) |
| U10 Motion and sound feedback: off/low/full, sound sets, motion lab | done | [#22](https://github.com/paraversal/file-crow/pull/22) |
| U11 Whole-window pass, docs, bench | done | [#23](https://github.com/paraversal/file-crow/pull/23) |

### M6.5 — settings window

Plan: [`plans/done/m6.5-settings.md`](plans/done/m6.5-settings.md).

| Item | Status | Where |
| --- | --- | --- |
| I1 The window: one Settings window, Cmd-, and Crow ▸ Settings…, Cmd-W/Esc | done | [#28](https://github.com/paraversal/file-crow/pull/28) |
| I2 Shared controls: switch, segmented, tile choice, stepper, choice list, buttons | done | [#28](https://github.com/paraversal/file-crow/pull/28) |
| I3 What the window shows: every setting, grouped | done | [#28](https://github.com/paraversal/file-crow/pull/28) |
| I4 Writing: live apply, background save, comments preserved | done | [#28](https://github.com/paraversal/file-crow/pull/28) |
| I5 Following the file: external edits, broken-file banner | done | [#28](https://github.com/paraversal/file-crow/pull/28) |
| I6 Reaching it: Cmd-, and palette commands | done | [#28](https://github.com/paraversal/file-crow/pull/28) |
| I7 Accessibility: roles, labels, states, VoiceOver | done | [#28](https://github.com/paraversal/file-crow/pull/28) |

### M7 — hardening

Index: [`plans/done/m7/README.md`](plans/done/m7/README.md). Findings: [`plans/done/m7/findings.md`](plans/done/m7/findings.md).

| Item | Status | Where |
| --- | --- | --- |
| H1 Seeded action fuzzer: side-effect stubs, generator, invariants, shrinking, replay | done | [#30](https://github.com/paraversal/file-crow/pull/30) |
| H2 Edge-case trees and churn | done | [#31](https://github.com/paraversal/file-crow/pull/31) |
| H3 `crow --fuzz`: real-window frame budget and slow-step report | done | [#33](https://github.com/paraversal/file-crow/pull/33) |
| H4 Findings batch 1 | done | [#32](https://github.com/paraversal/file-crow/pull/32) |
| H4 Findings batch 2 | done | [#37](https://github.com/paraversal/file-crow/pull/37) |
| H4 Findings batch 3 | done | [#43](https://github.com/paraversal/file-crow/pull/43) |
| H4 Findings batch 4 | done | [#44](https://github.com/paraversal/file-crow/pull/44) |
| H4 Findings batch 5 | done | [#45](https://github.com/paraversal/file-crow/pull/45) |
| H4 Findings batch 6 | done | [#46](https://github.com/paraversal/file-crow/pull/46) |
| H5 No disk I/O on the main thread | done | [#39](https://github.com/paraversal/file-crow/pull/39) |
| H6 Editable path bar in the bottom bar | done | [#42](https://github.com/paraversal/file-crow/pull/42) |
| H7 Accessibility hardening: disabled state, fuzzer invariant, VoiceOver | done | [#40](https://github.com/paraversal/file-crow/pull/40) |
| H8 Leftovers: hints, rounding, palette `~`, shortcut mismatch | done | [#47](https://github.com/paraversal/file-crow/pull/47) |

The H4 list has 28 findings; all are closed or marked won't-fix.

### Fix lists (done)

| List | Items | Status | Where |
| --- | --- | --- | --- |
| Housekeeping | 1 resident app; 2 drag promotion; 3 Escape cancels a drag; 4 expanded-folder drops; 5 checks | done | [#24](https://github.com/paraversal/file-crow/pull/24) |
| Housekeeping 2 | `crow` command; sidebar menu; Cmd-K file paths; app icons | done | [#27](https://github.com/paraversal/file-crow/pull/27) |
| Housekeeping 3 | 1 Cmd-⌫ in text fields; 2 Escape clears selection; 3 Copy Path on empty space; 4 checks | done | [#36](https://github.com/paraversal/file-crow/pull/36) |
| Context menu | C1 click empty space deselects; C2 menu model; C3 opening; C4 new commands; C5 scope and Vim | done | [#25](https://github.com/paraversal/file-crow/pull/25) |

---

## Post-MVP — M8: Finder parity (in progress)

Index: [`plans/m8/README.md`](plans/m8/README.md). Suggested order: F2 → F5 → F4 → F3 → F6 → F7. F6 is independent.

| Piece | Item | Status |
| --- | --- | --- |
| F1 Selection and motion | 1 Delete selects the next row | done |
| F1 | 2 Finder-style selection islands | done |
| F1 | 3 Complete the row-removal animation | done |
| F1 | 4 Copy/cut feedback | done |
| F1 | 5 Sidebar current-folder fill glides | done |
| F1 | 6 Tip card position | done |
| F1 | 7 Finish: checks, bench, bundle, docs | done |
| F2 Typing and keys | 1 Show the type-to-select query | planned |
| F2 | 2 Enter behavior setting | planned |
| F2 | 3 Context menus for search results and the sidebar | planned |
| F2 | 4 Palette home-row matching | planned |
| F3 Info pane | 1 The surface (Preview/Info tabs) | planned |
| F3 | 2 Fields | planned |
| F3 | 3 Metadata loading | planned |
| F3 | 4 Reaching it (Cmd-I, menu, palette) | planned |
| F4 Path bar | 1 Drag folders from the path bar | planned |
| F4 | 2 Discoverable edit affordance | planned |
| F4 | 3 Windows Explorer-style option (design; build in M9) | planned |
| F5 Tips and first launch | 1 Separate `tips` setting | planned |
| F5 | 2 First-launch popup | planned |
| F5 | 3 "Shut up" from the tip card | planned |
| F5 | 4 Tip position | planned |
| F6 Folder sizes | S1 Spike: measure before choosing | planned |
| F6 | S2 The sizer (`crow-fs`) | planned |
| F6 | S3 The cache | planned |
| F6 | S4 FSEvents history (`crow-macos`) | planned |
| F6 | S5 Scheduling (`crow-app`) | planned |
| F6 | S6 Showing sizes | planned |
| F6 | S7 Sorting by size | planned |
| F6 | S8 Setting and command | planned |
| F6 | S9 Docs, tests, bench | planned |
| F7 Housekeeping 4 | 1 Open Terminal Here | planned |
| F7 | 2 New File/Folder with a row selected | planned |
| F7 | 3 Window drag | planned |
| F7 | 4 Board-only details | planned |
| F7 | 5 NSSound volume hand check | planned |
| F7 | 6 Empty Trash (Cmd-Shift-Delete) | planned |

F1 is merged ([#48](https://github.com/paraversal/file-crow/pull/48), follow-up [#49](https://github.com/paraversal/file-crow/pull/49)). F2–F7 have plans but no progress files yet.

---

## Post-MVP — M8.5: search reliability (planned)

Index: [`plans/m8.5/README.md`](plans/m8.5/README.md). Starts after M8. The first piece builds the repro corpus; no behavior changes until the reported misses are reproduced.

| Piece | What | Status | Needs |
| --- | --- | --- | --- |
| S1 Repro corpus and diagnostics | Deterministic tree with dot names, `backup` names, exclusions, packages, symlinks and a 100k subtree; a query log that explains inclusion/skips. No behavior change. | planned | — |
| S2 Hidden and dot names | Cmd-F/Cmd-K policy for dot-named items; Cmd-K path mode matches files by name, not only prefix. | planned | S1 |
| S3 Scope and exclusions | Make the default scope and scope bar unambiguous; skipped folders and "Search everything" at a miss; packages, symlinks, unreadable folders. | planned | S1 |
| S4 Ranking and cap | Keep strong name hits ahead of content/weak fuzzy hits at the 20k cap; stable selection; actionable cap state. | planned | S1 |
| S5 Performance | Profile `back` vs `backup`; keep name hits streaming while contents read; cancellation and batching off the main thread. | planned | S1, S4 |
| S6 Feedback | Explain an empty search (hidden off, excluded folder, contents still running, cap reached); keep the skipped list reachable. | planned | S2, S3, S4 |
| S7 Regression and fuzz | Corpus assertions, brute-force oracle on small trees, search fuzz, bench on `target/fixture-search-200k`. | planned | S2–S5 |

The user's two reported cases are the acceptance tests: Cmd-K must find dot-named items, and Cmd-F must find "backup" in the reported folder.

---

## Phase 2 — M9: plugins, views, operations (planned)

Index: [`plans/m9/README.md`](plans/m9/README.md). Starts after M8.5. P9 runs in parallel, one item at a time.

| Piece | What | Status | Needs |
| --- | --- | --- | --- |
| P1 Public Rhai plugin API | `plugin.toml`, permissions, worker isolation, operation budget, extension points | planned | — |
| P2 Git plugin / Projects | Status letters, ignored-file dimming, lines-changed column, branch/ahead-behind, Projects section | planned | P1 |
| P3 Icon packs and views | Data-only icon packs; icon and gallery views with the same selection/preview/drag/keyboard rules | planned | P1; views can start after M8 F1 |
| P4 Saved queries and Tags | Live searches as virtual folders; Finder tags in the sidebar and as a filter | planned | M6 search; `crow-macos::tags` |
| P5 Copy queue | Pause, resume, conflict dialog for long copies; one queue in the activity list | planned | M4 operations; M8 F1 |
| P6 Batch rename with preview | Rename many items with preview, numbering, find/replace; undoable | planned | M4 operations |
| P7 Drop in and spring-loaded folders | Accept drops from other apps everywhere; spring-open on hover | planned | M8 F4; existing drag plumbing |
| P8 iCloud download status | Not-downloaded/downloading/downloaded; never trigger a download by browsing | planned | `crow-macos` iCloud keys |
| P9 Scale and maintenance | Test temp-dir hygiene; cross-run fuzz/test dashboard; split `window.rs`; move the palette view; Explorer path-bar setting; Solarized Light contrast | planned | — |

P9's temp-dir item is the 2026-11-17 finding: a full test run left 124,961 `crow-*` folders (5.8 GB) in the system temp folder. The fix is a per-test root removed when the test process ends, plus a clean-run check.

---

## Deferred and not planned

| Item | Where it stands |
| --- | --- |
| Rubber-band selection | Deferred (M8 index Q2). If picked up, add Cmd-drag additive selection. |
| Windows Explorer-style path bar | Designed in M8 F4.3; built in M9 P9. |
| Board-only details (line icons, syntax preview, 196 px columns, focused-column tint, scope token) | M8 F7.4, one at a time. |
| Remote file systems, desktop icons, Open/Save panels, free icon placement, Mac App Store | Spec's "Not planned" list. |
| Content indexing or a search database | Not planned; Crow stays a live walker. |
| Solarized Light contrast | The theme index's palette, not Crow code; tracked in M9 P9. |

---

## How to keep this page current

When a piece merges, update its row here and the plan's progress file. When a milestone finishes, move its folder to [`plans/done/`](plans/done/) and mark the milestone done. The plan and progress files remain the source of truth; this page is the one-screen summary.
