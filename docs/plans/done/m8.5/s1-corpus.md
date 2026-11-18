> **Superseded 2026-11-18** by [b1-streaming.md](b1-streaming.md) (see the M8.5 index). Kept for history; do not execute.

# S1 — Repro corpus and diagnostics

Audience: an implementing agent. Follow the `execute-plan` skill; read `AGENTS.md` and the M8.5 index first.
Base: branch `m8.5-s1-corpus` from `main`. Progress file: `docs/plans/m8.5/s1-corpus.progress.md`.
PR title: "M8.5 S1: search repro corpus and diagnostics".

## Why

The user reports that Cmd-F finds `back` quickly but not `backup`, and that search is unreliable in general. The fuzzy matcher is not the obvious cause: it scores `backup`, `.backup`, `Backups`, `backup-excludes`, `savegameBackup`, `do-not-backup` and `tm-backup` for both queries, with contiguous ranges (M8.5 index, "What we know"). The miss can come from the scope, the hidden policy, an exclusion set, the 20,000-result cap, the match mode, or the difference between Cmd-F (names + contents) and Cmd-K (folders by fuzzy name, files by prefix only). Without a reproducible tree and a diagnostic that names the stage, every later piece would guess.

S1 builds the repro corpus and the diagnostic tools. It changes no behavior: no UI, key, menu, palette command, result order or file operation changes.

## What we know

- `crow_search::search` walks with `ignore`, streams `Hit`s in batches, and returns `Summary { scanned, skipped, error }` (`crates/crow-search/src/engine.rs:112`, `:53`). A skipped folder carries the rule that skipped it (`crates/crow-search/src/rules.rs:54`).
- The walker applies hidden first (`.hidden(!show_hidden)`, `engine.rs:291`), then the exclusion sets (`filter_entry`, `:299`). It does not follow symlinks (`:297`). A read error is swallowed (`:213`), so an unreadable folder is a silent gap, not a result.
- Content search reads every non-directory (including a symlink to a file) and reports the first matching line plus a count of the rest (`engine.rs:222-245`, `ContentSink`).
- Cmd-F starts in `Scope::Folder` (`crates/crow-app/src/search.rs:49`, `:399`), defaults to `Match::All` (`:68`), and builds `Query` with the live hidden setting and rule set (`:404`). The result cap is `MAX_RESULTS = 20_000` (`:45`); reaching it cancels the walk and drops queued batches (`:447-449`, `:478-480`), so a strong name hit that arrives after the cap is lost. The footer already says "First 20000 files · refine the search" (`:1596`).
- Cmd-K path mode fuzzy-matches folders but prefix-matches files (`crates/crow-app/src/palette.rs:1048`, `:1055`, `:1102-1108`). Hidden folders are absent from the listing until the partial starts with `.`, because `spawn_path_listing` lists with the view's hidden setting (`crates/crow-app/src/window.rs:2850-2865`) and an existing test enshrines that (`crates/crow-app/src/window/tests.rs:4449`).
- The user's running config has `show_hidden_files = true` and the `[hidden]` exclusion set off (M8.5 index). The corpus must still cover hidden on and off and rules on and off, because the defaults and other machines differ.
- **The exact repro (Q1, answered 2026-11-18):** the folder is `~`, the item is `~/.backup_excludes` (a file directly in home), scope Folder, match All, Show Hidden Files on. Typing `.back` finds it; typing `.backu` clears the result list and leaves the footer searching. A direct probe of `FuzzyMatcher` on 2026-11-18 shows `.back`, `.backu`, `.backup`, `.backup_` and `.backup_e` all match `.backup_excludes` with contiguous ranges (`.backu` → score 152, range 0..6), so the matcher is not the cause. The miss is app-side (cap/cancellation/streaming) or a walker-exclusion the diagnostic must name, not a failed fuzzy score.
- Existing fixtures do not cover this: `EdgeTree` (`crates/crow-fs/src/testing.rs:112`) has dot names, packages and symlinks but no `backup` names, no exclusion-set layout and no expected outcomes; `--make-search-fixture` (`crates/crow-app/src/main.rs:68`) is a nested perf tree; `--bench-search` (`:85`) runs the engine without the app's scope, match, hidden or cap.
- Tests cover the pieces separately (dev rules and sentinels, hidden, fuzzy scoring, content hits, batches, cancellation; `docs/testing.md:64`), not the reported combinations, and nothing answers "why was this path not in the results?".

## Target

- One deterministic corpus, built by one function and one CLI command, used by `crow-search` tests, `crow-app` tests, the bench and S2–S7.
- `crow_search::explain(root, query, target)` returns the stage that decided the candidate: outside the root, hidden, excluded (with the rule), symlink not followed, unreadable, invalid query, no match, or included (with the name score/ranges and the content line).
- A query log for every Cmd-F search records root, scope, requested and effective match mode, hidden, exclusion sets, `everything`, scanned, skipped, files, hits and the cap; `tracing` lines carry it. `explain` answers the per-candidate question; the log answers the per-query one.
- Named cases for the user's report and for dot names, `backup` names, excluded folders, a package, a symlink and the 100k subtree. Current behavior is pinned; known misses are recorded and left for S2–S4 to flip.

## Items

### 1 Corpus builder and CLI

**Change:** add `crates/crow-search/src/corpus.rs` (public, no new dependency):

```text
corpus/
  backup/{backup.txt, back.txt, nested/backup-note.txt}
  .backup/.backup-inside.txt
  Backups/backup-plan.md
  backup-excludes/backup-rules.toml
  savegameBackup/savegame.txt
  tm-backup/tm-backup.txt
  do-not-backup.txt
  .backup.txt
  .backup_excludes               # the user's item; "needle\n"
  back.txt
  backup.txt
  notes.txt                      # "backup in contents\n"
  Cargo.toml                     # sentinel for target/
  package.json                   # sentinel for build/ and dist/
  .git/config
  node_modules/backup-pkg/backup-index.js
  target/debug/backup-binary
  build/backup-out.js
  dist/backup-bundle.js
  CrowBackup.app/Contents/{Info.plist, MacOS/CrowBackup}
  link-to-backup -> backup
  link-to-backup.txt -> backup.txt
  dangling -> missing-target
  loop-a -> loop-b
  loop-b -> loop-a
  unreadable/backup-hidden.txt   # folder mode 000
  empty/
  wide/backup-000000.txt …       # optional, N files
```

- `pub fn build(root: &Path, wide: usize) -> io::Result<()>`; `wide = 0` skips the 100k subtree. All regular files contain `"needle\n"` unless the layout says otherwise; the wide files contain `"needle in wide/{i}\n"`.
- Deterministic: fixed bytes, modes 0644/0755 (the unreadable folder 000), relative symlink targets, no timestamps asserted. Build twice under `/tmp` and compare a sorted manifest of relative path, kind, size, content hash, link target and mode.
- Cleanup restores the unreadable folder's mode before removal (`std::fs::set_permissions`); `remove_dir_all` must not follow the symlinks or the loop. The test skips the unreadable assertions when `read_dir` succeeds anyway (root CI).
- Add `crow --make-search-corpus DIR [N]` (`N` default 100000, `N=0` for the small tree) to the `main.rs` doc comment and argument parsing, next to `--make-search-fixture`. Mark the fixture writes `io-ok: CLI fixture, before any window opens`, as the existing fixture does.

**Tests:** build twice and compare the manifest; `wide = 0` builds in under a second; every named path exists with the expected kind; the 100k form is CLI-only (not in `cargo test`).

**Done when:** `cargo run --release -p crow-app -- --make-search-corpus target/fixture-search-corpus` prints the count and the tree matches the layout; the determinism test passes.

### 2 `crow_search::explain`

**Change:** add `crates/crow-search/src/explain.rs` and export it from `lib.rs`:

```rust
pub fn explain(root: &Path, query: &Query, target: &Path) -> Explanation;
pub struct Explanation { pub target: PathBuf, pub outcome: Outcome }
pub enum Outcome {
    Included { hits: Vec<Hit> },
    OutsideRoot { root: PathBuf },
    Hidden { path: PathBuf },
    Excluded { path: PathBuf, rule: String },
    SymlinkNotFollowed { path: PathBuf },
    Unreadable { path: PathBuf, error: String },
    NoMatch,
    Missing,
    InvalidQuery(String),
}
```

- Make the root and target absolute lexically (`std::path::absolute`), not canonical: a target inside a symlinked folder must keep the link's name. Require the target to be the root or below it; otherwise `OutsideRoot`.
- Walk the components from the root to the target in walker order: hidden first (`name.starts_with('.')` and `!show_hidden`; the root itself is never hidden at depth 0), then, for a real directory only, `RuleSet::skip` (`everything` uses `RuleSet::empty()`), then unreadable (`read_dir` error). A symlink component before the target is `SymlinkNotFollowed`; a final symlink is not followed either, so only its own name/content can match. Note that the walker passes `is_dir = false` for a symlink, so a symlink named like an excluded folder is not skipped by the rules; mirror that.
- Match the target exactly as the walker does: skip the root (depth 0); a name query uses `FuzzyMatcher::indices`; a content query uses `ContentPattern`/`ContentSearch` for a non-directory and reports the first line plus `more`. A file that matches both yields both hits, in the same order as the engine. A directory never produces a content hit. Reuse the engine's private types by making them `pub(crate)`; do not duplicate the regex, binary detection or `more` logic.
- `InvalidQuery` comes from the same `ContentPattern::new` error before any walk; `Missing` is a target that does not exist (use `symlink_metadata`, so a dangling or looping link is still an entry the walker sees: it can be a name hit and is not `Missing`); `NoMatch` is a reachable target with no name or content hit.
- A package is an ordinary directory to the engine; `explain` must not special-case `.app`. The app's row marks packages (`crates/crow-app/src/search.rs:2350`), and S3 decides whether search should descend; S1 records today's behavior.
- Add `impl fmt::Display for Explanation` for the CLI and logs, one line: `included: name score=… ranges=…; content line=…` or `excluded: <path> (<rule>)`.
- Add `crow --search-explain DIR QUERY TARGET` with `--hidden`/`--no-hidden`, `--everything`, `--names`/`--contents` (default All). It loads `settings.toml` and `exclude.toml` from `crow_core::config_dir()` exactly like the app; a config error prints and falls back to the built-in defaults so the diagnostic still runs. This is the command that diagnoses the user's actual folder from Q1.

**Tests:** one test per outcome (outside root, hidden file and hidden folder, name/sentinel/path exclusion, symlink ancestor, final symlink, unreadable, missing, no match, invalid regex, name hit, content hit, both). The agreement test: for every corpus case, run `search` and `explain`; the target is in the hits exactly when `Outcome::Included`. Add a case where a symlink to a file gets a content hit (the engine follows it) and one where a symlink to a folder is a name hit but its child is `SymlinkNotFollowed`.

**Done when:** the agreement test passes for every case; the CLI on the corpus prints the same reason the test asserts; nothing calls `explain` from the UI.

### 3 Query log

**Change:** add `crates/crow-app/src/search.rs::QueryLog` and keep the latest record on `SearchView`:

```rust
pub struct QueryLog {
    pub root: Option<PathBuf>,          // None for This Mac (Spotlight, names only)
    pub scope: Scope,
    pub matching: Match,                // the user's choice
    pub effective_matching: Match,      // This Mac forces Names
    pub show_hidden: bool,
    pub everything: bool,
    pub skip_label: Option<String>,     // "Skip dev folders": the enabled exclusion sets
    pub names: Option<String>,
    pub contents: Option<String>,
    pub scanned: u64,
    pub skipped: usize,
    pub error: Option<String>,
    pub files: usize,
    pub hits: usize,
    pub capped: bool,
    pub dropped_hits: usize,            // hits discarded after the cap
    pub dropped_names: Vec<PathBuf>,    // up to 10 name hits discarded at the cap
    pub cancelled: bool,                // a newer search or closing the panel
    pub started: Instant,
    pub elapsed: Option<Duration>,
}
```

- Set the record in `start_search` before spawning, after trimming and after `effective_match_for`; update `files`, `hits`, `capped`, `scanned`, `skipped` and `error` in `apply`/`add_hits`/`add_spotlight`; mark the previous record `cancelled` when `generation` advances or the panel closes. This is diagnostic state only; no rendering uses it.
- When `add_hits` stops at the cap, count the rest of that batch and record up to 10 name hits in `dropped_names`. If the user's `.backup_excludes` hit reached that batch, its path appears in the dropped list; if the walk was cancelled before visiting it, the log still says `capped` and `explain` says `Included`, which is enough to name the cap as the cause. The scan is one batch, not the whole walk, and changes no result.
- Emit `tracing::debug!` at start (`search start`: root, scope, matching, effective_matching, hidden, everything, skip_label, names, contents) and at done (`search done`: elapsed, scanned, skipped count, files, hits, capped, error). A search that only has the start line is still walking, which separates "keeps loading" from a miss. Emit each skipped folder and rule at `trace` (`search skip`), so a per-folder explanation is available without logging hundreds of lines at the default debug level. The normal app log is `~/Library/Logs/Crow/crow.log`; `RUST_LOG=crow=debug` (and `=trace` for skips) turns it on.
- Add `#[cfg(test)] pub fn last_query_log(&self) -> Option<&QueryLog>`; the skipped paths stay in `self.skipped` and are asserted there, not cloned 20,000 times into the record.

**Tests:** a Cmd-F folder search records the folder root, `Scope::Folder`, `Match::All`, the hidden setting, the enabled set label, names and contents; switching Home/This Mac, All/Names/Contents and hidden/everything updates the record and the effective match (This Mac is Names). A second query marks the first record `cancelled`. The cap test (below) asserts `capped`, `dropped_names` and the footer.

**Done when:** the log fields match the app's actual query; `RUST_LOG=crow=debug` shows one start line per search and a done line when it finishes; a superseded search is marked `cancelled`; the log does not affect results or timing.

### 4 Named cases

**Change:** add `pub fn cases() -> &'static [Case]` to `corpus.rs`:

```rust
pub struct Case {
    pub id: &'static str,
    pub root: &'static str,        // relative to the corpus root
    pub names: Option<&'static str>,
    pub contents: Option<&'static str>,
    pub show_hidden: bool,
    pub everything: bool,
    pub target: &'static str,      // relative to the corpus root
    pub current: Current,          // the behavior on main today
}
pub enum Current { Included, Hidden, Excluded(&'static str), SymlinkNotFollowed, Unreadable, OutsideRoot, NoMatch }
```

At least these cases, each with a test name in the failure message:

| id | root | query | target | current |
| --- | --- | --- | --- | --- |
| `backup-folder` | corpus | names+contents `backup`, hidden on | `backup/backup.txt` | Included |
| `backup-file` | corpus | names `backup` | `backup.txt` | Included |
| `backup-excludes-name` | corpus | names `.backu`, hidden on | `.backup_excludes` | Included (engine; the app repro is separate) |
| `back-prefix` | corpus | names `back` | `backup.txt` | Included |
| `backup-not-back` | corpus | names `backup` | `back.txt` | NoMatch |
| `backup-content` | corpus | contents `backup` | `notes.txt` | Included (line 1) |
| `backup-hidden-file` | corpus | names `backup`, hidden off | `.backup.txt` | Hidden |
| `backup-hidden-folder` | corpus | names `backup`, hidden off | `.backup/.backup-inside.txt` | Hidden |
| `backup-hidden-shown` | corpus | names `backup`, hidden on | `.backup.txt` | Included |
| `backup-excluded-name` | corpus | names `backup`, rules on | `node_modules/backup-pkg/backup-index.js` | Excluded (`dev: name "node_modules"`) |
| `backup-excluded-sentinel` | corpus | names `backup`, rules on | `target/debug/backup-binary` | Excluded (`dev: target beside Cargo.toml`) |
| `backup-everything` | corpus | names `backup`, everything on | `node_modules/backup-pkg/backup-index.js` | Included |
| `backup-outside-folder` | `backup` | names `backup` | `backup.txt` (sibling) | OutsideRoot |
| `backup-symlink-child` | corpus | names `backup` | `link-to-backup/backup.txt` | SymlinkNotFollowed |
| `backup-symlink-name` | corpus | names `backup` | `link-to-backup` | Included (name, not a folder) |
| `backup-symlink-content` | corpus | contents `needle` | `link-to-backup.txt` | Included (content; the walker follows a file link) |
| `backup-package` | corpus | names `backup` | `CrowBackup.app` | Included (directory) |
| `backup-package-inside` | corpus | contents `needle` | `CrowBackup.app/Contents/Info.plist` | Included (package descended today; S3 decides) |
| `backup-unreadable` | corpus | names `backup` | `unreadable/backup-hidden.txt` | Unreadable (skip as root) |

- The user's case from Q1 becomes the case `user_backup_excludes`: root `~`, query `.backu`, target `~/.backup_excludes`, scope Folder, match All, hidden on. The report is that `.back` finds it and `.backu` clears the list and keeps searching; the engine probe says `.backu` matches, so the case's corpus twin (`backup-excludes-name`) is `Included` at the engine and the real path is a hand check. Run `--search-explain ~ .backu ~/.backup_excludes`; if it says `Included`, run the app on `~` with `RUST_LOG=crow=debug`, reproduce `.backu`, and record `capped`, `files`, `hits`, `scanned`, `skipped` and whether the name hit arrived before the cap. If it says `Hidden`/`Excluded`/`OutsideRoot`, record that rule instead. The corpus twin is the CI case; the real path stays out of tests.
- Cmd-F wiring: one GPUI test opens the small corpus and searches `backup` in Folder scope, the only scope whose root is the fixture (Home is the real home in tests, and This Mac needs Spotlight). It switches All/Names/Contents, hidden on/off and rules on/off, then asserts the result set and the query log. Scope switching itself is covered by the existing empty-query test; the log records Home/This Mac when switched, without walking the real home.
- Cmd-K wiring: a GPUI test on the corpus types `<corpus>/back` and `<corpus>/ack`. Today `backup.txt` appears for `back` (prefix) and not for `ack`; a dot folder appears only after `.`. Name these as current-behavior tests; Q2/Q3 will change them in S2.
- Cap: a GPUI test feeds `MAX_RESULTS` synthetic name hits through `add_test_hits`, then a strong name hit, and asserts `capped`, the footer wording, that the strong hit is absent from the results, and that its path is in `dropped_names`. This reproduces the late-strong-hit loss without building 100k files. Make `MAX_RESULTS` `pub(crate)` or expose a test accessor; do not change the value.

**Tests:** the table is a test (`explain` and `search` agree); the app and palette tests above; the cap test; the user case.

**Done when:** every row has a passing characterization test, the progress file lists which rows are known misses and which decision (Q1–Q5 / S2–S4) flips each, and no behavior changed.

### 5 Docs

**Change:** update `docs/testing.md`:
- the `crates/crow-search` test paragraph (corpus, `explain`, agreement test);
- the Performance commands: `--make-search-corpus target/fixture-search-corpus`, `--bench-search` on `back` and `backup`, and the 100k record;
- a "Search corpus and diagnostics" subsection with `--search-explain`, `RUST_LOG=crow=debug` and `=trace`, and what each line means;
- the search hand checks: the corpus in Folder/Home, All/Names/Contents, hidden on/off, rules on/off, Cmd-K path mode, the 100k cap, and the user's case.

Link S1 from `docs/plans/m8.5/README.md`. Do not change README's status line; M8.5 is still next.

**Tests:** none.

**Done when:** a reader can build the corpus, run a search, read the log, and explain a candidate without reading the source.

## Risks and decisions

- **The 100k subtree must not enter `cargo test`.** It is CLI/bench/hand-check only; tests use `wide = 0`.
- **The unreadable folder needs cleanup.** Restore mode 000 before removal; if the process can read it anyway (root), skip the assertion rather than failing.
- **`explain` must mirror the walker exactly or it lies.** Hidden before rules, `is_dir = false` for symlinks, root depth 0, file symlinks followed for content, read errors swallowed. The agreement test is the guard; if it disagrees, fix `explain`, not the engine.
- **The package case is current truth, not a decision.** The engine descends into `.app`; the app marks the directory as a package. S3 decides whether that is desired. Do not "fix" it here.
- **No main-thread IO.** `explain` is for the CLI and tests; the UI never calls it. The query log only formats state the app already has.
- **`ignore`'s hidden filter is dot-only on Unix.** A Finder-hidden, non-dot folder such as `~/Library` is walked even with Show Hidden Files off. The corpus covers dot names; the Finder hidden flag is a S2/S3 decision. Record how much of the user's `~` search comes from `~/Library` in the repro log; it is a likely cap filler.

## Questions to ASK

**Answered 2026-11-18; do not re-ask.** The questions stay for context; the answers are the desired values for S2–S5 and the case table.

1. **Exact repro for "backup".** Which folder, which item, which scope (Folder/Home/This Mac), which match mode (All/Names/Contents), and was View > Show Hidden Files on? Recommended: add it to the corpus as `user_backup_excludes`; if it cannot be reproduced, `--search-explain` should still name the miss. Unblocks Item 4.
2. **Hidden and dot names.** Should Cmd-F and Cmd-K find dot-named items when Show Hidden Files is off? Finder's search can surface hidden items even when the list hides them. Recommended: search includes them, marks them as hidden, and Cmd-K path mode matches them by name without a leading dot. This is S2's product decision; S1 records it as the desired value of the hidden cases.
3. **Cmd-K files vs folders.** Today Cmd-K is "go to a folder": the empty query lists folders, and files appear only under a typed path, prefix-only. Should a typed query also fuzzy-match files in the current folder? Recommended: yes for a typed query, folders first; the empty query stays places-only. This is the closest fix to "Cmd-K can't find files"; S1's palette cases record the current prefix-only behavior.
4. **Cmd-F default scope.** Current folder (today), Home, or This Mac? Recommended: keep the current folder as the default and make the scope bar and the empty state say so; add a one-key "Search Home" path from a miss. Changing the default is a bigger product change and should be explicit.
5. **Cap policy.** When 20,000 results are reached, should exact/contiguous name hits always be kept even if they arrive late? Recommended: reserve the cap for content and weak fuzzy hits, and always keep a strong name hit; the footer should say what was dropped. S1's synthetic cap test records today's loss.

**Answers (2026-11-18).**

1. Folder `~`, item `~/.backup_excludes`, scope Folder, match All, Show Hidden Files on. `.back` finds it; `.backu` clears the result list and keeps loading. S1 must reproduce this and name the app-side cause; the engine matcher already includes the item.
2. **Include both.** Cmd-F and Cmd-K find dot names with Show Hidden Files off, mark them hidden, and Cmd-K matches them without a leading dot. S2 changes the hidden cases and the palette test.
3. **Fuzzy files.** A typed Cmd-K query fuzzy-matches files in the current folder, folders first; the empty query stays places-only. S2 changes `path_listing`'s file filter.
4. **Keep the current folder** as the Cmd-F default; make the scope bar and the empty state say so, and add a one-key Search Home from a miss. This is S3's scope work.
5. **Protect strong names.** Always keep exact/contiguous name hits; reserve the cap for content and weak fuzzy hits; the footer says what was dropped. This is S4's cap work; S1's synthetic test records today's loss.

## Hand checks

- Build the corpus and open it: Cmd-F `backup` in Folder and Home, All/Names/Contents, hidden on/off, rules on/off; compare with the case table and the footer. Names only is fast; Contents reads every file.
- Cmd-K on the corpus: type `<corpus>/back` and `<corpus>/ack`; a dot folder with hidden off and after typing `.`; record the current behavior (S2 changes it).
- `RUST_LOG=crow=debug` (and `=trace` for skips): search `backup`; check one start and one done line with root, scope, match, hidden, exclusions, scanned, skipped, capped. A cancelled query logs the old record as cancelled.
- The user's repro: `crow --search-explain ~ .backu ~/.backup_excludes --hidden` should print `included`; then run the app on `~`, Cmd-F `.back` (item appears), then `.backu` (the report: list clears, footer keeps searching). With `RUST_LOG=crow=debug`, record `capped`, `files`, `hits`, `scanned`, `skipped` and whether the name hit was dropped at the cap. If the engine says included and the app drops it, the S1 diagnosis is the cap/cancellation path, not the matcher or an exclusion; record it in the progress file.
- 100k: `--make-search-corpus target/fixture-search-corpus`, search `backup`; the footer reaches the cap, the app does not freeze, and the query log says `capped=true`. Run `--bench-search` on `back` and `backup` and record first-hit/total for S5's baseline.

## Finish

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- Build and run the corpus CLI twice (small) and compare the manifest; build the 100k corpus once and run the hand checks. Record the bench numbers; there is no S1 budget (S5 sets it).
- Update `docs/testing.md` and the M8.5 index link.
- Commit per item, keep the progress file current, stop at "Ready for critique". Do not push; the critic opens the PR.
