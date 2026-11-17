# F4 path bar progress
Branch: m8-f4-path-bar · PR: none · Last updated: 2026-11-17 · State: In critique

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 Drag folders from the path bar | done | every segment and the folded "…" drag `DraggedSelection` with one entry; rows/search now share `drag_combo`/`promote_drag`; tests cover the payload, no-navigate, the field having no drag, and the folded target |
| 2 Discoverable edit affordance | done | the V2 hover pill, the always-visible pencil at the path's end, single click edits, ⌘L and double-click kept; the 32 px field spans the left section and expands from the pill (Q5); the pill/pencil springs (Q6); the muted completion suffix; tests cover the pencil, no layout shift, the field hover reset, the deactivation reset and the completion suffix |
| 3 Windows Explorer-style option (design only) | done | `path_bar = "crow" \| "explorer"` design recorded in `docs/plans/m9/README.md` under P9; no code |

## Questions
- Q1 (asked 2026-11-17): folded `…` segment drags the deepest folder it stands for, or is non-draggable? → answer: deepest (user).
- Q2 (asked 2026-11-17): pencil on hover, hover-only background, or both? → answer: both (user).
- Q3 (asked 2026-11-17): single click on empty path space edits the path? → answer: yes (user).
- Q4 (asked 2026-11-17): internal drops from a path segment onto a pane/sidebar row? → answer: allow (user).
- Q5 (asked 2026-11-17, critique): the field expands from the pill's width (Q2 A) or from 0? → answer: from the pill's width (user).
- Q6 (asked 2026-11-17, critique): keep the 120 ms mount flash or use a spring that starts at rest? → answer: spring (user).

## Decisions
- Base branch `m8-f4-path-bar` from `main` (4c0a4b7) — plan — plan.
- Reuse `DraggedSelection` with one `(path, is_dir)` entry for a segment drag — keeps internal drops and native promotion identical to rows — plan + user (Q4).
- The last segment drags the entry it shows, which can be a file; middle segments and the folded "…" are folders. Matches the segment's click (open the file / go to the folder) — code convention; the plan's "folder" wording assumes the usual folder path.
- The pencil and the empty-space fill share one `group_hover("path-area")`, so hovering any part of the path shows both — user (Q2 "both").
- `drag_combo` and `promote_drag` moved out of the two row renderers into `pane.rs` helpers; rows, search results and the path bar use them — code convention, avoids a third copy of the native-promotion block.
- Explorer-style bar is designed here and built in M9 P9 — user, 2026-11-17 (plan).
- The V2 shape (38 px bar, 32 px pill, the pencil at the path's end, the 32 px field across the left section) — user, 2026-11-17 (f4-fix Q1-Q4); the fix2 motion and completion are in `f4-fix2.progress.md`.
- The field expands from the pill's width; the pill and pencil use springs (start at rest) — user, Q5/Q6.
- Hover clears on pointer exit (GPUI `MouseExited` patch) and on window deactivation (new GPUI patch); the observer resets `path_hovered` — user Q1 A + the fix2 hand check.

## Hand checks
- [ ] Drag a path segment to Finder: the folder or file arrives, not text and not the selection.
- [ ] Click a segment: navigates; a drag does not also navigate.
- [ ] Hover the path: the pill appears around the path and pencil; the pencil brightens; a single click on the empty space or the pencil edits; ⌘L and a double-click still edit.
- [ ] Open the field: it expands from the pill's width to the left section; the hints fade in; the pill/pencil do not flash on mount or after the field; motion Off is instant.
- [ ] The completion suffix shows muted after the text; Tab applies it; a complete path has none; a mid-text caret has none.
- [ ] Move the pointer out of the window and Cmd-Tab: the pill and the segment fills clear.
- [ ] Drop a segment on a pane row or a sidebar favorite: the folder moves like a row drag.
- [ ] Folded `…` segment: click goes to the deepest folder; drag drags that folder.
- [ ] While the path field is open, dragging selects text and starts no file drag.
- [ ] 640 px and wide, light and dark: bar height and segment positions unchanged.
- [ ] VoiceOver: the pencil reads "Edit path"; the segments keep their labels.

## Found along the way
- none yet

## Log
- 2026-11-17: branch created; read the plan, the M8 index, `bottom_bar.rs`, the drag plumbing in `pane.rs`, the path-edit code and tests. Four plan questions batched for the user.
- 2026-11-17: item 3 done (M9 P9 design block), committed 286dcd7.
- 2026-11-17: answers Q1 deepest, Q2 both, Q3 yes, Q4 allow.
- 2026-11-17: implemented segment drags (normal + folded), the hover pencil/fill and single-click edit; added `Glyph::Pencil`; extracted `drag_combo`/`promote_drag`; updated tests, README and `docs/testing.md`.
- 2026-11-17: `cargo test -p crow-app path_bar` and `folded_path_segment` pass (9 + 1 tests).
- 2026-11-17: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` pass (500 crow-app tests, 0 failed).
- 2026-11-17: bench on `target/fixture-100k`, branch vs `main` (4c0a4b7) in a worktree: branch cold first paint 153 ms, warm 11 ms, p99 8.59 ms; main 143 ms, 11 ms, 8.64 ms. Within the AGENTS.md budgets and within run-to-run noise (warm paint and p99 match; cold paint differs by 10 ms in one run).
- 2026-11-17: `./scripts/bundle.sh` built `target/bundle/Crow.app`. Screenshots `target/shots/f4-path-bar-firebox.png` (rest: segments unchanged, bar 30 px), `target/shots/f4-path-bar-hover.png` (Firebox dark) and `target/shots/f4-path-bar-hover-light.png` (Solarized Light): both hover shots show the `surface-raised` fill on the empty space and the pencil at its right edge, with the segments unmoved.

## Next
Ready for the user's test; the PR opens now. Q5/Q6 are answered and built; the hand checks above are the test.

## Critique
Critic session: 2026-11-17 · Base: `main` @ 4c0a4b7 · Head reviewed: 143fbbe

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | The empty space fills `surface-raised` as one large slab with a hard left edge; hovering any segment fills it too (one `group_hover` on `bar-path`), so the bar has two hover languages | `img/f4-current-hover.png`; `crates/crow-app/src/bottom_bar.rs:735-745` | UX / UI | ask | fixed by the V2 build: the hover pill around the path, no empty-space fill |
| C2 | The pencil sits at the far right of the path slot, next to the counts; it reads as a bar-level action, not "edit this path" | `img/f4-current-hover.png`; `bottom_bar.rs:706-735` | UX | ask | fixed by the V2 build: the pencil at the end of the path |
| C3 | A single click on the empty space edits the path; the empty space is most of the bar and nothing says so at rest (F4 Q3, user) | `bottom_bar.rs:750-760` | UX | ask | user kept "edits"; the V2 pill shows the target on hover |
| C4 | The separator is inside the segment's hover/click box; hovering a segment paints the chevron with it | `bottom_bar.rs:668-680` | UI | note | fixed by the V2 build: the separator is a sibling |
| C5 | The edit field is 24 px, 5 px radius, 6 px padding with a 2 px ring; next to the 32 px, 8 px search field it reads like a command line | `target/shots/f4-path-edit-firebox.png`; `bottom_bar.rs:614-638` | UI | note | fixed by the V2 build: 32 px, 8 px radius, 12 px padding, the left section with the hints inside |
| C6 | The hover test checks the pencil's bounds and click, not that it is hidden at rest; `debug_bounds` cannot read opacity | `crates/crow-app/src/window/tests.rs:5790-5815` | code / test | note | fixed by the V2 build: the pencil is always visible; the test checks its bounds and click |
| C7 | The drag test asserts the `DraggedSelection` entries, not the external file URLs (`promote_drag` is shared with rows and hand-checked) | `window/tests.rs:5740-5760` | code / test | note | left; the hand check covers Finder |
| C8 | The last segment drags the file it shows; the plan says "folder". Recorded as a Decision (code convention), and it matches the segment's click | `bottom_bar.rs:671-690`; progress Decisions | plan | note | ask if folder-only is wanted |
| C9 | The bar shows the cursor's path, so the last segment is the selected item, not the current folder (H6 C-Q1, user). The design's `fg` last segment can read as "you are here" | `crates/crow-app/src/window.rs:1173`; H6 C-Q1 | UX | note | keep the user decision; the V2 keeps the last segment `fg`; hand check |

Checks re-run: `cargo fmt --check` ✓ · `cargo clippy --workspace --all-targets -- -D warnings` ✓ · `cargo test --workspace` ✓ (500 app + 1 ignored, 99 core, 44 search, 6 fs, 20 macos, 6 theme) · bench on `target/fixture-100k`: cold 156 ms, warm 11 ms, p99 8.63 ms (main 143 / 11 / 8.64; the F4 branch 153 / 11 / 8.59); within the AGENTS.md budgets and run-to-run noise · `./scripts/bundle.sh` built `target/bundle/Crow.app`.
Looked at: the design mockups (six options in `docs/plans/m8/img/`), the V2 build's tests at 1280 and 640, the bench. Not hand-driven yet: the user tests the bundle; the drag-to-Finder, VoiceOver and light/dark checks are on the hand-check list.
Verdict: the drag work is solid; the V2 build replaces the slab/pencil/field with the pill, the path-end pencil and the left-section field. The user's test decides whether the shape is right.

### Critique 2 (fix 1 + fix 2)

Critic session: 2026-11-17 · Base: `main` @ 4c0a4b7 · Head reviewed: c3244cf, then this session's fixes

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C10 | With motion on, the pencil fades to transparent at rest: the animation paints `fg` with alpha `1-t`, so the "always visible" pencil disappears after 120 ms (and flashes `fg` on first paint); the docs and the user's decision say always visible | `bottom_bar.rs:717-735`; `docs/testing.md:370` | UI / plan | fix | fixed (779e493): rest `fg-subtle` → hover `fg`, no transparency; the rest screenshot shows it |
| C11 | The field's hints pop in at full opacity; the plan (Q2 A) says they fade in with the field | `bottom_bar.rs:970-985` | UI / plan | fix | fixed (779e493): the hints fade over `FIELD` with the field; motion Off is instant |
| C12 | The pill's hover state sticks after the field closes: the field replaces the path's `on_hover`, so a pointer that leaves while editing never sends `PathUnhover`; Escape brings the pill back wherever the pointer is | `window.rs:622-627`; `bottom_bar.rs:791-809` | UX / code | fix | fixed (779e493): `close_path_edit` resets `path_hovered`; test `path_bar_field_clears_the_hover` |
| C13 | The U5 board note still says 30 px; the fix plan asks for the note | `docs/plans/done/u5-bottom-bar.md:63-67` | plan | fix | fixed (779e493): a note records the 38 px bar and the 32 px pill/field |
| C14 | README and `docs/testing.md` say a segment drags "the folder"; the last segment can be a file (the recorded Decision) | `README.md:107`; `docs/testing.md:370` | plan / docs | fix | fixed (779e493): "the folder or file it shows" |
| C15 | The field expands from 0 width, not from the pill's width as Q2 A decided; the path text vanishes and the field sweeps from the left | `bottom_bar.rs:1015-1023`; `f4-fix2.md` Q2 A | UX / plan | ask | fixed (b3fc089): the field expands from the pill's width to the left section; `BarState.pill_width`/`field_width`; Q5 |
| C16 | The pill and pencil animations restart at their visible state on mount: the pill flashes `surface` and the pencil `fg` for 120 ms on first paint and after the field closes; `with_animation` cannot start a two-state fade at rest | `bottom_bar.rs:760-785` | UX / code | ask | fixed (b3fc089): the pill and pencil use springs (constant ID, start at rest); no mount flash; the `HOVER` constant is gone; Q6 |
| C17 | Hover survives the app losing focus: the GPUI patch clears on `MouseExited` only; Cmd-Tab or clicking another app leaves the segment fills and the pill; the fix2 hand check asks for focus loss | `patches/gpui-hover-on-deactivation.patch`; `window.rs:366` | UX / plan | fix | fixed (779e493): the new patch clears the hit test on deactivation; the observer resets `path_hovered`; tests `window_deactivation_clears_the_hover_hit_test` and `path_bar_hover_clears_when_the_window_deactivates`; the inactive screenshot has no pill or segment fill |
| C18 | The first segment's width guard was loosened 20 → 14 px; the segment is 16 px at 700 px with a long progress line. The separator moved out of the segment, so the old 20 included it; 16 px still fits the 14 px icon, and the name is clipped | `window/tests.rs:2024` | code / test | note | left: the separator move explains it; the icon stays |
| C19 | The drag test asserts the internal `DraggedSelection`, not the external file URLs; `promote_drag` is shared with rows and hand-checked | `window/tests.rs:5735` | code / test | note | left: the Finder hand check covers the external payload (C7) |
| C20 | The last segment drags the file it shows, not a folder; recorded as a Decision and matches the segment's click | `bottom_bar.rs:660`; progress Decisions | plan | note | left: the user can ask for folder-only |
| C21 | The completion preview has no debounce: every keystroke reads the parent dir; the plan allowed a short debounce if the reads are hot, and no measurement is recorded | `window.rs:723-746` | code / performance | note | left: background reads; the preview drops stale results; the hand check types a path |
| C22 | The bar shows the cursor's path, so the last segment is the selected item, not the current folder (H6 C-Q1, user) | `window.rs:1205` | UX | note | left: the user decision stands |
| C23 | The fix2 bench record is invalid: p99 33.68 ms and max 55.9 s include the runtime Metal shader compile; the plan asks for a bench against `main` | `f4-fix2.progress.md` Log | code / performance | note | re-ran frontmost: cold 144 ms, warm 10 ms, p99 8.55 ms, max 16.3 ms (main 143/11/8.64; within budget) |

Checks re-run: `cargo fmt --check` ✓ · `cargo clippy --workspace --all-targets -- -D warnings` ✓ · `cargo test --workspace` ✓ (502 app + 1 ignored, 99 core, 44 search, 6 fs, 20 macos, 6 theme) · GPUI from `/tmp/gpui-ws`: `mouse_exit_clears_the_hover_hit_test` ✓, `window_deactivation_clears_the_hover_hit_test` ✓ · bench on `target/fixture-100k` frontmost: cold 144 ms, warm 10 ms, p99 8.55 ms, max 16.3 ms (main 143/11/8.64; within the AGENTS.md budgets) · `./scripts/bundle.sh` ✓. The Q5/Q6 bar changes are in the final bundle.
Final bench after Q5/Q6: cold 119 ms, warm 9 ms, p50 8.33 ms, p95 8.48 ms, p99 8.72 ms, max 64.2 s (the first-frame runtime Metal shader compile; the 599 scroll frames' p99 is within budget; main 143/11/8.64).
Looked at: Firebox dark at 1200/1254/1280 px: rest (pencil visible), hover (pill and pencil), the open field (32 px, hints inside, counts outside), and inactive after Finder takes focus (no pill, no segment fill). The completion suffix is covered by the test; the field opens with the full path selected, so the shot has none. The final field start and the springs are code-verified and in the bundle; the motion itself is the user's test. Not looked at after the fixes: Solarized Light, 640 px, VoiceOver.
Verdict: better. The pencil is always visible, the hints fade, the field reads as the search field, hover no longer sticks after the field or focus loss, the field expands from the pill, and the springs start at rest. The user's hand test decides the motion.

