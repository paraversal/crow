# Keys

Generated from Crow's keymap by `cargo test -p crow-app keys_doc` (`CROW_UPDATE_DOCS=1` rewrites it); do not edit by hand.

Every command is in the menu bar and in the palette (`>`). The Finder layer is the default; `keymap = "vim"` adds the Vim layer, which keeps the Finder keys for menu commands. Rebind any row in Settings ▸ Keyboard or in `keys.toml` with the id in the last column.

Movement keys (arrows, Return, Space and Esc in lists, the sidebar, search results and sheets) are not rebindable.

## Crow

| Command | Finder | Vim layer | Id |
| --- | --- | --- | --- |
| Quit Crow | `⌘Q` |  | `quit` |

## File

| Command | Finder | Vim layer | Id |
| --- | --- | --- | --- |
| New Window | `⌘N` |  | `new-window` |
| New Tab | `⌘T` |  | `new-tab` |
| New Folder | `⇧⌘N` |  | `new-folder` |
| New File | `⌥⌘N` |  | `new-file` |
| Open Selected | `⌘↓` |  | `open` |
| Rename | `F2` | `cw` | `rename` |
| Duplicate | `⌘D` |  | `duplicate` |
| Quick Look | `Space` |  | `quick-look` |
| Move to Trash | `⌘⌫` |  | `move-to-trash` |
| Delete Permanently | `⌥⌘⌫` |  | `delete-permanently` |
| Empty Trash… | `⇧⌘⌫` |  | `empty-trash` |
| Add to Sidebar | `⌃⌘T` |  | `add-to-sidebar` |
| Eject Volume | `⌘E` |  | `eject` |

## Edit

| Command | Finder | Vim layer | Id |
| --- | --- | --- | --- |
| Undo | `⌘Z` |  | `undo` |
| Redo | `⇧⌘Z` |  | `redo` |
| Cut | `⌘X` | `dd` | `cut` |
| Copy | `⌘C` | `yy` | `copy` |
| Paste | `⌘V` | `p` | `paste` |
| Move Here | `⌥⌘V` |  | `paste-move` |
| Copy Path | `⌥⌘C` |  | `copy-path` |
| Select All | `⌘A` |  | `select-all` |
| Extend Selection to Start | `⌥⇧↑` |  | `extend-selection-to-start` |
| Extend Selection to End | `⌥⇧↓` |  | `extend-selection-to-end` |
| Filter | `/` |  | `start-filter` |
| Find… | `⌘F` |  | `show-search` |
| Find Everything… | `⇧⌘A` |  | `search-everything` |
| Search Home | `⇧⌘H` |  | `search-home` |

## View

| Command | Finder | Vim layer | Id |
| --- | --- | --- | --- |
| View as Icons | `⌘1` |  | `view-as-icons` |
| View as List | `⌘2` |  | `view-as-list` |
| View as Columns | `⌘3` |  | `view-as-columns` |
| View as Gallery | `⌘4` |  | `view-as-gallery` |
| Toggle Hidden Files | `⇧⌘.` |  | `toggle-hidden-files` |
| Show or Hide Right Sidebar | `⇧⌘P` |  | `toggle-preview` |
| Show or Hide Left Sidebar | `⌃⌘S` |  | `toggle-sidebar` |
| Show Info | `⌘I` |  | `show-info` |
| Cycle Right Sidebar | `⌥⌘P` |  | `cycle-right-sidebar` |
| Show Activity | `⌥⌘L` |  | `toggle-activity` |
| Theme… | `⌥⌘T` |  | `show-theme-picker` |

## Go

| Command | Finder | Vim layer | Id |
| --- | --- | --- | --- |
| Back | `⌘[` |  | `go-back` |
| Forward | `⌘]` |  | `go-forward` |
| Go to Parent Folder | `⌘↑` |  | `open-parent` |
| Go to… | `⌘K` |  | `show-command-palette` |
| Go to Folder… | `⇧⌘G` |  | `go-to-folder` |
| Edit Path | `⌘L` |  | `edit-path` |
| Reveal Result | `⌘R` |  | `search-reveal` |

## Window

| Command | Finder | Vim layer | Id |
| --- | --- | --- | --- |
| Previous Tab | `⌃⇧⇥` |  | `previous-tab` |
| Next Tab | `⌃⇥` |  | `next-tab` |
| Close Tab | `⌘W` |  | `close-tab` |

## Palette only

| Command | Finder | Vim layer | Id |
| --- | --- | --- | --- |
| Settings | `⌘,` |  | `open-settings` |
| Switch Focus | `⇥` |  | `switch-focus` |
| Remove from Sidebar | `⌘⌫` |  | `remove-from-sidebar` |
| Move Favorite Up | `⌥⌘↑` |  | `move-favorite-up` |
| Move Favorite Down | `⌥⌘↓` |  | `move-favorite-down` |
