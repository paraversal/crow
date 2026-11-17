# Crow

A Finder-style file explorer for macOS, written in Rust on [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui).
It keeps Finder's directness and aims to be faster, more predictable and keyboard-first.

Status: Phase 1 (the "daily driver" MVP) is done: M6, the UI overhaul, M6.5 (the settings window) and M7 (hardening) are merged. Next is M8 (Finder parity and polish), then M8.5 (search reliability); Phase 2 follows in M9. See [`docs/ROADMAP.md`](docs/ROADMAP.md) for every milestone and item.

## Build and run

```bash
./scripts/fetch-gpui.sh          # once: Zed's GPUI at a pinned commit, into vendor/, with patches/gpui-*.patch applied
cargo run --release -p crow-app  # opens your home folder
./scripts/bundle.sh --open       # builds and opens target/bundle/Crow.app
./scripts/bundle.sh --install    # ...and copies it to /Applications, installs `crow` in ~/.local/bin
```

Running as `Crow.app` makes macOS privacy prompts name Crow instead of your terminal. With a code-signing certificate named
"Crow Dev" in your keychain (see `scripts/bundle.sh`), macOS keeps those permissions from one build to the next.
The app icon's sources are SVGs in `crates/crow-app/assets/app-icon`; after editing one, run
`swift scripts/render-app-icons.swift` to redraw the PNGs the app and bundle use.

## Command line

`./scripts/bundle.sh --install` also installs the `crow` command into `$HOME/.local/bin`
(or `CROW_BIN_DIR`), creating the directory if needed. It prints a note if that directory
isn't on your PATH.

```bash
crow ~/Downloads          # a new window on the folder
crow .                    # the current folder
crow notes.txt            # its folder, with the file selected
crow                      # bring Crow to the front
```

The command is a small shell script that runs `open -a /Applications/Crow.app`; it is not a
copy of the app, so rebuilding or reinstalling Crow at that path needs no update. If you move
Crow, set `CROW_APP` or reinstall the command. The `crow` binary itself takes a directory too
(`cargo run --release -p crow-app -- DIR`), but launching the bundle through `open -a` keeps
macOS privacy prompts naming Crow.

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
| `settings.toml` | Theme, keymap (`"finder"` or `"vim"`), what Return does (`enter`: `"rename"` or `"open"`), key hints (`"auto"`, `"always"` or `"off"`), tips (`"auto"` or `"off"`), motion (`"auto"`, `"off"`, `"low"` or `"full"`), sounds (`"crow"`, `"finder"` or `"off"`), the app icon (`app_icon`: `"refined"`, `"sun"`, `"graphite"` or `"bone"`; the Dock switches live, `bundle.sh` bakes it into Crow.app), font, hidden files (shown by default), the preview (`show_preview`), whether views follow folders (`view_per_folder`), whether the palette remembers recent commands (`palette_recent_commands`), window size. ⌘, opens the Settings window for all of these; the file stays authoritative and the window follows edits made in a text editor |
| `views.toml` | How folders are shown, in three layers: `[defaults]`, `[[rule]]` path patterns, and `[folder."…"]` for one folder. Changing a folder's view or sort saves it as that folder's own setting. View ▸ Why This View? (Cmd-J) shows which layer decided what. |
| `sidebar.toml` | Groups of favorites (with optional `name`, `icon`, `color` and Vim mark `key`), and whether Recent and Locations show |
| `exclude.toml` | Folders search skips, in named sets: `dev` is on by default (`node_modules`, `target` beside `Cargo.toml`, `.git`, `build`/`dist` beside manifests, …); `dependencies`, `media`, `hidden` and `system` are off until enabled. The search footer counts results and files, and lists each skipped folder with the rule that skipped it. |

⌘, (or Crow ▸ Settings…) opens one Settings window. It edits `settings.toml` only: the file stays the single source of truth, every change writes just that key (comments survive), and edits made in a text editor appear in the window. Its Files section opens `settings.toml`, the config folder, and the other three files. ⌘K's "Settings" opens the window and "Edit settings.toml" opens the file directly.

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
Key-use counts, which retire idle bottom-bar hints, live next to it in `~/.local/state/crow/usage.json`, and the tips you have already seen in `tips.json`.
Logs go to `~/Library/Logs/Crow/crow.log`.

## Toolbar

The top band holds Back and Forward (drawn chevrons, faded at the ends of history), the folder
title, the list/columns switch, "Go to ⌘K" and the search field "Search <folder> ⌘F". Every
control has a tooltip that names it and its key; a disabled control is the same icon at 40 %
opacity. While searching, the field grows across the band with All / Names / Contents and Done esc.
Toolbar icons are [Lucide](https://lucide.dev) 1.72.0, ISC-licensed; the notice is at
`crates/crow-app/assets/icons/LICENSE` (also copied into `Crow.app/Contents/Resources/`).

## Menus

File, Edit, View, Go, Window and Help list every command with its key — always the Finder key,
also in the Vim layer — and grey out what the focused view cannot do. Edit carries Filter This
Folder, Find… and Find Everything…; Go carries Back, Forward, Enclosing Folder, Go to…, Go to
Folder…, Edit Path and Reveal Result. Find Everything… (⇧⌘A) opens search with every `exclude.toml` set off.
Use as Defaults, Reset View for This Folder and Zoom have no shortcut. Sheets (Why This View?,
the theme picker, Cmd-K and the delete/conflict questions) dim the window; clicking outside one
cancels it exactly like Esc.

## Hints and tips

The bottom bar has three slots: the path (⌘L or a double-click on its empty space edits it in
place), a transient one (type-to-select letters, hints, operation progress, messages, errors) and
counts. While you type a name, the transient slot shows the letters so far, left of the hints. Hints follow
focus: the path field (`↩` go, `⇥` complete, `esc` cancel), the filter field (Return to matches,
Esc clear, `⌘F` search deeper), the sidebar (`⌥↑↓` groups, `⇥`/`→` to list, `⌘⌥↑↓` reorder,
`⌘⌫` remove, `⌘E` eject a disk),
search results and the search field. The idle hint shows `/` filter (Search and Go to carry their
keys on the toolbar); after you have used it three times it retires (`hints = "always"` keeps every
hint, `"off"` hides hints and tips). Tips have their own switch: `tips = "off"` mutes them and
leaves the key hints alone. While an operation runs, a 2 px accent line along the bar's
top edge and "Copying … · n %" with Cancel esc replace the hints; the path and count stay. A broken
config file shows an error pill with its file name, line and an Open button, beside progress rather
than instead of it. Counts name the focused folder ("18 items in Documents"), the selection and its
size, or the cursor's sidebar group (while filtering, the field itself says "n of N"). When room
runs short the path keeps its first and last segments: its middle folds into a "…" (click it for
the deepest hidden folder), then hints drop from the right, then a long progress line or error is
cut with an ellipsis.

A contextual tip appears once, bottom right above the bar, when you visibly do something the slow
way: scrolling a folder of 200+ items ("Type / to filter this folder by name."), two filters in a
row that matched nothing ("⌘F searches subfolders and file contents too."), or typing a whole path
in the palette for a folder it already lists ("⌘K finds recent and favorite folders by a few
letters."). It retires the moment it shows; Got it, using its key, changing folder or opening an
overlay closes it. "Don't show tips" on the card mutes every later tip. The first launch asks
"Show tips as you go?" once; the answer lives in `~/.local/state/crow/onboarding.json`, so
deleting `settings.toml` does not ask again. The retired ids live in `~/.local/state/crow/tips.json`.

## Palette

Cmd-K opens the command palette over a scrim: folders are grouped into Recent, Favorites and
In <folder>, each row showing the name and the abbreviated path, so two folders called `git` are
told apart; the home folder matches its account name as well as `~`. Matched letters are accent in
both lines and Recent rows carry the last visit.
Return goes there and Cmd-Return opens a folder in a new tab. Tab appends the highlighted
folder's path and `/`, so the next letters search inside it; typing `/` or `~/` does the same by
hand, with the parent's subfolders listed, and files whose name starts with what follows the last `/`
(Return shows the file selected in its folder). Start with `>`: it becomes a Commands token
(Backspace on the empty field goes back to folders) and rows show the menu, the label, an on/off
chip for toggles and their keys. Commands you run from the palette are remembered under Recently
used; `palette_recent_commands = false` hides that section and stops recording.

## Search

Cmd-F, or the top band's Search field, searches names and contents in the current folder (in
columns, the focused column, not the subfolder previewed to its right: press Right first to search
that one); the same field moves into the band while searching, where All / Names / Contents choose
what to match (Names reads no file contents) and Cmd-Shift-M cycles the three. Cmd-F again focuses
the field and selects the query; Tab goes field → results → sidebar → field (no results: straight
to the sidebar); Done or Esc (from anywhere) closes, and with the skipped list open Esc closes that
first. The scope bar switches the folder, Home or This Mac (Spotlight, names only); its switch
skips the folders named in `exclude.toml` sets, as Cmd-Shift-A does for every set. Name matches
whose name contains the query come first with their letters highlighted; scattered fuzzy name hits
follow the content hits. A name match stays one line, a content-only hit shows its line number and
line under the name, and the footer counts results and files and lists what was skipped (relative
to the folder). With nothing typed, four examples fill the field. The right preview follows the
selected result in list and column view, at the hit line for content hits. Return in the field moves to the
first result; results open with Return, reveal with Cmd-R, Quick Look with Space, and take the usual copy, cut and trash keys.

## Keys

Finder's keys work as in Finder: arrows (Shift extends), Option-Up/Down, Cmd-Up/Down,
Cmd-[ and Cmd-], Cmd-A, Cmd-T, Cmd-W, Cmd-Shift-[ and ], Cmd-Shift-G (opens Cmd-K on this folder's
path: type a name, Tab completes, Return goes there or selects a file; a pasted path may be quoted,
shell-escaped or a `file://` URL), Cmd-L (or a double-click on empty space in the bottom bar) edits
the path the bar shows in place: Tab completes, Return goes there or selects a file, Esc puts the path
back, Cmd-, (Settings), Cmd-Shift-. and Space. Cmd-K's
"Settings" opens the window and "Edit settings.toml" opens the file directly. Cmd-Shift-P shows
or hides the preview pane on the right (in list and column view, search included; Quick Look still
works). Typing a name selects it and the bottom bar shows the letters. Tab moves focus between the
sidebar and the list.
Column headers sort (click again to reverse) and their dividers resize.
Type `/` to filter the focused folder: the field opens as the first row of the focused column
(or under the list header), showing the query and "n of N", and matching letters are highlighted.
Esc clears it, arrows move within the matches, and Return moves focus into them on the selected
match (the first unless you arrowed); there Cmd-Down opens it and Esc clears the filter. Cmd-F carries the
filter text into a search of that folder. Cmd-2 shows a list, Cmd-3 columns. A window shows each
folder in that folder's view, so moving into a folder with its own view (or a `views.toml` rule)
switches list and columns; with `view_per_folder = false` in `settings.toml` a window keeps the
view you give it, and Cmd-2/Cmd-3 save nothing. In the list, Right opens the selected folder inline (Right again goes into it); Left steps to the row that contains a nested one, closes an open folder, or goes up from the top level. In columns, Left and Right move between them, and Left in the leftmost column goes up a folder. Vim `h`/`l` follow the same keys.

Sidebar: drag favorites to reorder them (or Cmd-Option-Up/Down), drop files on any row to move or
copy them there, drop folders between two favorites (a line shows where) or on a group's title to add
them. Ctrl-Cmd-T adds the selected folder.
Same-named folders show their parent after the name (and "alias" for a symlink); Recent leaves out
folders that are favorites or already listed, also when reached through a symlink. With
`keymap = "vim"`, favorites with a `key` show the letter on the right. Locations shows free space
and a bar that warns near full. In the sidebar, Cmd-Delete removes a favorite or forgets a recent
folder, Cmd-E ejects a disk, and Option-Up/Down jump between groups. Right-click or Ctrl-click a row for
Open, Open in New Tab, Copy Path, Show in Finder and the row's own lines (Move Up/Down and Remove
for favorites, Add to Sidebar and Remove from Recent for recent folders, Eject for disks). Tab into the sidebar and arrow:
the folder shows in the center while focus stays in the sidebar.

With `keymap = "vim"`: j/k, h/l (parent and open, or columns), gg/G, yy/dd/p (copy, cut, paste),
u and Ctrl-R, cw renames, `-` goes up, `m x` marks the folder as favorite x and `'x` jumps there. Letters no longer select by name
in this layer.

Shortcuts shown in the command palette, tooltips and hints come from the live keymap, so they
follow `keymap = "vim"` too.

Closing the last window or quitting while an operation runs asks first: Esc keeps it running,
Return stops it cleanly (finished items stay and Undo can reverse them; the item in progress is never left half done) and then closes or quits. In every text field
⌥←/⌥→ move by word, ⌥⇧←/→ select by word and ⌥⌫ deletes a word. In list view the Name column
takes whatever width the window gives; the other columns keep the widths you drag them to, and
dragging Name's own divider trades width with the column after it.

File operations use Finder's keys: Return renames (with `enter = "open"` it opens instead; F2
always renames), Cmd-Shift-N new folder, Cmd-Option-N new file,
Cmd-D duplicate, Cmd-Delete moves to the Trash, Cmd-Option-Delete deletes permanently (after asking),
Cmd-C / Cmd-X / Cmd-V copy, cut and paste (also to and from Finder), Cmd-Option-V moves,
Cmd-Z / Cmd-Shift-Z undo and redo, Esc cancels a running operation, Cmd-Option-L shows activity.
Copying or cutting more than 1000 items keeps them in Crow's own clipboard (the status says
"Crow only"); smaller copies also go on the system pasteboard for Finder and other apps.
Cmd-Option-C copies the selected items' paths (one per line; the folder's own path with nothing
selected). Nothing is ever overwritten: name clashes ask first, and Replace moves the old item to the Trash.

Clicking empty space in the list deselects (focus stays on the list); in column view it also closes
the columns to the right of the one clicked. Right-click or Ctrl-click opens a native menu with the
same names and keys as the menu bar: on items, Open, Quick Look, Rename, Duplicate, Copy, Cut,
Paste, Copy Path, Show in Finder, Add to Sidebar and Move to Trash (an unselected row is selected
first, a row in the selection keeps it); on empty space, New Folder, New File, Paste and Show in
Finder (opens the folder itself in Finder). Search results have their own menu: Open, Reveal in
Folder, Quick Look, Copy, Cut, Copy Path and Move to Trash.

Testing, benchmarks and the hand-test checklist: [docs/testing.md](docs/testing.md).
Fuzzing: `cargo test -p crow-app fuzz_smoke`; long runs and replay: [docs/testing.md#fuzzing](docs/testing.md#fuzzing). For real frame times, `crow --fuzz` (built with `--features fuzz`) reports any step over its budget (50 ms; 100 ms for a bulk step acting on more than 10 000 paths) and fails on a heartbeat over 250 ms.

Dev tools: `crow --motion-lab` opens a fake folder where every feedback event (trash, move,
undo, new folder, refuse, copy, tip, empty Trash) can be fired in the three motion modes and
three sound sets, with slow motion ×5 and the cursor glide — the reference the app's motion
is tuned against. `crow --bench DIR` and the other `--bench-*` flags are listed in
[docs/testing.md](docs/testing.md#performance).
