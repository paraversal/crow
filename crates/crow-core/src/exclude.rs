//! Search exclusions from `exclude.toml`: named rule sets of folder names,
//! marker ("sentinel") rules and absolute paths. `dev` is on by default; the
//! other sets stay off until enabled. Missing fields fall back to the built-in
//! defaults, so deleting a line restores it.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use globset::GlobBuilder;
use serde::Deserialize;

use crate::config::write_atomic;

/// The documented default `exclude.toml`, written on first launch. Parsing it
/// must give exactly `Exclude::default()` (a test checks this).
pub const DEFAULT_EXCLUDE: &str = r#"# Crow search exclusions. A search skips matching folders and counts them in the
# results summary; "Search everything" (Cmd-Shift-A) ignores every set.
# Delete a line to get the built-in default for that field back.

[dev]
# Build and dependency folders developers rarely want in results. On by default.
enabled = true
# Folder names or glob patterns, skipped anywhere.
names = [
  "node_modules", ".git", "__pycache__", ".venv", "venv", ".tox",
  ".mypy_cache", ".pytest_cache", "DerivedData", ".next", ".nuxt",
  ".svelte-kit", ".gradle", ".idea", ".cache", ".logicprojects",
]
# Skip a folder when its parent contains one of these marker files.
[dev.sentinel]
target = ["Cargo.toml"]
build = ["package.json", "pyproject.toml", "setup.py", "build.gradle", "settings.gradle", "CMakeLists.txt", "Makefile"]
dist = ["package.json", "pyproject.toml", "setup.py"]
Pods = ["Podfile"]

[dependencies]
# Dependency folders of other ecosystems. Off by default.
enabled = false
names = ["vendor", ".bundle", "bower_components"]

[media]
# Media libraries are packages; searching their insides is rarely useful.
enabled = false
names = ["*.photoslibrary", "*.photolibrary", "*.imovielibrary", "*.fcpbundle"]

[hidden]
# Every folder whose name starts with a dot.
enabled = false
dot_folders = true

[system]
# System folders. Off by default.
enabled = false
paths = ["/System", "/Library/Caches"]
names = [".Trashes"]
"#;

const DEV_NAMES: &[&str] = &[
    "node_modules",
    ".git",
    "__pycache__",
    ".venv",
    "venv",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
    "DerivedData",
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".gradle",
    ".idea",
    ".cache",
    ".logicprojects",
];
const DEPENDENCIES_NAMES: &[&str] = &["vendor", ".bundle", "bower_components"];
const MEDIA_NAMES: &[&str] = &["*.photoslibrary", "*.photolibrary", "*.imovielibrary", "*.fcpbundle"];
const SYSTEM_NAMES: &[&str] = &[".Trashes"];
const SYSTEM_PATHS: &[&str] = &["/System", "/Library/Caches"];

/// The sets `exclude.toml` knows. Unknown sets are errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetName {
    Dev,
    Dependencies,
    Media,
    Hidden,
    System,
}

impl SetName {
    pub const ALL: [SetName; 5] =
        [SetName::Dev, SetName::Dependencies, SetName::Media, SetName::Hidden, SetName::System];

    pub fn id(self) -> &'static str {
        match self {
            SetName::Dev => "dev",
            SetName::Dependencies => "dependencies",
            SetName::Media => "media",
            SetName::Hidden => "hidden",
            SetName::System => "system",
        }
    }

    fn built_in(self) -> SetRules {
        let names = |names: &[&str]| names.iter().map(|n| (*n).to_owned()).collect();
        match self {
            SetName::Dev => SetRules {
                enabled: true,
                names: names(DEV_NAMES),
                paths: Vec::new(),
                sentinel: dev_sentinel(),
                dot_folders: false,
            },
            SetName::Dependencies => SetRules {
                enabled: false,
                names: names(DEPENDENCIES_NAMES),
                paths: Vec::new(),
                sentinel: BTreeMap::new(),
                dot_folders: false,
            },
            SetName::Media => SetRules {
                enabled: false,
                names: names(MEDIA_NAMES),
                paths: Vec::new(),
                sentinel: BTreeMap::new(),
                dot_folders: false,
            },
            SetName::Hidden => SetRules {
                enabled: false,
                names: Vec::new(),
                paths: Vec::new(),
                sentinel: BTreeMap::new(),
                dot_folders: true,
            },
            SetName::System => SetRules {
                enabled: false,
                names: names(SYSTEM_NAMES),
                paths: names(SYSTEM_PATHS),
                sentinel: BTreeMap::new(),
                dot_folders: false,
            },
        }
    }
}

/// One set's values in the file; `None` means "use the built-in default".
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExcludeSet {
    pub enabled: Option<bool>,
    /// Folder names or glob patterns, matched case-insensitively.
    pub names: Option<Vec<String>>,
    /// Absolute paths (or `~/…`) whose folders and contents are skipped.
    pub paths: Option<Vec<String>>,
    /// Folder name or glob -> marker files that must sit beside it.
    pub sentinel: Option<BTreeMap<String, Vec<String>>>,
    /// Skip every folder whose name starts with a dot.
    pub dot_folders: Option<bool>,
}

/// A set with the file's values and the built-in defaults merged in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetRules {
    pub enabled: bool,
    pub names: Vec<String>,
    pub paths: Vec<String>,
    pub sentinel: BTreeMap<String, Vec<String>>,
    pub dot_folders: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Exclude {
    pub dev: ExcludeSet,
    pub dependencies: ExcludeSet,
    pub media: ExcludeSet,
    pub hidden: ExcludeSet,
    pub system: ExcludeSet,
}

impl Default for Exclude {
    fn default() -> Self {
        Self {
            dev: ExcludeSet {
                enabled: Some(true),
                names: Some(DEV_NAMES.iter().map(|n| (*n).to_owned()).collect()),
                sentinel: Some(dev_sentinel()),
                ..ExcludeSet::default()
            },
            dependencies: ExcludeSet {
                enabled: Some(false),
                names: Some(DEPENDENCIES_NAMES.iter().map(|n| (*n).to_owned()).collect()),
                ..ExcludeSet::default()
            },
            media: ExcludeSet {
                enabled: Some(false),
                names: Some(MEDIA_NAMES.iter().map(|n| (*n).to_owned()).collect()),
                ..ExcludeSet::default()
            },
            hidden: ExcludeSet { enabled: Some(false), dot_folders: Some(true), ..ExcludeSet::default() },
            system: ExcludeSet {
                enabled: Some(false),
                names: Some(SYSTEM_NAMES.iter().map(|n| (*n).to_owned()).collect()),
                paths: Some(SYSTEM_PATHS.iter().map(|p| (*p).to_owned()).collect()),
                ..ExcludeSet::default()
            },
        }
    }
}

impl Exclude {
    /// `name`'s rules with the file's values over the built-in defaults.
    pub fn set(&self, name: SetName) -> SetRules {
        let built_in = name.built_in();
        let configured = match name {
            SetName::Dev => &self.dev,
            SetName::Dependencies => &self.dependencies,
            SetName::Media => &self.media,
            SetName::Hidden => &self.hidden,
            SetName::System => &self.system,
        };
        SetRules {
            enabled: configured.enabled.unwrap_or(built_in.enabled),
            names: configured.names.clone().unwrap_or(built_in.names),
            paths: configured.paths.clone().unwrap_or(built_in.paths),
            sentinel: configured.sentinel.clone().unwrap_or(built_in.sentinel),
            dot_folders: configured.dot_folders.unwrap_or(built_in.dot_folders),
        }
    }

    /// Reject folder globs that don't compile, so a broken `exclude.toml` shows
    /// in the status line at load instead of failing a search.
    pub fn validate(&self) -> Result<(), String> {
        for name in SetName::ALL {
            let rules = self.set(name);
            if !rules.enabled {
                continue;
            }
            for pattern in &rules.names {
                check_glob(name.id(), pattern)?;
            }
            for pattern in rules.sentinel.keys() {
                check_glob(name.id(), pattern)?;
            }
        }
        Ok(())
    }
}

fn dev_sentinel() -> BTreeMap<String, Vec<String>> {
    [
        ("target", &["Cargo.toml"][..]),
        (
            "build",
            &[
                "package.json",
                "pyproject.toml",
                "setup.py",
                "build.gradle",
                "settings.gradle",
                "CMakeLists.txt",
                "Makefile",
            ][..],
        ),
        ("dist", &["package.json", "pyproject.toml", "setup.py"][..]),
        ("Pods", &["Podfile"][..]),
    ]
    .into_iter()
    .map(|(folder, markers)| (folder.to_owned(), markers.iter().map(|marker| (*marker).to_owned()).collect()))
    .collect()
}

fn check_glob(set: &str, pattern: &str) -> Result<(), String> {
    GlobBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .map(|_| ())
        .map_err(|e| format!("set \"{set}\": bad pattern \"{pattern}\": {e}"))
}

pub fn exclude_path(config_dir: &Path) -> PathBuf {
    config_dir.join("exclude.toml")
}

/// Load `exclude.toml`, writing the documented default first if it's missing.
/// A broken file keeps the caller's last good rules and shows the message.
pub fn load_exclude(config_dir: &Path) -> Result<Exclude, String> {
    let path = exclude_path(config_dir);
    if !path.exists() {
        write_atomic(&path, DEFAULT_EXCLUDE).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let exclude: Exclude = toml::from_str(&text).map_err(|e| format!("{}: {}", path.display(), e.message()))?;
    exclude.validate().map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(exclude)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("crow-core-exclude-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn documented_defaults_match_code_defaults() {
        let parsed: Exclude = toml::from_str(DEFAULT_EXCLUDE).expect("default file parses");
        assert_eq!(parsed, Exclude::default());
    }

    #[test]
    fn missing_file_is_created_with_defaults() {
        let dir = temp_dir("create");
        let exclude = load_exclude(&dir).expect("loads");
        assert_eq!(fs::read_to_string(dir.join("exclude.toml")).unwrap(), DEFAULT_EXCLUDE);
        assert_eq!(exclude, Exclude::default());
    }

    #[test]
    fn partial_set_keeps_built_in_values() {
        let dir = temp_dir("partial");
        fs::write(dir.join("exclude.toml"), "[dev]\nenabled = false\n").unwrap();
        let dev = load_exclude(&dir).unwrap().set(SetName::Dev);
        assert!(!dev.enabled);
        assert!(dev.names.contains(&"node_modules".to_owned()));
        assert_eq!(dev.sentinel.get("target"), Some(&vec!["Cargo.toml".to_owned()]));
    }

    #[test]
    fn explicit_empty_names_stay_empty() {
        let dir = temp_dir("empty");
        fs::write(dir.join("exclude.toml"), "[dev]\nnames = []\n").unwrap();
        assert!(load_exclude(&dir).unwrap().set(SetName::Dev).names.is_empty());
    }

    #[test]
    fn bad_glob_names_the_file_and_pattern() {
        let dir = temp_dir("badglob");
        fs::write(dir.join("exclude.toml"), "[dev]\nnames = [\"[\"]\n").unwrap();
        let error = load_exclude(&dir).unwrap_err();
        assert!(error.contains("exclude.toml"), "{error}");
        assert!(error.contains("bad pattern"), "{error}");
        assert!(error.contains("dev"), "{error}");
    }

    #[test]
    fn unknown_keys_are_errors() {
        let dir = temp_dir("unknown");
        fs::write(dir.join("exclude.toml"), "[dev]\nnope = 1\n").unwrap();
        let error = load_exclude(&dir).unwrap_err();
        assert!(error.contains("exclude.toml"), "{error}");
    }

    #[test]
    fn logic_project_bundles_are_in_the_default_dev_set() {
        assert!(Exclude::default().set(SetName::Dev).names.contains(&".logicprojects".to_owned()));
    }

    #[test]
    fn disabled_sets_stay_off_until_enabled() {
        let exclude = Exclude::default();
        assert!(exclude.set(SetName::Dev).enabled);
        for name in [SetName::Dependencies, SetName::Media, SetName::Hidden, SetName::System] {
            assert!(!exclude.set(name).enabled, "{} starts off", name.id());
        }
    }
}
