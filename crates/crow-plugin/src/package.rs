//! Bundle discovery and immutable package snapshots. Everything here is
//! background I/O: reads are bounded, bundle-relative and no-follow.

use std::collections::{BTreeMap, HashMap};
use std::ffi::{CString, OsString};
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crow_core::PluginId;
use sha2::{Digest, Sha256};

use crate::limits::Limits;
use crate::manifest::{API_VERSION, Manifest, PackageError, PackageErrorKind, parse_manifest, validate_relative};

/// The exact bytes shown for approval. The approval route and the worker both
/// use this value; nothing re-reads the bundle to execute it.
#[derive(Clone, Debug)]
pub struct PackageSnapshot {
    pub manifest: Manifest,
    /// `plugin.toml`, the entry and every declared asset, keyed by relative path.
    pub files: BTreeMap<String, Arc<[u8]>>,
    /// Lowercase SHA-256 hex over the framed, sorted file table.
    pub digest: String,
}

impl PackageSnapshot {
    pub fn entry_source(&self) -> &[u8] {
        self.files.get(&self.manifest.entry).map(|bytes| bytes.as_ref()).unwrap_or_default()
    }

    pub fn asset(&self, name: &str) -> Option<&[u8]> {
        self.manifest
            .assets
            .iter()
            .any(|asset| asset == name)
            .then(|| self.files.get(name))
            .flatten()
            .map(|b| b.as_ref())
    }
}

/// One immediate child directory, valid or not. `id`/`name` are best-effort
/// display values even when validation failed.
#[derive(Clone, Debug)]
pub struct PackageCandidate {
    /// The immediate child directory name exactly as the OS gave it. This is
    /// the stable selection key; it is never a lossy display string.
    pub bundle: OsString,
    /// Display-only lossy form of `bundle`.
    pub dir_name: String,
    pub id: Option<PluginId>,
    pub name: Option<String>,
    pub version: Option<String>,
    pub snapshot: Result<Arc<PackageSnapshot>, PackageError>,
}

#[derive(Clone, Debug, Default)]
pub struct Discovery {
    pub candidates: Vec<PackageCandidate>,
    /// The plugins directory itself could not be read.
    pub error: Option<String>,
}

/// Read immediate child bundles in `plugins_dir`. A missing directory is empty,
/// not an error. Discovery never evaluates a script.
pub fn discover(plugins_dir: &Path, limits: &Limits) -> Discovery {
    let mut discovery = Discovery::default();
    let root = match open_dir(plugins_dir, false) {
        Ok(fd) => fd,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return discovery,
        Err(error) => {
            discovery.error = Some(format!("{}: {error}", plugins_dir.display()));
            return discovery;
        }
    };
    let entries = match std::fs::read_dir(plugins_dir) {
        Ok(entries) => entries,
        Err(error) => {
            discovery.error = Some(format!("{}: {error}", plugins_dir.display()));
            return discovery;
        }
    };
    for entry in entries.flatten() {
        let bundle = entry.file_name();
        let dir_name = bundle.to_string_lossy().into_owned();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                discovery.candidates.push(invalid(
                    bundle,
                    dir_name,
                    PackageError::new(PackageErrorKind::Read, error.to_string()),
                ));
                continue;
            }
        };
        if file_type.is_symlink() {
            discovery.candidates.push(invalid(
                bundle,
                dir_name,
                PackageError::new(PackageErrorKind::Read, "bundle is a symbolic link; install a real directory"),
            ));
            continue;
        }
        if !file_type.is_dir() {
            continue;
        }
        let name = match loadable_name(&bundle) {
            Ok(name) => name,
            Err(error) => {
                discovery.candidates.push(invalid(bundle, dir_name, error));
                continue;
            }
        };
        discovery.candidates.push(load_candidate(&root, &bundle, name, &dir_name, limits));
    }
    discovery.candidates.sort_by(|a, b| a.bundle.cmp(&b.bundle));
    reject_duplicate_ids(&mut discovery.candidates);
    discovery
}

/// Load a bundle by its immediate child name, reopening it no-follow. Approval
/// uses this to prove the displayed bytes are still the bytes on disk.
pub fn load_package_at(
    plugins_dir: &Path,
    dir_name: &str,
    limits: &Limits,
) -> Result<Arc<PackageSnapshot>, PackageError> {
    validate_relative(dir_name).map_err(|error| PackageError::new(PackageErrorKind::Read, error))?;
    if dir_name.contains('/') {
        return Err(PackageError::new(PackageErrorKind::Read, "bundle name must be one directory"));
    }
    let root = open_dir(plugins_dir, false)
        .map_err(|error| PackageError::new(PackageErrorKind::Read, format!("{}: {error}", plugins_dir.display())))?;
    load_snapshot(&root, dir_name, limits)
}

/// Load a bundle directory directly. Used by tests and by callers that already
/// hold the immediate directory path.
pub fn load_package(dir: &Path, limits: &Limits) -> Result<Arc<PackageSnapshot>, PackageError> {
    let root = open_dir(dir, true)
        .map_err(|error| PackageError::new(PackageErrorKind::Read, format!("{}: {error}", dir.display())))?;
    load_snapshot(&root, ".", limits)
}

fn load_candidate(
    root: &OwnedFd,
    bundle: &OsString,
    dir_name: &str,
    display_name: &str,
    limits: &Limits,
) -> PackageCandidate {
    let mut candidate = PackageCandidate {
        bundle: bundle.clone(),
        dir_name: display_name.to_owned(),
        id: None,
        name: None,
        version: None,
        snapshot: Err(placeholder()),
    };
    let snapshot = match load_snapshot(root, dir_name, limits) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            // A readable manifest can still name the package in the inventory.
            if let Ok(bytes) = read_relative(root, &format!("{dir_name}/plugin.toml"), limits.max_file_bytes)
                && let Ok(text) = std::str::from_utf8(&bytes)
            {
                let (id, name, version) = probe_identity(text, limits);
                candidate.id = id;
                candidate.name = name;
                candidate.version = version;
            }
            candidate.snapshot = Err(error);
            return candidate;
        }
    };
    candidate.id = Some(snapshot.manifest.id.clone());
    candidate.name = Some(snapshot.manifest.name.clone());
    candidate.version = Some(snapshot.manifest.version.clone());
    candidate.snapshot = Ok(snapshot);
    candidate
}

fn load_snapshot(root: &OwnedFd, dir_name: &str, limits: &Limits) -> Result<Arc<PackageSnapshot>, PackageError> {
    let prefix = if dir_name == "." { String::new() } else { format!("{dir_name}/") };
    let manifest_name = format!("{prefix}plugin.toml");
    let manifest_bytes = read_relative(root, &manifest_name, limits.max_file_bytes)?;
    let manifest_text = std::str::from_utf8(&manifest_bytes)
        .map_err(|_| PackageError::new(PackageErrorKind::Manifest, "plugin.toml is not UTF-8"))?;
    let manifest = parse_manifest(manifest_text, limits)?;

    let entry_name = format!("{prefix}{}", manifest.entry);
    let entry_bytes = read_relative(root, &entry_name, limits.max_file_bytes)?;
    let mut files: BTreeMap<String, Arc<[u8]>> = BTreeMap::new();
    let mut total = manifest_bytes.len() + entry_bytes.len();
    files.insert("plugin.toml".into(), Arc::from(manifest_bytes.into_boxed_slice()));
    files.insert(manifest.entry.clone(), Arc::from(entry_bytes.into_boxed_slice()));
    for asset in &manifest.assets {
        let bytes = read_relative(root, &format!("{prefix}{asset}"), limits.max_file_bytes)?;
        total += bytes.len();
        if total > limits.max_package_bytes {
            return Err(PackageError::new(
                PackageErrorKind::Read,
                format!("package is larger than {} bytes", limits.max_package_bytes),
            ));
        }
        files.insert(asset.clone(), Arc::from(bytes.into_boxed_slice()));
    }
    if total > limits.max_package_bytes {
        return Err(PackageError::new(
            PackageErrorKind::Read,
            format!("package is larger than {} bytes", limits.max_package_bytes),
        ));
    }
    let digest = digest_files(&files);
    Ok(Arc::new(PackageSnapshot { manifest, files, digest }))
}

/// Read `rel` under `root`, opening each component with `O_NOFOLLOW`. Regular
/// files only, capped at `max_bytes` both by metadata and by the read itself.
fn read_relative(root: &OwnedFd, rel: &str, max_bytes: usize) -> Result<Vec<u8>, PackageError> {
    let (bytes, _) = read_relative_with_len(root, rel, max_bytes)?;
    Ok(bytes)
}

fn read_relative_with_len(root: &OwnedFd, rel: &str, max_bytes: usize) -> Result<(Vec<u8>, usize), PackageError> {
    validate_relative(rel).map_err(|error| PackageError::new(PackageErrorKind::Read, error))?;
    let components: Vec<&str> = rel.split('/').collect();
    let mut current: Option<OwnedFd> = None;
    for (index, component) in components.iter().enumerate() {
        let final_component = index + 1 == components.len();
        let dir_fd = current.as_ref().map_or_else(|| root.as_raw_fd(), |fd| fd.as_raw_fd());
        // A FIFO or device opened without O_NONBLOCK would park the one service
        // thread before its type is checked. Open nonblocking, then verify the
        // regular-file type and size on the descriptor before any read.
        let flags = libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | if final_component { libc::O_NONBLOCK } else { libc::O_DIRECTORY };
        let fd = open_at(dir_fd, component, flags)
            .map_err(|error| PackageError::new(PackageErrorKind::Read, format!("{rel}: {}", friendly_open(error))))?;
        if final_component {
            let file = File::from(fd);
            let metadata = file
                .metadata()
                .map_err(|error| PackageError::new(PackageErrorKind::Read, format!("{rel}: {error}")))?;
            if !metadata.is_file() {
                return Err(PackageError::new(PackageErrorKind::Read, format!("{rel} is not a regular file")));
            }
            if metadata.len() > max_bytes as u64 {
                return Err(PackageError::new(
                    PackageErrorKind::Read,
                    format!("{rel} is larger than {max_bytes} bytes"),
                ));
            }
            let mut bytes = Vec::with_capacity(metadata.len() as usize);
            file.take(max_bytes as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| PackageError::new(PackageErrorKind::Read, format!("{rel}: {error}")))?;
            if bytes.len() > max_bytes {
                return Err(PackageError::new(
                    PackageErrorKind::Read,
                    format!("{rel} grew past {max_bytes} bytes while reading"),
                ));
            }
            return Ok((bytes, metadata.len() as usize));
        }
        current = Some(fd);
    }
    Err(PackageError::new(PackageErrorKind::Read, format!("{rel}: empty path")))
}

fn friendly_open(error: io::Error) -> String {
    match error.raw_os_error() {
        Some(libc::ELOOP) => "symbolic link components are not allowed in a bundle".into(),
        Some(libc::ENOTDIR) => "a path component is not a directory".into(),
        _ => error.to_string(),
    }
}

fn open_dir(path: &Path, no_follow: bool) -> io::Result<OwnedFd> {
    let name = CString::new(path.as_os_str().as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let mut flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC;
    if no_follow {
        flags |= libc::O_NOFOLLOW;
    }
    let fd = unsafe { libc::open(name.as_ptr(), flags) };
    if fd < 0 { Err(io::Error::last_os_error()) } else { Ok(unsafe { OwnedFd::from_raw_fd(fd) }) }
}

fn open_at(dir_fd: i32, name: &str, flags: i32) -> io::Result<OwnedFd> {
    let name = CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let fd = unsafe { libc::openat(dir_fd, name.as_ptr(), flags) };
    if fd < 0 { Err(io::Error::last_os_error()) } else { Ok(unsafe { OwnedFd::from_raw_fd(fd) }) }
}

/// SHA-256 over `(name length, name, content length, content)` for each sorted
/// relative name, all lengths little-endian u64. Lowercase hex.
pub fn digest_files(files: &BTreeMap<String, Arc<[u8]>>) -> String {
    let mut hasher = Sha256::new();
    for (name, bytes) in files {
        hasher.update((name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes.as_ref());
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

fn reject_duplicate_ids(candidates: &mut [PackageCandidate]) {
    let mut by_id: HashMap<PluginId, Vec<usize>> = HashMap::new();
    for (index, candidate) in candidates.iter().enumerate() {
        if let Some(id) = candidate.id.clone() {
            by_id.entry(id).or_default().push(index);
        }
    }
    for (id, indexes) in by_id {
        if indexes.len() < 2 {
            continue;
        }
        let dirs: Vec<String> = indexes.iter().map(|index| candidates[*index].dir_name.clone()).collect();
        let message = format!("plugin id \"{id}\" is claimed by {}", dirs.join(", "));
        for index in indexes {
            candidates[index].snapshot = Err(PackageError::new(PackageErrorKind::DuplicateId, message.clone()));
        }
    }
}

fn probe_identity(text: &str, limits: &Limits) -> (Option<PluginId>, Option<String>, Option<String>) {
    let Ok(value) = toml::from_str::<toml::Value>(text) else {
        return (None, None, None);
    };
    let id = value.get("id").and_then(toml::Value::as_str).and_then(|text| PluginId::parse(text).ok());
    let name = value
        .get("name")
        .and_then(toml::Value::as_str)
        .filter(|name| name.len() <= limits.max_display_bytes && !name.chars().any(char::is_control))
        .map(str::to_owned);
    let version = value.get("version").and_then(toml::Value::as_str).map(str::to_owned);
    (id, name, version)
}

/// A bundle is opened by its UTF-8 name. A raw name the loader cannot
/// represent becomes a named unreadable row instead of a lossy open.
fn loadable_name(bundle: &OsString) -> Result<&str, PackageError> {
    bundle.to_str().ok_or_else(|| {
        PackageError::new(PackageErrorKind::Read, "bundle name is not UTF-8; rename it to a portable directory name")
    })
}

fn invalid(bundle: OsString, dir_name: String, error: PackageError) -> PackageCandidate {
    PackageCandidate { bundle, dir_name, id: None, name: None, version: None, snapshot: Err(error) }
}

fn placeholder() -> PackageError {
    PackageError::new(PackageErrorKind::Read, "not loaded")
}

/// The API version this crate implements, re-exported for the inventory.
pub const SUPPORTED_API: u32 = API_VERSION;

/// The canonical bundle path for an immediate child, for error messages.
pub fn bundle_path(plugins_dir: &Path, dir_name: &str) -> PathBuf {
    plugins_dir.join(dir_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::symlink;

    const MANIFEST: &str = r#"
id = "example.relative-path"
name = "Copy Relative Path"
version = "0.1.0"
api = 1
entry = "main.rhai"
provides = ["command"]
permissions = { fs = "none", spawn = [], net = false }
"#;

    fn write_bundle(root: &Path, name: &str, manifest: &str, script: &str) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("plugin.toml"), manifest).unwrap();
        std::fs::write(dir.join("main.rhai"), script).unwrap();
        dir
    }

    fn limits() -> Limits {
        Limits::default()
    }

    #[test]
    fn loads_the_exact_snapshot_and_digest() {
        let temp = tempfile::tempdir().unwrap();
        let dir = write_bundle(temp.path(), "relative", MANIFEST, "fn register() { [] }");
        let snapshot = load_package(&dir, &limits()).unwrap();
        assert_eq!(snapshot.manifest.id.as_str(), "example.relative-path");
        assert_eq!(snapshot.entry_source(), b"fn register() { [] }");
        assert_eq!(snapshot.digest.len(), 64);
        assert!(snapshot.digest.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));
    }

    #[test]
    fn digest_frames_sorted_names_and_lengths() {
        let mut files = BTreeMap::new();
        files.insert("a".to_owned(), Arc::from(b"one".as_slice()));
        files.insert("b".to_owned(), Arc::from(b"two".as_slice()));
        let first = digest_files(&files);
        let mut reversed = BTreeMap::new();
        reversed.insert("b".to_owned(), Arc::from(b"two".as_slice()));
        reversed.insert("a".to_owned(), Arc::from(b"one".as_slice()));
        assert_eq!(first, digest_files(&reversed));
        let mut changed = files.clone();
        changed.insert("b".to_owned(), Arc::from(b"twO".as_slice()));
        assert_ne!(first, digest_files(&changed), "content identity changes with bytes");
    }

    #[test]
    fn path_escape_is_rejected_before_opening() {
        let temp = tempfile::tempdir().unwrap();
        let dir = write_bundle(temp.path(), "escape", &MANIFEST.replace("main.rhai", "../evil.rhai"), "");
        std::fs::write(temp.path().join("evil.rhai"), "fn register() { [] }").unwrap();
        let error = load_package(&dir, &limits()).unwrap_err();
        assert_eq!(error.kind, PackageErrorKind::Manifest);
    }

    #[test]
    fn symlinked_entry_and_asset_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let manifest =
            MANIFEST.replace("provides = [\"command\"]", "assets = [\"icon.toml\"]\nprovides = [\"command\"]");
        let dir = write_bundle(temp.path(), "linked", &manifest, "fn register() { [] }");
        std::fs::write(dir.join("real.toml"), "icon = \"x\"").unwrap();
        symlink(dir.join("real.toml"), dir.join("icon.toml")).unwrap();
        let error = load_package(&dir, &limits()).unwrap_err();
        assert_eq!(error.kind, PackageErrorKind::Read);
        assert!(error.message.contains("symbolic link"), "{}", error.message);

        std::fs::remove_file(dir.join("icon.toml")).unwrap();
        std::fs::rename(dir.join("main.rhai"), dir.join("real.rhai")).unwrap();
        symlink(dir.join("real.rhai"), dir.join("main.rhai")).unwrap();
        let error = load_package(&dir, &limits()).unwrap_err();
        assert_eq!(error.kind, PackageErrorKind::Read);
        assert!(error.message.contains("symbolic link"), "{}", error.message);
    }

    #[test]
    fn symlinked_bundle_directory_is_not_followed() {
        let temp = tempfile::tempdir().unwrap();
        let real = write_bundle(temp.path(), "real", MANIFEST, "fn register() { [] }");
        let plugins = temp.path().join("plugins");
        std::fs::create_dir(&plugins).unwrap();
        symlink(&real, plugins.join("linked")).unwrap();
        let discovery = discover(&plugins, &limits());
        assert_eq!(discovery.candidates.len(), 1);
        assert_eq!(discovery.candidates[0].dir_name, "linked");
        assert_eq!(discovery.candidates[0].snapshot.as_ref().unwrap_err().kind, PackageErrorKind::Read);
    }

    #[test]
    fn a_non_utf8_bundle_name_is_rejected_instead_of_opened_lossily() {
        // macOS file systems reject invalid UTF-8 names, so the classification
        // is exercised directly; the raw key is still an OsString end to end.
        let raw = std::ffi::OsString::from_vec(vec![b'b', 0xff, b'x']);
        let error = loadable_name(&raw).unwrap_err();
        assert_eq!(error.kind, PackageErrorKind::Read);
        assert!(error.message.contains("not UTF-8"), "{}", error.message);
        let candidate = invalid(raw.clone(), raw.to_string_lossy().into_owned(), error);
        assert_eq!(candidate.bundle, raw);
        assert_eq!(candidate.dir_name, "b\u{fffd}x");
        assert_eq!(loadable_name(&std::ffi::OsString::from("plain")).unwrap(), "plain");
    }

    #[test]
    fn duplicate_ids_keep_distinct_raw_bundle_keys() {
        let temp = tempfile::tempdir().unwrap();
        let plugins = temp.path().join("plugins");
        std::fs::create_dir(&plugins).unwrap();
        write_bundle(&plugins, "one", MANIFEST, "fn register() { [] }");
        write_bundle(&plugins, "two", MANIFEST, "fn register() { [] }");
        let discovery = discover(&plugins, &limits());
        assert_eq!(discovery.candidates.len(), 2);
        assert_ne!(discovery.candidates[0].bundle, discovery.candidates[1].bundle);
        assert_eq!(discovery.candidates[0].id, discovery.candidates[1].id);
        for candidate in &discovery.candidates {
            assert_eq!(candidate.snapshot.as_ref().unwrap_err().kind, PackageErrorKind::DuplicateId);
        }
    }

    #[test]
    fn a_nonregular_bundle_member_is_rejected_without_blocking() {
        let temp = tempfile::tempdir().unwrap();
        let dir = write_bundle(temp.path(), "fifo", MANIFEST, "");
        std::fs::remove_file(dir.join("main.rhai")).unwrap();
        let name = std::ffi::CString::new(dir.join("main.rhai").as_os_str().as_encoded_bytes()).unwrap();
        let result = unsafe { libc::mkfifo(name.as_ptr(), 0o600) };
        assert_eq!(result, 0, "mkfifo: {}", std::io::Error::last_os_error());
        let started = std::time::Instant::now();
        let error = load_package(&dir, &limits()).unwrap_err();
        assert!(started.elapsed() < std::time::Duration::from_secs(1), "opening the FIFO must not block");
        assert_eq!(error.kind, PackageErrorKind::Read);
        assert!(error.message.contains("main.rhai"), "{}", error.message);
        assert!(error.message.contains("not a regular file"), "{}", error.message);
    }

    #[test]
    fn a_nonregular_asset_is_rejected_with_its_relative_name() {
        let temp = tempfile::tempdir().unwrap();
        let manifest =
            MANIFEST.replace("provides = [\"command\"]", "assets = [\"icons/pack.toml\"]\nprovides = [\"command\"]");
        let dir = write_bundle(temp.path(), "fifo-asset", &manifest, "fn register() { [] }");
        std::fs::create_dir(dir.join("icons")).unwrap();
        let name = std::ffi::CString::new(dir.join("icons/pack.toml").as_os_str().as_encoded_bytes()).unwrap();
        let result = unsafe { libc::mkfifo(name.as_ptr(), 0o600) };
        assert_eq!(result, 0, "mkfifo: {}", std::io::Error::last_os_error());
        let error = load_package(&dir, &limits()).unwrap_err();
        assert_eq!(error.kind, PackageErrorKind::Read);
        assert!(error.message.contains("icons/pack.toml"), "{}", error.message);
        assert!(error.message.contains("not a regular file"), "{}", error.message);
    }

    #[test]
    fn oversized_files_are_rejected_not_read() {
        let temp = tempfile::tempdir().unwrap();
        let dir = write_bundle(temp.path(), "big", MANIFEST, "");
        std::fs::write(dir.join("main.rhai"), vec![b'a'; 1024]).unwrap();
        let mut limits = limits();
        limits.max_file_bytes = 256;
        let error = load_package(&dir, &limits).unwrap_err();
        assert_eq!(error.kind, PackageErrorKind::Read);
        assert!(error.message.contains("larger than"), "{}", error.message);
    }

    #[test]
    fn duplicate_ids_publish_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let plugins = temp.path().join("plugins");
        std::fs::create_dir(&plugins).unwrap();
        write_bundle(&plugins, "one", MANIFEST, "fn register() { [] }");
        write_bundle(&plugins, "two", MANIFEST, "fn register() { [] }");
        let discovery = discover(&plugins, &limits());
        assert_eq!(discovery.candidates.len(), 2);
        for candidate in &discovery.candidates {
            let error = candidate.snapshot.as_ref().unwrap_err();
            assert_eq!(error.kind, PackageErrorKind::DuplicateId);
            assert!(error.message.contains("one") && error.message.contains("two"), "{}", error.message);
        }
    }

    #[test]
    fn invalid_manifest_still_names_the_package() {
        let temp = tempfile::tempdir().unwrap();
        let plugins = temp.path().join("plugins");
        std::fs::create_dir(&plugins).unwrap();
        write_bundle(&plugins, "broken", &MANIFEST.replace("api = 1", "api = 9"), "fn register() { [] }");
        let discovery = discover(&plugins, &limits());
        let candidate = &discovery.candidates[0];
        assert_eq!(candidate.name.as_deref(), Some("Copy Relative Path"));
        assert_eq!(candidate.snapshot.as_ref().unwrap_err().kind, PackageErrorKind::IncompatibleApi);
    }

    #[test]
    fn snapshot_bytes_do_not_change_when_the_bundle_does() {
        let temp = tempfile::tempdir().unwrap();
        let dir = write_bundle(temp.path(), "swap", MANIFEST, "fn register() { [] }");
        let approved = load_package(&dir, &limits()).unwrap();
        std::fs::write(dir.join("main.rhai"), "fn register() { [1] }").unwrap();
        let current = load_package(&dir, &limits()).unwrap();
        assert_eq!(approved.entry_source(), b"fn register() { [] }");
        assert_ne!(approved.digest, current.digest);
    }
}
