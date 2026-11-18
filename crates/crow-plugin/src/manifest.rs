//! `plugin.toml`: deny-by-default permissions, known fields only, and the
//! API-1 capability set.

use std::path::{Component, Path};

use crow_core::{ExtensionKind, PluginId};
use serde::Deserialize;

use crate::limits::Limits;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsAccess {
    None,
    Read,
}

impl FsAccess {
    pub fn key(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Read => "read",
        }
    }

    pub fn permits_read(self) -> bool {
        self == Self::Read
    }
}

/// What the manifest asks for. API 1 grants `fs`; `spawn` and `net` are reserved
/// and rejected before a worker starts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Permissions {
    pub fs: Option<FsAccess>,
    pub spawn: Vec<String>,
    pub net: bool,
}

impl Permissions {
    pub fn fs(&self) -> FsAccess {
        self.fs.unwrap_or(FsAccess::None)
    }

    pub fn is_v1_supported(&self) -> bool {
        self.spawn.is_empty() && !self.net
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub id: PluginId,
    pub name: String,
    pub version: String,
    pub api: u32,
    pub entry: String,
    pub provides: Vec<ExtensionKind>,
    pub assets: Vec<String>,
    pub permissions: Permissions,
}

pub const API_VERSION: u32 = 1;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    id: String,
    name: String,
    version: String,
    api: u32,
    entry: String,
    #[serde(default)]
    provides: Vec<String>,
    #[serde(default)]
    assets: Vec<String>,
    #[serde(default)]
    permissions: RawPermissions,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPermissions {
    #[serde(default)]
    fs: Option<String>,
    #[serde(default)]
    spawn: Vec<String>,
    #[serde(default)]
    net: bool,
}

/// Why a package could not become a snapshot. The inventory keeps these
/// distinct so parse, ABI and capability failures read differently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageErrorKind {
    Read,
    Manifest,
    UnsupportedCapability,
    IncompatibleApi,
    ReservedId,
    DuplicateId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageError {
    pub kind: PackageErrorKind,
    pub message: String,
}

impl PackageError {
    pub fn new(kind: PackageErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }
}

/// Parse and validate `plugin.toml`.
pub fn parse_manifest(text: &str, limits: &Limits) -> Result<Manifest, PackageError> {
    let raw: RawManifest = toml::from_str(text).map_err(|error| {
        PackageError::new(PackageErrorKind::Manifest, format!("plugin.toml: {}", first_line(&error.to_string())))
    })?;

    let id = PluginId::parse(&raw.id)
        .map_err(|error| PackageError::new(PackageErrorKind::Manifest, format!("plugin.toml id: {error}")))?;
    if id.is_reserved() {
        return Err(PackageError::new(
            PackageErrorKind::ReservedId,
            format!("plugin id \"{id}\" is reserved for bundled providers"),
        ));
    }

    if raw.name.trim().is_empty() {
        return Err(PackageError::new(PackageErrorKind::Manifest, "plugin.toml name is empty"));
    }
    if raw.name.len() > limits.max_display_bytes || raw.name.chars().any(char::is_control) {
        return Err(PackageError::new(
            PackageErrorKind::Manifest,
            format!("plugin.toml name must be plain text of at most {} bytes", limits.max_display_bytes),
        ));
    }

    if !valid_version(&raw.version) {
        return Err(PackageError::new(
            PackageErrorKind::Manifest,
            format!("plugin.toml version \"{}\" is not a semantic version", raw.version),
        ));
    }

    if raw.api != API_VERSION {
        return Err(PackageError::new(
            PackageErrorKind::IncompatibleApi,
            format!("plugin API {} is not supported; this build runs api = {API_VERSION}", raw.api),
        ));
    }

    validate_relative(&raw.entry)
        .map_err(|error| PackageError::new(PackageErrorKind::Manifest, format!("plugin.toml entry: {error}")))?;
    if raw.entry == "plugin.toml" {
        return Err(PackageError::new(PackageErrorKind::Manifest, "plugin.toml entry cannot be the manifest"));
    }

    if raw.provides.is_empty() {
        return Err(PackageError::new(PackageErrorKind::Manifest, "plugin.toml provides no extension points"));
    }
    let mut provides = Vec::with_capacity(raw.provides.len());
    for key in &raw.provides {
        let kind = ExtensionKind::parse(key).ok_or_else(|| {
            PackageError::new(PackageErrorKind::Manifest, format!("plugin.toml provides unknown extension \"{key}\""))
        })?;
        if provides.contains(&kind) {
            return Err(PackageError::new(
                PackageErrorKind::Manifest,
                format!("plugin.toml provides \"{key}\" more than once"),
            ));
        }
        provides.push(kind);
    }

    let mut assets = Vec::with_capacity(raw.assets.len());
    for asset in &raw.assets {
        validate_relative(asset)
            .map_err(|error| PackageError::new(PackageErrorKind::Manifest, format!("plugin.toml asset: {error}")))?;
        if asset == "plugin.toml" || *asset == raw.entry {
            return Err(PackageError::new(
                PackageErrorKind::Manifest,
                format!("plugin.toml asset \"{asset}\" duplicates the manifest or entry"),
            ));
        }
        if assets.contains(asset) {
            return Err(PackageError::new(
                PackageErrorKind::Manifest,
                format!("plugin.toml asset \"{asset}\" is listed twice"),
            ));
        }
        assets.push(asset.clone());
    }

    let fs = match raw.permissions.fs.as_deref() {
        None | Some("none") => None,
        Some("read") => Some(FsAccess::Read),
        Some(other) => {
            return Err(PackageError::new(
                PackageErrorKind::UnsupportedCapability,
                format!("plugin.toml fs capability \"{other}\" is not available in api 1 (none or read)"),
            ));
        }
    };
    if !raw.permissions.spawn.is_empty() {
        return Err(PackageError::new(
            PackageErrorKind::UnsupportedCapability,
            format!("plugin.toml requests spawn ({}), which api 1 does not grant", raw.permissions.spawn.join(", ")),
        ));
    }
    if raw.permissions.net {
        return Err(PackageError::new(
            PackageErrorKind::UnsupportedCapability,
            "plugin.toml requests net, which api 1 does not grant",
        ));
    }

    Ok(Manifest {
        id,
        name: raw.name,
        version: raw.version,
        api: raw.api,
        entry: raw.entry,
        provides,
        assets,
        permissions: Permissions { fs, spawn: Vec::new(), net: false },
    })
}

/// A bundle-relative path with no traversal, absolute root or platform prefix.
pub fn validate_relative(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("path is empty".into());
    }
    if name.len() > 1024 {
        return Err("path is longer than 1024 bytes".into());
    }
    if name.contains('\0') || name.contains('\\') {
        return Err(format!("\"{name}\" is not a portable bundle path"));
    }
    let path = Path::new(name);
    let mut components = path.components();
    let Some(Component::Normal(_)) = components.next() else {
        return Err(format!("\"{name}\" must be a relative file path"));
    };
    if components.any(|component| !matches!(component, Component::Normal(_))) {
        return Err(format!("\"{name}\" must not contain . or .. segments"));
    }
    if path.is_absolute() {
        return Err(format!("\"{name}\" must be relative"));
    }
    Ok(())
}

/// Semver-shaped metadata validation: exactly major.minor.patch, optional
/// `-prerelease` and `+build`, no leading zeros on numeric identifiers.
fn valid_version(version: &str) -> bool {
    let (without_build, build) = match version.split_once('+') {
        Some((version, build)) => (version, Some(build)),
        None => (version, None),
    };
    let (core_part, pre) = match without_build.split_once('-') {
        Some((core_part, pre)) => (core_part, Some(pre)),
        None => (without_build, None),
    };
    let parts: Vec<&str> = core_part.split('.').collect();
    if parts.len() != 3 || !parts.iter().all(|part| numeric_identifier(part)) {
        return false;
    }
    pre.is_none_or(identifiers) && build.is_none_or(identifiers)
}

fn numeric_identifier(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()) && (text == "0" || !text.starts_with('0'))
}

fn identifiers(text: &str) -> bool {
    !text.is_empty()
        && text
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'))
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or(text).trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
id = "example.relative-path"
name = "Copy Relative Path"
version = "0.1.0"
api = 1
entry = "main.rhai"
provides = ["command"]
permissions = { fs = "none", spawn = [], net = false }
"#;

    fn limits() -> Limits {
        Limits::default()
    }

    #[test]
    fn minimal_manifest_is_accepted() {
        let manifest = parse_manifest(MINIMAL, &limits()).unwrap();
        assert_eq!(manifest.id.as_str(), "example.relative-path");
        assert_eq!(manifest.entry, "main.rhai");
        assert_eq!(manifest.provides, [ExtensionKind::Command]);
        assert_eq!(manifest.permissions.fs(), FsAccess::None);
        assert!(manifest.permissions.is_v1_supported());
    }

    #[test]
    fn unknown_fields_and_bad_ids_are_manifest_errors() {
        let unknown = MINIMAL.replace("entry = \"main.rhai\"", "entry = \"main.rhai\"\ncolour = \"red\"");
        assert_eq!(parse_manifest(&unknown, &limits()).unwrap_err().kind, PackageErrorKind::Manifest);
        let bad_id = MINIMAL.replace("example.relative-path", "Example.Relative");
        assert_eq!(parse_manifest(&bad_id, &limits()).unwrap_err().kind, PackageErrorKind::Manifest);
        let reserved = MINIMAL.replace("example.relative-path", "crow.git");
        assert_eq!(parse_manifest(&reserved, &limits()).unwrap_err().kind, PackageErrorKind::ReservedId);
    }

    #[test]
    fn api_and_capabilities_fail_before_any_work() {
        let api = MINIMAL.replace("api = 1", "api = 2");
        assert_eq!(parse_manifest(&api, &limits()).unwrap_err().kind, PackageErrorKind::IncompatibleApi);
        let spawn = MINIMAL.replace("spawn = []", "spawn = [\"git\"]");
        assert_eq!(parse_manifest(&spawn, &limits()).unwrap_err().kind, PackageErrorKind::UnsupportedCapability);
        let net = MINIMAL.replace("net = false", "net = true");
        assert_eq!(parse_manifest(&net, &limits()).unwrap_err().kind, PackageErrorKind::UnsupportedCapability);
        let fs = MINIMAL.replace("fs = \"none\"", "fs = \"write\"");
        assert_eq!(parse_manifest(&fs, &limits()).unwrap_err().kind, PackageErrorKind::UnsupportedCapability);
    }

    #[test]
    fn paths_and_versions_are_strict() {
        for bad in ["../escape.rhai", "/etc/passwd", "./main.rhai", "a/../../b.rhai", "a\\b.rhai", ""] {
            assert!(validate_relative(bad).is_err(), "{bad} should be rejected");
        }
        assert!(validate_relative("icons/rust.toml").is_ok());
        for good in ["0.1.0", "1.2.3-beta.1", "1.2.3+build.7", "10.0.0-rc.1+build"] {
            assert!(valid_version(good), "{good}");
        }
        for bad in ["1", "1.2", "01.2.3", "1.2.3-", "1.2.3+", "1.2.3-rc..1", "v1.2.3"] {
            assert!(!valid_version(bad), "{bad}");
        }
    }

    #[test]
    fn duplicate_provides_and_assets_are_rejected() {
        let duplicate_kind = MINIMAL.replace("provides = [\"command\"]", "provides = [\"command\", \"command\"]");
        assert_eq!(parse_manifest(&duplicate_kind, &limits()).unwrap_err().kind, PackageErrorKind::Manifest);
        let duplicate_asset =
            MINIMAL.replace("provides = [\"command\"]", "assets = [\"a.toml\", \"a.toml\"]\nprovides = [\"command\"]");
        assert_eq!(parse_manifest(&duplicate_asset, &limits()).unwrap_err().kind, PackageErrorKind::Manifest);
        let entry_asset =
            MINIMAL.replace("provides = [\"command\"]", "assets = [\"main.rhai\"]\nprovides = [\"command\"]");
        assert_eq!(parse_manifest(&entry_asset, &limits()).unwrap_err().kind, PackageErrorKind::Manifest);
    }
}
