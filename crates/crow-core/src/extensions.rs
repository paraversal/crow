//! Extension points. Built-in columns and previewers register through the same
//! API plugins will use, so it is exercised from the start.

use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

use crow_fs::{Entry, SortKey};

/// What a column needs from the app to render a value.
pub struct ColumnContext<'a> {
    pub kind: &'a dyn Fn(&Entry) -> Arc<str>,
    pub time: &'a dyn Fn(Option<SystemTime>) -> String,
    /// A folder's measured size, when the size service has one.
    pub size: &'a dyn Fn(&Entry) -> Option<KnownSize>,
}

/// A folder size the service knows: bytes, whether they are stale, and a version
/// that changes when a new measurement arrives (the fade key).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KnownSize {
    pub bytes: u64,
    pub stale: bool,
    pub version: u64,
}

/// One column's value for one row. `opacity` dims a stale folder size; `fade`
/// is the version to animate when a new value arrives; `spoken` overrides the
/// text for VoiceOver (a folder with no size says "size not calculated").
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnCell {
    pub text: String,
    pub spoken: Option<String>,
    pub opacity: f32,
    pub fade: Option<u64>,
}

impl ColumnCell {
    pub fn plain(text: String) -> Self {
        Self { text, spoken: None, opacity: 1.0, fade: None }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

/// A list-view column after Name.
pub trait Column: Send + Sync {
    /// Stable identifier, used in settings (`views.toml` from M5).
    fn id(&self) -> &'static str;
    fn title(&self) -> &str;
    /// Default width in points.
    fn width(&self) -> f32;
    /// The narrowest a user may drag it.
    fn min_width(&self) -> f32 {
        50.0
    }
    /// Shown unless the user hides it. Choosing columns per folder arrives with view settings (M5).
    fn visible_by_default(&self) -> bool {
        true
    }
    fn align(&self) -> Align {
        Align::Left
    }
    /// The sort this column's header applies, if it is sortable.
    fn sort_key(&self) -> Option<SortKey>;
    fn text(&self, entry: &Entry, cx: &ColumnContext) -> String;
    /// The styled value; plain text unless a column has a transient (the Size column).
    fn cell(&self, entry: &Entry, cx: &ColumnContext) -> ColumnCell {
        ColumnCell::plain(self.text(entry, cx))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewContent {
    Text(String),
    /// PNG bytes.
    Image(Arc<Vec<u8>>),
}

/// Produces the preview pane's picture of a file. Runs on a background thread;
/// `None` passes the entry to the next previewer.
pub trait Previewer: Send + Sync {
    fn id(&self) -> &'static str;
    fn preview(&self, entry: &Entry) -> Option<PreviewContent>;
}

#[derive(Clone, Default)]
pub struct Extensions {
    columns: Vec<Arc<dyn Column>>,
    previewers: Vec<Arc<dyn Previewer>>,
}

impl Extensions {
    /// The built-in columns and text previewer. The app adds the ones that need the OS.
    pub fn with_builtins() -> Self {
        let mut ext = Self::default();
        ext.register_column(Arc::new(ModifiedColumn));
        ext.register_column(Arc::new(SizeColumn));
        ext.register_column(Arc::new(KindColumn));
        ext.register_column(Arc::new(AddedColumn));
        ext.register_previewer(Arc::new(TextPreviewer));
        ext
    }

    pub fn register_column(&mut self, column: Arc<dyn Column>) {
        self.columns.push(column);
    }

    pub fn register_previewer(&mut self, previewer: Arc<dyn Previewer>) {
        self.previewers.push(previewer);
    }

    pub fn columns(&self) -> &[Arc<dyn Column>] {
        &self.columns
    }

    /// The columns a new list view shows, in order.
    pub fn default_columns(&self) -> Vec<Arc<dyn Column>> {
        self.columns.iter().filter(|c| c.visible_by_default()).cloned().collect()
    }

    /// The first previewer that has something for `entry`.
    pub fn preview(&self, entry: &Entry) -> Option<PreviewContent> {
        self.previewers.iter().find_map(|p| p.preview(entry))
    }
}

struct SizeColumn;
impl Column for SizeColumn {
    fn id(&self) -> &'static str {
        "size"
    }
    fn title(&self) -> &str {
        "Size"
    }
    fn width(&self) -> f32 {
        80.0
    }
    fn align(&self) -> Align {
        Align::Right
    }
    fn sort_key(&self) -> Option<SortKey> {
        Some(SortKey::Size)
    }
    fn text(&self, entry: &Entry, cx: &ColumnContext) -> String {
        self.cell(entry, cx).text
    }
    fn cell(&self, entry: &Entry, cx: &ColumnContext) -> ColumnCell {
        if entry.is_dir && entry.is_symlink {
            // The link's target is a folder; Crow does not follow it, so there is
            // no measured size (the Info pane says the same).
            ColumnCell { text: "--".into(), spoken: Some("size not calculated".into()), opacity: 1.0, fade: None }
        } else if entry.is_dir {
            match (cx.size)(entry) {
                None => ColumnCell {
                    text: "--".into(),
                    spoken: Some("size not calculated".into()),
                    opacity: 1.0,
                    fade: None,
                },
                Some(size) if size.stale => {
                    ColumnCell { text: crow_fs::human_size(size.bytes), spoken: None, opacity: 0.5, fade: None }
                }
                Some(size) => ColumnCell {
                    text: crow_fs::human_size(size.bytes),
                    spoken: None,
                    opacity: 1.0,
                    fade: (size.version > 0).then_some(size.version),
                },
            }
        } else {
            ColumnCell::plain(crow_fs::human_size(entry.size))
        }
    }
}

struct KindColumn;
impl Column for KindColumn {
    fn id(&self) -> &'static str {
        "kind"
    }
    fn title(&self) -> &str {
        "Kind"
    }
    fn width(&self) -> f32 {
        150.0
    }
    fn sort_key(&self) -> Option<SortKey> {
        Some(SortKey::Kind)
    }
    fn text(&self, entry: &Entry, cx: &ColumnContext) -> String {
        (cx.kind)(entry).to_string()
    }
}

struct ModifiedColumn;
impl Column for ModifiedColumn {
    fn id(&self) -> &'static str {
        "modified"
    }
    fn title(&self) -> &str {
        "Date Modified"
    }
    fn width(&self) -> f32 {
        170.0
    }
    fn sort_key(&self) -> Option<SortKey> {
        Some(SortKey::Modified)
    }
    fn text(&self, entry: &Entry, cx: &ColumnContext) -> String {
        (cx.time)(entry.modified)
    }
}

struct AddedColumn;
impl Column for AddedColumn {
    fn id(&self) -> &'static str {
        "added"
    }
    fn title(&self) -> &str {
        "Date Added"
    }
    fn width(&self) -> f32 {
        170.0
    }
    fn sort_key(&self) -> Option<SortKey> {
        Some(SortKey::Added)
    }
    fn text(&self, entry: &Entry, cx: &ColumnContext) -> String {
        (cx.time)(entry.added)
    }
    fn visible_by_default(&self) -> bool {
        false
    }
}

/// Plain-text preview: the first 40 lines of files that are valid UTF-8 text.
pub struct TextPreviewer;

impl TextPreviewer {
    const MAX_BYTES: usize = 8192;
    const MAX_LINES: usize = 40;
}

impl Previewer for TextPreviewer {
    fn id(&self) -> &'static str {
        "text"
    }

    fn preview(&self, entry: &Entry) -> Option<PreviewContent> {
        if entry.is_dir || entry.size == 0 {
            return None;
        }
        let mut head = Vec::with_capacity(Self::MAX_BYTES);
        std::fs::File::open(&entry.path).ok()?.take(Self::MAX_BYTES as u64).read_to_end(&mut head).ok()?;
        // Text, not binary: no NUL bytes. Cut at the last newline when the file was
        // truncated, so a multi-byte character split by the cap never fails the check.
        if head.contains(&0) {
            return None;
        }
        let end = if head.len() == Self::MAX_BYTES {
            head.iter().rposition(|b| *b == b'\n').map_or(head.len(), |i| i + 1)
        } else {
            head.len()
        };
        let text = std::str::from_utf8(&head[..end]).ok()?;
        Some(PreviewContent::Text(text.lines().take(Self::MAX_LINES).collect::<Vec<_>>().join("\n")))
    }
}

/// A window of a text file around a content match, for the search preview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Excerpt {
    /// 1-based number of the first line in `text`.
    pub first_line: u64,
    pub text: String,
    /// 1-based number of the matched line.
    pub hit: u64,
}

/// Read at most 1 MiB around `line`: `before` lines above it, `total` lines in
/// all. Refuses binary files and lines past the cap.
pub fn text_excerpt(path: &Path, line: u64, before: usize, total: usize) -> Option<Excerpt> {
    const MAX_BYTES: usize = 1 << 20;
    if line == 0 || total == 0 {
        return None;
    }
    let mut bytes = Vec::with_capacity(8192);
    std::fs::File::open(path).ok()?.take(MAX_BYTES as u64).read_to_end(&mut bytes).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    let end = if bytes.len() == MAX_BYTES {
        bytes.iter().rposition(|byte| *byte == b'\n').map_or(bytes.len(), |index| index + 1)
    } else {
        bytes.len()
    };
    let text = std::str::from_utf8(&bytes[..end]).ok()?;
    let lines: Vec<&str> = text.lines().collect();
    if line as usize > lines.len() {
        return None;
    }
    let first = (line as usize - 1).saturating_sub(before);
    let last = (first + total).min(lines.len());
    Some(Excerpt { first_line: (first + 1) as u64, text: lines[first..last].join("\n"), hit: line })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_for(path: &std::path::Path) -> Entry {
        crow_fs::stat_entry(path).unwrap().unwrap()
    }

    #[test]
    fn text_previewer_accepts_text_and_refuses_binary() {
        let dir = std::env::temp_dir().join(format!("crow-core-preview-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let text = dir.join("notes.md");
        std::fs::write(&text, "# Title\n\nbody\n").unwrap();
        let binary = dir.join("blob.bin");
        std::fs::write(&binary, [0x89, b'P', b'N', b'G', 0, 1, 2]).unwrap();
        let long = dir.join("long.txt");
        std::fs::write(&long, "ä\n".repeat(10_000)).unwrap();

        let ext = Extensions::with_builtins();
        assert_eq!(ext.preview(&entry_for(&text)), Some(PreviewContent::Text("# Title\n\nbody".into())));
        assert_eq!(ext.preview(&entry_for(&binary)), None);
        match ext.preview(&entry_for(&long)) {
            Some(PreviewContent::Text(t)) => assert_eq!(t.lines().count(), TextPreviewer::MAX_LINES),
            other => panic!("long UTF-8 file should preview, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn text_excerpt_centers_on_the_line() {
        let dir = std::env::temp_dir().join(format!("crow-core-excerpt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("lines.txt");
        let body: String = (1..=100).map(|n| format!("line {n}\n")).collect();
        std::fs::write(&path, body).unwrap();

        let excerpt = text_excerpt(&path, 60, 5, 12).expect("an excerpt");
        assert_eq!(excerpt.first_line, 55);
        assert_eq!(excerpt.hit, 60);
        assert!(excerpt.text.lines().any(|line| line == "line 60"), "{}", excerpt.text);
        assert_eq!(text_excerpt(&path, 60, 0, 3).expect("from the hit").first_line, 60);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn text_excerpt_refuses_binary_and_lines_past_the_cap() {
        let dir = std::env::temp_dir().join(format!("crow-core-excerpt-bin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let binary = dir.join("blob.bin");
        std::fs::write(&binary, b"text\0more\n").unwrap();
        assert_eq!(text_excerpt(&binary, 1, 2, 5), None);

        let big = dir.join("big.txt");
        let mut body = String::new();
        for n in 0..120_000 {
            body.push_str(&format!("line {n} ........................................\n"));
        }
        std::fs::write(&big, body).unwrap();
        assert_eq!(text_excerpt(&big, 120_000, 2, 5), None, "the line sits past 1 MiB");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn builtin_columns_are_registered_in_finder_order() {
        let ext = Extensions::with_builtins();
        let ids: Vec<_> = ext.columns().iter().map(|c| c.id()).collect();
        assert_eq!(ids, ["modified", "size", "kind", "added"]);
        let shown: Vec<_> = ext.default_columns().iter().map(|c| c.id()).collect();
        assert_eq!(shown, ["modified", "size", "kind"], "Finder's defaults; Date Added is optional");
    }

    #[test]
    fn a_symlink_to_a_folder_has_no_measured_size() {
        let entry = Entry {
            name: "link".into(),
            path: std::path::PathBuf::from("/tmp/link"),
            is_dir: true,
            is_symlink: true,
            hidden: false,
            size: 0,
            modified: None,
            created: None,
            added: None,
        };
        let kind = |_: &Entry| Arc::<str>::from("Folder");
        let time = |_: Option<SystemTime>| "—".to_owned();
        let size = |_: &Entry| Some(KnownSize { bytes: 1024, stale: false, version: 1 });
        let cx = ColumnContext { kind: &kind, time: &time, size: &size };
        let cell = SizeColumn.cell(&entry, &cx);
        assert_eq!(cell.text, "--", "Crow does not follow the link, so it has no size");
        assert_eq!(cell.spoken.as_deref(), Some("size not calculated"));
    }
}
