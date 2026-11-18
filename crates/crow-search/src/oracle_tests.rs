//! B5's independent oracle: seeded random trees, walked by a plain recursive
//! `read_dir` implementation of the documented search policy, compared as sets
//! with the real engine. Disagreement is the point: this module does not reuse
//! the walker, only the rule set and the fuzzy matcher the policy names.
//!
//! The policy under test is M8.5's Architecture section: root depth 0 never
//! matches; a rule-skipped real directory is neither matched nor entered; every
//! other entry is a fuzzy name hit; regular files and symlinks to regular files
//! are content hits (binary files skipped); a dot folder with hidden off is
//! matched then skipped, and a package likewise; symlinked folders are never
//! entered; an unreadable folder is a skip.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crow_core::Exclude;

use crate::{FuzzyMatcher, HitKind, Query, RuleSet, SkipReason, search};

/// How many trees to generate. The plan says 300; lower this (and record the
/// number in the progress file) if the test grows past ~5 s in debug.
const SEEDS: u64 = 25;

const QUERIES: [&str; 4] = ["ab", "a", ".a", "b-a"];

const NAMES: [&str; 8] = ["ab", "abc", ".ab", "b-a", "node_modules", "target", "x.app", ".hid"];

const CONTENTS: [&[u8]; 4] = [b"", b"ab\n", b"AB cd\n", b"\0ab"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Mode {
    All,
    Names,
    Contents,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum HitKey {
    Name,
    Content(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum SkipKey {
    Rule,
    HiddenFolder,
    Package,
    Unreadable,
}

type Hits = BTreeSet<(PathBuf, HitKey)>;
type Skips = BTreeSet<(PathBuf, SkipKey)>;

#[test]
fn oracle_agrees_on_random_trees() {
    let guard = crate::test_support::CleanTree::new("crow-search-oracle-");
    let root = guard.to_path_buf();
    let rules = Arc::new(
        RuleSet::from_config(&Exclude::default(), Path::new("/Users/nobody")).expect("default exclusion rules"),
    );
    let mut seen_skips = BTreeSet::new();
    let mut saw_name = false;
    let mut saw_content = false;

    for seed in 0..SEEDS {
        let tree = root.join(format!("seed-{seed}"));
        let manifest = build_tree(&tree, seed);
        for &query in &QUERIES {
            for hidden_on in [false, true] {
                for everything in [false, true] {
                    for mode in [Mode::All, Mode::Names, Mode::Contents] {
                        let (names, contents) = query_parts(query, mode);
                        let query = Query {
                            names: names.map(str::to_owned),
                            contents: contents.map(str::to_owned),
                            enter_hidden_folders: hidden_on,
                            include_hidden: true,
                            rules: rules.clone(),
                            everything,
                            read_gate: None,
                        };
                        let (engine_hits, engine_skips) = run_engine(&tree, &query);
                        let (oracle_hits, oracle_skips) = run_oracle(&tree, &query);
                        seen_skips.extend(engine_skips.iter().map(|(_, kind)| *kind));
                        saw_name |= engine_hits.iter().any(|(_, kind)| matches!(kind, HitKey::Name));
                        saw_content |= engine_hits.iter().any(|(_, kind)| matches!(kind, HitKey::Content(_)));
                        if engine_hits != oracle_hits || engine_skips != oracle_skips {
                            let diff = diff(&engine_hits, &oracle_hits, &engine_skips, &oracle_skips);
                            let tree_list = manifest.join("\n");
                            panic!(
                                "seed {seed}, query names={:?} contents={:?}, hidden {hidden_on}, everything {everything}, mode {mode:?}\n\
                                 {diff}\n\
                                 tree (relative to {}):\n{tree_list}",
                                query.names,
                                query.contents,
                                tree.display()
                            );
                        }
                    }
                }
            }
        }
        let _ = remove_tree(&tree);
    }

    // A green run is only meaningful if the generator reached every branch of
    // the policy; an empty seed list must fail here, not pass vacuously.
    for kind in [SkipKey::Rule, SkipKey::HiddenFolder, SkipKey::Package] {
        assert!(seen_skips.contains(&kind), "the random trees never produced a {kind:?} skip");
    }
    if unsafe { libc::geteuid() } != 0 {
        assert!(seen_skips.contains(&SkipKey::Unreadable), "the random trees never produced an unreadable folder");
    }
    assert!(saw_name, "the random trees never produced a name hit");
    assert!(saw_content, "the random trees never produced a content hit");
}

fn query_parts(text: &str, mode: Mode) -> (Option<&str>, Option<&str>) {
    match mode {
        Mode::All => (Some(text), Some(text)),
        Mode::Names => (Some(text), None),
        Mode::Contents => (None, Some(text)),
    }
}

fn run_engine(root: &Path, query: &Query) -> (Hits, Skips) {
    let control = crate::Control::new();
    let mut raw = Vec::new();
    let summary = search(root, query, |batch| raw.extend(batch), &control);
    assert!(summary.error.is_none(), "the engine failed: {:?}", summary.error);
    let hits = raw
        .into_iter()
        .map(|hit| {
            let kind = match hit.kind {
                HitKind::Name { .. } => HitKey::Name,
                HitKind::Content { line, .. } => HitKey::Content(line),
            };
            (hit.path, kind)
        })
        .collect();
    let skips = summary
        .skipped
        .into_iter()
        .map(|skip| {
            let kind = match skip.reason {
                SkipReason::Rule(_) => SkipKey::Rule,
                SkipReason::HiddenFolder => SkipKey::HiddenFolder,
                SkipReason::Package => SkipKey::Package,
                SkipReason::Unreadable(_) => SkipKey::Unreadable,
            };
            (skip.path, kind)
        })
        .collect();
    (hits, skips)
}

struct Oracle {
    enter_hidden: bool,
    visit_hidden: bool,
    enter_packages: bool,
    rules: Arc<RuleSet>,
    matcher: Option<FuzzyMatcher>,
    needle: Option<String>,
}

fn run_oracle(root: &Path, query: &Query) -> (Hits, Skips) {
    let names = query.names.as_deref().filter(|name| !name.is_empty());
    let contents = query.contents.as_deref().filter(|contents| !contents.is_empty());
    let mut oracle = Oracle {
        enter_hidden: query.enter_hidden_folders || query.everything,
        visit_hidden: query.include_hidden || query.enter_hidden_folders,
        enter_packages: query.everything,
        rules: if query.everything { Arc::new(RuleSet::empty()) } else { Arc::clone(&query.rules) },
        matcher: names.map(FuzzyMatcher::new),
        needle: contents.map(str::to_lowercase),
    };
    let mut hits = Hits::new();
    let mut skips = Skips::new();
    oracle.walk(root, &mut hits, &mut skips);
    (hits, skips)
}

impl Oracle {
    fn walk(&mut self, dir: &Path, hits: &mut Hits, skips: &mut Skips) {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => {
                skips.insert((dir.to_path_buf(), SkipKey::Unreadable));
                return;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !self.visit_hidden && name.starts_with('.') {
                continue;
            }
            let file_type = entry.file_type().ok();
            let is_dir = file_type.is_some_and(|kind| kind.is_dir());
            let is_file = file_type.is_some_and(|kind| kind.is_file());
            let is_symlink = file_type.is_some_and(|kind| kind.is_symlink());

            // Rules run first and only for real directories: a skipped folder is
            // neither matched nor entered.
            if is_dir && let Some(_rule) = self.rules.skip(&path, &name, true) {
                skips.insert((path, SkipKey::Rule));
                continue;
            }
            if let Some(matcher) = self.matcher.as_mut()
                && matcher.indices(&name).is_some()
            {
                hits.insert((path.clone(), HitKey::Name));
            }
            if let Some(needle) = self.needle.as_deref() {
                let readable = is_file || (is_symlink && fs::metadata(&path).is_ok_and(|meta| meta.is_file()));
                if readable && let Some(line) = content_line(&path, needle) {
                    hits.insert((path.clone(), HitKey::Content(line)));
                }
            }
            if !is_dir {
                continue;
            }
            if !self.enter_hidden && name.starts_with('.') {
                skips.insert((path, SkipKey::HiddenFolder));
                continue;
            }
            if !self.enter_packages && is_package(&name) {
                skips.insert((path, SkipKey::Package));
                continue;
            }
            self.walk(&path, hits, skips);
        }
    }
}

/// A package is a directory with one of Finder's bundle extensions; the engine
/// has the same list in `crow-fs`, but this oracle spells it out from the policy.
fn is_package(name: &str) -> bool {
    [".app", ".bundle", ".framework", ".photoslibrary"].iter().any(|suffix| name.ends_with(suffix))
}

/// The first matching line, 1-based, or `None`. Binary files are skipped whole,
/// like `BinaryDetection::quit(b'\0')`; the query is a case-insensitive literal.
fn content_line(path: &Path, needle: &str) -> Option<u64> {
    let bytes = fs::read(path).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    for (index, raw) in bytes.split(|byte| *byte == b'\n').enumerate() {
        let text = String::from_utf8_lossy(raw);
        let text = text.trim_end_matches('\r');
        if text.to_lowercase().contains(needle) {
            return Some(index as u64 + 1);
        }
    }
    None
}

fn diff(engine_hits: &Hits, oracle_hits: &Hits, engine_skips: &Skips, oracle_skips: &Skips) -> String {
    let mut out = String::new();
    if engine_hits != oracle_hits {
        out.push_str("hits the engine has and the oracle does not:\n");
        for key in engine_hits.difference(oracle_hits) {
            out.push_str(&format!("  + {:?}\n", display(key)));
        }
        out.push_str("hits the oracle has and the engine does not:\n");
        for key in oracle_hits.difference(engine_hits) {
            out.push_str(&format!("  - {:?}\n", display(key)));
        }
    }
    if engine_skips != oracle_skips {
        out.push_str("skips the engine has and the oracle does not:\n");
        for key in engine_skips.difference(oracle_skips) {
            out.push_str(&format!("  + {:?}\n", display_skip(key)));
        }
        out.push_str("skips the oracle has and the engine does not:\n");
        for key in oracle_skips.difference(engine_skips) {
            out.push_str(&format!("  - {:?}\n", display_skip(key)));
        }
    }
    out
}

fn display(key: &(PathBuf, HitKey)) -> (PathBuf, String) {
    let kind = match key.1 {
        HitKey::Name => "name".to_owned(),
        HitKey::Content(line) => format!("content line {line}"),
    };
    (key.0.clone(), kind)
}

fn display_skip(key: &(PathBuf, SkipKey)) -> (PathBuf, &'static str) {
    let kind = match key.1 {
        SkipKey::Rule => "rule",
        SkipKey::HiddenFolder => "hidden folder",
        SkipKey::Package => "package",
        SkipKey::Unreadable => "unreadable",
    };
    (key.0.clone(), kind)
}

/// One seeded tree of at most 40 entries and depth 4. Returns a listing for the
/// failure message.
fn build_tree(root: &Path, seed: u64) -> Vec<String> {
    fs::create_dir_all(root).expect("create the tree root");
    let mut rng = Rng::new(seed);
    // Leave room for two sentinels, two symlinks and an unreadable folder with a child.
    let count = 1 + rng.below(34);
    let mut dirs = vec![(root.to_path_buf(), 0usize)];
    let mut files: Vec<PathBuf> = Vec::new();
    let mut manifest = Vec::new();
    let mut created = 0;
    let mut attempts = 0;
    while created < count && attempts < count * 8 {
        attempts += 1;
        let (parent, depth) = dirs[rng.below(dirs.len())].clone();
        let name = NAMES[rng.below(NAMES.len())];
        let path = parent.join(name);
        if path.exists() {
            continue;
        }
        if depth < 3 && rng.below(100) < 45 {
            fs::create_dir(&path).expect("create a tree folder");
            manifest.push(format!("d {}", relative(root, &path)));
            dirs.push((path, depth + 1));
        } else {
            let content = CONTENTS[rng.below(CONTENTS.len())];
            fs::write(&path, content).expect("write a tree file");
            manifest.push(format!("f {} {:?}", relative(root, &path), content));
            files.push(path);
        }
        created += 1;
    }

    // A Cargo.toml sentinel makes a sibling `target` a rule skip. Write one in a
    // random folder; when a `target` exists, make sure its parent has one too so
    // the sentinel path is exercised on purpose, not by luck.
    let sentinel = dirs[rng.below(dirs.len())].0.clone();
    write_sentinel(root, &sentinel, &mut manifest);
    if let Some(target) = dirs.iter().find(|(path, _)| path.file_name() == Some(OsStr::new("target")))
        && let Some(parent) = target.0.parent()
    {
        write_sentinel(root, parent, &mut manifest);
    }

    // A symlink to a sibling file and (when possible) to a sibling folder.
    if let Some(target) = pick(&mut rng, &files) {
        let link = target.parent().unwrap_or(root).join("link-file");
        write_symlink(root, &link, target, &mut manifest);
    }
    if let Some((target, _)) = pick(&mut rng, &dirs[1..]) {
        let link = target.parent().unwrap_or(root).join("link-dir");
        write_symlink(root, &link, target, &mut manifest);
    }

    // Some trees have a folder the walk cannot read. Running as root, `read_dir`
    // succeeds anyway, so do not create one: the oracle would rightly agree that
    // there is no unreadable skip, but the case would test nothing.
    if unsafe { libc::geteuid() } != 0 && rng.below(100) < 50 {
        let parents: Vec<_> = dirs.iter().filter(|(_, depth)| *depth < 3).collect();
        let (parent, _) = parents[rng.below(parents.len())];
        let path = parent.join("000");
        if fs::symlink_metadata(&path).is_err() {
            fs::create_dir(&path).expect("create the unreadable folder");
            fs::write(path.join("inside.txt"), b"ab\n").expect("write inside the unreadable folder");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("chmod the unreadable folder");
            manifest.push(format!("d {} (mode 000)", relative(root, &path)));
            manifest.push(format!("f {}/inside.txt {:?}", relative(root, &path), b"ab\n"));
        }
    }

    manifest
}

fn write_symlink(root: &Path, link: &Path, target: &Path, manifest: &mut Vec<String>) {
    let name = target.file_name().expect("sibling name");
    symlink(name, link).expect("create the sibling symlink");
    manifest.push(format!("l {} -> {}", relative(root, link), name.to_string_lossy()));
}

fn write_sentinel(root: &Path, dir: &Path, manifest: &mut Vec<String>) {
    let path = dir.join("Cargo.toml");
    if path.exists() {
        return;
    }
    fs::write(&path, b"").expect("write the sentinel");
    manifest.push(format!("f {} (sentinel)", relative(root, &path)));
}

fn pick<'a, T>(rng: &mut Rng, items: &'a [T]) -> Option<&'a T> {
    (!items.is_empty()).then(|| &items[rng.below(items.len())])
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).display().to_string()
}

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        // `seed` is 0 for the first tree; xorshift cannot start from zero.
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    fn below(&mut self, limit: usize) -> usize {
        (self.next() % limit as u64) as usize
    }
}

/// Remove a tree that may hold a mode-000 folder: restore directory modes first.
fn remove_tree(root: &Path) -> std::io::Result<()> {
    crate::test_support::remove_tree(root)
}
