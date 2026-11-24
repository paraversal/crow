//! The file operation engine: every change Crow makes to files goes through here.
//!
//! Three stages, so nothing touches the disk before the user has seen what will happen:
//! 1. [`plan`] turns a request into items and marks conflicts. Read-only.
//! 2. [`Job`] carries out the plan with ordered events, pause/cancel gates and
//!    conflict decisions.
//! 3. [`undo`] reverses a finished (or partly finished) operation from its [`Record`].
//!
//! Safety rules, enforced here rather than trusted to callers:
//! - Nothing is ever overwritten in place. Copies are written into an exclusively
//!   created private container beside the target and renamed into place with
//!   `RENAME_EXCL`, which fails rather than replace.
//! - "Replace" stages the new bytes before moving the approved target to the Trash,
//!   so a failed copy cannot destroy the old item.
//! - A cancelled or failed copy leaves no partial item behind; the source is untouched.
//! - A move across volumes deletes the source only after the copy is complete.
//! - Undo refuses to overwrite anything that appeared in the meantime.

use crate::listing::{is_package_ext, is_package_ext_bytes};
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

mod batch;
mod job;

pub(crate) use batch::{BatchHooks, execute_batch, plan_approved_moves};
pub use batch::{
    BatchRenameEntry, BatchRenameInput, BatchRowState, BatchValidation, NameSemantics, RecoveryItem,
    is_owned_staging_path, name_semantics, names_equal, owned_staging_paths, validate_batch_rename,
};

#[cfg(test)]
pub(crate) use job::FORCE_CROSS_DEVICE;
#[cfg(test)]
pub(crate) use job::TestHooks;
#[cfg(test)]
pub use job::execute;
pub use job::{Conflict, ConflictKey, ConflictReason, Job, JobEvent, JobId, JobOptions};

/// Moves items to the Trash. The app uses macOS's Trash (so Finder can Put Back);
/// tests use a plain folder.
pub trait Trash: Send + Sync {
    /// Trash `path` and return where it now lives.
    fn trash(&self, path: &Path) -> io::Result<PathBuf>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    NewFolder {
        parent: PathBuf,
    },
    NewFile {
        parent: PathBuf,
    },
    Rename {
        path: PathBuf,
        new_name: String,
    },
    /// Several names in place, already approved with their source identities.
    BatchRename {
        moves: Arc<[BatchRenameEntry]>,
    },
    Duplicate {
        paths: Arc<[PathBuf]>,
    },
    Copy {
        paths: Arc<[PathBuf]>,
        into: PathBuf,
    },
    Move {
        paths: Arc<[PathBuf]>,
        into: PathBuf,
    },
    Trash {
        paths: Arc<[PathBuf]>,
    },
    /// Not undoable; the UI asks first.
    DeletePermanently {
        paths: Arc<[PathBuf]>,
    },
}

/// What to do when the target name is taken.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Resolution {
    /// Give the new item a free name ("report 2.pdf"), like Finder.
    #[default]
    KeepBoth,
    /// Move the existing item to the Trash, then put the new one in its place.
    Replace,
    Skip,
}

/// What kind of object a path names, without following a symlink.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    Other,
}

/// A no-follow identity snapshot, for detecting that a source or target changed.
/// `dev`/`ino`/`kind` are the object; the rest are change indicators.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileIdentity {
    pub dev: u64,
    pub ino: u64,
    pub kind: EntryKind,
    pub size: u64,
    pub mtime: i64,
    pub mtime_nsec: i64,
    pub ctime: i64,
    pub ctime_nsec: i64,
}

impl FileIdentity {
    /// The same filesystem object. A rename (into the Trash, for example) changes
    /// ctime, so only these fields survive the move.
    pub fn same_object(&self, other: &Self) -> bool {
        self.dev == other.dev && self.ino == other.ino && self.kind == other.kind
    }

    /// The same object after a rename: inode, type and the content indicators
    /// that a rename preserves. This also catches an inode reused by a new file.
    pub fn same_after_rename(&self, other: &Self) -> bool {
        self.same_object(other)
            && self.size == other.size
            && self.mtime == other.mtime
            && self.mtime_nsec == other.mtime_nsec
    }
}

fn identity(path: &Path) -> io::Result<FileIdentity> {
    use std::os::unix::fs::MetadataExt;
    let meta = path.symlink_metadata()?;
    let file_type = meta.file_type();
    let kind = if file_type.is_symlink() {
        EntryKind::Symlink
    } else if file_type.is_dir() {
        EntryKind::Dir
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    Ok(FileIdentity {
        dev: meta.dev(),
        ino: meta.ino(),
        kind,
        size: meta.len(),
        mtime: meta.mtime(),
        mtime_nsec: meta.mtime_nsec(),
        ctime: meta.ctime(),
        ctime_nsec: meta.ctime_nsec(),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedItem {
    pub source: Option<PathBuf>,
    pub target: PathBuf,
    /// The target name is already taken by a different item.
    pub conflict: bool,
    /// Bytes to copy or move, for progress.
    pub bytes: u64,
    /// The source at planning time, for a recheck before commit.
    pub source_identity: Option<FileIdentity>,
    /// The conflicting target at planning time. A different object at execution
    /// means the user's plan-time choice no longer applies.
    pub target_identity: Option<FileIdentity>,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub request: Request,
    pub items: Vec<PlannedItem>,
    /// For progress and the activity log: "Copy 3 items to Downloads".
    pub label: String,
    pub total_bytes: u64,
}

impl Plan {
    pub fn conflicts(&self) -> impl Iterator<Item = &PlannedItem> {
        self.items.iter().filter(|i| i.conflict)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub done_items: usize,
    pub total_items: usize,
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub current: PathBuf,
}

/// One change that happened on disk, with enough detail to reverse it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// A new item (folder, copy or duplicate).
    Created(PathBuf),
    Moved {
        from: PathBuf,
        to: PathBuf,
    },
    Trashed {
        original: PathBuf,
        trashed: PathBuf,
    },
    /// Permanently deleted; cannot be undone.
    Deleted(PathBuf),
    /// A batch rename committed as a group. Undo stages the inverse graph.
    BatchRenamed {
        moves: Vec<BatchRenameEntry>,
    },
}

/// What an operation did, for the activity log and undo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub label: String,
    pub steps: Vec<Step>,
}

impl Record {
    pub fn is_undoable(&self) -> bool {
        !self.steps.is_empty() && !self.steps.iter().any(|s| matches!(s, Step::Deleted(_)))
    }

    /// The items the operation produced (to select them afterwards).
    pub fn results(&self) -> Vec<PathBuf> {
        let mut results = Vec::new();
        for step in &self.steps {
            match step {
                Step::Created(path) | Step::Moved { to: path, .. } => results.push(path.clone()),
                Step::BatchRenamed { moves } => results.extend(moves.iter().map(|entry| entry.target.clone())),
                _ => {}
            }
        }
        results
    }

    /// Every old path → new path this record's moves and renames applied, in order.
    fn moves(&self) -> impl Iterator<Item = (&Path, &Path)> {
        self.steps.iter().flat_map(|step| -> Box<dyn Iterator<Item = (&Path, &Path)> + '_> {
            match step {
                Step::Moved { from, to } => Box::new(std::iter::once((from.as_path(), to.as_path()))),
                Step::BatchRenamed { moves } => {
                    Box::new(moves.iter().map(|entry| (entry.source.as_path(), entry.target.as_path())))
                }
                _ => Box::new(std::iter::empty()),
            }
        })
    }

    /// This record with the paths of the items it left behind carried through the
    /// moves and renames of `later` records, oldest first, so undoing an older
    /// entry ("New File", then "Rename") finds the item where it is now. Items a
    /// later record trashed are not followed into the Trash.
    pub fn forwarded<'a>(&self, later: impl IntoIterator<Item = &'a Record> + Clone) -> Record {
        let forward = |path: &Path| -> PathBuf {
            let mut path = path.to_path_buf();
            for record in later.clone() {
                for (from, to) in record.moves() {
                    if let Ok(rest) = path.strip_prefix(from) {
                        path = if rest.as_os_str().is_empty() { to.to_path_buf() } else { to.join(rest) };
                    }
                }
            }
            path
        };
        let steps = self
            .steps
            .iter()
            .map(|step| match step {
                Step::Created(path) => Step::Created(forward(path)),
                Step::Moved { from, to } => Step::Moved { from: from.clone(), to: forward(to) },
                Step::BatchRenamed { moves } => Step::BatchRenamed {
                    moves: moves
                        .iter()
                        .map(|entry| BatchRenameEntry { target: forward(&entry.target), ..entry.clone() })
                        .collect(),
                },
                other => other.clone(),
            })
            .collect();
        Record { label: self.label.clone(), steps }
    }

    /// Old path → new path for every item a batch rename step in this record moved.
    pub fn rename_map(&self) -> std::collections::HashMap<PathBuf, PathBuf> {
        self.steps
            .iter()
            .filter_map(|step| match step {
                Step::BatchRenamed { moves } => Some(moves),
                _ => None,
            })
            .flatten()
            .filter(|entry| entry.source != entry.target)
            .map(|entry| (entry.source.clone(), entry.target.clone()))
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct Outcome {
    /// Everything that was done, even if the operation stopped early, so it can be undone.
    pub record: Record,
    pub error: Option<OpError>,
    pub cancelled: bool,
}

#[derive(Clone, Debug)]
pub enum OpError {
    InvalidName(String),
    NameTaken(PathBuf),
    IntoItself(PathBuf),
    NotFound(PathBuf),
    /// Undo found something in the way and stopped rather than overwrite it.
    WouldOverwrite(PathBuf),
    NotUndoable,
    NotAFolder(PathBuf),
    Cancelled,
    /// The source changed while it was being read; the target is untouched.
    SourceChanged(PathBuf),
    /// A replacement's old target is in the Trash and could not be put back.
    RestoreBlocked {
        target: PathBuf,
        trashed: PathBuf,
        error: Arc<io::Error>,
    },
    /// A cross-volume move put the destination in place but could not remove the source.
    SourceKept {
        source: PathBuf,
        target: PathBuf,
        error: Arc<io::Error>,
    },
    /// This filesystem has no verified primitive for the operation.
    Unsupported(PathBuf, &'static str),
    /// The approved batch no longer matches the disk; nothing changed.
    BatchChanged(PathBuf),
    /// A batch could not restore everything. These are the exact locations.
    BatchRecovery {
        items: Vec<RecoveryItem>,
    },
    Io(PathBuf, Arc<io::Error>),
}

impl fmt::Display for OpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = |p: &Path| p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into());
        match self {
            OpError::InvalidName(why) => write!(f, "That name can't be used: {why}"),
            OpError::NameTaken(p) => write!(f, "The name “{}” is already taken", name(p)),
            OpError::IntoItself(p) => write!(f, "“{}” can't be moved into itself", name(p)),
            OpError::NotFound(p) => write!(f, "“{}” no longer exists", name(p)),
            OpError::WouldOverwrite(p) => write!(f, "Undo stopped: “{}” is in the way", name(p)),
            OpError::NotUndoable => write!(f, "This can't be undone"),
            OpError::NotAFolder(p) => write!(f, "“{}” is not a folder", name(p)),
            OpError::Cancelled => write!(f, "Cancelled"),
            OpError::SourceChanged(p) => write!(f, "“{}” changed while it was being copied", name(p)),
            OpError::RestoreBlocked { target, trashed, error } => {
                write!(f, "“{}” couldn't be put back from the Trash at “{}”: {error}", name(target), trashed.display())
            }
            OpError::SourceKept { source, target, error } => {
                write!(
                    f,
                    "“{}” was copied to “{}”, but the original couldn't be removed: {error}",
                    name(source),
                    name(target)
                )
            }
            OpError::Unsupported(p, what) => write!(f, "“{}”: {what} isn't supported on this volume", name(p)),
            OpError::BatchChanged(p) => {
                write!(f, "“{}” changed since the preview; nothing was renamed", name(p))
            }
            OpError::BatchRecovery { items } => {
                write!(f, "Batch rename couldn't restore {} item(s). Kept data:", items.len())?;
                for item in items {
                    write!(
                        f,
                        " “{}” is at “{}” (wanted “{}”)",
                        item.original.display(),
                        item.current.display(),
                        item.intended.display()
                    )?;
                }
                Ok(())
            }
            OpError::Io(p, e) => write!(f, "“{}”: {e}", name(p)),
        }
    }
}

impl std::error::Error for OpError {}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> OpError + '_ {
    move |e| OpError::Io(path.to_path_buf(), Arc::new(e))
}

// ----- names -----

/// Split "report.pdf" into ("report", ".pdf"). Folders keep their whole name,
/// except packages ("Tool.app"), like Finder. Shared with the batch-rename
/// transform in `crow-core`, so both preserve exactly the same extension.
pub fn split_name(name: &str, is_dir: bool) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 && (!is_dir || is_package_ext(&name[i + 1..])) => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// `split_name` over raw bytes, so names that are not UTF-8 keep every byte.
/// The split point is an ASCII dot, so both halves are whole names.
fn split_name_bytes(name: &[u8], is_dir: bool) -> (&[u8], &[u8]) {
    match name.iter().rposition(|&b| b == b'.') {
        Some(i) if i > 0 && (!is_dir || is_package_ext_bytes(&name[i + 1..])) => (&name[..i], &name[i..]),
        _ => (name, &[]),
    }
}

/// "stem{middle}ext" in `dir`, where the stem and extension come from `name` byte for byte.
fn suffixed(dir: &Path, name: &OsStr, is_dir: bool, middle: &str) -> PathBuf {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let (stem, ext) = split_name_bytes(name.as_bytes(), is_dir);
    let mut bytes = Vec::with_capacity(name.len() + middle.len());
    bytes.extend_from_slice(stem);
    bytes.extend_from_slice(middle.as_bytes());
    bytes.extend_from_slice(ext);
    dir.join(OsString::from_vec(bytes))
}

fn exists(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

/// The first free name in `dir` from `candidates`.
fn first_free(dir: &Path, candidates: impl Iterator<Item = String>) -> PathBuf {
    candidates.map(|n| dir.join(n)).find(|p| !exists(p)).expect("an unbounded sequence has a free name")
}

/// Finder's "keep both" name: "report 2.pdf", "report 3.pdf", …
pub fn keep_both_name(dir: &Path, name: &str, is_dir: bool) -> PathBuf {
    keep_both_candidates(dir, OsStr::new(name), is_dir)
        .find(|p| !exists(p))
        .expect("an unbounded sequence has a free name")
}

/// "report 2.pdf", "report 3.pdf", … without checking the disk.
fn keep_both_candidates<'a>(dir: &'a Path, name: &'a OsStr, is_dir: bool) -> impl Iterator<Item = PathBuf> + 'a {
    (2..).map(move |n| suffixed(dir, name, is_dir, &format!(" {n}")))
}

/// "report copy.pdf", "report copy 2.pdf", … without checking the disk.
fn copy_candidates<'a>(dir: &'a Path, name: &'a OsStr, is_dir: bool) -> impl Iterator<Item = PathBuf> + 'a {
    std::iter::once(" copy".to_owned())
        .chain((2..).map(|n| format!(" copy {n}")))
        .map(move |middle| suffixed(dir, name, is_dir, &middle))
}

/// Finder's duplicate name: "report copy.pdf", "report copy 2.pdf", …
pub fn copy_name(dir: &Path, name: &str, is_dir: bool) -> PathBuf {
    copy_candidates(dir, OsStr::new(name), is_dir).find(|p| !exists(p)).expect("an unbounded sequence has a free name")
}

/// The name a new folder gets before you rename it.
pub fn untitled_folder(dir: &Path) -> PathBuf {
    first_free(dir, std::iter::once("untitled folder".to_owned()).chain((2..).map(|n| format!("untitled folder {n}"))))
}

/// The name a new file gets before you rename it. No extension: the file is empty,
/// and a made-up type (".txt") would decide for the user what it becomes.
pub fn untitled_file(dir: &Path) -> PathBuf {
    first_free(dir, std::iter::once("untitled".to_owned()).chain((2..).map(|n| format!("untitled {n}"))))
}

/// Why `name` can't be a file name, if it can't.
pub fn check_name(name: &str) -> Result<(), OpError> {
    let why = if name.trim().is_empty() {
        "it is empty"
    } else if name.contains('/') {
        "it contains “/”"
    } else if name.contains('\0') {
        "it contains a NUL character"
    } else if name == "." || name == ".." {
        "it is reserved"
    } else if name.len() > 255 {
        "it is too long"
    } else {
        return Ok(());
    };
    Err(OpError::InvalidName(why.into()))
}

/// The final component with every byte intact, for names built from a source's.
fn file_name_os(path: &Path) -> Result<&OsStr, OpError> {
    path.file_name().ok_or_else(|| OpError::NotFound(path.to_path_buf()))
}

fn file_name(path: &Path) -> Result<String, OpError> {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or_else(|| OpError::NotFound(path.to_path_buf()))
}

/// Whether `a` and `b` are one directory entry spelled two ways (a case-only or
/// normalization-only rename on a volume that folds names). Two hard links to one
/// inode are different entries, so they are not "the same file".
fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let same_inode = match (a.symlink_metadata(), b.symlink_metadata()) {
        (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
        _ => false,
    };
    if !same_inode {
        return false;
    }
    let (Some(parent_a), Some(parent_b)) = (a.parent(), b.parent()) else { return false };
    if !same_directory(parent_a, parent_b) {
        return false;
    }
    let (Some(name_a), Some(name_b)) = (a.file_name(), b.file_name()) else { return false };
    if name_a == name_b {
        return true;
    }
    match (name_a.to_str(), name_b.to_str()) {
        (Some(x), Some(y)) => {
            let folds = NameSemantics { case_sensitive: false, normalization_insensitive: true };
            names_equal(x, y, name_semantics(parent_a).unwrap_or(folds))
        }
        _ => false,
    }
}

/// Whether both paths name the same folder, following symlinks. Used to see that
/// "move into this folder" through a symlinked parent is already there.
fn same_directory(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    a == b
        || matches!(
            (a.metadata(), b.metadata()),
            (Ok(x), Ok(y)) if x.dev() == y.dev() && x.ino() == y.ino()
        )
}

/// Finder's duplicate name, also avoiding names this plan already claimed.
fn copy_name_avoiding(dir: &Path, name: &OsStr, is_dir: bool, reserved: &HashMap<PathBuf, ()>) -> PathBuf {
    copy_candidates(dir, name, is_dir)
        .find(|p| !exists(p) && !reserved.contains_key(p))
        .expect("an unbounded sequence has a free name")
}

/// Bytes under `path` (for progress), not following links. Cancellation is
/// checked inside the traversal, so planning a huge folder is interruptible.
fn tree_size(path: &Path, cancel: &AtomicBool) -> Result<u64, OpError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(OpError::Cancelled);
    }
    let meta = path.symlink_metadata().map_err(io_err(path))?;
    if !meta.is_dir() {
        let file_type = meta.file_type();
        if !file_type.is_file() && !file_type.is_symlink() {
            return Err(OpError::Unsupported(path.to_path_buf(), "copying this file type"));
        }
        return Ok(meta.len());
    }
    let mut total = 0;
    // A worklist, not recursion: a tree thousands of folders deep must not
    // overflow the planning thread's stack.
    let mut folders = vec![path.to_path_buf()];
    while let Some(folder) = folders.pop() {
        if cancel.load(Ordering::Relaxed) {
            return Err(OpError::Cancelled);
        }
        for entry in std::fs::read_dir(&folder).map_err(io_err(&folder))? {
            let entry = entry.map_err(io_err(&folder))?;
            let child = entry.path();
            let meta = child.symlink_metadata().map_err(io_err(&child))?;
            if meta.is_dir() {
                folders.push(child);
            } else {
                let file_type = meta.file_type();
                if !file_type.is_file() && !file_type.is_symlink() {
                    return Err(OpError::Unsupported(child, "copying this file type"));
                }
                total += meta.len();
            }
        }
    }
    Ok(total)
}

fn target_identity(path: &Path) -> Result<Option<FileIdentity>, OpError> {
    match identity(path) {
        Ok(id) => Ok(Some(id)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_err(path)(e)),
    }
}

/// Whether `into` is the source or lies inside it. The canonical check closes
/// the symlink-to-parent case; a source symlink is copied as a link, so it
/// cannot contain the destination.
fn into_itself(source: &Path, into: &Path) -> bool {
    if into.starts_with(source) {
        return true;
    }
    let Ok(meta) = source.symlink_metadata() else { return false };
    if !meta.is_dir() {
        return false;
    }
    match (std::fs::canonicalize(source), std::fs::canonicalize(into)) {
        (Ok(source), Ok(into)) => into.starts_with(source),
        _ => false,
    }
}

// ----- planning -----

/// Work out what `request` would do, without changing anything.
pub fn plan(request: Request) -> Result<Plan, OpError> {
    static NEVER: AtomicBool = AtomicBool::new(false);
    plan_cancellable(request, &NEVER)
}

/// [`plan`], with a cancellation check between entries and inside tree measurement.
pub fn plan_cancellable(request: Request, cancel: &AtomicBool) -> Result<Plan, OpError> {
    let count = |n: usize| if n == 1 { "1 item".to_owned() } else { format!("{n} items") };
    let dir_name = |p: &Path| p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into());
    let mut reserved: HashMap<PathBuf, ()> = HashMap::new();
    let (items, label) = match &request {
        Request::NewFolder { parent } => (
            vec![PlannedItem {
                source: None,
                target: untitled_folder(parent),
                conflict: false,
                bytes: 0,
                source_identity: None,
                target_identity: None,
            }],
            "New Folder".into(),
        ),
        Request::NewFile { parent } => (
            vec![PlannedItem {
                source: None,
                target: untitled_file(parent),
                conflict: false,
                bytes: 0,
                source_identity: None,
                target_identity: None,
            }],
            "New File".into(),
        ),
        Request::Rename { path, new_name } => {
            check_name(new_name)?;
            if !exists(path) {
                return Err(OpError::NotFound(path.clone()));
            }
            let target = path.with_file_name(new_name);
            // A case-only rename on a case-insensitive volume finds "itself" at the target.
            if exists(&target) && !same_file(path, &target) {
                return Err(OpError::NameTaken(target));
            }
            let label = format!("Rename “{}” to “{new_name}”", file_name(path)?);
            (
                vec![PlannedItem {
                    source: Some(path.clone()),
                    target,
                    conflict: false,
                    bytes: 0,
                    source_identity: None,
                    target_identity: None,
                }],
                label,
            )
        }
        Request::BatchRename { moves } => {
            let items = plan_approved_moves(moves)?;
            let count_text = count(items.len());
            (items, format!("Rename {count_text}"))
        }
        Request::Duplicate { paths } => {
            let mut items = Vec::new();
            for path in paths.iter() {
                if cancel.load(Ordering::Relaxed) {
                    return Err(OpError::Cancelled);
                }
                let source = identity(path).map_err(io_err(path))?;
                let parent = path.parent().ok_or_else(|| OpError::NotFound(path.clone()))?;
                let is_dir = source.kind == EntryKind::Dir;
                let target = copy_name_avoiding(parent, file_name_os(path)?, is_dir, &reserved);
                reserved.insert(target.clone(), ());
                items.push(PlannedItem {
                    source: Some(path.clone()),
                    target,
                    conflict: false,
                    bytes: tree_size(path, cancel)?,
                    source_identity: Some(source),
                    target_identity: None,
                });
            }
            (items, format!("Duplicate {}", count(paths.len())))
        }
        Request::Copy { paths, into } | Request::Move { paths, into } => {
            let moving = matches!(request, Request::Move { .. });
            match std::fs::metadata(into) {
                Ok(meta) if meta.is_dir() => {}
                Ok(_) => return Err(OpError::NotAFolder(into.clone())),
                Err(e) => return Err(io_err(into)(e)),
            }
            let mut items = Vec::new();
            for source in paths.iter() {
                if cancel.load(Ordering::Relaxed) {
                    return Err(OpError::Cancelled);
                }
                let source_meta = identity(source).map_err(io_err(source))?;
                if into_itself(source, into) {
                    return Err(OpError::IntoItself(source.clone()));
                }
                let name = file_name_os(source)?;
                let target = into.join(name);
                // A symlinked parent names the same folder; moving there is a no-op.
                let same_parent = source.parent().is_some_and(|parent| same_directory(parent, into));
                if moving && same_parent {
                    continue;
                }
                // Copying into its own folder makes a copy, as Duplicate does.
                let target = if target == *source || same_parent {
                    copy_name_avoiding(into, name, source_meta.kind == EntryKind::Dir, &reserved)
                } else {
                    target
                };
                let target_identity = target_identity(&target)?;
                let conflict = target_identity.is_some() || reserved.contains_key(&target);
                reserved.insert(target.clone(), ());
                items.push(PlannedItem {
                    source: Some(source.clone()),
                    conflict,
                    target,
                    bytes: tree_size(source, cancel)?,
                    source_identity: Some(source_meta),
                    target_identity,
                });
            }
            let verb = if moving { "Move" } else { "Copy" };
            (items, format!("{verb} {} to {}", count(paths.len()), dir_name(into)))
        }
        Request::Trash { paths } => (
            paths
                .iter()
                .map(|p| PlannedItem {
                    source: Some(p.clone()),
                    target: p.clone(),
                    conflict: false,
                    bytes: 0,
                    source_identity: None,
                    target_identity: None,
                })
                .collect(),
            format!("Move {} to Trash", count(paths.len())),
        ),
        Request::DeletePermanently { paths } => (
            paths
                .iter()
                .map(|p| PlannedItem {
                    source: Some(p.clone()),
                    target: p.clone(),
                    conflict: false,
                    bytes: 0,
                    source_identity: None,
                    target_identity: None,
                })
                .collect(),
            format!("Delete {} permanently", count(paths.len())),
        ),
    };
    let total_bytes = items.iter().map(|i| i.bytes).sum();
    Ok(Plan { request, items, label, total_bytes })
}

// ----- low-level moves and copies -----

/// Rename without ever replacing: fails with `AlreadyExists` if `to` exists,
/// and with `CrossesDevices` when `to` is on another volume.
pub fn rename_no_clobber(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let (f, t) = (CString::new(from.as_os_str().as_bytes())?, CString::new(to.as_os_str().as_bytes())?);
        // SAFETY: both are valid NUL-terminated paths for the duration of the call.
        if unsafe { libc::renamex_np(f.as_ptr(), t.as_ptr(), libc::RENAME_EXCL) } == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    #[cfg(target_os = "linux")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let (f, t) = (CString::new(from.as_os_str().as_bytes())?, CString::new(to.as_os_str().as_bytes())?);
        // SAFETY: both are valid NUL-terminated paths for the duration of the call.
        if unsafe { libc::renameat2(libc::AT_FDCWD, f.as_ptr(), libc::AT_FDCWD, t.as_ptr(), libc::RENAME_NOREPLACE) }
            == 0
        {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (from, to);
        Err(io::Error::new(io::ErrorKind::Unsupported, "no verified no-clobber rename on this platform"))
    }
}

/// Remove a file, symlink or tree. Missing is success.
pub(crate) fn remove_tree(path: &Path) -> io::Result<()> {
    match path.symlink_metadata() {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// crow-fs has no logging dependency; failures to clean up are rare enough for stderr.
pub(crate) fn warn(message: &str) {
    eprintln!("crow-fs: {message}");
}

// ----- undo -----

/// Reverse `record`, newest step first. Returns the record of the reversal (undoing
/// that redoes the original). Stops at the first step it can't reverse safely;
/// the steps reversed before that stay reversed.
pub fn undo(record: &Record, trash: &dyn Trash) -> Result<Record, OpError> {
    if !record.is_undoable() {
        return Err(OpError::NotUndoable);
    }
    let never = AtomicBool::new(false);
    let mut reversed = Record { label: format!("Undo {}", record.label), steps: Vec::new() };
    let mut missing = None;
    for step in record.steps.iter().rev() {
        match step {
            Step::Created(path) => {
                // Undoing a creation moves it to the Trash rather than deleting it.
                // An item already gone is skipped, but an undo that changes nothing fails.
                if !exists(path) {
                    missing.get_or_insert_with(|| path.clone());
                    continue;
                }
                let trashed = trash.trash(path).map_err(io_err(path))?;
                reversed.steps.push(Step::Trashed { original: path.clone(), trashed });
            }
            Step::Moved { from, to } => {
                if exists(from) && !same_file(from, to) {
                    return Err(OpError::WouldOverwrite(from.clone()));
                }
                if same_file(from, to) {
                    std::fs::rename(to, from).map_err(io_err(to))?;
                } else {
                    job::move_item(to, from, &never, &mut |_| {})?;
                }
                reversed.steps.push(Step::Moved { from: to.clone(), to: from.clone() });
            }
            Step::BatchRenamed { moves } => {
                let inverse: Vec<BatchRenameEntry> = moves
                    .iter()
                    .map(|entry| BatchRenameEntry {
                        source: entry.target.clone(),
                        target: entry.source.clone(),
                        identity: entry.identity.clone(),
                    })
                    .collect();
                let undone = execute_batch(&inverse, &|| false, &mut |_| {}, &BatchHooks::default())?;
                reversed.steps.push(Step::BatchRenamed { moves: undone });
            }
            Step::Trashed { original, trashed } => {
                if exists(original) {
                    return Err(OpError::WouldOverwrite(original.clone()));
                }
                job::move_item(trashed, original, &never, &mut |_| {})?;
                reversed.steps.push(Step::Moved { from: trashed.clone(), to: original.clone() });
            }
            Step::Deleted(path) => return Err(OpError::NotFound(path.clone())),
        }
    }
    if reversed.steps.is_empty()
        && let Some(path) = missing
    {
        return Err(OpError::NotFound(path));
    }
    Ok(reversed)
}

#[cfg(test)]
mod name_tests {
    use super::*;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    fn bytes_path(dir: &Path, name: &[u8]) -> PathBuf {
        dir.join(OsString::from_vec(name.to_vec()))
    }

    #[test]
    fn generated_names_keep_the_bytes_of_non_utf8_names() {
        let dir = Path::new("/crow-no-such-folder");
        let name = OsStr::from_bytes(b"foo\xfe.txt");
        let reserved = HashMap::new();
        assert_eq!(copy_name_avoiding(dir, name, false, &reserved), bytes_path(dir, b"foo\xfe copy.txt"));
        let other = copy_name_avoiding(dir, OsStr::from_bytes(b"foo\xff.txt"), false, &reserved);
        assert_ne!(copy_name_avoiding(dir, name, false, &reserved), other, "two bad names stay two names");
        assert_eq!(keep_both_candidates(dir, name, false).next().unwrap(), bytes_path(dir, b"foo\xfe 2.txt"));
        assert_eq!(suffixed(dir, OsStr::from_bytes(b"T\xfe.app"), true, " copy"), bytes_path(dir, b"T\xfe copy.app"));
    }

    #[test]
    fn pkg_is_a_package_everywhere() {
        assert_eq!(split_name("Setup.pkg", true), ("Setup", ".pkg"));
        assert!(crate::listing::is_package_name("Setup.pkg"));
        assert!(crate::listing::is_package_name("Tool.APP"));
        assert!(!crate::listing::is_package_name("v1.2"));
    }
}
