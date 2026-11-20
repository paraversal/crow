//! The folders Finder's Empty Trash covers and the status line's wording.
//! The deleting itself is `crow_fs::trash`, run off the main thread.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use crow_fs::trash::Report;

/// The home Trash and each mounted volume's `.Trashes/<uid>`. Reads the home's
/// metadata for the uid, so call it off the main thread.
pub fn folders(home: &Path, volumes: &[PathBuf]) -> Vec<PathBuf> {
    let mut folders = vec![home.join(".Trash")];
    // io-ok: called from the empty-trash background count and removal tasks.
    let Ok(metadata) = std::fs::metadata(home) else { return folders };
    let uid = metadata.uid();
    for volume in volumes {
        let folder = volume.join(".Trashes").join(uid.to_string());
        if !folders.contains(&folder) {
            folders.push(folder);
        }
    }
    folders
}

/// The folders a jailed run (the fuzzer) may touch: only ones inside its tree.
/// Production has no jail. Mounted-volume trashes would otherwise be the user's
/// real Trash during a fuzz run.
pub fn in_jail(folders: Vec<PathBuf>, jail: Option<&Path>) -> Vec<PathBuf> {
    match jail {
        Some(jail) => folders.into_iter().filter(|folder| folder.starts_with(jail)).collect(),
        None => folders,
    }
}

/// Full Disk Access, where the user lets Crow read the Trash.
pub const PRIVACY_SETTINGS_URL: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles";

/// Said instead of "already empty" when a Trash folder cannot be listed and
/// there is no Finder fallback (a jailed run).
pub fn denied_message(denied: &[PathBuf]) -> String {
    let name = denied.first().map_or_else(String::new, |path| quoted_name(path));
    format!("Crow can't read the Trash {name}. Allow Full Disk Access in System Settings.")
}

/// Finder's own refusal (often a declined Automation prompt), kept readable.
pub fn finder_failure(error: &str) -> String {
    format!("Finder could not empty the Trash: {}", error.lines().next().unwrap_or("unknown error"))
}

/// What the status line says after `empty`. The second item is "this is an
/// error (red, stays)"; a partial failure is still an error.
pub fn message(report: &Report) -> (String, bool) {
    if report.is_empty() {
        return ("The Trash is already empty.".to_owned(), false);
    }
    let mut text = match report.removed {
        0 => "Could not empty the Trash".to_owned(),
        1 => "Emptied the Trash: 1 item".to_owned(),
        n => format!("Emptied the Trash: {n} items"),
    };
    if report.failed.is_empty() {
        text.push('.');
        return (text, false);
    }
    let names: Vec<String> = report.failed.iter().take(3).map(|failure| quoted_name(&failure.path)).collect();
    let more = report.failed.len().saturating_sub(names.len());
    text.push_str(&format!(
        "{} {} could not be removed: {}{}.",
        if report.removed == 0 { "." } else { ";" },
        report.failed.len(),
        names.join(", "),
        if more > 0 { format!(", and {more} more") } else { String::new() }
    ));
    (text, true)
}

fn quoted_name(path: &Path) -> String {
    let name = path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
    format!("“{}”", crate::style::display_text(&name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crow_fs::trash::Failure;

    #[test]
    fn an_empty_trash_says_so_without_an_error() {
        assert_eq!(message(&Report::default()), ("The Trash is already empty.".to_owned(), false));
    }

    #[test]
    fn removed_items_are_counted_in_singular_and_plural() {
        assert_eq!(
            message(&Report { removed: 1, failed: Vec::new() }),
            ("Emptied the Trash: 1 item.".to_owned(), false)
        );
        assert_eq!(
            message(&Report { removed: 12, failed: Vec::new() }),
            ("Emptied the Trash: 12 items.".to_owned(), false)
        );
    }

    #[test]
    fn failures_are_named_and_capped() {
        let failed = |name: &str| Failure { path: PathBuf::from("/tmp").join(name), error: "denied".into() };
        let report =
            Report { removed: 2, failed: vec![failed("a.txt"), failed("b.txt"), failed("c.txt"), failed("d.txt")] };
        assert_eq!(
            message(&report),
            (
                "Emptied the Trash: 2 items; 4 could not be removed: “a.txt”, “b.txt”, “c.txt”, and 1 more.".to_owned(),
                true
            )
        );
    }

    #[test]
    fn nothing_removed_is_an_error() {
        let report =
            Report { removed: 0, failed: vec![Failure { path: PathBuf::from("/tmp/x"), error: "denied".into() }] };
        assert_eq!(message(&report), ("Could not empty the Trash. 1 could not be removed: “x”.".to_owned(), true));
    }

    #[test]
    fn a_jail_keeps_only_folders_inside_it() {
        let folders = vec![PathBuf::from("/tmp/crow-tree/.Trash"), PathBuf::from("/Volumes/One/.Trashes/501")];
        assert_eq!(in_jail(folders.clone(), None), folders);
        assert_eq!(in_jail(folders, Some(Path::new("/tmp/crow-tree"))), vec![PathBuf::from("/tmp/crow-tree/.Trash")]);
    }

    #[test]
    fn folders_cover_home_and_every_volume_for_this_user() {
        let home = tempfile::Builder::new().prefix("crow-trash-home-").tempdir().expect("create home");
        let uid = std::fs::metadata(home.path()).expect("stat home").uid();
        let folders = folders(home.path(), &[PathBuf::from("/Volumes/One"), PathBuf::from("/Volumes/Two")]);
        assert_eq!(folders[0], home.path().join(".Trash"));
        assert!(folders.contains(&PathBuf::from(format!("/Volumes/One/.Trashes/{uid}"))), "{folders:?}");
        assert!(folders.contains(&PathBuf::from(format!("/Volumes/Two/.Trashes/{uid}"))), "{folders:?}");
        assert_eq!(folders.len(), 3, "no duplicates");
    }
}
