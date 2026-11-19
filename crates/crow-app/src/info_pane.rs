//! The right column's Info tab: Finder's Get Info for the cursor item or the
//! selection, sharing the column with the preview.
//!
//! The cheap fields (name, kind, size, where, dates, item count) come from the
//! listing and render at once. Permissions, owner/group and tags load off the
//! main thread; a result for a previous selection is dropped by generation.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crow_fs::{CloudStatus, Entry};
use crow_theme::Theme;
use gpui::{
    AnyElement, App, Context, Div, Entity, EventEmitter, FontWeight, IntoElement, Render, Role, Subscription, Window,
    div, prelude::*, px,
};

use crate::cloud_status::{self, CloudStatusService};
use crate::folder_sizes::SizeService;
use crate::style::{color, color_alpha, display_path, display_text, format_time, kind};
use crate::ui::Ui;

/// The tab strip's height, inside the existing column.
pub const TAB_STRIP_HEIGHT: f32 = 28.0;
/// How many selected items the multi-selection summary reads tags and sizes for.
/// Beyond this the Tags row says `—` rather than claiming a shared set.
pub const TAG_SCAN_CAP: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InfoTab {
    Preview,
    Info,
    GitDiff,
}

/// What the Info tab describes: one item, or a selection's summary.
#[derive(Clone, Debug)]
pub enum InfoTarget {
    Single { entry: Entry, items: Option<usize> },
    Many(SelectionSummary),
}

/// A multi-selection, reduced to what the summary shows. `bytes` is exact for the
/// file list; `None` when the loader must stat (search results).
#[derive(Clone, Debug)]
pub struct SelectionSummary {
    pub count: usize,
    pub bytes: Option<u64>,
    pub folders: usize,
    pub kind: Option<Arc<str>>,
    pub parent: Option<PathBuf>,
    pub tag_paths: Arc<[PathBuf]>,
    pub tags_truncated: bool,
}

impl InfoTarget {
    /// Whether the cheap fields changed; the expensive load is keyed separately.
    fn same_display(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Single { entry: a, items: ai }, Self::Single { entry: b, items: bi }) => a == b && ai == bi,
            (Self::Many(a), Self::Many(b)) => {
                a.count == b.count
                    && a.bytes == b.bytes
                    && a.folders == b.folders
                    && a.kind == b.kind
                    && a.parent == b.parent
                    && a.tags_truncated == b.tags_truncated
            }
            _ => false,
        }
    }
}

impl SelectionSummary {
    /// The key for the Info pane's load: the selection generation plus the ends of
    /// the capped path list, which differ when two tabs' selections share a generation.
    pub fn key(&self, generation: u64) -> InfoKey {
        InfoKey::Many {
            generation,
            count: self.count,
            first: self.tag_paths.first().cloned(),
            last: self.tag_paths.last().cloned(),
        }
    }
}

/// What makes the target a new item: a path, or a selection's identity. The
/// generation alone is per folder (and per search), so two tabs' selections can
/// share it; the first and last paths keep a switch between them from reusing the
/// other selection's tags and bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InfoKey {
    Single(PathBuf),
    Many { generation: u64, count: usize, first: Option<PathBuf>, last: Option<PathBuf> },
}

/// The expensive fields, read off the main thread.
#[derive(Clone, Debug, Default)]
pub struct LoadedInfo {
    pub entry: Option<Entry>,
    pub permissions: Option<crow_macos::Permissions>,
    pub tags: Vec<String>,
    pub tags_partial: bool,
    pub bytes: Option<u64>,
    pub bytes_partial: bool,
    /// The path is a symlink: its size and dates come from the target, as Finder shows.
    pub symlink: bool,
}

pub enum InfoPaneEvent {
    Navigate(PathBuf),
}

impl EventEmitter<InfoPaneEvent> for InfoPane {}

type Loader = Arc<dyn Fn(&InfoTarget) -> LoadedInfo + Send + Sync>;

struct GitDiffView {
    snapshot: Arc<crow_plugin_git::StatusSnapshot>,
    path: PathBuf,
    result: Option<Result<crow_plugin_git::FileDiff, ()>>,
}

pub struct InfoPane {
    tab: InfoTab,
    pub(crate) preferred_width: Option<gpui::Pixels>,
    splitter_focus: gpui::FocusHandle,
    git_diff: Option<GitDiffView>,
    git_diff_task: Option<gpui::Task<()>>,
    diff_staged: bool,
    /// The column's visibility in this window; `default` is `settings.show_preview`.
    visible: bool,
    /// The user opened or closed the column with ⌘I or ⇧⌘P, not the setting. A narrow
    /// search hides the column by default; an explicit open still shows it.
    explicit: bool,
    default: bool,
    target: Option<InfoTarget>,
    key: Option<InfoKey>,
    loaded: Option<LoadedInfo>,
    generation: u64,
    loader: Loader,
    /// This pane's selected-path source in the cloud-status service.
    cloud_source: u64,
    /// This pane's plugin Info binding.
    plugin_surface: crate::plugins::SurfaceId,
    _cloud_subscription: Option<Subscription>,
}

fn observe_sizes(cx: &mut Context<InfoPane>) {
    if let Some(service) = cx.try_global::<SizeService>() {
        let service = service.0.clone();
        cx.observe(&service, |_, _, cx| cx.notify()).detach();
    }
}

impl InfoPane {
    pub fn splitter_focus(&self) -> &gpui::FocusHandle {
        &self.splitter_focus
    }

    pub fn new(cx: &mut Context<Self>) -> Self {
        let default = cx.global::<Ui>().settings.show_preview;
        let pane = Self::with_default(default, Arc::new(real_load), cx);
        observe_sizes(cx);
        pane
    }

    fn with_default(default: bool, loader: Loader, cx: &mut Context<Self>) -> Self {
        let cloud_service = cx.try_global::<CloudStatusService>().map(|service| service.0.clone());
        let cloud_source =
            cloud_service.as_ref().map(|service| cloud_status::register_source(service, cx)).unwrap_or(0);
        let cloud_subscription = cloud_service.map(|service| cx.observe(&service, |_, _, cx| cx.notify()));
        Self {
            tab: InfoTab::Preview,
            preferred_width: None,
            splitter_focus: cx.focus_handle(),
            git_diff: None,
            git_diff_task: None,
            diff_staged: false,
            visible: default,
            explicit: false,
            default,
            target: None,
            key: None,
            loaded: None,
            generation: 0,
            loader,
            cloud_source,
            plugin_surface: crate::plugins::new_surface(),
            _cloud_subscription: cloud_subscription,
        }
    }

    #[cfg(test)]
    pub fn with_loader(cx: &mut Context<Self>, loader: Loader) -> Self {
        let default = cx.global::<Ui>().settings.show_preview;
        let pane = Self::with_default(default, loader, cx);
        observe_sizes(cx);
        pane
    }

    pub fn tab(&self) -> InfoTab {
        self.tab
    }

    pub fn visible(&self) -> bool {
        self.visible
    }

    /// Whether the column is showing because the user opened it, not because of the
    /// setting. A narrow search uses this to keep an explicit open visible.
    pub fn explicit(&self) -> bool {
        self.explicit
    }

    #[cfg(test)]
    pub fn target(&self) -> Option<&InfoTarget> {
        self.target.as_ref()
    }

    #[cfg(test)]
    pub fn loaded(&self) -> Option<&LoadedInfo> {
        self.loaded.as_ref()
    }

    /// Test-only: land a load result as the background task would, and report
    /// whether its generation was still current.
    #[cfg(test)]
    pub fn install_loaded(&mut self, generation: u64, loaded: LoadedInfo) -> bool {
        if self.generation == generation {
            self.loaded = Some(loaded);
            true
        } else {
            false
        }
    }

    pub fn sync_git_diff(
        &mut self,
        source: Option<(Arc<crow_plugin_git::StatusSnapshot>, PathBuf)>,
        cx: &mut Context<Self>,
    ) {
        use crow_plugin_git::{LineDiff, open_worktree};
        let Some((snapshot, path)) = source else {
            self.git_diff = None;
            self.git_diff_task = None;
            return;
        };
        if self
            .git_diff
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(&current.snapshot, &snapshot) && current.path == path)
        {
            return;
        }
        // Default to the first available changed layer. The layer controls can then be used without rereading the file.
        self.diff_staged = snapshot.entries.get(&path).is_some_and(|status| status.worktree.is_none());
        self.git_diff = Some(GitDiffView { snapshot: snapshot.clone(), path: path.clone(), result: None });
        let repo = snapshot.identity.worktree.clone();
        let worker_snapshot = snapshot.clone();
        let worker_path = path.clone();
        let task = cx.background_spawn(async move {
            let repo = open_worktree(&repo).map_err(|_| ())?;
            LineDiff::new(&repo).and_then(|mut diff| diff.diff(&worker_snapshot, &worker_path)).map_err(|_| ())
        });
        self.git_diff_task = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |info, cx| {
                if let Some(current) = &mut info.git_diff
                    && Arc::ptr_eq(&current.snapshot, &snapshot)
                    && current.path == path
                {
                    current.result = Some(result);
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    pub fn git_diff_body(entity: &Entity<Self>, theme: &Theme, cx: &App) -> AnyElement {
        use crow_plugin_git::{DiffLayer, DiffLine};
        let info = entity.read(cx);
        let Some(view) = &info.git_diff else {
            return div().child("Select a changed file to see its Git diff.").into_any_element();
        };
        let accent = color(theme.role("accent"));
        let staged = info.diff_staged;
        let controls = [false, true].map(|value| {
            let info = entity.clone();
            let label = if value { "Staged" } else { "Unstaged" };
            div()
                .id(("git-diff-layer", usize::from(value)))
                .role(Role::Tab)
                .aria_label(label)
                .aria_selected(value == staged)
                .px_2()
                .py_1()
                .rounded_sm()
                .when(value == staged, |tab| tab.bg(accent.opacity(0.13)))
                .child(label)
                .on_click(move |_, _, cx| {
                    info.update(cx, |info, cx| {
                        info.diff_staged = value;
                        cx.notify();
                    })
                })
        });
        let body = match view
            .result
            .as_ref()
            .map(|result| result.as_ref().map(|diff| if staged { &diff.staged } else { &diff.unstaged }))
        {
            None => div().child("Loading Git diff…"),
            Some(Err(())) | Some(Ok(DiffLayer::Unavailable)) => div().child("Text diff unavailable"),
            Some(Ok(DiffLayer::Clean)) => div().child(if staged { "No staged changes" } else { "No unstaged changes" }),
            Some(Ok(DiffLayer::Text(diff))) if diff.lines.is_empty() => div().child("No line edits"),
            Some(Ok(DiffLayer::Text(diff))) => {
                let rows = diff.lines.iter().map(|line| {
                    let (text, tone) = match line {
                        DiffLine::Hunk { before, after } => (format!("−{before}  +{after}"), "fg-muted"),
                        DiffLine::Removed(line) => (format!("− {line}"), "error"),
                        DiffLine::Added(line) => (format!("+ {line}"), "success"),
                    };
                    div().text_color(color(theme.role(tone))).child(text)
                });
                div()
                    .text_size(px(11.))
                    .font_family("Menlo")
                    .children(rows)
                    .when(diff.truncated, |body| body.child(div().child("Showing the first 240 diff lines")))
            }
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .id("git-diff-layers")
                    .role(Role::TabList)
                    .aria_label("Git diff layers")
                    .flex()
                    .gap_2()
                    .children(controls),
            )
            .child(body)
            .into_any_element()
    }

    pub fn set_tab(&mut self, tab: InfoTab, cx: &mut Context<Self>) {
        if self.tab != tab {
            self.tab = tab;
            if tab != InfoTab::Info {
                self.drop_plugin_info(cx);
            }
            cx.notify();
        }
    }

    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible != visible || self.explicit != visible {
            self.visible = visible;
            self.explicit = visible;
            if !visible {
                self.drop_plugin_info(cx);
            }
            cx.notify();
        }
    }

    #[cfg(test)]
    pub fn plugin_surface_for_test(&self) -> crate::plugins::SurfaceId {
        self.plugin_surface
    }

    /// Window teardown: the info surface is no longer observed.
    pub fn release_plugins(&mut self, cx: &mut Context<Self>) {
        self.drop_plugin_info(cx);
    }

    fn drop_plugin_info(&self, cx: &mut Context<Self>) {
        if cx.try_global::<crate::plugins::Plugins>().is_some() {
            cx.global_mut::<crate::plugins::Plugins>().surfaces.drop_surface(self.plugin_surface);
        }
    }

    /// The setting is the column's default; a change in Settings applies to every
    /// window, while an explicit ⌘I only changes this one. ⇧⌘P writes the setting and
    /// toggles this window, so a setting that now agrees with the explicit state keeps
    /// it (the narrow search's override survives the observer).
    pub fn set_default(&mut self, default: bool, cx: &mut Context<Self>) {
        if default != self.default {
            self.default = default;
            if !self.explicit || self.visible != default {
                self.visible = default;
                self.explicit = false;
            }
            cx.notify();
        }
    }

    /// Point the pane at a new item or selection. `key` is the cheap identity: a
    /// path, or a selection generation. The expensive fields load only when it changes.
    pub fn set_target(&mut self, target: Option<InfoTarget>, key: Option<InfoKey>, cx: &mut Context<Self>) {
        if self.key != key {
            self.key = key;
            self.target = target;
            self.loaded = None;
            self.generation += 1;
            if let Some(service) = cx.try_global::<CloudStatusService>() {
                let path = match &self.target {
                    Some(InfoTarget::Single { entry, .. }) => std::slice::from_ref(&entry.path),
                    _ => &[],
                };
                service.0.clone().update(cx, |service, cx| service.observe((self.cloud_source, 0), path, cx));
            }
            if let Some(target) = self.target.clone() {
                self.start_load(target, cx);
            }
            cx.notify();
        } else if let Some(target) = target {
            if !self.target.as_ref().is_some_and(|old| old.same_display(&target)) {
                self.target = Some(target);
                cx.notify();
            }
        } else if self.target.is_some() {
            self.target = None;
            cx.notify();
        }
    }

    fn start_load(&mut self, target: InfoTarget, cx: &mut Context<Self>) {
        let loader = self.loader.clone();
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let loaded = cx.background_spawn(async move { loader(&target) }).await;
            this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.loaded = Some(loaded);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// What VoiceOver reads for the column while Info is showing.
    pub fn label(&self, cx: &App) -> String {
        match &self.target {
            Some(InfoTarget::Single { entry, items }) => {
                let folder = entry.is_dir && !entry.is_package();
                let detail = if folder {
                    items.map(|n| format!("{n} item{}", if n == 1 { "" } else { "s" })).unwrap_or_default()
                } else if entry.is_dir {
                    "--".to_owned()
                } else {
                    crow_fs::human_size(shown_size(entry, self.loaded.as_ref()))
                };
                let name = display_text(&entry.name);
                let cloud = self
                    .cloud_status(&entry.path, cx)
                    .and_then(CloudStatus::spoken_suffix)
                    .map(|suffix| format!(", {suffix}"))
                    .unwrap_or_default();
                if detail.is_empty() {
                    format!("Info: {name}, {}{cloud}", kind(entry))
                } else {
                    format!("Info: {name}, {}, {detail}{cloud}", kind(entry))
                }
            }
            Some(InfoTarget::Many(summary)) => format!(
                "Info: {} item{}, {}",
                summary.count,
                if summary.count == 1 { "" } else { "s" },
                summary_size(summary, self.loaded.as_ref())
            ),
            None => "Info".to_owned(),
        }
    }
}

impl Render for InfoPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.visible
            && self.tab == InfoTab::Info
            && let Some(InfoTarget::Single { entry, .. }) = self.target.as_ref()
        {
            let parent = entry.path.parent().map_or_else(|| entry.path.clone(), Path::to_path_buf);
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            entry.path.hash(&mut hasher);
            entry.size.hash(&mut hasher);
            entry.modified.hash(&mut hasher);
            crate::plugins::request_info(
                cx,
                self.plugin_surface,
                crate::plugins::ScopeKey::Folder(parent.clone()),
                hasher.finish(),
                parent.display().to_string(),
                entry,
            );
        }
        let theme = cx.global::<Ui>().theme.clone();
        let home = cx.global::<Ui>().home.clone();
        self.body(&theme, &home, cx)
    }
}

impl InfoPane {
    fn cloud_status(&self, path: &Path, cx: &App) -> Option<CloudStatus> {
        cx.try_global::<CloudStatusService>().and_then(|service| service.0.read(cx).status(path))
    }

    fn body(&self, theme: &Theme, home: &Path, cx: &mut Context<Self>) -> Div {
        let Some(target) = &self.target else {
            return div().text_color(color(theme.role("fg-muted"))).child("No selection");
        };
        let loaded = self.loaded.as_ref();
        match target {
            InfoTarget::Single { entry, items } => {
                let cloud = self.cloud_status(&entry.path, cx);
                single_body(entry, *items, loaded, cloud, theme, home, self.plugin_surface, cx)
            }
            InfoTarget::Many(summary) => many_body(summary, loaded, theme, home, cx),
        }
    }
}

/// The tab strip at the top of the right column. Shared by list view, column view
/// and search so the two tabs look and behave the same everywhere.
pub fn tab_strip(info: &Entity<InfoPane>, theme: &Theme, cx: &mut App) -> gpui::Stateful<Div> {
    let selected = info.read(cx).tab;
    let tab = |id: &'static str, label: &'static str, value: InfoTab| {
        let info = info.clone();
        div()
            .id(id)
            .debug_selector(move || id.into())
            .role(Role::Tab)
            .aria_selected(selected == value)
            .aria_label(label)
            .h_full()
            .px_2()
            .flex()
            .items_center()
            .text_size(px(12.))
            .text_color(if selected == value { color(theme.role("fg")) } else { color(theme.role("fg-muted")) })
            .border_b_2()
            .border_color(if selected == value {
                color(theme.role("accent"))
            } else {
                color_alpha(theme.role("accent"), 0)
            })
            .on_click(move |_, _, cx| {
                info.update(cx, |info, cx| info.set_tab(value, cx));
            })
            .child(label)
    };
    div()
        .id("info-tabs")
        .debug_selector(|| "info-tabs".into())
        .role(Role::TabList)
        .aria_label("Preview and Info")
        .h(px(TAB_STRIP_HEIGHT))
        .flex_none()
        .flex()
        .items_stretch()
        .gap_2()
        .px_2()
        .border_b_1()
        .border_color(color(theme.role("border")))
        .child(tab("info-tab-preview", "Preview", InfoTab::Preview))
        .child(tab("info-tab-info", "Info", InfoTab::Info))
        .when(crate::git::provider_enabled(cx), |tabs| {
            tabs.child(tab("info-tab-git-diff", "Git Diff", InfoTab::GitDiff))
        })
}

#[allow(clippy::too_many_arguments)]
fn single_body(
    entry: &Entry,
    items: Option<usize>,
    loaded: Option<&LoadedInfo>,
    cloud: Option<CloudStatus>,
    theme: &Theme,
    home: &Path,
    plugin_surface: crate::plugins::SurfaceId,
    cx: &mut Context<InfoPane>,
) -> Div {
    let fresh = loaded.and_then(|l| l.entry.as_ref());
    let created = entry.created.or_else(|| fresh.and_then(|e| e.created));
    let modified = entry.modified.or_else(|| fresh.and_then(|e| e.modified));
    let added = entry.added.or_else(|| fresh.and_then(|e| e.added));
    let folder = entry.is_dir && !entry.is_package() && !entry.is_symlink;
    let (size, size_stale) = if entry.is_dir && !entry.is_symlink {
        if cx.global::<crate::ui::Ui>().settings.folder_sizes != crow_core::FolderSizesMode::Visible {
            ("--".to_owned(), false)
        } else {
            cx.try_global::<SizeService>()
                .and_then(|service| service.0.read(cx).size(&entry.path))
                .map_or_else(|| ("--".to_owned(), false), |size| (crow_fs::human_size(size.bytes), size.stale))
        }
    } else if entry.is_dir {
        ("--".to_owned(), false)
    } else {
        (crow_fs::human_size(shown_size(entry, loaded)), false)
    };
    let tags = match loaded {
        Some(l) if l.tags_partial => "—".to_owned(),
        Some(l) if l.tags.is_empty() => "No tags".to_owned(),
        Some(l) => l.tags.join(", "),
        None => "…".to_owned(),
    };
    let tags_muted = loaded.is_none() || loaded.is_some_and(|l| l.tags.is_empty() || l.tags_partial);
    let (mode, owner, group) = match loaded {
        Some(l) => (
            l.permissions.as_ref().map_or_else(|| "—".to_owned(), |p| p.mode_string()),
            l.permissions.as_ref().and_then(|p| p.owner.clone()).unwrap_or_else(|| "—".to_owned()),
            l.permissions.as_ref().and_then(|p| p.group.clone()).unwrap_or_else(|| "—".to_owned()),
        ),
        None => ("…".to_owned(), "…".to_owned(), "…".to_owned()),
    };
    let mut body = div()
        .flex()
        .flex_col()
        .child(field(
            "Name",
            div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(display_text(&entry.name).into_owned()),
            theme,
        ))
        .child(field("Kind", div().child(kind(entry).to_string()), theme));
    if let Some(status) = cloud.and_then(CloudStatus::info_text) {
        body = body.child(field(
            "iCloud",
            div().flex().items_center().gap_2().children(cloud_status::badge(cloud, theme)).child(status.to_owned()),
            theme,
        ));
    }
    body = body
        .child(field("Size", div().when(size_stale, |size| size.opacity(0.5)).child(size), theme))
        .child(field("Where", where_value(entry.path.parent(), home, theme, cx), theme))
        .child(field("Created", div().child(format_time(created)), theme))
        .child(field("Modified", div().child(format_time(modified)), theme));
    if added.is_some() {
        body = body.child(field("Added", div().child(format_time(added)), theme));
    }
    body = body.child(field(
        "Tags",
        div().when(tags_muted, |d| d.text_color(color(theme.role("fg-muted")))).child(tags),
        theme,
    ));
    body = body
        .child(field("Permissions", div().font_family("Menlo").child(mode), theme))
        .child(field("Owner", div().child(owner), theme))
        .child(field("Group", div().child(group), theme));
    if folder {
        let items = items.map_or_else(|| "—".to_owned(), |n| format!("{n} item{}", if n == 1 { "" } else { "s" }));
        body = body.child(field("Items", div().child(items), theme));
    }
    for row in crate::plugins::info_rows(cx, plugin_surface).into_iter().take(16) {
        body = body.child(field(&row.label, div().child(crate::style::display_text(&row.value).into_owned()), theme));
    }
    body
}

fn many_body(
    summary: &SelectionSummary,
    loaded: Option<&LoadedInfo>,
    theme: &Theme,
    home: &Path,
    cx: &mut Context<InfoPane>,
) -> Div {
    let count = format!("{} item{}", summary.count, if summary.count == 1 { "" } else { "s" });
    let kind = summary.kind.as_ref().map_or_else(|| "Mixed".to_owned(), |kind| kind.to_string());
    let size = summary_size(summary, loaded);
    let where_value = match &summary.parent {
        Some(parent) => where_value(Some(parent), home, theme, cx),
        None => div().text_color(color(theme.role("fg-muted"))).child("Multiple folders").into_any_element(),
    };
    let tags = match loaded {
        Some(l) if l.tags_partial => "—".to_owned(),
        Some(l) if l.tags.is_empty() => "No tags".to_owned(),
        Some(l) => l.tags.join(", "),
        None => "…".to_owned(),
    };
    let tags_muted = loaded.is_none() || loaded.is_some_and(|l| l.tags.is_empty() || l.tags_partial);
    div()
        .flex()
        .flex_col()
        .child(field("Name", div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(count), theme))
        .child(field("Kind", div().child(kind), theme))
        .child(field("Size", div().child(size), theme))
        .child(field("Where", where_value, theme))
        .child(field(
            "Tags",
            div().when(tags_muted, |d| d.text_color(color(theme.role("fg-muted")))).child(tags),
            theme,
        ))
}

/// A file's size: the listing's value, except for a symlink, whose target's size
/// the background stat read (Finder's Get Info follows the link).
fn shown_size(entry: &Entry, loaded: Option<&LoadedInfo>) -> u64 {
    if loaded.is_some_and(|l| l.symlink) {
        loaded.and_then(|l| l.entry.as_ref()).map_or(entry.size, |fresh| fresh.size)
    } else {
        entry.size
    }
}

fn summary_size(summary: &SelectionSummary, loaded: Option<&LoadedInfo>) -> String {
    let bytes = summary.bytes.or_else(|| loaded.and_then(|l| l.bytes));
    let partial = summary.bytes.is_none() && loaded.is_some_and(|l| l.bytes_partial);
    let mut text = match bytes {
        Some(b) if b > 0 || summary.folders == 0 => crow_fs::human_size(b),
        Some(_) => "--".to_owned(),
        None if loaded.is_some() => "--".to_owned(),
        None => "…".to_owned(),
    };
    if partial {
        text = format!("≥ {text}");
    }
    if summary.folders > 0 {
        text.push_str(&format!(" · {} folder{}", summary.folders, if summary.folders == 1 { "" } else { "s" }));
    }
    text
}

fn where_value(parent: Option<&Path>, home: &Path, theme: &Theme, cx: &mut Context<InfoPane>) -> AnyElement {
    match parent {
        Some(parent) => {
            let target = parent.to_path_buf();
            let text = display_path(parent, home);
            let label = format!("Show {text} in its folder");
            div()
                .id("info-where")
                .debug_selector(|| "info-where".into())
                .role(Role::Button)
                .aria_label(label)
                .text_color(color(theme.role("accent")))
                .cursor_pointer()
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.emit(InfoPaneEvent::Navigate(target.clone()));
                }))
                .child(text)
                .into_any_element()
        }
        None => div().text_color(color(theme.role("fg-muted"))).child("—").into_any_element(),
    }
}

fn field(label: &str, value: impl IntoElement, theme: &Theme) -> Div {
    div()
        .flex()
        .items_start()
        .gap_3()
        .py_1()
        .border_b_1()
        .border_color(color(theme.role("border")))
        .child(div().w(px(84.)).flex_none().text_color(color(theme.role("fg-muted"))).child(label.to_owned()))
        .child(div().flex_1().min_w_0().child(value))
}

fn real_load(target: &InfoTarget) -> LoadedInfo {
    match target {
        InfoTarget::Single { entry, .. } => {
            let path = &entry.path;
            // io-ok: runs in the Info pane's background loader (`real_load`).
            let symlink = std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink());
            LoadedInfo {
                entry: fresh_entry(path),
                permissions: crow_macos::permissions(path),
                tags: crow_macos::tags(path),
                tags_partial: false,
                bytes: None,
                bytes_partial: false,
                symlink,
            }
        }
        InfoTarget::Many(summary) => {
            let mut tags: Option<Vec<String>> = None;
            let mut bytes = summary.bytes;
            let mut scanned = 0usize;
            // A truncated selection has no honest shared-tag answer; skip the reads
            // and say so. Sizes still come from the capped scan when they are unknown.
            let read_tags = !summary.tags_truncated;
            for path in summary.tag_paths.iter() {
                if read_tags {
                    let item_tags = crow_macos::tags(path);
                    tags = Some(match tags {
                        None => item_tags,
                        Some(shared) => shared.into_iter().filter(|tag| item_tags.contains(tag)).collect(),
                    });
                }
                // io-ok: runs in the Info pane's background loader (`real_load`).
                if bytes.is_none()
                    && let Ok(meta) = std::fs::metadata(path) // io-ok: background loader
                    && meta.is_file()
                {
                    *bytes.get_or_insert(0) += meta.len();
                }
                scanned += 1;
            }
            LoadedInfo {
                entry: None,
                permissions: None,
                tags: tags.unwrap_or_default(),
                tags_partial: summary.tags_truncated || scanned < summary.count,
                bytes,
                bytes_partial: summary.bytes.is_none() && scanned < summary.count,
                symlink: false,
            }
        }
    }
}

/// A fresh stat, following a symlink so its target's size and dates show (Finder's
/// Get Info does); a dangling link keeps the link's own.
fn fresh_entry(path: &Path) -> Option<Entry> {
    let mut entry = crow_fs::stat_entry(path).ok().flatten()?;
    // io-ok: runs in the Info pane's background loader (`real_load`).
    if let Ok(meta) = std::fs::metadata(path) {
        // io-ok: background loader
        entry.is_dir = meta.is_dir();
        entry.size = if entry.is_dir { 0 } else { meta.len() };
        entry.modified = meta.modified().ok().or(entry.modified);
        entry.created = meta.created().ok().or(entry.created);
    }
    Some(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: PathBuf) -> Entry {
        Entry {
            name: path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
            path,
            is_dir: false,
            is_symlink: false,
            hidden: false,
            dataless: false,
            size: 12,
            modified: None,
            created: None,
            added: None,
        }
    }

    #[test]
    fn loaded_fields_match_stat() {
        use std::os::unix::fs::PermissionsExt;
        let file = tempfile::NamedTempFile::new().expect("temp file");
        let path = file.path().to_path_buf();
        std::fs::write(&path, vec![b'x'; 1234]).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        let entry = crow_fs::stat_entry(&path).unwrap().unwrap();
        let loaded = real_load(&InfoTarget::Single { entry, items: None });
        let fresh = loaded.entry.expect("a fresh stat");
        assert_eq!(fresh.size, 1234, "the size matches stat");
        assert_eq!(loaded.permissions.expect("permissions").mode_string(), "-rw-r-----");
    }

    #[test]
    fn a_symlink_reports_its_target_and_a_dangling_one_survives() {
        use std::os::unix::fs::symlink;
        let guard = tempfile::Builder::new().prefix("crow-info-link-").tempdir().expect("temp dir");
        let dir = guard.path();
        let target = dir.join("target.txt");
        std::fs::write(&target, vec![b'x'; 42]).unwrap();
        let link = dir.join("link.txt");
        symlink(&target, &link).unwrap();
        let entry = crow_fs::stat_entry(&link).unwrap().unwrap();
        let loaded = real_load(&InfoTarget::Single { entry: entry.clone(), items: None });
        assert!(loaded.symlink, "the loader noticed the link");
        assert_eq!(loaded.entry.as_ref().expect("a fresh stat").size, 42, "the target's size, as Finder shows");
        assert_eq!(shown_size(&entry, Some(&loaded)), 42, "the UI shows the target's size");
        assert!(loaded.permissions.expect("permissions").mode_string().starts_with('-'));

        let dangling = dir.join("dangling");
        symlink(dir.join("missing"), &dangling).unwrap();
        let entry = crow_fs::stat_entry(&dangling).unwrap().unwrap();
        let loaded = real_load(&InfoTarget::Single { entry, items: None });
        assert!(loaded.entry.is_some(), "the link itself still stats");
        assert!(loaded.permissions.is_some(), "the dangling link keeps its own mode");
    }

    #[gpui::test]
    fn a_new_multi_selection_reloads_when_two_generations_match(cx: &mut gpui::TestAppContext) {
        cx.update(crate::ui::init_for_tests);
        let pane = cx.new(|cx| InfoPane::with_loader(cx, Arc::new(|_| LoadedInfo::default())));
        let summary = |first: &str, last: &str| {
            InfoTarget::Many(SelectionSummary {
                count: 2,
                bytes: Some(0),
                folders: 0,
                kind: None,
                parent: Some(PathBuf::from("/tmp")),
                tag_paths: vec![PathBuf::from(first), PathBuf::from(last)].into(),
                tags_truncated: false,
            })
        };
        let first = summary("/tmp/a", "/tmp/b");
        let second = summary("/other/c", "/other/d");
        pane.update(cx, |pane, cx| {
            let InfoTarget::Many(summary) = &first else { unreachable!() };
            pane.set_target(Some(first.clone()), Some(summary.key(1)), cx);
        });
        let generation = pane.read_with(cx, |pane, _| pane.generation);
        pane.update(cx, |pane, _| {
            assert!(pane.install_loaded(generation, LoadedInfo { tags: vec!["old".into()], ..Default::default() }));
        });

        // Another tab's selection with the same generation and count: its tags must load.
        pane.update(cx, |pane, cx| {
            let InfoTarget::Many(summary) = &second else { unreachable!() };
            pane.set_target(Some(second.clone()), Some(summary.key(1)), cx);
        });
        assert!(
            pane.read_with(cx, |pane, _| pane.loaded().is_none()),
            "the other selection's tags must not survive the switch"
        );
    }

    #[gpui::test]
    fn a_stale_load_does_not_replace_the_new_selection(cx: &mut gpui::TestAppContext) {
        cx.update(crate::ui::init_for_tests);
        // The loader never runs here: the test lands results through `install_loaded`
        // so it can control which generation finishes first.
        let pane = cx.new(|cx| InfoPane::with_loader(cx, Arc::new(|_| LoadedInfo::default())));
        let first = PathBuf::from("/tmp/crow-info-first");
        let second = PathBuf::from("/tmp/crow-info-second");

        pane.update(cx, |pane, cx| {
            pane.set_target(
                Some(InfoTarget::Single { entry: entry(first.clone()), items: None }),
                Some(InfoKey::Single(first.clone())),
                cx,
            );
        });
        let first_generation = pane.read_with(cx, |pane, _| pane.generation);
        pane.update(cx, |pane, cx| {
            pane.set_target(
                Some(InfoTarget::Single { entry: entry(second.clone()), items: None }),
                Some(InfoKey::Single(second.clone())),
                cx,
            );
        });
        let second_generation = pane.read_with(cx, |pane, _| pane.generation);
        assert_ne!(first_generation, second_generation, "a new selection has a new generation");
        assert!(pane.read_with(cx, |pane, _| pane.loaded().is_none()), "a new selection clears the old fields");

        // The first load finishes late; its generation is stale and must be dropped.
        let stale = pane.update(cx, |pane, _| {
            pane.install_loaded(first_generation, LoadedInfo { tags: vec!["old".into()], ..Default::default() })
        });
        assert!(!stale, "the old generation is rejected");
        assert!(pane.read_with(cx, |pane, _| pane.loaded().is_none()), "the old item's tags never show");

        let current = pane.update(cx, |pane, _| {
            pane.install_loaded(second_generation, LoadedInfo { tags: vec!["new".into()], ..Default::default() })
        });
        assert!(current, "the current generation lands");
        assert_eq!(
            pane.read_with(cx, |pane, _| pane.loaded().map(|l| l.tags.clone())),
            Some(vec!["new".to_owned()]),
            "the current selection's tags show"
        );
    }
}
