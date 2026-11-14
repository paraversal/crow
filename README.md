# Crow

A Finder-style file explorer for macOS, written in Rust on [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui).
It keeps Finder's directness and aims to be faster, more predictable and keyboard-first.

Status: Phase 1 (the "daily driver" MVP), milestone M6.

## Build and run

```bash
./scripts/fetch-gpui.sh          # once: Zed's GPUI at a pinned commit, into vendor/, with patches/gpui-*.patch applied
cargo run --release -p crow-app  # opens your home folder
./scripts/bundle.sh --open       # builds and opens target/bundle/Crow.app
./scripts/bundle.sh --install    # ...and copies it to /Applications (CROW_INSTALL_DIR to change)
```

Running as `Crow.app` makes macOS privacy prompts name Crow instead of your terminal. With a code-signing certificate named
"Crow Dev" in your keychain (see `scripts/bundle.sh`), macOS keeps those permissions from one build to the next.

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
| `settings.toml` | Theme, keymap (`"finder"` or `"vim"`), key hints (`"auto"`, `"always"` or `"off"`), motion (`"auto"`, `"off"`, `"low"` or `"full"`), sounds (`"crow"`, `"finder"` or `"off"`), font, hidden files (shown by default), the preview (`show_preview`), whether views follow folders (`view_per_folder`), window size |
| `views.toml` | How folders are shown, in three layers: `[defaults]`, `[[rule]]` path patterns, and `[folder."…"]` for one folder. Changing a folder's view or sort saves it as that folder's own setting. View ▸ Why This View? (Cmd-J) shows which layer decided what. |
| `sidebar.toml` | Groups of favorites (with optional `name`, `icon`, `color` and Vim mark `key`), and whether Recent and Locations show |
| `exclude.toml` | Folders search skips, in named sets: `dev` is on by default (`node_modules`, `target` beside `Cargo.toml`, `.git`, `build`/`dist` beside manifests, …); `dependencies`, `media`, `hidden` and `system` are off until enabled. The search footer counts results and files, and lists each skipped folder with the rule that skipped it. |

Themes come from the global index in `~/.config/themes`. View ▸ Theme… (Cmd-Option-T)
previews each theme as you move through the list; Apply to All Apps (Cmd-Return) sets it for every app that reads the index.

Motion follows `motion`: `"auto"` (default) uses Full unless macOS Reduce Motion is on, then Low;
`"off"` changes state in one frame; `"low"` fades opacity and color only; `"full"` also moves —
a trashed row's icon becomes a trash and swallows the name, a move pulses its target, results
arrive, the selection fill glides between rows, and sheets and tips fade in. `sound` picks
`"crow"` (Finder's trash sounds plus soft ticks for move, new folder, undo and copy), `"finder"`
(only the sounds Finder itself makes) or `"off"`. The two are separate settings; macOS
"Play user interface sound effects" off silences everything but the alert. `crow --motion-lab`
opens a fake folder to try every mode and sound set (see below).

Recent folders are kept in `~/.local/state/crow/recent.json`, merged with zoxide's list when it's installed.
Commands run from the palette are remembered in `~/.local/state/crow/palette-commands.json`.
Key-use counts, which retire idle bottom-bar hints, live next to it in `~/.local/state/crow/usage.json`.
Logs go to `~/Library/Logs/Crow/crow.log`.

## Keys

Finder's keys work as in Finder: arrows (Shift extends), Option-Up/Down, Cmd-Up/Down,
Cmd-[ and Cmd-], Cmd-A, Cmd-T, Cmd-W, Cmd-Shift-[ and ], Cmd-Shift-G (opens Cmd-K on this folder's
path: type a name, Tab completes, Return goes there or selects a file),
Cmd-Shift-. and Space. Cmd-Shift-P shows or hides the preview pane on the right (in list and
column view, search included; Quick Look still works). Typing a name selects it. Tab moves focus between the sidebar and the list.
Column headers sort (click again to reverse) and their dividers resize.
Type `/` to filter the focused folder: the field opens as the first row of the focused column
(or under the list header), showing the query and "n of N", and matching letters are highlighted.
Esc clears it, arrows move within the matches, and Return opens the selection. Cmd-F carries the
filter text into a search of that folder. Cmd-2 shows a list, Cmd-3 columns. A window shows each
folder in that folder's view, so moving into a folder with its own view (or a `views.toml` rule)
switches list and columns; with `view_per_folder = false` in `settings.toml` a window keeps the
view you give it, and Cmd-2/Cmd-3 save nothing. In the list, Right opens the selected folder inline (Right again goes into it); Left steps to the row that contains a nested one, closes an open folder, or goes up from the top level. In columns, Left and Right move between them, and Left in the leftmost column goes up a folder. Vim `h`/`l` follow the same keys.

Sidebar: drag favorites to reorder them (or Cmd-Option-Up/Down), drop files on any row to move or
copy them there, drop folders between two favorites (a line shows where) or on a group's title to add
them. Ctrl-Cmd-T adds the selected folder.
Same-named folders show their parent after the name (and "alias" for a symlink); with
`keymap = "vim"`, favorites with a `key` show the letter on the right. Locations shows free space
and a bar that warns near full. In the sidebar, Cmd-Delete removes a favorite or forgets a recent
folder, Cmd-E ejects a disk, and Option-Up/Down jump between groups. Tab into the sidebar and arrow:
the folder shows in the center while focus stays in the sidebar.

With `keymap = "vim"`: j/k, h/l (parent and open, or columns), gg/G, yy/dd/p (copy, cut, paste),
u and Ctrl-R, cw renames, `-` goes up, `m x` marks the folder as favorite x and `'x` jumps there.

Menus list every command with its key and grey out what the focused view cannot do: File,
Edit (including Filter This Folder, Find… and Find Everything…), View, Go (Back, Forward,
Enclosing Folder, Go to…, Go to Folder…), Window and Help. Use as Defaults, Reset View for
This Folder and Zoom have no shortcut. Cmd-Shift-A from anywhere opens search with every
`exclude.toml` set off. Shortcuts shown in the command palette, tooltips and hints come from
the live keymap, so they follow `keymap = "vim"` too. Hover a nav arrow, a view button, the Go to button, the search field, a path segment, a disk row,
Search everything or Cancel during an operation to see its name and key. Sheets
(Why This View?, the theme picker, Cmd-K and the delete/conflict questions) dim the window;
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

A contextual tip appears once, bottom right above the bar, when you visibly do something the
slow way: scrolling a folder of 200+ items ("Type / to filter this folder by name."), two filters
in a row that matched nothing ("⌘F searches subfolders and file contents too."), or typing a whole
path in the palette for a folder it already lists ("⌘K finds recent and favorite folders by a few
letters."). It retires the moment it shows; Got it, using its key, changing folder or opening an
overlay closes it. The retired ids live in `~/.local/state/crow/tips.json`; `hints = "off"` hides
tips as well.

Closing the last window or quitting while an operation runs asks first: Esc keeps it running,
Return stops it cleanly (finished items stay and Undo can reverse them; the item in progress is never left half done) and then closes or quits. In every text field
⌥←/⌥→ move by word, ⌥⇧←/→ select by word and ⌥⌫ deletes a word. In list view the Name column
takes whatever width the window gives; the other columns keep the widths you drag them to, and
dragging Name's own divider trades width with the column after it.

Toolbar icons are [Lucide](https://lucide.dev) 1.72.0, ISC-licensed; the notice is at
`crates/crow-app/assets/icons/LICENSE` (also copied into `Crow.app/Contents/Resources/`).
Letters no longer select by name in this layer.

File operations use Finder's keys: Return renames, Cmd-Shift-N new folder, Cmd-Option-N new file,
Cmd-D duplicate, Cmd-Delete moves to the Trash, Cmd-Option-Delete deletes permanently (after asking),
Cmd-C / Cmd-X / Cmd-V copy, cut and paste (also to and from Finder), Cmd-Option-V moves,
Cmd-Z / Cmd-Shift-Z undo and redo, Esc cancels a running operation, Cmd-Option-L shows activity.
Nothing is ever overwritten: name clashes ask first, and Replace moves the old item to the Trash.

Cmd-K opens the command palette over a scrim: folders are grouped into Recent, Favorites and
In <folder>, each row showing the name and the abbreviated path, so two folders called `git` are
told apart; matched letters are accent in both lines and Recent rows carry the last visit.
Return goes there and Cmd-Return opens a folder in a new tab. Tab appends the highlighted
folder's path and `/`, so the next letters search inside it; typing `/` or `~/` does the same by
hand, with the parent's subfolders listed. Start with `>`: it becomes a Commands token
(Backspace on the empty field goes back to folders) and rows show the menu, the label, an on/off
chip for toggles and their keys. Commands you run from the palette are remembered under Recently
used; `palette_recent_commands = false` hides that section and stops recording.
Cmd-F, or the top band's Search field, searches names and contents in the current folder (in
columns, the focused column, not the subfolder previewed to its right: press Right first to search
that one); the same
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

Dev tools: `crow --motion-lab` opens a fake folder where every feedback event (trash, move,
undo, new folder, refuse, copy, tip, empty Trash) can be fired in the three motion modes and
three sound sets, with slow motion ×5 and the cursor glide — the reference the app's motion
is tuned against. `crow --bench DIR` and the other `--bench-*` flags are listed in
[docs/testing.md](docs/testing.md#performance).
