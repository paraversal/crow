# M7 H4 — Fix what the fuzzer finds

Audience: an implementing agent. Follow the `execute-plan` skill; read `AGENTS.md`, `docs/plans/done/m7/README.md` and `docs/plans/done/m7/findings.md` first.
Base: one branch per batch, `m7-h4-<n>` from `main`, **after the piece that found the items is merged** (H1, H2 or H3). Progress file per batch: `docs/plans/done/m7/h4-<n>.progress.md`.
PR title: "M7 H4 batch <n>: <short list>". Don't merge.

## Why
H1–H3 only record findings; this is where they get fixed, in reviewable batches, so a fuzzer PR never doubles as a pile of unrelated fixes.

## How a batch works
1. **Pick the batch.** Read `findings.md`. ASK the user which items go in this batch, proposing: every item marked "must fix" (a panic, lost data, a stuck focus or overlay, a wrong listing after settle, a freeze over 250 ms), then related items that touch the same code, up to about 6 items or one area. Mark the rest "later" or "won't fix" only with the user's answer.
2. **For each item:**
   - Turn the shrunk script into a named regression test in the right test module (`fuzz::replay(...)` or a plain keystroke test if shorter), named for the behavior, e.g. `escape_in_rename_inside_palette_returns_focus_to_list`. It must fail on `main`.
   - Find the cause; write it in the progress file with evidence (file:line, what state was wrong). For slow steps from H3: profile first (`xctrace`), as in `done/u7-fix.md` X6.
   - Fix it. Keep the fix to the cause; if the right fix is larger than the batch (a new model, a redesign of an overlay), ASK whether to split it out.
   - The test passes; `fuzz_smoke` and the item's original seed pass.
3. **Mark it** fixed in `findings.md` with the commit and test name.

## Rules for fixes
- File operations still go only through `crow_fs::ops`; no disk I/O on the main thread; theme roles only (AGENTS.md).
- A fix that changes visible behavior (what's selected after a delete, where focus lands after an overlay closes) is a UX decision: ASK, with a recommendation based on what Finder does and what the spec principles say.
- Don't weaken an invariant to make a finding go away. If an invariant was wrong, say so in the progress file and the PR body, with the user's agreement.

## Finish (per batch)
- fmt · clippy · tests; `fuzz_long` 10 minutes on the batch's seeds and one new seed, recorded.
- Bench vs `main` if a listing, row or scroll path changed.
- `./scripts/bundle.sh`; hand-check each fix in the app the way the shrunk script reached it.
- Commit, push, "Ready for critique".

## Questions to ASK (each batch)
1. Which findings are in this batch? (Proposal as in step 1.)
2. For each behavior-changing fix: the proposed behavior, with Finder's behavior as the recommended default.
