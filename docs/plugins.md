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
64 registrations, 128 entries per call, 4 KiB per display field, 64 KiB serialized result,
100,000 Rhai operations per call and a 50 ms call deadline measured from dequeue. The worker
also bounds expression depth, call levels, arrays, maps, strings and modules.

## Capability matrix (API 1)

| Capability | Manifest | Behavior |
| --- | --- | --- |
| File read | `fs = "read"` | Before a worker call, the Rust host may read at most 64 KiB of UTF-8 text for one eligible entry and pass it as `entries[i].excerpt`. The host opens it no-follow, verifies a regular file and the listing identity when present. A script never receives a path or a read function. `fs = "none"` means no excerpt is supplied. P1B checks P8 content eligibility before asking the host to read. |
| Spawn | `spawn = []` | Reserved. Any nonempty list is rejected before a worker starts. |
| Network | `net = false` | Reserved. `true` is rejected before a worker starts. |
| Bundle assets | `assets = [...]` | `asset(name) -> String` returns a declared asset only. Undeclared names are capability errors. |

No other host capability is exposed. Scripts get no raw filesystem paths, shell, sockets,
module imports, debugger, print/debug output or mutating host callback. A command's only
API-1 result is a checked `#{ kind: "copy_text", text }` request; P1B owns the larger action
allowlist.

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

Each descriptor has `kind`, `id`, `title`, optional `applies_to` and, for every kind except
`icon-pack`, a `function` defined with one parameter. Registrations are namespaced as
`plugin_id.local_id`; local IDs are unique, one segment, lowercase letters/digits/hyphens.
The whole batch is rejected on an unknown field, duplicate ID, undeclared kind or missing
function. Nothing is published until registration succeeds.

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
| `command` | `#{ kind: "copy_text", text: "..." }` |
| `column`, `decoration` | `[ #{ handle, text, spoken? } ]`, keyed by input handles |
| `preview` | `#{ text: "..." }` or `#{ table: [ ["a", "b"] ] }` (not both) |
| `sidebar` | `[ #{ id, title, meta?, badge? } ]` |
| `status` | `[ #{ text: "..." } ]` |
| `info` | `[ #{ label, value } ]` |
| `icon-pack` | `#{ icons: #{ "rust": #{ icon: "rust", color: "accent" } } }` |
| `hook` | `()` or `#{ refresh: true }`; hooks cannot invoke a command |

Unknown fields, unknown or duplicate handles, oversized fields and malformed batches reject
the whole reply. Hooks can request a fresh read; only commands can return a `CommandIntent`.

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
