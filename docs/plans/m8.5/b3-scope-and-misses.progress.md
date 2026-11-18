# B3 — Scope and misses progress

Branch: `m8.5-b3-scope` · PR: none · Last updated: 2026-11-18 · State: in progress

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 Search Home: ⇧⌘H | done | `SearchHome` action, `cmd-shift-h` in Search/SearchResults, window handler (open search → scope Home; open search → switches it), palette command `search-home`, Edit ▸ Search Home, idle Home example at index 3. `search_home_key_switches_scope` (field + results) and `search_idle_example_home_switches_scope` pass; key table and palette registry tests pass. |
| 2 Folder scope says which folder | todo | |
| 3 Empty state offers every way to widen | todo | |
| 4 Packages are not descended | todo | |
| 5 Every skip reason in the list | todo | |
| 6 Docs | todo | |

## Questions

- Q1 (asked 2026-11-18): Item 4 says move `is_package_name` to `crow-core` and make `crow-fs` `pub use crow_core::is_package_name`. That would be a dependency cycle: `crow-core` already depends on `crow-fs` (`crates/crow-core/Cargo.toml`, and `crates/crow-core/src/lib.rs:78` re-exports `crow_fs::natural_cmp`). Options: (a) keep the function in `crow-fs`, export it from the crate root, add `crow-fs` to `crow-search`'s dependencies (it is already in its transitive graph through `crow-core`) and use it from the walker and `Row::new`; (b) add a tiny `crow-names` crate both depend on; (c) invert the dependency by moving `natural_cmp` to `crow-core`; (d) duplicate the list in `crow-core`. → answer: (a) keep it in `crow-fs` (user, 2026-11-18). Export it from `crow-fs`'s crate root, add `crow-fs` to `crow-search`'s dependencies, use it from the walker and `Row::new`; one shared function, no new crate.

## Decisions

- Branch `m8.5-b3-scope` from `main` at `42966fe` (B2 merged as #57), as the plan requires.
- **Q1:** the shared package test stays in `crow-fs` (user chose option A). `crow_search` gains a `crow-fs` dependency rather than a new crate.
- **Search Home menu item is always enabled.** The plan says "enabled only while search is open, mirroring Search Everything", but the menu bar is static (`menus::app_menus` takes only the preview and folder-size settings) and Search Everything is not disabled without a search: it opens one. Search Home mirrors that: with a search open it switches scope; without one it opens a search already scoped Home. Dynamic per-overlay menus for one item were not worth the plumbing.
- The fuzzer skips `cmd-shift-h` (`allowed()` in `fuzz.rs`): `ui::init_for_tests` uses the real `crow_core::home()`, so the key would walk the developer's `~`. The Home segment is not in `SearchView::debug_selectors`, so no existing fuzz route reaches a Home walk; adding the key to the list was the only new route.
- `search_home_key_switches_scope` points `Ui.home` at its fixture, so the results half can switch scope with a non-empty query without walking the real home folder.

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

## Next

Item 4 first (packages): export `is_package_name` from `crow-fs`, add the dependency, use it in the walker and `Row::new`; then item 5's groups can include packages.
