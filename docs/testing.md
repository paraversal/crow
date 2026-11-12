# Testing Crow

## Automated

```bash
cargo test --workspace          # unit tests and GPUI keystroke tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

The keystroke tests (`crates/crow-app/src/window.rs`) drive a real window through
GPUI's test platform: key contexts, history, tabs, hidden files, multiple
selection, type-to-select, sorting, Go to Folder, the column view's focus rules, file operations
(new folder and rename, trash and undo, duplicate, copy and paste, conflicts, permanent delete),
drops, the sidebar (reordering, removing, Add to Sidebar, drops on favorites and group titles, Recent),
view settings (per-folder memory, live rules, Why This View?, broken files), the theme picker
(preview, revert, save), the `/` filter, the command palette, the search panel and the Vim layer
(motions, marks). Each test gets its own temp config folder.

`crates/crow-core` tests the config models on their own: view layers and glob rules, `exclude.toml`
defaults and validation, editing `views.toml` and `sidebar.toml` without losing comments, and
frecency with zoxide's scores.

`crates/crow-search` tests the engine on temp trees: dev rules and sentinels skip and are counted,
Search everything walks them, hidden files follow the setting, fuzzy names score, literal and
`/regex/` contents report the line, text and ranges, binary files are not read, and cancelling stops
the stream. The search panel adds keystroke tests for names and contents, Search everything,
navigating and revealing results, copy and trash, and the Vim layer.

The operation engine (`crates/crow-fs/src/ops_tests.rs`) is tested against real temp folders:
every operation and its undo, each conflict choice, cancelling midway, a read-only destination,
a source vanishing mid-operation, undo refusing to overwrite, a forced cross-volume move,
and 40 randomized rounds of "do, undo, compare the tree".

## Performance

```bash
cargo run --release -p crow-app -- --make-fixture target/fixture-100k 100000
cargo run --release -p crow-app -- --bench target/fixture-100k
cargo run --release -p crow-fs --example list_bench -- target/fixture-100k
cargo run --release -p crow-app -- --make-search-fixture target/fixture-search-200k 200000
cargo run --release -p crow-app -- --bench-search target/fixture-search-200k
```

`--bench` scrolls 600 frames and prints first paint, load time and frame times.
It needs a GPU, so it runs locally, not in CI.

| Measure | Budget |
| --- | --- |
| First rows on screen, 100k folder (warm app) | under 50 ms (M3: 9–10 ms; M5: 10–11 ms; M5-fix: 11 ms; M5-fix-2: 10 ms; M6: 10 ms) |
| Scroll, 99th percentile frame | under 1.5 × the display's frame time (M3: 8.6–8.8 ms at 120 Hz; M5: 9.7–13 ms, main on the same machine 8.8–14.6 ms; M5-fix: 8.9 ms; M5-fix-2: 9.0 ms; M6: 8.9–9.1 ms) |
| Search, 200k nested files, every file matches (names + contents) | first hit 4 ms; total 4.1 s (M6). `--bench-search` includes the walk, fuzzy names and content reads. |

## By hand (each milestone)

Run `scripts/bundle.sh --open` so macOS treats Crow as an app.

| Check | How |
| --- | --- |
| Drag out | Drag one row, then a multiple selection, onto a Finder window and into a Mail message. The image is the same small icon plus name pill inside and outside Crow (right edge at the cursor if the grab point is past it); leaving and re-entering the window keeps that one image, with no clipped frame or second icon; dropping back inside Crow moves the item. List and column rows keep one steady drop target while the pointer rests on them. |
| Quick Look | Select a file, press Space. Arrow keys while it is open move it along. |
| Column view | Cmd-3: arrows move within a column, Right moves into the selected folder, Left moves back; Left in the leftmost column goes up a folder. Only the focused column's selection is in the accent color. Selecting a file shows its preview as the last column. Cmd-2 returns to the list in the folder you were in. |
| Trash and Put Back | Cmd-Delete a file in Crow, then use Finder's Put Back on it in the Trash. |
| Finder clipboard | Copy in Crow, paste in Finder; copy in Finder, paste in Crow. |
| Drop in | Drag files from Finder onto a Crow list, onto a column, and onto sidebar rows and group titles: the target keeps one steady highlight while the pointer moves within it; same disk moves, other disk copies, Option copies. Drag a row from Crow's own list the same way. |
| Progress and cancel | Copy a large folder to another disk: progress shows in the status line; Esc or Cancel stops it with nothing half-copied. |
| Activity | Cmd-Option-L lists recent operations; Undo on an older one works or explains why not. |
| Path bar | Click a folder segment to go there (Back returns); click the selected file's segment to open it. |
| Table | Drag header dividers to resize; click headers to sort and reverse. |
| Preview pane | Select an image, a PDF, a text file and a folder: thumbnail, text, icon. Tags show for tagged files. |
| Live updates | With a folder open, create, rename and delete files from Terminal; the list follows without jumping. |
| Live theme | Switch macOS light/dark, or edit a scheme in the theme index: colors fade over about half a second (faster with Reduce Motion). |
| Theme picker | Cmd-Option-T, arrow through themes (each shows at once), Esc returns; Return keeps one and writes it to `settings.toml`. Apply to All Apps changes `~/.local/state/themes/current.json`. |
| Favorites | Drag a favorite up and down: one insertion line shows the exact target (under a group title for that group's start), the line never shifts rows, and the end of a group is reachable; dropping on a group title still moves the favorite to its start. Drag a folder from the list onto a group title; drop a file onto a favorite. Check `sidebar.toml` keeps your comments. Add `icon = "🚀"` and `color = "blue"` to one. |
| Recent | Visit a few folders (and `cd` around with zoxide): they appear under Recent, favorites excluded. |
| Locations | Mount a disk image: it appears without a restart, with a free-space bar; ⏏ ejects it. |
| View settings | Cmd-3 in one folder, then visit another: it keeps its own view; going up in column view stays in columns. Add a `[[rule]]` to `views.toml` with the folder open: it applies at once. Cmd-J explains each value. |
| Inline folders | In the list, the chevron (›/⌄) opens and closes a folder inline; Right opens a folder indented under it (Right again goes in), Left steps to the containing row, Left again closes it. Option-click is a normal click. Select across levels (Cmd-click or Shift-click a nested row) and copy or trash: every selected row is included. Navigate away and back: expansions are gone. |
| Vim layer | Set `keymap = "vim"`: hjkl, gg/G, `m a` then `'a` from elsewhere. |
| Filter | Press `/` in a folder: the field and "n of N" show; typing narrows the rows; arrows move within matches; Return opens; Esc restores focus and rows. Try it in column view, and open a folder inline first (the filter is flat until cleared). |
| Command palette | Cmd-K: type a recent folder and Return; try Cmd-Return for a new tab and `>` for commands; Vim j/k moves the cursor. |
| Search | Cmd-F, search a word in file names and in contents: rows show the matching line with highlights; switch This Folder / Home / Everywhere; Return, Cmd-R, Space, drag-out, Cmd-C and Cmd-Delete act on results. |
| Skipped folders | Search a folder with `node_modules`: the summary counts skipped folders; click it and check each folder is listed with the rule that skipped it. Cmd-Shift-A (Search everything) includes them. |
| Spotlight | Search "Everywhere" for a name outside the home folder: results stream in and the summary says Spotlight only sees indexed volumes. |
| VoiceOver | Cmd-F5, move through the list: each row reads name, details and "n of N". |
| Privacy prompts | Open Desktop, Documents, Downloads: the prompt names Crow. |
