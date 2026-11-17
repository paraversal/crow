# F1 — Selection and motion progress

Branch: `m8-f1-selection` · PR: none · Last updated: 2026-11-17 · State: Ready for critique

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 Delete selects the next row | done | `Folder.cursor_before_apply` + `select_next_after_removal`; covers trash, move, permanent delete, watcher and rescan. Tests: middle, last, multi, watcher cursor, external non-cursor, filtered, empty, inline child. |
| 2 Finder-style selection islands | done | `pane::selection_islands` + `island_edges`; list and column rows round the island ends and draw a 1 px `border` rule inside a run; the glide fill adopts the cursor row's island rounding. Tests: helper units and a render-bounds test for 1/2/3 rows and two islands, with no layout shift. |
| 3 Complete the row-removal animation | done | Audited the trash/move FLIP: operation effects already cover those. Watcher/permanent removals now start `Fx::Trash` at the old index when the batch has no additions (a rename is skipped); Low draws the ghost fade, Off starts nothing; the clear timer waits for the longest effect. Tests: shift non-zero→zero, reorder/rename/Off no effect, an existing effect is not duplicated, Low fades without moving. |
| 4 Copy/cut feedback | done | `Clipboard` carries the copy flash start, the cut dim flag and a path set. List, column and search rows flash copied rows (accent tint, `COPY_FADE`, none under Off) and dim cut rows (`CUT_DIM` on icon/text; the fill stays full). Escape/undo/paste/a new copy clear the dim; the 100 ms poll drops the clipboard when another app copies. Tests: flash state + render, cut dim, Escape/undo/paste, search results, 100k set lookup. |
| 5 Sidebar current-folder fill glides | done | One absolute fill behind the rows, targeted from the last prepaint's row bounds. Full springs old→new, Low fades in on the new row, Off jumps. The first layout requests one next-frame refresh so the fill appears. Tests: Full start/land, fast sequence, Off, Low, height match. |
| 6 Tip card position | done | The list and column bodies reserve a 64 px bottom inset (`tips::INSET`) while a tip is visible; the card stays bottom-right. `Pane.tip_inset` is set/cleared by `offer_tip`, `close_tip` and the render-time close. Test: at the bottom of a 250-row folder, the last row is above the card. |
| 7 Finish: checks, bench, bundle, docs | done | `fmt --check`, clippy, `test --workspace` (471 app tests + `fuzz_smoke`), bench vs `main` (warm 9 vs 10 ms, p99 8.63 vs 8.61 ms), bundle built, `docs/testing.md` updated. Visual checks: single and 3-row islands, cut message; the rest are listed for the critic. |

## Questions

- Q1 (asked 2026-11-17): Cursor ring on the island or the row? The pane rows draw no `cursor_ring` today (the cursor is the active selection fill and the glide; the U1 board shows a solid fill with no outline). → answer: **no ring**; keep the fill/glide (user).
- Q2 (asked 2026-11-17): Copy flash under `motion = "off"`? Recommended: no flash under Off; cut dimming stays (state, not motion). → answer: **no flash under Off**, cut dimming stays (user).
- Q3 (asked 2026-11-17): Does Escape clear the cut dim? Recommended: yes; clear the dim but keep the clipboard so Cmd-V still moves. → answer: **clear the dim, keep the clipboard** (user).
- Q4 (asked 2026-11-17): Sidebar glide in this piece? Recommended: yes. → answer: **yes, in F1** (user).
- Q5 (asked 2026-11-17): Tip position: reserve a bottom inset or move the card? The card's 280 px minimum does not fit over the 210 px sidebar. Recommended: reserve a bottom inset only while a tip is visible. → answer: **reserve a bottom inset while a tip is visible** (user).
- Q6 (asked 2026-11-17, M8 index Q2): Rubber-band selection in F1 or defer? Recommended: defer (F1 already changes selection rendering; rubber-band adds pointer geometry and auto-scroll). → answer: **defer**; wherever it is planned, also include Cmd-drag to add multiple items to a selection (user).
- Q7 (asked 2026-11-17): Copy/cut feedback in search results? → answer: **yes**, same flash/dim (user).

## Decisions

- Base `m8-f1-selection` from `main`; one branch for F1 — repo rule (AGENTS.md, M8 index Q3), not asked.
- The cursor row must stay selected (the pane's fuzz invariant checks it). When a removal drops the cursor row and other selected rows remain, the next row becomes the cursor **and joins the selection**; the remaining rows stay selected. "Move only the cursor" cannot leave the cursor on an unselected row without breaking the invariant.
- `Fx::Trash` is the leaving effect for a watcher/permanent removal: it already has the Full swallow + close and the Low fade, and the plan's "a trash still uses `Fx::Trash`" test implies the same effect is reused. No new motion-table variant.
- The plan's Finish says push; AGENTS.md says the executing agent commits locally and stops at Ready for critique, and the critic opens the PR. Followed AGENTS.md: no push, no PR.
- No cursor ring on pane rows (Q1): the active fill and the glide already show the cursor, and the U1 board has no outline.
- Copy/cut feedback also applies to search results (Q7); one clipboard read, no second source of truth.
- Rubber-band selection stays deferred (Q6). When it is planned, include Cmd-drag additive selection.

## Hand checks

- [ ] Delete a middle row, a last row, a multi-selection, and a watcher deletion (delete from Terminal): the next/previous row is selected, not nothing. (Tests cover it; the bundle check needs a temp folder.)
- [x] Multi-selection islands: three adjacent rows are one box with faint rules (bundle, dark). Still to check: two adjacent, two islands, light theme, unfocused pane, column view.
- [ ] Full motion, 20-row folder: delete a middle row and watch the rows close; delete the last row.
- [x] Cut rows: the status says "Cut 3 items: paste to move" and the fill stays full (bundle, dark; the dim is subtle). Still to check: the copy pulse, Escape/undo/paste clearing, search results, 100k copy.
- [ ] Sidebar: click folders quickly in Full (fill glides, one fill), Low (fade), Off (jump). (Render test covers it.)
- [ ] Tip: at 640 px and wide, light and dark, the list's last row is not under the card. (Render test covers the default width.)
- [ ] VoiceOver: row roles, `aria_selected`, and the current-folder description stay correct.
- [ ] 100k fixture and an empty folder in both themes; Vim on and off; column view.

## Found along the way

- An external `sidebar.toml` reload updates the model (`sidebar_looks` sees the new rows) but the rendered sidebar kept its old rows until a notify; the item 5 test forces one. Outside F1's scope: the app's `refresh_windows` may leave the sidebar's cached render stale after an external config edit. Reported here for F2/F7.

## Log

- 2026-11-17: Read AGENTS.md, the M8 index, the F1 plan, `folder.rs`, `pane.rs`, `sidebar.rs`, `tips.rs`, `motion.rs`, `selection.rs`, and the tip/glide tests. Created the branch and this progress file. Asked Q1–Q7; answers recorded.
- 2026-11-17: Item 1. `Selection::set_cursor` added; `Folder` remembers the cursor row before an apply and, when the apply drops it, selects the row now at that index (or the new last row). If other selected rows remain, the next row joins the selection so the cursor row stays selected. Tests updated where the old behavior asserted “nothing selected after trash”; new tests for middle/last/multi trash, watcher cursor/last/other, filtered, empty, inline child.
- 2026-11-17: Item 2. Islands in list and column view; the cursor glide fill takes the cursor row's island rounding so a multi-row island stays one box. No cursor ring (Q1). Render test asserts the top/bottom markers, the rule count and unchanged row bounds.
- 2026-11-17: Item 3. Audit: a trash/move starts its effect before the operation, the listing marks it removed, the ghost covers an early drop and `row_shift` closes the gap after the first phase; a watcher/permanent removal started nothing, so the gap jumped. Fixed: `start_removed_effects` starts `Fx::Trash` at the old row index/entry for a batch with removals and no additions (renames are skipped), capped at 20 leaving effects. `Ghosts` now renders in Low too (fade in place); Off starts nothing. `schedule_effect_clear` waits for the longest running effect, so a short effect cannot cut a longer one.
- 2026-11-17: Item 4. `Clipboard` gained `flash`, `dim` and a path set (one lookup per visible row). `motion::copy_flash` fades the tint over `COPY_FADE` (Off = 0). List, column and search rows draw the flash tint and dim cut content with `CUT_DIM`; the fill stays full. Escape, undo, a cut paste and a new copy/cut clear the dim; `ui::poll` drops the clipboard when the pasteboard changes. `notify_views` redraws every pane and search after a clipboard change.
- 2026-11-17: Item 5. The sidebar's current-folder fill is one absolute element behind the rows; its target comes from the last prepaint's row bounds (the prepaint listener now skips the fill). Full uses `with_spring`, Low fades in with `LOW_IN`, Off jumps. A first-layout next-frame refresh makes the fill appear on the second frame. The test covers Full start/land, a fast sequence, Off and Low.
- 2026-11-17: Item 6. The list and column bodies reserve `tips::INSET` (64 px) at the bottom while a tip is visible; the card stays bottom-right. `offer_tip`/`close_tip` and the render-time close set the pane's inset. The test scrolls a 250-row folder to the bottom and asserts the last row is above the card.
- 2026-11-17: Finish. `cargo fmt --check`, clippy (`-D warnings`) and `cargo test --workspace` pass (471 app tests, `fuzz_smoke`, `fuzz_smoke_edge_trees`). Bench on `target/fixture-100k`: branch warm 9 ms / p99 8.63 ms; `main` warm 10 ms / p99 8.61 ms (within run-to-run noise). `./scripts/bundle.sh` built and opened; the single-row and 3-row islands and the cut message were checked visually in the dark theme. `docs/testing.md` has the F1 hand checks. Remaining visual checks are in Hand checks for the critic.

## Next

Critique: the second agent runs the critique skill and opens the PR.
