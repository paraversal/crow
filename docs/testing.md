# Testing Crow

## Automated

```bash
cargo test --workspace          # unit tests and GPUI keystroke tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

The keystroke tests (`crates/crow-app/src/window.rs`) drive a real window through
GPUI's test platform: key contexts, history, tabs, hidden files, multiple
selection, type-to-select, sorting and Go to Folder.

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
| First rows on screen, 100k folder (warm app) | under 50 ms (M2: 7 ms) |
| Scroll, 99th percentile frame | under 1.5 × the display's frame time (M2: 8.5–9.3 ms at 120 Hz) |

## By hand (each milestone)

Run `scripts/bundle.sh --open` so macOS treats Crow as an app.

| Check | How |
| --- | --- |
| Drag out | Drag one row, then a multiple selection, onto a Finder window and into a Mail message. |
| Quick Look | Select a file, press Space. Arrow keys while it is open move it along. |
| Preview pane | Select an image, a PDF, a text file and a folder: thumbnail, text, icon. Tags show for tagged files. |
| Live updates | With a folder open, create, rename and delete files from Terminal; the list follows without jumping. |
| Live theme | Switch macOS light/dark, or edit a scheme in the theme index. |
| VoiceOver | Cmd-F5, move through the list: each row reads name, details and "n of N". |
| Privacy prompts | Open Desktop, Documents, Downloads: the prompt names Crow. |
