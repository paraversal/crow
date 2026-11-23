// An example package for the website's plugin shot: ratings, RAW badges and a
// sidebar section for a photo shoot's folders. Fixture data only.
fn register() {
    [
        #{ kind: "sidebar", id: "stages", title: "Shoot stages", function: "stages" },
        #{ kind: "column", id: "rating", title: "Rating", width: 96, min_width: 60, function: "rating" },
        #{ kind: "decoration", id: "flags", title: "Flags", function: "flags" },
    ]
}

fn stars(n) {
    let out = "";
    for i in 0..5 {
        out += if i < n { "★" } else { "☆" };
    }
    out
}

fn rating(ctx) {
    let out = [];
    for entry in ctx.entries {
        if entry.is_dir { continue; }
        let n = if entry.name.contains("sunrise") { 5 } else if entry.name.contains("fjord") { 4 } else { 3 };
        out.push(#{ handle: entry.handle, text: stars(n), spoken: n.to_string() + " stars" });
    }
    out
}

fn flags(ctx) {
    let out = [];
    for entry in ctx.entries {
        if entry.name.ends_with(".dng") {
            out.push(#{ handle: entry.handle, text: "RAW", tint: "info", spoken: "RAW" });
        } else if entry.name.contains("reject") {
            out.push(#{ handle: entry.handle, text: "", dim: true, spoken: "rejected" });
        }
    }
    out
}

fn stages(ctx) {
    let out = [];
    for entry in ctx.entries {
        if !entry.is_dir { continue; }
        let meta = if entry.name == "Selects" { "42 keepers" } else if entry.name == "Edits" { "18 graded" } else { "6 delivered" };
        out.push(#{ id: entry.name, title: entry.name, meta: meta, handle: entry.handle });
    }
    out
}
