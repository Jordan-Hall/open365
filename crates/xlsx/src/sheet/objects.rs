//! Worksheet-level support for the writers in `crate::write`: fragment parsing, ordered root
//! elements before `sheetData`, and the cell shifts of Insert/Delete cells.

use super::{FormulaKind, RawXml};
use crate::XlsxError;
use crate::refs::{MAX_COL, MAX_ROW, parse_range};
use crate::sheet::Rect;
use crate::structure::range_a1;
use crate::xml::{self, Attr, ElementHead, XmlElement};

/// Worksheet children that precede `sheetData`, in schema order.
const BEFORE_SHEET_DATA: [&str; 5] = [
    "sheetPr",
    "dimension",
    "sheetViews",
    "sheetFormatPr",
    "cols",
];

/// A shift of the cells in a band of rows or columns along one axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CellShift {
    /// Cells move along rows (up and down) when true, along columns (left and right) when false.
    pub rows_axis: bool,
    /// Inclusive range on the other axis: the columns of a vertical shift, the rows of a horizontal one.
    pub band: (u32, u32),
    /// First index on the shifting axis that is inserted before or deleted.
    pub at: u32,
    pub count: u32,
    pub delete: bool,
}

/// What a shift does to a rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RectFate {
    Unchanged,
    Moved(Rect),
    Gone,
    /// The shift would cut the rectangle in two.
    Cut,
}

impl CellShift {
    pub fn limit(&self) -> u32 {
        if self.rows_axis { MAX_ROW } else { MAX_COL }
    }

    pub fn end(&self) -> u32 {
        self.at + self.count - 1
    }

    /// Where index `i` on the shifting axis goes; `None` when a delete removes it.
    pub fn map_index(&self, i: u32) -> Option<u32> {
        if self.delete {
            if i < self.at {
                Some(i)
            } else if i <= self.end() {
                None
            } else {
                Some(i - self.count)
            }
        } else if i >= self.at {
            Some(i + self.count)
        } else {
            Some(i)
        }
    }

    /// The range `lo..=hi` on the shifting axis after the shift (Excel's rules for ranges).
    pub fn map_range(&self, lo: u32, hi: u32) -> Option<(u32, u32)> {
        if !self.delete {
            return Some(if lo >= self.at {
                (lo + self.count, hi + self.count)
            } else if hi >= self.at {
                (lo, hi + self.count)
            } else {
                (lo, hi)
            });
        }
        let end = self.end();
        if lo >= self.at && hi <= end {
            return None;
        }
        let new_lo = if lo < self.at {
            lo
        } else if lo <= end {
            self.at
        } else {
            lo - self.count
        };
        let new_hi = if hi < self.at {
            hi
        } else if hi <= end {
            self.at - 1
        } else {
            hi - self.count
        };
        Some((new_lo, new_hi))
    }

    pub fn in_band(&self, perp: u32) -> bool {
        (self.band.0..=self.band.1).contains(&perp)
    }

    fn split(&self, r: Rect) -> ((u32, u32), (u32, u32)) {
        // (along, perpendicular)
        if self.rows_axis {
            ((r.0, r.2), (r.1, r.3))
        } else {
            ((r.1, r.3), (r.0, r.2))
        }
    }

    fn join(&self, along: (u32, u32), perp: (u32, u32)) -> Rect {
        if self.rows_axis {
            (along.0, perp.0, along.1, perp.1)
        } else {
            (perp.0, along.0, perp.1, along.1)
        }
    }

    /// What the shift does to `rect`.
    pub fn fate(&self, rect: Rect) -> RectFate {
        let (along, perp) = self.split(rect);
        if perp.1 < self.band.0 || perp.0 > self.band.1 || along.1 < self.at {
            return RectFate::Unchanged;
        }
        if perp.0 < self.band.0 || perp.1 > self.band.1 {
            return RectFate::Cut;
        }
        if self.delete {
            let end = self.end();
            if along.0 >= self.at && along.1 <= end {
                RectFate::Gone
            } else if along.0 > end {
                RectFate::Moved(self.join((along.0 - self.count, along.1 - self.count), perp))
            } else {
                RectFate::Cut
            }
        } else if along.0 >= self.at {
            RectFate::Moved(self.join((along.0 + self.count, along.1 + self.count), perp))
        } else {
            RectFate::Cut
        }
    }
}

impl super::Worksheet {
    /// Parses an element written with its own namespace declarations; declarations the worksheet
    /// root already makes are dropped so the element reads like Excel's.
    pub(crate) fn fragment(&self, text: &str) -> Result<XmlElement, XlsxError> {
        parse_fragment(text, &self.root_head)
    }

    pub(crate) fn before_element(&self, local: &str) -> Option<&XmlElement> {
        self.before
            .iter()
            .filter_map(RawXml::as_element)
            .find(|e| e.head.is_main(local))
    }

    /// Replaces, inserts (in schema order) or with `None` removes a worksheet child before `sheetData`.
    pub(crate) fn put_before_element(&mut self, local: &str, element: Option<XmlElement>) {
        let at = self
            .before
            .iter()
            .position(|n| n.as_element().is_some_and(|e| e.head.is_main(local)));
        match (at, element) {
            (Some(i), Some(el)) => self.before[i] = RawXml::Element(el),
            (Some(i), None) => {
                self.before.remove(i);
            }
            (None, Some(el)) => {
                let rank = |name: &str| BEFORE_SHEET_DATA.iter().position(|n| *n == name);
                let mine = rank(local).unwrap_or(BEFORE_SHEET_DATA.len());
                let pos = self
                    .before
                    .iter()
                    .position(|n| {
                        n.as_element()
                            .and_then(|e| rank(e.head.local_name()))
                            .is_some_and(|r| r > mine)
                    })
                    .unwrap_or(self.before.len());
                self.before.insert(pos, RawXml::Element(el));
            }
            (None, None) => {}
        }
    }

    pub(crate) fn push_table(&mut self, table: crate::tables::Table) {
        self.tables.push(table);
    }

    /// Every worksheet child element (before and after `sheetData`), mutable.
    pub(crate) fn root_elements_mut(&mut self) -> impl Iterator<Item = &mut XmlElement> {
        self.before
            .iter_mut()
            .chain(self.after.iter_mut())
            .filter_map(RawXml::as_element_mut)
    }

    /// True when no cell of `rect` holds a value, formula or style.
    pub(crate) fn rect_is_blank(&self, rect: Rect) -> bool {
        !self.rows.iter().any(|row| {
            (rect.0..=rect.2).contains(&row.r)
                && row.cells.iter().any(|c| {
                    (rect.1..=rect.3).contains(&c.col)
                        && (c.formula.is_some() || !matches!(c.value, super::CellValue::Empty))
                })
        })
    }

    fn fix_dimension_now(&mut self) {
        let used = self.used_range();
        if let Some(dim) = self
            .before
            .iter_mut()
            .filter_map(RawXml::as_element_mut)
            .find(|e| e.head.is_main("dimension"))
        {
            let text = used.map_or_else(|| "A1".to_owned(), range_a1);
            dim.head.set_attr("ref", &text);
        }
    }

    /// Checks that Excel would allow the shift on this sheet.
    pub(crate) fn check_cell_shift(&self, s: &CellShift) -> Result<(), XlsxError> {
        let refuse = |msg: &str| Err(XlsxError::UnsupportedEdit(msg.to_owned()));
        if s.count == 0 || s.at == 0 || s.band.0 == 0 || s.band.0 > s.band.1 || s.end() > s.limit()
        {
            return refuse("the cells are outside the grid");
        }
        if !s.delete {
            let mut last = 0;
            for row in &self.rows {
                for cell in &row.cells {
                    let (along, perp) = if s.rows_axis {
                        (row.r, cell.col)
                    } else {
                        (cell.col, row.r)
                    };
                    let live =
                        cell.formula.is_some() || !matches!(cell.value, super::CellValue::Empty);
                    if live && s.in_band(perp) {
                        last = last.max(along);
                    }
                }
            }
            if last >= s.at && last + s.count > s.limit() {
                return refuse("inserting would push cells off the end of the worksheet");
            }
        }
        for &rect in &self.locked {
            match s.fate(rect) {
                RectFate::Cut => {
                    return refuse("you can't change part of an array or data table");
                }
                RectFate::Gone if s.delete => {
                    return refuse("you can't change part of an array or data table");
                }
                _ => {}
            }
        }
        if let Some(merges) = self.root_element("mergeCells") {
            for m in merges.elements() {
                if let Some(rect) = m.head.attr("ref").and_then(parse_range)
                    && s.fate(rect) == RectFate::Cut
                {
                    return refuse(
                        "this operation will cause some merged cells to unmerge; cut them first",
                    );
                }
            }
        }
        for t in &self.tables {
            if let Some(rect) = t.rect
                && s.fate(rect) == RectFate::Cut
            {
                return refuse("this operation is not allowed on part of a table");
            }
        }
        Ok(())
    }

    /// Moves the cells and the range-bearing elements for the shift. Formulas are rewritten separately.
    /// Returns the number of shared-string cells that were deleted.
    pub(crate) fn apply_cell_shift(&mut self, s: &CellShift) -> usize {
        self.materialise_all_shared_cells();
        let mut moved: Vec<(u32, super::Cell)> = Vec::new();
        let mut shared_removed = 0usize;
        let mut touched: Vec<u32> = Vec::new();
        for row in &mut self.rows {
            let r = row.r;
            let mut keep = Vec::with_capacity(row.cells.len());
            for cell in std::mem::take(&mut row.cells) {
                let (along, perp) = if s.rows_axis {
                    (r, cell.col)
                } else {
                    (cell.col, r)
                };
                if !s.in_band(perp) {
                    keep.push(cell);
                    continue;
                }
                match s.map_index(along) {
                    Some(n) if n == along => keep.push(cell),
                    Some(n) => {
                        if s.rows_axis {
                            moved.push((n, cell));
                        } else {
                            let mut cell = cell;
                            cell.col = n;
                            keep.push(cell);
                        }
                        touched.push(r);
                    }
                    None => {
                        if matches!(cell.value, super::CellValue::SharedString { .. }) {
                            shared_removed += 1;
                        }
                        touched.push(r);
                    }
                }
            }
            keep.sort_by_key(|c| c.col);
            row.cells = keep;
            if touched.last() == Some(&r) {
                row.head.remove_attr("spans");
            }
        }
        for (new_row, cell) in moved {
            let ri = match self.find_row(new_row) {
                Ok(i) => i,
                Err(i) => {
                    let mut head = self.sheet_data_head.sibling("row");
                    head.attrs.push(Attr::plain("r", &new_row.to_string()));
                    self.rows.insert(
                        i,
                        super::Row {
                            r: new_row,
                            head,
                            cells: Vec::new(),
                            extras: Vec::new(),
                        },
                    );
                    i
                }
            };
            let row = &mut self.rows[ri];
            row.head.remove_attr("spans");
            let at = row
                .cells
                .binary_search_by_key(&cell.col, |c| c.col)
                .unwrap_or_else(|i| i);
            row.cells.insert(at, cell);
        }
        // Array and data-table ranges that moved whole.
        for row in &mut self.rows {
            for cell in &mut row.cells {
                let Some(f) = cell.formula.as_deref_mut() else {
                    continue;
                };
                match &mut f.kind {
                    FormulaKind::Array { range: Some(range) } => {
                        if let Some(rect) = parse_range(range)
                            && let RectFate::Moved(n) = s.fate(rect)
                        {
                            *range = range_a1(n);
                        }
                    }
                    FormulaKind::DataTable => {
                        for a in &mut f.attrs {
                            if a.ns.is_none()
                                && a.name == "ref"
                                && let Some(rect) = parse_range(&a.value)
                                && let RectFate::Moved(n) = s.fate(rect)
                            {
                                a.value = range_a1(n);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        self.fix_shift_elements(s);
        self.fix_dimension_now();
        self.index_formulas();
        shared_removed
    }

    fn materialise_all_shared_cells(&mut self) {
        self.materialise_all_shared();
    }

    fn fix_shift_elements(&mut self, s: &CellShift) {
        let mut after = std::mem::take(&mut self.after);
        after.retain_mut(|node| {
            let RawXml::Element(e) = node else {
                return true;
            };
            match e.head.local_name() {
                "mergeCells" => {
                    e.children.retain_mut(|c| {
                        let RawXml::Element(m) = c else { return true };
                        let Some(rect) = m.head.attr("ref").and_then(parse_range) else {
                            return true;
                        };
                        match s.fate(rect) {
                            RectFate::Moved(n) => {
                                m.head.set_attr("ref", &range_a1(n));
                                true
                            }
                            RectFate::Gone => false,
                            _ => true,
                        }
                    });
                    let n = e.elements().count();
                    if n == 0 {
                        return false;
                    }
                    e.head.set_attr("count", &n.to_string());
                }
                "conditionalFormatting" => {
                    match e.head.attr("sqref").and_then(|t| shift_ref_list(t, s)) {
                        Some(new) => e.head.set_attr("sqref", &new),
                        None => return e.head.attr("sqref").is_none(),
                    }
                }
                "dataValidations" => {
                    e.children.retain_mut(|c| {
                        let RawXml::Element(m) = c else { return true };
                        match m.head.attr("sqref").and_then(|t| shift_ref_list(t, s)) {
                            Some(new) => {
                                m.head.set_attr("sqref", &new);
                                true
                            }
                            None => m.head.attr("sqref").is_none(),
                        }
                    });
                    let n = e.elements().count();
                    if n == 0 {
                        return false;
                    }
                    e.head.set_attr("count", &n.to_string());
                }
                "hyperlinks" => {
                    e.children.retain_mut(|c| {
                        let RawXml::Element(h) = c else { return true };
                        match h.head.attr("ref").and_then(|t| shift_ref_list(t, s)) {
                            Some(new) => {
                                h.head.set_attr("ref", &new);
                                true
                            }
                            None => h.head.attr("ref").is_none(),
                        }
                    });
                    if e.elements().next().is_none() {
                        return false;
                    }
                }
                _ => {}
            }
            true
        });
        self.after = after;
    }
}

/// The reference list `text` (`A1:B2 D4`) after the shift; `None` when every part is deleted.
pub(crate) fn shift_ref_list(text: &str, s: &CellShift) -> Option<String> {
    let mut out: Vec<String> = Vec::new();
    for token in text.split_whitespace() {
        match parse_range(token) {
            Some(rect) => match s.fate(rect) {
                RectFate::Moved(n) => out.push(range_a1(n)),
                RectFate::Gone => {}
                _ => out.push(token.to_owned()),
            },
            None => out.push(token.to_owned()),
        }
    }
    (!out.is_empty()).then(|| out.join(" "))
}

/// Parses an element with its own namespace declarations, then drops the declarations that
/// `root` already makes.
pub(crate) fn parse_fragment(text: &str, root: &ElementHead) -> Result<XmlElement, XlsxError> {
    let mut el = xml::parse(text.as_bytes())?.root;
    el.head.attrs.retain(|a| {
        !(a.ns.is_none()
            && (a.name == "xmlns" || a.name.starts_with("xmlns:"))
            && root
                .attrs
                .iter()
                .any(|r| r.name == a.name && r.value == a.value))
    });
    Ok(el)
}

impl super::Worksheet {
    pub(crate) fn after_mut(&mut self) -> &mut Vec<RawXml> {
        &mut self.after
    }

    /// Inserts a new worksheet child after the last child of the same name, or in schema order.
    pub(crate) fn insert_root_element(&mut self, local: &str, element: XmlElement) {
        use super::sort::AFTER_SHEET_DATA;
        let rank = |name: &str| AFTER_SHEET_DATA.iter().position(|n| *n == name);
        let mine = rank(local).unwrap_or(AFTER_SHEET_DATA.len());
        let pos = self
            .after
            .iter()
            .rposition(|n| n.as_element().is_some_and(|e| e.head.is_main(local)))
            .map(|i| i + 1)
            .or_else(|| {
                self.after.iter().position(|n| {
                    n.as_element()
                        .and_then(|e| rank(e.head.local_name()))
                        .is_some_and(|r| r > mine)
                })
            })
            .unwrap_or(self.after.len());
        self.after.insert(pos, RawXml::Element(element));
    }
}

impl super::Worksheet {
    /// Sets or clears `tabSelected` on the first sheet view; true when something changed.
    pub(crate) fn set_tab_selected(&mut self, on: bool) -> bool {
        let Some(views) = self
            .before
            .iter_mut()
            .filter_map(RawXml::as_element_mut)
            .find(|e| e.head.is_main("sheetViews"))
        else {
            return false;
        };
        let mut changed = false;
        for (i, v) in views
            .children
            .iter_mut()
            .filter_map(RawXml::as_element_mut)
            .filter(|e| e.head.is_main("sheetView"))
            .enumerate()
        {
            let want = on && i == 0;
            let has = matches!(v.head.attr("tabSelected"), Some("1" | "true"));
            if want && !has {
                v.head.set_attr("tabSelected", "1");
                changed = true;
            } else if !want && v.head.attr("tabSelected").is_some() {
                v.head.remove_attr("tabSelected");
                changed = true;
            }
        }
        changed
    }

    pub(crate) fn tab_color_element(&self) -> Option<&XmlElement> {
        self.before_element("sheetPr")?
            .elements()
            .find(|e| e.head.is_main("tabColor"))
    }

    /// Writes (or with `None` removes) `sheetPr/tabColor`; `attrs` is the attribute text (`rgb="FF0000FF"`).
    pub(crate) fn set_tab_color_element(&mut self, attrs: Option<&str>) -> Result<(), XlsxError> {
        let ns = crate::xml::MAIN_NS;
        let new = match attrs {
            Some(a) => Some(self.fragment(&format!("<tabColor xmlns=\"{ns}\" {a}/>"))?),
            None => None,
        };
        if self.before_element("sheetPr").is_none() {
            let Some(new) = new else { return Ok(()) };
            let mut pr = self.fragment(&format!("<sheetPr xmlns=\"{ns}\"/>"))?;
            pr.children.push(RawXml::Element(new));
            self.put_before_element("sheetPr", Some(pr));
            return Ok(());
        }
        if let Some(pr) = self
            .before
            .iter_mut()
            .filter_map(RawXml::as_element_mut)
            .find(|e| e.head.is_main("sheetPr"))
        {
            pr.children
                .retain(|c| !c.as_element().is_some_and(|e| e.head.is_main("tabColor")));
            if let Some(new) = new {
                pr.children.insert(0, RawXml::Element(new));
            }
        }
        Ok(())
    }
}

impl super::Worksheet {
    /// Drops the revision id (`xr:uid`) of the worksheet element, which a copy must not share.
    pub(crate) fn strip_revision_uid(&mut self) {
        self.root_head.attrs.retain(|a| a.name != "xr:uid");
    }
}
