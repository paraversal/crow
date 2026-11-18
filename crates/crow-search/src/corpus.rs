//! A deterministic search corpus: the user's `backup`/dot-name report plus
//! exclusion sets, a package, symlinks, a FIFO and an unreadable folder.
//!
//! `build` writes it, `remove` takes it away, and `cases` names what the engine
//! must find, miss or report as skipped. `crates/crow-search/tests/corpus_cases.rs`
//! runs the cases through the real walker; B2–B4 flip the rows the table marks.

use std::ffi::CString;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;

/// One search and its expected outcome. Paths are relative to the corpus root.
pub struct Case {
    pub id: &'static str,
    /// The folder searched; `""` is the corpus root.
    pub root: &'static str,
    pub names: Option<&'static str>,
    pub contents: Option<&'static str>,
    pub show_hidden: bool,
    pub everything: bool,
    pub target: &'static str,
    pub expect: Expect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expect {
    /// The target is in the hits.
    Hit,
    /// Reachable, but no hit.
    Miss,
    /// Not a hit; a skip for this ancestor (relative) exists.
    UnderSkip(&'static str),
    /// The target is a hit and the walk also recorded a skip for this ancestor
    /// (a dot folder whose name matched but was not descended into).
    HitAndSkip(&'static str),
    /// Not a hit and no skip: the walk never visited it.
    NotVisited,
}

/// Write the corpus at `root` (creating or overwriting it). Deterministic:
/// fixed bytes, files 0644, folders 0755 except the unreadable one at 000.
pub fn build(root: &Path) -> io::Result<()> {
    // io-ok: CLI fixture and tests, never the app's main thread
    fs::create_dir_all(root)?;
    let dirs = [
        "backup",
        "backup/nested",
        ".backup",
        "Backups",
        "backup-excludes",
        "savegameBackup",
        "tm-backup",
        ".git",
        "node_modules",
        "node_modules/backup-pkg",
        "target",
        "target/debug",
        "build",
        "dist",
        "CrowBackup.app",
        "CrowBackup.app/Contents",
        "CrowBackup.app/Contents/MacOS",
        "unreadable",
        "empty",
        "crowd",
    ];
    for dir in dirs {
        let path = root.join(dir);
        fs::create_dir_all(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
    }
    let files = [
        "backup/backup.txt",
        "backup/back.txt",
        "backup/nested/backup-note.txt",
        ".backup/.backup-inside.txt",
        "Backups/backup-plan.md",
        "backup-excludes/backup-rules.toml",
        "savegameBackup/savegame.txt",
        "tm-backup/tm-backup.txt",
        "do-not-backup.txt",
        ".backup.txt",
        ".backup_excludes",
        "back.txt",
        "backup.txt",
        "Cargo.toml",
        "package.json",
        ".git/config",
        "node_modules/backup-pkg/backup-index.js",
        "target/debug/backup-binary",
        "build/backup-out.js",
        "dist/backup-bundle.js",
        "CrowBackup.app/Contents/Info.plist",
        "CrowBackup.app/Contents/MacOS/CrowBackup",
        "unreadable/backup-hidden.txt",
    ];
    for file in files {
        let path = root.join(file);
        fs::write(&path, b"needle\n")?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644))?;
    }
    // 300 weak `.back` name matches: enough to fill one 256-hit batch, with no
    // `u` after the `k`, so `.backu` finds none of them.
    for i in 0..300 {
        let path = root.join("crowd").join(format!("note.b-a-c-k-{i:03}.txt"));
        fs::write(&path, b"needle\n")?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644))?;
    }
    // The one contents-only case: no `needle`, `backup` in the text.
    let notes = root.join("notes.txt");
    fs::write(&notes, b"backup in contents\n")?;
    fs::set_permissions(&notes, fs::Permissions::from_mode(0o644))?;

    for (name, target) in [
        ("link-to-backup", "backup"),
        ("link-to-backup.txt", "backup.txt"),
        ("dangling", "missing-target"),
        ("loop-a", "loop-b"),
        ("loop-b", "loop-a"),
    ] {
        remove_stale(&root.join(name))?;
        symlink(target, root.join(name))?;
    }

    let pipe = CString::new(root.join("pipe").as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a NUL in the corpus path"))?;
    remove_stale(&root.join("pipe"))?;
    // SAFETY: `pipe` is a valid NUL-terminated path under `root`.
    if unsafe { libc::mkfifo(pipe.as_ptr(), 0o644) } != 0 {
        return Err(io::Error::last_os_error());
    }

    // Last: once it is 000, nothing below it can be written.
    fs::set_permissions(root.join("unreadable"), fs::Permissions::from_mode(0o000))?;
    Ok(())
}

/// Remove a stale entry so `build` can run again over its own output. A
/// directory there is left alone: these entries must be files, links or a FIFO.
fn remove_stale(path: &Path) -> io::Result<()> {
    // io-ok: CLI fixture and tests, never the app's main thread
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Restore the unreadable folder's mode and remove the tree; never follows links.
pub fn remove(root: &Path) -> io::Result<()> {
    let unreadable = root.join("unreadable");
    // io-ok: CLI fixture and tests, never the app's main thread
    if unreadable.symlink_metadata().is_ok_and(|meta| meta.is_dir()) {
        let _ = fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o755));
    }
    // io-ok: CLI fixture and tests, never the app's main thread
    fs::remove_dir_all(root)
}

/// The named cases. Default rules from `Exclude::default()`; the corpus test
/// uses one rule set for all of them.
pub fn cases() -> &'static [Case] {
    &[
        Case {
            id: "backup-folder",
            root: "",
            names: Some("backup"),
            contents: Some("backup"),
            show_hidden: true,
            everything: false,
            target: "backup/backup.txt",
            expect: Expect::Hit,
        },
        Case {
            id: "backup-file",
            root: "",
            names: Some("backup"),
            contents: None,
            show_hidden: false,
            everything: false,
            target: "backup.txt",
            expect: Expect::Hit,
        },
        Case {
            id: "user-backup-excludes",
            root: "",
            names: Some(".backu"),
            contents: Some(".backu"),
            show_hidden: true,
            everything: false,
            target: ".backup_excludes",
            expect: Expect::Hit,
        },
        Case {
            id: "back-prefix",
            root: "",
            names: Some("back"),
            contents: None,
            show_hidden: false,
            everything: false,
            target: "backup.txt",
            expect: Expect::Hit,
        },
        Case {
            id: "backup-not-back",
            root: "",
            names: Some("backup"),
            contents: None,
            show_hidden: false,
            everything: false,
            target: "back.txt",
            expect: Expect::Miss,
        },
        Case {
            id: "backup-content",
            root: "",
            names: None,
            contents: Some("backup"),
            show_hidden: false,
            everything: false,
            target: "notes.txt",
            expect: Expect::Hit,
        },
        Case {
            id: "backup-hidden-file",
            root: "",
            names: Some("backup"),
            contents: None,
            show_hidden: false,
            everything: false,
            target: ".backup.txt",
            expect: Expect::Hit,
        },
        Case {
            id: "backup-hidden-folder",
            root: "",
            names: Some("backup"),
            contents: None,
            show_hidden: false,
            everything: false,
            target: ".backup/.backup-inside.txt",
            expect: Expect::UnderSkip(".backup"),
        },
        Case {
            id: "hidden-folder-name",
            root: "",
            names: Some("backup"),
            contents: None,
            show_hidden: false,
            everything: false,
            target: ".backup",
            expect: Expect::HitAndSkip(".backup"),
        },
        Case {
            id: "hidden-folder-everything",
            root: "",
            names: Some("backup"),
            contents: None,
            show_hidden: false,
            everything: true,
            target: ".backup/.backup-inside.txt",
            expect: Expect::Hit,
        },
        Case {
            id: "backup-hidden-shown",
            root: "",
            names: Some("backup"),
            contents: None,
            show_hidden: true,
            everything: false,
            target: ".backup.txt",
            expect: Expect::Hit,
        },
        Case {
            id: "backup-excluded-name",
            root: "",
            names: Some("backup"),
            contents: None,
            show_hidden: false,
            everything: false,
            target: "node_modules/backup-pkg/backup-index.js",
            expect: Expect::UnderSkip("node_modules"),
        },
        Case {
            id: "backup-excluded-sentinel",
            root: "",
            names: Some("backup"),
            contents: None,
            show_hidden: false,
            everything: false,
            target: "target/debug/backup-binary",
            expect: Expect::UnderSkip("target"),
        },
        Case {
            id: "backup-everything",
            root: "",
            names: Some("backup"),
            contents: None,
            show_hidden: false,
            everything: true,
            target: "node_modules/backup-pkg/backup-index.js",
            expect: Expect::Hit,
        },
        Case {
            id: "backup-outside-folder",
            root: "backup",
            names: Some("backup"),
            contents: None,
            show_hidden: false,
            everything: false,
            target: "backup.txt",
            expect: Expect::Miss,
        },
        Case {
            id: "backup-symlink-child",
            root: "",
            names: Some("backup"),
            contents: None,
            show_hidden: false,
            everything: false,
            target: "link-to-backup/backup.txt",
            expect: Expect::NotVisited,
        },
        Case {
            id: "backup-symlink-name",
            root: "",
            names: Some("backup"),
            contents: None,
            show_hidden: false,
            everything: false,
            target: "link-to-backup",
            expect: Expect::Hit,
        },
        Case {
            id: "symlink-content",
            root: "",
            names: None,
            contents: Some("needle"),
            show_hidden: false,
            everything: false,
            target: "link-to-backup.txt",
            expect: Expect::Hit,
        },
        Case {
            id: "backup-package",
            root: "",
            names: Some("backup"),
            contents: None,
            show_hidden: false,
            everything: false,
            target: "CrowBackup.app",
            expect: Expect::Hit,
        },
        Case {
            id: "package-inside",
            root: "",
            names: None,
            contents: Some("needle"),
            show_hidden: false,
            everything: false,
            target: "CrowBackup.app/Contents/Info.plist",
            expect: Expect::UnderSkip("CrowBackup.app"),
        },
        Case {
            id: "package-everything",
            root: "",
            names: None,
            contents: Some("needle"),
            show_hidden: false,
            everything: true,
            target: "CrowBackup.app/Contents/Info.plist",
            expect: Expect::Hit,
        },
        Case {
            id: "unreadable",
            root: "",
            names: Some("backup"),
            contents: None,
            show_hidden: false,
            everything: false,
            target: "unreadable/backup-hidden.txt",
            expect: Expect::UnderSkip("unreadable"),
        },
        Case {
            id: "fifo-finishes",
            root: "",
            names: None,
            contents: Some("needle"),
            show_hidden: false,
            everything: false,
            target: "pipe",
            expect: Expect::Miss,
        },
    ]
}
