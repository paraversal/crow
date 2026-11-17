# B1 — Streaming fix, corpus and trace

Audience: an implementing agent. Follow the `execute-plan` skill; read `AGENTS.md` and the M8.5 index (`docs/plans/m8.5/README.md`, especially "Architecture") first.
Base: branch `m8.5-b1-streaming` from `main`. Progress file: `docs/plans/m8.5/b1-streaming.progress.md`.
PR title: "M8.5 B1: search streaming fix, corpus and trace".

## Why

The user's repro: Cmd-F in `~`, Folder scope, match All, Show Hidden Files on. `.back` finds `~/.backup_excludes`; `.backu` clears the list and the footer keeps saying "Searching…". The matcher scores both. The engine is the cause: `crow_search::search` passes hits on only in batches of 256 or when the walk ends (`crates/crow-search/src/engine.rs:257-268`). `.backu` has fewer than 256 matches, and the walk reads every file in the home folder, so nothing arrives for minutes. Two related defects make it worse: content search opens FIFOs (which block forever), and a file's name hit waits for its own content read.

B1 fixes those three, adds typed skip reasons (with unreadable folders, which are silent today), and builds the small corpus and trace tool that B2–B5 test against. It does not change the hidden policy, the cap, the palette or any UI text.

## Items

### 1 `Batcher`: send the first hit at once, then every 30 ms or 256 hits

**Change** (`crates/crow-search/src/engine.rs`):

```rust
/// How long a hit may wait in `pending` before it is sent, once the first batch is out.
const FLUSH: Duration = Duration::from_millis(30);

/// Decides when pending hits go to the sink. The first hit goes at once (time to
/// first result); after that a batch goes when BATCH hits are pending or FLUSH has
/// passed since the last send. Pure: the caller passes the clock.
struct Batcher {
    pending: Vec<Hit>,
    last_sent: Option<Instant>,
}

impl Batcher {
    fn new() -> Self;
    /// Add hits; returns the batches to send now (each at most BATCH long).
    fn push(&mut self, hits: Vec<Hit>, now: Instant) -> Vec<Vec<Hit>>;
    /// Called when the receiver times out: the pending hits if FLUSH has passed.
    fn due(&mut self, now: Instant) -> Option<Vec<Hit>>;
    /// The walk ended: whatever is left.
    fn finish(self) -> Option<Vec<Hit>>;
}
```

Rules for `push`:
- If `last_sent` is `None` and `pending` is now non-empty, send it all at once, in BATCH-sized slices, and set `last_sent = now`.
- Otherwise, while `pending.len() >= BATCH`, send BATCH-sized slices (as today: `split_off`, so one huge file can't make one huge batch).
- Then, if `pending` is non-empty and `now - last_sent >= FLUSH`, send the rest.
- Every send sets `last_sent = now`.

Replace the receive loop (`for batch in rx { … }`) with:

```rust
let mut batcher = Batcher::new();
loop {
    let ready = match rx.recv_timeout(FLUSH) {
        Ok(hits) => batcher.push(hits, Instant::now()),
        Err(RecvTimeoutError::Timeout) => batcher.due(Instant::now()).into_iter().collect(),
        Err(RecvTimeoutError::Disconnected) => break,
    };
    for batch in ready {
        sink(batch);
        if cancel.load(Ordering::Relaxed) { break; }
    }
    if cancel.load(Ordering::Relaxed) { break; }
}
if !cancel.load(Ordering::Relaxed) && let Some(rest) = batcher.finish() { sink(rest); }
```

Keep `BATCH = 256`. The app already gives the window a frame every 8 ms while it drains (`search.rs`'s update loop), so a batch every 30 ms adds no main-thread work beyond one notify.

**Tests** (unit, in `engine.rs`, with `Instant`s built from one `base` plus offsets, never `sleep`):
- `batcher_sends_the_first_hit_at_once`: one hit at `t0` → one batch of 1.
- `batcher_holds_hits_until_flush`: after the first send, 10 hits at `t0+5ms` → nothing; `due(t0+20ms)` → None; `due(t0+36ms)` → the 10.
- `batcher_sends_full_batches_immediately`: after the first send, 600 hits at `t0+1ms` → two batches of 256; `finish` → 88.
- `batcher_never_sends_empty`: `due` and `finish` on an empty batcher → None.

**Done when:** the four tests pass and the existing `batches_are_bounded_for_many_files` still passes.

### 2 Name hits before contents; contents of regular files only

**Change** (the walker closure in `search`):
- Send the name hit with its own `tx.send(vec![hit])` before the content search runs, and send the content hits in a second `tx.send` afterwards. The `more` count logic stays on the content `Vec`. The app already merges a later content hit into an existing row (`add_hits`, `by_path` branch), and it lands on the same row because the name hit arrived first.
- Read contents only when the entry is a regular file, or a symlink whose target is a regular file:

```rust
let file_type = entry.file_type();
let readable = file_type.is_some_and(|t| t.is_file())
    || (file_type.is_some_and(|t| t.is_symlink())
        && std::fs::metadata(entry.path()).is_ok_and(|m| m.is_file()));
```

  `std::fs::metadata` costs one stat, and only for symlinks.

**Tests:**
- `content_search_skips_fifos`: create a FIFO (`libc::mkfifo`, add `libc = "0.2"` to `crow-search`'s `[dependencies]`; `crow-fs` and `crow-app` already use that version) next to a file containing `needle`. Run `search` for contents `needle` on a thread and wait on a channel with `recv_timeout(10 s)`. It must finish, with exactly the file's hit.
- `name_hit_arrives_before_its_content_hit`: one file whose name and contents match. The sink sees the `Name` hit no later than the `Content` hit (collect every batch in order and compare positions).
- The existing "symlink to a file gets a content hit" behavior still holds (add a test if none exists).

### 3 Typed skips, unreadable folders reported

**Change:**
- In `engine.rs`:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skip { pub path: PathBuf, pub reason: SkipReason }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// An exclude.toml set: "dev: name \"node_modules\"" (today's strings).
    Rule(String),
    /// The folder could not be read: "no permission", or the error text.
    Unreadable(String),
}

impl SkipReason {
    /// Whether Search everything would walk it.
    pub fn lifted_by_everything(&self) -> bool { matches!(self, SkipReason::Rule(_)) }
}
impl fmt::Display for SkipReason { /* Rule(s) => s, Unreadable(s) => s */ }
```

  `Summary.skipped` becomes `Vec<Skip>`. Export `Skip` and `SkipReason` from `lib.rs`. B2 and B3 add the `HiddenFolder` and `Package` variants; keep the enum non-exhaustive in spirit (match with a wildcard where only the UI's text is needed).
- In the walker closure, `Err(error)` records an `Unreadable` skip instead of being dropped. Take the path from `ignore::Error`: write `fn error_path(error: &ignore::Error) -> Option<&Path>`, which unwraps `WithDepth`, `WithLineNumber` and `WithPath { path, .. }` (recursively), and returns `None` otherwise. Text: `"no permission"` when the inner `io::Error` kind is `PermissionDenied`, otherwise the inner error's `to_string()`. Unreadable entries with no path are dropped as today. They go into the same `skipped` mutex; the sort by path stays.
- In `crates/crow-app/src/search.rs`, `self.skipped` becomes `Vec<Skip>`, and **in B1 it keeps only `Rule` skips** (filter in `apply`'s `Done` arm), so the footer, the list and the empty state behave exactly as today. B3 shows the other reasons. Update `render_skipped` and `skipped_display` for the new type.
- `main.rs`'s `bench_search` prints `skipped` as before (the count of every reason).

**Tests:** `unreadable_folder_is_reported`: a folder with mode `000` holding a matching file. The summary has one `Unreadable("no permission")` skip for it and no hit inside. Restore the mode before cleanup. Skip the assertion (with a `println!` saying why) when `read_dir` succeeds anyway (root). The existing rule-skip tests are updated to the new type, with the same strings.

### 4 Corpus

**Change:** add `crates/crow-search/src/corpus.rs`, public (`pub mod corpus` in `lib.rs`), with no new dependency beyond `libc`:

```text
<root>/
  backup/backup.txt
  backup/back.txt
  backup/nested/backup-note.txt
  .backup/.backup-inside.txt
  Backups/backup-plan.md
  backup-excludes/backup-rules.toml
  savegameBackup/savegame.txt
  tm-backup/tm-backup.txt
  do-not-backup.txt
  .backup.txt
  .backup_excludes                       # the user's item
  back.txt
  backup.txt
  notes.txt                              # "backup in contents\n"
  Cargo.toml                             # sentinel for target/
  package.json                           # sentinel for build/ and dist/
  .git/config
  node_modules/backup-pkg/backup-index.js
  target/debug/backup-binary
  build/backup-out.js
  dist/backup-bundle.js
  CrowBackup.app/Contents/Info.plist
  CrowBackup.app/Contents/MacOS/CrowBackup
  link-to-backup -> backup
  link-to-backup.txt -> backup.txt
  dangling -> missing-target
  loop-a -> loop-b
  loop-b -> loop-a
  pipe                                   # FIFO
  unreadable/backup-hidden.txt           # folder mode 000
  empty/
  crowd/note.b-a-c-k-000.txt … crowd/note.b-a-c-k-299.txt   # 300 weak matches for ".back"
```

- Every regular file contains `"needle\n"` except `notes.txt`. The `crowd/` names fuzzy-match `.back` and `back` (scattered: `.b-a-c-k`) but not `.backu` (no `u` after the `k`). Together they reproduce "`.back` fills a batch of 256, `.backu` doesn't" in a small tree.
- API:

```rust
pub fn build(root: &Path) -> io::Result<()>;
/// Restore the unreadable folder's mode and remove the tree; never follows links.
pub fn remove(root: &Path) -> io::Result<()>;
pub fn cases() -> &'static [Case];
```

- Deterministic: fixed bytes, modes 0644/0755 (the unreadable folder 000), relative symlink targets.
- No 100k subtree. Scale testing uses `--make-search-fixture` (it exists).
- CLI: `crow --make-search-corpus DIR` next to `--make-search-fixture` in `main.rs` (doc comment and argument parsing), marked `io-ok: CLI fixture, before any window opens`.

### 5 Named cases

**Change:** `Case` and its table live in `corpus.rs`; the test is `crates/crow-search/tests/corpus_cases.rs` (an integration test using only the public API).

```rust
pub struct Case {
    pub id: &'static str,
    pub root: &'static str,            // relative to the corpus root; "" is the root
    pub names: Option<&'static str>,
    pub contents: Option<&'static str>,
    pub show_hidden: bool,
    pub everything: bool,
    pub target: &'static str,          // relative to the corpus root
    pub expect: Expect,
}
pub enum Expect {
    Hit,                               // the target is in the hits
    Miss,                              // reachable, but no hit
    UnderSkip(&'static str),           // not a hit; a Skip for this ancestor (relative) exists
    NotVisited,                        // not a hit, no skip (today: hidden off, symlink child)
}
```

Default rules come from `Exclude::default()`. The table (B2 and B3 update the rows marked):

| id | root | query | target | expect |
| --- | --- | --- | --- | --- |
| `backup-folder` | "" | names+contents `backup`, hidden on | `backup/backup.txt` | Hit |
| `backup-file` | "" | names `backup` | `backup.txt` | Hit |
| `user-backup-excludes` | "" | names+contents `.backu`, hidden on | `.backup_excludes` | Hit |
| `back-prefix` | "" | names `back` | `backup.txt` | Hit |
| `backup-not-back` | "" | names `backup` | `back.txt` | Miss |
| `backup-content` | "" | contents `backup` | `notes.txt` | Hit |
| `backup-hidden-file` | "" | names `backup`, hidden off | `.backup.txt` | NotVisited (**B2 → Hit**) |
| `backup-hidden-folder` | "" | names `backup`, hidden off | `.backup/.backup-inside.txt` | NotVisited (**B2 → UnderSkip(".backup")**) |
| `backup-hidden-shown` | "" | names `backup`, hidden on | `.backup.txt` | Hit |
| `backup-excluded-name` | "" | names `backup` | `node_modules/backup-pkg/backup-index.js` | UnderSkip("node_modules") |
| `backup-excluded-sentinel` | "" | names `backup` | `target/debug/backup-binary` | UnderSkip("target") |
| `backup-everything` | "" | names `backup`, everything | `node_modules/backup-pkg/backup-index.js` | Hit |
| `backup-outside-folder` | `backup` | names `backup` | `backup.txt` | Miss (outside the root) |
| `backup-symlink-child` | "" | names `backup` | `link-to-backup/backup.txt` | NotVisited |
| `backup-symlink-name` | "" | names `backup` | `link-to-backup` | Hit |
| `symlink-content` | "" | contents `needle` | `link-to-backup.txt` | Hit |
| `backup-package` | "" | names `backup` | `CrowBackup.app` | Hit |
| `package-inside` | "" | contents `needle` | `CrowBackup.app/Contents/Info.plist` | Hit (**B3 → UnderSkip("CrowBackup.app")**) |
| `unreadable` | "" | names `backup` | `unreadable/backup-hidden.txt` | UnderSkip("unreadable") (skip the case as root) |
| `fifo-finishes` | "" | contents `needle` | `pipe` | Miss (and the search finishes) |

The test runs each case through `search` on a corpus built once per test binary (into `std::env::temp_dir()`, removed at the end), and fails with the case id. Run every search on a thread with a 10 s `recv_timeout`, so a hang fails instead of blocking CI.

**Why the user's case is covered.** On a small corpus the walk ends in milliseconds, so the old code also delivered `.backup_excludes` (at the end). No end-to-end test can show "it arrived before the walk ended" without timing. The proof is the `Batcher` unit tests (item 1) plus `user-backup-excludes` (it is found at all) plus the FIFO case (the walk ends). Do not add sleeps or timing-based assertions to get an end-to-end test.

### 6 `--search-trace` and log lines

**Change:**
- `crow --search-trace DIR QUERY [--target PATH] [--names|--contents] [--hidden|--no-hidden] [--everything]` in `main.rs`, next to `--bench-search`:
  - Load `settings.toml` and `exclude.toml` from `crow_core::config_dir()` the way the app does (hidden defaults to the setting; a config error prints and falls back to the defaults).
  - Run the real `crow_search::search` and print one line per batch, `+12ms batch 1: 3 hits`; then the totals, `first_hit=12ms total=4100ms hits=… scanned=… skipped=…`; then each skip, `skip <path> (<reason>)`.
  - With `--target`, print one verdict:
    - `target: name hit at 12ms (batch 1)` or `target: content hit line 3 at 40ms`;
    - `target: not found; under skipped <ancestor> (<reason>)`, from the summary's skips;
    - `target: not found; outside DIR`;
    - `target: not found; no skipped ancestor, and the name does not match` (checked with `FuzzyMatcher`; this is the engine's own matcher, not a copy of the walk).
  - Mark the reads `io-ok: CLI diagnostic, before any window opens`.
- `--bench-search DIR [QUERY]`: the optional query defaults to `needle` (today's behavior).
- In `search.rs`'s `start_search`, add `tracing::debug!(target: "crow::search", …)` with root, scope, matching, effective_match, show_hidden, everything, skip_label, and the query length (not its text: queries can be private). In the `Done`/`SpotlightDone` arms, add one with elapsed, first-hit ms, scanned, skipped, rows, hits, capped and error. Keep `started` and `first_hit_ms` on `SearchView` (two fields), set in `start_search` and on the first accepted hit.

**Tests:** none for the CLI beyond building; the hand checks run it.

### 7 Docs

Update `docs/testing.md`:
- the `crates/crow-search` paragraph (batcher, FIFO, typed skips, corpus cases);
- the Performance commands: `--make-search-corpus`, `--bench-search DIR QUERY`;
- a "Search diagnostics" subsection: `--search-trace` with an example, and `RUST_LOG=crow::search=debug` for the app log at `~/Library/Logs/Crow/crow.log`.

## Hand checks

- **The user's repro, before and after.** On `main` and on the branch: Cmd-F in `~`, Folder, All, Show Hidden Files on; type `.back`, then `.backu`. On the branch, `.backup_excludes` appears within a moment for both, while the footer still says "Searching…". Record how long until it appears.
- `crow --search-trace ~ .backu --hidden --target ~/.backup_excludes` prints a name hit in the first batch. Record the line and the total time in the progress file.
- `--make-search-corpus target/fixture-search-corpus`, open it, Cmd-F `backup` with hidden on and off: the same results as on `main` (B1 changes timing, not results). The search finishes despite `pipe`.
- Bench: `--bench-search target/fixture-search-200k` (`needle`), `back`, `backup`, on `main` and the branch. First hit should be the same or better. Total should be within run-to-run noise: the extra `recv_timeout` wakeups cost nothing measurable. Record both in the progress file.

## Questions to ASK

None. The product decisions are in the index. If a test shows the 30 ms flush makes the app visibly stutter during a 200k search, ASK before changing `FLUSH`.

## Finish

`cargo fmt --check` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo test --workspace` · the hand checks · commit per item · progress file current · stop at "Ready for critique" (do not push).
