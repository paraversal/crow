# Features

Everything Crow does today, by area. A change that adds or removes a feature updates this list in the same commit. Keys are in [keys.md](keys.md), settings in [configuration.md](configuration.md), and the task-by-task guide is the [manual](manual/index.html).

## Browsing
- Four views: icons (Compact, Standard, Large, optional caption), list (resizable, hideable columns, nested folders), columns, and gallery (a filename rail beside a large preview, with a draggable divider (double-click resets it); two selected files compare side by side).
- Per-folder view, sort and columns, saved when you change them. Folders open inside a list follow the sort, and a size sort re-orders as folder sizes arrive (the selection stays on its item and the view does not scroll). Rules by path pattern in `views.toml`.
- Tabs, Back and Forward history, Enclosing Folder, Go to Folder, and several windows.
- A sidebar of favorites in groups, Recent, Projects, Tags, saved searches and Locations (free space, eject). Five looks: ledger lines, cards, accordion, floating panel, pills. Groups take an icon; favorites take a name, icon, colour and Vim mark. The default Outline icons draw sidebar folders and special folders (Home, Desktop, Documents, Downloads, Applications…) as SF Symbol line glyphs, like Finder's sidebar, and the path bar and the title's folder icon follow; Finder keeps Finder's colored icons. An icon pack still wins.
- An opt-in Shelf at the bottom of the sidebar: carry files between folders, then Move or Copy Shelf Here.
- A right sidebar with Preview, Info, Activity and Git Diff tabs: ⇧⌘P shows or hides it, ⌥⌘P opens it or cycles its tabs, ⌘I opens it on Info, ⌥⌘L opens it on Activity. The left sidebar hides with ⌃⌘S; both slide, and a chevron at each sidebar's edge hides it too, and a chevron in the toolbar brings a hidden one back.
- Its Info tab shows kind, size, dates, tags, permissions and owner; a multi-selection shows totals and shared tags.
- Quick Look thumbnails and previews, syntax-highlighted text, hidden-file toggle, folder sizes, and cloud placeholders with their download badge.
- A path bar you can type into (⌘L) with Tab completion and a pasted path in any form; every segment drags the folder it shows, and right-clicking one opens that folder's menu (open, new tab, copy path, Terminal, add to sidebar; New Folder and Paste on the folder shown). A breadcrumb or an always-visible field.
- A toolbar folder-menu button (⌃⌘M) opens the shown folder's whole menu; its Pin to Toolbar submenu puts any of New Folder, New File, Paste, Copy Path, Show in Finder and Open Terminal on the toolbar as a button (right-click one to unpin). Narrow windows fold the pins back into the menu.
- Same-named folders show their parent; symlinks are marked.

## Moving around
- ⌘K palette: folders (Recent, Favorites, the current folder), files in the folder, paths typed from `/`, `~/`, `./` or `../` (a dot path is relative to the focused folder), every command (`>`), and recently used commands.
- `/` filters the focused folder as you type, with a tag filter and "n of N".
- Drag across empty space in any view to select what the rectangle touches (⇧ or ⌘ keeps the current selection); it scrolls at the edge. Type-to-select with the typed letters shown; Vim layer with `hjkl`, `gg`/`G`, `yy`/`dd`/`p`, marks and `cw`.
- Every command in the menu bar, the palette and a rebindable key (Settings ▸ Keyboard or `keys.toml`), with conflict handling.
- Key hints and one-time tips that retire as you use the keys.

## Search
- Names and contents, live, in the folder, Home or This Mac (Spotlight, names only). All, Names or Contents modes. Matching ignores case; wrapping the query in slashes (`/fn \w+_view/`) makes it a case-sensitive regular expression, flagged by a badge in the field. An empty search shows one quiet line; the scopes and modes explain themselves in tooltips.
- ripgrep's libraries and an fzf-style matcher run in-process; nothing to install.
- Skipped folders are visible with the rule that skipped them; `dev` set on by default (`node_modules`, `target`, `.git`, ...). Settings ▸ Search turns each set on or off (developer, dependencies, media, hidden, system) and edits the developer folder names; custom rules stay in `exclude.toml`. An opt-in set skips folders carrying the Time Machine exclusion mark (about 10 µs per folder searched while on), such as the dev folders [asimeow](https://github.com/mdnmdn/asimeow) marks. Find Everything turns them all off.
- Saved searches with kind, age and tag predicates become live sidebar places.
- Results show line numbers for content hits, preview at the hit line, and take the usual file keys; drag the header dividers to resize Name, Folder and Modified.

## File operations
- Copy, move, duplicate, trash, delete permanently, empty Trash (home and every volume), new folder and file, rename, batch rename (find/replace, prefix/suffix, numbering, live preview, one undo). A setting (`folder_target`) chooses whether New Folder, New File and Paste work in the folder shown, as in Finder, or in the one folder selected.
- Undo and redo for every operation, including trash and batch rename; older steps from the Activity tab (running jobs, queued conflicts and history; it stays open while you work).
- Nothing is overwritten: name clashes ask first, Replace sends the old item to the Trash.
- A copy queue with pause, conflicts shown up front and "Decide later"; Esc cancels cleanly, finished items stay undoable.
- Drag and drop within Crow and from or to Finder and other apps; Option copies; spring-loaded folders with a way back.
- Copy Path, Show in Finder, Open Terminal Here (any terminal app), Open With, Quick Look.

## Git and Projects
- A bundled, read-only plugin (approve it once): badges on changed files and folders, ignored files dimmed, branch and ahead/behind in the status bar, Lines changed column, a Git Diff tab, and Review Git Changes across pinned repositories.
- A Projects section lists the Git repositories directly inside one folder (`[projects] root`), by folder name, with the branch and a changed count. The heading takes a `name` and can show or hide the root. It is not a general folder list: use a custom favorites group for just the folders you want.

## Make it yours
- Ten built-in themes: Finder Light and Dark, Catppuccin Latte and Mocha, Gruvbox Light and Dark, Solarized Light and Dark, Rosé Pine and Rosé Pine Dawn; "System" follows macOS. Any Base24 theme in `~/.config/themes` is listed too, and Apply to All Apps sets it for every program reading that index.
- Icon style (Outline or Colored) with icon packs (SVG and PNG) layered over them in order; a font and size; a translucent sidebar.
- Motion (off, low, full), Finder-style or Crow sounds, respecting Reduce Motion.
- Plain TOML config with live reload and comments kept; a Settings window over the same files with search, its sections (General, Appearance, Browsing, Previews, Keyboard, Search, Plugins, Advanced) laid out as grouped cards.
- Plugins (Rhai packages with grants): commands, sidebar sections, preview providers, columns. See [plugins.md](plugins.md).
- A `crow` command to open a folder or reveal a file.

## Quality
- Folders of 100,000 files: first paint under 50 ms, smooth scrolling. Bench in [testing.md](testing.md#performance).
- Seeded fuzzing, edge-case trees and a frame-budget check; no disk I/O on the main thread.
- VoiceOver roles and labels on every interactive element.

## Not in Crow yet
AirDrop and the Share menu, iCloud sharing and collaboration, Quick Actions and Services, tag editing beyond what the Info pane offers, Get Info editing (permissions, default app), Spotlight smart folders, compress and archive, Connect to Server, Time Machine, Finder extensions (Dropbox and similar badges), and a signed, notarized build.
