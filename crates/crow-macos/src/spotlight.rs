//! Whole-disk name search through Spotlight's `mdfind`. It streams paths and
//! stats each result on the calling (background) thread; killing the child
//! stops the query. Spotlight only sees indexed volumes, which the UI says.

use std::ffi::OsString;
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

/// A metadata predicate Spotlight can answer for an empty text query. Kind is
/// decided by the content type tree (or an exact name suffix); the age upper
/// bound excludes future modification dates, which the engine predicate also
/// rejects client-side.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpotlightPredicate {
    pub content_type: Option<&'static str>,
    pub extension: Option<String>,
    pub modified_within_secs: Option<u64>,
}

impl SpotlightPredicate {
    /// The raw `mdfind` query, or `None` when nothing narrows the scan.
    pub fn expression(&self) -> Option<String> {
        let mut conditions: Vec<String> = Vec::new();
        if let Some(content_type) = self.content_type {
            conditions.push(format!("kMDItemContentTypeTree == \"{content_type}\""));
        }
        if let Some(extension) = &self.extension {
            conditions.push(format!("kMDItemFSName == \"*.{extension}\"cd"));
        }
        if let Some(seconds) = self.modified_within_secs {
            conditions.push(format!(
                "kMDItemFSContentChangeDate >= $time.now(-{seconds}) && kMDItemFSContentChangeDate <= $time.now(0)"
            ));
        }
        (!conditions.is_empty()).then(|| conditions.join(" && "))
    }
}

/// Run `mdfind -name query`, sending hits in batches. Returns an error only when
/// Spotlight itself fails; no results is success.
pub fn spotlight_names(query: &str, sink: impl FnMut(Vec<SpotlightHit>), cancel: &AtomicBool) -> Result<(), String> {
    run_mdfind(vec![OsString::from("-name"), OsString::from(query)], sink, cancel)
}

/// Run a raw metadata-only `mdfind` expression (no `-name`), for an empty text
/// query with kind/age predicates.
pub fn spotlight_expression(
    expression: &str,
    sink: impl FnMut(Vec<SpotlightHit>),
    cancel: &AtomicBool,
) -> Result<(), String> {
    run_mdfind(vec![OsString::from(expression)], sink, cancel)
}

fn run_mdfind(args: Vec<OsString>, mut sink: impl FnMut(Vec<SpotlightHit>), cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(());
    }
    let mut child = Command::new("mdfind")
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Spotlight search failed: {e}"))?;
    let stdout = child.stdout.take().ok_or_else(|| "Spotlight search failed: no output".to_owned())?;
    let mut batch = Vec::with_capacity(BATCH);
    for line in BufReader::new(stdout).lines() {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
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
