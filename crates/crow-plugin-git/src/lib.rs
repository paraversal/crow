//! Read-only Git repository access for Crow's bundled `git` plugin.
//!
//! Status is computed with gix but without any external filter drivers: the
//! repository's `.gitattributes` may name a clean/process filter, and gix would
//! otherwise run it while converting worktree bytes. Line-ending and encoding
//! attributes still apply. Nothing in this crate writes to a repository, opens
//! the network, or launches a hook.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use gix::bstr::BStr;
use gix::status::plumbing::index_as_worktree::traits::SubmoduleStatus;
use gix::status::plumbing::index_as_worktree_with_renames::{
    Context, DirwalkContext, Entry, Options, Recorder, Summary,
};

/// The largest blob side this provider will line-diff. Larger files are unknown.
pub const MAX_LINE_DIFF_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, Default)]
struct NoSubmodules;

impl SubmoduleStatus for NoSubmodules {
    type Output = ();

    fn status(&mut self, _: &gix::index::Entry, _: &BStr) -> gix::Result<Option<()>> {
        Ok(None)
    }
}

/// The real worktree/git-dir/common-dir identity of one repository instance.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepositoryIdentity {
    /// Absolute, symlink-resolved worktree root, with raw OS bytes.
    pub worktree: PathBuf,
    /// This worktree's git directory (a linked worktree has its own).
    pub git_dir: PathBuf,
    /// The shared git directory (refs, objects, config).
    pub common_dir: PathBuf,
    pub linked_worktree: bool,
}

impl RepositoryIdentity {
    pub fn from_repo(repo: &gix::Repository) -> gix::Result<Self> {
        let worktree =
            repo.workdir().ok_or_else(|| gix::Error::from_error(gix::error::message("repository has no worktree")))?;
        let worktree = std::fs::canonicalize(worktree).map_err(gix::Error::from_error)?;
        let current_dir = repo.current_dir().to_owned();
        let git_dir = gix::path::realpath_opts(repo.git_dir(), &current_dir, gix::path::realpath::MAX_SYMLINKS)?;
        let common_dir = gix::path::realpath_opts(repo.common_dir(), &current_dir, gix::path::realpath::MAX_SYMLINKS)?;
        let linked_worktree = git_dir != common_dir && git_dir.components().any(|part| part.as_os_str() == "worktrees");
        Ok(Self { worktree, git_dir, common_dir, linked_worktree })
    }

    /// Raw repository-relative bytes for `path`, or `None` when it belongs elsewhere.
    pub fn relative<'a>(&self, path: &'a Path) -> Option<&'a Path> {
        path.strip_prefix(&self.worktree).ok()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeadState {
    Branch { name: String },
    Detached { short_id: String },
    Unborn { branch: String },
    Bare,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AheadBehind {
    pub ahead: u32,
    pub behind: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadInfo {
    pub state: HeadState,
    /// Only present when a local upstream ref and a common commit exist.
    pub upstream: Option<AheadBehind>,
}

impl HeadInfo {
    pub fn branch_name(&self) -> Option<&str> {
        match &self.state {
            HeadState::Branch { name } | HeadState::Unborn { branch: name } => Some(name),
            HeadState::Detached { .. } | HeadState::Bare => None,
        }
    }

    pub fn is_unborn(&self) -> bool {
        matches!(self.state, HeadState::Unborn { .. })
    }

    pub fn is_detached(&self) -> bool {
        matches!(self.state, HeadState::Detached { .. })
    }
}

pub fn head_info(repo: &gix::Repository) -> gix::Result<HeadInfo> {
    if repo.is_bare() {
        return Ok(HeadInfo { state: HeadState::Bare, upstream: None });
    }
    let head = repo.head()?;
    if head.is_unborn() {
        let branch = head.referent_name().map(|name| name.shorten().to_string()).unwrap_or_else(|| "HEAD".into());
        return Ok(HeadInfo { state: HeadState::Unborn { branch }, upstream: None });
    }
    if head.is_detached() {
        let short_id = head.id().map(|id| id.shorten_or_id().to_string()).unwrap_or_default();
        return Ok(HeadInfo { state: HeadState::Detached { short_id }, upstream: None });
    }
    let Some(name) = head.referent_name() else {
        return Ok(HeadInfo { state: HeadState::Detached { short_id: String::new() }, upstream: None });
    };
    let branch = name.shorten().to_string();
    let upstream = calculate_ahead_behind(repo, name, head.id().map(|id| id.detach()))?;
    Ok(HeadInfo { state: HeadState::Branch { name: branch }, upstream })
}

fn calculate_ahead_behind(
    repo: &gix::Repository,
    branch: &gix::refs::FullNameRef,
    head: Option<gix::hash::ObjectId>,
) -> gix::Result<Option<AheadBehind>> {
    let Some(head) = head else { return Ok(None) };
    let Some(upstream) = repo.branch_remote_tracking_ref_name(branch, gix::remote::Direction::Fetch) else {
        return Ok(None);
    };
    let upstream = upstream?;
    let Some(upstream_id) = repo.find_reference(&upstream).ok().map(|reference| reference.id().detach()) else {
        return Ok(None);
    };
    let Some(base) = repo.merge_base(head, upstream_id)?.map(|id| id.detach()) else { return Ok(None) };
    let ahead = count_commits(repo, head, base)?;
    let behind = count_commits(repo, upstream_id, base)?;
    Ok(Some(AheadBehind { ahead, behind }))
}

fn count_commits(repo: &gix::Repository, tip: gix::hash::ObjectId, base: gix::hash::ObjectId) -> gix::Result<u32> {
    let mut count = 0u32;
    for item in repo.rev_walk([tip]).with_hidden([base]).all()? {
        if item.is_ok() {
            count = count.saturating_add(1);
        }
    }
    Ok(count)
}

/// Variants are ordered by display precedence, strongest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum StatusKind {
    Unmerged,
    Deleted,
    Renamed,
    Added,
    IntentToAdd,
    TypeChange,
    Modified,
    Untracked,
    Ignored,
}

impl StatusKind {
    pub fn badge(self) -> &'static str {
        match self {
            Self::Unmerged => "U",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Added => "A",
            Self::Modified => "M",
            Self::TypeChange => "T",
            Self::Untracked => "?",
            Self::IntentToAdd => "A",
            Self::Ignored => "",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Unmerged => "conflicted",
            Self::Deleted => "deleted",
            Self::Renamed => "renamed",
            Self::Added | Self::IntentToAdd => "added",
            Self::Modified => "modified",
            Self::TypeChange => "type changed",
            Self::Untracked => "untracked",
            Self::Ignored => "ignored",
        }
    }
}

/// The combined index/worktree state of one tracked or visible path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EntryStatus {
    pub index: Option<StatusKind>,
    pub worktree: Option<StatusKind>,
    pub ignored: bool,
    /// For a staged rename, the raw path the entry came from.
    pub renamed_from: Option<PathBuf>,
}

impl EntryStatus {
    pub fn is_changed(&self) -> bool {
        !self.ignored && (self.index.is_some() || self.worktree.is_some())
    }

    pub fn is_unmerged(&self) -> bool {
        self.index == Some(StatusKind::Unmerged) || self.worktree == Some(StatusKind::Unmerged)
    }

    /// The one badge a row shows, using the plan's precedence:
    /// conflict > deletion/rename > add > modify > untracked > ignored.
    pub fn badge(&self) -> Option<StatusKind> {
        [self.index, self.worktree]
            .into_iter()
            .flatten()
            .min()
            .map(|kind| if kind == StatusKind::IntentToAdd { StatusKind::Added } else { kind })
            .or_else(|| self.ignored.then_some(StatusKind::Ignored))
    }

    pub fn can_count_lines(&self) -> bool {
        self.is_changed()
            && !self.is_unmerged()
            && !matches!(self.index, Some(StatusKind::Deleted | StatusKind::Untracked | StatusKind::IntentToAdd))
            && !matches!(self.worktree, Some(StatusKind::Deleted | StatusKind::Untracked))
    }
}

/// One immutable status snapshot for a repository generation.
#[derive(Clone, Debug)]
pub struct StatusSnapshot {
    pub identity: RepositoryIdentity,
    pub head: HeadInfo,
    pub entries: BTreeMap<PathBuf, EntryStatus>,
    /// The strongest change below each affected folder, relative to the worktree root.
    pub affected_folders: BTreeMap<PathBuf, StatusKind>,
    pub changed_count: usize,
    pub has_conflicts: bool,
    pub renames_by_source: BTreeMap<PathBuf, PathBuf>,
    /// The first line of `HEAD`'s commit message, bounded for display.
    pub last_commit: Option<String>,
    /// True when the worktree is cloud-backed and status would require content
    /// reads; Crow shows "status unavailable" instead of risking hydration.
    pub unavailable: bool,
}

impl StatusSnapshot {
    pub fn lookup_with_ignored_ancestors(&self, relative: &Path) -> Option<&EntryStatus> {
        self.entries.get(relative).or_else(|| {
            relative.ancestors().skip(1).find_map(|parent| self.entries.get(parent).filter(|status| status.ignored))
        })
    }
}

/// A worktree whose files may be cloud placeholders. Status compares worktree
/// bytes and could hydrate them, so the provider reports status unavailable
/// instead. P8 owns the per-file read proof; this path check is the conservative
/// project-level gate.
pub fn is_cloud_backed_worktree(path: &Path) -> bool {
    path.ancestors().any(|ancestor| {
        ["Library/Mobile Documents", "com~apple~CloudDocs", "Library/CloudStorage"]
            .iter()
            .any(|suffix| ancestor.ends_with(suffix))
    })
}

/// A snapshot with branch and identity but no status reads.
pub fn unavailable_snapshot(repo: &gix::Repository) -> gix::Result<StatusSnapshot> {
    Ok(StatusSnapshot {
        identity: RepositoryIdentity::from_repo(repo)?,
        head: head_info(repo)?,
        entries: BTreeMap::new(),
        affected_folders: BTreeMap::new(),
        changed_count: 0,
        has_conflicts: false,
        renames_by_source: BTreeMap::new(),
        last_commit: None,
        unavailable: true,
    })
}

/// Open a repository whose worktree is exactly `worktree`, if it has one.
pub fn open_worktree(worktree: &Path) -> gix::Result<gix::Repository> {
    let repo = gix::open(worktree)?;
    if repo.workdir().is_none() {
        return Err(gix::Error::from_error(gix::error::message("repository has no worktree")));
    }
    Ok(repo)
}

/// Walk up from a canonical folder to the nearest repository marker.
pub fn enclosing_worktree(folder: &Path) -> Option<PathBuf> {
    folder
        .ancestors()
        .find(|candidate| candidate.join(".git").metadata().is_ok_and(|meta| meta.is_dir() || meta.is_file()))
        .map(Path::to_path_buf)
}

fn safe_filter_options(repo: &gix::Repository) -> gix::Result<gix::filter::plumbing::pipeline::Options> {
    let mut options = gix::filter::Pipeline::options(repo)?;
    options.drivers.clear();
    Ok(options)
}

fn safe_resource_cache(
    repo: &gix::Repository,
    index: &gix::index::State,
    mode: gix::diff::blob::pipeline::Mode,
    old_root: Option<PathBuf>,
    new_root: Option<PathBuf>,
) -> gix::Result<gix::diff::blob::Platform> {
    let attr_stack =
        repo.attributes_only(index, gix::worktree::stack::state::attributes::Source::WorktreeThenIdMapping)?.detach();
    let filter =
        gix::filter::plumbing::Pipeline::new(repo.command_context()?, repo.object_hash(), safe_filter_options(repo)?);
    Ok(gix::diff::blob::Platform::new(
        gix::diff::blob::platform::Options { algorithm: None, skip_internal_diff_if_external_is_configured: false },
        gix::diff::blob::Pipeline::new(
            gix::diff::blob::pipeline::WorktreeRoots { old_root, new_root },
            filter,
            Vec::new(),
            gix::diff::blob::pipeline::Options {
                large_file_threshold_bytes: MAX_LINE_DIFF_BYTES,
                ..Default::default()
            },
        ),
        mode,
        attr_stack,
    ))
}

/// Compute the combined status without running repository-configured filters.
pub fn status_snapshot(repo: &gix::Repository, cancel: &AtomicBool) -> gix::Result<StatusSnapshot> {
    let check_cancel = || -> gix::Result<()> {
        if cancel.load(Ordering::Relaxed) { Err(gix::error::message("status cancelled").into()) } else { Ok(()) }
    };
    check_cancel()?;
    let identity = RepositoryIdentity::from_repo(repo)?;
    let head = head_info(repo)?;
    let index = repo.index_or_empty()?;
    let mut entries: BTreeMap<PathBuf, EntryStatus> = BTreeMap::new();

    for staged in staged_status(repo, &index, cancel)? {
        let entry = entries.entry(staged.path).or_default();
        entry.index = Some(staged.kind);
        entry.renamed_from = staged.renamed_from;
    }
    check_cancel()?;
    for worktree in worktree_status(repo, &index, cancel)? {
        let entry = entries.entry(worktree.path).or_default();
        if worktree.ignored {
            entry.ignored = true;
        } else {
            if worktree.kind == StatusKind::Unmerged {
                entry.index = Some(StatusKind::Unmerged);
            }
            entry.worktree = Some(worktree.kind);
        }
    }
    check_cancel()?;
    let mut changed_count = 0;
    let mut has_conflicts = false;
    let mut renames_by_source = BTreeMap::new();
    let mut affected_folders = BTreeMap::new();
    for (path, status) in &entries {
        let Some(kind) = status.badge().filter(|_| status.is_changed()) else { continue };
        check_cancel()?;
        changed_count += 1;
        has_conflicts |= kind == StatusKind::Unmerged;
        mark_parent_folders(&mut affected_folders, path, kind);
        if let Some(source) = &status.renamed_from {
            mark_parent_folders(&mut affected_folders, source, StatusKind::Renamed);
            renames_by_source.insert(source.clone(), path.clone());
        }
    }
    let last_commit = repo.head_commit().ok().and_then(|commit| {
        let title = commit.message().ok()?.title;
        let title = String::from_utf8_lossy(title);
        let title = title.trim();
        (!title.is_empty()).then(|| title.chars().take(120).map(|c| if c.is_control() { ' ' } else { c }).collect())
    });
    Ok(StatusSnapshot {
        identity,
        head,
        entries,
        affected_folders,
        changed_count,
        has_conflicts,
        renames_by_source,
        last_commit,
        unavailable: false,
    })
}

fn mark_parent_folders(folders: &mut BTreeMap<PathBuf, StatusKind>, path: &Path, kind: StatusKind) {
    for parent in path.ancestors().skip(1).take_while(|parent| !parent.as_os_str().is_empty()) {
        if let Some(previous) = folders.get_mut(parent) {
            // Ancestors already have this strong a badge too.
            if *previous <= kind {
                break;
            }
            *previous = kind;
        } else {
            folders.insert(parent.to_path_buf(), kind);
        }
    }
}

struct StagedEntry {
    path: PathBuf,
    kind: StatusKind,
    renamed_from: Option<PathBuf>,
}

fn staged_status(
    repo: &gix::Repository,
    index: &gix::index::State,
    cancel: &AtomicBool,
) -> gix::Result<Vec<StagedEntry>> {
    let head_tree = repo.head_tree_id_or_empty()?;
    let mut lhs: gix::index::State = repo.index_from_tree(&head_tree)?.into();
    lhs.sort_entries();
    let mut search = repo
        .pathspec(false, None::<&str>, false, index, gix::worktree::stack::state::attributes::Source::IdMapping)?
        .into_parts()
        .0;
    let mut cache = safe_resource_cache(repo, index, gix::diff::blob::pipeline::Mode::ToGit, None, None)?;
    let mut out = Vec::new();
    let mut attributes = |_: &BStr,
                          _: gix::filter::plumbing::attributes::glob::pattern::Case,
                          _: bool,
                          _: &mut gix::filter::plumbing::attributes::search::Outcome| false;
    gix::diff::index(
        &lhs,
        index,
        |change| {
            if cancel.load(Ordering::Relaxed) {
                return Ok(std::ops::ControlFlow::Break(()));
            }
            match &change {
                gix::diff::index::ChangeRef::Addition { location, .. } => out.push(StagedEntry {
                    path: path_from_bstr(location)?,
                    kind: StatusKind::Added,
                    renamed_from: None,
                }),
                gix::diff::index::ChangeRef::Deletion { location, .. } => out.push(StagedEntry {
                    path: path_from_bstr(location)?,
                    kind: StatusKind::Deleted,
                    renamed_from: None,
                }),
                gix::diff::index::ChangeRef::Modification { location, previous_entry_mode, entry_mode, .. } => out
                    .push(StagedEntry {
                        path: path_from_bstr(location)?,
                        kind: if previous_entry_mode == entry_mode {
                            StatusKind::Modified
                        } else {
                            StatusKind::TypeChange
                        },
                        renamed_from: None,
                    }),
                gix::diff::index::ChangeRef::Rewrite { source_location, copy, location, .. } => {
                    if !*copy {
                        out.push(StagedEntry {
                            path: path_from_bstr(location)?,
                            kind: StatusKind::Renamed,
                            renamed_from: Some(path_from_bstr(source_location)?),
                        });
                    } else {
                        out.push(StagedEntry {
                            path: path_from_bstr(location)?,
                            kind: StatusKind::Added,
                            renamed_from: None,
                        });
                    }
                }
            }
            Ok(std::ops::ControlFlow::Continue(()))
        },
        Some(gix::diff::index::RewriteOptions {
            resource_cache: &mut cache,
            find: repo,
            rewrites: gix::diff::Rewrites::default(),
        }),
        &mut search,
        &mut attributes,
    )?;
    Ok(out)
}

struct WorktreeEntry {
    path: PathBuf,
    kind: StatusKind,
    ignored: bool,
}

fn worktree_status(
    repo: &gix::Repository,
    index: &gix::index::State,
    cancel: &AtomicBool,
) -> gix::Result<Vec<WorktreeEntry>> {
    let workdir =
        repo.workdir().ok_or_else(|| gix::Error::from_error(gix::error::message("repository has no worktree")))?;
    let attr_stack = repo
        .attributes(
            index,
            gix::worktree::stack::state::attributes::Source::WorktreeThenIdMapping,
            gix::worktree::stack::state::ignore::Source::WorktreeThenIdMappingIfNotSkipped,
            None,
        )?
        .detach();
    let filter =
        gix::filter::plumbing::Pipeline::new(repo.command_context()?, repo.object_hash(), safe_filter_options(repo)?);
    let resource_cache = gix::diff::blob::Platform::new(
        gix::diff::blob::platform::Options { algorithm: None, skip_internal_diff_if_external_is_configured: false },
        gix::diff::blob::Pipeline::new(
            gix::diff::blob::pipeline::WorktreeRoots { old_root: None, new_root: Some(workdir.to_owned()) },
            filter,
            Vec::new(),
            Default::default(),
        ),
        gix::diff::blob::pipeline::Mode::ToGit,
        attr_stack,
    );
    let (pathspec, _) = repo
        .pathspec(false, None::<&str>, false, index, gix::worktree::stack::state::attributes::Source::IdMapping)?
        .into_parts();
    let current_dir = repo.current_dir().to_owned();
    let git_dir_realpath = gix::path::realpath_opts(repo.git_dir(), &current_dir, gix::path::realpath::MAX_SYMLINKS)?;
    let dirwalk = repo
        .dirwalk_options()?
        .emit_untracked(gix::dir::walk::EmissionMode::Matching)
        .emit_ignored(Some(gix::dir::walk::EmissionMode::Matching));
    let fs = repo.filesystem_options()?;
    let accelerate = fs.ignore_case.then(|| index.prepare_icase_backing());
    let mut recorder: Recorder<'_, (), ()> = Recorder::default();
    let mut progress = gix::progress::Discard;
    gix::status::plumbing::index_as_worktree_with_renames(
        index,
        workdir,
        &mut recorder,
        gix::status::plumbing::index_as_worktree::traits::FastEq,
        NoSubmodules,
        repo.objects.clone().into_arc().expect("an arc-backed object store"),
        &mut progress,
        Context {
            pathspec,
            resource_cache,
            should_interrupt: cancel,
            dirwalk: DirwalkContext {
                git_dir_realpath: &git_dir_realpath,
                current_dir: &current_dir,
                ignore_case_index_lookup: accelerate.as_ref(),
            },
        },
        Options {
            sorting: None,
            tracked_file_modifications: gix::status::plumbing::index_as_worktree::Options {
                fs,
                thread_limit: Some(1),
                stat: repo.stat_options()?,
                fscache: false,
            },
            fscache: false,
            dirwalk: Some(dirwalk.into()),
            rewrites: None,
        },
    )?;

    let mut out = Vec::new();
    for entry in recorder.records {
        let ignored = matches!(
            &entry,
            Entry::DirectoryContents { entry, .. } if matches!(entry.status, gix::dir::entry::Status::Ignored(_))
        );
        if let Entry::Rewrite { source, dirwalk_entry, .. } = &entry {
            out.push(WorktreeEntry {
                path: path_from_bstr(source.rela_path())?,
                kind: StatusKind::Deleted,
                ignored: false,
            });
            out.push(WorktreeEntry {
                path: path_from_bstr(dirwalk_entry.rela_path.as_ref())?,
                kind: StatusKind::Untracked,
                ignored: false,
            });
            continue;
        }
        let path = path_from_bstr(entry.destination_rela_path())?;
        if ignored {
            out.push(WorktreeEntry { path, kind: StatusKind::Ignored, ignored: true });
            continue;
        }
        let Some(summary) = entry.summary() else { continue };
        let kind = match summary {
            Summary::Conflict => StatusKind::Unmerged,
            Summary::Removed => StatusKind::Deleted,
            Summary::Added => StatusKind::Untracked,
            Summary::Modified => StatusKind::Modified,
            Summary::TypeChange => StatusKind::TypeChange,
            Summary::Renamed => StatusKind::Renamed,
            Summary::Copied => StatusKind::Added,
            Summary::IntentToAdd => StatusKind::IntentToAdd,
        };
        out.push(WorktreeEntry { path, kind, ignored: false });
    }
    Ok(out)
}

fn path_from_bstr(path: &BStr) -> gix::Result<PathBuf> {
    Ok(gix::path::from_bstr(path)?.into_owned())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineCounts {
    pub insertions: u32,
    pub removals: u32,
}

#[derive(Clone, Copy)]
struct DiffSide<'a> {
    id: Option<gix::hash::ObjectId>,
    mode: gix::index::entry::Mode,
    path: &'a BStr,
}

fn prepare_sides(
    cache: &mut gix::diff::blob::Platform,
    old: DiffSide<'_>,
    new: DiffSide<'_>,
    repo: &gix::Repository,
) -> gix::Result<bool> {
    let kind = |mode: gix::index::entry::Mode| match mode {
        gix::index::entry::Mode::FILE | gix::index::entry::Mode::FILE_EXECUTABLE => {
            Some(gix::object::tree::EntryKind::Blob)
        }
        gix::index::entry::Mode::SYMLINK => Some(gix::object::tree::EntryKind::Link),
        _ => None,
    };
    let (Some(old_kind), Some(new_kind)) = (kind(old.mode), kind(new.mode)) else { return Ok(false) };
    cache.clear_resource_cache_keep_allocation();
    cache.set_resource(
        old.id.unwrap_or_else(|| gix::hash::ObjectId::null(repo.object_hash())),
        old_kind,
        old.path,
        gix::diff::blob::ResourceKind::OldOrSource,
        &repo.objects,
    )?;
    cache.set_resource(
        new.id.unwrap_or_else(|| gix::hash::ObjectId::null(repo.object_hash())),
        new_kind,
        new.path,
        gix::diff::blob::ResourceKind::NewOrDestination,
        &repo.objects,
    )?;
    Ok(true)
}

fn counts_for(
    cache: &mut gix::diff::blob::Platform,
    old: DiffSide<'_>,
    new: DiffSide<'_>,
    repo: &gix::Repository,
) -> gix::Result<Option<LineCounts>> {
    if !prepare_sides(cache, old, new, repo)? {
        return Ok(None);
    }
    let mut platform = gix::object::blob::diff::Platform { resource_cache: cache };
    Ok(platform.line_counts()?.map(|stats| LineCounts { insertions: stats.insertions, removals: stats.removals }))
}

use gix::diff::blob::{diff_with_slider_heuristics, platform::prepare_diff::Operation};

#[derive(Debug, PartialEq, Eq)]
pub enum DiffLayer<T> {
    Clean,
    Unavailable,
    Text(T),
}

pub struct FileDiff {
    pub staged: DiffLayer<TextDiff>,
    pub unstaged: DiffLayer<TextDiff>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct TextDiff {
    pub lines: Vec<DiffLine>,
    pub truncated: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum DiffLine {
    Hunk { before: u32, after: u32 },
    Removed(String),
    Added(String),
}

fn text_diff_for(
    cache: &mut gix::diff::blob::Platform,
    old: DiffSide<'_>,
    new: DiffSide<'_>,
    repo: &gix::Repository,
) -> gix::Result<Option<TextDiff>> {
    if !prepare_sides(cache, old, new, repo)? {
        return Ok(None);
    }
    let prepared = cache.prepare_diff()?;
    let Operation::InternalDiff { algorithm } = prepared.operation else { return Ok(None) };
    let input = prepared.interned_input();
    let diff = diff_with_slider_heuristics(algorithm, &input);
    let mut lines = Vec::new();
    const PREVIEW_LINES: usize = 240;
    for hunk in diff.hunks() {
        lines.push(DiffLine::Hunk { before: hunk.before.start + 1, after: hunk.after.start + 1 });
        let removed = hunk.before.map(|i| (false, input.before[i as usize]));
        let added = hunk.after.map(|i| (true, input.after[i as usize]));
        for (added, token) in removed.chain(added) {
            if lines.len() >= PREVIEW_LINES {
                lines.truncate(PREVIEW_LINES);
                return Ok(Some(TextDiff { lines, truncated: true }));
            }
            let line: String = String::from_utf8_lossy(input.interner[token])
                .chars()
                .take(400)
                .map(|ch| if ch.is_control() && ch != '\t' { ' ' } else { ch })
                .collect();
            lines.push(if added { DiffLine::Added(line) } else { DiffLine::Removed(line) });
        }
    }
    Ok(Some(TextDiff { lines, truncated: false }))
}

/// Reuse the index, HEAD tree and filter-free diff setup for a visible batch.
pub struct LineDiff<'repo> {
    repo: &'repo gix::Repository,
    index: gix::worktree::Index,
    head: gix::Tree<'repo>,
    staged: gix::diff::blob::Platform,
    unstaged: gix::diff::blob::Platform,
}

impl<'repo> LineDiff<'repo> {
    pub fn new(repo: &'repo gix::Repository) -> gix::Result<Self> {
        let workdir =
            repo.workdir().ok_or_else(|| gix::Error::from_error(gix::error::message("repository has no worktree")))?;
        let index = repo.index_or_empty()?;
        let mode = gix::diff::blob::pipeline::Mode::ToGit;
        let staged = safe_resource_cache(repo, &index, mode, None, None)?;
        let unstaged = safe_resource_cache(repo, &index, mode, None, Some(workdir.to_owned()))?;
        let head = repo.find_tree(repo.head_tree_id_or_empty()?)?;
        Ok(Self { repo, index, head, staged, unstaged })
    }

    pub fn counts(&mut self, snapshot: &StatusSnapshot, relative: &Path) -> gix::Result<Option<LineCounts>> {
        let (staged, unstaged) = self.layers(snapshot, relative, counts_for)?;
        let mut total = LineCounts { insertions: 0, removals: 0 };
        for layer in [staged, unstaged] {
            match layer {
                DiffLayer::Clean => {}
                DiffLayer::Unavailable => return Ok(None),
                DiffLayer::Text(count) => {
                    total.insertions = total.insertions.saturating_add(count.insertions);
                    total.removals = total.removals.saturating_add(count.removals);
                }
            }
        }
        Ok(Some(total))
    }

    pub fn diff(&mut self, snapshot: &StatusSnapshot, relative: &Path) -> gix::Result<FileDiff> {
        let (staged, unstaged) = self.layers(snapshot, relative, text_diff_for)?;
        Ok(FileDiff { staged, unstaged })
    }

    fn layers<T>(
        &mut self,
        snapshot: &StatusSnapshot,
        relative: &Path,
        diff: impl Fn(
            &mut gix::diff::blob::Platform,
            DiffSide<'_>,
            DiffSide<'_>,
            &gix::Repository,
        ) -> gix::Result<Option<T>>,
    ) -> gix::Result<(DiffLayer<T>, DiffLayer<T>)> {
        let Some(status) = snapshot.entries.get(relative).filter(|entry| entry.can_count_lines()) else {
            return Ok((DiffLayer::Unavailable, DiffLayer::Unavailable));
        };
        let path = gix::path::into_bstr(relative)?;
        let index = self.index.entry_by_path(path.as_ref());
        let new = DiffSide {
            id: index.map(|entry| entry.id),
            mode: index.map_or(gix::index::entry::Mode::FILE, |entry| entry.mode),
            path: path.as_ref(),
        };
        let staged = if status.index.is_some() {
            let old_path = status.renamed_from.as_deref().unwrap_or(relative);
            let entry = self.head.lookup_entry_by_path(old_path)?;
            diff(
                &mut self.staged,
                DiffSide {
                    id: entry.as_ref().map(|entry| entry.object_id()),
                    mode: entry
                        .as_ref()
                        .map_or(gix::index::entry::Mode::FILE, |entry| mode_from_tree(entry.mode().into())),
                    path: gix::path::into_bstr(old_path)?.as_ref(),
                },
                new,
                self.repo,
            )?
            .map_or(DiffLayer::Unavailable, DiffLayer::Text)
        } else {
            DiffLayer::Clean
        };
        let unstaged = if status.worktree.is_some() {
            let live = self.repo.workdir().expect("a worktree").join(relative);
            if std::fs::symlink_metadata(live).is_ok_and(|meta| meta.is_file() && meta.len() <= MAX_LINE_DIFF_BYTES) {
                diff(&mut self.unstaged, new, DiffSide { id: None, ..new }, self.repo)?
                    .map_or(DiffLayer::Unavailable, DiffLayer::Text)
            } else {
                DiffLayer::Unavailable
            }
        } else {
            DiffLayer::Clean
        };
        Ok((staged, unstaged))
    }
}

fn mode_from_tree(mode: gix::object::tree::EntryKind) -> gix::index::entry::Mode {
    match mode {
        gix::object::tree::EntryKind::Blob => gix::index::entry::Mode::FILE,
        gix::object::tree::EntryKind::BlobExecutable => gix::index::entry::Mode::FILE_EXECUTABLE,
        gix::object::tree::EntryKind::Link => gix::index::entry::Mode::SYMLINK,
        _ => gix::index::entry::Mode::COMMIT,
    }
}

/// A repository found by a bounded Projects root walk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredRepository {
    pub identity: RepositoryIdentity,
    pub head: HeadInfo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiscoveryLimits {
    pub max_depth: usize,
    pub max_directories: usize,
    pub max_repositories: usize,
}

impl Default for DiscoveryLimits {
    fn default() -> Self {
        Self { max_depth: 8, max_directories: 20_000, max_repositories: 256 }
    }
}

#[derive(Clone, Debug, Default)]
pub struct DiscoveryReport {
    pub repositories: Vec<DiscoveredRepository>,
    pub directories: usize,
    pub truncated: bool,
}

/// Walk `roots` for `.git` files and directories, pruning packages and
/// symlinks. Stops at the first limit and reports `truncated`.
pub fn discover(
    roots: &[PathBuf],
    limits: DiscoveryLimits,
    cancel: &AtomicBool,
    mut on_repository: impl FnMut(DiscoveredRepository),
) -> DiscoveryReport {
    let mut report = DiscoveryReport::default();
    let mut stack = Vec::new();
    for root in roots {
        let Ok(metadata) = std::fs::symlink_metadata(root) else { continue };
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            stack.push((root.clone(), 0usize));
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    while let Some((dir, depth)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            report.truncated = true;
            break;
        }
        if report.directories >= limits.max_directories || report.repositories.len() >= limits.max_repositories {
            report.truncated = true;
            break;
        }
        report.directories += 1;
        let dot_git = dir.join(".git");
        if (dot_git.is_dir() || dot_git.is_file())
            && let Ok(repo) = gix::open(&dir)
            && let Ok(identity) = RepositoryIdentity::from_repo(&repo)
            && seen.insert(identity.worktree.clone())
            && let Ok(head) = head_info(&repo)
        {
            let found = DiscoveredRepository { identity, head };
            on_repository(found.clone());
            report.repositories.push(found);
        }
        if depth >= limits.max_depth {
            continue;
        }
        let Ok(read_dir) = std::fs::read_dir(&dir) else { continue };
        for entry in read_dir.flatten() {
            let name = entry.file_name();
            if name == OsStr::new(".git") {
                continue;
            }
            let Ok(file_type) = entry.file_type() else { continue };
            if file_type.is_symlink() || !file_type.is_dir() {
                continue;
            }
            if crate::is_pruned_package(&name) {
                continue;
            }
            stack.push((entry.path(), depth + 1));
        }
    }
    report.repositories.sort_by(|a, b| a.identity.worktree.cmp(&b.identity.worktree));
    report
}

/// Package-ish directories that can contain thousands of files and are not
/// project roots in their own right.
pub fn is_pruned_package(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else { return false };
    matches!(
        Path::new(name).extension().and_then(|ext| ext.to_str()),
        Some("app" | "bundle" | "framework" | "photoslibrary" | "xcodeproj" | "xcworkspace" | "playground" | "dSYM")
    ) || matches!(
        name,
        "node_modules" | "target" | "Pods" | "DerivedData" | ".build" | ".venv" | "venv" | "__pycache__"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn repository_relative_paths_keep_raw_bytes() {
        let path = path_from_bstr(gix::bstr::BStr::new(b"bad\xff.txt")).unwrap();
        assert_eq!(path.as_os_str().as_bytes(), b"bad\xff.txt");
    }
}
