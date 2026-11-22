// The bundled Git plugin. Repository facts arrive from Crow's read-only
// provider as plain data; this script only maps them to P1B descriptors.

fn register() {
    [
        #{ kind: "column", id: "lines-changed", title: "Changed", width: 76,
           min_width: 60, align: "right", function: "lines_changed" },
        #{ kind: "decoration", id: "status", title: "Git status",
           function: "decoration" },
        #{ kind: "status", id: "repository", title: "Git",
           function: "status" },
        #{ kind: "info", id: "details", title: "Git",
           function: "info" },
    ]
}

fn decoration(ctx) {
    let rows = [];
    for entry in ctx.entries {
        if !entry.contains("git") { continue; }
        let git = entry.git;
        let tint = "info";
        if git.badge == "U" {
            tint = "error";
        } else if git.badge == "D" || git.badge == "R" {
            tint = "warning";
        } else if git.badge == "M" || git.badge == "A" {
            tint = "success";
        }
        rows.push(#{ handle: entry.handle, text: git.badge, spoken: git.label,
                     tint: tint, dim: git.ignored });
    }
    rows
}

fn lines_changed(ctx) {
    let rows = [];
    for entry in ctx.entries {
        if !entry.contains("git") { continue; }
        let git = entry.git;
        if git.contains("lines") {
            rows.push(#{ handle: entry.handle, text: git.lines.to_string() });
        } else {
            rows.push(#{ handle: entry.handle, text: "—" });
        }
    }
    rows
}

fn status(ctx) {
    if !ctx.contains("repository") { return (); }
    let repo = ctx.repository;
    let text = if repo.detached {
        "detached"
    } else if repo.unborn {
        repo.branch + " (unborn)"
    } else {
        repo.branch
    };
    if repo.unavailable {
        return [#{ text: text + " · status unavailable" }];
    }
    let ahead = if repo.contains("ahead") { repo.ahead } else { 0 };
    let behind = if repo.contains("behind") { repo.behind } else { 0 };
    if ahead > 0 || behind > 0 {
        if ahead > 0 {
            text += " ↑" + ahead.to_string();
        }
        if behind > 0 {
            text += " ↓" + behind.to_string();
        }
    }
    if repo.changed > 0 {
        text += " · " + repo.changed.to_string() + " changed";
    } else {
        text += " · clean";
    }
    [#{ text: text }]
}

fn info(ctx) {
    if !ctx.contains("repository") { return (); }
    let repo = ctx.repository;
    let rows = [];
    if ctx.entries.len == 1 && ctx.entries[0].contains("git") {
        let git = ctx.entries[0].git;
        if git.label != "" {
            rows.push(#{ label: "Status", value: git.label });
        }
        if git.contains("index") {
            rows.push(#{ label: "Index", value: git.index });
        }
        if git.contains("worktree") {
            rows.push(#{ label: "Worktree", value: git.worktree });
        }
        if git.contains("lines") {
            rows.push(#{ label: "Lines changed", value: git.lines.to_string() });
        }
    }
    if repo.detached {
        if repo.contains("head") {
            rows.push(#{ label: "Detached at", value: repo.head });
        }
    } else if repo.unborn {
        rows.push(#{ label: "Branch", value: repo.branch + " (unborn)" });
    } else {
        rows.push(#{ label: "Branch", value: repo.branch });
    }
    if repo.unavailable {
        rows.push(#{ label: "Status", value: "unavailable (cloud-backed worktree)" });
    }
    if repo.contains("last_commit") {
        rows.push(#{ label: "Last commit", value: repo.last_commit });
    }
    if repo.changed > 0 {
        rows.push(#{ label: "Changed paths", value: repo.changed.to_string() });
    }
    rows
}
