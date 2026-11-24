//! Column widths for the list view's table: one width per column, shared by the
//! header and every row. Which columns exist is decided by the column registry
//! (and, from M5, `views.toml`); this only keeps their sizes consistent.

/// A column's width in points, and the smallest it may be dragged to.
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnWidth {
    pub id: String,
    pub width: f32,
    pub min_width: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TableLayout {
    columns: Vec<ColumnWidth>,
}

/// An in-progress drag of a header divider.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResizeDrag {
    pub column: usize,
    start_x: f32,
    /// The widths of the columns either side of the divider when the drag began,
    /// as drawn (Name's is the room it had, not its target).
    start_left: f32,
    start_right: f32,
    /// Name's target width when the drag began.
    start_name: f32,
}

impl TableLayout {
    /// The first column is Name. Its width is a target for fitting, while the
    /// other columns' widths are their preferred sizes.
    pub fn new(columns: Vec<ColumnWidth>) -> Self {
        Self { columns }
    }

    pub fn columns(&self) -> &[ColumnWidth] {
        &self.columns
    }

    pub fn width_of(&self, id: &str) -> Option<f32> {
        self.columns.iter().find(|c| c.id == id).map(|c| c.width)
    }

    /// Total width of all columns at their set widths.
    pub fn total_width(&self) -> f32 {
        self.columns.iter().map(|c| c.width).sum()
    }

    /// The widths to draw in `available` points. Name takes the remaining space
    /// and metadata gives way from the right to reach Name's target width. When
    /// space is tighter, Name can shrink to its minimum before whole columns
    /// hide from the right (width `0.0`).
    pub fn fitted(&self, available: f32) -> Vec<f32> {
        let mut widths: Vec<f32> = self.columns.iter().map(|c| c.width).collect();
        let Some(name) = self.columns.first() else { return widths };
        let metadata_minimum: f32 = self.columns[1..].iter().map(|c| c.min_width).sum();
        let name_target = (available - metadata_minimum).clamp(name.min_width, name.width.max(name.min_width));
        let mut deficit = name_target + widths[1..].iter().sum::<f32>() - available;
        for (width, column) in widths.iter_mut().zip(&self.columns).skip(1).rev() {
            if deficit <= 0.0 {
                break;
            }
            let give = (*width - column.min_width).max(0.0).min(deficit);
            *width -= give;
            deficit -= give;
        }
        // Still too wide at every minimum: hide columns from the right.
        for width in widths.iter_mut().skip(1).rev() {
            if deficit <= 0.0 {
                break;
            }
            deficit -= *width;
            *width = 0.0;
        }
        let others: f32 = widths[1..].iter().sum();
        // Alone, Name fits the pane exactly, even below its minimum: its text ellipsizes.
        let floor = if others > 0.0 { name.min_width } else { 0.0 };
        widths[0] = (available - others).max(floor);
        widths
    }

    /// Start dragging the divider after `column` in a table `available` points wide.
    /// The drag starts from the widths as drawn, so a column that gave way to a
    /// narrow pane does not jump back first.
    pub fn begin_resize(&mut self, column: usize, x: f32, available: f32) -> Option<ResizeDrag> {
        self.columns.get(column + 1)?;
        let fitted = self.fitted(available);
        for (c, width) in self.columns.iter_mut().zip(fitted).skip(1) {
            if width > 0.0 {
                c.width = width;
            }
        }
        let name_room = (available - metadata_width(&self.columns)).max(self.columns[0].min_width);
        Some(ResizeDrag {
            column,
            start_x: x,
            start_left: if column == 0 { name_room } else { self.columns[column].width },
            start_right: self.columns[column + 1].width,
            start_name: self.columns[0].width,
        })
    }

    /// Follow the pointer during a divider drag. The divider trades width between the
    /// columns either side of it, so it stays under the pointer and no other column
    /// moves; it stops where either side reaches its minimum. Name, on the left of
    /// the first divider, is never narrower than its minimum, and its target width
    /// follows it.
    pub fn drag_to(&mut self, drag: &ResizeDrag, x: f32) {
        let k = drag.column;
        let Some(right) = self.columns.get(k + 1) else { return };
        let left = &self.columns[k];
        let delta =
            (x - drag.start_x).round().max(left.min_width - drag.start_left).min(drag.start_right - right.min_width);
        let left_width = if k == 0 {
            // Name keeps its target unless the drag leaves it less room than that.
            drag.start_name.min(drag.start_left + delta)
        } else {
            drag.start_left + delta
        };
        let left_width = left_width.max(left.min_width);
        self.columns[k + 1].width = (drag.start_right - delta).max(right.min_width);
        self.columns[k].width = left_width;
    }
}

/// The widths of every column after Name.
fn metadata_width(columns: &[ColumnWidth]) -> f32 {
    columns[1..].iter().map(|c| c.width).sum()
}
#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> TableLayout {
        TableLayout::new(vec![
            ColumnWidth { id: "name".into(), width: 200.0, min_width: 120.0 },
            ColumnWidth { id: "size".into(), width: 80.0, min_width: 50.0 },
            ColumnWidth { id: "kind".into(), width: 150.0, min_width: 60.0 },
        ])
    }

    #[test]
    fn a_divider_trades_width_with_the_next_column() {
        let mut t = layout();
        let drag = t.begin_resize(1, 500.0, 800.0).unwrap();
        t.drag_to(&drag, 540.4);
        assert_eq!(t.fitted(800.0), vec![570.0, 120.0, 110.0], "Size grows, Kind gives the same, Name stays");
        t.drag_to(&drag, 490.0);
        assert_eq!(t.fitted(800.0), vec![570.0, 70.0, 160.0], "the drag is relative to where it started");
    }

    #[test]
    fn names_divider_trades_with_the_next_column() {
        let mut t = layout();
        let drag = t.begin_resize(0, 300.0, 600.0).unwrap();
        t.drag_to(&drag, 320.0);
        assert_eq!(t.fitted(600.0), vec![390.0, 60.0, 150.0], "Name's divider moving right narrows Size");
        t.drag_to(&drag, 280.0);
        assert_eq!(t.fitted(600.0), vec![350.0, 100.0, 150.0]);
        assert_eq!(t.width_of("name"), Some(200.0), "Name keeps its target while it has room for it");
    }

    #[test]
    fn a_drag_stops_where_either_side_reaches_its_minimum() {
        let mut t = layout();
        let drag = t.begin_resize(1, 0.0, 800.0).unwrap();
        t.drag_to(&drag, 1000.0);
        assert_eq!(t.fitted(800.0), vec![570.0, 170.0, 60.0], "Kind stops at its minimum");
        t.drag_to(&drag, -1000.0);
        assert_eq!(t.fitted(800.0), vec![570.0, 50.0, 180.0], "Size stops at its minimum");
        t.drag_to(&drag, 10.0);
        assert_eq!(t.fitted(800.0), vec![570.0, 90.0, 140.0], "and the divider is under the pointer again at once");

        let mut t = layout();
        let drag = t.begin_resize(0, 0.0, 430.0).unwrap();
        t.drag_to(&drag, -1000.0);
        assert_eq!(t.fitted(430.0), vec![120.0, 160.0, 150.0], "Name's divider stops at Name's minimum");
        assert_eq!(t.width_of("name"), Some(120.0), "and Name's target follows it down");
    }

    #[test]
    fn a_drag_starts_from_the_widths_as_drawn() {
        let mut t = layout();
        // At 300 points Size and Kind have given way to their minimums.
        assert_eq!(t.fitted(300.0), vec![190.0, 50.0, 60.0]);
        let drag = t.begin_resize(1, 0.0, 300.0).unwrap();
        t.drag_to(&drag, 0.0);
        assert_eq!(t.fitted(300.0), vec![190.0, 50.0, 60.0], "pressing a divider moves nothing");
    }

    #[test]
    fn name_follows_the_pane_width() {
        let t = layout();
        assert_eq!(t.fitted(800.0), vec![570.0, 80.0, 150.0], "wide: Name takes the room");
        assert_eq!(t.fitted(400.0), vec![200.0, 80.0, 120.0], "Kind gives way to preserve Name's target");
        assert_eq!(t.fitted(300.0), vec![190.0, 50.0, 60.0], "Name can shrink when metadata reaches its minimum");
        assert_eq!(t.fitted(230.0), vec![120.0, 50.0, 60.0], "at every minimum");
        assert_eq!(t.fitted(200.0), vec![150.0, 50.0, 0.0], "then whole columns hide from the right");
        assert_eq!(t.fitted(100.0), vec![100.0, 0.0, 0.0], "Name always shows, fitted to the pane");
        assert_eq!(t.total_width(), 430.0, "fitting never changes the stored widths");
    }

    #[test]
    fn fractional_divider_drags_preserve_column_minima() {
        for width in (0..20).map(|step| 230.6 + step as f32 * 0.17) {
            for column in 0..2 {
                let mut table = layout();
                let drag = table.begin_resize(column, 0., width).unwrap();
                for position in [-1000., -31.4, -0.61, 0., 0.63, 45.7, 1000.] {
                    table.drag_to(&drag, position);
                    for c in table.columns() {
                        assert!(c.width >= c.min_width, "{c:?} at width {width}, divider {column}, x {position}");
                    }
                    table.fitted(width);
                }
            }
        }
    }

    #[test]
    fn fitting_tolerates_a_saved_name_width_below_its_minimum() {
        let mut table = layout();
        table.columns[0].width = 119.4;
        assert_eq!(table.fitted(230.), [120., 50., 60.]);
    }

    #[test]
    fn the_last_column_has_no_divider() {
        assert!(layout().begin_resize(2, 0.0, 800.0).is_none());
        assert!(layout().begin_resize(9, 0.0, 800.0).is_none());
    }
}
