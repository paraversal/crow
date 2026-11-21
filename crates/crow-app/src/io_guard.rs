//! Keeps disk I/O off the main thread (AGENTS.md).
//!
//! A filesystem call in `crates/crow-app/src` must be marked `// io-ok: <reason>`
//! on its line or the line above: it runs inside `background_spawn`, at startup
//! before a window exists, or in test-only code. `cargo test` fails on a new
//! unmarked call, so the review sees it rather than a beachball months later.
//!
//! The scan is textual: it catches the calls listed in `PATTERNS`, not a call
//! split across lines or hidden behind an import. It is a guard, not a proof.

use std::path::{Path, PathBuf};

/// Calls that touch the disk when they run on the main thread.
const PATTERNS: &[&str] = &[
    "std::fs::",
    ".exists()",
    ".is_dir()",
    ".is_file()",
    ".metadata(",
    ".symlink_metadata(",
    ".canonicalize(",
    ".read_dir(",
    // crow_macos calls that read the disk (xattrs, thumbnails, iCloud, TCC probes).
    "crow_macos::tags(",
    "crow_macos::tags_checked(",
    "crow_macos::thumbnail_png(",
    "crow_macos::cloud_status(",
    "crow_macos::cloud_status_batch(",
    "crow_macos::icon_png(",
    "crow_macos::permissions(",
    "write_atomic(",
];

/// Files that are not the app's main thread: the fuzz runner, test modules, this test.
fn skipped(name: &str) -> bool {
    name == "fuzz_live.rs" || name == "io_guard.rs" || name.ends_with("tests.rs")
}

#[test]
fn disk_calls_are_marked_background_startup_or_test_only() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect(&root, &mut files);
    let mut unmarked = Vec::new();
    for file in files {
        let name = file.strip_prefix(&root).expect("under src").to_string_lossy().replace('\\', "/");
        if skipped(&name) {
            continue;
        }
        let text = std::fs::read_to_string(&file).expect("source file");
        let mut in_tests = false;
        let mut previous = "";
        for (index, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("mod tests {") {
                in_tests = true;
            }
            let marked = line.contains("// io-ok:") || previous.contains("// io-ok:");
            if !in_tests
                && !trimmed.starts_with("//")
                && !marked
                && PATTERNS.iter().any(|pattern| line.contains(pattern))
            {
                unmarked.push(format!("{name}:{}: {trimmed}", index + 1));
            }
            previous = line;
        }
    }
    assert!(
        unmarked.is_empty(),
        "disk I/O on the main thread (or an unmarked call):\n{}\n\
         Move the call into `background_spawn`, or mark its line (or the line above) \
         `// io-ok: <reason>` when it runs at startup, in test-only code, or in a \
         background task.",
        unmarked.join("\n")
    );
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("source dir").filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}
