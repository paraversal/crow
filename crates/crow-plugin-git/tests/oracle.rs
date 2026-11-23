//! Git CLI oracle for the read-only provider. The CLI is a test dependency only.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

use crow_plugin_git::{DiscoveryLimits, HeadState, LineDiff, StatusKind, discover, head_info, status_snapshot};

struct Repo {
    dir: PathBuf,
    _temp: tempfile::TempDir,
}

impl Repo {
    fn new(name: &str) -> Self {
        let temp = tempfile::TempDir::new().unwrap();
        let dir = temp.path().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let repo = Self { dir, _temp: temp };
        repo.git(&["init", "-q", "-b", "main"]);
        repo.git(&["config", "user.name", "Crow Test"]);
        repo.git(&["config", "user.email", "crow@example.invalid"]);
        repo
    }

    fn command(&self, args: &[&str]) -> std::process::Output {
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(&self.dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Crow Test")
            .env("GIT_AUTHOR_EMAIL", "crow@example.invalid")
            .env("GIT_COMMITTER_NAME", "Crow Test")
            .env("GIT_COMMITTER_EMAIL", "crow@example.invalid");
        command.output().unwrap()
    }

    fn git(&self, args: &[&str]) -> String {
        let out = self.command(args);
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn try_git(&self, args: &[&str]) {
        let _ = self.command(args);
    }

    fn write(&self, path: &str, bytes: &[u8]) {
        let path = self.dir.join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, bytes).unwrap();
    }

    fn commit(&self, message: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", message]);
    }

    fn open(&self) -> gix::Repository {
        gix::open(&self.dir).unwrap()
    }

    fn snapshot(&self) -> crow_plugin_git::StatusSnapshot {
        status_snapshot(&self.open(), &AtomicBool::new(false)).unwrap()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Expected {
    index: Option<StatusKind>,
    worktree: Option<StatusKind>,
    ignored: bool,
    renamed_from: Option<PathBuf>,
}

fn kind(byte: u8) -> Option<StatusKind> {
    match byte {
        b'.' => None,
        b'M' => Some(StatusKind::Modified),
        b'T' => Some(StatusKind::TypeChange),
        b'A' => Some(StatusKind::Added),
        b'D' => Some(StatusKind::Deleted),
        b'R' => Some(StatusKind::Renamed),
        b'C' => Some(StatusKind::Added),
        b'U' => Some(StatusKind::Unmerged),
        other => panic!("unknown porcelain status {:?}", other as char),
    }
}

fn path_field(bytes: &[u8]) -> PathBuf {
    let bytes = bytes.strip_suffix(b"/").unwrap_or(bytes);
    PathBuf::from(OsString::from_vec(bytes.to_vec()))
}

fn porcelain(repo: &Repo) -> BTreeMap<PathBuf, Expected> {
    let out = repo.command(&["status", "--porcelain=v2", "-z", "--untracked-files=all", "--ignored=matching"]);
    assert!(out.status.success());
    let mut records = out.stdout.split(|byte| *byte == 0);
    let mut expected = BTreeMap::new();
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        match record[0] {
            b'1' => {
                let parts: Vec<&[u8]> = record.splitn(9, |byte| *byte == b' ').collect();
                let xy = parts[1];
                let path = path_field(parts[8]);
                expected.insert(
                    path,
                    Expected { index: kind(xy[0]), worktree: kind(xy[1]), ignored: false, renamed_from: None },
                );
            }
            b'2' => {
                let parts: Vec<&[u8]> = record.splitn(10, |byte| *byte == b' ').collect();
                let xy = parts[1];
                let path = path_field(parts[9]);
                let source = records.next().expect("rename source");
                expected.insert(
                    path,
                    Expected {
                        index: kind(xy[0]),
                        worktree: kind(xy[1]),
                        ignored: false,
                        renamed_from: Some(path_field(source)),
                    },
                );
            }
            b'u' => {
                let parts: Vec<&[u8]> = record.splitn(11, |byte| *byte == b' ').collect();
                expected.insert(
                    path_field(parts[10]),
                    Expected {
                        index: Some(StatusKind::Unmerged),
                        worktree: Some(StatusKind::Unmerged),
                        ignored: false,
                        renamed_from: None,
                    },
                );
            }
            b'?' => {
                expected.insert(
                    path_field(&record[2..]),
                    Expected { worktree: Some(StatusKind::Untracked), ..Default::default() },
                );
            }
            b'!' => {
                expected.insert(path_field(&record[2..]), Expected { ignored: true, ..Default::default() });
            }
            other => panic!("unknown porcelain record {:?}", other as char),
        }
    }
    expected
}

fn actual(snapshot: &crow_plugin_git::StatusSnapshot) -> BTreeMap<PathBuf, Expected> {
    snapshot
        .entries
        .iter()
        .map(|(path, entry)| {
            (
                path.clone(),
                Expected {
                    index: entry.index,
                    worktree: entry.worktree,
                    ignored: entry.ignored,
                    renamed_from: entry.renamed_from.clone(),
                },
            )
        })
        .collect()
}

fn assert_oracle(repo: &Repo) {
    let expected = porcelain(repo);
    let snapshot = repo.snapshot();
    assert_eq!(actual(&snapshot), expected);
}

fn index_stamp(identity: &crow_plugin_git::RepositoryIdentity) -> Option<(u64, std::time::SystemTime)> {
    let metadata = std::fs::metadata(identity.git_dir.join("index")).ok()?;
    Some((metadata.len(), metadata.modified().ok()?))
}

#[test]
fn status_matches_porcelain_and_never_writes_the_index() {
    let repo = Repo::new("staged");
    repo.write("both.txt", b"one\ntwo\nthree\nfour\n");
    repo.write("old.txt", b"abc\n");
    repo.write("gone.txt", b"x\n");
    repo.write("bin.dat", b"bin\0ary");
    repo.commit("base");
    repo.write("both.txt", b"one\nTWO\nthree\nfour\n");
    repo.git(&["add", "both.txt"]);
    repo.write("both.txt", b"one\nTWO\nTHREE\nfour\nfive\n");
    repo.write("untracked.txt", b"new\n");
    repo.write("bin.dat", b"bin\0CHANGED");
    repo.git(&["mv", "old.txt", "renamed.txt"]);
    std::fs::remove_file(repo.dir.join("gone.txt")).unwrap();

    let expected = porcelain(&repo);
    let identity = crow_plugin_git::RepositoryIdentity::from_repo(&repo.open()).unwrap();
    let before = index_stamp(&identity);
    let snapshot = repo.snapshot();
    let after = index_stamp(&identity);
    assert_eq!(before, after, "status must not write the index");
    assert_eq!(actual(&snapshot), expected);
    assert_eq!(snapshot.changed_count, 5);
    assert_eq!(snapshot.entries[Path::new("both.txt")].index, Some(StatusKind::Modified));
    assert_eq!(snapshot.entries[Path::new("both.txt")].worktree, Some(StatusKind::Modified));
    assert_eq!(snapshot.entries[Path::new("renamed.txt")].renamed_from.as_deref(), Some(Path::new("old.txt")));
}

#[test]
fn changes_mark_parent_folders_with_the_strongest_badge() {
    let repo = Repo::new("folder-badges");
    repo.write(".gitignore", b"*.log\n");
    repo.write("docs/plans/m1.md", b"original plan\n");
    repo.write("docs-copy/clean.txt", b"unchanged\n");
    repo.write("old/sub/move.txt", b"moved to another folder\n");
    repo.write("removed/sub/gone.txt", b"deleted from disk\n");
    repo.commit("base");

    repo.write("docs/new/draft.md", b"untracked\n");
    repo.write("docs/plans/m1.md", b"updated plan\n");
    repo.write("ignored/sub/cache.log", b"ignored\n");
    std::fs::create_dir_all(repo.dir.join("new/sub")).unwrap();
    repo.git(&["mv", "old/sub/move.txt", "new/sub/move.txt"]);
    std::fs::remove_file(repo.dir.join("removed/sub/gone.txt")).unwrap();

    let snapshot = repo.snapshot();
    assert_eq!(snapshot.changed_count, 4);
    assert_eq!(
        snapshot.affected_folders,
        BTreeMap::from([
            ("docs".into(), StatusKind::Modified),
            ("docs/new".into(), StatusKind::Untracked),
            ("docs/plans".into(), StatusKind::Modified),
            ("new".into(), StatusKind::Renamed),
            ("new/sub".into(), StatusKind::Renamed),
            ("old".into(), StatusKind::Renamed),
            ("old/sub".into(), StatusKind::Renamed),
            ("removed".into(), StatusKind::Deleted),
            ("removed/sub".into(), StatusKind::Deleted),
        ])
    );
    repo.commit("accept changes");
    assert!(repo.snapshot().affected_folders.is_empty());
}

#[test]
fn conflict_ignored_and_binary_statuses_are_typed() {
    let repo = Repo::new("conflict");
    repo.write("c.txt", b"base\n");
    repo.commit("base");
    repo.git(&["checkout", "-q", "-b", "side"]);
    repo.write("c.txt", b"side\n");
    repo.commit("side");
    repo.git(&["checkout", "-q", "main"]);
    repo.write("c.txt", b"main\n");
    repo.commit("main");
    repo.try_git(&["merge", "side"]);
    assert_oracle(&repo);
    let snapshot = repo.snapshot();
    assert_eq!(snapshot.entries[Path::new("c.txt")].badge(), Some(StatusKind::Unmerged));
    assert_eq!(snapshot.changed_count, 1);
    assert_eq!(LineDiff::new(&repo.open()).unwrap().counts(&snapshot, Path::new("c.txt")).unwrap(), None);

    let ignored = Repo::new("ignored");
    ignored.write(".gitignore", b"*.log\nbuild/\n");
    ignored.write("a.txt", b"a\n");
    ignored.commit("base");
    ignored.write("keep.log", b"tracked\n");
    ignored.git(&["add", "-f", "keep.log"]);
    ignored.git(&["commit", "-q", "-m", "keep"]);
    ignored.write("x.log", b"i\n");
    ignored.write("build/new.txt", b"u\n");
    assert_oracle(&ignored);
    let snapshot = ignored.snapshot();
    assert!(snapshot.entries[Path::new("x.log")].ignored);
    assert!(snapshot.entries[Path::new("build")].ignored);
    assert!(
        !snapshot.entries.contains_key(Path::new("keep.log")),
        "a tracked file matching an ignore rule stays clean"
    );
    assert_eq!(snapshot.changed_count, 0);
}

#[test]
fn external_filters_are_never_invoked() {
    let repo = Repo::new("filtered");
    let sentinel = repo.dir.join("sentinel");
    repo.write(".gitattributes", b"*.txt filter=sentinel\n");
    repo.git(&["config", "filter.sentinel.clean", &format!("sh -c 'echo ran >> {}; cat'", sentinel.display())]);
    repo.git(&["config", "filter.sentinel.smudge", "cat"]);
    repo.write("f.txt", b"aaa\n");
    repo.git(&["add", ".gitattributes", "f.txt"]);
    repo.git(&["commit", "-q", "-m", "base"]);
    repo.write("f.txt", b"bbb\n");
    std::fs::remove_file(&sentinel).ok();

    let snapshot = repo.snapshot();
    let counts = LineDiff::new(&repo.open()).unwrap().counts(&snapshot, Path::new("f.txt")).unwrap();
    assert_eq!(counts, Some(crow_plugin_git::LineCounts { insertions: 1, removals: 1 }));
    assert!(!sentinel.exists(), "the repo-local clean filter must not run");
    assert_eq!(snapshot.entries[Path::new("f.txt")].badge(), Some(StatusKind::Modified));
}

#[test]
fn git_diff_keeps_staged_and_unstaged_hunks_separate() {
    use crow_plugin_git::{DiffLayer, DiffLine};
    let repo = Repo::new("separate-diff-layers");
    repo.write("file.txt", b"before\n");
    repo.commit("base");
    repo.write("file.txt", b"staged\n");
    repo.git(&["add", "file.txt"]);
    repo.write("file.txt", b"unstaged\n");
    let snapshot = repo.snapshot();
    let diff = LineDiff::new(&repo.open()).unwrap().diff(&snapshot, Path::new("file.txt")).unwrap();
    let DiffLayer::Text(staged) = diff.staged else { panic!("expected staged text") };
    let DiffLayer::Text(unstaged) = diff.unstaged else { panic!("expected unstaged text") };
    assert_eq!(
        staged.lines,
        [DiffLine::Hunk { before: 1, after: 1 }, DiffLine::Removed("before".into()), DiffLine::Added("staged".into())]
    );
    assert_eq!(
        unstaged.lines,
        [
            DiffLine::Hunk { before: 1, after: 1 },
            DiffLine::Removed("staged".into()),
            DiffLine::Added("unstaged".into())
        ]
    );
}

#[test]
fn line_counts_match_git_numstat_and_skip_ineligible_rows() {
    let repo = Repo::new("lines");
    repo.write("both.txt", b"one\ntwo\nthree\nfour\n");
    repo.write("bin.dat", b"bin\0ary");
    repo.write("old.txt", b"abc\n");
    repo.commit("base");
    repo.write("both.txt", b"one\nTWO\nthree\nfour\n");
    repo.git(&["add", "both.txt"]);
    repo.write("both.txt", b"one\nTWO\nTHREE\nfour\nfive\n");
    repo.write("bin.dat", b"bin\0CHANGED");
    repo.git(&["mv", "old.txt", "renamed.txt"]);
    repo.write("untracked.txt", b"new\n");

    let snapshot = repo.snapshot();
    let both = LineDiff::new(&repo.open()).unwrap().counts(&snapshot, Path::new("both.txt")).unwrap().unwrap();
    let numstat = repo.git(&["diff", "--numstat", "HEAD"]);
    let line = numstat.lines().find(|line| line.ends_with("both.txt")).unwrap();
    let fields: Vec<&str> = line.split('\t').collect();
    assert_eq!(format!("{}\t{}", both.insertions, both.removals), format!("{}\t{}", fields[0], fields[1]));

    assert_eq!(LineDiff::new(&repo.open()).unwrap().counts(&snapshot, Path::new("bin.dat")).unwrap(), None);
    assert_eq!(LineDiff::new(&repo.open()).unwrap().counts(&snapshot, Path::new("untracked.txt")).unwrap(), None);
    assert_eq!(
        LineDiff::new(&repo.open()).unwrap().counts(&snapshot, Path::new("renamed.txt")).unwrap(),
        Some(crow_plugin_git::LineCounts { insertions: 0, removals: 0 })
    );
}

#[test]
fn a_binary_staged_side_cannot_be_hidden_by_a_text_worktree_diff() {
    let repo = Repo::new("binary-to-text");
    repo.write("mixed.txt", b"bin\0ary");
    repo.commit("base");
    repo.write("mixed.txt", b"one\n");
    repo.git(&["add", "mixed.txt"]);
    repo.write("mixed.txt", b"two\n");

    let snapshot = repo.snapshot();
    let repository = repo.open();
    let mut diff = LineDiff::new(&repository).unwrap();
    assert_eq!(diff.counts(&snapshot, Path::new("mixed.txt")).unwrap(), None);
}

#[test]
fn forced_text_attributes_cannot_exceed_the_line_diff_budget() {
    let repo = Repo::new("large-text");
    repo.write(".gitattributes", b"large.dat diff\n");
    repo.commit("base");
    let oversized = vec![b'x'; crow_plugin_git::MAX_LINE_DIFF_BYTES as usize + 1];
    repo.write("large.dat", &oversized);
    repo.git(&["add", "large.dat"]);
    repo.write("large.dat", b"now small\n");

    let snapshot = repo.snapshot();
    let repository = repo.open();
    let mut diff = LineDiff::new(&repository).unwrap();
    assert_eq!(diff.counts(&snapshot, Path::new("large.dat")).unwrap(), None);
}

#[test]
fn linked_worktrees_and_raw_paths_keep_their_identity() {
    let repo = Repo::new("linked");
    repo.write("a.txt", b"a\n");
    repo.commit("base");
    let worktree = repo.dir.with_file_name("linked-wt");
    repo.git(&["worktree", "add", "-q", worktree.to_str().unwrap(), "-b", "feature"]);
    let linked = gix::open(&worktree).unwrap();
    let identity = crow_plugin_git::RepositoryIdentity::from_repo(&linked).unwrap();
    assert!(identity.linked_worktree);
    assert_ne!(identity.git_dir, identity.common_dir);
    let head = head_info(&linked).unwrap();
    assert_eq!(head.state, HeadState::Branch { name: "feature".into() });
    std::fs::write(worktree.join("a.txt"), b"changed\n").unwrap();
    let snapshot = status_snapshot(&linked, &AtomicBool::new(false)).unwrap();
    assert_eq!(snapshot.entries[Path::new("a.txt")].badge(), Some(StatusKind::Modified));

    // APFS rejects invalid UTF-8 names, so the live path check uses a valid
    // multibyte name; the raw-byte conversion itself is pinned by a unit test.
    let raw = OsString::from("🍎 project.txt");
    repo.write("bad.txt", b"x\n");
    std::fs::rename(repo.dir.join("bad.txt"), repo.dir.join(&raw)).unwrap();
    repo.git(&["add", "-A"]);
    repo.git(&["commit", "-q", "-m", "raw"]);
    std::fs::write(repo.dir.join(&raw), b"changed\n").unwrap();
    let snapshot = repo.snapshot();
    let path = PathBuf::from(raw);
    assert!(snapshot.entries.contains_key(&path));
    assert_eq!(path.as_os_str().as_bytes(), "🍎 project.txt".as_bytes());
}

#[test]
fn ahead_behind_peels_a_symbolic_upstream() {
    let repo = Repo::new("symbolic-upstream");
    repo.write("a.txt", b"base");
    repo.commit("base");
    repo.git(&["remote", "add", "origin", "."]);
    repo.git(&["update-ref", "refs/remotes/origin/other", "HEAD"]);
    repo.git(&["symbolic-ref", "refs/remotes/origin/main", "refs/remotes/origin/other"]);
    repo.git(&["config", "branch.main.remote", "origin"]);
    repo.git(&["config", "branch.main.merge", "refs/heads/main"]);
    repo.write("a.txt", b"local");
    repo.commit("local");
    assert_eq!(head_info(&repo.open()).unwrap().upstream, Some(crow_plugin_git::AheadBehind { ahead: 1, behind: 0 }));
}

#[test]
fn a_tracking_ref_that_is_not_a_commit_means_no_upstream() {
    let repo = Repo::new("broken-upstream");
    repo.write("a.txt", b"base");
    repo.commit("base");
    let blob = repo.git(&["hash-object", "-w", "a.txt"]);
    repo.git(&["remote", "add", "origin", "."]);
    std::fs::create_dir_all(repo.dir.join(".git/refs/remotes/origin")).unwrap();
    std::fs::write(repo.dir.join(".git/refs/remotes/origin/main"), format!("{}\n", blob.trim())).unwrap();
    repo.git(&["config", "branch.main.remote", "origin"]);
    repo.git(&["config", "branch.main.merge", "refs/heads/main"]);
    let head = head_info(&repo.open()).expect("a broken upstream must not fail the whole status");
    assert_eq!(head.upstream, None);
    assert_eq!(head.state, HeadState::Branch { name: "main".into() });
}

#[test]
fn staging_an_executable_bit_change_is_a_modification() {
    let repo = Repo::new("mode-change");
    repo.write("script", b"echo test");
    repo.write("target", b"target");
    repo.write("link", b"regular file");
    repo.commit("base");

    repo.git(&["update-index", "--chmod=+x", "script"]);
    std::fs::remove_file(repo.dir.join("link")).unwrap();
    std::os::unix::fs::symlink("target", repo.dir.join("link")).unwrap();
    repo.git(&["add", "link"]);

    let snapshot = repo.snapshot();
    assert_eq!(snapshot.entries[Path::new("script")].index, Some(StatusKind::Modified));
    assert_eq!(snapshot.entries[Path::new("link")].index, Some(StatusKind::TypeChange));
}

#[test]
fn heads_cover_detached_unborn_and_local_ahead_behind() {
    let detached = Repo::new("detached");
    detached.write("a.txt", b"a\n");
    detached.commit("base");
    detached.git(&["checkout", "-q", "--detach", "HEAD"]);
    let head = head_info(&detached.open()).unwrap();
    assert!(matches!(head.state, HeadState::Detached { .. }));

    let unborn = Repo::new("unborn");
    unborn.write("a.txt", b"a\n");
    let head = head_info(&unborn.open()).unwrap();
    assert_eq!(head.state, HeadState::Unborn { branch: "main".into() });
    assert_eq!(unborn.snapshot().entries[Path::new("a.txt")].badge(), Some(StatusKind::Untracked));

    let noupstream = Repo::new("noupstream");
    noupstream.write("a.txt", b"a\n");
    noupstream.commit("base");
    assert_eq!(head_info(&noupstream.open()).unwrap().upstream, None);

    let temp = tempfile::TempDir::new().unwrap();
    let bare = temp.path().join("bare.git");
    let clone = temp.path().join("clone");
    let other = temp.path().join("other");
    let mut init = Command::new("git");
    init.args(["init", "-q", "--bare", "-b", "main"]).arg(&bare);
    assert!(init.status().unwrap().success());
    let mut clone_cmd = Command::new("git");
    clone_cmd.args(["clone", "-q"]).arg(&bare).arg(&clone);
    assert!(clone_cmd.status().unwrap().success());
    let clone_repo = Repo { dir: clone.clone(), _temp: tempfile::TempDir::new().unwrap() };
    clone_repo.git(&["config", "user.name", "Crow Test"]);
    clone_repo.git(&["config", "user.email", "crow@example.invalid"]);
    clone_repo.write("a.txt", b"a\n");
    clone_repo.commit("base");
    clone_repo.git(&["push", "-q", "origin", "main"]);
    clone_repo.write("a.txt", b"b\n");
    clone_repo.commit("local");
    let head = head_info(&clone_repo.open()).unwrap();
    assert_eq!(head.upstream, Some(crow_plugin_git::AheadBehind { ahead: 1, behind: 0 }));

    let mut other_cmd = Command::new("git");
    other_cmd.args(["clone", "-q"]).arg(&bare).arg(&other);
    assert!(other_cmd.status().unwrap().success());
    let other_repo = Repo { dir: other.clone(), _temp: tempfile::TempDir::new().unwrap() };
    other_repo.git(&["config", "user.name", "Crow Test"]);
    other_repo.git(&["config", "user.email", "crow@example.invalid"]);
    other_repo.write("b.txt", b"c\n");
    other_repo.commit("remote");
    other_repo.git(&["push", "-q", "origin", "main"]);
    clone_repo.git(&["fetch", "-q", "origin"]);
    let head = head_info(&clone_repo.open()).unwrap();
    assert_eq!(head.upstream, Some(crow_plugin_git::AheadBehind { ahead: 1, behind: 1 }));
}

#[test]
fn discovery_prunes_packages_symlinks_and_keeps_nested_repos() {
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path().join("root");
    std::fs::create_dir_all(&root).unwrap();
    let make = |dir: &Path| {
        std::fs::create_dir_all(dir).unwrap();
        let mut init = Command::new("git");
        init.args(["init", "-q", "-b", "main"]).arg(dir);
        assert!(init.status().unwrap().success());
    };
    make(&root.join("a"));
    make(&root.join("b").join("nested"));
    make(&root.join("pkg.app").join("inner"));
    std::os::unix::fs::symlink(root.join("a"), root.join("link")).unwrap();

    let report = discover(std::slice::from_ref(&root), DiscoveryLimits::default(), &AtomicBool::new(false), |_| {});
    let paths: BTreeSet<PathBuf> = report.repositories.iter().map(|repo| repo.identity.worktree.clone()).collect();
    assert!(paths.contains(&std::fs::canonicalize(root.join("a")).unwrap()));
    assert!(paths.contains(&std::fs::canonicalize(root.join("b/nested")).unwrap()));
    assert!(!paths.iter().any(|path| path.starts_with(root.join("pkg.app"))));
    assert!(!paths.iter().any(|path| path == &root.join("link")));

    // A root that is itself a symlink to a folder is scanned.
    let via_link = temp.path().join("via-link");
    std::os::unix::fs::symlink(&root, &via_link).unwrap();
    let linked = discover(&[via_link], DiscoveryLimits::default(), &AtomicBool::new(false), |_| {});
    assert!(
        linked.repositories.iter().any(|repo| repo.identity.worktree == std::fs::canonicalize(root.join("a")).unwrap())
    );

    let bounded = discover(
        &[root],
        DiscoveryLimits { max_depth: 8, max_directories: 1, max_repositories: 8 },
        &AtomicBool::new(false),
        |_| {},
    );
    assert!(bounded.truncated);
    assert!(bounded.repositories.len() <= 1);
}

#[test]
fn cloud_backed_worktrees_are_identified_and_skip_status_reads() {
    assert!(crow_plugin_git::is_cloud_backed_worktree(Path::new(
        "/Users/me/Library/Mobile Documents/com~apple~CloudDocs/code/repo"
    )));
    assert!(crow_plugin_git::is_cloud_backed_worktree(Path::new("/Users/me/Library/CloudStorage/Dropbox/repo")));
    assert!(!crow_plugin_git::is_cloud_backed_worktree(Path::new("/Users/me/code/repo")));
    let repo = Repo::new("cloud");
    repo.write("a.txt", b"a\n");
    repo.commit("base");
    repo.write("a.txt", b"b\n");
    let snapshot = crow_plugin_git::unavailable_snapshot(&repo.open()).unwrap();
    assert!(snapshot.unavailable);
    assert!(snapshot.entries.is_empty());
    assert_eq!(snapshot.changed_count, 0);
    assert_eq!(snapshot.head.state, HeadState::Branch { name: "main".into() });
}

/// A gate that records every path it is asked about and refuses the ones named.
struct RecordingGate {
    refuse: Vec<&'static str>,
    asked: std::sync::Mutex<Vec<PathBuf>>,
}

impl RecordingGate {
    fn new(refuse: &[&'static str]) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self { refuse: refuse.to_vec(), asked: Default::default() })
    }
}

impl crow_fs::ReadGate for RecordingGate {
    fn check(&self, path: &Path, _: crow_fs::ReadHint) -> crow_fs::ReadDecision {
        self.asked.lock().unwrap().push(path.to_path_buf());
        if self.refuse.iter().any(|name| path.ends_with(name)) {
            crow_fs::ReadDecision::SkipCloud
        } else {
            crow_fs::ReadDecision::Readable
        }
    }
}

/// Rewrite `path` with same-length `bytes` and a different mtime, so the index
/// stat no longer matches and only a content hash can tell whether it changed.
fn rewrite_same_size(repo: &Repo, path: &str, bytes: &[u8]) {
    repo.write(path, bytes);
    let file = std::fs::File::options().write(true).open(repo.dir.join(path)).unwrap();
    file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(3600)).unwrap();
}

#[test]
fn gated_status_never_hashes_refused_files() {
    let repo = Repo::new("gated-status");
    repo.write("local.txt", b"aaa\n");
    repo.write("placeholder.txt", b"aaa\n");
    repo.commit("base");
    rewrite_same_size(&repo, "local.txt", b"bbb\n");
    rewrite_same_size(&repo, "placeholder.txt", b"bbb\n");

    let open = repo.open();
    let open_snapshot = status_snapshot(&open, &AtomicBool::new(false)).unwrap();
    assert!(open_snapshot.entries[Path::new("placeholder.txt")].worktree.is_some(), "an ungated read sees the edit");

    let gate = RecordingGate::new(&["placeholder.txt"]);
    let snapshot = crow_plugin_git::status_snapshot_gated(&open, &AtomicBool::new(false), gate.clone()).unwrap();
    assert_eq!(snapshot.entries[Path::new("local.txt")].worktree, Some(StatusKind::Modified));
    assert!(!snapshot.entries.contains_key(Path::new("placeholder.txt")), "a refused file stays unread and unchanged");
    let asked = gate.asked.lock().unwrap();
    assert!(asked.iter().any(|path| path.ends_with("placeholder.txt")), "the gate is consulted before the read");
}

#[test]
fn gated_line_diff_skips_refused_worktree_files() {
    use crow_plugin_git::DiffLayer;
    let repo = Repo::new("gated-diff");
    repo.write("local.txt", b"aaa\n");
    repo.write("placeholder.txt", b"aaa\n");
    repo.commit("base");
    repo.write("local.txt", b"bbb\n");
    repo.write("placeholder.txt", b"bbb\n");
    let snapshot = repo.snapshot();
    let open = repo.open();
    let gate = RecordingGate::new(&["placeholder.txt"]);
    let mut diff = LineDiff::with_gate(&open, gate.clone()).unwrap();
    assert_eq!(diff.counts(&snapshot, Path::new("placeholder.txt")).unwrap(), None);
    let layers = diff.diff(&snapshot, Path::new("placeholder.txt")).unwrap();
    assert!(matches!(layers.unstaged, DiffLayer::Unavailable));
    assert_eq!(
        diff.counts(&snapshot, Path::new("local.txt")).unwrap(),
        Some(crow_plugin_git::LineCounts { insertions: 1, removals: 1 })
    );
}
