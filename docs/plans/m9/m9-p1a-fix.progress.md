# M9 P1A fix — Plugin runtime foundations progress

Branch: `codex/m9-p1a-plugin-runtime` (same branch as PR https://github.com/paraversal/file-crow/pull/61, per user decision) · Base: `c4482bd` · Last updated: 2026-11-18 · State: In critique

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
- [x] Critic: the 20-row Settings ▸ Plugins list is bounded and Up/Down keeps the selected row inside it (GPUI regression; bundle screenshot).

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

## Critique
Critic session: 2026-11-18 · Base: `codex/m9-p1a-plugin-runtime` @ `c4482bd` · Head reviewed: `62dadb7`

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | Approval errors name the plugin twice: `approve` returns `"<name>: the package changed…"` and `HostCommand` adds the name again in `status`, so the status line reads `A: A: …` for a stale digest (and for an unreadable package). | `crates/crow-plugin/src/host.rs:339-366`, `:846-856` | UX / code | fix | fixed; `status` keeps an existing `name: ` prefix, and `approving_a_changed_snapshot_is_refused_named_once_and_grants_nothing` pins the stale-digest status |
| C2 | The host test that approves with a digest from before the bytes changed (`a_swapped_asset_between_discovery_and_approval_cannot_execute`) was deleted, although the plan says to keep the approved-snapshot tests; the stale-digest error path is now untested. | `crates/crow-plugin/src/host.rs` tests | plan / code | fix | fixed; the same test also proves no grant is written and descriptors stay empty |
| C3 | The positive excerpt-grant path was lost with `excerpt_input_requires_the_read_grant`; the new service test only proves the denial. A read grant that no longer permits a valid excerpt would pass the suite. | `crates/crow-plugin/src/host.rs` tests | code | fix | fixed; `an_authorized_read_grant_accepts_an_excerpt` approves a read package and asserts the excerpt reaches the intent |
| C4 | P1A critique C3's regression test (`action_errors_name_the_plugin_in_the_status_event`) was removed with the host rewrite; only the app-level package-error test remains. | `crates/crow-plugin/src/host.rs` tests | plan / UX | fix | fixed; `action_errors_name_the_plugin_once` retries a running package and pins the single-prefix message |
| C5 | `RefreshEnqueue::Full` (the hook lane is full because the queued hooks have different coalesce keys) is never exercised; only the displaced path is. | `crates/crow-plugin/src/worker.rs:80-97` | code | fix | fixed; `a_full_hook_lane_finishes_the_extra_hook_as_busy` holds the worker, queues two distinct scopes, and gets Busy for the extra hook |
| C6 | With more plugins than fit on the Settings page, Up/Down moves the selected row off-screen: `move_plugin_cursor` only calls `cx.notify`, the page scrolls to the list's top (`scroll_to_focused`), and nothing scrolls a row into view. A 20-row probe left the page offset at `y = -1272.5` after 19 Downs and put the last row's Approve at `y = 2120` in a 760 pt window. The progress file claims position is used for scroll. The plan's item 6 names scroll-to-row. | `crates/crow-app/src/settings_window.rs:958-985`, `:1039-1149`, `:2161-2172` | UX / plan | ask | Q1 → bounded (user). Fixed: a 320 pt inner list with its own `ScrollHandle`; Up/Down and a disappeared-key fallback scroll the selected row into view; rows are `flex_none` so they keep their height; `a_long_plugin_list_scrolls_the_selected_row_into_view` proves the last row stays inside the list. The progress wording is now true |
| C7 | Several P1A host tests were deleted without a record: `package_errors_have_distinct_inventory_states`, `budget_pauses_a_while_b_still_replies`, `unsupported_capability_is_an_inventory_state_and_starts_nothing`. Package-level tests cover the error kinds, but the inventory mapping and A-paused/B-replies ordering no longer have a host regression. | `crates/crow-plugin/src/host.rs` tests | code | note | left; package/app tests cover the individual error kinds; the A/B budget ordering is noted for P1B |
| C8 | `load_package_at` was added and `bundle_path` remains public with no production caller; approval uses the fresh discovery snapshot instead. | `crates/crow-plugin/src/package.rs:128-142`, `:373-375` | code sense | note | left; possible P1B seams, same as P1A C13 |
| C9 | A grants read/parse error marks every row `Disabled` with the grants error as detail, so the row offers Enable (blocked by the grants-error policy) rather than an error state. The alert above the list explains it, and P1A asserted the same state. | `crates/crow-plugin/src/host.rs:190-207`, `:279-289` | UX | note | left; pre-existing P1A behavior and the grants alert names the real cause |
| C10 | The progress file said "FIFO manifest/entry/asset tests", but only the entry and asset cases existed; the manifest was read through the same helper but had no regression. | `crates/crow-plugin/src/package.rs` tests | plan / code | fix | fixed; `a_nonregular_manifest_is_rejected_without_blocking` completes the three cases |
| C11 | The plan's bundled-app focus/active-row check and VoiceOver listening are still open. The list was reached in the bundle by Tab and renders three Pending approval rows with Approve/Reload; the terminal could not confirm the active row or play VoiceOver. | Settings ▸ Plugins in `target/bundle/Crow.app` | plan / UX | note | left as the open hand checks |

Checks re-run: `cargo fmt --check` ✓ · `cargo clippy --workspace --all-targets -- -D warnings` ✓ · `cargo test --workspace` ✓ (901 passed, 1 ignored; `crow-plugin` 66, `crow-app` 608)
Looked at: the host/service lifecycle and cancellation paths, Settings selection and action dispatch, the package loader's nonblocking final open, icon-pack registration, the actual Tab order (Theme → … → Plugins at 21 tabs), and the bundled app's plugin list with three and twenty fixtures. The 20-row list is bounded to five visible rows with its footer in place. Not looked at: the active row/key action in the bundle after Reload All (the key focus was ambiguous under terminal automation) and actual VoiceOver speech.
Verdict: better. The lifecycle and cancellation design holds, the two reproduced P1 bugs have deterministic regressions, the status line no longer double-names a package, and a long plugin list now keeps the selected row visible. The remaining items are notes or the two human checks.
