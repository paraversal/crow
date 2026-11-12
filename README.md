# Crow

A Finder-style file explorer for macOS, written in Rust on [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui).
It keeps Finder's directness and aims to be faster, more predictable and keyboard-first.

Status: Phase 1 (the "daily driver" MVP), milestone M1.

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
| `crates/crow-core` | UI-free model: settings, tabs, history |
| `crates/crow-fs` | Folder snapshots and watching; the file operation engine from M4 |
| `crates/crow-macos` | The only AppKit code: icons, Finder names, Quick Look, dragging |
| `crates/crow-search` | Search and exclusion rules (M6) |
| `crates/crow-theme` | Reader for the global theme index in `~/.config/themes` |
| `spike/` | Phase 0 spike, kept for reference until M2 |

## Configuration

`~/.config/crow/settings.toml` is created on first launch with every option documented.
Changes apply immediately. The theme comes from `~/.config/themes`.
Logs go to `~/Library/Logs/Crow/crow.log`.

## Keys

Finder's keys work as in Finder: arrows, Cmd-Up/Down, Cmd-[ and Cmd-], Cmd-T, Cmd-W,
Cmd-Shift-[ and ], Cmd-Shift-. and Space. Tab moves focus between the sidebar and the list.
