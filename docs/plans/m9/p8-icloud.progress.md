# M9 P8 — iCloud metadata without hydration — progress

Branch: codex/m9-p8-icloud · PR: none · Last updated: 2026-11-18 · State: **in progress**
Base: `b035537` (M9 P9C, merged as #64)

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 Platform spike | done | Fixtures created and eviction/hydration matrix measured; results below |
| 2 Typed status, cache and UI | done | `crow-fs::cloud`, `crow-macos::cloud`, app `CloudStatuses`, row badges, Info/Preview/VoiceOver text |
| 3 Automatic-read audit and gate | done | Previewers, excerpts, Quick Look, icons, search content; folder sizes stay metadata-only |
| 4 Tests, hand checks and bench | done | 923 workspace tests, fmt/clippy green, local and iCloud benches below; VoiceOver not driven |

## Spike results (2026-11-18, macOS 27.2.1, build 26B131)

Scratch root: `~/Library/Mobile Documents/com~apple~CloudDocs/CrowP8Spike-20261118` (Crow's own,
created for this plan; no unrelated iCloud items touched). Ordinary local control:
`target/p8-spike/local` outside File Provider storage. `brctl evict` uses the same File Provider
eviction path as Finder's Remove Download; `fileproviderctl evaluate` was the Finder-side comparison.

| Case | `st_flags` | ubiquitous | downloading | downloading status | Finder `fileproviderctl` |
| --- | --- | --- | --- | --- | --- |
| Local control outside iCloud | `0x0` / `0x40` | absent/false | false | absent | not applicable |
| iCloud file, current | `0x40` | true | false | `…StatusCurrent` | `isDownloaded=1`, `isMostRecentVersionDownloaded=1` |
| iCloud file, evicted | `0x40000060` (`SF_DATALESS|UF_COMPRESSED|UF_TRACKED`) | true | false | `…StatusNotDownloaded` | `isDownloaded=0`, `isMostRecentVersionDownloaded=0` |
| iCloud file, downloading | `0x40`, then back to `0x40` | true | true, then false | `Current` during the short local-provider transfer | transfer was too fast to catch more |
| iCloud folder, contents evicted | `0x0` on the folder; `0x40000060` on each child | true | false | folder `Current`, children `NotDownloaded` | same per item |
| symlink to current file | symlink `0x0`, target `0x40` | true on the link | false | link `Current` | link treated as an item |
| symlink to evicted file | symlink `0x0`, target `0x40000060` | true on the link | false | link `Current` (the target's flag is the signal) | link lacks the target's downloaded bit |

Findings:

- Evicted items keep their ordinary names in `read_dir`; no extension or placeholder name.
- `SF_DATALESS` (`st_flags & 0x40000000`) is present in the existing `getattrlistbulk` `ATTR_CMN_FLAGS`
  path and in `lstat`/`symlink_metadata`; no extra call is needed for a bulk/portable listing. The
  folder itself is not dataless; its children are.
- `NSURLUbiquitousItemDownloadingStatusKey` distinguishes `Current`, `Downloaded` and `NotDownloaded`.
  `Downloading` takes precedence when `NSURLUbiquitousItemIsDownloadingKey` is true; the dataless flag
  can already be clear during that interval, so a flags-only gate cannot classify an active download.
- The `Downloaded` (obsolete-but-local) status could not be produced on this machine: with one local
  iCloud account there is no older version to retain. It is mapped to `Downloaded` per Apple's key
  documentation and covered by a pure mapping test.
- Foundation availability lookup measured with 2,000 calls in a loop: **~750 µs/item** under iCloud,
  **~23 µs/item** for an ordinary local path (the local path returns `isUbiquitous=false`). `lstat`
  is ~1 µs. So visible-row classification is a batched background task; the local fast path must not
  call Foundation per entry.
- Hydration checks (flags before/after; a separate scratch item per check):

| Automatic consumer | Hydrates? | Evidence |
| --- | --- | --- |
| `stat` / `lstat` / `ls` / bulk listing | no | flags stayed `0x40000060` |
| `head -c 1` (text preview, search content) | **yes** | `0x40000060` → `0x40`, status `NotDownloaded` → `Current` |
| Quick Look thumbnail (`qlmanage -t`) | **yes** | flags and status changed to current |
| Finder tags (`NSURLTagNamesKey`) | no | stayed evicted |
| `NSWorkspace.iconForFile` on an evicted image/folder | no | stayed evicted |
| `du` (metadata walk, Crow's folder-size walk) | no | stayed evicted, reported 0 bytes |
| Foundation resource keys (`isUbiquitous`, `isDownloading`, status) | no | stayed evicted |

- FSEvents fired for the evicted path on `brctl evict` and again on download (`ItemModified` /
  inode-metadata style flags), so the existing folder watcher invalidates cached statuses.
- `~/Library/CloudStorage` is empty on this machine; a third-party File Provider domain could not be
  exercised. The dataless flag is the same OS-level signal, so the implementation gates on it and on
  the two known provider roots, and fails closed for ambiguous symlinks.
- A package/app bundle could not be evicted (`NSFileProviderErrorDomain -2008`), so path-specific
  custom-icon hydration was not observed. The implementation still uses a generic type icon for a
  cloud-blocked directory.

## Questions

None. The guarantee is met by the dataless flag plus a fresh Foundation check for known cloud
candidates; no unresolved provider behavior blocks the plan.

## Decisions

- Status is split: `CloudStatus` is the cached, user-visible classification; `ReadDecision` is a fresh,
  cheap gate checked immediately before every automatic content read. A cached status never authorizes
  a read on its own.
- `SF_DATALESS` is carried in `crow_fs::Entry` from the existing bulk/portable metadata flags. No
  Foundation call is made while listing or rendering a local folder.
- Foundation status lookups run only for paths under `~/Library/Mobile Documents` or
  `~/Library/CloudStorage`, or for a symlink whose target is under one of those roots. A dataless path
  anywhere is `NotDownloaded` without Foundation. Classification is batched off-main and cached with
  a bounded 4096-entry cache; stale replies are dropped by per-path version.
- `Downloading` outranks `Current`/`Downloaded`/`NotDownloaded`; `Unknown` is distinct from `Local`.
- Rows follow Finder: no badge for local or downloaded/current items; a muted cloud glyph for not
  downloaded, an accent cloud glyph for downloading, and a warning cloud-off glyph for unknown. Info
  always shows the typed status when it is not `Local`. No download control.
- A cloud symlink is `SkipAmbiguous` for automatic reads unless its target is positively local. The
  user can still open it explicitly (Space, Return, drag, copy), which keeps Finder's explicit-action
  semantics.
- Names-only search never calls the gate or opens content. Content search keeps name/metadata hits,
  counts skipped cloud contents in the footer, and checks the gate again immediately before each open.
- `crow-plugin::read_excerpt` is not wired to the app gate yet; the existing plugin host has no
  automatic excerpt consumer until P1B. Recorded under Found along the way for P1B.
- Row badges are static glyphs; the row's aria label and the Info label carry the spoken status.
- Pane service observers are kept in `_service_subscriptions`, separate from the per-open-folder
  `subscriptions` that navigation rewrites. The old shared vector silently dropped the size observer
  on a mode change and would have dropped the cloud observer too.

## Verification

- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`: green.
- `cargo test --workspace`: 923 passed, 1 ignored. App 620, core 104, fs 71, macos 13, plugin 67,
  search 40 + 2 corpus, theme 6.
- New coverage: status precedence/local/unknown mapping; dataless/read-hint policy; gate refusal
  returns no preview/excerpt; app cache bound, versioned late replies, A/B isolation; fake gate
  placeholder → downloading → downloaded with preview re-arm; Info spoken status; search name hit
  survives with only eligible content reads; Names mode calls no gate; cloud footer copy.
- **Bench, local 100k** (`target/fixture-100k`, FireBox, 120 Hz; branch vs `main` `b035537`):
  branch cold paint 90 ms / warm 8 ms / cold load 355 ms / warm load 317 ms / p99 8.62 ms;
  main 89 ms / 9 ms / 371 ms / 316 ms / p99 8.69 ms. Parity.
- **Bench, search 200k** (`target/fixture-search-200k`, `--bench-search needle`): branch contents
  1557 ms, names 186 ms; main contents 1534 ms, names 188 ms. Content is ~1.5% slower from the
  per-file fresh lstat, names parity. The first branch run (2760 ms) exposed an allocation in
  `is_cloud_candidate`; caching the two roots brought it back to parity.
- **Bench, iCloud scroll**: 1,000 current files in a scratch iCloud folder, `--bench`: cold paint
  115 ms, warm first paint 2 ms, p99 8.61 ms over 599 frames; background status batches did not
  disturb frames.
- **iCloud search cost**: the 19-entry scratch tree, `--bench-search payload --contents`, 13.6 ms,
  0 hits, 1 skipped content; two evicted files stayed `0x40000060`.

## Hand checks

- [x] iCloud listing, FireBox dark and Solarized Light at 640 px: evicted text/image rows show the
  muted cloud glyph beside the name; current files and folders do not. The glyph reads through the
  theme in both.
- [x] Automatic preview on an evicted text file and an evicted image: metadata-only placeholder,
  no preview payload; flags stayed `0x40000060` in the probe after selection.
- [x] Info tab, dark and light: evicted file shows `iCloud  Not downloaded` with the cloud glyph;
  current file shows no iCloud row; changing to downloaded makes the row say Downloaded.
- [x] External download while the evicted file stayed selected: `brctl download` changed the row
  badge away and the Preview re-armed from the placeholder to the text without opening Quick Look.
- [x] Content search over the scratch tree (`payload`): footer read
  “No results · 6 cloud files not downloaded · 1 folder skipped”; `large.txt` stayed evicted.
- [ ] Names-mode hand search in the iCloud folder: the attempt lost focus to another window; covered
  by `names_mode_never_calls_the_read_gate` and the names CLI bench, not by a visual hand check.
- [x] Local `target/fixture-100k`: listing, scroll and search unchanged within noise; no Foundation
  calls for local rows (`is_cloud_candidate` is false before any provider call).
- [ ] VoiceOver: row/Info/Preview strings were added and the GPUI labels assert them; Cmd-F5 was not
  driven.
- [ ] Explicit Space Quick Look on an evicted image showed the system's “An error occurred with the
  preview of this document” and left the file evicted. The gate is not on the explicit path; this is
  the OS Quick Look result on this machine, not a Crow read policy decision.

## Found along the way

- Pane service observers shared `Pane::subscriptions` with per-open-folder subscriptions. A mode
  change rewrites that vector, so the size observer (and the new cloud observer) was silently
  dropped. Fixed by `_service_subscriptions`.
- `crow-plugin` host test `ticket_cancel_during_evaluation_is_cancelled_and_keeps_the_plugin_running`
  raced the operation budget: a 5 ms sleep sometimes let the LOOP script exhaust 100k operations
  before `ticket.cancel()`, reporting `WorkerFailed("operation budget exhausted")` instead of
  `Cancelled`. Fixed by capturing the call token before `Runtime::end()` and mapping any evaluation
  error to `Cancelled` when the token is set; the test now waits for the script to enter a gate, and a
  new regression uses a 10-operation budget to cover the budget/cancel race. The plugin stays
  `Running`.
- `crow-plugin::read_excerpt` still reads a dataless file if a future automatic plugin asks for an
  excerpt. P1B must pass the app `ReadGate` into the excerpt host path; the existing host has no
  automatic consumer yet.
- Quick Look (explicit Space) on a dataless image fails with an OS preview error on this macOS;
  recorded for the user's hand test.

## Log

- 2026-11-18: Read the plan, M9 index, AGENTS/testing docs and the current listing, preview, Info,
  search, icon and folder-size code. Created branch from `b035537`. Ran the metadata/hydration/FSEvents
  /cost spike; results above. Created this progress file.
- 2026-11-18: Added `crow-fs::cloud` (`CloudStatus`, `ReadDecision`, `ReadHint`, `ReadGate`,
  `SF_DATALESS`, cached roots), `Entry.dataless`, `crow-macos::cloud` (Foundation mapping, symlink
  target resolution, `SystemReadGate`), and the app `CloudStatuses` service with batched lookups,
  bounded cache, versioned replies, visible-source union and 500 ms active-download polling.
- 2026-11-18: Wired badges and spoken status into list, column, search, Info and Preview; added the
  metadata-only preview placeholder, cloud-aware type icons, and status invalidation from folder
  watcher events. Split pane service subscriptions from folder subscriptions.
- 2026-11-18: Gated `Previewer` (text and Quick Look), `text_excerpt`, and the search content reader;
  added `Query.read_gate`, `Summary.cloud_skipped` and the footer copy. Cached cloud roots after the
  first 200k search bench exposed the allocation.
- 2026-11-18: Fixed the plugin budget/cancel race reported during the full run, added the regression,
  ran workspace fmt/clippy/tests, local 100k and search benches, and the iCloud hand checks. Removed
  the iCloud scratch trees, bench worktree and throwaway probes.

## Next

Commit the plugin fix separately, then the P8 implementation and this progress file. The code and
hand checks are complete; leave VoiceOver and the names-mode iCloud visual check for critique/user.
