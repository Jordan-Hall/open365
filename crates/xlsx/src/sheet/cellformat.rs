//! Cell, row and column styles (`c/@s`, `row/@s` with `customFormat`, `col/@style`) and the
//! restyling of a range the way Excel does it:
//!
//! - a blank cell has the style of its row when the row has one, else of its column, else 0;
//! - formatting a rectangle writes the new style on every cell of it (creating the cells);
//! - formatting whole rows writes the row style and restyles the cells already in those rows;
//! - formatting whole columns writes the `col` styles (splitting entries at the edges, merging
//!   neighbours that end up identical) and restyles the cells already in those columns.

use std::collections::BTreeSet;

use super::{Cell, Rect, Row, Worksheet};
use crate::XlsxError;
use crate::refs::{MAX_COL, MAX_ROW};
use crate::xml::{Attr, ElementHead, RawXml, XmlElement};

/// Most cells one formatting action may create.
pub(crate) const MAX_FORMAT_CELLS: u64 = 4_000_000;
/// Most rows one whole-row formatting action may cover.
const MAX_FORMAT_ROWS: u32 = 200_000;

/// What a restyle closure gets: the cell's current style, then its 1-based row and column
/// (the row or the column is 0 when a whole column or row is restyled).
pub(crate) type Restyle<'a> = dyn FnMut(u32, u32, u32) -> Result<u32, XlsxError> + 'a;

fn style_attr(head: &ElementHead, name: &str) -> Option<u32> {
    head.attr(name)?.trim().parse().ok()
}

fn is_custom_format(head: &ElementHead) -> bool {
    matches!(head.attr("customFormat").map(str::trim), Some("1" | "true"))
}

#[derive(Clone)]
struct ColPiece {
    min: u32,
    max: u32,
    head: ElementHead,
    children: Vec<RawXml>,
}

impl ColPiece {
    fn style(&self) -> u32 {
        style_attr(&self.head, "style").unwrap_or(0)
    }

    fn same_attrs(&self, other: &Self) -> bool {
        let strip = |h: &ElementHead| -> Vec<(String, String)> {
            h.attrs
                .iter()
                .filter(|a| a.ns.is_some() || !matches!(a.name.as_str(), "min" | "max"))
                .map(|a| (a.name.clone(), a.value.clone()))
                .collect()
        };
        strip(&self.head) == strip(&other.head) && self.children == other.children
    }
}

impl Worksheet {
    fn cols_element_index(&self) -> Option<usize> {
        self.before
            .iter()
            .position(|n| n.as_element().is_some_and(|e| e.head.is_main("cols")))
    }

    fn col_pieces(&self) -> Vec<ColPiece> {
        let Some(cols) = self
            .cols_element_index()
            .and_then(|i| self.before[i].as_element())
        else {
            return Vec::new();
        };
        cols.elements()
            .filter(|e| e.head.is_main("col"))
            .filter_map(|e| {
                let min = style_attr(&e.head, "min")?;
                let max = style_attr(&e.head, "max")?;
                Some(ColPiece {
                    min,
                    max,
                    head: e.head.clone(),
                    children: e.children.clone(),
                })
            })
            .collect()
    }

    /// The style of row `row` when it has one of its own (`customFormat`).
    pub(crate) fn row_style(&self, row: u32) -> Option<u32> {
        let i = self.find_row(row).ok()?;
        let head = &self.rows[i].head;
        if is_custom_format(head) {
            style_attr(head, "s")
        } else {
            None
        }
    }

    /// The style of column `col` when a `col` entry gives it one.
    pub(crate) fn col_style(&self, col: u32) -> Option<u32> {
        let cols = self
            .cols_element_index()
            .and_then(|i| self.before[i].as_element())?;
        cols.elements()
            .filter(|e| e.head.is_main("col"))
            .find(|e| {
                style_attr(&e.head, "min").is_some_and(|m| m <= col)
                    && style_attr(&e.head, "max").is_some_and(|m| m >= col)
            })
            .and_then(|e| style_attr(&e.head, "style"))
    }

    /// The style index (`cellXfs`) in effect at `(row, col)`: the cell's own, else its row's,
    /// else its column's, else 0.
    pub fn effective_style(&self, row: u32, col: u32) -> u32 {
        if let Some(cell) = self.cell(row, col) {
            return cell.style.unwrap_or(0);
        }
        self.row_style(row)
            .or_else(|| self.col_style(col))
            .unwrap_or(0)
    }

    /// The style a cell created at `(row, col)` starts with (`None` for the default).
    pub(crate) fn inherited_style(&self, row: u32, col: u32) -> Option<u32> {
        self.row_style(row)
            .or_else(|| self.col_style(col))
            .filter(|s| *s != 0)
    }

    /// Restyles the rectangle cell by cell, creating the cells that need a style of their own.
    pub(crate) fn restyle_cells(
        &mut self,
        rect: Rect,
        f: &mut Restyle<'_>,
    ) -> Result<(), XlsxError> {
        let (r1, c1, r2, c2) = rect;
        let cells = u64::from(r2 - r1 + 1) * u64::from(c2 - c1 + 1);
        if cells > MAX_FORMAT_CELLS {
            return Err(XlsxError::UnsupportedEdit(format!(
                "{cells} cells is more than one formatting action may style"
            )));
        }
        let col_styles: Vec<u32> = if c2 - c1 < 20_000 {
            (c1..=c2).map(|c| self.col_style(c).unwrap_or(0)).collect()
        } else {
            Vec::new()
        };
        for r in r1..=r2 {
            let row_style = self.row_style(r);
            let existing = self.find_row(r).ok();
            let mut updates: Vec<(u32, u32)> = Vec::new();
            for c in c1..=c2 {
                let own = existing
                    .and_then(|i| {
                        self.rows[i]
                            .cells
                            .binary_search_by_key(&c, |x| x.col)
                            .ok()
                            .map(|j| (i, j))
                    })
                    .map(|(i, j)| self.rows[i].cells[j].style.unwrap_or(0));
                let old = own.unwrap_or_else(|| {
                    row_style
                        .unwrap_or_else(|| col_styles.get((c - c1) as usize).copied().unwrap_or(0))
                });
                let new = f(old, r, c)?;
                if new != old {
                    updates.push((c, new));
                }
            }
            if updates.is_empty() {
                continue;
            }
            for (c, new) in updates {
                let cell = self.cell_slot_plain(r, c);
                cell.style = (new != 0).then_some(new);
            }
        }
        Ok(())
    }

    /// The cell at `(row, col)`, created blank without inheriting a style (the restyle sets it).
    fn cell_slot_plain(&mut self, row: u32, col: u32) -> &mut Cell {
        let ri = match self.find_row(row) {
            Ok(i) => i,
            Err(i) => {
                let row_el = self.new_row(row);
                self.rows.insert(i, row_el);
                i
            }
        };
        let ci = match self.rows[ri].cells.binary_search_by_key(&col, |c| c.col) {
            Ok(i) => i,
            Err(i) => {
                self.rows[ri].cells.insert(i, Cell::empty(col));
                let head = &mut self.rows[ri].head;
                head.remove_attr("spans");
                i
            }
        };
        self.grow_dimension(row, col);
        &mut self.rows[ri].cells[ci]
    }

    /// Sets the cell format of one cell (creating the cell blank when it does not exist).
    pub(crate) fn set_cell_style(&mut self, row: u32, col: u32, xf: u32) {
        let cell = self.cell_slot_plain(row, col);
        cell.style = (xf != 0).then_some(xf);
    }

    fn new_row(&self, row: u32) -> Row {
        let mut head = self.sheet_data_head.sibling("row");
        head.attrs.push(Attr::plain("r", &row.to_string()));
        Row::new(row, head)
    }

    /// Restyles whole rows `first..=last`: the row style and the cells already in them.
    pub(crate) fn restyle_rows(
        &mut self,
        first: u32,
        last: u32,
        f: &mut Restyle<'_>,
    ) -> Result<(), XlsxError> {
        if last - first + 1 > MAX_FORMAT_ROWS {
            return Err(XlsxError::UnsupportedEdit(format!(
                "{} rows is more than one formatting action may style",
                last - first + 1
            )));
        }
        for r in first..=last {
            let old = self.row_style(r).unwrap_or(0);
            let new = f(old, r, 0)?;
            let ri = match self.find_row(r) {
                Ok(i) => Some(i),
                Err(i) if new != old => {
                    let row_el = self.new_row(r);
                    self.rows.insert(i, row_el);
                    Some(i)
                }
                Err(_) => None,
            };
            let Some(ri) = ri else { continue };
            if new != old {
                let head = &mut self.rows[ri].head;
                if new == 0 {
                    head.remove_attr("s");
                    head.remove_attr("customFormat");
                } else {
                    head.set_attr("s", &new.to_string());
                    head.set_attr("customFormat", "1");
                }
            }
            for k in 0..self.rows[ri].cells.len() {
                let (col, old) = {
                    let cell = &self.rows[ri].cells[k];
                    (cell.col, cell.style.unwrap_or(0))
                };
                let new = f(old, r, col)?;
                if new != old {
                    self.rows[ri].cells[k].style = (new != 0).then_some(new);
                }
            }
        }
        // Where a styled column crosses these rows the row style would hide the column's own
        // style, so Excel writes the cell out.
        let mut budget = 1000u32;
        for piece in self.col_pieces() {
            if piece.style() == 0 {
                continue;
            }
            for c in piece.min..=piece.max {
                if budget == 0 {
                    return Ok(());
                }
                budget -= 1;
                for r in first..=last {
                    if self.cell(r, c).is_some() {
                        continue;
                    }
                    let row_style = self.row_style(r).unwrap_or(0);
                    let new = f(piece.style(), r, c)?;
                    if new != row_style {
                        self.cell_slot_plain(r, c).style = (new != 0).then_some(new);
                    }
                }
            }
        }
        Ok(())
    }

    /// Restyles whole columns `first..=last`: the `col` styles and the cells already in them.
    /// A new `col` entry gets `default_width`.
    pub(crate) fn restyle_cols(
        &mut self,
        first: u32,
        last: u32,
        default_width: f64,
        f: &mut Restyle<'_>,
    ) -> Result<(), XlsxError> {
        let pieces = self.col_pieces();
        let template_head = self
            .cols_element_index()
            .and_then(|i| self.before[i].as_element())
            .map_or_else(
                || self.sheet_data_head.sibling("cols"),
                |e| ElementHead {
                    attrs: e.head.attrs.clone(),
                    ..e.head.clone()
                },
            );
        let col_head = template_head.sibling("col");
        let mut out: Vec<ColPiece> = Vec::new();
        // Entries outside the span stay (clipped at the edges).
        for p in &pieces {
            if p.max < first || p.min > last {
                out.push(p.clone());
                continue;
            }
            if p.min < first {
                let mut left = p.clone();
                left.max = first - 1;
                left.head.set_attr("max", &left.max.to_string());
                out.push(left);
            }
            if p.max > last {
                let mut right = p.clone();
                right.min = last + 1;
                right.head.set_attr("min", &right.min.to_string());
                out.push(right);
            }
        }
        // The span itself, one run per stretch of equal outcome.
        let mut runs: Vec<ColPiece> = Vec::new();
        let mut source: Option<usize> = None;
        for c in first..=last {
            let src = pieces.iter().position(|p| p.min <= c && c <= p.max);
            let old = src.map_or(0, |i| pieces[i].style());
            let new = f(old, 0, c)?;
            if let Some(run) = runs.last_mut()
                && run.max + 1 == c
                && source == src
                && run.style() == new
            {
                run.max = c;
                run.head.set_attr("max", &c.to_string());
                continue;
            }
            source = src;
            let mut piece = match src {
                Some(i) => pieces[i].clone(),
                None => {
                    let mut head = col_head.clone();
                    head.attrs.clear();
                    head.attrs.push(Attr::plain("min", "0"));
                    head.attrs.push(Attr::plain("max", "0"));
                    head.attrs
                        .push(Attr::plain("width", &format!("{default_width}")));
                    ColPiece {
                        min: c,
                        max: c,
                        head,
                        children: Vec::new(),
                    }
                }
            };
            piece.min = c;
            piece.max = c;
            piece.head.set_attr("min", &c.to_string());
            piece.head.set_attr("max", &c.to_string());
            if new == 0 {
                piece.head.remove_attr("style");
            } else {
                piece.head.set_attr("style", &new.to_string());
            }
            runs.push(piece);
        }
        // A fresh entry that ended up unstyled and default carries no information.
        runs.retain(|p| {
            !(p.children.is_empty()
                && p.style() == 0
                && p.head
                    .attrs
                    .iter()
                    .all(|a| a.ns.is_none() && matches!(a.name.as_str(), "min" | "max" | "width"))
                && p.head
                    .attr("width")
                    .and_then(|w| w.parse::<f64>().ok())
                    .is_some_and(|w| (w - default_width).abs() < 1e-6))
        });
        out.extend(runs);
        out.sort_by_key(|p| p.min);
        let mut merged: Vec<ColPiece> = Vec::new();
        for p in out {
            if let Some(prev) = merged.last_mut()
                && prev.max + 1 == p.min
                && prev.same_attrs(&p)
            {
                prev.max = p.max;
                prev.head.set_attr("max", &p.max.to_string());
                continue;
            }
            merged.push(p);
        }
        let existing = self.cols_element_index();
        if merged.is_empty() {
            if let Some(i) = existing {
                self.before.remove(i);
            }
        } else {
            let cols = XmlElement {
                head: template_head,
                children: merged
                    .into_iter()
                    .map(|p| {
                        RawXml::Element(XmlElement {
                            head: p.head,
                            children: p.children,
                        })
                    })
                    .collect(),
            };
            match existing {
                Some(i) => self.before[i] = RawXml::Element(cols),
                None => self.before.push(RawXml::Element(cols)),
            }
        }
        // Cells already in the columns.
        for ri in 0..self.rows.len() {
            let r = self.rows[ri].r;
            let start = self.rows[ri].cells.partition_point(|c| c.col < first);
            for k in start..self.rows[ri].cells.len() {
                let (col, old) = {
                    let cell = &self.rows[ri].cells[k];
                    (cell.col, cell.style.unwrap_or(0))
                };
                if col > last {
                    break;
                }
                let new = f(old, r, col)?;
                if new != old {
                    self.rows[ri].cells[k].style = (new != 0).then_some(new);
                }
            }
        }
        // Where a styled row crosses these columns the row style would hide the column's new
        // style, so Excel writes the cell out.
        if last - first < 1000 {
            let styled: Vec<(u32, u32)> = self
                .rows
                .iter()
                .filter(|r| is_custom_format(&r.head))
                .filter_map(|r| style_attr(&r.head, "s").map(|s| (r.r, s)))
                .collect();
            for (r, s) in styled {
                for c in first..=last {
                    if self.cell(r, c).is_none() {
                        let new = f(s, r, c)?;
                        if new != s {
                            self.cell_slot_plain(r, c).style = (new != 0).then_some(new);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Every style index the sheet uses: cells, rows and columns.
    pub(crate) fn collect_styles(&self, out: &mut BTreeSet<u32>) {
        for row in &self.rows {
            if is_custom_format(&row.head)
                && let Some(s) = style_attr(&row.head, "s")
            {
                out.insert(s);
            }
            for cell in &row.cells {
                if let Some(s) = cell.style {
                    out.insert(s);
                }
            }
        }
        for p in self.col_pieces() {
            if p.style() != 0 {
                out.insert(p.style());
            }
        }
    }

    /// Renumbers every style index through `map` (old index to new index).
    pub(crate) fn remap_styles(&mut self, map: &[u32]) {
        let lookup = |s: u32| map.get(s as usize).copied().unwrap_or(0);
        for row in &mut self.rows {
            if let Some(s) = style_attr(&row.head, "s") {
                row.head.set_attr("s", &lookup(s).to_string());
            }
            for cell in &mut row.cells {
                if let Some(s) = cell.style {
                    let n = lookup(s);
                    cell.style = (n != 0).then_some(n);
                }
            }
        }
        if let Some(i) = self.cols_element_index()
            && let Some(cols) = self.before[i].as_element_mut()
        {
            for child in &mut cols.children {
                if let RawXml::Element(e) = child
                    && let Some(s) = style_attr(&e.head, "style")
                {
                    e.head.set_attr("style", &lookup(s).to_string());
                }
            }
        }
    }

    /// The rows and columns the sheet can be formatted over, for range classification.
    pub(crate) fn classify(rect: Rect) -> RangeKind {
        let (r1, c1, r2, c2) = rect;
        let whole_rows = c1 == 1 && c2 == MAX_COL;
        let whole_cols = r1 == 1 && r2 == MAX_ROW;
        match (whole_rows, whole_cols) {
            (_, true) => RangeKind::Columns,
            (true, false) => RangeKind::Rows,
            _ => RangeKind::Cells,
        }
    }
}

/// What a rectangle means to Excel's formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RangeKind {
    Cells,
    Rows,
    Columns,
}
