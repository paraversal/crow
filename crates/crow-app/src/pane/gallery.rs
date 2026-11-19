//! Gallery view: a virtualized filename rail with one large preview, or two
//! side-by-side previews when exactly two entries are selected.
//!
//! The rail is the same `Folder` projection as List and Icons, so sorting,
//! filtering, hidden files, type-to-select, watcher updates and the shared
//! selection all work without a second model. File previews reuse the Preview
//! tab's provider pipeline. Plain folders get a bounded metadata sample of
//! their immediate children, loaded off the main thread.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use crow_core::motion::{self, Look};
use crow_core::{KnownSize, ViewMode};
use crow_fs::{AllowAll, CloudStatus, Entry, FolderSample, ReadDecision, ReadGate, ReadHint, SampleIncomplete};
use crow_theme::Theme;
use gpui::{
    Bounds, ClickEvent, Context, Div, IntoElement, MouseButton, MouseDownEvent, Pixels, Role, SharedString,
    UniformListScrollHandle, Window, div, img, prelude::*, px, uniform_list,
};

use super::{
    DragHandoff, DraggedSelection, FilterPlace, Pane, drag_combo, drop_outline, promote_drag, row_icon, shown_offset,
};
use crate::preview::{self, Preview};

/// Fixed rail row height. The metadata layout changes with width, not height,
/// so resizing never moves a row under the pointer.
pub(super) const ROW_HEIGHT: f32 = 48.0;
/// Points requested from Quick Look for a rail thumbnail.
const THUMBNAIL_POINTS: u32 = 20;
const SAMPLE_ROWS: usize = 12;
const SAMPLE_EXAMINED: usize = 256;
const RAIL_MIN: f32 = 170.0;
const RAIL_MAX: f32 = 300.0;
/// Below this preview width two cells stack instead of squeezing side by side.
const SIDE_BY_SIDE_MIN: f32 = 520.0;

pub(super) struct Gallery {
    pub scroll: UniformListScrollHandle,
    pub bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The two compared paths in rail order, while exactly two are selected.
    pair: Option<(PathBuf, PathBuf)>,
    second: Second,
    second_surface: crate::plugins::SurfaceId,
    samples: [Sample; 2],
    pub reveal: Option<Instant>,
}

struct Second {
    path: Option<PathBuf>,
    revision: u64,
    loading: bool,
    preview: Option<Preview>,
    cancel_flag: Arc<AtomicBool>,
}

impl Default for Second {
    fn default() -> Self {
        Self { path: None, revision: 0, loading: false, preview: None, cancel_flag: Arc::new(AtomicBool::new(false)) }
    }
}

#[derive(Default)]
struct Sample {
    path: Option<PathBuf>,
    key: u64,
    generation: u64,
    cancel: crow_fs::sizes::Cancel,
    load: SampleLoad,
}

#[derive(Default)]
enum SampleLoad {
    #[default]
    Idle,
    Loading,
    Ready(Arc<FolderSample>),
    Unavailable(CloudStatus),
    Error(SharedString),
}

enum SampleFailure {
    Unavailable(CloudStatus),
    Error(SharedString),
    Cancelled,
}

impl Gallery {
    pub fn new() -> Self {
        Self {
            scroll: UniformListScrollHandle::new(),
            bounds: Rc::new(Cell::new(None)),
            pair: None,
            second: Second::default(),
            second_surface: crate::plugins::new_surface(),
            samples: Default::default(),
            reveal: None,
        }
    }

    pub fn replace_surface(&mut self) -> crate::plugins::SurfaceId {
        std::mem::replace(&mut self.second_surface, crate::plugins::new_surface())
    }
}

/// The entries the preview shows, in rail order. Exactly two selected entries
/// compare; every other selection follows the cursor, which is always one of
/// the selected rows while one exists.
pub(super) fn targets(pane: &Pane, cx: &gpui::App) -> Vec<Entry> {
    let folder = pane.focused_folder().read(cx);
    if folder.selected_count() == 2 {
        let mut pair = Vec::with_capacity(2);
        for ix in 0..folder.row_count() {
            if let Some((entry, _)) = folder.row(ix)
                && folder.is_selected(&entry.path)
            {
                pair.push(entry.clone());
                if pair.len() == 2 {
                    return pair;
                }
            }
        }
        return pair;
    }
    folder.cursor_entry().cloned().into_iter().collect()
}

/// Start or keep the previews and samples a gallery frame needs. Idempotent:
/// it is called after folder events, view changes and mode switches.
pub(super) fn refresh(pane: &mut Pane, cx: &mut Context<Pane>) {
    if pane.mode != ViewMode::Gallery {
        return;
    }
    let entries = targets(pane, cx);
    let cursor = pane.cursor_entry(cx).map(|entry| entry.path.clone());
    let pair = (entries.len() == 2).then(|| (entries[0].path.clone(), entries[1].path.clone()));
    pane.gallery.pair = pair.clone();
    let second = pair.as_ref().and_then(|(left, right)| {
        let other = match &cursor {
            Some(cursor) if cursor == left => right,
            Some(cursor) if cursor == right => left,
            _ => return None,
        };
        entries.iter().find(|entry| &entry.path == other).cloned()
    });
    load_second(pane, second, cx);
    update_samples(pane, &entries, cx);
}

/// A provider registration or `preview_providers` edit invalidated every
/// preview: keep the targets but fetch them again.
pub(super) fn invalidate(pane: &mut Pane, cx: &mut Context<Pane>) {
    pane.gallery.second.path = None;
    pane.gallery.second.preview = None;
    pane.gallery.second.loading = false;
    refresh(pane, cx);
}

/// A cached cloud status changed: a blocked pair preview retries once when its
/// item becomes local. Other work is already gated fresh in the background.
pub(super) fn cloud_changed(pane: &mut Pane, cx: &mut Context<Pane>) {
    if pane.mode != ViewMode::Gallery {
        return;
    }
    let Some(path) = pane.gallery.second.path.clone() else { return };
    let Some(status) = pane.cloud_status(&path, cx) else { return };
    if let Some(preview) = pane.gallery.second.preview.as_mut()
        && preview.update_cloud(status)
    {
        pane.gallery.second.path = None;
        pane.gallery.second.preview = None;
        pane.gallery.second.loading = false;
        refresh(pane, cx);
    }
}

/// Stop work whose result can no longer be shown.
pub(super) fn release(pane: &mut Pane, cx: &mut Context<Pane>) {
    pane.gallery.second.path = None;
    pane.gallery.second.preview = None;
    pane.gallery.second.loading = false;
    pane.gallery.second.revision = pane.gallery.second.revision.wrapping_add(1);
    crate::plugins::cancel_previews(pane.gallery.second_surface, cx);
    for sample in &mut pane.gallery.samples {
        sample.cancel.cancel();
        sample.generation = sample.generation.wrapping_add(1);
        sample.path = None;
        sample.key = 0;
        sample.load = SampleLoad::Idle;
    }
    pane.gallery.pair = None;
}

fn key_for(pane: &Pane, entry: &Entry, cx: &gpui::App) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    pane.focused_folder().read(cx).plugin_entry_version(entry).hash(&mut hasher);
    cx.global::<crate::ui::Ui>().settings.show_hidden_files.hash(&mut hasher);
    pane.sort.key.hash(&mut hasher);
    pane.sort.ascending.hash(&mut hasher);
    pane.sort.folders_first.hash(&mut hasher);
    hasher.finish()
}

fn load_second(pane: &mut Pane, entry: Option<Entry>, cx: &mut Context<Pane>) {
    let surface = pane.gallery.second_surface;
    let Some(entry) = entry else {
        pane.gallery.second.path = None;
        pane.gallery.second.preview = None;
        pane.gallery.second.loading = false;
        pane.gallery.second.revision = pane.gallery.second.revision.wrapping_add(1);
        pane.gallery.second.cancel();
        crate::plugins::cancel_previews(surface, cx);
        return;
    };
    let revision = key_for(pane, &entry, cx);
    if pane.gallery.second.path.as_deref() == Some(entry.path.as_path())
        && pane.gallery.second.revision == revision
        && (pane.gallery.second.loading || pane.gallery.second.preview.is_some())
    {
        return;
    }
    pane.gallery.second.cancel();
    crate::plugins::cancel_previews(surface, cx);
    pane.gallery.second.path = Some(entry.path.clone());
    pane.gallery.second.revision = revision;
    pane.gallery.second.loading = true;
    pane.gallery.second.preview = None;
    let generation = revision;
    let configured = cx.global::<crate::ui::Ui>().views.resolve(pane.dir()).preview_providers.0.clone();
    let candidates = crate::plugins::preview_candidates(cx, &configured, &entry.path, entry.is_dir, entry.is_symlink);
    let extensions = cx.global::<crate::extensions::Ext>().0.clone();
    let (gate, cloud) = gate_and_cloud(cx, &entry.path);
    let cancel = Arc::new(AtomicBool::new(false));
    pane.gallery.second.cancel_flag = cancel.clone();
    let shown = entry.clone();
    cx.spawn(async move |this, cx| {
        let fetched = preview::load_with_plugins(
            cx,
            surface,
            candidates,
            entry,
            extensions,
            gate,
            cloud,
            None,
            generation,
            cancel.clone(),
        )
        .await;
        this.update(cx, |this, cx| {
            if cancel.load(Ordering::Relaxed)
                || this.gallery.second.revision != generation
                || this.gallery.second.path.as_deref() != Some(shown.path.as_path())
            {
                return;
            }
            let Some(mut fetched) = fetched else {
                this.gallery.second.loading = false;
                return;
            };
            if this.cloud_status(&shown.path, cx).is_some_and(|status| fetched.update_cloud(status)) {
                this.gallery.second.preview = None;
                this.gallery.second.path = None;
                load_second(this, Some(shown), cx);
            } else {
                this.gallery.second.loading = false;
                this.gallery.second.preview = Some(fetched);
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

fn gate_and_cloud(cx: &gpui::App, path: &Path) -> (Arc<dyn ReadGate>, Option<CloudStatus>) {
    match cx.try_global::<crate::cloud_status::CloudStatusService>() {
        Some(service) => {
            let service = service.0.read(cx);
            (service.gate(), service.status(path))
        }
        None => (Arc::new(AllowAll) as Arc<dyn ReadGate>, None),
    }
}

fn sampleable(entry: &Entry) -> bool {
    entry.is_dir && !entry.is_package() && !entry.is_symlink
}

fn update_samples(pane: &mut Pane, entries: &[Entry], cx: &mut Context<Pane>) {
    let wanted: [Option<(PathBuf, u64)>; 2] = std::array::from_fn(|i| {
        entries.get(i).filter(|entry| sampleable(entry)).map(|entry| (entry.path.clone(), key_for(pane, entry, cx)))
    });
    for i in 0..2 {
        let keep = match (&wanted[i], &pane.gallery.samples[i]) {
            (Some((path, key)), sample) => sample.path.as_ref() == Some(path) && sample.key == *key,
            (None, _) => false,
        };
        if !keep {
            pane.gallery.samples[i].cancel.cancel();
            pane.gallery.samples[i].generation = pane.gallery.samples[i].generation.wrapping_add(1);
            pane.gallery.samples[i].path = None;
            pane.gallery.samples[i].key = 0;
            pane.gallery.samples[i].load = SampleLoad::Idle;
        }
    }
    // A surviving entry keeps its result when the layout moves it to the other
    // cell (the cursor moved inside an existing pair).
    for i in 0..2 {
        let Some((path, key)) = wanted[i].clone() else { continue };
        if pane.gallery.samples[i].path.as_ref() == Some(&path) && pane.gallery.samples[i].key == key {
            continue;
        }
        let j = (0..2).find(|j| {
            *j != i && pane.gallery.samples[*j].path.as_ref() == Some(&path) && pane.gallery.samples[*j].key == key
        });
        if let Some(j) = j {
            let moved = std::mem::take(&mut pane.gallery.samples[j]);
            pane.gallery.samples[i] = moved;
        }
    }
    for i in 0..2 {
        let Some((path, key)) = wanted[i].clone() else { continue };
        if pane.gallery.samples[i].path.as_ref() == Some(&path) && pane.gallery.samples[i].key == key {
            continue;
        }
        let Some(entry) = entries.iter().find(|entry| entry.path == path).cloned() else { continue };
        if let Some(loaded) = pane.focused_folder().read(cx).loaded_listing(&path, cx) {
            pane.gallery.samples[i] = Sample {
                path: Some(path),
                key,
                generation: pane.gallery.samples[i].generation.wrapping_add(1),
                cancel: Default::default(),
                load: SampleLoad::Ready(Arc::new(from_listing(&loaded))),
            };
            continue;
        }
        start_sample(pane, i, entry, key, cx);
    }
}

fn from_listing(entries: &[Entry]) -> FolderSample {
    let incomplete =
        (entries.len() > SAMPLE_ROWS).then_some(SampleIncomplete::LimitReached { examined: entries.len() });
    FolderSample {
        // Already sorted by the folder's sort; only the visible cap is needed.
        entries: entries.iter().take(SAMPLE_ROWS).cloned().collect(),
        examined: entries.len(),
        hidden_examined: 0,
        incomplete,
    }
}

fn start_sample(pane: &mut Pane, slot: usize, entry: Entry, key: u64, cx: &mut Context<Pane>) {
    let cancel = crow_fs::sizes::Cancel::new();
    let sample = &mut pane.gallery.samples[slot];
    sample.cancel.cancel();
    sample.generation = sample.generation.wrapping_add(1);
    let generation = sample.generation;
    sample.path = Some(entry.path.clone());
    sample.key = key;
    sample.cancel = cancel.clone();
    sample.load = SampleLoad::Loading;
    let dir = entry.path.clone();
    let checked = dir.clone();
    let show_hidden = cx.global::<crate::ui::Ui>().settings.show_hidden_files;
    let sort = pane.sort;
    let (gate, cloud) = gate_and_cloud(cx, &dir);
    cx.spawn(async move |this, cx| {
        let result = cx
            .background_spawn(async move {
                if gate.check(&checked, ReadHint::for_path(&checked)) != ReadDecision::Readable {
                    return Err(SampleFailure::Unavailable(cloud.unwrap_or(CloudStatus::NotDownloaded)));
                }
                let mut sample =
                    crow_fs::list_sample(&checked, show_hidden, SAMPLE_ROWS, Some(SAMPLE_EXAMINED), &cancel)
                        .map_err(|error| SampleFailure::Error(format!("{error}").into()))?;
                if matches!(sample.incomplete, Some(SampleIncomplete::Cancelled { .. })) {
                    return Err(SampleFailure::Cancelled);
                }
                crow_fs::sort_entries(&mut sample.entries, &sort, &crate::style::kind);
                Ok(sample)
            })
            .await;
        this.update(cx, |this, cx| {
            let sample = &mut this.gallery.samples[slot];
            if sample.generation != generation || sample.path.as_deref() != Some(dir.as_path()) {
                return;
            }
            sample.load = match result {
                Ok(sample) => SampleLoad::Ready(Arc::new(sample)),
                Err(SampleFailure::Unavailable(status)) => SampleLoad::Unavailable(status),
                Err(SampleFailure::Error(error)) => SampleLoad::Error(error),
                Err(SampleFailure::Cancelled) => SampleLoad::Idle,
            };
            cx.notify();
        })
        .ok();
    })
    .detach();
}

impl Second {
    fn cancel(&mut self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
    }
}

/// One frame of the Gallery body.
pub(super) fn render(pane: &mut Pane, window: &mut Window, cx: &mut Context<Pane>) -> impl IntoElement + use<> {
    let folder_entity = pane.folders[0].clone();
    let (rows, folder_name) = {
        let folder = folder_entity.read(cx);
        (folder.row_count(), crate::style::display_name(folder.dir()).to_owned())
    };
    let width = f32::from(pane.list_width).max(1.0);
    let theme = cx.global::<crate::ui::Ui>().theme.clone();
    let rail_width = (width * 0.36).clamp(RAIL_MIN, RAIL_MAX).min(width);
    let preview_width = (width - rail_width - 1.0).max(0.0);
    let entries = targets(pane, cx);
    let cursor = pane.cursor_entry(cx).map(|entry| entry.path.clone());

    // Report the visible rail rows and the preview's folders in one source, so
    // opening Info or scrolling cannot drop the preview's cloud/size requests.
    let visible = visible_rows(pane, rows);
    let mut cloud_paths = Vec::new();
    let mut size_paths = Vec::new();
    {
        let folder = folder_entity.read(cx);
        for ix in visible {
            if let Some((entry, _)) = folder.row(ix) {
                cloud_paths.push(entry.path.clone());
                if entry.is_dir && !entry.is_symlink {
                    size_paths.push(entry.path.clone());
                }
            }
        }
    }
    for (i, entry) in entries.iter().enumerate() {
        cloud_paths.push(entry.path.clone());
        if entry.is_dir && !entry.is_symlink {
            size_paths.push(entry.path.clone());
        }
        if let SampleLoad::Ready(sample) = &pane.gallery.samples[i].load {
            for child in &sample.entries {
                cloud_paths.push(child.path.clone());
                if child.is_dir && !child.is_symlink {
                    size_paths.push(child.path.clone());
                }
            }
        }
    }
    pane.observe_cloud((pane.cloud_source, 0), &cloud_paths, cx);
    pane.observe_sizes((pane.size_source, 0), &size_paths, cx);

    let scrolling = pane.gallery.scroll.clone();
    let bounds = pane.gallery.bounds.clone();
    let render_rows = cx.processor(render_rows);
    let field = pane.render_filter_field(FilterPlace::List, window, cx);
    let no_matches = pane.render_no_matches(cx);
    let footer = pane.render_filter_footer(FilterPlace::List, cx);
    let rail = div()
        .id("gallery-rail")
        .debug_selector(|| "gallery-rail".into())
        .role(Role::ListBox)
        .aria_label(format!("{folder_name}, {rows} items"))
        .w(px(rail_width))
        .flex_none()
        .h_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .border_r_1()
        .border_color(crate::style::color(theme.role("border")))
        .children(field)
        .child({
            let body = match no_matches {
                Some(block) => block,
                None => div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .on_children_prepainted(move |children, _, _| {
                        bounds.set(children.first().copied());
                    })
                    .child(
                        uniform_list("gallery-entries", rows, render_rows).track_scroll(&scrolling).flex_1().min_h_0(),
                    )
                    .into_any_element(),
            };
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .overflow_hidden()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseDownEvent, window, cx| this.press_empty(0, event, window, cx)),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, event: &MouseDownEvent, window, cx| {
                        this.context_press_empty(0, event, window, cx)
                    }),
                )
                .on_drop(
                    cx.listener(|this, paths: &gpui::ExternalPaths, window, cx| this.drop_files(0, paths, window, cx)),
                )
                .on_drop(cx.listener(|this, selection: &DraggedSelection, window, cx| {
                    this.drop_files(0, &selection.external(), window, cx)
                }))
                .child(body)
        })
        .children(footer);

    let now = Instant::now();
    let mode = cx.global::<crate::ui::Ui>().feedback.mode;
    let reveal = pane
        .gallery
        .reveal
        .and_then(|started| motion::navigation_progress(mode, started, now, motion::ms::VIEW_CHANGE));
    if reveal.is_some() {
        window.request_animation_frame();
    } else {
        pane.gallery.reveal = None;
    }
    let opacity = reveal.map_or(1.0, |t| 0.4 + 0.6 * t);
    let preview = render_preview(pane, &entries, &cursor, preview_width, &theme, window, cx).opacity(opacity);
    div()
        .debug_selector(|| "gallery".into())
        .size_full()
        .flex()
        .flex_row()
        .overflow_hidden()
        .child(rail)
        .child(div().flex_1().min_w_0().h_full().flex().flex_col().overflow_hidden().child(preview))
}

fn visible_rows(pane: &Pane, rows: usize) -> std::ops::Range<usize> {
    let Some(bounds) = pane.gallery.bounds.get() else {
        return 0..rows.min(40);
    };
    let offset = shown_offset(&pane.gallery.scroll);
    let height = f32::from(bounds.size.height).max(0.0);
    let first = ((-offset / ROW_HEIGHT).floor().max(0.0) as usize).min(rows);
    let count = (height / ROW_HEIGHT).ceil() as usize + 1;
    first..(first + count).min(rows)
}

fn render_rows(
    pane: &mut Pane,
    range: std::ops::Range<usize>,
    window: &mut Window,
    cx: &mut Context<Pane>,
) -> Vec<impl IntoElement + use<>> {
    let folder_entity = pane.folders[0].clone();
    let folder = folder_entity.read(cx);
    let total = folder.row_count();
    let cursor = folder.cursor_ix();
    let multi = folder.selected_count() > 1;
    let motion = pane.view_motion.as_ref().and_then(|motion| {
        motion.progress(cx.global::<crate::ui::Ui>().feedback.mode, Instant::now()).map(|t| (motion, t))
    });
    let rows: Vec<(usize, Entry, bool, Option<std::ops::Range<usize>>)> = range
        .clone()
        .filter_map(|ix| folder.row(ix).map(|(entry, _)| (ix, entry.clone(), folder.is_selected(&entry.path), None)))
        .collect();
    let ranges: HashMap<usize, std::ops::Range<usize>> =
        range.clone().filter_map(|ix| folder.row_match(ix).map(|range| (ix, range))).collect();
    let selected_entries = folder.selected_entries();
    let active = pane.focus.is_focused(window);
    let theme = cx.global::<crate::ui::Ui>().theme.clone();
    let font_size = px(cx.global::<crate::ui::Ui>().settings.font_size);
    let muted = crate::style::color(theme.role("fg-muted"));
    let selection_bg =
        if active { crate::style::active_selection(&theme) } else { crate::style::inactive_selection(&theme) };
    let icons = pane.icon_resolver(0, cx);
    let thumbnails = crate::thumbnails::service(cx);
    let gate = crate::thumbnails::readable_gate(cx);
    let width = (f32::from(pane.list_width) * 0.36).clamp(RAIL_MIN, RAIL_MAX);
    let wide = width >= 240.0;
    let name_room = px((width - 34.0).max(40.0));
    let mut requests = Vec::new();
    let rows = rows
        .into_iter()
        .map(|(ix, entry, selected, _)| {
            let cloud = pane.cloud_status(&entry.path, cx).or(entry.dataless.then_some(CloudStatus::NotDownloaded));
            let key = crate::thumbnails::ThumbnailKey::new(
                entry.path.clone(),
                folder.plugin_entry_version(&entry),
                THUMBNAIL_POINTS,
            );
            let thumbnails = thumbnails.as_ref();
            let image = thumbnails.and_then(|service| service.read(cx).image(&key));
            let thumbnailable = thumbnails.is_some()
                && (!entry.is_dir || entry.is_package())
                && !entry.is_symlink
                && !entry.dataless
                && cloud.is_none_or(CloudStatus::is_local)
                && !crate::style::native_cloud_icon(cloud, &entry.path);
            if image.is_none() && thumbnailable && !thumbnails.is_some_and(|service| service.read(cx).is_cached(&key)) {
                requests.push(crate::thumbnails::ThumbnailRequest { key: key.clone(), gate: gate.clone() });
            }
            let icon = match image {
                Some(image) => img(image).flex_none().size(px(20.)).into_any_element(),
                None => row_icon(&entry, Look::default(), &theme, 1.0, cloud, &icons).into_any_element(),
            };
            let moving = motion.and_then(|(motion, t)| motion.row(folder_entity.entity_id(), &entry.path, t));
            let fade = motion.filter(|_| moving.is_none()).map_or(1.0, |(_, t)| 0.4 + 0.6 * t);
            let icon = match moving {
                Some(row) => row.icon(icon),
                None => icon,
            };
            let size = if entry.is_dir {
                cx.try_global::<crate::folder_sizes::SizeService>()
                    .and_then(|service| service.0.read(cx).size(&entry.path))
                    .map_or_else(|| "--".to_owned(), |size| crow_fs::human_size(size.bytes))
            } else {
                crow_fs::human_size(entry.size)
            };
            let date = crate::style::format_time_short(entry.modified);
            let name = crate::style::truncate_middle_parts(&entry.name, name_room, font_size, window);
            let name_ranges = ranges.get(&ix).map(|range| crate::style::map_range(&name, range.clone()));
            let dim = crate::style::content_opacity(entry.hidden, false);
            let drag_entries =
                if selected { selected_entries.clone() } else { Arc::new(vec![(entry.path.clone(), entry.is_dir)]) };
            let drag_count = drag_entries.len();
            let drag_label: SharedString = if drag_count > 1 {
                format!("{drag_count} items").into()
            } else {
                crate::style::display_text(&entry.name).into_owned().into()
            };
            let handoff = DragHandoff::new();
            let drag_icon = crate::style::cloud_icon(cloud, &entry.path, entry.is_dir, 22);
            let drop_folder =
                folder.drop_header(ix).and_then(|header| folder.row(header)).map(|(folder, _)| folder.path.clone());
            let drop_target = pane
                .drop_target
                .as_ref()
                .and_then(|target| target.rows.as_ref().map(|rows| rows.contains(&ix)))
                .unwrap_or(false);
            let cloud_spoken =
                cloud.and_then(CloudStatus::spoken_suffix).map(|suffix| format!(", {suffix}")).unwrap_or_default();
            let cursor_here = cursor == Some(ix);
            div()
                .id(ix)
                .role(Role::ListBoxOption)
                .aria_label(format!(
                    "{}, {}, {size}, {date}{cloud_spoken}",
                    crate::style::display_text(&entry.name),
                    crate::style::kind(&entry)
                ))
                .aria_position_in_set(ix + 1)
                .aria_size_of_set(total)
                .aria_selected(selected)
                .when(cursor_here, |row| row.aria_active_descendant())
                .h(px(ROW_HEIGHT))
                .mx(px(4.))
                .rounded(px(6.))
                .relative()
                .flex()
                .items_center()
                .when(selected, |row| row.bg(selection_bg))
                .when(fade < 1.0, |row| row.opacity(fade))
                .when(multi && cursor_here, |row| row.shadow(crate::style::cursor_ring(&theme)))
                .when(drop_target, |row| {
                    row.child(drop_outline(&theme).left(px(2.)).right(px(2.)).top(px(1.)).bottom(px(1.)))
                })
                .debug_selector(|| format!("gallery-row-{}", entry.name))
                .on_drag(DraggedSelection { entries: drag_entries }, {
                    let handoff = handoff.clone();
                    move |_value, offset, window, cx| {
                        drag_combo(drag_icon.clone(), &drag_label, offset, &handoff, window, cx)
                    }
                })
                .external_drag_payload::<DraggedSelection>(move |selection, window, cx| {
                    promote_drag(selection, &handoff, window, cx)
                })
                .when_some(drop_folder, |row, target| {
                    let target_selection = target.clone();
                    row.on_drop(cx.listener(move |this, paths: &gpui::ExternalPaths, window, cx| {
                        this.drop_into(target.clone(), paths, window, cx);
                        cx.stop_propagation();
                    }))
                    .on_drop(cx.listener(
                        move |this, selection: &DraggedSelection, window, cx| {
                            this.drop_into(target_selection.clone(), &selection.external(), window, cx);
                            cx.stop_propagation();
                        },
                    ))
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| this.press(0, ix, event, window, cx)),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        this.context_press(0, Some(ix), window, cx)
                    }),
                )
                .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                    if event.click_count() == 2 {
                        this.open_selected(cx);
                    }
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .justify_center()
                        .gap(px(2.))
                        .pl(px(6.))
                        .pr(px(6.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .min_w_0()
                                .child(icon)
                                .child({
                                    let renaming = pane.renaming.as_ref().is_some_and(|r| r.path == entry.path);
                                    let label = if renaming {
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(|this, _, _, _| this.rename_pressed = true),
                                            )
                                            .child(pane.renaming.as_ref().expect("checked").input.clone())
                                            .into_any_element()
                                    } else {
                                        let text = match name_ranges {
                                            Some(ranges) if ranges.iter().any(|range| !range.is_empty()) => {
                                                crate::style::highlight_text(
                                                    name.text.clone(),
                                                    &ranges,
                                                    &theme,
                                                    selected && active,
                                                )
                                                .into_any_element()
                                            }
                                            _ => div().child(name.text.clone()).into_any_element(),
                                        };
                                        crate::chrome::with_tooltip(
                                            div()
                                                .id(("gallery-rail-name", ix))
                                                .debug_selector(|| format!("gallery-row-name-{}", entry.name))
                                                .flex_1()
                                                .min_w_0()
                                                .overflow_hidden()
                                                .whitespace_nowrap()
                                                .text_ellipsis()
                                                .opacity(dim)
                                                .child(text),
                                            entry.name.clone(),
                                            None,
                                        )
                                        .into_any_element()
                                    };
                                    match moving {
                                        Some(row) => row.label(label),
                                        None => label,
                                    }
                                })
                                .children(crate::cloud_status::badge(cloud, &theme)),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .text_size(px(11.))
                                .text_color(muted)
                                .opacity(dim)
                                .when(wide, |meta| {
                                    meta.child(div().w(px(58.)).text_right().child(size.clone()))
                                        .child(div().flex_1().text_right().child(date.clone()))
                                })
                                .when(!wide, |meta| meta.child(format!("{size} · {date}"))),
                        ),
                )
        })
        .collect::<Vec<_>>();
    if let Some(service) = thumbnails {
        service.update(cx, |service, cx| service.request(requests, cx));
    }
    rows
}

fn render_preview(
    pane: &mut Pane,
    entries: &[Entry],
    cursor: &Option<PathBuf>,
    width: f32,
    theme: &Theme,
    window: &mut Window,
    cx: &mut Context<Pane>,
) -> Div {
    if entries.is_empty() {
        return div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_color(crate::style::color(theme.role("fg-muted")))
            .child("No selection");
    }
    let side_by_side = entries.len() == 2 && width >= SIDE_BY_SIDE_MIN;
    let mut wrapper = div()
        .debug_selector(|| "gallery-preview".into())
        .size_full()
        .p(px(12.))
        .gap(px(12.))
        .overflow_hidden()
        .when(side_by_side, |wrapper| wrapper.flex().flex_row())
        .when(!side_by_side, |wrapper| wrapper.flex().flex_col());
    for (i, entry) in entries.iter().enumerate() {
        let sample = &pane.gallery.samples[i.min(1)];
        let preview = if cursor.as_deref() == Some(entry.path.as_path()) {
            pane.preview(cx)
        } else if pane.gallery.second.path.as_deref() == Some(entry.path.as_path()) {
            pane.gallery.second.preview.as_ref()
        } else {
            None
        };
        let size = folder_size(pane, entry, cx);
        let cloud = pane.cloud_status(&entry.path, cx).or(entry.dataless.then_some(CloudStatus::NotDownloaded));
        let label = if sampleable(entry) {
            let items = match &sample.load {
                SampleLoad::Ready(sample) if sample.incomplete.is_none() => Some(sample.entries.len()),
                _ => None,
            };
            preview::folder_spoken(&entry.name, items, size)
        } else {
            preview::spoken(entry, preview, size, cloud)
        };
        let cell = div()
            .id(("gallery-cell", i))
            .debug_selector({
                let name = entry.name.clone();
                move || format!("gallery-cell-{name}")
            })
            .role(Role::Group)
            .aria_label(label)
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .flex()
            .flex_col()
            .when(side_by_side, |cell| cell.h_full())
            .when(!side_by_side, |cell| cell.w_full())
            .child(if sampleable(entry) {
                folder_cell(pane, entry, sample, size, cloud, theme, width, i, window, cx).into_any_element()
            } else {
                file_cell(pane, entry, preview, size, cloud, theme, i, window, cx).into_any_element()
            });
        wrapper = wrapper.child(cell);
    }
    wrapper
}

fn folder_size(_pane: &Pane, entry: &Entry, cx: &gpui::App) -> Option<KnownSize> {
    if !entry.is_dir || cx.global::<crate::ui::Ui>().settings.folder_sizes != crow_core::FolderSizesMode::Visible {
        return None;
    }
    cx.try_global::<crate::folder_sizes::SizeService>()
        .and_then(|service| service.0.read(cx).size(&entry.path))
        .map(Into::into)
}

#[allow(clippy::too_many_arguments)]
fn file_cell(
    pane: &Pane,
    entry: &Entry,
    preview: Option<&Preview>,
    folder_size: Option<KnownSize>,
    cloud: Option<CloudStatus>,
    theme: &Theme,
    cell: usize,
    _window: &mut Window,
    cx: &gpui::Context<Pane>,
) -> Div {
    let muted = crate::style::color(theme.role("fg-muted"));
    let size = if entry.is_dir {
        folder_size.map_or_else(|| "--".to_owned(), |size| crow_fs::human_size(size.bytes))
    } else {
        crow_fs::human_size(entry.size)
    };
    let name = crate::style::display_text(&entry.name).into_owned();
    let resolver = pane.icon_resolver(0, cx);
    div()
        .size_full()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .min_w_0()
                .child(row_icon(entry, Look::default(), theme, 1.0, cloud, &resolver))
                .child(crate::chrome::with_tooltip(
                    div()
                        .id(("gallery-file-name", cell))
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(13.))
                        .child(name.clone()),
                    name,
                    None,
                ))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(muted)
                        .child(format!("{} · {size}", crate::style::kind(entry))),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_hidden()
                .flex()
                .child(preview::gallery_content(entry, preview, theme, cloud, &resolver)),
        )
}

#[allow(clippy::too_many_arguments)]
fn folder_cell(
    pane: &Pane,
    entry: &Entry,
    sample: &Sample,
    folder_size: Option<KnownSize>,
    cloud: Option<CloudStatus>,
    theme: &Theme,
    width: f32,
    cell: usize,
    window: &mut Window,
    cx: &gpui::Context<Pane>,
) -> Div {
    let muted = crate::style::color(theme.role("fg-muted"));
    let border = crate::style::color(theme.role("border"));
    let resolver = pane.icon_resolver(0, cx);
    let size = folder_size.map_or_else(|| "--".to_owned(), |size| crow_fs::human_size(size.bytes));
    let mut list =
        div().id(("gallery-folder-list", cell)).flex().flex_col().flex_1().min_h_0().overflow_y_scroll().gap(px(1.));
    let footer_text: Option<String> = match &sample.load {
        SampleLoad::Idle | SampleLoad::Loading => Some("Loading…".to_owned()),
        SampleLoad::Ready(sample) if sample.entries.is_empty() && sample.incomplete.is_none() => {
            Some("Empty folder".to_owned())
        }
        SampleLoad::Ready(sample) => {
            for child in &sample.entries {
                let child_cloud =
                    pane.cloud_status(&child.path, cx).or(child.dataless.then_some(CloudStatus::NotDownloaded));
                let child_name = crate::style::truncate_middle_parts(
                    &child.name,
                    px((width - 60.0).max(80.0)),
                    px(cx.global::<crate::ui::Ui>().settings.font_size),
                    window,
                );
                let dim = crate::style::content_opacity(child.hidden, false);
                list = list.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .h(px(24.))
                        .pl(px(4.))
                        .opacity(dim)
                        .child(row_icon(child, Look::default(), theme, 1.0, child_cloud, &resolver))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(child_name.text.clone()),
                        ),
                );
            }
            match sample.incomplete {
                None => Some(format!("{} items", sample.entries.len())),
                Some(_) => Some("Partial contents".to_owned()),
            }
        }
        SampleLoad::Unavailable(status) => {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(folder_heading(entry, &cloud, &size, theme, &resolver, cell))
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(preview::cloud_note(*status, theme)),
                );
        }
        SampleLoad::Error(error) => {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(folder_heading(entry, &cloud, &size, theme, &resolver, cell))
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(12.))
                        .text_color(crate::style::color(theme.role("error")))
                        .child(format!("Can't read this folder · {error}")),
                );
        }
    };
    div()
        .size_full()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(folder_heading(entry, &cloud, &size, theme, &resolver, cell))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .w_full()
                .flex()
                .flex_col()
                .rounded(px(6.))
                .border_1()
                .border_color(border)
                .overflow_hidden()
                .child(div().flex_1().min_h_0().p(px(4.)).child(list))
                .when_some(footer_text, |frame, text| {
                    frame.child(
                        div()
                            .px(px(8.))
                            .py(px(4.))
                            .border_t_1()
                            .border_color(border)
                            .text_size(px(11.))
                            .text_color(muted)
                            .child(text),
                    )
                }),
        )
}

fn folder_heading(
    entry: &Entry,
    cloud: &Option<CloudStatus>,
    size: &str,
    theme: &Theme,
    resolver: &crate::icon_packs::Resolver,
    cell: usize,
) -> Div {
    let muted = crate::style::color(theme.role("fg-muted"));
    let mut detail = format!("Folder · {size}");
    if entry.modified.is_some() {
        detail.push_str(" · ");
        detail.push_str(&crate::style::format_time(entry.modified));
    }
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .min_w_0()
        .child(preview::path_icon(resolver, &entry.path, true, false, *cloud, theme, 40.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(crate::chrome::with_tooltip(
                    div()
                        .id(("gallery-folder-name", cell))
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(14.))
                        .child(crate::style::display_text(&entry.name).into_owned()),
                    entry.name.clone(),
                    None,
                ))
                .child(div().text_size(px(11.)).text_color(muted).child(detail)),
        )
}
