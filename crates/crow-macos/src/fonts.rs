//! Installed font families, for the settings window's font list.

use objc2::MainThreadMarker;
use objc2_app_kit::NSFontManager;

/// Every installed font family, sorted case-insensitively. Empty off the main
/// thread (AppKit only answers there); the settings window calls this once when
/// it opens, never on browser-window startup.
pub fn available_font_families() -> Vec<String> {
    let Some(mtm) = MainThreadMarker::new() else { return Vec::new() };
    let manager = NSFontManager::sharedFontManager(mtm);
    let families = manager.availableFontFamilies();
    let mut names: Vec<String> = families.iter().map(|name| name.to_string()).collect();
    names.sort_by_key(|name| name.to_lowercase());
    names.dedup();
    names
}
