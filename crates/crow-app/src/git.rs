//! Read-only Git snapshots for the bundled `git` plugin and Projects.
//!
//! Repository discovery and status run in GPUI's background executor, bounded
//! to one scan per pending folder/repository. Snapshots are shared by every
//! window; rows, status and Info read the cache and never touch disk. One
//! recursive watcher per active repository coalesces FSEvents refreshes.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crow_core::{SectionDestination, SectionRow};
use crow_plugin::{EntryHostData, RepositoryData};
use crow_plugin_git::{LineCounts, RepositoryIdentity, StatusKind, StatusSnapshot};
use gpui::{App, AppContext as _, Global};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

/// How many unknown folder scopes may be scanning at once.
const MAX_PENDING_FOLDERS: usize = 8;
/// How many line jobs one poll may start, and rows one job may contain.
const MAX_LINE_JOBS: usize = 4;
const MAX_LINE_ROWS: usize = 64;
/// A repository watcher burst is merged for this long before rescanning.
const WATCH_DEBOUNCE: Duration = Duration::from_millis(250);

#[derive(Clone, Debug)]
struct FolderCache {
    repo: Option<FolderRepo>,
    checked: Instant,
}

#[derive(Clone, Debug)]
struct FolderRepo {
    worktree: PathBuf,
    /// The requested folder's path relative to the worktree, for mapping row
    /// paths even when the request used a symlinked alias (e.g. /var vs /private/var).
    relative_prefix: PathBuf,
}

/// A known repository is rechecked on focus/render after this long; its watcher
/// is the primary invalidation path. A folder with no repository is rechecked
/// sooner, so a newly initialized repository appears without a restart.
const REPO_TTL: Duration = Duration::from_secs(30);
const NO_REPO_TTL: Duration = Duration::from_secs(2);

#[derive(Clone, Debug)]
struct CachedLines {
    version: u64,
    counts: Option<LineCounts>,
}

#[derive(Clone, Debug)]
struct LineWant {
    version: u64,
    renamed_from: Option<PathBuf>,
    /// The row's absolute path as the view knows it (which may use a symlinked alias).
    row_path: PathBuf,
}

enum GitEvent {
    Folder { generation: u64, folder: PathBuf, snapshot: Option<Arc<StatusSnapshot>>, relative_prefix: PathBuf },
    Repo { generation: u64, worktree: PathBuf, snapshot: Option<Arc<StatusSnapshot>> },
    Lines { generation: u64, worktree: PathBuf, results: Vec<(PathBuf, PathBuf, u64, Option<LineCounts>)> },
    Project { generation: u64, project: ProjectRow },
    ProjectsDone { generation: u64, truncated: bool },
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
    generation: u64,
    folders: HashMap<PathBuf, FolderCache>,
    snapshots: HashMap<PathBuf, Arc<StatusSnapshot>>,
    pending_folders: HashSet<PathBuf>,
    pending_repos: HashSet<PathBuf>,
    dirty: HashSet<PathBuf>,
    next_scan: HashMap<PathBuf, Instant>,
    lines: HashMap<(PathBuf, PathBuf), CachedLines>,
    wanted_lines: HashMap<(PathBuf, PathBuf), LineWant>,
    pending_lines: HashSet<(PathBuf, PathBuf)>,
    watchers: HashMap<PathBuf, RecommendedWatcher>,
    active: Option<PathBuf>,
    watch_quiet_until: HashMap<PathBuf, Instant>,
    project_roots: Vec<PathBuf>,
    project_rows: HashMap<PathBuf, ProjectRow>,
    project_pending: bool,
    project_truncated: bool,
    project_cancel: std::sync::Arc<AtomicBool>,
    projects_checked: Option<Instant>,
    watch_tx: Sender<PathBuf>,
    watch_rx: Receiver<PathBuf>,
    tx: Sender<GitEvent>,
    rx: Receiver<GitEvent>,
}

impl Global for Git {}

pub fn init_at(cx: &mut App) {
    let (tx, rx) = channel();
    let (watch_tx, watch_rx) = channel();
    cx.set_global(Git {
        enabled: false,
        generation: 0,
        folders: HashMap::new(),
        snapshots: HashMap::new(),
        pending_folders: HashSet::new(),
        pending_repos: HashSet::new(),
        dirty: HashSet::new(),
        next_scan: HashMap::new(),
        lines: HashMap::new(),
        wanted_lines: HashMap::new(),
        pending_lines: HashSet::new(),
        watchers: HashMap::new(),
        active: None,
        watch_quiet_until: HashMap::new(),
        project_roots: Vec::new(),
        project_rows: HashMap::new(),
        project_pending: false,
        project_truncated: false,
        project_cancel: std::sync::Arc::new(AtomicBool::new(false)),
        projects_checked: None,
        watch_tx,
        watch_rx,
        tx,
        rx,
    });
}

#[cfg(test)]
pub fn init_for_tests(cx: &mut App) {
    init_at(cx);
}

fn scanning_allowed(cx: &App) -> bool {
    cx.try_global::<Git>().is_some_and(|git| git.enabled)
}

/// Follow the plugin inventory: no provider work while the bundled package is
/// absent, pending approval, disabled or failed.
pub fn sync_enabled(cx: &mut App) {
    let running = crate::plugins::bundled_git_running(cx);
    let Some(git) = cx.try_global::<Git>() else { return };
    if git.enabled == running {
        return;
    }
    let git = cx.global_mut::<Git>();
    git.enabled = running;
    git.generation = git.generation.wrapping_add(1);
    if !running {
        git.folders.clear();
        git.snapshots.clear();
        git.pending_folders.clear();
        git.pending_repos.clear();
        git.dirty.clear();
        git.next_scan.clear();
        git.lines.clear();
        git.wanted_lines.clear();
        git.pending_lines.clear();
        git.watchers.clear();
        git.active = None;
        git.watch_quiet_until.clear();
        git.project_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        git.project_roots.clear();
        git.project_rows.clear();
        git.project_pending = false;
        git.project_truncated = false;
        git.projects_checked = None;
    }
    crate::plugins::invalidate_git(cx);
    cx.refresh_windows();
}

/// The folder's repository snapshot, if one has been scanned. Missing scopes
/// start one bounded background scan.
pub fn repository_data(cx: &mut App, folder: &Path) -> Option<RepositoryData> {
    request_folder(cx, folder);
    let git = cx.try_global::<Git>()?;
    let worktree = git.folders.get(folder)?.repo.as_ref()?.worktree.clone();
    let snapshot = git.snapshots.get(&worktree)?;
    Some(repository_data_from(snapshot))
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
pub fn entry_host(cx: &mut App, folder: &Path, path: &Path, version: u64, want_lines: bool) -> Option<EntryHostData> {
    request_folder(cx, folder);
    let git = cx.global_mut::<Git>();
    let repo = git.folders.get(folder)?.repo.as_ref()?;
    let worktree = repo.worktree.clone();
    let snapshot = git.snapshots.get(&worktree)?.clone();
    let relative = match path.strip_prefix(&worktree) {
        Ok(relative) => relative.to_path_buf(),
        Err(_) => repo.relative_prefix.join(path.strip_prefix(folder).ok()?),
    };
    let status = match snapshot.lookup(&relative) {
        Some(status) => status.clone(),
        None => {
            // A file inside an ignored directory has no own status entry; the
            // directory's rule still applies to the row.
            let mut parent = relative.parent();
            let mut ignored = false;
            while let Some(path) = parent {
                if snapshot.lookup(path).is_some_and(|status| status.ignored) {
                    ignored = true;
                    break;
                }
                parent = path.parent();
            }
            if ignored {
                crow_plugin_git::EntryStatus { ignored: true, ..Default::default() }
            } else {
                return None;
            }
        }
    };
    if status.is_clean() {
        return None;
    }
    let badge = status.badge().map(|kind| kind.badge().to_owned()).unwrap_or_default();
    let label = status.badge().map(|kind| kind.label().to_owned()).unwrap_or_default();
    let index = status.index.map(|kind| kind.label().to_owned());
    let worktree_status = status.worktree.map(|kind| kind.label().to_owned());
    let eligible = status.is_changed()
        && !status.ignored
        && !status.is_unmerged()
        && !matches!(status.index, Some(StatusKind::Deleted | StatusKind::Untracked | StatusKind::IntentToAdd))
        && !matches!(status.worktree, Some(StatusKind::Deleted | StatusKind::Untracked));
    let lines = if eligible && want_lines {
        let key = (worktree.clone(), relative.clone());
        match git.lines.get(&key) {
            Some(cached) if cached.version == version => cached.counts,
            _ => {
                git.wanted_lines.insert(
                    key,
                    LineWant { version, renamed_from: status.renamed_from.clone(), row_path: path.to_path_buf() },
                );
                None
            }
        }
    } else {
        None
    };
    Some(EntryHostData {
        handle: crow_core::EntryHandle(1),
        badge,
        label,
        index,
        worktree: worktree_status,
        ignored: status.ignored,
        lines_changed: lines.map(|counts| counts.insertions.saturating_add(counts.removals)),
    })
}

pub fn request_folder(cx: &mut App, folder: &Path) {
    if !scanning_allowed(cx) {
        return;
    }
    let folder = folder.to_path_buf();
    let Some(git) = cx.try_global::<Git>() else { return };
    let fresh = git.folders.get(&folder).is_some_and(|cached| {
        let ttl = if cached.repo.is_some() { REPO_TTL } else { NO_REPO_TTL };
        cached.checked.elapsed() < ttl
    });
    if fresh || git.pending_folders.contains(&folder) {
        return;
    }
    if git.pending_folders.len() >= MAX_PENDING_FOLDERS {
        return;
    }
    let sender = git.tx.clone();
    let generation = git.generation;
    {
        let git = cx.global_mut::<Git>();
        git.pending_folders.insert(folder.clone());
    }
    cx.background_spawn(async move {
        let (snapshot, relative_prefix) = scan_folder(&folder);
        sender.send(GitEvent::Folder { generation, folder, snapshot, relative_prefix }).ok();
    })
    .detach();
}

fn scan_folder(folder: &Path) -> (Option<Arc<StatusSnapshot>>, PathBuf) {
    let started = Instant::now();
    let Some(worktree) = crow_plugin_git::find_enclosing_worktree(folder, Path::new("/")) else {
        return (None, PathBuf::new());
    };
    let Ok(repo) = crow_plugin_git::open_worktree(&worktree) else { return (None, PathBuf::new()) };
    let snapshot = match if crow_plugin_git::is_cloud_backed_worktree(&worktree) {
        crow_plugin_git::unavailable_snapshot(&repo, 0)
    } else {
        crow_plugin_git::status_snapshot(&repo, 0, &AtomicBool::new(false))
    } {
        Ok(snapshot) => snapshot,
        Err(_) => return (None, PathBuf::new()),
    };
    tracing::info!(
        target: "crow::git",
        path = %snapshot.identity.worktree.display(),
        entries = snapshot.entries.len(),
        changed = snapshot.changed_count,
        elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
        "git status"
    );
    let relative_prefix = std::fs::canonicalize(folder) // io-ok: inside background_spawn
        .ok()
        .and_then(|folder| folder.strip_prefix(&snapshot.identity.worktree).ok().map(Path::to_path_buf))
        .unwrap_or_default();
    (Some(Arc::new(snapshot)), relative_prefix)
}

fn request_repo(cx: &mut App, worktree: PathBuf) {
    if !scanning_allowed(cx) {
        return;
    }
    let Some(git) = cx.try_global::<Git>() else { return };
    if git.pending_repos.contains(&worktree) || !git.dirty.contains(&worktree) {
        return;
    }
    let now = Instant::now();
    if git.next_scan.get(&worktree).is_some_and(|next| *next > now) {
        return;
    }
    let sender = git.tx.clone();
    let generation = git.generation;
    {
        let git = cx.global_mut::<Git>();
        git.pending_repos.insert(worktree.clone());
        git.next_scan.insert(worktree.clone(), now + WATCH_DEBOUNCE);
    }
    cx.background_spawn(async move {
        let started = Instant::now();
        let snapshot = crow_plugin_git::open_worktree(&worktree).ok().and_then(|repo| {
            let snapshot = if crow_plugin_git::is_cloud_backed_worktree(&worktree) {
                crow_plugin_git::unavailable_snapshot(&repo, 0)
            } else {
                crow_plugin_git::status_snapshot(&repo, 0, &AtomicBool::new(false))
            };
            snapshot.ok().map(Arc::new)
        });
        if let Some(snapshot) = &snapshot {
            tracing::info!(
                target: "crow::git",
                path = %snapshot.identity.worktree.display(),
                entries = snapshot.entries.len(),
                changed = snapshot.changed_count,
                elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
                "git status"
            );
        }
        sender.send(GitEvent::Repo { generation, worktree, snapshot }).ok();
    })
    .detach();
}

fn ensure_watch(git: &mut Git, identity: &RepositoryIdentity) {
    if git.watchers.contains_key(&identity.worktree) {
        git.active = Some(identity.worktree.clone());
        return;
    }
    git.watchers.clear();
    let worktree = identity.worktree.clone();
    let tx = git.watch_tx.clone();
    let mut watcher = match notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        if result.is_ok() {
            tx.send(worktree.clone()).ok();
        }
    }) {
        Ok(watcher) => watcher,
        Err(error) => {
            tracing::warn!("git watch: {error}");
            return;
        }
    };
    for path in [&identity.worktree, &identity.git_dir, &identity.common_dir] {
        if let Err(error) = watcher.watch(path, RecursiveMode::Recursive) {
            tracing::warn!("git watch {}: {error}", path.display());
        }
    }
    let _ = watcher.watch(&identity.common_dir.join("refs"), RecursiveMode::Recursive);
    git.watchers.insert(identity.worktree.clone(), watcher);
    git.watch_quiet_until.insert(identity.worktree.clone(), Instant::now() + WATCH_DEBOUNCE * 2);
    git.active = Some(identity.worktree.clone());
}

/// Drain background results and watcher events. Called by the 100 ms app poll.
pub fn poll(cx: &mut App) {
    if cx.try_global::<Git>().is_none_or(|git| !git.enabled) {
        return;
    }
    let watched: Vec<PathBuf> = {
        let git = cx.global::<Git>();
        git.watch_rx.try_iter().collect()
    };
    if !watched.is_empty() {
        let git = cx.global_mut::<Git>();
        let now = Instant::now();
        for worktree in watched {
            if git.watch_quiet_until.get(&worktree).is_none_or(|until| now >= *until) {
                git.dirty.insert(worktree);
            }
        }
    }
    let now = Instant::now();
    let dirty: Vec<PathBuf> = {
        let git = cx.global::<Git>();
        git.dirty
            .iter()
            .filter(|worktree| git.next_scan.get(*worktree).is_none_or(|next| *next <= now))
            .cloned()
            .collect()
    };
    for worktree in dirty {
        request_repo(cx, worktree);
    }
    schedule_lines(cx);

    let events: Vec<GitEvent> = {
        let git = cx.global_mut::<Git>();
        git.rx.try_iter().collect()
    };
    let mut invalidated = false;
    let mut line_paths = Vec::new();
    for event in events {
        let generation = cx.global::<Git>().generation;
        match event {
            GitEvent::Folder { generation: event_generation, folder, snapshot, relative_prefix }
                if event_generation == generation =>
            {
                let git = cx.global_mut::<Git>();
                git.pending_folders.remove(&folder);
                match snapshot {
                    Some(snapshot) => {
                        let worktree = snapshot.identity.worktree.clone();
                        ensure_watch(git, &snapshot.identity);
                        git.folders.insert(
                            folder,
                            FolderCache {
                                repo: Some(FolderRepo { worktree: worktree.clone(), relative_prefix }),
                                checked: Instant::now(),
                            },
                        );
                        git.snapshots.insert(worktree.clone(), snapshot);
                        git.dirty.remove(&worktree);
                        invalidated = true;
                    }
                    None => {
                        git.folders.insert(folder, FolderCache { repo: None, checked: Instant::now() });
                    }
                }
            }
            GitEvent::Repo { generation: event_generation, worktree, snapshot } if event_generation == generation => {
                let git = cx.global_mut::<Git>();
                git.pending_repos.remove(&worktree);
                git.dirty.remove(&worktree);
                match snapshot {
                    Some(snapshot) => {
                        ensure_watch(git, &snapshot.identity);
                        git.snapshots.insert(worktree.clone(), snapshot);
                        invalidated = true;
                    }
                    None => {
                        git.snapshots.remove(&worktree);
                        invalidated = true;
                    }
                }
            }
            GitEvent::Lines { generation: event_generation, worktree, results } if event_generation == generation => {
                let git = cx.global_mut::<Git>();
                for (row_path, relative, version, counts) in results {
                    git.pending_lines.remove(&(worktree.clone(), relative.clone()));
                    git.lines.insert((worktree.clone(), relative.clone()), CachedLines { version, counts });
                    line_paths.push(row_path);
                }
            }
            GitEvent::Project { generation: event_generation, project } if event_generation == generation => {
                cx.global_mut::<Git>().project_rows.insert(project.path.clone(), project);
                invalidated = true;
            }
            GitEvent::ProjectsDone { generation: event_generation, truncated } if event_generation == generation => {
                let git = cx.global_mut::<Git>();
                git.project_pending = false;
                git.project_truncated = truncated;
                git.projects_checked = Some(Instant::now());
                invalidated = true;
            }
            _ => {}
        }
    }
    if invalidated {
        crate::plugins::invalidate_git(cx);
    }
    let line_changed = !line_paths.is_empty();
    for path in line_paths {
        crate::plugins::invalidate_git_path(cx, &path);
    }
    if invalidated || line_changed {
        cx.refresh_windows();
    }
}

fn schedule_lines(cx: &mut App) {
    let keys: Vec<(PathBuf, PathBuf)> = {
        let git = cx.global::<Git>();
        git.wanted_lines.keys().filter(|key| !git.pending_lines.contains(*key)).cloned().collect()
    };
    type LineJob = (PathBuf, PathBuf, u64, Option<PathBuf>);
    let mut groups: HashMap<PathBuf, Vec<LineJob>> = HashMap::new();
    let mut rows = 0usize;
    {
        let git = cx.global_mut::<Git>();
        for key in keys {
            if rows >= MAX_LINE_ROWS || groups.len() >= MAX_LINE_JOBS {
                break;
            }
            if git.pending_lines.contains(&key) {
                continue;
            }
            let Some(want) = git.wanted_lines.remove(&key) else { continue };
            git.pending_lines.insert(key.clone());
            groups.entry(key.0).or_default().push((want.row_path, key.1, want.version, want.renamed_from));
            rows += 1;
        }
    }
    if groups.is_empty() {
        return;
    }
    let sender = cx.global::<Git>().tx.clone();
    let generation = cx.global::<Git>().generation;
    for (worktree, rows) in groups {
        let sender = sender.clone();
        cx.background_spawn(async move {
            let repo = crow_plugin_git::open_worktree(&worktree).ok();
            let mut results = Vec::with_capacity(rows.len());
            for (row_path, relative, version, renamed_from) in rows {
                let counts = repo.as_ref().and_then(|repo| {
                    crow_plugin_git::line_counts(repo, &relative, renamed_from.as_deref()).ok().flatten()
                });
                results.push((row_path, relative, version, counts));
            }
            sender.send(GitEvent::Lines { generation, worktree, results }).ok();
        })
        .detach();
    }
}

/// Build the one host-data payload for a status or Info request.
pub fn host_data(cx: &mut App, folder: &Path, entries: Vec<EntryHostData>) -> Option<crow_plugin::HostData> {
    let repository = repository_data(cx, folder)?;
    Some(crow_plugin::HostData { repository: Some(repository), entries })
}

/// A single entry for Info, including status and line counts.
pub fn info_host(cx: &mut App, folder: &Path, path: &Path) -> Option<EntryHostData> {
    let mut entry = entry_host(cx, folder, path, 0, false)?;
    entry.handle = crow_core::EntryHandle(1);
    Some(entry)
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
            text: crow_core::abbreviate_home(&project.path, home),
            meta: Some(project.head.clone()),
            badge: project
                .changed
                .filter(|changed| *changed > 0)
                .map(|changed| changed.to_string())
                .or_else(|| project.unavailable.then(|| "—".to_string())),
            destination: SectionDestination::Folder { path: project.path.clone(), identity: project.identity },
        })
        .collect();
    rows.sort_by(|a, b| a.text.cmp(&b.text));
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
    let Some(git) = cx.try_global::<Git>() else { return };
    if roots.is_empty() {
        let git = cx.global_mut::<Git>();
        git.project_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        git.project_roots.clear();
        git.project_rows.clear();
        git.project_pending = false;
        git.project_truncated = false;
        git.projects_checked = Some(Instant::now());
        return;
    }
    let generation = git.generation;
    let sender = git.tx.clone();
    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    let old = std::mem::replace(&mut cx.global_mut::<Git>().project_cancel, cancel.clone());
    old.store(true, std::sync::atomic::Ordering::Relaxed);
    {
        let git = cx.global_mut::<Git>();
        git.project_roots = roots.clone();
        git.project_rows.clear();
        git.project_pending = true;
        git.project_truncated = false;
        git.projects_checked = None;
    }
    cx.background_spawn(async move {
        let report = crow_plugin_git::discover(
            &roots,
            crow_plugin_git::DiscoveryLimits { max_depth: 6, max_directories: 20_000, max_repositories: 200 },
            &cancel,
            |found| {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                let Ok(metadata) = std::fs::metadata(&found.identity.worktree) else { return }; // io-ok: inside background_spawn // io-ok: inside background_spawn
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
                        crow_plugin_git::status_snapshot(&repo, 0, &AtomicBool::new(false))
                            .ok()
                            .map(|snapshot| snapshot.changed_count)
                    })
                };
                sender
                    .send(GitEvent::Project {
                        generation,
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
        sender.send(GitEvent::ProjectsDone { generation, truncated: report.truncated }).ok();
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

pub fn is_git_plugin(plugin: &crow_core::PluginId) -> bool {
    plugin.as_str() == PLUGIN_ID
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_id_is_the_bundled_git_package() {
        assert!(is_git_plugin(&crow_core::PluginId::parse("git").unwrap()));
        assert!(!is_git_plugin(&crow_core::PluginId::parse("example.git").unwrap()));
    }

    #[cfg(test)]
    pub fn set_enabled_for_test(cx: &mut App, enabled: bool) {
        let git = cx.global_mut::<Git>();
        git.enabled = enabled;
        git.generation = git.generation.wrapping_add(1);
    }

    #[gpui::test]
    fn a_repository_scans_in_the_background_and_feeds_status_and_rows(cx: &mut gpui::TestAppContext) {
        use std::process::Command;
        cx.update(crate::ui::init_for_tests);
        let temp = tempfile::TempDir::new().unwrap();
        let repo = temp.path().join("repo");
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
        let mut host = None;
        for _ in 0..200 {
            cx.update(poll);
            cx.run_until_parked();
            if let Some(row) = cx.update(|cx| entry_host(cx, &repo, &repo.join("a.txt"), 1, true))
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
    }

    #[gpui::test]
    fn stale_generation_events_are_dropped(cx: &mut gpui::TestAppContext) {
        cx.update(crate::ui::init_for_tests);
        cx.update(|cx| set_enabled_for_test(cx, true));
        let (tx, generation) = cx.update(|cx| {
            let git = cx.global::<Git>();
            (git.tx.clone(), git.generation)
        });
        tx.send(GitEvent::Lines {
            generation: generation.wrapping_sub(1),
            worktree: PathBuf::from("/tmp/repo"),
            results: vec![(
                PathBuf::from("/tmp/repo/a.txt"),
                PathBuf::from("a.txt"),
                1,
                Some(LineCounts { insertions: 1, removals: 1 }),
            )],
        })
        .unwrap();
        cx.update(poll);
        assert!(cx.read_global::<Git, _>(|git, _| git.lines.is_empty()), "a pre-disable line result is stale");
        tx.send(GitEvent::Lines {
            generation,
            worktree: PathBuf::from("/tmp/repo"),
            results: vec![(
                PathBuf::from("/tmp/repo/a.txt"),
                PathBuf::from("a.txt"),
                1,
                Some(LineCounts { insertions: 1, removals: 1 }),
            )],
        })
        .unwrap();
        cx.update(poll);
        assert_eq!(cx.read_global::<Git, _>(|git, _| git.lines.len()), 1);
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
                    repo: Some(FolderRepo { worktree: PathBuf::from("/tmp/repo"), relative_prefix: PathBuf::new() }),
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
        assert!(rows[0].text.ends_with("code/crow"), "the path is abbreviated against HOME: {}", rows[0].text);
        assert_eq!(rows[0].meta.as_deref(), Some("main"));
        assert_eq!(rows[0].badge.as_deref(), Some("1"));
        assert!(rows[0].folder_path().is_some_and(|path| path.ends_with("crow")));
        assert!(rows[0].folder_identity().is_some());
    }
}
