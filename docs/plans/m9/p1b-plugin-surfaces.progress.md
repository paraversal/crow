# M9 P1B — Plugin contributions in the app progress

Branch: `codex/m9-p1b-plugin-surfaces` · PR: none · Last updated: 2026-11-18 · State: in progress
Base: `d5cab1e` (M9 P8, merged as #65). Dependencies merged: P1A `083d40c` (#61, includes P1A fix), P8 `d5cab1e` (#65).

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 Verify runtime seams and run native spikes | done | Seams mapped; three bundle spikes observed (payload dispatch, per-type availability, text-field focus; exact range via `UniformListDecoration`; context menus restore focus, palette does not save its source). Spike code reverted. |
| 2 Complete the API-1 wire contract | done | `Descriptor` carries column width/min/align, command target/context-menu/suffixes and preview suffixes; `EntryValue` carries tint/dim; `SidebarRow` carries an optional handle; `CommandIntent` covers message/copy_text/reveal. Registry validates per-kind fields, unit replies, badge/extension bounds, rectangular tables and request-local reveal handles. `RequestTicket::is_cancelled`. 77 `crow-plugin` tests pass, including a real worker decoration/preview round trip. |
| 3 Build the app coordinator and hook delivery | todo | |
| 4 Wire native menu, command palette and context commands | todo | |
| 5 Render columns, decorations, status and Info | todo | |
| 6 Add the shared sidebar section shell | todo | |
| 7 Connect configured Preview providers and P8 read eligibility | todo | |
| 8 Ship the test fixture and author documentation | todo | |
| 9 Run the acceptance matrix and hand off | todo | |

## Questions

- Q1 (asked 2026-11-18): per-item plugin-command availability in the native menu. GPUI validates a menu item by **action type** (`App::is_action_available`, `vendor/zed/crates/gpui/src/app.rs:2991`, `key_dispatch.rs:382`), and the macOS delegate passes only the action instance to that check (`vendor/zed/crates/gpui_macos/src/platform.rs:1571-1595`). One `PluginCommand` action type therefore enables/disables every plugin command together; the bundle spike confirmed Alpha and Beta enable/disable as a pair, and a root-level handler keeps them enabled while the `/` filter field has focus. Options: (A, recommended) small GPUI patch adding a default-true per-action `is_available(window, cx)` checked by `is_action_available`; (B) one action type per surface/class, no per-command greying; (C) rebuild the menu on focus/selection transitions (contradicts the plan; GPUI's action store appends on every rebuild); (D) leave items enabled and no-op after revalidation. → waiting.

## Decisions

- Base `d5cab1e`; branch `codex/m9-p1b-plugin-surfaces`. The user's uncommitted M8→done / M9 docs reorganization stays untouched and uncommitted.
- API stays experimental at `api = 1`; no new GPUI patch unless a spike proves one is required, and that needs the user's check-in first.
- Q1 answered **A (user)**: add a small GPUI patch with a default-true per-action `is_available(window, cx)`, checked by `is_action_available`, so a plugin command can answer for its own descriptor without rebuilding menus. New `patches/gpui-*.patch`; documented in the progress file and PR body.
- Step 1 spikes are throwaway and reverted; the findings are in the Log. Regressions land with the implementation steps.

## Hand checks

- (to be filled as steps land)

## Found along the way

- (none yet)

## Log

- 2026-11-18 Read the plan, M9 execution contract, P1A/P1A-fix/P8 progress. Created the branch from `d5cab1e` and this progress file before code.
- 2026-11-18 Step 1 seams verified: `plugins::poll` drains `Inventory`, `Status` and `RequestFinished` (`plugins.rs:66-91`; the last is currently a no-op); `try_evaluate`/`try_notify` return `RequestTicket` but it exposes only `cancel()` (`crow-plugin/src/dto.rs:136-159`); P8 exposes `CloudStatuses::gate() -> Arc<dyn ReadGate>` (`cloud_status.rs:149`). Visible ranges are available from GPUI's unused `UniformListDecoration::compute` exact range (`vendor/zed/.../uniform_list.rs:518-533`), not from the row renderer. Preview owners are `Pane::load_preview` (`pane.rs:1285`) and `SearchView::load_preview` (`search.rs:854`); both keep their own generation. Hook sources: `FolderEvent::Changed` is too coarse (`folder.rs:1582` etc.); real watcher applies land in `apply_folder_changes`/`reconcile`; `operations::finish` (`operations.rs:331`) misses undo/redo; selection generations bump for some cursor-only and reset paths, so dedupe by actual selected set.
- 2026-11-18 Step 1 native spike: one `SpikePluginCommand` action type with two payloads in a `File ▸ Plugins Spike` submenu dispatched `example.alpha gen 7` and `example.beta gen 9` correctly; a handler-less `SpikeUnavailable` item was disabled (`enabled = false`); with Settings key, plugin items were disabled; with the list focused they were enabled; with the `/` filter field focused they stayed enabled because the handler is on the window root. GPUI's macOS action store appends on every `set_menus` and resolves old tags to old actions, so a stale open menu cannot call a different descriptor, but every rebuild leaks one action entry per item.
- 2026-11-18 Asked Q1 before designing the menu path; continuing with the wire contract (step 2) meanwhile.
- 2026-11-18 Step 2: extended the owned DTOs (`Descriptor` presentation fields, `Tint`, `EntryValue.dim`, `SidebarRow.handle`, `CommandIntent::{Message,CopyText,Reveal}`), added badge/suffix limits, and rewrote registration/reply validation per kind. Every new positive shape, default, unit, cap, bad handle, duplicate handle, unknown tint/effect, ragged table and published-descriptor recheck has a test; a real worker round-trips a decoration and a preview table. Q1 answered A (GPUI patch).

## Next

Step 3: app coordinator, capture/schedule/validate/cache/hooks.
