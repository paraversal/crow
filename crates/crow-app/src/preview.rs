//! The Finder-style preview: picture, name, kind and size, Information, Tags.
//! Shown as the right-hand pane in list view and as the last column in column view.

use std::path::PathBuf;
use std::sync::Arc;

use crow_core::{Excerpt, Extensions, PreviewContent, text_excerpt};
use crow_fs::Entry;
use crow_theme::Theme;
use gpui::{Div, FontWeight, Image, ImageFormat, div, img, prelude::*, px};

use crate::style::{color, color_alpha, display_text, format_time, icon, kind};

/// What the preview shows for one item, loaded off the main thread.
#[derive(Clone)]
pub struct Preview {
    pub path: PathBuf,
    pub content: Option<PreviewContent>,
    pub image: Option<Arc<Image>>,
    pub tags: Vec<String>,
    /// The lines around a content hit, when the search asked for one.
    pub excerpt: Option<Excerpt>,
}

/// Run the previewers and read the tags. Blocking: call on a background thread.
pub fn fetch(entry: &Entry, extensions: &Extensions) -> Preview {
    fetch_at(entry, extensions, None)
}

/// `fetch`, plus the lines around a content hit when the text previewer read it.
pub fn fetch_at(entry: &Entry, extensions: &Extensions, line: Option<u64>) -> Preview {
    let _span = tracing::debug_span!("preview load").entered();
    let content = extensions.preview(entry);
    let image = match &content {
        Some(PreviewContent::Image(png)) => Some(Arc::new(Image::from_bytes(ImageFormat::Png, png.to_vec()))),
        _ => None,
    };
    let excerpt = match (&content, line) {
        (Some(PreviewContent::Text(_)), Some(line)) => text_excerpt(&entry.path, line, 5, 12),
        _ => None,
    };
    Preview { path: entry.path.clone(), content, image, tags: crow_macos::tags(&entry.path), excerpt }
}

/// What VoiceOver reads for the preview.
pub fn spoken(entry: &Entry, preview: Option<&Preview>) -> String {
    let size = if entry.is_dir { String::new() } else { format!(", {}", crow_fs::human_size(entry.size)) };
    let tags = preview.map(|p| p.tags.join(", ")).filter(|t| !t.is_empty());
    format!(
        "Preview: {}, {}{size}, created {}, modified {}{}",
        display_text(&entry.name),
        kind(entry),
        format_time(entry.created),
        format_time(entry.modified),
        tags.map(|t| format!(", tags {t}")).unwrap_or_default()
    )
}

/// The preview's contents for `entry`, stacked top to bottom.
pub fn contents(entry: &Entry, preview: Option<&Preview>, theme: &Theme) -> Div {
    let muted = color(theme.role("fg-muted"));
    let border = color(theme.role("border"));
    let size = if entry.is_dir { "--".to_owned() } else { crow_fs::human_size(entry.size) };
    let kind = kind(entry);
    let tags = preview.map(|p| p.tags.join(", ")).filter(|t| !t.is_empty());
    let excerpt = preview.and_then(|p| p.excerpt.as_ref());
    let picture = match (preview.and_then(|p| p.image.clone()), preview.and_then(|p| p.content.clone())) {
        (Some(image), _) => {
            div().h(px(220.)).flex().items_center().justify_center().child(img(image).max_w_full().max_h(px(220.)))
        }
        (None, Some(PreviewContent::Text(_))) if excerpt.is_some() => {
            let excerpt = excerpt.expect("checked");
            let accent = color_alpha(theme.role("accent"), 0x1a);
            let lines = excerpt.text.lines().enumerate().map(|(i, line)| {
                let number = excerpt.first_line + i as u64;
                div()
                    .flex()
                    .gap_1()
                    .when(number == excerpt.hit, |row| row.bg(accent))
                    .child(
                        div()
                            .w(px(28.))
                            .flex_none()
                            .text_right()
                            .text_color(color(theme.role("fg-subtle")))
                            .child(format!("{number}")),
                    )
                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(line.to_owned()))
            });
            div()
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
                .children(lines)
        }
        (None, Some(PreviewContent::Text(text))) => div()
            .h(px(220.))
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(border)
            .bg(color(theme.role("bg-inset")))
            .overflow_hidden()
            .font_family("Menlo")
            .text_size(px(11.))
            .child(text),
        _ => div()
            .h(px(220.))
            .flex()
            .items_center()
            .justify_center()
            .children(icon(&entry.path, entry.is_dir, 128).map(|i| img(i).size(px(128.)))),
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
                .child(div().text_color(muted).child(format!("{kind} · {size}"))),
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
