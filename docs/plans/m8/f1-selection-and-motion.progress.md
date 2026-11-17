# F1 — Selection and motion progress

Branch: `m8-f1-selection` · PR: none · Last updated: 2026-11-17 · State: In critique

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 Delete selects the next row | done | `Folder.cursor_before_apply` + `select_next_after_removal`; covers trash, move, permanent delete, watcher and rescan. Tests: middle, last, multi, watcher cursor, external non-cursor, filtered, empty, inline child. |
| 2 Finder-style selection islands | done | `pane::selection_islands` + `island_edges`; list and column rows round the island ends and draw a 1 px `border` rule inside a run; the glide fill adopts the cursor row's island rounding. Tests: helper units and a render-bounds test for 1/2/3 rows and two islands, with no layout shift. |
| 3 Complete the row-removal animation | done | Audited the trash/move FLIP: operation effects already cover those. Watcher/permanent removals now start `Fx::Trash` at the old index when the batch has no additions (a rename is skipped); Low draws the ghost fade, Off starts nothing; the clear timer waits for the longest effect. Tests: shift non-zero→zero, reorder/rename/Off no effect, an existing effect is not duplicated, Low fades without moving. |
| 4 Copy/cut feedback | done | `Clipboard` carries the copy flash start, the cut mark flag and a path set. List, column and search rows flash copied rows (accent tint, `COPY_FADE`, none under Off) and mark cut rows with a dashed `fg` outline around the run at full contrast (`style::cut_outline`; the fill stays full). Escape/undo/paste/a new copy clear the mark; the 100 ms poll drops the clipboard when another app copies. Tests: flash state + render, cut outline in list and search, Escape/undo/paste, 100k set lookup. |
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
- Q8 (asked 2026-11-17, critique): After a removal drops the cursor row while other rows remain selected, should the next row join the selection? The plan says "move only the cursor"; the cursor-is-selected invariant needs a selected cursor row. → answer: **keep as built**: the next row becomes the cursor and joins the selection (user).

## Decisions

- Base `m8-f1-selection` from `main`; one branch for F1 — repo rule (AGENTS.md, M8 index Q3), not asked.
- The cursor row must stay selected (the pane's fuzz invariant checks it). When a removal drops the cursor row and other selected rows remain, the next row becomes the cursor **and joins the selection**; the remaining rows stay selected. "Move only the cursor" cannot leave the cursor on an unselected row without breaking the invariant.
- `Fx::Trash` is the leaving effect for a watcher/permanent removal: it already has the Full swallow + close and the Low fade, and the plan's "a trash still uses `Fx::Trash`" test implies the same effect is reused. No new motion-table variant.
- The plan's Finish says push; AGENTS.md says the executing agent commits locally and stops at Ready for critique, and the critic opens the PR. Followed AGENTS.md: no push, no PR.
- No cursor ring on pane rows (Q1): the active fill and the glide already show the cursor, and the U1 board has no outline.
- Copy/cut feedback also applies to search results (Q7); one clipboard read, no second source of truth.
- Cut feedback is a dashed outline, not a dim (user, 2026-11-17, critique): Cmd-X must look distinct from hidden files, so the plan's "dim cut rows" is superseded. `style::cut_outline` draws a 2 px dashed `fg` outline around each contiguous cut run; the icon, name and columns stay at full contrast and the selection fill stays full. `Clipboard.dim` is renamed `marked`.
- Rubber-band selection stays deferred (Q6). When it is planned, include Cmd-drag additive selection.

## Critique
Critic session: 2026-11-17 · Base: `main` @ 2accf3a · Head reviewed: c0f584f + 9b1fb3c

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | Cut rows looked exactly like hidden rows: `CUT_DIM` and `HIDDEN_DIM` were both 0.5 and both dimmed icon, name and columns. The user wants cut distinct. | `style.rs:76-82`, `pane.rs:1978`, `search.rs:1724` | UX | fix | fixed (9b1fb3c): a 2 px dashed `fg` outline around the cut run at full contrast, in list, column and search |
| C2 | After a removal drops the cursor row while other rows stay selected, the next row joins the selection. The plan says "keep them selected and move only the cursor"; the agent's Decision cites the cursor-is-selected invariant. | `folder.rs:1482-1508`, plan item 1 | plan | ask | Q8 → keep as built (user) |
| C3 | Item 2's test asserts marker selectors (`island-top-*`), not the rendered corner radii; the 1 px markers are invisible. | `window/tests.rs:4552`, `pane.rs:2874` | code | note | left: `island_edges`'s unit test covers the edges and the markers assert the same edges; GPUI has no style read for the row's radii |
| C4 | Executing agent ticked 2 of 8 hand checks; the plan's Finish requires the six items in light/dark, 640 px/wide, empty/100k, Vim on/off, VoiceOver. | progress Hand checks | honesty | fix | fixed (this review): the visual checks below; VoiceOver left for the user |
| C5 | An external `sidebar.toml` reload leaves the sidebar's cached rows (and the fill's target) stale until a notify. Pre-existing. | `sidebar.rs:1233` | code | note | left: outside F1, already in Found along the way for F2/F7 |

Checks re-run: `cargo fmt --check` ✓ · `cargo clippy --workspace --all-targets -- -D warnings` ✓ · `cargo test --workspace` ✓ (471 app, 96 core, 44 fs, 6 macos, 20 search, 6 theme; `fuzz_smoke` included) · bench branch warm 9 ms / p99 8.52 ms vs `main` @ 2accf3a warm 10 ms / p99 8.47 ms (budgets met; difference within run-to-run noise) · `./scripts/bundle.sh` ✓

Looked at: branch bundle with a temp config and state dir, FireBox dark and Solarized Light, 1200×760 and 640×600, list/column/search, an empty folder and `target/fixture-100k`, Vim on (`d d`) and off (menu Cut): the cut outline on a selected and an unselected row, a one-row and a two-row run, column and search; the copy pulse measured on the fill (+30 red over the selection fill, gone after `COPY_FADE`); trashing a middle row selected the next row; the tip inset with a 250-row folder scrolled to the bottom at 640 px (last row above the card); the empty folder clean; 100k loads. Not looked at: VoiceOver (the diff adds no roles or labels; the AX tree still exposes `selected` on the rows), the sidebar glide and the removal FLIP as motion (tests cover them; a static screenshot cannot show them).

Verdict: better — a cut item no longer reads as a hidden one, the outline holds when the row is unselected, and the rest of the piece matches the plan; no regressions seen in the flows around it.

## Hand checks

- [x] Delete a middle row in the bundle (dark): beta → delta selected. Last row, multi-selection and watcher deletion by tests.
- [x] Selection islands: one row and a two-row run in list, column and search, dark and light; three adjacent rows and two islands by tests.
- [ ] Full motion, 20-row folder: delete a middle row and watch the rows close; delete the last row. (Tests cover the shift; not watched as motion.)
- [x] Cut rows: a dashed outline on a selected and an unselected row, one-row and two-row runs, list/column/search, dark and light; Vim `d d` too. The copy pulse measured (+30 red over the fill, gone after `COPY_FADE`). Escape/undo/paste clearing and the 100k copy by tests.
- [ ] Sidebar: click folders quickly in Full (fill glides, one fill), Low (fade), Off (jump). (Render test covers it.)
- [x] Tip: 640 px, a 250-row folder scrolled to the bottom: the last row sits above the card.
- [ ] VoiceOver: row roles, `aria_selected`, and the current-folder description stay correct. (The diff adds no roles or labels; the AX tree still exposes `selected` on the rows.)
- [x] 100k fixture and an empty folder (dark); Vim on and off; column view. Light theme for the cut outline and the islands.

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
- 2026-11-17: Critique (9b1fb3c). C1: the cut dim was the hidden dim (`CUT_DIM` = `HIDDEN_DIM` = 0.5), so the user asked for a distinct look. Replaced it with `style::cut_outline`: a 2 px dashed `fg` outline around each contiguous cut run (one box per run, no rule between rows), full-contrast content, an absolute child so nothing shifts; list, column and search share `pane::island_edges`. `Clipboard.dim` → `marked`. Tests assert the outline appears and clears. Re-ran fmt/clippy/tests; bench branch 9/8.52 vs `main` 10/8.47; bundle checked in both themes, 640/wide, list/column/search, Vim, empty/100k. C2 asked (Q8: keep the next row joining the selection); C3 and C5 noted.

## Next

Critique: the second agent runs the critique skill and opens the PR.
