use std::path::PathBuf;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

/// `~/Library/Logs/Crow`, where Console.app looks for app logs.
pub fn log_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join("Library/Logs/Crow")
}

/// The non-blocking writer for `dir/crow.log`. An unwritable folder or file is
/// an `Err`, never a panic: Crow still runs with stderr logging alone.
fn open_log(dir: &std::path::Path) -> Result<(tracing_appender::non_blocking::NonBlocking, WorkerGuard), String> {
    // io-ok: startup: the log folder must exist before the subscriber starts
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let appender = tracing_appender::rolling::RollingFileAppender::builder()
        .rotation(tracing_appender::rolling::Rotation::NEVER)
        .filename_prefix("crow.log")
        .build(dir)
        .map_err(|e| e.to_string())?;
    Ok(tracing_appender::non_blocking(appender))
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
    match open_log(dir) {
        Ok((file, guard)) => {
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
    match open_log(&log_dir()) {
        Ok((file, guard)) => {
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

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    #[test]
    fn an_unwritable_log_folder_or_file_is_an_error_not_a_panic() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("logs");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        assert!(open_log(&dir).is_err(), "a read-only folder cannot hold crow.log");
        // A file where the folder should be.
        let file = temp.path().join("not-a-folder");
        std::fs::write(&file, "x").unwrap();
        assert!(open_log(&file).is_err());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(open_log(&dir).is_ok(), "a writable folder works");
    }
}
