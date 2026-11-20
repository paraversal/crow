use std::path::{Path, PathBuf};

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

/// Finder's name for one path, localized the way Finder shows it. This is a
/// LaunchServices filesystem query, so callers cache it. Off the main thread
/// there is no AppKit, and the file name stands in.
pub fn display_name_at(path: &Path) -> String {
    match main_thread() {
        Some(_) => NSFileManager::defaultManager().displayNameAtPath(&ns_string(path)).to_string(),
        None => plain_name(path),
    }
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
    fn off_main_thread_uses_plain_names() {
        let names: Vec<String> =
            display_components(Path::new("/Users/me/Notes")).into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, ["/", "Users", "me", "Notes"]);
        let on_volume: Vec<String> =
            display_components(Path::new("/Volumes/Backup/x")).into_iter().map(|(name, _)| name).collect();
        assert_eq!(on_volume, ["Backup", "x"]);
    }
}
