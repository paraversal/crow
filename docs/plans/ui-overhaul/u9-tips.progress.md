# u9-tips progress
Branch: `ui-u9-tips` · PR: none · Last updated: 2026-11-14 · State: Ready for critique

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 Tip model in `crow-core` | done | `crates/crow-core/src/tips.rs`: `TipId`, `TipState` (`tips.json`), `TipBook`, `ZeroMatchStreak`, constants; 7 unit tests. `tracing` added to `crow-core` (workspace dep, named by the plan). |
| I2 App glue: `Tips` global, persistence, key uses | done | `crates/crow-app/src/tips.rs`: `Tips` global (`tips.json` in the state dir, chained background saves), `init` from `ui::init` and `init_for_tests`, `request`/`closed`/`key_used`/`wants`/`key_text`/`text`; `key_used` called from `Pane::start_filter`, `CrowWindow::show_search` (refocus and open) and `show_palette`. |
| I3 Showing and closing the tip in the window | done | `CrowWindow.tip`, `offer_tip`/`close_tip`/`tip_blocked`; `render_tip` (surface card, accent `Key` cap, sentence, Got it, `Role::Status`); drawn after the bottom bar, before sheets; one render guard closes on hints off / another owner / a blocking state; `cx.on_release` clears the owner. Tests: Got it, folder change, hints off, busy states, one at a time, layout (12 px right, 8 px above the 30 px bar). |
| I4 Trigger: scrolling a long folder → `/` | done | `PaneEvent::LongFolderScrolled`; `.on_scroll` on the list body and each column; `first_visible_row` = `floor(-offset.y / 24)`; early return via `tips::wants`. Tests: 250-file list and column, short folder, key use. The listener sees the offset updated (the inner list's handler runs first). |
| I5 Trigger: two zero-match filters → ⌘F | done | `PaneEvent::FilterEnded { empty, matches }` from `close_filter`, `drop_filter` and `Pane::enter` (navigation takes the filter directly, not through `drop_filter`); `CrowWindow.zero_streak`; the offer is deferred past a navigation's `Entered`. Tests: two in a row, reset on a match, dropped by navigation, key retires. |
| I6 Trigger: Go to Folder for a known folder → ⌘K | done | Q10 applied: `confirm_palette`'s path-mode folder branch and `reveal_typed_path`'s directory branch call `known_folder` (`palette_recents`, not the current folder); the offer is deferred past the navigation's `Entered`. Tests: recent folder, new folder, ⌘K retires. |
| I7 Persistence across launches | done | `tip_state_survives_reload`: Got it, reload, no second scroll tip; a never-shown Search tip still shows (cooldown cleared). Saves are all in `background_spawn`. |

## Questions
- Q1 (asked 2026-11-14): retired when shown → **yes** (user).
- Q2 (asked 2026-11-14): first visible row ≥ 30 in a folder of ≥ 200 entries → **yes** (user).
- Q3 (asked 2026-11-14): pointer and touch scrolls only → **yes** (user).
- Q4 (asked 2026-11-14): `palette_recents` (Recent plus favorites) → **yes** (user).
- Q5 (asked 2026-11-14): 10 minutes after a tip was shown, in memory → **yes** (user).
- Q6 (asked 2026-11-14): Keycap style: the board's 22 px accent cap vs U1's `Key` (20 px, `surface`/`bg` fill, `fg` text). Recommended: U1 `Key` with an accent text role. **Settled by code:** `Keycap::text_role` exists (`crates/crow-app/src/chrome.rs:56-60`), so no ASK is needed; use `KeycapWeight::Key` + `.text_role("accent")`.
- Q7 (asked 2026-11-14): menu and palette runs retire the tip → **yes** (user).
- Q8 (asked 2026-11-14): `always` shows tips; only `off` suppresses → **yes** (user).
- Q9 (asked 2026-11-14): no new key; using the taught key or changing folder closes it → **yes** (user).
- Q10 (asked 2026-11-14): ⇧⌘G is the palette now, so the trigger is a **path-mode** palette confirmation landing on a folder in `Ui.palette_recents` → **yes** (user). Copy unchanged.

## Decisions
- Offers that follow a navigation are deferred (`cx.defer_in`): GPUI delivers `cx.emit` as an effect after the current update, so a synchronous offer was closed by the navigation's own `Entered` right after it appeared (backtrace: `apply_emit_effect` → `on_pane_event` → `close_tip`). Affects the GoTo tip and the FilterEnded → Search tip.
- `Pane::enter` emits `FilterEnded` itself: navigation takes `self.filtering` directly (`pane.rs:655`), so `drop_filter` is not on that path. The plan's "a filter dropped by navigation counts like Esc" needed this hook.
- Blocking states close a shown tip in one `render` guard (`tip_blocked`) rather than at each opener; the guard runs before the frame is built, so the tip is never drawn under an overlay. Openers still `notify()`.
- `TipBook::on_screen` stores `Option<TipId>` (not a bool) so `tips::key_used` can close only the tip whose key ran. The plan's sketch had a bool; the behavior needs the id.
- `tracing` added to `crow-core` (`crates/crow-core/Cargo.toml:16`); the plan's state-file spec names `tracing::warn!` and the crate is already a workspace dependency.
- `bottom_bar::HEIGHT = 30.` added and used by `render` and the tip's offset (the plan's `BOTTOM_BAR_HEIGHT`; U5 had the number inline).
- `file-list` gained a `debug_selector` so the scroll tests can find it (`column-0` already had one).
- The plan's Finish says push and open the PR; the execute-plan skill says commit locally and stop at "Ready for critique" (the critic opens the PR). Followed the skill.
- Q1–Q5, Q7–Q10: every recommended answer accepted by the user, 2026-11-14.
- `TipBook::on_screen` stores `Option<TipId>` (not a bool) so `tips::key_used` can close only the tip whose key ran. The plan's sketch had a bool; the behavior needs the id.
- `tracing` added to `crow-core` (`crates/crow-core/Cargo.toml:16`); the plan's state-file spec names `tracing::warn!` and the crate is already a workspace dependency.
- Q6 settled by code, not by the user: U1's `Keycap` has `text_role()` (`crates/crow-app/src/chrome.rs:56-60`), so the tip uses `KeycapWeight::Key` with `.text_role("accent")` instead of a second keycap style. Recorded here per the plan's Q6 fallback.
- Bottom bar height: U5 settled 30 px (`crates/crow-app/src/bottom_bar.rs:733`) but exported no constant. Add `bottom_bar::HEIGHT` and use it for `render` and the tip's `bottom` offset (the plan's `BOTTOM_BAR_HEIGHT`).
- Plan line numbers are stale (merged `window.rs` is 7529 lines). Current locations are recorded in the Log/Notes as items start.

## Bench (100k fixture, same machine, release)
| Run | warm first paint | p50 | p95 | p99 |
| --- | --- | --- | --- | --- |
| U9 branch 1 | 6 ms | 8.33 ms | 9.42 ms | 9.66 ms |
| U9 branch 2 | 9 ms | 8.33 ms | 9.51 ms | 9.68 ms |
| U9 branch 3 | 9 ms | 8.32 ms | 8.55 ms | 9.50 ms |
| `main` 1 (clean) | 9 ms | 8.34 ms | 8.50 ms | 9.33 ms |
| `main` 2 (machine loaded) | 6 ms | 8.34 ms | 33.31 ms | 33.52 ms |
| `main` 3 (machine loaded) | 9 ms | 8.34 ms | 8.57 ms | 33.38 ms |

The clean comparison is branch 9.50 ms vs `main` 9.33 ms p99: 0.17 ms, within run-to-run noise. Two `main` runs taken while the machine was loaded (33 ms p95/p99) were discarded; the branch's clean runs were taken before and after them.

## Hand checks
- [ ] Tips: with a fresh `CROW_STATE_DIR`, scroll a 200+ item folder → "Type / to filter this folder by name." bottom right above the bottom bar, in list and column view, FireBox and Solarized Light; Got it closes it and it never returns after relaunch. Two zero-match filters in a row → the ⌘F tip; ⇧⌘G to a recent folder → the ⌘K tip. Using the key closes and retires a tip. `hints = "off"` shows none. Opening Activity closes a tip; a narrow window wraps the text inside 12 px margins.
- [ ] Tips with VoiceOver: the tip is read once when it appears ("Tip: …"); "Got it" is a button that VoiceOver can press.

## Found along the way
- ⇧⌘G is no longer a separate sheet: it opens the command palette with the current path typed (`window.rs:444-449`). This is the Q10 conflict, not a bug.

## Log
- 2026-11-14: Finish. Bench (above) within noise of `main`; `./scripts/bundle.sh` built `target/bundle/Crow.app` signed "Crow Dev". `cargo fmt --check`, clippy (`-D warnings`) and `cargo test --workspace` green. State set to Ready for critique; not pushed, no PR (the critique agent opens it).
- 2026-11-14: I2–I7 done. `cargo fmt --check`, clippy (`-D warnings`) and `cargo test --workspace` green (275 app tests, 79 core). README and `docs/testing.md` updated (tips paragraph, automated list, hand rows). Bench on the branch: p99 9.66–9.68 ms, warm first paint 6–9 ms (first run after compiling: 34.96 ms, discarded as cold).
- 2026-11-14: read `AGENTS.md`, the U9 plan, the UI overhaul README, U5's progress file, and the merged code (`window.rs`, `pane.rs`, `folder.rs`, `ui.rs`, `usage.rs`, `keys.rs`, `chrome.rs`, `bottom_bar.rs`, GPUI's `uniform_list`/`on_scroll`). Created the branch and this file. Asked Q1–Q10.

## Next
Ready for critique. The critic should look at: the deferred offers (`cx.defer_in`) around navigation, the render-time tip guard, and the `enter` hook for `FilterEnded`. Hand checks (tips, VoiceOver) still need a human.
