//! Resolving series references (`c:f`) to values.
//!
//! A [`CellSource`] answers "what is in this cell" for a sheet name. The workbook's own
//! stored values implement it ([`WorkbookCells`]); the calculation engine can implement it
//! later and chart values then follow recalculation. A reference the source cannot answer
//! (unknown sheet, unparsable formula, a literal series) falls back to the cache stored in
//! the chart part.

use open365_xlsx::{CellValue, Workbook, parse_range};

use crate::model::{DataRef, PointValue};

/// What a cell holds, as far as a chart cares.
#[derive(Debug, Clone, PartialEq)]
pub enum CellLookup {
    /// The source has no sheet of that name; the cache applies.
    UnknownSheet,
    Blank,
    Number(f64),
    Text(String),
    /// An error value; charts treat it as a gap.
    Error,
}

/// A source of cell values by sheet name and 1-based row and column.
pub trait CellSource {
    fn cell(&self, sheet: &str, row: u32, col: u32) -> CellLookup;
}

/// A source with no sheets: every series falls back to its cache.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoCells;

impl CellSource for NoCells {
    fn cell(&self, _: &str, _: u32, _: u32) -> CellLookup {
        CellLookup::UnknownSheet
    }
}

/// The stored (cached) cell values of an opened workbook.
#[derive(Debug, Clone, Copy)]
pub struct WorkbookCells<'a>(pub &'a Workbook);

impl CellSource for WorkbookCells<'_> {
    fn cell(&self, sheet: &str, row: u32, col: u32) -> CellLookup {
        let wb = self.0;
        let Some(index) = wb.sheet_index(sheet) else {
            return CellLookup::UnknownSheet;
        };
        match wb.cell(index, row, col).map(|c| &c.value) {
            None | Some(CellValue::Empty) => CellLookup::Blank,
            Some(CellValue::Number(n)) => CellLookup::Number(*n),
            Some(CellValue::Bool(b)) => CellLookup::Number(if *b { 1.0 } else { 0.0 }),
            Some(CellValue::Error(_)) => CellLookup::Error,
            Some(other) => other
                .as_text()
                .map_or(CellLookup::Blank, |t| CellLookup::Text(t.to_owned())),
        }
    }
}

/// One rectangular area of a reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AreaRef {
    pub sheet: String,
    pub row1: u32,
    pub col1: u32,
    pub row2: u32,
    pub col2: u32,
}

/// Splits `Sheet!A1:B2`, a quoted sheet name or `(Sheet1!A1:A3,Sheet1!C1:C3)` into areas.
/// `None` when any part is not a plain area reference.
pub fn parse_areas(formula: &str) -> Option<Vec<AreaRef>> {
    let f = formula.trim();
    let f = f
        .strip_prefix('(')
        .and_then(|x| x.strip_suffix(')'))
        .unwrap_or(f);
    let mut areas = Vec::new();
    for part in split_top_level(f) {
        areas.push(parse_area(part.trim())?);
    }
    (!areas.is_empty()).then_some(areas)
}

/// Splits on commas that are outside single quotes.
fn split_top_level(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut quoted) = (0, false);
    for (i, c) in s.char_indices() {
        match c {
            '\'' => quoted = !quoted,
            ',' if !quoted => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

fn parse_area(s: &str) -> Option<AreaRef> {
    let (sheet, cells) = if let Some(rest) = s.strip_prefix('\'') {
        let mut name = String::new();
        let mut chars = rest.char_indices().peekable();
        let mut end = None;
        while let Some((i, c)) = chars.next() {
            if c == '\'' {
                if matches!(chars.peek(), Some((_, '\''))) {
                    name.push('\'');
                    chars.next();
                } else {
                    end = Some(i);
                    break;
                }
            } else {
                name.push(c);
            }
        }
        let tail = rest[end? + 1..].strip_prefix('!')?;
        (name, tail)
    } else {
        let (sheet, cells) = s.split_once('!')?;
        (sheet.to_owned(), cells)
    };
    let plain: String = cells.chars().filter(|c| *c != '$').collect();
    let (row1, col1, row2, col2) = parse_range(&plain)?;
    Some(AreaRef {
        sheet,
        row1,
        col1,
        row2,
        col2,
    })
}

/// Largest number of points one series resolves to; larger references fall back to the cache.
const MAX_POINTS: u64 = 1 << 20;

/// The values of `data`: read from `cells` when the reference resolves, else the cache.
/// A wide area is read row by row and a tall one column by column; the areas of a
/// multi-area reference follow each other.
pub fn resolve(data: &DataRef, cells: &dyn CellSource) -> Vec<PointValue> {
    let Some(areas) = data.formula.as_deref().and_then(parse_areas) else {
        return data.cache.clone();
    };
    let total: u64 = areas
        .iter()
        .map(|a| u64::from(a.row2 - a.row1 + 1) * u64::from(a.col2 - a.col1 + 1))
        .sum();
    if total > MAX_POINTS {
        return data.cache.clone();
    }
    let mut out = Vec::with_capacity(total as usize);
    for area in &areas {
        let by_row = area.col2 - area.col1 > area.row2 - area.row1;
        let (outer, inner): (Vec<u32>, Vec<u32>) = if by_row {
            (
                (area.row1..=area.row2).collect(),
                (area.col1..=area.col2).collect(),
            )
        } else {
            (
                (area.col1..=area.col2).collect(),
                (area.row1..=area.row2).collect(),
            )
        };
        for o in &outer {
            for i in &inner {
                let (row, col) = if by_row { (*o, *i) } else { (*i, *o) };
                match cells.cell(&area.sheet, row, col) {
                    CellLookup::UnknownSheet => return data.cache.clone(),
                    CellLookup::Blank | CellLookup::Error => out.push(PointValue::Empty),
                    CellLookup::Number(n) if data.is_text => {
                        out.push(PointValue::Text(PointValue::Number(n).text()));
                    }
                    CellLookup::Number(n) => out.push(PointValue::Number(n)),
                    CellLookup::Text(t) if data.is_text => out.push(PointValue::Text(t)),
                    // Text in a numeric series plots as zero in Excel.
                    CellLookup::Text(_) => out.push(PointValue::Number(0.0)),
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_quoted_and_union_references() {
        let a = parse_areas("Data!$B$2:$B$6").unwrap();
        assert_eq!(
            a,
            vec![AreaRef {
                sheet: "Data".into(),
                row1: 2,
                col1: 2,
                row2: 6,
                col2: 2
            }]
        );
        let q = parse_areas("'It''s, a sheet'!A1").unwrap();
        assert_eq!(q[0].sheet, "It's, a sheet");
        assert_eq!((q[0].row1, q[0].col1, q[0].row2, q[0].col2), (1, 1, 1, 1));
        let u = parse_areas("(S!A1:A2,S!C1:C2)").unwrap();
        assert_eq!(u.len(), 2);
        assert!(parse_areas("SUM(A1)").is_none());
        assert!(parse_areas("Sheet1!NamedRange").is_none());
    }

    #[test]
    fn unknown_sheet_falls_back_to_cache() {
        let data = DataRef {
            formula: Some("Nope!A1:A2".into()),
            cache: vec![PointValue::Number(1.0), PointValue::Number(2.0)],
            ..DataRef::default()
        };
        assert_eq!(resolve(&data, &NoCells), data.cache);
    }
}
