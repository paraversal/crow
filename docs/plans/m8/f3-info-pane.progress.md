# F3 — Info pane progress
Branch: m8-f3-info · PR: none · Last updated: 2026-11-17 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 The surface | todo | waiting on Q1, Q2, Q6 |
| 2 Fields | todo | waiting on Q3, Q4; F6 (folder sizes) is not merged, so folders show `--` |
| 3 Metadata loading | todo | waiting on Q3 |
| 4 Reaching it | todo | waiting on Q5 |
| Finish: checks, bench, bundle, README, testing.md | todo | |

## Questions
- Q1 (asked 2026-11-17): second ⌘I closes the column or returns to Preview? → pending
- Q2 (asked 2026-11-17): with `show_preview = false`, does ⌘I open the column? → pending
- Q3 (asked 2026-11-17): permissions and owner/group in v1? → pending
- Q4 (asked 2026-11-17): multi-selection summary only, or a capped list? → pending
- Q5 (asked 2026-11-17): search results and the sidebar in scope? → pending
- Q6 (asked 2026-11-17): confirm the tab strip design (28 px, Preview/Info text tabs) → pending

## Decisions
- Info shares the right column with Preview as two tabs; not a separate Get Info window — M8 index (user, 2026-11-17).
- F6 (folder sizes) is not merged; the pane shows `--` for a folder's size and item count until it is, and the F6 field is wired when that piece lands — plan + current `main`.
- Stop at "Ready for critique" without pushing; the critic opens the PR — AGENTS.md (wins over the plan's "push").

## Hand checks
- [ ] A file: ⌘I opens Info; Name, Kind, Size, Where, Created, Modified, Added, Tags, Permissions, Owner/Group match Finder/`stat`.
- [ ] A folder: Kind "Folder", size `--` (F6 not merged), Items `--` or loaded count, Where navigates.
- [ ] A package: shown as a file-like package, not walked as a folder.
- [ ] A symlink: Kind/label and Where are right; no crash on a dangling link.
- [ ] Multi-selection: "N items", total size, common kind/parent/tags; 100k selection stays responsive.
- [ ] ⌘I, ⇧⌘P, tab clicks, menu item, palette command; tab choice survives a folder change and a relaunch? (tab choice is per window unless the plan says otherwise).
- [ ] Light/dark, 640 px and wide, VoiceOver reads the pane role and labels.
- [ ] Selection change while metadata loads never shows the old item's permissions.

## Found along the way
- (none yet)

## Log
- 2026-11-17: branch `m8-f3-info` from `main`; read the plan, M8 index, README, testing.md, and the preview/pane/actions/menus/palette/settings code. Asked Q1–Q6.

## Next
Ask Q1–Q6, then build item 1 (the `InfoPane` surface and tab strip) once the answers are in.
