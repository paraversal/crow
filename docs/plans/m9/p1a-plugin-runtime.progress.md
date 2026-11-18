# P1A — Public Rhai package, permissions and worker progress

Branch: `codex/m9-p1a-plugin-runtime` · Base: `cb21dcb` (M8.5 B5, merged) · Last updated: 2026-11-18 · State: Ready for critique

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 package identity, discovery, snapshot, digest | done | `crow-plugin` manifest/package modules; deny-unknown TOML, ID/version/API/capability checks, no-follow `openat`, bounded reads, SHA-256 framing, duplicate IDs publish nothing |
| 2 grants store and lifecycle | done | `plugins.toml` approve/disable/enable, exact digest+capability match, malformed file withdraws all, generation invalidation, app path writes and reloads it |
| 3 capability matrix and host boundary | done | `docs/plugins.md`; API-1 `fs = none/read` enforced; spawn/net rejected before a worker; handle-scoped `read_excerpt` (64 KiB, no-follow), `asset`, no raw paths/shell/net |
| 4 worker, engine, budgets, mailbox | done | One thread+engine per plugin; 100k ops/50 ms; per-call cancel token + deadline; bounded command lane (busy) + coalesced refresh; pause/retry; cooperative stop; A/B test passes |
| 5 registration, namespace, DTOs | done | `plugin_id.local_id`, `provides` check, duplicate rejection, closed reply shapes, request-local handles, typed command allowlist (`copy-text`) |
| 6 app inventory and grant path | done | Settings ▸ Plugins list with approve/enable/disable/reload/retry, keys A/E/D/R/T/L/G/O, status-line errors, 100 ms event drain, config reload hook, app tests |
| 7 step checks and acceptance fixtures | done | 42 plugin tests plus 2 GPUI app tests: loader swaps, duplicate IDs, grant reload, capabilities, bounds, budget, A/B, queue saturation, stale replies, cancel, status line |
| 8 checks, docs and handoff | done | `cargo fmt --check`, workspace clippy, `cargo test --workspace` green; bundle built; hand checks recorded; Ready for critique |

## Questions

- None yet.

## Decisions

- Branch starts from merged `cb21dcb`; the user's uncommitted M8→done / M9 docs reorganization is left untouched and uncommitted (not mine).
- `rhai 1.27.0` is already in the local cargo cache; the plan names Rhai, so the dependency is expected. `sha2` (digest) and `libc` (directory-relative `openat`/`O_NOFOLLOW`) are required implementation dependencies for the plan's SHA-256 framing and no-follow read wording; both already exist in `Cargo.lock` through other crates. Recorded here rather than asking because the plan mandates the behavior and no existing workspace helper provides it.
- Wire boundary is owned by `crow-plugin`; shared extension identity/descriptor DTOs go in `crow-core::extensions` because P1B's consumer registry and the app need them without depending on Rhai.
- Worker notifications use a `std::sync::mpsc` channel drained by the existing 100 ms `ui::poll`, not a background-thread GPUI wake. GPUI's test scheduler rejects cross-thread executor wakes, and the app already coalesces config changes this way; the UI still never waits on a plugin.
- The P1A inventory lives in Settings as one focusable list (`ControlId::Plugins`) with per-row action buttons and list-local keys. Dynamic per-button Tab stops would fight the fixed `ControlId` traversal; the list itself is the keyboard route and every action has a key plus an aria key shortcut.
- `plugins.toml` is written with Crow's comment-preserving `edit_toml`; the first service refresh writes the documented default. `Reload All` covers packages that cannot be addressed by id (unreadable manifest/bundle).

## Hand checks

- [x] Bundle, temp config, valid + `api = 2` bundles: Settings ▸ Plugins showed Pending approval and Incompatible API rows with permissions and the API error; Approve by mouse changed Copy Relative Path to Running, "1 extension", Disable/Reload, and wrote the `plugins.toml` record with `enabled = true` and its digest. Shots: `target/shots/p1a-plugins-running-firebox.png`, `p1a-plugins-light.png`.
- [x] Status line: the main window showed `Broken Example: plugin API 9 is not supported; this build runs api = 1` (`target/shots/p1a-main-status-firebox.png`).
- [x] Light theme (Solarized Light) and the fixed 560 pt Settings width: names, state, permissions, wrapped error, compact action buttons and footer buttons stay legible with no clipping (`target/shots/p1a-plugins-list-light.png`). Fixed during the check: action labels were shortened to Approve/Disable/Reload/Retry (full plugin name stays in the aria label), the extension count grammar, and a visible focused-row tint.
- [ ] Disable/Enable/Reload keys in the bundle: the GPUI test simulates Enter then D and asserts Running→Disabled plus the grant line; manual key injection into the plugin list after a mouse approval was not reliable, so the critic/user should confirm the key path by hand.
- [ ] VoiceOver on the list and rows: the GPUI a11y invariant passes with real rows (roles, labels, positions, aria key shortcuts); spoken output needs a human check.

## Found along the way

- The worktree has the user's uncommitted docs reorganization (M8 moved to `docs/plans/done/m8/`, new untracked M9 plans). Left alone.

## Log

- 2026-11-18 Started. Read the plan, M9 index, finish contract, repo rules, `crow-core::extensions`, config/status conventions. Base `cb21dcb`; branch created. Scout mapping the settings/status/background integration points while the crate design is written.
- 2026-11-18 Progress file created before code.
- 2026-11-18 Built `crates/crow-plugin`: ids/DTOs in `crow-core::extensions`, manifest, package snapshot (no-follow, bounded, SHA-256), grants, Rhai worker, registry validation, host lifecycle + service wrapper. 38 unit tests pass (loader swap, duplicate IDs, symlink entry/asset, grant invalidation, malformed grants, denied read/native, oversized input/reply, budget, queue saturation, A/B, stale reply). Clippy clean for the crate.
- 2026-11-18 Wired the app: `plugins` service global, 100 ms event drain, config-reload hook, Settings ▸ Plugins list with approve/enable/disable/reload/retry (keys A/E/D/R/T, L reload all, G grants, O folder), status-line errors, `docs/plugins.md` capability matrix, README crate/config rows. Added GPUI tests for approve→disable and invalid-package status. 42 crow-plugin tests + GPUI settings suite pass; clippy clean.
- 2026-11-18 Final visual check with `target/bundle/Crow.app` and a temp config: pending/running/error states in FireBox and Solarized Light; approved by mouse, grant file checked. Shortened button labels after the first shot clipped at 560 pt; added focus-row tint and fixed "1 extension". Bundle built.
- 2026-11-18 Final gates: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` green; `./scripts/bundle.sh` built and signed. Hand-check shots: pending/running/error in FireBox and Solarized Light. Two human checks remain (list key Disable/Enable/Reload; VoiceOver speech).
- 2026-11-18 State set to Ready for critique. Branch commits: core runtime, then app inventory/docs.

## Next

Critique: review the branch with the `critique` skill, fix clear issues, and open the PR. The plan document and M9 docs reorganization are still the user's untracked changes and were not committed.
