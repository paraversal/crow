fn register() {
    [
        #{ kind: "column", id: "sample-column", title: "Sample", width: 140, min_width: 60,
           align: "right", function: "sample_column" },
        #{ kind: "decoration", id: "sample-decoration", title: "Sample badge", function: "sample_decoration" },
        #{ kind: "sidebar", id: "folders", title: "Fixture folders", function: "folders" },
        #{ kind: "status", id: "sample-status", title: "Fixture status", function: "sample_status" },
        #{ kind: "info", id: "sample-info", title: "Fixture info", function: "sample_info" },
        #{ kind: "preview", id: "sample-preview", title: "Fixture preview",
           extensions: ["txt"], function: "sample_preview" },
        #{ kind: "hook", id: "hook", title: "Fixture hook", function: "hook" },
        #{ kind: "command", id: "copy-relative", title: "Copy Relative Path",
           applies_to: "selection", context_menu: true, extensions: ["txt"], function: "copy_selected" },
        #{ kind: "icon-pack", id: "icons", title: "Fixture icons", asset: "icons/pack.toml" }
    ]
}

fn sample_column(ctx) {
    let out = [];
    for entry in ctx.entries {
        out.push(#{ handle: entry.handle, text: "sample" });
    }
    out
}

fn sample_decoration(ctx) {
    let out = [];
    for entry in ctx.entries {
        out.push(#{ handle: entry.handle, text: "P1B", tint: "success", spoken: "P1B fixture" });
    }
    out
}

fn folders(ctx) {
    for entry in ctx.entries {
        if entry.name == "fixture-folder" {
            return [#{ id: "fixture-folder", title: "Fixture folder", meta: "fixture", handle: entry.handle }];
        }
    }
    ()
}

fn sample_status(ctx) {
    [#{ text: "P1B fixture" }]
}

fn sample_info(ctx) {
    [#{ label: "Fixture", value: "P1B" }]
}

fn sample_preview(ctx) {
    if ctx.entries.len != 1 {
        return ();
    }
    #{ text: "Fixture preview" }
}

fn hook(ctx) {
    #{ refresh: true }
}

fn copy_selected(ctx) {
    if ctx.entries.len != 1 {
        return ();
    }
    let relative = ctx.entries[0].name;
    #{ kind: "copy_text", text: relative }
}
