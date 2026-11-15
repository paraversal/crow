# H2 edge trees and churn progress
Branch: `m7-h2-churn` · PR: none · Last updated: 2026-11-15 · State: in progress

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 `crow_fs::testing` | todo | |
| I2 Churn | todo | |
| I3 Runs | todo | |
| Finish | todo | |

## Questions
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
- [ ] Edge names in the bundled app: open a tree with every kind and arrow through it; long, RTL, emoji, newline and unreadable rows draw and do not panic; Quick Look on the unreadable file shows an error; the package opens as a folder.
- [ ] Churn by hand: `Churn` is test-only; the equivalent is a Terminal loop creating/renaming/deleting files in a folder Crow shows. The list follows, the cursor does not jump to a different file, no error log.
- [ ] Removable volume (Q2): `hdiutil create -size 200m -fs APFS -volname CrowTest /tmp/crow.dmg`, attach, browse, eject while shown; Crow shows the folder gone, no panic.

## Found along the way
- (none yet)

## Log
- 2026-11-15: read the plan, M7 index, H1 plan/progress, findings, testing docs and the code H2 touches; created the branch and this file; asked Q1–Q3.

## Next
Record the answers to Q1–Q3, spike the filesystem assumptions (255-byte names, NFC/NFD twins, trailing spaces, newline, sparse 10 GB), then start I1.
