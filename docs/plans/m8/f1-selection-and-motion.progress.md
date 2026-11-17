# F1 — Selection and motion progress

Branch: `m8-f1-selection` · PR: none · Last updated: 2026-11-17 · State: in progress

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 Delete selects the next row | todo | Plan: `folder.rs` `finish_apply`/`prune_selection`; cursor moves to the row now at the old index, last row if it was last. |
| 2 Finder-style selection islands | todo | `pane.rs` list and column rows; `selection_islands` helper; one fill per contiguous run; 1 px rule inside a run. |
| 3 Complete the row-removal animation | todo | Audit the trash/move FLIP, then a leaving effect for watcher/permanent removals with no operation effect. |
| 4 Copy/cut feedback | todo | Flash copied rows; dim cut rows from `Operations.clipboard`; clear on paste, undo, Escape, new copy/cut. |
| 5 Sidebar current-folder fill glides | todo | Full: glide old→new row; Low: fade; Off: jump. No layout shift. |
| 6 Tip card position | todo | Must not cover the list's last rows; ask Q5 before choosing inset vs move. |
| 7 Finish: checks, bench, bundle, docs | todo | fmt, clippy, workspace tests, bench vs `main`, bundle hand checks, `docs/testing.md`. |

## Questions

- Q1 (asked 2026-11-17): Cursor ring on the island or the row? The pane rows draw no `cursor_ring` today (the cursor is the active selection fill and the glide; the U1 board shows a solid fill with no outline). → answer: pending.
- Q2 (asked 2026-11-17): Copy flash under `motion = "off"`? Recommended: no flash under Off; cut dimming stays (state, not motion). → answer: pending.
- Q3 (asked 2026-11-17): Does Escape clear the cut dim? Recommended: yes; clear the dim but keep the clipboard so Cmd-V still moves. → answer: pending.
- Q4 (asked 2026-11-17): Sidebar glide in this piece? Recommended: yes. → answer: pending.
- Q5 (asked 2026-11-17): Tip position: reserve a bottom inset or move the card? The card's 280 px minimum does not fit over the 210 px sidebar. Recommended: reserve a bottom inset only while a tip is visible. → answer: pending.
- Q6 (asked 2026-11-17, M8 index Q2): Rubber-band selection in F1 or defer? Recommended: defer (F1 already changes selection rendering; rubber-band adds pointer geometry and auto-scroll). → answer: pending.

## Decisions

- Base `m8-f1-selection` from `main`; one branch for F1 — repo rule (AGENTS.md, M8 index Q3), not asked.
- The cursor row must stay selected (the pane's fuzz invariant checks it). When a removal drops the cursor row and other selected rows remain, the next row becomes the cursor **and joins the selection**; the remaining rows stay selected. "Move only the cursor" cannot leave the cursor on an unselected row without breaking the invariant.
- `Fx::Trash` is the leaving effect for a watcher/permanent removal: it already has the Full swallow + close and the Low fade, and the plan's "a trash still uses `Fx::Trash`" test implies the same effect is reused. No new motion-table variant.

## Hand checks

- [ ] Delete a middle row, a last row, a multi-selection, and a watcher deletion (delete from Terminal): the next/previous row is selected, not nothing.
- [ ] Multi-selection islands: two adjacent and three adjacent rows are one box with faint rules; two islands separated by an unselected row are two boxes; light and dark; focused and unfocused pane; list and column view.
- [ ] Full motion, 20-row folder: delete a middle row and watch the rows close; delete the last row.
- [ ] Copy rows: one short tint pulse. Cut rows: dim until paste/undo/Escape/new copy. 100k copy stays responsive.
- [ ] Sidebar: click folders quickly in Full (fill glides, one fill), Low (fade), Off (jump).
- [ ] Tip: at 640 px and wide, light and dark, the list's last row is not under the card.
- [ ] VoiceOver: row roles, `aria_selected`, and the current-folder description stay correct.

## Found along the way

- None yet.

## Log

- 2026-11-17: Read AGENTS.md, the M8 index, the F1 plan, `folder.rs`, `pane.rs`, `sidebar.rs`, `tips.rs`, `motion.rs`, `selection.rs`, and the tip/glide tests. Created the branch and this progress file. Asked Q1–Q6.

## Next

Ask Q1–Q6, then start item 1.
