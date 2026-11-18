# M8 — Finder parity (index)

Phase 1 is done: M6, the UI overhaul, M6.5 (the settings window) and M7 (hardening) are merged. Crow is a daily driver. M8 closes the small places where it still does not behave like Finder, and lands folder sizes, the one MVP-adjacent feature deferred since D2. Phase 2 starts in M9.

Drafted 2026-11-17 from the user's triage (the ten items below), the D3 inbox, and the deferrals in U9, U10 and the context-menu critique. M7's findings are closed; [`../done/m7/findings.md`](../done/m7/findings.md) is history.

## The pieces

Each piece is self-contained: its own branch from `main`, progress file next to it (`docs/plans/m8/<piece>.progress.md`), PR, and critique. Carry it out with the `execute-plan` skill; it ends at "Ready for critique", and a second agent runs the `critique` skill and opens the PR. Start a piece only when the pieces it needs are **merged to `main`** (ASK before stacking on an unmerged branch).

| Piece | What | Needs | Branch |
| --- | --- | --- | --- |
| [F1](f1-selection-and-motion.md) | Delete selects the next row, Finder-style selection islands, complete row-removal motion, copy/cut feedback, sidebar fill glide, tip position | — | `m8-f1-selection` |
| [F2](f2-typing-and-keys.md) | Visible type-to-select query, Enter rename/open setting, context menus for search results and the sidebar, palette home-row matching | — | `m8-f2-typing` |
| [F3](f3-info-pane.md) | Info pane (⌘I): metadata for one or many items, sharing the right column with preview | F1 for row selection; F6 for folder sizes if the pane shows them | `m8-f3-info` |
| [F4](f4-path-bar.md) | Drag folders from the bottom path bar, a visible edit affordance; the Windows Explorer-style bar is an M9 setting | — | `m8-f4-path-bar` |
| [F5](f5-tips-and-first-launch.md) | Separate `tips` setting, one-time first-launch popup, tip position | F1 if the tip position moves with the row work | `m8-f5-tips` |
| [F6](f6-folder-sizes.md) | Folder sizes: lazy measuring, cache, FSEvents history, column and bottom bar, setting | — | `m8-f6-folder-sizes` |
| [F7](f7-housekeeping-4.md) | Open Terminal Here, New File/Folder with a selection, window drag, board-only details, Empty Trash, NSSound hand check | — | `m8-f7-housekeeping` |
| [F8](f8-filter-focus.md) | One focus in the `/` filter: ↓ enters the matches, ↑ on the first row returns to the field, the list's selection is inactive while the field is focused | — | `m8-filter-focus` |

**Suggested order:** F1 → F2 → F5 → F4 → F3 → F6 → F7. F6 is independent and can start first or any time; never run it concurrently with F1 (both touch row rendering) or F3 (both touch the right column). F7 is small and can run any time, but it also touches `window.rs`/`pane.rs`; never two writers at once. F8 is independent and can run any time; it touches `pane.rs`/`pane_filter.rs` and the keymap, so do not run it beside F5 or F7 if they are editing the same files.

**M8 is done when:** every piece is merged; the first triage table below has no open item and the second list's M8 item (F7.6 Empty Trash) is done; `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass; the bench on `target/fixture-100k` is within the AGENTS.md budgets for the pieces that touch rows, the right column or first paint; the hand checks cover light and dark themes, 640 px and a wide window, an empty folder and the 100k fixture, Vim on and off, and VoiceOver for every new role; README's status line says M8.5 (search reliability) is next.

## Triage (2026-11-17)

Every item the user sent, with its tag and where it went. `f` feature · `p` performance · `ui` UI improvement · `b` bug · `d` future direction · `s` setting.

| # | Tag | Item | Where |
| --- | --- | --- | --- |
| 1 | `ui` | Deleting a file/folder focuses the next row like Finder, not nothing | F1.1 |
| 2 | `f` | Info pane (⌘I) | F3 |
| 3 | `ui` | Option to completely mute all tip toasts ("shut up" setting offered in a first-launch popup) | F5 |
| 4 | `ui` | Multi-selection like Finder: one continuous rounded box per contiguous island, faint rules between rows, each island its own highlight | F1.2 |
| 5 | `ui` | Drag folders from the bottom path bar | F4.1 |
| 6 | — | Fuzz runs as a timed job **in the skill** | Done 2026-11-17: `execute-plan` §7 "Timed jobs"; no plan needed |
| 7 | `ui` | Removing an item smoothly animates the rest up | F1.3 |
| 8 | `s` | "What does Enter do": rename vs open | F2.2 |
| 9 | `d` | Path bar like Windows Explorer, or at least the option; the current editable bar is undiscoverable | F4.3 (design now, setting in M9) |
| 10 | `ui` | Filter typing in a folder (without `/`) shows the query somewhere | F2.1 |
| 11 | `ui` | Filter: ↓ should move focus into the results, not leave the field and the list both looking focused | F8 |

## Triage (2026-11-17, second list)

The user's follow-up list, triaged after the M8 draft. Same tags.

| # | Tag | Item | Where |
| --- | --- | --- | --- |
| 1 | `b` | Test runs leave `crow-*` temp dirs: 124,961 in `$TMPDIR` (5.8 GB), plus a batch in `/tmp` the user already deleted. `ui::init_for_tests` makes one `crow-app-config-*` per GPUI test and never removes it; the fixture helpers do the same. | M9 P9 |
| 2 | `f` | Empty Trash: ⌘⇧⌫ should empty the Trash, with Finder's confirmation, a File menu item, a palette command and the `EmptyTrash` sound. Crow has no empty-trash action today (only Move to Trash and Delete Permanently). | F7.6 |
| 3 | `b`/`d` | Search reliability: Cmd-K misses dot-named items; Cmd-F finds "back" but not "backup"; generally unreliable. The fuzzy matcher itself matches all these names, so the miss is in scope, hidden policy, prefix-only file completion, exclusions or the result cap. Needs a repro corpus before a piece list. | M8.5 |

Existing items folded into M8:

| Source | Item | Where |
| --- | --- | --- |
| D3 inbox | Copy/cut feedback: flash copied rows, dim cut rows | F1.4 |
| D3 inbox | Open Terminal Here | F7.1 |
| D3 inbox | New File/Folder with a row selected: shown folder or the selected folder? | F7.2 |
| D3 inbox | Fuzz/test job overview (cross-run dashboard) | M9 P9 |
| D3 inbox | Window drag: toolbar controls move the window | F7.3 |
| D3 inbox | Sidebar background menu | Dropped: the user retracted it |
| U9 deferral | Tip card floats over the list's bottom-right rows | F1.6 / F5 |
| U10 deferral | Sidebar current-folder fill does not glide | F1.5 |
| U10 hand check | NSSound vs the alert volume | F7.5 |
| context-menu Q3 | Context menus for search results and the sidebar | F2.3 |
| U7 C6 | Palette home row displays `jules` but matches only `~` | F2.4 |
| U11 follow-up | Board-only details: line icons, clock icons, syntax preview, 196 px columns, focused-column tint, scope token | F7.4 |
| U11 follow-up | Palette view lives in `window.rs` (~700 lines) | M9 P9 |
| U11 follow-up | Split `window.rs` (9.4k lines) | M9 P9 |
| U11 note | Solarized Light contrast | Theme index, not Crow code; no Crow plan |

## Decisions (2026-11-17)

The user answered the scope questions before this index was written:

- **Milestone split:** M8 = Finder parity + folder sizes + polish; M9 = Phase 2 platform/operations (option A).
- **Info pane:** a right-side inspector pane, sharing the right column with preview (tabs), for one or many items; not a separate Get Info window.
- **Tips:** a separate `tips = "auto" | "off"` setting plus a one-time first-launch popup; hints stay independent.
- **Typing without `/`:** keep Finder type-to-select and show the typed prefix; do not turn it into a live filter.
- **Path bar:** M8 adds segment drag and a visible edit affordance; the full Windows Explorer-style breadcrumb is an M9 setting.
- **Enter in the file list:** default Rename (Finder); the setting can switch it to Open.

Each piece's "Questions to ASK" only covers what these decisions leave open.

## Shared names

Later pieces use these names. Whichever piece lands first builds the shared part with this shape; if merged code differs, use the merged name and record the mapping in the progress file.

| Part | Name | Built by |
| --- | --- | --- |
| Contiguous selection runs in display order | `pane::selection_islands(rows, selected) -> Vec<Range<usize>>` | F1 |
| Type-to-select prefix | `crow_core::TypeSelect::prefix() -> &str`; `Folder::type_select_prefix()` | F2 |
| Enter action that reads the setting | `EnterRow` | F2 |
| Info pane surface | `InfoPane` entity in `crates/crow-app/src/info_pane.rs` | F3 |
| Tips setting | `crow_core::TipsMode` (`"auto"` / `"off"`), `Settings.tips` | F5 |
| Path-bar style (M9) | `PathBarStyle` (`"crow"` / `"explorer"`), `Settings.path_bar` | M9 |
| Terminal seam | `Effects::open_terminal(path)` | F7 |

## Questions to ASK

Index-level questions for the user, asked by whoever starts the first piece:

1. **Order: F1 first or F6 first?** Recommended F1: the triage items are the visible daily annoyances, and F1 settles the row rendering F3 and F6 touch. F6 is already planned and independent; start it first only if folder sizes are the priority.
2. **Rubber-band selection (drag a rectangle on empty space) is a known Finder gap** (context-menu critique, "Found along the way"). It is not in the user's triage list. Include it in F1 or defer? Recommended defer: F1 already changes selection rendering, and rubber-band adds pointer geometry and auto-scroll.
3. **One branch per piece?** Recommended yes, as in M7. F1/F2/F3/F6/F7/F8 all touch `pane.rs`/`window.rs`; one writer at a time.

Each piece ends with its own "Questions to ASK"; ask those in one batch when that piece starts. Don't answer them all up front.

## Not in M8 (and where it goes)

- Public Rhai plugin API, git plugin / Projects, icon packs, icon/gallery views, saved queries, Tags, copy queue, batch rename, drop-in/spring-loaded folders, iCloud download status: Phase 2, [`../m9/README.md`](../m9/README.md).
- Windows Explorer-style path bar: designed in F4.3, built as an M9 setting.
- Search reliability (the second triage's item 3): its own milestone, [`../done/m8.5/README.md`](../done/m8.5/README.md). It needs a repro corpus before it can be split into pieces; it is not part of M8.
- Rubber-band selection: deferred unless the user says otherwise (index Q2).
- Remote file systems, desktop icons, Open/Save panels, free icon placement, Mac App Store: the spec's "Not planned" list.
- Solarized Light contrast: the theme index's palette, not Crow code.
