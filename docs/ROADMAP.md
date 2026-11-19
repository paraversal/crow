# Crow roadmap

Last updated 2026-11-19. The daily-driver MVP, Finder parity, and search reliability milestones are complete. Phase 2 is underway.

## Phase 2 — M9

Plugin support, Git Projects and review, iCloud status, and the first maintenance pieces have merged. The [M9 roadmap](plans/m9/README.md) records shipped work, dependencies, and the remaining scope.

| Area | Remaining work |
| --- | --- |
| Views and icons | Data-only icon packs, icon grid with rectangle selection, and Gallery |
| Search and organization | Saved queries and Finder tags |
| File operations | Copy queue with pause, resume and conflict handling, batch rename with preview, and external drops with spring-loaded folders |
| Appearance | Optional Explorer-style path bar and Solarized Light contrast in the shared theme index |

The [inbox](plans/inbox.md) holds work awaiting triage. Search will continue to walk the live filesystem. The [product spec](Crow%20%E2%80%94%20Product%20%26%20Technical%20Spec.md#mvp-scope) records the broader scope boundaries.

## Completed milestones

| Milestone | What shipped | Where |
| --- | --- | --- |
| M1–M4 · Foundation | App shell, fast folder listing, List and Columns, preview, and file operations with undo | [#1](https://github.com/paraversal/file-crow/pull/1), [#2](https://github.com/paraversal/file-crow/pull/2), [#3](https://github.com/paraversal/file-crow/pull/3), [#4](https://github.com/paraversal/file-crow/pull/4) |
| M5 · Sidebar and views | Favorites, per-folder view settings, and themes | [#5](https://github.com/paraversal/file-crow/pull/5), [#6](https://github.com/paraversal/file-crow/pull/6) |
| M6 · Search and navigation | Inline filter, command palette, filename and content search, and Spotlight scope | [#7](https://github.com/paraversal/file-crow/pull/7) |
| UI overhaul | Toolbar, menus, search, palette, sidebar, tips, motion, and sound | [Design overview and PRs](plans/ui-overhaul/README.md) |
| M6.5 · Settings | Live configuration from the Settings window | [#28](https://github.com/paraversal/file-crow/pull/28) |
| M7 · Hardening | Seeded fuzzing, edge trees, frame budgets, main-thread I/O cleanup, path editing, and accessibility | [Findings and replays](plans/done/m7/findings.md) |
| Housekeeping and context menus | App lifecycle, command-line launcher, menus, text bindings, and drag and drop | [#24](https://github.com/paraversal/file-crow/pull/24), [#25](https://github.com/paraversal/file-crow/pull/25), [#27](https://github.com/paraversal/file-crow/pull/27), [#36](https://github.com/paraversal/file-crow/pull/36) |

### M8 — Finder parity

| Piece | What shipped | PR |
| --- | --- | --- |
| F1 | Selection islands, delete focus, removal motion, and copy/cut feedback | [#48](https://github.com/paraversal/file-crow/pull/48), [#49](https://github.com/paraversal/file-crow/pull/49) |
| F2 | Visible type-to-select, configurable Return, and context menus | [#50](https://github.com/paraversal/file-crow/pull/50) |
| F3 | Info pane | [#53](https://github.com/paraversal/file-crow/pull/53) |
| F4 | Draggable path segments and discoverable path editing | [#52](https://github.com/paraversal/file-crow/pull/52) |
| F5 and F8 | Tips, first launch, and one focus target in the filter | [#51](https://github.com/paraversal/file-crow/pull/51) |
| F6 | Folder sizes | [#54](https://github.com/paraversal/file-crow/pull/54) |
| F7 | Open Terminal Here, new-item placement, window drag, and Empty Trash | [#55](https://github.com/paraversal/file-crow/pull/55) |

### M8.5 — Search reliability

| Batch | What shipped | PR |
| --- | --- | --- |
| B1 | Batched streaming, name hits before content reads, typed skips, and search tracing | [#56](https://github.com/paraversal/file-crow/pull/56) |
| B2 | Hidden-name matching and current-folder files in Cmd-K | [#57](https://github.com/paraversal/file-crow/pull/57) |
| B3 | Search Home, explicit Folder scope, and ways to widen a search | [#58](https://github.com/paraversal/file-crow/pull/58) |
| B4 | Separate result budgets and stable selection while results arrive | [#59](https://github.com/paraversal/file-crow/pull/59) |
| B5 | Regression corpus, randomized search oracle, and fuzz coverage | [#60](https://github.com/paraversal/file-crow/pull/60) |

Update the relevant roadmap and record the PR when a piece merges. Keep implementation steps and progress logs in the local planning files described in [AGENTS.md](../AGENTS.md).
