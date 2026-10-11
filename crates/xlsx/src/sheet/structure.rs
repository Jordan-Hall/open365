//! The worksheet part of inserting and deleting whole rows and columns: moving rows and cells, the `cols`
//! element and the range-bearing elements around `sheetData` (merges, conditional formats, validation,
//! hyperlinks, filters, breaks), and rewriting formulas.

use super::{FormulaKind, RawXml, Worksheet, XlsxError, XmlElement};
use crate::refs::parse_range;
use crate::structure::{
    EditAxis, FormulaShifter, StructEdit, shift_range_text, shift_sqref, shift_sqref_formats,
};

/// Local names of elements whose text is a formula, outside `sheetData`.
const FORMULA_ELEMENTS: [&str; 4] = ["formula", "formula1", "formula2", "f"];

use crate::structure::range_a1 as range_text;

fn set_text(el: &mut XmlElement, text: &str) {
    el.children = vec![RawXml::Text(text.to_owned())];
}

impl Worksheet {
    /// Turns every shared-formula group into normal formulas with their expanded text, so each formula can be
    /// rewritten on its own.
    pub(crate) fn materialise_all_shared(&mut self) {
        let mut targets: Vec<(u32, u32, String)> = Vec::new();
        for row in &self.rows {
            for cell in &row.cells {
                if let Some(f) = cell.formula.as_deref()
                    && matches!(f.kind, FormulaKind::Shared { .. })
                    && let Some(text) = self.formula_text(row.r, cell.col)
                {
                    targets.push((row.r, cell.col, text));
                }
            }
        }
        for (r, c, text) in targets {
            if let Ok(ri) = self.find_row(r)
                && let Ok(ci) = self.rows[ri].cells.binary_search_by_key(&c, |x| x.col)
                && let Some(f) = self.rows[ri].cells[ci].formula.as_deref_mut()
            {
                f.text = text;
                f.kind = FormulaKind::Normal;
            }
        }
        self.index_formulas();
    }

    /// Rewrites the text of every formula with `shifter`; `home` is this sheet's name. Returns the
    /// `(row, col)` of each formula whose text changed.
    pub(crate) fn shift_cell_formulas(
        &mut self,
        shifter: &dyn FormulaShifter,
        home: &str,
    ) -> Vec<(u32, u32)> {
        let mut changed = Vec::new();
        for row in &mut self.rows {
            for cell in &mut row.cells {
                let Some(f) = cell.formula.as_deref_mut() else {
                    continue;
                };
                if f.text.is_empty() {
                    continue;
                }
                if let Some(new) = shifter.shift(&f.text, home) {
                    f.text = new;
                    changed.push((row.r, cell.col));
                }
            }
        }
        self.index_formulas();
        changed
    }

    /// `(row, col)` of every formula cell and every other cell of an array formula.
    pub(crate) fn formula_positions(&self) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        for row in &self.rows {
            for cell in &row.cells {
                if cell.formula.is_some() || self.array_formula_at(row.r, cell.col).is_some() {
                    out.push((row.r, cell.col));
                }
            }
        }
        out
    }

    /// Replaces the cached result of an existing cell, keeping its formula, style and array membership.
    pub(crate) fn set_cached(&mut self, row: u32, col: u32, value: super::CellValue) -> bool {
        let Ok(ri) = self.find_row(row) else {
            return false;
        };
        let Ok(ci) = self.rows[ri].cells.binary_search_by_key(&col, |c| c.col) else {
            return false;
        };
        let cell = &mut self.rows[ri].cells[ci];
        cell.value = value;
        if let Some(extra) = cell.extra.as_deref_mut() {
            extra.raw_v = None;
            extra.v_attrs.clear();
            extra.t_override = None;
            extra.inline = None;
        }
        true
    }

    /// Rewrites the formulas of conditional formats, validation and extension lists. `anchored` is the edit
    /// when this is the edited sheet: relative references there are offsets from the top-left cell of the
    /// range the formula applies to.
    pub(crate) fn shift_raw_formulas(
        &mut self,
        shifter: &dyn FormulaShifter,
        home: &str,
        anchored: Option<&StructEdit>,
    ) {
        fn walk(
            el: &mut XmlElement,
            shifter: &dyn FormulaShifter,
            home: &str,
            anchored: Option<&StructEdit>,
            delta: Option<i64>,
        ) {
            let delta = match (anchored, el.head.attr("sqref")) {
                (Some(edit), Some(sqref)) => Some(crate::structure::anchor_delta(sqref, edit)),
                _ => delta,
            };
            if FORMULA_ELEMENTS.contains(&el.head.local_name()) {
                let text = el.own_text();
                let text = text.trim_start_matches('=');
                let new = match delta {
                    Some(d) => shifter.shift_anchored(text, home, d),
                    None => shifter.shift(text, home),
                };
                if let Some(new) = new {
                    set_text(el, &new);
                }
                return;
            }
            for child in &mut el.children {
                if let RawXml::Element(c) = child {
                    walk(c, shifter, home, anchored, delta);
                }
            }
        }
        for node in self.before.iter_mut().chain(self.after.iter_mut()) {
            if let RawXml::Element(e) = node {
                walk(e, shifter, home, anchored, None);
            }
        }
    }

    /// Fails with the message Excel gives when the edit cannot be done on this sheet.
    pub(crate) fn check_structure(&self, edit: &StructEdit) -> Result<(), XlsxError> {
        let (at, end) = edit.span();
        let other = |msg: &str| Err(XlsxError::UnsupportedEdit(msg.to_owned()));
        for &rect in &self.locked {
            let (lo, hi) = match edit.axis {
                EditAxis::Rows => (rect.0, rect.2),
                EditAxis::Cols => (rect.1, rect.3),
            };
            let cuts = if edit.delete {
                lo <= end && hi >= at && !(lo >= at && hi <= end)
            } else {
                lo < at && hi >= at
            };
            if cuts {
                return other("you can't change part of an array or data table");
            }
        }
        if !edit.delete {
            let limit = edit.limit();
            let last = match edit.axis {
                EditAxis::Rows => self
                    .rows
                    .iter()
                    .rev()
                    .find(|r| !r.cells.is_empty())
                    .map_or(0, |r| r.r),
                EditAxis::Cols => self
                    .rows
                    .iter()
                    .filter_map(|r| r.cells.last().map(|c| c.col))
                    .max()
                    .unwrap_or(0),
            };
            if last >= at && last.saturating_add(edit.count) > limit {
                return other("inserting would push cells off the end of the worksheet");
            }
        } else if edit.axis == EditAxis::Rows {
            for t in &self.tables {
                let Some((r1, _, r2, _)) = t.rect else {
                    continue;
                };
                let header = t.header_row_count > 0 && (at..=end).contains(&r1);
                let whole = at <= r1 && end >= r2;
                if header && !whole {
                    return other(
                        "you can't delete the header row of a table without deleting the table",
                    );
                }
            }
        }
        Ok(())
    }

    /// Moves rows, cells and the range-bearing elements for `edit`. Formulas are rewritten separately.
    pub(crate) fn move_structure(&mut self, edit: &StructEdit) {
        match edit.axis {
            EditAxis::Rows => self.move_rows(edit),
            EditAxis::Cols => {
                self.move_cells_in_rows(edit);
                self.fix_cols_element(edit);
            }
        }
        self.fix_range_elements(edit);
        self.fix_array_ranges(edit);
        self.fix_dimension();
        self.index_formulas();
    }

    fn move_rows(&mut self, edit: &StructEdit) {
        let (at, end) = edit.span();
        if edit.delete {
            self.rows.retain(|r| !(at..=end).contains(&r.r));
        }
        for row in &mut self.rows {
            if let Some(new) = edit.map_index(row.r)
                && new != row.r
            {
                row.r = new;
                row.head.set_attr("r", &new.to_string());
            }
        }
    }

    fn move_cells_in_rows(&mut self, edit: &StructEdit) {
        for row in &mut self.rows {
            let before = row.cells.len();
            row.cells.retain_mut(|c| match edit.map_index(c.col) {
                Some(n) => {
                    c.col = n;
                    true
                }
                None => false,
            });
            // `spans` is only an optimisation hint; the moved cells invalidate it.
            if before != 0 {
                row.head.remove_attr("spans");
            }
        }
    }

    fn fix_cols_element(&mut self, edit: &StructEdit) {
        let Some(i) = self
            .before
            .iter()
            .position(|n| n.as_element().is_some_and(|e| e.head.is_main("cols")))
        else {
            return;
        };
        let Some(cols) = self.before[i].as_element_mut() else {
            return;
        };
        cols.children.retain_mut(|child| {
            let RawXml::Element(c) = child else {
                return true;
            };
            if !c.head.is_main("col") {
                return true;
            }
            let (Some(min), Some(max)) = (
                c.head.attr("min").and_then(|v| v.parse::<u32>().ok()),
                c.head.attr("max").and_then(|v| v.parse::<u32>().ok()),
            ) else {
                return true;
            };
            match edit.map_range(min, max) {
                Some((lo, hi)) => {
                    c.head.set_attr("min", &lo.to_string());
                    c.head.set_attr("max", &hi.to_string());
                    true
                }
                None => false,
            }
        });
        if !cols.children.iter().any(|c| c.as_element().is_some()) {
            self.before.remove(i);
        }
    }

    fn fix_array_ranges(&mut self, edit: &StructEdit) {
        for row in &mut self.rows {
            for cell in &mut row.cells {
                let Some(f) = cell.formula.as_deref_mut() else {
                    continue;
                };
                match &mut f.kind {
                    FormulaKind::Array { range: Some(range) } => {
                        if let Some(rect) = parse_range(range).and_then(|r| edit.map_rect(r)) {
                            *range = range_text(rect);
                        }
                    }
                    FormulaKind::DataTable => {
                        for a in &mut f.attrs {
                            if a.ns.is_none()
                                && a.name == "ref"
                                && let Some(rect) =
                                    parse_range(&a.value).and_then(|r| edit.map_rect(r))
                            {
                                a.value = range_text(rect);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// `dimension` is advisory; make it match the cells again.
    fn fix_dimension(&mut self) {
        let used = self.used_range();
        if let Some(dim) = self
            .before
            .iter_mut()
            .filter_map(RawXml::as_element_mut)
            .find(|e| e.head.is_main("dimension"))
        {
            let text = used.map_or_else(|| "A1".to_owned(), range_text);
            dim.head.set_attr("ref", &text);
        }
    }

    fn fix_range_elements(&mut self, edit: &StructEdit) {
        let mut before = std::mem::take(&mut self.before);
        let mut after = std::mem::take(&mut self.after);
        for list in [&mut before, &mut after] {
            list.retain_mut(|node| match node {
                RawXml::Element(e) => fix_element(e, edit),
                _ => true,
            });
        }
        self.before = before;
        self.after = after;
    }

    /// Removes the `tablePart` with relationship id `rid` (and `tableParts` when it was the last).
    pub(crate) fn remove_table_part(&mut self, rid: &str) {
        let Some(i) = self
            .after
            .iter()
            .position(|n| n.as_element().is_some_and(|e| e.head.is_main("tableParts")))
        else {
            return;
        };
        let Some(parts) = self.after[i].as_element_mut() else {
            return;
        };
        parts.children.retain(|c| {
            c.as_element()
                .is_none_or(|p| p.head.attr_r("id") != Some(rid))
        });
        let n = parts
            .children
            .iter()
            .filter(|c| c.as_element().is_some())
            .count();
        if n == 0 {
            self.after.remove(i);
        } else {
            parts.head.set_attr("count", &n.to_string());
        }
    }

    /// Drops the table models whose part was removed.
    pub(crate) fn retain_tables(&mut self, keep: impl FnMut(&crate::tables::Table) -> bool) {
        self.tables.retain(keep);
    }

    pub(crate) fn tables_mut(&mut self) -> &mut Vec<crate::tables::Table> {
        &mut self.tables
    }
}

/// Rewrites the ranges of one top-level element; false removes the element.
fn fix_element(e: &mut XmlElement, edit: &StructEdit) -> bool {
    let is_row_axis = edit.axis == EditAxis::Rows;
    match e.head.local_name() {
        "mergeCells" => {
            e.children.retain_mut(|c| {
                let RawXml::Element(m) = c else { return true };
                let Some(r) = m.head.attr("ref").map(str::to_owned) else {
                    return true;
                };
                match shift_range_text(&r, edit) {
                    // A merge cut down to one cell is gone.
                    Some(new) if new.contains(':') && !same_corners(&new) => {
                        m.head.set_attr("ref", &new);
                        true
                    }
                    _ => false,
                }
            });
            let n = e
                .children
                .iter()
                .filter(|c| c.as_element().is_some())
                .count();
            if n == 0 {
                return false;
            }
            e.head.set_attr("count", &n.to_string());
        }
        "conditionalFormatting" => match e
            .head
            .attr("sqref")
            .and_then(|s| shift_sqref_formats(s, edit))
        {
            Some(new) => e.head.set_attr("sqref", &new),
            None => return false,
        },
        "dataValidations" | "ignoredErrors" | "protectedRanges" => {
            let formats = e.head.local_name() == "dataValidations";
            e.children.retain_mut(|c| {
                let RawXml::Element(m) = c else { return true };
                let moved = m.head.attr("sqref").and_then(|s| {
                    if formats {
                        shift_sqref_formats(s, edit)
                    } else {
                        shift_sqref(s, edit)
                    }
                });
                match moved {
                    Some(new) => {
                        m.head.set_attr("sqref", &new);
                        true
                    }
                    None => m.head.attr("sqref").is_none(),
                }
            });
            let n = e
                .children
                .iter()
                .filter(|c| c.as_element().is_some())
                .count();
            if n == 0 {
                return false;
            }
            if e.head.attr("count").is_some() {
                e.head.set_attr("count", &n.to_string());
            }
        }
        "hyperlinks" => {
            e.children.retain_mut(|c| {
                let RawXml::Element(h) = c else { return true };
                match h.head.attr("ref").and_then(|s| shift_range_text(s, edit)) {
                    Some(new) => {
                        h.head.set_attr("ref", &new);
                        true
                    }
                    None => false,
                }
            });
            if !e.children.iter().any(|c| c.as_element().is_some()) {
                return false;
            }
        }
        "autoFilter" | "sortState" => {
            match e.head.attr("ref").and_then(|s| shift_range_text(s, edit)) {
                Some(new) => e.head.set_attr("ref", &new),
                None => return false,
            }
            for c in &mut e.children {
                if let RawXml::Element(s) = c
                    && s.head.is_main("sortCondition")
                    && let Some(new) = s.head.attr("ref").and_then(|r| shift_range_text(r, edit))
                {
                    s.head.set_attr("ref", &new);
                }
            }
        }
        "rowBreaks" | "colBreaks" => {
            if is_row_axis != (e.head.local_name() == "rowBreaks") {
                return true;
            }
            e.children.retain_mut(|c| {
                let RawXml::Element(b) = c else { return true };
                match b
                    .head
                    .attr("id")
                    .and_then(|v| v.parse::<u32>().ok())
                    .and_then(|id| edit.map_index(id))
                {
                    Some(new) => {
                        b.head.set_attr("id", &new.to_string());
                        true
                    }
                    None => false,
                }
            });
            let n = e
                .children
                .iter()
                .filter(|c| c.as_element().is_some())
                .count();
            if n == 0 {
                return false;
            }
            e.head.set_attr("count", &n.to_string());
            if e.head.attr("manualBreakCount").is_some() {
                e.head.set_attr("manualBreakCount", &n.to_string());
            }
        }
        "extLst" => fix_ext(e, edit),
        _ => {}
    }
    true
}

fn same_corners(range: &str) -> bool {
    range
        .split_once(':')
        .is_some_and(|(a, b)| a.replace('$', "") == b.replace('$', ""))
}

/// `xm:sqref` elements of extension lists (x14 conditional formats, validation, sparklines).
fn fix_ext(e: &mut XmlElement, edit: &StructEdit) {
    for child in &mut e.children {
        let RawXml::Element(c) = child else { continue };
        if c.head.local_name() == "sqref" {
            let text = c.own_text();
            if let Some(new) = shift_sqref(&text, edit) {
                set_text(c, &new);
            }
        } else {
            fix_ext(c, edit);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_cut_to_one_cell_is_dropped() {
        assert!(same_corners("A1:A1"));
        assert!(!same_corners("A1:B1"));
    }
}
