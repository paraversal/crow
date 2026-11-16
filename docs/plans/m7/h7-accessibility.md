# M7 H7 — Accessibility hardening

Audience: an implementing agent. Follow the `execute-plan` skill; read `AGENTS.md` and `docs/plans/m7/README.md` first.
Base: branch `m7-h7-a11y` from `main`, **after H1** (for I2; ASK if H1 isn't merged, I1 and I3 can go first). Progress file: `docs/plans/m7/h7-accessibility.progress.md`.
PR title: "M7 H7: disabled state and VoiceOver pass". Don't merge.

## Why
AGENTS.md: every interactive element has a role and an aria label. Disabled controls fake it: GPUI has no `aria_disabled`, so U3 appended ", unavailable" to labels (`done/u3-top-band.md` Q6: "a GPUI patch exposing accesskit's `Disabled` can come in M7"). See `window.rs` ≈ 521 and `search.rs` ≈ 1097. No full VoiceOver pass has been recorded since the overhaul (U11 and the context-menu critique both list VoiceOver as "not looked at").

## Items
### I1 GPUI patch: disabled state
- In `vendor/zed/crates/gpui`, find where element roles and labels become AccessKit nodes (search `accesskit` / `aria_label`). Add `.aria_disabled(bool)` that sets the node's `Disabled` flag (AccessKit `Node::set_disabled`).
- Save it as `patches/gpui-aria-disabled.patch` (applied by `scripts/fetch-gpui.sh`, like the existing three). Keep it small so it rebases onto future Zed revisions.
- Replace ", unavailable" label suffixes with `.aria_disabled(true)`. `rg 'unavailable' crates/crow-app/src` lists them.
- Test: a GPUI test reading the accessibility tree (if the test platform exposes it; otherwise a unit test on the element builder) and a VoiceOver hand check: a disabled toolbar button reads "dimmed".
### I2 Fuzzer invariant
Add to H1's invariant 3: every element with an `on_click` or focus handle on screen has a role and a non-empty aria label. If GPUI doesn't let us enumerate them in tests, add a `#[cfg(feature = "fuzz")]` registry filled where roles are set. ASK if neither is practical.
### I2b Fuzz the Settings window (moved from H1, critique Q11)
H1 opens the Settings window with ⌘, but neither drives nor checks it. Extend the runner to drive it as a second surface: steps from its keymap (`settings_window::keymap`) and its controls' selectors, invariants per window (focus inside the window on a control with a role and label, the shown values equal `settings.toml` after a settle). Same caps and recorder as H1.
### I3 VoiceOver pass
With VoiceOver on, in the bundle: sidebar, list and column view, filter, search, palette, rename, conflict sheet, context menu, tips, bottom bar, path bar (H6 if merged). Record per area in the progress file: what VoiceOver says, what's wrong. Fix clear defects (missing/duplicate labels, wrong roles, focus not announced); bring anything larger to the user.

## Finish
fmt · clippy · tests; re-run `./scripts/fetch-gpui.sh` from clean (`rm -rf vendor/zed`) to prove the patch applies; CI green. Commit, push, "Ready for critique".

## Questions to ASK
1. **Patch GPUI, or keep the label suffix?** Recommended: patch; it's the correct semantics and small. Keep the suffix only if the AccessKit mapping in GPUI's macOS adapter doesn't expose `Disabled` (check before asking).
2. **Upstream the patch to Zed?** Recommended: offer it to the user as a follow-up; don't open an upstream PR from this piece.
