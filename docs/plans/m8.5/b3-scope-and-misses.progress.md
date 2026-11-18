# B3 — Scope and misses progress

Branch: `m8.5-b3-scope` · PR: none · Last updated: 2026-11-18 · State: In critique

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 Search Home: ⇧⌘H | done | `SearchHome` action, `cmd-shift-h` in Search/SearchResults, window handler (open search → scope Home; open search → switches it), palette command `search-home`, Edit ▸ Search Home, idle Home example at index 3. `search_home_key_switches_scope` (field + results) and `search_idle_example_home_switches_scope` pass; key table and palette registry tests pass. |
| 2 Folder scope says which folder | done | Folder glyph, `~`-abbreviated path tooltip, aria "Search in <name>, <path>"; Home tooltip "Search Home ⇧⌘H". `search_scope_buttons_switch_scope` asserts the aria label. No new bar text. |
| 3 Empty state offers every way to widen | done | `Widen` + `widen_options` (order: everything, Home, names-and-contents), buttons with keys in the existing style, selectors `search-empty-{everything,home,all-match}`; unit test for the order/gates, GPUI tests click Home and All. |
| 4 Packages are not descended | done | `crow_fs::is_package_name` exported from the crate root (Q1 option A); `crow-search` depends on `crow-fs`; the walker records `SkipReason::Package` after the hidden check, so the package name is still a hit; `Row::new` uses the same test. Corpus: `package-inside` → `UnderSkip`, `package-everything` → `Hit`, `backup-package` → `Hit`; engine tests pin the hit/skip and everything. |
| 5 Every skip reason in the list | done | No filter: all four reasons kept. `skip_groups` (one function for renderer and tests) orders Excluded / Hidden folders / Packages / Couldn't read with heading rows (`Role::Heading`, level 2) and sorted paths. Unreadable rows show "no permission"; that group adds "Open Privacy & Security…" → `cx.open_url(...Privacy_AllFiles)`. Footer hint keys off `liftable > 0 || everything`. GPUI groups test plus the privacy click test pass. |
| 6 Docs | done | `docs/testing.md`: automated prose, 23 corpus cases, engine package policy, scope bar/Search Home/empty state/skipped groups hand checks, idle examples, the privacy action. |

## Questions

- Q1 (asked 2026-11-18): Item 4 says move `is_package_name` to `crow-core` and make `crow-fs` `pub use crow_core::is_package_name`. That would be a dependency cycle: `crow-core` already depends on `crow-fs` (`crates/crow-core/Cargo.toml`, and `crates/crow-core/src/lib.rs:78` re-exports `crow_fs::natural_cmp`). Options: (a) keep the function in `crow-fs`, export it from the crate root, add `crow-fs` to `crow-search`'s dependencies (it is already in its transitive graph through `crow-core`) and use it from the walker and `Row::new`; (b) add a tiny `crow-names` crate both depend on; (c) invert the dependency by moving `natural_cmp` to `crow-core`; (d) duplicate the list in `crow-core`. → answer: (a) keep it in `crow-fs` (user, 2026-11-18). Export it from `crow-fs`'s crate root, add `crow-fs` to `crow-search`'s dependencies, use it from the walker and `Row::new`; one shared function, no new crate.

## Decisions

- Branch `m8.5-b3-scope` from `main` at `42966fe` (B2 merged as #57), as the plan requires.
- **Q1:** the shared package test stays in `crow-fs` (user chose option A). `crow_search` gains a `crow-fs` dependency rather than a new crate.
- **Search Home menu item is always enabled.** The plan says "enabled only while search is open, mirroring Search Everything", but the menu bar is static (`menus::app_menus` takes only the preview and folder-size settings) and Search Everything is not disabled without a search: it opens one. Search Home mirrors that: with a search open it switches scope; without one it opens a search already scoped Home. Dynamic per-overlay menus for one item were not worth the plumbing.
- The fuzzer skips `cmd-shift-h` (`allowed()` in `fuzz.rs`): `ui::init_for_tests` uses the real `crow_core::home()`, so the key would walk the developer's `~`. The Home segment is not in `SearchView::debug_selectors`, so no existing fuzz route reaches a Home walk; adding the key to the list was the only new route.
- `search_home_key_switches_scope` points `Ui.home` at its fixture, so the results half can switch scope with a non-empty query without walking the real home folder.
- Package skips are recorded after the hidden-folder check, so a dot-named package is one `HiddenFolder` skip. `everything` enters both hidden folders and packages.
- `package-inside` now expects `UnderSkip`; the corpus runner checks the skip path only, so `packages_are_hits_but_not_descended` pins `SkipReason::Package` in the engine.
- The empty state's buttons come from `widen_options`; the Everything button is now gated on `liftable` rather than on any skip existing, so a package/hidden-only miss also offers it and a miss with zero skips does not.
- `skip_groups` is the single grouping function: the renderer and the test-only `skipped_groups()` both call it, so the test cannot drift from the paint order.
- The privacy row calls `cx.open_url` directly. No `Effects` method was needed: the test platform records the URL (`opened_url()`), so a test click never opens System Settings.

## Hand checks

- [ ] In `~/Desktop`, Cmd-F a word only in `~/Documents`: empty state shows "Search Home ⇧⌘H"; ⇧⌘H finds it; Folder segment tooltip shows its path; VoiceOver reads "Search in Desktop, ~/Desktop".
- [ ] Names-only miss: empty state offers "Search names and contents"; clicking switches the control to All.
- [ ] Cmd-F in `/Applications` for `Info.plist` with Contents: no hits from inside apps; the list shows "Packages"; ⇧⌘A finds them.
- [ ] Cmd-F in `~` without Home access (or a `chmod 000` folder): the list shows "Couldn't read"; the Privacy action opens System Settings.
- [ ] 640 px and wide, light and dark: scope bar and empty-state buttons fit, keycaps align; VoiceOver reads each button with its key.

## Found along the way

## Log

- 2026-11-18 Plan read, branch created from `main` (`42966fe`), progress file written.
- 2026-11-18 Q1 asked and answered (package test stays in `crow-fs`). Item 1 done: `SearchHome` action, ⇧⌘H, menu/palette/command wiring, idle example; fuzzer guard; two GPUI tests and the registry tests pass.
- 2026-11-18 Items 2 and 4: folder glyph/path tooltip/aria; package skips in the walker and the shared test in `Row::new`; corpus and engine tests. `cargo test -p crow-search` 33 + 2 pass; scope-bar test passes.
- 2026-11-18 Items 3 and 5: `widen_options` and the empty-state buttons; all skip reasons kept and grouped; the privacy action; footer hint gating. `cargo test -p crow-app search` 83 pass.
- 2026-11-18 Item 6: `docs/testing.md` updated for the scope bar, Search Home, empty state, skipped groups, packages, 23 corpus cases and the hand checks.
- 2026-11-18 Finish: `cargo fmt --check` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace` 591 app + 101 core + 67 fs + 11 macos + 33 search + 2 corpus + 6 theme pass, 1 ignored. The menu-order expectation gained Search Home. No bench: this batch touches no listing, scrolling or first-paint path.

## Critique

Critic session: 2026-11-18 · Base: `main` @ `42966fe` · Head reviewed: `96cde9a`

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | The fuzzer skipped ⇧⌘H under the real home *and* in `crow --fuzz`, whose home is the scratch root (`fuzz_live.rs:217`). B5 expects the generator to reach ⇧⌘H (`b5-regression.md:52`), and the plan's cap was conditional ("if it's the real home folder"). | `crates/crow-app/src/fuzz.rs:335` | code | fix | fixed (this critique commit): `GenState.home_is_real`, set from `Ui.home == crow_core::home()`; the test fuzzer skips, the live fuzzer presses it. New unit test. |
| C2 | The skipped-folders hand check said Search everything makes the link go away, but unreadable folders stay skipped and keep the link. | `docs/testing.md:407` | docs | fix | fixed (this critique commit): the sentence now names excluded/hidden/package folders and says unreadable ones stay listed. |
| C3 | README's search/menus text is now stale: "four examples" (there are up to five), and Edit carries Filter This Folder, Find… and Find Everything… (Search Home is missing). | `README.md:95`, `README.md:169` | docs | note | left: B5 owns README (`b5-regression.md:60`); its docs item does not name these two lines, so it should. |
| C4 | The index's Architecture 8 wanted the package test in `crow-core` so `crow-search` need not depend on `crow-fs`; Q1's options omitted the existing re-export pattern (`crow_core` already re-exports `crow_fs::natural_cmp`), and option A adds the direct dependency. | `crates/crow-search/Cargo.toml:10`, `crates/crow-core/src/lib.rs:78` | code sense | note | left: user chose A after Q1; the behavior is right. A `pub use crow_fs::is_package_name` in `crow-core` would drop the direct dep if wanted later. |
| C5 | The Edit ▸ Search Home item is always enabled, not "only while search is open" as the plan says; with no search it opens one scoped Home. | `crates/crow-app/src/palette.rs:434` | plan / UX | note | left: the Decision explains it, and it mirrors Find Everything…, which is also always enabled. I agree with the choice. |
| C6 | `search_empty_offers_search_home` only asserts the selector exists, not the visible "Search Home" label or its spoken key; `search_empty_offers_all_match` likewise. | `crates/crow-app/src/window/tests.rs:2665` | tests | note | left: the labels are covered by code and the a11y fuzzer; adding an aria assertion would make the two tests stricter. |

Checks re-run: `cargo fmt --check` ✓ · `cargo clippy --workspace --all-targets -- -D warnings` ✓ · `cargo test --workspace` ✓ (811 pass + 1 ignored before the critique; 812 + 1 after the new fuzzer unit test) · budgets not needed: no listing, scrolling or first-paint path.
Looked at: built `target/bundle/Crow.app`; 1280×800 and 640×800, FireBox (dark) and Solarized Light; search idle (Home example between This Mac and everything), empty state with Home + names-and-contents, grouped skipped list (Excluded, Hidden folders, Packages, Couldn't read), privacy row, folder glyph and 640 px fit. Could not see: the Folder tooltip and VoiceOver (hand checks), a real-home result (GPUI tests use a fixture home), and the menu item's key equivalent.
Verdict: better. The miss now names the folder, offers the three ways to widen it, and lists every reason; packages are a name hit but not walked, as Finder does. No regression seen.

## Next

Ready for critique. The five hand checks below need the built app; the critic builds `scripts/bundle.sh --open` and runs them.
