//! The list decoration that publishes a bounded visible-row range. It runs in
//! GPUI's layout pass, reads the rows already in the folder entity, and only
//! queues demand; the coordinator submits it on the next 100 ms poll.

use crow_core::ExtensionId;
use gpui::{AnyElement, App, Bounds, Pixels, Point, UniformListDecoration, Window, div, prelude::*};

use super::{Plugins, RowInput, ScopeKey, SurfaceId};
use crate::folder::Folder;

pub struct PluginRows {
    folder: gpui::Entity<Folder>,
    surface: SurfaceId,
    scope: ScopeKey,
    descriptors: Vec<ExtensionId>,
}

impl PluginRows {
    pub fn new(
        folder: gpui::Entity<Folder>,
        surface: SurfaceId,
        scope: ScopeKey,
        descriptors: Vec<ExtensionId>,
    ) -> Self {
        Self { folder, surface, scope, descriptors }
    }
}

impl UniformListDecoration for PluginRows {
    fn compute(
        &self,
        visible_range: std::ops::Range<usize>,
        _bounds: Bounds<Pixels>,
        _scroll_offset: Point<Pixels>,
        _item_height: Pixels,
        _item_count: usize,
        _window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        if self.descriptors.is_empty() {
            return div().into_any_element();
        }
        let (scope_version, scope_display, rows) = {
            let folder = self.folder.read(cx);
            let rows = visible_range
                .filter_map(|ix| {
                    let (entry, _) = folder.row(ix)?;
                    Some(RowInput {
                        path: entry.path.clone(),
                        version: folder.plugin_entry_version(entry),
                        name: entry.name.clone(),
                        is_dir: entry.is_dir,
                        size: entry.size,
                        kind: crate::style::kind(entry).to_string(),
                        selected: folder.is_selected(&entry.path),
                        git: None,
                    })
                })
                .collect::<Vec<_>>();
            (folder.plugin_scope_version(), folder.dir().to_path_buf(), rows)
        };
        let dir = scope_display;
        let repository = crate::git::repository_data(cx, &dir);
        let want_lines = self
            .descriptors
            .iter()
            .any(|id| crate::git::is_git_plugin(&id.plugin) && id.local.as_str() == "lines-changed");
        let rows = rows
            .into_iter()
            .map(|mut row| {
                row.git = crate::git::entry_host(cx, &dir, &row.path, row.version, want_lines);
                row
            })
            .collect();
        if cx.try_global::<Plugins>().is_some() {
            cx.global_mut::<Plugins>().surfaces.queue_rows(
                self.surface,
                self.scope.clone(),
                scope_version,
                dir.display().to_string(),
                0,
                self.descriptors.clone(),
                rows,
                repository,
            );
        }
        div().into_any_element()
    }
}
