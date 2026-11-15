//! The theme list shared by the ⌥⌘T picker and the settings window: "System"
//! first, then the index's themes dark before light. Moving the cursor previews
//! a theme in every window; the caller decides when to keep it.

use crow_theme::{ThemeInfo, Variant};
use gpui::{App, AppContext, Task};

use crate::ui::{self, Ui};

pub struct ThemeChoice {
    /// `None` is "System".
    pub entries: Vec<Option<ThemeInfo>>,
    pub cursor: usize,
}

impl ThemeChoice {
    pub fn new(infos: Vec<ThemeInfo>, current: &str) -> Self {
        let mut infos = infos;
        // Grouped like the theme index's own split: dark themes, then light.
        infos.sort_by_key(|info| (info.variant == Variant::Light, info.name.to_lowercase()));
        let mut entries: Vec<Option<ThemeInfo>> = vec![None];
        entries.extend(infos.into_iter().map(Some));
        let cursor = entries.iter().position(|entry| entry_id(entry) == current).unwrap_or(0);
        Self { entries, cursor }
    }

    pub fn id(&self, index: usize) -> String {
        self.entries.get(index).map_or_else(|| "system".into(), entry_id)
    }

    pub fn current_id(&self) -> String {
        self.id(self.cursor)
    }

    /// Move the cursor by `delta`, clamped to the list. True if it moved.
    pub fn move_cursor(&mut self, delta: isize) -> bool {
        let last = self.entries.len().saturating_sub(1);
        let next = self.cursor.saturating_add_signed(delta).min(last);
        if next == self.cursor {
            return false;
        }
        self.cursor = next;
        true
    }

    /// Select `index`. True if it changed.
    pub fn select(&mut self, index: usize) -> bool {
        if index >= self.entries.len() || index == self.cursor {
            return false;
        }
        self.cursor = index;
        true
    }

    /// Put the cursor on `current` without changing the list.
    pub fn sync_to(&mut self, current: &str) {
        if let Some(index) = self.entries.iter().position(|entry| entry_id(entry) == current) {
            self.cursor = index;
        }
    }

    /// The display name of entry `index`.
    pub fn label(&self, index: usize) -> String {
        self.entries.get(index).map_or_else(
            || "System (follows macOS)".into(),
            |entry| entry.as_ref().map_or_else(|| "System (follows macOS)".into(), |info| info.name.clone()),
        )
    }
}

fn entry_id(entry: &Option<ThemeInfo>) -> String {
    entry.as_ref().map_or_else(|| "system".into(), |info| info.id.clone())
}

/// List the index off the main thread.
pub fn load(cx: &mut App) -> Task<Vec<ThemeInfo>> {
    let dir = crow_theme::themes_dir();
    cx.background_spawn(async move { crow_theme::list_theme_infos(&dir) })
}

/// Show `id` in every window without keeping it.
pub fn preview(id: &str, cx: &mut App) {
    cx.global_mut::<Ui>().preview_theme = Some(id.to_owned());
    ui::reload_theme(cx);
}

/// Drop the preview and show the saved theme again.
pub fn clear_preview(cx: &mut App) {
    cx.global_mut::<Ui>().preview_theme = None;
    ui::reload_theme(cx);
}
