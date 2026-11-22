# Themes and icon packs

## Built in
Ten themes ship in the app: Finder Light and Dark, Catppuccin Latte and Mocha, Gruvbox Light and Dark, Solarized Light and Dark, Rosé Pine and Rosé Pine Dawn (sources and licences in `crates/crow-theme/themes/NOTICE.md`). `theme = "system"` follows macOS and shows Finder Light or Dark; Finder's selection follows your accent colour.

Pick one in Settings ▸ Appearance or View ▸ Theme… (⌥⌘T), which previews each theme as you move through the list. In `settings.toml`, `theme = "<id>"`.

## The global theme index
If `~/.config/themes/schemes/` exists, the themes there are listed too, and a theme with a built-in's id replaces it. In the Theme picker, Apply to All Apps (⌘↩) sets the theme for every program that reads the index. Crow reads and writes no theme files of its own. Change the theme anywhere and Crow updates live.

## Writing a theme
Drop a Base24 scheme JSON into `~/.config/themes/schemes/` and it appears at once:

```json
{
  "name": "My Theme",
  "variant": "dark",
  "palette": { "base00": "1e1e2e", "...": "24 slots, base00 to base17" },
  "roles": { "fg": "base05" }
}
```

- `variant` is `light` or `dark`.
- `palette` has the 24 Base24 slots `base00`–`base17`.
- `roles` is optional and maps Crow's semantic roles (`bg`, `fg`, `fg-muted`, `surface`, `surface-raised`, `border`, `accent`, ...) to slots or colours; a role you omit is derived from the palette. A role can also be `system:<name>` to follow a macOS colour.

Every colour in Crow comes from a role, so a theme restyles all of it. Start from a built-in theme's JSON in `crates/crow-theme/themes/`.

## Icon packs
A pack is a folder, `~/.config/crow/icon-packs/<id>/`, with a `pack.toml` and its icons beside it. It maps exact paths, file and folder names, extensions, UTType identifiers and the generic file and folder to an icon and a colour (a theme role or `base08`–`base0F`, `base12`–`base17`).

- **SVG** icons are single-colour glyphs, tinted with the pack's colour.
- **PNG** icons (up to 1024 px, 256 KiB) are drawn as they are; their colour is ignored.

[examples/icon-pack](examples/icon-pack) is a complete Lucide pack to copy. Pick a pack in Settings ▸ Appearance ▸ Icon pack, or set `icon_pack = "id"` in `settings.toml`. Pick several to layer them: `icon_pack = ["lucide", "linework"]` uses the first pack that has an icon for the item, then the next, then Finder's. An `icon_source` in a `views.toml` layer overrides the choice for one folder. A pack that fails to load keeps the last good icons and shows the error. Plugins can ship packs too ([plugins.md](plugins.md)).
