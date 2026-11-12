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
            sets.push(Set { name: name.id(), patterns, sentinels, paths, dot_folders: rules.dot_folders });
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
        }
        None
    }
}

fn compile(set: &str, pattern: &str) -> Result<GlobMatcher, String> {
    GlobBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .map(|glob| glob.compile_matcher())
        .map_err(|e| format!("set \"{set}\": bad pattern \"{pattern}\": {e}"))
}
