# F5 — Tips and first launch

Audience: an implementing agent. Follow the `execute-plan` skill; read `AGENTS.md` and the M8 index first.
Base: branch `m8-f5-tips` from `main`. Progress file: `docs/plans/m8/f5-tips-and-first-launch.progress.md`.
PR title: "M8 F5: tips setting and first-launch choice".

## Why

The contextual tips (U9) are useful once, but the only way to mute them is `hints = "off"`, which also hides every bottom-bar key hint. The user wants a "shut up" setting that turns tips off completely, offered in a popup on the first launch so it is discoverable (user, 2026-11-17). The tip card also covers the list's last rows (U9 deferral, F1.6).

## Current state

- **Hints and tips are coupled.** `DEFAULT_SETTINGS` (`crow-core/src/settings.rs:12`) documents `hints = "auto" | "always" | "off"`; the comment says `"off" hides every hint and tip` (`:25`). `TipBook::may_show(..., hints_off)` (`crow-core/src/tips.rs:113`) refuses a tip when hints are off; `tips::wants`/`request` (`crow-app/src/tips.rs:47`, `:58`) pass that flag.
- **The tip card.** `tips.rs:157` `render_tip`: keycap, one sentence, "Got it"; `absolute().right(12).bottom(bottom_bar::HEIGHT + 8)` (`:179`), so it covers the bottom-right rows. The card is transient and retires the tip on show (`TipBook::show`, `tips.rs:122`).
- **State.** `tips.json` in the state dir (`TIPS_FILE`, `crow-core/src/tips.rs:14`); `TipState::load/save`; `tips.rs:35` loads it at startup. No first-launch popup exists.
- **Settings window.** `ControlId::Hints` (`settings_window.rs:121`), label "Key hints" (`:221`), description at `:268`, segmented control at `:1134-1141`. A new `Tips` control follows the same pattern.
- **First launch.** `Ui::load_settings` (`ui.rs:152`) creates `settings.toml` with `DEFAULT_SETTINGS` when it is missing (`settings.rs:285`). There is no onboarding state; `~/.local/state/crow/` holds `recent.json`, `palette-commands.json`, `usage.json`, `tips.json`.

## Items

### 1 A separate `tips` setting (s/ui)

**Change:** add `tips = "auto" | "off"` to `settings.toml`, documented default `"auto"`. `"off"` mutes every tip; `hints` stays independent, so a user can keep key hints and mute tips (or vice versa). `hints = "off"` keeps muting tips as today (the existing coupling stays as a fallback). Add `crow_core::TipsMode` (like `HintsMode`), `Settings.tips`, parsing tests, and a `ControlId::Tips` segmented control under Behaviour in the Settings window, with a one-line description. Live reload; `edit_toml` keeps comments; the default file parses to `Settings::default()`.

**Tests:** parse `"auto"`/`"off"`; a missing line is Auto; a bad value is the broken-file case; `tips = "off"` refuses `TipBook::may_show`; `hints = "off"` still refuses; the settings window writes only the `tips` line; a reload does not flip the control.

### 2 First-launch popup (ui)

**Change:** on the first launch, show a small sheet (not a separate window) asking whether to show tips: **"Show tips as you go?"** with **Show tips** (default) and **No tips**. The choice writes `tips`; the sheet never shows again. Use a state file, e.g. `~/.local/state/crow/onboarding.json` with `{"tips_choice": true}` (or a field in `tips.json` if that is cleaner), so deleting `settings.toml` does not re-show it and a user who chose "No tips" is not asked again. The sheet must be accessible (role, labels, Return/Escape), must not block the window for more than the choice, and must not appear in tests unless the test asks for it.

**Tests:** a fresh state dir shows the sheet once; choosing either option writes the setting and closes it; a second launch does not show it; a broken onboarding file shows it again (state, not config; no status-line error); Escape keeps the default and closes it; the sheet has a role and labels; the fuzzer's settings-window smoke does not hang on it.

**Copy:** "Show tips as you go?" / "Crow can point out a few keys the first time they matter. You can change this later in Settings." / buttons "Show tips" and "No tips". Keep it to one sentence; principle 9 (show, don't tell) allows this one-time choice.

### 3 "Shut up" from the tip card (ui)

**Change:** optional, ASK. If wanted, add a small "Don't show tips" link/button to the tip card (next to "Got it") that sets `tips = "off"` and closes the card. Do not add a permanent menu item just for this; the Settings window row and the first-launch popup are the discoverable paths.

**Tests:** the button sets the setting and closes the card; the next tip does not appear; the card's aria label names the action.

### 4 Tip position (ui)

**Change:** coordinate with F1.6. The tip must not cover the last rows. Recommended: move the card to the bottom-left over the sidebar if it fits, or reserve a bottom inset in the list while a tip is visible. No permanent chrome; the card stays transient.

**Tests:** a layout test asserts the list's last row is not under the card (or the card is outside the list); hand check at 640 px and wide, light/dark.

## Questions to ASK

1. **Setting name and values:** `tips = "auto" | "off"` (recommended, matches `hints`) or a boolean `show_tips = true`? The user chose a separate setting on 2026-11-17.
2. **Does "No tips" also turn off key hints?** Recommended: no; hints stay independent. The first-launch copy should say so only if it is short.
3. **First-launch detection:** a state file `onboarding.json` (recommended; deleting config does not re-ask) or the absence of `settings.toml` (simpler, but a user who deletes config gets the popup again)?
4. **"Shut up" on the tip card:** add it (recommended: a small link next to "Got it") or keep the setting only in the first-launch popup and Settings?
5. **Tip position:** move to the bottom-left over the sidebar (recommended if it fits) or reserve a bottom inset in the list?

## Finish

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- Bench not needed unless the tip inset changes the list layout; if it does, compare with `main`.
- `./scripts/bundle.sh`; hand-check a fresh state dir (popup once), both choices, the Settings row, a tip, the "shut up" path, light/dark, 640 px, VoiceOver.
- Update README's settings table and `docs/testing.md`.
- Commit, push, "Ready for critique". Don't merge.
