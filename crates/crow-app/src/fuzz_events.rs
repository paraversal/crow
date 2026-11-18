//! Opt-in machine-readable events for `scripts/test-report.py`.
//!
//! Human output is unchanged. When `CROW_TEST_EVENTS=1`, each event is one
//! compact JSON object on its own line, prefixed `CROW_EVENT `. The headless
//! (`fuzz_long`) and live (`crow --fuzz`) runners emit the same field names
//! where their data overlaps, so one decoder reads both.
//!
//! A replay carries the command, only the intended `CROW_*` environment, and
//! the failed script. The collector writes the script beside the report and
//! rewrites the command's `--script` path to that relative artifact; it never
//! copies the environment.

use std::collections::BTreeMap;
use std::io::Write;
use std::time::Duration;

use serde_json::{Value, json};

const SCHEMA: u64 = 1;
const PREFIX: &str = "CROW_EVENT ";

#[cfg(test)]
pub const HEADLESS_FORMAT: &str = "rust";
#[cfg(feature = "fuzz")]
pub const LIVE_FORMAT: &str = "step-lines";

/// How to reproduce one seed or failure, with only the options that belong in
/// a report. `script` is filled in at a failure boundary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Replay {
    pub command: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub script: Option<String>,
    pub script_format: &'static str,
}

impl Replay {
    fn value(&self) -> Value {
        json!({
            "command": self.command,
            "env": self.env,
            "script": self.script,
            "script_format": self.script_format,
        })
    }
}

/// One live run's frame budget numbers. Null-free: a live report always has
/// the heartbeat. The collector still treats a missing `frame-summary` event
/// as unavailable rather than zero.
#[cfg(feature = "fuzz")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frames {
    pub heartbeats: u64,
    pub threshold_ms: f64,
    pub over_budget: usize,
    pub worst_frame_ms: f64,
    pub freeze_threshold_ms: f64,
    pub freeze: bool,
    pub slow_steps: usize,
    pub steps: usize,
    pub executed_steps: usize,
    pub duration: Duration,
}

fn enabled() -> bool {
    std::env::var("CROW_TEST_EVENTS").ok().as_deref() == Some("1")
}

fn emit(value: Value) {
    if !enabled() {
        return;
    }
    println!("{PREFIX}{value}");
    let _ = std::io::stdout().flush();
}

pub fn seed_start(mode: &str, seed: u64, tree: &str, churn: bool, requested_steps: Option<usize>) {
    emit(json!({
        "schema": SCHEMA,
        "event": "seed-start",
        "mode": mode,
        "seed": seed,
        "tree": tree,
        "churn": churn,
        "requested_steps": requested_steps,
    }));
}

pub fn seed_end(mode: &str, seed: u64, executed_steps: usize, duration: Duration, result: &str) {
    emit(json!({
        "schema": SCHEMA,
        "event": "seed-end",
        "mode": mode,
        "seed": seed,
        "executed_steps": executed_steps,
        "duration_ms": duration.as_secs_f64() * 1000.0,
        "result": result,
    }));
}

#[allow(clippy::too_many_arguments)] // one event; a builder struct would only rename these fields
pub fn seed_failure(
    mode: &str,
    seed: u64,
    failing_index: usize,
    executed_steps: usize,
    invariant: &str,
    detail: &str,
    duration: Duration,
    replay: &Replay,
) {
    emit(json!({
        "schema": SCHEMA,
        "event": "seed-failure",
        "mode": mode,
        "seed": seed,
        "failing_index": failing_index,
        "executed_steps": executed_steps,
        "invariant": invariant,
        "detail": detail,
        "duration_ms": duration.as_secs_f64() * 1000.0,
        "replay": replay.value(),
    }));
}

/// A failure boundary where no duration or step count is available (`HANG`).
#[cfg(feature = "fuzz")]
pub fn seed_failure_without_counts(
    mode: &str,
    seed: u64,
    failing_index: usize,
    invariant: &str,
    detail: &str,
    replay: &Replay,
) {
    emit(json!({
        "schema": SCHEMA,
        "event": "seed-failure",
        "mode": mode,
        "seed": seed,
        "failing_index": failing_index,
        "executed_steps": null,
        "invariant": invariant,
        "detail": detail,
        "duration_ms": null,
        "replay": replay.value(),
    }));
}

#[cfg(feature = "fuzz")]
pub fn frame_summary(seed: u64, frames: Frames) {
    emit(json!({
        "schema": SCHEMA,
        "event": "frame-summary",
        "mode": "fuzz-live",
        "seed": seed,
        "heartbeats": frames.heartbeats,
        "threshold_ms": frames.threshold_ms,
        "over_budget": frames.over_budget,
        "worst_frame_ms": frames.worst_frame_ms,
        "freeze_threshold_ms": frames.freeze_threshold_ms,
        "freeze": frames.freeze,
        "slow_steps": frames.slow_steps,
        "steps": frames.steps,
        "executed_steps": frames.executed_steps,
        "duration_ms": frames.duration.as_secs_f64() * 1000.0,
    }));
}
