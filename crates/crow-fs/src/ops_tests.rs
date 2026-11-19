//! Tests for the operation engine. The safety rules from `ops.rs` are checked
//! directly, plus a randomized "do, undo, compare" property test.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::ops::*;

/// A request's paths are shared: a 100k selection hands them over as one refcount.
fn paths(paths: Vec<PathBuf>) -> Arc<[PathBuf]> {
    paths.into()
}

/// A Trash that is just a folder, so tests can see what was trashed.
struct FolderTrash(PathBuf);

impl Trash for FolderTrash {
    fn trash(&self, path: &Path) -> io::Result<PathBuf> {
        static N: AtomicU64 = AtomicU64::new(0);
        let name = path.file_name().ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        let dest = self.0.join(format!("{}-{}", N.fetch_add(1, Ordering::Relaxed), name.to_string_lossy()));
        std::fs::rename(path, &dest)?;
        Ok(dest)
    }
}

struct Fixture {
    root: PathBuf,
    trash: FolderTrash,
    _guard: crate::test_support::TempDir,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let guard = crate::test_support::TempDir::new(&format!("crow-ops-{name}-"));
        std::fs::create_dir_all(guard.join(".trash")).unwrap();
        let root = guard.to_path_buf();
        Self { trash: FolderTrash(root.join(".trash")), root, _guard: guard }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    fn file(&self, rel: &str, content: &str) -> PathBuf {
        self.file_bytes(rel, content.as_bytes())
    }

    fn file_bytes(&self, rel: &str, content: &[u8]) -> PathBuf {
        let p = self.path(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, content).unwrap();
        p
    }

    fn dir(&self, rel: &str) -> PathBuf {
        let p = self.path(rel);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// Everything under the root except the trash: relative path → file content (or "<dir>").
    fn snapshot(&self) -> BTreeMap<String, String> {
        fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
            for entry in std::fs::read_dir(dir).unwrap().filter_map(Result::ok) {
                let path = entry.path();
                let rel = path.strip_prefix(base).unwrap().to_string_lossy().into_owned();
                if rel == ".trash" {
                    continue;
                }
                if path.is_dir() {
                    out.insert(rel, "<dir>".into());
                    walk(base, &path, out);
                } else {
                    out.insert(rel, std::fs::read_to_string(&path).unwrap_or_else(|_| "<binary>".into()));
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(&self.root, &self.root, &mut out);
        out
    }

    fn run(&self, request: Request) -> Outcome {
        self.run_with(request, &HashMap::new())
    }

    fn run_with(&self, request: Request, resolutions: &HashMap<PathBuf, Resolution>) -> Outcome {
        let plan = plan(request).expect("plan");
        execute(&plan, resolutions, &self.trash, &AtomicBool::new(false), &mut |_| {})
    }
}

fn ok(outcome: &Outcome) {
    assert!(outcome.error.is_none(), "unexpected error: {:?}", outcome.error);
    assert!(!outcome.cancelled);
}

#[test]
fn names_follow_finder() {
    let f = Fixture::new("names");
    f.file("report.pdf", "");
    f.file("report 2.pdf", "");
    f.file(".zshrc", "");
    f.dir("Tool.app");
    f.dir("v1.2");
    assert_eq!(keep_both_name(&f.root, "report.pdf", false), f.path("report 3.pdf"));
    assert_eq!(copy_name(&f.root, "report.pdf", false), f.path("report copy.pdf"));
    assert_eq!(copy_name(&f.root, ".zshrc", false), f.path(".zshrc copy"), "a leading dot is not an extension");
    assert_eq!(copy_name(&f.root, "Tool.app", true), f.path("Tool copy.app"), "packages keep their extension");
    assert_eq!(copy_name(&f.root, "v1.2", true), f.path("v1.2 copy"), "folders keep their whole name");
    assert_eq!(untitled_folder(&f.root), f.path("untitled folder"));
    for bad in ["", "  ", "a/b", ".", ".."] {
        assert!(check_name(bad).is_err(), "{bad:?}");
    }
    assert!(check_name("ok: really.txt").is_ok());
}

#[test]
fn copy_keeps_contents_and_undo_trashes_the_copies() {
    let f = Fixture::new("copy");
    f.file("src/a.txt", "alpha");
    f.file("src/folder/deep/b.txt", "beta");
    let dest = f.dir("dest");
    let before = f.snapshot();

    let out =
        f.run(Request::Copy { paths: paths(vec![f.path("src/a.txt"), f.path("src/folder")]), into: dest.clone() });
    ok(&out);
    let after = f.snapshot();
    assert_eq!(after["dest/a.txt"], "alpha");
    assert_eq!(after["dest/folder/deep/b.txt"], "beta");
    assert_eq!(after["src/a.txt"], "alpha", "the source is untouched");
    assert_eq!(out.record.results(), vec![dest.join("a.txt"), dest.join("folder")]);

    let redo = undo(&out.record, &f.trash).expect("undo");
    assert_eq!(f.snapshot(), before);
    undo(&redo, &f.trash).expect("redo");
    assert_eq!(f.snapshot(), after);
}

#[test]
fn copying_into_the_same_folder_duplicates() {
    let f = Fixture::new("copy-self");
    f.file("a.txt", "x");
    ok(&f.run(Request::Copy { paths: paths(vec![f.path("a.txt")]), into: f.root.clone() }));
    assert_eq!(f.snapshot()["a copy.txt"], "x");
}

#[test]
fn move_and_undo_and_moving_into_itself_is_refused() {
    let f = Fixture::new("move");
    f.file("a/inner.txt", "i");
    let dest = f.dir("b");
    let before = f.snapshot();
    let out = f.run(Request::Move { paths: paths(vec![f.path("a")]), into: dest.clone() });
    ok(&out);
    assert_eq!(f.snapshot()["b/a/inner.txt"], "i");
    assert!(!f.path("a").exists());
    undo(&out.record, &f.trash).unwrap();
    assert_eq!(f.snapshot(), before);

    assert!(matches!(
        plan(Request::Move { paths: paths(vec![f.path("a")]), into: f.path("a") }),
        Err(OpError::IntoItself(_))
    ));
    assert!(
        plan(Request::Move { paths: paths(vec![f.path("a")]), into: f.root.clone() }).unwrap().items.is_empty(),
        "already there"
    );
}

#[test]
fn conflicts_are_planned_and_each_choice_is_honoured() {
    let f = Fixture::new("conflict");
    f.file("new/a.txt", "new");
    f.file("dest/a.txt", "old");
    let request = Request::Copy { paths: paths(vec![f.path("new/a.txt")]), into: f.path("dest") };
    let planned = plan(request.clone()).unwrap();
    assert_eq!(planned.conflicts().count(), 1);

    let target = f.path("dest/a.txt");
    let skip = f.run_with(request.clone(), &HashMap::from([(target.clone(), Resolution::Skip)]));
    ok(&skip);
    assert_eq!(f.snapshot()["dest/a.txt"], "old");
    assert!(skip.record.steps.is_empty());

    ok(&f.run(request.clone()));
    assert_eq!(f.snapshot()["dest/a 2.txt"], "new", "keep both is the default");
    assert_eq!(f.snapshot()["dest/a.txt"], "old");

    let before = f.snapshot();
    let replace = f.run_with(request, &HashMap::from([(target.clone(), Resolution::Replace)]));
    ok(&replace);
    assert_eq!(f.snapshot()["dest/a.txt"], "new");
    assert!(std::fs::read_dir(f.path(".trash")).unwrap().count() == 1, "the replaced file went to the Trash");
    undo(&replace.record, &f.trash).unwrap();
    assert_eq!(f.snapshot(), before, "undoing a replace brings the old file back");
}

#[test]
fn a_name_taken_after_planning_is_never_overwritten() {
    let f = Fixture::new("race");
    f.file("src/a.txt", "mine");
    let dest = f.dir("dest");
    let planned = plan(Request::Copy { paths: paths(vec![f.path("src/a.txt")]), into: dest }).unwrap();
    assert_eq!(planned.conflicts().count(), 0);
    f.file("dest/a.txt", "appeared meanwhile");
    ok(&execute(&planned, &HashMap::new(), &f.trash, &AtomicBool::new(false), &mut |_| {}));
    assert_eq!(f.snapshot()["dest/a.txt"], "appeared meanwhile");
    assert_eq!(f.snapshot()["dest/a 2.txt"], "mine");
}

#[test]
fn cancelling_stops_between_items_and_leaves_no_partial_copies() {
    let f = Fixture::new("cancel");
    for i in 0..5 {
        f.file(&format!("src/{i}.txt"), &"x".repeat(1000));
    }
    let paths: Arc<[PathBuf]> = (0..5).map(|i| f.path(&format!("src/{i}.txt"))).collect::<Vec<_>>().into();
    let planned = plan(Request::Copy { paths, into: f.dir("dest") }).unwrap();
    let cancel = AtomicBool::new(false);
    let out = execute(&planned, &HashMap::new(), &f.trash, &cancel, &mut |p| {
        if p.done_items == 2 {
            cancel.store(true, Ordering::Relaxed);
        }
    });
    assert!(out.cancelled && out.error.is_none());
    let copied: Vec<String> = f.snapshot().into_keys().filter(|k| k.starts_with("dest/")).collect();
    assert_eq!(copied.len(), out.record.steps.len(), "every copied item is in the record");
    assert!(copied.len() < 5);
    assert!(!copied.iter().any(|k| k.contains(".crow-partial")), "no partial copies");
    undo(&out.record, &f.trash).unwrap();
    assert!(!f.snapshot().keys().any(|k| k.starts_with("dest/")), "a cancelled copy undoes cleanly");
}

#[test]
fn a_failed_copy_leaves_nothing_behind_and_keeps_the_source() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new("readonly");
    f.file("src/a.txt", "a");
    let dest = f.dir("dest");
    std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o555)).unwrap();
    let out = f.run(Request::Copy { paths: paths(vec![f.path("src/a.txt")]), into: dest.clone() });
    std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(out.error.is_some(), "writing into a read-only folder fails");
    assert!(out.record.steps.is_empty());
    assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 0, "no partial copy");
    assert_eq!(f.snapshot()["src/a.txt"], "a");
}

#[test]
fn a_vanished_source_stops_the_operation_with_a_clear_error() {
    let f = Fixture::new("vanished");
    f.file("a.txt", "a");
    f.file("b.txt", "b");
    let planned =
        plan(Request::Copy { paths: paths(vec![f.path("a.txt"), f.path("b.txt")]), into: f.dir("dest") }).unwrap();
    std::fs::remove_file(f.path("b.txt")).unwrap();
    let out = execute(&planned, &HashMap::new(), &f.trash, &AtomicBool::new(false), &mut |_| {});
    assert!(matches!(out.error, Some(OpError::NotFound(_))));
    assert_eq!(out.record.steps.len(), 1, "the first copy happened and is recorded");
}

#[test]
fn undo_never_overwrites_what_appeared_meanwhile() {
    let f = Fixture::new("undo-safe");
    f.file("a.txt", "original");
    let out = f.run(Request::Rename { path: f.path("a.txt"), new_name: "b.txt".into() });
    ok(&out);
    f.file("a.txt", "newcomer");
    assert!(matches!(undo(&out.record, &f.trash), Err(OpError::WouldOverwrite(_))));
    assert_eq!(f.snapshot()["a.txt"], "newcomer");
    assert_eq!(f.snapshot()["b.txt"], "original");
}

#[test]
fn renames_check_names_and_handle_case_changes() {
    let f = Fixture::new("rename");
    f.file("Notes.txt", "n");
    f.file("other.txt", "o");
    assert!(matches!(
        plan(Request::Rename { path: f.path("Notes.txt"), new_name: "other.txt".into() }),
        Err(OpError::NameTaken(_))
    ));
    assert!(matches!(
        plan(Request::Rename { path: f.path("Notes.txt"), new_name: "a/b".into() }),
        Err(OpError::InvalidName(_))
    ));
    let out = f.run(Request::Rename { path: f.path("Notes.txt"), new_name: "notes.txt".into() });
    ok(&out);
    assert!(f.snapshot().contains_key("notes.txt"), "case-only rename works on case-insensitive volumes");
    undo(&out.record, &f.trash).unwrap();
    assert!(f.snapshot().contains_key("Notes.txt"));
}

#[test]
fn new_folder_trash_and_permanent_delete() {
    let f = Fixture::new("misc");
    let out = f.run(Request::NewFolder { parent: f.root.clone() });
    ok(&out);
    assert_eq!(f.snapshot()["untitled folder"], "<dir>");
    ok(&f.run(Request::NewFolder { parent: f.root.clone() }));
    assert!(f.snapshot().contains_key("untitled folder 2"));

    f.file("t.txt", "t");
    let trashed = f.run(Request::Trash { paths: paths(vec![f.path("t.txt")]) });
    ok(&trashed);
    assert!(!f.path("t.txt").exists());
    undo(&trashed.record, &f.trash).unwrap();
    assert_eq!(f.snapshot()["t.txt"], "t", "Put Back");

    let deleted = f.run(Request::DeletePermanently { paths: paths(vec![f.path("t.txt")]) });
    ok(&deleted);
    assert!(!deleted.record.is_undoable());
    assert!(matches!(undo(&deleted.record, &f.trash), Err(OpError::NotUndoable)));
}

#[test]
fn new_file_creates_an_empty_file_with_a_free_name() {
    let f = Fixture::new("new-file");
    let out = f.run(Request::NewFile { parent: f.root.clone() });
    ok(&out);
    assert_eq!(f.snapshot()["untitled"], "");
    ok(&f.run(Request::NewFile { parent: f.root.clone() }));
    assert!(f.snapshot().contains_key("untitled 2"), "a taken name gets the next number");
    assert!(!f.path("untitled 2").is_dir(), "a file, not a folder");

    // Undo moves the new file to the Trash and can be redone.
    let undone = undo(&out.record, &f.trash).expect("undo");
    assert!(!f.path("untitled").exists(), "undo removes the new file");
    assert!(f.snapshot().contains_key("untitled 2"), "and leaves the other one");
    undo(&undone, &f.trash).expect("redo");
    assert!(f.path("untitled").exists(), "undoing the undo puts it back");
}

#[test]
fn progress_counts_items_and_bytes() {
    let f = Fixture::new("progress");
    f.file("src/a", &"x".repeat(3000));
    f.file("src/b", &"y".repeat(1000));
    let planned =
        plan(Request::Copy { paths: paths(vec![f.path("src/a"), f.path("src/b")]), into: f.dir("dest") }).unwrap();
    assert_eq!(planned.total_bytes, 4000);
    let mut last = Progress::default();
    ok(&execute(&planned, &HashMap::new(), &f.trash, &AtomicBool::new(false), &mut |p| last = p.clone()));
    assert_eq!((last.done_items, last.total_items, last.done_bytes), (2, 2, 4000));
}

/// Random trees, random operations: doing then undoing always restores the tree exactly.
#[test]
fn do_then_undo_restores_the_tree() {
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut rand = move |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
    };
    let mut exercised = 0;
    for round in 0..40 {
        let f = Fixture::new(&format!("prop-{round}"));
        let mut items = Vec::new();
        for i in 0..(2 + rand(5)) {
            let p = if rand(3) == 0 {
                f.file(&format!("d{i}/f{}.txt", rand(3)), &format!("c{i}"))
            } else {
                f.file(&format!("f{i}.txt"), &format!("c{i}"))
            };
            items.push(p);
        }
        let sub = f.dir(&format!("target{}", rand(2)));
        // Make a conflict sometimes.
        if rand(2) == 0 {
            f.file(&format!("{}/f0.txt", sub.file_name().unwrap().to_string_lossy()), "occupied");
        }
        let before = f.snapshot();
        let pick = items[rand(items.len())].clone();
        let request = match rand(5) {
            0 => Request::Copy { paths: paths(vec![pick]), into: sub.clone() },
            1 => Request::Move { paths: paths(vec![pick]), into: sub.clone() },
            2 => Request::Trash { paths: paths(vec![pick]) },
            3 => Request::Duplicate { paths: paths(vec![pick]) },
            _ => Request::Rename { path: pick, new_name: format!("renamed-{round}.txt") },
        };
        let resolution = [Resolution::KeepBoth, Resolution::Replace, Resolution::Skip][rand(3)];
        let resolutions: HashMap<PathBuf, Resolution> = plan(request.clone())
            .unwrap()
            .items
            .iter()
            .filter(|i| i.conflict)
            .map(|i| (i.target.clone(), resolution))
            .collect();
        let out = f.run_with(request.clone(), &resolutions);
        ok(&out);
        if out.record.steps.is_empty() {
            assert_eq!(f.snapshot(), before, "round {round}: a skipped operation changes nothing");
            continue;
        }
        assert_ne!(f.snapshot(), before, "round {round}: {request:?} changed something");
        undo(&out.record, &f.trash).unwrap_or_else(|e| panic!("round {round}: undo {request:?}: {e}"));
        assert_eq!(f.snapshot(), before, "round {round}: undo of {request:?} with {resolution:?}");
        exercised += 1;
    }
    assert!(exercised >= 25, "only {exercised} of 40 rounds did something; the test is too weak");
}

#[test]
fn moving_across_volumes_copies_then_removes_the_source() {
    let f = Fixture::new("cross");
    f.file("src/tree/a.txt", "a");
    f.file("src/tree/sub/b.txt", "b");
    let before = f.snapshot();
    FORCE_CROSS_DEVICE.store(true, Ordering::Relaxed);
    let out = f.run(Request::Move { paths: paths(vec![f.path("src/tree")]), into: f.dir("dest") });
    let undone = undo(&out.record, &f.trash);
    FORCE_CROSS_DEVICE.store(false, Ordering::Relaxed);
    ok(&out);
    undone.expect("undo across volumes");
    let after_undo = f.snapshot();
    assert_eq!(after_undo.get("src/tree/sub/b.txt").map(String::as_str), Some("b"));
    assert!(!after_undo.keys().any(|k| k.starts_with("dest/tree")), "the moved copy went back");
    assert_eq!(after_undo.into_iter().filter(|(k, _)| k != "dest").collect::<BTreeMap<_, _>>(), before);
}

// ----- job engine -----

#[derive(Default)]
struct EventState {
    events: Vec<JobEvent>,
    terminal: bool,
}

struct EventLog {
    state: Mutex<EventState>,
    cv: Condvar,
}

impl EventLog {
    fn new() -> Self {
        Self { state: Mutex::new(EventState::default()), cv: Condvar::new() }
    }

    fn push(&self, event: JobEvent) {
        let mut state = self.state.lock().unwrap();
        if matches!(event, JobEvent::Terminal { .. }) {
            state.terminal = true;
        }
        state.events.push(event);
        self.cv.notify_all();
    }

    fn wait_for(&self, predicate: impl Fn(&JobEvent) -> bool) -> JobEvent {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut state = self.state.lock().unwrap();
        loop {
            if let Some(event) = state.events.iter().find(|event| predicate(event)) {
                return event.clone();
            }
            assert!(!state.terminal, "terminal before the expected event: {:?}", state.events);
            let now = Instant::now();
            assert!(now < deadline, "timed out waiting; events: {:?}", state.events);
            let (next, _) = self.cv.wait_timeout(state, deadline - now).unwrap();
            state = next;
        }
    }

    fn terminal(&self) -> Arc<Outcome> {
        match self.wait_for(|event| matches!(event, JobEvent::Terminal { .. })) {
            JobEvent::Terminal { outcome, .. } => outcome,
            _ => unreachable!(),
        }
    }

    fn events(&self) -> Vec<JobEvent> {
        self.state.lock().unwrap().events.clone()
    }
}

struct Harness {
    job: Job,
    log: Arc<EventLog>,
}

impl Harness {
    fn terminal(&self) -> Arc<Outcome> {
        self.log.terminal()
    }
}

#[derive(Default)]
struct GateState {
    reached: bool,
    released: bool,
}

struct Gate {
    state: Mutex<GateState>,
    cv: Condvar,
}

impl Gate {
    fn new() -> Arc<Self> {
        Arc::new(Self { state: Mutex::new(GateState::default()), cv: Condvar::new() })
    }

    fn reach(&self) {
        let mut state = self.state.lock().unwrap();
        state.reached = true;
        self.cv.notify_all();
        while !state.released {
            state = self.cv.wait(state).unwrap();
        }
    }

    fn wait_reached(&self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut state = self.state.lock().unwrap();
        while !state.reached {
            let now = Instant::now();
            assert!(now < deadline, "the transfer never reached the test gate");
            let (next, _) = self.cv.wait_timeout(state, deadline - now).unwrap();
            state = next;
        }
    }

    fn release(&self) {
        let mut state = self.state.lock().unwrap();
        state.released = true;
        self.cv.notify_all();
    }
}

fn gate_hook(gate: &Arc<Gate>) -> Arc<dyn Fn() + Send + Sync> {
    let gate = gate.clone();
    Arc::new(move || gate.reach())
}

fn job_options(hooks: TestHooks) -> JobOptions {
    JobOptions { progress_interval: Duration::ZERO, hooks: Some(hooks), ..Default::default() }
}

impl Fixture {
    fn start_plan(&self, plan: Plan, options: JobOptions) -> Harness {
        let log = Arc::new(EventLog::new());
        let sink = log.clone();
        let job = Job::spawn(plan, options, Arc::new(FolderTrash(self.path(".trash"))), move |event| sink.push(event))
            .expect("spawn job");
        Harness { job, log }
    }

    fn start(&self, request: Request, options: JobOptions) -> Harness {
        self.start_plan(plan(request).expect("plan"), options)
    }
}

fn stage_container(f: &Fixture, dir: &str) -> PathBuf {
    std::fs::read_dir(f.path(dir))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.file_name().is_some_and(|name| name.to_string_lossy().starts_with(".crow-partial-")))
        .expect("a staging container")
}

#[test]
fn a_paused_copy_resumes_the_staged_transfer() {
    let f = Fixture::new("job-pause");
    let data = vec![b'x'; 8 * 1024 * 1024];
    f.file_bytes("src/big.bin", &data);
    let dest = f.dir("dest");
    let gate = Gate::new();
    let options =
        job_options(TestHooks { on_first_bytes: Some(gate_hook(&gate)), no_clone: true, ..Default::default() });
    let planned = plan(Request::Copy { paths: paths(vec![f.path("src/big.bin")]), into: dest.clone() }).unwrap();
    let harness = f.start_plan(planned, options);

    gate.wait_reached();
    assert!(harness.job.pause(), "a transfer accepts a pause");
    gate.release();
    let ack = harness.log.wait_for(|event| matches!(event, JobEvent::PauseAcknowledged { .. }));
    let paused_bytes = match ack {
        JobEvent::PauseAcknowledged { progress, .. } => progress.done_bytes,
        _ => unreachable!(),
    };
    assert!(paused_bytes > 0 && paused_bytes < data.len() as u64, "paused inside the transfer: {paused_bytes}");

    let stage = stage_container(&f, "dest");
    let staged = std::fs::metadata(stage.join("item")).unwrap().len();
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(std::fs::metadata(stage.join("item")).unwrap().len(), staged, "bytes stay put while paused");
    assert!(!dest.join("big.bin").exists(), "the target is untouched while paused");

    harness.job.resume();
    let outcome = harness.terminal();
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert!(!outcome.cancelled);
    assert_eq!(std::fs::read(dest.join("big.bin")).unwrap(), data);
    assert_eq!(
        harness.log.events().iter().filter(|event| matches!(event, JobEvent::Terminal { .. })).count(),
        1,
        "exactly one terminal outcome"
    );
    assert!(
        !std::fs::read_dir(&dest)
            .unwrap()
            .any(|entry| { entry.unwrap().file_name().to_string_lossy().starts_with(".crow-partial-") }),
        "the stage is removed after commit"
    );
}

#[test]
fn a_paused_copy_cancels_and_removes_only_its_stage() {
    let f = Fixture::new("job-pause-cancel");
    let data = vec![b'y'; 8 * 1024 * 1024];
    f.file_bytes("src/big.bin", &data);
    let dest = f.dir("dest");
    let gate = Gate::new();
    let options =
        job_options(TestHooks { on_first_bytes: Some(gate_hook(&gate)), no_clone: true, ..Default::default() });
    let planned = plan(Request::Copy { paths: paths(vec![f.path("src/big.bin")]), into: dest.clone() }).unwrap();
    let harness = f.start_plan(planned, options);

    gate.wait_reached();
    assert!(harness.job.pause());
    gate.release();
    harness.log.wait_for(|event| matches!(event, JobEvent::PauseAcknowledged { .. }));
    harness.job.cancel();
    let outcome = harness.terminal();
    assert!(outcome.cancelled, "{outcome:?}");
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert!(outcome.record.steps.is_empty());
    assert!(!dest.join("big.bin").exists());
    assert!(
        !std::fs::read_dir(&dest)
            .unwrap()
            .any(|entry| { entry.unwrap().file_name().to_string_lossy().starts_with(".crow-partial-") }),
        "the stage is gone"
    );
    assert_eq!(std::fs::read(f.path("src/big.bin")).unwrap(), data, "the source is untouched");
}

#[test]
fn a_late_conflict_wait_is_keyed_and_current() {
    let f = Fixture::new("job-late");
    f.file("src/a.txt", "new");
    let dest = f.dir("dest");
    let planned = plan(Request::Copy { paths: paths(vec![f.path("src/a.txt")]), into: dest.clone() }).unwrap();
    f.file("dest/a.txt", "occupied");
    let harness = f.start_plan(planned, JobOptions { progress_interval: Duration::ZERO, ..Default::default() });

    let event = harness.log.wait_for(|event| matches!(event, JobEvent::Conflict { .. }));
    let conflict = match event {
        JobEvent::Conflict(conflict) => conflict,
        _ => unreachable!(),
    };
    assert_eq!(conflict.reason, ConflictReason::Appeared);
    assert_eq!(conflict.target, dest.join("a.txt"));
    assert_eq!(f.snapshot()["dest/a.txt"], "occupied", "the occupant is untouched while waiting");
    assert!(!harness.job.resolve(ConflictKey { attempt: 999, ..conflict.key }, Resolution::Skip, false));
    assert!(harness.job.resolve(conflict.key, Resolution::KeepBoth, false));

    let outcome = harness.terminal();
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert_eq!(f.snapshot()["dest/a.txt"], "occupied");
    assert_eq!(f.snapshot()["dest/a 2.txt"], "new");
}

#[test]
fn resolving_for_an_old_attempt_is_a_no_op() {
    let f = Fixture::new("job-stale");
    f.file("src/a.txt", "new");
    f.file("dest/a.txt", "old");
    let planned = plan(Request::Copy { paths: paths(vec![f.path("src/a.txt")]), into: f.dir("dest") }).unwrap();
    let harness = f.start_plan(planned, JobOptions { progress_interval: Duration::ZERO, ..Default::default() });
    let first = match harness.log.wait_for(|event| matches!(event, JobEvent::Conflict { .. })) {
        JobEvent::Conflict(conflict) => conflict,
        _ => unreachable!(),
    };
    // Replace the occupant, then answer the old attempt: the engine must notice
    // the change and ask again rather than trash the new occupant.
    f.file("dest/a.txt", "different");
    assert!(harness.job.resolve(first.key, Resolution::Replace, false));
    let second = match harness
        .log
        .wait_for(|event| matches!(event, JobEvent::Conflict(conflict) if conflict.reason == ConflictReason::Changed))
    {
        JobEvent::Conflict(conflict) => conflict,
        _ => unreachable!(),
    };
    assert_ne!(second.key, first.key);
    assert!(!harness.job.resolve(first.key, Resolution::Skip, false), "the old key is stale");
    assert!(harness.job.resolve(second.key, Resolution::Skip, false));
    let outcome = harness.terminal();
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert_eq!(f.snapshot()["dest/a.txt"], "different", "the new occupant is preserved");
    assert_eq!(f.snapshot()["src/a.txt"], "new");
}

#[test]
fn an_approved_target_replaced_while_paused_reopens_the_conflict() {
    let f = Fixture::new("job-approved-changed");
    f.file("src/a.txt", "new");
    let dest = f.dir("dest");
    f.file("dest/a.txt", "old");
    let planned = plan(Request::Copy { paths: paths(vec![f.path("src/a.txt")]), into: dest.clone() }).unwrap();
    let gate = Gate::new();
    let mut options =
        job_options(TestHooks { after_stage: Some(gate_hook(&gate)), no_clone: true, ..Default::default() });
    options.resolutions.insert(dest.join("a.txt"), Resolution::Replace);
    let harness = f.start_plan(planned, options);

    gate.wait_reached();
    assert!(harness.job.pause());
    gate.release();
    harness.log.wait_for(|event| matches!(event, JobEvent::PauseAcknowledged { .. }));
    std::fs::remove_file(dest.join("a.txt")).unwrap();
    f.file("dest/a.txt", "new occupant");
    harness.job.resume();

    let conflict = match harness
        .log
        .wait_for(|event| matches!(event, JobEvent::Conflict(conflict) if conflict.reason == ConflictReason::Changed))
    {
        JobEvent::Conflict(conflict) => conflict,
        _ => unreachable!(),
    };
    assert!(harness.job.resolve(conflict.key, Resolution::Skip, false));
    let outcome = harness.terminal();
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert!(outcome.record.steps.is_empty(), "the old approval was discarded, not applied");
    assert_eq!(conflict.source, f.path("src/a.txt"), "the conflict names the original source, not its stage");
    assert_eq!(f.snapshot()["dest/a.txt"], "new occupant");
    assert!(!f.snapshot().contains_key("dest/a 2.txt"));
}

#[test]
fn a_cancel_during_final_placement_keeps_the_committed_step() {
    let f = Fixture::new("job-cancel-commit");
    f.file("src/a.txt", "a");
    let dest = f.dir("dest");
    let gate = Gate::new();
    let options = job_options(TestHooks { before_commit: Some(gate_hook(&gate)), ..Default::default() });
    let planned = plan(Request::Copy { paths: paths(vec![f.path("src/a.txt")]), into: dest.clone() }).unwrap();
    let harness = f.start_plan(planned, options);
    gate.wait_reached();
    harness.job.cancel();
    gate.release();
    let outcome = harness.terminal();
    assert!(outcome.cancelled, "{outcome:?}");
    assert_eq!(outcome.record.steps.len(), 1, "the placement finished its journal step");
    assert_eq!(f.snapshot()["dest/a.txt"], "a");
}

#[test]
fn keep_both_retries_a_collision() {
    let f = Fixture::new("job-keep-both-race");
    f.file("src/a.txt", "new");
    let dest = f.dir("dest");
    f.file("dest/a.txt", "old");
    let planned = plan(Request::Copy { paths: paths(vec![f.path("src/a.txt")]), into: dest.clone() }).unwrap();
    let gate = Gate::new();
    let mut options = job_options(TestHooks { before_commit: Some(gate_hook(&gate)), ..Default::default() });
    options.resolutions.insert(dest.join("a.txt"), Resolution::KeepBoth);
    let harness = f.start_plan(planned, options);
    gate.wait_reached();
    f.file("dest/a 2.txt", "raced");
    gate.release();
    let outcome = harness.terminal();
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert_eq!(f.snapshot()["dest/a.txt"], "old");
    assert_eq!(f.snapshot()["dest/a 2.txt"], "raced");
    assert_eq!(f.snapshot()["dest/a 3.txt"], "new");
}

fn xattrs(path: &Path) -> String {
    String::from_utf8_lossy(&std::process::Command::new("/usr/bin/xattr").arg("-l").arg(path).output().unwrap().stdout)
        .into_owned()
}

fn acl(path: &Path) -> String {
    String::from_utf8_lossy(&std::process::Command::new("/bin/ls").arg("-led").arg(path).output().unwrap().stdout)
        .into_owned()
}

#[test]
fn metadata_xattrs_and_acl_survive_both_copy_paths() {
    use std::os::unix::fs::MetadataExt;
    let f = Fixture::new("job-metadata");
    let tree = f.dir("src/tree");
    f.file("src/tree/meta.txt", "metadata");
    std::fs::create_dir_all(tree.join("sub")).unwrap();
    f.file("src/tree/sub/deep.txt", "deep");
    std::os::unix::fs::symlink("/nonexistent/crow-target", tree.join("dangling")).unwrap();
    let meta = tree.join("meta.txt");
    assert!(
        std::process::Command::new("/usr/bin/xattr")
            .args(["-w", "user.crow-test", "hello", meta.to_str().unwrap()])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        std::process::Command::new("/bin/chmod")
            .args(["+a", "everyone allow read", meta.to_str().unwrap()])
            .status()
            .unwrap()
            .success()
    );
    let source_meta = std::fs::metadata(&meta).unwrap();

    for (dest, no_clone) in [("dest-clone", false), ("dest-noclone", true)] {
        let out = f
            .start(
                Request::Copy { paths: paths(vec![tree.clone()]), into: f.dir(dest) },
                job_options(TestHooks { no_clone, ..Default::default() }),
            )
            .terminal();
        assert!(out.error.is_none(), "{dest}: {:?}", out.error);
        let copy = f.path(&format!("{dest}/tree/meta.txt"));
        let copied = std::fs::metadata(&copy).unwrap();
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), "metadata");
        assert_eq!(copied.mtime(), source_meta.mtime());
        assert_eq!(copied.mtime_nsec(), source_meta.mtime_nsec());
        assert_eq!(copied.mode(), source_meta.mode());
        assert!(xattrs(&copy).contains("user.crow-test: hello"), "{dest}: {}", xattrs(&copy));
        assert!(acl(&copy).contains("everyone allow read"), "{dest}: {}", acl(&copy));
        assert!(std::fs::symlink_metadata(f.path(&format!("{dest}/tree/dangling"))).unwrap().file_type().is_symlink());
        assert_eq!(
            std::fs::read_link(f.path(&format!("{dest}/tree/dangling"))).unwrap(),
            Path::new("/nonexistent/crow-target")
        );
    }
}

#[test]
fn a_cross_volume_move_keeps_the_destination_when_the_source_cannot_be_removed() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new("job-source-kept");
    f.file("src/tree/a.txt", "a");
    let dest = f.dir("dest");
    let gate = Gate::new();
    let options = job_options(TestHooks {
        force_cross_device: true,
        before_source_retire: Some(gate_hook(&gate)),
        no_clone: true,
        ..Default::default()
    });
    let planned = plan(Request::Move { paths: paths(vec![f.path("src/tree")]), into: dest.clone() }).unwrap();
    let harness = f.start_plan(planned, options);
    gate.wait_reached();
    std::fs::set_permissions(f.path("src"), std::fs::Permissions::from_mode(0o555)).unwrap();
    gate.release();
    let outcome = harness.terminal();
    std::fs::set_permissions(f.path("src"), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(outcome.error, Some(OpError::SourceKept { .. })), "{:?}", outcome.error);
    assert_eq!(f.snapshot()["dest/tree/a.txt"], "a", "the complete destination stays");
    assert!(f.path("src/tree").exists(), "the source was not fully retired");
    assert!(matches!(outcome.record.steps.as_slice(), [Step::Created(target)] if *target == dest.join("tree")));
    undo(&outcome.record, &f.trash).unwrap();
    assert!(!f.path("dest/tree").exists(), "the partial record is undoable");
}

#[test]
fn a_failed_placement_restores_the_old_target() {
    let f = Fixture::new("job-restore");
    f.file("src/a.txt", "new");
    let dest = f.dir("dest");
    f.file("dest/a.txt", "old");
    let planned = plan(Request::Copy { paths: paths(vec![f.path("src/a.txt")]), into: dest.clone() }).unwrap();
    let gate = Gate::new();
    let mut options =
        job_options(TestHooks { after_replace_trash: Some(gate_hook(&gate)), no_clone: true, ..Default::default() });
    options.resolutions.insert(dest.join("a.txt"), Resolution::Replace);
    let harness = f.start_plan(planned, options);
    gate.wait_reached();
    let stage = stage_container(&f, "dest");
    std::fs::remove_file(stage.join("item")).unwrap();
    gate.release();
    let outcome = harness.terminal();
    assert!(outcome.error.is_some(), "the placement failed");
    assert!(outcome.record.steps.is_empty(), "a fully restored replacement has no step");
    assert_eq!(f.snapshot()["dest/a.txt"], "old", "the old target is back");
    assert!(
        !std::fs::read_dir(&dest)
            .unwrap()
            .any(|entry| { entry.unwrap().file_name().to_string_lossy().starts_with(".crow-partial-") })
    );
}

#[test]
fn a_blocked_restore_keeps_the_recoverable_trash_step() {
    let f = Fixture::new("job-restore-blocked");
    f.file("src/a.txt", "new");
    let dest = f.dir("dest");
    f.file("dest/a.txt", "old");
    let planned = plan(Request::Copy { paths: paths(vec![f.path("src/a.txt")]), into: dest.clone() }).unwrap();
    let gate = Gate::new();
    let mut options =
        job_options(TestHooks { after_replace_trash: Some(gate_hook(&gate)), no_clone: true, ..Default::default() });
    options.resolutions.insert(dest.join("a.txt"), Resolution::Replace);
    let harness = f.start_plan(planned, options);
    gate.wait_reached();
    f.file("dest/a.txt", "new occupant");
    gate.release();
    let outcome = harness.terminal();
    let (trashed, original) = match &outcome.error {
        Some(OpError::RestoreBlocked { target, trashed, .. }) => (trashed.clone(), target.clone()),
        other => panic!("expected a blocked restore, got {other:?}"),
    };
    assert_eq!(original, dest.join("a.txt"));
    assert_eq!(f.snapshot()["dest/a.txt"], "new occupant", "the new occupant is preserved");
    assert_eq!(std::fs::read_to_string(&trashed).unwrap(), "old", "the old target is recoverable");
    assert!(
        matches!(outcome.record.steps.as_slice(), [Step::Trashed { original: o, trashed: t }] if *o == original && *t == trashed)
    );
}

#[test]
fn a_same_volume_move_replaces_and_undoes() {
    let f = Fixture::new("job-move-replace");
    f.file("src/a.txt", "new");
    let dest = f.dir("dest");
    f.file("dest/a.txt", "old");
    let before = f.snapshot();
    let planned = plan(Request::Move { paths: paths(vec![f.path("src/a.txt")]), into: dest.clone() }).unwrap();
    let mut options = JobOptions { progress_interval: Duration::ZERO, ..Default::default() };
    options.resolutions.insert(dest.join("a.txt"), Resolution::Replace);
    let outcome = f.start_plan(planned, options).terminal();
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert!(!f.path("src/a.txt").exists());
    assert_eq!(f.snapshot()["dest/a.txt"], "new");
    assert!(
        matches!(
            outcome.record.steps.as_slice(),
            [Step::Trashed { .. }, Step::Moved { from, to }] if *from == f.path("src/a.txt") && *to == dest.join("a.txt")
        ),
        "{:?}",
        outcome.record.steps
    );
    undo(&outcome.record, &f.trash).unwrap();
    assert_eq!(f.snapshot(), before);
}

#[test]
fn cancel_after_replace_trash_still_places_and_records() {
    let f = Fixture::new("job-cancel-replace");
    f.file("src/a.txt", "new");
    let dest = f.dir("dest");
    f.file("dest/a.txt", "old");
    let before = f.snapshot();
    let planned = plan(Request::Copy { paths: paths(vec![f.path("src/a.txt")]), into: dest.clone() }).unwrap();
    let gate = Gate::new();
    let mut options =
        job_options(TestHooks { after_replace_trash: Some(gate_hook(&gate)), no_clone: true, ..Default::default() });
    options.resolutions.insert(dest.join("a.txt"), Resolution::Replace);
    let harness = f.start_plan(planned, options);
    gate.wait_reached();
    harness.job.cancel();
    gate.release();
    let outcome = harness.terminal();
    assert!(outcome.cancelled, "{outcome:?}");
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert_eq!(outcome.record.steps.len(), 2, "trash and placement both finished: {:?}", outcome.record.steps);
    assert_eq!(f.snapshot()["dest/a.txt"], "new");
    undo(&outcome.record, &f.trash).unwrap();
    assert_eq!(f.snapshot(), before);
}

#[test]
fn apply_to_all_answers_later_conflicts() {
    let f = Fixture::new("job-apply-all");
    f.file("src/a.txt", "A");
    f.file("src/b.txt", "B");
    let dest = f.dir("dest");
    f.file("dest/a.txt", "old A");
    f.file("dest/b.txt", "old B");
    let planned =
        plan(Request::Copy { paths: paths(vec![f.path("src/a.txt"), f.path("src/b.txt")]), into: dest.clone() })
            .unwrap();
    let harness = f.start_plan(planned, JobOptions { progress_interval: Duration::ZERO, ..Default::default() });
    let first = match harness.log.wait_for(|event| matches!(event, JobEvent::Conflict { .. })) {
        JobEvent::Conflict(conflict) => conflict,
        _ => unreachable!(),
    };
    assert!(harness.job.resolve(first.key, Resolution::Replace, true));
    let auto =
        match harness.log.wait_for(|event| matches!(event, JobEvent::Conflict(conflict) if conflict.auto.is_some())) {
            JobEvent::Conflict(conflict) => conflict,
            _ => unreachable!(),
        };
    assert_eq!(auto.auto, Some(Resolution::Replace));
    assert_eq!(auto.policy, Some(Resolution::Replace));
    let outcome = harness.terminal();
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert_eq!(f.snapshot()["dest/a.txt"], "A");
    assert_eq!(f.snapshot()["dest/b.txt"], "B");
    assert_eq!(outcome.record.steps.len(), 4, "trash + create for each item");
    undo(&outcome.record, &f.trash).unwrap();
    assert_eq!(f.snapshot()["dest/a.txt"], "old A");
    assert_eq!(f.snapshot()["dest/b.txt"], "old B");
}

#[test]
fn cancelling_after_one_commit_leaves_an_undoable_prefix() {
    let f = Fixture::new("job-prefix");
    f.file("src/a.txt", "A");
    f.file("src/b.txt", "B");
    let dest = f.dir("dest");
    let gate = Gate::new();
    let options = job_options(TestHooks { after_commit: Some(gate_hook(&gate)), no_clone: true, ..Default::default() });
    let planned =
        plan(Request::Copy { paths: paths(vec![f.path("src/a.txt"), f.path("src/b.txt")]), into: dest.clone() })
            .unwrap();
    let harness = f.start_plan(planned, options);
    gate.wait_reached();
    harness.job.cancel();
    gate.release();
    let outcome = harness.terminal();
    assert!(outcome.cancelled);
    assert_eq!(outcome.record.steps.len(), 1);
    assert_eq!(f.snapshot().get("dest/a.txt").map(String::as_str), Some("A"));
    assert!(!f.snapshot().contains_key("dest/b.txt"));
    undo(&outcome.record, &f.trash).unwrap();
    assert!(!f.path("dest/a.txt").exists(), "the committed prefix is undoable");
}

#[test]
fn a_source_change_while_paused_fails_before_the_target_is_touched() {
    let f = Fixture::new("job-source-changed");
    f.file("src/a.txt", "one");
    let dest = f.dir("dest");
    let planned = plan(Request::Copy { paths: paths(vec![f.path("src/a.txt")]), into: dest.clone() }).unwrap();
    let gate = Gate::new();
    let options = job_options(TestHooks { after_stage: Some(gate_hook(&gate)), no_clone: true, ..Default::default() });
    let harness = f.start_plan(planned, options);
    gate.wait_reached();
    assert!(harness.job.pause());
    gate.release();
    harness.log.wait_for(|event| matches!(event, JobEvent::PauseAcknowledged { .. }));
    f.file("src/a.txt", "one and a half");
    harness.job.resume();
    let outcome = harness.terminal();
    assert!(matches!(outcome.error, Some(OpError::SourceChanged(_))), "{:?}", outcome.error);
    assert!(!f.snapshot().contains_key("dest/a.txt"));
    assert!(outcome.record.steps.is_empty());
    assert!(
        !std::fs::read_dir(&dest)
            .unwrap()
            .any(|entry| { entry.unwrap().file_name().to_string_lossy().starts_with(".crow-partial-") })
    );
}

#[test]
fn progress_never_goes_backwards() {
    let f = Fixture::new("job-progress-monotonic");
    let a = f.file_bytes("src/tree/a.bin", &vec![b'z'; 4 * 1024 * 1024]);
    f.file_bytes("src/tree/b.bin", &vec![b'z'; 4 * 1024 * 1024]);
    assert!(
        std::process::Command::new("/usr/bin/xattr")
            .args(["-w", "user.crow-test", "hello", a.to_str().unwrap()])
            .status()
            .unwrap()
            .success()
    );
    let dest = f.dir("dest");
    let planned = plan(Request::Copy { paths: paths(vec![f.path("src/tree")]), into: dest }).unwrap();
    let harness = f.start_plan(planned, job_options(TestHooks { no_clone: true, ..Default::default() }));
    let outcome = harness.terminal();
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    let mut last = 0;
    for event in harness.log.events() {
        if let JobEvent::Progress { progress, .. } = event {
            assert!(progress.done_bytes >= last, "progress went backwards: {last} -> {}", progress.done_bytes);
            last = progress.done_bytes;
        }
    }
    assert_eq!(last, 8 * 1024 * 1024);
}

#[test]
fn a_case_folded_target_is_a_conflict() {
    let f = Fixture::new("job-case-fold");
    f.file("src/a.txt", "new");
    let dest = f.dir("dest");
    f.file("dest/A.txt", "old");
    if !dest.join("a.txt").exists() {
        eprintln!("the test volume is case-sensitive; skipping the case-folded alias");
        return;
    }
    let planned = plan(Request::Copy { paths: paths(vec![f.path("src/a.txt")]), into: dest.clone() }).unwrap();
    assert!(planned.items[0].conflict, "the case-folded name is taken");
    let mut options = JobOptions { progress_interval: Duration::ZERO, ..Default::default() };
    options.resolutions.insert(dest.join("a.txt"), Resolution::KeepBoth);
    let outcome = f.start_plan(planned, options).terminal();
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert_eq!(f.snapshot()["dest/A.txt"], "old");
    assert_eq!(f.snapshot()["dest/a 2.txt"], "new");
}

#[test]
fn a_dangling_symlink_is_copied_as_a_link() {
    let f = Fixture::new("job-dangling");
    let src = f.dir("src");
    std::os::unix::fs::symlink("/nonexistent/crow-target", src.join("link")).unwrap();
    let out = f
        .start(
            Request::Copy { paths: paths(vec![src]), into: f.dir("dest") },
            job_options(TestHooks { no_clone: true, ..Default::default() }),
        )
        .terminal();
    assert!(out.error.is_none(), "{:?}", out.error);
    let copied = f.path("dest/src/link");
    assert!(std::fs::symlink_metadata(&copied).unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read_link(copied).unwrap(), Path::new("/nonexistent/crow-target"));
}

#[test]
fn duplicate_basenames_are_planned_as_conflicts_and_duplicates_get_unique_names() {
    let f = Fixture::new("job-duplicates");
    f.file("one/a.txt", "1");
    f.file("two/a.txt", "2");
    let dest = f.dir("dest");
    let planned =
        plan(Request::Copy { paths: paths(vec![f.path("one/a.txt"), f.path("two/a.txt")]), into: dest.clone() })
            .unwrap();
    assert_eq!(planned.items[0].target, dest.join("a.txt"));
    assert_eq!(planned.items[1].target, dest.join("a.txt"));
    assert!(!planned.items[0].conflict);
    assert!(planned.items[1].conflict);
    let mut options = JobOptions { progress_interval: Duration::ZERO, ..Default::default() };
    options.resolutions.insert(dest.join("a.txt"), Resolution::KeepBoth);
    let outcome = f.start_plan(planned, options).terminal();
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert_eq!(f.snapshot()["dest/a.txt"], "1");
    assert_eq!(f.snapshot()["dest/a 2.txt"], "2");

    let planned = plan(Request::Duplicate { paths: paths(vec![f.path("one/a.txt"), f.path("two/a.txt")]) }).unwrap();
    assert_eq!(planned.items[0].target, f.path("one/a copy.txt"));
    assert_eq!(planned.items[1].target, f.path("two/a copy.txt"));
    let outcome =
        f.start_plan(planned, JobOptions { progress_interval: Duration::ZERO, ..Default::default() }).terminal();
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert!(f.path("one/a copy.txt").exists());
    assert!(f.path("two/a copy.txt").exists());
}

#[test]
fn nesting_through_a_symlink_is_refused() {
    let f = Fixture::new("job-nested-link");
    f.dir("root/sub");
    std::os::unix::fs::symlink(f.path("root"), f.path("root/link")).unwrap();
    assert!(matches!(
        plan(Request::Copy { paths: paths(vec![f.path("root")]), into: f.path("root/link/sub") }),
        Err(OpError::IntoItself(_))
    ));
    assert!(matches!(
        plan(Request::Copy { paths: paths(vec![f.path("root")]), into: f.path("root/sub/nope") }),
        Err(OpError::Io(..))
    ));
}

#[test]
fn a_symlinked_parent_counts_as_the_same_folder() {
    let f = Fixture::new("job-parent-link");
    f.file("root/a.txt", "a");
    std::os::unix::fs::symlink(f.path("root"), f.path("link")).unwrap();

    let planned = plan(Request::Move { paths: paths(vec![f.path("root/a.txt")]), into: f.path("link") }).unwrap();
    assert!(planned.items.is_empty(), "moving through a symlinked parent is already there: {:?}", planned.items);

    let out = f
        .start(
            Request::Copy { paths: paths(vec![f.path("root/a.txt")]), into: f.path("link") },
            job_options(TestHooks { no_clone: true, ..Default::default() }),
        )
        .terminal();
    assert!(out.error.is_none(), "{:?}", out.error);
    assert_eq!(std::fs::read_to_string(f.path("root/a.txt")).unwrap(), "a");
    assert_eq!(
        std::fs::read_to_string(f.path("root/a copy.txt")).unwrap(),
        "a",
        "the copy uses Duplicate naming, not a conflict"
    );
    assert!(!f.path("root/a 2.txt").exists());
}

#[test]
fn a_cancelled_plan_stops_measuring() {
    let f = Fixture::new("job-plan-cancel");
    for i in 0..100 {
        f.file(&format!("src/{i}.txt"), "x");
    }
    let cancel = AtomicBool::new(true);
    assert!(matches!(
        plan_cancellable(Request::Copy { paths: paths(vec![f.path("src")]), into: f.dir("dest") }, &cancel),
        Err(OpError::Cancelled)
    ));
}

#[test]
fn pause_is_unavailable_while_waiting_on_a_conflict() {
    let f = Fixture::new("job-pause-conflict");
    f.file("src/a.txt", "new");
    f.file("dest/a.txt", "old");
    let planned = plan(Request::Copy { paths: paths(vec![f.path("src/a.txt")]), into: f.path("dest") }).unwrap();
    let harness = f.start_plan(planned, JobOptions { progress_interval: Duration::ZERO, ..Default::default() });
    let conflict = match harness.log.wait_for(|event| matches!(event, JobEvent::Conflict { .. })) {
        JobEvent::Conflict(conflict) => conflict,
        _ => unreachable!(),
    };
    assert!(!harness.job.pause(), "Pause is unavailable while a conflict waits");
    assert!(harness.job.resolve(conflict.key, Resolution::Skip, false));
    harness.terminal();
}

#[test]
fn a_clone_acknowledges_pause_after_the_clone() {
    let f = Fixture::new("job-clone-pause");
    f.file("src/a.txt", "clone me");
    let dest = f.dir("dest");
    let gate = Gate::new();
    let options = job_options(TestHooks { after_stage: Some(gate_hook(&gate)), ..Default::default() });
    let planned = plan(Request::Copy { paths: paths(vec![f.path("src/a.txt")]), into: dest.clone() }).unwrap();
    let harness = f.start_plan(planned, options);
    gate.wait_reached();
    assert!(harness.job.pause());
    gate.release();
    let ack = harness.log.wait_for(|event| matches!(event, JobEvent::PauseAcknowledged { .. }));
    let done = match ack {
        JobEvent::PauseAcknowledged { progress, .. } => progress.done_bytes,
        _ => unreachable!(),
    };
    assert_eq!(done, 8, "the clone is complete before the pause is acknowledged");
    assert!(!dest.join("a.txt").exists(), "the clone is still staged");
    harness.job.resume();
    let outcome = harness.terminal();
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"clone me");
}

#[test]
fn cancelling_while_a_conflict_waits_leaves_the_target_alone() {
    let f = Fixture::new("job-cancel-conflict");
    f.file("src/a.txt", "new");
    f.file("dest/a.txt", "old");
    let planned = plan(Request::Copy { paths: paths(vec![f.path("src/a.txt")]), into: f.path("dest") }).unwrap();
    let harness = f.start_plan(planned, JobOptions { progress_interval: Duration::ZERO, ..Default::default() });
    harness.log.wait_for(|event| matches!(event, JobEvent::Conflict { .. }));
    harness.job.cancel();
    let outcome = harness.terminal();
    assert!(outcome.cancelled);
    assert!(outcome.record.steps.is_empty());
    assert_eq!(f.snapshot()["dest/a.txt"], "old");
    assert_eq!(f.snapshot()["src/a.txt"], "new");
}

#[test]
fn special_files_are_refused_in_planning() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let f = Fixture::new("job-special");
    let fifo = f.path("src/fifo");
    std::fs::create_dir_all(fifo.parent().unwrap()).unwrap();
    let path = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    // SAFETY: a valid NUL-terminated path and a normal mode.
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    assert!(matches!(
        plan(Request::Copy { paths: paths(vec![fifo]), into: f.dir("dest") }),
        Err(OpError::Unsupported(..))
    ));
}

#[test]
fn copying_into_a_file_is_refused() {
    let f = Fixture::new("job-into-file");
    f.file("src/a.txt", "a");
    f.file("not-a-folder", "nope");
    assert!(matches!(
        plan(Request::Copy { paths: paths(vec![f.path("src/a.txt")]), into: f.path("not-a-folder") }),
        Err(OpError::NotAFolder(_))
    ));
}

#[test]
fn replace_stages_before_trashing_the_old_target() {
    struct ProbeTrash {
        inner: FolderTrash,
        parent: PathBuf,
    }

    impl Trash for ProbeTrash {
        fn trash(&self, path: &Path) -> io::Result<PathBuf> {
            let staged = std::fs::read_dir(&self.parent)
                .unwrap()
                .any(|entry| entry.unwrap().file_name().to_string_lossy().starts_with(".crow-partial-"));
            assert!(staged, "Replace must stage the new bytes before touching the old target");
            self.inner.trash(path)
        }
    }

    let f = Fixture::new("stage-before-trash");
    f.file("src/a.txt", "new");
    let dest = f.dir("dest");
    f.file("dest/a.txt", "old");
    let request = Request::Copy { paths: paths(vec![f.path("src/a.txt")]), into: dest.clone() };
    let planned = plan(request).unwrap();
    let trash = ProbeTrash { inner: FolderTrash(f.path(".trash")), parent: dest.clone() };
    let resolutions = HashMap::from([(dest.join("a.txt"), Resolution::Replace)]);
    let outcome = execute(&planned, &resolutions, &trash, &AtomicBool::new(false), &mut |_| {});
    ok(&outcome);
    assert_eq!(f.snapshot()["dest/a.txt"], "new");
    assert_eq!(std::fs::read_dir(f.path(".trash")).unwrap().count(), 1);
}

#[test]
fn a_target_swapped_between_verify_and_trash_is_restored() {
    let f = Fixture::new("job-swap-before-trash");
    f.file("src/a.txt", "new");
    let dest = f.dir("dest");
    f.file("dest/a.txt", "approved old");
    let planned = plan(Request::Copy { paths: paths(vec![f.path("src/a.txt")]), into: dest.clone() }).unwrap();
    let gate = Gate::new();
    let mut options =
        job_options(TestHooks { before_replace_trash: Some(gate_hook(&gate)), no_clone: true, ..Default::default() });
    options.resolutions.insert(dest.join("a.txt"), Resolution::Replace);
    let harness = f.start_plan(planned, options);
    gate.wait_reached();
    // This is the Q1 boundary: the approval covered the old inode, but the
    // Trash call will move whatever is at the path now.
    std::fs::remove_file(dest.join("a.txt")).unwrap();
    f.file("dest/a.txt", "new occupant");
    gate.release();
    let conflict = match harness
        .log
        .wait_for(|event| matches!(event, JobEvent::Conflict(conflict) if conflict.reason == ConflictReason::Changed))
    {
        JobEvent::Conflict(conflict) => conflict,
        _ => unreachable!(),
    };
    assert!(harness.job.resolve(conflict.key, Resolution::Skip, false));
    let outcome = harness.terminal();
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert!(outcome.record.steps.is_empty(), "the swapped occupant was restored, not recorded");
    assert_eq!(f.snapshot()["dest/a.txt"], "new occupant");
}
