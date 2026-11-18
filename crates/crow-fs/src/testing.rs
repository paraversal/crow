//! Edge-case trees and file-system churn for tests, the fuzzer and `--make-fixture`.
//!
//! Enabled by the `testing` feature (and always for this crate's own tests).
//! `EdgeTree::wide` is the bench fixture's writer, so the bench and the fuzzer
//! share one generator: `--make-fixture DIR N` is `EdgeTree::wide(N)`.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

/// One awkward thing a real folder can contain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeKind {
    /// Two 255-byte names: ASCII, and 3-byte UTF-8 characters.
    LongName,
    /// `café` in NFC and NFD. APFS is normalization-insensitive, so the second
    /// spelling resolves to the first entry; a filesystem that keeps both gets two.
    NfcNfdTwins,
    /// Emoji, including a ZWJ family and a flag.
    Emoji,
    /// Hebrew, Arabic and a bidi override.
    Rtl,
    /// A dot-file and a dot-folder.
    LeadingDot,
    /// Leading, trailing and only spaces.
    Spaces,
    /// A name containing `\n`.
    Newline,
    /// `n` nested folders.
    Deep(usize),
    /// `n` flat files: the bench fixture's shape.
    Wide(usize),
    /// `a -> b -> a`.
    SymlinkLoop,
    /// A symlink to a missing target.
    DanglingSymlink,
    /// A mode-000 folder and file.
    Unreadable,
    /// An `.app` folder.
    Package,
    /// Two names for one inode.
    Hardlinks,
    /// An empty folder.
    EmptyFolder,
    /// A sparse 10 GiB file.
    HugeFile,
}

impl EdgeKind {
    /// The name used by `CROW_FUZZ_TREE` and the findings table.
    pub fn label(&self) -> String {
        match self {
            EdgeKind::LongName => "long-name".into(),
            EdgeKind::NfcNfdTwins => "nfc-nfd-twins".into(),
            EdgeKind::Emoji => "emoji".into(),
            EdgeKind::Rtl => "rtl".into(),
            EdgeKind::LeadingDot => "leading-dot".into(),
            EdgeKind::Spaces => "spaces".into(),
            EdgeKind::Newline => "newline".into(),
            EdgeKind::Deep(n) => format!("deep-{n}"),
            EdgeKind::Wide(n) => format!("wide-{n}"),
            EdgeKind::SymlinkLoop => "symlink-loop".into(),
            EdgeKind::DanglingSymlink => "dangling-symlink".into(),
            EdgeKind::Unreadable => "unreadable".into(),
            EdgeKind::Package => "package".into(),
            EdgeKind::Hardlinks => "hardlinks".into(),
            EdgeKind::EmptyFolder => "empty-folder".into(),
            EdgeKind::HugeFile => "huge-file".into(),
        }
    }

    /// Parse one label from `EdgeKind::label`.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let kind = match spec {
            "long-name" => EdgeKind::LongName,
            "nfc-nfd-twins" => EdgeKind::NfcNfdTwins,
            "emoji" => EdgeKind::Emoji,
            "rtl" => EdgeKind::Rtl,
            "leading-dot" => EdgeKind::LeadingDot,
            "spaces" => EdgeKind::Spaces,
            "newline" => EdgeKind::Newline,
            "symlink-loop" => EdgeKind::SymlinkLoop,
            "dangling-symlink" => EdgeKind::DanglingSymlink,
            "unreadable" => EdgeKind::Unreadable,
            "package" => EdgeKind::Package,
            "hardlinks" => EdgeKind::Hardlinks,
            "empty-folder" => EdgeKind::EmptyFolder,
            "huge-file" => EdgeKind::HugeFile,
            other => {
                if let Some(n) = other.strip_prefix("deep-") {
                    EdgeKind::Deep(n.parse().map_err(|_| format!("bad depth in {other:?}"))?)
                } else if let Some(n) = other.strip_prefix("wide-") {
                    EdgeKind::Wide(n.parse().map_err(|_| format!("bad count in {other:?}"))?)
                } else {
                    return Err(format!("unknown edge kind {other:?}"));
                }
            }
        };
        Ok(kind)
    }
}

/// A list of edge kinds to build. `parse` reads `small`, `all`, or comma-separated
/// labels (`long-name,deep-20`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EdgeTree {
    kinds: Vec<EdgeKind>,
}

impl EdgeTree {
    pub fn new(kinds: impl IntoIterator<Item = EdgeKind>) -> Self {
        EdgeTree { kinds: kinds.into_iter().collect() }
    }

    pub fn single(kind: EdgeKind) -> Self {
        Self::new([kind])
    }

    /// The bench fixture: `count` flat files.
    pub fn wide(count: usize) -> Self {
        Self::single(EdgeKind::Wide(count))
    }

    /// Every kind except the two expensive ones (Q1): `Wide` and `HugeFile`.
    pub fn small() -> Self {
        Self::new([
            EdgeKind::LongName,
            EdgeKind::NfcNfdTwins,
            EdgeKind::Emoji,
            EdgeKind::Rtl,
            EdgeKind::LeadingDot,
            EdgeKind::Spaces,
            EdgeKind::Newline,
            EdgeKind::Deep(20),
            EdgeKind::SymlinkLoop,
            EdgeKind::DanglingSymlink,
            EdgeKind::Unreadable,
            EdgeKind::Package,
            EdgeKind::Hardlinks,
            EdgeKind::EmptyFolder,
        ])
    }

    /// Every kind, with the expensive ones at their plan sizes.
    pub fn all() -> Self {
        let mut tree = Self::small();
        tree.kinds.push(EdgeKind::Wide(100_000));
        tree.kinds.push(EdgeKind::HugeFile);
        tree
    }

    /// Parse `small`, `all`, `none`, or comma-separated `EdgeKind` labels.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let spec = spec.trim();
        match spec {
            "" | "none" => Ok(Self::default()),
            "small" => Ok(Self::small()),
            "all" => Ok(Self::all()),
            _ => spec.split(',').map(|part| EdgeKind::parse(part.trim())).collect::<Result<Vec<_>, _>>().map(Self::new),
        }
    }

    pub fn kinds(&self) -> &[EdgeKind] {
        &self.kinds
    }

    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }

    /// A short name for run logs and findings rows.
    pub fn label(&self) -> String {
        if self.kinds.is_empty() {
            return "none".into();
        }
        if self.kinds.len() == 1 {
            return self.kinds[0].label();
        }
        if self.kinds == Self::small().kinds {
            return "small".into();
        }
        if self.kinds == Self::all().kinds {
            return "all".into();
        }
        self.kinds.iter().map(EdgeKind::label).collect::<Vec<_>>().join("+")
    }

    /// The parseable form for `CROW_FUZZ_TREE`.
    pub fn spec(&self) -> String {
        if self.kinds.is_empty() {
            return "none".into();
        }
        if self.kinds == Self::small().kinds {
            return "small".into();
        }
        if self.kinds == Self::all().kinds {
            return "all".into();
        }
        self.kinds.iter().map(EdgeKind::label).collect::<Vec<_>>().join(",")
    }
}

/// A tree `make_tree` built. Dropping it deletes nothing: the bench fixture is
/// a tree too. Call `remove` to restore permissions and delete.
#[derive(Debug)]
pub struct Tree {
    root: PathBuf,
    entries: usize,
}

impl Tree {
    /// Direct children in the root, as the disk reported them after creation.
    pub fn entries(&self) -> usize {
        self.entries
    }

    /// Restore permissions and delete the tree.
    pub fn remove(self) -> io::Result<()> {
        remove_tree(&self.root)
    }
}

/// Build `tree` under `root`. `seed` shuffles the creation order, so directory
/// order varies between runs; names and contents stay deterministic (the bench
/// fixture is hashed).
pub fn make_tree(root: &Path, tree: &EdgeTree, seed: u64) -> io::Result<Tree> {
    std::fs::create_dir_all(root)?;
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut kinds = tree.kinds.clone();
    kinds.shuffle(&mut rng);
    for kind in kinds {
        build(root, kind)?;
    }
    let entries = std::fs::read_dir(root)?.filter_map(Result::ok).count();
    Ok(Tree { root: root.to_path_buf(), entries })
}

/// Restore permissions and delete `root`. Mode-000 folders otherwise survive
/// `remove_dir_all` (EACCES), and a fuzzer tree with an `Unreadable` kind would
/// leak into the next run.
pub fn remove_tree(root: &Path) -> io::Result<()> {
    restore_permissions(root);
    match std::fs::remove_dir_all(root) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn build(root: &Path, kind: EdgeKind) -> io::Result<()> {
    match kind {
        EdgeKind::LongName => {
            // 251 + ".txt" = 255 bytes.
            std::fs::write(root.join(format!("{}.txt", "a".repeat(251))), b"long ascii\n")?;
            // 85 x 3 bytes = 255 bytes.
            std::fs::write(root.join("中".repeat(85)), b"long 3-byte\n")?;
        }
        EdgeKind::NfcNfdTwins => {
            create_new(&root.join("caf\u{e9}.txt"), b"nfc\n")?;
            // APFS maps this to the NFC entry; a filesystem that keeps both gets two.
            create_new(&root.join("cafe\u{301}.txt"), b"nfd\n")?;
        }
        EdgeKind::Emoji => {
            std::fs::write(root.join("🙂 party 🎉.txt"), b"emoji\n")?;
            std::fs::write(root.join("👨\u{200d}👩\u{200d}👧\u{200d}👦 family.txt"), b"zwj\n")?;
            std::fs::write(root.join("🇯🇵 flag.txt"), b"flag\n")?;
            std::fs::create_dir_all(root.join("📁 emoji folder"))?;
        }
        EdgeKind::Rtl => {
            std::fs::write(root.join("שלום.txt"), b"hebrew\n")?;
            std::fs::write(root.join("مرحبا.txt"), b"arabic\n")?;
            std::fs::write(root.join("abc\u{202e}gnp.exe"), b"bidi\n")?;
        }
        EdgeKind::LeadingDot => {
            std::fs::write(root.join(".hidden-file"), b"hidden\n")?;
            std::fs::create_dir_all(root.join(".hidden-folder"))?;
            std::fs::write(root.join(".hidden-folder/inside.txt"), b"inside\n")?;
        }
        EdgeKind::Spaces => {
            std::fs::write(root.join(" leading.txt"), b"leading\n")?;
            std::fs::write(root.join("trailing .txt"), b"trailing\n")?;
            std::fs::write(root.join("   "), b"spaces\n")?;
        }
        EdgeKind::Newline => {
            std::fs::write(root.join("line\nbreak.txt"), b"newline\n")?;
        }
        EdgeKind::Deep(n) => {
            let mut path = root.to_path_buf();
            for level in 0..n.max(1) {
                path.push(format!("level-{level:02}"));
            }
            std::fs::create_dir_all(&path)?;
            std::fs::write(path.join("bottom.txt"), b"bottom\n")?;
        }
        EdgeKind::Wide(n) => make_wide(root, n)?,
        EdgeKind::SymlinkLoop => {
            symlink("loop-b", &root.join("loop-a"))?;
            symlink("loop-a", &root.join("loop-b"))?;
        }
        EdgeKind::DanglingSymlink => {
            symlink("missing-target", &root.join("dangling"))?;
        }
        EdgeKind::Unreadable => {
            let dir = root.join("unreadable-dir");
            std::fs::create_dir_all(&dir)?;
            std::fs::write(dir.join("inside.txt"), b"inside\n")?;
            let file = root.join("unreadable-file");
            std::fs::write(&file, b"secret\n")?;
            set_mode(&dir, 0o000)?;
            set_mode(&file, 0o000)?;
        }
        EdgeKind::Package => {
            let app = root.join("CrowTest.app/Contents");
            std::fs::create_dir_all(app.join("MacOS"))?;
            std::fs::write(
                app.join("Info.plist"),
                b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict/></plist>\n",
            )?;
            std::fs::write(app.join("MacOS/CrowTest"), b"#!/bin/sh\n")?;
        }
        EdgeKind::Hardlinks => {
            std::fs::write(root.join("hard-a"), b"hardlink\n")?;
            match std::fs::hard_link(root.join("hard-a"), root.join("hard-b")) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        EdgeKind::EmptyFolder => {
            std::fs::create_dir_all(root.join("empty-folder"))?;
        }
        EdgeKind::HugeFile => {
            let file = std::fs::File::create(root.join("huge.bin"))?;
            file.set_len(10 * 1024 * 1024 * 1024)?;
        }
    }
    Ok(())
}

/// The bench fixture: `count` empty files with the same names and contents as
/// before the `testing` module existed (the fixture hash depends on it).
pub fn make_wide(dir: &Path, count: usize) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let exts = ["txt", "md", "rs", "png", "json", "pdf", "toml", "swift"];
    for i in 0..count {
        std::fs::File::create(dir.join(format!("file-{i:06}.{}", exts[i % exts.len()])))?;
    }
    Ok(())
}

fn create_new(path: &Path, content: &[u8]) -> io::Result<()> {
    match std::fs::OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => file.write_all(content),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn symlink(target: &str, link: &Path) -> io::Result<()> {
    match std::os::unix::fs::symlink(target, link) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(not(unix))]
fn symlink(_: &str, _: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_: &Path, _: u32) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn restore_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(meta) = std::fs::symlink_metadata(path) else { return };
    if meta.file_type().is_symlink() {
        return;
    }
    if meta.is_dir() {
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
        if let Ok(read) = std::fs::read_dir(path) {
            for entry in read.flatten() {
                restore_permissions(&entry.path());
            }
        }
    } else {
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644));
    }
}

#[cfg(not(unix))]
fn restore_permissions(_: &Path) {}

/// A background thread that changes `dir` while a window shows it.
///
/// It creates, renames, deletes, rewrites, chmods and moves files and folders
/// in `dir` and one level down, at random intervals of 1–50 ms. Candidates are
/// read from the current listing, so the item the fuzzer has selected is in the
/// draw. A 100k folder is scanned up to `MAX_CANDIDATES` entries per operation.
pub struct Churn;

impl Churn {
    pub fn start(dir: &Path, seed: u64) -> ChurnHandle {
        let shared = Arc::new(Shared { state: Mutex::new(State::default()), wake: Condvar::new() });
        let operations = Arc::new(AtomicU64::new(0));
        let worker = {
            let dir = dir.to_path_buf();
            let shared = shared.clone();
            let operations = operations.clone();
            std::thread::Builder::new()
                .name("crow-churn".into())
                .spawn(move || run(&dir, seed, &shared, &operations))
                .expect("spawn churn thread")
        };
        ChurnHandle { shared, thread: Some(worker), operations }
    }
}

pub struct ChurnHandle {
    shared: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<()>>,
    operations: Arc<AtomicU64>,
}

impl ChurnHandle {
    /// Stop changing the disk and wait until the worker is parked. After this
    /// returns, the tree is quiet until `resume`.
    pub fn pause(&self) {
        let mut state = self.shared.state.lock().unwrap();
        state.paused = true;
        self.shared.wake.notify_all();
        while !state.idle && !state.stop {
            state = self.shared.wake.wait(state).unwrap();
        }
    }

    pub fn resume(&self) {
        let mut state = self.shared.state.lock().unwrap();
        state.paused = false;
        state.idle = false;
        self.shared.wake.notify_all();
    }

    /// How many operations the worker attempted (including ones the disk refused).
    pub fn operations(&self) -> u64 {
        self.operations.load(Ordering::Relaxed)
    }

    fn stop_inner(&mut self) {
        let Some(thread) = self.thread.take() else { return };
        {
            let mut state = self.shared.state.lock().unwrap();
            state.stop = true;
            state.paused = false;
            self.shared.wake.notify_all();
        }
        let _ = thread.join();
    }
}

impl Drop for ChurnHandle {
    fn drop(&mut self) {
        self.stop_inner();
    }
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

#[derive(Default)]
struct State {
    stop: bool,
    paused: bool,
    idle: bool,
}

/// Bound the per-operation scan so churn on a 100k folder stays cheap. The
/// selected row is usually near the start of the directory.
const MAX_CANDIDATES: usize = 2000;

fn run(dir: &Path, seed: u64, shared: &Shared, operations: &AtomicU64) {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut counter = 0u64;
    loop {
        {
            let mut state = shared.state.lock().unwrap();
            while state.paused && !state.stop {
                state.idle = true;
                shared.wake.notify_all();
                state = shared.wake.wait(state).unwrap();
            }
            state.idle = false;
            if state.stop {
                return;
            }
        }
        let delay = Duration::from_millis(rng.random_range(1..=50));
        {
            let state = shared.state.lock().unwrap();
            let (state, _) = shared.wake.wait_timeout(state, delay).unwrap();
            if state.stop {
                return;
            }
            if state.paused {
                continue;
            }
        }
        // Errors are expected: churn races the app, itself, and mode-000 entries.
        let _ = churn_once(dir, &mut rng, &mut counter);
        operations.fetch_add(1, Ordering::Relaxed);
    }
}

fn churn_once(dir: &Path, rng: &mut ChaCha8Rng, counter: &mut u64) -> io::Result<()> {
    *counter += 1;
    let n = *counter;
    let mut files = Vec::new();
    let mut folders = Vec::new();
    collect(dir, 0, &mut files, &mut folders);
    match rng.random_range(0..7) {
        0 => {
            std::fs::write(dir.join(format!("churn-{n:06}.txt")), format!("churn {n}\n"))?;
        }
        1 => {
            std::fs::create_dir_all(dir.join(format!("churn-dir-{n:06}")))?;
        }
        2 => {
            if let Some(path) = pick(rng, &files).or_else(|| pick(rng, &folders)) {
                rename(path, n)?;
            }
        }
        3 => {
            if let Some(path) = pick(rng, &files) {
                let _ = std::fs::remove_file(path);
            } else if let Some(path) = pick(rng, &folders) {
                let _ = std::fs::remove_dir_all(path);
            }
        }
        4 => {
            if let Some(path) = pick(rng, &files) {
                let _ = std::fs::write(path, format!("rewritten {n}\n"));
            }
        }
        5 => {
            if let Some(path) = pick(rng, &files).or_else(|| pick(rng, &folders)) {
                toggle_mode(path);
            }
        }
        _ => {
            if let Some(path) = pick(rng, &files).or_else(|| pick(rng, &folders)) {
                move_to(rng, path, dir, &folders, n);
            }
        }
    }
    Ok(())
}

/// `dir`'s direct children and one level down. Symlinks are never followed.
fn collect(dir: &Path, depth: usize, files: &mut Vec<PathBuf>, folders: &mut Vec<PathBuf>) {
    if files.len() + folders.len() >= MAX_CANDIDATES {
        return;
    }
    let Ok(read) = std::fs::read_dir(dir) else { return };
    for entry in read.flatten() {
        if files.len() + folders.len() >= MAX_CANDIDATES {
            return;
        }
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            folders.push(path.clone());
            if depth == 0 {
                collect(&path, 1, files, folders);
            }
        } else if meta.is_file() {
            files.push(path);
        }
    }
}

fn pick<'a>(rng: &mut ChaCha8Rng, items: &'a [PathBuf]) -> Option<&'a Path> {
    if items.is_empty() { None } else { Some(items[rng.random_range(0..items.len())].as_path()) }
}

fn rename(path: &Path, n: u64) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    std::fs::rename(path, parent.join(format!("churn-renamed-{n:06}")))
}

fn toggle_mode(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let Ok(meta) = std::fs::symlink_metadata(path) else { return };
        if meta.file_type().is_symlink() {
            return;
        }
        let mode = meta.permissions().mode() & 0o777;
        let next = if mode == 0 { if meta.is_dir() { 0o755 } else { 0o644 } } else { 0o000 };
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(next));
    }
    #[cfg(not(unix))]
    let _ = path;
}

fn move_to(rng: &mut ChaCha8Rng, path: &Path, dir: &Path, folders: &[PathBuf], n: u64) {
    let mut destinations: Vec<&Path> =
        folders.iter().map(PathBuf::as_path).filter(|folder| !folder.starts_with(path)).collect();
    destinations.push(dir);
    let destination = destinations[rng.random_range(0..destinations.len())];
    let name = path.file_name().map(|name| name.to_string_lossy()).unwrap_or_default();
    let target = destination.join(format!("churn-moved-{n:06}-{name}"));
    let _ = std::fs::rename(path, target);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> crate::test_support::CleanTree {
        crate::test_support::CleanTree::new(&format!("crow-fs-testing-{name}-"))
    }

    fn listed(root: &Path) -> Vec<String> {
        let mut names = Vec::new();
        crate::list_dir(root, true, |batch| names.extend(batch.into_iter().map(|entry| entry.name)))
            .expect("list the tree");
        names.sort();
        names
    }

    fn snapshot(root: &Path) -> Vec<(PathBuf, u64, Option<std::time::SystemTime>)> {
        fn walk(dir: &Path, out: &mut Vec<(PathBuf, u64, Option<std::time::SystemTime>)>) {
            let Ok(read) = std::fs::read_dir(dir) else { return };
            for entry in read.flatten() {
                let path = entry.path();
                let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
                out.push((path.clone(), meta.len(), meta.modified().ok()));
                if meta.is_dir() {
                    walk(&path, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(root, &mut out);
        out.sort();
        out
    }

    /// How many direct children each kind puts in the root. `NfcNfdTwins` is 1 on
    /// APFS (normalization-insensitive) and 2 where both spellings can exist.
    fn expected(kind: EdgeKind) -> std::ops::RangeInclusive<usize> {
        let n = match kind {
            EdgeKind::NfcNfdTwins => return 1..=2,
            EdgeKind::LongName => 2,
            EdgeKind::Emoji => 4,
            EdgeKind::Rtl => 3,
            EdgeKind::LeadingDot => 2,
            EdgeKind::Spaces => 3,
            EdgeKind::Newline => 1,
            EdgeKind::Deep(_) => 1,
            EdgeKind::Wide(n) => n,
            EdgeKind::SymlinkLoop => 2,
            EdgeKind::DanglingSymlink => 1,
            EdgeKind::Unreadable => 2,
            EdgeKind::Package => 1,
            EdgeKind::Hardlinks => 2,
            EdgeKind::EmptyFolder => 1,
            EdgeKind::HugeFile => 1,
        };
        n..=n
    }

    #[test]
    fn each_kind_builds_and_lists() {
        let small = EdgeTree::small();
        let kinds = small.kinds().iter().copied().chain([EdgeKind::Wide(25), EdgeKind::HugeFile]);
        for kind in kinds {
            let root = temp(&format!("kind-{}", kind.label()));
            let tree = make_tree(&root, &EdgeTree::single(kind), 1).expect("make tree");
            let names = listed(&root);
            assert!(expected(kind).contains(&names.len()), "{kind:?}: {names:?}");
            assert_eq!(names.len(), tree.entries(), "{kind:?}: {names:?}");
            tree.remove().expect("remove tree");
            assert!(!root.exists(), "{kind:?} cleaned up");
        }
    }

    /// One bad child (unreadable, looping, dangling, newline) must not fail the
    /// listing of the folder that holds all of them.
    #[test]
    fn every_small_kind_lists_in_one_folder() {
        let root = temp("small");
        let tree = make_tree(&root, &EdgeTree::small(), 1).expect("make tree");
        let names = listed(&root);
        let (low, high) = EdgeTree::small()
            .kinds()
            .iter()
            .fold((0, 0), |(low, high), kind| (low + expected(*kind).start(), high + expected(*kind).end()));
        assert!((low..=high).contains(&names.len()), "{low}..={high}: {names:?}");
        tree.remove().expect("remove tree");
    }

    #[test]
    fn long_names_are_255_bytes() {
        let root = temp("long-names");
        let tree = make_tree(&root, &EdgeTree::single(EdgeKind::LongName), 1).unwrap();
        let names = listed(&root);
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names.iter().any(|name| name.len() == 255 && name.ends_with(".txt")), "{names:?}");
        assert!(names.iter().any(|name| name.len() == 255 && name.chars().all(|c| c == '中')), "{names:?}");
        tree.remove().unwrap();
    }

    #[test]
    fn nfc_nfd_twins_share_one_entry_on_apfs() {
        let root = temp("nfc-nfd");
        let tree = make_tree(&root, &EdgeTree::single(EdgeKind::NfcNfdTwins), 1).unwrap();
        let names = listed(&root);
        assert!(names.len() == 1 || names.len() == 2, "{names:?}");
        let nfc = root.join("caf\u{e9}.txt");
        let nfd = root.join("cafe\u{301}.txt");
        assert!(nfc.exists() && nfd.exists(), "both spellings resolve");
        if names.len() == 1 {
            assert!(names[0] == "caf\u{e9}.txt" || names[0] == "cafe\u{301}.txt", "{names:?}");
        }
        tree.remove().unwrap();
    }

    #[test]
    fn unreadable_child_does_not_fail_the_parent() {
        let root = temp("unreadable");
        let tree = make_tree(&root, &EdgeTree::single(EdgeKind::Unreadable), 1).unwrap();
        let names = listed(&root);
        assert!(names.iter().any(|name| name == "unreadable-dir"), "{names:?}");
        assert!(names.iter().any(|name| name == "unreadable-file"), "{names:?}");
        assert!(std::fs::read_dir(root.join("unreadable-dir")).is_err(), "the child is unreadable");
        assert!(std::fs::read(root.join("unreadable-file")).is_err(), "the file is unreadable");
        tree.remove().expect("cleanup restores permissions");
        assert!(!root.exists());
    }

    #[test]
    fn symlink_loop_is_listed_not_followed() {
        let root = temp("symlink-loop");
        let tree = make_tree(&root, &EdgeTree::single(EdgeKind::SymlinkLoop), 1).unwrap();
        let names = listed(&root);
        assert!(names.iter().any(|name| name == "loop-a"), "{names:?}");
        assert!(names.iter().any(|name| name == "loop-b"), "{names:?}");
        assert!(std::fs::metadata(root.join("loop-a")).is_err(), "stat follows into ELOOP");
        assert!(std::fs::symlink_metadata(root.join("loop-a")).is_ok(), "lstat does not follow");
        tree.remove().unwrap();
    }

    #[test]
    fn huge_file_is_sparse() {
        let root = temp("huge");
        let tree = make_tree(&root, &EdgeTree::single(EdgeKind::HugeFile), 1).unwrap();
        let meta = std::fs::metadata(root.join("huge.bin")).unwrap();
        assert_eq!(meta.len(), 10 * 1024 * 1024 * 1024);
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert!(meta.blocks() < 1024, "sparse: {} blocks", meta.blocks());
        }
        tree.remove().unwrap();
    }

    #[test]
    fn wide_matches_the_bench_fixture() {
        let root = temp("wide");
        let tree = make_tree(&root, &EdgeTree::wide(16), 0).unwrap();
        let names = listed(&root);
        let mut expected: Vec<String> = ["txt", "md", "rs", "png", "json", "pdf", "toml", "swift"]
            .iter()
            .cycle()
            .take(16)
            .enumerate()
            .map(|(i, ext)| format!("file-{i:06}.{ext}"))
            .collect();
        expected.sort();
        assert_eq!(names, expected);
        for name in &names {
            assert_eq!(std::fs::metadata(root.join(name)).unwrap().len(), 0);
        }
        tree.remove().unwrap();
    }

    #[test]
    fn deep_has_twenty_levels() {
        let root = temp("deep");
        let tree = make_tree(&root, &EdgeTree::single(EdgeKind::Deep(20)), 1).unwrap();
        let mut path = root.to_path_buf();
        for level in 0..20 {
            path.push(format!("level-{level:02}"));
        }
        assert!(path.join("bottom.txt").exists());
        tree.remove().unwrap();
    }

    #[test]
    fn labels_parse_back() {
        for kind in EdgeTree::small().kinds().iter().copied().chain([EdgeKind::Wide(100_000), EdgeKind::HugeFile]) {
            assert_eq!(EdgeKind::parse(&kind.label()).unwrap(), kind);
        }
        assert_eq!(EdgeTree::parse("small").unwrap(), EdgeTree::small());
        assert_eq!(EdgeTree::parse("all").unwrap(), EdgeTree::all());
        let mixed = EdgeTree::new([EdgeKind::LongName, EdgeKind::Deep(20)]);
        assert_eq!(EdgeTree::parse("long-name,deep-20").unwrap(), mixed);
        assert_eq!(EdgeTree::parse(&mixed.spec()).unwrap(), mixed);
        assert!(EdgeTree::parse("nope").is_err());
    }

    #[test]
    fn churn_changes_and_pauses() {
        let root = temp("churn");
        let tree = make_tree(&root, &EdgeTree::single(EdgeKind::Wide(50)), 1).unwrap();
        let churn = Churn::start(&root, 7);
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while churn.operations() == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(churn.operations() > 0, "churn ran");
        churn.pause();
        let before = snapshot(&root);
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(snapshot(&root), before, "paused churn is quiet");
        let operations = churn.operations();
        churn.resume();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while churn.operations() == operations && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        churn.pause();
        assert!(churn.operations() > operations, "resume starts again");
        drop(churn);
        tree.remove().unwrap();
    }

    #[test]
    fn churn_never_removes_the_root() {
        let root = temp("churn-root");
        let tree = make_tree(&root, &EdgeTree::single(EdgeKind::Wide(20)), 1).unwrap();
        let churn = Churn::start(&root, 3);
        std::thread::sleep(Duration::from_millis(300));
        churn.pause();
        assert!(root.exists(), "churn never deletes the folder it churns");
        drop(churn);
        tree.remove().unwrap();
    }

    #[test]
    fn churn_survives_unreadable_folders() {
        let root = temp("churn-unreadable");
        let tree = make_tree(&root, &EdgeTree::single(EdgeKind::Unreadable), 1).unwrap();
        let churn = Churn::start(&root, 5);
        std::thread::sleep(Duration::from_millis(300));
        churn.pause();
        drop(churn);
        tree.remove().expect("cleanup after churn");
    }
}
