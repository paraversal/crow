//! Batch rename: read-only preflight, volume name semantics, and the staged
//! executor shared by rename, undo and redo.
//!
//! Every changing source moves into a private, exclusively created container
//! beside its real parent, then to its final name. That is what makes swaps,
//! longer cycles and case-only renames safe: all sources are vacated before any
//! placement, and every move is `rename_no_clobber`. A failure reverses the
//! placements it made and restores the originals, again without replacing.
//!
//! Staging containers are registered in this process's active list, so normal
//! view reconciliation can ignore them. They are never inferred from their name
//! alone and never removed unless this process created and still owns them. A
//! crash leaves them in place; the paths in a recovery error are the actionable
//! record. A startup can therefore report leftovers by exact path without ever
//! deleting them by name.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use unicode_normalization::UnicodeNormalization;

use super::*;

/// One preview row's filesystem check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BatchRowState {
    Ready,
    Unchanged,
    Invalid(String),
}

/// One row to validate: the frozen source, its original name and the proposed
/// directory-entry name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchRenameInput {
    pub source: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub target_name: String,
}

/// An approved move. `identity` is the source snapshot the user saw; the engine
/// never silently adopts a new source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchRenameEntry {
    pub source: PathBuf,
    pub target: PathBuf,
    pub identity: FileIdentity,
}

/// The read-only preflight result. `moves` has only changing, valid rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchValidation {
    pub rows: Vec<BatchRowState>,
    pub moves: Vec<BatchRenameEntry>,
    /// A batch-wide reason no row can be renamed (an ancestor pair, for example).
    pub error: Option<String>,
}

/// How this volume compares directory-entry names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NameSemantics {
    pub case_sensitive: bool,
    /// Canonically equivalent Unicode spellings name the same entry.
    pub normalization_insensitive: bool,
}

/// Where data is when a rollback could not complete: its original path, where
/// it is now, and where the rollback was trying to put it (its original name).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryItem {
    pub original: PathBuf,
    pub current: PathBuf,
    pub intended: PathBuf,
}

#[cfg(test)]
type AfterStageHook = Arc<dyn Fn(&Path) + Send + Sync>;

/// Test-only gates for the batch executor. Production calls it with defaults.
#[derive(Clone, Default)]
pub(crate) struct BatchHooks {
    pub before_stage: Option<Arc<dyn Fn(usize) + Send + Sync>>,
    #[cfg(test)]
    pub after_stage: Option<AfterStageHook>,
    pub before_place: Option<Arc<dyn Fn(usize) + Send + Sync>>,
    pub before_restore: Option<Arc<dyn Fn(usize) + Send + Sync>>,
}

static ACTIVE_STAGING: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub(crate) fn register_staging(path: PathBuf) {
    ACTIVE_STAGING.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push(path);
}

pub(crate) fn unregister_staging(path: &Path) {
    let mut active = ACTIVE_STAGING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    active.retain(|candidate| candidate != path);
}

/// A snapshot of the private containers active in this process. Callers that
/// filter a large listing take one snapshot instead of locking per entry.
pub fn owned_staging_paths() -> Vec<PathBuf> {
    ACTIVE_STAGING.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
}

/// Whether `path` is inside a private container this process created for an
/// active operation. Reconciliation ignores these; nothing else is inferred.
pub fn is_owned_staging_path(path: &Path) -> bool {
    let active = ACTIVE_STAGING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    active.iter().any(|container| path.starts_with(container))
}

/// This volume's name comparison, proven from the mount's own capabilities.
/// APFS and HFS+ compare canonically equivalent Unicode spellings as one name;
/// other filesystems are refused rather than guessed.
pub fn name_semantics(dir: &Path) -> Result<NameSemantics, OpError> {
    #[cfg(target_os = "macos")]
    {
        macos_name_semantics(dir)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = dir;
        Err(OpError::Unsupported(dir.to_path_buf(), "batch rename on this platform"))
    }
}

#[cfg(target_os = "macos")]
fn macos_name_semantics(dir: &Path) -> Result<NameSemantics, OpError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c = CString::new(dir.as_os_str().as_bytes()).map_err(|_| OpError::InvalidName("NUL in a path".into()))?;
    let mut fs: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `fs` is writable.
    if unsafe { libc::statfs(c.as_ptr(), &mut fs) } != 0 {
        return Err(io_err(dir)(io::Error::last_os_error()));
    }
    let fstype = unsafe { std::ffi::CStr::from_ptr(fs.f_fstypename.as_ptr()) }.to_string_lossy().into_owned();
    if !matches!(fstype.as_str(), "apfs" | "hfs") {
        return Err(OpError::Unsupported(dir.to_path_buf(), "this volume's name comparison"));
    }

    #[repr(C)]
    struct Reply {
        length: u32,
        capabilities: libc::vol_capabilities_attr_t,
    }
    let mut list = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: 0,
        volattr: libc::ATTR_VOL_CAPABILITIES,
        dirattr: 0,
        fileattr: 0,
        forkattr: 0,
    };
    let mut reply: Reply = unsafe { std::mem::zeroed() };
    // SAFETY: the buffer is the C layout `getattrlist` expects for one
    // ATTR_VOL_CAPABILITIES value, and `list` is initialized.
    let read = unsafe {
        libc::getattrlist(
            c.as_ptr(),
            &mut list as *mut libc::attrlist as *mut std::ffi::c_void,
            &mut reply as *mut Reply as *mut std::ffi::c_void,
            std::mem::size_of::<Reply>(),
            0,
        )
    };
    if read != 0 {
        return Err(io_err(dir)(io::Error::last_os_error()));
    }
    let index = libc::VOL_CAPABILITIES_FORMAT;
    let valid = reply.capabilities.valid[index] & libc::VOL_CAP_FMT_CASE_SENSITIVE != 0;
    if !valid {
        return Err(OpError::Unsupported(dir.to_path_buf(), "this volume's case comparison"));
    }
    let case_sensitive = reply.capabilities.capabilities[index] & libc::VOL_CAP_FMT_CASE_SENSITIVE != 0;
    Ok(NameSemantics { case_sensitive, normalization_insensitive: true })
}

/// A name folded the way this volume compares directory entries.
fn folded_name(name: &str, semantics: NameSemantics) -> String {
    let name = if semantics.case_sensitive { name.to_owned() } else { name.to_lowercase() };
    if semantics.normalization_insensitive { name.nfc().collect() } else { name }
}

/// Whether `a` and `b` name the same directory entry under `semantics`.
pub fn names_equal(a: &str, b: &str, semantics: NameSemantics) -> bool {
    a == b || folded_name(a, semantics) == folded_name(b, semantics)
}

/// One directory entry keyed by its real parent and this volume's name rules.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct NameKey {
    parent: PathBuf,
    semantics: NameSemantics,
    folded: String,
}

fn parent_key(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Which sources contain, or are inside, another source. One canonicalize per
/// source and one walk up its parents, instead of a pair and two canonicalize
/// calls per pair.
fn ancestor_marks(paths: &[&Path], cancel: Option<&AtomicBool>) -> Result<(Vec<bool>, Vec<bool>), OpError> {
    let mut inside = vec![false; paths.len()];
    let mut contains = vec![false; paths.len()];
    let mut lexical: HashMap<&Path, Vec<usize>> = HashMap::new();
    let mut canonical: HashMap<PathBuf, Vec<usize>> = HashMap::new();
    for (index, path) in paths.iter().enumerate() {
        lexical.entry(*path).or_default().push(index);
        // Only a real directory can contain another selected item. A symlink
        // and the file or directory it points at are independent entries.
        if path.symlink_metadata().is_ok_and(|meta| meta.is_dir())
            && let Ok(real) = std::fs::canonicalize(path)
        {
            canonical.entry(real).or_default().push(index);
        }
    }
    for (index, path) in paths.iter().enumerate() {
        if index % 1024 == 0 && cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
            return Err(OpError::Cancelled);
        }
        let mut cursor = Some(*path);
        while let Some(parent) = cursor {
            if let Some(others) = lexical.get(parent) {
                for &other in others {
                    if other == index {
                        continue;
                    }
                    if parent == *path {
                        inside[index] = true;
                        inside[other] = true;
                    } else {
                        inside[index] = true;
                        contains[other] = true;
                    }
                }
            }
            cursor = parent.parent();
        }
        if let Ok(source) = std::fs::canonicalize(path) {
            let mut cursor = Some(source.as_path());
            while let Some(parent) = cursor {
                if let Some(others) = canonical.get(parent) {
                    for &other in others {
                        if other == index {
                            continue;
                        }
                        if parent == source.as_path() {
                            inside[index] = true;
                            inside[other] = true;
                        } else {
                            inside[index] = true;
                            contains[other] = true;
                        }
                    }
                }
                cursor = parent.parent();
            }
        }
    }
    Ok((inside, contains))
}

/// Mark every selected item that contains, or is inside, another selected item.
fn mark_ancestor_conflicts(
    inputs: &[BatchRenameInput],
    rows: &mut [BatchRowState],
    cancel: &AtomicBool,
) -> Result<bool, OpError> {
    let paths: Vec<&Path> = inputs.iter().map(|input| input.source.as_path()).collect();
    let (inside, contains) = ancestor_marks(&paths, Some(cancel))?;
    let mut found = false;
    for index in 0..inputs.len() {
        if inside[index] {
            rows[index] = BatchRowState::Invalid("it is inside another selected item".to_owned());
            found = true;
        } else if contains[index] {
            rows[index] = BatchRowState::Invalid("it contains another selected item".to_owned());
            found = true;
        }
    }
    Ok(found)
}

/// Read-only validation of the preview. Never mutates the disk.
pub fn validate_batch_rename(inputs: &[BatchRenameInput], cancel: &AtomicBool) -> Result<BatchValidation, OpError> {
    let mut rows = vec![BatchRowState::Ready; inputs.len()];
    let mut error = None;
    let mut identities: Vec<Option<FileIdentity>> = vec![None; inputs.len()];
    let mut semantics: Vec<Option<NameSemantics>> = vec![None; inputs.len()];

    // Source names must round-trip exactly; the text rename path cannot rewrite raw bytes.
    for (index, input) in inputs.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(OpError::Cancelled);
        }
        let original = input.source.file_name().and_then(|name| name.to_str());
        if original != Some(input.name.as_str()) {
            rows[index] = BatchRowState::Invalid("its name isn't valid text".to_owned());
            continue;
        }
        match identity(&input.source) {
            Ok(id) => identities[index] = Some(id),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                rows[index] = BatchRowState::Invalid("it no longer exists".to_owned());
            }
            Err(err) => rows[index] = BatchRowState::Invalid(err.to_string()),
        }
        if let Err(why) = check_name(&input.target_name) {
            rows[index] = BatchRowState::Invalid(match why {
                OpError::InvalidName(reason) => reason,
                other => other.to_string(),
            });
        }
        if rows[index] == BatchRowState::Ready && input.target_name == input.name {
            rows[index] = BatchRowState::Unchanged;
        }
    }

    // An ancestor and its descendant cannot be independent entries in one batch.
    if mark_ancestor_conflicts(inputs, &mut rows, cancel)? {
        error.get_or_insert_with(|| "Select items that don't contain one another".to_owned());
    }

    // One semantics probe per real parent.
    let mut parent_semantics: HashMap<PathBuf, Option<NameSemantics>> = HashMap::new();
    let mut parent_key_cache: HashMap<PathBuf, PathBuf> = HashMap::new();
    let mut parent_keys: Vec<Option<PathBuf>> = vec![None; inputs.len()];
    for (index, input) in inputs.iter().enumerate() {
        if index % 1024 == 0 && cancel.load(Ordering::Relaxed) {
            return Err(OpError::Cancelled);
        }
        if matches!(rows[index], BatchRowState::Invalid(_)) {
            continue;
        }
        let Some(parent) = input.source.parent() else {
            rows[index] = BatchRowState::Invalid("it has no parent folder".to_owned());
            continue;
        };
        let key = parent_key_cache.entry(parent.to_path_buf()).or_insert_with(|| parent_key(parent)).clone();
        let found = *parent_semantics.entry(key.clone()).or_insert_with(|| name_semantics(parent).ok());
        match found {
            Some(found) => {
                semantics[index] = Some(found);
                parent_keys[index] = Some(key);
            }
            None => {
                rows[index] = BatchRowState::Invalid("Crow can't compare names on this volume".to_owned());
            }
        }
    }

    // Two selected spellings of one directory entry must not be renamed twice.
    // Hard links have different names and pass; a symlinked parent spelling does
    // not, because both keys resolve to the same real parent and folded name.
    let mut source_entries: HashMap<NameKey, usize> = HashMap::new();
    for (index, input) in inputs.iter().enumerate() {
        if index % 1024 == 0 && cancel.load(Ordering::Relaxed) {
            return Err(OpError::Cancelled);
        }
        if matches!(rows[index], BatchRowState::Invalid(_)) {
            continue;
        }
        let (Some(parent), Some(semantics)) = (parent_keys[index].as_ref(), semantics[index]) else {
            continue;
        };
        let key = NameKey { parent: parent.clone(), semantics, folded: folded_name(&input.name, semantics) };
        if let Some(previous) = source_entries.insert(key, index) {
            rows[index] = BatchRowState::Invalid("it is the same item as another selected item".to_owned());
            rows[previous] = BatchRowState::Invalid("it is the same item as another selected item".to_owned());
        }
    }

    // Duplicate final names under each volume's own comparison rules.
    let mut target_keys: HashMap<NameKey, Vec<usize>> = HashMap::new();
    for (index, input) in inputs.iter().enumerate() {
        if index % 1024 == 0 && cancel.load(Ordering::Relaxed) {
            return Err(OpError::Cancelled);
        }
        if rows[index] != BatchRowState::Ready {
            continue;
        }
        let (Some(parent), Some(semantics)) = (parent_keys[index].as_ref(), semantics[index]) else {
            continue;
        };
        target_keys
            .entry(NameKey { parent: parent.clone(), semantics, folded: folded_name(&input.target_name, semantics) })
            .or_default()
            .push(index);
    }
    for group in target_keys.values() {
        if group.len() > 1 {
            for &index in group {
                rows[index] = BatchRowState::Invalid("another selected item would get this name".to_owned());
            }
        }
    }

    // Occupied destinations: a changing source may vacate one; anything else blocks.
    // A row waiting on a source that later becomes invalid is invalidated through
    // `remaining`, so one pass replaces the old repeated scan.
    let mut source_keys: Vec<Option<NameKey>> = vec![None; inputs.len()];
    let mut remaining: HashMap<NameKey, usize> = HashMap::new();
    for (index, input) in inputs.iter().enumerate() {
        if index % 1024 == 0 && cancel.load(Ordering::Relaxed) {
            return Err(OpError::Cancelled);
        }
        if rows[index] != BatchRowState::Ready {
            continue;
        }
        let (Some(parent), Some(semantics)) = (parent_keys[index].as_ref(), semantics[index]) else {
            continue;
        };
        let key = NameKey { parent: parent.clone(), semantics, folded: folded_name(&input.name, semantics) };
        source_keys[index] = Some(key.clone());
        *remaining.entry(key).or_default() += 1;
    }
    let mut dependents: HashMap<NameKey, Vec<usize>> = HashMap::new();
    let mut invalidated: Vec<usize> = Vec::new();
    for (index, input) in inputs.iter().enumerate() {
        if index % 1024 == 0 && cancel.load(Ordering::Relaxed) {
            return Err(OpError::Cancelled);
        }
        if rows[index] != BatchRowState::Ready {
            continue;
        }
        let (Some(parent), Some(semantics)) = (parent_keys[index].as_ref(), semantics[index]) else {
            continue;
        };
        // A target that is this entry under another spelling is the same
        // directory entry, not an occupant to vacate.
        if names_equal(&input.name, &input.target_name, semantics) {
            continue;
        }
        let target = parent.join(&input.target_name);
        match target.symlink_metadata() {
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                rows[index] = BatchRowState::Invalid(err.to_string());
                invalidated.push(index);
            }
            Ok(_) => {
                let key =
                    NameKey { parent: parent.clone(), semantics, folded: folded_name(&input.target_name, semantics) };
                if remaining.contains_key(&key) {
                    dependents.entry(key).or_default().push(index);
                } else {
                    rows[index] = BatchRowState::Invalid("that name is already taken".to_owned());
                    invalidated.push(index);
                }
            }
        }
    }
    let mut head = 0;
    while head < invalidated.len() {
        let index = invalidated[head];
        head += 1;
        let Some(key) = source_keys[index].take() else { continue };
        let Some(count) = remaining.get_mut(&key) else { continue };
        *count = count.saturating_sub(1);
        if *count == 0
            && let Some(waiting) = dependents.get(&key)
        {
            for &other in waiting {
                if rows[other] == BatchRowState::Ready {
                    rows[other] = BatchRowState::Invalid("that name is already taken".to_owned());
                    invalidated.push(other);
                }
            }
        }
    }

    let moves = inputs
        .iter()
        .enumerate()
        .filter(|(index, _)| rows[*index] == BatchRowState::Ready)
        .filter_map(|(index, input)| {
            Some(BatchRenameEntry {
                source: input.source.clone(),
                target: input.source.parent()?.join(&input.target_name),
                identity: identities[index].clone()?,
            })
        })
        .collect();
    Ok(BatchValidation { rows, moves, error })
}

/// Revalidate an already approved mapping before any mutation. Any change makes
/// the whole batch fail with zero filesystem changes.
pub(crate) fn plan_approved_moves(moves: &[BatchRenameEntry]) -> Result<Vec<PlannedItem>, OpError> {
    if moves.is_empty() {
        return Err(OpError::InvalidName("no names would change".to_owned()));
    }
    let paths: Vec<&Path> = moves.iter().map(|move_| move_.source.as_path()).collect();
    let (inside, contains) = ancestor_marks(&paths, None)?;
    if inside.iter().chain(contains.iter()).any(|flag| *flag) {
        return Err(OpError::InvalidName("the selection contains a folder and one of its items".to_owned()));
    }
    let mut parent_semantics: HashMap<PathBuf, NameSemantics> = HashMap::new();
    let mut parent_key_cache: HashMap<PathBuf, PathBuf> = HashMap::new();
    let mut parent_keys = Vec::with_capacity(moves.len());
    let mut semantics = Vec::with_capacity(moves.len());
    for move_ in moves {
        let source_parent = move_.source.parent().ok_or_else(|| OpError::NotFound(move_.source.clone()))?;
        if move_.target.parent() != Some(source_parent) {
            return Err(OpError::InvalidName("batch rename only changes names in place".to_owned()));
        }
        let key =
            parent_key_cache.entry(source_parent.to_path_buf()).or_insert_with(|| parent_key(source_parent)).clone();
        let sem = match parent_semantics.get(&key) {
            Some(found) => *found,
            None => {
                let found = name_semantics(source_parent)?;
                parent_semantics.insert(key.clone(), found);
                found
            }
        };
        parent_keys.push(key);
        semantics.push(sem);
    }
    // The entries this batch will vacate. One build, then one hash lookup per
    // occupied target instead of a metadata call against every other move.
    let mut vacating: HashSet<NameKey> = HashSet::with_capacity(moves.len());
    for (index, move_) in moves.iter().enumerate() {
        let name = super::file_name(&move_.source)?;
        vacating.insert(NameKey {
            parent: parent_keys[index].clone(),
            semantics: semantics[index],
            folded: folded_name(&name, semantics[index]),
        });
    }
    let mut targets = HashSet::with_capacity(moves.len());
    let mut items = Vec::with_capacity(moves.len());
    for (index, move_) in moves.iter().enumerate() {
        let target_name = super::file_name(&move_.target)?;
        check_name(&target_name)?;
        let current = identity(&move_.source).map_err(io_err(&move_.source))?;
        if !current.same_after_rename(&move_.identity) {
            return Err(OpError::BatchChanged(move_.source.clone()));
        }
        let key = NameKey {
            parent: parent_keys[index].clone(),
            semantics: semantics[index],
            folded: folded_name(&target_name, semantics[index]),
        };
        if !targets.insert(key.clone()) {
            return Err(OpError::InvalidName("two items would get the same name".to_owned()));
        }
        let target_identity = target_identity(&move_.target)?;
        if target_identity.is_some() {
            let source_name = super::file_name(&move_.source)?;
            if !names_equal(&source_name, &target_name, semantics[index]) && !vacating.contains(&key) {
                return Err(OpError::NameTaken(move_.target.clone()));
            }
        }
        items.push(PlannedItem {
            source: Some(move_.source.clone()),
            target: move_.target.clone(),
            conflict: false,
            bytes: 0,
            source_identity: Some(move_.identity.clone()),
            target_identity,
        });
    }
    Ok(items)
}

struct Staging {
    container: PathBuf,
    keep: bool,
}

impl Staging {
    fn create(parent: &Path) -> io::Result<Self> {
        use std::os::unix::fs::DirBuilderExt;
        let parent = parent_key(parent);
        for _ in 0..64 {
            let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let container = parent.join(format!(".crow-batch-{}-{sequence}", std::process::id()));
            let mut builder = std::fs::DirBuilder::new();
            builder.mode(0o700);
            match builder.create(&container) {
                Ok(()) => {
                    register_staging(container.clone());
                    return Ok(Self { container, keep: false });
                }
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(err),
            }
        }
        Err(io::Error::new(io::ErrorKind::AlreadyExists, "no private batch staging name was free"))
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        unregister_staging(&self.container);
        if !self.keep
            && let Err(err) = std::fs::remove_dir(&self.container)
        {
            warn(&format!("could not remove batch staging {}: {err}", self.container.display()));
        }
    }
}

struct Staged {
    index: usize,
    original: PathBuf,
    staged: PathBuf,
    target: PathBuf,
    identity: FileIdentity,
}

fn staging_slot(containers: &mut HashMap<PathBuf, Staging>, parent: &Path, index: usize) -> io::Result<PathBuf> {
    let key = parent_key(parent);
    if !containers.contains_key(&key) {
        containers.insert(key.clone(), Staging::create(parent)?);
    }
    Ok(containers[&key].container.join(format!("item-{index}")))
}

/// Run `moves` as one atomic-looking rename. `cancel` is consulted before each
/// mutation; once placements start, the commit segment finishes (or rolls back
/// on its own failure) before returning.
pub(crate) fn execute_batch(
    moves: &[BatchRenameEntry],
    cancel: &(dyn Fn() -> bool + Sync),
    placed: &mut dyn FnMut(usize),
    hooks: &BatchHooks,
) -> Result<Vec<BatchRenameEntry>, OpError> {
    plan_approved_moves(moves)?;
    let mut containers: HashMap<PathBuf, Staging> = HashMap::new();
    let mut staged: Vec<Staged> = Vec::with_capacity(moves.len());

    for (index, move_) in moves.iter().enumerate() {
        if cancel() {
            return rollback(moves, containers, staged, Vec::new(), OpError::Cancelled, hooks);
        }
        if let Some(hook) = &hooks.before_stage {
            hook(index);
        }
        let parent = match move_.source.parent() {
            Some(parent) => parent,
            None => {
                return rollback(moves, containers, staged, Vec::new(), OpError::NotFound(move_.source.clone()), hooks);
            }
        };
        let staged_path = match staging_slot(&mut containers, parent, index) {
            Ok(path) => path,
            Err(err) => {
                return rollback(moves, containers, staged, Vec::new(), io_err(&move_.source)(err), hooks);
            }
        };
        let current = match identity(&move_.source) {
            Ok(current) => current,
            Err(err) => {
                let error = if err.kind() == io::ErrorKind::NotFound {
                    OpError::BatchChanged(move_.source.clone())
                } else {
                    io_err(&move_.source)(err)
                };
                return rollback(moves, containers, staged, Vec::new(), error, hooks);
            }
        };
        if !current.same_after_rename(&move_.identity) {
            return rollback(moves, containers, staged, Vec::new(), OpError::BatchChanged(move_.source.clone()), hooks);
        }
        if let Err(err) = rename_no_clobber(&move_.source, &staged_path) {
            let error = if err.kind() == io::ErrorKind::AlreadyExists {
                OpError::NameTaken(staged_path.clone())
            } else {
                io_err(&move_.source)(err)
            };
            return rollback(moves, containers, staged, Vec::new(), error, hooks);
        }
        staged.push(Staged {
            index,
            original: move_.source.clone(),
            staged: staged_path.clone(),
            target: move_.target.clone(),
            identity: move_.identity.clone(),
        });
        #[cfg(test)]
        if let Some(hook) = &hooks.after_stage {
            hook(&staged_path);
        }
        match identity(&staged_path) {
            Ok(after) if after.same_after_rename(&move_.identity) => {}
            Ok(_) => {
                return rollback(
                    moves,
                    containers,
                    staged,
                    Vec::new(),
                    OpError::BatchChanged(move_.source.clone()),
                    hooks,
                );
            }
            Err(err) => {
                return rollback(moves, containers, staged, Vec::new(), io_err(&staged_path)(err), hooks);
            }
        }
    }

    if cancel() {
        return rollback(moves, containers, staged, Vec::new(), OpError::Cancelled, hooks);
    }

    let mut placed_items: Vec<usize> = Vec::new();
    for item in &staged {
        if let Some(hook) = &hooks.before_place {
            hook(item.index);
        }
        match rename_no_clobber(&item.staged, &item.target) {
            Ok(()) => {
                placed_items.push(item.index);
                placed(placed_items.len());
            }
            Err(err) => {
                let error = if err.kind() == io::ErrorKind::AlreadyExists {
                    OpError::BatchChanged(item.target.clone())
                } else {
                    io_err(&item.target)(err)
                };
                return rollback(moves, containers, staged, placed_items, error, hooks);
            }
        }
    }

    for staging in containers.values_mut() {
        staging.keep = false;
    }
    drop(containers);
    Ok(staged
        .into_iter()
        .map(|item| BatchRenameEntry { source: item.original, target: item.target, identity: item.identity })
        .collect())
}

fn rollback(
    moves: &[BatchRenameEntry],
    mut containers: HashMap<PathBuf, Staging>,
    staged: Vec<Staged>,
    placed: Vec<usize>,
    error: OpError,
    hooks: &BatchHooks,
) -> Result<Vec<BatchRenameEntry>, OpError> {
    let mut recovery = Vec::new();
    // Bring placed items back into their staging slots, newest first.
    for index in placed.iter().rev() {
        if let Some(hook) = &hooks.before_restore {
            hook(*index);
        }
        let item = staged.iter().find(|item| item.index == *index).expect("placed item was staged");
        // Never move a replacement some other writer put at the final name into
        // our staging. The identity we staged is the item this batch owns.
        let still_ours = identity(&moves[*index].target).is_ok_and(|current| current.same_object(&item.identity));
        if !still_ours || rename_no_clobber(&moves[*index].target, &item.staged).is_err() {
            recovery.push(RecoveryItem {
                original: item.original.clone(),
                current: item.target.clone(),
                intended: item.original.clone(),
            });
        }
    }
    // Restore every item that is still in its staging slot, newest first.
    for item in staged.iter().rev() {
        if recovery.iter().any(|rec| rec.original == item.original) {
            continue;
        }
        if let Some(hook) = &hooks.before_restore {
            hook(item.index);
        }
        if rename_no_clobber(&item.staged, &item.original).is_err() {
            recovery.push(RecoveryItem {
                original: item.original.clone(),
                current: item.staged.clone(),
                intended: item.original.clone(),
            });
        }
    }
    if !recovery.is_empty() {
        for staging in containers.values_mut() {
            staging.keep = staged
                .iter()
                .any(|item| item.staged.starts_with(&staging.container) && item.staged.symlink_metadata().is_ok());
        }
        return Err(OpError::BatchRecovery { items: recovery });
    }
    for staging in containers.values_mut() {
        staging.keep = false;
    }
    Err(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_staging_preserves_a_nonempty_container() {
        let dir = crate::test_support::TempDir::new("batch-drop-nonempty");
        let staging = Staging::create(&dir).unwrap();
        let item = staging.container.join("item-0");
        std::fs::write(&item, "contents").unwrap();
        drop(staging);
        assert_eq!(std::fs::read_to_string(item).unwrap(), "contents");
    }
}
