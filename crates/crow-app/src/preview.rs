//! The Finder-style preview: picture, name, kind and size, Information, Tags.
//! Shown as the right-hand pane in list view and as the last column in column view.

use std::path::PathBuf;
use std::sync::Arc;

use crow_core::{Excerpt, Extensions, KnownSize, PreviewContent, text_excerpt};
use crow_fs::{CloudStatus, Entry, ReadDecision, ReadGate, ReadHint};
use crow_theme::Theme;
use gpui::{Div, FontWeight, HighlightStyle, Image, ImageFormat, Rgba, StyledText, div, img, prelude::*, px};

use crate::icons::{Glyph, glyph};
use crate::style::{cloud_icon, color, color_alpha, display_text, format_time, icon, kind};
use crate::syntax::{self, Language};

/// What the preview shows for one item, loaded off the main thread.
#[derive(Clone)]
pub struct Preview {
    pub path: PathBuf,
    pub content: Option<PreviewContent>,
    pub image: Option<Arc<Image>>,
    pub tags: Vec<String>,
    /// The lines around a content hit, when the search asked for one.
    pub excerpt: Option<Excerpt>,
    /// The cached cloud status when this preview was requested; `None` means the
    /// caller had no status yet (ordinary local previews do not need one).
    pub cloud: Option<CloudStatus>,
    /// The fresh gate refused an automatic read. `content` is empty and the
    /// column shows a metadata-only placeholder.
    pub blocked: bool,
}

impl Preview {
    /// Request one fresh read when a blocked preview learns that the item is local.
    pub fn update_cloud(&mut self, status: CloudStatus) -> bool {
        let retry = self.blocked && status.is_local() && self.cloud != Some(status);
        self.blocked |= !status.is_local();
        self.cloud = Some(status);
        retry
    }
}

/// `fetch`, plus the lines around a content hit when the text previewer read it.
/// `gate` is a fresh read policy; `cloud` is only for the placeholder's text.
pub fn fetch_at(
    entry: &Entry,
    extensions: &Extensions,
    line: Option<u64>,
    gate: &dyn ReadGate,
    cloud: Option<CloudStatus>,
) -> Preview {
    let _span = tracing::debug_span!("preview load").entered();
    let decision = gate.check(&entry.path, ReadHint::for_path(&entry.path));
    let blocked = decision != ReadDecision::Readable;
    let content = if blocked { None } else { extensions.preview(entry, gate) };
    let image = match &content {
        Some(PreviewContent::Image(png)) => Some(Arc::new(Image::from_bytes(ImageFormat::Png, png.to_vec()))),
        _ => None,
    };
    let excerpt = match (&content, line) {
        (Some(PreviewContent::Text(_)), Some(line)) => text_excerpt(&entry.path, line, 5, 12, gate),
        _ => None,
    };
    Preview { path: entry.path.clone(), content, image, tags: crow_macos::tags(&entry.path), excerpt, cloud, blocked }
}

/// What VoiceOver reads for the preview. `folder_size` is the measured size of
/// the folder (or package) when the size service has one.
pub fn spoken(
    entry: &Entry,
    preview: Option<&Preview>,
    folder_size: Option<KnownSize>,
    cloud: Option<CloudStatus>,
) -> String {
    let size = if entry.is_dir {
        folder_size.map(|size| format!(", {}", crow_fs::human_size(size.bytes))).unwrap_or_default()
    } else {
        format!(", {}", crow_fs::human_size(entry.size))
    };
    let tags = preview.map(|p| p.tags.join(", ")).filter(|t| !t.is_empty());
    let cloud = cloud.or_else(|| preview.and_then(|p| p.cloud));
    format!(
        "Preview: {}, {}{size}, created {}, modified {}{}{}",
        display_text(&entry.name),
        kind(entry),
        format_time(entry.created),
        format_time(entry.modified),
        tags.map(|t| format!(", tags {t}")).unwrap_or_default(),
        cloud.and_then(CloudStatus::spoken_suffix).map(|s| format!(", {s}")).unwrap_or_default()
    )
}

/// A metadata-only block where a picture or text preview would be. There is no
/// control here: downloading stays an explicit Finder/user action.
fn cloud_placeholder(cloud: Option<CloudStatus>, theme: &Theme) -> Div {
    let (glyph_name, role, text) = match cloud {
        Some(CloudStatus::Downloading) => (Glyph::Cloud, "accent", "Downloading from iCloud…"),
        Some(CloudStatus::NotDownloaded) => (Glyph::Cloud, "fg-muted", "Not downloaded from iCloud"),
        Some(CloudStatus::Unknown) => (Glyph::CloudOff, "warning", "iCloud status unavailable"),
        _ => (Glyph::CloudOff, "fg-muted", "Preview unavailable for this iCloud item"),
    };
    div()
        .h(px(220.))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .text_color(color(theme.role(role)))
        .child(glyph(glyph_name, 64.0, role, theme))
        .child(div().text_size(px(12.)).child(text))
}

/// How many lines of a text file the preview draws: the box clips the rest, so
/// a large file costs a page, not the whole file.
const PREVIEW_TEXT_LINES: usize = 80;

/// One numbered line, its tokens in the syntax roles. A search hit's line gets
/// the accent wash; the excerpt's numbers start where the file's line does.
fn code_line(index: usize, number: u64, text: &str, hit: Option<u64>, language: Language, theme: &Theme) -> Div {
    let runs = syntax::spans(text, language).into_iter().map(|(range, kind)| {
        let color: gpui::Hsla = color(theme.role(kind.role())).into();
        (range, HighlightStyle { color: Some(color), ..Default::default() })
    });
    div()
        .debug_selector(move || format!("preview-code-line-{index}"))
        .flex()
        .gap_1()
        .when(hit == Some(number), |row| row.bg(color_alpha(theme.role("accent"), 0x1a)))
        .child(
            div()
                .w(px(32.))
                .flex_none()
                .text_right()
                .text_color(color(theme.role("fg-subtle")))
                .child(format!("{number}")),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .child(StyledText::new(text.to_owned()).with_highlights(runs)),
        )
}

fn code_box(theme: &Theme, border: Rgba) -> gpui::Stateful<Div> {
    div()
        .id("preview-code")
        .debug_selector(|| "preview-code".into())
        .h(px(220.))
        .p_2()
        .rounded_md()
        .border_1()
        .border_color(border)
        .bg(color(theme.role("bg-inset")))
        .overflow_hidden()
        .font_family("Menlo")
        .text_size(px(11.))
        .flex()
        .flex_col()
}

/// The preview's contents for `entry`, stacked top to bottom. `folder_size` is
/// the service's measured size for a folder or package; a file uses its own.
pub fn contents(
    entry: &Entry,
    preview: Option<&Preview>,
    theme: &Theme,
    folder_size: Option<KnownSize>,
    cloud: Option<CloudStatus>,
) -> Div {
    let muted = color(theme.role("fg-muted"));
    let border = color(theme.role("border"));
    let size = if entry.is_dir { folder_size } else { Some(KnownSize { bytes: entry.size, stale: false, version: 0 }) };
    let size_text = size.map_or_else(|| "--".to_owned(), |size| crow_fs::human_size(size.bytes));
    let size_stale = size.is_some_and(|size| size.stale);
    let kind = kind(entry);
    let tags = preview.map(|p| p.tags.join(", ")).filter(|t| !t.is_empty());
    let excerpt = preview.and_then(|p| p.excerpt.as_ref());
    let language = Language::from_path(&entry.path);
    let picture: gpui::AnyElement = if preview.is_some_and(|p| p.blocked) {
        cloud_placeholder(preview.and_then(|p| p.cloud).or(cloud), theme).into_any_element()
    } else {
        match (preview.and_then(|p| p.image.clone()), preview.and_then(|p| p.content.clone())) {
            (Some(image), _) => div()
                .h(px(220.))
                .flex()
                .items_center()
                .justify_center()
                .child(img(image).max_w_full().max_h(px(220.)))
                .into_any_element(),
            (None, Some(PreviewContent::Text(_))) if excerpt.is_some() => {
                let excerpt = excerpt.expect("checked");
                let lines = excerpt.text.lines().take(PREVIEW_TEXT_LINES).enumerate().map(|(i, line)| {
                    code_line(i, excerpt.first_line + i as u64, line, Some(excerpt.hit), language, theme)
                });
                code_box(theme, border).children(lines).into_any_element()
            }
            (None, Some(PreviewContent::Text(text))) => {
                let lines = text
                    .lines()
                    .take(PREVIEW_TEXT_LINES)
                    .enumerate()
                    .map(|(i, line)| code_line(i, i as u64 + 1, line, None, language, theme));
                code_box(theme, border).children(lines).into_any_element()
            }
            _ => div()
                .h(px(220.))
                .flex()
                .items_center()
                .justify_center()
                .children(cloud_icon(cloud, &entry.path, entry.is_dir, 128).map(|i| img(i).size(px(128.))))
                .into_any_element(),
        }
    };
    let info_row = |label: &str, value: String| {
        div()
            .flex()
            .justify_between()
            .gap_2()
            .py_1()
            .border_b_1()
            .border_color(border)
            .child(div().flex_none().text_color(muted).child(label.to_owned()))
            .child(div().text_right().child(value))
    };
    let detail = div()
        .flex()
        .items_center()
        .text_color(muted)
        .child(format!("{kind} · "))
        .child(div().when(size_stale, |size| size.opacity(0.5)).child(size_text));
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(picture)
        .child(
            div()
                .flex()
                .flex_col()
                .child(div().text_size(px(15.)).child(display_text(&entry.name).into_owned()))
                .child(detail),
        )
        .child(div().pt_2().font_weight(FontWeight::SEMIBOLD).child("Information"))
        .child(info_row("Created", format_time(entry.created)))
        .child(info_row("Modified", format_time(entry.modified)))
        .when(entry.added.is_some(), |d| d.child(info_row("Added", format_time(entry.added))))
        .child(div().pt_2().font_weight(FontWeight::SEMIBOLD).child("Tags"))
        .child(match tags {
            Some(tags) => div().child(tags),
            None => div().text_color(muted).child("No tags"),
        })
}

/// A folder's summary in the preview column.
pub struct FolderSummary<'a> {
    pub path: &'a std::path::Path,
    pub name: &'a str,
    pub modified: Option<std::time::SystemTime>,
    pub items: Option<usize>,
    pub size: Option<crow_core::KnownSize>,
}

/// What VoiceOver reads for the folder summary.
pub fn folder_spoken(name: &str, items: Option<usize>, size: Option<crow_core::KnownSize>) -> String {
    let name = display_text(name);
    let items = match items {
        Some(n) => format!(", {n} item{}", if n == 1 { "" } else { "s" }),
        None => String::new(),
    };
    let size = match size {
        Some(size) => format!(", {}", crow_fs::human_size(size.bytes)),
        None => String::new(),
    };
    format!("{name}, Folder{items}{size}")
}

/// The preview column when the cursor is a folder (or the focused folder has no
/// rows): its icon, name, "Folder · n items · size" and the modified date.
pub fn folder_summary(summary: FolderSummary<'_>, theme: &Theme) -> Div {
    let muted = color(theme.role("fg-muted"));
    let items = match summary.items {
        Some(1) => "Folder · 1 item".to_owned(),
        Some(n) => format!("Folder · {n} items"),
        None => "Folder".to_owned(),
    };
    let detail = div().text_color(muted).flex().items_center().child(items).when_some(summary.size, |detail, size| {
        detail
            .child(" · ")
            .child(div().when(size.stale, |size| size.opacity(0.5)).child(crow_fs::human_size(size.bytes)))
    });
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .h(px(220.))
                .flex()
                .items_center()
                .justify_center()
                .children(icon(summary.path, true, 128).map(|i| img(i).size(px(128.)))),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .child(div().text_size(px(15.)).child(display_text(summary.name).into_owned()))
                .child(detail),
        )
        .when(summary.modified.is_some(), |panel| {
            panel.child(
                div()
                    .flex()
                    .justify_between()
                    .gap_2()
                    .py_1()
                    .border_b_1()
                    .border_color(color(theme.role("border")))
                    .child(div().flex_none().text_color(muted).child("Modified"))
                    .child(div().text_right().child(format_time(summary.modified))),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool, size: u64) -> Entry {
        Entry {
            name: name.into(),
            path: std::path::PathBuf::from("/tmp").join(name),
            is_dir,
            is_symlink: false,
            hidden: false,
            dataless: false,
            size,
            modified: None,
            created: None,
            added: None,
        }
    }

    struct Refuse;
    impl ReadGate for Refuse {
        fn check(&self, _: &std::path::Path, _: ReadHint) -> ReadDecision {
            ReadDecision::SkipCloud
        }
    }

    struct CountReads(std::sync::atomic::AtomicUsize);
    impl ReadGate for CountReads {
        fn check(&self, _: &std::path::Path, _: ReadHint) -> ReadDecision {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            ReadDecision::Readable
        }
    }

    #[test]
    fn a_blocked_cloud_preview_keeps_metadata_only() {
        let dir = tempfile::tempdir().expect("temp");
        let path = dir.path().join("note.txt");
        std::fs::write(&path, "cloud text\n").unwrap();
        let entry = entry("note.txt", false, 11);
        let mut entry = entry;
        entry.path = path;
        let ext = Extensions::with_builtins();
        let blocked = fetch_at(&entry, &ext, None, &Refuse, Some(CloudStatus::NotDownloaded));
        assert!(blocked.blocked);
        assert!(blocked.content.is_none());
        assert!(blocked.image.is_none());
        assert!(blocked.excerpt.is_none());
        assert_eq!(blocked.cloud, Some(CloudStatus::NotDownloaded));

        let gate = CountReads(Default::default());
        let mut ready = fetch_at(&entry, &ext, None, &gate, Some(CloudStatus::Downloaded));
        assert!(!ready.blocked);
        assert!(matches!(ready.content, Some(PreviewContent::Text(_))), "a downloaded file previews");
        assert!(gate.0.load(std::sync::atomic::Ordering::Relaxed) >= 1, "the gate is consulted");
        assert!(!ready.update_cloud(CloudStatus::NotDownloaded));
        assert!(ready.blocked, "eviction replaces an already loaded preview with the placeholder");
        assert!(ready.update_cloud(CloudStatus::Downloaded));
        assert!(!ready.update_cloud(CloudStatus::Downloaded), "one transition schedules only one retry");
    }

    #[test]
    fn a_folder_preview_speaks_the_measured_size() {
        let folder = entry("nested", true, 0);
        let line = spoken(&folder, None, Some(KnownSize { bytes: 7_340_032, stale: false, version: 1 }), None);
        assert!(line.contains("7.3 MB"), "{line}");
    }

    #[test]
    fn a_file_preview_keeps_its_own_size() {
        let file = entry("a.txt", false, 512_000);
        let line = spoken(&file, None, None, None);
        assert!(line.contains("512.0 KB"), "{line}");
    }
}
