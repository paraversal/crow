use std::path::{Path, PathBuf};

use objc2::rc::autoreleasepool;
use objc2_foundation::NSFileManager;

use crate::{main_thread, ns_string};

/// The paths `display_components` names: `path`'s ancestors from the volume down.
/// Paths on other volumes start at the volume.
pub fn display_component_paths(path: &Path) -> Vec<PathBuf> {
    let on_volume = path.starts_with("/Volumes");
    let mut ancestors: Vec<&Path> = path.ancestors().collect();
    ancestors.reverse();
    ancestors
        .into_iter()
        .filter(|p| *p != Path::new("/Volumes") && !(on_volume && *p == Path::new("/")))
        .map(Path::to_path_buf)
        .collect()
}

/// Finder's name for UI callers. Off the main thread use the path component.
pub fn display_name_at(path: &Path) -> String {
    match main_thread() {
        Some(_) => NSFileManager::defaultManager().displayNameAtPath(&ns_string(path)).to_string(),
        None => plain_name(path),
    }
}

/// Resolve a localized name on a filesystem worker. Foundation's shared file
/// manager is thread-safe.
pub fn localized_display_name_at(path: &Path) -> String {
    let Some(exact_path) = path.to_str() else { return plain_name(path) };
    autoreleasepool(|_| {
        let display_name = NSFileManager::defaultManager().displayNameAtPath(&ns_string(path)).to_string();
        // Foundation returns the full input path when it cannot resolve a name.
        if display_name == exact_path || display_name.is_empty() { plain_name(path) } else { display_name }
    })
}

/// Finder's path components for `path`: `Macintosh HD › Users › me › …`,
/// localized the way Finder shows it. Paths on other volumes start at the volume.
pub fn display_components(path: &Path) -> Vec<(String, PathBuf)> {
    display_component_paths(path).into_iter().map(|p| (display_name_at(&p), p)).collect()
}

fn plain_name(path: &Path) -> String {
    path.file_name().map_or_else(|| "/".to_owned(), |n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_worker_resolves_the_same_display_name_as_the_calling_thread() {
        let volume = Path::new("/");
        let name = std::thread::spawn(move || localized_display_name_at(volume)).join().unwrap();
        assert_eq!(name, localized_display_name_at(volume));
        assert!(!name.is_empty());
    }

    #[test]
    fn nonexistent_names_fall_back_to_the_path_components() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("missing/Notes");
        assert_eq!(localized_display_name_at(path.parent().unwrap()), "missing");
        assert_eq!(localized_display_name_at(&path), "Notes");
    }
}
