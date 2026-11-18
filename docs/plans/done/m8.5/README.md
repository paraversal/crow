# M8.5 — Search reliability (index)

Between M8 (Finder parity) and M9 (Phase 2). Five batches, each a plan in this folder with its own branch, progress file, critique and PR. Execute them in order (B1 → B5). Do not start a batch until the one before it is merged to `main` (ASK before stacking on an unmerged branch): B2, B3 and B4 all write `crates/crow-app/src/search.rs`, and B2 and B4 both change `crow_search`'s public API.

Drafted 2026-11-17 from the user's second triage list: Cmd-K cannot find files/folders whose names start with a dot; Cmd-F finds "back" quickly but not "backup"; search is "kind of really unreliable" in general. **Re-cut into batches on 2026-11-18** after a code review found the likely cause of the "backup" report (below). The first draft's S1–S7 map onto the batches as shown in the table; `s1-corpus.md` is superseded by `b1-streaming.md` and kept only for history.

## What we know

- **The fuzzy matcher is not the cause.** `crow_search::FuzzyMatcher` scores `backup`, `.backup`, `Backups`, `backup-excludes`, `savegameBackup`, `do-not-backup` and `tm-backup` for both `back` and `backup`, and `.backu` against `.backup_excludes` (score 152, range 0..6). Probes on 2026-11-17 and 2026-11-18.
- **The engine holds small result sets until the walk ends.** `crow_search::search` (`crates/crow-search/src/engine.rs:257-268`) sends hits to the sink only when 256 (`BATCH`) are pending, or when the walk finishes. Nothing sends on a timer. A query with many matches (`.back` in `~`) fills a batch quickly and shows results; a query with a few matches (`.backu`) shows nothing until the walk ends. With match All, rooted at `~` and with hidden on, the walk reads the contents of every file in the home folder (`~/Library` included), which takes minutes. **This matches the user's repro exactly**: the list clears and the footer keeps saying "Searching…".
- **Content search opens every non-directory, FIFOs included** (`engine.rs:237-238`: `!is_dir`). Opening a named pipe with no writer blocks that walker thread forever, so a folder with a FIFO in it never finishes searching. It's a second possible cause of "keeps searching", and it's fixed in B1.
- **A file's name hit waits for its content read.** The walker builds one `Vec` per entry (name hit, then content hits) and sends it after reading the file, so a large file delays its own name hit.
- **Unreadable folders are silent gaps.** `Err(entry)` is dropped (`engine.rs:213`); a TCC-protected folder in `~` is neither searched nor reported.
- **Cmd-K is the palette, not file search.** Its empty query lists Recent, Favorites and the current folder's subfolders (`palette::folder_candidates`). A typed non-path query matches folders only. A typed path lists folders by fuzzy match but files by **prefix only** (`palette::path_listing`, `crates/crow-app/src/palette.rs:1048`). Path mode lists hidden entries only once the partial starts with `.` (`window.rs:2801`, `spawn_path_listing` at `:2854`); a test enshrines that (`palette_path_mode_hides_dot_folders_until_typed`, `crates/crow-app/src/window/tests.rs:4449`).
- **Cmd-F searches the current folder by default** (`Scope::Folder`), names and contents (`Match::All`). The result cap is 20,000 rows (`MAX_RESULTS`, `crates/crow-app/src/search.rs:45`); reaching it cancels the walk (`:478-481`), so a strong name hit that arrives later is lost.
- **Exclusions are on by default** (`exclude.toml` `dev` set). The walker honors Show Hidden Files with `hidden(!show_hidden)` (`engine.rs:291`), so with it off, dot files and dot folders are not visited at all.

## Decisions

### Product (the user, 2026-11-18; do not re-ask)

- **The repro:** `~/.backup_excludes`, searched from `~`, scope Folder, match All, Show Hidden Files on. `.back` finds it; `.backu` clears the list and keeps the footer searching.
- **Hidden and dot names:** Cmd-F and Cmd-K find them with Show Hidden Files off, mark them hidden, and Cmd-K matches them without a leading dot.
- **Cmd-K files:** a typed query fuzzy-matches files in the current folder, folders first; the empty query stays places-only.
- **Cmd-F default scope:** keep the current folder as the default; make the scope bar and the empty state say so, and add a one-key Search Home from a miss.
- **Cap:** always keep exact/contiguous name hits; reserve the cap for content and weak fuzzy hits; the footer says what was dropped.

### Architecture (review, 2026-11-18; binding for the batches)

These are decisions, not suggestions. An executing agent that finds one impossible ASKs instead of picking another.

1. **No `explain` mirror of the walker.** Diagnostics run the real engine and read its output (`crow --search-trace`). A second implementation of the walk rules would drift. The only independent reimplementation is B5's test oracle, where disagreement is the point.
2. **Batching is a small pure type** (`Batcher` in `engine.rs`) with a clock passed in, so it can be unit-tested. Policy: the first hit goes out at once; after that, a batch goes out when 256 are pending or 30 ms have passed since the last send.
3. **Content search reads regular files only** (and symlinks whose target is a regular file). FIFOs, sockets and devices are never opened.
4. **A name hit is sent before its file's contents are read.**
5. **Skips are typed.** `Summary.skipped: Vec<Skip>`, with `Skip { path, reason: SkipReason }` and `SkipReason::{Rule(String), HiddenFolder, Package, Unreadable(String)}`. `SkipReason::lifted_by_everything()` decides whether "Search everything" helps: true for Rule, HiddenFolder and Package, false for Unreadable. B1 adds `Rule` and `Unreadable`; B2 adds `HiddenFolder`; B3 adds `Package`.
6. **The hidden policy in search** (B2) is how the user's "find them with Show Hidden Files off" works without walking every dot folder in `~`:
   - The walker always visits dot-named entries, so their names and contents match at any level it reaches.
   - With Show Hidden Files off, it does not descend into a dot folder. It records a `HiddenFolder` skip instead. Search everything descends.
   - With Show Hidden Files on, it descends, as today. The `[hidden]` exclusion set still applies.
   - Every hit carries `hidden` (dot name or the macOS `UF_HIDDEN` flag), and rows draw it the way the list does (`style::content_opacity`).

   This keeps the default walk cost where it is today while finding `~/.backup_excludes`, `~/.zshrc` and `~/.config` itself.
7. **Cmd-K always lists with hidden entries** and filters or demotes them in one pure function (`palette::match_entries`). With Show Hidden Files off: an empty partial hides them; a typed partial includes them, ranked after visible matches unless the partial starts with `.`. Both the path mode and the typed non-path query use it.
8. **Search does not descend into packages** (`.app`, `.bundle`, `.framework`, `.photoslibrary`); the package itself is still a name hit, and Search everything descends. This follows Finder (principle 5). The package test moves from `crow-fs` to `crow-core` (`crow_core::is_package_name`) so `crow-search` can use it without depending on `crow-fs`; `crow-fs` and `search.rs`'s own list both call it.
9. **The cap is two budgets** (B4): 20,000 strong name rows, and 20,000 other rows (content and weak fuzzy). When the other budget fills, the engine **stops reading contents** but keeps walking for names (`Control::stop_contents`). Only filling the strong budget cancels the walk. A `Control` struct replaces the bare `cancel: &Arc<AtomicBool>` parameter.
10. **`Query.show_hidden` is renamed `enter_hidden_folders`** in B2, because it no longer decides what matches.

## The batches

All five batches are merged; B5 carries the final checks and moves this folder to `docs/plans/done/m8.5/`.

| Batch | What | Was | Branch | Merged |
| --- | --- | --- | --- | --- |
| [B1](b1-streaming.md) Streaming fix, corpus, trace | Time-based flush, name hits before contents, regular files only for contents, typed skips with unreadable folders, a small deterministic corpus with named cases, `--search-trace`, start/done log lines. Fixes the user's report. | S1 (trimmed) + the streaming half of S5 | `m8.5-b1-streaming` | [#56](https://github.com/paraversal/file-crow/pull/56) |
| [B2](b2-hidden-and-palette.md) Hidden names and Cmd-K files | Hidden policy in the engine, `hidden` on hits and rows, Cmd-K path mode lists hidden entries and fuzzy-matches files, typed Cmd-K query matches files in the current folder. | S2 | `m8.5-b2-hidden` | [#57](https://github.com/paraversal/file-crow/pull/57) |
| [B3](b3-scope-and-misses.md) Scope and misses | Search Home (⇧⌘H), a clearer Folder scope, the empty state offers every way to widen the search, skipped list shows all reasons, packages not descended, unreadable folders listed. | S3 + S6 | `m8.5-b3-scope` | [#58](https://github.com/paraversal/file-crow/pull/58) |
| [B4](b4-cap-and-performance.md) Cap and performance | `Control` with stop-contents, two-budget cap, footer says what was dropped, selection stable under late strong names, bench with a query argument and the budget recorded. | S4 + the rest of S5 | `m8.5-b4-cap` | [#59](https://github.com/paraversal/file-crow/pull/59) |
| [B5](b5-regression.md) Regression and finish | Oracle on random small trees, the app-level corpus matrix, search actions in the fuzzer, docs, README status line. | S7 + the milestone finish | `m8.5-b5-regression` | this PR |

**Order:** B1 → B2 → B3 → B4 → B5, one at a time; none can run in parallel (see the top). B1 is small and fixes the reported bug, so the user can hand-test it on day one.

**M8.5 is done when:**
- the user's `.backu` case and the dot-name cases are in the corpus and pass;
- Cmd-K and Cmd-F agree on the hidden policy;
- the scope, skips and cap are visible and explained at a miss;
- the search bench on `target/fixture-search-200k` is no worse than `main` (first hit and total; B4 sets the margin);
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass;
- the hand checks cover light/dark, 640 px and wide, hidden on/off, all three scopes, all three match modes, and VoiceOver for new roles;
- README's status line names M9 next.

## Not in M8.5

- Saved queries, Tags, Spotlight indexing and a search-history UI: M9 P4 and later.
- Content indexing or a database: Crow stays a live walker; no index to keep fresh.
- Regex syntax changes: `/…/` already works.
- Remote file systems and iCloud placeholder downloads: out of scope, as in M8/M9.
- Following symlinked folders during a walk: stays off (loops, duplicates). A symlink is a name hit; a symlink to a file is content-searched.
- A size limit for content reads, or a "names only inside `~/Library`" rule: revisit after B4's numbers if Home searches are still slow.
