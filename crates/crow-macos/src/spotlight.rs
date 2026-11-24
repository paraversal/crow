//! Whole-disk name search through Spotlight's `mdfind`. It streams paths and
//! stats each result on the calling (background) thread; killing the child
//! stops the query. Spotlight only sees indexed volumes, which the UI says.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::ffi::OsStringExt;
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
    /// Finder tag values; every one must be present (Spotlight ANDs them).
    pub tags: Vec<String>,
    /// A name substring, for a tag query that also has text. `mdfind -name`
    /// cannot be combined with raw attributes, so the unified expression uses
    /// `kMDItemFSName` with the same case-insensitive substring semantics.
    pub name_contains: Option<String>,
}

/// Escape a `mdfind` quoted-literal value. `&`, `(` and `)` are literal inside
/// quotes; `*` and `?` are wildcards even there, and a quote or backslash needs
/// escaping. Verified with `mdfind` on macOS 27: `"star*tag"` also matched a
/// file tagged `starXXtag`, while `"star\*tag"` matched only the literal name.
pub fn escape_literal(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '*' => out.push_str("\\*"),
            '?' => out.push_str("\\?"),
            _ => out.push(ch),
        }
    }
    out
}

impl SpotlightPredicate {
    /// The raw `mdfind` query, or `None` when nothing narrows the scan.
    pub fn expression(&self) -> Option<String> {
        let mut conditions: Vec<String> = Vec::new();
        if let Some(content_type) = self.content_type {
            conditions.push(format!("kMDItemContentTypeTree == \"{}\"", escape_literal(content_type)));
        }
        if let Some(extension) = &self.extension {
            conditions.push(format!("kMDItemFSName == \"*.{}\"cd", escape_literal(extension)));
        }
        if let Some(name) = &self.name_contains
            && !name.is_empty()
        {
            conditions.push(format!("kMDItemFSName == \"*{}*\"cd", escape_literal(name)));
        }
        for tag in &self.tags {
            conditions.push(format!("kMDItemUserTags == \"{}\"", escape_literal(tag)));
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
    let mut child = Command::new("/usr/bin/mdfind")
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Spotlight search failed: {e}"))?;
    let stdout = child.stdout.take().ok_or_else(|| "Spotlight search failed: no output".to_owned())?;
    let mut batch = Vec::with_capacity(BATCH);
    let mut reader = BufReader::new(stdout);
    let mut line = Vec::new();
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(());
        }
        // Raw bytes: one path that is not UTF-8 must not end the whole search.
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        if line.is_empty() {
            continue;
        }
        let path = PathBuf::from(OsString::from_vec(line.clone()));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_and_name_terms_are_escaped_literals() {
        let predicate = SpotlightPredicate {
            tags: vec!["say \"hi\"".into(), "a\\b".into(), "star*tag".into(), "q?tag".into()],
            name_contains: Some("wo*rk?".into()),
            ..Default::default()
        };
        assert_eq!(
            predicate.expression().as_deref(),
            Some(
                r#"kMDItemFSName == "*wo\*rk\?*"cd && kMDItemUserTags == "say \"hi\"" && kMDItemUserTags == "a\\b" && kMDItemUserTags == "star\*tag" && kMDItemUserTags == "q\?tag""#
            )
        );
        assert_eq!(SpotlightPredicate::default().expression(), None);
    }

    #[test]
    fn the_extension_is_an_escaped_literal() {
        let predicate = SpotlightPredicate { extension: Some("t\"x*".into()), ..Default::default() };
        assert_eq!(predicate.expression().as_deref(), Some(r#"kMDItemFSName == "*.t\"x\*"cd"#));
    }

    #[test]
    fn a_tag_only_predicate_is_one_condition_per_value() {
        let predicate = SpotlightPredicate { tags: vec!["Work".into(), "review".into()], ..Default::default() };
        assert_eq!(
            predicate.expression().as_deref(),
            Some(r#"kMDItemUserTags == "Work" && kMDItemUserTags == "review""#)
        );
    }
}
