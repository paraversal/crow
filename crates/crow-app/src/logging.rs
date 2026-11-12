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
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let stderr = fmt::layer().with_writer(std::io::stderr).with_target(false);
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
