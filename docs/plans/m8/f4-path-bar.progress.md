# F4 path bar progress
Branch: m8-f4-path-bar · PR: none · Last updated: 2026-11-17 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 Drag folders from the path bar | todo | Q1 (folded segment) and Q4 (internal drops) open |
| 2 Discoverable edit affordance | todo | Q2 (pencil vs background) and Q3 (single click) open |
| 3 Windows Explorer-style option (design only) | done | `path_bar = "crow" \| "explorer"` design recorded in `docs/plans/m9/README.md` under P9; no code |

## Questions
- Q1 (asked 2026-11-17): folded `…` segment drags the deepest folder it stands for, or is non-draggable? → pending
- Q2 (asked 2026-11-17): pencil on hover, hover-only background, or both? → pending
- Q3 (asked 2026-11-17): single click on empty path space edits the path? → pending
- Q4 (asked 2026-11-17): internal drops from a path segment onto a pane/sidebar row? → pending

## Decisions
- Base branch `m8-f4-path-bar` from `main` (4c0a4b7) — plan — plan.
- Reuse `DraggedSelection` with one `(path, true)` entry for a segment drag — plan recommends it; keeps internal drops and the native promotion path identical to rows — plan/code convention (final choice waits on Q4).
- The Explorer-style bar is designed here and built in M9 P9 — user, 2026-11-17 (plan).

## Hand checks
- [ ] Drag a path segment to Finder: the folder arrives, not text and not the selection.
- [ ] Click a segment: navigates; a drag does not also navigate.
- [ ] Hover the path area: the edit affordance appears; a single click on empty space edits (if Q3 is yes); ⌘L still edits.
- [ ] Folded `…` segment: behaves as chosen in Q1.
- [ ] 640 px and wide, light and dark: bar height and segment positions unchanged.
- [ ] VoiceOver: the new affordance and the segments read correctly.

## Found along the way
- none yet

## Log
- 2026-11-17: branch created; read the plan, the M8 index, `bottom_bar.rs`, the drag plumbing in `pane.rs`, the path-edit code and tests. Four plan questions batched for the user.

## Next
Record the Explorer-style design in the M9 index, then ask Q1–Q4.
