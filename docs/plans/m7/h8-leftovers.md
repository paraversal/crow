# M7 H8 — Leftovers from earlier critiques

Audience: an implementing agent. Follow the `execute-plan` skill; read `AGENTS.md` and `docs/plans/m7/README.md` first.
Base: branch `m7-h8-leftovers` from `main`. Progress file: `docs/plans/m7/h8-leftovers.progress.md`.
PR title: "M7 H8: leftovers (hints, rounding, palette ~, Open shortcut)". Don't merge.

## Why
Small items earlier critiques recorded as "left" or "follow-up". Each is minutes to an hour; together they're one PR. Re-check each on `main` first; drop any that's already gone, and say so.

## Items
### L1 Sidebar hints miss ⌥↑/↓ and → (U8 C6)
`done/u8-sidebar.progress.md` C6: with the sidebar focused, the bottom bar shows "⌥⌘↑↓ reorder · ⌘⌫ remove · ⇥ to list" but not the group jumps (⌥↑/⌥↓) or → to the list. Add them in the U5 hint list for sidebar focus, in priority order so `fit` drops the least useful first (Q1). Test: the hint text with the sidebar focused at 1280 px.
### L2 `short_bytes` rounds after choosing the unit (U8)
`sidebar.rs` `short_bytes` (≈ 174): 999 950 bytes → "1000 KB" instead of "1 MB". Round first, then pick the unit (or re-check after rounding). Unit test the boundaries (999 949, 999 950, 1 000 000; same at GB/TB).
### L3 Palette home row reads "~" twice (U7)
`done/u11-finish.progress.md` I9: the home row in ⌘K shows "~" as both name and path. Show "Home" (or the user's short name, as Finder does) as the name; keep "~" as the path. Test in `palette_rows`.
### L4 Open: ⌘↓ in the context menu, ⌘O in the menu bar (context-menu C4)
GPUI's menu bar takes the first binding, the hints the last (`actions.rs` ≈ 126). Make both show the same key. Q2.
### L5 Vim j/k block typing those letters (U7)
Recorded in `done/u11-finish.progress.md` I9. Find where (which field, which context) it still happens; if the Vim layer's bindings apply while a text field has focus, scope them to the list/sidebar contexts. Keystroke test: with the Vim layer on, typing "jk" into the palette, filter, rename and search inserts "jk".

## Finish
fmt · clippy · tests; bench not needed (no listing/scroll path; say so). Bundle; hand-check each item. Commit, push, "Ready for critique".

## Questions to ASK
1. **L1: which hints drop first when the bar is narrow?** Recommended: keep "⇥ to list" and "⌥↑↓ groups", drop reorder and remove first (they're in the menu and context menu).
2. **L4: which key wins for Open, ⌘↓ or ⌘O?** Recommended: ⌘↓ everywhere (Finder shows ⌘↓ in its File menu as "Open"), keep ⌘O bound but not shown.
3. **L3: "Home" or the account's short name?** Recommended: the short name, as Finder's sidebar does; fall back to "Home".
