//! The search itself: a parallel `ignore` walk, fuzzy name scoring through
//! `nucleo-matcher`, and literal/regex content search through `grep-searcher`.
//! Hits stream to the caller in batches; `Control` stops the walk or just the
//! content reads.

use std::io;
use std::ops::Range;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use grep_matcher::Matcher as _;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkMatch};
use ignore::{WalkBuilder, WalkParallel, WalkState};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use unicode_segmentation::UnicodeSegmentation;

use crate::rules::RuleSet;

/// Hits are handed to the sink in batches of at most this many. A single file
/// with a great many matching lines can produce more hits than this; sending
/// them all at once makes the UI freeze while it drains one batch.
const BATCH: usize = 256;

/// How long a hit may wait in `pending` before it is sent, once the first batch is out.
const FLUSH: Duration = Duration::from_millis(30);

/// Decides when pending hits go to the sink. The first hit goes at once (time to
/// first result); after that a batch goes when `BATCH` hits are pending or `FLUSH`
/// has passed since the last send. Pure: the caller passes the clock.
struct Batcher {
    pending: Vec<Hit>,
    last_sent: Option<Instant>,
}

impl Batcher {
    fn new() -> Self {
        Batcher { pending: Vec::new(), last_sent: None }
    }

    /// Add hits; returns the batches to send now (each at most `BATCH` long).
    fn push(&mut self, hits: Vec<Hit>, now: Instant) -> Vec<Vec<Hit>> {
        self.pending.extend(hits);
        let mut ready = Vec::new();
        if self.pending.is_empty() {
            return ready;
        }
        let Some(last_sent) = self.last_sent else {
            // The first result must not wait for a full batch or a timer.
            while !self.pending.is_empty() {
                ready.push(self.take_batch());
            }
            self.last_sent = Some(now);
            return ready;
        };
        while self.pending.len() >= BATCH {
            ready.push(self.take_batch());
        }
        if !ready.is_empty() {
            self.last_sent = Some(now);
        }
        if !self.pending.is_empty() && now.saturating_duration_since(last_sent) >= FLUSH {
            ready.push(std::mem::take(&mut self.pending));
            self.last_sent = Some(now);
        }
        ready
    }

    /// Called when the receiver times out: the pending hits if `FLUSH` has passed.
    fn due(&mut self, now: Instant) -> Option<Vec<Hit>> {
        if self.pending.is_empty() {
            return None;
        }
        let due = self.last_sent.is_none_or(|last| now.saturating_duration_since(last) >= FLUSH);
        if due {
            self.last_sent = Some(now);
            Some(std::mem::take(&mut self.pending))
        } else {
            None
        }
    }

    /// The walk ended: whatever is left.
    fn finish(self) -> Option<Vec<Hit>> {
        (!self.pending.is_empty()).then_some(self.pending)
    }

    /// The first `BATCH` hits: one huge file can make `pending` huge.
    fn take_batch(&mut self) -> Vec<Hit> {
        if self.pending.len() <= BATCH {
            return std::mem::take(&mut self.pending);
        }
        let rest = self.pending.split_off(BATCH);
        std::mem::replace(&mut self.pending, rest)
    }
}

/// Shared between the caller and a running search.
#[derive(Clone, Debug, Default)]
pub struct Control {
    cancel: Arc<AtomicBool>,
    stop_contents: Arc<AtomicBool>,
}

impl Control {
    pub fn new() -> Self {
        Self::default()
    }

    /// Stop the walk and every read.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// The walk's cancel flag, for APIs that take one (Spotlight's `mdfind`).
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    /// Keep walking for names, but read no more file contents (and stop the current read).
    pub fn stop_contents(&self) {
        self.stop_contents.store(true, Ordering::Relaxed);
    }

    pub fn contents_stopped(&self) -> bool {
        self.stop_contents.load(Ordering::Relaxed)
    }
}

/// What to look for. `contents` is literal and case-insensitive unless wrapped
/// in `/…/`, which makes it a regex. `everything` ignores every exclusion set.
pub struct Query {
    pub names: Option<String>,
    pub contents: Option<String>,
    /// Descend into dot folders. Dot-named entries are visited and matched
    /// while `include_hidden` is on; this only decides whether the walk goes
    /// inside them.
    pub enter_hidden_folders: bool,
    /// Visit dot-named entries at all while Show Hidden Files is off. This is
    /// the `search_includes_hidden` setting; Show Hidden Files on overrides it.
    /// Search everything ignores exclude rules, not this setting.
    pub include_hidden: bool,
    /// Compiled once per `exclude.toml` generation and shared by every search
    /// that starts from it: the glob compile is ~14 ms on the main thread.
    pub rules: Arc<RuleSet>,
    pub everything: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub path: PathBuf,
    pub is_dir: bool,
    /// A dot name, or the macOS Finder hidden flag (`UF_HIDDEN`).
    pub hidden: bool,
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
    /// The first matching line: its 1-based number, the line without its
    /// terminator, and the byte ranges of the matches within `text`. `more` is
    /// how many further lines matched: the results table shows one excerpt per
    /// file, and carrying every line made huge files produce millions of hits.
    Content { line: u64, text: String, ranges: Vec<Range<usize>>, more: u64 },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// Directory entries visited (files and folders).
    pub scanned: u64,
    /// Folders the walk did not enter, and why.
    pub skipped: Vec<Skip>,
    /// The walk kept running for names after contents were stopped.
    pub contents_stopped: bool,
    /// Set when the query itself is invalid (a bad regex); no walk ran.
    pub error: Option<String>,
}

/// A folder the walk did not enter, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skip {
    pub path: PathBuf,
    pub reason: SkipReason,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// An exclude.toml set: `"dev: name \"node_modules\""` (the rule's text).
    Rule(String),
    /// A dot folder the walk did not descend into (Show Hidden Files is off).
    HiddenFolder,
    /// An application bundle or other package: a name hit, not a folder to walk.
    Package,
    /// The folder could not be read: `"no permission"`, or the error text.
    Unreadable(String),
}

impl SkipReason {
    /// Whether Search everything would walk it.
    pub fn lifted_by_everything(&self) -> bool {
        !matches!(self, SkipReason::Unreadable(_))
    }
}

impl std::fmt::Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkipReason::Rule(text) | SkipReason::Unreadable(text) => f.write_str(text),
            SkipReason::HiddenFolder => f.write_str("hidden folder"),
            SkipReason::Package => f.write_str("package"),
        }
    }
}

/// A pasted query can be a 100k-file clipboard (~10 MB). Nucleo scans the
/// needle per candidate; cap it (findings #25). The palette caps its own query
/// at 1024; the search field shares this matcher, so keep its long-needle test
/// (4000 chars) meaningful.
const MAX_NEEDLE_CHARS: usize = 4096;

/// A reusable nucleo fuzzy matcher for names and paths.
pub struct FuzzyMatcher {
    needle: String,
    /// The needle in nucleo's grapheme form, built once. Empty when the needle is
    /// ASCII (nucleo borrows the bytes directly). `Utf32Str::new` walks the whole
    /// string, so rebuilding it per candidate made a long query quadratic.
    needle_chars: Vec<char>,
    matcher: Matcher,
    haystack_buf: Vec<char>,
}

impl FuzzyMatcher {
    pub fn new(needle: &str) -> FuzzyMatcher {
        let mut config = Config::DEFAULT;
        config.ignore_case = true;
        // With `ignore_case`, nucleo compares against a lowercased haystack and
        // expects the needle lowercased too; an uppercase needle letter can pass
        // its prefilter and then trip an assert ("should have been caught by prefilter").
        // Cap first: a 100k-file clipboard is ~10 MB and made scoring quadratic.
        let needle: String = needle.chars().take(MAX_NEEDLE_CHARS).collect();
        let needle = needle.to_lowercase();
        let mut needle_chars = Vec::new();
        if !needle.is_ascii() {
            let _ = Utf32Str::new(&needle, &mut needle_chars);
            // A non-ASCII string whose graphemes all start with ASCII is matched as
            // bytes; keep the buffer empty so the matcher picks the same form.
            if needle_chars.iter().all(|c| c.is_ascii()) {
                needle_chars.clear();
            }
        }
        FuzzyMatcher { needle, needle_chars, matcher: Matcher::new(config), haystack_buf: Vec::new() }
    }

    pub fn score(&mut self, haystack: &str) -> Option<u32> {
        let needle = if self.needle_chars.is_empty() {
            Utf32Str::Ascii(self.needle.as_bytes())
        } else {
            Utf32Str::Unicode(&self.needle_chars)
        };
        let haystack = Utf32Str::new(haystack, &mut self.haystack_buf);
        self.matcher.fuzzy_match(haystack, needle).map(u32::from)
    }

    /// The score and the matched characters as byte ranges in `haystack`.
    /// Nucleo indexes its own `Utf32Str`: bytes for ASCII (and for non-ASCII
    /// strings whose every grapheme starts with an ASCII char), else one index
    /// per grapheme. Adjacent matches merge into one range.
    pub fn indices(&mut self, haystack: &str) -> Option<(u32, Vec<Range<usize>>)> {
        let needle = if self.needle_chars.is_empty() {
            Utf32Str::Ascii(self.needle.as_bytes())
        } else {
            Utf32Str::Unicode(&self.needle_chars)
        };
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

/// Walk `root`, streaming matching hits to `sink` in batches. `control` is
/// checked between entries and matches; cancelling stops the walk, while
/// stopping contents keeps names streaming.
pub fn search(root: &Path, query: &Query, mut sink: impl FnMut(Vec<Hit>), control: &Control) -> Summary {
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

    let rules = if query.everything { Arc::new(RuleSet::empty()) } else { Arc::clone(&query.rules) };
    let enter_hidden = query.enter_hidden_folders || query.everything;
    // Search everything ignores exclude.toml; Show Hidden Files is its own setting,
    // so it must not smuggle dot entries back in when search_includes_hidden is off.
    let visit_hidden = query.include_hidden || query.enter_hidden_folders;
    let enter_packages = query.everything;
    let skipped = Arc::new(Mutex::new(Vec::new()));
    let scanned = Arc::new(AtomicU64::new(0));
    let walker = build_walker(root, visit_hidden, rules, skipped.clone(), control.clone());
    let (tx, rx) = mpsc::channel::<Vec<Hit>>();

    let scanned_thread = scanned.clone();
    let skipped_thread = skipped.clone();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            walker.run(|| {
                let tx = tx.clone();
                let control = control.clone();
                let scanned = scanned_thread.clone();
                let skipped = skipped_thread.clone();
                let names = names.clone();
                let pattern = content_pattern.clone();
                let mut fuzzy = names.as_deref().map(FuzzyMatcher::new);
                let mut content = pattern.as_ref().and_then(|p| ContentSearch::new(p).ok());
                Box::new(move |result| {
                    if control.is_cancelled() {
                        return WalkState::Quit;
                    }
                    let entry = match result {
                        Ok(entry) => entry,
                        Err(error) => {
                            // A folder the walk could not read is a gap the user must see,
                            // not silence: record it with its reason.
                            if let Some(path) = error_path(&error)
                                && let Ok(mut list) = skipped.lock()
                            {
                                list.push(Skip {
                                    path: path.to_path_buf(),
                                    reason: SkipReason::Unreadable(error_text(&error)),
                                });
                            }
                            return WalkState::Continue;
                        }
                    };
                    if entry.depth() == 0 {
                        return WalkState::Continue;
                    }
                    scanned.fetch_add(1, Ordering::Relaxed);
                    let file_type = entry.file_type();
                    let is_dir = file_type.is_some_and(|kind| kind.is_dir());
                    let metadata = entry.metadata().ok();
                    let modified = metadata.as_ref().and_then(|meta| meta.modified().ok());
                    let size = metadata.as_ref().map_or(0, |meta| meta.len());
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    let hidden = is_hidden(name.as_ref(), metadata.as_ref());
                    if let Some(fuzzy) = fuzzy.as_mut()
                        && let Some((score, ranges)) = fuzzy.indices(name.as_ref())
                    {
                        // Send the name hit before this file's contents are read: a large
                        // file must not delay its own name, or the results that follow it.
                        let _ = tx.send(vec![Hit {
                            path: entry.path().to_path_buf(),
                            is_dir,
                            hidden,
                            score,
                            kind: HitKind::Name { ranges },
                            modified,
                            size,
                        }]);
                    }
                    // Contents only from regular files, and symlinks to regular files. A
                    // FIFO with no writer blocks its reader forever, which would stall the
                    // whole search; sockets and devices are no better.
                    let readable = file_type.is_some_and(|kind| kind.is_file())
                        || (file_type.is_some_and(|kind| kind.is_symlink())
                            && std::fs::metadata(entry.path()).is_ok_and(|meta| meta.is_file()));
                    if let Some(content) = content.as_mut()
                        && readable
                        && !control.contents_stopped()
                    {
                        let mut content_hits = Vec::new();
                        let matches = content.search(entry.path(), modified, size, hidden, &mut content_hits, &control);
                        if matches > 1
                            && let Some(Hit { kind: HitKind::Content { more, .. }, .. }) = content_hits.first_mut()
                        {
                            *more = matches - 1;
                        }
                        if !content_hits.is_empty() {
                            let _ = tx.send(content_hits);
                        }
                    }
                    // A dot folder is visited, so its name matches, but not descended
                    // into while Show Hidden Files is off: one visible skip instead of
                    // walking every dot folder in the home folder. Search everything enters it.
                    if is_dir && !enter_hidden && name.starts_with('.') {
                        if let Ok(mut list) = skipped.lock() {
                            list.push(Skip { path: entry.path().to_path_buf(), reason: SkipReason::HiddenFolder });
                        }
                        return WalkState::Skip;
                    }
                    // A package is a name hit, not a folder to walk; Finder opens it
                    // rather than browsing it. Search everything looks inside.
                    if is_dir && !enter_packages && crow_fs::is_package_name(name.as_ref()) {
                        if let Ok(mut list) = skipped.lock() {
                            list.push(Skip { path: entry.path().to_path_buf(), reason: SkipReason::Package });
                        }
                        return WalkState::Skip;
                    }
                    WalkState::Continue
                })
            });
        });

        let mut batcher = Batcher::new();
        loop {
            let ready = match rx.recv_timeout(FLUSH) {
                Ok(hits) => batcher.push(hits, Instant::now()),
                Err(RecvTimeoutError::Timeout) => batcher.due(Instant::now()).into_iter().collect(),
                Err(RecvTimeoutError::Disconnected) => break,
            };
            for batch in ready {
                sink(batch);
                if control.is_cancelled() {
                    break;
                }
            }
            if control.is_cancelled() {
                break;
            }
        }
        if !control.is_cancelled()
            && let Some(rest) = batcher.finish()
        {
            sink(rest);
        }
    });

    let mut skipped = skipped.lock().map(|mut list| std::mem::take(&mut *list)).unwrap_or_default();
    skipped.sort_by(|a, b| a.path.cmp(&b.path));
    Summary {
        scanned: scanned.load(Ordering::Relaxed),
        skipped,
        contents_stopped: control.contents_stopped(),
        error: None,
    }
}

fn build_walker(
    root: &Path,
    visit_hidden: bool,
    rules: Arc<RuleSet>,
    skipped: Arc<Mutex<Vec<Skip>>>,
    control: Control,
) -> WalkParallel {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(8));
    let mut builder = WalkBuilder::new(root);
    builder
        // When dot entries are visited, the visit closure decides whether to
        // descend into a dot folder and records the skip when it does not.
        .hidden(!visit_hidden)
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .parents(false)
        .follow_links(false)
        .threads(threads)
        .filter_entry(move |entry| {
            if control.is_cancelled() {
                return false;
            }
            if entry.depth() == 0 || !entry.file_type().is_some_and(|kind| kind.is_dir()) {
                return true;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(rule) = rules.skip(entry.path(), name.as_ref(), true) {
                if let Ok(mut list) = skipped.lock() {
                    list.push(Skip { path: entry.path().to_path_buf(), reason: SkipReason::Rule(rule) });
                }
                return false;
            }
            true
        });
    builder.build_parallel()
}

/// The same "hidden" the list uses: a dot name, or the macOS Finder hidden
/// flag (`UF_HIDDEN`) on the metadata the walk already read.
fn is_hidden(name: &str, metadata: Option<&std::fs::Metadata>) -> bool {
    if name.starts_with('.') {
        return true;
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt as _;
        metadata.is_some_and(|meta| meta.st_flags() & libc::UF_HIDDEN != 0)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = metadata;
        false
    }
}

/// The path an `ignore` walk error is about, when it carries one.
fn error_path(error: &ignore::Error) -> Option<&Path> {
    match error {
        ignore::Error::WithDepth { err, .. } => error_path(err),
        ignore::Error::WithLineNumber { err, .. } => error_path(err),
        ignore::Error::WithPath { path, .. } => Some(path),
        _ => None,
    }
}

/// The reason text for a folder the walk could not read.
fn error_text(error: &ignore::Error) -> String {
    match error.io_error() {
        Some(io_error) if io_error.kind() == io::ErrorKind::PermissionDenied => "no permission".to_owned(),
        Some(io_error) => io_error.to_string(),
        None => error.to_string(),
    }
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

/// Check between reads too: a nonmatching file never calls `ContentSink::matched`.
struct ContentReader<'a, R> {
    inner: R,
    control: &'a Control,
}

impl<R: io::Read> io::Read for ContentReader<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.control.is_cancelled() || self.control.contents_stopped() {
            return Ok(0);
        }
        self.inner.read(buffer)
    }
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

    #[allow(clippy::too_many_arguments)]
    fn search(
        &mut self,
        path: &Path,
        modified: Option<SystemTime>,
        size: u64,
        hidden: bool,
        hits: &mut Vec<Hit>,
        control: &Control,
    ) -> u64 {
        // A file or symlink can be replaced after the walk checked its type.
        // A nonblocking open keeps a replacement FIFO from hanging the walk.
        let Ok(file) = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(path) else {
            return 0;
        };
        if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
            return 0;
        }
        let reader = ContentReader { inner: file, control };
        let mut sink = ContentSink { matcher: &self.matcher, path, modified, size, hidden, hits, control, matches: 0 };
        let _ = self.searcher.search_reader(&self.matcher, reader, &mut sink);
        sink.matches
    }
}

struct ContentSink<'a> {
    matcher: &'a RegexMatcher,
    path: &'a Path,
    modified: Option<SystemTime>,
    size: u64,
    hidden: bool,
    hits: &'a mut Vec<Hit>,
    control: &'a Control,
    /// Matching lines seen in this file; only the first becomes a `Hit`.
    matches: u64,
}

impl Sink for ContentSink<'_> {
    type Error = io::Error;

    fn matched(&mut self, _: &Searcher, mat: &SinkMatch<'_>) -> Result<bool, io::Error> {
        if self.control.is_cancelled() || self.control.contents_stopped() {
            return Ok(false);
        }
        self.matches += 1;
        // The first line is the excerpt the UI shows; the rest are only counted.
        if self.matches > 1 {
            return Ok(true);
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
            hidden: self.hidden,
            score: 0,
            kind: HitKind::Content { line: mat.line_number().unwrap_or(0), text: text.to_owned(), ranges, more: 0 },
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

    fn root(name: &str) -> crate::test_support::CleanTree {
        crate::test_support::CleanTree::new(&format!("crow-search-{name}-"))
    }

    fn write(root: &Path, path: &str, bytes: &[u8]) {
        let path = root.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(path, bytes).expect("write fixture");
    }

    fn rules() -> Arc<RuleSet> {
        Arc::new(RuleSet::from_config(&Exclude::default(), Path::new("/Users/nobody")).expect("default rules"))
    }

    fn assert_active_content_read_stops(text: &[u8], cancel: bool) {
        struct StopAfterRead<'a> {
            remaining: &'a [u8],
            control: &'a Control,
            cancel: bool,
            reads: usize,
        }
        impl io::Read for StopAfterRead<'_> {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                self.reads += 1;
                let count = self.remaining.read(buffer)?;
                if self.cancel {
                    self.control.cancel();
                } else {
                    self.control.stop_contents();
                }
                Ok(count)
            }
        }

        let control = Control::new();
        let bytes = text.repeat(100_000);
        let mut input = StopAfterRead { remaining: &bytes, control: &control, cancel, reads: 0 };
        let mut content = ContentSearch::new(&ContentPattern::new("needle").expect("pattern")).expect("searcher");
        let mut hits = Vec::new();
        let mut sink = ContentSink {
            matcher: &content.matcher,
            path: Path::new("active.txt"),
            modified: None,
            size: bytes.len() as u64,
            hidden: false,
            hits: &mut hits,
            control: &control,
            matches: 0,
        };
        let reader = ContentReader { inner: &mut input, control: &control };
        content.searcher.search_reader(&content.matcher, reader, &mut sink).expect("controlled read");
        assert_eq!(sink.matches, 0, "buffered matches stop too");
        assert!(hits.is_empty());
        assert_eq!(input.reads, 1, "no further underlying reads after either flag");
        assert!(!input.remaining.is_empty(), "the nonmatching tail was not drained");

        // Early EOF must not leave the reusable searcher's buffers stopped.
        let root = root(&format!("active-read-{cancel}-{}", text.len()));
        let path = root.join("next.txt");
        write(&root, "next.txt", b"needle\n");
        let control = Control::new();
        assert_eq!(content.search(&path, None, 7, false, &mut hits, &control), 1);
        assert_eq!(hits[0].path, path);
        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[test]
    fn active_nonmatching_content_reads_stop_between_buffers() {
        for cancel in [false, true] {
            assert_active_content_read_stops(b"nothing here\n", cancel);
        }
    }

    #[test]
    fn active_matching_content_reads_stop_buffered_matches() {
        for cancel in [false, true] {
            assert_active_content_read_stops(b"needle\n", cancel);
        }
    }

    fn run(
        root: &Path,
        names: Option<&str>,
        contents: Option<&str>,
        enter_hidden_folders: bool,
        everything: bool,
    ) -> (Vec<Hit>, Summary) {
        let query = Query {
            names: names.map(str::to_owned),
            contents: contents.map(str::to_owned),
            enter_hidden_folders,
            include_hidden: true,
            rules: rules(),
            everything,
        };
        let control = Control::new();
        let mut hits = Vec::new();
        let summary = search(root, &query, |batch| hits.extend(batch), &control);
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
                HitKind::Content { line, text, ranges, .. } => Some((*line, text.clone(), ranges.clone())),
                HitKind::Name { .. } => None,
            })
            .collect()
    }

    fn extra_matches(hits: &[Hit]) -> u64 {
        hits.iter()
            .filter_map(|hit| match &hit.kind {
                HitKind::Content { more, .. } => Some(*more),
                HitKind::Name { .. } => None,
            })
            .sum()
    }

    fn dev_tree(name: &str) -> crate::test_support::CleanTree {
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
        let skipped: BTreeSet<_> = summary.skipped.iter().map(|skip| skip.path.clone()).collect();
        let expected: BTreeSet<_> =
            [".git", ".logicprojects", ".venv", "__pycache__", "build", "dist", "node_modules", "target"]
                .into_iter()
                .map(|name| root.join(name))
                .collect();
        assert_eq!(skipped, expected);
        assert!(summary.skipped.iter().any(|skip| {
            skip.path == root.join("target")
                && matches!(&skip.reason, SkipReason::Rule(rule) if rule.contains("dev") && rule.contains("Cargo.toml"))
        }));
        assert!(summary.skipped.iter().any(|skip| {
            skip.path == root.join("node_modules")
                && matches!(&skip.reason, SkipReason::Rule(rule) if rule.contains("name"))
        }));
        assert!(summary.scanned > 0);
    }

    #[test]
    fn unreadable_folder_is_reported() {
        use std::os::unix::fs::PermissionsExt;
        let root = root("unreadable");
        write(&root, "unreadable/backup-hidden.txt", b"needle\n");
        let folder = root.join("unreadable");
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o000)).expect("chmod 000");
        let (hits, summary) = run(&root, Some("backup"), None, true, true);
        // Check the mode before restoring it: a folder this user cannot read is
        // the case under test. Restoring first made the check succeed always and
        // the assertions below never ran.
        let unreadable = fs::read_dir(&folder).is_err();
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).expect("restore mode");
        if !unreadable {
            println!("read_dir succeeded on mode 000 (running as root?); skipping the unreadable assertion");
            return;
        }
        assert!(hits.is_empty(), "nothing inside an unreadable folder is a hit: {hits:?}");
        assert_eq!(
            summary.skipped,
            [Skip { path: folder, reason: SkipReason::Unreadable("no permission".to_owned()) }]
        );
    }

    #[test]
    fn packages_are_hits_but_not_descended() {
        let root = root("packages");
        write(&root, "CrowBackup.app/Contents/Info.plist", b"needle\n");
        let (hits, summary) = run(&root, Some("backup"), Some("needle"), false, false);
        let paths = paths(&hits);
        assert!(paths.contains(&root.join("CrowBackup.app")), "the package's own name: {paths:?}");
        assert!(!hits.iter().any(|hit| hit.path.ends_with("Info.plist")), "not walked: {hits:?}");
        assert_eq!(summary.skipped, [Skip { path: root.join("CrowBackup.app"), reason: SkipReason::Package }]);
    }

    #[test]
    fn everything_enters_packages() {
        let root = root("packages-everything");
        write(&root, "CrowBackup.app/Contents/Info.plist", b"needle\n");
        let (hits, summary) = run(&root, None, Some("needle"), false, true);
        assert_eq!(paths(&hits), [root.join("CrowBackup.app/Contents/Info.plist")]);
        assert!(summary.skipped.is_empty(), "{:?}", summary.skipped);
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
    fn dot_entries_are_visited_but_dot_folders_are_not_descended() {
        let root = root("hidden-policy");
        write(&root, ".secret.txt", b"needle\n");
        write(&root, ".folder/inside.txt", b"needle\n");
        write(&root, "visible.txt", b"needle\n");

        // Hidden off: a dot file is visited, so its name and contents match.
        let (hits, _) = run(&root, Some("secret"), None, false, false);
        assert_eq!(paths(&hits), [root.join(".secret.txt")]);
        assert!(hits[0].hidden, "a dot name is hidden");
        let (contents, _) = run(&root, None, Some("needle"), false, false);
        assert_eq!(paths(&contents), [root.join(".secret.txt"), root.join("visible.txt")]);

        // A dot folder's own name matches, but the walk does not go inside, and
        // says so once instead of silently leaving a gap.
        let (names, summary) = run(&root, Some("folder"), None, false, false);
        assert_eq!(paths(&names), [root.join(".folder")]);
        assert!(names[0].is_dir && names[0].hidden);
        assert_eq!(summary.skipped, [Skip { path: root.join(".folder"), reason: SkipReason::HiddenFolder }]);

        // Show Hidden Files on, or Search everything: the contents are found.
        let (shown, _) = run(&root, None, Some("needle"), true, false);
        assert!(paths(&shown).contains(&root.join(".folder/inside.txt")), "{shown:?}");
        let (everything, summary) = run(&root, None, Some("needle"), false, true);
        assert!(paths(&everything).contains(&root.join(".folder/inside.txt")), "{everything:?}");
        assert!(!summary.skipped.iter().any(|skip| skip.reason == SkipReason::HiddenFolder));
    }

    #[test]
    fn hits_carry_the_hidden_flag() {
        let root = root("hidden-flag");
        write(&root, ".backup.txt", b"");
        write(&root, "backup.txt", b"");
        let (hits, _) = run(&root, Some("backup.txt"), None, false, false);
        let hidden = hits.iter().find(|hit| hit.path == root.join(".backup.txt")).expect("the dot name");
        let visible = hits.iter().find(|hit| hit.path == root.join("backup.txt")).expect("the plain name");
        assert!(hidden.hidden);
        assert!(!visible.hidden);
    }

    #[test]
    fn search_includes_hidden_off_leaves_dot_entries_unvisited() {
        let root = root("hidden-setting-off");
        write(&root, ".secret.txt", b"needle\n");
        write(&root, ".folder/inside.txt", b"needle\n");
        write(&root, "visible.txt", b"needle\n");
        let mut hits = Vec::new();
        let query = Query {
            names: Some("needle".into()),
            contents: Some("needle".into()),
            enter_hidden_folders: false,
            include_hidden: false,
            rules: rules(),
            everything: false,
        };
        let control = Control::new();
        let summary = search(&root, &query, |batch| hits.extend(batch), &control);
        assert_eq!(paths(&hits), [root.join("visible.txt")]);
        assert!(!summary.skipped.iter().any(|skip| skip.reason == SkipReason::HiddenFolder));

        // Search everything ignores exclude.toml; it must not undo the hidden setting.
        let mut hits = Vec::new();
        let query = Query {
            names: Some("needle".into()),
            contents: Some("needle".into()),
            enter_hidden_folders: false,
            include_hidden: false,
            rules: rules(),
            everything: true,
        };
        let summary = search(&root, &query, |batch| hits.extend(batch), &control);
        assert_eq!(paths(&hits), [root.join("visible.txt")], "Search everything keeps dot entries out");
        assert!(!summary.skipped.iter().any(|skip| skip.reason == SkipReason::HiddenFolder));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_finder_hidden_flag_marks_a_plain_name_hidden() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let root = root("uf-hidden");
        write(&root, "plain.txt", b"");
        let path = CString::new(root.join("plain.txt").as_os_str().as_bytes()).expect("no NUL in a temp path");
        // SAFETY: `path` is a valid NUL-terminated path under `root`.
        let flagged = unsafe { libc::chflags(path.as_ptr(), libc::UF_HIDDEN) };
        assert_eq!(flagged, 0, "chflags: {}", io::Error::last_os_error());
        let (hits, _) = run(&root, Some("plain"), None, false, false);
        let hit = hits.iter().find(|hit| hit.path == root.join("plain.txt")).expect("a name hit");
        assert!(hit.hidden, "UF_HIDDEN marks it hidden");
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
    fn uppercase_needles_match_without_panicking() {
        // nucleo 0.3.1 asserted here before the needle was lowercased.
        let mut matcher = FuzzyMatcher::new("Docs");
        assert!(matcher.score("~/Documents/Docs").is_some());
        assert!(matcher.indices("~/Documents/Docs").is_some());
        let mut matcher = FuzzyMatcher::new("dT");
        assert!(matcher.score("~/a/dT/b/dTx").is_some());
    }

    #[test]
    fn fuzzy_indices_are_byte_ranges() {
        let haystack = "~/Projects/git";
        let mut matcher = FuzzyMatcher::new("git");
        let (_, ranges) = matcher.indices(haystack).expect("a match");
        let start = haystack.len() - 3;
        assert_eq!(ranges.len(), 1, "one merged range");
        assert_eq!(ranges[0], start..start + 3, "the last three bytes");
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
        // One excerpt per file: its first matching line, plus the count of the rest.
        assert_eq!(content(&hits), [(2, "NEEDLE here".to_owned(), ranges(0, 6))]);
        assert_eq!(extra_matches(&hits), 1, "the third line is counted, not carried");
    }

    fn hit(path: &str) -> Hit {
        Hit {
            path: PathBuf::from(path),
            is_dir: false,
            hidden: false,
            score: 1,
            kind: HitKind::Name { ranges: Vec::new() },
            modified: None,
            size: 0,
        }
    }

    fn hits(count: usize) -> Vec<Hit> {
        (0..count).map(|i| hit(&format!("h{i}"))).collect()
    }

    #[test]
    fn batcher_sends_the_first_hit_at_once() {
        let base = Instant::now();
        let mut batcher = Batcher::new();
        let ready = batcher.push(vec![hit("first")], base);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].len(), 1);
    }

    #[test]
    fn batcher_holds_hits_until_flush() {
        let base = Instant::now();
        let mut batcher = Batcher::new();
        assert_eq!(batcher.push(vec![hit("first")], base).len(), 1);
        assert!(batcher.push(hits(10), base + Duration::from_millis(5)).is_empty(), "under FLUSH, nothing is due");
        assert!(batcher.due(base + Duration::from_millis(20)).is_none());
        assert_eq!(batcher.due(base + Duration::from_millis(36)).map(|batch| batch.len()), Some(10));
    }

    #[test]
    fn batcher_sends_full_batches_immediately() {
        let base = Instant::now();
        let mut batcher = Batcher::new();
        assert_eq!(batcher.push(vec![hit("first")], base).len(), 1);
        let ready = batcher.push(hits(600), base + Duration::from_millis(1));
        let lengths: Vec<usize> = ready.iter().map(Vec::len).collect();
        assert_eq!(lengths, [256, 256]);
        assert_eq!(batcher.finish().map(|batch| batch.len()), Some(88));
    }

    #[test]
    fn batcher_never_sends_empty() {
        let base = Instant::now();
        let mut batcher = Batcher::new();
        assert!(batcher.due(base + Duration::from_millis(100)).is_none());
        assert!(batcher.finish().is_none());
    }

    #[test]
    fn content_search_skips_fifos() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let root = root("fifo");
        write(&root, "file.txt", b"needle\n");
        let pipe = root.join("pipe");
        let path = CString::new(pipe.as_os_str().as_bytes()).expect("no NUL in a temp path");
        // SAFETY: `path` is a valid NUL-terminated path in `root`.
        let made = unsafe { libc::mkfifo(path.as_ptr(), 0o644) };
        assert_eq!(made, 0, "mkfifo: {}", io::Error::last_os_error());

        let query = Query {
            names: None,
            contents: Some("needle".into()),
            enter_hidden_folders: false,
            include_hidden: true,
            rules: rules(),
            everything: true,
        };
        let control = Control::new();
        let (tx, rx) = mpsc::channel();
        let search_root = root.to_path_buf();
        std::thread::spawn(move || {
            let mut hits = Vec::new();
            // Exercise the case where the path became a FIFO after the walk
            // had already accepted it as a regular file.
            let pattern = ContentPattern::new("needle").unwrap();
            let mut content = ContentSearch::new(&pattern).unwrap();
            assert_eq!(content.search(&pipe, None, 0, false, &mut hits, &control), 0);
            let summary = search(&search_root, &query, |batch| hits.extend(batch), &control);
            let _ = tx.send((hits, summary));
        });
        let (hits, summary) = rx.recv_timeout(Duration::from_secs(10)).expect("the search finishes despite the FIFO");
        assert!(summary.error.is_none(), "{:?}", summary.error);
        assert_eq!(paths(&hits), [root.join("file.txt")], "the FIFO is never read");
    }

    #[test]
    fn name_hit_arrives_before_its_content_hit() {
        let root = root("name-before-content");
        write(&root, "needle.txt", b"needle\n");
        let query = Query {
            names: Some("needle".into()),
            contents: Some("needle".into()),
            enter_hidden_folders: false,
            include_hidden: true,
            rules: rules(),
            everything: true,
        };
        let control = Control::new();
        let mut batches: Vec<Vec<bool>> = Vec::new();
        let summary = search(
            &root,
            &query,
            |batch| batches.push(batch.into_iter().map(|hit| matches!(hit.kind, HitKind::Name { .. })).collect()),
            &control,
        );
        assert!(summary.error.is_none(), "{:?}", summary.error);
        let name = batches.iter().position(|batch| batch.iter().any(|is_name| *is_name)).expect("a name hit");
        let content = batches.iter().position(|batch| batch.iter().any(|is_name| !*is_name)).expect("a content hit");
        // The name hit leaves before its file is read. The old engine sent both
        // in one batch, after the content read; the position check alone passed
        // because the name hit came first inside that shared batch.
        assert!(name < content, "the name hit arrives before its own content read: {batches:?}");
    }

    #[test]
    fn a_symlink_to_a_file_gets_a_content_hit() {
        let root = root("symlink-content");
        write(&root, "target.txt", b"needle\n");
        std::os::unix::fs::symlink("target.txt", root.join("link.txt")).expect("symlink");
        let (hits, summary) = run(&root, None, Some("needle"), false, true);
        assert!(summary.error.is_none(), "{:?}", summary.error);
        let link = hits.iter().find(|hit| hit.path == root.join("link.txt")).expect("the symlink is read");
        assert!(matches!(link.kind, HitKind::Content { .. }));
    }

    #[test]
    fn a_file_with_many_matches_sends_one_hit_and_a_count() {
        let root = root("batches");
        let mut text = String::new();
        for i in 0..2000 {
            text.push_str(&format!("needle {i}\n"));
        }
        write(&root, "many.txt", text.as_bytes());
        let query = Query {
            names: None,
            contents: Some("needle".into()),
            enter_hidden_folders: false,
            include_hidden: true,
            rules: rules(),
            everything: true,
        };
        let control = Control::new();
        let mut hits = Vec::new();
        let summary = search(&root, &query, |batch| hits.extend(batch), &control);
        assert!(summary.error.is_none(), "{:?}", summary.error);
        assert_eq!(hits.len(), 1, "only the first line becomes a hit");
        assert_eq!(extra_matches(&hits), 1999, "the other lines are counted");
    }

    #[test]
    fn batches_are_bounded_for_many_files() {
        let root = root("batch-files");
        for i in 0..1000 {
            write(&root, &format!("file-{i}.txt"), b"needle\n");
        }
        let query = Query {
            names: None,
            contents: Some("needle".into()),
            enter_hidden_folders: false,
            include_hidden: true,
            rules: rules(),
            everything: true,
        };
        let control = Control::new();
        let mut batches = Vec::new();
        let summary = search(&root, &query, |batch| batches.push(batch.len()), &control);
        assert!(summary.error.is_none(), "{:?}", summary.error);
        assert!(batches.iter().all(|&len| len <= BATCH), "batches stay bounded: {batches:?}");
        assert_eq!(batches.iter().sum::<usize>(), 1000, "every file arrives");
    }

    #[test]
    fn slash_wrapped_queries_are_regexes() {
        let root = root("regex");
        write(&root, "lines.txt", b"alpha\nneedle here\nbeta needle\n");
        let (hits, _) = run(&root, None, Some("/n(eed)le/"), false, true);
        assert_eq!(content(&hits), [(2, "needle here".to_owned(), ranges(0, 6))]);
        assert_eq!(extra_matches(&hits), 1, "the second matching line is counted");
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
            enter_hidden_folders: true,
            include_hidden: true,
            rules: rules(),
            everything: false,
        };
        let control = Control::new();
        control.cancel();
        let mut hits = Vec::new();
        let summary = search(&root, &query, |batch| hits.extend(batch), &control);
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
            enter_hidden_folders: false,
            include_hidden: true,
            rules: rules(),
            everything: true,
        };
        let control = Control::new();
        let mut hits = Vec::new();
        let flag = control.clone();
        let summary = search(
            &root,
            &query,
            |batch| {
                hits.extend(batch);
                flag.cancel();
            },
            &control,
        );
        assert!(hits.len() < 4000, "cancelling stopped the stream");
        assert!(summary.scanned < 4000, "the walk stopped early");
    }

    fn names_and_contents_tree(name: &str) -> (crate::test_support::CleanTree, Query) {
        let root = root(name);
        for i in 0..50 {
            write(&root, &format!("content-{i:02}.txt"), b"needle\n");
        }
        for i in 0..50 {
            write(&root, &format!("needle-{i:02}.txt"), b"nothing here\n");
        }
        let query = Query {
            names: Some("needle".into()),
            contents: Some("needle".into()),
            enter_hidden_folders: false,
            include_hidden: true,
            rules: rules(),
            everything: true,
        };
        (root, query)
    }

    fn names(hits: &[Hit]) -> BTreeSet<PathBuf> {
        hits.iter()
            .filter(|hit| hit.path.file_name().is_some_and(|name| name.to_string_lossy().starts_with("needle-")))
            .map(|hit| hit.path.clone())
            .collect()
    }

    #[test]
    fn stop_contents_keeps_names_streaming() {
        let (root, query) = names_and_contents_tree("stop-contents");
        let control = Control::new();
        let mut hits = Vec::new();
        let summary = search(
            &root,
            &query,
            |batch| {
                hits.extend(batch);
                if hits.iter().any(|hit| matches!(hit.kind, HitKind::Content { .. })) {
                    control.stop_contents();
                }
            },
            &control,
        );
        assert!(summary.contents_stopped, "the flag is reported");
        assert!(!control.is_cancelled(), "names keep the walk alive");
        assert_eq!(names(&hits).len(), 50, "every needle-* name still arrives");
    }

    /// Deterministic form of the same rule: the walk starts with contents
    /// stopped, so no file is read, and every name still streams.
    #[test]
    fn contents_stopped_before_the_walk_reads_no_contents() {
        let (root, query) = names_and_contents_tree("stop-contents-up-front");
        let control = Control::new();
        control.stop_contents();
        let mut hits = Vec::new();
        let summary = search(&root, &query, |batch| hits.extend(batch), &control);
        assert!(summary.contents_stopped);
        assert!(!control.is_cancelled());
        assert_eq!(names(&hits).len(), 50);
        assert!(
            !hits.iter().any(|hit| matches!(hit.kind, HitKind::Content { .. })),
            "contents are never read once stopped"
        );
    }

    /// The palette scores every candidate on each keystroke. A long non-ASCII
    /// query must not re-walk its graphemes per candidate: 10k names with a
    /// 4 000-char needle took seconds before the matcher cached the needle form.
    #[test]
    fn a_long_unicode_needle_scores_many_names_quickly() {
        let needle: String = std::iter::repeat_n('ש', 4_000).collect();
        let mut fuzzy = FuzzyMatcher::new(&needle);
        let started = std::time::Instant::now();
        for i in 0..10_000 {
            assert!(
                fuzzy.score(&format!("file-{i:06}.txt")).is_none(),
                "a 4 000-char needle cannot match a short name"
            );
        }
        let elapsed = started.elapsed();
        assert!(elapsed < std::time::Duration::from_millis(500), "10k scores took {elapsed:?}");
    }

    /// The cached form must match what `Utf32Str::new` built before it: an emoji,
    /// RTL letters and a precomposed name still score and report byte ranges.
    #[test]
    fn unicode_needles_keep_their_scores_and_ranges() {
        let mut fuzzy = FuzzyMatcher::new("🙂");
        assert_eq!(fuzzy.indices("a🙂b.txt").map(|(_, ranges)| ranges), Some(vec![Range { start: 1, end: 5 }]));
        let mut fuzzy = FuzzyMatcher::new("ש");
        assert_eq!(fuzzy.indices("שלום.txt").map(|(_, ranges)| ranges), Some(vec![Range { start: 0, end: 2 }]));
        let mut fuzzy = FuzzyMatcher::new("café");
        assert!(fuzzy.score("café").is_some(), "a non-ASCII needle still scores");
        assert!(FuzzyMatcher::new("café").score("other.txt").is_none());
    }
}
