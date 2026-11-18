# M9 P1A fix — Plugin runtime foundations progress

Branch: `codex/m9-p1a-plugin-runtime` (same branch as PR https://github.com/paraversal/file-crow/pull/61, per user decision) · Base: `c4482bd` · Last updated: 2026-11-18 · State: Ready for critique

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 Reconcile grants and workers together | done | One `RuntimeRecord` per valid ID with snapshot/state/worker/descriptors/packs; service-wide generation allocator; grant re-read withdraws revoked snapshots before the scan; reconcile keeps identical Starting/Running workers and sticky Paused/Failed; Approve/Enable/Disable/Retry route through it. Two-package regression, capability, malformed, identical Reload, changed bytes, Retry and queued-reply tests pass. |
| 2 Make request cancellation authoritative | done | Admitted requests store extension + full stamp + token; replies consume the record once and discard cancelled results; withdrawal finishes requests as Cancelled; dequeue checks the token; a displaced hook is returned to the host; fatal worker failure still stops the worker after ticket cancellation. Tests cover ready-reply cancellation, queued cancellation, disable-before-reply, duplicates and fatal-after-cancel. |
| 3 Reject nonregular bundle files without holding up the service | done | Final `openat` uses `O_NONBLOCK`; type/size checked on the descriptor; FIFO manifest/entry/asset tests and a service test with a valid second plugin and subsequent approval pass. |
| 4 Expose the nonblocking request path on PluginHandle | done | `try_evaluate`/`try_notify` return `RequestTicket` or `SubmitError::{Busy,Stopped}`; one shared request counter; bounded `sync_channel` ingress with one wake per submission; admission checks generation/extension/kind/input/excerpt/hook scope; `HostEvent::RequestFinished { extension, stamp, result }` with typed errors; stopped service closes the event stream. Service tests cover intent + hook stamps, stale generation, bad extension/input, missing capability, both queue-full paths, held-worker saturation with another plugin, coalescing, cancellation, shared counter and stream close. |
| 5 Register icon packs from the approved asset snapshot | done | `DescriptorSource::{Function,Asset}` in `crow-core`; icon packs require a declared asset from the approved snapshot, forbid functions and reject `asset` on other kinds; `RegisteredIconPack` carries ID/generation/asset/private snapshot and clones bytes from it; inventory derives packs from Running; old reply decoder, `ContributionValues::IconPack` and icon-mapping DTOs removed; `docs/plugins.md` updated. Tests cover exact bytes, missing/undeclared/traversal/wrong-kind/wrong-snapshot assets, pack/glyph edits requiring approval, and registration failure. |
| 6 Keep Settings actions bound to a bundle identity | done | Discovery keeps the raw `OsString` bundle key and a separate lossy display name; invalid and duplicate rows keep distinct keys; Settings stores the selected key and uses position only for Up/Down, successor fallback, scroll and AX; one action policy for buttons/Return/keys with the grants-error restriction; keyboard approval compares the rendered bundle+ID+digest target with the latest inventory. GPUI tests cover insertion ahead of the selection, successor/previous fallback, duplicate IDs and blocked keys during a grants error. |

## Questions

- Q1 (asked 2026-11-18): where the P1A fixes land, since #61 is still open → answer: put the fixes on #61 itself (user, "same").

## Decisions

- Base is the open PR head `c4482bd` on `codex/m9-p1a-plugin-runtime`, not `main`; the plan's `codex/m9-p1a-fix` branch is superseded by the user's choice. Commit locally and stop at Ready for critique; do not push.
- The user's uncommitted M8→done / M9 docs reorganization stays untouched and uncommitted.
- `PluginHandle` sends one submission wake per successful `try_send`; the service drains exactly one ingress item per wake, matching the plan and bounding both the ingress and the wake queue.
- `RegisteredIconPack` lives in `crow-plugin::dto`, next to `PluginEntry`, because it must keep the approved `PackageSnapshot` private while the app holds a clone.
- On macOS the file system rejects invalid UTF-8 bundle names, so the non-UTF-8 name rule is covered through the `loadable_name` classifier; the raw `OsString` key is still carried end to end in `PackageCandidate`/`PluginEntry`.

## Hand checks

- [x] GPUI keystroke test: select B, insert a bundle ahead of B, press A, only B's grant is written.
- [x] GPUI keystroke test: remove the selected middle row → successor; remove the last row → previous row.
- [x] GPUI keystroke test: two duplicate-ID rows keep distinct keys and cannot be approved.
- [x] GPUI keystroke test: A/E/D/T are no-ops while `plugins.toml` is invalid; R still reloads.
- [x] Bundled app launches from `target/bundle/Crow.app` with a temp HOME and three fixture bundles; Settings opens and renders (shot: `target/shots/p1a-fix-settings.png`). The Plugins list is below the fixed 792 pt window fold; macOS Accessibility scroll was unavailable to the terminal, so the list itself was not driven in the bundle.
- [ ] Bundled app: focus and active row after Reload All, by mouse and keyboard; both key layers. The same selection policy is covered by the four GPUI tests, but the real window/focus feel needs the user.
- [ ] VoiceOver: listen once to the plugin list and its action buttons.

## Found along the way

- macOS APFS rejects invalid UTF-8 directory names with `EILSEQ`, so a discovery-level fixture cannot be created there; the classifier is unit-tested instead. This does not change the runtime rule: a name the loader cannot represent is never opened lossily.

## Log

- 2026-11-18 Read the plan, M9 index, P1A progress, PR #61 state. Asked and recorded the branch decision. Reading `crow-plugin` and the Settings integration before changing code.
- 2026-11-18 Item 3 first: final component opened `O_NONBLOCK`; FIFO entry/asset loader tests; service test with a FIFO bundle, a valid plugin and a later approval.
- 2026-11-18 Rewrote `host.rs` around one runtime record. Added request records, typed tickets/terminal results, the bounded ingress and the service wake path. Added 20 host/service regression tests (lifecycle, cancellation order, saturation, FIFO, stream close, icon packs).
- 2026-11-18 Item 5: typed `DescriptorSource` in `crow-core`; registration asset validation and `RegisteredIconPack`; removed the unreachable icon-pack reply path and docs updated.
- 2026-11-18 Item 6: raw bundle keys through discovery and inventory; Settings stores the key with position only for movement/fallback/AX; unified action policy and rendered approval target; four GPUI tests added.
- 2026-11-18 `cargo fmt --check`, workspace Clippy with `-D warnings` and `cargo test --workspace` pass.
- 2026-11-18 `./scripts/bundle.sh` built and signed `target/bundle/Crow.app`; launched it with a temp HOME and three fixtures, opened Settings, captured `target/shots/p1a-fix-settings.png`. The plugin section sits below the fixed window fold and terminal scroll automation was blocked, so the two human checks stay open.

## Next

Commit the fix list, set Ready for critique, and hand the two human checks to the user/critic.
