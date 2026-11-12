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
# Each [[group]] is a section of favorites. Items take:
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

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct SidebarConfig {
    pub group: Vec<Group>,
    pub recent: RecentConfig,
    pub locations: LocationsConfig,
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
        toml::from_str(text).map_err(|e| e.message().to_owned())
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

/// Where a favorite is: group index, item index.
pub type Slot = (usize, usize);

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
    Remove(Slot),
    /// Give `path` the mark `key`, adding it to the first group if it isn't a favorite yet.
    /// The key is taken from whichever favorite had it before.
    SetKey {
        path: PathBuf,
        key: char,
    },
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
    SidebarConfig::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
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
    fn bad_values_are_errors() {
        let err = SidebarConfig::parse("[[group]]\nname = \"x\"\nitems = [{ path = \"~\", colour = \"red\" }]\n");
        assert!(err.unwrap_err().contains("colour"));
    }
}
