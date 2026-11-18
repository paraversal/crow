# P1A — Public Rhai package, permissions and worker progress

Branch: `codex/m9-p1a-plugin-runtime` · Base: `cb21dcb` (M8.5 B5, merged) · Last updated: 2026-11-18 · State: in progress

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 package identity, discovery, snapshot, digest | done | `crow-plugin` manifest/package modules; deny-unknown TOML, ID/version/API/capability checks, no-follow `openat`, bounded reads, SHA-256 framing, duplicate IDs publish nothing |
| 2 grants store and lifecycle | in progress | Core `plugins.toml` approve/disable/enable, exact digest+capability match, malformed file withdraws all, generation invalidation; app grant path left |
| 3 capability matrix and host boundary | in progress | API-1 `fs = none/read` enforced; spawn/net rejected before a worker; handle-scoped `read_excerpt`, `asset`, no raw paths/shell/net; matrix doc left |
| 4 worker, engine, budgets, mailbox | done | One thread+engine per plugin; 100k ops/50 ms; bounded command lane (busy) + coalesced refresh; pause/retry; cooperative stop; A/B test passes |
| 5 registration, namespace, DTOs | done | `plugin_id.local_id`, `provides` check, duplicate rejection, closed reply shapes, request-local handles, typed command allowlist (`copy-text`) |
| 6 app inventory and grant path | todo | Minimal inspectable UI, status-line errors, background discovery/config reload |
| 7 step checks and acceptance fixtures | in progress | Package/loader/grants/worker/host fixtures pass; app inventory/grant test pending |
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
- 2026-11-18 Built `crates/crow-plugin`: ids/DTOs in `crow-core::extensions`, manifest, package snapshot (no-follow, bounded, SHA-256), grants, Rhai worker, registry validation, host lifecycle + service wrapper. 38 unit tests pass (loader swap, duplicate IDs, symlink entry/asset, grant invalidation, malformed grants, denied read/native, oversized input/reply, budget, queue saturation, A/B, stale reply). Clippy clean for the crate.
- 2026-11-18 Next: wire the app inventory/grant path and status line, then capability matrix doc and full checks.

## Next

Wire `crow-plugin` into the app: background service global, settings inventory with approve/disable/reload/retry, status-line errors, config/plugin watcher reload.
