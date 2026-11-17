# F6 — Folder sizes progress
Branch: m8-f6-folder-sizes · PR: none · Last updated: 2026-11-17 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| S1 Spike: measure before choosing | in progress | timing the three walks and checking TCC/dataless behaviour |
| S2 The sizer (`crow-fs`) | todo | |
| S3 The cache | todo | |
| S4 FSEvents history (`crow-macos`) | todo | |
| S5 Scheduling (`crow-app`) | todo | |
| S6 Showing sizes | todo | |
| S7 Sorting by size | todo | |
| S8 Setting and command | todo | |
| S9 Docs, tests, bench | todo | |

## Questions
- Q1 (to ask after S1): logical or allocated size? Recommended logical.
- Q2 (to ask after S1): default `folder_sizes` off or visible? Recommended off.
- Q3 (to ask after S1): re-sort as sizes arrive, or once when visible sizes are in? Recommended once.
- Q4 (to ask after S1): cache in `~/Library/Caches/crow/` or the state dir? Recommended Caches.
- Q5 (to ask after S1): one FSEvents stream for the boot volume or one per measured folder? Answer from S1.
- Q6 (to ask after S1): network/removable volumes measured or always `--`? Recommended `--`.

## Decisions
- Branch `m8-f6-folder-sizes` from `main` @ cb49ae4 — M8 index.
- Stop at "Ready for critique" without pushing; the critic opens the PR — AGENTS.md (wins over the plan's Finish wording).

## Hand checks
- [ ] Home in list view with the setting on: sizes fill in top first, fade in, and match Finder's Get Info (logical) within rounding.
- [ ] Edit a file deep in a measured folder: that folder's size and its parents update within a couple of seconds. Quit Crow, change something, relaunch: the change shows.
- [ ] Scroll a 100k folder of folders quickly: no stutter, and no work continues for rows that scrolled away (Activity Monitor CPU).
- [ ] No permission prompt appears just from viewing `~`.
- [ ] An iCloud Drive folder with files not downloaded: they stay not downloaded.

## Found along the way
- (none yet)

## Log
- 2026-11-17: branch `m8-f6-folder-sizes` from `main` @ cb49ae4; read the plan, M8 index and F3's progress file. Starting S1.

## Next
S1: write the throwaway walk benchmark and collect the numbers.
