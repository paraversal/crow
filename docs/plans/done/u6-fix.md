# U6 fix — issues from reviewing the search panel

Audience: an implementing agent. Follow `.claude/skills/execute-plan/SKILL.md`; read `AGENTS.md`, `docs/plans/ui-overhaul/README.md` and `docs/plans/ui-overhaul/u6-search-panel.md` (the plan this fixes) first.
Base: branch `ui-u6-search-panel` (PR https://github.com/paraversal/file-crow/pull/14, not merged). **Fix on that branch** and push to the same PR, unless the user says otherwise (ASK if PR #14 has been merged meanwhile; then branch `ui-u6-fix` from `main`).
Progress file: `docs/plans/ui-overhaul/u6-fix.progress.md`.
Design reference: canvas boards "Search panel" B and D and "Overhaul, search mode" (https://claude.ai/artifact/EbZ7gFCy24MaccdpbSsFtE); local copies in `docs/plans/ui-overhaul/img/` (`search-panel-b.png`, `search-panel-d.png`, `assembled-search.png`). The boards are a guide, not doctrine: where the build improved on them, keep it.

## How this list was made (2026-11-13)
Reviewed the U6 diff (`git diff main...ui-u6-search-panel`), ran `cargo fmt --check`, clippy `-D warnings` and `cargo test --workspace` (all green: 190 app, 57 core, 27 search, 11 fs, 4 macos, 6 theme), built `./scripts/bundle.sh`, opened `target/bundle/Crow.app` on the repo folder and searched `m5` and `config` by hand. Two engine facts were confirmed with throwaway probe tests (since removed):
- nucleo `Utf32Str::new` (`~/.cargo/registry/src/*/nucleo-matcher-0.3.1/src/utf32_str.rs:44–55`) indexes by **grapheme** (first char of each grapheme cluster, feature `unicode-segmentation` is on), and when every grapheme's first char is ASCII it falls back to `Ascii(str.as_bytes())` and indexes **bytes**. Probe: `FuzzyMatcher::new("final").indices("Re\u{301}sume\u{301} final.txt")` returned `[13..18]` = `"nal.t"`.
- `grep_regex::RegexMatcherBuilder::new().build("(")` displays as `regex parse error:\n    (?:()\n    ^\nerror: unclosed group` (four lines, and the pattern shown is the engine's rewrite, not what the user typed).

Not checked by hand (computer-use couldn't press the scope radio buttons in the background): This Mac scope UI, narrow window (≤ 760 / < 900 px), light theme, VoiceOver. These stay on the U6 hand-check list.

What already works and must stay: idle body with four examples and live key line; scope bar with folder-named segment and the Skip switch (AX reads it as a switch); footer counts "n results in m files"; skipped link opening the inline list with the rule per folder; Esc closing the skipped list first; footer "Search everything" / "Skip folders again" flipping the switch; 300 px preview with numbered excerpt and tinted hit line; name matches highlighted; content line under the name; `.` for files at the scope root.

## Items

### F1 Selection stays on the first-arrived row (bug, high)
**Observed:** search `config` in the repo: the selected row (inactive tint) was `Dockerfile-collab`, the 5th row, a content-only hit. The preview showed it and Return would open it, while the top row was `configuration.md`.
**Cause:** `add_hits` → `follow_selection` (`crates/crow-app/src/search.rs:498–511`) selects the first row of the **first batch**. Later batches insert better name rows above it (`insert_rows`, 471–494), and `follow_selection` keeps the cursor on the same *path*, so the selection drifts down. The scroll position doesn't follow either.
**Change:** add `user_moved: bool` to `SearchView` (reset to false in `start_search`; set true in `select_only`, `press`, `extend_to`, `select_all`, and the row double-click handler, i.e. anything the user does to the selection). In `follow_selection`: when `!user_moved`, always `select_only(results[0])`, set `cursor = 0` (and keep scroll at top: `self.scroll.scroll_to_item(0, ScrollStrategy::Top)` only if the cursor moved). When `user_moved`, keep today's behavior (follow the path). `add_hits`/`add_spotlight` already call `load_preview` when the cursor path changes; keep that.
**Edge cases:** user arrows down during streaming → `user_moved` → rows inserted above shift the selected row down; acceptable (Finder does the same), but scroll so the cursor stays visible: `scroll_to_item(cursor, ScrollStrategy::Nearest)` if GPUI has it (check `ScrollStrategy` variants in `vendor/zed/crates/gpui/src/elements/uniform_list.rs`), else leave the scroll alone. Focus in the field (not the list) with Return → `open_selected` uses `self.cursor`, which now is 0 until the user moves; that is the point.
**Tests:** in `window.rs`, extend or add next to `search_name_hits_outrank_content_hits` (`window.rs:3416`): fixture where a content-only hit is walked **before** a name hit (e.g. `a/notes.txt` containing "zebra" and `b/zebra.txt`; check walk order in `crates/crow-search/src/engine.rs` — the walker is `ignore::WalkBuilder`, sorted? If not deterministic, use many content files in a first-walked folder so the first batch is content-only, or call `add_hits` directly through a `#[cfg(test)]` hook with two hand-made batches; prefer the hook for determinism). Assert after done: `cursor() == 0`, `selected_entry()` path is the name hit, `preview_path()` is the name hit (`wait_for_preview`). Second test: `tab`, `down` during/after → cursor follows the user's row after another batch arrives.
**Done when:** after any search finishes without user input, the top row is selected and previewed.

### F2 Modified column wraps to two lines (bug, high, visible in every screenshot)
**Observed:** "12. November / 2026 at 21:35" wraps in the 100 px Modified column; rows look like two-line rows everywhere. The board shows "Today 14:32", "Yesterday", "4. Nov".
**Cause:** `style::format_time` (`crates/crow-app/src/style.rs:92–100`) returns `"%-d. %B %Y at %H:%M"` for anything not today. It is also used by the main list's Date Modified column and the preview, so **don't change it**.
**Change:** add `pub fn format_time_short(t: Option<SystemTime>) -> String` in `style.rs`: today → `"Today %H:%M"`; yesterday → `"Yesterday"`; same year → `"%-d. %b"` ("4. Nov"); older → `"%-d. %b %Y"` ("4. Nov 2024"); None → "—". Make it testable with a `now` parameter: `format_time_short_at(t, now)` + wrapper. Use it in `render_rows` (search.rs:1699). Also add `.whitespace_nowrap().overflow_hidden().text_ellipsis()` to the Modified cell so nothing ever wraps. Keep the row's aria label (`Row::label`, 1840) on the long `format_time` (it reads better spoken).
**ASK the user** whether 100 px still fits "4. Nov 2024" at 12 px (it should, ~75 px); if not, widen to 110.
**Tests:** unit `format_time_short_buckets` in `style.rs` (today, yesterday, this year, last year, None).
**Done when:** every search row's Modified fits one line; hand-check in the bundle.

### F3 "1 folders skipped" (bug, low)
**Where:** `render_footer` (`search.rs:1185` aria, `1195` text) and `render_empty` (1443, 1451, "k folders were skipped").
**Change:** `1 folder skipped` / `n folders skipped`; aria `1 folder skipped, show list`; empty state `1 folder was skipped.` / `n folders were skipped.`. Put the wording in a small pure fn (`skipped_text(n) -> String`) and unit-test it with 1 and 3.
**Done when:** singular reads correctly in the footer, the aria label and the empty state.

### F4 Regex errors are four lines and show the engine's rewrite (bug, medium)
**Where:** `crates/crow-search/src/engine.rs:264` and `279` (`format!("invalid pattern: {e}")`); `search.rs` `render_error` (1458–1481, checks `starts_with("invalid pattern")`), `footer_summary` (1804), app test `search_invalid_regex_shows_the_error` (`window.rs:3479`, asserts `starts_with("invalid pattern")`).
**Change:** in the engine, build a one-line message: take the last non-empty line of `e.to_string()`, strip a leading `"error: "`, and produce `"Invalid pattern: unclosed group"`. Put that in a helper `fn pattern_error(e: impl Display) -> String` used by both call sites. Update `render_error`'s check to `starts_with("Invalid pattern")` and the app test. Also guard the UI: the footer text cell gets `.whitespace_nowrap().overflow_hidden().text_ellipsis().min_w_0()` and the error string is passed through `.lines().next()` for the footer only, so any future multi-line error stays on one line there (the body may show the full text).
**Tests:** engine unit `pattern_errors_are_one_line` (`(` → `"Invalid pattern: unclosed group"`, no `\n`); update the app test to assert the footer equals `"Invalid pattern: unclosed group"` and contains no newline.
**Done when:** `/(/` shows one red line in the footer and "Can't search: Invalid pattern: unclosed group" plus the slashes hint in the body.

### F5 Name highlights misplaced on decomposed (NFD) names (bug, medium)
**Where:** `FuzzyMatcher::indices` (`crates/crow-search/src/engine.rs:91–109`); test `name_hits_carry_match_ranges` (≈ line 490).
**Cause:** the code maps nucleo indices through `haystack.char_indices()`, but nucleo indexes graphemes (or raw bytes in its all-ASCII-first-char fallback; see "How this list was made").
**Change:** build the offset table the way nucleo builds its haystack. Mirror `Utf32Str::new`: if `haystack.is_ascii()` → byte offsets (index = byte). Else compute grapheme starts with `unicode_segmentation::UnicodeSegmentation::grapheme_indices(haystack, true)` (add `unicode-segmentation` to `crates/crow-search/Cargo.toml`; it's already in the lock via nucleo); if every grapheme's first char is ASCII, nucleo used **bytes** of the original string: then an index is a byte offset into `haystack`, and the range must be widened to the enclosing grapheme (so a hit on `e` covers `e\u{301}`). Otherwise index *i* = grapheme *i*, range = that grapheme's byte span. Merge adjacent ranges as today. Check `u7-palette.md` I1: U7 needs the same conversion; if `crow-app` already has a helper for it (grep `grapheme` in `crates/`), share one function in `crow-search` and reuse it rather than writing two.
**Edge cases:** the all-ASCII-first-char byte fallback means nucleo may match a needle char against a combining-mark byte? No: combining marks are non-ASCII bytes ≥ 0x80 and the needle is matched case-folded ASCII, so only ASCII bytes match; widening to the grapheme is enough. Ranges must stay on char boundaries (assert in the test).
**Tests:** extend `name_hits_carry_match_ranges`: NFD `"Re\u{301}sume\u{301} final.txt"` with "final" → matched text is `"final"`; NFD `"re\u{301}sume\u{301}.txt"` with "sum" → `"sum"`; a mixed name with a non-ASCII-first grapheme (e.g. `"日本 notes.txt"`, "notes") → `"notes"`. Keep the precomposed case.
**Bench:** `--bench-search target/fixture-search-200k` vs current branch numbers in `u6-search-panel.progress.md` (3.7–4.4 s); only matched names pay the cost.
**Done when:** highlights cover the typed letters for NFC, NFD and CJK names.

### F6 `insert_rows` rebuilds everything per batch (perf, measure first)
**Where:** `insert_rows` (`search.rs:471–494`), `add_spotlight` (513–542); engine `BATCH = 64` (`engine.rs:21`).
**Problem:** each 64-hit batch drains and re-collects `results` and rebuilds the whole `by_path` map (cloning every `PathBuf`). Up to `MAX_RESULTS = 20_000` rows that is ~300 batches × up to 20k clones on the main thread, i.e. O(n²).
**First measure:** search `e` in Home with Skip dev on (many hits) in the release bundle; log the time spent in `add_hits` per batch (temporary `std::time::Instant` + `eprintln!`, removed afterwards) and note the worst batch and total in the progress file. If the worst batch is < 2 ms, write that down and **skip the change** (ASK the user if unsure).
**Change if needed:** content-only rows are appended (no index change for existing rows) → only insert their new paths into `by_path`. Name rows: insert each new name row at its sorted position in the name block with `binary_search_by` on score (descending, stable: after equal scores), then only re-index from the first insertion point to the end (`for i in first..len`). Keep `name_count`. `add_spotlight`: only index the appended rows. Don't touch the walker.
**Tests:** existing `search_name_hits_outrank_content_hits` and F1's tests must stay green; add a unit test on a pure helper (`merge_name_rows(existing_scores, new_scores) -> order`) if you extract one.
**Done when:** numbers recorded; if changed, the per-batch cost no longer grows with the result count.

### F7 Small polish (do all; each is a few lines)
1. **Idle example keycap**: example 3 renders three separate keycaps (⇧ ⌘ A) inside the 150 px example button, a box inside a box, while the footer shows one ⇧⌘A keycap. Use `KeycapWeight::Text` (no border) for the example face so it reads like the other monospace examples; keep the live-keymap lookup (`Keycap::for_action`, `crates/crow-app/src/chrome.rs:54`). Check the board D look: plain `⌘⇧A` text in the button.
2. **Example button text alignment**: board D left-aligns the example text in the button (`px 12`); the build centers it (`justify_center`, search.rs:1383). Left-align to match the board.
3. **Idle heading follows the match mode**: All → "Search names and contents in <scope>", Names → "Search names in <scope>", Contents → "Search contents in <scope>", This Mac → "Search names on this Mac" (unchanged). `render_idle` (1281–1289) uses `self.effective_match()`.
4. **Hide example 3 when no exclusion set is enabled** (`skip_label.is_none()`): it would flip an invisible flag. Same rule the footer hint already uses (1197).
5. **Folder label cap**: the folder segment should truncate with an ellipsis beyond 160 px (U6 plan Target §1; not done): `.max_w(px(160.)).overflow_hidden().text_ellipsis()` on the label child in `scope_segment` (1123–1157). The aria label keeps the full name.
6. **`content_excerpt` once per row**: it runs on every render for every visible row (1563). Compute it in `Row::new` and store the shown text and shifted ranges in `Row.content` (or a new field `content_shown`), as the U6 plan I5 asked. `Row::label` keeps the full line text.
7. **Bottom bar says "0 results" in the idle state** (U5's `search_count`, `crates/crow-app/src/bottom_bar.rs:148–160`). When the query is empty, show nothing in the count slot (or the scope, e.g. "file-crow"); ASK the user which. The caller needs to pass whether the query is empty (find where `search_count` is called in `window.rs`). Add a unit test in `bottom_bar.rs`.

### F8 Design questions (ASK, don't decide)
Put these to the user with the recommendation first; implement only what they pick.
1. **Content line on name-match rows.** Today a file that matches by name *and* contents shows the content line too (e.g. `m5-fix.md` / `1: # M5-fix — issues…`). The assembled board keeps name hits to one line, and the user asked that name matches stand out more. Options: (a, recommended) name-match rows show only the name, vertically centered; content line only on contents-only rows; (b) keep as is.
2. **Weak fuzzy name hits outrank strong content hits.** With "names first", `gpui-stop-synthetic-drag-on-file-drag.patch` (c…o…n…f…i…g scattered) ranks above real `config` content hits. Options: (a, recommended) only names with a **contiguous** case-insensitive substring match (or fuzzy score ≥ a threshold; read the nucleo score scale first) go in the name block; weaker fuzzy name hits go after the content rows, or are dropped; (b) keep pure fuzzy. If (a), say whether weak hits go after content rows or are dropped.
3. **Content line format.** The board shows `4  pub fn …` (number, two spaces, no colon); the build shows `4: pub fn …`. Recommended: match the board (no colon, number right-aligned in a fixed 28 px column so lines align).
4. **Skipped list paths.** The list shows absolute paths (`/Users/me/Projects/git/file-crow/target`). Recommended: same rule as the Folder column (relative to the scope root for the folder scope, `~` for Home).

## Finish
- `cargo fmt --check` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo test --workspace`.
- `./scripts/bundle.sh`, open `target/bundle/Crow.app` on the repo folder, search `config` and `m5`: the top row selected and previewed, dates on one line, "1 folder skipped ›", `/(/` one line. Screenshot the result next to `../ui-overhaul/img/assembled-search.png` for the PR.
- Update `docs/testing.md` hand rows if wording changed (skipped singular, short dates); README only if behavior described there changed.
- Progress file: each item's state, measurements (F5 bench, F6 timings), user answers to F8 and F7.7.
- Push to `ui-u6-search-panel` (PR #14) with a comment listing the fixes; don't merge. U6 hand checks (light theme, VoiceOver, narrow window, This Mac) remain for the user.
