# B3 — Scope and misses

Audience: an implementing agent. Follow the `execute-plan` skill; read `AGENTS.md` and the M8.5 index (`docs/plans/m8.5/README.md`, "Decisions" and "Architecture" 5 and 8) first.
Base: branch `m8.5-b3-scope` from `main` **after B2 is merged**.
Progress file: `docs/plans/m8.5/b3-scope-and-misses.progress.md`. PR title: "M8.5 B3: search scope, misses and skips".

## Why

A miss in Cmd-F has five causes the user can't see today:
- the item is outside the current folder (scope);
- it's in an excluded folder, a hidden folder (B2) or a package;
- the match mode is Names or Contents;
- the folder couldn't be read.

The user decided to keep the current folder as the default scope, to make the scope bar and the empty state say so, and to add a one-key Search Home from a miss. This batch also stops descending into packages (Finder does the same), and lists unreadable folders, which B1 started recording.

Principle 9 applies throughout: every new element is an **action** (a button with its key), not a caption.

## Items

### 1 Search Home: ⇧⌘H

**Change:**
- New action `SearchHome` in `actions.rs`. Bind `cmd-shift-h` in the `Search` and `SearchResults` contexts, next to `SearchEverything`. ⇧⌘H is Finder's Go ▸ Home, and Crow has no binding for it today (checked 2026-11-18). In a search, "go Home" means "search Home", the same idea. If `keys.rs`'s conflict tests disagree, ASK.
- `SearchView::search_home`: `set_scope(Scope::Home, cx)` (a no-op when already Home), then keep focus where it was.
- Add a palette command (`CommandId::SearchHome`, key `"search-home"`, label "Search Home") and a menu item next to "Search Everything", in whichever menu `menus.rs` puts it. It's enabled only while search is open, mirroring Search Everything.
- Add an idle-state example, between "This Mac" and the everything row: face `Home`, description "your whole home folder", `ExampleAction::Home`, aria "Search Home, Shift Command H" (spoken via `keys::spoken_for`).

**Fuzzer:** `fuzz.rs` draws steps from the live keymap, so `fuzz_smoke` will press ⇧⌘H. Check what `Ui.home` is under `ui::init_for_tests` and in the fuzz runner. If it's the real home folder, a Home search would walk the developer's `~` during tests. In that case, exclude `SearchHome` from the fuzzer's allowed bindings the same way other unsafe keys are capped. Also check whether clicking the existing Home segment (`search-scope`) is already reachable by the fuzzer; if it is, the same problem exists today, so note it in the progress file and give both the same fix.

**Tests:** GPUI keystroke tests `search_home_key_switches_scope` (in the field and in the results), plus the palette command test (`settings_commands_are_in_the_palette`-style registry checks pick it up). `command_keys_are_stable_and_unique` must still pass.

### 2 The Folder scope says which folder

**Change** (`render_scope_bar`, `scope_segment`):
- The Folder segment shows a folder glyph before the name (`icons::glyph(Glyph::Folder, 12., …)`, or the closest existing glyph; don't add a new icon asset). The name keeps its 160 px ellipsis.
- Its tooltip is the full `~`-abbreviated path (`with_tooltip`), and its aria label is `"Search in <name>, <path>"`.
- The Home segment's tooltip is `"Search Home ⇧⌘H"`, via `with_action_tooltip` with `SearchHome`.

No new text in the bar. Check that the bar at 640 px still fits the three segments plus the skip switch. If it doesn't, the folder name ellipsizes first (it already has `max_w`).

**Tests:** the existing layout test for the bar's height still passes. Add an aria assertion for the Folder segment's label.

### 3 The empty state offers every way to widen

**Change:**
- A pure function in `search.rs`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Widen {
    /// Search everything: `n` skipped folders that it would walk.
    Everything(usize),
    /// Scope is a folder other than Home.
    Home,
    /// Match is Names or Contents (not This Mac).
    AllMatch,
}

/// The ways to widen a search that found nothing, most specific first.
fn widen_options(scope: Scope, folder_is_home: bool, matching: Match, liftable: usize, everything: bool)
    -> Vec<Widen>;
```

  The order is: `Everything` (when `liftable > 0 && !everything`), then `Home` (when `scope == Folder && !folder_is_home`), then `AllMatch` (when `matching != All && scope != ThisMac`).
- `render_empty` (not while `searching`) keeps its message `No results for "x" in <scope>`, then one button per option, in the style of the existing `search-empty-everything` row (accent text, keycap after):
  - `Everything(n)`: `"{skipped_was_text(n)}. Search everything"` + ⇧⌘A (the existing row, now computed from `liftable`);
  - `Home`: `"Search Home"` + ⇧⌘H;
  - `AllMatch`: `"Search names and contents"`, with no keycap (⇧⌘M cycles, so it isn't this action). Clicking sets `Match::All`.
  - Each button has `Role::Button`, an aria label with its key spoken, and a debug selector `search-empty-<kind>`.
- `liftable` = the number of skips with `reason.lifted_by_everything()`.

**Tests:** unit tests for `widen_options` (every row of the order above, plus This Mac → only `Everything` if liftable). GPUI: in the corpus's `backup` subfolder search `savegame` (it's a sibling, outside): the empty state shows "Search Home", and ⇧⌘H switches scope. Don't assert Home results; Home is the real home in tests, so assert only `scope() == Home`.

### 4 Packages are not descended

**Change:**
- Move `is_package_name` from `crow-fs/src/listing.rs` to `crow-core` (`crow_core::is_package_name`, in the module that fits best, e.g. a new `crow-core/src/names.rs` re-exported from `lib.rs`). `crow_fs::is_package_name` becomes a `pub use crow_core::is_package_name`, so callers don't change. Replace `search.rs`'s own `matches!(ext, "app" | "bundle" | "framework" | "photoslibrary")` in `Row::new` with the shared function.
- Walker closure: after matching a directory entry, if `!everything && is_package_name(name)` (and `is_dir`), record `Skip { reason: SkipReason::Package }` and return `WalkState::Skip`. The package itself stays a name hit. Order relative to the hidden check from B2: hidden first, then package. Either way it's one skip.
- `SkipReason::Package`: `Display` is `"package"`, and `lifted_by_everything()` is true. `skip_reason_text` shows `"package (Search everything looks inside)"`.

**Tests:** flip the corpus case `package-inside` to `UnderSkip("CrowBackup.app")`; add `package-everything` (contents `needle`, everything) → `Hit`. `backup-package` stays `Hit`.

### 5 Every skip reason in the list; unreadable folders shown

**Change:**
- Stop filtering `self.skipped`: keep every reason. The footer link counts all of them (`"N folders skipped ›"`). The list groups by reason, in this order: exclusion rules, hidden folders, packages, unreadable. Each group has a small header row (`"Excluded"`, `"Hidden folders"`, `"Packages"`, `"Couldn't read"`), in the existing row style, and paths are sorted within a group. Header rows use `Role::Heading` with an aria label, as the sidebar's section headers do (`sidebar.rs:996`).
- Unreadable rows read `"no permission"` (or the error text) on the reason line. For `"no permission"`, add one action row at the end of that group: `"Open Privacy & Security…"`, which opens `x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles` with GPUI's `cx.open_url(…)`. Don't add AppKit code: `cx.open_url` already goes through the platform. In tests, route it through `crate::effects` if the other platform calls there are (check how `open_path` is faked), so a test click doesn't open System Settings.
- The footer's "Search everything" hint is shown when `liftable > 0 || self.everything` (instead of `skip_label.is_some()`), so hidden-folder and package skips get the hint even with no exclusion set enabled.

**Tests:**
- GPUI `search_skipped_list_groups_reasons`: corpus root, names `backup`, hidden off. The list has the Excluded (`node_modules`, `target`, `build`, `dist`, `.git`), Hidden folders (`.backup`) and Packages (`CrowBackup.app`) groups. Unreadable (`unreadable`) appears unless running as root.
- Update `search_skipped_link_opens_the_list_and_escape_closes_it_first` (`window/tests.rs:2957`) if its counts change.

### 6 Docs

`docs/testing.md`: the scope bar, Search Home, the empty state's options, the skipped groups, packages. Update the "Skipped folders" hand check text ("Folders skipped by exclude.toml" no longer exists).

## Hand checks

- In `~/Desktop` (or any folder below Home), Cmd-F a word that's only in `~/Documents`:
  - the empty state shows "Search Home ⇧⌘H"; ⇧⌘H finds it;
  - hover the Folder segment: the tooltip shows its path;
  - VoiceOver reads "Search in Desktop, ~/Desktop".
- Names-only miss: the empty state also offers "Search names and contents"; clicking it switches the control to All.
- Cmd-F in `/Applications` for `Info.plist` with Contents: no hits from inside apps; the list shows "Packages"; ⇧⌘A finds them.
- Cmd-F in `~` with Home access not granted to Crow (or a folder with `chmod 000`): the list shows "Couldn't read"; the Privacy action opens System Settings.
- 640 px and wide, light and dark: the scope bar and the empty-state buttons fit and the keycaps align. VoiceOver reads each button with its key.

## Questions to ASK

None expected. ASK if ⇧⌘H conflicts with an existing binding or menu item.

## Finish

`cargo fmt --check` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo test --workspace` · the hand checks · commit per item · progress file current · stop at "Ready for critique" (do not push).
