//! The icon grid's geometry: one source for rendering, hit tests, keys and
//! scroll anchors. Pure arithmetic, so every edge can be unit-tested without a
//! window.

use serde::Deserialize;

/// The three icon sizes. Each fixes its tile and thumbnail box; the name keeps
/// one or two lines inside it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IconSize {
    Compact,
    #[default]
    Standard,
    Large,
}

impl IconSize {
    pub const ALL: &'static [IconSize] = &[IconSize::Compact, IconSize::Standard, IconSize::Large];

    pub fn key(self) -> &'static str {
        match self {
            IconSize::Compact => "compact",
            IconSize::Standard => "standard",
            IconSize::Large => "large",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            IconSize::Compact => "Compact",
            IconSize::Standard => "Standard",
            IconSize::Large => "Large",
        }
    }

    pub fn metrics(self) -> GridMetrics {
        match self {
            IconSize::Compact => GridMetrics {
                tile_width: 80.0,
                tile_height: 84.0,
                icon_size: 40.0,
                gap: 6.0,
                inset_x: 8.0,
                inset_y: 0.0,
            },
            IconSize::Standard => GridMetrics {
                tile_width: 100.0,
                tile_height: 100.0,
                icon_size: 56.0,
                gap: 6.0,
                inset_x: 8.0,
                inset_y: 0.0,
            },
            IconSize::Large => GridMetrics {
                tile_width: 144.0,
                tile_height: 136.0,
                icon_size: 92.0,
                gap: 8.0,
                inset_x: 8.0,
                inset_y: 0.0,
            },
        }
    }
}

/// One size's fixed tile, gap and outer inset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridMetrics {
    pub tile_width: f32,
    pub tile_height: f32,
    pub icon_size: f32,
    pub gap: f32,
    pub inset_x: f32,
    pub inset_y: f32,
}

impl GridMetrics {
    pub fn column_pitch(&self) -> f32 {
        self.tile_width + self.gap
    }

    pub fn row_pitch(&self) -> f32 {
        self.tile_height + self.gap
    }
}

/// A rectangle in content coordinates: `y` includes the scroll offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl GridRect {
    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    pub fn intersects(&self, other: &GridRect) -> bool {
        self.x < other.right() && other.x < self.right() && self.y < other.bottom() && other.y < self.bottom()
    }
}

/// Row-major tiles in a viewport. `columns` is clamped to at least one, so a
/// narrow pane still shows one full tile; the caller may center or clip it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridGeometry {
    count: usize,
    columns: usize,
    rows: usize,
    width: f32,
    viewport_height: f32,
    metrics: GridMetrics,
}

impl GridGeometry {
    pub fn new(count: usize, width: f32, viewport_height: f32, mut metrics: GridMetrics) -> Self {
        let usable = (width - 2.0 * metrics.inset_x).max(0.0);
        let columns = (((usable + metrics.gap) / metrics.column_pitch()).floor() as usize).max(1);
        let occupied = columns as f32 * metrics.column_pitch() - metrics.gap;
        // Center the column tracks, including when the final row is incomplete.
        metrics.inset_x = ((width - occupied) / 2.0).floor().max(0.0);
        let rows = count.div_ceil(columns);
        Self { count, columns, rows, width, viewport_height, metrics }
    }

    pub fn count(&self) -> usize {
        self.count
    }

    pub fn columns(&self) -> usize {
        self.columns
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn width(&self) -> f32 {
        self.width
    }

    pub fn viewport_height(&self) -> f32 {
        self.viewport_height
    }

    pub fn with_viewport_height(mut self, height: f32) -> Self {
        self.viewport_height = height;
        self
    }

    pub fn metrics(&self) -> GridMetrics {
        self.metrics
    }

    pub fn row_pitch(&self) -> f32 {
        self.metrics.row_pitch()
    }

    pub fn tile_height(&self) -> f32 {
        self.metrics.tile_height
    }

    /// The row a tile lives in, for `uniform_list` scrolling.
    pub fn row_of(&self, index: usize) -> usize {
        index / self.columns
    }

    /// The first tile index in `row`.
    pub fn first_index_of_row(&self, row: usize) -> usize {
        row.saturating_mul(self.columns)
    }

    /// The last tile index in `row`, clamped to the listing.
    pub fn last_index_of_row(&self, row: usize) -> Option<usize> {
        if row >= self.rows {
            return None;
        }
        Some((self.first_index_of_row(row) + self.columns - 1).min(self.count.saturating_sub(1)))
    }

    /// A tile's rectangle in content coordinates. `None` past the listing.
    pub fn tile_rect(&self, index: usize) -> Option<GridRect> {
        if index >= self.count {
            return None;
        }
        let (row, column) = (self.row_of(index), index % self.columns);
        Some(GridRect {
            x: self.metrics.inset_x + column as f32 * self.metrics.column_pitch(),
            y: self.metrics.inset_y + row as f32 * self.metrics.row_pitch(),
            width: self.metrics.tile_width,
            height: self.metrics.tile_height,
        })
    }

    /// The full row's rectangle, including the gutters between and beside tiles.
    /// Empty-space marquee starts here, not on a tile.
    pub fn row_rect(&self, row: usize) -> Option<GridRect> {
        (row < self.rows).then(|| GridRect {
            x: 0.0,
            y: self.metrics.inset_y + row as f32 * self.metrics.row_pitch(),
            width: self.width,
            height: self.metrics.tile_height,
        })
    }

    pub fn tile_at(&self, x: f32, y: f32) -> Option<usize> {
        if x < self.metrics.inset_x || y < self.metrics.inset_y {
            return None;
        }
        let column = ((x - self.metrics.inset_x) / self.metrics.column_pitch()).floor();
        let row = ((y - self.metrics.inset_y) / self.metrics.row_pitch()).floor();
        if column < 0.0 || row < 0.0 || column >= self.columns as f32 {
            return None;
        }
        let index = row as usize * self.columns + column as usize;
        self.tile_rect(index).filter(|rect| rect.contains(x, y)).map(|_| index)
    }

    /// The first row showing at `scroll_offset` (positive when scrolled down).
    pub fn first_visible_row(&self, scroll_offset: f32) -> usize {
        let y = scroll_offset - self.metrics.inset_y;
        if y <= 0.0 {
            return 0;
        }
        ((y / self.metrics.row_pitch()).floor() as usize).min(self.rows.saturating_sub(1))
    }

    /// The rows intersecting the viewport at `scroll_offset`.
    pub fn visible_rows(&self, scroll_offset: f32) -> std::ops::Range<usize> {
        if self.rows == 0 {
            return 0..0;
        }
        let first = self.first_visible_row(scroll_offset);
        let end_y = scroll_offset + self.viewport_height - self.metrics.inset_y;
        let last =
            if end_y <= 0.0 { first } else { ((end_y / self.metrics.row_pitch()).ceil() as usize).min(self.rows) };
        first..last.max(first + 1)
    }

    /// The content-space top of a row.
    pub fn row_top(&self, row: usize) -> f32 {
        self.metrics.inset_y + row as f32 * self.metrics.row_pitch()
    }

    /// The scroll offset that puts `row` at the viewport's top, clamped so the
    /// last screen stays full when the listing is shorter.
    pub fn offset_for_row(&self, row: usize) -> f32 {
        let max = (self.content_height() - self.viewport_height).max(0.0);
        self.row_top(row).min(max)
    }

    pub fn content_height(&self) -> f32 {
        2.0 * self.metrics.inset_y + self.rows as f32 * self.metrics.row_pitch()
    }

    /// Every tile intersecting `rect` (content coordinates), in row-major order.
    /// Tiles are visited through the rows and columns the rectangle covers, not
    /// by scanning the whole listing.
    pub fn tiles_intersecting(&self, rect: &GridRect, mut hit: impl FnMut(usize, GridRect)) {
        if self.count == 0 || rect.width <= 0.0 || rect.height <= 0.0 {
            return;
        }
        let row = ((rect.y - self.metrics.inset_y) / self.metrics.row_pitch()).floor().max(0.0) as usize;
        let end_row = ((rect.bottom() - self.metrics.inset_y) / self.metrics.row_pitch()).ceil().max(0.0) as usize;
        let column = ((rect.x - self.metrics.inset_x) / self.metrics.column_pitch()).floor().max(0.0) as usize;
        let end_column = ((rect.right() - self.metrics.inset_x) / self.metrics.column_pitch()).ceil().max(0.0) as usize;
        for row in row..end_row.min(self.rows) {
            let first = self.first_index_of_row(row);
            let last = self.last_index_of_row(row).unwrap_or(first);
            for column in column..end_column.min(self.columns) {
                let index = first + column;
                if index > last {
                    break;
                }
                if let Some(tile) = self.tile_rect(index)
                    && tile.intersects(rect)
                {
                    hit(index, tile);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry(count: usize, width: f32) -> GridGeometry {
        GridGeometry::new(count, width, 400.0, IconSize::Standard.metrics())
    }

    #[test]
    fn columns_are_derived_from_width_and_clamped_to_one() {
        // Standard tiles are 100 wide with a 6 pt gap and minimum 8 pt insets.
        assert_eq!(geometry(8, 751.0).columns(), 6);
        let grid = geometry(8, 784.0);
        assert_eq!(grid.columns(), 7);
        let first = grid.tile_rect(0).unwrap();
        assert_eq!(first.x, grid.width() - grid.tile_rect(6).unwrap().right());
        assert_eq!(grid.tile_rect(7).unwrap().x, first.x, "the last row keeps the same tracks");
        assert_eq!(geometry(8, 400.0).columns(), 3);
        assert_eq!(geometry(8, 60.0).columns(), 1, "a narrow pane still shows one tile");
        assert_eq!(geometry(0, 600.0).rows(), 0);
    }

    #[test]
    fn index_maps_row_major_and_the_last_row_clamps() {
        let grid = geometry(8, 400.0);
        assert_eq!(grid.columns(), 3);
        assert_eq!(grid.rows(), 3);
        assert_eq!((grid.row_of(0), grid.row_of(2), grid.row_of(3)), (0, 0, 1));
        assert_eq!(grid.first_index_of_row(2), 6);
        assert_eq!(grid.last_index_of_row(2), Some(7), "the 2-entry final row ends at index 7");
        assert_eq!(grid.last_index_of_row(3), None);
        assert_eq!(grid.tile_rect(8), None);
    }

    #[test]
    fn tile_and_point_hit_tests_stop_at_the_gutter() {
        let grid = geometry(8, 400.0);
        let tile = grid.tile_rect(1).unwrap();
        assert_eq!(grid.tile_at(tile.x + 1.0, tile.y + 1.0), Some(1));
        assert_eq!(grid.tile_at(tile.right() - 0.1, tile.bottom() - 0.1), Some(1));
        assert_eq!(grid.tile_at(tile.right() + 1.0, tile.y + 1.0), None, "the gap between tiles hits nothing");
        assert_eq!(grid.tile_at(tile.x + 1.0, tile.bottom() + 1.0), None, "the gap below hits nothing");
        assert_eq!(grid.tile_at(2.0, tile.y + 1.0), None, "the outer inset hits nothing");
        assert_eq!(grid.tile_at(tile.x + 1.0, grid.tile_rect(7).unwrap().y + 1.0), Some(7), "row-major");
    }

    #[test]
    fn visible_rows_follow_the_scroll_offset() {
        let grid = geometry(100, 400.0);
        let pitch = grid.row_pitch();
        assert_eq!(grid.first_visible_row(0.0), 0);
        assert_eq!(grid.first_visible_row(pitch * 2.5), 2);
        assert_eq!(grid.visible_rows(0.0).start, 0);
        assert!(grid.visible_rows(0.0).end >= 4, "400 high / 106 pitch covers four rows");
        assert_eq!(grid.offset_for_row(10), grid.row_top(10));
        assert_eq!(grid.offset_for_row(10_000), (grid.content_height() - 400.0).max(0.0), "clamped at the end");
    }

    #[test]
    fn intersecting_tiles_are_row_major_and_ignore_gutter_rectangles() {
        let grid = geometry(8, 400.0);
        let row = grid.row_rect(0).unwrap();
        let mut hits = Vec::new();
        // A rectangle across the first row's gutter top-to-bottom hits all three tiles.
        grid.tiles_intersecting(&row, |index, _| hits.push(index));
        assert_eq!(hits, vec![0, 1, 2]);
        // A rectangle only in the vertical gap between columns hits none.
        let gap = GridRect {
            x: grid.tile_rect(0).unwrap().right() + 1.0,
            y: row.y + 1.0,
            width: 4.0,
            height: row.height - 2.0,
        };
        hits.clear();
        grid.tiles_intersecting(&gap, |index, _| hits.push(index));
        assert!(hits.is_empty());
        // A rectangle over the final, incomplete row sees only its two tiles.
        let last = grid.row_rect(2).unwrap();
        hits.clear();
        grid.tiles_intersecting(&last, |index, _| hits.push(index));
        assert_eq!(hits, vec![6, 7]);
    }

    #[test]
    fn each_preset_has_fixed_tiles() {
        let compact = IconSize::Compact.metrics();
        let standard = IconSize::Standard.metrics();
        let large = IconSize::Large.metrics();
        assert!(compact.tile_width < standard.tile_width && standard.tile_width < large.tile_width);
        assert!(compact.icon_size < standard.icon_size && standard.icon_size < large.icon_size);
        assert_eq!(IconSize::default(), IconSize::Standard);
    }
}
