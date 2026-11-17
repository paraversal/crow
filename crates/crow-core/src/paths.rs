//! Paths typed by the user (Go to Folder).

use std::path::{Path, PathBuf};

/// Turn what was typed into an absolute path: `~` is home, relative paths start at `cwd`.
/// `..` and `.` are resolved by text, so the result needn't exist yet.
pub fn resolve_typed(input: &str, cwd: &Path, home: &Path) -> PathBuf {
    let input = input.trim();
    let raw = if input == "~" {
        home.to_path_buf()
    } else if let Some(rest) = input.strip_prefix("~/") {
        home.join(rest)
    } else if input.starts_with('/') {
        PathBuf::from(input)
    } else {
        cwd.join(input)
    };
    let mut out = PathBuf::from("/");
    for part in raw.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::Normal(p) => out.push(p),
            _ => {}
        }
    }
    out
}

/// Tab completion for a typed path, like Finder's: complete the last component to
/// the longest prefix all matching names share (case-insensitive), adding `/` when
/// exactly one folder matches. `None` when nothing matches.
pub fn complete_typed(input: &str, cwd: &Path, home: &Path) -> Option<String> {
    let (parent_text, partial) = match input.rfind('/') {
        Some(i) => (&input[..=i], &input[i + 1..]),
        None if input == "~" => return Some("~/".into()),
        None => ("", input),
    };
    let parent = resolve_typed(if parent_text.is_empty() { "." } else { parent_text }, cwd, home);
    let wanted = partial.to_lowercase();
    let mut matches: Vec<(String, bool)> = std::fs::read_dir(parent)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let visible = !name.starts_with('.') || partial.starts_with('.');
            (visible && name.to_lowercase().starts_with(&wanted)).then(|| (name, e.path().is_dir()))
        })
        .collect();
    matches.sort();
    let (first, first_is_dir) = matches.first()?.clone();
    if matches.len() == 1 {
        let slash = if first_is_dir { "/" } else { "" };
        return Some(format!("{parent_text}{first}{slash}"));
    }
    let common = matches.iter().skip(1).fold(first.clone(), |acc, (name, _)| common_prefix(&acc, name));
    Some(format!(
        "{parent_text}{}",
        if common.chars().count() >= partial.chars().count() { common } else { partial.to_owned() }
    ))
}

/// Longest common prefix, compared case-insensitively, keeping the first name's spelling.
fn common_prefix(a: &str, b: &str) -> String {
    a.chars().zip(b.chars()).take_while(|(x, y)| x.to_lowercase().eq(y.to_lowercase())).map(|(x, _)| x).collect()
}

/// A path as people paste it, made plain: surrounding whitespace and matching quotes
/// go, a `file://` URL becomes its path, and a shell's backslash escapes (`My\ Files`)
/// are undone. `None` when the text isn't a path (`/…`, `~/…` or `~`).
pub fn clean_pasted_path(input: &str) -> Option<String> {
    let mut text = input.trim();
    for quote in ['\'', '"'] {
        if text.len() >= 2 && text.starts_with(quote) && text.ends_with(quote) {
            text = text[1..text.len() - 1].trim();
        }
    }
    let text = match text.strip_prefix("file://") {
        Some(rest) => percent_decode(rest.strip_prefix("localhost").unwrap_or(rest))?,
        None if text.contains('\\') => unescape_shell(text),
        None => text.to_owned(),
    };
    (text.starts_with('/') || text.starts_with("~/") || text == "~").then_some(text)
}

/// `%20` and friends; `None` when the bytes aren't UTF-8.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (bytes[i], bytes.get(i + 1).and_then(|&b| hex(b)), bytes.get(i + 2).and_then(|&b| hex(b))) {
            (b'%', Some(hi), Some(lo)) => {
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            (byte, ..) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// `a\ b` → `a b`: each backslash keeps the character after it.
fn unescape_shell(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.extend(chars.next()),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_home_relative_and_dots() {
        let (cwd, home) = (Path::new("/Users/me/Files"), Path::new("/Users/me"));
        assert_eq!(resolve_typed("~", cwd, home), PathBuf::from("/Users/me"));
        assert_eq!(resolve_typed("~/Documents/", cwd, home), PathBuf::from("/Users/me/Documents"));
        assert_eq!(resolve_typed("../Downloads", cwd, home), PathBuf::from("/Users/me/Downloads"));
        assert_eq!(resolve_typed("/tmp/./x/..", cwd, home), PathBuf::from("/tmp"));
        assert_eq!(resolve_typed("  notes ", cwd, home), PathBuf::from("/Users/me/Files/notes"));
        assert_eq!(resolve_typed("/../..", cwd, home), PathBuf::from("/"));
    }

    #[test]
    fn completes_unique_folders_and_shared_prefixes() {
        let root = std::env::temp_dir().join(format!("crow-core-complete-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["Documents", "Downloads", "Desktop", ".hidden"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::write(root.join("notes.txt"), "").unwrap();
        let home = Path::new("/nowhere");
        let at = |s: &str| complete_typed(s, &root, home);

        assert_eq!(at("Doc").as_deref(), Some("Documents/"), "one folder: completed with a slash");
        assert_eq!(at("do").as_deref(), Some("Do"), "several: their shared prefix, in their spelling");
        assert_eq!(at("des").as_deref(), Some("Desktop/"), "the typed tail matches case-insensitively");
        assert_eq!(at("no").as_deref(), Some("notes.txt"), "a file gets no slash");
        assert_eq!(at("zz"), None);
        assert_eq!(at(".h").as_deref(), Some(".hidden/"), "hidden names only when asked for");
        let absolute = format!("{}/Des", root.display());
        assert_eq!(complete_typed(&absolute, Path::new("/"), home), Some(format!("{}/Desktop/", root.display())));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn pasted_paths_come_out_plain() {
        let clean = |s: &str| clean_pasted_path(s);
        assert_eq!(clean("/a/b.txt").as_deref(), Some("/a/b.txt"));
        assert_eq!(clean("  /a/b.txt \n").as_deref(), Some("/a/b.txt"), "whitespace");
        assert_eq!(clean("'/a/My File.txt'").as_deref(), Some("/a/My File.txt"), "single quotes");
        assert_eq!(clean("\"~/a b\"").as_deref(), Some("~/a b"), "double quotes");
        assert_eq!(clean("/a/My\\ File\\ \\(1\\).txt").as_deref(), Some("/a/My File (1).txt"), "shell escapes");
        assert_eq!(clean("file:///a/My%20File.txt").as_deref(), Some("/a/My File.txt"), "file URL");
        assert_eq!(clean("file://localhost/a/%C3%A9").as_deref(), Some("/a/é"), "localhost and UTF-8");
        assert_eq!(clean("~").as_deref(), Some("~"));
        assert_eq!(clean("alpha"), None, "a name is not a path");
        assert_eq!(clean("'alpha'"), None);
        assert_eq!(clean("'/a"), None, "an unmatched quote is not stripped");
    }
}
