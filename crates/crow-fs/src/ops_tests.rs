//! Tests for the operation engine. The safety rules from `ops.rs` are checked
//! directly, plus a randomized "do, undo, compare" property test.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::ops::*;

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
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("crow-ops-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".trash")).unwrap();
        let root = root.canonicalize().unwrap();
        Self { trash: FolderTrash(root.join(".trash")), root }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    fn file(&self, rel: &str, content: &str) -> PathBuf {
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

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
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

    let out = f.run(Request::Copy { paths: vec![f.path("src/a.txt"), f.path("src/folder")], into: dest.clone() });
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
    ok(&f.run(Request::Copy { paths: vec![f.path("a.txt")], into: f.root.clone() }));
    assert_eq!(f.snapshot()["a copy.txt"], "x");
}

#[test]
fn move_and_undo_and_moving_into_itself_is_refused() {
    let f = Fixture::new("move");
    f.file("a/inner.txt", "i");
    let dest = f.dir("b");
    let before = f.snapshot();
    let out = f.run(Request::Move { paths: vec![f.path("a")], into: dest.clone() });
    ok(&out);
    assert_eq!(f.snapshot()["b/a/inner.txt"], "i");
    assert!(!f.path("a").exists());
    undo(&out.record, &f.trash).unwrap();
    assert_eq!(f.snapshot(), before);

    assert!(matches!(plan(Request::Move { paths: vec![f.path("a")], into: f.path("a") }), Err(OpError::IntoItself(_))));
    assert!(
        plan(Request::Move { paths: vec![f.path("a")], into: f.root.clone() }).unwrap().items.is_empty(),
        "already there"
    );
}

#[test]
fn conflicts_are_planned_and_each_choice_is_honoured() {
    let f = Fixture::new("conflict");
    f.file("new/a.txt", "new");
    f.file("dest/a.txt", "old");
    let request = Request::Copy { paths: vec![f.path("new/a.txt")], into: f.path("dest") };
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
    let planned = plan(Request::Copy { paths: vec![f.path("src/a.txt")], into: dest }).unwrap();
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
    let paths: Vec<PathBuf> = (0..5).map(|i| f.path(&format!("src/{i}.txt"))).collect();
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
    let out = f.run(Request::Copy { paths: vec![f.path("src/a.txt")], into: dest.clone() });
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
    let planned = plan(Request::Copy { paths: vec![f.path("a.txt"), f.path("b.txt")], into: f.dir("dest") }).unwrap();
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
    let trashed = f.run(Request::Trash { paths: vec![f.path("t.txt")] });
    ok(&trashed);
    assert!(!f.path("t.txt").exists());
    undo(&trashed.record, &f.trash).unwrap();
    assert_eq!(f.snapshot()["t.txt"], "t", "Put Back");

    let deleted = f.run(Request::DeletePermanently { paths: vec![f.path("t.txt")] });
    ok(&deleted);
    assert!(!deleted.record.is_undoable());
    assert!(matches!(undo(&deleted.record, &f.trash), Err(OpError::NotUndoable)));
}

#[test]
fn progress_counts_items_and_bytes() {
    let f = Fixture::new("progress");
    f.file("src/a", &"x".repeat(3000));
    f.file("src/b", &"y".repeat(1000));
    let planned = plan(Request::Copy { paths: vec![f.path("src/a"), f.path("src/b")], into: f.dir("dest") }).unwrap();
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
            0 => Request::Copy { paths: vec![pick], into: sub.clone() },
            1 => Request::Move { paths: vec![pick], into: sub.clone() },
            2 => Request::Trash { paths: vec![pick] },
            3 => Request::Duplicate { paths: vec![pick] },
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
    let out = f.run(Request::Move { paths: vec![f.path("src/tree")], into: f.dir("dest") });
    let undone = undo(&out.record, &f.trash);
    FORCE_CROSS_DEVICE.store(false, Ordering::Relaxed);
    ok(&out);
    undone.expect("undo across volumes");
    let after_undo = f.snapshot();
    assert_eq!(after_undo.get("src/tree/sub/b.txt").map(String::as_str), Some("b"));
    assert!(!after_undo.keys().any(|k| k.starts_with("dest/tree")), "the moved copy went back");
    assert_eq!(after_undo.into_iter().filter(|(k, _)| k != "dest").collect::<BTreeMap<_, _>>(), before);
}
