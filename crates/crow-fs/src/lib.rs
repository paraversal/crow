//! Folder listings, sorting, watching and, from M4, the file operation engine.
//!
//! No UI types here: everything is plain data so it can be tested without a window.

mod listing;
pub mod ops;
mod snapshot;
mod sort;
mod watch;

pub use listing::{Entry, list_dir, stat_entry};
pub use snapshot::{KindOf, Snapshot, apply_changes, apply_stats, make_fixture, read_dir, stat_changes};
pub use sort::{Sort, SortKey, compare, natural_cmp, sort_entries};
pub use watch::{FolderChange, FolderWatcher, watch_folder};

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

/// Two byte counts in one unit, so "1.2 of 4.8 GB" lines up. The unit is the
/// total's; a done value under 1000 bytes stays in bytes ("512 bytes of 2.0 KB").
pub fn human_size_pair(done: u64, total: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    if done < 1000 {
        return format!("{done} bytes of {}", human_size(total));
    }
    let mut total_value = total as f64;
    let mut unit = 0;
    while total_value >= 1000.0 && unit < UNITS.len() - 1 {
        total_value /= 1000.0;
        unit += 1;
    }
    let scale = 1000f64.powi(unit as i32);
    format!("{:.1} of {:.1} {}", done as f64 / scale, total_value, UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(human_size(12), "12 bytes");
        assert_eq!(human_size(12_300), "12.3 KB");
        assert_eq!(human_size(4_200_000_000), "4.2 GB");
        assert_eq!(human_size_pair(1_200_000_000, 4_800_000_000), "1.2 of 4.8 GB");
        assert_eq!(human_size_pair(512, 2_000), "512 bytes of 2.0 KB");
        assert_eq!(human_size_pair(1_200, 4_800), "1.2 of 4.8 KB");
    }
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod ops_tests;
