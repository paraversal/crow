# u2-menu-bar progress
Branch: `ui-u2-menu-bar` · PR: none · Last updated: 2026-11-12

## Status
| Item | State | Notes |
| --- | --- | --- |
| I1 Verify GPUI/AppKit menu behavior (spike) | todo | Build with temporary menus, record the six findings and macOS version. |
| I2 One command registry with menu slots | todo | U1 already changed `palette::Command` to `{ id, label, action, scope }`; add `menu`/`palette` to that shape. |
| I3 Menu builder from the registry | todo | New `menus.rs`; move global handlers and `set_menus` out of `main.rs`. |
| I4 Find Everything with no search open (Q8) | todo | Add `SearchStart`, `open_search`, Crow-context ⇧⌘A and `set_everything`. |
| I5 App, Window and Help menus | blocked (Q3, Q4) | Build only the menus the user approves. |
| I6 Docs | todo | README and `docs/testing.md`. |

## Questions
- Q1 (asked 2026-11-12, pending): Reveal Result ⌘R under Go? Recommended: yes, Go section 1, menu-only.
- Q2 (pending I1): plain `/`, Space, ↵ key equivalents; decide after the spike.
- Q3 (asked 2026-11-12, pending): Crow menu contents and new bindings ⌘H, ⌥⌘H, ⌘,; recommended About, Services, Hide trio, Settings… opening `settings.toml`.
- Q4 (asked 2026-11-12, pending): Window and Help menus and ⌘M; recommended both, with tab items moved to Window.
- Q5 (asked 2026-11-12, pending): ⇧⌘A with search open and focus outside it; recommended flip the open search's rules and focus its field.
- Q6 (asked 2026-11-12, pending): keep menu titles and palette labels separate; recommended yes.
- Q7 (asked 2026-11-12, pending): Vim layer keys in menus; recommended Finder keys only, documented.
- Q8 (asked 2026-11-12, pending): commands left out of menus; recommended add Open (File §0) and Eject (File §2), leave the rest palette-only.
- Q9 (asked 2026-11-12, pending): New Window location; recommended current folder when focused, Home only when no window.
- Q10 (asked 2026-11-12, pending): no check marks / dynamic Show–Hide titles; recommended no.
- Q11 (asked 2026-11-12, pending): Edit ▸ Cut/Copy/Paste/Select All in text fields; recommended TextInput listens for the Crow actions too.

## Decisions
- Registry shape follows the merged U1 code, not the plan's older snippet: `Command { id, label, action: &'static dyn Action, scope: KeyScope }`. U2 adds `menu: Option<MenuSlot>` and `palette: bool` and keeps `action`/`scope`; there is no `keys` field (U1 derives keys from the live keymap).
- PR #7 and U1 (PR #8) are merged on `main`; branch `ui-u2-menu-bar` starts at `c666799`.
- The user's uncommitted `AGENTS.md` edit and untracked files are left untouched.

## Log
- 2026-11-12: Created `ui-u2-menu-bar`, recorded the plan and current code shape. Read `palette.rs`, `actions.rs`, `main.rs`, `window.rs` search/palette/listener sections, `search.rs` and `text_input.rs`.

## Next
Ask Q1 and Q3–Q11, then run I1's temporary-menu spike and ask Q2 with its findings.
