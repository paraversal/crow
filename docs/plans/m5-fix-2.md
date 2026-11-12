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

---

## Finish
`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, bench (`cargo run --release -p crow-app -- --bench target/fixture-100k`, record in `docs/testing.md`), `./scripts/bundle.sh`, update `docs/testing.md` hand-test rows and README, push and update the PR with each issue's cause and verification. Don't merge.
