//! Whole-disk name search through Spotlight's `mdfind`. It streams paths and
//! stats each result on the calling (background) thread; killing the child
//! stops the query. Spotlight only sees indexed volumes, which the UI says.

use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

const BATCH: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpotlightHit {
    pub path: PathBuf,
    pub is_dir: bool,
    pub modified: Option<SystemTime>,
    pub size: u64,
}

/// Run `mdfind -name query`, sending hits in batches. Returns an error only when
/// Spotlight itself fails; no results is success.
pub fn spotlight_names(
    query: &str,
    mut sink: impl FnMut(Vec<SpotlightHit>),
    cancel: &AtomicBool,
) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(());
    }
    let mut child = Command::new("mdfind")
        .arg("-name")
        .arg(query)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Spotlight search failed: {e}"))?;
    let stdout = child.stdout.take().ok_or_else(|| "Spotlight search failed: no output".to_owned())?;
    let mut batch = Vec::with_capacity(BATCH);
    for line in BufReader::new(stdout).lines() {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            return Ok(());
        }
        let Ok(line) = line else { break };
        if line.is_empty() {
            continue;
        }
        let path = PathBuf::from(line);
        let metadata = std::fs::metadata(&path).ok();
        let is_dir = metadata.as_ref().is_some_and(|meta| meta.is_dir());
        let modified = metadata.as_ref().and_then(|meta| meta.modified().ok());
        let size = metadata.as_ref().map_or(0, |meta| meta.len());
        batch.push(SpotlightHit { path, is_dir, modified, size });
        if batch.len() >= BATCH {
            sink(std::mem::take(&mut batch));
        }
    }
    if !batch.is_empty() {
        sink(batch);
    }
    let status = child.wait().map_err(|e| format!("Spotlight search failed: {e}"))?;
    if !status.success() {
        let mut stderr = String::new();
        if let Some(mut error) = child.stderr.take() {
            let _ = error.read_to_string(&mut stderr);
        }
        return Err(format!("Spotlight search failed: {}", stderr.trim()));
    }
    Ok(())
}
