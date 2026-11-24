//! Drag-to-select over empty space in the row views (List, Columns, Gallery's
//! rail). The Icons grid has its own geometry in `pane.rs`; the rest share this:
//! rows are uniform, so the rectangle's vertical span names the rows it touches.

use gpui::{Bounds, Pixels, Point, UniformListScrollHandle, point, px, size};

use super::{LIST_SIDE_INSET, LIST_TOP_INSET, Pane, ROW_HEIGHT, ViewMode, gallery, shown_offset};

/// The row metrics of one row view's list.
#[derive(Clone, Copy)]
pub(super) struct RowSpace {
    pub row_height: f32,
    /// Space above the first row inside the list's bounds.
    pub top_inset: f32,
    /// Space left and right of the rows inside the list's bounds.
    pub side_inset: f32,
}

/// A row view's list as laid out now: its bounds, scroll state and metrics.
pub(super) struct RowStrip {
    pub bounds: Bounds<Pixels>,
    /// The scroll offset (zero or negative), clamped like the list draws it.
    pub offset: f32,
    pub space: RowSpace,
    pub scroll: UniformListScrollHandle,
}

impl RowStrip {
    /// `pointer` in content coordinates: x from the bounds' left, y from the top of
    /// the content (before the top inset), whatever the scroll offset.
    pub fn content_point(&self, pointer: Point<Pixels>) -> Point<Pixels> {
        point(pointer.x - self.bounds.left(), pointer.y - self.bounds.top() - px(self.offset))
    }

    /// The row indices a content-space rectangle touches, for `count` rows.
    pub fn rows_touched(&self, a: Point<Pixels>, b: Point<Pixels>, count: usize) -> std::ops::Range<usize> {
        let (x0, x1) = (f32::from(a.x.min(b.x)), f32::from(a.x.max(b.x)));
        let (y0, y1) = (f32::from(a.y.min(b.y)) - self.space.top_inset, f32::from(a.y.max(b.y)) - self.space.top_inset);
        let width = f32::from(self.bounds.size.width);
        if count == 0 || y1 < 0.0 || x1 < self.space.side_inset || x0 > width - self.space.side_inset {
            return 0..0;
        }
        let first = (y0.max(0.0) / self.space.row_height).floor() as usize;
        let last = ((y1 / self.space.row_height).floor() as usize).min(count - 1);
        if first > last { 0..0 } else { first..last + 1 }
    }

    /// The content height of `count` rows, and the viewport's height.
    pub fn heights(&self, count: usize) -> (f32, f32) {
        (self.space.top_inset + count as f32 * self.space.row_height, f32::from(self.bounds.size.height))
    }
}

impl Pane {
    /// Folder `k`'s list in the current row view, or `None` in Icons.
    pub(super) fn row_strip(&self, k: usize, cx: &gpui::App) -> Option<RowStrip> {
        let (bounds, scroll, space) = match self.mode {
            ViewMode::List => (
                self.list_bounds.get()?,
                self.folders.first()?.read(cx).scroll().clone(),
                RowSpace { row_height: ROW_HEIGHT, top_inset: LIST_TOP_INSET, side_inset: LIST_SIDE_INSET },
            ),
            ViewMode::Columns => (
                *self.column_bounds.borrow().get(&k)?,
                self.folders.get(k)?.read(cx).scroll().clone(),
                RowSpace { row_height: ROW_HEIGHT, top_inset: 0.0, side_inset: 4.0 },
            ),
            ViewMode::Gallery => (
                self.gallery.bounds.get()?,
                self.gallery.scroll.clone(),
                RowSpace { row_height: gallery::ROW_HEIGHT, top_inset: 0.0, side_inset: LIST_SIDE_INSET },
            ),
            ViewMode::Icons => return None,
        };
        let offset = shown_offset(&scroll);
        Some(RowStrip { bounds, offset, space, scroll })
    }

    /// The live rectangle of a row-view drag, in window coordinates and clipped to
    /// the list, for the overlay.
    pub(super) fn row_marquee_rect(&self, cx: &gpui::App) -> Option<Bounds<Pixels>> {
        let marquee = self.marquee.as_ref().filter(|marquee| marquee.started && marquee.rows)?;
        let strip = self.row_strip(marquee.folder, cx)?;
        let bounds = strip.bounds;
        let anchor = point(bounds.left() + marquee.anchor.x, bounds.top() + marquee.anchor.y + px(strip.offset));
        let pointer = marquee.pointer;
        let left = anchor.x.min(pointer.x).max(bounds.left());
        let top = anchor.y.min(pointer.y).max(bounds.top());
        // A straight drag is a rectangle one pixel wide, like the grid's.
        let right = anchor.x.max(pointer.x).max(left + px(1.)).min(bounds.right());
        let bottom = anchor.y.max(pointer.y).max(top + px(1.)).min(bounds.bottom());
        (right > left && bottom > top).then(|| Bounds::new(point(left, top), size(right - left, bottom - top)))
    }
}
