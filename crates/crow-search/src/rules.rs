//! The compiled form of `exclude.toml`'s sets: folder-name globs, marker
//! ("sentinel") rules and absolute paths. A search asks `skip` about every
//! directory and never descends into one that matches.

use std::path::{Path, PathBuf};

use crow_core::{Exclude, SetName, expand_home};
use globset::{GlobBuilder, GlobMatcher};

#[derive(Clone, Debug, Default)]
pub struct RuleSet {
    sets: Vec<Set>,
}

#[derive(Clone, Debug)]
struct Set {
    name: &'static str,
    patterns: Vec<(GlobMatcher, String)>,
    sentinels: Vec<(GlobMatcher, String, Vec<String>)>,
    paths: Vec<(PathBuf, String)>,
    dot_folders: bool,
    time_machine: bool,
}

impl RuleSet {
    /// No rules, for "Search everything".
    pub fn empty() -> RuleSet {
        RuleSet::default()
    }

    /// Compile the enabled sets in `config`. `home` expands `~/…` in path rules.
    pub fn from_config(config: &Exclude, home: &Path) -> Result<RuleSet, String> {
        let mut sets = Vec::new();
        for name in SetName::ALL {
            let rules = config.set(name);
            if !rules.enabled {
                continue;
            }
            let mut patterns = Vec::new();
            for pattern in &rules.names {
                patterns.push((compile(name.id(), pattern)?, pattern.clone()));
            }
            let mut sentinels = Vec::new();
            for (folder, markers) in &rules.sentinel {
                sentinels.push((compile(name.id(), folder)?, folder.clone(), markers.clone()));
            }
            let paths = rules.paths.iter().map(|path| (expand_home(path, home), path.clone())).collect();
            sets.push(Set {
                name: name.id(),
                patterns,
                sentinels,
                paths,
                dot_folders: rules.dot_folders,
                time_machine: rules.time_machine,
            });
        }
        Ok(RuleSet { sets })
    }

    /// Why `path` (a directory named `name`) should be skipped, if it should.
    /// Files never match: the rules describe folders.
    pub(crate) fn skip(&self, path: &Path, name: &str, is_dir: bool) -> Option<String> {
        if !is_dir {
            return None;
        }
        for set in &self.sets {
            if set.dot_folders && name.starts_with('.') {
                return Some(format!("{}: hidden folder", set.name));
            }
            for (matcher, pattern) in &set.patterns {
                if matcher.is_match(name) {
                    return Some(format!("{}: name \"{pattern}\"", set.name));
                }
            }
            for (matcher, folder, markers) in &set.sentinels {
                if !matcher.is_match(name) {
                    continue;
                }
                let Some(parent) = path.parent() else { continue };
                for marker in markers {
                    if parent.join(marker).exists() {
                        return Some(format!("{}: {folder} beside {marker}", set.name));
                    }
                }
            }
            for (root, original) in &set.paths {
                if path.starts_with(root) {
                    return Some(format!("{}: path \"{original}\"", set.name));
                }
            }
            // Last, since it asks the disk: one attribute read per folder.
            if set.time_machine && has_backup_exclusion_mark(path) {
                return Some(format!("{}: excluded from Time Machine", set.name));
            }
        }
        None
    }
}

/// Whether `dir` carries Time Machine's sticky exclusion mark (what `tmutil addexclusion`
/// and Finder set). Only the attribute's presence matters, so its value is not read.
#[cfg(target_os = "macos")]
fn has_backup_exclusion_mark(dir: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else { return false };
    // SAFETY: both strings are NUL-terminated and outlive the call; a null buffer of size 0
    // asks only for the attribute's length.
    let size = unsafe {
        libc::getxattr(
            path.as_ptr(),
            c"com.apple.metadata:com_apple_backup_excludeItem".as_ptr(),
            std::ptr::null_mut(),
            0,
            0,
            libc::XATTR_NOFOLLOW,
        )
    };
    size >= 0
}

#[cfg(not(target_os = "macos"))]
fn has_backup_exclusion_mark(_: &Path) -> bool {
    false
}

fn compile(set: &str, pattern: &str) -> Result<GlobMatcher, String> {
    GlobBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .map(|glob| glob.compile_matcher())
        .map_err(|e| format!("set \"{set}\": bad pattern \"{pattern}\": {e}"))
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;

    /// Mark `dir` the way `tmutil addexclusion` does.
    fn mark_backup_excluded(dir: &Path) {
        let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).unwrap();
        let value = b"com.apple.backupd";
        // SAFETY: valid NUL-terminated strings and a buffer of the length passed.
        let status = unsafe {
            libc::setxattr(
                path.as_ptr(),
                c"com.apple.metadata:com_apple_backup_excludeItem".as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
                0,
            )
        };
        assert_eq!(status, 0, "set the exclusion mark");
    }

    fn rules(enabled: bool) -> RuleSet {
        let backup_excluded =
            crow_core::ExcludeSet { enabled: Some(enabled), time_machine: Some(true), ..Default::default() };
        let config = Exclude { backup_excluded, ..Exclude::default() };
        RuleSet::from_config(&config, Path::new("/Users/nobody")).unwrap()
    }

    #[test]
    fn the_time_machine_set_skips_marked_folders_only_when_on() {
        let root = tempfile::tempdir().unwrap();
        let marked = root.path().join("marked");
        let plain = root.path().join("plain");
        std::fs::create_dir_all(&marked).unwrap();
        std::fs::create_dir_all(&plain).unwrap();
        mark_backup_excluded(&marked);

        let on = rules(true);
        let reason = on.skip(&marked, "marked", true).expect("a marked folder is skipped");
        assert!(reason.contains("backup-excluded") && reason.contains("Time Machine"), "{reason}");
        assert_eq!(on.skip(&plain, "plain", true), None, "an unmarked folder is searched");
        assert_eq!(on.skip(&marked, "marked", false), None, "files are never skipped");

        assert_eq!(rules(false).skip(&marked, "marked", true), None, "off by default");
        let default = RuleSet::from_config(&Exclude::default(), Path::new("/Users/nobody")).unwrap();
        assert_eq!(default.skip(&marked, "marked", true), None);
    }
}
