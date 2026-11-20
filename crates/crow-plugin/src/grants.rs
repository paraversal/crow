//! `plugins.toml`: one record authorizes exactly one package snapshot and its
//! API-1 capabilities. Comments survive edits; any invalid record withdraws
//! every grant until the file is fixed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crow_core::{PluginId, edit_toml, set_value, write_atomic};
use serde::Deserialize;
use toml_edit::{DocumentMut, Item, Table};

use crate::manifest::FsAccess;
use crate::package::PackageSnapshot;

pub const DEFAULT_PLUGINS: &str = r#"# Crow plugins. Copy a bundle directory into <config>/plugins, then approve it
# in Settings ▸ Plugins. Discovery alone never runs a script.
#
# One record authorizes exactly one snapshot: the package digest plus the
# capabilities the manifest requested. Changing plugin.toml, the entry script or
# a declared asset changes the digest and returns the package to pending
# approval. A missing or denied record leaves the package inert.
#
# [plugin."example.relative-path"]
# enabled = true
# digest = "<64 lowercase hex digits>"
# fs = "none"
"#;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    pub enabled: bool,
    pub digest: String,
    pub fs: FsAccess,
}

impl Grant {
    /// The snapshot this record authorizes, with exactly the requested
    /// capability set. Extra or missing capabilities do not match.
    pub fn authorizes(&self, snapshot: &PackageSnapshot) -> bool {
        self.enabled && self.digest == snapshot.digest && self.fs == snapshot.manifest.permissions.fs()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Grants {
    records: BTreeMap<PluginId, Grant>,
}

impl Grants {
    pub fn get(&self, id: &PluginId) -> Option<&Grant> {
        self.records.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&PluginId, &Grant)> {
        self.records.iter()
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// The grant that authorizes this exact snapshot, if any.
    pub fn authorizing(&self, id: &PluginId, snapshot: &PackageSnapshot) -> Option<&Grant> {
        self.records.get(id).filter(|grant| grant.authorizes(snapshot))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGrants {
    #[serde(default)]
    plugin: BTreeMap<String, RawGrant>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGrant {
    enabled: bool,
    digest: String,
    fs: String,
}

pub fn grants_path(config_dir: &Path) -> PathBuf {
    config_dir.join("plugins.toml")
}

/// Parse the whole file. A single invalid record makes the entire store
/// invalid: the caller withdraws every grant and shows the error.
pub fn parse_grants(text: &str) -> Result<Grants, String> {
    let raw: RawGrants = toml::from_str(text).map_err(|error| crow_core::parse_error(&error, text))?;
    let mut records = BTreeMap::new();
    for (key, record) in raw.plugin {
        let id = PluginId::parse(&key).map_err(|error| format!("plugin.\"{key}\": {error}"))?;
        if record.digest.len() != 64
            || !record.digest.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(format!("plugin.\"{key}\".digest must be 64 lowercase hex digits"));
        }
        let fs = match record.fs.as_str() {
            "none" => FsAccess::None,
            "read" => FsAccess::Read,
            other => {
                return Err(format!("plugin.\"{key}\".fs \"{other}\" must be \"none\" or \"read\""));
            }
        };
        records.insert(id, Grant { enabled: record.enabled, digest: record.digest, fs });
    }
    Ok(Grants { records })
}

pub fn load_grants(path: &Path) -> Result<Grants, String> {
    let text = std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    parse_grants(&text).map_err(|error| crow_core::file_message(path, error))
}

/// Write the documented default when the file is missing. Called from the
/// background discovery task, never from the main thread.
pub fn ensure_grants_file(path: &Path) -> Result<(), String> {
    match std::fs::metadata(path) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            write_atomic(path, DEFAULT_PLUGINS).map_err(|error| format!("{}: {error}", path.display()))
        }
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

/// Approve exactly this snapshot. Creates or replaces the record, keeping the
/// rest of the file (including comments and stale plugin IDs).
pub fn approve(path: &Path, id: &PluginId, snapshot: &PackageSnapshot) -> Result<(), String> {
    edit_toml(path, DEFAULT_PLUGINS, |doc| {
        let table = plugin_table(doc)?;
        let entry = table.entry(id.as_str()).or_insert_with(|| Item::Table(Table::new()));
        let entry = entry.as_table_mut().ok_or_else(|| format!("plugins.toml: plugin.\"{id}\" must be a table"))?;
        set_value(entry, "enabled", true.into());
        set_value(entry, "digest", snapshot.digest.clone().into());
        set_value(entry, "fs", snapshot.manifest.permissions.fs().key().into());
        Ok(())
    })
}

/// Disable or re-enable the existing record. The digest and capability set stay,
/// so re-enabling still requires the same snapshot.
pub fn set_enabled(path: &Path, id: &PluginId, enabled: bool) -> Result<(), String> {
    edit_toml(path, DEFAULT_PLUGINS, |doc| {
        let table = plugin_table(doc)?;
        let Some(entry) = table.get_mut(id.as_str()) else {
            return Ok(());
        };
        let entry = entry.as_table_mut().ok_or_else(|| format!("plugins.toml: plugin.\"{id}\" must be a table"))?;
        set_value(entry, "enabled", enabled.into());
        Ok(())
    })
}

/// Remove a record entirely, withdrawing approval.
pub fn withdraw(path: &Path, id: &PluginId) -> Result<(), String> {
    edit_toml(path, DEFAULT_PLUGINS, |doc| {
        let table = plugin_table(doc)?;
        table.remove(id.as_str());
        Ok(())
    })
}

fn plugin_table(doc: &mut DocumentMut) -> Result<&mut Table, String> {
    let plugins = doc.entry("plugin").or_insert_with(|| {
        let mut table = Table::new();
        table.set_implicit(true);
        Item::Table(table)
    });
    plugins.as_table_mut().ok_or_else(|| "plugins.toml: `plugin` must be a table".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Manifest, Permissions};
    use std::sync::Arc;

    fn snapshot(id: &str, digest: &str, fs: FsAccess) -> Arc<PackageSnapshot> {
        let manifest = Manifest {
            id: PluginId::parse(id).unwrap(),
            name: "Test".into(),
            version: "0.1.0".into(),
            api: 1,
            entry: "main.rhai".into(),
            provides: Vec::new(),
            assets: Vec::new(),
            permissions: Permissions { fs: Some(fs), spawn: Vec::new(), net: false },
        };
        Arc::new(PackageSnapshot { manifest, files: BTreeMap::new(), digest: digest.to_owned() })
    }

    fn plugin_id(text: &str) -> PluginId {
        PluginId::parse(text).unwrap()
    }

    #[test]
    fn default_file_is_empty_and_valid() {
        let grants = parse_grants(DEFAULT_PLUGINS).unwrap();
        assert!(grants.is_empty());
    }

    #[test]
    fn approve_disable_and_reapprove_preserve_comments_and_other_records() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("plugins.toml");
        ensure_grants_file(&path).unwrap();
        std::fs::write(
            &path,
            format!(
                "{}\n# a note\n[plugin.\"other.plugin\"]\nenabled = false\ndigest = \"{}\"\nfs = \"none\"\n",
                DEFAULT_PLUGINS,
                "a".repeat(64)
            ),
        )
        .unwrap();
        let package = snapshot("example.relative-path", &"b".repeat(64), FsAccess::Read);
        let id = plugin_id("example.relative-path");
        approve(&path, &id, &package).unwrap();
        let grants = load_grants(&path).unwrap();
        assert!(grants.authorizing(&id, &package).is_some());
        assert!(grants.get(&plugin_id("other.plugin")).is_some(), "stale records are preserved");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# a note"), "{text}");

        set_enabled(&path, &id, false).unwrap();
        let grants = load_grants(&path).unwrap();
        assert!(grants.authorizing(&id, &package).is_none(), "disabled is inert");
        let record = grants.get(&id).unwrap();
        assert_eq!(record.digest, "b".repeat(64), "disable keeps the snapshot identity");
        assert_eq!(record.fs, FsAccess::Read);

        set_enabled(&path, &id, true).unwrap();
        assert!(load_grants(&path).unwrap().authorizing(&id, &package).is_some());
    }

    #[test]
    fn edited_keys_keep_their_inline_comments() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("plugins.toml");
        let id = plugin_id("example.relative-path");
        std::fs::write(
            &path,
            format!(
                "[plugin.\"{id}\"]\nenabled = false # parked\ndigest = \"{}\" # old build\nfs = \"none\" # least\n",
                "a".repeat(64)
            ),
        )
        .unwrap();
        approve(&path, &id, &snapshot("example.relative-path", &"b".repeat(64), FsAccess::Read)).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        for note in ["# parked", "# old build", "# least"] {
            assert!(text.contains(note), "{note} survives approve: {text}");
        }
        set_enabled(&path, &id, false).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("# parked"));
    }

    #[test]
    fn a_changed_snapshot_or_capability_invalidates_the_grant() {
        let package = snapshot("example.relative-path", &"c".repeat(64), FsAccess::Read);
        let id = plugin_id("example.relative-path");
        let text = format!("[plugin.\"{id}\"]\nenabled = true\ndigest = \"{}\"\nfs = \"read\"\n", "c".repeat(64));
        let grants = parse_grants(&text).unwrap();
        assert!(grants.authorizing(&id, &package).is_some());

        let edited = snapshot("example.relative-path", &"d".repeat(64), FsAccess::Read);
        assert!(grants.authorizing(&id, &edited).is_none());
        let narrower = snapshot("example.relative-path", &"c".repeat(64), FsAccess::None);
        assert!(grants.authorizing(&id, &narrower).is_none(), "requested capabilities must match exactly");
    }

    #[test]
    fn a_broken_file_names_its_line() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("plugins.toml");
        std::fs::write(&path, "# note\n[plugin.\"example.relative-path\"]\nenabled = maybe\n").unwrap();
        let error = load_grants(&path).unwrap_err();
        assert!(error.starts_with(&format!("{} line 3:", path.display())), "{error}");
        assert!(!error.contains('\n'), "one line for the status bar: {error}");
        // A well-formed file with a bad record keeps the plain form.
        std::fs::write(
            &path,
            "[plugin.\"example.relative-path\"]\nenabled = true\ndigest = \"short\"\nfs = \"none\"\n",
        )
        .unwrap();
        assert!(load_grants(&path).unwrap_err().contains("digest must be 64"));
    }

    #[test]
    fn any_invalid_record_withdraws_the_whole_file() {
        let id = plugin_id("example.relative-path");
        for text in [
            format!("[plugin.\"{id}\"]\nenabled = true\ndigest = \"short\"\nfs = \"none\"\n"),
            format!("[plugin.\"{id}\"]\nenabled = true\ndigest = \"{}\"\nfs = \"write\"\n", "e".repeat(64)),
            format!("[plugin.\"{id}\"]\nenabled = true\ndigest = \"{}\"\nfs = \"none\"\nextra = 1\n", "e".repeat(64)),
            format!("[plugin.\"Bad.Id\"]\nenabled = true\ndigest = \"{}\"\nfs = \"none\"\n", "e".repeat(64)),
        ] {
            assert!(parse_grants(&text).is_err(), "{text}");
        }
    }
}
