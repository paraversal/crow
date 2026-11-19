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
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, Value, value};

use crate::config::{abbreviate_home, edit_toml, expand_home};
use crate::grid::IconSize;
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
#   view = "icons" | "list" | "columns"
#   icon_size = "compact" | "standard" | "large"
#   icon_caption = "off" | "<plugin>.<column>"
#   sort = "name" | "size" | "kind" | "modified" | "added"
#   ascending = true | false     (default: A to Z for names and kinds,
#                                 newest and largest first otherwise)
#   folders_first = true | false
#   columns = ["modified", "size", "kind", "added"]   list view columns after Name
#     Include "git.lines-changed" for Lines when the bundled Git plugin is enabled.
#   preview_providers = ["example.pack.preview"]       plugin previews tried before native
#   icon_source = "native" | "<pack-id>"               icons for this folder; unset
#                                                      follows settings.toml

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
    pub icon_size: Option<IconSize>,
    pub icon_caption: Option<String>,
    #[serde(default, deserialize_with = "sort_key")]
    pub sort: Option<SortKey>,
    pub ascending: Option<bool>,
    pub folders_first: Option<bool>,
    pub columns: Option<Vec<String>>,
    pub preview_providers: Option<Vec<String>>,
    pub icon_source: Option<String>,
}

// `flatten` can't be combined with `deny_unknown_fields`, so a rule repeats the layer's keys.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFile {
    path: String,
    view: Option<ViewMode>,
    icon_size: Option<IconSize>,
    icon_caption: Option<String>,
    #[serde(default, deserialize_with = "sort_key")]
    sort: Option<SortKey>,
    ascending: Option<bool>,
    folders_first: Option<bool>,
    columns: Option<Vec<String>>,
    preview_providers: Option<Vec<String>>,
    icon_source: Option<String>,
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
    pub icon_size: (IconSize, Source),
    /// The plugin column shown under tile names; `None` is Off.
    pub icon_caption: (Option<String>, Source),
    pub sort: (SortKey, Source),
    pub ascending: (bool, Source),
    pub folders_first: (bool, Source),
    pub columns: (Vec<String>, Source),
    pub preview_providers: (Vec<String>, Source),
    /// `None`: no layer chose one, so `settings.toml`'s icon pack applies.
    pub icon_source: Option<(String, Source)>,
    /// Every rule that matched, in order (for the explanation, even those whose
    /// values were all overridden).
    pub matched_rules: Vec<String>,
}

impl ResolvedView {
    pub fn sort(&self) -> Sort {
        Sort { key: self.sort.0, ascending: self.ascending.0, folders_first: self.folders_first.0 }
    }
}

#[derive(Clone)]
struct Rule {
    pattern: String,
    matchers: Vec<GlobMatcher>,
    layer: ViewLayer,
}

/// The parsed `views.toml`.
#[derive(Clone)]
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

fn validate_icon_source(layer: &ViewLayer, where_: &str) -> Result<(), String> {
    let Some(source) = &layer.icon_source else { return Ok(()) };
    crate::icon_pack::validate_source(source).map_err(|error| format!("{where_}: icon_source: {error}"))
}

fn validate_icon_caption(layer: &ViewLayer, where_: &str) -> Result<(), String> {
    let Some(caption) = &layer.icon_caption else { return Ok(()) };
    if caption == "off" {
        return Ok(());
    }
    crate::extensions::ExtensionId::parse(caption)
        .map(|_| ())
        .map_err(|error| format!("{where_}: icon_caption \"{caption}\": {error}"))
}

fn validate_preview_providers(layer: &ViewLayer, where_: &str) -> Result<(), String> {
    let Some(ids) = &layer.preview_providers else { return Ok(()) };
    for id in ids {
        crate::extensions::ExtensionId::parse(id)
            .map_err(|error| format!("{where_}: preview provider \"{id}\": {error}"))?;
    }
    Ok(())
}

impl Views {
    pub fn parse(text: &str, home: &Path) -> Result<Views, String> {
        let file: ViewsFile = toml::from_str(text).map_err(|e| crate::config::parse_error(&e, text))?;
        validate_preview_providers(&file.defaults, "defaults")?;
        validate_icon_source(&file.defaults, "defaults")?;
        validate_icon_caption(&file.defaults, "defaults")?;
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
                    icon_size: r.icon_size,
                    icon_caption: r.icon_caption,
                    sort: r.sort,
                    ascending: r.ascending,
                    folders_first: r.folders_first,
                    columns: r.columns,
                    preview_providers: r.preview_providers,
                    icon_source: r.icon_source,
                };
                validate_preview_providers(&layer, &format!("rule \"{}\"", r.path))?;
                validate_icon_source(&layer, &format!("rule \"{}\"", r.path))?;
                validate_icon_caption(&layer, &format!("rule \"{}\"", r.path))?;
                Ok(Rule { pattern: r.path, matchers, layer })
            })
            .collect::<Result<_, String>>()?;
        for (path, layer) in &file.folder {
            validate_preview_providers(layer, &format!("folder \"{path}\""))?;
            validate_icon_source(layer, &format!("folder \"{path}\""))?;
            validate_icon_caption(layer, &format!("folder \"{path}\""))?;
        }
        let folders = file.folder.into_iter().map(|(path, layer)| (expand_home(&path, home), layer)).collect();
        Ok(Views { defaults: file.defaults, rules, folders })
    }

    /// Resolve `dir`'s view: built-in defaults, then `[defaults]`, matching rules, the folder's own.
    pub fn resolve(&self, dir: &Path) -> ResolvedView {
        let mut out = ResolvedView {
            view: (ViewMode::List, Source::Builtin),
            icon_size: (IconSize::Standard, Source::Builtin),
            icon_caption: (None, Source::Builtin),
            sort: (SortKey::Name, Source::Builtin),
            ascending: (true, Source::Builtin),
            folders_first: (true, Source::Builtin),
            columns: (vec!["modified".into(), "size".into(), "kind".into()], Source::Builtin),
            preview_providers: (Vec::new(), Source::Builtin),
            icon_source: None,
            matched_rules: Vec::new(),
        };
        let mut apply = |layer: &ViewLayer, source: Source| {
            if let Some(v) = layer.view {
                out.view = (v, source.clone());
            }
            if let Some(size) = layer.icon_size {
                out.icon_size = (size, source.clone());
            }
            if let Some(caption) = &layer.icon_caption {
                out.icon_caption = ((caption != "off").then(|| caption.clone()), source.clone());
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
                out.columns = (c.clone(), source.clone());
            }
            if let Some(p) = &layer.preview_providers {
                out.preview_providers = (p.clone(), source.clone());
            }
            if let Some(icon) = &layer.icon_source {
                out.icon_source = Some((icon.clone(), source));
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

    /// The `[defaults]` view, where a window starts when views don't follow folders.
    pub fn default_view(&self) -> ViewMode {
        self.defaults.view.unwrap_or_default()
    }

    /// The raw `[defaults]` layer, for controls that edit only that layer.
    pub fn defaults_layer(&self) -> &ViewLayer {
        &self.defaults
    }

    /// Every path rule in file order; `index` identifies one even when two
    /// rules use the same pattern.
    pub fn rules(&self) -> Vec<RuleInfo> {
        self.rules
            .iter()
            .enumerate()
            .map(|(index, rule)| RuleInfo {
                index,
                pattern: rule.pattern.clone(),
                preview_providers: rule.layer.preview_providers.clone(),
            })
            .collect()
    }

    /// A rule's own preview list: `None` for an unknown index, `Some(None)` when
    /// the rule has no override (it inherits).
    pub fn rule_preview_providers(&self, index: usize) -> Option<Option<Vec<String>>> {
        self.rules.get(index).map(|rule| rule.layer.preview_providers.clone())
    }

    /// The identity a preview edit uses to prove the rule did not move or change.
    pub fn rule_locator(&self, index: usize) -> Option<RuleLocator> {
        let rule = self.rules.get(index)?;
        Some(RuleLocator { index, pattern: rule.pattern.clone(), layer: rule.layer.clone() })
    }

    /// A folder's own raw layer, for an editor that writes only that layer.
    pub fn folder_layer(&self, dir: &Path) -> Option<&ViewLayer> {
        self.folders.get(dir)
    }

    pub fn has_folder_override(&self, dir: &Path) -> bool {
        self.folders.contains_key(dir)
    }
}

/// One path rule as the preview editor lists it. Repeated patterns keep
/// distinct indexes in on-disk order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleInfo {
    pub index: usize,
    pub pattern: String,
    /// The rule's own `preview_providers`; `None` inherits.
    pub preview_providers: Option<Vec<String>>,
}

/// The stable target a preview edit is allowed to change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleLocator {
    pub index: usize,
    pub pattern: String,
    pub layer: ViewLayer,
}

/// A change to one folder's view, as the user made it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewChange {
    View(ViewMode),
    IconSize(IconSize),
    /// `None` writes `"off"`, overriding an inherited source.
    IconCaption(Option<String>),
    Sort(Sort),
    Columns(Vec<String>),
}

/// A change to `[defaults]` only, independent of the folder on screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DefaultsChange {
    View(ViewMode),
    /// Sets only `sort`; a saved `ascending` stays as the user wrote it.
    SortKey(SortKey),
    /// `Some` writes `ascending`; `None` removes it so the natural direction applies.
    Ascending(Option<bool>),
    FoldersFirst(bool),
    PreviewProviders(Vec<String>),
    /// Remove the defaults list so the built-in Native behavior applies.
    InheritPreviewProviders,
}

fn mode_name(mode: ViewMode) -> &'static str {
    match mode {
        ViewMode::List => "list",
        ViewMode::Icons => "icons",
        ViewMode::Columns => "columns",
    }
}

fn write_change(table: &mut Table, change: ViewChange) {
    match change {
        ViewChange::View(mode) => table["view"] = value(mode_name(mode)),
        ViewChange::IconSize(size) => table["icon_size"] = value(size.key()),
        ViewChange::IconCaption(caption) => table["icon_caption"] = value(caption.as_deref().unwrap_or("off")),
        ViewChange::Columns(columns) => {
            let old = table.get("columns").and_then(Item::as_array);
            let mut array = old.cloned().unwrap_or_default();
            array.clear();
            for column in columns {
                if let Some(existing) = old.and_then(|old| old.iter().find(|item| item.as_str() == Some(&column))) {
                    array.push_formatted(existing.clone());
                } else {
                    array.push(column);
                }
            }
            table["columns"] = value(array);
        }
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

/// Change `[defaults]` only, leaving every folder and rule layer untouched.
/// `sort` and `ascending` are separate: choosing a sort key alone keeps a saved
/// direction, and removing `ascending` restores the natural direction.
pub fn save_defaults_change(config_dir: &Path, home: &Path, change: DefaultsChange) -> Result<(), String> {
    edit_toml(&views_path(config_dir), DEFAULT_VIEWS, |doc| {
        Views::parse(&doc.to_string(), home)?;
        let defaults = doc.entry("defaults").or_insert_with(|| Item::Table(Table::new()));
        let table = defaults.as_table_mut().ok_or_else(|| "views.toml: `defaults` must be a table".to_owned())?;
        match change {
            DefaultsChange::View(mode) => write_view_value(table, "view", Value::from(mode_name(mode))),
            DefaultsChange::SortKey(key) => write_view_value(table, "sort", Value::from(sort_key_name(key))),
            DefaultsChange::Ascending(Some(ascending)) => write_view_value(table, "ascending", Value::from(ascending)),
            DefaultsChange::Ascending(None) => {
                table.remove("ascending");
            }
            DefaultsChange::FoldersFirst(folders_first) => {
                write_view_value(table, "folders_first", Value::from(folders_first))
            }
            DefaultsChange::PreviewProviders(providers) => write_preview_list(table, &providers),
            DefaultsChange::InheritPreviewProviders => {
                table.remove("preview_providers");
            }
        }
        Ok(())
    })
}

/// Replace one layer's `preview_providers`, keeping the formatting of items
/// that survived (their comments are part of the item).
fn write_preview_list(table: &mut Table, providers: &[String]) {
    let old = table.get("preview_providers").and_then(Item::as_array);
    let mut array = old.cloned().unwrap_or_default();
    array.clear();
    for provider in providers {
        if let Some(existing) = old.and_then(|old| old.iter().find(|item| item.as_str() == Some(provider))) {
            array.push_formatted(existing.clone());
        } else {
            array.push(provider);
        }
    }
    write_view_value(table, "preview_providers", Value::from(array));
}

fn write_view_value(table: &mut Table, key: &str, mut value: Value) {
    if let Some(old) = table.get(key).and_then(Item::as_value) {
        *value.decor_mut() = old.decor().clone();
    }
    table[key] = Item::Value(value);
}

/// The `[folder]` keys whose expanded path is exactly `dir`. More than one is
/// an ambiguity an editor must report instead of picking one.
fn existing_folder_keys(doc: &DocumentMut, home: &Path, dir: &Path) -> Vec<String> {
    let Some(folders) = doc.get("folder").and_then(Item::as_table) else { return Vec::new() };
    folders
        .iter()
        .filter(|(_, item)| item.is_table())
        .filter(|(key, _)| expand_home(key, home) == dir)
        .map(|(key, _)| key.to_owned())
        .collect()
}

/// The spelling for a folder key. Unlike `abbreviate_home`, a path that is not
/// UTF-8 is an error: TOML cannot round-trip the raw bytes.
fn checked_folder_key(dir: &Path, home: &Path) -> Result<String, String> {
    let key = match dir.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Ok(rest) => format!(
            "~/{}",
            rest.to_str()
                .ok_or_else(|| format!("{} is not valid UTF-8; views.toml cannot store this folder", dir.display()))?
        ),
        Err(_) => dir
            .to_str()
            .ok_or_else(|| format!("{} is not valid UTF-8; views.toml cannot store this folder", dir.display()))?
            .to_owned(),
    };
    Ok(key)
}

/// Set or clear one folder's `preview_providers`. `None` removes the key so the
/// folder inherits again; `Some([])` is the explicit Native-only list. An
/// existing `~/…` or absolute spelling is reused, and two equivalent spellings
/// are reported instead of writing a third.
pub fn save_folder_preview_providers(
    config_dir: &Path,
    home: &Path,
    dir: &Path,
    providers: Option<Vec<String>>,
) -> Result<(), String> {
    edit_toml(&views_path(config_dir), DEFAULT_VIEWS, |doc| {
        Views::parse(&doc.to_string(), home)?;
        let existing = existing_folder_keys(doc, home, dir);
        if existing.len() > 1 {
            return Err(format!(
                "views.toml: folder \"{}\" has {} equivalent keys ({}); resolve them before editing preview providers",
                dir.display(),
                existing.len(),
                existing.join(", ")
            ));
        }
        match providers {
            None => {
                if let Some(key) = existing.first()
                    && let Some(folders) = doc.get_mut("folder").and_then(Item::as_table_mut)
                    && let Some(table) = folders.get_mut(key).and_then(Item::as_table_mut)
                {
                    table.remove("preview_providers");
                    if table.iter().next().is_none() {
                        folders.remove(key);
                    }
                }
                Ok(())
            }
            Some(providers) => {
                let key = match existing.first() {
                    Some(key) => key.clone(),
                    None => checked_folder_key(dir, home)?,
                };
                let table = folder_tables(doc)?.entry(&key).or_insert_with(|| Item::Table(Table::new()));
                let table =
                    table.as_table_mut().ok_or_else(|| format!("views.toml: folder {key:?} must be a table"))?;
                write_preview_list(table, &providers);
                Ok(())
            }
        }
    })
}

/// Set or clear one rule's `preview_providers`, proving first that the rule at
/// the locator's index is still the one the editor drew. Clearing leaves every
/// other field of a mixed-purpose rule intact.
pub fn save_rule_preview_providers(
    config_dir: &Path,
    home: &Path,
    locator: &RuleLocator,
    providers: Option<Vec<String>>,
) -> Result<(), String> {
    edit_toml(&views_path(config_dir), DEFAULT_VIEWS, |doc| {
        let current = Views::parse(&doc.to_string(), home)?;
        if current.rule_locator(locator.index).as_ref() != Some(locator) {
            return Err("views.toml: the selected rule changed; choose it again".into());
        }
        let rules = doc
            .get_mut("rule")
            .and_then(Item::as_array_of_tables_mut)
            .ok_or_else(|| "views.toml: `rule` must be an array of tables".to_owned())?;
        let table = rules
            .get_mut(locator.index)
            .ok_or_else(|| "views.toml: the selected rule is gone; choose it again".to_owned())?;
        match providers {
            Some(providers) => write_preview_list(table, &providers),
            None => {
                table.remove("preview_providers");
            }
        }
        Ok(())
    })
}

/// Validate a path pattern the same way the resolver does, including the
/// `/**` form that also matches the base folder.
fn validate_rule_pattern(pattern: &str, home: &Path) -> Result<(), String> {
    if pattern.trim().is_empty() {
        return Err("a path rule needs a pattern".into());
    }
    let expanded = expand_home(pattern, home);
    let text = expanded.to_string_lossy().into_owned();
    let mut patterns = vec![text.clone()];
    if let Some(base) = text.strip_suffix("/**") {
        patterns.push(base.to_owned());
    }
    for pattern in patterns {
        GlobBuilder::new(&pattern)
            .literal_separator(true)
            .build()
            .map_err(|error| format!("rule \"{pattern}\": {error}"))?;
    }
    Ok(())
}

/// Append a preview-only rule after the existing rules. Later rules win in the
/// resolver, so the new one overrides earlier matches for the folders it covers.
pub fn add_preview_rule(
    config_dir: &Path,
    home: &Path,
    index: usize,
    pattern: &str,
    providers: Vec<String>,
) -> Result<(), String> {
    validate_rule_pattern(pattern, home)?;
    edit_toml(&views_path(config_dir), DEFAULT_VIEWS, |doc| {
        let current = Views::parse(&doc.to_string(), home)?;
        if current.rules.len() != index {
            return Err("views.toml: the rule count changed; check the insertion position and try again".into());
        }
        let rules = doc
            .entry("rule")
            .or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()))
            .as_array_of_tables_mut()
            .ok_or_else(|| "views.toml: `rule` must be an array of tables".to_owned())?;
        let mut table = Table::new();
        table["path"] = value(pattern);
        write_preview_list(&mut table, &providers);
        rules.push(table);
        Ok(())
    })
}

/// Save the chosen columns to this folder and `[defaults]` in the same write.
pub fn save_default_columns(config_dir: &Path, home: &Path, dir: &Path, columns: Vec<String>) -> Result<(), String> {
    edit_toml(&views_path(config_dir), DEFAULT_VIEWS, |doc| {
        let defaults = doc.entry("defaults").or_insert_with(|| Item::Table(Table::new()));
        let defaults = defaults.as_table_mut().ok_or_else(|| "views.toml: `defaults` must be a table".to_owned())?;
        write_change(defaults, ViewChange::Columns(columns.clone()));
        let key = abbreviate_home(dir, home);
        let table = folder_tables(doc)?.entry(&key).or_insert_with(|| Item::Table(Table::new()));
        let table = table.as_table_mut().ok_or_else(|| format!("views.toml: folder {key:?} must be a table"))?;
        write_change(table, ViewChange::Columns(columns));
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
    Views::parse(&text, home).map_err(|e| crate::config::file_message(&path, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/Users/me";

    fn views(extra: &str) -> Views {
        Views::parse(&format!("{DEFAULT_VIEWS}\n{extra}"), Path::new(HOME)).expect("parses")
    }

    #[test]
    fn preview_edit_rejects_swapped_rules_that_only_differ_in_providers() {
        let dir = tempfile::tempdir().unwrap();
        let path = views_path(dir.path());
        let home = Path::new(HOME);
        let first = "[[rule]]\npath = \"~/Pictures\"\npreview_providers = [\"example.one.preview\"]\n";
        let second = "[[rule]]\npath = \"~/Pictures\"\npreview_providers = [\"example.two.preview\"]\n";
        let original = format!("{first}\n{second}");
        let locator = Views::parse(&original, home).unwrap().rule_locator(0).unwrap();

        let reordered = format!("{second}\n{first}");
        std::fs::write(&path, &reordered).unwrap();
        assert!(save_rule_preview_providers(dir.path(), home, &locator, None).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), reordered);
    }

    #[test]
    fn settings_writers_leave_semantically_invalid_views_intact() {
        let dir = tempfile::tempdir().unwrap();
        let path = views_path(dir.path());
        let home = Path::new(HOME);
        let malformed = "[defaults]\nsort = \"unknown\"\n";
        std::fs::write(&path, malformed).unwrap();

        assert!(save_defaults_change(dir.path(), home, DefaultsChange::SortKey(SortKey::Name)).is_err());
        assert!(save_folder_preview_providers(dir.path(), home, home, Some(vec![])).is_err());
        assert!(add_preview_rule(dir.path(), home, 0, "~/Pictures", vec![]).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), malformed);
    }

    #[test]
    fn settings_view_edits_keep_trailing_comments() {
        let dir = tempfile::tempdir().unwrap();
        let path = views_path(dir.path());
        let home = Path::new(HOME);
        std::fs::write(
            &path,
            "[defaults]\nsort = \"name\" # sort comment\npreview_providers = [\"example.one.preview\"] # list comment\n",
        )
        .unwrap();
        save_defaults_change(dir.path(), home, DefaultsChange::SortKey(SortKey::Size)).unwrap();
        save_defaults_change(dir.path(), home, DefaultsChange::PreviewProviders(vec![])).unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.contains("sort = \"size\" # sort comment"));
        assert!(text.contains("preview_providers = [] # list comment"));
    }

    #[test]
    fn icon_size_and_caption_resolve_and_write_per_folder() {
        let dir = tempfile::tempdir().unwrap();
        let home = Path::new(HOME);
        let folder = home.join("Pictures");
        let text = format!(
            "[folder.\"{}\"]\nview = \"icons\"\nicon_size = \"compact\"\nicon_caption = \"git.lines-changed\"\n",
            folder.display()
        );
        std::fs::write(views_path(dir.path()), &text).unwrap();
        let views = Views::parse(&text, home).unwrap();
        let resolved = views.resolve(&folder);
        assert_eq!(resolved.view.0, ViewMode::Icons);
        assert_eq!(resolved.icon_size, (IconSize::Compact, Source::Folder));
        assert_eq!(resolved.icon_caption, (Some("git.lines-changed".into()), Source::Folder));

        save_folder_change(dir.path(), home, &folder, ViewChange::IconSize(IconSize::Large)).unwrap();
        save_folder_change(dir.path(), home, &folder, ViewChange::IconCaption(None)).unwrap();
        let saved = std::fs::read_to_string(views_path(dir.path())).unwrap();
        assert!(saved.contains("icon_size = \"large\""), "{saved}");
        assert!(saved.contains("icon_caption = \"off\""), "Off is written explicitly: {saved}");
        let resolved = Views::parse(&saved, home).unwrap().resolve(&folder);
        assert_eq!(resolved.icon_caption, (None, Source::Folder), "an explicit Off beats an inherited source");
        assert!(Views::parse("[defaults]\nicon_caption = \"not an id\"\n", home).is_err());
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
    fn preview_providers_inherit_and_an_empty_folder_list_selects_native_only() {
        let v = Views::parse(
            r#"
[defaults]
preview_providers = ["example.pack.preview", "example.other.preview"]

[[rule]]
path = "~/Pictures/**"
preview_providers = ["example.pictures.preview"]

[folder."~/Pictures/Empty"]
preview_providers = []
"#,
            Path::new(HOME),
        )
        .unwrap();
        assert_eq!(
            v.resolve(Path::new("/Users/me/Documents")).preview_providers.0,
            ["example.pack.preview", "example.other.preview"]
        );
        assert_eq!(v.resolve(Path::new("/Users/me/Pictures/One")).preview_providers.0, ["example.pictures.preview"]);
        assert!(v.resolve(Path::new("/Users/me/Pictures/Empty")).preview_providers.0.is_empty());

        assert!(
            Views::parse("[defaults]\npreview_providers = [\"not-qualified\"]", Path::new(HOME)).is_err(),
            "a provider ID without plugin.local is a config error, not a runtime skip"
        );
    }

    #[test]
    fn icon_source_inherits_and_an_explicit_native_wins_at_its_layer() {
        let v = Views::parse(
            r#"
[defaults]
icon_source = "sample"

[[rule]]
path = "~/Pictures/**"
icon_source = "native"

[folder."~/Pictures/Plain"]
"#,
            Path::new(HOME),
        )
        .unwrap();
        let inherited = v.resolve(Path::new("/Users/me/Documents"));
        assert_eq!(
            inherited.icon_source,
            Some(("sample".to_owned(), Source::Defaults)),
            "an unset layer follows the one below; the setting is the final fallback"
        );
        let native = v.resolve(Path::new("/Users/me/Pictures/One"));
        assert_eq!(native.icon_source, Some(("native".to_owned(), Source::Rule("~/Pictures/**".into()))));
        assert_eq!(
            v.resolve(Path::new("/Users/me/Pictures/Plain")).icon_source,
            Some(("native".to_owned(), Source::Rule("~/Pictures/**".into()))),
            "a folder layer without icon_source does not reset it"
        );
        assert!(
            Views::parse("[defaults]\nicon_source = \"My Pack\"", Path::new(HOME)).is_err(),
            "a malformed pack id is a config error"
        );
        assert!(Views::parse("[defaults]\nicon_source = \"example.pack.theme\"", Path::new(HOME)).is_ok());
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
        assert!(err.contains("line 2"), "the parser's span is a line: {err}");
        let err = Views::parse("[defaults]\nsortt = \"name\"\n", Path::new(HOME)).err().unwrap();
        assert!(err.contains("sortt"), "{err}");
    }

    #[test]
    fn defaults_changes_touch_only_defaults_and_keep_a_saved_direction() {
        let dir = tempfile::tempdir().unwrap();
        let home = Path::new(HOME);
        std::fs::write(
            dir.path().join("views.toml"),
            r#"# mine
[defaults]
view = "list"
sort = "name"
folders_first = true # group
preview_providers = ["example.pack.preview"] # first

[[rule]]
path = "~/Pictures"
view = "columns"

[folder."~/Pictures"]
sort = "size"
"#,
        )
        .unwrap();

        save_defaults_change(dir.path(), home, DefaultsChange::SortKey(SortKey::Modified)).unwrap();
        let views = load_views(dir.path(), home).unwrap();
        let defaults = views.defaults_layer();
        assert_eq!(defaults.sort, Some(SortKey::Modified));
        assert_eq!(defaults.ascending, None, "a sort key alone does not invent a direction");
        assert_eq!(views.resolve(Path::new("/tmp")).ascending, (false, Source::Defaults), "modified is newest first");
        let text = std::fs::read_to_string(dir.path().join("views.toml")).unwrap();
        assert!(text.starts_with("# mine\n"), "{text}");
        assert!(text.contains("folders_first = true # group"), "{text}");
        assert!(text.contains("# first"), "list item comments survive: {text}");
        assert!(text.contains("view = \"columns\""), "a rule is untouched: {text}");
        assert!(text.contains("[folder.\"~/Pictures\"]\nsort = \"size\""), "a folder is untouched: {text}");

        save_defaults_change(dir.path(), home, DefaultsChange::Ascending(Some(true))).unwrap();
        assert_eq!(load_views(dir.path(), home).unwrap().defaults_layer().ascending, Some(true));
        save_defaults_change(dir.path(), home, DefaultsChange::Ascending(None)).unwrap();
        assert_eq!(load_views(dir.path(), home).unwrap().defaults_layer().ascending, None, "natural is absence");
        let text = std::fs::read_to_string(dir.path().join("views.toml")).unwrap();
        assert!(!text.contains("ascending"), "{text}");

        save_defaults_change(dir.path(), home, DefaultsChange::PreviewProviders(vec!["example.other.preview".into()]))
            .unwrap();
        let views = load_views(dir.path(), home).unwrap();
        assert_eq!(views.defaults_layer().preview_providers, Some(vec!["example.other.preview".into()]));
        assert_eq!(views.resolve(Path::new("/tmp")).preview_providers.1, Source::Defaults);
        save_defaults_change(dir.path(), home, DefaultsChange::PreviewProviders(Vec::new())).unwrap();
        assert_eq!(
            load_views(dir.path(), home).unwrap().resolve(Path::new("/tmp")).preview_providers.0,
            Vec::<String>::new(),
            "an explicit empty list is Native only, not inheritance"
        );
    }

    #[test]
    fn preview_writes_target_one_layer_without_duplicating_keys() {
        let dir = tempfile::tempdir().unwrap();
        let home = Path::new(HOME);
        let pictures = Path::new("/Users/me/Pictures");
        std::fs::write(
            dir.path().join("views.toml"),
            r#"# mine
[defaults]
preview_providers = ["example.pack.first"] # default list

[[rule]]
path = "~/Pictures"
view = "columns"

[[rule]]
path = "~/Pictures"
sort = "size"
preview_providers = ["example.pack.rule"]

[folder."~/Pictures"]
view = "list" # keep me
"#,
        )
        .unwrap();

        save_folder_preview_providers(dir.path(), home, pictures, Some(vec!["example.pack.folder".into()])).unwrap();
        let text = std::fs::read_to_string(dir.path().join("views.toml")).unwrap();
        assert!(text.starts_with("# mine\n"), "{text}");
        assert!(text.contains("view = \"list\" # keep me"), "the folder's other fields and comments stay: {text}");
        assert_eq!(text.matches("[folder.\"~/Pictures\"]").count(), 1, "no second spelling: {text}");
        let views = load_views(dir.path(), home).unwrap();
        assert_eq!(views.folder_layer(pictures).unwrap().preview_providers, Some(vec!["example.pack.folder".into()]));
        assert_eq!(views.resolve(pictures).preview_providers.1, Source::Folder);

        save_folder_preview_providers(dir.path(), home, pictures, None).unwrap();
        let views = load_views(dir.path(), home).unwrap();
        assert_eq!(views.folder_layer(pictures).unwrap().preview_providers, None);
        assert_eq!(views.folder_layer(pictures).unwrap().view, Some(ViewMode::List), "the mixed table survives");
        assert_eq!(
            views.resolve(pictures).preview_providers.0,
            ["example.pack.rule"],
            "clearing falls back to the later matching rule"
        );

        // Repeated rules are distinct targets in file order.
        let rule_one = views.rule_locator(1).expect("second rule");
        save_rule_preview_providers(dir.path(), home, &rule_one, None).unwrap();
        let views = load_views(dir.path(), home).unwrap();
        assert_eq!(views.rule_preview_providers(0), Some(None), "the first rule was never touched");
        assert_eq!(views.rule_preview_providers(1), Some(None));
        assert_eq!(views.rules()[1].pattern, "~/Pictures", "the rule itself stays");

        // An external reorder changes the target at that index: refuse, keep the file.
        let stale = views.rule_locator(1).unwrap();
        let before = std::fs::read_to_string(dir.path().join("views.toml")).unwrap();
        std::fs::write(
            dir.path().join("views.toml"),
            r#"[[rule]]
path = "~/Other"
preview_providers = ["example.pack.other"]

[[rule]]
path = "~/Pictures"
view = "columns"
"#,
        )
        .unwrap();
        let after_external = std::fs::read_to_string(dir.path().join("views.toml")).unwrap();
        let error = save_rule_preview_providers(dir.path(), home, &stale, Some(vec!["example.pack.rule".into()]))
            .expect_err("a moved rule is not the drawn rule");
        assert!(error.contains("changed"), "{error}");
        assert_eq!(std::fs::read_to_string(dir.path().join("views.toml")).unwrap(), after_external);
        assert_ne!(before, after_external, "the test did change the file outside");

        // A new preview-only rule is appended after the existing ones and wins for matching folders.
        add_preview_rule(dir.path(), home, 2, "~/Other/**", vec!["example.pack.new".into()]).unwrap();
        let views = load_views(dir.path(), home).unwrap();
        assert_eq!(views.rules().len(), 3);
        assert_eq!(views.rules()[2].pattern, "~/Other/**");
        assert_eq!(
            views.resolve(Path::new("/Users/me/Other/Inside")).preview_providers.0,
            ["example.pack.new"],
            "later rules win"
        );
        assert_eq!(
            views.resolve(Path::new("/Users/me/Other")).preview_providers.0,
            ["example.pack.new"],
            "/** also matches the base folder"
        );
        assert!(
            add_preview_rule(dir.path(), home, 0, "~/[", vec![]).is_err(),
            "the resolver's glob validation applies"
        );
    }

    #[test]
    fn ambiguous_folder_spellings_are_reported_and_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let home = Path::new(HOME);
        let pictures = Path::new("/Users/me/Pictures");
        let text = r#"[folder."~/Pictures"]
view = "list"

[folder."/Users/me/Pictures"]
sort = "size"
"#;
        std::fs::write(dir.path().join("views.toml"), text).unwrap();
        let error = save_folder_preview_providers(dir.path(), home, pictures, Some(vec!["example.pack.a".into()]))
            .expect_err("two spellings");
        assert!(error.contains("equivalent keys"), "{error}");
        assert_eq!(std::fs::read_to_string(dir.path().join("views.toml")).unwrap(), text);
    }

    #[cfg(unix)]
    #[test]
    fn a_non_utf8_folder_is_refused_before_writing() {
        use std::os::unix::ffi::OsStringExt;
        let dir = tempfile::tempdir().unwrap();
        let home = Path::new(HOME);
        let bad = PathBuf::from(std::ffi::OsString::from_vec(b"/Users/me/Bad-\xff".to_vec()));
        let error = save_folder_preview_providers(dir.path(), home, &bad, Some(vec![]))
            .expect_err("TOML cannot round-trip the bytes");
        assert!(error.contains("not valid UTF-8"), "{error}");
        assert!(!dir.path().join("views.toml").exists(), "nothing was written");
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
