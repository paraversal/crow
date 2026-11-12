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
    /// The first column is the name column; the last one stretches to fill the
    /// pane when there is room (the view does that, its width here is a minimum).
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

    pub fn begin_resize(&self, column: usize, x: f32) -> Option<ResizeDrag> {
        let c = self.columns.get(column)?;
        Some(ResizeDrag { column, start_x: x, start_width: c.width })
    }

    /// Follow the pointer during a divider drag. Widths never go below the minimum.
    pub fn drag_to(&mut self, drag: &ResizeDrag, x: f32) {
        if let Some(c) = self.columns.get_mut(drag.column) {
            c.width = (drag.start_width + x - drag.start_x).max(c.min_width).round();
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
        let drag = t.begin_resize(0, 500.0).unwrap();
        t.drag_to(&drag, 540.4);
        assert_eq!(t.width_of("name"), Some(340.0));
        assert_eq!(t.width_of("size"), Some(80.0));
        t.drag_to(&drag, 490.0);
        assert_eq!(t.width_of("name"), Some(290.0), "the drag is relative to where it started");
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
