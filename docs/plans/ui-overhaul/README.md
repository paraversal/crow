# UI overhaul: make M6 visible (index)

Every M6 feature works, but nothing on screen mentions `/`, ⌘F or ⌘K. The menu bar has no Go or Find menu, the toolbar has no search field, there are no tooltips, and the overlays and status line each grew separately. The canvas board "Today, annotated" lists twelve findings; these plans fix them.

Source of scope: the user's pick of the **recommended** variants on the Crow storyboards canvas (https://claude.ai/artifact/EbZ7gFCy24MaccdpbSsFtE, section "UI overhaul: proposal"), 2026-11-12. Not a Phase 1 milestone: the overhaul runs after M6 (PR #7) and before M7 (hardening). Animations are out of scope (a later plan).

## Where we start, where we end

![Today, annotated: the app after M6 with eight numbered findings](img/today-annotated.png)

![Target: column view while filtering, with the new top band and bottom bar](img/assembled-filter.png)

![Target: search mode with the field in the toolbar, scope bar, results and preview](img/assembled-search.png)

All images in `img/` are 2× renders of the canvas boards (2026-11-12). Per-variant crops (`top-band-*`, `filter-*`, `bottom-bar-*`, `search-panel-*`, `palette-*`, `discovery-*`) are embedded in the plans that build them. If the canvas changes, re-render rather than editing the PNGs.

## The plans
Each plan is self-contained: its own branch, progress file and PR, carried out with the `execute-plan` skill. Start one only when the plans it needs are **merged to `main`** (ASK the user before stacking on an unmerged branch).

| Plan                     | What                                                                                        | Needs  | Branch               |
| ------------------------ | ------------------------------------------------------------------------------------------- | ------ | -------------------- |
| [U1](../done/u1-shared-parts.md) | Keycaps from the live keymap, tooltip, one Sheet overlay, SVG icons, selection tints        | PR #7  | `ui-u1-shared-parts` |
| [U2](../done/u2-menu-bar.md)     | Menu bar built from the command registry: Edit, Go, File, View                              | PR #7  | `ui-u2-menu-bar`     |
| [U3](../done/u3-top-band.md)     | Toolbar: search field ⌘F, Go to ⌘K, icons; search mode band with All / Names / Contents     | U1     | `ui-u3-top-band`     |
| [U4](../done/u4-filter.md)       | `/` filter inside the focused column, highlights, ⌘F escalation, no-match state             | U1     | `ui-u4-filter`       |
| [U5](../done/u5-bottom-bar.md)   | Bottom bar slots: path · hints/progress/errors · counts; `hints` setting and key-use counts | U1     | `ui-u5-bottom-bar`   |
| [U6](../done/u6-search-panel.md) | Search scope bar, footer summary, idle examples, preview, This Mac, no Kind                 | U3     | `ui-u6-search-panel` |
| [U7](../done/u7-palette.md)      | Palette sections, two-line rows, Commands token, ⇥ completion, recent commands              | U1, U2 | `ui-u7-palette`      |
| [U8](u8-sidebar.md)      | Sidebar twins, neutral disk bar, one fill per meaning, mark letters                         | U1     | `ui-u8-sidebar`      |
| [U9](u9-tips.md)         | One-time contextual tips                                                                    | U5     | `ui-u9-tips`         |
| [U10](u10-finish.md)     | Whole-window check against the assembled boards, docs pass, bench                           | all    | `ui-u10-finish`      |

Later, after U10 (not a U plan yet): a **motion pass**. Moving and trashing give feedback: an inline animation (for example a small trash in the row that swallows the entry) and an optional sound. User request, 2026-11-13.

Suggested order: U1 → U2 → U3 → U6 → U4 → U5 → U7 → U8 → U9 → U10. U1 and U2 need only PR #7 and can run side by side; U3, U4, U5 and U8 only need U1 and can go in any order.

## Shared names and who builds them
Plans refer to each other by these names. Whichever plan lands first builds the shared piece with exactly this shape; the later one reuses it. If merged code differs, use the merged name and record the mapping in the progress file.

| Piece | Name | Built by (fallback) |
| --- | --- | --- |
| Keys from the live keymap | `keys::{KeyScope, binding_for, caps, chord_text, spoken, text}` | U1 |
| Keycaps | `chrome::{Keycap, KeycapWeight::{Outline, Key, Text}}` | U1 |
| Tooltips | `chrome::{with_tooltip, with_action_tooltip, tooltip_text}` | U1 |
| Overlay container | `chrome::{sheet, sheet_panel, SheetWidth::{Compact, Narrow, Wide}}`, `style::scrim` (window.rs's private `enum Sheet` is a different thing) | U1 |
| SVG icons | `icons::{Glyph, glyph, Assets}` (`style::icon` stays the Finder file icon) | U1 |
| Selection tints | `style::{active_selection, inactive_selection, current_location, cursor_ring, drop_target}`; `cursor_ring` is an inset shadow and never changes layout | U1 |
| Command registry with menus | `palette::Command { menu: Option<MenuSlot>, palette, action, scope }`, `menus::{app_menus, init}` | U2 |
| Open search with a query | `CrowWindow::open_search(SearchStart { folder, query, everything })` | U2 (U3, U4 reuse) |
| Highlighted substrings | `style::highlight_text(text, ranges, theme, on_selection)` | U6 (U4 if first) |
| Fuzzy match ranges | `crow_search::FuzzyMatcher::indices` → byte ranges | U6 (U7 if first) |
| Hints setting and use counts | `crow_core::HintsMode`, `Settings.hints`; `usage::{uses, record, record_key, count_key}` | U5 (U9 reuses) |

Known gaps the plans raise for the user:
- Finding 6 ("dead quarter", empty space right of the last column) has no owner in U1–U9. U10 proposes building it (its Q2).
- Board-only details (line icons in the sidebar, a 220 px sidebar, syntax-coloured preview, 196 px columns) are deferred unless the user asks (U10 Q3, U8 Q8).

## Questions
Every plan ends with "Questions to ASK", each with a recommended answer. The implementing agent asks them in one batch when it starts that plan (per `execute-plan`), so answers can use what earlier plans revealed. Don't answer them all up front.

## Answers from the user (2026-11-12)
Each plan restates the answers it depends on.
- Q1 Branch: after PR #7 merges, before M7. (Split into one PR per plan on 2026-11-12.)
- Q2 Icons: embedded SVGs (an `AssetSource` with `include_bytes!`, rendered with GPUI `svg()`, tinted by theme role).
- Q3 Search field gets **All / Names / Contents**, default All (= M6 behavior). Names skips reading contents; Contents hides name-only hits.
- Q4 `hints = "auto" | "always" | "off"` in `settings.toml`, default `auto`: each idle hint retires after its key has been used 3 times (counts in `~/.local/state/crow/`).
- Q5 Rename the Everywhere scope to **This Mac**.
- Q6 Drop the Kind column from search results.
- Q7 Palette: add **⇥ completion**; **remember recent commands across launches** (state dir), hidable with `palette_recent_commands` in `settings.toml`. The user wants an in-app **settings page** eventually (not in these plans): keep new options flat, typed and documented so a page can list them.
- Q8 Edit ▸ Find Everything… (⇧⌘A) with no search open: open search in the current folder with every rule set off.
- Q9 Contextual tips: included (U9).
- Q10 Path editing stays with the bottom path bar and is decided in M7.

## Ground rules for every U plan
- `AGENTS.md` applies in full; restated here because these plans are all UI.
- Theme roles only (`theme.role("…")`); no hex colors except Finder's icon colors already in use. New tints go into `style.rs` as named helpers (U1) and every surface uses them.
- Every interactive element has a role and an aria label; tooltips never replace aria labels.
- Every new key behavior gets a keystroke test in `crates/crow-app/src/window.rs` (`ui::init_for_tests`).
- Config options: documented default written on first launch, live reload, errors in the status line, Crow's edits via `crow_core::edit_toml`.
- Plans that touch per-frame rendering of the pane (U1, U4, U5) run the bench (`--bench target/fixture-100k`) and compare with `main`. U8 runs it once and records the numbers (the sidebar renders every frame, but no pane rows change).
- Each plan's PR updates README and `docs/testing.md` for what it changed. Before every commit: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`. Don't merge.
- Match the canvas boards named in each plan. When a board and the plan disagree, the plan wins; when the plan is silent, ASK.
