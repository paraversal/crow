# F2 — Typing and keys progress

Branch: `m8-f2-typing` · PR: none · Last updated: 2026-11-17 · State: in progress

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 Show the type-to-select query | done | `TypeSelect::prefix`/`clear`; `Folder::type_select_prefix`/`cancel_type_select`; the bar's transient slot shows the prefix left of the hints. The run expires on a generation-checked `TIMEOUT` task (`FolderEvent::TypeSelect`), not the 100 ms poll. Escape dismisses it and keeps the selection; starting a filter, a rename or another column clears it. Tests: core prefix/clear; bar shows `no`, Escape clears and keeps the row, the timeout clears, the filter field and Vim never show it. |
| 2 Enter behavior setting | todo | `enter = "rename" \| "open"`, `EnterRow`, Settings segmented control. |
| 3 Context menus for search results and the sidebar | todo | Sidebar menu already exists (`sidebar.rs:756-790`, tests at `window/tests.rs:601`); only search results remain. |
| 4 Palette home row matching | todo | Score name and label; highlight whichever matched. |
| 5 Finish: checks, bundle, docs | todo | fmt, clippy, tests, bundle, README + `docs/testing.md`. |

## Questions

- Q1 (asked 2026-11-17): Type-to-select display — bottom bar or overlay? → answer: **bottom bar** (user).
- Q2 (asked 2026-11-17): Confirm the setting name and values: `enter = "rename" | "open"`, default `"rename"`. → answer: **yes**, with a more descriptive Settings label ("What Return does"), and **F2 must always rename in both modes** (user). F2 is now bound to `Rename` in `LIST`; the menu/palette show F2 for Rename in both modes, so Q3 dissolved.
- Q3 (asked 2026-11-17): The ↩ display question. Explained: the native menu bar shows an action's first binding, and the palette/context menu show its winning binding; once Return dispatches `EnterRow`, Rename has no key to advertise. F2 → Rename gives it a real, mode-independent key, so Rename shows F2 and the question is moot. → answer: **explain again**; resolved by F2 (user).
- Q4 (asked 2026-11-17): Search-result menu items. → answer: **Open, Reveal in Folder, Quick Look, Copy, Cut, Copy Path, Move to Trash** (user; Cut added to the plan's list).
- Q5 (asked 2026-11-17): Palette home-row matching in F2 or defer? → answer: **include in F2** (user).

## Decisions

- Base `m8-f2-typing` from `main` (`2964848`); F1 is merged. One branch for F2 — repo rule (AGENTS.md, M8 index Q3), not asked.
- The plan's "Current state" is stale for the sidebar: `context_menu.rs` already has `SidebarMenuKind`/`sidebar_entries`, wired in `sidebar.rs` with tests. Item 3 therefore only builds the search-result menu; the sidebar is verified, not rebuilt.
- Escape while a typing run is active clears the prefix and is consumed (the selection stays), matching Finder's dismissal of its type-select overlay. Escape with no run keeps today's behaviour (Vim mark, then clear selection).
- Starting a filter or a rename clears the type-select run, so the bar never shows a stale prefix beside the filter field or the name field.
- The prefix clears on a timer per typing run: `type_char` bumps a generation and spawns one `TypeSelect::TIMEOUT` task; the task clears the buffer and emits `FolderEvent::TypeSelect` only if its generation is still current. This is testable with `advance_clock` (the folder's 100 ms poll checks `std::time::Instant`, which a test cannot advance) and cancels itself when the folder drops.
- F2 → `Rename` in `LIST`, always, in both Enter modes (user). Return dispatches `EnterRow`, which reads `settings.enter`. The sidebar and search results keep `OpenSelection`; Vim keeps `c w`.
- The Settings label is "What Return does" (user asked for a descriptive label), values "Rename" / "Open", description names F2.
- The search-result menu reuses the command registry; `SearchReveal` gets the row title "Reveal in Folder", and `CopyPath` gains a search handler because the pane's handler is not in the search overlay's focus tree.
- Palette matching scores the name only when it is not a suffix of the label (the home row, `name_start: None`). Scoring every name would double the 100k-folder work and change the ranking of ordinary folders whose label already contains the name.

## Hand checks

- [ ] Type `no` in a folder: the bottom bar shows `no` left of the hints; after a second it clears; Escape clears it at once; typing in the `/` field never shows it; Vim letters do not show it.
- [ ] Enter with `enter = "rename"`: the name field opens; set `enter = "open"`: Enter opens the row; sidebar and search results still open in both; Vim `c w` still renames.
- [ ] Settings window: the Return key control switches modes, writes only the `enter` line, survives a reload and keeps comments.
- [ ] Right-click a search result: Open, Reveal in Folder, Quick Look, Copy, Cut, Copy Path, Move to Trash; the chosen action runs once. The file list and sidebar menus are unchanged.
- [ ] Palette: `jul` shows the home row with `jul` highlighted in the name; `~` still shows it; other folder rows unchanged.
- [ ] Light and dark, 640 px and wide, Vim on/off, VoiceOver for the new menu items and the bar query.

## Found along the way

- An external `sidebar.toml` reload can leave the sidebar's cached rows stale until a notify (F1 C5). Outside F2; noted for F7.

## Log

- 2026-11-17: Read AGENTS.md, the M8 index, the F2 plan and F1's progress; read `selection.rs`, `folder.rs`, `pane.rs`, `bottom_bar.rs`, `context_menu.rs`, `sidebar.rs`, `search.rs`, `palette.rs`, `settings.rs`, `settings_window.rs`, `ui.rs`, `actions.rs`, `menus.rs`. Created the branch and this progress file. Asked Q1–Q5; answers recorded.
- 2026-11-17: Item 1. `TypeSelect` gained `prefix`/`clear`; `Folder` gained `type_select_prefix`, `cancel_type_select` and a per-run generation timer that clears the buffer and emits `FolderEvent::TypeSelect`. The bar's transient slot draws the prefix (aria "Type to select: …") only while the list owns the keys; `fit_to_window` gives it room. Escape consumes the run before the Vim mark and the selection; the filter, rename and a column focus change clear it. Tests pass.

## Next

Item 2: `enter = "rename" | "open"`, `EnterRow`, F2 → Rename, the Settings control, README and tests.
