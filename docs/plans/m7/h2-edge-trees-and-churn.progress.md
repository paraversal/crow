# H2 edge trees and churn progress
Branch: `m7-h2-churn` · PR: none · Last updated: 2026-11-15 · State: Ready for critique

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 `crow_fs::testing` | done | `testing.rs`: `EdgeKind` (16 kinds), `EdgeTree` (`small`/`all`/`parse`/`label`), `make_tree`, `Tree::remove` (permission-restoring), `Churn`/`ChurnHandle`. `--make-fixture` now calls `EdgeTree::wide`; old `snapshot::make_fixture` removed. `crow-fs` 42 tests pass; clippy with and without `testing`; `cargo check -p crow-app`. |
| I2 Churn | done | `Step::DeleteShownFolder`/`ReplaceShownFolder` (2% each), `FuzzOptions` (files/edge/churn), churn pauses around every `Settle` deep check, `fuzz_long` reads `CROW_FUZZ_TREE`/`CROW_FUZZ_CHURN` and alternates churn, `Failure` records tree/churn/without-churn. Tests: delete/replace shown folder, churn 2 s on 1k then listing = disk, edge replay. Full app suite 401 passed, 2 ignored. |
| I3 Runs | done | 35.8 min of fuzzing (3 parallel jobs, 15 kinds × 2 min, 600 steps/seed, churn on) plus the quick matrix. All 15 kinds ran: **3 clean** (nfc-nfd-twins, emoji, leading-dot), **10 listing failures** (#7: long-name, rtl, spaces, deep-20, symlink-loop, dangling-symlink, package, hardlinks, empty-folder, wide-100000), **2 focus artifacts** (#8: unreadable, huge-file), and `newline` is #6. |
| Finish | done | fmt ✓ · clippy workspace ✓ · clippy `--features fuzz` ✓ · `cargo test --workspace` 401 app + 92 core + 42 fs + 5 macos + 18 search + 6 theme, 3 ignored. New fixture hash `b659b9a8…` = main's; bench warm 7 ms, p99 8.70 ms. |

## Questions
- Q4 (asked 2026-11-15): the new edge smoke finds a real panic (H4 #6: a `\n` in a file name crashes GPUI text shaping when the row is drawn). Fix it in H2, or record it for H4 and keep the normal suite green? → answer: **record for H4**; `fuzz_smoke_edge_trees` is `#[ignore = "H4 #6"]`.
- Q1 (asked 2026-11-15): Should `Unreadable` and `SymlinkLoop` trees also be in `fuzz_smoke` (every `cargo test`)? Recommended: yes for a 50-entry tree with every kind except `Wide` and `HugeFile`; those stay in `fuzz_long`. → answer: **yes**.
- Q2 (asked 2026-11-15): Removable/slow volumes (D1 note): simulate with a slow FUSE-like layer, or leave to a hand check with a USB stick / `hdiutil` disk image? Recommended: hand check with a disk image, recorded in `docs/testing.md`; no simulation layer. → answer: **hand check with a disk image**.
- Q3 (asked 2026-11-15): Churn in iCloud Drive / cloud placeholders? Recommended: no; out of scope for the test platform, noted as a hand check. → answer: **no**.

## Decisions
- Branch from `main` @ `cf85599` (H1 merged, PR #30); H2's base condition is met. — plan.
- Plan's `NfcNfdTwins` says "on APFS both can exist". **Spike (2026-11-15): false on this machine's APFS.** Creating `café.txt` NFC then NFD succeeds (same entry, first spelling preserved; `read_dir` shows one name); both spellings resolve to it. `make_tree` creates both with `create_new`, tolerates `AlreadyExists`, and the test accepts one entry on APFS (two on a filesystem that allows both). — plan vs. platform, measured.
- **Spike, name limits:** 255-byte ASCII works, 256 fails; 85 × 3-byte chars (255 bytes) works. The APFS limit is 255 UTF-16 units, not bytes (255 × 3-byte chars = 765 bytes also worked, 256 failed). The plan's 255-byte names are valid and stay as specified.
- **Spike, other kinds:** leading/trailing/only-space names, `\n` names, hardlinks, `.app` folders all work. `chmod 000` on a dir/file: the parent still lists it, reading it fails EACCES, and `remove_dir_all` fails until permissions are restored (so `Tree::remove` chmods dirs/files first). Symlink loop: `lstat` works, `stat` fails ELOOP. `File::set_len(10 GiB)` on APFS allocates 0 blocks (sparse).
- Preview reads are bounded (`TextPreviewer` 8 KB, `text_excerpt` 1 MB), so selecting the 10 GiB sparse file cannot read it all. — `crow-core/src/extensions.rs:187,230`.
- `--make-fixture` stays a shipped CLI flag, so `crow-app` enables `crow-fs/testing` in its normal dependency (the generator is a CLI utility, not test-only). The `testing` feature still exists so other crates can opt out. — plan says move the writer behind it; keeps the documented command unchanged.
- H1's `fuzz_smoke` is 3 seeds × 30 steps on 40 files (3.7 s); the edge smoke adds one 30-step run on `EdgeTree::small()` (Q1), no churn. — H1 progress + Q1.

## Hand checks
- [ ] Edge names in the bundled app: open a tree with every kind and arrow through it; long, RTL, emoji and unreadable rows draw; Quick Look on the unreadable file shows an error; the package opens as a folder. A name with `\n` still panics GPUI text shaping (H4 #6), so skip it or expect the panic.
- [ ] Shown folder replaced: with a folder shown, `rm -rf` it and recreate it with new files; Crow should show the new contents. Today it keeps the old rows (H4 #7).
- [ ] Churn by hand: `Churn` is test-only; the equivalent is a Terminal loop creating/renaming/deleting files in a folder Crow shows. The list follows, the cursor does not jump to a different file, no error log.
- [ ] Removable volume (Q2): `hdiutil create -size 200m -fs APFS -volname CrowTest /tmp/crow.dmg`, attach, browse, eject while shown; Crow shows the folder gone, no panic.

## Found along the way
- **H4 #6** (open): a name containing `\n` panics GPUI text shaping. The edge smoke's seed 4 shrinks to `Step::Keys("alt-down")` (`MoveToLast`) on the `small` tree: the row for `line\nbreak.txt` draws its raw name. `vendor/zed/crates/gpui/src/text_system.rs:679` asserts no newlines; `pane.rs:1843,2172` pass `entry.name` through `truncate_middle_parts`. Recorded in `findings.md`; not fixed (H4). The edge smoke is blocked on Q4.

## Log
- 2026-11-15: read the plan, M7 index, H1 plan/progress, findings, testing docs and the code H2 touches; created the branch and this file; asked Q1–Q3.
- 2026-11-15: I1. Added `crow-fs/src/testing.rs` (edge kinds, seeded `make_tree`, permission-restoring `remove_tree`, `Churn`), the `testing` feature, and moved `--make-fixture` onto `EdgeTree::wide`. `cargo test -p crow-fs` 42 passed (0.37 s); clippy clean with and without `testing`; `cargo check -p crow-app` clean.
- 2026-11-15: I2. Added the two shown-folder steps, `FuzzOptions`, churn pause/resume around deep checks, `CROW_FUZZ_TREE`/`CROW_FUZZ_CHURN`, and the new tests. The edge smoke found H4 #6 on its first run (newline name → GPUI panic); Q4 says H4, so `fuzz_smoke_edge_trees` is ignored. Full `cargo test -p crow-app`: 401 passed, 2 ignored (22.8 s); clippy clean, also with `--features fuzz`.
- 2026-11-15: I3 quick matrix (16 kinds × 50 steps, churn on): all pass except `newline`, which panics at step 0 with #6 (`alt-cmd-n`); added that trigger to the findings row. Started 3 parallel 2-minute-per-kind jobs (600 steps/seed, shrink 60 s, churn on) for the 15 remaining kinds. `docs/testing.md` now documents the kinds, `CROW_FUZZ_TREE`/`CROW_FUZZ_CHURN`, the ignored edge smoke, edge replay and the volume hand checks. Baseline bench fixture hash: `b659b9a8…` (100k files, `target/fixture-100k`).
- 2026-11-15: I3 runs. 3 jobs, 13.1 min wall, 35.8 min of fuzzing. All 15 kinds ran: 3 clean (nfc-nfd-twins, emoji, leading-dot), 10 listing failures (#7: long-name, rtl, spaces, deep-20, symlink-loop, dangling-symlink, package, hardlinks, empty-folder, wide-100000), 2 focus artifacts (#8: unreadable, huge-file); `newline` is #6. All 10 listing mismatches involve `DeleteShownFolder`/`ReplaceShownFolder`; a deterministic spike showed replacing the shown root leaves the old rows (`replacing_the_shown_folder_reloads`, ignored for H4 #7); deleting the root alone is handled. A 10 s churn with no shown-folder step settled cleanly, so the listing class is not settle latency. The two focus failures end with a separate window (`alt-cmd-n` uncapped, `cmd-,` Settings) — recorded as #8, a fuzzer artifact. Removed the spikes; kept the ignored #7 test.
- 2026-11-15: Finish. `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` (also `-p crow-app --features fuzz`) and `cargo test --workspace` pass: 401 app (3 ignored), 92 core, 42 fs, 5 macos, 18 search, 6 theme. `--make-fixture` writes the same 100k fixture (manifest hash `b659b9a8…`). `--bench`: warm first paint 7 ms, p99 8.70 ms, avg 8.42 ms, max 40.00 ms. `docs/testing.md` has the kinds, flags, edge replay, volume hand checks and the numbers.

## Next
Ready for critique. The critic reviews the branch and opens the PR; the user hand-tests #6/#7 and the disk-image check.
