# Plugins

Crow runs locally installed [Rhai](https://rhai.rs) packages in one dedicated worker per
package. The public API is experimental: `api = 1` is the only supported revision, and a
package's semantic version is metadata. P1A implements the package, permission, worker and
registration runtime; P1B adds the full menu, palette and author-facing surfaces.

## Install and review

1. Copy a bundle directory into `~/.config/crow/plugins/`. A bundle is `plugin.toml`, its
   entry script and any declared assets. Discovery only reads files; it never runs a script.
   If Crow is already open, choose **Reload All** in Settings ▸ Plugins after copying.
2. Open **Settings** (⌘,) and find **Plugins**. The row shows the package name, version,
   state and requested `fs` capability.
3. **Approve** the exact snapshot. Crow re-reads the bundle, compares its digest with the
   one shown, writes a grant to `~/.config/crow/plugins.toml`, then starts the worker.
   If the bundle changed between reading and approval, approval is refused and the new
   snapshot is shown instead.
4. **Disable** keeps the grant record but stops the worker and withdraws its extensions.
   **Enable** starts the same approved snapshot again. **Reload** re-reads grants and
   bundles; changed bytes return the package to pending approval. **Retry** starts a paused
   or failed package with a new generation.

The grants file is comment-preserving and atomic like Crow's other config files:

```toml
[plugin."example.relative-path"]
enabled = true
digest = "<64 lowercase hex digits>"
fs = "none"
```

One record authorizes exactly one `(plugin id, digest, capability set)`. A missing record,
a disabled record, a digest change or a different requested `fs` leaves the package inert.
If `plugins.toml` is unreadable or invalid, Crow withdraws every grant, names the file in
the status line and re-enables only matching, currently enabled records after it is fixed.

## Manifest

`plugin.toml` is a closed schema: unknown fields are errors.

```toml
id = "example.relative-path"
name = "Copy Relative Path"
version = "0.1.0"
api = 1
entry = "main.rhai"
assets = ["icons.toml"]
provides = ["command"]
permissions = { fs = "none", spawn = [], net = false }
```

| Field | Rule |
| --- | --- |
| `id` | 1–96 ASCII bytes; dot-separated lowercase letters, digits and hyphens. `crow` and `crow.*` are reserved for bundled providers. |
| `name` | Display text, at most 4 KiB, no control characters. |
| `version` | Semantic version metadata (`major.minor.patch` with optional pre-release/build). |
| `api` | Exact integer dispatch key. Only `1` is accepted. |
| `entry` | Bundle-relative regular file; no absolute paths or `..`; no symlink components. |
| `assets` | Optional declared bundle-relative files, unique, not the manifest or entry. |
| `provides` | Nonempty subset of `column`, `decoration`, `preview`, `sidebar`, `command`, `hook`, `status`, `info`, `icon-pack`; duplicates rejected. |
| `permissions` | Deny-by-default; see the matrix. |

A package must fit these limits: 256 KiB per file, 2 MiB for manifest + entry + assets,
64 registrations, 128 entries per call, 4 KiB per display field, 16 bytes per badge,
32 suffix filters, 64 KiB serialized result, 100,000 Rhai operations per call and a 50 ms
call deadline measured from dequeue. The worker also bounds expression depth, call levels,
arrays, maps, strings and modules.

## Capability matrix (API 1)

| Capability | Manifest | Behavior |
| --- | --- | --- |
| File read | `fs = "read"` | Before a worker call, the Rust host may read at most 64 KiB of UTF-8 text for one eligible entry and pass it as `entries[i].excerpt`. The host opens it no-follow, verifies a regular file and the listing identity when present. A script never receives a path or a read function. `fs = "none"` means no excerpt is supplied. P1B checks P8 content eligibility before asking the host to read. |
| Spawn | `spawn = []` | Reserved. Any nonempty list is rejected before a worker starts. |
| Network | `net = false` | Reserved. `true` is rejected before a worker starts. |
| Bundle assets | `assets = [...]` | `asset(name) -> String` returns a declared asset only. Undeclared names are capability errors. |

No other host capability is exposed. Scripts get no raw filesystem paths, shell, sockets,
module imports, debugger, print/debug output or mutating host callback. API 1 commands can
request only a transient message, clipboard text or a reveal of a handle from that
invocation. File mutations, raw-path intents, network, spawn, plugin-defined shortcuts and
general-purpose native callbacks stay out of the allowlist.

## Script contract

`register()` is deterministic and returns one bounded array of descriptors:

```rhai
fn register() {
    [
        #{ kind: "command", id: "relative-path", title: "Copy Relative Path",
           applies_to: "selection", function: "relative_path" },
    ]
}

fn relative_path(ctx) {
    let entry = ctx.entries[0];
    #{ kind: "copy_text", text: ctx.scope + "/" + entry.name }
}
```

Each descriptor has `kind`, `id`, `title` and exactly one source. Computed kinds name a
`function` defined with one parameter. An `icon-pack` names an `asset` instead: a
bundle-relative file declared in the manifest's `assets`. The two are not interchangeable.
Fields that do not belong to the descriptor's kind are rejected.

| Kind | Extra registration fields |
| --- | --- |
| `column` | Optional integer `width` (default 100) and `min_width` (default 50), both 32–480 with `min_width <= width`; `align = "left"` (default) or `"right"`. |
| `preview` | Optional `extensions` list of 1–32 unique ASCII suffixes (letters, digits, hyphens, underscores; no leading dot, 1–32 bytes each). Omission matches every eligible file. |
| `command` | `applies_to = "scope"` (default) or `"selection"`; optional `context_menu = true` (selection only); optional `extensions` filter (selection only). |
| `decoration`, `sidebar`, `status`, `info`, `hook`, `icon-pack` | No extra fields. | Registrations are namespaced as `plugin_id.local_id`; local IDs are unique,
one segment, lowercase letters/digits/hyphens. The whole batch is rejected on an unknown
field, duplicate ID, undeclared kind, missing function, a missing or undeclared pack asset,
or a pack asset that is not in the approved snapshot. Nothing is published until
registration succeeds.

An icon pack is data-only; it has no computed reply:

```toml
assets = ["icons/pack.toml", "icons/rust.svg"]
provides = ["icon-pack"]
```

```rhai
fn register() {
    [ #{ kind: "icon-pack", id: "theme", title: "Theme", asset: "icons/pack.toml" } ]
}
```

The pack file and its glyph references must be declared bundle assets. Changing the pack
TOML or a declared glyph changes the package digest, withdraws the old generation and
returns the package to pending approval. The pack is selected by its namespaced id,
`<plugin-id>.<pack-id>` (for example `example.pack.theme`).

The pack format is versioned and data-only:

```toml
api = 1
id = "theme"          # optional; informational for a plugin pack
name = "Theme"

[item]
"~/work/vendor.txt" = { icon = "vendor.svg", color = "accent" }

[file_name]
"Cargo.toml" = { icon = "cargo.svg", color = "accent" }

[folder_name]
"src" = { icon = "src.svg", color = "base0D" }

[extension]
rs = { icon = "rust.svg", color = "base09" }

[type]
"public.image" = { icon = "image.svg", color = "base0E" }

[generic]
file = { icon = "file.svg", color = "fg-muted" }
folder = { icon = "folder.svg", color = "base0D" }
```

Resolution is first match wins: exact item path, exact file name, exact folder name,
ASCII-case-folded extension, ASCII-case-folded UTType identifier (exact type first, then
the types it conforms to, so `public.image` covers PNG and JPEG), then generic file or
folder. Names match the raw path bytes exactly; a non-UTF-8 name falls through to the
later tiers. `color` is optional and defaults to `fg`; it names one of the theme's roles
or a palette hue (`base08`–`base0F`, `base12`–`base17`).

`icon` is a relative path to an `.svg` in the pack. Each glyph must be a regular file
(no symlinks), stay inside the pack folder, be at most 256 KiB, and the pack may total
at most 8 MiB across 512 glyphs. Glyphs must be UTF-8 SVG without scripts, embedded
images, external `href`s, event attributes or external CSS references. An unknown role,
missing asset, bad path or malformed glyph rejects the whole pack; Crow keeps the last
good pack snapshot and reports the error. Removing a pack or disabling its plugin falls
back to native icons and keeps the configured id. Local packs live in
`icon-packs/<id>/pack.toml` under the config folder and use the folder name as their id.

Computed functions receive one map:

```text
#{ api: 1, scope: "/display/scope", selection: 3, entries: [
    #{ handle: 3, name: "notes.txt", is_dir: false, size: 120, kind: "Text",
       excerpt: "host-read text, when the fs capability allows it" }
] }
```

`scope` is host display text, not an I/O handle. `selection` is present only when there is one.
`entries` is capped at 128; handles are valid only for this invocation.

| Kind | Return |
| --- | --- |
| `command` | `()`, `#{ kind: "message", text }`, `#{ kind: "copy_text", text }` or `#{ kind: "reveal", handle }` |
| `column` | `[ #{ handle, text, spoken? } ]`; unit or an omitted handle leaves that cell blank |
| `decoration` | `[ #{ handle, text, spoken?, tint?, dim? } ]`; empty text allows tint/dim/spoken-only output. `tint` is `accent`, `info`, `success`, `warning` or `error`; `dim` defaults false |
| `preview` | `#{ text: "..." }` or `#{ table: [ ["a", "b"] ] }` (not both; rectangular, 128 rows, 64 columns, 1,024 cells); unit, empty text/table or `#{}` declines |
| `sidebar` | `[ #{ id, title, meta?, badge?, handle? } ]` or unit; a known directory handle makes a path row, omission a display-only row |
| `status` | `[ #{ text: "..." } ]` or unit; empty text contributes nothing |
| `info` | `[ #{ label, value } ]` or unit |
| `icon-pack` | Registration only (see above); never evaluated for a reply |
| `hook` | `()` or `#{ refresh: true }`; hooks cannot invoke a command |

Unknown fields, mixed preview shapes, ragged tables, unknown or duplicate handles, oversized
fields and malformed batches reject the whole reply. Handles are request-local: a command's
`reveal` handle is checked against the entries of that invocation, never a path. Hooks can
request a fresh read. A message is nonempty single-line text; `copy_text` keeps the P1A
multiline clipboard rules. Display strings are lossy labels (controls replaced, UTF-8 bounded);
paths and file identities stay host-owned. `()`, unit and an omitted handle mean no value.

## App surfaces

All computed kinds require the package to be **Running** in its approved generation. A
withdrawn, paused, failed, disabled or replaced generation contributes nothing, and queued
replies from it are ignored.

- **Columns.** Add a descriptor ID to `views.toml`'s `columns` array. Active columns render
  after Name in config order with the descriptor's width, minimum and alignment; plugin
  headers are not sortable. Unknown or disabled IDs stay in the file.
- **Decorations.** List, Columns and search results share the presentation: the first
  nonempty badge and first tint in lexical fully qualified ID order, OR of all dim flags,
  and spoken descriptions in the same order. A contributing row reserves a fixed 32 pt
  badge slot. Selected rows keep Crow's normal foreground.
- **Status.** The first nonempty Running status descriptor (lexical ID order) occupies the
  bottom bar's optional slot. It yields before hints, path segments, counts, config errors
  and operation progress, and is hidden during path editing, overlays, transient messages
  and running operations.
- **Info.** The visible Info tab appends up to 16 labeled fields for a single selected entry,
  after the native metadata, ordered by descriptor ID and each reply's order.
- **Preview.** Add provider IDs to the resolved `preview_providers` list. One selected
  ordinary file tries configured providers in list order for at most 250 ms total, then
  falls through to the native preview. `fs = "none"` providers receive the metadata snapshot;
  `fs = "read"` gets one 64 KiB excerpt only when a fresh P8 gate says the file is locally
  readable. An approved read grant never makes Crow download an iCloud placeholder, and a
  metadata placeholder stays in place on `SkipCloud`/`SkipAmbiguous`. `Space` remains native
  Quick Look and does not wait for a plugin.
- **Commands.** A scope command acts on the focused List/Columns column or search results and
  receives no entries. A selection command needs 1–128 selected rows; an optional `extensions`
  filter requires every selected row to be a matching file by ASCII-folded raw filename
  suffix. Commands appear in `File ▸ Plugins`, the `Cmd-K` `>` palette and, when
  `context_menu = true`, the file context menu. Text fields, the sidebar, sheets and Settings
  offer no command target. The app revalidates focus, scope, selection and generation before
  applying any effect; `reveal` also rechecks no-follow identity. A stale menu item is inert.
- **Sidebar.** A shown, expanded plugin section gets the current real folder's first 128
  ordinary immediate folders in display order. A known directory handle yields a navigable
  row; omitted handles are display-only. Sections can be ordered/shown/collapsed in
  `sidebar.toml`; the heading's collapse control writes only that section's `collapsed` flag.
  Disabling or withdrawing the package removes its rows and paths. Favorite and volume
  actions never act on plugin rows.
- **Hooks.** `scope-changed`, `selection-changed`, `files-changed` and `operation-done` are
  debounced over one 100 ms UI poll per owner/kind. `files-changed` comes only from an
  applied watcher update; `operation-done` includes cancelled and failed operations that
  reached the shared completion path. A `#{ refresh: true }` reply invalidates only that
  plugin's passive values for the captured scope. Hooks never fire from a repaint or a
  result-driven refresh.

```toml
# views.toml
[defaults]
columns = ["modified", "size", "example.p1b-fixture.sample-column", "kind"]
preview_providers = ["example.p1b-fixture.sample-preview"]

[folder."~/Documents"]
preview_providers = []
```

```toml
# sidebar.toml. Existing [[group]], [recent] and [locations] entries still work.
[sections]
order = ["recent", "example.p1b-fixture.folders", "locations"]

[section."example.p1b-fixture.folders"]
show = true
collapsed = false
```

## Bundled Git package

Crow embeds one package with the reserved ID `git`. It is not installed from `plugins/` and
an installed bundle cannot claim that ID. It appears in Settings ▸ Plugins as
**git (bundled)** and contributes nothing until you approve its exact digest; **Disable**
removes every Git row decoration, status value, Info field, Projects section and line count
without restarting Crow.

```toml
# sidebar.toml
[projects]
roots = ["~/code", "~/work"]

[sections]
order = ["recent", "crow.projects", "locations"]

[section."crow.projects"]
show = true
collapsed = false
```

```toml
# views.toml
[defaults]
columns = ["modified", "size", "git.lines-changed", "kind"]
```

The palette has **Add Current Folder as Project Root**, **Remove Current Folder from Project
Roots** and **Refresh Projects**. Project rows open like ordinary folders. Discovery scans
only the configured roots, stops at a bound (then the section shows a Refresh Projects row),
skips packages and symlinked folders, and lists nested repositories separately. Each row
shows its abbreviated path, branch and the number of tracked/untracked changed paths, with
ignored paths excluded.

The package registers a decoration, a column (`git.lines-changed`), a status value and Info
fields. Its scripts only map numbers and labels; all repository access is in Crow's Rust
provider. For this package only, the request context also carries:

```rhai
ctx.repository == #{ branch?, head?, detached, unborn, ahead?, behind?, changed, last_commit? }
ctx.entries[0].git == #{ badge, label, index?, worktree?, ignored, lines? }
```

`badge` uses conflict > deleted/renamed > added > modified > untracked > ignored. `lines` is
the staged-plus-unstaged text line edits for that row and is absent when the file is binary,
untracked, unmerged, unreadable, oversized or still being measured; the column shows `—`
rather than zero. A repository with no local upstream omits `ahead`/`behind`, a detached
head carries `head`, and an unborn branch carries its name.

The provider opens repositories read-only: it never writes the index, never fetches, never
opens a network transport, and clears every external filter/diff driver before asking gix to
compare bytes, so a repository-local clean/process filter, fsmonitor helper or diff command is
never run. Status and line-count work happens off the main thread and is cancelled or dropped
when the tab, repository or plugin generation changes.

## Test fixture

The copyable fixture lives at
`crates/crow-app/tests/fixtures/plugins/p1b-fixture/`. Copy the directory into
`~/.config/crow/plugins/`, approve `example.p1b-fixture`, then open its `tree/` folder. It
declares every API-1 kind: a column, decoration, sidebar section (`folders`), status, Info,
a `.txt` preview, hook, selection command and icon-pack asset. Its command copies the selected
file's name:

```rhai
fn copy_selected(ctx) {
    if ctx.entries.len != 1 { return (); }
    let relative = ctx.entries[0].name;
    #{ kind: "copy_text", text: relative }
}
```

The sibling `p1b-invalid-handle/`, `p1b-invalid-field/` and `p1b-budget/` fixtures are for
the runtime tests: an unknown handle, an oversized field and an operation-budget loop. They
load as packages and fail only when evaluated.

## Lifecycle

| Trigger | State | Result |
| --- | --- | --- |
| Discover valid unapproved package | Pending approval | Inventory only; no worker calls |
| Approve this snapshot | Starting → Running | Registry published after registration succeeds |
| Reload identical approved snapshot | Running | Generation and registry retained |
| Change manifest, entry or asset bytes | Pending approval | Old generation invalidated, contributions withdrawn |
| Disable or remove | Disabled or absent | Worker cancelled; old replies ignored |
| Invalid descriptor, script or package | Failed | Error named in the status line; other plugins keep running |
| Budget exhausted | Paused | Results invalidated; no command intent applied; Retry starts a new generation |

The status line and log name the plugin for parse, API, capability, registration and runtime
errors. A worker that exceeds its operation or deadline budget is paused, not retried
silently. Disabling or quitting marks the worker cancelled and detaches the UI immediately;
the worker exits cooperatively. A native host call is bounded separately; a Rhai operation
limit cannot interrupt one, so none is unbounded.
