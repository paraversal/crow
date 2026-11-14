# u7-fix progress
Branch: `ui-u7-fix` · PR: none (critic opens it) · Last updated: 2026-11-14 · State: PR open

## Status
| Item | State | Notes |
| --- | --- | --- |
| X1 Palette: more room under section headers | done | `PALETTE_HEADER_ROW` 34, `pt(10)/pb(8)`, label `line_height(16)`; test `palette_header_leaves_room_above_the_first_row` (8 px label-to-row gap). Verified in the bundle against board B. |
| X2 Text fields: blinking caret, more room on the left | done | 2 px text inset (caret, selection, IME bounds, hit-testing), caret just before the first glyph, x-height-centered text, 500/500 blink via a per-field executor-clock loop. Test `caret_blinks_in_text_fields`; bundle: caret blinks through two alternating screenshots and no longer covers the "G". |
| X3 Search field: ↓ moves into the results | done | ↓ focuses the results and selects the cursor row (nothing without results); ↑ on row 0 returns to the field. Tests `search_down_from_field_focuses_results`, `search_down_without_results_stays_in_the_field`, `search_up_from_first_row_returns_to_field`, keymap unit test. |
| X4 New File | done | `Request::NewFile` in `crow_fs::ops` (unique "untitled", `create_new`, undo trashes it), File menu, palette (`new-file`), ⌥⌘N, inline rename. Ops test and window test `new_file_creates_and_renames`; hand: ⌥⌘N → "untitled" selected → typed name → file renamed. README + `docs/testing.md`. |
| X5 Hide the preview | done | `Settings.show_preview` (documented default, live reload), View ▸ Show/Hide Preview whose title follows the state, palette toggle chip, ⇧⌘P. Hides list, column and search previews. Tests `preview_toggle_hides_it_in_list_and_columns` (also checks the saved bool), `show_preview_false_in_settings_hides_it`, `preview_menu_names_the_action`; hand: ⇧⌘P removed the pane and wrote `show_preview = false`. |
| X6 UI freezes | done | Cause: the search stream. One file's matching lines made batches of up to 2.9M hits (`add_hits` 40-75 ms, once 203 ms; 327 slow batches in one `f` search in ~); a 50 ms heartbeat caught a 3.0-4.9 s main-thread block while the stream drained. Icons (H1) are minor: 5 calls ≥2 ms, max 24 ms cold. Fix: one excerpt per file (the first matching line plus a count of the rest), batches capped at 256, queued hits dropped after the result cap, and the receive loop yields every 8 ms. After: `add_hits` max 5 ms, no heartbeat late ≥100 ms; the 6.2 GB peak memory is gone by construction. |
| X7 Deleted file stays in the list | done | H1 confirmed: every change batch cloned `entries`, diffed in the background and replaced the listing when it landed, so a later batch replayed the pre-deletion listing. Fix: `stat_changes` in the background, `apply_stats` merges on the main thread against current entries. H2: watcher paths through a symlinked ancestor are rewritten into the folder's form. Tests `a_late_change_batch_does_not_bring_back_a_deleted_file` (fails on the old pipeline), `changes_through_a_symlinked_ancestor_still_match`, duplicate-path dedupe covered. |

## Questions
- Q1 (X2, asked 2026-11-14): which fields need more room on the left, and how much? → answer: 2 px text inset in every field, and the text should also be vertically centered (it read slightly low). Both implemented.
- Q2 (X3, asked 2026-11-14): should ↑ on the first result row return to the search field? → answer: yes.
- Q3 (X4, asked 2026-11-14): key for New File → ⌥⌘N.
- Q4 (X4, asked 2026-11-14): new file name → "untitled" without extension, the whole name selected; no warning when an extension changes (Crow has no such warning today).
- Q5 (X5, asked 2026-11-14): key for Show/Hide Preview → ⇧⌘P.
- Q6 (X5, asked 2026-11-14): where the preview state lives → `show_preview` in `settings.toml`.
- Q7 (X5, asked 2026-11-14): hide the search preview too? → yes, one switch hides every right-hand preview.

## Decisions
- Base: `main` at a3a8e96; PR #16 (U7) is merged, so the branch starts from it (the plan's condition).
- X1 header: the 12 px label's default line height is 19.5 px, so 34 px with `pt(10)/pb(8)` left only 4.5 px. Added `line_height(16)` so 10 + 16 + 8 = 34 px and the label-to-row gap is 8 px, as on board B.
- X1 test measures the header *label* bounds, not the header box: the header and the first row are adjacent by construction, while the plan's test asks for a ≥ 6 px gap. Label bottom to row top is the visible gap.
- X2 blink: focus listeners registered during render never fired in tests, so the blink is a per-field executor-clock loop plus a focus check in `paint`. The caret's phase uses `BackgroundExecutor::now()` (the test clock).
- X2 vertical centering: the text is centered on its x-height band (font metrics), not ascent+descent; the caret still spans the line box.
- X3 bindings: a GPUI key predicate (`Search && TextInput`) only matches tags of one context level, so ↓ is bound on `Search` (the band's level). The results' plain ↓ still wins there because `SearchResults` matches one level deeper.
- X4: no extension warning exists in Crow's rename, so the user's "don't warn" preference needs no code.
- X5 menu title: the menu bar is rebuilt on toggle and on live settings reload, so View says "Hide Preview" while the preview shows and "Show Preview" while hidden (GPUI `set_menus` accepts a rebuild).
- X5 saving: `save_setting` writes strings, so `show_preview = "true"` failed to reload. Added `crow_core::save_bool_setting` (typed bool, comments kept) and a crow-core test.
- X6 fix, no visible count change: the engine sends one content hit per file (its first matching line) and carries the other matching lines as a count, so the footer's "N results in M files" stays the same while memory and batch size collapse.
- X6 batching: `BATCH` is now a maximum (256), not a minimum; a huge `pending` is split with `split_off` instead of sent whole.
- X6 responsiveness: the receive loop yields with a 1 ms timer after 8 ms of work in production. Tests (`BackgroundExecutor::is_test`) drain at once, so the fake clock and `wait_for_search` are unaffected. This is scheduling, not a workaround: the amount of work is bounded by the per-file excerpt fix; the yield only keeps frames drawing while a large result set streams.
- X7: the watcher stays the only source (no reload after operations). `apply_changes` remains for the crow-fs API and is now `stat_changes` + `apply_stats`; duplicate paths in a burst are deduped.

## Hand checks
- [x] Palette headers: ⌘K: clear room between "Recent" and its first row (bundle, dark). Solarized Light not opened by hand; the test is theme-independent.
- [x] Caret: palette and filter: blinks (two alternating screenshots 0.3 s apart), no longer sits on the first letter; stays solid while typing and hides without focus (test).
- [ ] Search ↓/↑: covered by keystroke tests; not driven by hand in the bundle.
- [x] New File: ⌥⌘N created "untitled" with the name selected; typing and Return renamed it to `notes2`.
- [x] Preview: ⇧⌘P removed the right pane in list view and wrote `show_preview = false`; toggling back restored it. Column view covered by the test.
- [x] Freezes: measured with a release bundle and temp config: `f` in ~ + opening /Applications; no heartbeat block ≥100 ms after the fix, `add_hits` max 5 ms.
- [ ] Delete: covered by the new late-batch test; not repeated by hand in the bundle.
- [ ] Go to Folder: ⇧⌘G opens ⌘K with this folder's path and `/`; letters, Tab, Return go there; a file's full path selects it; a missing path beeps and stays open; typing `Docs` doesn't crash.
- [ ] View lock: `view_per_folder = false`, ⌘3, visit a folder with its own list view and go Back: stays in columns; ⌘T opens columns; ⌘J says "this window"; `views.toml` unchanged.
- [ ] Copying many files into a large folder (e.g. 500 into `target/fixture-100k`): no visible stall while the rows arrive.

## Found along the way
- The working tree had user changes before this branch began (`.gitignore`, `AGENTS.md`, `u2-menu-bar.md`, `u5-bottom-bar.md`, untracked `package.json`/`package-lock.json`). Left alone; commits touch only this plan's files.
- A stray `target/debug/crow` process was running during the hand checks (not started by this plan's commands). It captured one System Events keystroke; killed, and the hand checks were redone against the bundle only.

## Log
- 2026-11-14 Plan read; branch `ui-u7-fix` created from `main` (a3a8e96, U7 merged).
- 2026-11-14 X1 implemented and tested; verified against board B in the bundle.
- 2026-11-14 X2 implemented and tested; blink and inset verified in the bundle.
- 2026-11-14 X3 implemented and tested (41 search tests, keymap tests).
- 2026-11-14 X4 implemented and tested; README and `docs/testing.md` updated; hand check passed.
- 2026-11-14 X5 implemented and tested; `save_bool_setting` added after the string write was caught in the bundle.
- 2026-11-14 X6 measured (pre-fix), fixed, measured again (post-fix). `sample` of the running app showed the main thread idle during the sampled run and a 6.2 GB peak footprint pre-fix.
- 2026-11-14 X7 fixed with a deterministic failing-then-passing test; the duplicate-row regression in `filter_follows_live_changes` found the missing dedupe.
- 2026-11-14 `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` green (236 app, 62 core, 29 fs, 4 macos, 17 search).
- 2026-11-14 Bench (release bundle, `target/fixture-100k`): warm first paint 7 ms, cold 55 ms, scroll p99 8.86 ms (119 fps), max 27.76 ms. The changed paths (preview hidden by default, text input, search, watcher merges) are not on the scroll path; no regression claim vs `main` was needed. Budgets met.
- 2026-11-14 Follow-ups from the user's hand test: (1) Cmd-F scope kept on the focused column (user's choice; README now says Right first searches the previewed subfolder). (2) Shift-Cmd-G opens Cmd-K with the current path typed; the Go to Folder sheet is gone; Return on a path with no folder row reveals a file or beeps. (3) `view_per_folder` setting (default true): false keeps each window's view, Cmd-2/3 save nothing, new windows start in `[defaults]`, new tabs copy their window, Cmd-J says "this window". Found along the way: `FuzzyMatcher` passed an uppercase needle to nucleo with `ignore_case`, which asserts ("should have been caught by prefilter") — typing e.g. `Docs` in Cmd-K could crash Crow. Needle is now lowercased; regression test in crow-search. fmt, clippy, tests green (237 app, 63 core, 29 fs, 4 macos, 18 search).

## Critique
Critic session: 2026-11-14 · Base: main @ a3a8e96 · Head reviewed: 1bce797 (fixes on top: 68254bc, f1beb7d)

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | X7 moved the batch merge onto the main thread, but `apply_stats` compared every listed entry with every changed path and inserted changes one `Vec::insert` at a time. 100k folder, release: 10 changes 23 ms, 100 changes 171 ms, 1000 changes 1.6 s on the main thread (spec principle 2; X6's own goal). | `crates/crow-fs/src/snapshot.rs` `apply_stats` | code | fix | fixed (68254bc): hash set + one merge pass, ~5 ms for any burst size; ordering test added |
| C2 | Stale doc line from the removed Go to Folder sheet sat on the `sheet` field; `text_input` module doc still named Go to Folder. | `window.rs:76`, `text_input.rs:3` | code | fix | fixed (f1beb7d) |
| C3 | `FuzzyMatcher` handed nucleo an uppercase needle with `ignore_case`; nucleo asserts ("should have been caught by prefilter"), so ⌘K or a name search with e.g. `Docs` could crash Crow. Pre-existing on `main`, found while building ⇧⌘G. | `crow-search/src/engine.rs` `FuzzyMatcher::new` | code | fix | fixed in 1bce797 (lowercased needle + regression test) |
| C4 | ⇧⌘G while search is open now does nothing: it opens ⌘K, which U7 blocks over search. Before, the Go to Folder sheet opened over search. | `window.rs` `open_palette` guard | UX | note | left: it matches ⌘K, as the user asked ("⇧⌘G opens ⌘K"); revisit if it bothers in use |
| C5 | Return on a typed file path reveals it only when no folder row matches; if the file name fuzzy-matches a sibling folder, Return goes to that folder. | `window.rs` `confirm_palette` | UX | note | left: rows win, as in the rest of the palette |
| C6 | The caret blink loop runs for each text field's whole life (one 500 ms timer per field, notifying only while focused). | `text_input.rs` `start_blink_loop` | code sense | note | left: cheap, and the plan's epoch design would need focus listeners that don't fire in tests (Decision on record) |

Checks re-run: fmt ✓ · clippy ✓ · tests ✓ (237 app, 63 core, 30 fs, 4 macos, 18 search) · bench vs main, alternating runs on `target/fixture-100k`: p99 8.61/8.66 ms (main) vs 8.61/8.67 ms (branch), warm first paint 8 ms on both; cold first paint noisy on both (33-65 ms). A third pair hit a multi-second machine stall on both and is discarded.
Looked at in the app: not driven: this session has no computer-use tools. Visual items (caret, palette headers, preview toggle) rest on the executing agent's bundle checks and the rendered-bounds tests; the new behaviors are in Hand checks above.
Verdict: better. Every item does what the plan asks with a test that fails without it; the one real regression (C1, main-thread merge cost) is fixed, and the crash in C3 was on `main` before this branch.

## Next
Ready for critique: run the `critique` skill on this branch, fix what is clearly wrong, open the PR titled "UI U7 fix: palette spacing, caret, search ↓, New File, hide preview, freezes, stale delete", and hand the reviewed branch to the user.
