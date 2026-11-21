//! The sidebar's configuration, `sidebar.toml`: groups of favorites, and whether
//! Recent and Locations are shown. Crow edits the file in place when you reorder,
//! add or remove favorites, keeping your comments.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use toml_edit::{Array, DocumentMut, InlineTable, Item, Value};

use crate::config::{abbreviate_home, edit_toml, expand_home};

pub const DEFAULT_SIDEBAR: &str = r##"# Crow sidebar. Crow updates this file when you drag favorites around or add
# and remove them; your comments stay.
#
# Each [[group]] is a section of favorites. A group takes `collapsed = true` to
# hide its items under the title (Crow writes it when you collapse the group).
# Items take:
#   path   the folder ("~" is your home folder)
#   name   label, if not the folder's own name
#   icon   a short text or emoji shown instead of the folder icon
#   color  a theme role ("accent", "error", …), a color name (red, orange,
#          yellow, green, cyan, blue, purple, pink) or "#rrggbb"
#   key    one letter: with the Vim layer, 'x jumps to the favorite whose key is x
#          (m x sets it)

[[group]]
name = "Favorites"
items = [
  { path = "~" },
  { path = "~/Desktop" },
  { path = "~/Documents" },
  { path = "~/Downloads" },
  { path = "/Applications" },
]

[recent]
show = true
# How many recently used folders to list. Crow also reads zoxide's list when installed.
count = 6

[locations]
show = true

# Finder tags shown even when no indexed item uses them (Crow discovers used
# tags from Spotlight; Finder's unused list is private). Case-sensitive.
# pinned_tags = ["Work", "review"]

# Optional section layout. `tags`, `saved-queries`, `recent` and `locations` are
# built-ins; a plugin section uses its exact package ID. Unlisted sections follow
# in the usual order.
# [sections]
# order = ["tags", "saved-queries", "recent", "example.p1b-fixture.folders", "locations"]
#
# [section."example.p1b-fixture.folders"]
# show = true
# collapsed = false

# Project roots. Crow scans these folders for Git repositories; empty by default.
# The native section is "crow.projects" in [sections]/[section] like any other.
[projects]
roots = []
"##;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FavoriteItem {
    pub path: String,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub key: Option<char>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub name: String,
    #[serde(default)]
    pub items: Vec<FavoriteItem>,
    /// Hides the group's favorites under its title.
    #[serde(default)]
    pub collapsed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct RecentConfig {
    pub show: bool,
    pub count: usize,
}

impl Default for RecentConfig {
    fn default() -> Self {
        Self { show: true, count: 6 }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct LocationsConfig {
    pub show: bool,
}

impl Default for LocationsConfig {
    fn default() -> Self {
        Self { show: true }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SectionConfig {
    pub show: Option<bool>,
    pub collapsed: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SectionsConfig {
    /// Section IDs in display order. Built-ins are `recent` and `locations`;
    /// plugin sections use their fully qualified ExtensionId.
    pub order: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct ProjectsConfig {
    /// Folders to scan for Git repositories. Empty by default.
    pub roots: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct SidebarConfig {
    pub group: Vec<Group>,
    pub recent: RecentConfig,
    pub locations: LocationsConfig,
    /// Finder tags shown even when Spotlight has not discovered a use for them.
    pub pinned_tags: Vec<String>,
    pub sections: SectionsConfig,
    pub section: std::collections::BTreeMap<String, SectionConfig>,
    pub projects: ProjectsConfig,
}

/// A sidebar section independent of the native favorites/Recent/Locations
/// adapters. Plugin rows carry an optional real folder destination; display-only
/// rows have none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub id: String,
    pub title: String,
    pub collapsed: bool,
    pub rows: Vec<SectionRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionRow {
    pub id: String,
    pub text: String,
    pub meta: Option<String>,
    pub badge: Option<String>,
    pub destination: SectionDestination,
}

/// Where a section row goes. P1B has one typed navigable destination: a real
/// folder admitted by the host, with the no-follow identity captured off-thread.
/// P4 adds typed Saved Query and Tag destinations here rather than encoding
/// virtual IDs as paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SectionDestination {
    Folder { path: PathBuf, identity: (u64, u64) },
    SavedQuery { id: crate::saved_queries::SavedQueryId },
    Tag { id: crate::saved_queries::TagId },
    DisplayOnly,
}

impl SectionRow {
    pub fn folder_path(&self) -> Option<&Path> {
        match &self.destination {
            SectionDestination::Folder { path, .. } => Some(path),
            SectionDestination::SavedQuery { .. }
            | SectionDestination::Tag { .. }
            | SectionDestination::DisplayOnly => None,
        }
    }

    pub fn folder_identity(&self) -> Option<(u64, u64)> {
        match &self.destination {
            SectionDestination::Folder { identity, .. } => Some(*identity),
            SectionDestination::SavedQuery { .. }
            | SectionDestination::Tag { .. }
            | SectionDestination::DisplayOnly => None,
        }
    }

    pub fn saved_query_id(&self) -> Option<&crate::saved_queries::SavedQueryId> {
        match &self.destination {
            SectionDestination::SavedQuery { id } => Some(id),
            SectionDestination::Folder { .. } | SectionDestination::Tag { .. } | SectionDestination::DisplayOnly => {
                None
            }
        }
    }

    pub fn tag_id(&self) -> Option<&crate::saved_queries::TagId> {
        match &self.destination {
            SectionDestination::Tag { id } => Some(id),
            SectionDestination::Folder { .. }
            | SectionDestination::SavedQuery { .. }
            | SectionDestination::DisplayOnly => None,
        }
    }

    pub fn is_navigable(&self) -> bool {
        !matches!(self.destination, SectionDestination::DisplayOnly)
    }
}

/// A favorite ready to show: its path resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Favorite {
    pub path: PathBuf,
    /// The configured label; `None` shows Finder's name for the folder.
    pub name: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub key: Option<char>,
}

impl SidebarConfig {
    pub fn parse(text: &str) -> Result<SidebarConfig, String> {
        let mut config: SidebarConfig = toml::from_str(text).map_err(|e| crate::config::parse_error(&e, text))?;
        config.pinned_tags =
            crate::tags::normalize_tags(&config.pinned_tags).map_err(|error| format!("sidebar.toml: {error}"))?;
        for id in config.sections.order.iter().chain(config.section.keys()) {
            if !valid_section_id(id) {
                return Err(format!("sidebar.toml: \"{id}\" is not a section id"));
            }
        }
        Ok(config)
    }

    pub fn section(&self, id: &str) -> SectionConfig {
        self.section.get(id).cloned().unwrap_or_default()
    }

    pub fn section_shown(&self, id: &str) -> bool {
        self.section(id).show.unwrap_or(true)
    }

    pub fn group_collapsed(&self, group: usize) -> bool {
        self.group.get(group).is_some_and(|g| g.collapsed)
    }

    pub fn section_collapsed(&self, id: &str) -> bool {
        self.section(id).collapsed.unwrap_or(false)
    }

    /// Configured project roots with `~` expanded, duplicates removed in order.
    pub fn project_roots(&self, home: &Path) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        for root in &self.projects.roots {
            if root.trim().is_empty() {
                continue;
            }
            let path = expand_home(root, home);
            if !out.contains(&path) {
                out.push(path);
            }
        }
        out
    }

    /// Each group's favorites with `~` expanded.
    pub fn favorites(&self, home: &Path) -> Vec<(String, Vec<Favorite>)> {
        self.group
            .iter()
            .map(|g| {
                let items = g
                    .items
                    .iter()
                    .map(|i| Favorite {
                        path: expand_home(&i.path, home),
                        name: i.name.clone(),
                        icon: i.icon.clone(),
                        color: i.color.clone(),
                        key: i.key,
                    })
                    .collect();
                (g.name.clone(), items)
            })
            .collect()
    }

    /// The favorite marked with `key` (Vim's `'x`).
    pub fn find_key(&self, key: char, home: &Path) -> Option<PathBuf> {
        self.group.iter().flat_map(|g| &g.items).find(|i| i.key == Some(key)).map(|i| expand_home(&i.path, home))
    }
}

fn valid_section_id(id: &str) -> bool {
    if id == "recent" || id == "locations" || id == "saved-queries" || id == "tags" {
        return true;
    }
    let Some((plugin, local)) = id.rsplit_once('.') else { return false };
    !plugin.is_empty()
        && !local.is_empty()
        && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-' || byte == b'_')
        && !id.starts_with('.')
        && !id.ends_with('.')
}

/// Where a favorite is: group index, item index.
pub type Slot = (usize, usize);

/// A sidebar row as the twin comparison sees it: what it shows, where it is, and
/// its ancestors' display names nearest first (ending with the volume name).
pub struct TwinInput<'a> {
    pub label: &'a str,
    pub path: &'a Path,
    pub ancestors: &'a [String],
}

/// For each row, the distance of the ancestor that tells it apart from the other
/// rows with the same shown label. `None` when it is not a twin; `Some(usize::MAX)`
/// when no ancestor separates it (the caller falls back to the parent's path).
///
/// Labels compare case-insensitively. The same path twice is one place: both copies
/// get the same hint, and a path is never a twin of itself.
pub fn twin_hints(rows: &[TwinInput]) -> Vec<Option<usize>> {
    let mut hints = vec![None; rows.len()];
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let label = row.label.to_lowercase();
        match groups.iter_mut().find(|(name, _)| *name == label) {
            Some((_, group)) => group.push(i),
            None => groups.push((label, vec![i])),
        }
    }
    for (_, group) in groups {
        let mut paths: Vec<&Path> = Vec::new();
        let mut first_row: Vec<usize> = Vec::new();
        let mut unique_of: Vec<usize> = Vec::with_capacity(group.len());
        for &row in &group {
            match paths.iter().position(|path| *path == rows[row].path) {
                Some(unique) => unique_of.push(unique),
                None => {
                    unique_of.push(paths.len());
                    paths.push(rows[row].path);
                    first_row.push(row);
                }
            }
        }
        if paths.len() < 2 {
            continue;
        }
        let depth = first_row.iter().map(|&row| rows[row].ancestors.len()).max().unwrap_or(0);
        let mut resolved: Vec<Option<usize>> = vec![None; paths.len()];
        let mut left: Vec<usize> = (0..paths.len()).collect();
        for distance in 1..=depth {
            let names: Vec<Option<String>> = left
                .iter()
                .map(|&unique| {
                    let ancestors = rows[first_row[unique]].ancestors;
                    ancestors.get(distance).or_else(|| ancestors.last()).map(|name| name.to_lowercase())
                })
                .collect();
            let mut still = Vec::new();
            for (n, &unique) in left.iter().enumerate() {
                let Some(name) = &names[n] else {
                    still.push(unique);
                    continue;
                };
                if names.iter().filter(|other| other.as_deref() == Some(name.as_str())).count() == 1 {
                    resolved[unique] = Some(distance);
                } else {
                    still.push(unique);
                }
            }
            left = still;
            if left.is_empty() {
                break;
            }
        }
        for unique in left {
            resolved[unique] = Some(usize::MAX);
        }
        for (&row, &unique) in group.iter().zip(&unique_of) {
            hints[row] = resolved[unique];
        }
    }
    hints
}

/// One change to the favorites, as the sidebar makes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidebarEdit {
    /// Move the item at `from` so it sits at `to` (indices in the list as it is before the move).
    Move {
        from: Slot,
        to: Slot,
    },
    /// Add `path` at `at` (an index one past the end appends). Already a favorite in that group: nothing.
    Add {
        path: PathBuf,
        at: Slot,
    },
    /// Add `paths` at `at` in order, in one write. Paths already in that group are skipped.
    AddMany {
        paths: Vec<PathBuf>,
        at: Slot,
    },
    Remove(Slot),
    /// Give `path` the mark `key`, adding it to the first group if it isn't a favorite yet.
    /// The key is taken from whichever favorite had it before.
    SetKey {
        path: PathBuf,
        key: char,
    },
    /// Write only one section's collapse flag, keeping every other line intact.
    SetSectionCollapsed {
        id: String,
        collapsed: bool,
    },
    /// Write only one favorites group's collapse flag.
    SetGroupCollapsed {
        group: usize,
        collapsed: bool,
    },
    /// Add one Projects discovery root, keeping comments and the rest of the file.
    AddProjectRoot(PathBuf),
    /// Remove one exact Projects discovery root, keeping comments.
    RemoveProjectRoot(PathBuf),
}

fn groups(doc: &mut DocumentMut) -> Result<&mut toml_edit::ArrayOfTables, String> {
    if doc.get("group").is_none() {
        let mut groups = toml_edit::ArrayOfTables::new();
        let mut first = toml_edit::Table::new();
        first["name"] = toml_edit::value("Favorites");
        first["items"] = toml_edit::value(Array::new());
        groups.push(first);
        doc.insert("group", Item::ArrayOfTables(groups));
    }
    doc.get_mut("group").and_then(Item::as_array_of_tables_mut).ok_or_else(|| "`group` must be [[group]] tables".into())
}

fn project_roots_array(doc: &mut DocumentMut) -> Result<&mut Array, String> {
    let projects = doc
        .entry("projects")
        .or_insert_with(|| Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .ok_or_else(|| "`projects` must be a table".to_owned())?;
    projects
        .entry("roots")
        .or_insert_with(|| toml_edit::value(Array::new()))
        .as_array_mut()
        .ok_or_else(|| "`projects.roots` must be an array".to_owned())
}

fn items(doc: &mut DocumentMut, group: usize) -> Result<&mut Array, String> {
    let table = groups(doc)?.get_mut(group).ok_or_else(|| format!("no group {group}"))?;
    if !table.contains_key("items") {
        table["items"] = toml_edit::value(Array::new());
    }
    table["items"].as_array_mut().ok_or_else(|| "a group's `items` must be an array".into())
}

/// Put an item on its own line, like the others in the default file.
fn on_own_line(mut v: Value) -> Value {
    let prefix = v.decor().prefix().and_then(|p| p.as_str()).unwrap_or("").to_owned();
    if !prefix.contains('\n') {
        v.decor_mut().set_prefix("\n  ");
    }
    v
}

fn finish(array: &mut Array) {
    array.set_trailing_comma(true);
    array.set_trailing("\n");
}

fn item_path(v: &Value) -> Option<&str> {
    v.as_inline_table()?.get("path")?.as_str()
}

fn apply(doc: &mut DocumentMut, edit: &SidebarEdit, home: &Path) -> Result<(), String> {
    match edit {
        SidebarEdit::AddProjectRoot(path) => {
            project_roots_array(doc)?.push(abbreviate_home(path, home));
        }
        SidebarEdit::RemoveProjectRoot(path) => {
            let roots = project_roots_array(doc)?;
            let mut index = 0;
            while index < roots.len() {
                let matches =
                    roots.get(index).and_then(Value::as_str).is_some_and(|spelled| expand_home(spelled, home) == *path);
                if matches {
                    roots.remove(index);
                } else {
                    index += 1;
                }
            }
        }
        SidebarEdit::SetGroupCollapsed { group, collapsed } => {
            let table = groups(doc)?.get_mut(*group).ok_or_else(|| format!("no group {group}"))?;
            crate::config::set_value(table, "collapsed", (*collapsed).into());
        }
        SidebarEdit::SetSectionCollapsed { id, collapsed } => {
            if id.is_empty() || id.contains('"') {
                return Err("a section id must be nonempty".into());
            }
            let section = doc
                .entry("section")
                .or_insert_with(|| Item::Table(toml_edit::Table::new()))
                .as_table_mut()
                .ok_or_else(|| "`section` must be a table".to_owned())?;
            let entry = section.entry(id).or_insert_with(|| Item::Table(toml_edit::Table::new()));
            let table = entry.as_table_mut().ok_or_else(|| format!("section \"{id}\" must be a table"))?;
            crate::config::set_value(table, "collapsed", (*collapsed).into());
        }
        SidebarEdit::Move { from, to } => {
            let value = {
                let source = items(doc, from.0)?;
                if from.1 >= source.len() {
                    return Err("nothing to move there".into());
                }
                let value = source.remove(from.1);
                finish(source);
                value
            };
            // Removing first shifts later items in the same group up by one.
            let index = if from.0 == to.0 && to.1 > from.1 { to.1 - 1 } else { to.1 };
            let target = items(doc, to.0)?;
            target.insert(index.min(target.len()), on_own_line(value));
            finish(target);
        }
        SidebarEdit::Add { path, at } => {
            let spelled = abbreviate_home(path, home);
            let target = items(doc, at.0)?;
            if target.iter().any(|v| item_path(v).is_some_and(|p| expand_home(p, home) == *path)) {
                return Ok(());
            }
            let mut table = InlineTable::new();
            table.insert("path", spelled.into());
            target.insert(at.1.min(target.len()), on_own_line(Value::InlineTable(table)));
            finish(target);
        }
        SidebarEdit::AddMany { paths, at } => {
            let target = items(doc, at.0)?;
            let mut inserted = 0;
            for path in paths {
                if target.iter().any(|v| item_path(v).is_some_and(|p| expand_home(p, home) == *path)) {
                    continue;
                }
                let mut table = InlineTable::new();
                table.insert("path", abbreviate_home(path, home).into());
                target.insert((at.1 + inserted).min(target.len()), on_own_line(Value::InlineTable(table)));
                inserted += 1;
            }
            finish(target);
        }
        SidebarEdit::Remove((g, i)) => {
            let target = items(doc, *g)?;
            if *i < target.len() {
                target.remove(*i);
                finish(target);
            }
        }
        SidebarEdit::SetKey { path, key } => {
            let count = groups(doc)?.len();
            let mut found = false;
            for g in 0..count {
                for v in items(doc, g)?.iter_mut() {
                    let Some(table) = v.as_inline_table_mut() else { continue };
                    let is_target =
                        table.get("path").and_then(Value::as_str).is_some_and(|p| expand_home(p, home) == *path);
                    let has_key = table.get("key").and_then(Value::as_str) == Some(key.to_string().as_str());
                    if is_target && !found {
                        table.insert("key", key.to_string().into());
                        found = true;
                    } else if has_key {
                        table.remove("key");
                    }
                }
            }
            if !found {
                let target = items(doc, 0)?;
                let mut table = InlineTable::new();
                table.insert("path", abbreviate_home(path, home).into());
                table.insert("key", key.to_string().into());
                target.push_formatted(on_own_line(Value::InlineTable(table)));
                finish(target);
            }
        }
    }
    Ok(())
}

pub fn sidebar_path(config_dir: &Path) -> PathBuf {
    config_dir.join("sidebar.toml")
}

/// Apply `edit` to `sidebar.toml` in `config_dir`, keeping comments and layout.
pub fn edit_sidebar(config_dir: &Path, home: &Path, edit: &SidebarEdit) -> Result<(), String> {
    edit_toml(&sidebar_path(config_dir), DEFAULT_SIDEBAR, |doc| apply(doc, edit, home))
}

/// Load `sidebar.toml`, writing the documented default first if it's missing.
pub fn load_sidebar(config_dir: &Path) -> Result<SidebarConfig, String> {
    let path = sidebar_path(config_dir);
    if !path.exists() {
        crate::config::write_atomic(&path, DEFAULT_SIDEBAR).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    SidebarConfig::parse(&text).map_err(|e| crate::config::file_message(&path, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/Users/me";

    fn paths(dir: &Path, group: usize) -> Vec<String> {
        load_sidebar(dir).unwrap().group[group].items.iter().map(|i| i.path.clone()).collect()
    }

    fn edit(dir: &Path, e: SidebarEdit) {
        edit_sidebar(dir, Path::new(HOME), &e).unwrap();
    }

    #[test]
    fn default_file_parses() {
        let c = SidebarConfig::parse(DEFAULT_SIDEBAR).unwrap();
        assert_eq!(c.group.len(), 1);
        assert_eq!(c.recent, RecentConfig::default());
        let favorites = c.favorites(Path::new(HOME));
        assert_eq!(favorites[0].1[1].path, PathBuf::from("/Users/me/Desktop"));
    }

    #[test]
    fn section_layout_parses_and_a_collapse_edit_preserves_comments() {
        let text = r#"
# keep this comment
[[group]]
name = "Favorites"
items = [{ path = "~" }]

[sections]
order = ["recent", "example.p1b-fixture.folders", "locations"]

[section."example.p1b-fixture.folders"]
show = true
collapsed = false

[recent]
show = true

[locations]
show = true
"#;
        let config = SidebarConfig::parse(text).unwrap();
        assert_eq!(config.sections.order[1], "example.p1b-fixture.folders");
        assert!(config.section_shown("example.p1b-fixture.folders"));
        assert!(!config.section_collapsed("example.p1b-fixture.folders"));
        assert!(SidebarConfig::parse("[sections]\norder = [\"not a section\"]").is_err());

        let mut doc = text.parse::<DocumentMut>().unwrap();
        apply(
            &mut doc,
            &SidebarEdit::SetSectionCollapsed { id: "example.p1b-fixture.folders".into(), collapsed: true },
            Path::new(HOME),
        )
        .unwrap();
        let out = doc.to_string();
        assert!(out.contains("# keep this comment"));
        assert!(out.contains("collapsed = true"));
        assert!(out.contains("[[group]]"));
        assert!(out.contains("order = [\"recent\", \"example.p1b-fixture.folders\", \"locations\"]"));
        let reparsed = SidebarConfig::parse(&out).unwrap();
        assert!(reparsed.section_collapsed("example.p1b-fixture.folders"));

        let mut doc = "[section.\"x.y\"]\ncollapsed = false # my note\n".parse::<DocumentMut>().unwrap();
        apply(&mut doc, &SidebarEdit::SetSectionCollapsed { id: "x.y".into(), collapsed: true }, Path::new(HOME))
            .unwrap();
        assert!(doc.to_string().contains("collapsed = true # my note"), "{doc}");
    }

    #[test]
    fn reorder_within_and_between_groups() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        load_sidebar(d).unwrap();
        // Desktop (1) to the end of the list.
        edit(d, SidebarEdit::Move { from: (0, 1), to: (0, 5) });
        assert_eq!(paths(d, 0), ["~", "~/Documents", "~/Downloads", "/Applications", "~/Desktop"]);
        // And back to the top.
        edit(d, SidebarEdit::Move { from: (0, 4), to: (0, 0) });
        assert_eq!(paths(d, 0), ["~/Desktop", "~", "~/Documents", "~/Downloads", "/Applications"]);

        std::fs::write(
            d.join("sidebar.toml"),
            std::fs::read_to_string(d.join("sidebar.toml")).unwrap()
                + "\n# my projects\n[[group]]\nname = \"Work\"\nitems = [\n  { path = \"~/Work\", color = \"blue\" },\n]\n",
        )
        .unwrap();
        edit(d, SidebarEdit::Move { from: (0, 0), to: (1, 1) });
        assert_eq!(paths(d, 1), ["~/Work", "~/Desktop"]);
        let text = std::fs::read_to_string(d.join("sidebar.toml")).unwrap();
        assert!(text.contains("# my projects"), "comments survive: {text}");
        assert!(text.contains("color = \"blue\""), "{text}");
        assert!(text.contains("\n  { path = \"~/Desktop\" },\n]"), "one item per line: {text}");
    }

    #[test]
    fn move_down_one_and_onto_itself() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        load_sidebar(d).unwrap();
        // Documents past Downloads: the bottom half of a row inserts after that row.
        edit(d, SidebarEdit::Move { from: (0, 2), to: (0, 4) });
        assert_eq!(paths(d, 0), ["~", "~/Desktop", "~/Downloads", "~/Documents", "/Applications"]);
        let before = paths(d, 0);
        // Dropping on either half of the dragged favorite itself changes nothing.
        edit(d, SidebarEdit::Move { from: (0, 2), to: (0, 2) });
        edit(d, SidebarEdit::Move { from: (0, 2), to: (0, 3) });
        assert_eq!(paths(d, 0), before);
    }

    #[test]
    fn add_remove_and_keys() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        edit(d, SidebarEdit::Add { path: "/Users/me/Projects".into(), at: (0, 1) });
        assert_eq!(paths(d, 0)[1], "~/Projects", "written home-relative");
        edit(d, SidebarEdit::Add { path: "/Users/me/Projects".into(), at: (0, 3) });
        assert_eq!(paths(d, 0).len(), 6, "already there: not added twice");
        edit(d, SidebarEdit::Remove((0, 1)));
        assert!(!paths(d, 0).contains(&"~/Projects".to_owned()));

        edit(d, SidebarEdit::SetKey { path: "/Users/me/Downloads".into(), key: 'd' });
        let c = load_sidebar(d).unwrap();
        assert_eq!(c.find_key('d', Path::new(HOME)), Some(PathBuf::from("/Users/me/Downloads")));
        // Moving the mark to another folder takes it from Downloads; a new folder is added.
        edit(d, SidebarEdit::SetKey { path: "/tmp/x".into(), key: 'd' });
        let c = load_sidebar(d).unwrap();
        assert_eq!(c.find_key('d', Path::new(HOME)), Some(PathBuf::from("/tmp/x")));
        assert_eq!(c.group[0].items.iter().filter(|i| i.key.is_some()).count(), 1);
        assert_eq!(c.group[0].items.last().unwrap().path, "/tmp/x");
    }

    #[test]
    fn add_many_keeps_every_path_in_one_write() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        edit(
            d,
            SidebarEdit::AddMany {
                paths: vec!["/Users/me/Projects".into(), "/Users/me/Work".into(), "/Users/me/Downloads".into()],
                at: (0, 1),
            },
        );
        assert_eq!(
            paths(d, 0),
            ["~", "~/Projects", "~/Work", "~/Desktop", "~/Documents", "~/Downloads", "/Applications"]
        );
        // A duplicate is skipped and does not push the new path past the slot.
        edit(d, SidebarEdit::AddMany { paths: vec!["/Users/me/Work".into(), "/Users/me/Play".into()], at: (0, 0) });
        assert_eq!(paths(d, 0)[0..2], ["~/Play", "~"], "the duplicate does not shift the new path");
        assert_eq!(paths(d, 0).len(), 8);
    }

    #[test]
    fn twins_need_two_paths() {
        let a = ["git".to_owned(), "one".to_owned(), "/".to_owned()];
        let one = [TwinInput { label: "git", path: Path::new("/one/git"), ancestors: &a }];
        assert_eq!(twin_hints(&one), [None]);
        let twice = [
            TwinInput { label: "git", path: Path::new("/one/git"), ancestors: &a },
            TwinInput { label: "git", path: Path::new("/one/git"), ancestors: &a },
        ];
        assert_eq!(twin_hints(&twice), [None, None], "a path is not its own twin");
    }

    #[test]
    fn twin_hint_is_the_parent() {
        let one = ["git".to_owned(), "one".to_owned(), "/".to_owned()];
        let two = ["git".to_owned(), "two".to_owned(), "/".to_owned()];
        let rows = [
            TwinInput { label: "git", path: Path::new("/one/git"), ancestors: &one },
            TwinInput { label: "git", path: Path::new("/two/git"), ancestors: &two },
        ];
        assert_eq!(twin_hints(&rows), [Some(1), Some(1)]);
    }

    #[test]
    fn twin_hints_walk_up_until_parents_differ() {
        let a = ["git".to_owned(), "x".to_owned(), "a".to_owned(), "/".to_owned()];
        let b = ["git".to_owned(), "x".to_owned(), "b".to_owned(), "/".to_owned()];
        let rows = [
            TwinInput { label: "git", path: Path::new("/a/x/git"), ancestors: &a },
            TwinInput { label: "git", path: Path::new("/b/x/git"), ancestors: &b },
        ];
        assert_eq!(twin_hints(&rows), [Some(2), Some(2)]);
    }

    #[test]
    fn twins_mix_levels() {
        let a = ["git".to_owned(), "x".to_owned(), "a".to_owned(), "/".to_owned()];
        let b = ["git".to_owned(), "x".to_owned(), "b".to_owned(), "/".to_owned()];
        let c = ["git".to_owned(), "c".to_owned(), "/".to_owned()];
        let rows = [
            TwinInput { label: "git", path: Path::new("/a/x/git"), ancestors: &a },
            TwinInput { label: "git", path: Path::new("/b/x/git"), ancestors: &b },
            TwinInput { label: "git", path: Path::new("/c/git"), ancestors: &c },
        ];
        assert_eq!(twin_hints(&rows), [Some(2), Some(2), Some(1)]);
    }

    #[test]
    fn twins_compare_shown_names_case_insensitively() {
        let one = ["git".to_owned(), "one".to_owned(), "/".to_owned()];
        let two = ["git".to_owned(), "two".to_owned(), "/".to_owned()];
        let rows = [
            TwinInput { label: "Git", path: Path::new("/one/git"), ancestors: &one },
            TwinInput { label: "git", path: Path::new("/two/git"), ancestors: &two },
        ];
        assert_eq!(twin_hints(&rows), [Some(1), Some(1)]);
    }

    #[test]
    fn same_path_twice_shares_its_hint() {
        let one = ["git".to_owned(), "one".to_owned(), "/".to_owned()];
        let two = ["git".to_owned(), "two".to_owned(), "/".to_owned()];
        let rows = [
            TwinInput { label: "git", path: Path::new("/one/git"), ancestors: &one },
            TwinInput { label: "git", path: Path::new("/one/git"), ancestors: &one },
            TwinInput { label: "git", path: Path::new("/two/git"), ancestors: &two },
        ];
        assert_eq!(twin_hints(&rows), [Some(1), Some(1), Some(1)]);
    }

    #[test]
    fn twin_at_volume_root_uses_volume_name() {
        let root = ["git".to_owned(), "/".to_owned()];
        let backup = ["git".to_owned(), "Backup".to_owned()];
        let rows = [
            TwinInput { label: "git", path: Path::new("/git"), ancestors: &root },
            TwinInput { label: "git", path: Path::new("/Volumes/Backup/git"), ancestors: &backup },
        ];
        assert_eq!(twin_hints(&rows), [Some(1), Some(1)]);
    }

    #[test]
    fn indistinguishable_twins_fall_back_to_parent_path() {
        let backup = ["git".to_owned(), "Backup".to_owned()];
        let rows = [
            TwinInput { label: "git", path: Path::new("/Volumes/A/git"), ancestors: &backup },
            TwinInput { label: "git", path: Path::new("/Volumes/B/git"), ancestors: &backup },
        ];
        assert_eq!(twin_hints(&rows), [Some(usize::MAX), Some(usize::MAX)]);
    }

    #[test]
    fn bad_values_are_errors() {
        let err = SidebarConfig::parse("[[group]]\nname = \"x\"\nitems = [{ path = \"~\", colour = \"red\" }]\n");
        assert!(err.unwrap_err().contains("colour"));
    }
}

#[cfg(test)]
mod project_tests {
    use super::*;

    #[test]
    fn project_roots_parse_expand_and_edit_without_losing_comments() {
        let text = r#"# keep me
[[group]]
name = "Favorites"
items = [{ path = "~" }]

[projects]
roots = ["~/code", "~/work"]
"#;
        let config = SidebarConfig::parse(text).unwrap();
        assert_eq!(
            config.project_roots(Path::new("/Users/me")),
            vec![PathBuf::from("/Users/me/code"), PathBuf::from("/Users/me/work")]
        );

        let mut doc = text.parse::<DocumentMut>().unwrap();
        apply(&mut doc, &SidebarEdit::AddProjectRoot(PathBuf::from("/Volumes/Work/src")), Path::new("/Users/me"))
            .unwrap();
        apply(&mut doc, &SidebarEdit::RemoveProjectRoot(PathBuf::from("/Users/me/work")), Path::new("/Users/me"))
            .unwrap();
        let out = doc.to_string();
        assert!(out.contains("# keep me"));
        assert!(out.contains("/Volumes/Work/src"));
        assert!(!out.contains("~/work"));
        let reparsed = SidebarConfig::parse(&out).unwrap();
        assert_eq!(
            reparsed.project_roots(Path::new("/Users/me")),
            vec![PathBuf::from("/Users/me/code"), PathBuf::from("/Volumes/Work/src")]
        );
    }

    #[test]
    fn a_group_collapse_flag_is_written_in_place() {
        let mut doc = "[[group]]\nname = \"A\"\nitems = []\n[[group]]\nname = \"B\" # mine\nitems = []\n"
            .parse::<DocumentMut>()
            .unwrap();
        apply(&mut doc, &SidebarEdit::SetGroupCollapsed { group: 1, collapsed: true }, Path::new("/Users/me")).unwrap();
        let config = SidebarConfig::parse(&doc.to_string()).unwrap();
        assert!(!config.group_collapsed(0) && config.group_collapsed(1));
        assert!(doc.to_string().contains("# mine"), "{doc}");
        assert!(
            apply(&mut doc, &SidebarEdit::SetGroupCollapsed { group: 5, collapsed: true }, Path::new("/Users/me"))
                .is_err()
        );
    }
}
