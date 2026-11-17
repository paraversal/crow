# B1 — Streaming fix, corpus and trace progress

Branch: `m8.5-b1-streaming` · PR: none · Last updated: 2026-11-18 · State: in progress

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 Batcher | in progress | |
| 2 Name before contents, regular files only | todo | |
| 3 Typed skips, unreadable folders | todo | |
| 4 Corpus | todo | |
| 5 Named cases | todo | |
| 6 Trace and log lines | todo | |
| 7 Docs | todo | |

## Questions

None; the plan says there are none and the index's decisions are binding.

## Decisions

- The uncommitted `docs/plans/m8.5/README.md` re-cut and the new batch plans are the user's drafts; the progress file is committed as the plan requires, the plan documents are left as the user staged them.

## Hand checks

- [ ] The user's repro, before and after: on `main` and on the branch, Cmd-F in `~`, Folder, All, Show Hidden Files on; `.back`, then `.backu` → `.backup_excludes` appears within a moment on the branch while the footer still says "Searching…". Record the time.
- [ ] `crow --search-trace ~ .backu --hidden --target ~/.backup_excludes` → a name hit in the first batch. Record the line and the total time.
- [ ] `--make-search-corpus target/fixture-search-corpus`, open it, Cmd-F `backup` with hidden on and off: the same results as on `main`; the search finishes despite `pipe`.
- [ ] Bench: `--bench-search target/fixture-search-200k` for `needle`, `back`, `backup` on `main` and the branch. First hit same or better; total within noise. Record both.

## Found along the way

## Log

- 2026-11-18 Plan read, branch created, progress file started.

## Next

Item 1: `Batcher` in `crates/crow-search/src/engine.rs` with its four unit tests.
