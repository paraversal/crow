# Crow

A Finder-style file explorer for macOS, written in Rust on [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui).
It keeps Finder's directness and aims to be faster, more predictable and keyboard-first.

Status: Phase 1 (the "daily driver" MVP), milestone M6.

## Build and run

```bash
./scripts/fetch-gpui.sh          # once: Zed's GPUI at a pinned commit, into vendor/, with patches/gpui-*.patch applied
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

Four files in `~/.config/crow/`, each created on first launch with every option documented.
Changes apply immediately, and Crow keeps your comments when it writes to them.

| File | What it holds |
| --- | --- |
| `settings.toml` | Theme, keymap (`"finder"` or `"vim"`), key hints (`"auto"`, `"always"` or `"off"`), font, hidden files, window size |
| `views.toml` | How folders are shown, in three layers: `[defaults]`, `[[rule]]` path patterns, and `[folder."…"]` for one folder. Changing a folder's view or sort saves it as that folder's own setting. View ▸ Why This View? (Cmd-J) shows which layer decided what. |
| `sidebar.toml` | Groups of favorites (with optional `name`, `icon`, `color` and Vim mark `key`), and whether Recent and Locations show |
| `exclude.toml` | Folders search skips, in named sets: `dev` is on by default (`node_modules`, `target` beside `Cargo.toml`, `.git`, `build`/`dist` beside manifests, …); `dependencies`, `media`, `hidden` and `system` are off until enabled. The search footer counts results and files, and lists each skipped folder with the rule that skipped it. |

Themes come from the global index in `~/.config/themes`. View ▸ Theme… (Cmd-Option-T)
previews each theme as you move through the list; Apply to All Apps (Cmd-Return) sets it for every app that reads the index.
Recent folders are kept in `~/.local/state/crow/recent.json`, merged with zoxide's list when it's installed.
Key-use counts, which retire idle bottom-bar hints, live next to it in `~/.local/state/crow/usage.json`.
Logs go to `~/Library/Logs/Crow/crow.log`.

## Keys

Finder's keys work as in Finder: arrows (Shift extends), Option-Up/Down, Cmd-Up/Down,
Cmd-[ and Cmd-], Cmd-A, Cmd-T, Cmd-W, Cmd-Shift-[ and ], Cmd-Shift-G (Tab completes),
Cmd-Shift-. and Space. Typing a name selects it. Tab moves focus between the sidebar and the list.
Column headers sort (click again to reverse) and their dividers resize.
Type `/` to filter the focused folder: the field opens as the first row of the focused column
(or under the list header), showing the query and "n of N", and matching letters are highlighted.
Esc clears it, arrows move within the matches, and Return opens the selection. Cmd-F carries the
filter text into a search of that folder. Cmd-2 shows a list, Cmd-3 columns. In the list, Right opens the selected folder inline (Right again goes into it); Left steps to the row that contains a nested one, closes an open folder, or goes up from the top level. In columns, Left and Right move between them, and Left in the leftmost column goes up a folder. Vim `h`/`l` follow the same keys.

Sidebar: drag favorites to reorder them (or Cmd-Option-Up/Down), drop files on any row to move or
copy them there, drop folders on a group's title to add them. Ctrl-Cmd-T adds the selected folder.
In the sidebar, Cmd-Delete removes a favorite or forgets a recent folder, and Cmd-E ejects a disk.

With `keymap = "vim"`: j/k, h/l (parent and open, or columns), gg/G, yy/dd/p (copy, cut, paste),
u and Ctrl-R, cw renames, `-` goes up, `m x` marks the folder as favorite x and `'x` jumps there.

Menus list every command with its key and grey out what the focused view cannot do: File,
Edit (including Filter This Folder, Find… and Find Everything…), View, Go (Back, Forward,
Enclosing Folder, Go to…, Go to Folder…), Window and Help. Use as Defaults, Reset View for
This Folder and Zoom have no shortcut. Cmd-Shift-A from anywhere opens search with every
`exclude.toml` set off. Shortcuts shown in the command palette, tooltips and hints come from
the live keymap, so they follow `keymap = "vim"` too. Hover a nav arrow, a view button, the Go to button, the search field, a path segment, a disk row,
Search everything or Cancel during an operation to see its name and key. Sheets (Go to Folder,
Why This View?, the theme picker, Cmd-K and the delete/conflict questions) dim the window;
clicking outside one cancels it exactly like Esc.

The bottom bar has three slots: the path, a transient one (hints, operation progress, messages,
errors) and counts. The idle hint shows `/` filter (Search and Go to carry their keys on the
toolbar); after you have used it three times it retires (`hints = "always"` keeps every hint, `"off"` hides hints and the
one-time tips). Hints follow focus: the filter field (Return open, Esc clear, `⌘F` search deeper),
the sidebar (`⌘⌥↑↓` reorder, `⌘⌫` remove, `⌘E` eject a disk), search results and the search field.
While an operation runs, a 2 px accent line along the bar's top edge and "Copying … · n %" with
Cancel esc replace the hints; the path and count stay. A broken config file shows an error pill with
its file name, line and an Open button, beside progress rather than instead of it. Counts name the
focused folder ("18 items in Documents"), the selection and its size, or the cursor's sidebar
group (while filtering, the field itself says "n of N"). When room runs short the path keeps its
first and last segments: its middle folds into a "…" (click it for the deepest hidden folder),
then hints drop from the right, then a long progress line or error is cut with an ellipsis.

Closing the last window or quitting while an operation runs asks first: Esc keeps it running,
Return stops it cleanly (finished items stay and Undo can reverse them; the item in progress is never left half done) and then closes or quits. In every text field
⌥←/⌥→ move by word, ⌥⇧←/→ select by word and ⌥⌫ deletes a word. In list view the Name column
takes whatever width the window gives; the other columns keep the widths you drag them to, and
dragging Name's own divider trades width with the column after it.

Toolbar icons are [Lucide](https://lucide.dev) 1.72.0, ISC-licensed; the notice is at
`crates/crow-app/assets/icons/LICENSE` (also copied into `Crow.app/Contents/Resources/`).
Letters no longer select by name in this layer.

File operations use Finder's keys: Return renames, Cmd-Shift-N new folder, Cmd-D duplicate,
Cmd-Delete moves to the Trash, Cmd-Option-Delete deletes permanently (after asking),
Cmd-C / Cmd-X / Cmd-V copy, cut and paste (also to and from Finder), Cmd-Option-V moves,
Cmd-Z / Cmd-Shift-Z undo and redo, Esc cancels a running operation, Cmd-Option-L shows activity.
Nothing is ever overwritten: name clashes ask first, and Replace moves the old item to the Trash.

Cmd-K opens the command palette: type to go to a favorite, recent or subfolder; start with `>`
for commands (labels and their keys). Return goes there, Cmd-Return opens a folder in a new tab.
Cmd-F, or the top band's Search field, searches names and contents in the current folder; the same
field moves into the band while searching, where All / Names / Contents choose what to match (Names
reads no file contents) and Cmd-Shift-M cycles the three. Cmd-F again focuses the field and selects
the query; Tab goes field → results → sidebar → field (no results: straight to the sidebar); Done or
Esc (from anywhere) closes, and with the skipped list open Esc closes that first. The scope bar
switches the folder, Home or This Mac (Spotlight, names only); its switch skips the folders named in
`exclude.toml` sets, as Cmd-Shift-A does for every set. Name matches whose name contains the query
come first with their letters highlighted; scattered fuzzy name hits follow the content hits. A name
match stays one line, a content-only hit shows its line number and line under the name, and the
footer counts results and files and lists what was skipped (relative to the folder). With nothing
typed, four examples fill the field. The right preview follows the selected result in list and
column view, at the hit line for content hits.
Results open with Return, reveal with Cmd-R, Quick Look with Space, and take the usual copy, cut and
trash keys. "Go to ⌘K" in the band opens the command palette.

Testing, benchmarks and the hand-test checklist: [docs/testing.md](docs/testing.md).
