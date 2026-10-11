//! Column width, row height, hide and unhide edits: `<cols><col min max width customWidth
//! hidden>`, `<row ht customHeight hidden>` and `<sheetFormatPr>`. Every other attribute of an
//! existing `col`, `row` or `sheetFormatPr` is kept.
//!
//! The rules follow Excel's output, measured through COM (`tools/office/grid-hide-oracle.ps1`,
//! `fixtures/reference/xlsx/grid-hide/xml.json`):
//!
//! - Hiding sets `hidden="1"` and keeps the stored size; a column that had no `col` gets
//!   `width="0" customWidth="1"`. A width or height of 0 is the same edit as hiding.
//! - A positive width or height on a hidden column or row shows it.
//! - Unhiding removes `hidden`; a column left without a width (or with width 0) gets the
//!   sheet's default width and `customWidth="1"`. A row element left with nothing to say is
//!   dropped.
//! - Setting the width of every column replaces `cols` with `sheetFormatPr/@defaultColWidth`;
//!   setting the height of every row writes `defaultRowHeight` with `customHeight="1"`,
//!   resizes and shows the rows that have content and drops the rest.

use super::{Attr, ElementHead, Row, Worksheet, XlsxError, XmlElement, format_number};
use crate::refs::{MAX_COL, MAX_ROW};
use crate::xml::RawXml;

/// Excel allows a column at most 255 characters wide and a row at most 409.5 points tall.
pub const MAX_COLUMN_WIDTH: f64 = 255.0;
pub const MAX_ROW_HEIGHT: f64 = 409.5;
/// Most rows one height edit creates `row` elements for.
const MAX_ROW_SPAN: u32 = 16_384;

/// The `cols` element and `sheetFormatPr` as they were, enough to put them back.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnsSnapshot {
    cols: Option<XmlElement>,
    format: Option<ElementHead>,
}

#[derive(Debug, Clone, PartialEq)]
struct SavedRow {
    r: u32,
    /// The row's start tag; `None` when the row element did not exist.
    head: Option<ElementHead>,
}

/// The start tags of a range of rows, and `sheetFormatPr`, as they were.
#[derive(Debug, Clone, PartialEq)]
pub struct RowHeightsSnapshot {
    rows: Vec<SavedRow>,
    format: Option<ElementHead>,
}

#[derive(Clone)]
struct Piece {
    min: u32,
    max: u32,
    head: ElementHead,
    children: Vec<RawXml>,
    touched: bool,
}

impl Piece {
    fn same_attrs(&self, other: &Self) -> bool {
        let rest = |p: &Piece| -> Vec<Attr> {
            p.head
                .attrs
                .iter()
                .filter(|a| !(a.ns.is_none() && (a.name == "min" || a.name == "max")))
                .cloned()
                .collect()
        };
        self.children.is_empty() && other.children.is_empty() && rest(self) == rest(other)
    }

    fn width(&self) -> Option<f64> {
        self.head
            .attr("width")
            .and_then(|v| v.trim().parse::<f64>().ok())
    }
}

/// What an edit does to the columns it covers.
#[derive(Clone, Copy)]
enum ColOp {
    /// Positive width; shows a hidden column.
    Width(f64),
    /// Auto-fitted width (`bestFit="1"`), as Excel writes it (measured: `fixtures/reference/xlsx/
    /// autofit-implicit/autofit.json`: `bestFit="1" customWidth="1"`).
    BestFit(f64),
    Hide,
    /// Shows the column; a missing or zero width becomes the given default.
    Unhide(f64),
}

fn check_span(first: u32, last: u32, limit: u32, what: &str) -> Result<(), XlsxError> {
    if first < 1 || last < first || last > limit {
        return Err(XlsxError::UnsupportedEdit(format!(
            "{what} range {first}..={last} is outside 1..={limit}"
        )));
    }
    Ok(())
}

fn num(v: f64) -> String {
    let mut s = String::new();
    format_number(v, &mut s);
    s
}

impl ColOp {
    fn apply(self, p: &mut Piece) {
        p.head.set_attr("min", &p.min.to_string());
        p.head.set_attr("max", &p.max.to_string());
        match self {
            Self::Width(w) => {
                p.head.set_attr("width", &num(w));
                p.head.set_attr("customWidth", "1");
                p.head.remove_attr("hidden");
            }
            Self::BestFit(w) => {
                p.head.set_attr("width", &num(w));
                p.head.set_attr("customWidth", "1");
                p.head.set_attr("bestFit", "1");
                p.head.remove_attr("hidden");
            }
            Self::Hide => {
                if p.head.attr("width").is_none() {
                    p.head.set_attr("width", "0");
                    p.head.set_attr("customWidth", "1");
                }
                p.head.set_attr("hidden", "1");
            }
            Self::Unhide(default) => {
                p.head.remove_attr("hidden");
                if p.width().is_none_or(|w| w <= 0.0) {
                    p.head.set_attr("width", &num(default));
                    p.head.set_attr("customWidth", "1");
                }
            }
        }
        p.touched = true;
    }

    /// Whether columns with no `col` element get one.
    fn creates(self) -> bool {
        !matches!(self, Self::Unhide(_))
    }
}

/// True when the row start tag says nothing beyond its number (`spans` and the Office
/// `dyDescent` hint do not count), other than the attributes in `ignorable`.
fn is_trivial(head: &ElementHead, ignorable: &[&str]) -> bool {
    head.attrs.iter().all(|a| {
        if a.ns.is_some() {
            a.local_name() == "dyDescent"
        } else {
            matches!(a.name.as_str(), "r" | "spans") || ignorable.contains(&a.name.as_str())
        }
    })
}

impl Worksheet {
    fn cols_index(&self) -> Option<usize> {
        self.before
            .iter()
            .position(|n| n.as_element().is_some_and(|e| e.head.is_main("cols")))
    }

    fn format_index(&self) -> Option<usize> {
        self.before.iter().position(|n| {
            n.as_element()
                .is_some_and(|e| e.head.is_main("sheetFormatPr"))
        })
    }

    pub(crate) fn columns_snapshot(&self) -> ColumnsSnapshot {
        ColumnsSnapshot {
            cols: self
                .cols_index()
                .and_then(|i| self.before[i].as_element().cloned()),
            format: self.format_head(),
        }
    }

    fn format_head(&self) -> Option<ElementHead> {
        self.format_index()
            .and_then(|i| self.before[i].as_element())
            .map(|e| e.head.clone())
    }

    /// Puts `sheetFormatPr` back to `head` (or removes it when `None`).
    fn restore_format(&mut self, head: &Option<ElementHead>) {
        match (head, self.format_index()) {
            (Some(h), Some(i)) => {
                if let Some(e) = self.before[i].as_element_mut() {
                    e.head = h.clone();
                }
            }
            (Some(h), None) => self.insert_format(h.clone()),
            (None, Some(i)) => {
                self.before.remove(i);
            }
            (None, None) => {}
        }
    }

    /// Inserts `sheetFormatPr` where the schema wants it: before `cols`, else last.
    fn insert_format(&mut self, head: ElementHead) {
        let el = RawXml::Element(XmlElement {
            head,
            children: Vec::new(),
        });
        match self.cols_index() {
            Some(i) => self.before.insert(i, el),
            None => self.before.push(el),
        }
    }

    /// The `sheetFormatPr` start tag for editing, created when the sheet has none.
    fn format_mut(&mut self) -> &mut ElementHead {
        if self.format_index().is_none() {
            let mut head = self.sheet_data_head.sibling("sheetFormatPr");
            head.attrs.push(Attr::plain("defaultRowHeight", "15"));
            self.insert_format(head);
        }
        let i = self.format_index().unwrap_or(0);
        let el = self.before[i]
            .as_element_mut()
            .expect("sheetFormatPr is an element");
        &mut el.head
    }

    pub(crate) fn restore_columns(&mut self, snapshot: &ColumnsSnapshot) {
        match (&snapshot.cols, self.cols_index()) {
            (Some(el), Some(i)) => self.before[i] = RawXml::Element(el.clone()),
            (Some(el), None) => self.before.push(RawXml::Element(el.clone())),
            (None, Some(i)) => {
                self.before.remove(i);
            }
            (None, None) => {}
        }
        self.restore_format(&snapshot.format);
    }

    /// Sets the width (characters, padding included) of columns `first..=last`, splitting
    /// `col` entries at the edges and merging neighbours that end up identical. A width of 0
    /// hides the columns; a positive width shows them. Covering every column replaces `cols`
    /// with `sheetFormatPr/@defaultColWidth`, as Excel does for a select-all resize.
    pub(crate) fn set_column_width(
        &mut self,
        first: u32,
        last: u32,
        width: f64,
    ) -> Result<(), XlsxError> {
        check_span(first, last, MAX_COL, "column")?;
        if width <= 0.0 {
            return self.edit_columns(first, last, ColOp::Hide);
        }
        if first == 1 && last == MAX_COL {
            if let Some(i) = self.cols_index() {
                self.before.remove(i);
            }
            self.format_mut().set_attr("defaultColWidth", &num(width));
            return Ok(());
        }
        self.edit_columns(first, last, ColOp::Width(width))
    }

    /// Stores a measured auto-fit width (stored characters, padding included) on one column.
    pub(crate) fn set_column_best_fit(&mut self, col: u32, width: f64) -> Result<(), XlsxError> {
        check_span(col, col, MAX_COL, "column")?;
        self.edit_columns(col, col, ColOp::BestFit(width))
    }

    pub(crate) fn hide_columns(&mut self, first: u32, last: u32) -> Result<(), XlsxError> {
        check_span(first, last, MAX_COL, "column")?;
        self.edit_columns(first, last, ColOp::Hide)
    }

    pub(crate) fn unhide_columns(
        &mut self,
        first: u32,
        last: u32,
        default_width: f64,
    ) -> Result<(), XlsxError> {
        check_span(first, last, MAX_COL, "column")?;
        self.edit_columns(first, last, ColOp::Unhide(default_width))
    }

    fn edit_columns(&mut self, first: u32, last: u32, op: ColOp) -> Result<(), XlsxError> {
        let existing = self.cols_index();
        let template = existing
            .and_then(|i| self.before[i].as_element())
            .map(|e| XmlElement {
                head: e.head.clone(),
                children: Vec::new(),
            })
            .unwrap_or_else(|| XmlElement {
                head: self.sheet_data_head.sibling("cols"),
                children: Vec::new(),
            });
        let mut pieces: Vec<Piece> = Vec::new();
        if let Some(cols) = existing.and_then(|i| self.before[i].as_element()) {
            for child in &cols.children {
                let RawXml::Element(e) = child else { continue };
                let num = |n: &str| e.head.attr(n).and_then(|v| v.trim().parse::<u32>().ok());
                let (Some(min), Some(max)) = (num("min"), num("max")) else {
                    return Err(XlsxError::UnsupportedEdit(
                        "a col element has no valid min and max".to_owned(),
                    ));
                };
                pieces.push(Piece {
                    min,
                    max,
                    head: e.head.clone(),
                    children: e.children.clone(),
                    touched: false,
                });
            }
        }

        let new_piece = |min: u32, max: u32| {
            let mut head = template.head.sibling("col");
            head.attrs.push(Attr::plain("min", &min.to_string()));
            head.attrs.push(Attr::plain("max", &max.to_string()));
            Piece {
                min,
                max,
                head,
                children: Vec::new(),
                touched: false,
            }
        };

        let mut out: Vec<Piece> = Vec::new();
        let mut covered: Vec<(u32, u32)> = Vec::new();
        for p in pieces {
            if p.max < first || p.min > last {
                out.push(p);
                continue;
            }
            if p.min < first {
                let mut left = p.clone();
                left.max = first - 1;
                left.head.set_attr("max", &left.max.to_string());
                out.push(left);
            }
            let mut mid = p.clone();
            mid.min = p.min.max(first);
            mid.max = p.max.min(last);
            op.apply(&mut mid);
            covered.push((mid.min, mid.max));
            out.push(mid);
            if p.max > last {
                let mut right = p;
                right.min = last + 1;
                right.head.set_attr("min", &right.min.to_string());
                out.push(right);
            }
        }
        if op.creates() {
            covered.sort_unstable();
            let mut next = first;
            for (a, b) in covered {
                if a > next {
                    let mut gap = new_piece(next, a - 1);
                    op.apply(&mut gap);
                    out.push(gap);
                }
                next = next.max(b + 1);
            }
            if next <= last {
                let mut gap = new_piece(next, last);
                op.apply(&mut gap);
                out.push(gap);
            }
        }
        out.sort_by_key(|p| p.min);

        let mut merged: Vec<Piece> = Vec::new();
        for p in out {
            if let Some(prev) = merged.last_mut()
                && prev.max + 1 == p.min
                && (prev.touched || p.touched)
                && prev.same_attrs(&p)
            {
                prev.max = p.max;
                prev.head.set_attr("max", &p.max.to_string());
                prev.touched = true;
                continue;
            }
            merged.push(p);
        }

        if merged.is_empty() {
            if let Some(i) = existing {
                self.before.remove(i);
            }
            return Ok(());
        }
        let mut cols = template;
        cols.children = merged
            .into_iter()
            .map(|p| {
                RawXml::Element(XmlElement {
                    head: p.head,
                    children: p.children,
                })
            })
            .collect();
        match existing {
            Some(i) => self.before[i] = RawXml::Element(cols),
            None => {
                // `cols` goes right before `sheetData`: after `sheetFormatPr`.
                self.before.push(RawXml::Element(cols));
            }
        }
        Ok(())
    }

    /// The start tags of rows `first..=last` that exist (or, with `all`, of every row in the
    /// range, noting those that do not exist).
    pub(crate) fn row_heights_snapshot(&self, first: u32, last: u32) -> RowHeightsSnapshot {
        let start = self.rows.partition_point(|r| r.r < first);
        let mut rows: Vec<SavedRow> = Vec::new();
        let mut next = first;
        for row in self.rows[start..].iter().take_while(|r| r.r <= last) {
            // Rows between existing ones are noted too, so a created row is removed again.
            if last - first < MAX_ROW_SPAN {
                for r in next..row.r {
                    rows.push(SavedRow { r, head: None });
                }
            }
            rows.push(SavedRow {
                r: row.r,
                head: Some(row.head.clone()),
            });
            next = row.r + 1;
        }
        if last - first < MAX_ROW_SPAN {
            for r in next..=last {
                rows.push(SavedRow { r, head: None });
            }
        }
        RowHeightsSnapshot {
            rows,
            format: self.format_head(),
        }
    }

    pub(crate) fn restore_row_heights(&mut self, snapshot: &RowHeightsSnapshot) {
        for saved in &snapshot.rows {
            match (self.find_row(saved.r), &saved.head) {
                (Ok(i), Some(head)) => self.rows[i].head = head.clone(),
                (Err(i), Some(head)) => self.rows.insert(i, Row::new(saved.r, head.clone())),
                (Ok(i), None) => {
                    if self.rows[i].cells.is_empty() && self.rows[i].extras.is_empty() {
                        self.rows.remove(i);
                    }
                }
                (Err(_), None) => {}
            }
        }
        self.restore_format(&snapshot.format);
    }

    fn row_index_or_create(&mut self, r: u32) -> usize {
        match self.find_row(r) {
            Ok(i) => i,
            Err(i) => {
                let mut head = self.sheet_data_head.sibling("row");
                head.attrs.push(Attr::plain("r", &r.to_string()));
                self.rows.insert(i, Row::new(r, head));
                i
            }
        }
    }

    /// Sets the height in points of rows `first..=last`, creating empty `row` elements where
    /// none exist. A height of 0 hides the rows; a positive height shows them. Covering every
    /// row sets the sheet default instead (see the module notes).
    pub(crate) fn set_row_height(
        &mut self,
        first: u32,
        last: u32,
        points: f64,
    ) -> Result<(), XlsxError> {
        check_span(first, last, MAX_ROW, "row")?;
        if points <= 0.0 {
            return self.hide_rows(first, last);
        }
        if first == 1 && last == MAX_ROW {
            self.set_all_row_heights(points);
            return Ok(());
        }
        if last - first >= MAX_ROW_SPAN {
            return Err(XlsxError::UnsupportedEdit(format!(
                "a height edit covers at most {MAX_ROW_SPAN} rows"
            )));
        }
        let value = num(points);
        for r in first..=last {
            let i = self.row_index_or_create(r);
            let head = &mut self.rows[i].head;
            head.set_attr("ht", &value);
            head.set_attr("customHeight", "1");
            head.remove_attr("hidden");
        }
        Ok(())
    }

    /// Stores a measured auto-fit height on row `r`: `ht` without `customHeight`, as Excel does
    /// (measured); a height equal to the sheet default writes no `ht`. The row is shown.
    pub(crate) fn set_row_auto_height(&mut self, r: u32, points: f64) -> Result<(), XlsxError> {
        check_span(r, r, MAX_ROW, "row")?;
        let default = self.sheet_format().default_row_height;
        let i = self.row_index_or_create(r);
        let head = &mut self.rows[i].head;
        if (points - default).abs() < 0.05 {
            head.remove_attr("ht");
        } else {
            head.set_attr("ht", &num(points));
        }
        head.remove_attr("customHeight");
        head.remove_attr("hidden");
        Ok(())
    }

    pub(crate) fn hide_rows(&mut self, first: u32, last: u32) -> Result<(), XlsxError> {
        check_span(first, last, MAX_ROW, "row")?;
        if last - first >= MAX_ROW_SPAN {
            return Err(XlsxError::UnsupportedEdit(format!(
                "a hide edit covers at most {MAX_ROW_SPAN} rows"
            )));
        }
        for r in first..=last {
            let i = self.row_index_or_create(r);
            self.rows[i].head.set_attr("hidden", "1");
        }
        Ok(())
    }

    /// Shows the rows of `first..=last` that are hidden; an empty row left with nothing to say
    /// is dropped, as Excel does.
    pub(crate) fn unhide_rows(&mut self, first: u32, last: u32) -> Result<(), XlsxError> {
        check_span(first, last, MAX_ROW, "row")?;
        let start = self.rows.partition_point(|r| r.r < first);
        let end = self.rows.partition_point(|r| r.r <= last);
        let mut keep = Vec::with_capacity(end - start);
        for mut row in self.rows.drain(start..end) {
            row.head.remove_attr("hidden");
            let empty = row.cells.is_empty() && row.extras.is_empty();
            if !(empty && is_trivial(&row.head, &[])) {
                keep.push(row);
            }
        }
        self.rows.splice(start..start, keep);
        Ok(())
    }

    /// Select-all row height: the default row height becomes `points` (custom), every row with
    /// content takes it and is shown, and rows that say nothing else are dropped.
    fn set_all_row_heights(&mut self, points: f64) {
        let value = num(points);
        let format = self.format_mut();
        format.set_attr("defaultRowHeight", &value);
        format.set_attr("customHeight", "1");
        format.remove_attr("zeroHeight");
        let rows = std::mem::take(&mut self.rows);
        self.rows = rows
            .into_iter()
            .filter_map(|mut row| {
                let empty = row.cells.is_empty() && row.extras.is_empty();
                if empty && is_trivial(&row.head, &["ht", "customHeight", "hidden"]) {
                    return None;
                }
                row.head.set_attr("ht", &value);
                row.head.set_attr("customHeight", "1");
                row.head.remove_attr("hidden");
                Some(row)
            })
            .collect();
    }
}
