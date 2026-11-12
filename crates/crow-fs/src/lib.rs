//! Folder snapshots, watching and, from M4, the file operation engine.
//!
//! No UI types here: everything is plain data so it can be tested without a window.

mod snapshot;
mod sort;
mod watch;

pub use snapshot::{Entry, Snapshot, make_fixture, read_dir};
pub use sort::natural_cmp;
pub use watch::{FolderWatcher, watch_folder};

/// Finder-style byte counts: decimal units, one decimal place.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return format!("{bytes} bytes");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    format!("{:.1} {}", value, UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(human_size(12), "12 bytes");
        assert_eq!(human_size(12_300), "12.3 KB");
        assert_eq!(human_size(4_200_000_000), "4.2 GB");
    }
}
