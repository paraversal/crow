# Testing Crow

## Automated

```bash
cargo test --workspace          # unit tests and GPUI keystroke tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

The keystroke tests (`crates/crow-app/src/window.rs`) drive a real window through
GPUI's test platform: key contexts, history, tabs, hidden files, multiple
selection, type-to-select, sorting, Go to Folder, the column view's focus rules and file operations
(new folder and rename, trash and undo, duplicate, copy and paste, conflicts, permanent delete).

The operation engine (`crates/crow-fs/src/ops_tests.rs`) is tested against real temp folders:
every operation and its undo, each conflict choice, cancelling midway, a read-only destination,
a source vanishing mid-operation, undo refusing to overwrite, a forced cross-volume move,
and 40 randomized rounds of "do, undo, compare the tree".

## Performance

```bash
cargo run --release -p crow-app -- --make-fixture target/fixture-100k 100000
cargo run --release -p crow-app -- --bench target/fixture-100k
cargo run --release -p crow-fs --example list_bench -- target/fixture-100k
```

`--bench` scrolls 600 frames and prints first paint, load time and frame times.
It needs a GPU, so it runs locally, not in CI.

| Measure | Budget |
| --- | --- |
| First rows on screen, 100k folder (warm app) | under 50 ms (M3: 9–10 ms) |
| Scroll, 99th percentile frame | under 1.5 × the display's frame time (M3: 8.6–8.8 ms at 120 Hz) |

## By hand (each milestone)

Run `scripts/bundle.sh --open` so macOS treats Crow as an app.

| Check | How |
| --- | --- |
| Drag out | Drag one row, then a multiple selection, onto a Finder window and into a Mail message. |
| Quick Look | Select a file, press Space. Arrow keys while it is open move it along. |
| Column view | Cmd-3: arrows move within a column, Right moves into the selected folder, Left moves back. Only the focused column's selection is in the accent color. Selecting a file shows its preview as the last column. Cmd-2 returns to the list in the folder you were in. |
| Trash and Put Back | Cmd-Delete a file in Crow, then use Finder's Put Back on it in the Trash. |
| Finder clipboard | Copy in Crow, paste in Finder; copy in Finder, paste in Crow. |
| Drop in | Drag files from Finder onto a Crow list and onto a column: same disk moves, other disk copies, Option copies. |
| Progress and cancel | Copy a large folder to another disk: progress shows in the status line; Esc or Cancel stops it with nothing half-copied. |
| Activity | Cmd-Option-L lists recent operations; Undo on an older one works or explains why not. |
| Table | Drag header dividers to resize; click headers to sort and reverse. |
| Preview pane | Select an image, a PDF, a text file and a folder: thumbnail, text, icon. Tags show for tagged files. |
| Live updates | With a folder open, create, rename and delete files from Terminal; the list follows without jumping. |
| Live theme | Switch macOS light/dark, or edit a scheme in the theme index. |
| VoiceOver | Cmd-F5, move through the list: each row reads name, details and "n of N". |
| Privacy prompts | Open Desktop, Documents, Downloads: the prompt names Crow. |
