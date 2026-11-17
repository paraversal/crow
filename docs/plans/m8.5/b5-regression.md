# B5 — Regression and finish

Audience: an implementing agent. Follow the `execute-plan` skill; read `AGENTS.md` and the M8.5 index (`docs/plans/m8.5/README.md`) first.
Base: branch `m8.5-b5-regression` from `main` **after B4 is merged**.
Progress file: `docs/plans/m8.5/b5-regression.progress.md`. PR title: "M8.5 B5: search oracle, corpus matrix, finish".

## Why

B1–B4 changed the walk policy (hidden folders, packages, unreadable folders), the streaming, and the cap. The corpus cases pin the reported examples. This batch adds a check that doesn't depend on someone thinking of the example: an independent oracle on random small trees. It also adds an app-level matrix over the settings the user can flip, and it closes the milestone.

## Items

### 1 Oracle on random trees

**Change:** a unit test module `crates/crow-search/src/oracle_tests.rs` (`#[cfg(test)] mod oracle_tests;` in `lib.rs`). It's inside the crate so it can use `RuleSet::skip` and `FuzzyMatcher`. The test has two parts.

**Generator.** A seeded generator (use `std`'s `u64` xorshift written in ten lines; `crow-search` has no `rand`, and adding one for a test isn't worth it) builds trees of at most 40 entries, depth at most 4, under `std::env::temp_dir()`:
- names from a small alphabet that collides on purpose: `ab`, `abc`, `.ab`, `b-a`, `node_modules`, `target`, `x.app`, `.hid`;
- some folders with a `Cargo.toml` sentinel;
- file contents from `"", "ab\n", "AB cd\n", "\0ab"` (the last one is binary);
- symlinks to a sibling file and to a sibling folder;
- one `000` folder in some trees, skipped when running as root.

**Oracle.** A plain recursive `std::fs::read_dir` walk that implements the **documented** policy, written from the index's "Architecture" section and not by copying `engine.rs`:
- the root is depth 0 and never matched;
- an exclusion rule (`RuleSet::skip(path, name, is_dir)` for real directories, rules first) gives a `Rule` skip, and the folder is neither matched nor entered;
- every other entry is matched by name (`FuzzyMatcher::indices`);
- regular files, and symlinks to regular files, are matched by contents. Use a case-insensitive literal `contains` for ASCII queries, and skip any file with a NUL in it, the way `BinaryDetection::quit` does;
- a dot folder with hidden off and not everything is matched, then gives a `HiddenFolder` skip and is not entered; a package (not everything) the same with `Package`;
- symlinked folders are never entered;
- an unreadable folder gives an `Unreadable` skip.

For each of 300 seeds × queries `{ab, a, .a, b-a}` × hidden {on, off} × everything {on, off} × match {names, contents, both}, compare as **sets**:
- `(path, kind)`, where kind is `Name` or `Content(line)`;
- `(skip path, reason variant)`.

Print the seed, the tree (as a list) and the diff on failure. Keep the test under ~5 s in debug. If it's slower, lower the seed count and record the number.

**Done when:** the oracle agrees on all seeds. **If it disagrees, decide which side is wrong against the index's policy text.** If the policy text itself is ambiguous, ASK; don't change the engine to match the oracle, or the oracle to match the engine, without that.

### 2 App-level corpus matrix

**Change:** one GPUI test in `window/tests.rs`, `search_corpus_matrix`. It opens the B1 corpus in Folder scope and runs `backup` and `needle` through {All, Names, Contents} × hidden {on, off} × everything {on, off}. For each of the 24 runs it asserts:
- the result path set equals the engine's hits for the same query (call `crow_search::search` directly in the test, minus the app's cap, which can't fill on the corpus);
- `footer_summary_text()` is the done form with the right count;
- the skipped count equals the engine's `summary.skipped.len()`.

This ties the app to the engine, so a filter in `search.rs` (like B1's temporary rule-only skip filter) can't hide results again unnoticed.

### 3 Fuzzer

**Change:** none to the generator, if B3's check holds: the keymap-driven generator already reaches Cmd-F, ⇧⌘A, ⇧⌘M and ⇧⌘H. Confirm in the progress file that `fuzz_smoke` produced at least one search step: grep a smoke run's step log for the search band's selectors or keys. If it never opens search, add `cmd-f` to whatever weighting the generator uses for "open a mode" keys, and explain why in the progress file.

**Run (named by this plan; do it once, at Finish, with `run_in_background`):** `fuzz_long` for 5 minutes. Record the seeds, the minutes and the result in the progress file. A failure: replay the seed with `CROW_FUZZ_SHRINK_SECS=0`, fix with a regression test, and do not re-run the long fuzz without asking.

### 4 Bench and docs

- Re-run B4's bench table on `main` and the branch. B5 changes no engine code, so the numbers should match B4's. Record them.
- `docs/testing.md`: the oracle, the matrix and the final search hand-check table (below). Remove any row that describes behavior B1–B4 changed and still says the old thing (search for "exclude.toml", "First 20000", "hidden files follow the setting").
- `README.md`: the status line names M9 next. Mention in the feature list that search finds hidden items and that Cmd-K finds files, in the README's existing style.
- `docs/plans/m8.5/README.md`: mark every batch merged, then move the folder to `docs/plans/done/m8.5/` **in this PR** (the batches are history after this).

## Hand checks (the milestone's)

Run each on the bundle built from this branch, in light and dark, at 640 px and wide:

| Check | Steps |
| --- | --- |
| The user's report | Cmd-F in `~`, Folder, All, hidden on: `.back` and `.backu` both show `~/.backup_excludes` within a moment. Then hidden off: the same. |
| Dot names in Cmd-K | Hidden off: Cmd-K `zshrc` finds it under "Files in …"; `~/ssh` finds `~/.ssh`. |
| Scopes | A miss in a subfolder offers Search Home; ⇧⌘H finds it. This Mac finds a name outside Home. |
| Match modes | All, Names, Contents on one word; a Names miss offers "Search names and contents". |
| Skips | `~` with hidden off: the skipped list groups Excluded, Hidden folders, Packages, Couldn't read; ⇧⌘A lifts the first three. |
| Cap | `target/fixture-search-200k`, `needle`, then an exact file name: top row. |
| VoiceOver | Hidden rows read "…, hidden"; the empty-state buttons read with their keys; the Folder segment reads its path; the footer reads once. |

## Questions to ASK

None expected. ASK if the oracle and the engine disagree on something the policy text doesn't decide.

## Finish

`cargo fmt --check` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo test --workspace` · `fuzz_long` 5 minutes (once, background) · bench recorded · the hand checks · commit per item · progress file current · stop at "Ready for critique" (do not push).
