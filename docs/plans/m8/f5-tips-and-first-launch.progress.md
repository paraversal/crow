# F5 tips and first launch progress
Branch: m8-f5-tips · PR: none · Last updated: 2026-11-17 · State: Ready for critique

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 A separate `tips` setting | done | `crow_core::TipsMode` (`"auto"`/`"off"`), `Settings.tips`, `Settings::tips_muted()` (`tips = "off"` or `hints = "off"`); `TipBook::may_show` takes the computed mute flag; `wants`/`request` and the render-time close use it. Settings ▸ Behaviour ▸ Tips (Auto/Off) writes only the `tips` line. Tests: parse/missing/bad value, both mute paths, independence, the one-key write, the control follows reloads, `tips = "off"` suppresses and hides a tip while hints stay. |
| 2 First-launch popup | done | `Sheet::Onboarding` in `window.rs`; state in `~/.local/state/crow/onboarding.json` (`crow_core::OnboardingState`, `crow-app::onboarding`). The first window of the launch takes the pending question; Show tips / Return writes `tips = "auto"`, No tips writes `"off"`, Escape/scrim records the current setting and closes. Tests: fresh state shows once, both choices write, relaunch does not ask, broken state asks again with no status error, Escape keeps the default, role/labels, tests start past it. |
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

## Hand checks
- [x] Fresh state dir (`CROW_STATE_DIR=$(mktemp -d)`): the "Show tips as you go?" sheet appeared on the first window (bundle, 1200×760, FireBox dark); Return (Show tips) closed it and wrote `tips = "auto"` plus `{"tips_choice": true}`; relaunching with the same state dir showed no sheet.
- [x] Tip card in the bundle: a 250-file folder scrolled with the wheel; the card shows the keycap, the two-line sentence, Got it and "Don't show tips"; the last row sits above the card.
- [ ] No tips in the bundle: covered by `first_launch_no_tips_writes_off`; the bundle click was not driven (no AX click automation in this session).
- [ ] "Don't show tips" in the bundle: covered by `tip_no_tips_mutes_and_closes`; the bundle click was not driven.
- [ ] The sheet's choice writes only the `tips` line in `settings.toml`; deleting `settings.toml` does not bring the sheet back.
- [ ] Settings ▸ Behaviour ▸ Tips: Auto/Off changes at once, writes only the `tips` line, comments survive; an external edit updates the control; a broken file disables it.
- [ ] `tips = "off"`: no tip after a long-folder scroll; `hints` still show. `hints = "off"`: no tips and no hints.
- [ ] Tip card at 640 px and wide, light and dark: text wraps, last row stays above the card; "Don't show tips" is legible in both themes.
- [ ] VoiceOver: the onboarding sheet's title and both buttons are spoken; the Tips control is a radio group with its label and description; the card's two buttons read.

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

## Next
Ready for critique: the branch is local, the progress file is current, and the critique agent reviews it and opens the PR. Remaining hand checks are light/dark, 640 px, VoiceOver, the Settings row, and the two bundle clicks the session could not drive.
