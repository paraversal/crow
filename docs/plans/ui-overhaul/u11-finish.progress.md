# u11-finish progress
Branch: `ui-u11-finish` · PR: none (do not merge; user said "DONT merge back to main") · Last updated: 2026-11-14 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 Start: baseline, inherited list, bench on `main` | done (this commit) | U1–U10 merged; inherited table below; current-state facts re-read; 6 bench lines below. |
| I2 Reproducible window screenshots | blocked (Q1) | Needs `scripts/shot.sh` + `scripts/window-id.swift`; also needs Screen Recording permission. |
| I3 Findings checklist | todo | 12 rows; walk in the bundle. |
| I4 Visual drift sweep | todo | Needs I2 shots; constants + 3 tests. |
| I5 Preview fills the rest (only if Q2 = build) | blocked (Q2) | |
| I6 Accessibility sweep | todo | Q7 mechanism already shipped (Status labels); extend to counts. |
| I7 Docs pass | todo | README status line blocked on Q11. |
| I8 Bench vs `main` | todo | Procedure proven in I1. |
| I9 Follow-ups recorded | todo | Q10 for the index note. |
| I10 PR | todo | Q8 for screenshots; the critic opens the PR per the skill. |

## Questions
- Q1 (asked 2026-11-14): may I add `scripts/shot.sh` and `scripts/window-id.swift`? Recommended yes. → pending
- Q2 (asked 2026-11-14): finding 6 (dead quarter) — build in U11 (I5) or defer? Recommended build. → pending
- Q3 (asked 2026-11-14): board-only details (line icons per favorite, clock icons, syntax-coloured preview, 196 px columns, darker focused-column tint) — build or defer? Recommended defer. → pending
- Q5 (asked 2026-11-14): unify text-field focus rings (filter 1 px vs search 2 px)? Recommended one `field_ring` 2 px. → pending
- Q8 (asked 2026-11-14): commit screenshots as `docs/plans/ui-overhaul/img/built-*.png` or keep them out? Recommended commit. → pending
- Q9 (asked 2026-11-14): bench baseline `main` only or also M6? Recommended both (already measured both in I1). → pending
- Q10 (asked 2026-11-14): add a "Later" list to `docs/plans/ui-overhaul/README.md`? Recommended yes. → pending
- Q11 (asked 2026-11-14): README status line wording. Recommended "Status: Phase 1 (the "daily driver" MVP): M6 and the UI overhaul are done; next is M7 (hardening)." → pending

Settled without asking (plan or code decides):
- Q4 copy conflicts: the plan wins (index ground rule). Bottom bar "clear / search deeper" (U5), palette menu column shows U2's menu names.
- Q6 chord order: macOS order (⌃⌥⇧⌘). Already what U1's `keys::caps` prints; U5 recorded `⌥⌘↑↓` as the live formatter's output. Nothing to change.
- Q7 VoiceOver announcements: U5/U6/U9 shipped the `Status`-role label mechanism (U5 R5, U6 footer status, U9 tip `Role::Status`). U11 extends it to the filter and search counts; no GPUI patch, no `crow-macos` notification unless the hand check fails.

## Decisions
- Branch `ui-u11-finish` from `main` at `def19f0` (U10 merged). No merge to `main`, no push: the execute-plan skill says the critic opens the PR; the user repeated "DONT merge back to main" (2026-11-14).
- M6 bench worktree uses the current `vendor/` through a symlink (same pinned GPUI rev; the only patch added since M6 is `gpui-enter-menu-equivalent.patch`, which touches menu key equivalents, not row rendering). Recorded so the numbers are not overclaimed.
- Bench config for every run: `/tmp/u11-bench/config/settings.toml` (`theme = "firebox"`, `hints = "always"`, `[window] width = 1280 height = 800`), `views.toml` `[defaults] view = "list"`, fresh `CROW_STATE_DIR=/tmp/u11-bench/state`.

## Merged plans (I1)
| Plan | PR | Merge commit |
| --- | --- | --- |
| U1 shared parts | #8 | `c666799` |
| U2 menu bar | #9 | `07f49cd` |
| U3 top band | #10 | `a539089` |
| U4 filter | #12 | `77479d6` |
| U5 bottom bar | #13 | `0295148` |
| U6 search panel | #14 | `50a4d17` |
| U6 fix | #15 | `0b4fdaa` |
| U7 palette | #16 | `a3a8e96` |
| U7 fix | #18 | `32c1a81` (list spacing) / `a3da329` (U7 fix) |
| U8 sidebar | #20 | `ac510b3` |
| U9 tips | #21 | `0718a7c` |
| U10 motion and sound | #22 | `def19f0` |

## Inherited (needs hand check, deferred, or open question)
| From | Item | State |
| --- | --- | --- |
| U1 | Light-theme scrim pair; tooltips for every Target element; Vim palette keys in the bundle; `cursor_ring` crispness | hand check |
| U2 | Settings… and Hide Others/Show All not exercised; Undo in text fields; menu checks were run via System Events | hand check |
| U3 | Tooltip hover pass; Names-mode speed on a big tree; VoiceOver band | hand check |
| U4 | Filter no-match state in a light theme; VoiceOver filter; IME in text fields | hand check |
| U5 | Whole bottom-bar hand list (hints, progress, error, counts, narrow, VoiceOver) | hand check |
| U6 | Scope bar themes, idle examples, footer, skipped list, preview, VoiceOver, narrow, light/dark highlights | hand check |
| U6-fix | Light theme, VoiceOver, idle look, This Mac, narrow window | hand check |
| U7 | Whole palette hand list; VoiceOver | hand check |
| U7 | Vim `j`/`k` block typing those letters in the palette (kept by plan) | deferred |
| U7 | Home row reads "~" twice | deferred |
| U7 | Palette view lives in `window.rs` (~700 lines) | deferred (Q13, follow-up) |
| U7-fix | Search ↓/↑, Delete, Go to Folder, view lock, bulk copy | hand check |
| U8 | Sidebar hand list (twins, cursor follows, fills, disk, marks, recent walk, group jumps) | hand check |
| U8 | New keys (⌥↑↓ group jump, → to list) missing from U5's sidebar hints | open (C6) |
| U8 | `short_bytes` rounds after unit choice ("1000 GB") | deferred (cosmetic) |
| U9 | Tips + VoiceOver | hand check |
| U9 | Tip floats over the list's bottom-right rows | deferred (plan position) |
| U10 | Motion/sound hand list; NSSound vs alert volume (R3) | hand check |
| U10 | Sidebar current-folder fill does not glide | deferred (user) |
| U10 | Bench inconclusive on a 30 Hz display | superseded by U11 I1/I8 |

## Current state (re-read on `main` @ `def19f0`, replacing the plan's facts)
- Window: `window.rs:32-50` `open()`, default 1200×760 (`settings.rs` `[window]`), min 640×400, traffic lights (14,19), band left edge 86 px.
- `CrowWindow::render` `window.rs:2938`: band, optional tab bar (>1 tab), `sidebar | center | preview`, bottom bar, tip, then sheet/activity/why/picker/palette. Search replaces the center; the list preview is separate.
- Measures as merged: band 52 px (`window.rs:680`), nav buttons 28×28 (`:521`), view switch 32×26 (`view_button`), Go to 30 px (`:608`), band search button 240×30 radius 8 (`:641`), active search field 32 px + `border_2` (`search.rs:1042-1056`); sidebar 210 px (`sidebar.rs:1049`); bottom bar 30 px (`bottom_bar.rs:33`); rows 24 px (`pane.rs:54`), search rows 40 px (`search.rs:42`); columns 220–360 (`pane.rs:72-73`), preview column 300 (`pane.rs:74`); filter field 28 px radius 6 `border_1` (`pane/pane_filter.rs:52,60`), filter footer 26 px (`:122`); scope bar 38 px (`search.rs:1173`); palette top 52, width 620, input 50, folder row 44, command row 40, header 34, footer 34 (`window.rs:166-178`).
- Tints: `style.rs:31-61` `scrim`, `active_selection` (accent 0x55), `inactive_selection`/`current_location` (`selection`), `error_soft`, `drop_target`; `cursor_ring` is an inset 1.5 px shadow (`style.rs:91`).
- Menus: `menus.rs:42` `app_menus`, `:85` `init`; commands in `palette.rs` `COMMANDS` in menu-bar order.
- Keymaps: `actions.rs:113+` (Finder), Vim layer adds on top; `keys::caps` prints macOS order.
- Status: `bottom_bar.rs` slots (path · transient · counts), `Role::Status` for progress/messages/errors, `Role::Group` for hints.
- Bench: `pane.rs:1311-1360` `bench_step`, 600 frames, prints `BENCH …`; `main.rs` `--bench`, `--bench-filter`, `--make-fixture`.

## Bench (I1, 100k fixture, 1280×800, firebox, same machine)
| Run | warm first paint | p50 | p95 | p99 | max |
| --- | --- | --- | --- | --- | --- |
| `main` (U10) 1 | 7 ms | 8.33 | 8.56 | 9.64 | 18.23 |
| `main` (U10) 2 | 8 ms | 8.33 | 8.54 | 9.76 | 19.96 |
| `main` (U10) 3 | 10 ms | 8.32 | 8.60 | 9.77 | 19.89 |
| M6 (`ad93ce1`) 1 | 7 ms | 8.33 | 8.55 | 8.67 | 21.43 |
| M6 (`ad93ce1`) 2 | 8 ms | 8.32 | 8.57 | 8.69 | 22.02 |
| M6 (`ad93ce1`) 3 | 10 ms | 8.33 | 8.53 | 8.66 | 22.37 |

`main` p99 is ~1.1 ms above M6 (spread within each side ≤ 0.13 ms), consistent with U10's Full motion (U10 measured Full 9.73–9.83 ms, Off 9.07). Budget is p99 < 1.5 × 8.33 ms = 12.5 ms; both are inside it, so no optimization question yet. Warm first paint 7–10 ms on both, well under 50 ms.

## Hand checks
- [ ] (from the plan's consolidated table) run after I7; recorded per row.

## Found along the way
- `package.json` / `package-lock.json` are untracked in the working tree (not mine; left alone).

## Log
- 2026-11-14: read AGENTS.md, the execute-plan skill, the U11 plan, the index, all U1–U10 progress files. Confirmed U1–U10 merged on `main` (`def19f0`). Created `ui-u11-finish` and this file. Ran the I1 bench on `main` and on the M6 worktree (`target/m6-wt`, `CARGO_TARGET_DIR=target/m6-target`). Asked Q1, Q2, Q3, Q5, Q8–Q11.

## Next
Ask Q1–Q11 in one batch; while waiting, start I4's constant inventory (grep for duplicated literals) and I3's checklist rows that need no screenshots.
