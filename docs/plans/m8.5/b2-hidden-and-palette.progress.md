# B2 — Hidden names and Cmd-K files progress

Branch: `m8.5-b2-hidden` · PR: none · Last updated: 2026-11-18 · State: in progress

## Status

| Item | State | Notes |
| --- | --- | --- |
| 1 Engine: the hidden policy | todo | |
| 2 Cmd-F rows and skips | todo | |
| 3 Cmd-K: one matcher for entries | todo | |
| 4 Cmd-K path mode | todo | |
| 5 Cmd-K typed query: files here | todo | |
| 6 Docs and hand checks | todo | |

## Questions

None; the plan says there are none and the index's decisions are binding.

## Decisions

- Branch `m8.5-b2-hidden` from `main` at `fb760a8` (B1 merged as #56), as the plan requires.

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

## Next

Item 1: rename `Query.show_hidden` to `enter_hidden_folders`, always visit dot entries, skip descending into dot folders with a `HiddenFolder` skip, and add `Hit.hidden`.
