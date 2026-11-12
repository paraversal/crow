//! The Finder-style preview: picture, name, kind and size, Information, Tags.
//! Shown as the right-hand pane in list view and as the last column in column view.

use std::path::PathBuf;
use std::sync::Arc;

use crow_core::{Extensions, PreviewContent};
use crow_fs::Entry;
use crow_theme::Theme;
use gpui::{Div, FontWeight, Image, ImageFormat, div, img, prelude::*, px};

use crate::style::{color, format_time, icon, kind};

/// What the preview shows for one item, loaded off the main thread.
pub struct Preview {
    pub path: PathBuf,
    pub content: Option<PreviewContent>,
    pub image: Option<Arc<Image>>,
    pub tags: Vec<String>,
}

/// Run the previewers and read the tags. Blocking: call on a background thread.
pub fn fetch(entry: &Entry, extensions: &Extensions) -> Preview {
    let content = extensions.preview(entry);
    let image = match &content {
        Some(PreviewContent::Image(png)) => Some(Arc::new(Image::from_bytes(ImageFormat::Png, png.to_vec()))),
        _ => None,
    };
    Preview { path: entry.path.clone(), content, image, tags: crow_macos::tags(&entry.path) }
}

/// What VoiceOver reads for the preview.
pub fn spoken(entry: &Entry, preview: Option<&Preview>) -> String {
    let size = if entry.is_dir { String::new() } else { format!(", {}", crow_fs::human_size(entry.size)) };
    let tags = preview.map(|p| p.tags.join(", ")).filter(|t| !t.is_empty());
    format!(
        "Preview: {}, {}{size}, created {}, modified {}{}",
        entry.name,
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
    let picture = match (preview.and_then(|p| p.image.clone()), preview.and_then(|p| p.content.clone())) {
        (Some(image), _) => {
            div().h(px(220.)).flex().items_center().justify_center().child(img(image).max_w_full().max_h(px(220.)))
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
                .child(div().text_size(px(15.)).child(entry.name.clone()))
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
