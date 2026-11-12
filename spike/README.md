# Crow Phase 0 spike

Throwaway code that answers one question: can GPUI carry a Finder-class file
browser? The reusable part is `crates/crow-theme`.

The spike builds against Zed's own GPUI, which has AccessKit accessibility
(the crates.io release, 0.2.2, has none). Fetch it once (about 19 MB, pinned commit):

```bash
./scripts/fetch-gpui.sh
```

```bash
cargo run --release -p crow-spike -- ~/Downloads
```

Make the large-folder fixture and run the scroll benchmark:

```bash
cargo run --release -p crow-spike -- --make-fixture target/spike-100k 100000
cargo run --release -p crow-spike -- --bench target/spike-100k
```

## Exit criteria and how to check each

| # | Criterion | How to check |
| --- | --- | --- |
| 1 | 100k-row folder scrolls at display refresh rate, with live updates | `--bench` prints frame times. For live updates, run `touch target/spike-100k/zzz` while the folder is open: the item count in the path bar goes up. |
| 2 | Drag a file into Mail and into Finder | Drag a row out of the list onto a Finder window, the Desktop, or a Mail compose window. |
| 3 | Native Quick Look panel | Select a file, press Space. Arrow keys while it is open retarget it. |
| 4 | Live theme switch through the theme index | Toggle macOS light/dark, or edit a scheme file in the dotfiles repo: colors change without a restart. To test `current.json` without touching your real global theme, start with `THEMES_STATE_DIR=$(mktemp -d)` and write a `current.json` there. |
| 5 | Keymap contexts | Tab moves focus between sidebar and list. Return in the sidebar opens the favorite; Return in the list reports "rename". Up/Down move a different cursor in each. |
| 6 | VoiceOver reads the file list | Start with `CROW_A11Y_DUMP=target/a11y-tree.json`, press Cmd-F5, move through the list with the arrow keys. VoiceOver should read each row as "name, kind, size, modified …, n of N". The dump shows the tree VoiceOver received. |

## Keys

| Key | File list | Sidebar |
| --- | --- | --- |
| Up / Down | Move selection | Move sidebar cursor |
| Return | Rename (stub) | Open favorite |
| Cmd-Down | Open folder or file | |
| Cmd-Up | Parent folder | Parent folder |
| Space | Quick Look | |
| Tab | Focus sidebar | Focus list |
| Cmd-Shift-. | Toggle hidden files | Toggle hidden files |
