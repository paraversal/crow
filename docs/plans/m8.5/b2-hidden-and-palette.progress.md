# B2 — Hidden names and Cmd-K files progress

Branch: `m8.5-b2-hidden` · PR: [#57](https://github.com/paraversal/file-crow/pull/57) · Last updated: 2026-11-18 · State: PR open

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 Engine: the hidden policy | done | `Query.enter_hidden_folders`; the walker always visits dot entries and returns `WalkState::Skip` for a dot folder with a `HiddenFolder` skip; `Hit.hidden` from the dot name or `UF_HIDDEN`. 30 crow-search unit tests + all corpus cases pass, including the new `UF_HIDDEN` and dot-folder cases. |
| 2 Cmd-F rows and skips | done | `Row.hidden`, dimmed icon/name only, aria `, hidden`; `HiddenFolder` skips kept; header "Skipped folders"; `skip_reason_text`. GPUI `search_finds_dot_names_with_hidden_off` and `search_everything_enters_hidden_folders` pass on the corpus. |
| 3 Cmd-K: one matcher for entries | done | `PathEntry.hidden`, `HiddenMatch`, `match_entries`, `narrow_files`, `count_text_with_files`; `Item::Folder.hidden`; palette rows dim icon/name and speak ", hidden". Unit tests cover order, demotion, exact-first and the 78k-file budget. |
| 4 Cmd-K path mode | done | Path listings always include hidden entries; the policy decides matching, so no relist on a hidden toggle. Files are fuzzy (`FILE_LIMIT` 10) and highlights come from `EntryMatch.ranges`. GPUI tests for `hid`, `.h`, and mid-name files pass. |
| 5 Cmd-K typed query: files here | done | `PaletteDir`/`list_palette_dir` shared by path mode and the current folder; hidden subfolders appended as candidates; "Files in <here>" after folder rows; incremental `narrow_files` cache; `{places} · {files} files` count. GPUI test types `backup`, checks order/dimming, and Returns on `backup.txt`. |
| 6 Docs and hand checks | done (hand checks open) | README palette/search/settings prose; `docs/testing.md` engine, trace and hand-check rows; the two new settings documented. Hand checks that need the GUI are listed below; the 100k perf was measured at the matcher level. |
| 7 Search always includes hidden files (user addition) | done | `search_includes_hidden` (default on), Settings ▸ Search switch, live re-run of an open Cmd-F and rebuild of an open Cmd-K. Off restores the plain hidden filter in the engine (`Query.include_hidden`). |
| 8 Default search match (user addition) | done | `search_match` (`all`/`names`/`contents`, default all), Settings ▸ Search segmented control, read when a Cmd-F panel opens; the open panel keeps its own All/Names/Contents control. |

## Questions

None; the plan says there are none and the index's decisions are binding.

The user added two settings during execution (2026-11-18), quoted: "cmd+f and cmd+k always matching hidden files (even when hidden files are hidden) should be a setting on the settings page ('search always includes hidden files')" and "a setting to change the default search scope setting (all, names, contents - give users the option to change the default)". Implemented as items 7–8; the plan document is unchanged.

## Decisions

- Branch `m8.5-b2-hidden` from `main` at `fb760a8` (B1 merged as #56), as the plan requires.
- Items 1 and 2 share one commit: the app does not compile with `Hit.hidden` until `Row` carries it, and the hidden-folder skip is only visible in the app through item 2. The engine tests and the GPUI tests are in that commit.
- `corpus::Expect` gains `HitAndSkip(ancestor)`: `hidden-folder-name` expects `.backup` to be both a name hit and a skip. The plan asks for both in one case; the existing variants express either alone. The runner checks the path only; the engine unit test pins `HiddenFolder` as the reason.
- The GPUI corpus tests toggle Show Hidden Files off with `cmd-shift-.` first: the test config's default is `show_hidden_files = true` (Settings default), unlike the plan's premise. `open()` keeps the app's default settings.
- **Search hidden setting (user, 2026-11-18).** `search_includes_hidden` defaults to `true`, keeping the index's decision as the default; setting it off restores the old policy when Show Hidden Files is off: the walker does not visit dot entries at all, Cmd-K leaves them out of typed queries, and no `HiddenFolder` skips appear. The engine gained `Query.include_hidden`; `enter_hidden_folders` keeps the plan's meaning (descend into a dot folder). Show Hidden Files on overrides both.
- **Cmd-K empty query stays places-only.** The plan's table leaves hidden entries out of an empty partial when hidden is off, and the setting only changes typed matching. A path partial starting with `.` still asks for hidden entries.
- **Default search match (user, 2026-11-18).** `search_match` is read when a Cmd-F panel opens (`SearchView::new`); changing it does not override the All/Names/Contents control of an open panel. This is what the Settings description says and avoids a live change fighting the user's in-panel choice.
- `FolderCandidate.hidden` is computed from a dot name for every candidate, not only the palette's current-folder listing. The plan said favorites and recents are false, but a dot-named favorite should also be dimmed and demoted.
- `EntryMatch` keeps `score` and `demoted` because they are the matcher's contract and the unit tests read them; the listings use the sorted order and the entry's own `hidden`. The struct carries `#[allow(dead_code)]` with that comment. `score` is not read outside tests.
- `hidden_folder_candidate` was added to `palette.rs`: hidden folders from the current-folder listing need a label/name computed the same way as the other candidates, and `folder_name` is private.
- `PaletteDir` is shared by path mode and the current-folder listing (`list_palette_dir`). Path mode still uses `missing` for "No folder at".
- The live-setting GPUI test calls `CrowWindow::set_search_includes_hidden` directly after `ui::set_setting`: the test harness's window handle is borrowed, so the `apply_search_includes_hidden` loop cannot update it (the same reason `toggle_hidden` updates its own window first). The settings window path is separate and calls the same method for every browser window.
- Palette file matches are capped at 10 each for path mode and for the current folder (`FILE_LIMIT`/`TYPED_FILE_LIMIT`); folder rows take the rest of the 50-row budget.
- The 78k-file matcher test measured 204 ms (debug) and 11.7 ms (release) for five partials; the debug assertion is 650 ms (about 3×).

## Hand checks

- [ ] Hidden off, Cmd-F in `~` for `zshrc` (or another dotfile): found, dimmed, VoiceOver reads "…, hidden". The footer counts the skipped dot folders; the list shows them as "hidden folder (Show Hidden Files is off)". ⇧⌘A walks into them. Automated at corpus level; the GUI/VoiceOver pass is for the critic.
- [ ] Hidden on: the same search behaves as on `main`, except for B1's faster streaming.
- [ ] Cmd-K in `~`, hidden off: the empty query shows no dot folders; typing `config` lists `.config` after visible matches, dimmed; `.co` puts it first. Type `zshrc`: it appears under "Files in <you>", and Return reveals it. Automated on the corpus; the `~` walk is for the critic.
- [ ] Cmd-K path mode: `~/ssh` lists `~/.ssh`; `~/Desktop/<part of a file name>` matches mid-name. Automated on the corpus (`hid`, `.h`, `ack`); the real folders are for the critic.
- [~] `target/fixture-100k`: the full palette refresh was not driven in the app. The matcher path was measured: `match_entries` over 78k files, five partials, 204 ms debug / 11.7 ms release (≈2.4 ms per keystroke release), well under the 16 ms bar. The debug log's `palette refresh` times remain for the critic/user.
- [ ] Light/dark, 640 px: dimmed rows stay readable and the "Files in" header aligns with "In <here>".
- [ ] Settings ▸ Search: the switch and the All/Names/Contents control in light/dark and at 640 px; VoiceOver reads the switch state and "Motion, Auto, 1 of 4"-style selection. Automated state/role/write tests only.

## Found along the way

Nothing outside the plan's scope.

## Log

- 2026-11-18 Plan read, branch created from `main` (`fb760a8`), progress file written.
- 2026-11-18 Items 1–2: hidden policy in the engine, `Hit.hidden`, `HiddenFolder` skips, corpus cases flipped/added, dimmed rows and aria, skip list reason. `cargo test -p crow-search` 30 unit + 2 corpus tests pass; the two new GPUI tests pass; fmt and clippy clean.
- 2026-11-18 User additions: `search_includes_hidden` and `search_match` settings, Settings ▸ Search, live re-run for the hidden setting. The engine gained `Query.include_hidden`.
- 2026-11-18 Items 3–5: `HiddenMatch`/`match_entries`/`narrow_files`/`count_text_with_files`, fuzzy path-mode files, current-folder files with hidden subfolder candidates, incremental narrowing, dimmed palette rows. Unit + GPUI tests pass.
- 2026-11-18 Item 6: README and docs/testing updated (engine policy, 22 corpus cases, trace flags, hand checks, settings rows).
- 2026-11-18 Checks: `cargo fmt --check` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace` 583 app + 101 core + 67 fs + 11 macos + 31 search + 2 corpus + 6 theme pass, 1 ignored. `scripts/bundle.sh` built `target/bundle/Crow.app`.

## Next

PR [#57](https://github.com/paraversal/file-crow/pull/57) is open for the user's hand test and merge. After it merges, the executing agent's request is **execute plan B3** (`docs/plans/m8.5/b3-scope-and-misses.md`).

## Critique

Critic session: 2026-11-18 · Base: `main` @ `fb760a8` · Head reviewed: `c3c5db0` · Fixes: `4b8737f`

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | With `search_includes_hidden = false`, Search everything still visited dot entries: `visit_hidden` included `everything`, so `--no-search-hidden --everything` found `.backup` and `.backup/…`, contradicting the recorded decision ("the walker does not visit dot entries at all") and the setting's copy. | `crates/crow-search/src/engine.rs:309-312` | code / plan | fix | fixed (4b8737f): `visit_hidden = include_hidden \|\| enter_hidden_folders`; engine + GPUI tests cover the setting-off Search everything case. |
| C2 | The new Search controls broke settings tab order: `ControlId::ALL` put `SearchHidden`/`SearchMatch` after `ShowHidden`, but the page draws them in a Search section after the whole Behaviour section; Tab jumped down to Search then back up to Show preview, against the ALL comment and `docs/testing.md`'s reading-order hand check. | `crates/crow-app/src/settings_window.rs:146-175`, page at `:1438-1482` | UI / plan | fix | fixed (4b8737f): moved both after `PaletteRecent`; new `controls_are_tabbed_in_page_order` test fails without the move. |
| C3 | `docs/testing.md` did not carry the plan's remaining hand checks: Cmd-K hidden folders from the current folder, the 640 px/light "Files in" alignment, and the 100k palette budget. | `docs/testing.md:412-425` | plan / docs | fix | fixed (4b8737f): extended "Palette files here" and added "Palette budget". |
| C4 | The typed Cmd-K path that appends hidden folders from `here_dir` had no automated coverage; only the manual hand check. | `crates/crow-app/src/window.rs:2835-2870`, `window/tests.rs:4683` | test / plan | fix | fixed (4b8737f): `palette_typed_query_finds_files_here` now asserts `.backup` is listed, after a visible match, and marked hidden. |
| C5 | `Query.enter_hidden_folders`' doc said dot entries are "always visited and matched"; `include_hidden` (the user's setting) makes that false. | `crates/crow-search/src/engine.rs:105-112` | code / docs | fix | fixed (4b8737f): the comments now name the setting and its override, and state that Search everything ignores exclude rules, not this setting. |
| N1 | Items 3–8 are one commit, though Finish says "commit per item"; not recorded under Decisions. | `git log fb760a8..HEAD` | plan / record | note | left: the branch is squash-merged, so rewriting history would not help the review. |
| N2 | `EntryMatch.score` and `EntryMatch.demoted` are read only by tests, hence `#[allow(dead_code)]`. | `crates/crow-app/src/palette.rs:1180-1196` | code sense | note | left: the plan asks for these fields as the matcher's contract. |
| N3 | Cmd-K file rows announce "In <here>" (the group announces "Files in <here>"); "No places · 10 files" reads oddly when no folder matches. | `crates/crow-app/src/palette.rs:886-891`, `:855-863` | UX / aria | note | left: the group label names the files; the count format is the plan's. |

Checks re-run: `cargo fmt --check` ✓ · `cargo clippy --workspace --all-targets -- -D warnings` ✓ · `cargo test --workspace` ✓ (802 passed, 1 ignored; 12 suites) · `target/release/crow --search-trace` on a dot fixture with `--no-hidden`, `--no-search-hidden` and `--everything` ✓ (dot entries stay out; `--hidden` enters `.backup/.backup-inside.txt`) · `scripts/bundle.sh` ✓ after the fixes; the rebuilt app reruns an open Cmd-F with the setting off and Search everything stays at 2 visible results.
Budgets: 100k palette refresh on `target/fixture-100k` in the release bundle: empty query 11 µs; typed current-folder `file-0`/`file-00`/`file-000` 12.4–15.4 ms per keystroke (one 17.6 ms sample during the initial 100k background listing; three clean rounds stayed ≤15.4 ms), path mode `…/fixture-100k/file-0` 11.1–13.9 ms; the plan's bar is under 16 ms. The 78k-file `match_entries` debug test is 204 ms (assertion 650 ms).
Looked at: FireBox and Solarized Light at 1280 and 640 px; Cmd-F "backup" with Show Hidden Files off (dimmed dot rows, 2 skipped dot folders, "Skipped folders" panel with "hidden folder (Show Hidden Files is off)", ⇧⌘A descends); the setting toggled off/on via config reload with an open Cmd-F and Cmd-K (results rerun, palette updates); Cmd-K empty, typed "backup" (hidden folder and dot files after visible, dimmed, "Files in fixture"), path mode `hid`/`.h`/`ack`, Return on a file (automated); 100k palette; Settings ▸ Search in the window; the accessibility tree reads ".backup, hidden, …", ".backup.txt, hidden, …", "Search always includes hidden files", and "Search matches → All/Names/Contents". Not seen: the real `~`/`Desktop` walks (privacy prompts) and VoiceOver audio; the a11y labels and roles were read from the AX tree instead.
Verdict: better. The hidden policy is coherent across Cmd-F and Cmd-K, the palette files section lands in the right place without moving rows above it, and the setting live-updates what is open; the Search-everything bug was a real contradiction and is fixed. The tab-order regression was the other user-visible blemish.
