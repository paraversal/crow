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

[backup-excluded]
# Folders carrying the Time Machine exclusion mark, whichever tool set it: `tmutil
# addexclusion`, Finder's Options, or asimeow (https://github.com/paraversal/asimeow),
# which marks dev folders such as node_modules and target. Off by default: "not backed up"
# does not mean "never search it". Exclusions made with `tmutil addexclusion -p` leave no
# mark on the folder, so they do not count. Checking costs one attribute read per folder.
enabled = false
time_machine = true
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
    /// Folders marked as excluded from Time Machine backups.
    BackupExcluded,
}

impl SetName {
    pub const ALL: [SetName; 6] = [
        SetName::Dev,
        SetName::Dependencies,
        SetName::Media,
        SetName::Hidden,
        SetName::System,
        SetName::BackupExcluded,
    ];

    pub fn id(self) -> &'static str {
        match self {
            SetName::Dev => "dev",
            SetName::Dependencies => "dependencies",
            SetName::Media => "media",
            SetName::Hidden => "hidden",
            SetName::System => "system",
            SetName::BackupExcluded => "backup-excluded",
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
                time_machine: false,
            },
            SetName::Dependencies => SetRules {
                enabled: false,
                names: names(DEPENDENCIES_NAMES),
                paths: Vec::new(),
                sentinel: BTreeMap::new(),
                dot_folders: false,
                time_machine: false,
            },
            SetName::Media => SetRules {
                enabled: false,
                names: names(MEDIA_NAMES),
                paths: Vec::new(),
                sentinel: BTreeMap::new(),
                dot_folders: false,
                time_machine: false,
            },
            SetName::Hidden => SetRules {
                enabled: false,
                names: Vec::new(),
                paths: Vec::new(),
                sentinel: BTreeMap::new(),
                dot_folders: true,
                time_machine: false,
            },
            SetName::System => SetRules {
                enabled: false,
                names: names(SYSTEM_NAMES),
                paths: names(SYSTEM_PATHS),
                sentinel: BTreeMap::new(),
                dot_folders: false,
                time_machine: false,
            },
            SetName::BackupExcluded => SetRules {
                enabled: false,
                names: Vec::new(),
                paths: Vec::new(),
                sentinel: BTreeMap::new(),
                dot_folders: false,
                time_machine: true,
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
    /// Skip every folder carrying the Time Machine exclusion mark.
    pub time_machine: Option<bool>,
}

/// A set with the file's values and the built-in defaults merged in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetRules {
    pub enabled: bool,
    pub names: Vec<String>,
    pub paths: Vec<String>,
    pub sentinel: BTreeMap<String, Vec<String>>,
    pub dot_folders: bool,
    pub time_machine: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Exclude {
    pub dev: ExcludeSet,
    pub dependencies: ExcludeSet,
    pub media: ExcludeSet,
    pub hidden: ExcludeSet,
    pub system: ExcludeSet,
    #[serde(rename = "backup-excluded")]
    pub backup_excluded: ExcludeSet,
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
            backup_excluded: ExcludeSet { enabled: Some(false), time_machine: Some(true), ..ExcludeSet::default() },
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
            SetName::BackupExcluded => &self.backup_excluded,
        };
        SetRules {
            enabled: configured.enabled.unwrap_or(built_in.enabled),
            names: configured.names.clone().unwrap_or(built_in.names),
            paths: configured.paths.clone().unwrap_or(built_in.paths),
            sentinel: configured.sentinel.clone().unwrap_or(built_in.sentinel),
            dot_folders: configured.dot_folders.unwrap_or(built_in.dot_folders),
            time_machine: configured.time_machine.unwrap_or(built_in.time_machine),
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

/// Turn a set on or off in `exclude.toml`, keeping the file's comments and the rest of it.
pub fn save_exclude_enabled(config_dir: &Path, set: SetName, enabled: bool) -> Result<(), String> {
    edit_set(config_dir, set, |table| {
        let mut value = toml_edit::Value::from(enabled);
        if let Some(old) = table.get("enabled").and_then(toml_edit::Item::as_value) {
            *value.decor_mut() = old.decor().clone();
        }
        table["enabled"] = toml_edit::Item::Value(value);
    })
}

/// Replace a set's folder names in `exclude.toml`, keeping the rest of the file.
pub fn save_exclude_names(config_dir: &Path, set: SetName, names: &[String]) -> Result<(), String> {
    edit_set(config_dir, set, |table| {
        let array: toml_edit::Array = names.iter().map(String::as_str).collect();
        let mut value = toml_edit::Value::Array(array);
        if let Some(old) = table.get("names").and_then(toml_edit::Item::as_value) {
            *value.decor_mut() = old.decor().clone();
        }
        table["names"] = toml_edit::Item::Value(value);
    })
}

fn edit_set(config_dir: &Path, set: SetName, edit: impl FnOnce(&mut toml_edit::Table)) -> Result<(), String> {
    let path = exclude_path(config_dir);
    crate::config::edit_toml(&path, DEFAULT_EXCLUDE, |doc| {
        let table = doc
            .entry(set.id())
            .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()))
            .as_table_mut()
            .ok_or_else(|| format!("{}: `{}` is not a table", path.display(), set.id()))?;
        edit(table);
        Ok(())
    })
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
    let exclude: Exclude =
        toml::from_str(&text).map_err(|e| crate::config::file_message(&path, crate::config::parse_error(&e, &text)))?;
    exclude.validate().map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(exclude)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> crate::test_support::TempDir {
        let dir = crate::test_support::TempDir::new(&format!("crow-core-exclude-{name}-"));
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
    fn the_time_machine_set_is_off_by_default_and_marks_folders() {
        let rules = Exclude::default().set(SetName::BackupExcluded);
        assert!(!rules.enabled && rules.time_machine && rules.names.is_empty());
        assert_eq!(SetName::BackupExcluded.id(), "backup-excluded");
    }

    #[test]
    fn toggling_a_set_and_editing_its_names_keep_the_rest_of_the_file() {
        let dir = temp_dir("edit");
        load_exclude(&dir).unwrap();
        save_exclude_enabled(&dir, SetName::Dev, false).unwrap();
        save_exclude_enabled(&dir, SetName::BackupExcluded, true).unwrap();
        save_exclude_names(&dir, SetName::Dev, &["node_modules".to_owned(), "out".to_owned()]).unwrap();
        let text = fs::read_to_string(dir.join("exclude.toml")).unwrap();
        assert!(text.contains("# Build and dependency folders developers rarely want"), "comments stay: {text}");
        assert!(text.contains("[dev.sentinel]"), "{text}");
        let exclude = load_exclude(&dir).unwrap();
        assert!(!exclude.set(SetName::Dev).enabled);
        assert_eq!(exclude.set(SetName::Dev).names, ["node_modules", "out"]);
        assert!(exclude.set(SetName::BackupExcluded).enabled);
        assert!(exclude.set(SetName::BackupExcluded).time_machine, "the mark rule stays on");
        assert!(!exclude.set(SetName::Media).enabled, "other sets are untouched");
    }

    #[test]
    fn editing_a_set_the_file_lacks_adds_its_table() {
        let dir = temp_dir("edit-missing");
        fs::write(dir.join("exclude.toml"), "[dev]\nenabled = true\n").unwrap();
        save_exclude_enabled(&dir, SetName::BackupExcluded, true).unwrap();
        let exclude = load_exclude(&dir).unwrap();
        assert!(exclude.set(SetName::BackupExcluded).enabled);
        assert!(exclude.set(SetName::BackupExcluded).time_machine);
    }

    #[test]
    fn disabled_sets_stay_off_until_enabled() {
        let exclude = Exclude::default();
        assert!(exclude.set(SetName::Dev).enabled);
        for name in [SetName::Dependencies, SetName::Media, SetName::Hidden, SetName::System, SetName::BackupExcluded] {
            assert!(!exclude.set(name).enabled, "{} starts off", name.id());
        }
    }
}
