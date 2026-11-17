# F5 tips and first launch progress
Branch: m8-f5-tips (combined PR head `m8-f5-f8`) · PR: #51 · Last updated: 2026-11-17 · State: PR open

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 A separate `tips` setting | done | `crow_core::TipsMode` (`"auto"`/`"off"`), `Settings.tips`, `Settings::tips_muted()` (`tips = "off"` or `hints = "off"`); `TipBook::may_show` takes the computed mute flag; `wants`/`request` and the render-time close use it. Settings ▸ Behaviour ▸ Tips (Auto/Off) writes only the `tips` line. Tests: parse/missing/bad value, both mute paths, independence, the one-key write, the control follows reloads, `tips = "off"` suppresses and hides a tip while hints stay. |
| 2 First-launch popup | done | `Sheet::Onboarding` in `window.rs`; state in `~/.local/state/crow/onboarding.json` (`crow_core::OnboardingState`, `crow-app::onboarding`). The first window of the launch takes the pending question; Show tips / Return writes `tips = "auto"`, No tips writes `"off"`, Escape/scrim records the current setting and closes. Tab/Shift-Tab move a visible focus ring between the two choices and Return activates the focused one; the sheet sits 8 px lower than the other sheets. Tests: fresh state shows once, both choices write, relaunch does not ask, broken state asks again with no status error, Escape keeps the default, role/labels, the tab ring, tests start past it. |
| 3 "Shut up" from the tip card | done | "Don't show tips" link beside Got it (`tip-no-tips`): closes the card and writes `tips = "off"`; the next tip does not appear. Role, label and description set. Test: `tip_no_tips_mutes_and_closes`. |
| 4 Tip position | done (F1, #48) | F1's Q5 answer: the list and column bodies reserve `tips::INSET` while a tip is visible; `tip_does_not_cover_the_last_row` asserts the last row sits above the card. F5 grew `INSET` from 64 to 80 because the card's second action wraps the sentence to two lines (58 px card + 8 px above the bar). |

## Questions
- Q1 (plan) setting name and values: `tips = "auto" | "off"`. → answered by the M8 index decisions (2026-11-17): a separate `tips = "auto" | "off"` setting; hints stay independent.
- Q2 (plan) does "No tips" also turn off key hints? → answered by the M8 index decisions: no, hints stay independent.
- Q3 (asked 2026-11-17): first-launch detection: `~/.local/state/crow/onboarding.json` (recommended) or a field in `tips.json`? → answer: **`onboarding.json`** (user).
- Q4 (asked 2026-11-17): add a small "Don't show tips" link to the tip card (recommended) or keep the setting in the first-launch popup and Settings only? → answer: **add it** (user).
- Q5 (plan) tip position: move the card or reserve a bottom inset? → answered by F1's Q5 (user): reserve a bottom inset while a tip is visible. Landed in F1 (#48); item 4 is therefore done.

## Decisions
- Separate `tips` setting with `"auto"` / `"off"`, hints independent — user, via the M8 index decisions (2026-11-17).
- `hints = "off"` keeps muting tips as a fallback — plan (item 1).
- The first-launch answer lives in its own state file, `onboarding.json` — user (Q3).
- The tip card gets "Don't show tips" — user (Q4).
- Tip position: the list and column bodies reserve a bottom inset only while a tip is visible — user, via F1's Q5; built in F1 (#48).
- The inset is 80 px, not F1's 64: the card's second action narrows the sentence to two lines, so the card is 58 px plus 8 px above the bar. The inset is transient (only while a tip is visible).
- Tests start already onboarded: `ui::init_for_tests` writes an answered `onboarding.json`, so only tests that remove it see the sheet. The live fuzzer's `prepare` writes the same answer; the settings fuzz smoke uses `init_for_tests` and does not hang on it.
- The first-launch sheet sits 8 px lower than the other sheets (`.mt_2()` on its panel only) — user, 2026-11-17.
- Both choices form a Tab/Shift-Tab ring with a visible focus ring; Return activates the focused choice, Escape keeps the current setting — user, 2026-11-17, answering the critic's keyboard question.

## Hand checks
- [x] Fresh state dir (`CROW_STATE_DIR=$(mktemp -d)`): the "Show tips as you go?" sheet appeared on the first window (bundle, 1200×760, FireBox dark); Return (Show tips) closed it and wrote `tips = "auto"` plus `{"tips_choice": true}`; relaunching with the same state dir showed no sheet.
- [x] Tip card in the bundle: a 250-file folder scrolled with the wheel; the card shows the keycap, the two-line sentence, Got it and "Don't show tips"; the last row sits above the card.
- [x] No tips in the bundle (critic): clicked "No tips" on a fresh sheet; it closed, `settings.toml` gained `tips = "off"` and `onboarding.json` became `{"tips_choice": false}`.
- [x] "Don't show tips" in the bundle (critic): clicked the link on the card; it closed, `tips = "off"`, and `tips.json` retired `filter`.
- [x] The sheet's choice writes only the `tips` line in `settings.toml`; deleting `settings.toml` does not bring the sheet back (critic: custom 5-line file gained only `tips = "off"`; relaunch after deleting the file showed no sheet).
- [x] Settings ▸ Behaviour ▸ Tips: Auto/Off shows the current value and writes only the `tips` line (critic screenshot, Off selected after "Don't show tips"); external edit and broken-file behaviour are covered by `tips_control_writes_off_and_follows_reloads`.
- [x] `tips = "off"`: no tip after a long-folder scroll; `hints` still show (critic: FireBox and Solarized Light; `hints = "always"` kept the bottom-bar hints). `hints = "off"`: no tips and no hints (`tip_hints_off_suppresses_and_hides`).
- [x] Tip card at 640 px and wide, light and dark: text wraps, last row stays above the card; "Don't show tips" is legible (critic: FireBox 1280, Solarized Light 1280 and 640; the 640 card is two lines and the action fits).
- [x] Keyboard (critic): Tab rings Show tips → No tips, Shift-Tab goes back, Return on No tips writes `tips = "off"`; `first_launch_tab_reaches_no_tips` covers it.
- [ ] VoiceOver: the onboarding sheet's title and both buttons are spoken; the Tips control is a radio group with its label and description; the card's two buttons read. Not driven in this session; the roles and labels are asserted by `first_launch_asks_once_and_show_tips_writes_auto`, `tip_no_tips_mutes_and_closes` and the settings accessibility test.

## Found along the way
- None yet.

## Log
- 2026-11-17: Read AGENTS.md, the M8 index, the F5 plan, `crow-core/src/tips.rs`, `crow-app/src/tips.rs`, `settings.rs`, `settings_window.rs`, `ui.rs`, and the window's sheet/tip code. F1 already landed item 4. Created the branch and this file. Asked Q3 and Q4.
- 2026-11-17: Item 1. `TipsMode`, `Settings.tips`, `tips_muted()`, the Settings row, the write path, and the tests. `cargo test -p crow-core settings`, `-p crow-core tips`, `-p crow-app settings_window::tests`, `each_setting_writes_only_its_key`, `tip_tips_off_suppresses_and_hides_but_keeps_hints` pass.
- 2026-11-17: Item 2. `OnboardingState` + app-side `Onboarding`, `Sheet::Onboarding`, the first window takes it, both choices write `tips`, Escape records the current setting. Five first-launch tests pass; `fuzz_settings_smoke` passes.
- 2026-11-17: Item 3. "Don't show tips" on the card; `tip_no_tips_mutes_and_closes` passes. `INSET` 64 → 80 because the card now wraps to two lines; all 24 tip tests pass.
- 2026-11-17: README's settings table and hints/tips section, `docs/testing.md`'s tips, Behaviour and VoiceOver rows updated.
- 2026-11-17: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass (489 crow-app tests, 1 ignored). The IO guard flagged the test-only onboarding writes; both lines now carry `// io-ok: test-only state`.
- 2026-11-17: `./scripts/bundle.sh` built; a fresh state dir showed the sheet, Return wrote `tips = "auto"` and `{"tips_choice": true}`, and a relaunch showed no sheet. A 250-file folder scrolled with the wheel showed the card with both actions and the last row above it. `scripts/shot.sh` now writes an answered `onboarding.json`, so screenshots do not capture the sheet.
- 2026-11-17: No bench: the inset only applies while a tip is visible, and the bench (`--bench`) shows no tips; the normal list layout and first paint are unchanged.
- 2026-11-17: Critique. The user asked for the popup slightly lower and for "No tips" to be keyboard-reachable. Added `.mt_2()` to the onboarding panel and a Tab/Shift-Tab focus ring over the two choices with Return activating the focused one; the render focus invariant now allows both choice handles. `first_launch_tab_reaches_no_tips` passes; fmt, clippy and the workspace tests pass (490 crow-app). The bundle hand checks cover the lower sheet, the Tab ring, both bundle clicks, the Settings row and the tip card at 640 px and in both themes.

## Critique
Critic session: 2026-11-17 · Base: `main` @ `42ba5f4` · Head reviewed: `31dd883`

| # | Finding | Where | Area | Kind | Outcome |
| --- | --- | --- | --- | --- | --- |
| C1 | User: the first-launch popup should sit slightly lower on the screen. The panel was at the shared sheet position, 8 px below the band. | `crates/crow-app/src/window.rs:1810` | UI | fix | fixed: `.mt_2()` on the onboarding panel only; screenshot `/tmp/f5-onb-initial2.png`. |
| C2 | User: "No tips" was mouse/VoiceOver-only. Tab and arrows are inert in the `Sheet` keymap, and Escape keeps the default, so a keyboard-only user could not choose it. | `crates/crow-app/src/window.rs:420-429`, `:1527-1540`, `:1558-1563`, `:1788-1808`; `crates/crow-app/src/window/tests.rs:9698` | UI / a11y | fix | fixed: the two choices have focus handles, a Tab/Shift-Tab ring with a focus ring, Return activates the focused choice, and the render focus invariant allows the choices; `first_launch_tab_reaches_no_tips` covers it. |
| C3 | The primary choice's focus ring is `cursor_ring` (accent at 0x99) over an accent fill, so it blends into the button; the secondary ring is clear. | `crates/crow-app/src/window.rs:1598` | UI | note | left: the primary choice is already visually the default; a contrasting ring would be a design change beyond the user's request. |
| C4 | `hints = "off"` still mutes tips, so "Show tips" on a machine with hints off writes `tips = "auto"` but shows no tips. | `crates/crow-core/src/settings.rs:257`, `crates/crow-app/src/window.rs:1516-1521` | code / UX | note | left: the plan keeps `hints = "off"` as the older one-switch fallback; the sheet is a first-launch path and a fresh file has `hints = "auto"`. |
| C5 | VoiceOver was not driven in this session. | onboarding sheet, Tips control, tip card | a11y | note | left: roles and labels are asserted by the onboarding, tip-card and settings accessibility tests; the user's VoiceOver hand check remains. |

Checks re-run: `cargo fmt --check` ✓ · `cargo clippy --workspace --all-targets -- -D warnings` ✓ · `cargo test --workspace` ✓ (490 crow-app, 99 crow-core, 44 crow-fs, 6 crow-macos, 20 crow-search, 6 crow-theme; 1 ignored) · bench not needed: no list or first-paint change; the tip inset only applies while a tip is visible.
Looked at: fresh onboarding sheet in FireBox dark (position, mouse No tips, Tab ring, Return on No tips), relaunch after deleting `settings.toml`, Settings ▸ Behaviour ▸ Tips (Off selected), tip card in FireBox dark and Solarized Light at 1280 and 640 px, "Don't show tips" click. VoiceOver not driven; the sheet in light theme was not separately captured.
Verdict: better. The sheet sits lower, both choices work by mouse, VoiceOver and keyboard, and the tip card fits at 640 px without covering the last row.

## Next
PR #51 is open (combined with F8 at the user's request). The user hand-tests, then squash-merges when CI is green. Next M8 piece: F4 (path bar).
