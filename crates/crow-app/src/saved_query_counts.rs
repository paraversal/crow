//! A bounded, cancellable count for a saved query the sidebar shows but is not
//! viewing. The visible destination streams its own count; this covers the rest
//! one query at a time.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crow_core::{Exclude, SavedExclusions, SavedMatch, SavedQuery, SavedScope};
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
    cancel: &Arc<AtomicBool>,
    read_gate: Arc<dyn crow_fs::ReadGate>,
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
            count_engine(&root, query, text, home, exclude, cancel, read_gate)
        }
    }
}

fn count_engine(
    root: &std::path::Path,
    query: &SavedQuery,
    text: &str,
    home: &std::path::Path,
    exclude: &Exclude,
    cancel: &Arc<AtomicBool>,
    read_gate: Arc<dyn crow_fs::ReadGate>,
) -> CountResult {
    let predicates = predicates_for_saved(query);
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
        everything: query.exclusions == SavedExclusions::Everything,
        read_gate: Some(read_gate),
        predicates,
        tag_reader: Some(crate::tags::reader()),
    };
    let control = Control::with_parent_cancel(cancel.clone());
    let mut paths: HashSet<PathBuf> = HashSet::new();
    let mut partial = false;
    let summary = crow_search::search(
        root,
        &query,
        |batch| {
            for hit in batch {
                if paths.len() == COUNT_CAP && !paths.contains(&hit.path) {
                    partial = true;
                    control.cancel();
                    return;
                }
                paths.insert(hit.path);
            }
        },
        &control,
    );
    CountResult {
        count: paths.len() as u64,
        partial: partial
            || summary.capped
            || summary.cancelled
            || summary.error.is_some()
            || summary.tag_errors > 0
            || summary.skipped.iter().any(|skip| matches!(skip.reason, crow_search::SkipReason::Unreadable(_))),
    }
}

fn count_spotlight(query: &SavedQuery, text: &str, predicates: &Predicates, cancel: &AtomicBool) -> CountResult {
    let run_started = std::time::SystemTime::now();
    let mut paths: HashSet<PathBuf> = HashSet::new();
    let mut partial = false;
    let local_cancel = AtomicBool::new(false);
    let mut sink = |batch: Vec<crow_macos::SpotlightHit>| {
        if cancel.load(Ordering::Relaxed) {
            local_cancel.store(true, Ordering::Relaxed);
            return;
        }
        let known_tags = (!predicates.tags.is_empty()).then_some(predicates.tags.as_slice());
        for hit in batch {
            if predicates.matches_with_tags(&hit.path, hit.is_dir, hit.modified, run_started, known_tags) {
                if paths.len() == COUNT_CAP && !paths.contains(&hit.path) {
                    partial = true;
                    local_cancel.store(true, Ordering::Relaxed);
                    return;
                }
                paths.insert(hit.path);
            }
        }
    };
    let result = if text.is_empty() || !predicates.tags.is_empty() {
        match spotlight_expression_for(predicates, text) {
            Some(expression) => crow_macos::spotlight_expression(&expression, &mut sink, &local_cancel),
            None => return CountResult::default(),
        }
    } else if query.matching == SavedMatch::Contents {
        // Spotlight cannot read contents; this combination is rejected by the
        // config validator, so a hand-edited file simply counts nothing here.
        return CountResult { count: 0, partial: false };
    } else {
        crow_macos::spotlight_names(text, &mut sink, &local_cancel)
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
            tags: Vec::new(),
        };
        let count = count_saved_query(
            &query,
            &root,
            &Exclude::default(),
            &Arc::new(AtomicBool::new(false)),
            Arc::new(crow_fs::AllowAll),
        );
        assert_eq!(count, CountResult { count: 2, partial: false });
    }

    struct RefuseAll(std::sync::atomic::AtomicUsize);

    impl crow_fs::ReadGate for RefuseAll {
        fn check(&self, _: &std::path::Path, _: crow_fs::ReadHint) -> crow_fs::ReadDecision {
            self.0.fetch_add(1, Ordering::Relaxed);
            crow_fs::ReadDecision::SkipCloud
        }
    }

    #[test]
    fn background_counts_consult_the_read_gate_before_content_reads() {
        let guard = tempfile::Builder::new().prefix("crow-count-gate-").tempdir().unwrap();
        let root = guard.path().to_path_buf();
        std::fs::write(root.join("note.txt"), "needle").unwrap();
        let query = SavedQuery {
            id: SavedQueryId::new("needle").unwrap(),
            name: "Needle".into(),
            scope: SavedScope::Folder,
            root: Some(root.display().to_string()),
            text: "needle".into(),
            matching: crow_core::SavedMatch::Contents,
            include_hidden: true,
            enter_hidden_folders: false,
            exclusions: crow_core::SavedExclusions::Configured,
            kind: None,
            modified_within_days: None,
            tags: Vec::new(),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let allowed = count_saved_query(&query, &root, &Exclude::default(), &cancel, Arc::new(crow_fs::AllowAll));
        assert_eq!(allowed.count, 1, "an ordinary local file is searched");
        let gate = Arc::new(RefuseAll(Default::default()));
        let refused = count_saved_query(&query, &root, &Exclude::default(), &cancel, gate.clone());
        assert!(gate.0.load(Ordering::Relaxed) > 0, "the gate is asked before a content read");
        assert_eq!(refused.count, 0, "a refused file is not opened, so it cannot match its contents");
    }
}
