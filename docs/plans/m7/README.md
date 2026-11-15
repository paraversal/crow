# M7 — Hardening (index)

Phase 1's last milestone. M6 and the UI overhaul made every MVP feature work and visible. M7 makes Crow hard to break before it counts as the daily driver: random input across overlays, focus contexts and views; the file system changing underneath; awkward names and huge folders; nothing on the main thread waiting on the disk. It also takes the few items earlier plans deferred to M7.

Drafted 2026-11-15 from: `docs/plans/after-ui-overhaul.md` D1 (the user's fuzzing note, 2026-11-14), the deferrals in finished plans (`done/u11-finish.progress.md` I9, `done/u3-top-band.md` Q6, `done/u5-bottom-bar.md`, `done/u7-fix.md` X7, `done/context-menu.progress.md`), and a scan of `main` @ 56db832.

## The pieces
Each piece is self-contained: its own branch from `main`, progress file next to it (`docs/plans/m7/<piece>.progress.md`), PR, and critique. Carry each out with the `execute-plan` skill; it ends at "Ready for critique", and a second agent runs the `critique` skill and opens the PR. Start a piece only when the pieces it needs are **merged to `main`** (ASK before stacking on an unmerged branch).

| Piece | What | Needs | Branch |
| --- | --- | --- | --- |
| [H1](h1-fuzzer.md) | Seeded action fuzzer in the GPUI test platform: side-effect stubs, action generator from the live keymap, invariants, shrinking, replay. Covers the browser windows (the Settings window moved to H7, H1 critique Q11) | — | `m7-h1-fuzzer` |
| [H2](h2-edge-trees-and-churn.md) | Edge-case trees (long, NFD/NFC, emoji/RTL, deep, loops, unreadable, 100k) and a churn thread changing the shown folder while the fuzzer acts | H1 | `m7-h2-churn` |
| [H3](h3-slowness-finder.md) | `crow --fuzz`: the same generator in the real app with a frame-time budget; slow steps reported with their seed and step | H1 | `m7-h3-slowness` |
| [H4](h4-findings.md) | The living fix list: every failure H1–H3 find, each fixed with a named regression test | H1 (grows with H2, H3) | one branch per batch, `m7-h4-<n>` |
| [H5](h5-main-thread-io.md) | No disk I/O on the main thread: fix the remaining `stat` calls, add a debug-build guard | — | `m7-h5-main-thread-io` |
| [H6](h6-editable-path-bar.md) | Editable path bar in the bottom bar (UI overhaul Q10) | — | `m7-h6-path-bar` |
| [H7](h7-accessibility.md) | Real disabled state for VoiceOver (GPUI patch for AccessKit `Disabled`), fuzzer a11y invariant, VoiceOver pass. Also drives the Settings window under the fuzzer (from H1) | H1 for the invariant | `m7-h7-a11y` |
| [H8](h8-leftovers.md) | Small leftovers from earlier critiques (hint text, rounding, palette `~`, shortcut mismatch) | — | `m7-h8-leftovers` |

**Before M7:** M6.5, the settings window ([`../m6.5-settings.md`](../m6.5-settings.md)), lands first; every M7 piece branches from a `main` that has it. H7 treats the Settings window as a surface to cover, in the fuzzer and in the VoiceOver pass.

**Suggested order:** H1 → H2 → H4 (first batch) → H3 → H4 (second batch) → H5 → H7 → H6 → H8.
H5, H6 and H8 don't need the fuzzer and can run any time, one at a time (they all touch `window.rs`/`pane.rs`; never two writers at once). H1 comes first because its stubs and invariants make every later piece safer to test.

**M7 is done when:** a 30-minute fuzz run (H1 + H2 churn, all edge trees) finds nothing new on three different seeds; `crow --fuzz` reports no frame over 50 ms on the 100k fixture; the findings list has no open item marked "must fix"; H5–H8 are merged; README's status line says Phase 1 is done.

## Shared names and who builds them
Later pieces use these names. Whichever piece lands first builds the shared part with this shape; if merged code differs, use the merged name and record the mapping in the progress file.

| Part | Name | Built by |
| --- | --- | --- |
| Side-effect seam (open, reveal, Quick Look, pasteboard, sound, native menus, notifications) | `crow_app::effects::{Effects, SystemEffects, RecordingEffects}`, a GPUI global | H1 |
| Fuzz steps and generator | `crow_app::fuzz::{Step, StepGen, Script}`; `Script` prints as replayable Rust (`script![...]`) | H1 |
| Invariants | `CrowWindow::check_invariants(&self, cx) -> Vec<Violation>` (`#[cfg(any(test, feature = "fuzz"))]`) | H1 |
| Settle | `fuzz::settle(&view, cx)`: run until parked, then wait for watchers and listings to quiet (bounded) | H1 |
| Edge trees | `crow_fs::testing::{EdgeTree, make_tree}` behind the `testing` feature (also used by `--make-fixture`) | H2 |
| Churn | `crow_fs::testing::Churn::start(dir, seed) -> ChurnHandle` | H2 |
| Findings list | [`findings.md`](findings.md) (table; H1–H3 append, H4 fixes) | this plan (empty) |

## Not in M7 (and where it goes)
- Icon packs, public Rhai plugin API, git plugin / Projects: Phase 2 (spec "MVP scope", "Later").
- Folder sizes: already planned, `docs/plans/folder-sizes.md` (D2); after M7 unless the user says otherwise.
- Context menu for search results and the sidebar (`done/context-menu.progress.md` Q3): a feature; separate plan if wanted.
- Splitting `window.rs` (9.4k lines, about 6k of tests; palette view still inside, U7 Q13): ask in H1 Q5 whether the fuzzer's module is the moment to move tests out.
- Solarized Light contrast (`done/u11-finish.progress.md` I9): the theme index's palette, not Crow code.

## Questions
Each piece ends with "Questions to ASK", each with a recommended answer. The implementing agent asks them in one batch when it starts that piece (per `execute-plan`). Don't answer them all up front. Index-level questions for the user, asked by whoever starts H1:
1. **Is the order above right, or should H6 (path bar, user-visible) come earlier?** Recommended: as above; the path bar benefits from the fuzzer finding focus bugs around it.
2. **Nightly fuzz in CI?** The repo is public, so macOS minutes are free. Recommended: yes, a scheduled workflow from H1 on (`.github/workflows/fuzz.yml`, 20 minutes, a new seed each night, failing seed in the job summary).
