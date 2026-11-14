//! The search itself: a parallel `ignore` walk, fuzzy name scoring through
//! `nucleo-matcher`, and literal/regex content search through `grep-searcher`.
//! Hits stream to the caller in batches; `cancel` stops the walk and reads.

use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::SystemTime;

use grep_matcher::Matcher as _;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkMatch};
use ignore::{WalkBuilder, WalkParallel, WalkState};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use unicode_segmentation::UnicodeSegmentation;

use crate::rules::RuleSet;

/// Hits are handed to the sink in batches of at least this many.
const BATCH: usize = 64;

/// What to look for. `contents` is literal and case-insensitive unless wrapped
/// in `/…/`, which makes it a regex. `everything` ignores every exclusion set.
pub struct Query {
    pub names: Option<String>,
    pub contents: Option<String>,
    pub show_hidden: bool,
    pub rules: RuleSet,
    pub everything: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub path: PathBuf,
    pub is_dir: bool,
    /// Higher is a better fuzzy name match; content hits score 0.
    pub score: u32,
    pub kind: HitKind,
    /// For the results table; read with the hit so the UI never stats on its thread.
    pub modified: Option<SystemTime>,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HitKind {
    /// The file name matched; `ranges` are the matched graphemes as byte ranges
    /// in the name.
    Name { ranges: Vec<Range<usize>> },
    /// One matching line: its 1-based number, the line without its terminator,
    /// and the byte ranges of the matches within `text`.
    Content { line: u64, text: String, ranges: Vec<Range<usize>> },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// Directory entries visited (files and folders).
    pub scanned: u64,
    /// Folders the rules removed, with the rule that removed each one.
    pub skipped: Vec<(PathBuf, String)>,
    /// Set when the query itself is invalid (a bad regex); no walk ran.
    pub error: Option<String>,
}

/// A reusable nucleo fuzzy matcher for names and paths.
pub struct FuzzyMatcher {
    needle: String,
    matcher: Matcher,
    haystack_buf: Vec<char>,
    needle_buf: Vec<char>,
}

impl FuzzyMatcher {
    pub fn new(needle: &str) -> FuzzyMatcher {
        let mut config = Config::DEFAULT;
        config.ignore_case = true;
        FuzzyMatcher {
            needle: needle.to_owned(),
            matcher: Matcher::new(config),
            haystack_buf: Vec::new(),
            needle_buf: Vec::new(),
        }
    }

    pub fn score(&mut self, haystack: &str) -> Option<u32> {
        let needle = Utf32Str::new(&self.needle, &mut self.needle_buf);
        let haystack = Utf32Str::new(haystack, &mut self.haystack_buf);
        self.matcher.fuzzy_match(haystack, needle).map(u32::from)
    }

    /// The score and the matched characters as byte ranges in `haystack`.
    /// Nucleo indexes its own `Utf32Str`: bytes for ASCII (and for non-ASCII
    /// strings whose every grapheme starts with an ASCII char), else one index
    /// per grapheme. Adjacent matches merge into one range.
    pub fn indices(&mut self, haystack: &str) -> Option<(u32, Vec<Range<usize>>)> {
        let needle = Utf32Str::new(&self.needle, &mut self.needle_buf);
        let haystack_utf32 = Utf32Str::new(haystack, &mut self.haystack_buf);
        let mut indices = Vec::new();
        let score = self.matcher.fuzzy_indices(haystack_utf32, needle, &mut indices)?;
        indices.sort_unstable();
        indices.dedup();
        let mut ranges: Vec<Range<usize>> = Vec::new();
        let mut push = |span: Range<usize>| {
            if span.is_empty() {
                return;
            }
            match ranges.last_mut() {
                Some(last) if last.end >= span.start => last.end = last.end.max(span.end),
                _ => ranges.push(span),
            }
        };
        if haystack.is_ascii() {
            for index in indices {
                let start = index as usize;
                push(start..start + 1);
            }
        } else {
            let graphemes: Vec<(usize, &str)> = haystack.grapheme_indices(true).collect();
            let ascii_starts =
                graphemes.iter().all(|(_, grapheme)| grapheme.chars().next().is_some_and(|c| c.is_ascii()));
            if ascii_starts {
                // Nucleo matched raw bytes; an index is the start of a grapheme,
                // widened to the whole grapheme (a hit on `e` covers `e\u{301}`).
                for index in indices {
                    let start = index as usize;
                    if let Some((byte, grapheme)) = graphemes.iter().find(|entry| entry.0 == start) {
                        push(*byte..*byte + grapheme.len());
                    }
                }
            } else {
                for index in indices {
                    if let Some(&(byte, grapheme)) = graphemes.get(index as usize) {
                        push(byte..byte + grapheme.len());
                    }
                }
            }
        }
        Some((u32::from(score), ranges))
    }
}

/// Walk `root`, streaming matching hits to `sink` in batches. `cancel` is
/// checked between entries and matches; the walk stops as soon as it is set.
pub fn search(root: &Path, query: &Query, mut sink: impl FnMut(Vec<Hit>), cancel: &Arc<AtomicBool>) -> Summary {
    let names = query.names.as_deref().filter(|q| !q.is_empty()).map(str::to_owned);
    let contents = query.contents.as_deref().filter(|q| !q.is_empty());
    let content_pattern = match contents {
        Some(text) => match ContentPattern::new(text) {
            Ok(pattern) => Some(pattern),
            Err(error) => return Summary { error: Some(error), ..Summary::default() },
        },
        None => None,
    };
    if names.is_none() && content_pattern.is_none() {
        return Summary::default();
    }

    let rules = Arc::new(if query.everything { RuleSet::empty() } else { query.rules.clone() });
    let skipped = Arc::new(Mutex::new(Vec::new()));
    let scanned = Arc::new(AtomicU64::new(0));
    let walker = build_walker(root, query.show_hidden, rules, skipped.clone(), cancel.clone());
    let (tx, rx) = mpsc::channel::<Vec<Hit>>();

    let scanned_thread = scanned.clone();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            walker.run(|| {
                let tx = tx.clone();
                let cancel = cancel.clone();
                let scanned = scanned_thread.clone();
                let names = names.clone();
                let pattern = content_pattern.clone();
                let mut fuzzy = names.as_deref().map(FuzzyMatcher::new);
                let mut content = pattern.as_ref().and_then(|p| ContentSearch::new(p).ok());
                Box::new(move |result| {
                    if cancel.load(Ordering::Relaxed) {
                        return WalkState::Quit;
                    }
                    let Ok(entry) = result else { return WalkState::Continue };
                    if entry.depth() == 0 {
                        return WalkState::Continue;
                    }
                    scanned.fetch_add(1, Ordering::Relaxed);
                    let is_dir = entry.file_type().is_some_and(|kind| kind.is_dir());
                    let metadata = entry.metadata().ok();
                    let modified = metadata.as_ref().and_then(|meta| meta.modified().ok());
                    let size = metadata.as_ref().map_or(0, |meta| meta.len());
                    let mut hits = Vec::new();
                    if let Some(fuzzy) = fuzzy.as_mut() {
                        let name = entry.file_name();
                        let name = name.to_string_lossy();
                        if let Some((score, ranges)) = fuzzy.indices(name.as_ref()) {
                            hits.push(Hit {
                                path: entry.path().to_path_buf(),
                                is_dir,
                                score,
                                kind: HitKind::Name { ranges },
                                modified,
                                size,
                            });
                        }
                    }
                    if let Some(content) = content.as_mut()
                        && !is_dir
                    {
                        content.search(entry.path(), modified, size, &mut hits, &cancel);
                    }
                    if !hits.is_empty() {
                        let _ = tx.send(hits);
                    }
                    WalkState::Continue
                })
            });
        });

        let mut pending = Vec::new();
        for batch in rx {
            pending.extend(batch);
            if pending.len() >= BATCH {
                sink(std::mem::take(&mut pending));
            }
            if cancel.load(Ordering::Relaxed) {
                break;
            }
        }
        if !pending.is_empty() {
            sink(pending);
        }
    });

    let mut skipped = skipped.lock().map(|mut list| std::mem::take(&mut *list)).unwrap_or_default();
    skipped.sort_by(|a, b| a.0.cmp(&b.0));
    Summary { scanned: scanned.load(Ordering::Relaxed), skipped, error: None }
}

fn build_walker(
    root: &Path,
    show_hidden: bool,
    rules: Arc<RuleSet>,
    skipped: Arc<Mutex<Vec<(PathBuf, String)>>>,
    cancel: Arc<AtomicBool>,
) -> WalkParallel {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(8));
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(!show_hidden)
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .parents(false)
        .follow_links(false)
        .threads(threads)
        .filter_entry(move |entry| {
            if cancel.load(Ordering::Relaxed) {
                return false;
            }
            if entry.depth() == 0 || !entry.file_type().is_some_and(|kind| kind.is_dir()) {
                return true;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(rule) = rules.skip(entry.path(), name.as_ref(), true) {
                if let Ok(mut list) = skipped.lock() {
                    list.push((entry.path().to_path_buf(), rule));
                }
                return false;
            }
            true
        });
    builder.build_parallel()
}

#[derive(Clone, Debug)]
struct ContentPattern {
    pattern: String,
    case_insensitive: bool,
}

impl ContentPattern {
    fn new(query: &str) -> Result<ContentPattern, String> {
        let (text, case_insensitive) = if query.len() >= 2 && query.starts_with('/') && query.ends_with('/') {
            (&query[1..query.len() - 1], false)
        } else {
            (query, true)
        };
        if text.is_empty() {
            return Err("the pattern is empty".to_owned());
        }
        let pattern = if case_insensitive { regex_syntax::escape(text) } else { text.to_owned() };
        RegexMatcherBuilder::new().case_insensitive(case_insensitive).build(&pattern).map_err(pattern_error)?;
        Ok(ContentPattern { pattern, case_insensitive })
    }
}

/// The last line of a regex error, without the engine's multi-line rewrite:
/// `"regex parse error:\n    (?:()\n    ^\nerror: unclosed group"` →
/// `"Invalid pattern: unclosed group"`.
fn pattern_error(e: impl std::fmt::Display) -> String {
    let text = e.to_string();
    let line = text.lines().map(str::trim).rfind(|line| !line.is_empty()).unwrap_or("the pattern");
    let line = line.strip_prefix("error: ").unwrap_or(line);
    format!("Invalid pattern: {line}")
}

struct ContentSearch {
    matcher: RegexMatcher,
    searcher: Searcher,
}

impl ContentSearch {
    fn new(pattern: &ContentPattern) -> Result<ContentSearch, String> {
        let matcher = RegexMatcherBuilder::new()
            .case_insensitive(pattern.case_insensitive)
            .build(&pattern.pattern)
            .map_err(pattern_error)?;
        let searcher = SearcherBuilder::new().line_number(true).binary_detection(BinaryDetection::quit(b'\0')).build();
        Ok(ContentSearch { matcher, searcher })
    }

    fn search(
        &mut self,
        path: &Path,
        modified: Option<SystemTime>,
        size: u64,
        hits: &mut Vec<Hit>,
        cancel: &Arc<AtomicBool>,
    ) {
        let mut sink = ContentSink { matcher: &self.matcher, path, modified, size, hits, cancel };
        let _ = self.searcher.search_path(&self.matcher, path, &mut sink);
    }
}

struct ContentSink<'a> {
    matcher: &'a RegexMatcher,
    path: &'a Path,
    modified: Option<SystemTime>,
    size: u64,
    hits: &'a mut Vec<Hit>,
    cancel: &'a Arc<AtomicBool>,
}

impl Sink for ContentSink<'_> {
    type Error = io::Error;

    fn matched(&mut self, _: &Searcher, mat: &SinkMatch<'_>) -> Result<bool, io::Error> {
        if self.cancel.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let raw = mat.bytes();
        let text = String::from_utf8_lossy(raw);
        let text = text.trim_end_matches(['\n', '\r']);
        if text.is_empty() {
            return Ok(true);
        }
        let mut ranges = Vec::new();
        self.matcher
            .find_iter(text.as_bytes(), |m| {
                if m.end() > m.start() {
                    ranges.push(m.start()..m.end());
                }
                true
            })
            .map_err(|e| io::Error::other(e.to_string()))?;
        self.hits.push(Hit {
            path: self.path.to_path_buf(),
            is_dir: false,
            score: 0,
            kind: HitKind::Content { line: mat.line_number().unwrap_or(0), text: text.to_owned(), ranges },
            modified: self.modified,
            size: self.size,
        });
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crow_core::Exclude;
    use std::collections::BTreeSet;
    use std::fs;

    fn root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("crow-search-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create root");
        dir
    }

    fn write(root: &Path, path: &str, bytes: &[u8]) {
        let path = root.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(path, bytes).expect("write fixture");
    }

    fn rules() -> RuleSet {
        RuleSet::from_config(&Exclude::default(), Path::new("/Users/nobody")).expect("default rules")
    }

    fn run(
        root: &Path,
        names: Option<&str>,
        contents: Option<&str>,
        show_hidden: bool,
        everything: bool,
    ) -> (Vec<Hit>, Summary) {
        let query = Query {
            names: names.map(str::to_owned),
            contents: contents.map(str::to_owned),
            show_hidden,
            rules: rules(),
            everything,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let mut hits = Vec::new();
        let summary = search(root, &query, |batch| hits.extend(batch), &cancel);
        hits.sort_by(|a, b| {
            a.path.cmp(&b.path).then_with(|| match (&a.kind, &b.kind) {
                (HitKind::Content { line: a, .. }, HitKind::Content { line: b, .. }) => a.cmp(b),
                _ => 0.cmp(&0),
            })
        });
        (hits, summary)
    }

    fn paths(hits: &[Hit]) -> Vec<PathBuf> {
        hits.iter().map(|hit| hit.path.clone()).collect::<BTreeSet<_>>().into_iter().collect()
    }

    fn ranges(start: usize, end: usize) -> Vec<Range<usize>> {
        std::iter::once(start..end).collect()
    }

    fn content(hits: &[Hit]) -> Vec<(u64, String, Vec<Range<usize>>)> {
        hits.iter()
            .filter_map(|hit| match &hit.kind {
                HitKind::Content { line, text, ranges } => Some((*line, text.clone(), ranges.clone())),
                HitKind::Name { .. } => None,
            })
            .collect()
    }

    fn dev_tree(name: &str) -> PathBuf {
        let root = root(name);
        write(&root, "keep.txt", b"needle\n");
        write(&root, "src/main.rs", b"fn main() {}\n");
        write(&root, "node_modules/pkg/index.js", b"needle\n");
        write(&root, "Cargo.toml", b"");
        write(&root, "target/debug/app", b"needle\n");
        write(&root, ".git/config", b"needle\n");
        write(&root, ".venv/bin/python", b"");
        write(&root, "__pycache__/x.pyc", b"");
        write(&root, ".logicprojects/data", b"needle\n");
        write(&root, "package.json", b"{}");
        write(&root, "build/out.js", b"needle\n");
        write(&root, "dist/out.js", b"needle\n");
        write(&root, "sub/build/notes.txt", b"needle\n");
        root
    }

    #[test]
    fn dev_rules_skip_and_count_folders() {
        let root = dev_tree("rules");
        let (hits, summary) = run(&root, None, Some("needle"), true, false);
        assert_eq!(paths(&hits), [root.join("keep.txt"), root.join("sub/build/notes.txt")]);
        let skipped: BTreeSet<_> = summary.skipped.iter().map(|(path, _)| path.clone()).collect();
        let expected: BTreeSet<_> =
            [".git", ".logicprojects", ".venv", "__pycache__", "build", "dist", "node_modules", "target"]
                .into_iter()
                .map(|name| root.join(name))
                .collect();
        assert_eq!(skipped, expected);
        assert!(summary.skipped.iter().any(|(path, rule)| {
            path == &root.join("target") && rule.contains("dev") && rule.contains("Cargo.toml")
        }));
        assert!(
            summary.skipped.iter().any(|(path, rule)| { path == &root.join("node_modules") && rule.contains("name") })
        );
        assert!(summary.scanned > 0);
    }

    #[test]
    fn everything_walks_the_skipped_folders() {
        let root = dev_tree("everything");
        let (hits, summary) = run(&root, None, Some("needle"), true, true);
        let expected: BTreeSet<_> = [
            ".git/config",
            ".logicprojects/data",
            "build/out.js",
            "dist/out.js",
            "keep.txt",
            "node_modules/pkg/index.js",
            "sub/build/notes.txt",
            "target/debug/app",
        ]
        .into_iter()
        .map(|path| root.join(path))
        .collect();
        assert_eq!(paths(&hits).into_iter().collect::<BTreeSet<_>>(), expected);
        assert!(summary.skipped.is_empty());
    }

    #[test]
    fn hidden_files_follow_the_setting() {
        let root = root("hidden");
        write(&root, ".secret.txt", b"needle\n");
        write(&root, "visible.txt", b"needle\n");
        let (hidden, _) = run(&root, None, Some("needle"), false, true);
        assert_eq!(paths(&hidden), [root.join("visible.txt")]);
        let (shown, _) = run(&root, None, Some("needle"), true, true);
        assert_eq!(paths(&shown), [root.join(".secret.txt"), root.join("visible.txt")]);
    }

    #[test]
    fn names_are_fuzzy_and_scored() {
        let root = dev_tree("names");
        let (hits, _) = run(&root, Some("mn"), None, false, false);
        let main = hits.iter().find(|hit| hit.path == root.join("src/main.rs")).expect("fuzzy match");
        assert!(main.score > 0);
        assert!(!main.is_dir);
    }

    #[test]
    fn name_hits_carry_match_ranges() {
        let root = root("name-ranges");
        write(&root, "config.rs", b"");
        write(&root, "r\u{e9}sum\u{e9}.txt", b"");
        let (hits, _) = run(&root, Some("cfg"), None, false, true);
        let hit = hits.iter().find(|hit| hit.path == root.join("config.rs")).expect("fuzzy name match");
        let HitKind::Name { ranges } = &hit.kind else { panic!("a name hit") };
        let matched: String = ranges.iter().map(|range| &"config.rs"[range.clone()]).collect();
        assert_eq!(matched, "cfg");

        let (hits, _) = run(&root, Some("sum"), None, false, true);
        let hit = hits.iter().find(|hit| hit.path == root.join("r\u{e9}sum\u{e9}.txt")).expect("multibyte match");
        let HitKind::Name { ranges } = &hit.kind else { panic!("a name hit") };
        let expected: Vec<Range<usize>> = std::iter::once(3..6).collect();
        assert_eq!(ranges, &expected);
        assert!(ranges.iter().all(|range| {
            let name = "r\u{e9}sum\u{e9}.txt";
            name.is_char_boundary(range.start) && name.is_char_boundary(range.end)
        }));
    }

    #[test]
    fn name_ranges_follow_nucleo_indexing() {
        fn matched(haystack: &str, needle: &str) -> Vec<String> {
            let mut matcher = FuzzyMatcher::new(needle);
            let (_, ranges) = matcher.indices(haystack).expect("a match");
            assert!(
                ranges
                    .iter()
                    .all(|range| haystack.is_char_boundary(range.start) && haystack.is_char_boundary(range.end)),
                "{haystack:?} gave ranges {ranges:?}"
            );
            ranges.iter().map(|range| haystack[range.clone()].to_owned()).collect()
        }
        assert_eq!(matched("Re\u{301}sume\u{301} final.txt", "final"), ["final"]);
        assert_eq!(matched("re\u{301}sume\u{301}.txt", "sum"), ["sum"]);
        assert_eq!(matched("e\u{301}x", "e"), ["e\u{301}"]);
        assert_eq!(matched("日本 notes.txt", "notes"), ["notes"]);
        assert_eq!(matched("r\u{e9}sum\u{e9}.txt", "sum"), ["sum"]);
    }

    #[test]
    fn fuzzy_indices_are_byte_ranges() {
        let haystack = "~/Files/Coding/git";
        let mut matcher = FuzzyMatcher::new("git");
        let (_, ranges) = matcher.indices(haystack).expect("a match");
        let start = haystack.len() - 3;
        assert_eq!(ranges, [start..start + 3], "the last three bytes, merged into one range");
    }

    #[test]
    fn fuzzy_indices_score_matches_score() {
        let haystack = "~/Files/notes.txt";
        let mut matcher = FuzzyMatcher::new("notes");
        let (score, _) = matcher.indices(haystack).expect("a match");
        assert_eq!(Some(score), matcher.score(haystack));
    }

    #[test]
    fn content_hits_report_line_text_and_ranges() {
        let root = root("content");
        write(&root, "lines.txt", b"alpha\nNEEDLE here\nbeta needle\n");
        let (hits, summary) = run(&root, None, Some("needle"), false, true);
        assert_eq!(summary.error, None);
        assert_eq!(
            content(&hits),
            [(2, "NEEDLE here".to_owned(), ranges(0, 6)), (3, "beta needle".to_owned(), ranges(5, 11)),]
        );
    }

    #[test]
    fn slash_wrapped_queries_are_regexes() {
        let root = root("regex");
        write(&root, "lines.txt", b"alpha\nneedle here\nbeta needle\n");
        let (hits, _) = run(&root, None, Some("/n(eed)le/"), false, true);
        assert_eq!(content(&hits).len(), 2);
        assert_eq!(content(&hits)[0].2, ranges(0, 6));
        assert_eq!(content(&hits)[1].2, ranges(5, 11));
    }

    #[test]
    fn pattern_errors_are_one_line() {
        let error = ContentPattern::new("/(/").unwrap_err();
        assert_eq!(error, "Invalid pattern: unclosed group");
        assert!(!error.contains('\n'), "{error}");
    }

    #[test]
    fn invalid_regex_reports_an_error_without_walking() {
        let root = root("badregex");
        write(&root, "x.txt", b"needle\n");
        let (hits, summary) = run(&root, None, Some("/[a/"), false, true);
        assert!(hits.is_empty());
        assert!(summary.error.is_some(), "invalid regex should report an error");
        assert_eq!(summary.scanned, 0);
    }

    #[test]
    fn binary_files_are_not_read() {
        let root = root("binary");
        write(&root, "bin.dat", b"needle\0tail");
        let (hits, _) = run(&root, None, Some("needle"), false, true);
        assert!(hits.is_empty(), "a NUL byte marks the file as binary");
    }

    #[test]
    fn cancel_before_the_walk_stops_immediately() {
        let root = dev_tree("cancel");
        let query = Query {
            names: None,
            contents: Some("needle".to_owned()),
            show_hidden: true,
            rules: rules(),
            everything: false,
        };
        let cancel = Arc::new(AtomicBool::new(true));
        let mut hits = Vec::new();
        let summary = search(&root, &query, |batch| hits.extend(batch), &cancel);
        assert!(hits.is_empty());
        assert_eq!(summary.scanned, 0);
    }

    #[test]
    fn cancel_mid_walk_stops_streaming() {
        let root = root("cancel-mid");
        for i in 0..4000 {
            write(&root, &format!("many/f{i:04}.txt"), b"needle\n");
        }
        let query = Query {
            names: None,
            contents: Some("needle".to_owned()),
            show_hidden: false,
            rules: rules(),
            everything: true,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let mut hits = Vec::new();
        let flag = cancel.clone();
        let summary = search(
            &root,
            &query,
            |batch| {
                hits.extend(batch);
                flag.store(true, Ordering::Relaxed);
            },
            &cancel,
        );
        assert!(hits.len() < 4000, "cancelling stopped the stream");
        assert!(summary.scanned < 4000, "the walk stopped early");
    }
}
