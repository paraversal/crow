# B2 — Hidden names and Cmd-K files

Audience: an implementing agent. Follow the `execute-plan` skill; read `AGENTS.md` and the M8.5 index (`docs/plans/m8.5/README.md`, "Decisions" and "Architecture" 5–7 and 10) first.
Base: branch `m8.5-b2-hidden` from `main` **after B1 is merged** (it needs `Skip`, the corpus and `--search-trace`).
Progress file: `docs/plans/m8.5/b2-hidden-and-palette.progress.md`. PR title: "M8.5 B2: hidden names in search, files in Cmd-K".

## Why

The user decided that Cmd-F and Cmd-K find dot-named items with Show Hidden Files off, mark them hidden, and that Cmd-K matches them without a leading dot. They also decided that a typed Cmd-K query fuzzy-matches files in the current folder, folders first. Today:
- the search walker skips every dot entry when hidden is off (`engine.rs` `build_walker`: `.hidden(!show_hidden)`);
- Cmd-K path mode lists hidden entries only after a typed `.`, and matches files by prefix only (`palette::path_listing`);
- a typed non-path Cmd-K query matches folders only.

## The policy (one table; every item below implements a row of it)

| Where | Show Hidden Files on | Show Hidden Files off |
| --- | --- | --- |
| Cmd-F walk | visits and descends into everything (minus exclusion sets), as today | visits dot entries (names and contents match), does **not descend** into dot folders: one `HiddenFolder` skip each. Search everything descends. |
| Cmd-F row | dimmed if hidden (`content_opacity`), as the list does | the same |
| Cmd-K, empty partial / empty query | lists hidden entries, dimmed | leaves them out |
| Cmd-K, typed partial / query | fuzzy-matches them, dimmed, ranked by score with the rest | fuzzy-matches them, dimmed, ranked **after** visible matches unless the partial starts with `.` |

"Hidden" is the same everywhere: the name starts with `.`, or the macOS `UF_HIDDEN` flag is set (`crow_fs::Entry::hidden` already computes this for listings).

## Items

### 1 Engine: the hidden policy

**Change** (`crates/crow-search/src/engine.rs`):
- Rename `Query.show_hidden` → `Query.enter_hidden_folders` with the doc comment: "Descend into dot folders. Dot-named entries are always visited and matched; this only decides whether the walk goes inside them." Update every caller (`search.rs`, `main.rs` bench and trace, tests).
- `build_walker`: `.hidden(false)` always. Drop its `show_hidden` parameter.
- In the walker closure, after the name and content matching for an entry:

```rust
if is_dir && !enter_hidden && name.starts_with('.') {
    record(Skip { path, reason: SkipReason::HiddenFolder });
    return WalkState::Skip;   // matched itself, not descended
}
```

  `enter_hidden` is `query.enter_hidden_folders || query.everything`. Exclusion rules still run first, in `filter_entry` (so `.git` stays a `Rule` skip under `dev`, and is not a name hit, as today).
- `SkipReason::HiddenFolder`: `Display` is `"hidden folder"`, and `lifted_by_everything()` is true.
- `Hit` gains `pub hidden: bool`: the name starts with `.`, or (macOS) `metadata.st_flags() & libc::UF_HIDDEN != 0`, using `std::os::macos::fs::MetadataExt` on the metadata the closure already reads. Not macOS: dot names only. Content hits carry the same value as their file.

**Tests** (`engine.rs` and the corpus cases):
- Flip `backup-hidden-file` to `Hit` and `backup-hidden-folder` to `UnderSkip(".backup")`.
- Add `hidden-folder-name` (names `backup`, hidden off, target `.backup`): `Hit`, and the skip list contains `.backup` with `HiddenFolder`.
- Add `hidden-folder-everything` (names `backup`, hidden off, everything, target `.backup/.backup-inside.txt`): `Hit`.
- Unit test: `Hit.hidden` is true for `.backup.txt` and false for `backup.txt`. On macOS also set `UF_HIDDEN` with `libc::chflags` on a plain-named file and assert true.

### 2 Cmd-F rows and skips

**Change** (`crates/crow-app/src/search.rs`):
- `Row.hidden: bool` from the hit (Spotlight rows: dot name only).
- Draw the icon and the name with `style::content_opacity(row.hidden, false)`, like `pane.rs:3233-3238`. Keep the folder line and the content line at full opacity, so excerpts stay legible.
- `Row::label` (aria) appends `", hidden"` after the name when hidden.
- `Query` is built with `enter_hidden_folders: self.show_hidden`. `set_show_hidden` already restarts the search.
- B1 filtered `self.skipped` to `Rule` skips. Now keep `Rule` **and** `HiddenFolder`; Unreadable stays filtered until B3.
- The skipped list header `"Folders skipped by exclude.toml"` becomes `"Skipped folders"` (the reason line under each row already says which rule). For `HiddenFolder` the reason line reads `"hidden folder (Show Hidden Files is off)"`; write it as a small `fn skip_reason_text(&SkipReason) -> String` in `search.rs`, which B3 extends.
- The empty state's existing "N folders were skipped. Search everything" now counts `HiddenFolder` skips too, which is correct: Search everything lifts them. The footer's "Search everything" hint is still shown only when `skip_label` is set; B3 changes that.

**Tests** (GPUI, in `window/tests.rs`, on the B1 corpus built in a temp dir; hidden off):
- `search_finds_dot_names_with_hidden_off`: Cmd-F, Names, type `backup`. The results include `.backup.txt`, `.backup_excludes` and the folder `.backup`, not `.backup/.backup-inside.txt`. `row_label` of `.backup.txt` contains `", hidden"`. The skipped count includes `.backup`.
- `search_everything_enters_hidden_folders`: then ⇧⌘A: `.backup/.backup-inside.txt` appears.

### 3 Cmd-K: one matcher for entries

**Change** (`crates/crow-app/src/palette.rs`):
- `PathEntry` gains `pub hidden: bool`. Its constructor becomes `PathEntry::new(path, name, hidden)`; the test helper `pe` passes `name.starts_with('.')`.
- New pure function, used by path mode (folders and files) and the typed non-path query (files):

```rust
pub struct EntryMatch<'a> {
    pub entry: &'a PathEntry,
    pub score: u32,
    /// Matched bytes in `entry.name`; empty for an empty partial.
    pub ranges: Vec<Range<usize>>,
    /// Hidden and Show Hidden Files is off: drawn dimmed and ranked after visible matches.
    pub demoted: bool,
}

/// Entries matching `partial`, best first, at most `limit`.
/// Empty partial: every visible entry (hidden ones too when `show_hidden`), by lowercased name.
/// Typed partial: fuzzy by name. Order: exact name (lowercased) first, then not demoted
/// before demoted, then score (high first), then lowercased name. A partial starting
/// with "." demotes nothing. Ranges are computed only for the kept entries.
pub fn match_entries<'a>(partial: &str, entries: &'a [PathEntry], show_hidden: bool, limit: usize)
    -> Vec<EntryMatch<'a>>;
```

  Use `FuzzyMatcher::score` for every entry and `indices` only after truncating, as `folder_listing` does.
- `Item::Folder` gains `hidden: bool`, which drives dimming and aria. The row renderer in `window.rs` (the palette row code near `:3391-3422`) applies `content_opacity(hidden, false)` to the icon and name, and appends `", hidden"` to the row's aria label.

**Tests** (unit, `palette.rs`):
- `match_entries_hides_hidden_for_an_empty_partial` (show_hidden off) / `…_lists_them_when_shown`.
- `match_entries_matches_dot_names_without_the_dot`: `backup` finds `.backup_excludes`, after `backup.txt` and `Backups`, with `demoted = true`.
- `a_dot_partial_demotes_nothing`: `.b` ranks `.backup` by score with the rest.
- `exact_name_comes_first`: `LICENSE` with files `LICENSE` and `LICENSE-MIT`.
- Keep the spirit of `path_completion_filters_many_files_quickly` (78,000 entries): rewrite it to time `match_entries` over five partials and assert it stays under a generous budget measured in debug. Record the measured value in the progress file and set the assertion at 3× that. Use `std::hint::black_box` on the input.

### 4 Cmd-K path mode

**Change:**
- `window.rs` `refresh_palette`: delete `let hidden = show_hidden || partial.starts_with('.')`, `PathState.hidden` and the relist-on-hidden-change branch. `spawn_path_listing` drops its `hidden` parameter and always calls `crow_fs::list_dir(&target, true, …)`, passing `entry.hidden` into `PathEntry::new`.
- `palette::path_listing(parent, partial, children, files, home, show_hidden)`: build the folder rows from `match_entries(partial, children, show_hidden, TOTAL_LIMIT)` and the file rows from `match_entries(partial, files, show_hidden, FILE_LIMIT)`. The order stays: an exact file name first, then "this folder" for an empty partial, then folders, then files. **Files are now fuzzy**: drop the prefix filter. Raise `FILE_LIMIT` from 5 to 10, and update its doc comment ("Files shown under a typed path: the best fuzzy matches").
- File row highlight ranges come from `EntryMatch.ranges` shifted by `name_start`, as folder rows already do. This replaces the "typed prefix" range code.

**Tests:**
- Rewrite `palette_path_mode_hides_dot_folders_until_typed` (`window/tests.rs:4449`) as `palette_path_mode_matches_dot_names_without_the_dot`, with hidden off:
  - `<root>/` does not list `.hidden`;
  - `<root>/hid` lists `.hidden` after visible matches, marked hidden;
  - `<root>/.h` lists it first.
- `palette_path_mode_fuzzy_matches_files`: in the corpus, `<corpus>/ack` lists `back.txt` and `backup.txt` (prefix matching found neither).
- Update `path_listing_puts_this_folder_first` and `a_file_named_as_typed_comes_first_and_others_follow_folders` for the new signature. Their expectations stay.

### 5 Cmd-K typed query: files in the current folder

**Change:**
- When the palette opens (`open_palette`, `window.rs`), start one background listing of `cwd` with hidden entries. Refactor `spawn_path_listing`'s task body into:

```rust
/// One folder for the palette, listed off the main thread with hidden entries.
struct PaletteDir { folders: Vec<PathEntry>, files: Vec<PathEntry>, missing: bool }
fn list_palette_dir(parent: PathBuf, cx: &App) -> Task<PaletteDir>;
```

  Use it for path mode and for this listing.
- `PaletteState` (the palette struct in `window.rs`) gains `here_dir: Option<PaletteDir>`, set when the listing arrives if the palette is still open on the same `cwd` (compare the path; the palette is closed and reopened per use, so a stale result just finds `None`). Then refresh.
- Hidden subfolders: when `here_dir` arrives, append its hidden folders that are not already candidates to `palette.folders` as `FolderCandidate { source: Source::Here, hidden: true, … }`. `FolderCandidate` gains `hidden: bool` (false for favorites and recents). In `folder_listing`:
  - an empty query leaves out `hidden` candidates when Show Hidden Files is off;
  - a typed query demotes them with the same rule as `match_entries`.
  - `folder_listing` gains a `show_hidden: bool` parameter.
- Files: `folder_listing(query, folders, here, files: &[PathEntry], show_hidden)`. With a non-empty query and a non-empty `files`, after the folder rows: `Row::Header(format!("Files in {here}"))`, then up to `TYPED_FILE_LIMIT = 10` rows from `match_entries(query, files, show_hidden, 10)` as `Item::Folder { source: Source::File, … }`. Return on one uses the existing `Source::File` behavior: reveal and select it in its folder (`window.rs:2998`). The folder rows are capped at `TOTAL_LIMIT - file rows`. Folders come first, and the files section is appended below them when the listing arrives, so no row above it moves (principle 3).
- Count text: files are not "places". Keep `count_text` for folders, and when file rows exist show `"{places} · {files} files"`. Add a `count_text_with_files(places, files)` helper with its own unit test.
- **Per-keystroke cost.** Score files incrementally. `PaletteState` keeps `file_narrow: Option<(String, Vec<u32>)>`: the last typed query and the indices of the files it matched. When the new query starts with the old one (and `here_dir` hasn't changed), score only those indices; otherwise score all. Put this in a pure `fn narrow_files(query, files, previous: Option<(&str, &[u32])>) -> Vec<u32>` in `palette.rs`; `match_entries` then runs over the narrowed slice (build a `Vec<&PathEntry>`, or give `match_entries` an index filter: pick one, and document it). Clear the cache when `here_dir` changes.

**Tests:**
- Unit: `narrow_files` returns the same set as a full scan for `b`, `ba`, `bac` (and for a non-extending query `x` after `bac`).
- Unit: `folder_listing` with files puts folder rows first, then the "Files in …" header, then files; an empty query lists no files.
- GPUI keystroke test `palette_typed_query_finds_files_here`: open the corpus folder, Cmd-K, type `backup`:
  - folder rows (`backup`, `Backups`, …) come before the "Files in" header;
  - `backup.txt` is among the files, and `.backup.txt` comes after the visible files (hidden off);
  - Return on `backup.txt` closes the palette with `backup.txt` selected in the list.
- The existing U7 budget log ("palette refresh: … for N items") stays. Hand-check it on `target/fixture-100k` (item 6).

### 6 Docs and hand checks

Update `docs/testing.md`'s Search and palette rows: hidden policy, dimmed rows, "Skipped folders", Cmd-K files. Add the hand checks below as rows.

## Hand checks

- Hidden off, Cmd-F in `~` for `zshrc` (or another dotfile you have): found, dimmed, VoiceOver reads "…, hidden". The footer counts the skipped dot folders; the list shows them as "hidden folder (Show Hidden Files is off)". ⇧⌘A walks into them.
- Hidden on: the same search behaves as on `main` (dot folders are descended), except for B1's faster streaming.
- Cmd-K in `~`, hidden off: the empty query shows no dot folders; typing `config` lists `.config` after visible matches, dimmed; `.co` puts it first. Type `zshrc`: it appears under "Files in <you>", and Return reveals it.
- Cmd-K path mode: `~/ssh` lists `~/.ssh`; `~/Desktop/<part of a file name>` matches mid-name.
- `target/fixture-100k`: open it, Cmd-K, type `file-0`, `file-00`, `file-000` quickly. No visible lag. Record the `palette refresh` debug times; the U7 budget is under 4 ms per keystroke for 10,000 subfolders, so with 100k files under 16 ms is the bar. If it's over, ASK before adding a background scorer.
- Light/dark, 640 px: the dimmed rows stay readable and the "Files in" header aligns with "In <here>".

## Questions to ASK

None expected. ASK if:
- `UF_HIDDEN` makes `~/Library` rows dimmed in a way that looks wrong to you in the hand check (that is Finder's behavior, so the default is to keep it);
- the 100k Cmd-K bar above isn't met.

## Finish

`cargo fmt --check` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo test --workspace` · the hand checks · commit per item · progress file current · stop at "Ready for critique" (do not push).
