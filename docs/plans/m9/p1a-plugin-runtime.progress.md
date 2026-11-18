# P1A — Public Rhai package, permissions and worker progress

Branch: `codex/m9-p1a-plugin-runtime` · Base: `cb21dcb` (M8.5 B5, merged) · Last updated: 2026-11-18 · State: in progress

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 package identity, discovery, snapshot, digest | todo | New GPUI-free `crow-plugin` crate; manifest validation, symlink-safe bounded reads, SHA-256 snapshot |
| 2 grants store and lifecycle | todo | `plugins.toml`, approve/disable/reload, generation invalidation, status with plugin name |
| 3 capability matrix and host boundary | todo | Document API-1 (`fs = none/read`), reject `spawn`/`net`, handle-scoped reads, no raw paths |
| 4 worker, engine, budgets, mailbox | todo | One thread + engine per plugin, 100k ops/50 ms, bounded lanes, coalesced refresh, pause/retry |
| 5 registration, namespace, DTOs | todo | `plugin_id.local_id`, validation against `provides`, atomic publish, closed contribution values |
| 6 app inventory and grant path | todo | Minimal inspectable UI, status-line errors, background discovery/config reload |
| 7 step checks and acceptance fixtures | todo | Loader swap, A/B workers, grant reload, unsupported capabilities, bounds, stale replies |
| 8 checks, docs and handoff | todo | fmt/clippy/test, capability matrix, Ready for critique |

## Questions

- None yet.

## Decisions

- Branch starts from merged `cb21dcb`; the user's uncommitted M8→done / M9 docs reorganization is left untouched and uncommitted (not mine).
- `rhai 1.27.0` is already in the local cargo cache; the plan names Rhai, so the dependency is expected. `sha2` (digest) and `libc` (directory-relative `openat`/`O_NOFOLLOW`) are required implementation dependencies for the plan's SHA-256 framing and no-follow read wording; both already exist in `Cargo.lock` through other crates. Recorded here rather than asking because the plan mandates the behavior and no existing workspace helper provides it.
- Wire boundary is owned by `crow-plugin`; shared extension identity/descriptor DTOs go in `crow-core::extensions` because P1B's consumer registry and the app need them without depending on Rhai.
- Worker notifications use `futures::channel::mpsc::UnboundedSender<HostEvent>` (the pattern `folder_sizes`/`operations` already use) so the crate stays GPUI-free and the UI never waits.

## Hand checks

- [ ] Open the plugin inventory with a valid bundle, an invalid bundle and no plugins: each state is legible, approving runs the fixture, disabling withdraws it, reload of changed bytes returns it to pending approval.
- [ ] Plugin status/error text names the plugin and appears in the status line; denied capability and budget pause are distinguishable.
- [ ] Theme light/dark and VoiceOver on the inventory rows/actions (after the UI exists).

## Found along the way

- The worktree has the user's uncommitted docs reorganization (M8 moved to `docs/plans/done/m8/`, new untracked M9 plans). Left alone.

## Log

- 2026-11-18 Started. Read the plan, M9 index, finish contract, repo rules, `crow-core::extensions`, config/status conventions. Base `cb21dcb`; branch created. Scout mapping the settings/status/background integration points while the crate design is written.
- 2026-11-18 Progress file created before code.

## Next

Finish the integration map, then create `crates/crow-plugin` with identity/manifest/snapshot (item 1).
