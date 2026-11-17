# F4 path bar progress
Branch: m8-f4-path-bar · PR: none · Last updated: 2026-11-17 · State: Ready for critique

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 Drag folders from the path bar | done | every segment and the folded "…" drag `DraggedSelection` with one entry; rows/search now share `drag_combo`/`promote_drag`; tests cover the payload, no-navigate, the field having no drag, and the folded target |
| 2 Discoverable edit affordance | done | hover pencil + `surface-raised` fill on the empty space, single click edits, ⌘L and double-click kept; tests cover the pencil's bounds/click, segment clicks, and no layout shift at 1280 and 640 |
| 3 Windows Explorer-style option (design only) | done | `path_bar = "crow" \| "explorer"` design recorded in `docs/plans/m9/README.md` under P9; no code |

## Questions
- Q1 (asked 2026-11-17): folded `…` segment drags the deepest folder it stands for, or is non-draggable? → answer: deepest (user).
- Q2 (asked 2026-11-17): pencil on hover, hover-only background, or both? → answer: both (user).
- Q3 (asked 2026-11-17): single click on empty path space edits the path? → answer: yes (user).
- Q4 (asked 2026-11-17): internal drops from a path segment onto a pane/sidebar row? → answer: allow (user).

## Decisions
- Base branch `m8-f4-path-bar` from `main` (4c0a4b7) — plan — plan.
- Reuse `DraggedSelection` with one `(path, is_dir)` entry for a segment drag — keeps internal drops and native promotion identical to rows — plan + user (Q4).
- The last segment drags the entry it shows, which can be a file; middle segments and the folded "…" are folders. Matches the segment's click (open the file / go to the folder) — code convention; the plan's "folder" wording assumes the usual folder path.
- The pencil and the empty-space fill share one `group_hover("path-area")`, so hovering any part of the path shows both — user (Q2 "both").
- `drag_combo` and `promote_drag` moved out of the two row renderers into `pane.rs` helpers; rows, search results and the path bar use them — code convention, avoids a third copy of the native-promotion block.
- Explorer-style bar is designed here and built in M9 P9 — user, 2026-11-17 (plan).

## Hand checks
- [ ] Drag a path segment to Finder: the folder arrives, not text and not the selection.
- [ ] Click a segment: navigates; a drag does not also navigate.
- [ ] Hover the path area: the pencil and the empty-space fill appear; a single click on empty space or the pencil edits; ⌘L and a double-click still edit.
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
Ready for critique. The critic builds the branch and walks the hand checks above, especially dragging a segment to Finder and dropping one on a pane/sidebar row.
