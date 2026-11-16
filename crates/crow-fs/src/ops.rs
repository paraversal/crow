//! The file operation engine: every change Crow makes to files goes through here.
//!
//! Three stages, so nothing touches the disk before the user has seen what will happen:
//! 1. [`plan`] turns a request into items and marks conflicts. Read-only.
//! 2. [`execute`] carries out the plan with the user's conflict choices, reporting
//!    progress and stopping cleanly when cancelled.
//! 3. [`undo`] reverses a finished (or partly finished) operation from its [`Record`].
//!
//! Safety rules, enforced here rather than trusted to callers:
//! - Nothing is ever overwritten in place. Copies are written under a temporary
//!   name and renamed into place with `RENAME_EXCL`, which fails rather than replace.
//! - "Replace" first moves the existing item to the Trash, so it can be undone.
//! - A cancelled or failed copy leaves no partial item behind; the source is untouched.
//! - A move across volumes deletes the source only after the copy is complete.
//! - Undo refuses to overwrite anything that appeared in the meantime.

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedItem {
    pub source: Option<PathBuf>,
    pub target: PathBuf,
    /// The target name is already taken by a different item.
    pub conflict: bool,
    /// Bytes to copy or move, for progress.
    pub bytes: u64,
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
        self.steps
            .iter()
            .filter_map(|s| match s {
                Step::Created(p) | Step::Moved { to: p, .. } => Some(p.clone()),
                _ => None,
            })
            .collect()
    }
}

pub struct Outcome {
    /// Everything that was done, even if the operation stopped early, so it can be undone.
    pub record: Record,
    pub error: Option<OpError>,
    pub cancelled: bool,
}

#[derive(Debug)]
pub enum OpError {
    InvalidName(String),
    NameTaken(PathBuf),
    IntoItself(PathBuf),
    NotFound(PathBuf),
    /// Undo found something in the way and stopped rather than overwrite it.
    WouldOverwrite(PathBuf),
    NotUndoable,
    Io(PathBuf, io::Error),
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
            OpError::Io(p, e) => write!(f, "“{}”: {e}", name(p)),
        }
    }
}

impl std::error::Error for OpError {}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> OpError + '_ {
    move |e| OpError::Io(path.to_path_buf(), e)
}

// ----- names -----

const PACKAGE_EXTS: [&str; 5] = ["app", "bundle", "framework", "photoslibrary", "pkg"];

/// Split "report.pdf" into ("report", ".pdf"). Folders keep their whole name,
/// except packages ("Tool.app"), like Finder.
fn split_name(name: &str, is_dir: bool) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 && (!is_dir || PACKAGE_EXTS.contains(&&name[i + 1..])) => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
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
    let (stem, ext) = split_name(name, is_dir);
    first_free(dir, (2..).map(|n| format!("{stem} {n}{ext}")))
}

/// Finder's duplicate name: "report copy.pdf", "report copy 2.pdf", …
pub fn copy_name(dir: &Path, name: &str, is_dir: bool) -> PathBuf {
    let (stem, ext) = split_name(name, is_dir);
    first_free(dir, std::iter::once(format!("{stem} copy{ext}")).chain((2..).map(|n| format!("{stem} copy {n}{ext}"))))
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
    } else if name == "." || name == ".." {
        "it is reserved"
    } else if name.len() > 255 {
        "it is too long"
    } else {
        return Ok(());
    };
    Err(OpError::InvalidName(why.into()))
}

fn file_name(path: &Path) -> Result<String, OpError> {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or_else(|| OpError::NotFound(path.to_path_buf()))
}

fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (a.symlink_metadata(), b.symlink_metadata()) {
        (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
        _ => false,
    }
}

/// Bytes under `path` (for progress), not following links.
fn tree_size(path: &Path) -> u64 {
    let Ok(meta) = path.symlink_metadata() else { return 0 };
    if !meta.is_dir() {
        return meta.len();
    }
    std::fs::read_dir(path).map_or(0, |d| d.filter_map(Result::ok).map(|e| tree_size(&e.path())).sum())
}

// ----- planning -----

/// Work out what `request` would do, without changing anything.
pub fn plan(request: Request) -> Result<Plan, OpError> {
    let count = |n: usize| if n == 1 { "1 item".to_owned() } else { format!("{n} items") };
    let dir_name = |p: &Path| p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into());
    let (items, label) = match &request {
        Request::NewFolder { parent } => (
            vec![PlannedItem { source: None, target: untitled_folder(parent), conflict: false, bytes: 0 }],
            "New Folder".into(),
        ),
        Request::NewFile { parent } => (
            vec![PlannedItem { source: None, target: untitled_file(parent), conflict: false, bytes: 0 }],
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
            (vec![PlannedItem { source: Some(path.clone()), target, conflict: false, bytes: 0 }], label)
        }
        Request::Duplicate { paths } => {
            let items = paths
                .iter()
                .map(|p| {
                    let parent = p.parent().ok_or_else(|| OpError::NotFound(p.clone()))?;
                    Ok(PlannedItem {
                        source: Some(p.clone()),
                        target: copy_name(parent, &file_name(p)?, p.is_dir()),
                        conflict: false,
                        bytes: tree_size(p),
                    })
                })
                .collect::<Result<Vec<_>, OpError>>()?;
            (items, format!("Duplicate {}", count(paths.len())))
        }
        Request::Copy { paths, into } | Request::Move { paths, into } => {
            let moving = matches!(request, Request::Move { .. });
            let mut items = Vec::new();
            for source in paths.iter() {
                if !exists(source) {
                    return Err(OpError::NotFound(source.clone()));
                }
                if into.starts_with(source) {
                    return Err(OpError::IntoItself(source.clone()));
                }
                let name = file_name(source)?;
                let target = into.join(&name);
                if moving && source.parent() == Some(into.as_path()) {
                    continue; // already there
                }
                // Copying into its own folder makes a copy, as Duplicate does.
                let target = if target == *source { copy_name(into, &name, source.is_dir()) } else { target };
                items.push(PlannedItem {
                    source: Some(source.clone()),
                    conflict: exists(&target),
                    target,
                    bytes: tree_size(source),
                });
            }
            let verb = if moving { "Move" } else { "Copy" };
            (items, format!("{verb} {} to {}", count(paths.len()), dir_name(into)))
        }
        Request::Trash { paths } => (
            paths
                .iter()
                .map(|p| PlannedItem { source: Some(p.clone()), target: p.clone(), conflict: false, bytes: 0 })
                .collect(),
            format!("Move {} to Trash", count(paths.len())),
        ),
        Request::DeletePermanently { paths } => (
            paths
                .iter()
                .map(|p| PlannedItem { source: Some(p.clone()), target: p.clone(), conflict: false, bytes: 0 })
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
    #[cfg(not(target_os = "macos"))]
    {
        if exists(to) {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        std::fs::rename(from, to)
    }
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_beside(target: &Path) -> PathBuf {
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    target.with_file_name(format!(".crow-partial-{}-{n}", std::process::id()))
}

fn remove_tree(path: &Path) -> io::Result<()> {
    match path.symlink_metadata() {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Copy `source` to `target` (which must be free): write under a temporary name,
/// then rename into place. On error or cancel the temporary copy is removed.
/// `on_bytes` gets the bytes copied so far within this item.
fn copy_into_place(
    source: &Path,
    target: &Path,
    cancel: &AtomicBool,
    on_bytes: &mut dyn FnMut(u64),
) -> Result<(), OpError> {
    let temp = temp_beside(target);
    let copied = copy_tree(source, &temp, cancel, on_bytes);
    let placed = copied.and_then(|()| rename_no_clobber(&temp, target).map_err(io_err(target)));
    if placed.is_err()
        && let Err(e) = remove_tree(&temp)
    {
        warn(&format!("could not remove partial copy {}: {e}", temp.display()));
    }
    placed
}

/// crow-fs has no logging dependency; failures to clean up are rare enough for stderr.
fn warn(message: &str) {
    eprintln!("crow-fs: {message}");
}

#[cfg(target_os = "macos")]
fn copy_tree(source: &Path, dest: &Path, cancel: &AtomicBool, on_bytes: &mut dyn FnMut(u64)) -> Result<(), OpError> {
    use std::ffi::{CString, c_char, c_int, c_void};
    use std::os::unix::ffi::OsStrExt;

    struct Context<'a> {
        cancel: &'a AtomicBool,
        on_bytes: &'a mut dyn FnMut(u64),
        finished_files: u64,
    }

    extern "C" fn callback(
        what: c_int,
        stage: c_int,
        state: libc::copyfile_state_t,
        _src: *const c_char,
        _dst: *const c_char,
        ctx: *mut c_void,
    ) -> c_int {
        // SAFETY: ctx is the &mut Context passed below, alive for the whole copyfile call.
        let ctx = unsafe { &mut *ctx.cast::<Context>() };
        if ctx.cancel.load(Ordering::Relaxed) {
            return libc::COPYFILE_QUIT;
        }
        if what == libc::COPYFILE_COPY_DATA && stage == libc::COPYFILE_PROGRESS {
            let mut copied: libc::off_t = 0;
            // SAFETY: state is valid during the callback; COPIED writes one off_t.
            unsafe { libc::copyfile_state_get(state, libc::COPYFILE_STATE_COPIED as u32, (&raw mut copied).cast()) };
            (ctx.on_bytes)(ctx.finished_files + copied.max(0) as u64);
        }
        if what == libc::COPYFILE_RECURSE_FILE && stage == libc::COPYFILE_FINISH {
            let mut copied: libc::off_t = 0;
            // SAFETY: as above.
            unsafe { libc::copyfile_state_get(state, libc::COPYFILE_STATE_COPIED as u32, (&raw mut copied).cast()) };
            ctx.finished_files += copied.max(0) as u64;
            (ctx.on_bytes)(ctx.finished_files);
        }
        libc::COPYFILE_CONTINUE
    }

    let (src, dst) = (
        CString::new(source.as_os_str().as_bytes()).map_err(|e| OpError::Io(source.to_path_buf(), e.into()))?,
        CString::new(dest.as_os_str().as_bytes()).map_err(|e| OpError::Io(dest.to_path_buf(), e.into()))?,
    );
    let mut ctx = Context { cancel, on_bytes, finished_files: 0 };
    let flags = libc::COPYFILE_METADATA
        | libc::COPYFILE_DATA
        | libc::COPYFILE_RECURSIVE
        | libc::COPYFILE_CLONE
        | libc::COPYFILE_NOFOLLOW
        | libc::COPYFILE_EXCL;
    // SAFETY: the state is freed below; callback and context outlive the copyfile call.
    let rc = unsafe {
        let state = libc::copyfile_state_alloc();
        libc::copyfile_state_set(state, libc::COPYFILE_STATE_STATUS_CB as u32, callback as *const c_void);
        libc::copyfile_state_set(state, libc::COPYFILE_STATE_STATUS_CTX as u32, (&raw mut ctx).cast());
        let rc = libc::copyfile(src.as_ptr(), dst.as_ptr(), state, flags);
        libc::copyfile_state_free(state);
        rc
    };
    if cancel.load(Ordering::Relaxed) {
        return Err(OpError::Io(source.to_path_buf(), io::Error::from(io::ErrorKind::Interrupted)));
    }
    if rc == 0 { Ok(()) } else { Err(OpError::Io(source.to_path_buf(), io::Error::last_os_error())) }
}

#[cfg(not(target_os = "macos"))]
fn copy_tree(source: &Path, dest: &Path, cancel: &AtomicBool, on_bytes: &mut dyn FnMut(u64)) -> Result<(), OpError> {
    let meta = source.symlink_metadata().map_err(io_err(source))?;
    if cancel.load(Ordering::Relaxed) {
        return Err(OpError::Io(source.to_path_buf(), io::Error::from(io::ErrorKind::Interrupted)));
    }
    if meta.is_dir() {
        std::fs::create_dir(dest).map_err(io_err(dest))?;
        for entry in std::fs::read_dir(source).map_err(io_err(source))?.filter_map(Result::ok) {
            copy_tree(&entry.path(), &dest.join(entry.file_name()), cancel, on_bytes)?;
        }
        Ok(())
    } else {
        let n = std::fs::copy(source, dest).map_err(io_err(source))?;
        on_bytes(n);
        Ok(())
    }
}

/// Tests flip this to take the across-volumes path without a second volume.
#[cfg(test)]
pub(crate) static FORCE_CROSS_DEVICE: AtomicBool = AtomicBool::new(false);

/// Move without replacing anything; across volumes, copy then delete the source.
fn move_item(from: &Path, to: &Path, cancel: &AtomicBool, on_bytes: &mut dyn FnMut(u64)) -> Result<(), OpError> {
    #[cfg(test)]
    let renamed = if FORCE_CROSS_DEVICE.load(Ordering::Relaxed) {
        Err(io::Error::from(io::ErrorKind::CrossesDevices))
    } else {
        rename_no_clobber(from, to)
    };
    #[cfg(not(test))]
    let renamed = rename_no_clobber(from, to);
    match renamed {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
            copy_into_place(from, to, cancel, on_bytes)?;
            // The copy is complete and in place; only now is the source removed.
            remove_tree(from).map_err(io_err(from))
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(OpError::NameTaken(to.to_path_buf())),
        Err(e) => Err(OpError::Io(from.to_path_buf(), e)),
    }
}

// ----- execution -----

/// Carry out `plan`. `resolutions` maps a conflicting target to the user's choice
/// (missing entries keep both). Stops at the first error or when `cancel` is set;
/// the returned record covers everything that was done either way.
pub fn execute(
    plan: &Plan,
    resolutions: &HashMap<PathBuf, Resolution>,
    trash: &dyn Trash,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(&Progress),
) -> Outcome {
    let mut record = Record { label: plan.label.clone(), steps: Vec::new() };
    let mut p = Progress { total_items: plan.items.len(), total_bytes: plan.total_bytes, ..Progress::default() };
    for item in &plan.items {
        if cancel.load(Ordering::Relaxed) {
            return Outcome { record, error: None, cancelled: true };
        }
        p.current = item.source.clone().unwrap_or_else(|| item.target.clone());
        progress(&p);
        let before = p.done_bytes;
        let result = {
            let mut on_bytes = |n: u64| {
                p.done_bytes = before + n.min(item.bytes);
                progress(&p);
            };
            run_item(&plan.request, item, resolutions, trash, cancel, &mut on_bytes, &mut record.steps)
        };
        p.done_bytes = before + item.bytes;
        match result {
            Ok(()) => {
                p.done_items += 1;
                progress(&p);
            }
            Err(e) => {
                let cancelled = cancel.load(Ordering::Relaxed);
                return Outcome { record, error: (!cancelled).then_some(e), cancelled };
            }
        }
    }
    Outcome { record, error: None, cancelled: false }
}

fn run_item(
    request: &Request,
    item: &PlannedItem,
    resolutions: &HashMap<PathBuf, Resolution>,
    trash: &dyn Trash,
    cancel: &AtomicBool,
    on_bytes: &mut dyn FnMut(u64),
    steps: &mut Vec<Step>,
) -> Result<(), OpError> {
    let target = &item.target;
    match request {
        Request::NewFolder { .. } => {
            std::fs::create_dir(target).map_err(io_err(target))?;
            steps.push(Step::Created(target.clone()));
        }
        Request::NewFile { .. } => {
            // `create_new` never replaces a file that appeared since planning.
            std::fs::OpenOptions::new().write(true).create_new(true).open(target).map_err(io_err(target))?;
            steps.push(Step::Created(target.clone()));
        }
        Request::Rename { path, .. } => {
            if same_file(path, target) {
                // Case-only rename: the "existing" target is the item itself.
                std::fs::rename(path, target).map_err(io_err(path))?;
            } else {
                rename_no_clobber(path, target).map_err(|e| match e.kind() {
                    io::ErrorKind::AlreadyExists => OpError::NameTaken(target.clone()),
                    _ => OpError::Io(path.clone(), e),
                })?;
            }
            steps.push(Step::Moved { from: path.clone(), to: target.clone() });
        }
        Request::Duplicate { .. } | Request::Copy { .. } | Request::Move { .. } => {
            let source = item.source.as_ref().ok_or_else(|| OpError::NotFound(target.clone()))?;
            if !exists(source) {
                return Err(OpError::NotFound(source.clone()));
            }
            let mut target = target.clone();
            if exists(&target) {
                match resolutions.get(&target).copied().unwrap_or_default() {
                    Resolution::Skip => return Ok(()),
                    Resolution::KeepBoth => {
                        let parent = target.parent().ok_or_else(|| OpError::NotFound(target.clone()))?;
                        target = keep_both_name(parent, &file_name(&target)?, source.is_dir());
                    }
                    Resolution::Replace => {
                        let trashed = trash.trash(&target).map_err(io_err(&target))?;
                        steps.push(Step::Trashed { original: target.clone(), trashed });
                    }
                }
            }
            if matches!(request, Request::Move { .. }) {
                move_item(source, &target, cancel, on_bytes)?;
                steps.push(Step::Moved { from: source.clone(), to: target });
            } else {
                copy_into_place(source, &target, cancel, on_bytes)?;
                steps.push(Step::Created(target));
            }
        }
        Request::Trash { .. } => {
            if !exists(target) {
                return Err(OpError::NotFound(target.clone()));
            }
            let trashed = trash.trash(target).map_err(io_err(target))?;
            steps.push(Step::Trashed { original: target.clone(), trashed });
        }
        Request::DeletePermanently { .. } => {
            remove_tree(target).map_err(io_err(target))?;
            steps.push(Step::Deleted(target.clone()));
        }
    }
    Ok(())
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
    for step in record.steps.iter().rev() {
        match step {
            Step::Created(path) => {
                // Undoing a creation moves it to the Trash rather than deleting it.
                if !exists(path) {
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
                    move_item(to, from, &never, &mut |_| {})?;
                }
                reversed.steps.push(Step::Moved { from: to.clone(), to: from.clone() });
            }
            Step::Trashed { original, trashed } => {
                if exists(original) {
                    return Err(OpError::WouldOverwrite(original.clone()));
                }
                move_item(trashed, original, &never, &mut |_| {})?;
                reversed.steps.push(Step::Moved { from: trashed.clone(), to: original.clone() });
            }
            Step::Deleted(path) => return Err(OpError::NotFound(path.clone())),
        }
    }
    Ok(reversed)
}
