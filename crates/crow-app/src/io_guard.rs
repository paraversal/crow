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
    // Config and theme loaders: each reads a file, so the call hides disk work
    // behind a helper name.
    "load_settings(",
    "load_views(",
    "load_sidebar(",
    "load_exclude(",
    "load_keys(",
    "crow_theme::load_theme(",
    "crow_theme::resolve_theme_id(",
    "crow_theme::missing_saved(",
    "crow_theme::set_global(",
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
        unmarked.extend(unmarked_lines(&text).into_iter().map(|(line, trimmed)| format!("{name}:{line}: {trimmed}")));
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

/// The 1-based line numbers and text of every unmarked disk call outside `mod tests`.
fn unmarked_lines(text: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
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
            && !is_fn_signature(trimmed)
            && PATTERNS.iter().any(|pattern| calls(line, pattern))
        {
            found.push((index + 1, trimmed.to_owned()));
        }
        previous = line;
    }
    found
}

#[test]
fn an_unmarked_config_load_fails_the_guard() {
    let bad = "fn reload(cx: &mut App) {\n    let settings = load_settings(&dir);\n}\n";
    assert_eq!(unmarked_lines(bad).len(), 1, "an unmarked load_settings( is caught");
    let good = "fn reload(cx: &mut App) {\n    // io-ok: background task\n    let settings = load_settings(&dir);\n}\n";
    assert!(unmarked_lines(good).is_empty());
    let theme = "let t = crow_theme::load_theme(&dir, &id)?;\n";
    assert_eq!(unmarked_lines(theme).len(), 1);
}

/// A definition names a loader without calling it.
fn is_fn_signature(trimmed: &str) -> bool {
    trimmed.strip_prefix("pub ").unwrap_or(trimmed).starts_with("fn ")
        || trimmed.strip_prefix("pub(crate) ").unwrap_or(trimmed).starts_with("fn ")
}

/// Whether `line` holds `pattern` as a call, not the tail of a longer name:
/// `reload_keys(` must not count as `load_keys(`.
fn calls(line: &str, pattern: &str) -> bool {
    let word = pattern.starts_with(|c: char| c.is_ascii_alphabetic());
    line.match_indices(pattern)
        .any(|(at, _)| !word || !line[..at].ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_'))
}

#[test]
fn the_guard_sees_loader_calls_but_not_lookalikes() {
    assert!(calls("let x = load_settings(&dir);", "load_settings("));
    assert!(calls("ui.load_keys();", "load_keys("));
    assert!(!calls("ui::reload_keys(cx);", "load_keys("));
    assert!(is_fn_signature("pub fn load_views(&mut self) -> bool {"));
    assert!(!is_fn_signature("let x = load_views(&dir);"));
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
