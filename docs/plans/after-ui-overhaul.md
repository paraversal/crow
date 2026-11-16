# After the UI overhaul

Work that is too big for a fix list and belongs after U11 (the finish pass), with or after M7 (hardening). This is an inbox, not a plan: before any item is built, it gets its own plan in `docs/plans/` (with questions to ASK), carried out with the `execute-plan` skill.

Items move here from fix lists when they turn out larger than the list can hold. Say where each one came from.

## D1 Robustness fuzzing for the UI (direction)
**From:** the user's notes, 2026-11-14: "fuzzing script for UI robustness? i.e. erratic input to see if anything unexpected happens, edge cases like long names or huge directories, see what slows it down (some of these could also be tackled with tests, like long file names)". The "tests" half went to `done/u11-finish.md` I4 (edge-case layout checks).
**Why:** each U plan tests its own keys and states. Nobody tests random sequences across overlays, focus contexts, views and the file system changing underneath. U7-fix X7 (a deleted file staying in the list, seen once) is the kind of bug this would find.
**Planned:** M7, [`m7/README.md`](m7/README.md) (H1 fuzzer, H2 edge trees and churn, H3 slowness finder, H4 fixes), 2026-11-15. The sketch below is kept for history.
**Shape to plan:**
- **Action fuzzer.** Random but reproducible (seeded) sequences of keystrokes and mouse actions drawn from the live keymap and menu registry, run in the GPUI test platform (`simulate_keystrokes`, `crate::ui::init_for_tests`) over generated temp trees. After every step, check invariants:
  - no panic;
  - the cursor and selection point at rows that exist;
  - exactly one focused element with a role;
  - an overlay open ⇒ focus is inside it;
  - after settling, the listing equals the disk.

  When an invariant fails, shrink the sequence (e.g. `proptest`) and print the seed.
- **File-system churn.** A background thread creates, renames and deletes files in the shown folder while the fuzzer acts, so watcher races (X7's H1) show up.
- **Generators for edge cases:** names at 255 bytes, NFD/NFC twins, emoji and RTL names, deep paths, 100k-entry folders, unreadable folders, symlink loops, a folder deleted while shown, slow or removable volumes.
- **Slowness finder:** a frame-time budget check during fuzzing (any frame over ~50 ms is reported with its step), next to the existing `--bench`. Ties in with U7-fix X6.
- **Where it runs:** locally and nightly, not on every CI push (it is long). Decide the dependency (`proptest` vs a small hand-rolled shrinker) in its plan.
**Questions for its plan:** test platform only, or also the real app driven via accessibility? How long per run in CI? Which invariants are must-haves for M7?

## D2 Folder sizes (planned)
**From:** the user, 2026-11-14: show folder sizes, maybe from an index updated as things change.
**Plan:** [`folder-sizes.md`](folder-sizes.md). The sizes are measured lazily for the folders on screen, cached, and kept correct with FSEvents history (a folder's mtime doesn't change when files deeper inside do). Counting rules borrowed from asimeow.

## D3 Housekeeping triage (2026-11-16)
**From:** the user's housekeeping list and the triage on 2026-11-16. The keyboard/menu items went to [`housekeeping-3.md`](housekeeping-3.md); these are the rest.
- **Copy/cut feedback.** Flash the copied rows once and optionally play a tick; dim cut rows until paste/undo. Needs a motion/sound event and clipboard state. The copy flash is small; cut dimming is the larger half.
- **Open Terminal Here.** `Effects::open_terminal(path)`, a context-menu line, and a decision on Terminal.app vs iTerm and whether it gets a key.
- **New File/Folder with a row selected.** Decide whether they always create in the shown folder (recommended) or inside a selected folder; then add them to the Items menu.
- **Fuzz/test job overview.** H3 already prints a per-run report and CI has a step summary; a cross-run dashboard is separate tooling. Clarify scope first.
- **Window drag.** Dragging on toolbar controls moves the window because AppKit's native titlebar drag is active (`WindowOptions::app_owns_titlebar_drag` is false). Likely fix: set it true and add an explicit drag strip, then hand-test traffic lights and double-click. Timebox; if it needs a vendor patch, it waits.
- **Sidebar background menu.** Dropped: row menus already exist and the user retracted the empty-background request.

## Moved here later
- (U7-fix X6 or X7, if they turn out larger than the fix list: add them here with their findings.)
