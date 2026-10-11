//! Merged cells (`mergeCells`, ECMA-376 Part 1 §18.3.1.55): Excel's Merge Cells, Merge Across and
//! Unmerge Cells. Merging keeps the upper-left cell of every merged area; clearing the other cells
//! is the caller's job (it must recalculate), see [`Workbook::merge_discards`].

use super::Workbook;
use crate::XlsxError;
use crate::format::BorderEdge;
use crate::refs::{MAX_COL, MAX_ROW};
use crate::sheet::Rect;
use crate::styles_edit::{BorderPatch, FormatPatch};

/// Which Merge & Center menu action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeKind {
    /// Merge Cells (and Merge & Center, whose centring is a format change): one area.
    Merge,
    /// Merge Across: one area per row of the range.
    Across,
    /// Unmerge Cells: every merge that touches the range goes.
    Unmerge,
}

fn intersects(a: Rect, b: Rect) -> bool {
    a.0 <= b.2 && b.0 <= a.2 && a.1 <= b.3 && b.1 <= a.3
}

/// The areas `kind` merges inside `rect`; single-cell areas are left out. Empty for `Unmerge`.
pub fn merge_areas(rect: Rect, kind: MergeKind) -> Vec<Rect> {
    let (r1, c1, r2, c2) = rect;
    let areas: Vec<Rect> = match kind {
        MergeKind::Merge => vec![rect],
        MergeKind::Across => (r1..=r2).map(|r| (r, c1, r, c2)).collect(),
        MergeKind::Unmerge => Vec::new(),
    };
    areas
        .into_iter()
        .filter(|&(a, b, c, d)| (a, b) != (c, d))
        .collect()
}

fn check(rect: Rect) -> Result<(), XlsxError> {
    let (r1, c1, r2, c2) = rect;
    if r1 == 0 || c1 == 0 || r1 > r2 || c1 > c2 || r2 > MAX_ROW || c2 > MAX_COL {
        return Err(XlsxError::UnsupportedEdit(format!(
            "range ({r1}, {c1}) to ({r2}, {c2}) is outside the sheet"
        )));
    }
    Ok(())
}

/// Whether all `edges` are the same line (all absent counts).
fn uniform(edges: &[BorderEdge]) -> Option<BorderEdge> {
    let first = *edges.first()?;
    edges.iter().all(|e| *e == first).then_some(first)
}

impl Workbook {
    /// What Excel does to the styles of an area it merges (measured, `fixtures/reference/xlsx/
    /// autofit-merge-styles/merge.json`): every cell of the area takes the upper-left cell's style
    /// (fill, font, number format, alignment); the borders of the inside edges go; a border on the
    /// area's outer edge stays on the cells along that edge only when all of them draw the same
    /// line there, and is dropped otherwise (a mixed edge disappears, even on a single cell).
    fn restyle_covered(&mut self, sheet: usize, area: Rect) -> Result<(), XlsxError> {
        let (r1, c1, r2, c2) = area;
        let top_left = self.cell_xf(sheet, r1, c1);
        let edge = |wb: &Self, r: u32, c: u32| wb.cell_format(sheet, r, c).border;
        let (mut left, mut right, mut top, mut bottom) = (vec![], vec![], vec![], vec![]);
        for r in r1..=r2 {
            left.push(edge(self, r, c1).left);
            right.push(edge(self, r, c2).right);
        }
        for c in c1..=c2 {
            top.push(edge(self, r1, c).top);
            bottom.push(edge(self, r2, c).bottom);
        }
        let none = BorderEdge::default();
        let keep = [
            uniform(&left).unwrap_or(none),
            uniform(&right).unwrap_or(none),
            uniform(&top).unwrap_or(none),
            uniform(&bottom).unwrap_or(none),
        ];
        let mut memo = std::collections::HashMap::new();
        self.restyle(sheet, area, &mut |styles, _, r, c| {
            let key = (r == r1, r == r2, c == c1, c == c2);
            if let Some(n) = memo.get(&key) {
                return Ok(*n);
            }
            let pick = |on_edge: bool, e: BorderEdge| Some(if on_edge { e } else { none });
            let patch = FormatPatch {
                border: BorderPatch {
                    left: pick(key.2, keep[0]),
                    right: pick(key.3, keep[1]),
                    top: pick(key.0, keep[2]),
                    bottom: pick(key.1, keep[3]),
                    touch: true,
                    ..BorderPatch::default()
                },
                ..FormatPatch::default()
            };
            let n = styles.derive_xf(top_left, &patch)?;
            memo.insert(key, n);
            Ok(n)
        })?;
        Ok(())
    }

    /// Cells that merging `rect` would empty: every non-blank cell of an area except its upper-left
    /// one. Excel asks for confirmation when this is not empty.
    ///
    /// # Errors
    /// The range is outside the sheet, or the sheet is not a worksheet.
    pub fn merge_discards(
        &self,
        sheet: usize,
        rect: Rect,
        kind: MergeKind,
    ) -> Result<Vec<(u32, u32)>, XlsxError> {
        check(rect)?;
        let ws = self.sheet(sheet).ok_or(XlsxError::NotAWorksheet(sheet))?;
        Ok(ws.merge_discards(&merge_areas(rect, kind)))
    }

    /// Merges or unmerges. Existing merges that touch a new area are replaced by it, as in Excel.
    /// Cell values are not touched.
    ///
    /// # Errors
    /// The range is outside the sheet, spans every row or every column (Excel refuses that too),
    /// or the sheet is not a worksheet.
    pub fn merge_cells(
        &mut self,
        sheet: usize,
        rect: Rect,
        kind: MergeKind,
    ) -> Result<(), XlsxError> {
        check(rect)?;
        if kind != MergeKind::Unmerge
            && (rect.0 == 1 && rect.2 == MAX_ROW || rect.1 == 1 && rect.3 == MAX_COL)
        {
            return Err(XlsxError::UnsupportedEdit(
                "whole rows and columns cannot be merged".to_owned(),
            ));
        }
        self.edited()?;
        if kind != MergeKind::Unmerge {
            for area in merge_areas(rect, kind) {
                self.restyle_covered(sheet, area)?;
            }
        }
        let ws = self.worksheet_mut(sheet)?;
        let mut keep = ws.merged_cells();
        let areas = merge_areas(rect, kind);
        if kind == MergeKind::Unmerge {
            keep.retain(|&m| !intersects(m, rect));
        } else {
            keep.retain(|&m| !areas.iter().any(|&a| intersects(m, a)));
            keep.extend(areas);
        }
        ws.put_merges(&keep);
        self.sheets[sheet].dirty = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn across_makes_one_area_per_row_and_skips_single_columns() {
        assert_eq!(
            merge_areas((2, 1, 4, 3), MergeKind::Across),
            vec![(2, 1, 2, 3), (3, 1, 3, 3), (4, 1, 4, 3)]
        );
        assert!(merge_areas((2, 1, 4, 1), MergeKind::Across).is_empty());
        assert_eq!(merge_areas((1, 1, 1, 1), MergeKind::Merge), vec![]);
    }
}
