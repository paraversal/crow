//! The named corpus cases, run through the real engine. Each search gets a
//! 10 s deadline, so a walker that blocks (a FIFO, an unreadable folder) fails
//! the case instead of hanging CI.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use crow_core::Exclude;
use crow_search::corpus::{self, Case, Expect};
use crow_search::{Control, Query, RuleSet, search};

#[test]
fn corpus_cases_match_the_engine() {
    let guard = tempfile::Builder::new().prefix("crow-search-corpus-").tempdir().expect("corpus parent");
    let root = guard.path().canonicalize().expect("canonical corpus root");
    corpus::build(&root).expect("build the corpus");
    let _root = CorpusRoot(root.clone());
    let rules = Arc::new(RuleSet::from_config(&Exclude::default(), Path::new("/Users/nobody")).expect("default rules"));
    let mut failures = Vec::new();
    for case in corpus::cases() {
        if case.id == "unreadable" && std::fs::read_dir(root.join("unreadable")).is_ok() {
            println!("corpus case unreadable skipped: read_dir succeeds as this user (root?)");
            continue;
        }
        if let Err(failure) = run_case(&root, case, &rules) {
            failures.push(format!("{}: {failure}", case.id));
        }
    }
    assert!(failures.is_empty(), "corpus cases failed:\n{}", failures.join("\n"));
}

#[test]
fn corpus_can_be_rebuilt_over_itself() {
    let guard = tempfile::Builder::new().prefix("crow-search-corpus-rebuild-").tempdir().expect("corpus parent");
    let root = guard.path().canonicalize().expect("canonical corpus root");
    corpus::build(&root).expect("first build");
    let _root = CorpusRoot(root.clone());
    corpus::build(&root).expect("second build over the same folder");
}

/// Removes the corpus (restoring the mode-000 folder) even when a case fails.
struct CorpusRoot(PathBuf);

impl Drop for CorpusRoot {
    fn drop(&mut self) {
        let _ = corpus::remove(&self.0);
    }
}

fn run_case(root: &Path, case: &Case, rules: &Arc<RuleSet>) -> Result<(), String> {
    let search_root = root.join(case.root);
    let query = Query {
        names: case.names.map(str::to_owned),
        contents: case.contents.map(str::to_owned),
        enter_hidden_folders: case.show_hidden,
        include_hidden: true,
        rules: rules.clone(),
        everything: case.everything,
        read_gate: None,
    };
    let (tx, rx) = mpsc::channel();
    let walked: PathBuf = search_root.clone();
    let handle = std::thread::spawn(move || {
        let control = Control::new();
        let mut hits = Vec::new();
        let summary = search(&walked, &query, |batch| hits.extend(batch), &control);
        let _ = tx.send((hits, summary));
    });
    let (hits, summary) = match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(found) => found,
        Err(mpsc::RecvTimeoutError::Timeout) => return Err("the search did not finish within 10 s".to_owned()),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let _ = handle.join();
            return Err("the search thread panicked".to_owned());
        }
    };
    handle.join().map_err(|_| "the search thread panicked".to_owned())?;
    if let Some(error) = &summary.error {
        return Err(format!("the query failed: {error}"));
    }

    let target = root.join(case.target);
    let hit = hits.iter().any(|hit| hit.path == target);
    match case.expect {
        Expect::Hit => {
            if hit {
                Ok(())
            } else {
                Err(format!("no hit for {}", target.display()))
            }
        }
        Expect::Miss => {
            if hit {
                Err(format!("unexpected hit for {}", target.display()))
            } else {
                Ok(())
            }
        }
        Expect::UnderSkip(ancestor) => {
            if hit {
                return Err(format!("unexpected hit for {}", target.display()));
            }
            let ancestor = root.join(ancestor);
            if summary.skipped.iter().any(|skip| skip.path == ancestor) {
                Ok(())
            } else {
                let skips: Vec<String> =
                    summary.skipped.iter().map(|skip| format!("{} ({})", skip.path.display(), skip.reason)).collect();
                Err(format!("no skip for {}; skips: {skips:?}", ancestor.display()))
            }
        }
        Expect::HitAndSkip(ancestor) => {
            if !hit {
                return Err(format!("no hit for {}", target.display()));
            }
            let ancestor = root.join(ancestor);
            if summary.skipped.iter().any(|skip| skip.path == ancestor) {
                Ok(())
            } else {
                Err(format!("hit for {}, but no skip for {}", target.display(), ancestor.display()))
            }
        }
        Expect::NotVisited => {
            if hit {
                return Err(format!("unexpected hit for {}", target.display()));
            }
            let mut ancestor = target.parent();
            while let Some(parent) = ancestor {
                if parent == root {
                    break;
                }
                if summary.skipped.iter().any(|skip| skip.path == parent) {
                    return Err(format!("{} is under skipped {}", target.display(), parent.display()));
                }
                ancestor = parent.parent();
            }
            Ok(())
        }
    }
}
