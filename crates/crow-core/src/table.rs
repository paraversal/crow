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
    start_width: f32,
}

impl TableLayout {
    /// The first column is the name column. It follows the pane's width (`fitted`);
    /// the others keep the widths they were dragged to.
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

    /// The widths to draw in `available` points, as Finder does: Name takes what the
    /// other columns leave, down to its minimum; past that the others give way from the
    /// right, each down to its own minimum; past that whole columns are hidden from the
    /// right (width `0.0`) so no cell is ever cut off at the pane's edge. Name always shows.
    pub fn fitted(&self, available: f32) -> Vec<f32> {
        let mut widths: Vec<f32> = self.columns.iter().map(|c| c.width).collect();
        let Some(name) = self.columns.first() else { return widths };
        let mut deficit = name.min_width + widths[1..].iter().sum::<f32>() - available;
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
        widths[0] = (available - others).max(floor).floor();
        widths
    }

    /// Start dragging the divider after `column`. Name's width follows the pane, so its
    /// divider moves width between Name and the column after it.
    pub fn begin_resize(&self, column: usize, x: f32) -> Option<ResizeDrag> {
        let target = if column == 0 { 1 } else { column };
        let c = self.columns.get(column).and(self.columns.get(target))?;
        Some(ResizeDrag { column, start_x: x, start_width: c.width })
    }

    /// Follow the pointer during a divider drag. Widths never go below the minimum.
    pub fn drag_to(&mut self, drag: &ResizeDrag, x: f32) {
        // Name's divider: the column after Name grows as the divider moves left.
        let (target, delta) = if drag.column == 0 { (1, drag.start_x - x) } else { (drag.column, x - drag.start_x) };
        if let Some(c) = self.columns.get_mut(target) {
            c.width = (drag.start_width + delta).max(c.min_width).round();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> TableLayout {
        TableLayout::new(vec![
            ColumnWidth { id: "name".into(), width: 300.0, min_width: 120.0 },
            ColumnWidth { id: "size".into(), width: 80.0, min_width: 50.0 },
            ColumnWidth { id: "kind".into(), width: 150.0, min_width: 60.0 },
        ])
    }

    #[test]
    fn dragging_a_divider_resizes_only_that_column() {
        let mut t = layout();
        let drag = t.begin_resize(1, 500.0).unwrap();
        t.drag_to(&drag, 540.4);
        assert_eq!(t.width_of("size"), Some(120.0));
        assert_eq!(t.width_of("kind"), Some(150.0));
        t.drag_to(&drag, 490.0);
        assert_eq!(t.width_of("size"), Some(70.0), "the drag is relative to where it started");
    }

    #[test]
    fn names_divider_trades_with_the_next_column() {
        let mut t = layout();
        let drag = t.begin_resize(0, 300.0).unwrap();
        t.drag_to(&drag, 320.0);
        assert_eq!(t.width_of("size"), Some(60.0), "Name's divider moving right narrows Size");
        assert_eq!(t.fitted(600.0), vec![390.0, 60.0, 150.0], "so Name gains the same");
        t.drag_to(&drag, 280.0);
        assert_eq!(t.width_of("size"), Some(100.0));
    }

    #[test]
    fn name_follows_the_pane_width() {
        let t = layout();
        assert_eq!(t.fitted(800.0), vec![570.0, 80.0, 150.0], "wide: Name takes the room");
        assert_eq!(t.fitted(400.0), vec![170.0, 80.0, 150.0], "narrow: Name gives way first");
        assert_eq!(t.fitted(300.0), vec![120.0, 80.0, 100.0], "then the others, from the right");
        assert_eq!(t.fitted(240.0), vec![120.0, 60.0, 60.0], "never below their minimums");
        assert_eq!(t.fitted(200.0), vec![150.0, 50.0, 0.0], "then whole columns hide from the right");
        assert_eq!(t.fitted(100.0), vec![100.0, 0.0, 0.0], "Name always shows, fitted to the pane");
        assert_eq!(t.total_width(), 530.0, "fitting never changes the stored widths");
    }

    #[test]
    fn widths_stop_at_the_minimum() {
        let mut t = layout();
        let drag = t.begin_resize(1, 0.0).unwrap();
        t.drag_to(&drag, -500.0);
        assert_eq!(t.width_of("size"), Some(50.0));
        assert_eq!(t.total_width(), 300.0 + 50.0 + 150.0);
    }

    #[test]
    fn unknown_column_has_no_drag() {
        assert!(layout().begin_resize(9, 0.0).is_none());
    }
}
