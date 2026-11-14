# U6 fix progress
Branch: `ui-u6-fix` · PR: https://github.com/paraversal/file-crow/pull/15 · Last updated: 2026-11-14

## Status
| Item | State | Notes |
| --- | --- | --- |
| F1 Selection on the first-arrived row | done (e389af0) | `user_moved`; two window tests (best row until the user moves, cursor follows the user's path) |
| F2 Modified column wraps | done (e389af0) | `format_time_short(_at)`, 110 px, no-wrap cell; bucket test | |
| F3 "1 folders skipped" | done (2a85632) | `skipped_text`/`skipped_was_text`, unit-tested for 1 and 3 |
| F4 Regex errors four lines | done (2a85632) | `pattern_error` in the engine, footer one-line guard; engine + app tests assert "Invalid pattern: unclosed group" |
| F5 NFD name highlights | done (c132d81) | mirrors `Utf32Str`: ASCII bytes, ASCII-first grapheme byte fallback (widened to the grapheme), else grapheme indices; `name_ranges_follow_nucleo_indexing` (NFC, NFD, CJK, combining) |
| F6 `insert_rows` per-batch rebuild | done (3b92330) | measured; content-only batches extend + index only new rows, strong names merge and re-index the moved suffix, Spotlight indexes appended rows. Release probe, 20k rows / 313 batches: `add_hits` worst 55 µs (insert 9 µs); before the change the same probe measured 7.0 ms worst wall (≈2.5 ms inside `add_hits`; the rest is the test harness repainting synchronously). Probe removed. | |
| F7.1 Idle example keycap | done (3b92330) | `KeycapWeight::Text`, `text_role("fg")` |
| F7.2 Example button alignment | done (3b92330) | left-aligned, `px 12` |
| F7.3 Idle heading follows match mode | done (3b92330) | All / Names / Contents headings; This Mac unchanged |
| F7.4 Hide example 3 without an enabled set | done (3b92330) | `.when(self.skip_label.is_some())` |
| F7.5 Folder label cap | done (3b92330) | 160 px max width, end ellipsis; aria keeps the full name |
| F7.6 `content_excerpt` once per row | done (3b92330) | `Row.content_shown` filled by `Row::set_content` in `Row::new` and on merges |
| F7.7 Bottom bar "0 results" in idle | done (3b92330) | `search_count(..., query_empty)` returns an empty count; the bar omits the slot; `search_count_hides_in_idle` | |
| F8.1 Content line on name rows | done (3b92330) | name rows render one centered line; visibility gated on `!row.name_match` | |
| F8.2 Weak fuzzy name hits | done (3b92330) | `Row.strong_name` from `name_has_query`; layout strong names \| content \| weak names; `search_contiguous_name_hits_come_before_contents_and_weak_names` | |
| F8.3 Content line format | done (3b92330) | 28 px right-aligned line number, no colon | |
| F8.4 Skipped list paths | done (3b92330) | `skipped_display` uses `strip_prefix(folder)` / `abbreviate_home` | |

## Questions
- Q1 (asked 2026-11-14): Modified column width. Measured "4. Nov 2024" at 12 px SF system font = 69.5 px. → answer: widen to 110 px
- Q2 (asked 2026-11-14): content line on name-match rows (F8.1). → answer: name-match rows show only the name (recommended option)
- Q3 (asked 2026-11-14): bottom-bar count in idle (F7.7). → answer: show nothing
- Q4 (asked 2026-11-14): weak fuzzy name hits vs content hits (F8.2). → answer: contiguous substring only in the name block; scattered fuzzy names after content rows, not dropped
- Q5 (asked 2026-11-14): content line format (F8.3). → answer: board format, no colon, right-aligned 28 px number
- Q6 (asked 2026-11-14): skipped list paths (F8.4). → answer: relative to the scope root; ~ for Home

## Decisions
- PR #14 was already merged to `main` when this list started, so the fix branches `ui-u6-fix` from updated `main` (plan's fallback) instead of pushing to PR #14 — plan.
- `ScrollStrategy::Nearest` exists (`vendor/zed/crates/gpui/src/elements/uniform_list.rs:84-99`), so F1's user-moved case can scroll the cursor into view.

## Hand checks
- [x] F2 search rows: every Modified value on one line; verified in the bundle screenshot (Yesterday / 12. Nov / Today 12:09). A light theme still needs a look.
- [x] F3 footer reads "1 folder skipped ›" in the bundle (screenshot). VoiceOver still needs a look.
- [ ] F8.3 content line: number right-aligned, no colon, 8 px gap before the text, indent trimmed. The first screenshot showed the number running into the text (`16use`, `7#`) and indented lines with wide gaps; fixed after review, not re-checked in the bundle yet.
- [x] F4 covered by `search_invalid_regex_shows_the_error` (error and footer equal the one-line text); the red body is not in the screenshot.
- [ ] F7 idle: not visible in the search screenshot; covered by `search_idle_*` tests for behaviour, the look still needs a manual pass.
- [x] F8.1/F8.2/F8.4 checked in the bundle screenshot: name rows one line, content rows after them with a right-aligned number, weak/fuzzy names below content rows; skipped path wording is singular. Light theme still needs a look.

## Found along the way
- Matching gap (not fixed, separate from this list): an accented query never matches an NFD name (`résumé` vs `Re\u{301}sume\u{301}.pdf`), because nucleo normalizes only the haystack and its all-ASCII-first-grapheme byte path can't match a non-ASCII needle char. `resume` does match. Fix would fold the needle in `FuzzyMatcher::new` (nucleo `chars::normalize` + NFD-strip) — ask before doing it.

## Log
- 2026-11-14: read the plan, U6 plan, U6 progress, `search.rs`, `engine.rs`, `style.rs`, `bottom_bar.rs`, `chrome.rs`, window search tests. PR #14 merged; branched `ui-u6-fix` from `origin/main` (`50a4d17`). Probed nucleo scores for "config": contiguous names 166/159, scattered `gpui-stop-synthetic-drag-on-file-drag.patch` 109.
- 2026-11-14: measured "4. Nov 2024" width for F2: 69.5 px at 12 px. Asked Q1-Q6; all answered (see Questions).
- 2026-11-14: F1 and F2 implemented; both new window tests plus `search_name_hits_outrank_content_hits` and `format_time_short_buckets` pass.
- 2026-11-14: F3 and F4 implemented; `skipped_text`, `footer_summary_copy`, `pattern_errors_are_one_line` and `search_invalid_regex_shows_the_error` pass.
- 2026-11-14: `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace` green (197 app, 57 core, 27 search, 4 macos, 13 search, 6 theme). README search paragraph and `docs/testing.md` (automated search sentence, Search, Skipped folders, Search idle, Search errors rows) updated.
- 2026-11-14: F6 measured and optimised; F7.1–F7.7 and F8.1–F8.4 implemented per the answers. All `search_*` window tests (37), `search::tests` (8) and `bottom_bar::tests` (5) pass.
- 2026-11-14: bundle check: built `target/bundle/Crow.app`, launched it on the repo folder with a temporary debug env hook that opens search with "config" (hook reverted), and screenshotted `../ui-overhaul/img/u6-fix-search.png`. Verified the top row selected and previewed, one-line name rows, content lines with right-aligned numbers, one-line Modified dates, "1 folder skipped ›" and the footer counts. The OS screen-recording prompt from Ghostty overlaps the middle of the capture; it is not Crow UI. Light theme, VoiceOver and the idle state still need a human pass.
- 2026-11-14: F5 implemented (`unicode-segmentation` added to crow-search). Bench `--bench-search target/fixture-search-200k`: first hit 5 ms, total 2491 ms, 200000 hits — no regression vs the 3.7–4.4 s in the U6 progress (names don't match "needle" in that fixture, so only the non-ASCII path is theoretically touched).

- 2026-11-14 (review fixes): F8.2 strong names now come from nucleo's own ranges (one merged range), with the case-insensitive substring as a fallback, so accent-folded (`resume` → `Résumé.pdf`) and NFD names rank in the name block; `contiguous_name_matches_are_strong` uses the real matcher. F8.3 content line: 8 px gap between number and text, leading indent trimmed in `content_excerpt` (ranges shifted; test extended). fmt, clippy, `cargo test --workspace` green (197 app, 57 core, 27 search, 4 macos, 13, 6 theme).

## Next
PR open; CI runs. The unchecked hand checks (light theme, VoiceOver, idle look, This Mac, narrow window) are for the user before the squash merge.
