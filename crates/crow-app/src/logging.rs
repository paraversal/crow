use std::path::PathBuf;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

/// `~/Library/Logs/Crow`, where Console.app looks for app logs.
pub fn log_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join("Library/Logs/Crow")
}

/// Log to `~/Library/Logs/Crow/crow.log` and to stderr. `RUST_LOG` overrides the
/// default level (`info`). Keep the guard alive for the whole run so lines flush.
pub fn init() -> Option<WorkerGuard> {
    init_with("info")
}

/// The fuzz runner installs `trace` as an extra layer, so slow steps can print
/// the spans that were running; `RUST_LOG` still wins for the filter. The file
/// goes to the run's own folder, not the user's `~/Library/Logs/Crow`.
#[cfg(feature = "fuzz")]
pub fn init_fuzz(trace: crate::fuzz_live::TraceLayer, dir: &std::path::Path) -> Option<WorkerGuard> {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,crow=debug,crow_macos=debug"));
    let stderr = fmt::layer().with_writer(std::io::stderr).with_target(false);
    // io-ok: startup: the log folder must exist before the subscriber starts
    match std::fs::create_dir_all(dir) {
        Ok(()) => {
            let (file, guard) = tracing_appender::non_blocking(tracing_appender::rolling::never(dir, "crow.log"));
            let file = fmt::layer().with_writer(file).with_ansi(false);
            tracing_subscriber::registry().with(filter).with(stderr).with(file).with(trace).init();
            Some(guard)
        }
        Err(e) => {
            tracing_subscriber::registry().with(filter).with(stderr).with(trace).init();
            tracing::warn!("no log file: {e}");
            None
        }
    }
}

fn init_with(default: &str) -> Option<WorkerGuard> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default));
    let stderr = fmt::layer().with_writer(std::io::stderr).with_target(false);
    // io-ok: startup: the log folder must exist before the subscriber starts
    match std::fs::create_dir_all(log_dir()) {
        Ok(()) => {
            let (file, guard) = tracing_appender::non_blocking(tracing_appender::rolling::never(log_dir(), "crow.log"));
            let file = fmt::layer().with_writer(file).with_ansi(false);
            tracing_subscriber::registry().with(filter).with(stderr).with(file).init();
            Some(guard)
        }
        Err(e) => {
            tracing_subscriber::registry().with(filter).with(stderr).init();
            tracing::warn!("no log file: {e}");
            None
        }
    }
}
