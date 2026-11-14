use std::path::{Path, PathBuf};

use objc2_foundation::NSFileManager;

use crate::{main_thread, ns_string};

/// Finder's path components for `path`: `Macintosh HD › Users › me › …`,
/// localized the way Finder shows them. Paths on other volumes start at the volume.
pub fn display_components(path: &Path) -> Vec<(String, PathBuf)> {
    let on_volume = path.starts_with("/Volumes");
    let mut ancestors: Vec<&Path> = path.ancestors().collect();
    ancestors.reverse();
    let ancestors =
        ancestors.into_iter().filter(|p| *p != Path::new("/Volumes") && !(on_volume && *p == Path::new("/")));
    match main_thread() {
        Some(_) => {
            let manager = NSFileManager::defaultManager();
            ancestors.map(|p| (manager.displayNameAtPath(&ns_string(p)).to_string(), p.to_path_buf())).collect()
        }
        None => ancestors.map(|p| (plain_name(p), p.to_path_buf())).collect(),
    }
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
