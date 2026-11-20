//! Shared plumbing for Crow's config files: where they live, writing them without
//! losing the user's comments, and `~` in paths.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use toml_edit::DocumentMut;

/// `$CROW_STATE_DIR`, else `$XDG_STATE_HOME/crow`, else `~/.local/state/crow`: data
/// Crow keeps for itself (recent folders), as opposed to settings people edit.
pub fn state_dir() -> PathBuf {
    if let Some(dir) = env::var_os("CROW_STATE_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(dir) = env::var_os("XDG_STATE_HOME") {
        return PathBuf::from(dir).join("crow");
    }
    home().join(".local").join("state").join("crow")
}

/// `$CROW_CACHE_DIR`, else `~/Library/Caches/crow` on macOS, else
/// `$XDG_CACHE_HOME/crow`, else `~/.cache/crow`: data that can be re-computed
/// (folder sizes). Time Machine skips `~/Library/Caches`, so losing it is cheap.
pub fn cache_dir() -> PathBuf {
    if let Some(dir) = env::var_os("CROW_CACHE_DIR") {
        return PathBuf::from(dir);
    }
    #[cfg(target_os = "macos")]
    {
        home().join("Library").join("Caches").join("crow")
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Some(dir) = env::var_os("XDG_CACHE_HOME") {
            return PathBuf::from(dir).join("crow");
        }
        home().join(".cache").join("crow")
    }
}

pub fn home() -> PathBuf {
    env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

/// `~` and `~/…` against `home`; anything else as written.
pub fn expand_home(path: &str, home: &Path) -> PathBuf {
    match path.strip_prefix('~') {
        Some("") => home.to_path_buf(),
        Some(rest) if rest.starts_with('/') => home.join(&rest[1..]),
        _ => PathBuf::from(path),
    }
}

/// `~/…` for paths in `home`, so config files read naturally and survive a new user name.
pub fn abbreviate_home(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Write via a temp file and rename, so a watcher (or a crash) never sees half a file.
pub fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));
    fs::write(&tmp, text)?;
    fs::rename(tmp, path)
}

/// A TOML error with the line it points at: "line 12: unknown field `sortt`".
/// Error strings keep this prefix so `file_message` can put the path before it.
pub fn parse_error(error: &toml::de::Error, text: &str) -> String {
    match error.span() {
        Some(span) => {
            let head = text.get(..span.start.min(text.len())).unwrap_or(text);
            let line = head.matches('\n').count() + 1;
            format!("line {line}: {}", error.message())
        }
        None => error.message().to_owned(),
    }
}

/// Join an error with its file: "views.toml line 12: …", "sidebar.toml: …".
pub fn file_message(path: &Path, message: String) -> String {
    match message.strip_prefix("line ") {
        Some(rest) => format!("{} line {rest}", path.display()),
        None => format!("{}: {message}", path.display()),
    }
}

/// Set `key` in `table`, keeping the comment and spacing written beside the old
/// value, so an edit from Crow does not eat `# my note`.
pub fn set_value(table: &mut toml_edit::Table, key: &str, mut new: toml_edit::Value) {
    if let Some(old) = table.get(key).and_then(toml_edit::Item::as_value) {
        *new.decor_mut() = old.decor().clone();
    }
    table[key] = toml_edit::Item::Value(new);
}

/// Read `path` as an editable document (`default` if it doesn't exist), let `edit`
/// change it, and write it back with every comment the user wrote still in place.
pub fn edit_toml(
    path: &Path,
    default: &str,
    edit: impl FnOnce(&mut DocumentMut) -> Result<(), String>,
) -> Result<(), String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => default.to_owned(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let mut doc: DocumentMut = text.parse().map_err(|e| format!("{}: {e}", path.display()))?;
    edit(&mut doc)?;
    write_atomic(path, &doc.to_string()).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_paths_round_trip() {
        let home = Path::new("/Users/me");
        assert_eq!(expand_home("~", home), PathBuf::from("/Users/me"));
        assert_eq!(expand_home("~/Pictures", home), PathBuf::from("/Users/me/Pictures"));
        assert_eq!(expand_home("~other/x", home), PathBuf::from("~other/x"));
        assert_eq!(abbreviate_home(Path::new("/Users/me/Pictures"), home), "~/Pictures");
        assert_eq!(abbreviate_home(Path::new("/Users/me"), home), "~");
        assert_eq!(abbreviate_home(Path::new("/Applications"), home), "/Applications");
    }

    #[test]
    fn edits_keep_comments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.toml");
        fs::write(&path, "# mine\ntheme = \"a\" # why\nfont_size = 13.0\n").unwrap();
        edit_toml(&path, "", |doc| {
            doc["theme"] = toml_edit::value("b");
            Ok(())
        })
        .unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# mine\ntheme = \"b\""), "{text}");
        assert!(text.contains("font_size = 13.0"), "{text}");
    }
}
