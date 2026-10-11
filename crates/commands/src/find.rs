//! Find and replace in workbooks with Excel's semantics (`Range.Find`, `Range.Replace`).
//!
//! The rules were measured on Excel with `tools/office/find-oracle.ps1`:
//!
//! * `*` matches any run of characters (shortest first), `?` any one character, and `~` makes
//!   the next character literal. These work with every search, as in Excel.
//! * Case-insensitive unless `match_case`. `match_entire_cell` compares the whole cell text.
//! * Text searched: the formula text (`=SUM(A1:A3)`, or the constant as the formula bar shows
//!   it) with `LookIn::Formulas`, the displayed text with `LookIn::Values`. Replace always works
//!   on the formula text, as Excel's replace does.
//! * A replacement is entered as if typed: `=` starts a formula, a number becomes a number.
//!
//! Word's find lives in `open365_docx` ([`FindOptions`]).

use open365_calc::Value;
use open365_calc::numfmt::{builtin_format_code, format_value};
use open365_xlsx::{CellValue, SheetKind, Workbook, format_number};
use serde::{Deserialize, Serialize};

pub use open365_docx::{DocMatch, FindError, FindOptions};

use crate::command::CellRef;

/// Where a workbook search looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum FindScope {
    /// One worksheet (0-based index).
    Sheet { sheet: usize },
    /// Every worksheet, in tab order.
    Workbook,
}

/// What a find compares against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LookIn {
    /// Formula text, or the constant as typed.
    #[default]
    Formulas,
    /// Displayed text (the value through the cell's number format).
    Values,
}

/// An Excel find request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XlsxFind {
    pub text: String,
    pub scope: FindScope,
    /// Visit cells column by column instead of row by row.
    #[serde(default)]
    pub by_columns: bool,
    #[serde(default)]
    pub look_in: LookIn,
    #[serde(default)]
    pub match_case: bool,
    #[serde(default)]
    pub match_entire_cell: bool,
}

impl XlsxFind {
    pub fn new(text: impl Into<String>, scope: FindScope) -> Self {
        Self {
            text: text.into(),
            scope,
            by_columns: false,
            look_in: LookIn::Formulas,
            match_case: false,
            match_entire_cell: false,
        }
    }
}

/// One cell a replace-all changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedEdit {
    pub cell: CellRef,
    /// The cell as typed after the replacement.
    pub text: String,
    /// Replacements made inside the cell.
    pub occurrences: usize,
}

/// A number the way the formula bar shows it: up to 15 significant digits.
fn number_text(v: f64) -> String {
    let rounded = format!("{v:.14e}")
        .parse::<f64>()
        .ok()
        .filter(|r| r.is_finite())
        .unwrap_or(v);
    let mut out = String::new();
    format_number(rounded, &mut out);
    out
}

/// The cell as the formula bar shows it: `=formula`, or the constant.
pub fn cell_source_text(wb: &Workbook, sheet: usize, row: u32, col: u32) -> String {
    if let Some(f) = wb.formula_text(sheet, row, col) {
        return format!("={f}");
    }
    match wb.cell(sheet, row, col).map(|c| &c.value) {
        None | Some(CellValue::Empty) => String::new(),
        Some(CellValue::Number(n)) => number_text(*n),
        Some(CellValue::Bool(b)) => if *b { "TRUE" } else { "FALSE" }.to_owned(),
        Some(CellValue::Error(e)) => e.code().to_owned(),
        Some(other) => other.as_text().unwrap_or_default().to_owned(),
    }
}

/// The text a cell displays: its value through its number format.
pub fn cell_display_text(wb: &Workbook, sheet: usize, row: u32, col: u32) -> String {
    let Some(cell) = wb.cell(sheet, row, col) else {
        return String::new();
    };
    let styles = wb.styles();
    let id = styles.num_fmt_id(cell.style.unwrap_or(0));
    let code = if styles.custom_formats().contains_key(&id) {
        styles.format_code(id)
    } else {
        builtin_format_code(id).unwrap_or_else(|| styles.format_code(id))
    };
    match &cell.value {
        CellValue::Empty => String::new(),
        CellValue::Number(n) => format_value(&Value::Number(*n), code, wb.date1904()),
        CellValue::Bool(true) => "TRUE".to_owned(),
        CellValue::Bool(false) => "FALSE".to_owned(),
        CellValue::Error(e) => e.code().to_owned(),
        other => {
            let text = other.as_text().unwrap_or_default();
            if code == "General" || code.is_empty() {
                text.to_owned()
            } else {
                format_value(&Value::Text(text.to_owned()), code, wb.date1904())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    Lit(char),
    Any,
    Star,
}

fn tokens(pattern: &str, match_case: bool) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut chars = pattern.chars();
    let fold = |c: char| if match_case { c } else { fold_char(c) };
    while let Some(c) = chars.next() {
        out.push(match c {
            '*' => Tok::Star,
            '?' => Tok::Any,
            '~' => match chars.next() {
                Some(n) => Tok::Lit(fold(n)),
                None => Tok::Lit('~'),
            },
            other => Tok::Lit(fold(other)),
        });
    }
    out
}

fn fold_char(c: char) -> char {
    let mut it = c.to_lowercase();
    match (it.next(), it.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

/// End of the shortest match of `toks` starting at `text[pos]`; with `whole` it must reach the
/// end of the text.
fn match_from(
    toks: &[Tok],
    text: &[char],
    pos: usize,
    whole: bool,
    match_case: bool,
) -> Option<usize> {
    let Some((first, rest)) = toks.split_first() else {
        return (!whole || pos == text.len()).then_some(pos);
    };
    match first {
        Tok::Star => {
            (pos..=text.len()).find_map(|end| match_from(rest, text, end, whole, match_case))
        }
        Tok::Any => {
            (pos < text.len()).then(|| match_from(rest, text, pos + 1, whole, match_case))?
        }
        Tok::Lit(l) => {
            let c = *text.get(pos)?;
            let c = if match_case { c } else { fold_char(c) };
            (c == *l).then(|| match_from(rest, text, pos + 1, whole, match_case))?
        }
    }
}

/// Non-overlapping matches of `pattern` in `text` as character ranges. With `entire` the
/// only possible match is the whole text.
pub fn excel_matches(
    text: &str,
    pattern: &str,
    match_case: bool,
    entire: bool,
) -> Vec<(usize, usize)> {
    if pattern.is_empty() {
        return Vec::new();
    }
    let toks = tokens(pattern, match_case);
    let chars: Vec<char> = text.chars().collect();
    if entire {
        return match_from(&toks, &chars, 0, true, match_case)
            .map(|end| vec![(0, end)])
            .unwrap_or_default();
    }
    let mut out = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        match match_from(&toks, &chars, start, false, match_case) {
            Some(end) if end > start => {
                out.push((start, end));
                start = end;
            }
            _ => start += 1,
        }
    }
    out
}

fn sheet_indexes(wb: &Workbook, scope: FindScope) -> Vec<usize> {
    match scope {
        FindScope::Sheet { sheet } => vec![sheet],
        FindScope::Workbook => (0..wb.sheets().len()).collect(),
    }
}

fn is_worksheet(wb: &Workbook, sheet: usize) -> bool {
    wb.sheets()
        .get(sheet)
        .is_some_and(|s| s.kind == SheetKind::Worksheet)
}

/// Cells that exist on `sheet`, in the requested visiting order.
fn cells_in_order(wb: &Workbook, sheet: usize, by_columns: bool) -> Vec<(u32, u32)> {
    let Some(ws) = wb.sheet(sheet).filter(|_| is_worksheet(wb, sheet)) else {
        return Vec::new();
    };
    let mut cells: Vec<(u32, u32)> = ws
        .iter_cells()
        .filter(|(_, _, c)| !matches!(c.value, CellValue::Empty) || c.formula.is_some())
        .map(|(r, c, _)| (r, c))
        .collect();
    if by_columns {
        cells.sort_by_key(|&(r, c)| (c, r));
    }
    cells
}

/// Every cell that matches, in the search order.
pub fn xlsx_find_all(wb: &Workbook, find: &XlsxFind) -> Vec<CellRef> {
    let mut out = Vec::new();
    for sheet in sheet_indexes(wb, find.scope) {
        for (row, col) in cells_in_order(wb, sheet, find.by_columns) {
            let text = match find.look_in {
                LookIn::Formulas => cell_source_text(wb, sheet, row, col),
                LookIn::Values => cell_display_text(wb, sheet, row, col),
            };
            if !excel_matches(&text, &find.text, find.match_case, find.match_entire_cell).is_empty()
            {
                out.push(CellRef { sheet, row, col });
            }
        }
    }
    out
}

/// The match after (`forward`) or before `from` in `hits` (which is in search order), wrapping
/// around. `None` when there are no matches.
pub fn next_match(
    hits: &[CellRef],
    from: Option<CellRef>,
    forward: bool,
    by_columns: bool,
) -> Option<CellRef> {
    let key = |c: &CellRef| {
        if by_columns {
            (c.sheet, c.col, c.row)
        } else {
            (c.sheet, c.row, c.col)
        }
    };
    let first = hits.first().copied();
    let last = hits.last().copied();
    let Some(from) = from else {
        return if forward { first } else { last };
    };
    let k = key(&from);
    if forward {
        hits.iter().find(|h| key(h) > k).copied().or(first)
    } else {
        hits.iter().rev().find(|h| key(h) < k).copied().or(last)
    }
}

/// The cells a replace-all would change and what they would hold. Works on the formula text.
pub fn xlsx_replace_plan(wb: &Workbook, find: &XlsxFind, replacement: &str) -> Vec<PlannedEdit> {
    let mut out = Vec::new();
    for sheet in sheet_indexes(wb, find.scope) {
        for (row, col) in cells_in_order(wb, sheet, find.by_columns) {
            // A non-master cell of a legacy array formula has no text of its own.
            if wb
                .cell(sheet, row, col)
                .is_some_and(|c| c.formula.is_some())
                && wb.formula_text(sheet, row, col).is_none()
            {
                continue;
            }
            let text = cell_source_text(wb, sheet, row, col);
            let hits = excel_matches(&text, &find.text, find.match_case, find.match_entire_cell);
            if hits.is_empty() {
                continue;
            }
            let chars: Vec<char> = text.chars().collect();
            let mut result = String::new();
            let mut at = 0;
            for &(s, e) in &hits {
                result.extend(&chars[at..s]);
                result.push_str(replacement);
                at = e;
            }
            result.extend(&chars[at..]);
            out.push(PlannedEdit {
                cell: CellRef { sheet, row, col },
                text: result,
                occurrences: hits.len(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_follows_excel() {
        assert_eq!(excel_matches("Apple apple", "apple", false, false).len(), 2);
        assert_eq!(
            excel_matches("Apple apple", "apple", true, false),
            [(6, 11)]
        );
        assert_eq!(excel_matches("apple", "app", false, true), []);
        assert_eq!(excel_matches("apple", "a*e", false, true), [(0, 5)]);
        assert_eq!(excel_matches("a1b", "a?b", false, true), [(0, 3)]);
        assert_eq!(excel_matches("a*b", "a~*b", false, true), [(0, 3)]);
        assert_eq!(excel_matches("axb", "a~*b", false, true), []);
    }

    #[test]
    fn next_match_wraps() {
        let c = |row, col| CellRef { sheet: 0, row, col };
        let hits = [c(1, 1), c(2, 2), c(3, 1)];
        assert_eq!(next_match(&hits, Some(c(2, 2)), true, false), Some(c(3, 1)));
        assert_eq!(next_match(&hits, Some(c(3, 1)), true, false), Some(c(1, 1)));
        assert_eq!(
            next_match(&hits, Some(c(1, 1)), false, false),
            Some(c(3, 1))
        );
        assert_eq!(next_match(&hits, None, true, false), Some(c(1, 1)));
    }

    #[test]
    fn numbers_read_like_the_formula_bar() {
        assert_eq!(number_text(0.1 + 0.2), "0.3");
        assert_eq!(number_text(12.0), "12");
        assert_eq!(number_text(1.5), "1.5");
    }
}
