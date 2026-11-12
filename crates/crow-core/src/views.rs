//! View settings in three layers, each overriding the one before:
//!
//! 1. `[defaults]` for every folder;
//! 2. `[[rule]]` path patterns, in file order (later rules win);
//! 3. `[folder."<path>"]` for one folder, which Crow writes when you change that
//!    folder's view or sort.
//!
//! Every resolved value remembers which layer it came from, for "Why this view?".

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use crow_fs::{Sort, SortKey};
use globset::{GlobBuilder, GlobMatcher};
use serde::Deserialize;
use toml_edit::{DocumentMut, Item, Table, value};

use crate::config::{abbreviate_home, edit_toml, expand_home};
use crate::settings::ViewMode;

pub const DEFAULT_VIEWS: &str = r#"# Crow view settings. Changes apply when you next open a folder.
#
# Three layers, each overriding the one before:
#   1. [defaults]                 every folder
#   2. [[rule]]                   folders matching a path pattern (later rules win)
#   3. [folder."~/Some/Folder"]   one folder; Crow writes these when you change
#                                 a folder's view or sort
# View ▸ Why This View? (Cmd-J) shows which layer decided what.
#
# Keys, in any layer:
#   view = "list" | "columns"
#   sort = "name" | "size" | "kind" | "modified" | "added"
#   ascending = true | false     (default: A to Z for names and kinds,
#                                 newest and largest first otherwise)
#   folders_first = true | false
#   columns = ["modified", "size", "kind", "added"]   list view columns after Name

[defaults]
view = "list"
sort = "name"
folders_first = true
columns = ["modified", "size", "kind"]

# Pattern rules. `path` is a glob: "~/Downloads" is that folder only,
# "~/Projects/**" every folder inside it, "**/node_modules" any folder of that name.
#
# [[rule]]
# path = "~/Downloads"
# sort = "added"
#
# [[rule]]
# path = "~/Pictures/**"
# view = "columns"
"#;

/// One layer's settings; unset keys fall through to the layer below.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewLayer {
    pub view: Option<ViewMode>,
    #[serde(default, deserialize_with = "sort_key")]
    pub sort: Option<SortKey>,
    pub ascending: Option<bool>,
    pub folders_first: Option<bool>,
    pub columns: Option<Vec<String>>,
}

// `flatten` can't be combined with `deny_unknown_fields`, so a rule repeats the layer's keys.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFile {
    path: String,
    view: Option<ViewMode>,
    #[serde(default, deserialize_with = "sort_key")]
    sort: Option<SortKey>,
    ascending: Option<bool>,
    folders_first: Option<bool>,
    columns: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ViewsFile {
    #[serde(default)]
    defaults: ViewLayer,
    #[serde(default)]
    rule: Vec<RuleFile>,
    #[serde(default)]
    folder: BTreeMap<String, ViewLayer>,
}

pub fn sort_key_name(key: SortKey) -> &'static str {
    match key {
        SortKey::Name => "name",
        SortKey::Size => "size",
        SortKey::Kind => "kind",
        SortKey::Modified => "modified",
        SortKey::Added => "added",
    }
}

fn parse_sort_key(name: &str) -> Option<SortKey> {
    Some(match name {
        "name" => SortKey::Name,
        "size" => SortKey::Size,
        "kind" => SortKey::Kind,
        "modified" => SortKey::Modified,
        "added" => SortKey::Added,
        _ => return None,
    })
}

fn sort_key<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<SortKey>, D::Error> {
    let name = String::deserialize(d)?;
    parse_sort_key(&name).map(Some).ok_or_else(|| {
        serde::de::Error::custom(format!("unknown sort \"{name}\"; use name, size, kind, modified or added"))
    })
}

/// Finder's starting direction for a sort key: names and kinds A to Z, the rest largest or newest first.
pub fn natural_ascending(key: SortKey) -> bool {
    matches!(key, SortKey::Name | SortKey::Kind)
}

/// Where a resolved value came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// Crow's own default (nothing in `views.toml` set it).
    Builtin,
    Defaults,
    /// The rule with this pattern, as written.
    Rule(String),
    /// This folder's own override.
    Folder,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::Builtin => write!(f, "Crow's default"),
            Source::Defaults => write!(f, "[defaults] in views.toml"),
            Source::Rule(pattern) => write!(f, "rule \"{pattern}\" in views.toml"),
            Source::Folder => write!(f, "this folder's own setting"),
        }
    }
}

/// The view for one folder, each value with its source.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedView {
    pub view: (ViewMode, Source),
    pub sort: (SortKey, Source),
    pub ascending: (bool, Source),
    pub folders_first: (bool, Source),
    pub columns: (Vec<String>, Source),
    /// Every rule that matched, in order (for the explanation, even those whose
    /// values were all overridden).
    pub matched_rules: Vec<String>,
}

impl ResolvedView {
    pub fn sort(&self) -> Sort {
        Sort { key: self.sort.0, ascending: self.ascending.0, folders_first: self.folders_first.0 }
    }
}

struct Rule {
    pattern: String,
    matchers: Vec<GlobMatcher>,
    layer: ViewLayer,
}

/// The parsed `views.toml`.
pub struct Views {
    defaults: ViewLayer,
    rules: Vec<Rule>,
    folders: BTreeMap<PathBuf, ViewLayer>,
}

impl Default for Views {
    fn default() -> Self {
        Views::parse(DEFAULT_VIEWS, Path::new("/")).expect("the documented default parses")
    }
}

impl Views {
    pub fn parse(text: &str, home: &Path) -> Result<Views, String> {
        let file: ViewsFile = toml::from_str(text).map_err(|e| e.message().to_owned())?;
        let rules = file
            .rule
            .into_iter()
            .map(|r| {
                let expanded = expand_home(&r.path, home);
                // "a/**" should also match "a" itself, which the glob alone doesn't.
                let text = expanded.to_string_lossy().into_owned();
                let mut patterns = vec![text.clone()];
                if let Some(base) = text.strip_suffix("/**") {
                    patterns.push(base.to_owned());
                }
                let matchers = patterns
                    .iter()
                    .map(|p| GlobBuilder::new(p).literal_separator(true).build().map(|g| g.compile_matcher()))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("rule \"{}\": {e}", r.path))?;
                let layer = ViewLayer {
                    view: r.view,
                    sort: r.sort,
                    ascending: r.ascending,
                    folders_first: r.folders_first,
                    columns: r.columns,
                };
                Ok(Rule { pattern: r.path, matchers, layer })
            })
            .collect::<Result<_, String>>()?;
        let folders = file.folder.into_iter().map(|(path, layer)| (expand_home(&path, home), layer)).collect();
        Ok(Views { defaults: file.defaults, rules, folders })
    }

    /// Resolve `dir`'s view: built-in defaults, then `[defaults]`, matching rules, the folder's own.
    pub fn resolve(&self, dir: &Path) -> ResolvedView {
        let mut out = ResolvedView {
            view: (ViewMode::List, Source::Builtin),
            sort: (SortKey::Name, Source::Builtin),
            ascending: (true, Source::Builtin),
            folders_first: (true, Source::Builtin),
            columns: (vec!["modified".into(), "size".into(), "kind".into()], Source::Builtin),
            matched_rules: Vec::new(),
        };
        let mut apply = |layer: &ViewLayer, source: Source| {
            if let Some(v) = layer.view {
                out.view = (v, source.clone());
            }
            if let Some(key) = layer.sort {
                out.sort = (key, source.clone());
                // A layer that picks a sort without a direction gets Finder's direction for it.
                out.ascending = (natural_ascending(key), source.clone());
            }
            if let Some(a) = layer.ascending {
                out.ascending = (a, source.clone());
            }
            if let Some(f) = layer.folders_first {
                out.folders_first = (f, source.clone());
            }
            if let Some(c) = &layer.columns {
                out.columns = (c.clone(), source);
            }
        };
        apply(&self.defaults, Source::Defaults);
        let mut matched = Vec::new();
        for rule in self.rules.iter().filter(|r| r.matchers.iter().any(|m| m.is_match(dir))) {
            apply(&rule.layer, Source::Rule(rule.pattern.clone()));
            matched.push(rule.pattern.clone());
        }
        if let Some(layer) = self.folders.get(dir) {
            apply(layer, Source::Folder);
        }
        out.matched_rules = matched;
        out
    }

    pub fn has_folder_override(&self, dir: &Path) -> bool {
        self.folders.contains_key(dir)
    }
}

/// A change to one folder's view, as the user made it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewChange {
    View(ViewMode),
    Sort(Sort),
}

fn mode_name(mode: ViewMode) -> &'static str {
    match mode {
        ViewMode::List => "list",
        ViewMode::Columns => "columns",
    }
}

fn write_change(table: &mut Table, change: ViewChange) {
    match change {
        ViewChange::View(mode) => table["view"] = value(mode_name(mode)),
        ViewChange::Sort(sort) => {
            table["sort"] = value(sort_key_name(sort.key));
            table["ascending"] = value(sort.ascending);
        }
    }
}

fn views_path(config_dir: &Path) -> PathBuf {
    config_dir.join("views.toml")
}

fn folder_tables(doc: &mut DocumentMut) -> Result<&mut Table, String> {
    let folders = doc.entry("folder").or_insert_with(|| {
        let mut t = Table::new();
        t.set_implicit(true);
        Item::Table(t)
    });
    folders.as_table_mut().ok_or_else(|| "views.toml: `folder` must be a table".to_owned())
}

/// Remember `change` for `dir` alone (layer 3).
pub fn save_folder_change(config_dir: &Path, home: &Path, dir: &Path, change: ViewChange) -> Result<(), String> {
    edit_toml(&views_path(config_dir), DEFAULT_VIEWS, |doc| {
        let key = abbreviate_home(dir, home);
        let folders = folder_tables(doc)?;
        let table = folders.entry(&key).or_insert_with(|| Item::Table(Table::new()));
        let table = table.as_table_mut().ok_or_else(|| format!("views.toml: folder \"{key}\" must be a table"))?;
        write_change(table, change);
        Ok(())
    })
}

/// Forget `dir`'s own settings, so it shows the defaults and rules again.
pub fn reset_folder(config_dir: &Path, home: &Path, dir: &Path) -> Result<(), String> {
    edit_toml(&views_path(config_dir), DEFAULT_VIEWS, |doc| {
        let key = abbreviate_home(dir, home);
        if let Some(folders) = doc.get_mut("folder").and_then(Item::as_table_mut) {
            folders.remove(&key);
            // Also an absolute spelling someone may have written by hand.
            folders.remove(&dir.display().to_string());
        }
        Ok(())
    })
}

/// Make `view`'s view and sort the defaults for every folder (layer 1), and drop
/// `dir`'s own override, which now says nothing new.
pub fn save_as_defaults(config_dir: &Path, home: &Path, dir: &Path, view: &ResolvedView) -> Result<(), String> {
    edit_toml(&views_path(config_dir), DEFAULT_VIEWS, |doc| {
        let defaults = doc.entry("defaults").or_insert_with(|| Item::Table(Table::new()));
        let defaults = defaults.as_table_mut().ok_or_else(|| "views.toml: `defaults` must be a table".to_owned())?;
        write_change(defaults, ViewChange::View(view.view.0));
        write_change(defaults, ViewChange::Sort(view.sort()));
        defaults["folders_first"] = value(view.folders_first.0);
        if let Some(folders) = doc.get_mut("folder").and_then(Item::as_table_mut) {
            folders.remove(&abbreviate_home(dir, home));
        }
        Ok(())
    })
}

/// Load `views.toml` from `config_dir`, writing the documented default first if it's missing.
pub fn load_views(config_dir: &Path, home: &Path) -> Result<Views, String> {
    let path = views_path(config_dir);
    if !path.exists() {
        crate::config::write_atomic(&path, DEFAULT_VIEWS).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    Views::parse(&text, home).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/Users/me";

    fn views(extra: &str) -> Views {
        Views::parse(&format!("{DEFAULT_VIEWS}\n{extra}"), Path::new(HOME)).expect("parses")
    }

    #[test]
    fn defaults_apply_everywhere() {
        let v = views("").resolve(Path::new("/tmp"));
        assert_eq!(v.view, (ViewMode::List, Source::Defaults));
        assert_eq!(v.sort, (SortKey::Name, Source::Defaults));
        assert!(v.ascending.0);
        assert_eq!(v.columns.0, ["modified", "size", "kind"]);
        assert!(v.matched_rules.is_empty());
    }

    #[test]
    fn rules_match_folders_and_later_rules_win() {
        let v = views(
            r#"
[[rule]]
path = "~/Downloads"
sort = "added"

[[rule]]
path = "~/Projects/**"
view = "columns"

[[rule]]
path = "**/node_modules"
view = "list"
"#,
        );
        let downloads = v.resolve(Path::new("/Users/me/Downloads"));
        assert_eq!(downloads.sort, (SortKey::Added, Source::Rule("~/Downloads".into())));
        assert!(!downloads.ascending.0, "dates start newest first");
        assert_eq!(v.resolve(Path::new("/Users/me/Downloads/x")).sort.1, Source::Defaults, "no ** = that folder only");

        assert_eq!(v.resolve(Path::new("/Users/me/Projects")).view.0, ViewMode::Columns, "** includes the folder");
        assert_eq!(v.resolve(Path::new("/Users/me/Projects/crow/src")).view.0, ViewMode::Columns);
        let modules = v.resolve(Path::new("/Users/me/Projects/web/node_modules"));
        assert_eq!(modules.view, (ViewMode::List, Source::Rule("**/node_modules".into())));
        assert_eq!(modules.matched_rules, ["~/Projects/**", "**/node_modules"]);
    }

    #[test]
    fn a_folders_own_setting_beats_rules() {
        let v = views(
            r#"
[[rule]]
path = "~/Pictures"
view = "columns"
sort = "modified"

[folder."~/Pictures"]
view = "list"
"#,
        );
        let p = v.resolve(Path::new("/Users/me/Pictures"));
        assert_eq!(p.view, (ViewMode::List, Source::Folder));
        assert_eq!(p.sort, (SortKey::Modified, Source::Rule("~/Pictures".into())), "unset keys fall through");
    }

    #[test]
    fn mistakes_are_errors_with_the_reason() {
        let err = Views::parse("[defaults]\nsort = \"colour\"\n", Path::new(HOME)).err().unwrap();
        assert!(err.contains("colour"), "{err}");
        let err = Views::parse("[defaults]\nsortt = \"name\"\n", Path::new(HOME)).err().unwrap();
        assert!(err.contains("sortt"), "{err}");
    }

    #[test]
    fn saving_changes_writes_the_folder_layer_and_keeps_comments() {
        let dir = tempfile::tempdir().unwrap();
        let home = Path::new(HOME);
        let pictures = Path::new("/Users/me/Pictures");
        let views = load_views(dir.path(), home).unwrap();
        assert!(!views.has_folder_override(pictures));

        save_folder_change(dir.path(), home, pictures, ViewChange::View(ViewMode::Columns)).unwrap();
        let sort = Sort { key: SortKey::Size, ascending: false, folders_first: true };
        save_folder_change(dir.path(), home, pictures, ViewChange::Sort(sort)).unwrap();
        let text = std::fs::read_to_string(dir.path().join("views.toml")).unwrap();
        assert!(text.starts_with("# Crow view settings"), "comments survive");
        assert!(text.contains("[folder.\"~/Pictures\"]"), "{text}");

        let views = load_views(dir.path(), home).unwrap();
        let p = views.resolve(pictures);
        assert_eq!((p.view.0, p.sort()), (ViewMode::Columns, sort));
        assert_eq!(p.view.1, Source::Folder);

        save_as_defaults(dir.path(), home, pictures, &p).unwrap();
        let views = load_views(dir.path(), home).unwrap();
        assert!(!views.has_folder_override(pictures));
        let tmp = views.resolve(Path::new("/tmp"));
        assert_eq!((tmp.view.clone(), tmp.sort()), ((ViewMode::Columns, Source::Defaults), sort));

        save_folder_change(dir.path(), home, pictures, ViewChange::View(ViewMode::List)).unwrap();
        reset_folder(dir.path(), home, pictures).unwrap();
        assert!(!load_views(dir.path(), home).unwrap().has_folder_override(pictures));
    }
}
