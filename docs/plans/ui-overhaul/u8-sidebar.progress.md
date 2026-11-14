# u8-sidebar progress
Branch: `ui-u8-sidebar` · PR: none · Last updated: 2026-11-14 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 Row model and name cache | todo | |
| I2 Twin hints | todo | |
| I3 One fill per meaning | todo | |
| I4 Disk row | todo | |
| I5 Mark letters | todo | |
| I6 Alias marker | todo | Q3 = yes |
| I7 Favorites header hint | skipped | Q7 = no |
| I8 Center follows the sidebar cursor | todo | Q11 = yes, Q12 = replace, Q13 = yes |
| I9 ⌥↑/⌥↓ jump between groups | todo | User request from the U5 review |

## Questions
- Q1 (asked 2026-11-14): twin rules → answer: yes as written (all favorites groups + Recent, shown label, case-insensitive, same path twice is not a twin).
- Q2 (asked 2026-11-14): hint placement → answer: Discovery B (after the name, twins only).
- Q3 (asked 2026-11-14): aliases → answer: I6 yes, no merging of symlink and target in Recent.
- Q4 (asked 2026-11-14): unresolvable twins → answer: just the distinguishing ancestor name; abbreviated parent path only when no ancestor separates.
- Q5 (asked 2026-11-14): disk aria at warning → answer: append ", almost full".
- Q6 (asked 2026-11-14): free-space refresh → answer: also after a file operation finishes.
- Q7 (asked 2026-11-14): "⌃⌘T adds" header hint → answer: no; I7 skipped.
- Q8 (asked 2026-11-14): board-only differences → answer: keep 210 px, Finder icons, 3 px padding.
- Q9 (asked 2026-11-14): bench → answer: run once, record numbers.
- Q10 (asked 2026-11-14): current folder spoken → answer: yes, aria description "current folder".
- Q11 (answered 2026-11-13, in the plan): I8 scope → yes.
- Q12 (asked 2026-11-14): history while arrowing → answer: replace (one Back step for the walk; first move pushes the folder you were in).
- Q13 (asked 2026-11-14): → in the sidebar → answer: bind `right` to OpenSelection.

## Decisions
- Q7 = no: I7 is not built.
- U1 helper names as merged: `style::current_location`, `style::cursor_ring` (inset zero-blur shadow), `style::drop_target` (the plan's `drop_highlight` equivalent). U8 uses `drop_target`.
- Q12 = replace: `History::replace_current` plus a `Pane` flag; the first sidebar `Show` is a normal visit (so Back returns to the folder you were in), later ones replace the current entry.
- Q10 = yes: the current row gets `aria_description("current folder")`.
- Q6 = yes: `operations::finish` calls `ui::refresh_volumes`.
- I9 action names: `PreviousGroup` / `NextGroup`, bound to `alt-up` / `alt-down` in the `Sidebar` context (both layers).

## Hand checks
- [ ] Sidebar twins: visit two folders with the same name (`~/a/git`, `~/b/git`): Recent shows "git a" and "git b" with the hint dimmer; a narrow window truncates the hint before the name. VoiceOver reads "git, in a".
- [ ] Sidebar follows the cursor (I8): Tab into the sidebar: nothing navigates. Arrow down through favorites: each shows in the center, focus stays in the sidebar (ring moves, the list has no focus). Tab or → (or Return): the list has focus on that folder. With a search open, Tab to the sidebar and arrow: search closes, the folder shows. Back after an arrow walk returns to the folder you were in before the walk. List and column view.
- [ ] Sidebar fills: open a favorite: only it is filled. Tab into the sidebar and arrow: an accent ring moves, no second fill; Tab out: the ring goes. Drag a file over the current row: the drop highlight shows. Check in a light and a dark theme.
- [ ] Disk row: Locations shows "<n> GB free" right-aligned and a grey bar; fill a disk image past 90 %: the bar turns to the warning color. VoiceOver still reads free and total.
- [ ] Mark letters: with `keymap = "vim"`, favorites with a `key` show the letter on the right; `m x` on another folder moves it; Finder layer shows none.
- [ ] ⌥↑/⌥↓: Tab into the sidebar, press them: the cursor jumps to the first row of the previous/next group (Favorites groups, Recent, Locations).

## Found along the way
- (none yet)

## Log
- 2026-11-14: read AGENTS.md, the plan, the ui-overhaul index, `sidebar.rs`, `style.rs`, `ui.rs`, `pane.rs`, `window.rs` (focus, events, tests), `crow-core/sidebar.rs`, `crow-macos/names.rs`, `volumes.rs`. Asked Q1–Q13; all recommended answers chosen. Branch created from `main` (`a3da329`).

## Next
I1: add `RowLook`/`BarTone`, `looks`, `looks_for_test`, and the per-path display-name cache; keep today's rendering; add `sidebar_looks_follow_rows`.
