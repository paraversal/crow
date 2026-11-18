# B2 — Hidden names and Cmd-K files progress

Branch: `m8.5-b2-hidden` · PR: none · Last updated: 2026-11-18 · State: in progress

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 Engine: the hidden policy | done | `Query.enter_hidden_folders`; the walker always visits dot entries and returns `WalkState::Skip` for a dot folder with a `HiddenFolder` skip; `Hit.hidden` from the dot name or `UF_HIDDEN`. 30 crow-search unit tests + all corpus cases pass, including the new `UF_HIDDEN` and dot-folder cases. |
| 2 Cmd-F rows and skips | done | `Row.hidden`, dimmed icon/name only, aria `, hidden`; `HiddenFolder` skips kept; header "Skipped folders"; `skip_reason_text`. GPUI `search_finds_dot_names_with_hidden_off` and `search_everything_enters_hidden_folders` pass on the corpus. |
| 3 Cmd-K: one matcher for entries | todo | |
| 4 Cmd-K path mode | todo | |
| 5 Cmd-K typed query: files here | todo | |
| 6 Docs and hand checks | todo | |

## Questions

None; the plan says there are none and the index's decisions are binding.

## Decisions

- Branch `m8.5-b2-hidden` from `main` at `fb760a8` (B1 merged as #56), as the plan requires.
- Items 1 and 2 share one commit: the app does not compile with `Hit.hidden` until `Row` carries it, and the hidden-folder skip is only visible in the app through item 2. The engine tests and the GPUI tests are in that commit.
- `corpus::Expect` gains `HitAndSkip(ancestor)`: `hidden-folder-name` expects `.backup` to be both a name hit and a skip. The plan asks for both in one case; the existing variants express either alone. The runner checks the path only; the engine unit test pins `HiddenFolder` as the reason.
- The GPUI corpus tests toggle Show Hidden Files off with `cmd-shift-.` first: the test config's default is `show_hidden_files = true` (Settings default), unlike the plan's premise. `open()` keeps the app's default settings.

## Hand checks

- [ ] Hidden off, Cmd-F in `~` for `zshrc` (or another dotfile): found, dimmed, VoiceOver reads "…, hidden". The footer counts the skipped dot folders; the list shows them as "hidden folder (Show Hidden Files is off)". ⇧⌘A walks into them.
- [ ] Hidden on: the same search behaves as on `main`, except for B1's faster streaming.
- [ ] Cmd-K in `~`, hidden off: the empty query shows no dot folders; typing `config` lists `.config` after visible matches, dimmed; `.co` puts it first. Type `zshrc`: it appears under "Files in <you>", and Return reveals it.
- [ ] Cmd-K path mode: `~/ssh` lists `~/.ssh`; `~/Desktop/<part of a file name>` matches mid-name.
- [ ] `target/fixture-100k`: open it, Cmd-K, type `file-0`, `file-00`, `file-000` quickly. No visible lag; record the `palette refresh` debug times against the 16 ms bar.
- [ ] Light/dark, 640 px: dimmed rows stay readable and the "Files in" header aligns with "In <here>".

## Found along the way

## Log

- 2026-11-18 Plan read, branch created from `main` (`fb760a8`), progress file written.
- 2026-11-18 Items 1–2: hidden policy in the engine, `Hit.hidden`, `HiddenFolder` skips, corpus cases flipped/added, dimmed rows and aria, skip list reason. `cargo test -p crow-search` 30 unit + 2 corpus tests pass; the two new GPUI tests pass; fmt and clippy clean.

## Next

Item 3: `palette::match_entries`, `PathEntry.hidden`, `Item::Folder.hidden`, dimmed palette rows, and the unit tests including the 78k-entry timing rewrite.
