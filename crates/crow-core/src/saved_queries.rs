//! Saved searches, persisted in `saved-queries.toml`. The application owns these
//! persisted specs; `crow-search` receives resolved predicates.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Deserializer};

use crate::{config, edit_toml};
use toml_edit::{ArrayOfTables, Item, Table, value};

/// The documented default `saved-queries.toml`, written on first launch.
pub const DEFAULT_SAVED_QUERIES: &str = r#"# Saved searches are live destinations in the sidebar. A saved search may search
# names and contents (scope "folder" or "home") or names through Spotlight
# (scope "this-mac"). Predicates combine with AND. An empty `text` is allowed
# when `kind` or `modified_within_days` can produce results.
#
# Each record is a [[query]] table:
#
#   id                    stable, opaque id; editing the title keeps it
#   name                  the sidebar, tab and breadcrumb title
#   scope                 "folder", "home" or "this-mac"
#   root                  an absolute folder or "~/…"; only with scope "folder"
#   text                  query text; names and contents for folder/home
#   match                 "all", "names" or "contents" (this-mac is names only)
#   include_hidden        visit dot-named entries while Show Hidden Files is off
#   enter_hidden_folders  descend into dot folders
#   exclusions            "configured" (live exclude.toml rules) or "everything"
#   kind                  optional "folder", "file", "image", "pdf", "audio",
#                         "video", "archive", "text", or an exact extension
#   modified_within_days  optional positive age in days
#
# Example:
#
# [[query]]
# id = "recent-pdfs"
# name = "Recent PDFs in Documents"
# scope = "folder"
# root = "~/Documents"
# text = ""
# match = "names"
# include_hidden = true
# enter_hidden_folders = false
# exclusions = "configured"
# kind = "pdf"
# modified_within_days = 30
"#;

/// A stable opaque saved-search id. Editing a search keeps it; history and tabs
/// keep referring to the id, never to a path.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(transparent)]
pub struct SavedQueryId(String);

impl SavedQueryId {
    pub fn new(id: impl Into<String>) -> Result<Self, String> {
        let id = id.into();
        let valid = !id.is_empty()
            && id.len() <= 80
            && !id.starts_with('.')
            && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
        if !valid {
            return Err(format!(
                "\"{id}\" is not a saved-search id; use letters, digits, '-', '_' or '.' (not starting with '.')"
            ));
        }
        Ok(Self(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SavedQueryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A tag destination id. P4B builds tag predicates on top of this type; the
/// location variant exists now so saved searches and tags share one history.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TagId(String);

impl TagId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SavedScope {
    #[default]
    Folder,
    Home,
    ThisMac,
}

impl SavedScope {
    pub fn key(self) -> &'static str {
        match self {
            SavedScope::Folder => "folder",
            SavedScope::Home => "home",
            SavedScope::ThisMac => "this-mac",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SavedMatch {
    #[default]
    All,
    Names,
    Contents,
}

impl SavedMatch {
    pub fn key(self) -> &'static str {
        match self {
            SavedMatch::All => "all",
            SavedMatch::Names => "names",
            SavedMatch::Contents => "contents",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SavedExclusions {
    #[default]
    Configured,
    Everything,
}

impl SavedExclusions {
    pub fn key(self) -> &'static str {
        match self {
            SavedExclusions::Configured => "configured",
            SavedExclusions::Everything => "everything",
        }
    }
}

/// A stable kind predicate, independent of the localized Finder kind name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SavedKind {
    Folder,
    File,
    Image,
    Pdf,
    Audio,
    Video,
    Archive,
    Text,
    Extension(String),
}

impl SavedKind {
    /// Parse a stable kind value. A category name is case-insensitive; anything
    /// else is an exact extension, with or without a leading dot.
    pub fn from_key(key: &str) -> Result<Self, String> {
        let key = key.trim();
        if key.is_empty() {
            return Err("kind is empty".into());
        }
        let lower = key.to_ascii_lowercase();
        let kind = match lower.as_str() {
            "folder" => SavedKind::Folder,
            "file" => SavedKind::File,
            "image" => SavedKind::Image,
            "pdf" => SavedKind::Pdf,
            "audio" => SavedKind::Audio,
            "video" => SavedKind::Video,
            "archive" => SavedKind::Archive,
            "text" => SavedKind::Text,
            _ => {
                let extension = key.strip_prefix('.').unwrap_or(key);
                if extension.is_empty()
                    || extension.len() > 32
                    || !extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
                {
                    return Err(format!(
                        "\"{key}\" is not a kind; use folder, file, image, pdf, audio, video, archive, text, or an extension"
                    ));
                }
                SavedKind::Extension(extension.to_ascii_lowercase())
            }
        };
        Ok(kind)
    }

    /// The value written to `saved-queries.toml`.
    pub fn key(&self) -> &str {
        match self {
            SavedKind::Folder => "folder",
            SavedKind::File => "file",
            SavedKind::Image => "image",
            SavedKind::Pdf => "pdf",
            SavedKind::Audio => "audio",
            SavedKind::Video => "video",
            SavedKind::Archive => "archive",
            SavedKind::Text => "text",
            SavedKind::Extension(extension) => extension,
        }
    }
}

impl<'de> Deserialize<'de> for SavedKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        SavedKind::from_key(&text).map_err(serde::de::Error::custom)
    }
}

fn default_true() -> bool {
    true
}

/// One persisted saved search. `text` may be empty when a predicate can produce
/// results. Predicates combine with AND.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedQuery {
    pub id: SavedQueryId,
    pub name: String,
    pub scope: SavedScope,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub text: String,
    #[serde(default, rename = "match")]
    pub matching: SavedMatch,
    #[serde(default = "default_true")]
    pub include_hidden: bool,
    #[serde(default)]
    pub enter_hidden_folders: bool,
    #[serde(default)]
    pub exclusions: SavedExclusions,
    #[serde(default)]
    pub kind: Option<SavedKind>,
    #[serde(default)]
    pub modified_within_days: Option<u32>,
}

impl SavedQuery {
    /// A fresh id for Save Search: the title, slugged, with a numeric suffix when
    /// another saved search already uses it.
    pub fn allocate_id(name: &str, existing: &SavedQueries) -> SavedQueryId {
        let mut stem: String = name
            .trim()
            .to_ascii_lowercase()
            .chars()
            .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
            .collect();
        while stem.contains("--") {
            stem = stem.replace("--", "-");
        }
        let stem = stem.trim_matches('-').to_owned();
        let stem = if stem.is_empty() { "search".to_owned() } else { stem };
        let stem: String = stem.chars().take(60).collect();
        if !existing.contains(&SavedQueryId(stem.clone())) {
            return SavedQueryId(stem);
        }
        for suffix in 2..10_000 {
            let id = format!("{stem}-{suffix}");
            if !existing.contains(&SavedQueryId(id.clone())) {
                return SavedQueryId(id);
            }
        }
        SavedQueryId(format!("{stem}-{}", std::process::id()))
    }

    /// Resolve and validate the folder root: absolute, `~` expanded, and present
    /// only for scope Folder. The path need not exist yet.
    pub fn root_path(&self, home: &Path) -> Result<Option<PathBuf>, String> {
        match self.scope {
            SavedScope::Folder => {
                let Some(root) = self.root.as_deref().filter(|root| !root.trim().is_empty()) else {
                    return Err("scope \"folder\" needs a root".into());
                };
                let root = config::expand_home(root.trim(), home);
                if !root.is_absolute() {
                    return Err(format!("root \"{}\" is not absolute; use ~/… or a full path", root.display()));
                }
                Ok(Some(root))
            }
            SavedScope::Home | SavedScope::ThisMac => {
                if self.root.as_deref().is_some_and(|root| !root.trim().is_empty()) {
                    return Err(format!(
                        "root is only valid with scope \"folder\"; {} does not use it",
                        self.scope.key()
                    ));
                }
                Ok(None)
            }
        }
    }

    /// The rolling age predicate as a duration, checked for arithmetic overflow.
    pub fn modified_within(&self) -> Result<Option<Duration>, String> {
        match self.modified_within_days {
            None => Ok(None),
            Some(0) => Err("modified_within_days must be positive".into()),
            Some(days) => {
                let seconds = u64::from(days)
                    .checked_mul(24 * 60 * 60)
                    .ok_or_else(|| "modified_within_days is too large".to_owned())?;
                Ok(Some(Duration::from_secs(seconds)))
            }
        }
    }

    /// Reject combinations the producer cannot honour, with an explicit reason.
    pub fn validate(&self, home: &Path) -> Result<(), String> {
        SavedQueryId::new(self.id.as_str())?;
        if self.name.trim().is_empty() {
            return Err("name is empty".into());
        }
        if self.text.contains('\n') || self.text.contains('\r') {
            return Err("text must be one line".into());
        }
        self.root_path(home)?;
        self.modified_within()?;
        if self.scope == SavedScope::ThisMac && self.matching != SavedMatch::Names {
            return Err("This Mac searches names through Spotlight; use match = \"names\" or scope \"home\"".into());
        }
        if self.text.trim().is_empty() && self.kind.is_none() && self.modified_within_days.is_none() {
            return Err("a saved search needs text, a kind, or modified_within_days".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedQueries {
    #[serde(default, rename = "query")]
    pub queries: Vec<SavedQuery>,
}

impl SavedQueries {
    pub fn parse(text: &str) -> Result<Self, String> {
        let queries: SavedQueries = toml::from_str(text).map_err(|error| config::parse_error(&error, text))?;
        let mut ids = std::collections::HashSet::new();
        for query in &queries.queries {
            SavedQueryId::new(query.id.as_str())?;
            if !ids.insert(query.id.clone()) {
                return Err(format!("duplicate saved-search id \"{}\"", query.id));
            }
            if query.name.trim().is_empty() {
                return Err(format!("saved search \"{}\": name is empty", query.id));
            }
        }
        Ok(queries)
    }

    /// Parse and validate every record against the current home directory.
    pub fn parse_and_validate(text: &str, home: &Path) -> Result<Self, String> {
        let queries = Self::parse(text)?;
        for query in &queries.queries {
            query.validate(home).map_err(|error| format!("saved search \"{}\": {error}", query.id))?;
        }
        Ok(queries)
    }

    pub fn get(&self, id: &SavedQueryId) -> Option<&SavedQuery> {
        self.queries.iter().find(|query| &query.id == id)
    }

    pub fn contains(&self, id: &SavedQueryId) -> bool {
        self.get(id).is_some()
    }

    pub fn is_empty(&self) -> bool {
        self.queries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.queries.len()
    }
}

pub fn saved_queries_path(config_dir: &Path) -> PathBuf {
    config_dir.join("saved-queries.toml")
}

/// Load `saved-queries.toml`, writing the documented default first if it's missing.
pub fn load_saved_queries(config_dir: &Path, home: &Path) -> Result<SavedQueries, String> {
    let path = saved_queries_path(config_dir);
    if !path.exists() {
        config::write_atomic(&path, DEFAULT_SAVED_QUERIES).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    SavedQueries::parse_and_validate(&text, home).map_err(|e| config::file_message(&path, e))
}

/// Insert or update one saved search by id, keeping the file's comments and the
/// record's other fields and layout.
pub fn save_saved_query(config_dir: &Path, home: &Path, query: &SavedQuery) -> Result<(), String> {
    query.validate(home)?;
    let path = saved_queries_path(config_dir);
    edit_toml(&path, DEFAULT_SAVED_QUERIES, |doc| {
        let current = SavedQueries::parse(&doc.to_string())?;
        let tables = query_tables(doc)?;
        let index = current.queries.iter().position(|existing| existing.id == query.id);
        match index {
            Some(index) => write_query(tables.get_mut(index).expect("indexed table"), query),
            None => {
                let mut table = Table::new();
                write_query(&mut table, query);
                tables.push(table);
            }
        }
        Ok(())
    })
}

/// Remove one saved search by id. Other records and all comments stay.
pub fn remove_saved_query(config_dir: &Path, id: &SavedQueryId) -> Result<(), String> {
    let path = saved_queries_path(config_dir);
    edit_toml(&path, DEFAULT_SAVED_QUERIES, |doc| {
        let current = SavedQueries::parse(&doc.to_string())?;
        let Some(index) = current.queries.iter().position(|query| &query.id == id) else {
            return Err(format!("saved-queries.toml: no saved search \"{id}\""));
        };
        let tables = query_tables(doc)?;
        tables.remove(index);
        Ok(())
    })
}

/// The `[[query]]` array, turning the documented `query = []` placeholder into
/// an array of tables on the first write.
fn query_tables(doc: &mut toml_edit::DocumentMut) -> Result<&mut ArrayOfTables, String> {
    let item = doc.entry("query").or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()));
    if item.as_array().is_some_and(|array| array.is_empty()) {
        *item = Item::ArrayOfTables(ArrayOfTables::new());
    }
    item.as_array_of_tables_mut().ok_or_else(|| "saved-queries.toml: `query` must be an array of tables".to_owned())
}

fn write_query(table: &mut Table, query: &SavedQuery) {
    set_query_value(table, "id", value(query.id.as_str()));
    set_query_value(table, "name", value(query.name.as_str()));
    set_query_value(table, "scope", value(query.scope.key()));
    match query.root.as_deref() {
        Some(root) if !root.trim().is_empty() => set_query_value(table, "root", value(root)),
        _ => {
            table.remove("root");
        }
    }
    set_query_value(table, "text", value(query.text.as_str()));
    set_query_value(table, "match", value(query.matching.key()));
    set_query_value(table, "include_hidden", value(query.include_hidden));
    set_query_value(table, "enter_hidden_folders", value(query.enter_hidden_folders));
    set_query_value(table, "exclusions", value(query.exclusions.key()));
    match &query.kind {
        Some(kind) => set_query_value(table, "kind", value(kind.key())),
        None => {
            table.remove("kind");
        }
    }
    match query.modified_within_days {
        Some(days) => set_query_value(table, "modified_within_days", value(i64::from(days))),
        None => {
            table.remove("modified_within_days");
        }
    }
}

fn set_query_value(table: &mut Table, key: &str, mut item: Item) {
    if let (Some(old), Some(new)) = (table.get(key).and_then(Item::as_value), item.as_value_mut()) {
        *new.decor_mut() = old.decor().clone();
    }
    table[key] = item;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    const HOME: &str = "/Users/me";

    fn query(id: &str) -> SavedQuery {
        SavedQuery {
            id: SavedQueryId::new(id).unwrap(),
            name: "Recent PDFs".into(),
            scope: SavedScope::Folder,
            root: Some("~/Documents".into()),
            text: String::new(),
            matching: SavedMatch::Names,
            include_hidden: true,
            enter_hidden_folders: false,
            exclusions: SavedExclusions::Configured,
            kind: Some(SavedKind::Pdf),
            modified_within_days: Some(30),
        }
    }

    #[test]
    fn documented_default_parses_to_the_code_default() {
        assert_eq!(SavedQueries::parse(DEFAULT_SAVED_QUERIES).unwrap(), SavedQueries::default());
    }

    #[test]
    fn scopes_resolve_and_reject_unsupported_combinations() {
        let mut pdf = query("pdfs");
        pdf.root = Some("~/Documents".into());
        assert_eq!(pdf.root_path(Path::new(HOME)).unwrap(), Some(PathBuf::from("/Users/me/Documents")));

        pdf.scope = SavedScope::Home;
        pdf.root = Some("~/Documents".into());
        assert!(pdf.root_path(Path::new(HOME)).is_err());
        pdf.root = None;
        assert!(pdf.validate(Path::new(HOME)).is_ok());

        pdf.scope = SavedScope::ThisMac;
        pdf.matching = SavedMatch::Names;
        assert!(pdf.validate(Path::new(HOME)).is_ok());
        pdf.matching = SavedMatch::Contents;
        let error = pdf.validate(Path::new(HOME)).unwrap_err();
        assert!(error.contains("names"), "{error}");
        pdf.matching = SavedMatch::All;
        assert!(pdf.validate(Path::new(HOME)).is_err());

        pdf.scope = SavedScope::Folder;
        pdf.root = Some("Documents".into());
        let error = pdf.root_path(Path::new(HOME)).unwrap_err();
        assert!(error.contains("absolute"), "{error}");
    }

    #[test]
    fn empty_text_needs_a_predicate_and_age_is_checked() {
        let mut pdf = query("pdfs");
        pdf.kind = None;
        pdf.modified_within_days = None;
        assert!(pdf.validate(Path::new(HOME)).is_err());
        pdf.kind = Some(SavedKind::Pdf);
        assert!(pdf.validate(Path::new(HOME)).is_ok());
        pdf.kind = None;
        pdf.modified_within_days = Some(0);
        assert!(pdf.modified_within().is_err());
        pdf.modified_within_days = Some(30);
        assert_eq!(pdf.modified_within().unwrap(), Some(Duration::from_secs(30 * 86_400)));
    }

    #[test]
    fn kinds_are_stable_and_extensions_are_exact() {
        assert_eq!(SavedKind::from_key("PDF").unwrap(), SavedKind::Pdf);
        assert_eq!(SavedKind::from_key("folder").unwrap(), SavedKind::Folder);
        assert_eq!(SavedKind::from_key(".Rust").unwrap(), SavedKind::Extension("rust".into()));
        assert_eq!(SavedKind::Extension("rs".into()).key(), "rs");
        assert!(SavedKind::from_key("").is_err());
        assert!(SavedKind::from_key("no/slash").is_err());
        assert!(SavedKind::from_key("*").is_err());
    }

    #[test]
    fn duplicate_ids_and_unknown_fields_are_rejected() {
        let text = format!(
            "{DEFAULT_SAVED_QUERIES}\n[[query]]\nid=\"a\"\nname=\"A\"\nscope=\"home\"\ntext=\"x\"\nkind=\"pdf\"\n\n[[query]]\nid=\"a\"\nname=\"B\"\nscope=\"home\"\ntext=\"y\"\nkind=\"pdf\"\n"
        );
        let error = SavedQueries::parse(&text).unwrap_err();
        assert!(error.contains("duplicate"), "{error}");

        let text = text.replace("id=\"a\"\nname=\"B\"", "id=\"b\"\nname=\"B\"\nmystery=true");
        assert!(SavedQueries::parse(&text).is_err());
    }

    #[test]
    fn save_and_remove_keep_comments_and_the_id() {
        let dir = TempDir::new("saved-queries");
        let config = dir.as_ref();
        let loaded = load_saved_queries(config, Path::new(HOME)).unwrap();
        assert!(loaded.is_empty());

        let mut first = query("pdfs");
        first.name = "First name".into();
        save_saved_query(config, Path::new(HOME), &first).unwrap();
        let text = std::fs::read_to_string(saved_queries_path(config)).unwrap();
        assert!(text.contains("# Saved searches are live destinations"), "{text}");
        assert!(text.contains("id = \"pdfs\""), "{text}");

        // A title edit keeps the id and any comment in the file.
        let path = saved_queries_path(config);
        let with_comment = text.replace("# Example:", "# Example: kept\n# mine:");
        std::fs::write(&path, with_comment).unwrap();
        first.name = "Second name".into();
        first.kind = Some(SavedKind::Text);
        save_saved_query(config, Path::new(HOME), &first).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# mine:"), "{text}");
        assert!(text.contains("name = \"Second name\""), "{text}");
        assert!(text.contains("kind = \"text\""), "{text}");
        assert_eq!(load_saved_queries(config, Path::new(HOME)).unwrap().get(&first.id), Some(&first));

        let mut second = query("images");
        second.name = "Images".into();
        second.kind = Some(SavedKind::Image);
        save_saved_query(config, Path::new(HOME), &second).unwrap();
        assert_eq!(load_saved_queries(config, Path::new(HOME)).unwrap().len(), 2);

        remove_saved_query(config, &first.id).unwrap();
        let loaded = load_saved_queries(config, Path::new(HOME)).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded.queries[0].id, second.id);
        assert!(std::fs::read_to_string(&path).unwrap().contains("# mine:"));
    }

    #[test]
    fn allocated_ids_are_stable_and_unique() {
        let mut existing = SavedQueries::default();
        let id = SavedQuery::allocate_id("Recent PDFs", &existing);
        assert_eq!(id.as_str(), "recent-pdfs");
        existing.queries.push(query(id.as_str()));
        let next = SavedQuery::allocate_id("Recent PDFs", &existing);
        assert_eq!(next.as_str(), "recent-pdfs-2");
        assert_ne!(next, id);
        assert_eq!(SavedQuery::allocate_id("Images", &existing).as_str(), "images");
    }
}
