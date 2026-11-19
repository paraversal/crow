# M9 — Plugins, views, and operations

Phase 2 builds on the [completed daily-driver milestones](../../ROADMAP.md#completed-milestones). The table records work merged to `main` and dependencies for the remaining pieces.

## Shipped

| Piece | Delivered | PR |
| --- | --- | --- |
| P1A | Versioned Rhai packages, permissions, registration, and budgeted workers | [#61](https://github.com/paraversal/file-crow/pull/61) |
| P9A | Test and fuzz fixture ownership and cleanup | [#62](https://github.com/paraversal/file-crow/pull/62) |
| P9B | Cross-run test and fuzz reporting with a baseline | [#63](https://github.com/paraversal/file-crow/pull/63) |
| P9C | Palette and window module extraction | [#64](https://github.com/paraversal/file-crow/pull/64) |
| P8 | Read-only iCloud download status and metadata-only browsing for placeholders | [#65](https://github.com/paraversal/file-crow/pull/65) |
| P1B | App extension surfaces and [plugin author documentation](../../plugins.md) | [#66](https://github.com/paraversal/file-crow/pull/66) |
| P2 | Read-only Git status, lines changed, and Projects with user-selected roots | [#67](https://github.com/paraversal/file-crow/pull/67) |
| P2 follow-up | Git review and diff, pinned repository status, and resizable side panes | [#68](https://github.com/paraversal/file-crow/pull/68) |
| Navigation follow-up | List and Columns transitions, reveal feedback, persistent sidebar width, and bulk selection scaling tests | [#69](https://github.com/paraversal/file-crow/pull/69) |

The [P2 code audit](../../m9-code-audit.md) records the performance baseline and open findings.

## Remaining

| Piece | Deliverable | Remaining dependencies |
| --- | --- | --- |
| P3A | Local data-only icon packs, resolution, discovery, and settings | — |
| P3B | Sorted icon grid with List and Columns interaction parity, rectangle selection, and Cmd-drag additive selection | — |
| P3C | Gallery using the shared thumbnail and selection systems | P3B |
| P4A | Saved queries as live destinations, with typed kind, modification-date, and tag predicates | — |
| P4B | Finder tags in the sidebar, local filter, and saved-query predicates | P4A |
| P5A | Resumable copy jobs and a conflict protocol | — |
| P5B | One app-wide copy queue in Activity with session pause, resume, and conflict sheets | P5A |
| P6 | Batch rename with preview, literal find/replace, add text, numbering, and composite undo/redo | P5B |
| P9D | Live opt-in Explorer-style breadcrumb editing | P4A |
| P7 | External file URL drops and spring-loaded folders | P3C, P4B, P5B, P9D |
| P9E | Solarized Light contrast in the shared theme index | — |
| Finish | Integrated verification, documentation, and milestone close | Remaining feature and maintenance pieces |

The existing sequence for app work starts P3A → P3B → P3C → P4A → P4B, then P5B → P6 → P9D → P7. P5A can start independently. P9E lives in the theme index repository. Follow the [planning and hand-off workflow](../../../AGENTS.md#working-on-a-plan) for each piece.
