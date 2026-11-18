# M9 P2 — Read-only Git plugin and Projects progress

Branch: `codex/m9-p2-git-projects` · PR: none · Last updated: 2026-11-18 · State: in progress
Base: `7b58509` (M9 P1B, merged as #66). Dependencies merged: P1B `7b58509`; P8 `d5cab1e`; P1A fix in `083d40c`.

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 Spike `gix` and prove the read-only boundary | in progress | Exact feature set, status/index/HEAD/upstream/worktrees/ignored/line counts, CLI oracle, no writes/filters/network. |
| 2 Repository identity, snapshots and invalidation | todo | Worktree identity + separate git/common dirs, raw paths, shared per-generation cache, bounded scans, watcher/focus refresh. |
| 3 Row decorations, columns, status, Info | todo | Conflict > deleted/renamed > added > modified > untracked > ignored; `git.lines-changed` on demand; unknown not zero. |
| 4 `[projects]` config, discovery and sidebar section | todo | User-chosen roots, none by default; bounded cancellable discovery; stable rows; refresh on bounds. |
| 5 Acceptance matrix, oracle tests and bench | todo | Synthetic repo matrix, disable/stale checks, inactive vs enabled bench. |

## Questions

- Q1 (open, asked 2026-11-18): plugin identity and native binding. The plan names the bundled package ID `git` and column `git.lines-changed` (also the example in `crow-core`'s `ExtensionId` test), but P1A reserves only `crow`/`crow.*` and has no implemented bundled-package path or native-provider binding yet (`crates/crow-plugin/src/manifest.rs:122`, `crates/crow-core/src/extensions.rs:39`). Need the user's choice of ID and how the provider binding is keyed to the bundled bytes.
- Q2 (open, asked 2026-11-18): Projects ownership. P1B's Sidebar descriptor only receives the current folder's first 128 folders, so it cannot scan configured roots. Need a decision between a native `crow.projects` section fed by the Git provider and extending the plugin sidebar input.
- Q3 (open, asked 2026-11-18): how Rhai receives provider data. The plan says the Rust provider runs Git and "Rhai maps those snapshots to P1B descriptors", but API 1 has no script-facing service and `RequestInput` carries only bounded display entries plus one excerpt. Need the bridge shape.

## Decisions

- Base `7b58509`; branch `codex/m9-p2-git-projects`. The user's uncommitted `docs/plans/m8` → `done/m8` / M9 index edits stay untouched and uncommitted.
- Spike first (plan item 1), before choosing or changing the Git backend. The spike is throwaway except for recorded observations; backend changes need the user's answer.
- No new GPUI patch unless a spike proves one is required; that needs the user's check-in first.

## Hand checks

- [ ] TBD after implementation.

## Found along the way

- (none yet)

## Log

- 2026-11-18 Read the plan, M9 execution contract, P1A/P1A-fix/P1B progress and docs/plugins.md. Created the branch from `7b58509` and this progress file before code. Started three read-only scouts for the plugin runtime seam, app integration points and fs/test/bench seams.
- 2026-11-18 Environment: `git 2.56.1`, `rustc 1.100.0`, crates.io reachable. Latest `gix` is `0.90.0` (rust-version 1.88); its `status` feature pulls `gix-status`, `dirwalk`, `index`, `blob-diff`, and `blob-diff` → `attributes` → `command` (external drivers compiled in), so the spike must prove runtime options never invoke them.

## Next

Run the `gix` spike on synthetic repositories and record observations; ask Q1-Q3 before building the dependent integration.
