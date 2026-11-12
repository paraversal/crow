# Crow

A Finder-style file explorer for macOS, written in Rust on [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui).
It keeps Finder's directness and aims to be faster, more predictable and keyboard-first.

Status: Phase 1 (the "daily driver" MVP), milestone M5.

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
| `crates/crow-macos` | The only AppKit code: icons, kinds, tags, thumbnails, Finder names, Quick Look |
| `crates/crow-search` | Search and exclusion rules (M6) |
| `crates/crow-theme` | Reader for the global theme index in `~/.config/themes` |

## Configuration

Three files in `~/.config/crow/`, each created on first launch with every option documented.
Changes apply immediately, and Crow keeps your comments when it writes to them.

| File | What it holds |
| --- | --- |
| `settings.toml` | Theme, keymap (`"finder"` or `"vim"`), font, hidden files, window size |
| `views.toml` | How folders are shown, in three layers: `[defaults]`, `[[rule]]` path patterns, and `[folder."…"]` for one folder. Changing a folder's view or sort saves it as that folder's own setting. View ▸ Why This View? (Cmd-J) shows which layer decided what. |
| `sidebar.toml` | Groups of favorites (with optional `name`, `icon`, `color` and Vim mark `key`), and whether Recent and Locations show |

Themes come from the global index in `~/.config/themes`. View ▸ Theme… (Cmd-Option-T)
previews each theme as you move through the list; Apply to All Apps (Cmd-Return) sets it for every app that reads the index.
Recent folders are kept in `~/.local/state/crow/recent.json`, merged with zoxide's list when it's installed.
Logs go to `~/Library/Logs/Crow/crow.log`.

## Keys

Finder's keys work as in Finder: arrows (Shift extends), Option-Up/Down, Cmd-Up/Down,
Cmd-[ and Cmd-], Cmd-A, Cmd-T, Cmd-W, Cmd-Shift-[ and ], Cmd-Shift-G (Tab completes),
Cmd-Shift-. and Space. Typing a name selects it. Tab moves focus between the sidebar and the list.
Column headers sort (click again to reverse) and their dividers resize.
Cmd-2 shows a list, Cmd-3 columns. In the list, Right opens the selected folder inline (Right again goes into it); Left steps to the row that contains a nested one, closes an open folder, or goes up from the top level. In columns, Left and Right move between them, and Left in the leftmost column goes up a folder. Vim `h`/`l` follow the same keys.

Sidebar: drag favorites to reorder them (or Cmd-Option-Up/Down), drop files on any row to move or
copy them there, drop folders on a group's title to add them. Ctrl-Cmd-T adds the selected folder.
In the sidebar, Cmd-Delete removes a favorite or forgets a recent folder, and Cmd-E ejects a disk.

With `keymap = "vim"`: j/k, h/l (parent and open, or columns), gg/G, yy/dd/p (copy, cut, paste),
u and Ctrl-R, cw renames, `-` goes up, `m x` marks the folder as favorite x and `'x` jumps there.
Letters no longer select by name in this layer.

File operations use Finder's keys: Return renames, Cmd-Shift-N new folder, Cmd-D duplicate,
Cmd-Delete moves to the Trash, Cmd-Option-Delete deletes permanently (after asking),
Cmd-C / Cmd-X / Cmd-V copy, cut and paste (also to and from Finder), Cmd-Option-V moves,
Cmd-Z / Cmd-Shift-Z undo and redo, Esc cancels a running operation, Cmd-Option-L shows activity.
Nothing is ever overwritten: name clashes ask first, and Replace moves the old item to the Trash.

Testing, benchmarks and the hand-test checklist: [docs/testing.md](docs/testing.md).
