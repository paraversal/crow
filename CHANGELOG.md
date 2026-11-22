# Changelog

All notable changes to Crow are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and Crow uses
[semantic versioning](https://semver.org/).

## [0.1.0] - Unreleased

The first public release: a Finder-style file explorer for macOS that is fast on very large
folders, keyboard-first, and themeable. Free software under the GPL-3.0-or-later.

### Browsing
- List, Columns, icon grid and Gallery views, with per-folder settings, tabs and a sidebar of
  favorites, Projects, tags and saved queries.
- Folders of 100,000 files open and scroll smoothly; a folder deleted while it is shown is handled.
- A path bar you can drag from and edit, Quick Look-style preview, and an Info pane.

### Search and navigation
- Command palette (Cmd-K) for commands, folders and files; filename and content search with
  Home, Folder and Spotlight scopes, in resizable columns; saved queries as live sidebar destinations.
- Finder's keys first, with an optional Vim layer on top; every command has a menu item, a
  palette entry and a rebindable key (Settings > Keyboard, `keys.toml`).

### File operations
- Copy, move, trash, rename and batch rename with preview, all undoable. Nothing is overwritten in
  place and deletes go to the Trash. Copies run in a resumable queue with pause and conflict choices.
- Drops from other apps, and spring-loaded folders.

### Look and feel
- Built-in themes (Finder Light/Dark, Catppuccin, Gruvbox, Solarized, Rosé Pine) plus any theme
  in `~/.config/themes`; icon packs (SVG or PNG, layered in order); motion and sound settings.
- Five sidebar styles (ledger lines, cards, accordion, floating panel, pills), group icons, and an
  opt-in Shelf for carrying files between folders.
- Settings window and plain-text config files in `~/.config/crow` with live reload.

### Integrations
- Read-only Git status and review, iCloud Drive download status, Finder tags, and Rhai plugins.

### Known limitations
- The app is **not signed or notarized** (no paid Developer ID). macOS blocks the first open;
  see [docs/install.md](docs/install.md). Permissions you grant (Full Disk Access, Desktop,
  Documents) may have to be granted again after each update.
- No AirDrop and no iCloud Drive sharing.
- The Intel half of the universal build is compiled but has not been run on an Intel Mac.

[0.1.0]: https://github.com/paraversal/crow/releases/tag/v0.1.0
