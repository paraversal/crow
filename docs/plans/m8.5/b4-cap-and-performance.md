# B4 — Cap and performance

Audience: an implementing agent. Follow the `execute-plan` skill; read `AGENTS.md` and the M8.5 index (`docs/plans/m8.5/README.md`, "Decisions" and "Architecture" 9) first.
Base: branch `m8.5-b4-cap` from `main` **after B3 is merged**.
Progress file: `docs/plans/m8.5/b4-cap-and-performance.progress.md`. PR title: "M8.5 B4: two-budget result cap, stop-contents, search bench".

## Why

Today the 20,000-row cap (`MAX_RESULTS`, `search.rs:45`) counts every row. When it fills, `add_hits` cancels the walk (`:478-481`) and `apply` drops every queued batch (`:449`). So a broad content match in a large tree can push out a file whose **name** is exactly what the user typed. The user decided: always keep strong (exact/contiguous) name hits, spend the cap on content and weak fuzzy hits, and have the footer say what was dropped.

## Items

### 1 `Control`: cancel, or stop reading contents

**Change** (`crates/crow-search/src/engine.rs`, exported from `lib.rs`):

```rust
/// Shared between the caller and a running search.
#[derive(Clone, Debug, Default)]
pub struct Control {
    cancel: Arc<AtomicBool>,
    stop_contents: Arc<AtomicBool>,
}

impl Control {
    pub fn new() -> Self;
    /// Stop the walk and every read.
    pub fn cancel(&self);
    pub fn is_cancelled(&self) -> bool;
    /// Keep walking for names, but read no more file contents (and stop the current read).
    pub fn stop_contents(&self);
    pub fn contents_stopped(&self) -> bool;
}
```

- `search(root, query, sink, control: &Control) -> Summary` replaces the `cancel: &Arc<AtomicBool>` parameter. `build_walker`'s `filter_entry`, the walker closure and the receive loop check `control.is_cancelled()`. The content search runs only when `!control.contents_stopped()`. `ContentSink::matched` returns `Ok(false)` on either flag. Use `Ordering::Relaxed` throughout, as today.
- `Summary` gains `pub contents_stopped: bool`, the flag's value when the walk ended.
- Update every caller: `search.rs`, `main.rs` (bench, trace), the engine tests and the corpus test.

**Tests:**
- `stop_contents_keeps_names_streaming`: a tree with 50 files containing `needle` and 50 files named `needle-*`. The sink calls `control.stop_contents()` on the first content hit. Afterwards every `needle-*` name hit still arrives, at most the in-flight content hits arrive, and `summary.contents_stopped` is true.
- `cancel_stops_everything`: the existing cancellation test, ported to `Control`.

### 2 Two budgets

**Change** (`crates/crow-app/src/search.rs`):
- Replace `MAX_RESULTS` with:

```rust
/// Rows for strong name matches (the name contains the query contiguously). When
/// these fill, the search is cancelled: the query is too broad to be useful.
const MAX_STRONG_ROWS: usize = 20_000;
/// Rows for everything else: content-only and weak (scattered) fuzzy name hits.
/// When these fill, contents stop being read; names keep streaming.
const MAX_OTHER_ROWS: usize = 20_000;
```

- `SearchView` replaces `capped: bool` and `cancel: Arc<AtomicBool>` with `control: Control`, `names_capped: bool`, `contents_stopped: bool` and `dropped: usize` (hits that would have been new rows but didn't fit). Reset all four in `start_search`.
- `add_hits` decides per hit, **before** building a `Row`:
  - A path already shown (`by_path` or `fresh_at`): merge as today. No budget is used.
  - New strong name (`HitKind::Name` and `is_strong_name(&hit.path, &ranges, &query)`; compute it once and pass it to `Row::from_hit` instead of computing it again inside): if `name_count + fresh strong >= MAX_STRONG_ROWS`, set `names_capped = true`, call `self.control.cancel()`, and stop processing the batch. Otherwise accept it.
  - Any other new row: if `content_count + weak rows + fresh others >= MAX_OTHER_ROWS`, add 1 to `dropped`, and the first time also set `contents_stopped = true` and call `self.control.stop_contents()`. If the query has no names part (`Match::Contents`), call `self.control.cancel()` instead, since nothing more can be shown. **Continue** with the next hit (a strong name later in the same batch must still get in).
- `apply`: `Update::Hits(_) if self.names_capped => {}` (replaces `self.capped`).
- `add_spotlight`: every Spotlight row is strong, so cap it with `MAX_STRONG_ROWS` and `names_capped`.
- Keep the weak-row count explicit: add `weak_count: usize`, maintained in `insert_rows` (rows after `name_count + content_count`). Don't derive it with subtraction in several places.

**Tests** (GPUI, through `add_test_hits`; make the constants `pub(crate)` for the tests):
- `strong_name_after_the_cap_is_kept`: feed `MAX_OTHER_ROWS` content hits, then one strong name hit. The strong row is first, `result_count() == MAX_OTHER_ROWS + 1`, `contents_stopped()` is true, and the walk is not cancelled.
- `weak_hits_after_the_cap_are_counted`: then 5 weak name hits and 5 content hits for new paths: no new rows, `dropped == 10`.
- `strong_names_cap_cancels`: feed `MAX_STRONG_ROWS + 1` strong hits: `names_capped`, and the control is cancelled.
- `contents_only_cap_cancels`: Contents match, `MAX_OTHER_ROWS + 1` content hits: the control is cancelled.
- Port the existing cap test(s); search for `capped` and `First 20000` in `window/tests.rs`.

### 3 Footer

**Change:** `FooterState` replaces `capped` with `names_capped`, `contents_stopped` and `dropped`. `footer_summary`, in order:

1. `error` → as today.
2. `names_capped` → `"First 20000 name matches · refine the search"`.
3. `contents_stopped` and searching → `"Searching names… {results} · {dropped} more not shown"`.
4. `contents_stopped` and done → `"{results} · {dropped} more not shown · contents stopped at 20000"`.
5. Otherwise → as today.

`{results}` is today's `"N results"` / `"N results in M files"` text. Write `20000` from the constants, unformatted, to match today's "First 20000 files". Colour: `warning` for 2–4, as `capped` is today. The aria label is the same string (it is announced once per finished search, as today). The text ellipsizes at 640 px, as today.

**Tests:** unit tests for `footer_summary`, one per branch, with exact strings.

### 4 Selection stays put under late strong names

**Change:** none expected. `insert_rows` only moves rows to the right and `follow_selection` keeps the cursor on its path. Add the missing test, and fix only if it fails.

**Test** `late_strong_name_keeps_the_cursor_row`: 30 content hits, then the user presses Down twice (cursor on row 2, `user_moved`), then 5 strong names arrive. The selected path is unchanged, and `cursor()` is now its new index (7).

### 5 Bench and budget

**Change:** `--bench-search DIR [QUERY] [--names|--contents]` (from B1, plus the match flags). It prints `first_hit`, `first_name_hit`, `total`, `hits`, `scanned`, `skipped` and `contents_stopped`. It doesn't apply the app's cap (it is an engine bench), and it says so in its doc comment.

Record, on `main` (in a separate worktree, built `--release`) and on the branch, **three runs each, median**, on `target/fixture-search-200k`:

| query | flags |
| --- | --- |
| `needle` | (all) |
| `back` | (all) |
| `backup` | (all) |
| `file-01-02` | `--names` |

**Budget** (write it into `docs/testing.md`'s performance table): first hit ≤ `main` + 5 ms; total ≤ `main` × 1.10. A regression past that is a blocker: profile with `samply` or Instruments before changing code. If the cause isn't obvious, ASK.

### 6 Docs

`docs/testing.md`: the footer states, the two budgets, the bench command and budget. Remove "First 20000 files · refine the search" wherever it's quoted.

## Hand checks

- `target/fixture-search-200k`, Cmd-F `needle` (All): the footer reaches "… more not shown · contents stopped at 20000". Then type `file-01-02-00001` without leaving search: that exact file is the top row. Before this batch it could be missing.
- The same folder, Names `file`: the footer says "First 20000 name matches · refine the search", and typing more narrows.
- While results stream, arrow down a few rows: the selection doesn't jump when batches arrive.
- Light/dark, 640 px: the footer text ellipsizes and doesn't wrap; VoiceOver reads the footer once when the search settles.

## Questions to ASK

None expected. ASK if the bench budget is missed and the cause isn't in this batch's change.

## Finish

`cargo fmt --check` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo test --workspace` · bench recorded in the progress file · the hand checks · commit per item · progress file current · stop at "Ready for critique" (do not push).
