# u8-sidebar progress
Branch: `ui-u8-sidebar` · PR: none · Last updated: 2026-11-14 · State: In critique

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 Row model and name cache | done | `RowLook`/`BarTone`, `looks`, `looks_for_test`, per-path name cache; `sidebar_looks_follow_rows`; no visible change intended |
| I2 Twin hints | done | `crow_core::twin_hints` + 8 unit tests; hints/spoken in `looks`; hint rendered after the name; 4 window tests |
| I3 One fill per meaning | done | current folder `selection`, cursor an inset accent ring; drop replaces the fill; tests |
| I4 Disk row | done | neutral `fg-subtle` bar, warning at 90 %, "182 GB free" right-aligned, aria ", almost full"; volumes refresh after operations |
| I5 Mark letters | done | Vim layer shows the key in the right slot; live `m x` test |
| I6 Alias marker | done | `Ui.aliases` from the Recent background task; " · alias" on twin hints and spoken labels; symlink test |
| I7 Favorites header hint | skipped | Q7 = no |
| I8 Center follows the sidebar cursor | done | `SidebarEvent::Show`, `History::replace_current`, `Pane::show_from_sidebar`, `right` binding; 6 tests |
| I9 ⌥↑/⌥↓ jump between groups | done | `PreviousGroup`/`NextGroup`, `alt-up`/`alt-down` in `Sidebar`; test |

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
- I4 test isolation: `refresh_volumes` returns early when the app has no `Watchers` global. `init_for_tests` installs none, so tests keep `volumes` empty (the plan's invariant); production always has watchers by the time it refreshes.
- I8 test contract: `same_keys_mean_different_things_in_sidebar_and_list` now asserts that Down in the sidebar *shows* beta and the list follows; the old "leaves the list alone" assertion was the behavior I8 replaces. `sidebar_cursor_is_a_ring...` now expects the fill to move with the pane after a cursor move.
- Finish: the plan says push and open the PR; the `execute-plan` skill (and AGENTS.md) says the executing agent commits locally and stops at Ready for critique, and the critic opens the PR. Followed the skill; no push, no PR.

## Hand checks
- [ ] Sidebar twins: visit two folders with the same name (`~/a/git`, `~/b/git`): Recent shows "git a" and "git b" with the hint dimmer; with a long name (the sidebar is a fixed 210 px) the hint truncates and goes before the name does. VoiceOver reads "git, in a".
- [ ] Sidebar follows the cursor (I8): Tab into the sidebar: nothing navigates. Arrow down through favorites: each shows in the center, focus stays in the sidebar (ring moves, the list has no focus). Tab or → (or Return): the list has focus on that folder. With a search open, Tab to the sidebar and arrow: search closes, the folder shows. Back after an arrow walk returns to the folder you were in before the walk. List and column view.
- [ ] Sidebar fills: open a favorite: only it is filled. Tab into the sidebar and arrow: an accent ring moves, no second fill; Tab out: the ring goes. Drag a file over the current row: the drop highlight shows. Check in a light and a dark theme.
- [ ] Disk row: Locations shows "<n> GB free" right-aligned and a grey bar; fill a disk image past 90 %: the bar turns to the warning color. VoiceOver still reads free and total.
- [ ] Mark letters: with `keymap = "vim"`, favorites with a `key` show the letter on the right; `m x` on another folder moves it; Finder layer shows none.
- [ ] ⌥↑/⌥↓: Tab into the sidebar, press them: the cursor jumps to the first row of the previous/next group (Favorites groups, Recent, Locations).

## Found along the way
- (none yet)

## Log
- 2026-11-14: read AGENTS.md, the plan, the ui-overhaul index, `sidebar.rs`, `style.rs`, `ui.rs`, `pane.rs`, `window.rs` (focus, events, tests), `crow-core/sidebar.rs`, `crow-macos/names.rs`, `volumes.rs`. Asked Q1–Q13; all recommended answers chosen. Branch created from `main` (`a3da329`).

- 2026-11-14: I1 done. `RowLook` reproduces today's fills/bar; name cache keyed by path, pruned when the row set changes. `cargo fmt --check`, clippy, workspace tests green. Not hand-compared against a screenshot; the element tree is unchanged apart from reading the look.

- 2026-11-14: I2 done. `twin_hints` dedupes paths, compares labels and ancestors case-insensitively, walks up unresolved twins only, falls back to `usize::MAX`. `looks` maps the distance to the ancestor name (home shown "~", spoken by its display name) and appends the mark after the hint. Line spacing now explicit (icon 8, hint 6, dot 8, right 6, eject 8) so the hint margin is not doubled by the old flex gap. Tests: 8 crow-core unit, 4 window. Workspace green.

- 2026-11-14: I3 done. Removed the temporary cursor fill; `ring = focused && cursor` draws `style::cursor_ring` as an inset shadow. Current fill is skipped while a drop highlight covers the row. Tests: ring moves with Tab/Down and Vim j, no ring when unfocused, drop look clears. Not yet hand-checked on screen (ring contrast/light-dark) — listed under Hand checks.

- 2026-11-14: I4 done. `short_bytes`, `bar_tone` (`>= 0.9`), free text in the right slot, track `surface-raised`, fill `fg-subtle`/`warning`, spoken ", almost full". `operations::finish` now calls `refresh_volumes`; `refresh_volumes` returns early when the app has no watchers, so `init_for_tests` keeps `volumes` empty (the plan's invariant; otherwise real volumes appeared in every sidebar test). Unit tests for sizes/tone; window test for the disk row and warning. Workspace green.

- 2026-11-14: I5 done. `right` is the mark letter only in the Vim layer; spoken label keeps "mark k" after any twin hint. Window test covers Finder layer, Vim layer, and `m a` moving the letter. Workspace green.

- 2026-11-14: I6 done. The existing `refresh_recent` background task now also stats favorites + the Recent list and stores symlinks in `Ui.aliases` on the main thread; `load_sidebar` itself stays I/O-free (every sidebar reload calls `refresh_recent`). Alias text only on twins, as the plan says: "link · alias", spoken "git, in link, alias". Test creates a real symlink. Workspace green.

- 2026-11-14: I8 done. `SidebarEvent::Show` after a real cursor move; the window closes search, navigates the active pane via `Pane::show_from_sidebar`, and refocuses the sidebar. `History::replace_current` keeps the walk to one Back step (first move is a normal visit, so Back returns to the folder you were in). Return/→/Cmd-Down/click use `open_from_sidebar`, which ends the walk and navigates only if the pane isn't already there. `right` is bound to `OpenSelection` in `Sidebar`. Updated `same_keys_mean_different_things_in_sidebar_and_list` (its old assertion encoded "the sidebar leaves the list alone", which I8 changes) and `sidebar_cursor_is_a_ring...` (the fill now follows the pane). New tests: arrows show/keep focus, Return/→ focus, search closes, Tab-in does not navigate, one-step history. Workspace green.
- 2026-11-14: I9 done. `PreviousGroup`/`NextGroup` on `alt-up`/`alt-down` in the `Sidebar` context, skipping empty groups; the jump emits `Show` too. Test with two favorite groups + Recent.
- 2026-11-14: Finish docs. README sidebar paragraph now mentions twin parents/alias, Vim mark letters, Locations free space + bar, group jumps and the center following the cursor. `docs/testing.md`: automated summary and five hand rows (twins, follows cursor, fills, disk row, mark letters) plus a group-jump row; the Locations row now says "with free space and a bar".
- 2026-11-14: Bench (Q9), once: `cold_first_paint=149ms cold_load=802ms frames=599 avg=8.35ms p50=8.33ms p95=8.54ms p99=8.61ms max=16.47ms warm_first_paint=9ms warm_load=262ms`. No comparison run; the pane rows did not change.
- 2026-11-14: Q6 also covers undo and redo (both change free space): `operations::undo`/`redo` success now refresh volumes too.
- 2026-11-14: `./scripts/bundle.sh` builds `target/bundle/Crow.app`. Hand checks not run (need a human at the screen); listed above.

## Critique
Critic session: 2026-11-14 · Base: main @ a3da329 · Head reviewed: 234c82f

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | Q10 ("current folder" aria description) is recorded under Decisions as done, but no row ever set it. | `sidebar.rs` row element | plan | fix | fixed (b123be3) |
| C2 | Walking Recent with the arrows reorders Recent under the cursor: every `Show` goes through `PaneEvent::Entered` → `ui::record_visit`, which re-ranks Recent. Throwaway test (alpha 2 visits, beta 3; Tab on beta, Down to alpha): alpha jumps to the top, the ring stays on index 1 (now beta) while the center shows alpha; the next Down lands on a row already passed. Also every folder walked past gets a frecency visit. Not visible in my real-app run because zoxide scores dominated. | `window.rs` `on_pane_event` Entered, `sidebar.rs` `move_down` | UX / code | ask | Q14 |
| C3 | The twins hand check said "a narrow window truncates the hint", but the sidebar is a fixed 210 px; only a long name truncates it. | `docs/testing.md`, Hand checks | record | fix | fixed (critique docs commit) |
| C4 | `sidebar_walk` is a bool reset only by `navigate`/back/forward; history visits from `apply_mode` (pane.rs:630) and the root-column ← (:938) don't reset it, so a later walk could replace that step. Tried two key sequences; neither reproduced it (columns don't move history the way I expected). Anchoring the walk on the folder it last showed (`Option<PathBuf>`) would make it independent of who else navigates. | `pane.rs:247`, `:436-443` | code | note | left: no repro |
| C5 | `refresh_volumes` returns early without the `Watchers` global, a test-only guard in production code. Production installs watchers (ui.rs:222) before the first refresh (:231), so nothing breaks; recorded as a Decision by the agent. | `ui.rs:502-507` | code sense | note | left |
| C6 | The new keys (⌥↑/⌥↓ group jumps, → to the list) are not in U5's sidebar-focus hints, which still show "⌥⌘↑↓ reorder · ⌘⌫ remove · ⇥ to list" (screenshot). The plan says "so U5's sidebar hints can show them", which doesn't clearly ask U8 to add them. | bottom bar, sidebar focus | UX | note | left (U9/U10 or a follow-up) |
| C7 | `short_bytes` rounds after choosing the unit: 9.96 GB prints "10.0 GB", 999.6 GB prints "1000 GB". Cosmetic. | `sidebar.rs` `short_bytes` | code | note | left |
| C8 | `symlinked_twin_says_alias` polls with `thread::sleep` up to 2 s for the background alias scan. Test-only; the same wait pattern as other background-I/O tests. | `window.rs` test | code | note | left |

Checks re-run: fmt ✓ · clippy ✓ · tests ✓ (385) · bench not needed: the pane's rows are unchanged (agent's single run recorded above)
Looked at in the app: branch build with a throwaway `CROW_CONFIG_DIR`/`CROW_STATE_DIR`, FireBox (dark), Vim layer on: twins "git Coding / Work / Projects · alias", Recent "projects ~ · alias", marks f/w on the right, red color dot, "Macintosh HD 1.6 TB free" with a neutral bar; Tab + Down shows the folder with fill and ring on the same row, list keeps no focus; ⌥↓ into Recent and arrows walk it. Not looked at: a light theme, VoiceOver, a nearly full disk, `main` side by side (the sidebar changes are all intended and listed in the plan).
Verdict: pending Q14.

## Next
Ready for critique. The critic should hand-check the ring/shadow and light/dark contrast, plus the I8 feel in list and column view; the plan's `Finish` push/PR step is left to the critic (execute-plan skill).
