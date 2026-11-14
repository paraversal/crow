# u9-tips progress
Branch: `ui-u9-tips` · PR: none · Last updated: 2026-11-14 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 Tip model in `crow-core` | done | `crates/crow-core/src/tips.rs`: `TipId`, `TipState` (`tips.json`), `TipBook`, `ZeroMatchStreak`, constants; 7 unit tests. `tracing` added to `crow-core` (workspace dep, named by the plan). |
| I2 App glue: `Tips` global, persistence, key uses | todo | |
| I3 Showing and closing the tip in the window | todo | |
| I4 Trigger: scrolling a long folder → `/` | todo | |
| I5 Trigger: two zero-match filters → ⌘F | todo | |
| I6 Trigger: Go to Folder for a known folder → ⌘K | blocked (Q10) | plan's `go_to_folder` no longer exists as a sheet; ⇧⌘G opens the palette |
| I7 Persistence across launches | todo | |

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
- Q1–Q5, Q7–Q10: every recommended answer accepted by the user, 2026-11-14.
- `TipBook::on_screen` stores `Option<TipId>` (not a bool) so `tips::key_used` can close only the tip whose key ran. The plan's sketch had a bool; the behavior needs the id.
- `tracing` added to `crow-core` (`crates/crow-core/Cargo.toml:16`); the plan's state-file spec names `tracing::warn!` and the crate is already a workspace dependency.
- Q6 settled by code, not by the user: U1's `Keycap` has `text_role()` (`crates/crow-app/src/chrome.rs:56-60`), so the tip uses `KeycapWeight::Key` with `.text_role("accent")` instead of a second keycap style. Recorded here per the plan's Q6 fallback.
- Bottom bar height: U5 settled 30 px (`crates/crow-app/src/bottom_bar.rs:733`) but exported no constant. Add `bottom_bar::HEIGHT` and use it for `render` and the tip's `bottom` offset (the plan's `BOTTOM_BAR_HEIGHT`).
- Plan line numbers are stale (merged `window.rs` is 7529 lines). Current locations are recorded in the Log/Notes as items start.

## Hand checks
- [ ] Tips: with a fresh `CROW_STATE_DIR`, scroll a 200+ item folder → "Type / to filter this folder by name." bottom right above the bottom bar, in list and column view, FireBox and Solarized Light; Got it closes it and it never returns after relaunch. Two zero-match filters in a row → the ⌘F tip; ⇧⌘G to a recent folder → the ⌘K tip. Using the key closes and retires a tip. `hints = "off"` shows none. Opening Activity closes a tip; a narrow window wraps the text inside 12 px margins.
- [ ] Tips with VoiceOver: the tip is read once when it appears ("Tip: …"); "Got it" is a button that VoiceOver can press.

## Found along the way
- ⇧⌘G is no longer a separate sheet: it opens the command palette with the current path typed (`window.rs:444-449`). This is the Q10 conflict, not a bug.

## Log
- 2026-11-14: read `AGENTS.md`, the U9 plan, the UI overhaul README, U5's progress file, and the merged code (`window.rs`, `pane.rs`, `folder.rs`, `ui.rs`, `usage.rs`, `keys.rs`, `chrome.rs`, `bottom_bar.rs`, GPUI's `uniform_list`/`on_scroll`). Created the branch and this file. Asked Q1–Q10.

## Next
I2: `crates/crow-app/src/tips.rs` (`Tips` global, persistence, key uses).
