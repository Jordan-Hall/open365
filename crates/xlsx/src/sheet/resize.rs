//! Column width and row height edits: `<cols><col min max width customWidth>` and
//! `<row ht customHeight>`. Every other attribute of an existing `col` or `row` is kept.

use super::{Attr, ElementHead, Row, Worksheet, XlsxError, XmlElement, format_number};
use crate::refs::{MAX_COL, MAX_ROW};
use crate::xml::RawXml;

/// Excel allows a column at most 255 characters wide and a row at most 409 points tall.
pub const MAX_COLUMN_WIDTH: f64 = 255.0;
pub const MAX_ROW_HEIGHT: f64 = 409.0;
/// Most rows one height edit creates `row` elements for.
const MAX_ROW_SPAN: u32 = 16_384;

/// The `cols` element as it was, enough to put it back.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnsSnapshot(Option<XmlElement>);

#[derive(Debug, Clone, PartialEq)]
struct SavedRow {
    r: u32,
    existed: bool,
    ht: Option<String>,
    custom_height: Option<String>,
}

/// The height attributes of a range of rows as they were.
#[derive(Debug, Clone, PartialEq)]
pub struct RowHeightsSnapshot(Vec<SavedRow>);

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
}

fn check_span(first: u32, last: u32, limit: u32, what: &str) -> Result<(), XlsxError> {
    if first < 1 || last < first || last > limit {
        return Err(XlsxError::UnsupportedEdit(format!(
            "{what} range {first}..={last} is outside 1..={limit}"
        )));
    }
    Ok(())
}

impl Worksheet {
    fn cols_index(&self) -> Option<usize> {
        self.before
            .iter()
            .position(|n| n.as_element().is_some_and(|e| e.head.is_main("cols")))
    }

    pub(crate) fn columns_snapshot(&self) -> ColumnsSnapshot {
        ColumnsSnapshot(
            self.cols_index()
                .and_then(|i| self.before[i].as_element().cloned()),
        )
    }

    pub(crate) fn restore_columns(&mut self, snapshot: &ColumnsSnapshot) {
        match (&snapshot.0, self.cols_index()) {
            (Some(el), Some(i)) => self.before[i] = RawXml::Element(el.clone()),
            (Some(el), None) => self.before.push(RawXml::Element(el.clone())),
            (None, Some(i)) => {
                self.before.remove(i);
            }
            (None, None) => {}
        }
    }

    /// Sets the width (characters, padding included) of columns `first..=last`, splitting
    /// `col` entries at the edges and merging neighbours that end up identical.
    pub(crate) fn set_column_width(
        &mut self,
        first: u32,
        last: u32,
        width: f64,
    ) -> Result<(), XlsxError> {
        check_span(first, last, MAX_COL, "column")?;
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
        let mut value = String::new();
        format_number(width, &mut value);

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
        let apply = |p: &mut Piece| {
            p.head.set_attr("min", &p.min.to_string());
            p.head.set_attr("max", &p.max.to_string());
            p.head.set_attr("width", &value);
            p.head.set_attr("customWidth", "1");
            p.touched = true;
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
            apply(&mut mid);
            covered.push((mid.min, mid.max));
            out.push(mid);
            if p.max > last {
                let mut right = p;
                right.min = last + 1;
                right.head.set_attr("min", &right.min.to_string());
                out.push(right);
            }
        }
        covered.sort_unstable();
        let mut next = first;
        for (a, b) in covered {
            if a > next {
                let mut gap = new_piece(next, a - 1);
                apply(&mut gap);
                out.push(gap);
            }
            next = next.max(b + 1);
        }
        if next <= last {
            let mut gap = new_piece(next, last);
            apply(&mut gap);
            out.push(gap);
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
            None => self.before.push(RawXml::Element(cols)),
        }
        Ok(())
    }

    pub(crate) fn row_heights_snapshot(&self, first: u32, last: u32) -> RowHeightsSnapshot {
        RowHeightsSnapshot(
            (first..=last)
                .map(|r| match self.find_row(r) {
                    Ok(i) => SavedRow {
                        r,
                        existed: true,
                        ht: self.rows[i].head.attr("ht").map(str::to_owned),
                        custom_height: self.rows[i].head.attr("customHeight").map(str::to_owned),
                    },
                    Err(_) => SavedRow {
                        r,
                        existed: false,
                        ht: None,
                        custom_height: None,
                    },
                })
                .collect(),
        )
    }

    pub(crate) fn restore_row_heights(&mut self, snapshot: &RowHeightsSnapshot) {
        for saved in &snapshot.0 {
            let Ok(i) = self.find_row(saved.r) else {
                continue;
            };
            let row = &mut self.rows[i];
            if !saved.existed && row.cells.is_empty() && row.extras.is_empty() {
                self.rows.remove(i);
                continue;
            }
            for (name, value) in [("ht", &saved.ht), ("customHeight", &saved.custom_height)] {
                match value {
                    Some(v) => row.head.set_attr(name, v),
                    None => row.head.remove_attr(name),
                }
            }
        }
    }

    /// Sets the height in points of rows `first..=last`, creating empty `row` elements where
    /// none exist.
    pub(crate) fn set_row_height(
        &mut self,
        first: u32,
        last: u32,
        points: f64,
    ) -> Result<(), XlsxError> {
        check_span(first, last, MAX_ROW, "row")?;
        if last - first >= MAX_ROW_SPAN {
            return Err(XlsxError::UnsupportedEdit(format!(
                "a height edit covers at most {MAX_ROW_SPAN} rows"
            )));
        }
        let mut value = String::new();
        format_number(points, &mut value);
        for r in first..=last {
            let i = match self.find_row(r) {
                Ok(i) => i,
                Err(i) => {
                    let mut head = self.sheet_data_head.sibling("row");
                    head.attrs.push(Attr::plain("r", &r.to_string()));
                    self.rows.insert(i, Row::new(r, head));
                    i
                }
            };
            let head = &mut self.rows[i].head;
            head.set_attr("ht", &value);
            head.set_attr("customHeight", "1");
        }
        Ok(())
    }
}
