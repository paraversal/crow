# Crow

A Finder-style file explorer for macOS, written in Rust on [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui).
It keeps Finder's directness and aims to be faster, more predictable and keyboard-first.

Status: Phase 1 (the "daily driver" MVP), milestone M4.

## Build and run

```bash
./scripts/fetch-gpui.sh          # once: Zed's GPUI at a pinned commit, into vendor/
cargo run --release -p crow-app  # opens your home folder
./scripts/bundle.sh --open       # builds and opens target/bundle/Crow.app
```

Running as `Crow.app` makes macOS privacy prompts name Crow instead of your terminal.

## Layout

| Crate | What it does |
| --- | --- |
| `crates/crow-app` | The `crow` binary: windows, tabs, sidebar, file list, preview, path bar |
| `crates/crow-core` | UI-free model: settings, tabs, history, selection, extension points |
| `crates/crow-fs` | Fast folder listing (`getattrlistbulk`), sorting, change diffs; file operations from M4 |
| `crates/crow-macos` | The only AppKit code: icons, kinds, tags, thumbnails, Finder names, Quick Look, dragging |
| `crates/crow-search` | Search and exclusion rules (M6) |
| `crates/crow-theme` | Reader for the global theme index in `~/.config/themes` |

## Configuration

`~/.config/crow/settings.toml` is created on first launch with every option documented.
Changes apply immediately. The theme comes from `~/.config/themes`.
Logs go to `~/Library/Logs/Crow/crow.log`.

## Keys

Finder's keys work as in Finder: arrows (Shift extends), Option-Up/Down, Cmd-Up/Down,
Cmd-[ and Cmd-], Cmd-A, Cmd-T, Cmd-W, Cmd-Shift-[ and ], Cmd-Shift-G (Tab completes),
Cmd-Shift-. and Space. Typing a name selects it. Tab moves focus between the sidebar and the list.
Column headers sort (click again to reverse) and their dividers resize.
Cmd-2 shows a list, Cmd-3 columns; in columns, Left and Right move between them.
The default view is set by `view` in `settings.toml`.

File operations use Finder's keys: Return renames, Cmd-Shift-N new folder, Cmd-D duplicate,
Cmd-Delete moves to the Trash, Cmd-Option-Delete deletes permanently (after asking),
Cmd-C / Cmd-X / Cmd-V copy, cut and paste (also to and from Finder), Cmd-Option-V moves,
Cmd-Z / Cmd-Shift-Z undo and redo, Esc cancels a running operation, Cmd-Option-L shows activity.
Nothing is ever overwritten: name clashes ask first, and Replace moves the old item to the Trash.

Testing, benchmarks and the hand-test checklist: [docs/testing.md](docs/testing.md).
