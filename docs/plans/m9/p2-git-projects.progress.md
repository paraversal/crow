# M9 P2 — Read-only Git plugin and Projects progress

Branch: `codex/m9-p2-git-projects` · PR: none · Last updated: 2026-11-18 · State: in progress
Base: `7b58509` (M9 P1B, merged as #66). Dependencies merged: P1B `7b58509`; P8 `d5cab1e`; P1A fix in `083d40c`.

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 Spike `gix` and prove the read-only boundary | done | gix 0.90.0 pinned; filter-free status proven on staged/conflict/ignored/filtered/linked-worktree/unborn/detached/nested repos; index unchanged; ahead/behind and line counts match the CLI. Provider `crow-plugin-git` has 7 CLI-oracle tests. |
| 2 Repository identity, snapshots and invalidation | in progress | `RepositoryIdentity`, head/upstream, merged staged+unstaged status, raw paths, discovery and line counts are in `crow-plugin-git`; the shared cache/watcher/service is next. |
| 3 Row decorations, columns, status, Info | todo | Conflict > deleted/renamed > added > modified > untracked > ignored; `git.lines-changed` on demand; unknown not zero. |
| 4 `[projects]` config, discovery and sidebar section | todo | User-chosen roots, none by default; bounded cancellable discovery; stable rows; refresh on bounds. |
| 5 Acceptance matrix, oracle tests and bench | todo | Synthetic repo matrix, disable/stale checks, inactive vs enabled bench. |

## Questions

- Q1 (asked 2026-11-18, answered **A**): bundled package ID `git`, reserved for the embedded digest-bound package; external installs of `git` rejected. Descriptors stay `git.*`.
- Q2 (asked 2026-11-18, answered **A**): Projects is a native `crow.projects` sidebar section fed by the provider; `[projects] roots` and `[section."crow.projects"]` in `sidebar.toml`; the `git` package owns rows/columns/status/Info.
- Q3 (asked 2026-11-18, answered **A**): add a bounded, closed host-data carrier to `RequestInput` (repository summary + per-entry status + optional line counts), populated only for the digest-bound embedded `git` package; Rhai maps it to descriptors.

## Decisions

- Q4 answered **A** (user): the embedded package appears in Settings as Pending approval; no grant is written until the user approves. Projects roots stay empty by default and no decorations appear until approval.
- Q1-Q3 answered A (user): ID `git` reserved for the embedded package and bound by digest; Projects is a native `crow.projects` section; provider facts cross to Rhai through a bounded host-data field on `RequestInput`.
- Base `7b58509`; branch `codex/m9-p2-git-projects`. The user's uncommitted `docs/plans/m8` → `done/m8` / M9 index edits stay untouched and uncommitted.
- Spike first (plan item 1), before choosing or changing the Git backend. The spike is throwaway except for recorded observations; backend changes need the user's answer.
- No new GPUI patch unless a spike proves one is required; that needs the user's check-in first.

## Hand checks

- [ ] TBD after implementation.

## Found along the way

- APFS rejects invalid UTF-8 file names with `EILSEQ`, so a live non-UTF-8 path fixture cannot be created on the default volume. Raw bytes are still preserved end to end and pinned by `crow-plugin-git`'s `repository_relative_paths_keep_raw_bytes` test; a live fixture would need a differently formatted volume.

## Log

- 2026-11-18 Read the plan, M9 execution contract, P1A/P1A-fix/P1B progress and docs/plugins.md. Created the branch from `7b58509` and this progress file before code. Started three read-only scouts for the plugin runtime seam, app integration points and fs/test/bench seams.
- 2026-11-18 Environment: `git 2.56.1`, `rustc 1.100.0`, crates.io reachable. Latest `gix` is `0.90.0` (rust-version 1.88); its `status` feature pulls `gix-status`, `dirwalk`, `index`, `blob-diff`, and `blob-diff` → `attributes` → `command` (external drivers compiled in), so the spike must prove runtime options never invoke them.
- 2026-11-18 Spike fixture (throwaway, `/tmp/crow-p2-spike`, not in the repo): `clean`, `staged` (staged+unstaged same file, untracked, binary, staged rename, deleted), `conflict`, `ignored` (ignored file + dir + a tracked file matching `*.log`), `filtered` (`.gitattributes` `filter=sentinel` with a repo-local clean command writing a sentinel), `detached`, `unborn`, `noupstream`, local bare `clone` ahead 1 / behind 0, linked worktree `wt`, nested `outer/inner`. Commands: `git status --porcelain=v2`, `git diff --numstat`, `git diff --cached --numstat`.
- 2026-11-18 Spike observations. `gix::open` gives worktree/git-dir/common-dir; a linked worktree's git-dir is `<main>/.git/worktrees/wt` and common-dir is that path plus `/../..` (normalize before use). `head()` gives `is_detached`/`is_unborn`/`referent_name`/`id`; `branch_remote_tracking_ref_name` + `find_reference` + `merge_base` + two `rev_walk(...).with_hidden([base])` counts give local ahead/behind (clone: 1/0; no-upstream: none; detached: id only; unborn: branch only; unrelated histories: no merge base). Default `repo.status()` **ran the repo-local clean filter** (sentinel created) because status converts worktree bytes with `gix_filter`; ignoring that would violate the plan.
- 2026-11-18 Safe status path. Copied gix's `index_worktree_status` composition into `crow-plugin-git::status` but builds `gix_filter::pipeline::Options` and clears `drivers` (and passes no diff drivers), then calls `gix_status::index_as_worktree_with_renames` with a recording visitor. It keeps eol/ident/encoding conversion but cannot spawn a filter. Result on the same `filtered` repo: `Modified f.txt`, sentinel absent. The `ignored` repo emits `x.log`, `y.log`, `build/` as ignored and omits the clean tracked `keep.log`. Index size and mtime are unchanged on every run. Small-repo status took ~0.26–0.58 ms.
- 2026-11-18 Line counts. `staged_line_counts` (HEAD tree vs index) and `unstaged_line_counts` (index vs worktree) match `git diff --cached --numstat` / `git diff --numstat` on `both.txt` (1/1 and 2/1), binary `bin.dat` (`None`), deleted `gone.txt` (0/0 staged, 0/1 unstaged), and filtered `f.txt` (1/1, no sentinel). A direct HEAD lookup gives a rename destination a bogus addition count; the provider must take the rename source from the status snapshot (Git reports `0 0` for the rename). Untracked files must return unknown before diffing. Conflict rows must not count.
- 2026-11-18 `crow-fs` listing now keeps raw name bytes in `bulk::RawEntry` (the `Entry.name` display string stays lossy, `Entry.path` is exact). APFS rejects invalid UTF-8 names (`EILSEQ`), so the live test uses an emoji path; the raw-byte conversion has a direct unit test and the plan requirement is preserved in code for volumes that accept it.
- 2026-11-18 Dependency surface: `gix = 0.90.0` with `default-features = false`, features `status`, `revision`, `attributes`, `excludes`, `index`, `sha1`. `attributes` compiles `gix-command`/`gix-protocol`/`gix-transport` (no HTTP, TLS, SSH or curl/reqwest transport feature); the safe pipeline passes zero drivers, so no compiled facility is invoked. No network client feature is enabled.

## Next

Run the `gix` spike on synthetic repositories and record observations; ask Q1-Q3 before building the dependent integration.
