# Architecture

Crow is a Cargo workspace. The UI is GPUI (Zed's GPU-accelerated framework), vendored in `vendor/` at a pinned commit by `scripts/fetch-gpui.sh`, with Crow's changes kept as `patches/gpui-*.patch`.

## Crates

| Crate | What it does |
| --- | --- |
| `crow-app` | The `crow` binary: windows, tabs, sidebar, file views, preview, palette, settings window, and the glue between the others |
| `crow-core` | UI-free model: settings and the other config files, views, sidebar, selection, tabs, history, grid geometry, key overrides, icon-pack validation, the Shelf. Pure Rust, unit-tested |
| `crow-fs` | Folder listing (`getattrlistbulk`), sorting, change watching, and the file-operation engine with undo |
| `crow-macos` | The only AppKit and Foundation code: icons, kinds, tags, thumbnails, Quick Look, trash, the Dock icon |
| `crow-search` | The search engine: walking, matching, exclusion rules |
| `crow-plugin` | Versioned Rhai plugin packages, grants and workers |
| `crow-plugin-git` | The bundled, read-only Git integration |
| `crow-theme` | The built-in themes and the reader for the global theme index |

New logic goes in `crow-core` or `crow-fs` when it needs no UI, so it can be tested without a window; AppKit stays in `crow-macos`.

## Threads
**No disk I/O on the main thread.** Anything that touches the disk runs through GPUI's `background_spawn` and the result is applied back on the main thread. The startup config loads are the one exception. A guard in tests fails when `crow-fs` or `crow-macos` is called off its expected thread, and an `// io-ok:` comment marks the few reviewed exceptions.

## File operations
Every file operation goes through `crow_fs::ops`. Operations are jobs with a plan, a queue, conflict decisions up front and progress; nothing is overwritten in place (a replaced item goes to the Trash), and every job records an undo step. Cancelling stops between items and leaves finished ones undoable.

## State and configuration
Settings, views, sidebar, exclusions, saved queries, keys and plugin grants are TOML files in `~/.config/crow/`. They load at startup, reload on change, report errors with file and line, and are edited through `crow_core::edit_toml` so comments survive. Mutable app state (recent folders, usage counts, the Shelf) is in `~/.local/state/crow/`. The Settings window and the files are two views of the same data.

## UI structure
`CrowWindow` (`window.rs`) owns a window: the left sidebar, the active tab's pane, the right sidebar, the bottom bar and the overlays (palette, sheets, tips). Panes render folder listings in the four views. The sidebar is a flat list of rows with a layout of bounds measured each frame, which drives the card backdrops, drag and drop hit tests and keyboard navigation. Commands are defined once in `palette.rs` (`COMMANDS`) and reused by the menu bar, the palette and the key bindings.

## Keys
Finder and Vim bindings are declared in `actions.rs`; user overrides from `keys.toml` are applied on top (`keymap.rs`). Menus, tooltips, hints and [keys.md](keys.md) all read the live keymap.

## Plugins
Plugins are Rhai packages, run in workers with explicit grants, using the same extension points as Crow's own features (commands, sidebar sections, preview providers, columns, icon packs). See [plugins.md](plugins.md).

## Testing
Unit tests in each crate, GPUI keystroke and layout tests in `crow-app`, a seeded fuzzer, and a bench for 100k-file folders. See [testing.md](testing.md).
