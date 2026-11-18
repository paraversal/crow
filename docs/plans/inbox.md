# Plans inbox

Unsorted work. New triage items wait here until they get a tag and a milestone folder; finished plans move to `done/`. Current milestones: [`m8/`](m8/README.md) (Finder parity), [`m9/`](m9/README.md) (Phase 2); M8.5's index is in [`done/m8.5/`](done/m8.5/README.md).

Format: one line per item, with a tag (`f` feature · `p` performance · `ui` UI improvement · `b` bug · `d` future direction · `s` setting), the source, and what is unclear. An item moves out when a milestone index names it and a plan exists (or the index says which piece will plan it).

## Untriaged

- `f` Custom keybindings: let the user set, see and persist a key for a command (the F7 answer asked for a bindable Open Terminal Here; F7 shipped the command with no key). Source: F7 Q1, 2026-11-17. Unclear: config shape (`settings.toml` table vs its own file), how conflicts between layers are reported, and whether it waits for the M9 settings/plugin work.
- The first 2026-11-17 list is triaged in [`m8/README.md`](m8/README.md#triage-2026-11-17); the Phase 2 items are in [`m9/README.md`](m9/README.md). The second 2026-11-17 list is triaged in [`m8/README.md`](m8/README.md#triage-2026-11-17-second-list): Empty Trash is F7.6, the test temp-dir leak is M9 P9, and search reliability is now its own milestone, [`done/m8.5/README.md`](done/m8.5/README.md).
