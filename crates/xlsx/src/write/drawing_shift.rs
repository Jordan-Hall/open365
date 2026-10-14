//! What happens to an anchored drawing object when the cells beneath it change: rows or columns
//! inserted or deleted, cells shifted, rows or columns hidden or resized.
//!
//! The rules were measured in Excel with `tools/office/drawing-ops-oracle.ps1` (scenarios in
//! `fixtures/reference/xlsx/drawing-ops/ops.json`, checked by `tests/drawing_ops_oracle.rs`):
//!
//! - A marker (`from` or `to` of an anchor) is a cell plus an offset. Whole rows or columns inserted
//!   at or before a `from` marker push it along; a `to` marker at the very start of the inserted
//!   cell (offset zero) stays, so an object ending on a cell boundary does not grow.
//! - Deleted rows or columns: a marker inside them moves to the first row or column after the gap
//!   with offset zero; markers after the gap move back. A move-and-size object whose two markers meet
//!   on the edited axis is deleted.
//! - Shifting a part of a row or column (Insert/Delete Cells): a move-and-size object follows only if
//!   both markers lie in the shifted band, a move-only object if its `from` marker does; each marker
//!   then follows the whole-row rule.
//! - Hiding or resizing cells moves nothing in the file for a move-and-size object (its markers keep
//!   their cells); a move-only object keeps its size and an absolute one keeps position and size, so
//!   their markers are recomputed in the new geometry.

use crate::drawing::{Anchor, AnchorMarker, EMU_PER_POINT, EditAs, RectPt};
use crate::geometry::{GridMetrics, SheetGeometry};
use crate::refs::{MAX_COL, MAX_ROW};
use crate::sheet::Worksheet;

/// One insert or delete on an axis, restricted to a band of the other axis when cells (not whole
/// rows or columns) shift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AxisShift {
    /// True when rows move (the edit is along rows), false for columns.
    pub rows: bool,
    /// Columns (for a row shift) or rows (for a column shift) the shift covers, 1-based inclusive;
    /// `None` for whole rows or columns.
    pub band: Option<(u32, u32)>,
    /// First row or column affected, 1-based.
    pub at: u32,
    pub count: u32,
    pub delete: bool,
}

impl AxisShift {
    fn end(&self) -> u32 {
        self.at + self.count - 1
    }

    fn limit(&self) -> u32 {
        if self.rows { MAX_ROW } else { MAX_COL }
    }

    /// The marker's index along the edited axis (1-based) and its offset there.
    fn along(&self, m: &AnchorMarker) -> (u32, i64) {
        if self.rows {
            (m.row + 1, m.row_off)
        } else {
            (m.col + 1, m.col_off)
        }
    }

    fn in_band(&self, m: &AnchorMarker) -> bool {
        let perp = if self.rows { m.col + 1 } else { m.row + 1 };
        self.band.is_none_or(|(lo, hi)| perp >= lo && perp <= hi)
    }

    fn set_along(&self, m: &mut AnchorMarker, index1: u32, off: Option<i64>) {
        if self.rows {
            m.row = index1 - 1;
            if let Some(o) = off {
                m.row_off = o;
            }
        } else {
            m.col = index1 - 1;
            if let Some(o) = off {
                m.col_off = o;
            }
        }
    }

    /// Moves `m` for this shift. `is_to` selects the rule of an end marker.
    fn map_marker(&self, m: &AnchorMarker, is_to: bool) -> AnchorMarker {
        let mut out = *m;
        if !self.in_band(m) {
            return out;
        }
        let (idx, off) = self.along(m);
        if !self.delete {
            let pushed = idx > self.at || (idx == self.at && (!is_to || off > 0));
            if pushed {
                self.set_along(&mut out, (idx + self.count).min(self.limit()), None);
            }
        } else if idx >= self.at && idx <= self.end() {
            self.set_along(&mut out, self.at, Some(0));
        } else if idx > self.end() {
            self.set_along(&mut out, idx - self.count, None);
        }
        out
    }
}

/// An object's frame under one geometry, in points from the sheet origin.
pub(crate) struct Place {
    geo: SheetGeometry,
    metrics: GridMetrics,
}

impl Place {
    pub(crate) fn new(ws: &Worksheet, metrics: GridMetrics) -> Self {
        Self {
            geo: ws.geometry(metrics),
            metrics,
        }
    }

    fn col_left(&self, col0: u32) -> f64 {
        self.metrics.px_to_pt(self.geo.col_x(col0 + 1) as u32)
    }

    fn row_top(&self, row0: u32) -> f64 {
        self.metrics.px_to_pt(self.geo.row_y(row0 + 1) as u32)
    }

    fn col_width(&self, col0: u32) -> f64 {
        self.metrics.px_to_pt(self.geo.col_px(col0 + 1))
    }

    fn row_height(&self, row0: u32) -> f64 {
        self.metrics.px_to_pt(self.geo.row_px(row0 + 1))
    }

    pub(crate) fn x_of(&self, m: &AnchorMarker) -> f64 {
        self.col_left(m.col) + (m.col_off as f64 / EMU_PER_POINT).clamp(0.0, self.col_width(m.col))
    }

    pub(crate) fn y_of(&self, m: &AnchorMarker) -> f64 {
        self.row_top(m.row) + (m.row_off as f64 / EMU_PER_POINT).clamp(0.0, self.row_height(m.row))
    }

    /// The marker at the point `(x, y)` points from the sheet origin.
    pub(crate) fn marker_at(&self, x: f64, y: f64) -> AnchorMarker {
        let px = |pt: f64| (pt.max(0.0) * f64::from(self.metrics.dpi) / 72.0).floor() as u64;
        let col1 = self.geo.col_at(px(x), MAX_COL);
        let row1 = self.geo.row_at(px(y), MAX_ROW);
        let (col, row) = (col1 - 1, row1 - 1);
        AnchorMarker {
            col,
            col_off: ((x - self.col_left(col)).max(0.0) * EMU_PER_POINT).round() as i64,
            row,
            row_off: ((y - self.row_top(row)).max(0.0) * EMU_PER_POINT).round() as i64,
        }
    }

    pub(crate) fn rect(&self, anchor: &Anchor) -> RectPt {
        match *anchor {
            Anchor::TwoCell { from, to, .. } => {
                let (x, y) = (self.x_of(&from), self.y_of(&from));
                RectPt {
                    x,
                    y,
                    width: (self.x_of(&to) - x).max(0.0),
                    height: (self.y_of(&to) - y).max(0.0),
                }
            }
            Anchor::OneCell { from, cx, cy } => RectPt {
                x: self.x_of(&from),
                y: self.y_of(&from),
                width: cx as f64 / EMU_PER_POINT,
                height: cy as f64 / EMU_PER_POINT,
            },
            Anchor::Absolute { x, y, cx, cy } => RectPt {
                x: x as f64 / EMU_PER_POINT,
                y: y as f64 / EMU_PER_POINT,
                width: cx as f64 / EMU_PER_POINT,
                height: cy as f64 / EMU_PER_POINT,
            },
        }
    }
}

/// The anchor after the change, or `None` when Excel deletes the object. `before` is the object's
/// frame under the old geometry (`size` overrides its width and height when the file stores an exact
/// extent), `now` the geometry after the change. `shift` is `None` for hide and resize.
pub(crate) fn reflow(
    anchor: &Anchor,
    before: RectPt,
    size: Option<(f64, f64)>,
    now: &Place,
    shift: Option<&AxisShift>,
) -> Option<Anchor> {
    let (w, h) = size.unwrap_or((before.width, before.height));
    match *anchor {
        Anchor::TwoCell { from, to, edit_as } => {
            let (mut f, mut t) = (from, to);
            // Which markers must lie in the shifted band for the object to follow it.
            let follows = |s: &AxisShift| match edit_as {
                EditAs::TwoCell => s.in_band(&from) && s.in_band(&to),
                EditAs::OneCell => s.in_band(&from),
                EditAs::Absolute => false,
            };
            if let Some(s) = shift
                && follows(s)
            {
                f = s.map_marker(&from, false);
                t = s.map_marker(&to, true);
            }
            match edit_as {
                EditAs::TwoCell => {
                    if let Some(s) = shift
                        && s.delete
                        && s.along(&from) != s.along(&to)
                        && s.along(&f) == s.along(&t)
                    {
                        return None;
                    }
                    Some(Anchor::TwoCell {
                        from: f,
                        to: t,
                        edit_as,
                    })
                }
                EditAs::OneCell => {
                    let (x, y) = (now.x_of(&f), now.y_of(&f));
                    Some(Anchor::TwoCell {
                        from: f,
                        to: now.marker_at(x + w, y + h),
                        edit_as,
                    })
                }
                EditAs::Absolute => Some(Anchor::TwoCell {
                    from: now.marker_at(before.x, before.y),
                    to: now.marker_at(before.x + w, before.y + h),
                    edit_as,
                }),
            }
        }
        Anchor::OneCell { from, cx, cy } => {
            // A one-cell anchor follows its single marker.
            let f = match shift {
                Some(s) => s.map_marker(&from, false),
                None => from,
            };
            Some(Anchor::OneCell { from: f, cx, cy })
        }
        Anchor::Absolute { .. } => Some(*anchor),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker(col: u32, col_off: i64, row: u32, row_off: i64) -> AnchorMarker {
        AnchorMarker {
            col,
            col_off,
            row,
            row_off,
        }
    }

    fn rows(at: u32, count: u32, delete: bool) -> AxisShift {
        AxisShift {
            rows: true,
            band: None,
            at,
            count,
            delete,
        }
    }

    #[test]
    fn insert_pushes_from_but_not_an_end_marker_on_the_boundary() {
        let s = rows(20, 1, false);
        // Row 20 (1-based) is index 19: the object starting there moves down.
        assert_eq!(s.map_marker(&marker(2, 0, 19, 0), false).row, 20);
        // An end marker at the start of row 20 (offset 0) stays; with an offset it moves.
        assert_eq!(s.map_marker(&marker(2, 0, 19, 0), true).row, 19);
        assert_eq!(s.map_marker(&marker(2, 0, 19, 5), true).row, 20);
        // Rows above stay.
        assert_eq!(s.map_marker(&marker(2, 0, 18, 5), true).row, 18);
    }

    #[test]
    fn delete_collapses_markers_inside_to_the_gap() {
        let s = rows(4, 6, true);
        // from at row 5 -> row 4 offset 0 (index 3); a marker below shifts up.
        let f = s.map_marker(&marker(2, 4, 4, 38_100), false);
        assert_eq!((f.row, f.row_off), (3, 0));
        let t = s.map_marker(&marker(5, 1, 9, 12_700), true);
        assert_eq!((t.row, t.row_off), (3, 12_700));
    }
}
