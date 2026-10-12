//! Cell formatting on a workbook: the Home tab's font, fill, border, alignment, number format,
//! cell style, Format Painter and Clear Formats actions. Every action returns a
//! [`FormatSnapshot`] of what it changed, so the caller can undo it.
//!
//! A range is a rectangle `(row1, col1, row2, col2)`. As in Excel, a rectangle that spans every
//! column formats whole rows, and one that spans every row formats whole columns.

use std::collections::{BTreeSet, HashMap};

use super::Workbook;
use crate::XlsxError;
use crate::format::{BorderEdge, CellFormat};
use crate::geometry::{BASE_DPI, GridMetrics};
use crate::refs::{MAX_COL, MAX_ROW};
use crate::sheet::cellformat::{RangeKind, Restyle};
use crate::sheet::{CellValue, Rect, Worksheet};
use crate::styles::Styles;
use crate::styles_edit::{BorderPatch, FormatPatch, NumberFormat};
use crate::styles_numfmt::{adjust_decimals, general_decimals};

/// A worksheet and the style sheet as they were before a formatting action.
#[derive(Debug, Clone)]
pub struct FormatSnapshot {
    sheet: usize,
    styles: Box<Styles>,
    worksheet: Box<Worksheet>,
}

/// The border dialog: which lines to draw around and inside a range. `None` leaves a line as it is;
/// an edge with [`crate::BorderStyle::None`] removes it. Diagonals apply to every cell.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BorderSpec {
    pub left: Option<BorderEdge>,
    pub right: Option<BorderEdge>,
    pub top: Option<BorderEdge>,
    pub bottom: Option<BorderEdge>,
    pub inside_horizontal: Option<BorderEdge>,
    pub inside_vertical: Option<BorderEdge>,
    pub diagonal_down: Option<BorderEdge>,
    pub diagonal_up: Option<BorderEdge>,
}

impl BorderSpec {
    /// All four outer edges of the range.
    pub fn outline(edge: BorderEdge) -> Self {
        Self {
            left: Some(edge),
            right: Some(edge),
            top: Some(edge),
            bottom: Some(edge),
            ..Self::default()
        }
    }

    /// Outer edges and the lines between cells.
    pub fn all(edge: BorderEdge) -> Self {
        Self {
            inside_horizontal: Some(edge),
            inside_vertical: Some(edge),
            ..Self::outline(edge)
        }
    }

    /// The patch for the cell at `(row, col)` of a range, and a key that tells equal patches apart.
    fn patch_at(&self, kind: RangeKind, rect: Rect, row: u32, col: u32) -> (u16, BorderPatch) {
        let (r1, c1, r2, c2) = rect;
        let mut mask = 0u16;
        let mut p = BorderPatch::default();
        let by_row = kind != RangeKind::Columns && row != 0;
        let by_col = kind != RangeKind::Rows && col != 0;
        let mut put = |bit: u16, edge: Option<BorderEdge>, slot: &mut Option<BorderEdge>| {
            if edge.is_some() {
                mask |= 1 << bit;
                *slot = edge;
            }
        };
        if by_col && col == c1 {
            put(0, self.left, &mut p.left);
        }
        if by_col && col == c2 {
            put(1, self.right, &mut p.right);
        }
        if by_row && row == r1 {
            put(2, self.top, &mut p.top);
        }
        if by_row && row == r2 {
            put(3, self.bottom, &mut p.bottom);
        }
        if by_row && row < r2 && self.inside_horizontal.is_some() {
            put(4, self.inside_horizontal, &mut p.bottom);
        }
        if by_row && row > r1 && self.inside_horizontal.is_some() {
            put(5, self.inside_horizontal, &mut p.top);
        }
        if by_col && col < c2 && self.inside_vertical.is_some() {
            put(6, self.inside_vertical, &mut p.right);
        }
        if by_col && col > c1 && self.inside_vertical.is_some() {
            put(7, self.inside_vertical, &mut p.left);
        }
        put(8, self.diagonal_down, &mut p.diagonal_down);
        put(9, self.diagonal_up, &mut p.diagonal_up);
        (mask, p)
    }
}

/// A restyle closure that may also edit the style sheet: `(styles, old xf, row, col)`.
pub(super) type StyleFn<'a> = dyn FnMut(&mut Styles, u32, u32, u32) -> Result<u32, XlsxError> + 'a;

fn check_rect(rect: Rect) -> Result<(), XlsxError> {
    let (r1, c1, r2, c2) = rect;
    if r1 == 0 || c1 == 0 || r1 > r2 || c1 > c2 || r2 > MAX_ROW || c2 > MAX_COL {
        return Err(XlsxError::UnsupportedEdit(format!(
            "range ({r1}, {c1}) to ({r2}, {c2}) is outside the sheet"
        )));
    }
    Ok(())
}

impl Workbook {
    /// The worksheet and style sheet as they are, for [`Self::restore_format`].
    pub fn format_snapshot(&self, sheet: usize) -> Result<FormatSnapshot, XlsxError> {
        let ws = self.sheet(sheet).ok_or(XlsxError::NotAWorksheet(sheet))?;
        Ok(FormatSnapshot {
            sheet,
            styles: Box::new(self.styles.clone()),
            worksheet: Box::new(ws.clone()),
        })
    }

    /// Puts a worksheet and the style sheet back as a snapshot had them.
    pub fn restore_format(&mut self, snapshot: FormatSnapshot) -> Result<(), XlsxError> {
        let entry = self
            .sheets
            .get_mut(snapshot.sheet)
            .ok_or(XlsxError::NotAWorksheet(snapshot.sheet))?;
        entry.data = Some(*snapshot.worksheet);
        entry.dirty = true;
        self.styles = *snapshot.styles;
        self.styles_dirty = true;
        Ok(())
    }

    /// The `cellXfs` index in effect at a cell: its own, else its row's, else its column's.
    pub fn cell_xf(&self, sheet: usize, row: u32, col: u32) -> u32 {
        self.sheet(sheet)
            .map_or(0, |ws| ws.effective_style(row, col))
    }

    /// The resolved format (font, fill, border, alignment, number format id) at a cell.
    pub fn cell_format(&self, sheet: usize, row: u32, col: u32) -> &CellFormat {
        self.styles.cell_format(self.cell_xf(sheet, row, col))
    }

    /// Gives many cells a `cellXfs` index in one step: one snapshot of the sheet for the whole
    /// list (returned for undo) instead of one per cell. Nothing changes when any entry is out
    /// of range or names an unknown `xf`. Later entries for the same cell win.
    pub fn set_cell_xfs(
        &mut self,
        sheet: usize,
        cells: &[(u32, u32, u32)],
    ) -> Result<FormatSnapshot, XlsxError> {
        let xfs = self.styles.cell_xf_count();
        for &(r, c, xf) in cells {
            check_rect((r, c, r, c))?;
            if xf as usize >= xfs.max(1) {
                return Err(XlsxError::UnsupportedEdit(format!(
                    "cell format {xf} does not exist"
                )));
            }
        }
        self.worksheet_mut(sheet)?;
        let before = self.format_snapshot(sheet)?;
        self.edited()?;
        let Some(ws) = self.sheets.get_mut(sheet).and_then(|e| e.data.as_mut()) else {
            return Err(XlsxError::NotAWorksheet(sheet));
        };
        for &(r, c, xf) in cells {
            ws.set_cell_style(r, c, xf);
        }
        if let Some(e) = self.sheets.get_mut(sheet) {
            e.dirty = true;
        }
        self.styles_dirty = true;
        Ok(before)
    }

    /// Core of every action: restyles `rect` with `f(styles, old xf, row, col)`, undoing the
    /// action if `f` fails.
    pub(super) fn restyle(
        &mut self,
        sheet: usize,
        rect: Rect,
        f: &mut StyleFn<'_>,
    ) -> Result<FormatSnapshot, XlsxError> {
        check_rect(rect)?;
        self.worksheet_mut(sheet)?;
        let before = self.format_snapshot(sheet)?;
        self.edited()?;
        let metrics = GridMetrics::for_styles(&self.styles, BASE_DPI);
        let outcome = {
            let styles = &mut self.styles;
            let Some(entry) = self.sheets.get_mut(sheet) else {
                return Err(XlsxError::NotAWorksheet(sheet));
            };
            let Some(ws) = entry.data.as_mut() else {
                return Err(XlsxError::NotAWorksheet(sheet));
            };
            let default_width = ws.default_column_width(metrics);
            let mut g = |old: u32, r: u32, c: u32| f(styles, old, r, c);
            let g: &mut Restyle<'_> = &mut g;
            let (r1, c1, r2, c2) = rect;
            match Worksheet::classify(rect) {
                RangeKind::Cells => ws.restyle_cells(rect, g),
                RangeKind::Rows => ws.restyle_rows(r1, r2, g),
                RangeKind::Columns => ws.restyle_cols(c1, c2, default_width, g),
            }
            .inspect(|()| entry.dirty = true)
        };
        match outcome {
            Ok(()) => {
                self.styles_dirty = true;
                Ok(before)
            }
            Err(e) => {
                self.restore_format(before)?;
                Err(e)
            }
        }
    }

    /// Applies a formatting change to every cell of `rect` (font, fill, alignment, number
    /// format, protection, or single border edges).
    pub fn format_range(
        &mut self,
        sheet: usize,
        rect: Rect,
        patch: &FormatPatch,
    ) -> Result<FormatSnapshot, XlsxError> {
        let mut memo: HashMap<u32, u32> = HashMap::new();
        self.restyle(sheet, rect, &mut |styles, old, _, _| {
            if let Some(n) = memo.get(&old) {
                return Ok(*n);
            }
            let n = styles.derive_xf(old, patch)?;
            memo.insert(old, n);
            Ok(n)
        })
    }

    /// The border dialog: outline, inside lines and diagonals of `rect`.
    pub fn set_borders(
        &mut self,
        sheet: usize,
        rect: Rect,
        spec: &BorderSpec,
    ) -> Result<FormatSnapshot, XlsxError> {
        let kind = Worksheet::classify(rect);
        let mut memo: HashMap<(u32, u16), u32> = HashMap::new();
        let before = self.restyle(sheet, rect, &mut |styles, old, r, c| {
            let (mask, mut border) = spec.patch_at(kind, rect, r, c);
            if let Some(n) = memo.get(&(old, mask)) {
                return Ok(*n);
            }
            // Cells the range covers count as bordered even when none of their edges change.
            border.touch = true;
            let patch = FormatPatch {
                border,
                ..FormatPatch::default()
            };
            let n = styles.derive_xf(old, &patch)?;
            memo.insert((old, mask), n);
            Ok(n)
        })?;
        if kind == RangeKind::Cells {
            self.clear_facing_edges(sheet, rect, spec)?;
        }
        Ok(before)
    }

    /// Removing an outer edge also removes the matching edge the neighbouring cell outside the range
    /// draws on the same line (Excel keeps the two in step when a line goes away, but does not copy
    /// a new line onto the neighbour).
    fn clear_facing_edges(
        &mut self,
        sheet: usize,
        rect: Rect,
        spec: &BorderSpec,
    ) -> Result<(), XlsxError> {
        let (r1, c1, r2, c2) = rect;
        let none = |e: &Option<BorderEdge>| e.is_some_and(|e| e.style == crate::BorderStyle::None);
        let cleared = BorderEdge::default();
        let mut jobs: Vec<(u32, u32, BorderPatch)> = Vec::new();
        for r in r1..=r2 {
            if none(&spec.left) && c1 > 1 {
                jobs.push((
                    r,
                    c1 - 1,
                    BorderPatch {
                        right: Some(cleared),
                        ..BorderPatch::default()
                    },
                ));
            }
            if none(&spec.right) && c2 < MAX_COL {
                jobs.push((
                    r,
                    c2 + 1,
                    BorderPatch {
                        left: Some(cleared),
                        ..BorderPatch::default()
                    },
                ));
            }
        }
        for c in c1..=c2 {
            if none(&spec.top) && r1 > 1 {
                jobs.push((
                    r1 - 1,
                    c,
                    BorderPatch {
                        bottom: Some(cleared),
                        ..BorderPatch::default()
                    },
                ));
            }
            if none(&spec.bottom) && r2 < MAX_ROW {
                jobs.push((
                    r2 + 1,
                    c,
                    BorderPatch {
                        top: Some(cleared),
                        ..BorderPatch::default()
                    },
                ));
            }
        }
        for (r, c, border) in jobs {
            let f = self.cell_format(sheet, r, c).border;
            let has = (border.right.is_some() && f.right.style != crate::BorderStyle::None)
                || (border.left.is_some() && f.left.style != crate::BorderStyle::None)
                || (border.bottom.is_some() && f.bottom.style != crate::BorderStyle::None)
                || (border.top.is_some() && f.top.style != crate::BorderStyle::None);
            if has {
                let patch = FormatPatch {
                    border,
                    ..FormatPatch::default()
                };
                self.format_range(sheet, (r, c, r, c), &patch)?;
            }
        }
        Ok(())
    }

    /// Applies a named cell style (`Good`, `Heading 1`, `Percent`, ...) to `rect`. The cells take
    /// every aspect of the style.
    pub fn apply_cell_style(
        &mut self,
        sheet: usize,
        rect: Rect,
        name: &str,
    ) -> Result<FormatSnapshot, XlsxError> {
        let mut resolved: Option<u32> = None;
        self.restyle(sheet, rect, &mut |styles, _, _, _| {
            if let Some(n) = resolved {
                return Ok(n);
            }
            let n = styles.derive_named_style(name)?.ok_or_else(|| {
                XlsxError::UnsupportedEdit(format!("no cell style named {name:?}"))
            })?;
            resolved = Some(n);
            Ok(n)
        })
    }

    /// Clear Formats: every cell of `rect` goes back to the Normal style.
    pub fn clear_formats(&mut self, sheet: usize, rect: Rect) -> Result<FormatSnapshot, XlsxError> {
        self.restyle(sheet, rect, &mut |_, _, _, _| Ok(0))
    }

    /// Format Painter: copies the formats of `src` onto `dst`, repeating the source across a
    /// larger destination.
    pub fn copy_formats(
        &mut self,
        sheet: usize,
        src: Rect,
        dst: Rect,
    ) -> Result<FormatSnapshot, XlsxError> {
        check_rect(src)?;
        check_rect(dst)?;
        let (sr1, sc1, sr2, sc2) = src;
        let (h, w) = (sr2 - sr1 + 1, sc2 - sc1 + 1);
        let ws = self.sheet(sheet).ok_or(XlsxError::NotAWorksheet(sheet))?;
        // Whole rows or columns are read at their first cell.
        let (rows, cols) = (h.min(1000), w.min(1000));
        let grid: Vec<u32> = (0..rows)
            .flat_map(|r| (0..cols).map(move |c| (r, c)))
            .map(|(r, c)| ws.effective_style(sr1 + r, sc1 + c))
            .collect();
        let (dr1, dc1, ..) = dst;
        self.restyle(sheet, dst, &mut |_, _, r, c| {
            let r = if r == 0 { dr1 } else { r };
            let c = if c == 0 { dc1 } else { c };
            let at = ((r - dr1) % rows) * cols + (c - dc1) % cols;
            Ok(grid.get(at as usize).copied().unwrap_or(0))
        })
    }

    /// Increase (`delta` > 0) or Decrease Decimal on `rect`. Excel works out the new format from
    /// the top left cell and gives it to the whole range. Returns `None` when there is nothing to
    /// change (text, or a blank cell with the General format).
    pub fn change_decimals(
        &mut self,
        sheet: usize,
        rect: Rect,
        delta: i32,
    ) -> Result<Option<FormatSnapshot>, XlsxError> {
        check_rect(rect)?;
        let ws = self.sheet(sheet).ok_or(XlsxError::NotAWorksheet(sheet))?;
        let (r1, c1, ..) = rect;
        let xf = ws.effective_style(r1, c1);
        let code = self.styles.number_format(xf).to_owned();
        let new_code = if code == "General" {
            let Some(CellValue::Number(n)) = ws.cell(r1, c1).map(|c| &c.value) else {
                return Ok(None);
            };
            let width = 8; // what General shows in a column of the default width
            let d = general_decimals(*n, width) as i32 + delta.signum();
            if d <= 0 {
                "0".to_owned()
            } else {
                format!("0.{}", "0".repeat(d as usize))
            }
        } else {
            match adjust_decimals(&code, delta.signum()) {
                Some(c) => c,
                None => return Ok(None),
            }
        };
        let patch = FormatPatch {
            number_format: Some(NumberFormat::Code(new_code)),
            ..FormatPatch::default()
        };
        self.format_range(sheet, rect, &patch).map(Some)
    }

    /// Every style index a worksheet uses.
    pub(super) fn used_style_indexes(&self) -> BTreeSet<u32> {
        let mut used = BTreeSet::new();
        for entry in &self.sheets {
            if let Some(ws) = &entry.data {
                ws.collect_styles(&mut used);
            }
        }
        used
    }
}
