//! App-wide file operations: running them in the background with progress and
//! cancel, the undo/redo stacks, the activity log and the file clipboard.
//! The engine itself (planning, safety rules, undo) lives in `crow_fs::ops`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use crow_fs::ops::{self, Outcome, Plan, Progress, Record, Resolution, Trash};
use futures::StreamExt;
use futures::channel::mpsc::unbounded;
use gpui::{App, Global, SharedString, prelude::*};

/// What dropping `paths` on the folder `into` does, like Finder: a move within a
/// volume, a copy across volumes; `copy` (Option held) forces a copy. Items already
/// in `into`, and folders dropped onto themselves or into their own contents, are
/// left out; `None` when nothing is left.
pub fn drop_request(paths: &[PathBuf], into: PathBuf, copy: bool) -> Option<ops::Request> {
    let paths: Vec<PathBuf> =
        paths.iter().filter(|p| p.parent() != Some(into.as_path()) && !into.starts_with(p)).cloned().collect();
    if paths.is_empty() {
        return None;
    }
    let same_volume = paths.iter().all(|p| same_volume(p, &into));
    Some(if same_volume && !copy { ops::Request::Move { paths, into } } else { ops::Request::Copy { paths, into } })
}

fn same_volume(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (a.symlink_metadata(), b.metadata()) {
        (Ok(x), Ok(y)) => x.dev() == y.dev(),
        _ => false,
    }
}

pub struct LogEntry {
    pub record: Record,
    pub at: SystemTime,
    pub error: Option<String>,
    pub cancelled: bool,
    pub undone: bool,
}

pub struct Running {
    pub label: String,
    pub progress: Progress,
    pub cancel: Arc<AtomicBool>,
}

/// What Cmd-C / Cmd-X put on the clipboard. The paths also go on the system
/// pasteboard; `change_count` tells whether it still holds them (or someone copied since).
pub struct Clipboard {
    pub paths: Vec<PathBuf>,
    pub cut: bool,
    pub change_count: Option<isize>,
}

pub struct Operations {
    trash: Arc<dyn Trash>,
    undo_stack: Vec<usize>,
    redo_stack: Vec<Record>,
    pub log: Vec<LogEntry>,
    pub running: Option<Running>,
    pub clipboard: Option<Clipboard>,
    /// The latest result or error, for the status line.
    pub message: Option<(SharedString, bool)>,
}

impl Global for Operations {}

pub fn init(cx: &mut App, trash: Arc<dyn Trash>) {
    cx.set_global(Operations {
        trash,
        undo_stack: Vec::new(),
        redo_stack: Vec::new(),
        log: Vec::new(),
        running: None,
        clipboard: None,
        message: None,
    });
}

/// Carry out `plan` in the background with the user's conflict choices.
/// `on_done` gets the outcome on the main thread (to select the results).
pub fn execute(
    plan: Plan,
    resolutions: HashMap<PathBuf, Resolution>,
    cx: &mut App,
    on_done: impl FnOnce(&Outcome, &mut App) + 'static,
) {
    let ops = cx.global_mut::<Operations>();
    if ops.running.is_some() {
        ops.message = Some(("Another operation is still running".into(), true));
        cx.refresh_windows();
        return;
    }
    let cancel = Arc::new(AtomicBool::new(false));
    ops.running = Some(Running { label: plan.label.clone(), progress: Progress::default(), cancel: cancel.clone() });
    ops.message = None;
    let trash = ops.trash.clone();
    let (tx, mut rx) = unbounded::<Progress>();
    let task = cx.background_spawn(async move {
        let mut last_sent = Instant::now() - Duration::from_secs(1);
        let outcome = ops::execute(&plan, &resolutions, trash.as_ref(), &cancel, &mut |p| {
            // At most ~30 updates a second reach the UI.
            if last_sent.elapsed() >= Duration::from_millis(33) || p.done_items == p.total_items {
                last_sent = Instant::now();
                tx.unbounded_send(p.clone()).ok();
            }
        });
        drop(tx);
        outcome
    });
    cx.spawn(async move |cx| {
        while let Some(progress) = rx.next().await {
            cx.update(|cx| {
                if let Some(running) = cx.global_mut::<Operations>().running.as_mut() {
                    running.progress = progress;
                }
                cx.refresh_windows();
            });
        }
        let outcome = task.await;
        cx.update(|cx| {
            finish(&outcome, cx);
            on_done(&outcome, cx);
            cx.refresh_windows();
        });
    })
    .detach();
}

fn finish(outcome: &Outcome, cx: &mut App) {
    let ops = cx.global_mut::<Operations>();
    ops.running = None;
    let label = outcome.record.label.clone();
    ops.message = match (&outcome.error, outcome.cancelled) {
        (Some(e), _) => Some((e.to_string().into(), true)),
        (None, true) => Some((format!("{label}: cancelled").into(), false)),
        (None, false) => None,
    };
    if outcome.record.steps.is_empty() {
        return;
    }
    tracing::info!(label = %label, steps = outcome.record.steps.len(), "operation done");
    ops.log.push(LogEntry {
        record: outcome.record.clone(),
        at: SystemTime::now(),
        error: outcome.error.as_ref().map(|e| e.to_string()),
        cancelled: outcome.cancelled,
        undone: false,
    });
    if outcome.record.is_undoable() {
        ops.undo_stack.push(ops.log.len() - 1);
    }
    ops.redo_stack.clear();
}

/// Undo log entry `index` (the newest by default). Undo checks the disk first and
/// refuses rather than overwrite anything that changed since.
pub fn undo(index: Option<usize>, cx: &mut App, on_done: impl FnOnce(Vec<PathBuf>, &mut App) + 'static) {
    let ops = cx.global_mut::<Operations>();
    let Some(index) = index.or_else(|| ops.undo_stack.last().copied()) else { return };
    let Some(entry) = ops.log.get(index).filter(|e| !e.undone && e.record.is_undoable()) else { return };
    let (record, trash) = (entry.record.clone(), ops.trash.clone());
    cx.spawn(async move |cx| {
        let result = cx.background_spawn(async move { ops::undo(&record, trash.as_ref()) }).await;
        cx.update(|cx| {
            let ops = cx.global_mut::<Operations>();
            match result {
                Ok(reversed) => {
                    ops.undo_stack.retain(|&i| i != index);
                    if let Some(entry) = ops.log.get_mut(index) {
                        entry.undone = true;
                    }
                    ops.message = Some((reversed.label.clone().into(), false));
                    let restored = reversed.results();
                    ops.redo_stack.push(reversed);
                    on_done(restored, cx);
                }
                Err(e) => ops.message = Some((e.to_string().into(), true)),
            }
            cx.refresh_windows();
        });
    })
    .detach();
}

/// Redo the most recently undone operation.
pub fn redo(cx: &mut App, on_done: impl FnOnce(Vec<PathBuf>, &mut App) + 'static) {
    let ops = cx.global_mut::<Operations>();
    let Some(reversed) = ops.redo_stack.pop() else { return };
    let trash = ops.trash.clone();
    cx.spawn(async move |cx| {
        let result = cx.background_spawn(async move { ops::undo(&reversed, trash.as_ref()) }).await;
        cx.update(|cx| {
            let ops = cx.global_mut::<Operations>();
            match result {
                Ok(redone) => {
                    let label = redone.label.trim_start_matches("Undo Undo ").to_owned();
                    let results = redone.results();
                    ops.log.push(LogEntry {
                        record: Record { label: label.clone(), steps: redone.steps },
                        at: SystemTime::now(),
                        error: None,
                        cancelled: false,
                        undone: false,
                    });
                    ops.undo_stack.push(ops.log.len() - 1);
                    ops.message = Some((format!("Redo {label}").into(), false));
                    on_done(results, cx);
                }
                Err(e) => ops.message = Some((e.to_string().into(), true)),
            }
            cx.refresh_windows();
        });
    })
    .detach();
}

pub fn cancel_running(cx: &mut App) {
    if let Some(running) = &cx.global::<Operations>().running {
        running.cancel.store(true, Ordering::Relaxed);
    }
}

/// A Trash that is a plain folder, so tests never touch the real one.
#[cfg(test)]
pub struct FolderTrash(pub PathBuf);

#[cfg(test)]
impl Trash for FolderTrash {
    fn trash(&self, path: &std::path::Path) -> std::io::Result<PathBuf> {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let name = path.file_name().ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
        let dest = self.0.join(format!("{}-{}", N.fetch_add(1, Ordering::Relaxed), name.to_string_lossy()));
        std::fs::rename(path, &dest)?;
        Ok(dest)
    }
}
