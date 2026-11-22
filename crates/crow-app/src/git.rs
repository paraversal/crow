//! Read-only Git snapshots for the bundled `git` plugin and Projects.
//!
//! Repository discovery and status run in GPUI's background executor, bounded
//! to one scan per pending folder/repository. Snapshots are shared by every
//! window; rows, status and Info read the cache and never touch disk. One
//! recursive watcher per active repository coalesces FSEvents refreshes.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crow_core::{SectionDestination, SectionRow};
use crow_plugin::{EntryHostData, RepositoryData};
use crow_plugin_git::{LineCounts, RepositoryIdentity, StatusSnapshot};
use gpui::{App, AppContext as _, Global};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

/// How many unknown folder scopes may be scanning at once.
const MAX_PENDING_SCANS: usize = 8;
const MAX_WATCHED_REPOS: usize = 16;
/// Repo totals share the visible-row diff cache and have their own per-repo limit.
const MAX_SUMMARY_FILES: usize = 64;
/// How many line batches can run concurrently, and rows started per poll.
const MAX_LINE_JOBS: usize = 4;
const MAX_LINE_ROWS: usize = 64;
/// A repository watcher burst is merged for this long before rescanning.
const WATCH_DEBOUNCE: Duration = Duration::from_millis(250);

#[derive(Clone, Debug)]
struct FolderCache {
    repo: Option<FolderRepo>,
    checked: Instant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FolderRepo {
    worktree: PathBuf,
    /// The requested folder's path relative to the worktree, for mapping row
    /// paths even when the request used a symlinked alias (e.g. /var vs /private/var).
    alias_prefix: Option<PathBuf>,
}

/// A known repository is rechecked on focus/render after this long; its watcher
/// is the primary invalidation path. A folder with no repository is rechecked
/// sooner, so a newly initialized repository appears without a restart.
const REPO_TTL: Duration = Duration::from_secs(30);
const NO_REPO_TTL: Duration = Duration::from_secs(2);

struct RepoCache {
    snapshot: Option<Arc<StatusSnapshot>>,
    checked: Instant,
}

struct RepoWatch {
    worktree: PathBuf,
    changed: Arc<AtomicBool>,
    watcher: Option<RecommendedWatcher>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PluginRefresh {
    Lines,
    Status,
    Reset,
}

fn plugin_refresh(previous: Option<&StatusSnapshot>, next: Option<&StatusSnapshot>) -> Option<PluginRefresh> {
    match (previous, next) {
        (None, None) => None,
        // File contents can change while the Git status stays the same.
        (Some(old), Some(new)) if old == new => Some(PluginRefresh::Lines),
        (Some(old), Some(new)) if old.identity == new.identity && old.unavailable == new.unavailable => {
            Some(PluginRefresh::Status)
        }
        _ => Some(PluginRefresh::Reset),
    }
}

enum GitEvent {
    Repo { worktree: PathBuf, snapshot: Option<Arc<StatusSnapshot>>, refresh: Option<PluginRefresh> },
    Watch { changed: Arc<AtomicBool>, watcher: Option<RecommendedWatcher> },
    Lines { worktree: PathBuf, snapshot: Arc<StatusSnapshot>, results: Vec<(PathBuf, Option<LineCounts>)> },
    Project { scan: Arc<AtomicBool>, project: ProjectRow },
    ProjectsDone { scan: Arc<AtomicBool>, truncated: bool },
}

#[derive(Clone, Debug)]
struct ProjectRow {
    path: PathBuf,
    identity: (u64, u64),
    head: String,
    changed: Option<usize>,
    unavailable: bool,
}

pub struct Git {
    enabled: bool,
    folders: HashMap<PathBuf, FolderCache>,
    repos: HashMap<PathBuf, RepoCache>,
    pending_folders: HashSet<PathBuf>,
    pending_repos: HashSet<PathBuf>,
    dirty: HashMap<PathBuf, Instant>,
    cancel: Arc<AtomicBool>,
    lines: HashMap<(PathBuf, PathBuf), Option<LineCounts>>,
    wanted_lines: HashMap<(PathBuf, PathBuf), HashSet<PathBuf>>,
    pending_lines: HashSet<(PathBuf, PathBuf)>,
    pending_line_jobs: usize,
    watches: Vec<RepoWatch>,
    project_roots: Vec<PathBuf>,
    project_rows: HashMap<PathBuf, ProjectRow>,
    project_truncated: bool,
    project_cancel: Arc<AtomicBool>,
    tx: Sender<GitEvent>,
    rx: Receiver<GitEvent>,
}

impl Global for Git {}

pub fn init_at(cx: &mut App) {
    cx.set_global(Git::new(false));
}

impl Git {
    fn new(enabled: bool) -> Self {
        let (tx, rx) = channel();
        Self {
            enabled,
            folders: HashMap::new(),
            repos: HashMap::new(),
            pending_folders: HashSet::new(),
            pending_repos: HashSet::new(),
            dirty: HashMap::new(),
            cancel: Arc::new(AtomicBool::new(false)),
            lines: HashMap::new(),
            wanted_lines: HashMap::new(),
            pending_lines: HashSet::new(),
            pending_line_jobs: 0,
            watches: Vec::new(),
            project_roots: Vec::new(),
            project_rows: HashMap::new(),
            project_truncated: false,
            project_cancel: Arc::new(AtomicBool::new(false)),
            tx,
            rx,
        }
    }

    fn snapshot(&self, worktree: &Path) -> Option<&Arc<StatusSnapshot>> {
        self.repos.get(worktree)?.snapshot.as_ref()
    }

    fn queue_repo_if_stale(&mut self, worktree: &Path) {
        let fresh = self.repos.get(worktree).is_some_and(|cache| {
            let ttl = if cache.snapshot.is_some() { REPO_TTL } else { NO_REPO_TTL };
            cache.checked.elapsed() < ttl
        });
        if !fresh && !self.pending_repos.contains(worktree) && !self.dirty.contains_key(worktree) {
            self.dirty.insert(worktree.to_path_buf(), Instant::now());
        }
    }
}

#[cfg(test)]
pub fn init_for_tests(cx: &mut App) {
    init_at(cx);
}

/// Follow the bundled package's approved runtime state.
pub fn sync_enabled(cx: &mut App) {
    let running = crate::plugins::bundled_git_running(cx);
    let Some(git) = cx.try_global::<Git>() else { return };
    if git.enabled == running {
        return;
    }
    git.cancel.store(true, Ordering::Relaxed);
    git.project_cancel.store(true, Ordering::Relaxed);
    // Old jobs own senders to the retired channel.
    cx.set_global(Git::new(running));
    crate::plugins::invalidate_git(cx);
    cx.refresh_windows();
}

/// The folder's repository snapshot, if one has been scanned. Missing scopes
/// start one bounded background scan.
pub fn repository_data(cx: &mut App, folder: &Path) -> Option<RepositoryData> {
    request_folder(cx, folder);
    let git = cx.try_global::<Git>()?;
    let worktree = &git.folders.get(folder)?.repo.as_ref()?.worktree;
    Some(repository_data_from(git.snapshot(worktree)?))
}

/// Look up the exact folder and return its worktree-relative scope.
pub fn snapshot_for_folder(cx: &mut App, folder: &Path) -> Option<(Arc<StatusSnapshot>, PathBuf)> {
    request_folder(cx, folder);
    let git = cx.try_global::<Git>()?;
    let repo = git.folders.get(folder)?.repo.as_ref()?;
    let relative = repo.alias_prefix.as_deref().or_else(|| folder.strip_prefix(&repo.worktree).ok())?;
    Some((git.snapshot(&repo.worktree)?.clone(), relative.to_owned()))
}

pub struct PinSummary {
    pub branch: String,
    pub badge: String,
    pub spoken: String,
}

pub fn pin_summary(cx: &mut App, path: &Path, want_lines: bool) -> Option<PinSummary> {
    let (snapshot, relative) = snapshot_for_folder(cx, path)?;
    if !relative.as_os_str().is_empty() {
        return None;
    }
    let data = repository_data_from(&snapshot);
    let branch = if data.detached {
        format!("Detached at {}", data.head.as_deref().unwrap_or("HEAD"))
    } else if data.unborn {
        format!("{} (unborn)", data.branch.as_deref().unwrap_or("HEAD"))
    } else {
        data.branch.unwrap_or_else(|| "HEAD".into())
    };
    let mut detail = branch.clone();
    if let Some((ahead, behind)) = data.ahead.zip(data.behind) {
        detail.push_str(&format!(" ↑{ahead} ↓{behind}"));
    }
    let mut spoken = branch;
    if let Some((ahead, behind)) = data.ahead.zip(data.behind) {
        spoken.push_str(&format!(", {ahead} ahead and {behind} behind last fetched upstream"));
    }
    let badge = if data.unavailable {
        spoken.push_str(", Git status unavailable");
        "—".into()
    } else {
        spoken.push_str(&format!(", {} changed paths", data.changed));
        if snapshot.has_conflicts {
            spoken.push_str(", contains conflicts");
            format!("U {}", data.changed)
        } else {
            data.changed.to_string()
        }
    };
    if want_lines && data.changed > 0 && snapshot.entries.len() <= MAX_SUMMARY_FILES {
        let (mut additions, mut removals, mut files) = (0u64, 0u64, 0);
        let mut pending = false;
        let git = cx.global_mut::<Git>();
        for (relative, _) in snapshot.entries.iter().filter(|(_, status)| status.can_count_lines()) {
            let key = (snapshot.identity.worktree.clone(), relative.clone());
            match git.lines.get(&key) {
                Some(Some(counts)) => {
                    additions += u64::from(counts.insertions);
                    removals += u64::from(counts.removals);
                    files += 1;
                }
                Some(None) => {}
                None => {
                    git.wanted_lines.entry(key).or_default().insert(path.join(relative));
                    pending = true;
                }
            }
        }
        if !pending && files > 0 {
            let partial = if files == data.changed { "" } else { "~" };
            detail.push_str(&format!(" · {partial}+{additions}/−{removals}"));
            spoken
                .push_str(&format!(", {additions} additions and {removals} deletions across {files} countable files"));
        }
    }
    Some(PinSummary { branch: detail, badge, spoken })
}

fn repository_data_from(snapshot: &StatusSnapshot) -> RepositoryData {
    let (branch, head) = match &snapshot.head.state {
        crow_plugin_git::HeadState::Branch { name } => (Some(name.clone()), None),
        crow_plugin_git::HeadState::Unborn { branch } => (Some(branch.clone()), None),
        crow_plugin_git::HeadState::Detached { short_id } => (None, Some(short_id.clone())),
        crow_plugin_git::HeadState::Bare => (None, None),
    };
    RepositoryData {
        branch,
        head,
        detached: snapshot.head.is_detached(),
        unborn: snapshot.head.is_unborn(),
        ahead: snapshot.head.upstream.map(|upstream| upstream.ahead),
        behind: snapshot.head.upstream.map(|upstream| upstream.behind),
        changed: snapshot.changed_count.min(u32::MAX as usize) as u32,
        last_commit: snapshot.last_commit.clone(),
        unavailable: snapshot.unavailable,
    }
}

/// One row's host data. `None` means clean (no contribution).
pub fn entry_host(cx: &mut App, folder: &Path, path: &Path, is_dir: bool, want_lines: bool) -> Option<EntryHostData> {
    request_folder(cx, folder);
    let git = cx.try_global::<Git>()?;
    let repo = git.folders.get(folder)?.repo.as_ref()?;
    let snapshot = git.snapshot(&repo.worktree)?;
    let relative = match &repo.alias_prefix {
        Some(prefix) => Cow::Owned(prefix.join(path.strip_prefix(folder).ok()?)),
        None => Cow::Borrowed(path.strip_prefix(&repo.worktree).ok()?),
    };
    let status = snapshot.lookup_with_ignored_ancestors(&relative);
    if is_dir
        && status.is_none_or(|status| !status.is_changed())
        && let Some(kind) = snapshot.affected_folders.get(relative.as_ref())
    {
        return Some(EntryHostData {
            handle: crow_core::EntryHandle(1),
            badge: kind.badge().to_owned(),
            label: format!("contains {} paths", kind.label()),
            index: None,
            worktree: None,
            ignored: false,
            lines_changed: None,
        });
    }
    let status = status?;
    let badge = status.badge();
    let mut data = EntryHostData {
        handle: crow_core::EntryHandle(1),
        badge: badge.map(|kind| kind.badge().to_owned()).unwrap_or_default(),
        label: badge.map(|kind| kind.label().to_owned()).unwrap_or_default(),
        index: status.index.map(|kind| kind.label().to_owned()),
        worktree: status.worktree.map(|kind| kind.label().to_owned()),
        ignored: status.ignored,
        lines_changed: None,
    };
    if want_lines && status.can_count_lines() {
        let key = (repo.worktree.clone(), relative.into_owned());
        if let Some(counts) = git.lines.get(&key) {
            data.lines_changed = counts.map(|counts| counts.insertions.saturating_add(counts.removals));
        } else {
            // Several windows may request the same Git path through different aliases.
            cx.global_mut::<Git>().wanted_lines.entry(key).or_default().insert(path.to_path_buf());
        }
    }
    Some(data)
}

fn request_folder(cx: &mut App, folder: &Path) {
    let Some(git) = cx.try_global::<Git>().filter(|git| git.enabled) else { return };
    if let Some(cache) = git.folders.get(folder) {
        let ttl = if cache.repo.is_some() { REPO_TTL } else { NO_REPO_TTL };
        if cache.checked.elapsed() < ttl {
            return;
        }
    }
    if git.pending_folders.len() >= MAX_PENDING_SCANS || git.pending_folders.contains(folder) {
        return;
    }
    let folder = folder.to_path_buf();
    let cancel = git.cancel.clone();
    cx.global_mut::<Git>().pending_folders.insert(folder.clone());
    let task = cx.background_spawn(async move {
        let repo = resolve_folder(&folder);
        (folder, repo)
    });
    cx.spawn(async move |cx| {
        let (folder, repo) = task.await;
        cx.update(|cx| {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let git = cx.global_mut::<Git>();
            git.pending_folders.remove(&folder);
            let watch = repo.as_ref().and_then(|repo| {
                git.queue_repo_if_stale(&repo.worktree);
                git.snapshot(&repo.worktree).map(|snapshot| snapshot.identity.clone())
            });
            let now = Instant::now();
            let old_repo = git.folders.get(&folder).and_then(|cache| cache.repo.as_ref());
            let changed = old_repo != repo.as_ref();
            let invalidated = changed && old_repo.is_some();
            git.folders.insert(folder.clone(), FolderCache { repo, checked: now });
            if let Some(identity) = watch {
                ensure_watch(cx, identity);
            }
            request_repos(cx, now);
            if invalidated {
                crate::plugins::refresh_git(cx, PluginRefresh::Reset, |path| path == folder);
            }
            if changed {
                cx.refresh_windows();
            }
        });
    })
    .detach();
}

fn resolve_folder(folder: &Path) -> Option<FolderRepo> {
    let canonical = std::fs::canonicalize(folder).ok()?; // io-ok: inside background_spawn
    let worktree = crow_plugin_git::enclosing_worktree(&canonical)?;
    let relative = canonical.strip_prefix(&worktree).ok()?;
    let alias_prefix = (folder.strip_prefix(&worktree).ok() != Some(relative)).then(|| relative.to_path_buf());
    Some(FolderRepo { worktree, alias_prefix })
}

fn scan_repository(worktree: &Path, cancel: &AtomicBool, gate: Arc<dyn crow_fs::ReadGate>) -> Option<StatusSnapshot> {
    let started = Instant::now();
    let repo = crow_plugin_git::open_worktree(worktree).ok()?;
    let snapshot = if crow_plugin_git::is_cloud_backed_worktree(worktree) {
        crow_plugin_git::unavailable_snapshot(&repo)
    } else {
        crow_plugin_git::status_snapshot_gated(&repo, cancel, gate)
    }
    .ok()?;
    tracing::info!(
        target: "crow::git",
        path = %snapshot.identity.worktree.display(),
        entries = snapshot.entries.len(),
        changed = snapshot.changed_count,
        elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
        "git status"
    );
    Some(snapshot)
}

fn request_repos(cx: &mut App, now: Instant) {
    let git = cx.global_mut::<Git>();
    let jobs: Vec<_> = git
        .dirty
        .iter()
        .filter(|(path, due)| **due <= now && !git.pending_repos.contains(*path))
        .take(MAX_PENDING_SCANS.saturating_sub(git.pending_repos.len()))
        .map(|(path, _)| path.clone())
        .collect();
    let sender = git.tx.clone();
    let cancel = git.cancel.clone();
    for worktree in &jobs {
        git.dirty.remove(worktree);
        git.pending_repos.insert(worktree.clone());
    }
    let gate = crate::cloud_status::read_gate(cx);
    for worktree in jobs {
        let gate = gate.clone();
        let previous = cx.global::<Git>().snapshot(&worktree).cloned();
        let sender = sender.clone();
        let cancel = cancel.clone();
        cx.background_spawn(async move {
            let snapshot = scan_repository(&worktree, &cancel, gate).map(Arc::new);
            let refresh = plugin_refresh(previous.as_deref(), snapshot.as_deref());
            sender.send(GitEvent::Repo { worktree, snapshot, refresh }).ok();
        })
        .detach();
    }
}

fn ensure_watch(cx: &mut App, identity: RepositoryIdentity) {
    let git = cx.global_mut::<Git>();
    if let Some(index) = git.watches.iter().position(|watch| watch.worktree == identity.worktree) {
        let watch = git.watches.remove(index);
        git.watches.push(watch);
        return;
    }
    if git.watches.len() == MAX_WATCHED_REPOS {
        git.watches.remove(0);
    }
    let changed = Arc::new(AtomicBool::new(false));
    let sender = git.tx.clone();
    git.watches.push(RepoWatch { worktree: identity.worktree.clone(), changed: changed.clone(), watcher: None });
    cx.background_spawn(async move {
        let watcher = watch_repository(&identity, changed.clone());
        sender.send(GitEvent::Watch { changed, watcher }).ok();
    })
    .detach();
}

fn watch_repository(identity: &RepositoryIdentity, changed: Arc<AtomicBool>) -> Option<RecommendedWatcher> {
    let mut watcher = match notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok() {
            changed.store(true, Ordering::Relaxed);
        }
    }) {
        Ok(watcher) => watcher,
        Err(error) => {
            tracing::warn!("git watch: {error}");
            return None;
        }
    };
    let mut paths = watcher.paths_mut();
    let worktree = &identity.worktree;
    let common = &identity.common_dir;
    let mut watch_paths = vec![worktree];
    if !common.starts_with(worktree) {
        watch_paths.push(common);
    }
    if !identity.git_dir.starts_with(worktree) && !identity.git_dir.starts_with(common) {
        watch_paths.push(&identity.git_dir);
    }
    for path in watch_paths {
        if let Err(error) = paths.add(path, RecursiveMode::Recursive) {
            tracing::warn!("git watch {}: {error}", path.display());
        }
    }
    if let Err(error) = paths.commit() {
        tracing::warn!("git watch {}: {error}", identity.worktree.display());
    }
    Some(watcher)
}

/// Drain background results and watcher events. Called by the 100 ms app poll.
pub fn poll(cx: &mut App) {
    let Some(git) = cx.try_global::<Git>().filter(|git| git.enabled) else { return };
    let now = Instant::now();
    let events: Vec<_> = git.rx.try_iter().collect();
    let git = cx.global_mut::<Git>();
    for watch in &git.watches {
        if watch.changed.swap(false, Ordering::Relaxed) {
            git.dirty.insert(watch.worktree.clone(), now + WATCH_DEBOUNCE);
        }
    }

    let mut repo_refreshes = Vec::new();
    let mut projects_changed = false;
    let mut line_paths = Vec::new();
    let mut watches = Vec::new();
    for event in events {
        match event {
            GitEvent::Repo { worktree, snapshot, refresh } => {
                git.pending_repos.remove(&worktree);
                if let Some(snapshot) = &snapshot {
                    watches.push(snapshot.identity.clone());
                }
                if let Some(refresh) = refresh {
                    git.lines.retain(|(repo, _), _| repo != &worktree);
                    git.wanted_lines.retain(|(repo, _), _| repo != &worktree);
                    repo_refreshes.push((worktree.clone(), refresh));
                }
                git.repos.insert(worktree, RepoCache { snapshot, checked: now });
            }
            GitEvent::Watch { changed, watcher } => {
                if let Some(watch) = git.watches.iter_mut().find(|watch| Arc::ptr_eq(&watch.changed, &changed)) {
                    watch.watcher = watcher;
                }
            }
            GitEvent::Lines { worktree, snapshot, results } => {
                git.pending_line_jobs -= 1;
                let current = git.snapshot(&worktree).is_some_and(|active| Arc::ptr_eq(active, &snapshot));
                for (relative, counts) in results {
                    let key = (worktree.clone(), relative);
                    git.pending_lines.remove(&key);
                    if current {
                        line_paths.extend(git.wanted_lines.remove(&key).into_iter().flatten());
                        git.lines.insert(key, counts);
                    }
                }
            }
            GitEvent::Project { scan, project } if Arc::ptr_eq(&scan, &git.project_cancel) => {
                git.project_rows.insert(project.path.clone(), project);
                projects_changed = true;
            }
            GitEvent::ProjectsDone { scan, truncated } if Arc::ptr_eq(&scan, &git.project_cancel) => {
                git.project_truncated = truncated;
                projects_changed = true;
            }
            _ => {}
        }
    }
    for identity in watches {
        ensure_watch(cx, identity);
    }
    request_repos(cx, now);
    schedule_lines(cx);
    for (worktree, refresh) in &repo_refreshes {
        let folders: HashSet<PathBuf> = cx
            .global::<Git>()
            .folders
            .iter()
            .filter(|(_, cache)| cache.repo.as_ref().is_some_and(|repo| &repo.worktree == worktree))
            .map(|(folder, _)| folder.clone())
            .collect();
        crate::plugins::refresh_git(cx, *refresh, |path| folders.contains(path));
    }
    if !line_paths.is_empty() {
        crate::plugins::invalidate_git_lines(cx, &line_paths);
    }
    if !repo_refreshes.is_empty() || projects_changed || !line_paths.is_empty() {
        cx.refresh_windows();
    }
}

fn schedule_lines(cx: &mut App) {
    let git = cx.global_mut::<Git>();
    let available = MAX_LINE_JOBS.saturating_sub(git.pending_line_jobs);
    if available == 0 {
        return;
    }
    let mut groups: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
    for (worktree, relative) in
        git.wanted_lines.keys().filter(|key| !git.pending_lines.contains(*key)).take(MAX_LINE_ROWS)
    {
        if groups.len() == available && !groups.contains_key(worktree) {
            continue;
        }
        groups.entry(worktree.clone()).or_default().push(relative.clone());
    }
    git.pending_line_jobs += groups.len();
    let sender = git.tx.clone();
    let cancel = git.cancel.clone();
    let gate = crate::cloud_status::read_gate(cx);
    for (worktree, rows) in groups {
        let gate = gate.clone();
        let git = cx.global_mut::<Git>();
        git.pending_lines.extend(rows.iter().map(|relative| (worktree.clone(), relative.clone())));
        let snapshot = git.snapshot(&worktree).expect("lines need a snapshot").clone();
        let sender = sender.clone();
        let cancel = cancel.clone();
        cx.background_spawn(async move {
            let repo = crow_plugin_git::open_worktree(&worktree).ok();
            let mut diff = repo.as_ref().and_then(|repo| crow_plugin_git::LineDiff::with_gate(repo, gate).ok());
            let results = rows
                .into_iter()
                .take_while(|_| !cancel.load(Ordering::Relaxed))
                .map(|relative| {
                    let counts = diff.as_mut().and_then(|diff| diff.counts(&snapshot, &relative).ok().flatten());
                    (relative, counts)
                })
                .collect();
            sender.send(GitEvent::Lines { worktree, snapshot, results }).ok();
        })
        .detach();
    }
}

/// Build the one host-data payload for a status or Info request.
pub fn host_data(cx: &mut App, folder: &Path, entries: Vec<EntryHostData>) -> Option<crow_plugin::HostData> {
    let repository = repository_data(cx, folder)?;
    Some(crow_plugin::HostData { repository: Some(repository), entries })
}

/// The native Projects section's rows for the configured roots. Starts one
/// bounded, cancellable discovery worker per roots generation and publishes
/// rows as each repository's status completes.
pub fn projects_rows(cx: &mut App, roots: Vec<PathBuf>, home: &Path) -> Vec<SectionRow> {
    let Some(git) = cx.try_global::<Git>() else { return Vec::new() };
    if !git.enabled {
        return Vec::new();
    }
    if roots != git.project_roots {
        start_projects(cx, roots);
    }
    let git = cx.global::<Git>();
    let mut rows: Vec<SectionRow> = git
        .project_rows
        .values()
        .map(|project| SectionRow {
            id: hex_path(&project.path),
            text: project.path.file_name().map_or_else(
                || crow_core::abbreviate_home(&project.path, home),
                |name| name.to_string_lossy().into_owned(),
            ),
            meta: Some(project.head.clone()),
            badge: project
                .changed
                .filter(|changed| *changed > 0)
                .map(|changed| changed.to_string())
                .or_else(|| project.unavailable.then(|| "—".to_string())),
            destination: SectionDestination::Folder { path: project.path.clone(), identity: project.identity },
        })
        .collect();
    rows.sort_by(|a, b| a.text.to_lowercase().cmp(&b.text.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
    if git.project_truncated {
        rows.push(SectionRow {
            id: "discovery-limit".into(),
            text: "Discovery stopped at the limit".into(),
            meta: Some("Refresh Projects".into()),
            badge: None,
            destination: SectionDestination::DisplayOnly,
        });
    }
    rows
}

/// Discard discovered rows and scan the configured roots again.
pub fn refresh_projects(cx: &mut App) {
    let Some(git) = cx.try_global::<Git>() else { return };
    let roots = git.project_roots.clone();
    start_projects(cx, roots);
}

fn start_projects(cx: &mut App, roots: Vec<PathBuf>) {
    let Some(git) = cx.try_global::<Git>().filter(|git| git.enabled) else { return };
    let sender = git.tx.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let git = cx.global_mut::<Git>();
    std::mem::replace(&mut git.project_cancel, cancel.clone()).store(true, Ordering::Relaxed);
    git.project_roots.clone_from(&roots);
    git.project_rows.clear();
    git.project_truncated = false;
    if roots.is_empty() {
        return;
    }
    let gate = crate::cloud_status::read_gate(cx);
    cx.background_spawn(async move {
        let report = crow_plugin_git::discover(
            &roots,
            crow_plugin_git::DiscoveryLimits { max_depth: 1, max_directories: 20_000, max_repositories: 200 },
            &cancel,
            |found| {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let Ok(metadata) = std::fs::metadata(&found.identity.worktree) else { return }; // io-ok: inside background_spawn
                use std::os::unix::fs::MetadataExt;
                let head = match &found.head.state {
                    crow_plugin_git::HeadState::Branch { name } => name.clone(),
                    crow_plugin_git::HeadState::Unborn { branch } => format!("{branch} (unborn)"),
                    crow_plugin_git::HeadState::Detached { short_id } => format!("detached {short_id}"),
                    crow_plugin_git::HeadState::Bare => return,
                };
                let unavailable = crow_plugin_git::is_cloud_backed_worktree(&found.identity.worktree);
                let changed = if unavailable {
                    None
                } else {
                    crow_plugin_git::open_worktree(&found.identity.worktree).ok().and_then(|repo| {
                        crow_plugin_git::status_snapshot_gated(&repo, &cancel, gate.clone())
                            .ok()
                            .map(|snapshot| snapshot.changed_count)
                    })
                };
                sender
                    .send(GitEvent::Project {
                        scan: cancel.clone(),
                        project: ProjectRow {
                            path: found.identity.worktree,
                            identity: (metadata.dev(), metadata.ino()),
                            head,
                            changed,
                            unavailable,
                        },
                    })
                    .ok();
            },
        );
        sender.send(GitEvent::ProjectsDone { scan: cancel, truncated: report.truncated }).ok();
    })
    .detach();
}

fn hex_path(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::with_capacity(path.as_os_str().as_bytes().len() * 2);
    for byte in path.as_os_str().as_bytes() {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// The plugin id the bundled package uses. Only this ID receives host data.
pub const PLUGIN_ID: &str = "git";
pub const PROJECTS_SECTION: &str = "crow.projects";

pub fn provider_enabled(cx: &App) -> bool {
    cx.try_global::<Git>().is_some_and(|git| git.enabled)
}

/// Discovered repository roots for the palette's Places list.
pub fn project_folders(cx: &App) -> Vec<PathBuf> {
    cx.try_global::<Git>()
        .filter(|git| git.enabled)
        .map(|git| git.project_rows.keys().cloned().collect())
        .unwrap_or_default()
}

pub fn is_lines_column(id: &crow_core::ExtensionId) -> bool {
    is_git_plugin(&id.plugin) && id.local.as_str() == "lines-changed"
}

pub fn is_git_plugin(plugin: &crow_core::PluginId) -> bool {
    plugin.as_str() == PLUGIN_ID
}

#[cfg(test)]
pub fn set_enabled_for_test(cx: &mut App, enabled: bool) {
    cx.set_global(Git::new(enabled));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_id_is_the_bundled_git_package() {
        assert!(is_git_plugin(&crow_core::PluginId::parse("git").unwrap()));
        assert!(!is_git_plugin(&crow_core::PluginId::parse("example.git").unwrap()));
    }

    #[gpui::test]
    fn a_repository_scans_in_the_background_and_feeds_status_and_rows(cx: &mut gpui::TestAppContext) {
        use std::process::Command;
        cx.update(crate::ui::init_for_tests);
        let temp = tempfile::TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        let sibling = repo.join("sub");
        std::fs::create_dir_all(&sibling).unwrap();
        let alias = repo.join("alias");
        std::os::unix::fs::symlink("sub", &alias).unwrap();
        let git = |args: &[&str]| {
            let out = Command::new("git").arg("-C").arg(&repo).args(args).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.name", "Crow Test"]);
        git(&["config", "user.email", "crow@example.invalid"]);
        std::fs::write(sibling.join("a.txt"), "a\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "base"]);
        std::fs::write(sibling.join("a.txt"), "b\n").unwrap();

        cx.update(|cx| set_enabled_for_test(cx, true));
        cx.update(|cx| request_folder(cx, &repo));
        let mut snapshot = None;
        for _ in 0..200 {
            cx.update(poll);
            cx.run_until_parked();
            if let Some(data) = cx.update(|cx| repository_data(cx, &repo)) {
                snapshot = Some(data);
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let data = snapshot.expect("repository snapshot");
        assert_eq!(data.branch.as_deref(), Some("main"));
        assert_eq!(data.changed, 1);
        cx.update(|cx| {
            let summary = pin_summary(cx, &repo, false).expect("pinned repository summary");
            assert_eq!(summary.branch, "main");
            assert_eq!(summary.badge, "1");
            let folder = entry_host(cx, &repo, &sibling, true, true).unwrap();
            assert_eq!(folder.badge, "M");
            assert_eq!(folder.label, "contains modified paths");
            assert_eq!((folder.index, folder.worktree, folder.lines_changed), (None, None, None));
            assert!(cx.global::<Git>().wanted_lines.is_empty());
        });
        let canonical = std::fs::canonicalize(&repo).unwrap();
        let original = cx.read_global::<Git, _>(|git, _| git.snapshot(&canonical).unwrap().clone());
        let alias = canonical.join("alias");
        // Resolve the second folder before checking the repository cache.
        cx.update(|cx| request_folder(cx, &alias));
        cx.run_until_parked();
        cx.update(|cx| {
            assert_eq!(repository_data(cx, &alias).unwrap().changed, 1);
            assert!(pin_summary(cx, &alias, false).is_none(), "pins inside a repository are ordinary folders");
            let git = cx.global::<Git>();
            assert!(Arc::ptr_eq(git.snapshot(&canonical).unwrap(), &original));
            assert!(git.pending_repos.is_empty(), "a sibling folder must reuse the existing scan");
        });
        let mut host = None;
        for _ in 0..200 {
            cx.update(poll);
            cx.run_until_parked();
            if let Some(row) = cx.update(|cx| entry_host(cx, &alias, &alias.join("a.txt"), false, true))
                && row.lines_changed.is_some()
            {
                host = Some(row);
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let host = host.expect("row host data with line counts");
        assert_eq!(host.badge, "M");
        assert_eq!(host.lines_changed, Some(2));
        let pin = cx.update(|cx| pin_summary(cx, &repo, true).unwrap());
        assert_eq!(pin.branch, "main · +1/−1");

        cx.run_until_parked();
        cx.update(poll);
        assert!(cx.read_global::<Git, _>(|git, _| git.watches.iter().any(|watch| watch.watcher.is_some())));
        git(&["add", "sub/a.txt"]);
        for _ in 0..200 {
            cx.update(poll);
            cx.run_until_parked();
            if let Some(row) = cx.update(|cx| entry_host(cx, &alias, &alias.join("a.txt"), false, false))
                && row.index.as_deref() == Some("modified")
            {
                assert_eq!(row.worktree, None);
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the repository watcher did not pick up the changed Git index");
    }

    #[gpui::test]
    fn stale_line_results_are_dropped(cx: &mut gpui::TestAppContext) {
        cx.update(crate::ui::init_for_tests);
        cx.update(|cx| set_enabled_for_test(cx, true));
        let worktree = PathBuf::from("/tmp/repo");
        let snapshot = Arc::new(StatusSnapshot {
            identity: RepositoryIdentity {
                worktree: worktree.clone(),
                git_dir: worktree.join(".git"),
                common_dir: worktree.join(".git"),
                linked_worktree: false,
            },
            head: crow_plugin_git::HeadInfo {
                state: crow_plugin_git::HeadState::Unborn { branch: "main".into() },
                upstream: None,
            },
            entries: Default::default(),
            affected_folders: Default::default(),
            changed_count: 0,
            has_conflicts: false,
            renames_by_source: Default::default(),
            last_commit: None,
            unavailable: false,
        });
        let current = Arc::new((*snapshot).clone());
        let tx = cx.update(|cx| {
            let git = cx.global_mut::<Git>();
            git.repos.insert(worktree.clone(), RepoCache { snapshot: Some(current.clone()), checked: Instant::now() });
            git.tx.clone()
        });
        let send = |cx: &mut gpui::TestAppContext, snapshot| {
            cx.update(|cx| cx.global_mut::<Git>().pending_line_jobs += 1);
            tx.send(GitEvent::Lines {
                worktree: worktree.clone(),
                snapshot,
                results: vec![(PathBuf::from("a.txt"), None)],
            })
            .unwrap();
        };
        send(cx, snapshot);
        cx.update(poll);
        assert!(cx.read_global::<Git, _>(|git, _| git.lines.is_empty()));
        send(cx, current);
        cx.update(poll);
        assert_eq!(cx.read_global::<Git, _>(|git, _| git.lines.len()), 1);
    }

    #[gpui::test]
    fn replaced_projects_scans_cannot_publish(cx: &mut gpui::TestAppContext) {
        cx.update(crate::ui::init_for_tests);
        cx.update(|cx| set_enabled_for_test(cx, true));
        cx.update(|cx| {
            let git = cx.global::<Git>();
            let stale = git.project_cancel.clone();
            let tx = git.tx.clone();
            start_projects(cx, Vec::new());
            tx.send(GitEvent::Project {
                scan: stale.clone(),
                project: ProjectRow {
                    path: PathBuf::from("/tmp/old"),
                    identity: (1, 2),
                    head: "main".into(),
                    changed: Some(0),
                    unavailable: false,
                },
            })
            .unwrap();
            tx.send(GitEvent::ProjectsDone { scan: stale, truncated: true }).unwrap();
        });
        cx.update(poll);
        assert!(cx.update(|cx| projects_rows(cx, Vec::new(), Path::new("/tmp"))).is_empty());
    }

    #[gpui::test]
    fn disabling_the_provider_clears_every_cached_projection(cx: &mut gpui::TestAppContext) {
        cx.update(crate::ui::init_for_tests);
        cx.update(|cx| set_enabled_for_test(cx, true));
        cx.update(|cx| {
            let git = cx.global_mut::<Git>();
            git.folders.insert(
                PathBuf::from("/tmp/repo"),
                FolderCache {
                    repo: Some(FolderRepo { worktree: PathBuf::from("/tmp/repo"), alias_prefix: None }),
                    checked: Instant::now(),
                },
            );
            git.project_rows.insert(
                PathBuf::from("/tmp/repo"),
                ProjectRow {
                    path: PathBuf::from("/tmp/repo"),
                    identity: (1, 2),
                    head: "main".into(),
                    changed: Some(1),
                    unavailable: false,
                },
            );
        });
        cx.update(sync_enabled);
        assert!(!cx.read_global::<Git, _>(|git, _| git.enabled));
        assert!(cx.read_global::<Git, _>(|git, _| git.folders.is_empty()));
        assert!(cx.read_global::<Git, _>(|git, _| git.project_rows.is_empty()));
        assert!(!cx.update(|cx| provider_enabled(cx)));
    }

    #[gpui::test]
    fn projects_discovery_publishes_rows_with_branch_and_changed_count(cx: &mut gpui::TestAppContext) {
        use std::process::Command;
        cx.update(crate::ui::init_for_tests);
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join("code");
        let repo = root.join("crow");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            let out = Command::new("git").arg("-C").arg(&repo).args(args).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.name", "Crow Test"]);
        git(&["config", "user.email", "crow@example.invalid"]);
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "base"]);
        std::fs::write(repo.join("a.txt"), "b\n").unwrap();

        cx.update(|cx| set_enabled_for_test(cx, true));
        let home = std::fs::canonicalize(temp.path()).unwrap();
        let root = std::fs::canonicalize(&root).unwrap();
        let mut rows = Vec::new();
        for _ in 0..300 {
            rows = cx.update(|cx| projects_rows(cx, vec![root.clone()], &home));
            cx.update(poll);
            cx.run_until_parked();
            if !rows.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(rows.len(), 1, "one discovered repository: {rows:?}");
        assert_eq!(rows[0].text, "crow", "a row shows its folder name; the root is in the heading");
        assert_eq!(rows[0].meta.as_deref(), Some("main"));
        assert_eq!(rows[0].badge.as_deref(), Some("1"));
        assert!(rows[0].folder_path().is_some_and(|path| path.ends_with("crow")));
        assert!(rows[0].folder_identity().is_some());
    }
}
