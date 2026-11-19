# Crow UI overhaul — design overview

The overhaul gave the M6 search and navigation commands visible entry points and unified the toolbar, menus, overlays, and bottom bar. The boards below show the starting point and the selected layouts for filtering and search.

## Design boards

![The app after M6, annotated with design findings](img/today-annotated.png)

![Target column view while filtering, with the new toolbar and bottom bar](img/assembled-filter.png)

![Target search mode with the toolbar field, scope bar, results, and preview](img/assembled-search.png)

The 2× board renders and variant crops live in [img/](img/). Files named `built-*` are screenshots of the implementation. Use the boards alongside the [product principles](../../Crow%20%E2%80%94%20Product%20%26%20Technical%20Spec.md#principles) and [hand checks](../../testing.md#by-hand-each-milestone).

## What shipped

| Piece | Change | PR |
| --- | --- | --- |
| U1 | Keycaps from the live keymap, tooltips, shared sheets, SVG icons, and selection tints | [#8](https://github.com/paraversal/file-crow/pull/8) |
| U2 | Native menus built from the command registry | [#9](https://github.com/paraversal/file-crow/pull/9) |
| U3 | Toolbar with Search, Go To, and All / Names / Contents | [#10](https://github.com/paraversal/file-crow/pull/10) |
| U4 | Inline filter in the focused column with match highlighting | [#12](https://github.com/paraversal/file-crow/pull/12) |
| U5 | Bottom bar with path, hints, progress, errors, and counts | [#13](https://github.com/paraversal/file-crow/pull/13) |
| U6 | Search scope, footer, idle examples, and preview | [#14](https://github.com/paraversal/file-crow/pull/14), [#15](https://github.com/paraversal/file-crow/pull/15) |
| U7 | Palette sections, tab completion, and recent commands | [#16](https://github.com/paraversal/file-crow/pull/16), [#18](https://github.com/paraversal/file-crow/pull/18) |
| U8 | Sidebar selection treatments, disk usage, and Vim marks | [#20](https://github.com/paraversal/file-crow/pull/20) |
| U9 | One-time contextual tips | [#21](https://github.com/paraversal/file-crow/pull/21) |
| U10 | Motion and sound feedback, Reduce Motion support, and the motion lab | [#22](https://github.com/paraversal/file-crow/pull/22) |
| U11 | Whole-window check against the assembled boards, docs, and benchmark | [#23](https://github.com/paraversal/file-crow/pull/23) |

## Design decisions

- Embedded SVGs take their colors from theme roles. Active selection, inactive selection, the current location, keyboard focus, and drop targets have distinct treatments. The inset focus ring leaves row dimensions stable.
- Search defaults to All. Names skips content reads, Contents hides name-only hits, and This Mac names the Spotlight scope. The results omit Kind. Find Everything opens a search of the current folder with every exclusion set off.
- Idle hints default to `auto` and retire after three uses of their key. The palette remembers recent commands across launches and supports Tab completion.
- The bottom bar owns path editing. File operations give motion and sound feedback according to the user's settings.
- Later UI ideas from the boards are in the [inbox](../inbox.md).
