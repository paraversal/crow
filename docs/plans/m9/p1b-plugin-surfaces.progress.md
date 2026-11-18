# M9 P1B — Plugin contributions in the app progress

Branch: `codex/m9-p1b-plugin-surfaces` · PR: none · Last updated: 2026-11-18 · State: in progress
Base: `d5cab1e` (M9 P8, merged as #65). Dependencies merged: P1A `083d40c` (#61, includes P1A fix), P8 `d5cab1e` (#65).

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 Verify runtime seams and run native spikes | in progress | |
| 2 Complete the API-1 wire contract | todo | |
| 3 Build the app coordinator and hook delivery | todo | |
| 4 Wire native menu, command palette and context commands | todo | |
| 5 Render columns, decorations, status and Info | todo | |
| 6 Add the shared sidebar section shell | todo | |
| 7 Connect configured Preview providers and P8 read eligibility | todo | |
| 8 Ship the test fixture and author documentation | todo | |
| 9 Run the acceptance matrix and hand off | todo | |

## Questions

- none yet

## Decisions

- Base `d5cab1e`; branch `codex/m9-p1b-plugin-surfaces`. The user's uncommitted M8→done / M9 docs reorganization stays untouched and uncommitted.
- API stays experimental at `api = 1`; no new GPUI patch unless a spike proves one is required, and that needs the user's check-in first.

## Hand checks

- (to be filled as steps land)

## Found along the way

- (none yet)

## Log

- 2026-11-18 Read the plan, M9 execution contract, P1A/P1A-fix/P8 progress. Created the branch from `d5cab1e` and this progress file before code.

## Next

Step 1: verify the runtime seams in `crow-app`/`crow-plugin` and run the three native spikes.
