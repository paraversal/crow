# M9 P2 — Read-only Git plugin and Projects progress

Branch: `codex/m9-p2-git-projects` · PR: none · Last updated: 2026-11-19 · State: Ready for critique
Base: `7b58509` (M9 P1B, merged as #66). Dependencies merged: P1B `7b58509`; P8 `d5cab1e`; P1A fix in `083d40c`.

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 Spike `gix` and prove the read-only boundary | done | gix 0.90.0 pinned; filter-free status proven on staged/conflict/ignored/filtered/linked-worktree/unborn/detached/nested repos; index unchanged; ahead/behind and line counts match the CLI. Provider `crow-plugin-git` has 8 CLI-oracle tests. |
| 2 Repository identity, snapshots and invalidation | done | `crow-plugin-git` models worktree/git/common identity, raw paths, head/upstream, merged staged+unstaged status, discovery and line counts. The app's `git` service shares one snapshot per worktree, merges watcher bursts, bounds folder/repo/line scans, drops stale generations and keeps only the active repository watched. |
| 3 Row decorations, columns, status, Info | done | Badge precedence conflict > deletion/rename > add > modify > untracked > ignored; ignored directories dim their children; `git.lines-changed` is computed on demand for visible changed rows and shows `—` when unknown; Status and Info carry branch, ahead/behind, changed count, index/worktree labels and last commit. |
| 4 `[projects]` config, discovery and sidebar section | done | `[projects] roots` parses/edits with comments; native `crow.projects` section, stable hex worktree IDs, bounded cancellable discovery, branch + deduplicated changed count, nested/linked repos, package/symlink pruning, refresh row on bounds, palette add/remove/refresh and palette Places rows. Hidden while the package is not Running; cleared on disable. |
| 5 Acceptance matrix, oracle tests and bench | done | 8 provider CLI-oracle tests, app service/bundled-package tests and workspace gates pass. Bench parity recorded. Hand checks cover list/Info/disable, 640 pt/light and columns; search/two-window/VoiceOver/palette checks remain listed below for the human. |

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

- [x] Bundle with a `plugins.toml` grant for the embedded package and `[projects] roots` on a three-repo fixture: List showed `a.txt M`, `new.txt ?`, dim `x.log`, Lines `2`/`—`, status bar `main · 2 changed`, and Projects rows for `alpha` (badge `2`, `main`), `beta` and `nested/inner`. Shot `target/shots/p2-hand.png`.
- [x] Info for `a.txt`: `Status modified`, `Worktree modified`, `Branch main`, `Last commit base`, `Changed paths 2`. Shot `target/shots/p2-hand-info3.png`.
- [x] Setting the grant `enabled = false` removed the badges, dimming, line values, status bar and Projects section without restarting. Shot `target/shots/p2-hand-disabled.png`. The configured `git.lines-changed` header remains as a blank configured column, as P1B keeps unknown/disabled IDs in `views.toml`.
- [x] 640 pt and Solarized Light: badges, ignored dimming, Projects rows, status and Info remain legible with no layout shift. Shot `target/shots/p2-hand-narrow-light.png`. The bottom-bar Git status yields at 640 pt to the path/count (P1B priority); Branch and changed count stay in Info.
- [x] Columns view: `M`/`?` badges and ignored dimming still show; the opt-in Lines column is a List column (P1B plugin columns are List-only). Shot `target/shots/p2-hand-columns.png`.
- [ ] Search results and two windows: badges/status in a search over the repo; switching tab while a line job is pending.
- [ ] VoiceOver: decorated rows, status, Info fields, Projects rows and collapse.
- [ ] Palette Add/Remove Current Folder as Project Root and Refresh Projects; roots persist after relaunch.
- [ ] Projects collapse/visibility edits in `sidebar.toml`; Refresh row when discovery is bounded.

## Found along the way

- APFS rejects invalid UTF-8 file names with `EILSEQ`, so a live non-UTF-8 path fixture cannot be created on the default volume. Raw bytes are still preserved end to end and pinned by `crow-plugin-git`'s `repository_relative_paths_keep_raw_bytes` test; a live fixture would need a differently formatted volume.

## Log

- 2026-11-18 Read the plan, M9 execution contract, P1A/P1A-fix/P1B progress and docs/plugins.md. Created the branch from `7b58509` and this progress file before code. Started three read-only scouts for the plugin runtime seam, app integration points and fs/test/bench seams.
- 2026-11-18 Environment: `git 2.56.1`, `rustc 1.100.0`, crates.io reachable. Latest `gix` is `0.90.0` (rust-version 1.88); its `status` feature pulls `gix-status`, `dirwalk`, `index`, `blob-diff`, and `blob-diff` → `attributes` → `command` (external drivers compiled in), so the spike must prove runtime options never invoke them.
- 2026-11-18 Spike fixture (throwaway, `/tmp/crow-p2-spike`, not in the repo): `clean`, `staged` (staged+unstaged same file, untracked, binary, staged rename, deleted), `conflict`, `ignored` (ignored file + dir + a tracked file matching `*.log`), `filtered` (`.gitattributes` `filter=sentinel` with a repo-local clean command writing a sentinel), `detached`, `unborn`, `noupstream`, local bare `clone` ahead 1 / behind 0, linked worktree `wt`, nested `outer/inner`. Commands: `git status --porcelain=v2`, `git diff --numstat`, `git diff --cached --numstat`.
- 2026-11-18 Spike observations. `gix::open` gives worktree/git-dir/common-dir; a linked worktree's git-dir is `<main>/.git/worktrees/wt` and common-dir is that path plus `/../..` (normalize before use). `head()` gives `is_detached`/`is_unborn`/`referent_name`/`id`; `branch_remote_tracking_ref_name` + `find_reference` + `merge_base` + two `rev_walk(...).with_hidden([base])` counts give local ahead/behind (clone: 1/0; no-upstream: none; detached: id only; unborn: branch only; unrelated histories: no merge base). Default `repo.status()` **ran the repo-local clean filter** (sentinel created) because status converts worktree bytes with `gix_filter`; ignoring that would violate the plan.
- 2026-11-18 Decisions after the spike: the provider uses `gix 0.90.0` with `default-features = false`; it does not need a backend change. Status composes gix’s index-worktree walk with a `gix_filter` pipeline whose external drivers are cleared, so `.gitattributes` clean/process filters cannot run; the sentinel fixture proves it. Diff and rename tracking use the same empty-driver cache. Cloud-backed worktrees (`Library/Mobile Documents`, `com~apple~CloudDocs`, `Library/CloudStorage`) report `status unavailable` without reading worktree content.
- 2026-11-18 Safe status path. Copied gix's `index_worktree_status` composition into `crow-plugin-git::status` but builds `gix_filter::pipeline::Options` and clears `drivers` (and passes no diff drivers), then calls `gix_status::index_as_worktree_with_renames` with a recording visitor. It keeps eol/ident/encoding conversion but cannot spawn a filter. Result on the same `filtered` repo: `Modified f.txt`, sentinel absent. The `ignored` repo emits `x.log`, `y.log`, `build/` as ignored and omits the clean tracked `keep.log`. Index size and mtime are unchanged on every run. Small-repo status took ~0.26–0.58 ms.
- 2026-11-18 Line counts. `staged_line_counts` (HEAD tree vs index) and `unstaged_line_counts` (index vs worktree) match `git diff --cached --numstat` / `git diff --numstat` on `both.txt` (1/1 and 2/1), binary `bin.dat` (`None`), deleted `gone.txt` (0/0 staged, 0/1 unstaged), and filtered `f.txt` (1/1, no sentinel). A direct HEAD lookup gives a rename destination a bogus addition count; the provider must take the rename source from the status snapshot (Git reports `0 0` for the rename). Untracked files must return unknown before diffing. Conflict rows must not count.
- 2026-11-18 `crow-fs` listing now keeps raw name bytes in `bulk::RawEntry` (the `Entry.name` display string stays lossy, `Entry.path` is exact). APFS rejects invalid UTF-8 names (`EILSEQ`), so the live test uses an emoji path; the raw-byte conversion has a direct unit test and the plan requirement is preserved in code for volumes that accept it.
- 2026-11-18 Integration. `RequestInput` now carries a bounded `HostData` (repository + per-entry Git facts), validated against `max_host_bytes`; the worker exposes `ctx.repository` and `entries[i].git` only for the embedded package. The app's `git` service runs folder/repository scans, line batches and Projects discovery in `background_spawn`, shares snapshots across windows, watches the active repository recursively plus its git/common dirs, merges bursts over 250 ms, and drops every result from an older generation. The bundled package `git` is embedded with `include_str!`, appears as pending approval, and is rejected as an installed bundle ID. List/column rows skip clean Git rows entirely, so a clean 100k worktree causes no per-row worker calls.
- 2026-11-18 Projects and UI. `[projects] roots` parses and edits with comments; the native `crow.projects` section publishes stable hex worktree rows with abbreviated path, branch and changed count, prunes packages/symlinks, finds nested/linked repos, hides when the package is not Running, and is cleared on disable. Palette gained Add/Remove Project Root and Refresh Projects, and a Projects group in folder search. Info shows Status/Worktree/Branch/Last commit/Changed paths; status shows branch, optional local ahead/behind and changed count; cloud-backed worktrees show status unavailable instead of reading content.
- 2026-11-19 Commits: provider core `020389c8`; integration `8af68e4b` + `3082bf5f` (the commit tool split the integration into provider/progress and app files). Final gates green; bundle built.
- 2026-11-18 Bench and hand checks. Alternating inactive/enabled runs on `target/fixture-100k` are at parity (median warm 4 ms, p99 8.52–8.53 ms); 100k clean status completes in 207–242 ms in the background. Bundle shots show badges, ignored dimming, Lines `2`/`—`, Projects rows, status `main · 2 changed`, Info details and full withdrawal on disable.
- 2026-11-18 Dependency surface: `gix = 0.90.0` with `default-features = false`, features `status`, `revision`, `attributes`, `excludes`, `index`, `sha1`. `attributes` compiles `gix-command`/`gix-protocol`/`gix-transport` (no HTTP, TLS, SSH or curl/reqwest transport feature); the safe pipeline passes zero drivers, so no compiled facility is invoked. No network client feature is enabled.

## Bench

`target/fixture-100k` (100,001 entries, now a clean Git worktree) at 1280×800, FireBox, 120 Hz, alternating runs. Warm first paint / scroll p99:

| Condition | Run 1 | Run 2 | Run 3 | Median |
| --- | --- | --- | --- | --- |
| Git inactive | 5 ms / 8.54 | 4 ms / 8.52 | 4 ms / 8.53 | 4 ms / 8.53 |
| Git enabled (Running, clean 100k worktree) | 3 ms / 8.51 | 4 ms / 8.53 | 4 ms / 8.52 | 4 ms / 8.52 |

Status completion on the clean 100k worktree: 207–242 ms in the background, `entries=0 changed=0`. The enabled runs stayed at 120 fps and cached zero clean-row values; after the clean-row filter only six to eight passive submissions (status startup) happened over the whole run. Cold first paint varied 83–121 ms in both conditions. The 100k fixture gained a `.git` directory in `target/` only; it is not tracked.

## Verified

- `cargo fmt --check` ✓
- `cargo clippy --workspace --all-targets -- -D warnings` ✓
- `cargo test --workspace` ✓ (657 app + 8 provider + 80 plugin + 107 core + 71 fs + 40 search + 13 theme + 6 macos, 1 ignored)
- `./scripts/bundle.sh` ✓
- Bench inactive/enabled parity on the 100k worktree; 100k clean status 207–242 ms in the background.
- Bundle hand checks: list badges/dimming/Lines, status bar, Projects rows, Info fields, disable withdrawal, columns, 640 pt/Solarized Light.

## Not verified

- VoiceOver, search-result badges in the live bundle, two windows and a tab switch with a held line job, palette add/remove-root execution, and Projects collapse/visibility edits (all listed as hand checks).
- The invalid-UTF-8 live path fixture cannot exist on APFS; raw-byte conversion is unit-tested instead.

## Next

Hand-test the bundle with the pending checks above, then the critique agent opens the PR. The next implementation piece in the M9 order is [P3A](p3a-icon-packs.md).
