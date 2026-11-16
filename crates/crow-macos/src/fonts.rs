//! Installed font families, for the settings window's font list.

use objc2_core_foundation::CFString;
use objc2_core_text::CTFontManagerCopyAvailableFontFamilyNames;

/// Every installed font family, sorted case-insensitively. CoreText answers off
/// the main thread, so the settings window enumerates these in the background
/// instead of blocking its first frame (AppKit's `NSFontManager` paid ~100 ms).
pub fn available_font_families() -> Vec<String> {
    // Safety: the function copies the array; the strings are valid for its
    // lifetime, which outlives this loop.
    let families = unsafe { CTFontManagerCopyAvailableFontFamilyNames() };
    let mut names = Vec::with_capacity(families.count() as usize);
    for index in 0..families.count() {
        // Safety: `index` is in range and the array holds CFStrings.
        let value = unsafe { families.value_at_index(index) };
        if value.is_null() {
            continue;
        }
        let name = unsafe { &*(value as *const CFString) };
        names.push(name.to_string());
    }
    names.sort_by_key(|name| name.to_lowercase());
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::available_font_families;

    /// H4 #20: the settings window enumerates fonts in the background. AppKit's
    /// `NSFontManager` answered only on the main thread (empty elsewhere); the
    /// CoreText call must answer on a worker thread.
    #[test]
    fn font_families_answer_off_the_main_thread() {
        let names = std::thread::spawn(available_font_families).join().expect("the font thread");
        assert!(!names.is_empty(), "CoreText lists the installed families off the main thread");
    }
}
