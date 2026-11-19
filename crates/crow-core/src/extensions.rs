//! Extension points. Built-in columns and previewers register through the same
//! API plugins will use, so it is exercised from the start.

use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

use crow_fs::{Entry, ReadDecision, ReadGate, ReadHint, SortKey};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Stable plugin package identity: 1–96 ASCII bytes, split on dots into
/// nonempty lowercase alphanumeric/hyphen segments. `crow` and `crow.*` are
/// reserved for bundled providers.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PluginId(String);

impl PluginId {
    pub const MAX_BYTES: usize = 96;
    pub const RESERVED: &'static str = "crow";

    pub fn parse(text: &str) -> Result<Self, String> {
        if text.is_empty() || text.len() > Self::MAX_BYTES {
            return Err(format!("plugin id must be 1-{} ASCII bytes", Self::MAX_BYTES));
        }
        if !text.split('.').all(is_segment) {
            return Err(
                "plugin id segments must be nonempty lowercase letters, digits or hyphens, split by dots".into()
            );
        }
        Ok(Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The bundled namespace: installed bundles cannot claim it.
    pub fn is_reserved(&self) -> bool {
        self.0 == Self::RESERVED || self.0.starts_with("crow.")
    }
}

fn is_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

impl std::fmt::Display for PluginId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for PluginId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for PluginId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// One plugin-local extension name. Local IDs are one segment, so
/// `git.lines-changed` decodes at its last dot.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LocalId(String);

impl LocalId {
    pub const MAX_BYTES: usize = 96;

    pub fn parse(text: &str) -> Result<Self, String> {
        if text.is_empty() || text.len() > Self::MAX_BYTES {
            return Err(format!("local id must be 1-{} ASCII bytes", Self::MAX_BYTES));
        }
        if !is_segment(text) {
            return Err("local id must be lowercase letters, digits or hyphens, without dots".into());
        }
        Ok(Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for LocalId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for LocalId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for LocalId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// A published extension point: `plugin_id.local_id`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ExtensionId {
    pub plugin: PluginId,
    pub local: LocalId,
}

impl ExtensionId {
    pub fn new(plugin: PluginId, local: LocalId) -> Self {
        Self { plugin, local }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let (plugin, local) = text.rsplit_once('.').ok_or("extension id needs a plugin and local part")?;
        Ok(Self { plugin: PluginId::parse(plugin)?, local: LocalId::parse(local)? })
    }
}

impl std::fmt::Display for ExtensionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.plugin, self.local)
    }
}

impl Serialize for ExtensionId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ExtensionId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// The extension points API 1 can publish. Manifest `provides` and registration
/// descriptors use the same keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ExtensionKind {
    Column,
    Decoration,
    Preview,
    Sidebar,
    Command,
    Hook,
    Status,
    Info,
    IconPack,
}

impl ExtensionKind {
    pub const ALL: [Self; 9] = [
        Self::Column,
        Self::Decoration,
        Self::Preview,
        Self::Sidebar,
        Self::Command,
        Self::Hook,
        Self::Status,
        Self::Info,
        Self::IconPack,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Column => "column",
            Self::Decoration => "decoration",
            Self::Preview => "preview",
            Self::Sidebar => "sidebar",
            Self::Command => "command",
            Self::Hook => "hook",
            Self::Status => "status",
            Self::Info => "info",
            Self::IconPack => "icon-pack",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == text)
    }
}

/// Where a descriptor gets its data. Computed kinds name one script function;
/// an icon pack names a manifest-declared bundle asset. The two are distinct so
/// a contract bug cannot turn one into the other.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DescriptorSource {
    Function(String),
    Asset(String),
}

/// What a command may act on. A scope command receives no entries; a selection
/// command requires 1-128 selected rows and may opt into a context menu.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CommandTarget {
    #[default]
    Scope,
    Selection,
}

impl CommandTarget {
    pub fn key(self) -> &'static str {
        match self {
            Self::Scope => "scope",
            Self::Selection => "selection",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "scope" => Some(Self::Scope),
            "selection" => Some(Self::Selection),
            _ => None,
        }
    }
}

/// A decoration's named color. The app maps these to theme roles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tint {
    Accent,
    Info,
    Success,
    Warning,
    Error,
}

impl Tint {
    pub fn key(self) -> &'static str {
        match self {
            Self::Accent => "accent",
            Self::Info => "info",
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "accent" => Some(Self::Accent),
            "info" => Some(Self::Info),
            "success" => Some(Self::Success),
            "warning" => Some(Self::Warning),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
}

/// One validated registration. Data-only kinds (an icon pack) carry a declared
/// asset instead of a function. Presentation fields are concrete defaults; the
/// registry rejects fields that do not belong to the descriptor's kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Descriptor {
    pub kind: ExtensionKind,
    pub id: ExtensionId,
    pub title: String,
    pub source: DescriptorSource,
    /// Column width in points (32-480).
    pub width: u16,
    /// Narrowest a column may be dragged (32-width).
    pub min_width: u16,
    pub align: Align,
    /// Command target; `Scope` for kinds that do not use it.
    pub applies_to: CommandTarget,
    /// A selection command also appears in file context menus.
    pub context_menu: bool,
    /// Preview/command suffix filter, already ASCII-folded. `None` matches all.
    pub extensions: Option<Vec<String>>,
}

impl Descriptor {
    pub fn new(kind: ExtensionKind, id: ExtensionId, title: String, source: DescriptorSource) -> Self {
        Self {
            kind,
            id,
            title,
            source,
            width: 100,
            min_width: 50,
            align: Align::Left,
            applies_to: CommandTarget::Scope,
            context_menu: false,
            extensions: None,
        }
    }

    pub fn function(&self) -> Option<&str> {
        match &self.source {
            DescriptorSource::Function(function) => Some(function),
            DescriptorSource::Asset(_) => None,
        }
    }

    pub fn asset(&self) -> Option<&str> {
        match &self.source {
            DescriptorSource::Asset(asset) => Some(asset),
            DescriptorSource::Function(_) => None,
        }
    }
}

/// A request-local entry identity. It is never persisted and cannot be minted by
/// a decoded value; the host keeps the handle → entry table for one invocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntryHandle(pub u32);

/// One entry's bounded text contribution, keyed by its request-local handle.
/// `tint` and `dim` are decoration-only and stay `None`/false for columns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryValue {
    pub handle: EntryHandle,
    /// A column cell or a decoration badge (16 bytes for decorations).
    pub text: String,
    pub spoken: Option<String>,
    pub tint: Option<Tint>,
    pub dim: bool,
}

/// The preview provider's bounded output for v1: plain text or a table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PluginPreview {
    pub text: Option<String>,
    pub table: Option<Vec<Vec<String>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidebarRow {
    pub id: String,
    pub title: String,
    pub meta: Option<String>,
    pub badge: Option<String>,
    /// A request-local directory handle; `None` is a display-only row.
    pub handle: Option<EntryHandle>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusItem {
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InfoRow {
    pub label: String,
    pub value: String,
}

/// A closed reply shape keyed to the descriptor kind. A decoded value never
/// carries a path or a fresh handle to another invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContributionValues {
    Column(Vec<EntryValue>),
    Decoration(Vec<EntryValue>),
    Preview(PluginPreview),
    Sidebar(Vec<SidebarRow>),
    Status(Vec<StatusItem>),
    Info(Vec<InfoRow>),
}

impl ContributionValues {
    pub fn kind(&self) -> ExtensionKind {
        match self {
            Self::Column(_) => ExtensionKind::Column,
            Self::Decoration(_) => ExtensionKind::Decoration,
            Self::Preview(_) => ExtensionKind::Preview,
            Self::Sidebar(_) => ExtensionKind::Sidebar,
            Self::Status(_) => ExtensionKind::Status,
            Self::Info(_) => ExtensionKind::Info,
        }
    }
}

/// A checked action a command may request. The app applies it only while the
/// activation's captured owner, scope and selection are still current.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandIntent {
    Message(String),
    CopyText(String),
    /// A handle from this activation's request table, never a path.
    Reveal(EntryHandle),
}

/// The hook events API 1 delivers. `scope` is host-provided display text, never
/// an I/O capability.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HookEvent {
    ScopeChanged { scope: String },
    SelectionChanged { scope: String },
    FilesChanged { scope: String },
    OperationDone,
}

impl HookEvent {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::ScopeChanged { .. } => "scope-changed",
            Self::SelectionChanged { .. } => "selection-changed",
            Self::FilesChanged { .. } => "files-changed",
            Self::OperationDone => "operation-done",
        }
    }

    pub fn scope(&self) -> Option<&str> {
        match self {
            Self::ScopeChanged { scope } | Self::SelectionChanged { scope } | Self::FilesChanged { scope } => {
                Some(scope)
            }
            Self::OperationDone => None,
        }
    }
}

/// Host-provided metadata for one entry in an invocation. No path: the host
/// keeps the path and answers a granted read by handle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryData {
    pub handle: EntryHandle,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub kind: String,
}

/// The bounded input map a script sees for one call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// Display text for the current scope (a folder path in P1B), not an I/O handle.
    pub scope: String,
    pub selection: Option<EntryHandle>,
    pub entries: Vec<EntryData>,
}

/// What a column needs from the app to render a value.
pub type PluginValueFn<'a> = &'a dyn Fn(&ExtensionId, &Path, u64) -> Option<ColumnCell>;
pub type PluginVersionFn<'a> = &'a dyn Fn(&Entry) -> u64;

pub struct ColumnContext<'a> {
    pub kind: &'a dyn Fn(&Entry) -> Arc<str>,
    pub time: &'a dyn Fn(Option<SystemTime>) -> String,
    /// A folder's measured size, when the size service has one.
    pub size: &'a dyn Fn(&Entry) -> Option<KnownSize>,
    /// A ready plugin cell for this entry, keyed by descriptor and version.
    pub plugin_value: Option<PluginValueFn<'a>>,
    /// The entry version the cache was keyed by.
    pub plugin_version: Option<PluginVersionFn<'a>>,
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
    /// Stable identifier, used in settings (`views.toml` from M5). Plugin
    /// columns own their fully qualified ID and borrow it here.
    fn id(&self) -> &str;
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
    /// `gate` is checked immediately before any automatic content read. A
    /// previewer must not open a file when the fresh decision is not `Readable`.
    fn preview(&self, entry: &Entry, gate: &dyn ReadGate) -> Option<PreviewContent>;
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
    pub fn preview(&self, entry: &Entry, gate: &dyn ReadGate) -> Option<PreviewContent> {
        self.previewers.iter().find_map(|p| p.preview(entry, gate))
    }
}

struct SizeColumn;
impl Column for SizeColumn {
    fn id(&self) -> &str {
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
    fn id(&self) -> &str {
        "kind"
    }
    fn title(&self) -> &str {
        "Kind"
    }
    fn width(&self) -> f32 {
        112.0
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
    fn id(&self) -> &str {
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
    fn id(&self) -> &str {
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

    fn preview(&self, entry: &Entry, gate: &dyn ReadGate) -> Option<PreviewContent> {
        if entry.is_dir || entry.size == 0 {
            return None;
        }
        if gate.check(&entry.path, ReadHint::for_path(&entry.path)) != ReadDecision::Readable {
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
pub fn text_excerpt(path: &Path, line: u64, before: usize, total: usize, gate: &dyn ReadGate) -> Option<Excerpt> {
    const MAX_BYTES: usize = 1 << 20;
    if line == 0 || total == 0 {
        return None;
    }
    if gate.check(path, ReadHint::for_path(path)) != ReadDecision::Readable {
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
    use crow_fs::AllowAll;

    fn entry_for(path: &std::path::Path) -> Entry {
        crow_fs::stat_entry(path).unwrap().unwrap()
    }

    #[test]
    fn text_previewer_accepts_text_and_refuses_binary() {
        let dir = crate::test_support::TempDir::new("crow-core-preview-");
        let text = dir.join("notes.md");
        std::fs::write(&text, "# Title\n\nbody\n").unwrap();
        let binary = dir.join("blob.bin");
        std::fs::write(&binary, [0x89, b'P', b'N', b'G', 0, 1, 2]).unwrap();
        let long = dir.join("long.txt");
        std::fs::write(&long, "ä\n".repeat(10_000)).unwrap();

        let ext = Extensions::with_builtins();
        assert_eq!(ext.preview(&entry_for(&text), &AllowAll), Some(PreviewContent::Text("# Title\n\nbody".into())));
        assert_eq!(ext.preview(&entry_for(&binary), &AllowAll), None);
        match ext.preview(&entry_for(&long), &AllowAll) {
            Some(PreviewContent::Text(t)) => assert_eq!(t.lines().count(), TextPreviewer::MAX_LINES),
            other => panic!("long UTF-8 file should preview, got {other:?}"),
        }
    }

    struct Refuse;
    impl ReadGate for Refuse {
        fn check(&self, _: &std::path::Path, _: ReadHint) -> ReadDecision {
            ReadDecision::SkipCloud
        }
    }

    #[test]
    fn a_refused_read_returns_no_preview_or_excerpt() {
        let dir = crate::test_support::TempDir::new("crow-core-cloud-preview-");
        let text = dir.join("notes.md");
        std::fs::write(&text, "cloud text\n").unwrap();
        let ext = Extensions::with_builtins();
        assert_eq!(ext.preview(&entry_for(&text), &Refuse), None);
        assert_eq!(text_excerpt(&text, 1, 1, 3, &Refuse), None);
    }

    #[test]
    fn text_excerpt_centers_on_the_line() {
        let dir = crate::test_support::TempDir::new("crow-core-excerpt-");
        let path = dir.join("lines.txt");
        let body: String = (1..=100).map(|n| format!("line {n}\n")).collect();
        std::fs::write(&path, body).unwrap();

        let excerpt = text_excerpt(&path, 60, 5, 12, &AllowAll).expect("an excerpt");
        assert_eq!(excerpt.first_line, 55);
        assert_eq!(excerpt.hit, 60);
        assert!(excerpt.text.lines().any(|line| line == "line 60"), "{}", excerpt.text);
        assert_eq!(text_excerpt(&path, 60, 0, 3, &AllowAll).expect("from the hit").first_line, 60);
    }

    #[test]
    fn text_excerpt_refuses_binary_and_lines_past_the_cap() {
        let dir = crate::test_support::TempDir::new("crow-core-excerpt-bin-");
        let binary = dir.join("blob.bin");
        std::fs::write(&binary, b"text\0more\n").unwrap();
        assert_eq!(text_excerpt(&binary, 1, 2, 5, &AllowAll), None);

        let big = dir.join("big.txt");
        let mut body = String::new();
        for n in 0..120_000 {
            body.push_str(&format!("line {n} ........................................\n"));
        }
        std::fs::write(&big, body).unwrap();
        assert_eq!(text_excerpt(&big, 120_000, 2, 5, &AllowAll), None, "the line sits past 1 MiB");
    }

    #[test]
    fn builtin_columns_are_registered_in_finder_order() {
        let ext = Extensions::with_builtins();
        let ids: Vec<_> = ext.columns().iter().map(|c| c.id()).collect();
        assert_eq!(ids, ["modified", "size", "kind", "added"]);
        let columns = ext.default_columns();
        let shown: Vec<_> = columns.iter().map(|c| c.id()).collect();
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
            dataless: false,
            size: 0,
            modified: None,
            created: None,
            added: None,
        };
        let kind = |_: &Entry| Arc::<str>::from("Folder");
        let time = |_: Option<SystemTime>| "—".to_owned();
        let size = |_: &Entry| Some(KnownSize { bytes: 1024, stale: false, version: 1 });
        let cx = ColumnContext { kind: &kind, time: &time, size: &size, plugin_value: None, plugin_version: None };
        let cell = SizeColumn.cell(&entry, &cx);
        assert_eq!(cell.text, "--", "Crow does not follow the link, so it has no size");
        assert_eq!(cell.spoken.as_deref(), Some("size not calculated"));
    }

    #[test]
    fn plugin_ids_validate_and_reserve_the_bundled_namespace() {
        assert_eq!(PluginId::parse("example.relative-path").unwrap().as_str(), "example.relative-path");
        assert!(PluginId::parse("Crow").is_err(), "uppercase is not stable");
        assert!(PluginId::parse("a..b").is_err(), "empty segment");
        assert!(PluginId::parse(".a").is_err());
        assert!(PluginId::parse("a_b").is_err(), "underscore is not in the id alphabet");
        assert!(PluginId::parse(&"a".repeat(PluginId::MAX_BYTES + 1)).is_err());
        assert!(PluginId::parse("crow").unwrap().is_reserved());
        assert!(PluginId::parse("crow.git").unwrap().is_reserved());
        assert!(!PluginId::parse("crowbar.git").unwrap().is_reserved());
        assert!(LocalId::parse("lines-changed").is_ok());
        assert!(LocalId::parse("git.lines").is_err(), "a local id is one segment");
    }

    #[test]
    fn extension_ids_round_trip_at_the_last_dot() {
        let id = ExtensionId::parse("git.lines-changed").unwrap();
        assert_eq!(id.plugin.as_str(), "git");
        assert_eq!(id.local.as_str(), "lines-changed");
        assert_eq!(id.to_string(), "git.lines-changed");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"git.lines-changed\"");
        assert_eq!(serde_json::from_str::<ExtensionId>(&json).unwrap(), id);
        assert!(serde_json::from_str::<ExtensionId>("\"git\"").is_err());
    }
}
