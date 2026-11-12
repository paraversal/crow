//! Extension points. Built-in columns and previewers register through the same
//! API plugins will use, so it is exercised from the start.

use std::io::Read;
use std::sync::Arc;
use std::time::SystemTime;

use crow_fs::{Entry, SortKey};

/// What a column needs from the app to render a value.
pub struct ColumnContext<'a> {
    pub kind: &'a dyn Fn(&Entry) -> Arc<str>,
    pub time: &'a dyn Fn(Option<SystemTime>) -> String,
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
    fn text(&self, entry: &Entry, _: &ColumnContext) -> String {
        if entry.is_dir { "--".into() } else { crow_fs::human_size(entry.size) }
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
    fn builtin_columns_are_registered_in_finder_order() {
        let ext = Extensions::with_builtins();
        let ids: Vec<_> = ext.columns().iter().map(|c| c.id()).collect();
        assert_eq!(ids, ["modified", "size", "kind", "added"]);
        let shown: Vec<_> = ext.default_columns().iter().map(|c| c.id()).collect();
        assert_eq!(shown, ["modified", "size", "kind"], "Finder's defaults; Date Added is optional");
    }
}
