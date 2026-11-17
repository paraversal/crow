# M8.5 — Search reliability (index)

Between M8 (Finder parity) and M9 (Phase 2). This is a roadmap, not a set of plans: each piece gets its own plan in this folder when it starts, with its own "Questions to ASK", branch, progress file and critique. Do not start a piece until the pieces it needs are merged to `main` (ASK before stacking on an unmerged branch).

Drafted 2026-11-17 from the user's second triage list: Cmd-K cannot find files/folders whose names start with a dot; Cmd-F finds "back" quickly but not "backup"; search is "kind of really unreliable" in general. The user made it its own milestone because the problem spans the engine, the palette, the search panel and the band, and because a repro corpus is needed before the causes can be split.

## What we know

- **The fuzzy matcher is not the cause of the "backup" miss.** `crow_search::FuzzyMatcher` scores `backup`, `.backup`, `Backups`, `backup-excludes`, `savegameBackup`, `do-not-backup` and `tm-backup` for both `back` and `backup`; all match, with contiguous ranges. A scratch test confirmed this on 2026-11-17.
- **Hidden files are on in the user's config** (`show_hidden_files = true`), and the `[hidden]` exclusion set is off (`~/.config/crow/exclude.toml`). So a plain dotfile miss is not explained by the setting alone.
- **Cmd-K is the palette, not file search.** Its empty query lists Recent, Favorites and the current folder's subfolders (`palette::folder_candidates`). A typed path lists folders by fuzzy match but files by **prefix only** (`palette::path_listing`, `crates/crow-app/src/palette.rs:972`), so `.backup.txt` is not found by typing `backup`. Path mode hides dot folders until the partial starts with `.` when hidden files are off; a test enshrines that (`palette_path_mode_hides_dot_folders_until_typed`, `crates/crow-app/src/window/tests.rs:4067`).
- **Cmd-F searches the current folder by default**, not Home or This Mac. `SearchView` starts in `Scope::Folder`; the scope bar switches to Home or This Mac. The band shows the scope, but a user who expects a global search sees a miss.
- **Cmd-F searches names and contents by default** (`Match::All`). Name hits stream first; content hits read every file. The result cap is 20,000 (`MAX_RESULTS`, `crates/crow-app/src/search.rs:42`); hitting it cancels the walk. A strong name hit that arrives after the cap is dropped.
- **Exclusions are on by default** (`exclude.toml` dev set: `node_modules`, `.git`, `target`, `build`, `dist`, …). A target inside an excluded folder is skipped and only counted in the skipped list.
- **The walker honors the hidden setting** (`build_walker`, `crates/crow-search/src/engine.rs:291`): `hidden(!show_hidden)`. With hidden off, dotfiles and dot folders are not visited at all.

The reported "backup" miss is therefore one of: scope (the item is outside the current folder), hidden policy (the target is dot-named and hidden is off in the running app), exclusion (the target is inside a skipped folder), the 20k cap, or the match mode (Contents only). The first piece must reproduce it and name the cause before any behavior changes.

## The pieces

| Piece | What | Needs | Branch |
| --- | --- | --- | --- |
| S1 Repro corpus and diagnostics | Repro corpus and diagnostics: a deterministic tree with dot names, `backup` names, excluded folders, packages, symlinks and a 100k subtree; a query log that records scope, match mode, hidden, exclusions and cap, and explains why each candidate was included or skipped. The user's failing folder/name becomes a named case. No behavior change. | — | `m8.5-s1-corpus` |
| S2 Hidden and dot names | Hidden and dot-name policy for Cmd-F and Cmd-K: decide whether search finds dot-named items when View > Show Hidden Files is off; make Cmd-K path mode match files by name (fuzzy or prefix, per the user), not only by prefix; remove the "type the dot first" trap if the user wants. | S1 | `m8.5-s2-hidden` |
| S3 Scope and exclusions | Scope and recall: make the default scope and the scope bar unambiguous; decide whether Cmd-F should start in the current folder or Home; show skipped folders and "Search everything" at the point of a miss; cover packages, symlinks and unreadable folders. | S1 | `m8.5-s3-scope` |
| S4 Ranking and cap | Ranking, cap and streaming: keep exact/contiguous name hits ahead of content and weak fuzzy hits even when the cap is reached; make the cap per file or reserve room for names; keep selection stable while batches arrive; make the footer's "First 20000 files" state actionable. | S1 | `m8.5-s4-ranking` |
| S5 Performance | Performance and cancellation: profile `back` vs `backup` on the corpus and the 200k search fixture; keep name hits streaming while contents are read; ensure cancellation and batching never block the main thread; record the budget. | S1, S4 | `m8.5-s5-performance` |
| S6 Feedback | Feedback and recovery: say why a search is empty (hidden off, excluded folder, contents still running, cap reached), keep the skipped list reachable, and make the scope/match/hidden state visible without a caption. | S2, S3, S4 | `m8.5-s6-feedback` |
| S7 Regression | Regression and fuzz: corpus assertions for every reported case, an oracle that compares the engine to a brute-force walk on small trees, and a search fuzz pass; bench on `target/fixture-search-200k`. | S2–S5 | `m8.5-s7-regression` |

**Suggested order:** S1 → S2 → S3 → S4 → S5 → S6 → S7. S1 is first because every later piece needs a failing case. S2/S3/S4 all touch `search.rs`, `palette.rs` and the band; never two writers at once. S5 can run after S4; S6 after the behavior settles; S7 last.

**M8.5 is done when:** the user's dot-name and "backup" cases are in the corpus and pass; Cmd-K and Cmd-F agree on hidden/dot policy; the scope and cap are visible and explained at a miss; the search bench on `target/fixture-search-200k` is no worse than `main` (first hit and total); `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass; the hand checks cover light/dark, 640 px and wide, hidden on/off, all three scopes, all three match modes, and VoiceOver for new roles; README's status line names M9 next.

## Questions to ASK

Index-level questions for the user, asked by whoever starts S1:

1. **Exact repro for "backup".** Which folder, which item, which scope (Folder/Home/This Mac), which match mode (All/Names/Contents), and was View > Show Hidden Files on? Recommended: add it to S1's corpus as the named case; if it cannot be reproduced, S1's diagnostics should still explain the miss.
2. **Hidden and dot names.** Should Cmd-F and Cmd-K find dot-named items when Show Hidden Files is off? Finder's search can surface hidden items even when the list hides them. Recommended: search includes them, marks them as hidden, and Cmd-K path mode matches them by name without requiring a leading dot. This is a product decision, not a code default.
3. **Cmd-K files vs folders.** Today Cmd-K is "go to a folder": the empty query lists folders, and files appear only under a typed path. Should a typed query also fuzzy-match files in the current folder? Recommended: yes for a typed query, with folders first; the empty query stays places-only. This is the closest fix to "Cmd-K can't find files".
4. **Cmd-F default scope.** Current folder (today), Home, or This Mac? Recommended: keep the current folder as the default and make the scope bar and the empty state say so; add a one-key "Search Home" path from a miss. Changing the default is a bigger product change and should be explicit.
5. **Cap policy.** When 20,000 results are reached, should exact/contiguous name hits always be kept even if they arrive late? Recommended: reserve the cap for content and weak fuzzy hits, and always keep a strong name hit; the footer should say what was dropped.

Each piece ends with its own "Questions to ASK"; ask those in one batch when that piece starts. Don't answer them all up front.

## Not in M8.5

- Saved queries, Tags, Spotlight indexing and a search-history UI: M9 P4 and later.
- Content indexing or a database: Crow stays a live walker; no index to keep fresh.
- Regex syntax changes: `/…/` already works; S4/S5 may tune its cost, not its language.
- Remote file systems and iCloud placeholder downloads: out of scope, as in M8/M9.
