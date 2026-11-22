//! The Finder-style preview: picture, name, kind and size, Information, Tags.
//! Shown as the right-hand pane in list view and as the last column in column view.

use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crow_core::{EntryHandle, Excerpt, Extensions, KnownSize, PluginPreview, PreviewContent, text_excerpt};
use crow_fs::{CloudStatus, Entry, ReadDecision, ReadGate, ReadHint};
use crow_plugin::ExcerptInput;
use crow_theme::Theme;
use futures::lock::Mutex;
use gpui::{
    Div, FontWeight, HighlightStyle, Image, ImageFormat, ObjectFit, RenderImage, Rgba, Role, StyledText, SvgRenderer,
    div, img, prelude::*, px,
};

use crate::icons::{Glyph, glyph};
use crate::style::{cloud_icon, color, color_alpha, display_text, format_time, kind};
use crate::syntax::{self, Language};

pub enum ImageSize {
    Pane,
    /// One bounded Retina rendition works for both comparison layouts and Info.
    /// Each cell's worker prevents superseded requests from overlapping.
    Gallery(Arc<Mutex<()>>),
}

impl ImageSize {
    pub fn points(&self) -> u32 {
        match self {
            Self::Pane => 256,
            Self::Gallery(_) => 1024,
        }
    }
}

/// What the preview shows for one item, loaded off the main thread.
#[derive(Clone)]
pub struct Preview {
    pub path: PathBuf,
    pub content: Option<PreviewContent>,
    pub image: Option<Arc<RenderImage>>,
    pub tags: Vec<String>,
    /// The lines around a content hit, when the search asked for one.
    pub excerpt: Option<Excerpt>,
    /// The cached cloud status when this preview was requested; `None` means the
    /// caller had no status yet (ordinary local previews do not need one).
    pub cloud: Option<CloudStatus>,
    /// The fresh gate refused an automatic read. `content` is empty and the
    /// column shows a metadata-only placeholder.
    pub blocked: bool,
    /// An accepted configured plugin provider, before native fallback.
    pub plugin: Option<PluginPreviewView>,
}

#[derive(Clone)]
pub struct PluginPreviewView {
    pub provider: String,
    pub preview: PluginPreview,
}

impl Preview {
    pub fn release_image(&mut self, cx: &mut gpui::App) {
        if let Some(image) = self.image.take() {
            // Let the window finish updating before clearing its sprite atlas.
            cx.defer(move |cx| cx.drop_image(image, None));
        }
    }

    /// Request one fresh read when a blocked preview learns that the item is local.
    pub fn update_cloud(&mut self, status: CloudStatus) -> bool {
        let retry = self.blocked && status.is_local() && self.cloud != Some(status);
        self.blocked |= !status.is_local();
        self.cloud = Some(status);
        retry
    }
}

/// One bounded host read for a preview provider that holds the read grant.
pub fn read_plugin_excerpt(entry: &Entry, gate: &dyn ReadGate) -> Option<ExcerptInput> {
    if gate.check(&entry.path, ReadHint::for_path(&entry.path)) != ReadDecision::Readable {
        return None;
    }
    // io-ok: called inside background_spawn before the provider request.
    let metadata = std::fs::symlink_metadata(&entry.path).ok()?;
    // Compare the live no-follow metadata with the listing's captured version;
    // a same-path replacement must not be previewed under the old entry.
    // io-ok: this function runs inside background_spawn.
    if !metadata.is_file()
        || metadata.len() != entry.size
        || entry.modified.is_some_and(|modified| metadata.modified().ok() != Some(modified))
    {
        return None;
    }
    let identity = crow_plugin::EntryRef::with_identity(&entry.path, metadata.dev(), metadata.ino());
    let text = crow_plugin::read_excerpt(&identity, 64 * 1024).ok()?;
    Some(ExcerptInput { handle: EntryHandle(1), text })
}

/// Try the configured providers in order for at most 250 ms total. The native
/// pipeline runs when every candidate declines, fails or is busy.
#[allow(clippy::too_many_arguments)]
pub async fn plugin_preview(
    cx: &mut gpui::AsyncApp,
    surface: crate::plugins::SurfaceId,
    candidates: Vec<crate::plugins::PreviewCandidate>,
    entry: Entry,
    gate: Arc<dyn ReadGate>,
    cloud: Option<CloudStatus>,
    revision: u64,
    cancelled: Arc<AtomicBool>,
) -> Option<Preview> {
    if candidates.is_empty() || cancelled.load(Ordering::Relaxed) {
        return None;
    }
    let started = Instant::now();
    let excerpt = if candidates.iter().any(|candidate| candidate.read_allowed) {
        let entry = entry.clone();
        let gate = gate.clone();
        let excerpt = cx.background_spawn(async move { read_plugin_excerpt(&entry, gate.as_ref()) }).await;
        if cancelled.load(Ordering::Relaxed) {
            return None;
        }
        excerpt
    } else {
        None
    };
    for candidate in candidates {
        if cancelled.load(Ordering::Relaxed) {
            return None;
        }
        let Some(remaining) = Duration::from_millis(250).checked_sub(started.elapsed()) else {
            // The deadline owns this binding; stop its outstanding ticket unless a
            // newer preview has already replaced it.
            if !cancelled.load(Ordering::Relaxed) {
                cx.update(|cx| crate::plugins::cancel_previews(surface, cx));
            }
            return None;
        };
        let receiver = match cx.update(|cx| {
            crate::plugins::try_preview(
                cx,
                surface,
                &candidate,
                &entry,
                if candidate.read_allowed { excerpt.clone() } else { None },
                revision,
            )
        }) {
            Some(receiver) => receiver,
            // Busy, stopped or a superseded descriptor: try the next candidate.
            None => continue,
        };
        let receive = async move { receiver.await.ok() };
        let timeout = cx.background_executor().timer(remaining);
        futures::pin_mut!(receive, timeout);
        match futures::future::select(receive, timeout).await {
            futures::future::Either::Left((
                Some(crow_plugin::RequestResult::Values(crow_core::ContributionValues::Preview(preview))),
                _,
            )) if preview.text.is_some() || preview.table.is_some() => {
                if cancelled.load(Ordering::Relaxed) {
                    return None;
                }
                let path = entry.path.clone();
                // io-ok: inside `background_spawn`.
                let tags = cx.background_spawn(async move { crow_macos::tags(&path) }).await;
                if cancelled.load(Ordering::Relaxed) {
                    return None;
                }
                return Some(Preview {
                    path: entry.path.clone(),
                    content: None,
                    image: None,
                    tags,
                    excerpt: None,
                    cloud,
                    blocked: false,
                    plugin: Some(PluginPreviewView { provider: candidate.package, preview }),
                });
            }
            futures::future::Either::Left(_) => continue,
            futures::future::Either::Right(_) => {
                if !cancelled.load(Ordering::Relaxed) {
                    cx.update(|cx| crate::plugins::cancel_previews(surface, cx));
                }
                return None;
            }
        }
    }
    if !cancelled.load(Ordering::Relaxed) {
        cx.update(|cx| crate::plugins::cancel_previews(surface, cx));
    }
    None
}

/// Test-only holds: a path's next preview request signals that it started and
/// waits for the test to release it, so stale replies can be released out of order.
#[cfg(test)]
pub mod test_hold {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{LazyLock, Mutex};

    struct Hold {
        started: std::sync::mpsc::Sender<()>,
        release: futures::channel::oneshot::Receiver<()>,
    }

    static HOLDS: LazyLock<Mutex<HashMap<PathBuf, Vec<Hold>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

    /// Hold the next request for `path`. Returns the "started" receiver and the
    /// sender that releases it.
    pub fn install(path: PathBuf) -> (std::sync::mpsc::Receiver<()>, futures::channel::oneshot::Sender<()>) {
        let (started, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = futures::channel::oneshot::channel();
        HOLDS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(path)
            .or_default()
            .push(Hold { started, release: release_rx });
        (started_rx, release_tx)
    }

    pub(super) async fn wait(path: &std::path::Path) {
        let hold = {
            let mut holds = HOLDS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let hold = holds.get_mut(path).and_then(Vec::pop);
            if holds.get(path).is_some_and(Vec::is_empty) {
                holds.remove(path);
            }
            hold
        };
        if let Some(hold) = hold {
            let _ = hold.started.send(());
            let _ = hold.release.await;
        }
    }
}

/// The provider pipeline shared by the Preview pane and Gallery: try the
/// configured plugin providers in order, then the native previewers. The caller
/// owns target/generation checks; this only does the bounded work.
#[allow(clippy::too_many_arguments)]
pub async fn load_with_plugins(
    cx: &mut gpui::AsyncApp,
    surface: crate::plugins::SurfaceId,
    candidates: Vec<crate::plugins::PreviewCandidate>,
    entry: Entry,
    extensions: Arc<Extensions>,
    gate: Arc<dyn ReadGate>,
    cloud: Option<CloudStatus>,
    line: Option<u64>,
    revision: u64,
    image_size: ImageSize,
    cancelled: Arc<AtomicBool>,
) -> Option<Preview> {
    #[cfg(test)]
    test_hold::wait(&entry.path).await;
    if cancelled.load(Ordering::Relaxed) {
        return None;
    }
    if !candidates.is_empty()
        && cx
            .background_spawn({
                let path = entry.path.clone();
                let gate = gate.clone();
                async move { gate.check(&path, ReadHint::for_path(&path)) == ReadDecision::Readable }
            })
            .await
        && !cancelled.load(Ordering::Relaxed)
        && let Some(preview) =
            plugin_preview(cx, surface, candidates, entry.clone(), gate.clone(), cloud, revision, cancelled.clone())
                .await
    {
        return Some(preview);
    }
    if cancelled.load(Ordering::Relaxed) {
        return None;
    }
    let image_points = image_size.points();
    let worker = match image_size {
        ImageSize::Pane => None,
        ImageSize::Gallery(worker) => Some(worker.lock_owned().await),
    };
    let svg = cx.update(|cx| cx.svg_renderer());
    cx.background_spawn(async move {
        let _worker = worker;
        (!cancelled.load(Ordering::Relaxed))
            .then(|| fetch_at(&entry, &extensions, line, gate.as_ref(), cloud, image_points, svg))
    })
    .await
}

/// `fetch`, plus the lines around a content hit when the text previewer read it.
/// `gate` is a fresh read policy; `cloud` is only for the placeholder's text.
fn fetch_at(
    entry: &Entry,
    extensions: &Extensions,
    line: Option<u64>,
    gate: &dyn ReadGate,
    cloud: Option<CloudStatus>,
    image_points: u32,
    svg: SvgRenderer,
) -> Preview {
    let _span = tracing::debug_span!("preview load").entered();
    let decision = gate.check(&entry.path, ReadHint::for_path(&entry.path));
    let blocked = decision != ReadDecision::Readable;
    let content = if blocked { None } else { extensions.preview(entry, gate, image_points) };
    let image = match &content {
        Some(PreviewContent::Image(png)) => Image::from_bytes(ImageFormat::Png, png.to_vec()).to_image_data(svg).ok(),
        _ => None,
    };
    let excerpt = match (&content, line) {
        (Some(PreviewContent::Text(_)), Some(line)) => text_excerpt(&entry.path, line, 5, 12, gate),
        _ => None,
    };
    Preview {
        path: entry.path.clone(),
        content,
        image,
        // io-ok: `fetch_at` runs on the preview loader's background task.
        tags: crow_macos::tags(&entry.path),
        excerpt,
        cloud,
        blocked,
        plugin: None,
    }
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
    let mut spoken = format!(
        "Preview: {}, {}{size}, created {}, modified {}{}{}",
        display_text(&entry.name),
        kind(entry),
        format_time(entry.created),
        format_time(entry.modified),
        tags.map(|t| format!(", tags {t}")).unwrap_or_default(),
        cloud.and_then(CloudStatus::spoken_suffix).map(|s| format!(", {s}")).unwrap_or_default()
    );
    if let Some(plugin) = preview.and_then(|p| p.plugin.as_ref()) {
        spoken.push_str(&format!(", plugin preview from {}", plugin.provider));
    }
    spoken
}

/// A metadata-only block where a picture or text preview would be. There is no
/// control here: downloading stays an explicit Finder/user action. `height`
/// `None` fills the cell for Gallery.
fn cloud_placeholder(cloud: Option<CloudStatus>, theme: &Theme, height: Option<gpui::Pixels>) -> Div {
    let (glyph_name, role, text) = match cloud {
        Some(CloudStatus::Downloading) => (Glyph::Cloud, "accent", "Downloading from iCloud…"),
        Some(CloudStatus::NotDownloaded) => (Glyph::Cloud, "fg-muted", "Not downloaded from iCloud"),
        Some(CloudStatus::Unknown) => (Glyph::CloudOff, "warning", "iCloud status unavailable"),
        _ => (Glyph::CloudOff, "fg-muted", "Preview unavailable for this iCloud item"),
    };
    div()
        .when_some(height, |block, height| block.h(height))
        .when(height.is_none(), |block| block.size_full())
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .text_color(color(theme.role(role)))
        .child(glyph(glyph_name, 64.0, role, theme))
        .child(div().text_size(px(12.)).child(text))
}

/// Gallery's metadata-only cloud block, without the Preview column's fixed height.
pub(crate) fn cloud_note(cloud: CloudStatus, theme: &Theme) -> Div {
    cloud_placeholder(Some(cloud), theme, None)
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

fn plugin_contents(plugin: &PluginPreviewView, theme: &Theme, tall: bool) -> gpui::AnyElement {
    let border = color(theme.role("border"));
    let muted = color(theme.role("fg-muted"));
    let provider = div().text_size(px(11.)).text_color(muted).child(format!("{} preview", plugin.provider));
    let height = (!tall).then(|| px(220.));
    if let Some(text) = &plugin.preview.text {
        let lines = text
            .lines()
            .take(PREVIEW_TEXT_LINES)
            .enumerate()
            .map(|(i, line)| code_line(i, i as u64 + 1, line, None, Language::Plain, theme));
        return div()
            .flex()
            .flex_col()
            .gap_1()
            .when(tall, |block| block.size_full().min_h_0())
            .child(provider)
            .child(code_box(theme, border, height).children(lines))
            .into_any_element();
    }
    let table = plugin.preview.table.as_ref().expect("preview has text or table");
    let label = table.iter().flatten().cloned().collect::<Vec<_>>().join(", ");
    div()
        .id("preview-plugin-table")
        .debug_selector(|| "preview-plugin-table".into())
        .role(Role::Group)
        .aria_label(format!("{} preview: {label}", plugin.provider))
        .flex()
        .flex_col()
        .gap_1()
        .when(tall, |block| block.size_full().min_h_0())
        .child(provider)
        .child(
            div()
                .id("preview-plugin-table-body")
                .when_some(height, |body, height| body.max_h(height))
                .when(tall, |body| body.flex_1().min_h_0())
                .overflow_y_scroll()
                .rounded_md()
                .border_1()
                .border_color(border)
                .text_size(px(12.))
                .children(table.iter().map(|row| {
                    div().flex().border_b_1().border_color(border).children(row.iter().map(|cell| {
                        div().flex_1().min_w_0().px_2().py_1().overflow_hidden().whitespace_nowrap().child(cell.clone())
                    }))
                })),
        )
        .into_any_element()
}

fn code_box(theme: &Theme, border: Rgba, height: Option<gpui::Pixels>) -> gpui::Stateful<Div> {
    div()
        .id("preview-code")
        .debug_selector(|| "preview-code".into())
        .when_some(height, |box_, height| box_.h(height))
        .when(height.is_none(), |box_| box_.flex_1().min_h_0())
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

/// A packed glyph, or Finder's icon when the source is native, the pack has no
/// match, or the item is a cloud placeholder.
pub(crate) fn path_icon(
    resolver: &crate::icon_packs::Resolver,
    path: &std::path::Path,
    is_dir: bool,
    is_package: bool,
    cloud: Option<CloudStatus>,
    theme: &Theme,
    size: f32,
) -> gpui::AnyElement {
    let packed =
        (!crate::style::native_cloud_icon(cloud, path)).then(|| resolver.resolve(path, is_dir, is_package)).flatten();
    render_icon(packed, || cloud_icon(cloud, path, is_dir, size as u32), theme, size)
}

pub(crate) fn entry_icon(
    resolver: &crate::icon_packs::Resolver,
    entry: &Entry,
    cloud: Option<CloudStatus>,
    theme: &Theme,
    size: f32,
) -> gpui::AnyElement {
    let packed = (!crate::style::native_cloud_icon(cloud, &entry.path))
        .then(|| resolver.resolve(&entry.path, entry.is_dir, entry.is_package()))
        .flatten();
    render_icon(packed, || crate::style::entry_cloud_icon(cloud, entry, size as u32), theme, size)
}

fn render_icon(
    packed: Option<crate::icon_packs::ResolvedIcon>,
    native: impl FnOnce() -> Option<Arc<gpui::Image>>,
    theme: &Theme,
    size: f32,
) -> gpui::AnyElement {
    match packed {
        Some(icon) => crate::icons::pack_glyph(&icon, size, color(icon.color.rgb(theme))),
        None => div().children(native().map(|image| img(image).flex_none().size(px(size)))).into_any_element(),
    }
}

/// The preview's contents for `entry`, stacked top to bottom. `folder_size` is
/// the service's measured size for a folder or package; a file uses its own.
pub fn contents(
    entry: &Entry,
    preview: Option<&Preview>,
    theme: &Theme,
    folder_size: Option<KnownSize>,
    cloud: Option<CloudStatus>,
    resolver: &crate::icon_packs::Resolver,
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
    let picture: gpui::AnyElement = if preview.is_some_and(|p| p.plugin.is_some()) {
        plugin_contents(preview.and_then(|p| p.plugin.as_ref()).expect("checked"), theme, false)
    } else if preview.is_some_and(|p| p.blocked) {
        cloud_placeholder(preview.and_then(|p| p.cloud).or(cloud), theme, Some(px(220.))).into_any_element()
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
                code_box(theme, border, Some(px(220.))).children(lines).into_any_element()
            }
            (None, Some(PreviewContent::Text(text))) => {
                let lines = text
                    .lines()
                    .take(PREVIEW_TEXT_LINES)
                    .enumerate()
                    .map(|(i, line)| code_line(i, i as u64 + 1, line, None, language, theme));
                code_box(theme, border, Some(px(220.))).children(lines).into_any_element()
            }
            _ => div()
                .h(px(220.))
                .flex()
                .items_center()
                .justify_center()
                .child(entry_icon(resolver, entry, cloud, theme, 128.))
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
            .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_right().child(value))
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

/// Gallery's large cell content: the same provider output as the Preview tab,
/// sized to fill the cell instead of the column's fixed 220-point blocks.
pub(crate) fn gallery_content(
    entry: &Entry,
    preview: Option<&Preview>,
    theme: &Theme,
    cloud: Option<CloudStatus>,
    resolver: &crate::icon_packs::Resolver,
) -> gpui::AnyElement {
    let border = color(theme.role("border"));
    if let Some(plugin) = preview.and_then(|preview| preview.plugin.as_ref()) {
        return plugin_contents(plugin, theme, true);
    }
    if preview.is_some_and(|preview| preview.blocked) {
        return cloud_placeholder(preview.and_then(|preview| preview.cloud).or(cloud), theme, None).into_any_element();
    }
    match (preview.and_then(|preview| preview.image.clone()), preview.and_then(|preview| preview.content.clone())) {
        (Some(image), _) => div()
            .size_full()
            .relative()
            .debug_selector(|| "gallery-image-frame".into())
            .child(img(image).absolute().inset_0().size_full().aspect_square().object_fit(ObjectFit::Contain))
            .into_any_element(),
        (None, Some(PreviewContent::Text(text))) => {
            let language = Language::from_path(&entry.path);
            let lines = text
                .lines()
                .take(PREVIEW_TEXT_LINES)
                .enumerate()
                .map(|(i, line)| code_line(i, i as u64 + 1, line, None, language, theme));
            code_box(theme, border, None).children(lines).into_any_element()
        }
        _ => div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(entry_icon(resolver, entry, cloud, theme, 160.))
            .into_any_element(),
    }
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
pub fn folder_summary(summary: FolderSummary<'_>, theme: &Theme, resolver: &crate::icon_packs::Resolver) -> Div {
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
        .child(div().h(px(220.)).flex().items_center().justify_center().child(path_icon(
            resolver,
            summary.path,
            true,
            false,
            None,
            theme,
            128.,
        )))
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
            icon_kind: crow_fs::IconKind::default(),
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
        let blocked = fetch_at(
            &entry,
            &ext,
            None,
            &Refuse,
            Some(CloudStatus::NotDownloaded),
            256,
            SvgRenderer::new(Arc::new(())),
        );
        assert!(blocked.blocked);
        assert!(blocked.content.is_none());
        assert!(blocked.image.is_none());
        assert!(blocked.excerpt.is_none());
        assert_eq!(blocked.cloud, Some(CloudStatus::NotDownloaded));

        let gate = CountReads(Default::default());
        let mut ready =
            fetch_at(&entry, &ext, None, &gate, Some(CloudStatus::Downloaded), 256, SvgRenderer::new(Arc::new(())));
        assert!(!ready.blocked);
        assert!(matches!(ready.content, Some(PreviewContent::Text(_))), "a downloaded file previews");
        assert!(gate.0.load(std::sync::atomic::Ordering::Relaxed) >= 1, "the gate is consulted");
        assert!(!ready.update_cloud(CloudStatus::NotDownloaded));
        assert!(ready.blocked, "eviction replaces an already loaded preview with the placeholder");
        assert!(ready.update_cloud(CloudStatus::Downloaded));
        assert!(!ready.update_cloud(CloudStatus::Downloaded), "one transition schedules only one retry");
    }

    #[test]
    fn a_read_grant_excerpt_checks_the_captured_entry_version() {
        let dir = tempfile::tempdir().expect("temp");
        let path = dir.path().join("note.txt");
        std::fs::write(&path, "hello\n").unwrap();
        let entry = crow_fs::stat_entry(&path).expect("stat").expect("entry");
        assert!(read_plugin_excerpt(&entry, &crow_fs::AllowAll).is_some());

        let mut stale = entry;
        stale.size = 0;
        assert!(
            read_plugin_excerpt(&stale, &crow_fs::AllowAll).is_none(),
            "a same-path replacement must not be read under the old entry version"
        );
    }

    #[test]
    fn a_folder_preview_speaks_the_measured_size() {
        let folder = entry("nested", true, 0);
        let line = spoken(&folder, None, Some(KnownSize { bytes: 7_340_032, stale: false, version: 1 }), None);
        assert!(line.contains("7.3 MB"), "{line}");
    }

    #[test]
    fn a_plugin_preview_names_its_provider_in_the_spoken_text() {
        let file = entry("a.txt", false, 3);
        let preview = Preview {
            path: file.path.clone(),
            content: None,
            image: None,
            tags: vec![],
            excerpt: None,
            cloud: None,
            blocked: false,
            plugin: Some(PluginPreviewView {
                provider: "Example Tools".into(),
                preview: PluginPreview { text: Some("hello".into()), table: None },
            }),
        };
        let line = spoken(&file, Some(&preview), None, None);
        assert!(line.contains("plugin preview from Example Tools"), "{line}");
    }

    #[test]
    fn a_file_preview_keeps_its_own_size() {
        let file = entry("a.txt", false, 512_000);
        let line = spoken(&file, None, None, None);
        assert!(line.contains("512.0 KB"), "{line}");
    }
}
