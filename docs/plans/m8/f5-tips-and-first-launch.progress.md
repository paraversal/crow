# F5 tips and first launch progress
Branch: m8-f5-tips · PR: none · Last updated: 2026-11-17 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| 1 A separate `tips` setting | in progress | `crow_core::TipsMode`, `Settings.tips`, `tips_muted()`, Settings row, parse and write tests |
| 2 First-launch popup | todo | depends on Q3 |
| 3 "Shut up" from the tip card | blocked (Q4) | optional in the plan |
| 4 Tip position | done (F1, #48) | F1's Q5 answer: the list and column bodies reserve `tips::INSET` (64 px) while a tip is visible; `tip_does_not_cover_the_last_row` asserts the last row sits above the card |

## Questions
- Q1 (plan) setting name and values: `tips = "auto" | "off"`. → answered by the M8 index decisions (2026-11-17): a separate `tips = "auto" | "off"` setting; hints stay independent.
- Q2 (plan) does "No tips" also turn off key hints? → answered by the M8 index decisions: no, hints stay independent.
- Q3 (asked 2026-11-17): first-launch detection: `~/.local/state/crow/onboarding.json` (recommended) or a field in `tips.json`? Both survive deleting `settings.toml`; the question is which state file owns the answer. → answer: pending.
- Q4 (asked 2026-11-17): add a small "Don't show tips" link to the tip card (recommended) or keep the setting in the first-launch popup and Settings only? → answer: pending.
- Q5 (plan) tip position: move the card or reserve a bottom inset? → answered by F1's Q5 (user): reserve a bottom inset while a tip is visible. Landed in F1 (#48); item 4 is therefore done.

## Decisions
- Separate `tips` setting with `"auto"` / `"off"`, hints independent — user, via the M8 index decisions (2026-11-17).
- `hints = "off"` keeps muting tips as a fallback — plan (item 1).
- Tip position: the list and column bodies reserve a bottom inset only while a tip is visible — user, via F1's Q5; built in F1 (#48).

## Hand checks
- [ ] Fresh state dir (`CROW_STATE_DIR=$(mktemp -d)`): the "Show tips as you go?" sheet appears once on the first window; Show tips and No tips both close it; a relaunch does not show it.
- [ ] The sheet's choice writes only the `tips` line in `settings.toml`; deleting `settings.toml` does not bring the sheet back.
- [ ] Settings ▸ Behaviour ▸ Tips: Auto/Off changes at once, writes only the `tips` line, comments survive; an external edit updates the control; a broken file disables it.
- [ ] `tips = "off"`: no tip after a long-folder scroll; `hints` still show. `hints = "off"`: no tips and no hints.
- [ ] Tip card at 640 px and wide, light and dark: text wraps, last row stays above the card.
- [ ] VoiceOver: the onboarding sheet's title and both buttons are spoken; the Tips control is a radio group with its label and description.
- [ ] If Q4 is yes: "Don't show tips" writes `tips = "off"`, closes the card, and the next tip does not appear.

## Found along the way
- None yet.

## Log
- 2026-11-17: Read AGENTS.md, the M8 index, the F5 plan, `crow-core/src/tips.rs`, `crow-app/src/tips.rs`, `settings.rs`, `settings_window.rs`, `ui.rs`, and the window's sheet/tip code. F1 already landed item 4. Created the branch and this file. Asked Q3 and Q4; item 1 does not depend on either.

## Next
Implement item 1: `TipsMode` + `Settings.tips` + `tips_muted()` in `crow-core`, the Settings row, the write path, and tests.
