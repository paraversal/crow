# P1A — Public Rhai package, permissions and worker progress

Branch: `codex/m9-p1a-plugin-runtime` · Base: `cb21dcb` (M8.5 B5, merged) · Last updated: 2026-11-18 · State: In critique

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 package identity, discovery, snapshot, digest | done | `crow-plugin` manifest/package modules; deny-unknown TOML, ID/version/API/capability checks, no-follow `openat`, bounded reads, SHA-256 framing, duplicate IDs publish nothing |
| 2 grants store and lifecycle | done | `plugins.toml` approve/disable/enable, exact digest+capability match, malformed file withdraws all, generation invalidation, app path writes and reloads it |
| 3 capability matrix and host boundary | done | `docs/plugins.md`; API-1 `fs = none/read` enforced; spawn/net rejected before a worker; host-read 64 KiB excerpt passed as request input, `asset`, no raw paths/shell/net |
| 4 worker, engine, budgets, mailbox | done | One thread+engine per plugin; 100k ops/50 ms; per-call cancel token + deadline; bounded command lane (busy) + coalesced refresh; pause/retry; cooperative stop; A/B test passes |
| 5 registration, namespace, DTOs | done | `plugin_id.local_id`, `provides` check, duplicate rejection, closed reply shapes, request-local handles, typed command allowlist (`copy_text`) |
| 6 app inventory and grant path | done | Settings ▸ Plugins list with approve/enable/disable/reload/retry, keys A/E/D/R/T/L/G/O, status-line errors, 100 ms event drain, config reload hook, app tests |
| 7 step checks and acceptance fixtures | done | 49 plugin tests plus 3 GPUI app tests: loader swaps, duplicate IDs, grant reload, capabilities, bounds, budget, A/B, queue saturation, stale replies, cancel, status line, long-name layout, read-grant gate |
| 8 checks, docs and handoff | done | `cargo fmt --check`, workspace clippy, `cargo test --workspace` green; bundle built; hand checks recorded; Ready for critique |

## Questions

- Q1 (critique): read capability shape. **Answered by the user:** prefetch excerpts in the Rust host before enqueueing; pass the bounded excerpt as request input; remove `read_excerpt(handle)` from Rhai. P1B checks P8 content eligibility before reading live files.
- Q2 (critique): command intent shape. **Answered by the user:** align P1A with P1B; `#{ kind: "copy_text", text: relative }` is the sole API-1 clipboard intent shape, with decoder, docs and tests updated.

## Decisions

- Branch starts from merged `cb21dcb`; the user's uncommitted M8→done / M9 docs reorganization is left untouched and uncommitted (not mine).
- `rhai 1.27.0` is already in the local cargo cache; the plan names Rhai, so the dependency is expected. `sha2` (digest) and `libc` (directory-relative `openat`/`O_NOFOLLOW`) are required implementation dependencies for the plan's SHA-256 framing and no-follow read wording; both already exist in `Cargo.lock` through other crates. Recorded here rather than asking because the plan mandates the behavior and no existing workspace helper provides it.
- Wire boundary is owned by `crow-plugin`; shared extension identity/descriptor DTOs go in `crow-core::extensions` because P1B's consumer registry and the app need them without depending on Rhai.
- Worker notifications use a `std::sync::mpsc` channel drained by the existing 100 ms `ui::poll`, not a background-thread GPUI wake. GPUI's test scheduler rejects cross-thread executor wakes, and the app already coalesces config changes this way; the UI still never waits on a plugin.
- The P1A inventory lives in Settings as one focusable list (`ControlId::Plugins`) with per-row action buttons and list-local keys. Dynamic per-button Tab stops would fight the fixed `ControlId` traversal; the list itself is the keyboard route and every action has a key plus an aria key shortcut.
- `plugins.toml` is written with Crow's comment-preserving `edit_toml`; the first service refresh writes the documented default. `Reload All` covers packages that cannot be addressed by id (unreadable manifest/bundle).
- Critique Q1: the worker no longer performs file I/O. `crow_plugin::read_excerpt(&EntryRef, max_bytes)` is the host-side bounded reader; `RequestInput.excerpt` carries one bounded text into a call, and `PluginHost::evaluate` refuses an excerpt when the worker slot has no `fs = read` grant. P1B calls the helper before enqueue and applies the P8 eligibility gate.
- Critique Q2: the command decoder accepts only `#{ kind: "copy_text", text }`; the `action`/`copy-text` shape and docs were removed.

## Hand checks

- [x] Bundle, temp config, valid + `api = 2` bundles: Settings ▸ Plugins showed Pending approval and Incompatible API rows with permissions and the API error; Approve by mouse changed Copy Relative Path to Running, "1 extension", Disable/Reload, and wrote the `plugins.toml` record with `enabled = true` and its digest. Shots: `target/shots/p1a-plugins-running-firebox.png`, `p1a-plugins-light.png`.
- [x] Status line: the main window showed `Broken Example: plugin API 9 is not supported; this build runs api = 1` (`target/shots/p1a-main-status-firebox.png`).
- [x] Light theme (Solarized Light) and the fixed 560 pt Settings width: names, state, permissions, wrapped error, compact action buttons and footer buttons stay legible with no clipping (`target/shots/p1a-plugins-list-light.png`). Fixed during the check: action labels were shortened to Approve/Disable/Reload/Retry (full plugin name stays in the aria label), the extension count grammar, and a visible focused-row tint.
- [x] Disable/Enable/Reload keys in the bundle: critic verified the key path by mouse-approving Copy Relative Path, then Down to the row and D (Running→Disabled, grant `enabled = false`), then E (Disabled→Running). The GPUI test covers the same path.
- [x] Long names and the fixed 560 pt width: a 129-byte manifest name now truncates with an ellipsis; version, state, Approve and Reload stay inside the list in FireBox and Solarized Light. Before/after: `target/shots/p1a-crit-long-name-firebox.png`, `p1a-crit-long-name-fixed-firebox.png`, `p1a-crit-long-name-fixed-light.png`.
- [x] Plugin action errors: an approval whose bytes changed on disk showed `Copy Relative Path: the package changed on disk; review the new snapshot…` immediately, without another redraw (`target/shots/p1a-crit-status-firebox.png`). A 640 pt main window still shows the plugin name before truncating the detail (`p1a-crit-640-light.png`).
- [x] AX tree: the plugin list exposes `AXRows` and `AXSelectedRows` for all three packages; the selected row reports `AXSelected=1`, each row keeps its full label and help text, and every action is an `AXButton` with the plugin name. Actual VoiceOver speech still needs a human check.
- [ ] VoiceOver speech on the list and rows: the accessible structure is present, but listen to the plugin list and its action buttons once with VoiceOver on.

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

## Critique

Critic session: 2026-11-18 · Base: `cb21dcb` · Head reviewed: `023376d` (after the critique commit; original head `732b6b1`)

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | A manifest name up to the allowed 4 KiB was not bounded in the row: a 129-byte name ran past the 560 pt Settings width and pushed Approve/Reload out of the list. | `crates/crow-app/src/settings_window.rs:2019-2058` | UI | fix | fixed (`023376d`); name truncates, version/state stay next to it, actions stay inside. Before/after: `p1a-crit-long-name-firebox.png`, `p1a-crit-long-name-fixed-firebox.png` |
| C2 | A plugin action error was written to `Operations.message` but never invalidated the window; it only appeared after a forced resize. | `crates/crow-app/src/plugins.rs:79-90` | UX | fix | fixed (`023376d`); every `HostEvent::Status` refreshes the windows. Verified by an approval-error status shot |
| C3 | Action errors sent by `PluginHost::status` reached the status line without the plugin name, unlike parse/runtime errors. | `crates/crow-plugin/src/host.rs:600-607` | plan / UX | fix | fixed (`023376d`); status messages are prefixed with the plugin display name and a host test pins it |
| C4 | Plugin rows were `AXGroup`, not list options; `AXSelectedRows` was empty and the cursor was not announced. | `crates/crow-app/src/settings_window.rs:2044-2050` | plan / a11y | fix | fixed (`023376d`); rows are `ListBoxOption` with `aria_selected`/active descendant. AX now reports 3 `AXRows` and 1 `AXSelectedRows`; selected row has `AXSelected=1` |
| C5 | `read_excerpt` truncated at byte 65536 before UTF-8 decoding; a valid file with no newline and a multibyte character on the boundary returned "file is not UTF-8 text". | `crates/crow-plugin/src/excerpt.rs:44-63` | code | fix | fixed (`023376d`); a newline or `valid_up_to()` cut never splits a character, and tests cover both the boundary and genuinely invalid bytes |
| C6 | README said Settings "edits `settings.toml` only", but Settings ▸ Plugins writes `plugins.toml` approvals. | `README.md:66` | docs | fix | fixed (`023376d`) |
| C7 | After approval wrote the grant, a reload error was swallowed by `unwrap_or_default()` and `grants_error = None`; every grant vanished from memory. | `crates/crow-plugin/src/host.rs:280-284` | code | fix | fixed (`023376d`); a reload failure now refreshes, surfaces the error and returns it |
| C8 | A/E/D/R/T called `plugin_action` even when that action was not shown; Retry on a running package produced an error for a hidden action. | `crates/crow-app/src/settings_window.rs:930-937` | UX | fix | fixed (`023376d`); keys respect `PluginAction::enabled`, and the settings test presses T on Running and asserts no message |
| C9 | `read_excerpt` did file I/O inside Rhai evaluation, contrary to the plan's host-fetch model; it had no P8 gate and could block past the deadline. | `crates/crow-plugin/src/worker.rs`, `docs/plugins.md` | plan / code sense | ask | fixed per Q1 (`023376d`); the worker has no read function or file I/O, `RequestInput.excerpt` carries host-read text, `read_excerpt(&EntryRef, …)` is a host helper, and the host refuses an excerpt without the grant. P1B applies P8 before calling it |
| C10 | P1B's fixture contract was `#{ kind: "copy_text", text }`; P1A implemented `#{ action: "copy-text", text }`. | `crates/crow-plugin/src/registry.rs:140-150` | plan | ask | fixed per Q2 (`023376d`); decoder, docs and all fixtures use `kind: "copy_text"` |
| C11 | Docs did not say copying a bundle while Crow is open needs Reload All. | `docs/plugins.md` install steps | docs | note | fixed in `docs/plugins.md` as part of the critique |
| C12 | Registry tests covered column, command and hook replies but not the other valid shapes. | `crates/crow-plugin/src/registry.rs` tests | code sense | note | fixed; `every_other_reply_shape_is_validated` covers preview, sidebar, status, info and icon-pack plus three rejection cases |
| C13 | Public helpers with no caller: `Grants::iter`, `withdraw`, `SUPPORTED_API`, `bundle_path`, `entry_data`. | `crates/crow-plugin/src/{grants,package,dto}.rs` | code sense | note | left; likely P1B seams, no behavior risk. Remove if P1B does not use them |
| C14 | A disabled package whose bytes change stays `Disabled` instead of the plan table's `Pending approval`; enabling then reveals Pending approval. | `crates/crow-plugin/src/host.rs:207-239` | plan | note | left; a disabled grant is already inert, and forcing Pending would offer Approve (which re-enables) to a user who chose Disabled. Revisit if P1B adds an explicit "changed while disabled" note |
| C15 | The approval row shows "0 extensions" before registration and never shows the manifest's `provides`. | `crates/crow-app/src/settings_window.rs:2057-2065` | UX | note | left; the plan's minimal inventory shows state and permissions. P1B/polish can show provided kinds before approval |

Checks re-run: `cargo fmt --check` ✓ · `cargo clippy --workspace --all-targets -- -D warnings` ✓ · `cargo test --workspace` ✓ (879 passed across 14 suites, 1 ignored `fuzz_long`; `crow-plugin` has 49 tests, `crow-app` 604) · budget not needed: no listing, scroll or first-paint path changed; the 100 ms plugin drain is two constant channel checks.
Looked at: FireBox and Solarized Light Settings with pending/incompatible/long-name inventories at 560 pt; mouse Approve; Down + D/E key path; changed-byte reload and its status line; 640 pt main window; AX rows, selection and action buttons; rebuilt bundle. Not looked at: actual VoiceOver speech (structure verified, listening left to the user).
Verdict: better. The runtime boundary now keeps I/O out of Rhai, the command contract matches P1B, and the three UI/status defects that made approval hard to use are fixed; the remaining notes are polish or future policy, not blockers.

## Next

Push the branch and open the P1A PR from this reviewed head; the plan document and M9 docs reorganization remain the user's untracked changes.
