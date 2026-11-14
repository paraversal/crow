# U6 fix progress
Branch: `ui-u6-fix` · PR: none yet · Last updated: 2026-11-14

## Status
| Item | State | Notes |
| --- | --- | --- |
| F1 Selection on the first-arrived row | todo | |
| F2 Modified column wraps | todo | |
| F3 "1 folders skipped" | todo | |
| F4 Regex errors four lines | todo | |
| F5 NFD name highlights | todo | |
| F6 `insert_rows` per-batch rebuild | todo (measure first) | |
| F7.1 Idle example keycap | todo | |
| F7.2 Example button alignment | todo | |
| F7.3 Idle heading follows match mode | todo | |
| F7.4 Hide example 3 without an enabled set | todo | |
| F7.5 Folder label cap | todo | |
| F7.6 `content_excerpt` once per row | todo | |
| F7.7 Bottom bar "0 results" in idle | todo (Q3) | |
| F8.1 Content line on name rows | todo (Q4) | |
| F8.2 Weak fuzzy name hits | todo (Q5) | |
| F8.3 Content line format | todo (Q6) | |
| F8.4 Skipped list paths | todo (Q7) | |

## Questions
- Q1 (asked 2026-11-14): Modified column width. Measured "4. Nov 2024" at 12 px SF system font = 69.5 px (`osascript` AppKit bridge, `sizeWithAttributes`), so 100 px fits with room. Keep 100 px or widen to 110? → pending
- Q2 (asked 2026-11-14): content line on name-match rows (F8.1). → pending
- Q3 (asked 2026-11-14): bottom-bar count in idle (F7.7). → pending
- Q4 (asked 2026-11-14): weak fuzzy name hits vs content hits (F8.2). → pending
- Q5 (asked 2026-11-14): content line format (F8.3). → pending
- Q6 (asked 2026-11-14): skipped list paths (F8.4). → pending

## Decisions
- PR #14 was already merged to `main` when this list started, so the fix branches `ui-u6-fix` from updated `main` (plan's fallback) instead of pushing to PR #14 — plan.
- `ScrollStrategy::Nearest` exists (`vendor/zed/crates/gpui/src/elements/uniform_list.rs:84-99`), so F1's user-moved case can scroll the cursor into view.

## Hand checks
- [ ] F2 search rows: every Modified value on one line; today shows "Today 14:32", yesterday "Yesterday", older "4. Nov"
- [ ] F3 one skipped folder reads "1 folder skipped ›" and VoiceOver says "1 folder skipped, show list"
- [ ] F4 `/(/`: one red footer line, body "Can't search: Invalid pattern: unclosed group" plus the slashes hint
- [ ] F7 idle: example 3 is plain `⌘⇧A` text (or the live chord), example text left-aligned, heading follows All/Names/Contents, folder segment ellipsizes past 160 px
- [ ] F8 checked against the chosen answers in both themes

## Found along the way
- (none yet)

## Log
- 2026-11-14: read the plan, U6 plan, U6 progress, `search.rs`, `engine.rs`, `style.rs`, `bottom_bar.rs`, `chrome.rs`, window search tests. PR #14 merged; branched `ui-u6-fix` from `origin/main` (`50a4d17`). Probed nucleo scores for "config": contiguous names 166/159, scattered `gpui-stop-synthetic-drag-on-file-drag.patch` 109.
- 2026-11-14: measured "4. Nov 2024" width for F2: 69.5 px at 12 px.

## Next
Ask Q1–Q6; then implement F1 (no open question).
