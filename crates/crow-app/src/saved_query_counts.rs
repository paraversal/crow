//! A bounded, cancellable count for a saved query the sidebar shows but is not
//! viewing. The visible destination streams its own count; this covers the rest
//! one query at a time.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crow_core::{Exclude, SavedMatch, SavedQuery, SavedScope};
use crow_search::{Control, Predicates, Query, RuleSet};

use crate::search::{predicates_for_saved, spotlight_expression_for};

/// Counts are for a sidebar label, not a result list: stop after this many
/// unique paths and mark the label partial.
pub const COUNT_CAP: usize = 20_000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CountResult {
    pub count: u64,
    pub partial: bool,
}

pub fn count_saved_query(
    query: &SavedQuery,
    home: &std::path::Path,
    exclude: &Exclude,
    cancel: &AtomicBool,
) -> CountResult {
    if cancel.load(Ordering::Relaxed) {
        return CountResult::default();
    }
    let predicates = predicates_for_saved(query);
    let text = query.text.trim();
    match query.scope {
        SavedScope::ThisMac => count_spotlight(query, text, &predicates, cancel),
        SavedScope::Folder | SavedScope::Home => {
            let root = match query.scope {
                SavedScope::Folder => match query.root_path(home) {
                    Ok(Some(root)) => root,
                    _ => return CountResult::default(),
                },
                _ => home.to_path_buf(),
            };
            count_engine(&root, query, text, &predicates, home, exclude, cancel)
        }
    }
}

fn count_engine(
    root: &std::path::Path,
    query: &SavedQuery,
    text: &str,
    predicates: &Predicates,
    home: &std::path::Path,
    exclude: &Exclude,
    cancel: &AtomicBool,
) -> CountResult {
    let rules = match RuleSet::from_config(exclude, home) {
        Ok(rules) => Arc::new(rules),
        Err(_) => return CountResult { count: 0, partial: true },
    };
    let (names, contents) = match query.matching {
        SavedMatch::All => (Some(text.to_owned()), Some(text.to_owned())),
        SavedMatch::Names => (Some(text.to_owned()), None),
        SavedMatch::Contents => (None, Some(text.to_owned())),
    };
    let query = Query {
        names: names.filter(|text| !text.is_empty()),
        contents: contents.filter(|text| !text.is_empty()),
        enter_hidden_folders: query.enter_hidden_folders,
        include_hidden: query.include_hidden,
        rules,
        everything: false,
        read_gate: None,
        predicates: predicates.clone(),
    };
    let control = Control::new();
    let mut paths: HashSet<PathBuf> = HashSet::new();
    let mut partial = false;
    let summary = crow_search::search(
        root,
        &query,
        |batch| {
            for hit in batch {
                if paths.len() >= COUNT_CAP {
                    partial = true;
                    control.cancel();
                    return;
                }
                paths.insert(hit.path);
            }
            if cancel.load(Ordering::Relaxed) {
                control.cancel();
            }
        },
        &control,
    );
    CountResult { count: paths.len() as u64, partial: partial || summary.capped || summary.cancelled }
}

fn count_spotlight(query: &SavedQuery, text: &str, predicates: &Predicates, cancel: &AtomicBool) -> CountResult {
    let run_started = std::time::SystemTime::now();
    let mut paths: HashSet<PathBuf> = HashSet::new();
    let mut partial = false;
    let mut sink = |batch: Vec<crow_macos::SpotlightHit>| {
        for hit in batch {
            if paths.len() >= COUNT_CAP {
                partial = true;
                cancel.store(true, Ordering::Relaxed);
                return;
            }
            if predicates.matches(&hit.path, hit.is_dir, hit.modified, run_started) {
                paths.insert(hit.path);
            }
        }
    };
    let result = if text.is_empty() {
        match spotlight_expression_for(predicates) {
            Some(expression) => crow_macos::spotlight_expression(&expression, &mut sink, cancel),
            None => return CountResult::default(),
        }
    } else if query.matching == SavedMatch::Contents {
        // Spotlight cannot read contents; this combination is rejected by the
        // config validator, so a hand-edited file simply counts nothing here.
        return CountResult { count: 0, partial: false };
    } else {
        crow_macos::spotlight_names(text, &mut sink, cancel)
    };
    CountResult { count: paths.len() as u64, partial: partial || result.is_err() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crow_core::{SavedQueryId, SavedScope};

    #[test]
    fn counts_a_saved_query_without_the_ui() {
        let dir = tempfile::Builder::new().prefix("crow-saved-count-").tempdir().expect("temp");
        let root = dir.path().canonicalize().expect("canonical");
        std::fs::write(root.join("a.pdf"), b"x").expect("write");
        std::fs::write(root.join("b.txt"), b"x").expect("write");
        std::fs::create_dir(root.join("sub")).expect("mkdir");
        std::fs::write(root.join("sub/c.pdf"), b"x").expect("write");
        let query = SavedQuery {
            id: SavedQueryId::new("pdfs").unwrap(),
            name: "PDFs".into(),
            scope: SavedScope::Folder,
            root: Some(root.display().to_string()),
            text: String::new(),
            matching: crow_core::SavedMatch::Names,
            include_hidden: true,
            enter_hidden_folders: false,
            exclusions: crow_core::SavedExclusions::Configured,
            kind: Some(crow_core::SavedKind::Pdf),
            modified_within_days: None,
        };
        let count = count_saved_query(&query, &root, &Exclude::default(), &AtomicBool::new(false));
        assert_eq!(count, CountResult { count: 2, partial: false });
    }
}
