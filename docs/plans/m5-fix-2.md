# M5-fix-2 — drag feedback, sidebar insertion, disclosure and expand-all

Audience: an implementing agent. Follow `.claude/skills/execute-plan/SKILL.md`; read `AGENTS.md` first.
Follow-up to `docs/plans/m5-fix.md` (PR #6, still open). Progress file: `docs/plans/m5-fix-2.progress.md`.
Continue on the `m5-fix` branch and update PR #6 unless the user picks a separate branch/PR.

This document lists **observed behavior and wanted behavior**. Causes below marked *hypothesis* are not yet measured; reproduce and record the cause in the progress file before fixing.

## Answers from the user (2026-11-12)
- Sidebar reordering flicker is the **double insertion target**: “above B” (bottom of A) and “below A” (top of B) both exist, so the bar jumps with fine movement. → handled by D2.
- The folder/file drag highlight is a **separate rendering problem**: the correct target is chosen, but its highlight flickers only **while the pointer moves**, not while it rests. Wanted: the would-be target stays lit the whole time the pointer is over it.
- The lagging drag image is the file icon from Crow's own list **and** the favorite label; both. Drags from Crow's list and from Finder are affected. Switching in-app drags to GPUI's drag is approved.
- After D4, Option-click does a **normal expand/collapse** (same as a click).
- Dropping a favorite on a group **title** stays: it moves the favorite to that group's start, shown by one line under the title.
- Continue on `m5-fix` and update PR #6.
- Future (explicitly not this plan): dropping an item on the sidebar should choose between “add as favorite” and “move into that folder” by cursor position, with Finder-like “+” tooltip feedback.

---

## D1 Drag feedback: target flickers, dragged file lags

- **Observed (two separate problems):**
  1. Reordering favorites shows two “between A and B” targets; the bar jumps with fine movement. That is D2, not this item.
  2. Dragging a folder/file over a row chooses the right target, but the highlight **blinks while the pointer moves** (steady while it rests). The drag image (file icon from Crow's list, favorite label in the sidebar) lags the cursor and runs at low fps. Both Crow→Crow and Finder→Crow drags.
- **Wanted:** the would-be target stays highlighted continuously while the pointer moves within it; the highlight changes only when the pointer enters another target. The drag image tracks the cursor smoothly at display rate. Drops keep working.
- **Where to look:** `crates/crow-app/src/pane.rs` (list rows still use `drag_over::<ExternalPaths>`; the pane also has its own mouse-drag start), `crates/crow-app/src/sidebar.rs` (`drop_target` computed in `render`), `crates/crow-macos/src/drag.rs` (`begin_drag` starts an `NSDraggingSession` immediately), `vendor/zed/crates/gpui/src/window.rs` (every mouse move during an active drag calls `window.refresh()`), `vendor/zed/crates/gpui_macos/src/window.rs` (`FileDropEvent::Pending` becomes a synthetic `MouseMove`).
- **Hypotheses to test, not diagnoses:**
  1. A frame is drawn between the position update and the drop-target computation, or the highlight is computed from the previous frame's hit test while the window is refreshed per drag event; the target is right but its style is absent for single frames while moving.
  2. The drag image lag is frame-time pressure: GPUI refreshes the whole window on every move while a drag is active, and Crow repaints the visible rows per refresh. The in-app drag image (GPUI-drawn favorite label, AppKit-drawn file icon) can only move as often as frames are produced.
  3. The file drag still starts a native AppKit session inside the window, so its image is AppKit's rather than GPUI's, and the app's main thread also handles `draggingUpdated` plus the synthetic mouse events.
- **Plan:**
  1. Measure first: instrument `FileDropEvent` kind/time, `window.mouse_position()`, computed target, refresh/frame count, and main-thread time per drag update for a Crow→Crow drag and a Finder→Crow drag; `sample crow` during a drag. Record whether the highlight drops to `None` for single frames and where the time goes.
  2. Make each surface's target single-sourced and frame-stable. List rows move from `drag_over` styles to pane state (like the sidebar), recomputed from the drag pointer and remembered row bounds; clearing happens only on `Exited`/`Ended`/no active drag. Add hysteresis at row boundaries so pixel jitter cannot alternate.
  3. Cut per-move work: avoid row/folder rebuilds and full-window refreshes when the target did not change; keep target lookup O(visible rows).
  4. Switch Crow→Crow dragging from the immediate native session to GPUI's internal drag with `external_drag_payload`, so the in-window image is GPUI's and the native session starts only when the pointer leaves the window. Keep drag-out to Finder/Mail, multi-item drags and Option-copy semantics; `ExternalPaths` remains the drop value so existing handlers keep working. Verify the frame-rate gain; if GPUI's per-move refresh still starves the image, profile the row repaint during a drag and reduce it before considering any vendored change.
- **Done when:** with the pointer moving within one row, a recording shows the row highlighted in every frame and no target change until a boundary; the drag image visibly follows the cursor (human check). A Crow→Crow drag still drops into folders, the sidebar and the Trash and still drags out to Finder and Mail; existing drop tests pass. Automated: list-view drop target state is constant across repeated moving-drag frames (same pattern as the sidebar test).

## D2 One insertion target between sidebar items

- **Observed (confirmed by the user):** between favorites A and B there are two targets: “above B” (the bottom half of A) and “below A” (the top half of B). The line visibly jumps between the two as the pointer moves between them.
- **Wanted:** exactly one insertion line between two rows, at one position, with one drop meaning; the line never shifts other rows. Dragging down still works, the end of a group is reachable, and dropping onto a group title still puts the favorite at that group's start.
- **Where to look:** `crates/crow-app/src/sidebar.rs` (two 13 px drop halves per favorite row with `border_t_2`/`border_b_2`, `SidebarEdit::Move`, the `layout` captured by `on_children_prepainted`), `crates/crow-core/src/sidebar.rs` (`Move` index rules).
- **Plan:** compute one insertion slot from the pointer against the remembered row bounds (boundaries: under a group title, between adjacent favorites, after the last; group-aware), render a single absolutely positioned 2 px line at that boundary once (not a border on a row), and use the same slot for the drop. Dropping a favorite on a group title remains and uses the line under the title (user's answer). Keep the row background for external file/folder drops; favorite drags show only the line.
- **Done when:** core tests still cover up, down by one, down by several, across groups and onto itself; a UI test asserts one line/state slot around a boundary and that dropping at it lands after the earlier row; hand check confirms no double target and no row shift.

## D3 Larger chevron disclosure

- **Observed:** the list-view disclosure is a tiny triangle.
- **Wanted:** a much larger chevron, clearly visible, that points right when collapsed and down when expanded; same behavior and keyboard support.
- **Plan:** replace the 10 pt triangle glyph with a chevron at 16 pt semibold (`›` / `⌄`) in a 20 px wide gutter, larger hit area covering the gutter, hover feedback, existing role/label and `disclosure-<name>` selector retained. Indent calculation follows the wider gutter. (Sizes are a starting point; make it visibly larger than today, matching the app's text size.)
- **Done when:** visual check in the app; the disclosure keystroke tests still pass and VoiceOver reads Expand/Collapse.

## D4 Remove Option-click expand-all (freeze)

- **Observed:** Option-click on a folder expands every folder and subfolder in the whole focused listing (not just the clicked subtree), which on dev trees (node_modules) freezes the app.
- **Wanted:** remove "expand everything below"; no unbounded recursive expansion. Regular Right-click/chevron expansion stays lazy (one folder on demand, listing loaded in the background as today).
- **Where to look:** `Folder.expand_all`, `sync_children` (it currently opens every directory of the folder that owns the click), the `option_clicking_a_triangle_expands_everything_below` test, README/`docs/testing.md`.
- **Plan:** delete the `expand_all` field and its cascade; Option-click then expands/collapses that one folder like a click (user's answer). Keep the existing manual expansion recursion limit (`MAX_DEPTH`) as a safety net. Update docs.
- **Done when:** Option-click cannot open a subtree; the app stays responsive on a dev-directory fixture; tests updated.

## D5 Drag image: Finder-like combo, one image at a time

- **Observed:** the in-window drag image is anchored at the grab point inside the row, so it sits near the original item instead of the cursor (image 2). After dragging out of the window and back in, the native drag image and the GPUI image are both visible.
- **Wanted:** the drag image is a small icon plus a pill with the item name (Finder’s “onhover combo”). It tracks the cursor with a fixed relative position, except when the cursor lies beyond the combo’s width: then the combo’s right side touches the cursor. Only one image is visible at a time.
- **Plan:** `DraggedFiles` renders icon (22 px) + name pill; the `on_drag` offset anchors it. Measure the combo width at prepaint and, when `cursor.x > width`, shift it so its right edge sits at the cursor. A thread-local flag is set in `external_drag_payload` (native promotion) and cleared at the next drag’s constructor; the restored GPUI view renders empty while the platform owns the drag.
- **Done when:** visual check of both grab positions (near the row’s icon and near the name) and a drag-out-and-back recording shows one image; drag-out still starts a native session.

## D6 Chevron rotation, not glyph swap

- **Observed:** the expanded chevron is a different glyph and sits lower.
- **Wanted:** expanding rotates the same chevron in place; its box and position do not move.
- **Plan:** paint the chevron as a stroked path (`canvas` + `PathBuilder`) in a fixed 14 px box; collapsed and expanded differ only in the vertex coordinates.
- **Done when:** visual check shows the same box; expand/collapse tests still pass.

## D7 Drag handoff: keep the combo, smooth the swap

- **Observed:** after drag-out-and-back the native icon is back but the name pill is gone. Crossing the window edge is jumpy: the GPUI combo is clipped by the window (the “pushes below the top edge” frame), then the native image appears; the native image can flash at the source row for a frame. The handoff is a hard swap.
- **Wanted:** one combo (icon + pill) throughout, including the native session, with a smooth handoff at the window edge and no flash at the source row. If the native image cannot carry the pill, say so plainly.
- **Where to look:** `crates/crow-app/src/pane.rs` (`DraggedFiles`, `external_drag_payload`, `PLATFORM_DRAG`), `crates/crow-macos` (no drag module today), `vendor/zed/crates/gpui` (`promote_external_drag_to_platform`, `restore_platform_drag`), `vendor/zed/crates/gpui_macos` (`start_external_drag`, which builds a 32 px `NSDraggingItem` at the mouse-down event).
- **Plan (answers: own native image, seamless swap, measured start, “N items”):**
  1. Preferred: own the native image. When a GPUI file drag leaves the viewport (detected from the drag’s own move events), start an `NSDraggingSession` from `crow-macos` with a composite image (icon + name pill, same look and anchor rule), then end the GPUI drag. Re-entry then shows that native combo and no second image; GPUI’s platform-created `ExternalPaths` drag has an empty view and keeps the drop targets working.
  2. Alternative: keep GPUI’s native image (plain icon) and only smooth the in-window side with a fade at the edge; the pill cannot follow outside the window.
  3. Anchor the native image at the cursor from the first frame (fixes the flash at the source row), and trigger the handoff before the GPUI combo would be clipped by the window bounds.
  4. Transition polish: choose from no animation (seamless swap), a short cross-fade, or a fade + tiny scale (see Q2). The combo should never visibly clip at the edge.
- **Done when:** dragging out and back shows the icon + pill the whole time (or the agreed fallback), no source-row flash, no clipped frame; drops and drag-out to Finder/Mail still work.

## D8 Combo start and attraction easing

- **Observed:** when the drag starts, the combo first appears near the source row and then slides very quickly to the cursor/anchor once the pill width is known. The attraction shift is jarring.
- **Wanted:** no visible first-frame jump. The combo appears already anchored (grab point or right edge at the cursor); if the anchor changes discontinuously, ease it.
- **Plan:** measure the pill before the first paint (text shaping at drag start) so the initial left is correct on frame one. Optional 100–150 ms ease only for the first placement / anchor flips, never for normal cursor tracking.
- **Done when:** a recording shows no slide from the row to the cursor at drag start; the anchor rule still tests green.

---

## Finish
`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, bench (`cargo run --release -p crow-app -- --bench target/fixture-100k`, record in `docs/testing.md`), `./scripts/bundle.sh`, update `docs/testing.md` hand-test rows and README, push and update the PR with each issue's cause and verification. Don't merge.
