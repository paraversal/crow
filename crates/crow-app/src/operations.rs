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
    let paths: Vec<PathBuf> = paths.iter().filter(|p| drop_moves(p, &into)).cloned().collect();
    if paths.is_empty() {
        return None;
    }
    let same_volume = paths.iter().all(|p| same_volume(p, &into));
    Some(if same_volume && !copy { ops::Request::Move { paths, into } } else { ops::Request::Copy { paths, into } })
}

/// Whether dropping `paths` into `into` would do anything: not every item is already
/// there, and none is `into` itself or a folder above it.
pub fn drop_would_act(paths: &[PathBuf], into: &Path) -> bool {
    paths.iter().any(|path| drop_moves(path, into))
}

fn drop_moves(path: &Path, into: &Path) -> bool {
    path.parent() != Some(into) && !into.starts_with(path)
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
    /// When the operation started, so a short copy stays silent.
    pub started: Instant,
}

/// The plan's label in the progressive form the bar shows: "Copy 3 items" → "Copying 3 items".
pub fn running_label(label: &str) -> String {
    let (verb, rest) = label.split_once(' ').unwrap_or((label, ""));
    let ing = match verb {
        "Copy" => "Copying",
        "Move" => "Moving",
        "Duplicate" => "Duplicating",
        "Delete" => "Deleting",
        _ => verb,
    };
    if rest.is_empty() { ing.to_owned() } else { format!("{ing} {rest}") }
}

/// What Cmd-C / Cmd-X put on the clipboard. The paths also go on the system
/// pasteboard; `change_count` tells whether it still holds them (or someone copied since).
pub struct Clipboard {
    pub paths: Vec<PathBuf>,
    pub cut: bool,
    pub change_count: Option<isize>,
}

/// What a paste would bring in, and whether it moves them: our clipboard if the
/// pasteboard still holds what we put there; otherwise whatever another app (Finder) copied.
pub fn pasteboard_paths(cx: &App) -> (Vec<PathBuf>, bool) {
    let ours = cx
        .global::<Operations>()
        .clipboard
        .as_ref()
        .filter(|c| c.change_count == crow_macos::pasteboard_change_count());
    match ours {
        Some(c) => (c.paths.clone(), c.cut),
        None => (crow_macos::read_file_urls(), false),
    }
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
    /// Bumped by every message, so an old timer cannot clear a newer one and the
    /// bar can fade in a new one.
    pub message_generation: u64,
    /// The fraction a finished operation's bar shows while it fades, and its id.
    pub progress_done: Option<(f32, u64)>,
    progress_generation: u64,
}

/// Info messages clear after this long; errors stay until something replaces them.
const MESSAGE_LIFETIME: Duration = Duration::from_secs(5);

impl Global for Operations {}

/// Show a message in the bar. Info messages retire after five seconds (Q2); errors
/// stay until the next operation, navigation or a fixed config file.
pub fn message(text: impl Into<SharedString>, is_error: bool, cx: &mut App) {
    let generation = {
        let ops = cx.global_mut::<Operations>();
        ops.message_generation += 1;
        ops.message = Some((text.into(), is_error));
        ops.message_generation
    };
    if is_error {
        return;
    }
    cx.spawn(async move |cx| {
        cx.background_executor().timer(MESSAGE_LIFETIME).await;
        cx.update(|cx| {
            let ops = cx.global_mut::<Operations>();
            if ops.message_generation == generation {
                ops.message = None;
                cx.refresh_windows();
            }
        });
    })
    .detach();
}

/// Drop the message now; a pending info timer becomes a no-op.
pub fn clear_message(cx: &mut App) {
    let ops = cx.global_mut::<Operations>();
    ops.message_generation += 1;
    ops.message = None;
}

pub fn init(cx: &mut App, trash: Arc<dyn Trash>) {
    cx.set_global(Operations {
        trash,
        undo_stack: Vec::new(),
        redo_stack: Vec::new(),
        log: Vec::new(),
        running: None,
        clipboard: None,
        message: None,
        message_generation: 0,
        progress_done: None,
        progress_generation: 0,
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
    if cx.global::<Operations>().running.is_some() {
        message("Another operation is still running", true, cx);
        cx.refresh_windows();
        return;
    }
    let ops = cx.global_mut::<Operations>();
    let cancel = Arc::new(AtomicBool::new(false));
    ops.running = Some(Running {
        label: plan.label.clone(),
        progress: Progress::default(),
        cancel: cancel.clone(),
        started: Instant::now(),
    });
    ops.message_generation += 1;
    ops.message = None;
    let trash = ops.trash.clone();
    let event = crate::feedback::event_for(&plan.request);
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
            finish(&outcome, event, cx);
            on_done(&outcome, cx);
            cx.refresh_windows();
        });
    })
    .detach();
}

fn finish(outcome: &Outcome, event: Option<crow_core::motion::Event>, cx: &mut App) {
    let label = outcome.record.label.clone();
    let shown = {
        let ops = cx.global::<Operations>();
        ops.running.as_ref().is_some_and(|running| running.started.elapsed() >= Duration::from_secs(1))
    };
    let done_fraction = {
        let ops = cx.global::<Operations>();
        ops.running.as_ref().map(|running| {
            let p = &running.progress;
            let (done, total) = if p.total_bytes > 0 {
                (p.done_bytes, p.total_bytes)
            } else {
                (p.done_items as u64, p.total_items as u64)
            };
            if total == 0 { 0.0 } else { (done as f32 / total as f32).clamp(0.0, 1.0) }
        })
    };
    let progress_id = {
        let ops = cx.global_mut::<Operations>();
        ops.running = None;
        if let Some(fraction) = done_fraction {
            ops.progress_generation += 1;
            let generation = ops.progress_generation;
            ops.progress_done = Some((fraction, generation));
            Some(generation)
        } else {
            None
        }
    };
    {
        let ops = cx.global_mut::<Operations>();
        if !outcome.record.steps.is_empty() {
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
    }
    match (&outcome.error, outcome.cancelled) {
        (Some(e), _) => {
            crate::feedback::play(crow_core::motion::Event::Refuse, cx);
            message(e.to_string(), true, cx);
        }
        (None, true) => message(format!("{label}: cancelled"), false, cx),
        (None, false) => {
            // A copy that never showed its progress bar stays silent, and so does a
            // plan that did nothing.
            if let Some(event) = event
                && !outcome.record.steps.is_empty()
                && (event != crow_core::motion::Event::CopyDone || shown)
            {
                crate::feedback::play(event, cx);
            }
            clear_message(cx);
        }
    }
    // Copying or trashing changes free space; the sidebar shouldn't wait for the minute poll.
    crate::ui::refresh_volumes(cx);
    if let Some(generation) = progress_id {
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_millis(crow_core::motion::ms::COPY_FADE)).await;
            cx.update(|cx| {
                let ops = cx.global_mut::<Operations>();
                if ops.progress_generation == generation {
                    ops.progress_done = None;
                    cx.refresh_windows();
                }
            });
        })
        .detach();
    }
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
                    let label = reversed.label.clone();
                    ops.undo_stack.retain(|&i| i != index);
                    if let Some(entry) = ops.log.get_mut(index) {
                        entry.undone = true;
                    }
                    let restored = reversed.results();
                    ops.redo_stack.push(reversed);
                    message(label, false, cx);
                    crate::feedback::play(crow_core::motion::Event::Undo, cx);
                    crate::ui::refresh_volumes(cx);
                    on_done(restored, cx);
                }
                Err(e) => message(e.to_string(), true, cx),
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
                    message(format!("Redo {label}"), false, cx);
                    crate::feedback::play(crow_core::motion::Event::Undo, cx);
                    crate::ui::refresh_volumes(cx);
                    on_done(results, cx);
                }
                Err(e) => message(e.to_string(), true, cx),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_label_uses_the_ing_form() {
        // The labels `crow_fs::ops::plan` makes.
        let cases = [
            ("Copy 3 items to Backup", "Copying 3 items to Backup"),
            ("Move 1 item to Trash", "Moving 1 item to Trash"),
            ("Duplicate 2 items", "Duplicating 2 items"),
            ("Delete 2 items permanently", "Deleting 2 items permanently"),
            ("New Folder", "New Folder"),
            ("Rename “a” to “b”", "Rename “a” to “b”"),
        ];
        for (label, expected) in cases {
            assert_eq!(running_label(label), expected, "{label}");
        }
    }
}
