# Configuration

Crow keeps its settings in plain TOML files in `~/.config/crow/` (override the folder with `CROW_CONFIG_DIR`). Each file is created on first launch with every option documented in comments; changes apply immediately, a mistake shows in the status line with the file and line, and when Crow writes to a file it keeps your comments. The Settings window (⌘,) edits the same files, so you can use either; its Config Files page opens each one.

The blocks below are the files exactly as Crow writes them on first launch. They are generated from the app and checked by a test (`CROW_UPDATE_DOCS=1 cargo test -p crow-core --test docs` rewrites them), so they cannot drift.

Recent folders, key-use counts, seen tips and the Shelf live in the state folder (`~/.local/state/crow/`, `CROW_STATE_DIR`). Logs go to `~/Library/Logs/Crow/crow.log`.

## settings.toml

Theme, keymap, Return key, hints and tips, folder sizes, motion and sound, sidebar look, icon pack, path bar, font, terminal, hidden files, search defaults, preview, window size. `icon_pack` takes one pack id or a list (`["lucide", "linework"]`): the first pack with an icon for an item wins, then the next, then Finder's icons. `sidebar_style` is `ledger` (default), `cards`, `accordion`, `floating-panel` or `pills`.

<!-- defaults:settings.toml -->
```toml
# Crow settings. Changes apply immediately; delete a line to get its default back.

# Theme from ~/.config/themes: "system" follows macOS light/dark through the
# theme index; otherwise a theme id such as "gruvbox-dark-hard".
# View ▸ Theme… (Cmd-Option-T) picks one and writes it here.
theme = "system"

# "finder" (default) or "vim". The Vim layer adds hjkl, gg/G, yy/p, marks
# (m x, 'x) and more on top of the Finder keys; letters then no longer select by name.
keymap = "finder"

# What Return does in the file list: "rename" (default, like Finder) edits the
# name in place; "open" opens the item. F2 renames in either mode.
enter = "rename"

# Key hints in the bottom bar: "auto" (default), "always" or "off".
# "auto" retires each idle hint (/ filter, Cmd-F search, Cmd-K go to) after you
# have used its key 3 times; hints for the focused filter, sidebar or search stay.
# "off" hides every hint and tip.
hints = "auto"

# Contextual tips: "auto" (default) or "off". Tips point out a key the first
# time it matters. "off" mutes every tip; key hints stay as set above.
tips = "auto"

# Folder sizes in the Size column: "off" (default) leaves folders as "--";
# "visible" measures the folders on screen in the background and keeps the
# cache fresh with FSEvents. View ▸ Calculate Folder Sizes measures the current
# folder's folders once even when this is off.
folder_sizes = "off"

# Motion: "auto" (default) follows macOS Reduce Motion (full when it is off, low
# when it is on); "off" changes state in one frame; "low" fades opacity and color
# only; "full" also moves and resizes (trash swallows the row, the gap closes).
motion = "auto"

# Feedback sounds: "crow" (default) plays Finder's trash sounds plus soft ticks
# for move, new folder, undo and copy; "finder" plays only the sounds Finder
# itself makes (trash, empty trash, the alert); "off" is silent. macOS "Play user
# interface sound effects" off silences all but the alert.
sound = "crow"

# Icons in the file list, sidebar, palette and previews: "native" (default) uses
# macOS icons. Set a pack id to use a data-only pack from icon-packs/<id>/ or a
# plugin pack named <plugin-id>.<pack-id>. views.toml can override this per folder.
icon_pack = "native"

# The bottom bar's path: "crow" (default) is the Finder-style breadcrumb that
# turns into a field on Cmd-L or a click. "explorer" keeps it a visible path
# field with a Go button; Return, Escape and Tab work the same in both.
path_bar = "crow"

# How the sidebar draws its groups: "ledger" (default) is plain headings with a hairline,
# "cards" puts each group on a surface under a tinted heading strip, "accordion" uses
# full-width heading bars, "floating-panel" lifts the whole sidebar into one rounded panel,
# and "pills" shows each heading as a small colored pill. Any group takes an `icon` in
# sidebar.toml.
sidebar_style = "ledger"

# Let the sidebar show the blurred desktop behind it, like Finder's. Off by default.
# It stays opaque while macOS Reduce Transparency is on. Themes can't turn it on.
translucent_sidebar = false

# Interface font. Empty uses the system font.
font_family = ""
font_size = 13.0

# The app "Open Terminal Here" launches: a name ("Terminal", "iTerm") or a path
# to an app. The folder is passed as the app's document, the way `open -a` does.
terminal = "Terminal"

# List dotfiles and other hidden items. View ▸ Show Hidden Files (Shift-Cmd-.) toggles it.
show_hidden_files = true

# Search (Cmd-F) and Go to (Cmd-K) match dot-named items even while Show Hidden
# Files is off: dimmed, and Cmd-K ranks them after visible matches. false leaves
# them out of search until View ▸ Show Hidden Files is on. A Go to path typed
# with a leading dot still lists them.
search_includes_hidden = true

# What a new Cmd-F search looks for: "all" (names and contents), "names" or
# "contents". The panel's All/Names/Contents control changes the open search;
# this is the mode each new search starts in.
search_match = "all"

# The right column in list view and the last column in column view: a Preview tab
# and an Info tab (Cmd-I). View ▸ Hide Preview (Shift-Cmd-P) toggles the column;
# off is the default only, Cmd-I still opens it. Quick Look still works.
show_preview = true

# Cmd-K: list the commands you ran from the palette under "Recently used"
# (kept in ~/.local/state/crow/palette-commands.json). false hides the list and stops recording.
palette_recent_commands = true

# How folders are shown (list or columns, sort, list columns) is set in views.toml.
# true: each folder opens in its own view, so a window switches between list and
# columns as you move. false: a window keeps the view you give it (Cmd-2, Cmd-3,
# which then save nothing); new windows start in views.toml's [defaults] view and
# new tabs copy their window. Sort and list columns still follow each folder.
view_per_folder = true

[window]
# Size of every new window, in points. Restored sessions keep their own sizes.
width = 1200
height = 760
# Last chosen left sidebar width, in points (140–480).
sidebar_width = 210
```
<!-- /defaults -->

## views.toml

How folders are shown, in three layers: `[defaults]`, `[[rule]]` path patterns and `[folder."..."]` for one folder. Changing a folder's view or sort saves it as that folder's own setting. View ▸ Why This View? (⌘J) shows which layer decided what. An `icon_source` in any layer overrides the icon pack for that folder.

<!-- defaults:views.toml -->
```toml
# Crow view settings. Changes apply when you next open a folder.
#
# Three layers, each overriding the one before:
#   1. [defaults]                 every folder
#   2. [[rule]]                   folders matching a path pattern (later rules win)
#   3. [folder."~/Some/Folder"]   one folder; Crow writes these when you change
#                                 a folder's view or sort
# View ▸ Why This View? (Cmd-J) shows which layer decided what.
#
# Keys, in any layer:
#   view = "icons" | "list" | "columns" | "gallery"
#   icon_size = "compact" | "standard" | "large"
#   icon_caption = "off" | "<plugin>.<column>"
#   sort = "name" | "size" | "kind" | "modified" | "added"
#   ascending = true | false     (default: A to Z for names and kinds,
#                                 newest and largest first otherwise)
#   folders_first = true | false
#   columns = ["modified", "size", "kind", "added"]   list view columns after Name
#     Include "git.lines-changed" for Lines when the bundled Git plugin is enabled.
#   preview_providers = ["example.pack.preview"]       plugin previews tried before native
#   icon_source = "native" | "<pack-id>"               icons for this folder; unset
#                                                      follows settings.toml

[defaults]
view = "list"
sort = "name"
folders_first = true
columns = ["modified", "size", "kind"]

# Pattern rules. `path` is a glob: "~/Downloads" is that folder only,
# "~/Projects/**" every folder inside it, "**/node_modules" any folder of that name.
#
# [[rule]]
# path = "~/Downloads"
# sort = "added"
#
# [[rule]]
# path = "~/Pictures/**"
# view = "columns"
```
<!-- /defaults -->

## sidebar.toml

Groups of favorites (a group takes an optional `icon`, a short text or emoji; favorites take `name`, `icon`, `color` and a Vim mark `key`), whether Recent and Locations show, `pinned_tags`, the order, visibility and collapse state of built-in and plugin sections, `[projects] roots` for the Projects section, and `[shelf] show = true` for the opt-in Shelf: drag files onto it, carry them to another folder, then Move Shelf Here or Copy Shelf Here (palette and File menu). The Shelf holds references and is saved in the state folder.

<!-- defaults:sidebar.toml -->
```toml
# Crow sidebar. Crow updates this file when you drag favorites around or add
# and remove them; your comments stay.
#
# Each [[group]] is a section of favorites. A group takes `collapsed = true` to
# hide its items under the title (Crow writes it when you collapse the group) and
# an optional `icon` (a short text or emoji shown before the title; none by default).
# Items take:
#   path   the folder ("~" is your home folder)
#   name   label, if not the folder's own name
#   icon   a short text or emoji shown instead of the folder icon
#   color  a theme role ("accent", "error", …), a color name (red, orange,
#          yellow, green, cyan, blue, purple, pink) or "#rrggbb"
#   key    one letter: with the Vim layer, 'x jumps to the favorite whose key is x
#          (m x sets it)

[[group]]
name = "Favorites"
items = [
  { path = "~" },
  { path = "~/Desktop" },
  { path = "~/Documents" },
  { path = "~/Downloads" },
  { path = "/Applications" },
]

[recent]
show = true
# How many recently used folders to list. Crow also reads zoxide's list when installed.
count = 6

[locations]
show = true

# Finder tags shown even when no indexed item uses them (Crow discovers used
# tags from Spotlight; Finder's unused list is private). Case-sensitive.
# pinned_tags = ["Work", "review"]

# Optional section layout. `tags`, `saved-queries`, `recent` and `locations` are
# built-ins; a plugin section uses its exact package ID. Unlisted sections follow
# in the usual order.
# [sections]
# order = ["tags", "saved-queries", "recent", "example.p1b-fixture.folders", "locations"]
#
# [section."example.p1b-fixture.folders"]
# show = true
# collapsed = false
# icon = "📁"   # a short text or emoji before the section title; none by default

# The shelf: a strip at the bottom of the sidebar that holds files while you move between
# folders (drag files onto it, or use Add to Shelf). It holds references, not copies, and
# is remembered between launches. Off by default.
[shelf]
show = false

# Project roots. Crow scans these folders for Git repositories; empty by default.
# The native section is "crow.projects" in [sections]/[section] like any other.
[projects]
roots = []
```
<!-- /defaults -->

## exclude.toml

Folders search skips, in named sets: `dev` is on by default (`node_modules`, `target` beside `Cargo.toml`, `.git`, `build`/`dist` beside manifests, ...); `dependencies`, `media`, `hidden` and `system` are off until enabled. The search footer lists each skipped folder with the rule that skipped it, and Find Everything (⇧⌘A) searches with every set off.

<!-- defaults:exclude.toml -->
```toml
# Crow search exclusions. A search skips matching folders and counts them in the
# results summary; "Search everything" (Cmd-Shift-A) ignores every set.
# Delete a line to get the built-in default for that field back.

[dev]
# Build and dependency folders developers rarely want in results. On by default.
enabled = true
# Folder names or glob patterns, skipped anywhere.
names = [
  "node_modules", ".git", "__pycache__", ".venv", "venv", ".tox",
  ".mypy_cache", ".pytest_cache", "DerivedData", ".next", ".nuxt",
  ".svelte-kit", ".gradle", ".idea", ".cache", ".logicprojects",
]
# Skip a folder when its parent contains one of these marker files.
[dev.sentinel]
target = ["Cargo.toml"]
build = ["package.json", "pyproject.toml", "setup.py", "build.gradle", "settings.gradle", "CMakeLists.txt", "Makefile"]
dist = ["package.json", "pyproject.toml", "setup.py"]
Pods = ["Podfile"]

[dependencies]
# Dependency folders of other ecosystems. Off by default.
enabled = false
names = ["vendor", ".bundle", "bower_components"]

[media]
# Media libraries are packages; searching their insides is rarely useful.
enabled = false
names = ["*.photoslibrary", "*.photolibrary", "*.imovielibrary", "*.fcpbundle"]

[hidden]
# Every folder whose name starts with a dot.
enabled = false
dot_folders = true

[system]
# System folders. Off by default.
enabled = false
paths = ["/System", "/Library/Caches"]
names = [".Trashes"]
```
<!-- /defaults -->

## saved-queries.toml

Live search destinations in the sidebar. Predicates combine with AND; every listed tag must be present. File ▸ Save Search... writes one, and a saved row's context menu edits or removes it.

<!-- defaults:saved-queries.toml -->
```toml
# Saved searches are live destinations in the sidebar. A saved search may search
# names and contents (scope "folder" or "home") or names through Spotlight
# (scope "this-mac"). Predicates combine with AND. An empty `text` is allowed
# when `kind` or `modified_within_days` can produce results.
#
# Each record is a [[query]] table:
#
#   id                    stable, opaque id; editing the title keeps it
#   name                  the sidebar, tab and breadcrumb title
#   scope                 "folder", "home" or "this-mac"
#   root                  an absolute folder or "~/…"; only with scope "folder"
#   text                  query text; names and contents for folder/home
#   match                 "all", "names" or "contents" (this-mac is names only)
#   include_hidden        visit dot-named entries while Show Hidden Files is off
#   enter_hidden_folders  descend into dot folders
#   exclusions            "configured" (live exclude.toml rules) or "everything"
#   kind                  optional "folder", "file", "image", "pdf", "audio",
#                         "video", "archive", "text", or an exact extension
#   modified_within_days  optional positive age in days
#   tags                  optional Finder tag names; every one must be present
#                         (case-sensitive, Unicode-normalized)
#
# Example:
#
# [[query]]
# id = "recent-pdfs"
# name = "Recent PDFs in Documents"
# scope = "folder"
# root = "~/Documents"
# text = ""
# match = "names"
# include_hidden = true
# enter_hidden_folders = false
# exclusions = "configured"
# kind = "pdf"
# modified_within_days = 30
```
<!-- /defaults -->

## keys.toml

Your keyboard shortcuts: `[finder]` and `[vim]` tables map a command id to a list of keys such as `["cmd-shift-g"]`; a Vim sequence is `["y y"]`. A command you do not list keeps its default and `[]` removes its shortcut. Every id is in [keys.md](keys.md).

<!-- defaults:keys.toml -->
```toml
# Crow keyboard shortcuts. Change them in Settings > Keyboard or here; edits
# apply as soon as you save.
#
# Each line maps a command id to its keys. Commands you do not list keep their
# defaults, and an empty list ([]) removes the shortcut. Keys are written like
# "cmd-shift-g" (modifiers ctrl, cmd, alt, shift); a sequence is separated by
# spaces ("g g"). The [finder] table applies to the Finder keymap and [vim] to
# the Vim keymap.
#
# [finder]
# go-to-folder = ["cmd-shift-g"]
# new-tab = ["cmd-t", "cmd-alt-n"]
#
# [vim]
# copy-files = ["y y"]
```
<!-- /defaults -->

## plugins.toml

One approval record per plugin, including the bundled Git package: `enabled`, the package `digest` and the API-1 `fs` capability. Installed bundles live in `plugins/`; see [plugins.md](plugins.md). Approve plugins in Settings ▸ Plugins rather than by hand.

## Icon packs

Data-only: `icon-packs/<id>/pack.toml` with its icons beside it (SVG, tinted with a theme role or palette hue, or PNG up to 1024 px, drawn as is). A pack maps item paths, file names, folder names, extensions, UTType identifiers and generic files and folders to icons. [docs/examples/icon-pack](examples/icon-pack) is a complete pack. Pick packs in Settings ▸ Appearance ▸ Icon pack, and pick several to layer them. See [themes.md](themes.md).
