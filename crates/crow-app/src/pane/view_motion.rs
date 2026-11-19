//! Short, viewport-local transitions for explicit view changes and new columns.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crow_core::ViewMode;
use crow_core::motion::{self, Effective};
use gpui::{
    AnyElement, App, Bounds, Corners, Element, ElementId, EntityId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Point, Rgba, Styled, Window, canvas, fill, point, px, size,
};

use super::{
    CELL_PADDING, INDENT, IslandEdges, LIST_TOP_INSET, NAME_INSET, Pane, ROW_HEIGHT, TRIANGLE_WIDTH, shown_offset,
};

pub(super) struct ViewMotion {
    started: Instant,
    mode: Effective,
    pub folder: EntityId,
    origins: HashMap<PathBuf, RowOrigin>,
}

#[derive(Clone, Copy)]
struct RowOrigin {
    icon_x: Pixels,
    label_x: Pixels,
    row: Bounds<Pixels>,
    selected: bool,
}

impl RowOrigin {
    fn interpolate(self, to: Self, t: f32) -> Self {
        Self {
            icon_x: lerp(self.icon_x, to.icon_x, t),
            label_x: lerp(self.label_x, to.label_x, t),
            row: interpolate_bounds(self.row, to.row, t),
            selected: to.selected,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct MovingRow {
    origin: RowOrigin,
    t: f32,
}

impl ViewMotion {
    pub fn capture(pane: &Pane, mode: Effective, now: Instant, cx: &App) -> Option<Self> {
        if mode == Effective::Off || pane.renaming.is_some() || cx.has_active_drag() {
            return None;
        }
        let folder = pane.focused_folder();
        let origins = if mode == Effective::Full { Self::visible_origins(pane, now, cx) } else { HashMap::new() };
        Some(Self { started: now, mode, folder: folder.entity_id(), origins })
    }

    pub fn progress(&self, mode: Effective, now: Instant) -> Option<f32> {
        if self.mode != mode {
            return None;
        }
        motion::navigation_progress(mode, self.started, now, motion::ms::VIEW_CHANGE)
    }

    pub fn row(&self, folder: EntityId, path: &Path, t: f32) -> Option<MovingRow> {
        if self.folder != folder {
            return None;
        }
        Some(MovingRow { origin: *self.origins.get(path)?, t })
    }

    #[cfg(test)]
    pub fn captured_row(&self, path: &Path) -> Option<Bounds<Pixels>> {
        self.origins.get(path).map(|origin| origin.row)
    }

    fn visible_origins(pane: &Pane, now: Instant, cx: &App) -> HashMap<PathBuf, RowOrigin> {
        let folder = pane.focused_folder();
        // Several keystrokes can land before the new layout has painted.
        if pane.presented_layout != Some((pane.mode, folder.entity_id())) {
            return pane.view_motion.as_ref().map_or_else(HashMap::new, |motion| motion.origins.clone());
        }
        let (area, row_inset, top_inset) = match pane.mode {
            ViewMode::List => (pane.list_bounds.get(), 8.0, LIST_TOP_INSET),
            ViewMode::Columns => (pane.column_bounds.borrow().get(&pane.focused).copied(), 4.0, 0.0),
        };
        let Some(area) = area else { return HashMap::new() };
        let columns = pane.mode == ViewMode::Columns;
        let strip = pane.strip_scroll.bounds();
        let base_x = area.left() + px(row_inset);
        let width = (area.size.width - px(2.0 * row_inset)).max(px(0.));
        let folder_id = folder.entity_id();
        let folder = folder.read(cx);
        let offset = shown_offset(folder.scroll());
        let first = ((-offset - top_inset) / ROW_HEIGHT).floor().max(0.0) as usize;
        let last = ((f32::from(area.size.height) - offset - top_inset) / ROW_HEIGHT).ceil().max(0.0) as usize;
        let in_flight =
            pane.view_motion.as_ref().and_then(|motion| motion.progress(Effective::Full, now).map(|t| (motion, t)));
        (first..last.min(folder.row_count()))
            .filter_map(|ix| {
                let (entry, depth) = folder.row(ix)?;
                let icon_x = base_x
                    + px(if columns {
                        CELL_PADDING
                    } else {
                        NAME_INSET + depth as f32 * INDENT + TRIANGLE_WIDTH + 8.0
                    });
                if columns && (icon_x + px(16.) <= strip.left() || icon_x >= strip.right()) {
                    return None;
                }
                let top = area.top()
                    + px(top_inset
                        + offset
                        + ix as f32 * ROW_HEIGHT
                        + folder.row_shift(ix, Effective::Full, now, ROW_HEIGHT));
                let target = RowOrigin {
                    icon_x,
                    label_x: icon_x + px(24.0),
                    row: Bounds::new(point(base_x, top), size(width, px(ROW_HEIGHT))),
                    selected: folder.is_selected(&entry.path),
                };
                let painted = in_flight
                    .and_then(|(motion, t)| motion.row(folder_id, &entry.path, t))
                    .map_or(target, |row| row.origin.interpolate(target, row.t));
                Some((entry.path.clone(), painted))
            })
            .collect()
    }
}

impl MovingRow {
    pub fn icon(self, visual: impl IntoElement) -> AnyElement {
        self.track(visual, self.origin.icon_x)
    }

    pub fn label(self, visual: AnyElement) -> AnyElement {
        self.track(visual, self.origin.label_x)
    }

    fn track(self, visual: impl IntoElement, x: Pixels) -> AnyElement {
        ShiftedVisual { child: visual.into_any_element(), center: point(x, self.origin.row.center().y), t: self.t }
            .into_any_element()
    }

    pub fn tracks_selection(self) -> bool {
        self.origin.selected
    }

    pub fn selection(self, color: Rgba, island: Option<IslandEdges>) -> impl IntoElement {
        let rounded = |edge| if edge { px(6.) } else { px(0.) };
        let top = rounded(island.is_some_and(|edge| edge.top));
        let bottom = rounded(island.is_some_and(|edge| edge.bottom));
        canvas(
            |_, _, _| (),
            move |bounds, (), window, _| {
                let bounds = interpolate_bounds(self.origin.row, bounds, self.t);
                let mut highlight = fill(bounds, color);
                highlight.corner_radii =
                    Corners { top_left: top, top_right: top, bottom_right: bottom, bottom_left: bottom };
                window.paint_quad(highlight);
            },
        )
        .absolute()
        .inset_0()
    }
}

fn lerp(from: Pixels, to: Pixels, t: f32) -> Pixels {
    from + (to - from) * t
}

fn interpolate_bounds(from: Bounds<Pixels>, to: Bounds<Pixels>, t: f32) -> Bounds<Pixels> {
    Bounds::new(
        point(lerp(from.left(), to.left(), t), lerp(from.top(), to.top(), t)),
        size(lerp(from.size.width, to.size.width, t), lerp(from.size.height, to.size.height, t)),
    )
}

/// Only the artwork moves. The row retains its final layout, hitbox and accessibility bounds.
struct ShiftedVisual {
    child: AnyElement,
    center: Point<Pixels>,
    t: f32,
}

impl Element for ShiftedVisual {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let anchor = point(bounds.left(), bounds.center().y);
        let delta = (self.center - anchor) * (1.0 - self.t);
        window.with_element_offset(delta, |window| self.child.prepaint(window, cx));
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}
