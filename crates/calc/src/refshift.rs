//! Reference adjustment for inserting and deleting whole rows or columns, as Excel does it.
//!
//! The rules are those measured on Excel (`tools/office/insdel-oracle.ps1`):
//!
//! * Inserting at `at` moves every row (column) `>= at` by `count`. A range whose first row is `>= at` moves
//!   as a whole; a range that starts above `at` and reaches it or beyond grows; a range that ends above `at`
//!   stays. Absolute (`$`) and relative references behave alike.
//! * Deleting `at..at+count` turns a single cell inside it into `#REF!`. A range loses the deleted rows
//!   (columns) and moves up (left) past them; a range wholly inside becomes `#REF!`. A reference qualified with
//!   a sheet becomes `Sheet!#REF!` ([`Ast::DeadRef`]).
//! * Only references that point at the edited sheet change: an unqualified reference points at the sheet the
//!   formula lives on.

use crate::ast::{AreaKind, AreaRef, Ast, CellRef, MAX_COL, MAX_ROW, to_formula_string};
use crate::parser::{ParseError, parse_formula};
use crate::value::ErrorKind;

/// Rows or columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Whole rows.
    Row,
    /// Whole columns.
    Col,
}

/// Insert or delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    /// New blank rows or columns; what was there moves down or right.
    Insert,
    /// Rows or columns removed; what followed moves up or left.
    Delete,
}

/// One structural edit of a sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuralEdit {
    /// Name of the edited sheet.
    pub sheet: String,
    /// Rows or columns.
    pub axis: Axis,
    /// Insert or delete.
    pub kind: EditKind,
    /// First row or column affected (1-based).
    pub at: u32,
    /// How many rows or columns.
    pub count: u32,
}

impl StructuralEdit {
    /// Last row or column of the grid on this axis.
    pub fn limit(&self) -> u32 {
        match self.axis {
            Axis::Row => MAX_ROW,
            Axis::Col => MAX_COL,
        }
    }

    /// Last deleted index (`at + count - 1`).
    fn end(&self) -> u32 {
        self.at.saturating_add(self.count).saturating_sub(1)
    }

    /// Where the row or column `index` ends up; `None` when a delete removes it (or an insert pushes it off the grid).
    pub fn map_index(&self, index: u32) -> Option<u32> {
        match self.kind {
            EditKind::Insert => {
                if index < self.at {
                    Some(index)
                } else {
                    let moved = index.checked_add(self.count)?;
                    (moved <= self.limit()).then_some(moved)
                }
            }
            EditKind::Delete => {
                if index < self.at {
                    Some(index)
                } else if index <= self.end() {
                    None
                } else {
                    Some(index - self.count)
                }
            }
        }
    }

    /// Where the range `lo..=hi` ends up (`lo <= hi`); `None` when a delete removes all of it.
    pub fn map_range(&self, lo: u32, hi: u32) -> Option<(u32, u32)> {
        match self.kind {
            EditKind::Insert => {
                if lo >= self.at {
                    let new_lo = lo.checked_add(self.count)?;
                    if new_lo > self.limit() {
                        return None;
                    }
                    Some((new_lo, hi.saturating_add(self.count).min(self.limit())))
                } else if hi >= self.at {
                    Some((lo, hi.saturating_add(self.count).min(self.limit())))
                } else {
                    Some((lo, hi))
                }
            }
            EditKind::Delete => {
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
        }
    }
}

/// How relative references are treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Every reference follows the cells it points at.
    Plain,
    /// Relative coordinates are offsets from an anchor cell (the top-left cell of a conditional format or
    /// validation range; `A1` for a defined name). They move with the anchor, by `delta` on the edited axis;
    /// `$`-anchored coordinates follow the cells as usual.
    Anchored(i64),
    /// Relative coordinates never move (defined names: offsets from `A1`).
    Fixed,
}

/// True when a reference qualified with `qualifier` (or unqualified, on `home`) points at the edited sheet.
fn targets(edit: &StructuralEdit, home: Option<&str>, qualifier: Option<&str>) -> bool {
    qualifier
        .or(home)
        .is_some_and(|s| s.eq_ignore_ascii_case(&edit.sheet))
}

fn dead(sheet: Option<&str>) -> Ast {
    match sheet {
        Some(s) => Ast::DeadRef {
            sheet: s.to_owned(),
        },
        None => Ast::Error(ErrorKind::Ref),
    }
}

fn shift_cell_ref(r: &CellRef, home: Option<&str>, edit: &StructuralEdit, mode: Mode) -> Ast {
    if !targets(edit, home, r.sheet.as_deref()) {
        return Ast::Ref(r.clone());
    }
    let mut out = r.clone();
    let (slot, absolute) = match edit.axis {
        Axis::Row => (&mut out.row, r.row_abs),
        Axis::Col => (&mut out.col, r.col_abs),
    };
    if mode == Mode::Fixed && !absolute {
        return Ast::Ref(out);
    }
    if let Mode::Anchored(delta) = mode
        && !absolute
    {
        let moved = edit
            .map_index(*slot)
            .or_else(|| offset(*slot, delta, edit.limit()));
        return match moved {
            Some(i) => {
                *slot = i;
                Ast::Ref(out)
            }
            None => dead(r.sheet.as_deref()),
        };
    }
    match edit.map_index(*slot) {
        Some(i) => {
            *slot = i;
            Ast::Ref(out)
        }
        None => dead(r.sheet.as_deref()),
    }
}

fn offset(value: u32, delta: i64, limit: u32) -> Option<u32> {
    let moved = i64::from(value) + delta;
    u32::try_from(moved)
        .ok()
        .filter(|v| (1..=limit).contains(v))
}

fn shift_area_ref(a: &AreaRef, home: Option<&str>, edit: &StructuralEdit, mode: Mode) -> Ast {
    let affected = !matches!(
        (a.kind, edit.axis),
        (AreaKind::Cols, Axis::Row) | (AreaKind::Rows, Axis::Col)
    );
    if !affected || !targets(edit, home, a.sheet.as_deref()) {
        return Ast::Area(a.clone());
    }
    let mut out = a.clone();
    if mode == Mode::Fixed {
        let (abs1, abs2) = match edit.axis {
            Axis::Row => (a.row1_abs, a.row2_abs),
            Axis::Col => (a.col1_abs, a.col2_abs),
        };
        if !abs1 && !abs2 {
            return Ast::Area(out);
        }
        if abs1 != abs2 {
            let (c1, c2) = match edit.axis {
                Axis::Row => (&mut out.row1, &mut out.row2),
                Axis::Col => (&mut out.col1, &mut out.col2),
            };
            for (c, abs) in [(c1, abs1), (c2, abs2)] {
                if abs {
                    match edit.map_index(*c) {
                        Some(i) => *c = i,
                        None => return dead(a.sheet.as_deref()),
                    }
                }
            }
            return Ast::Area(out);
        }
    }
    if let Mode::Anchored(delta) = mode {
        let (abs1, abs2) = match edit.axis {
            Axis::Row => (a.row1_abs, a.row2_abs),
            Axis::Col => (a.col1_abs, a.col2_abs),
        };
        if !(abs1 && abs2) {
            // Corners move on their own: fixed ones with the cells, relative ones with the anchor.
            let (c1, c2) = match edit.axis {
                Axis::Row => (&mut out.row1, &mut out.row2),
                Axis::Col => (&mut out.col1, &mut out.col2),
            };
            for (c, abs) in [(c1, abs1), (c2, abs2)] {
                let moved = edit
                    .map_index(*c)
                    .or_else(|| (!abs).then(|| offset(*c, delta, edit.limit())).flatten());
                match moved {
                    Some(i) => *c = i,
                    None => return dead(a.sheet.as_deref()),
                }
            }
            return Ast::Area(out);
        }
    }
    // Excel writes the corners in order, whatever order they were typed.
    let (lo, hi) = match edit.axis {
        Axis::Row => {
            if out.row1 > out.row2 {
                std::mem::swap(&mut out.row1, &mut out.row2);
                std::mem::swap(&mut out.row1_abs, &mut out.row2_abs);
            }
            (out.row1, out.row2)
        }
        Axis::Col => {
            if out.col1 > out.col2 {
                std::mem::swap(&mut out.col1, &mut out.col2);
                std::mem::swap(&mut out.col1_abs, &mut out.col2_abs);
            }
            (out.col1, out.col2)
        }
    };
    let Some((new_lo, new_hi)) = edit.map_range(lo, hi) else {
        return dead(a.sheet.as_deref());
    };
    match edit.axis {
        Axis::Row => {
            out.row1 = new_lo;
            out.row2 = new_hi;
        }
        Axis::Col => {
            out.col1 = new_lo;
            out.col2 = new_hi;
        }
    }
    Ast::Area(out)
}

/// `ast` with its references adjusted for `edit`. `home` is the sheet unqualified references point at (the
/// sheet the formula is on; `None` for a workbook-level name, whose unqualified references never match).
pub fn shift_ast(ast: &Ast, home: Option<&str>, edit: &StructuralEdit) -> Ast {
    shift_node(ast, home, edit, Mode::Plain)
}

/// As [`shift_ast`] for the formula of a defined name: relative references keep their offset from `A1`, so only
/// `$`-anchored coordinates move.
pub fn shift_name_ast(ast: &Ast, edit: &StructuralEdit) -> Ast {
    shift_node(ast, None, edit, Mode::Fixed)
}

/// As [`shift_ast`] for a formula whose relative references are offsets from an anchor cell that itself moves by
/// `delta` rows or columns (the conditional format or validation formulas of a range whose top-left cell moves).
pub fn shift_anchored_ast(ast: &Ast, home: &str, edit: &StructuralEdit, delta: i64) -> Ast {
    shift_node(ast, Some(home), edit, Mode::Anchored(delta))
}

fn shift_node(ast: &Ast, home: Option<&str>, edit: &StructuralEdit, mode: Mode) -> Ast {
    let go = |a: &Ast| shift_node(a, home, edit, mode);
    let boxed = |a: &Ast| Box::new(shift_node(a, home, edit, mode));
    match ast {
        Ast::Ref(r) => shift_cell_ref(r, home, edit, mode),
        Ast::Area(a) => shift_area_ref(a, home, edit, mode),
        Ast::Func { name, xlfn, args } => Ast::Func {
            name: name.clone(),
            xlfn: *xlfn,
            args: args.iter().map(go).collect(),
        },
        Ast::Unary { op, operand } => Ast::Unary {
            op: *op,
            operand: boxed(operand),
        },
        Ast::Percent(inner) => Ast::Percent(boxed(inner)),
        Ast::Binary { op, lhs, rhs } => Ast::Binary {
            op: *op,
            lhs: boxed(lhs),
            rhs: boxed(rhs),
        },
        Ast::Array(rows) => Ast::Array(rows.iter().map(|r| r.iter().map(go).collect()).collect()),
        Ast::Invoke { callee, args } => Ast::Invoke {
            callee: boxed(callee),
            args: args.iter().map(go).collect(),
        },
        other => other.clone(),
    }
}

/// The formula text `formula` (with or without a leading `=`) after `edit`, or `None` when no reference in it
/// changes. The returned text has no leading `=`. A formula that does not parse is an error; callers keep it
/// as it is.
///
/// # Errors
/// The formula does not parse.
pub fn shift_formula_text(
    formula: &str,
    home: Option<&str>,
    edit: &StructuralEdit,
) -> Result<Option<String>, ParseError> {
    let ast = parse_formula(formula)?;
    let shifted = shift_ast(&ast, home, edit);
    Ok((shifted != ast).then(|| to_formula_string(&shifted)))
}

/// As [`shift_formula_text`] for the formula of a defined name (see [`shift_name_ast`]).
///
/// # Errors
/// The formula does not parse.
pub fn shift_name_text(formula: &str, edit: &StructuralEdit) -> Result<Option<String>, ParseError> {
    let ast = parse_formula(formula)?;
    let shifted = shift_name_ast(&ast, edit);
    Ok((shifted != ast).then(|| to_formula_string(&shifted)))
}

/// As [`shift_formula_text`] for a conditional format or validation formula (see [`shift_anchored_ast`]).
///
/// # Errors
/// The formula does not parse.
pub fn shift_anchored_text(
    formula: &str,
    home: &str,
    edit: &StructuralEdit,
    delta: i64,
) -> Result<Option<String>, ParseError> {
    let ast = parse_formula(formula)?;
    let shifted = shift_anchored_ast(&ast, home, edit, delta);
    Ok((shifted != ast).then(|| to_formula_string(&shifted)))
}

/// True when the result of a formula can change although none of its references does: it uses a defined name or
/// a table, or a whole-row or whole-column reference across the edited axis, or a range that reaches the edit.
/// `home` is the sheet the formula is on.
pub fn depends_on_edit_band(ast: &Ast, home: &str, edit: &StructuralEdit) -> bool {
    match ast {
        Ast::Name(_) | Ast::Structured { .. } => true,
        Ast::Area(a) => {
            if !targets(edit, Some(home), a.sheet.as_deref()) {
                return false;
            }
            match (a.kind, edit.axis) {
                (AreaKind::Cols, Axis::Row) | (AreaKind::Rows, Axis::Col) => true,
                (AreaKind::Cols, Axis::Col)
                | (AreaKind::Rows, Axis::Row)
                | (AreaKind::Cells, _) => {
                    let hi = match edit.axis {
                        Axis::Row => a.row1.max(a.row2),
                        Axis::Col => a.col1.max(a.col2),
                    };
                    hi >= edit.at
                }
            }
        }
        Ast::Func { args, .. } => args.iter().any(|x| depends_on_edit_band(x, home, edit)),
        Ast::Unary { operand, .. } => depends_on_edit_band(operand, home, edit),
        Ast::Percent(inner) => depends_on_edit_band(inner, home, edit),
        Ast::Binary { lhs, rhs, .. } => {
            depends_on_edit_band(lhs, home, edit) || depends_on_edit_band(rhs, home, edit)
        }
        Ast::Array(rows) => rows
            .iter()
            .flatten()
            .any(|x| depends_on_edit_band(x, home, edit)),
        Ast::Invoke { callee, args } => {
            depends_on_edit_band(callee, home, edit)
                || args.iter().any(|x| depends_on_edit_band(x, home, edit))
        }
        _ => false,
    }
}

/// A cell range in A1 notation (`C3`, `C3:F9`, `A:C`, `3:5`, optionally `$`-marked) after `edit`.
/// `None` when a delete removes it all; the input must be an unqualified reference.
pub fn shift_range_text(range: &str, edit: &StructuralEdit) -> Option<String> {
    let ast = parse_formula(range).ok()?;
    match shift_ast(&ast, Some(&edit.sheet), edit) {
        shifted @ (Ast::Ref(_) | Ast::Area(_)) => Some(to_formula_string(&shifted)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(axis: Axis, kind: EditKind, at: u32, count: u32) -> StructuralEdit {
        StructuralEdit {
            sheet: "S".into(),
            axis,
            kind,
            at,
            count,
        }
    }

    fn run(formula: &str, home: &str, e: &StructuralEdit) -> String {
        shift_formula_text(formula, Some(home), e)
            .expect("parses")
            .unwrap_or_else(|| formula.trim_start_matches('=').to_owned())
    }

    #[test]
    fn insert_moves_and_grows() {
        let e = edit(Axis::Row, EditKind::Insert, 5, 2);
        assert_eq!(run("=C3", "S", &e), "C3");
        assert_eq!(run("=C5", "S", &e), "C7");
        assert_eq!(run("=$C$9", "S", &e), "$C$11");
        assert_eq!(run("=SUM(C3:C9)", "S", &e), "SUM(C3:C11)");
        assert_eq!(run("=SUM(C5:C9)", "S", &e), "SUM(C7:C11)");
        assert_eq!(run("=SUM(C3:C4)", "S", &e), "SUM(C3:C4)");
        assert_eq!(run("=SUM(C:C)", "S", &e), "SUM(C:C)");
        assert_eq!(run("=SUM(3:5)", "S", &e), "SUM(3:7)");
    }

    #[test]
    fn delete_shrinks_and_kills() {
        let e = edit(Axis::Row, EditKind::Delete, 4, 2);
        assert_eq!(run("=C3", "S", &e), "C3");
        assert_eq!(run("=C4", "S", &e), "#REF!");
        assert_eq!(run("=C7", "S", &e), "C5");
        assert_eq!(run("=SUM(C3:C9)", "S", &e), "SUM(C3:C7)");
        assert_eq!(run("=SUM(C4:C5)", "S", &e), "SUM(#REF!)");
        assert_eq!(run("=SUM(C5:C9)", "S", &e), "SUM(C4:C7)");
        assert_eq!(run("=SUM(C3:C5)", "S", &e), "SUM(C3:C3)");
    }

    #[test]
    fn other_sheets_and_qualifiers() {
        let e = edit(Axis::Col, EditKind::Delete, 3, 1);
        assert_eq!(run("=S!C1+S!D1", "O", &e), "S!#REF!+S!C1");
        assert_eq!(run("=C1+D1", "O", &e), "C1+D1");
        assert_eq!(run("=C1", "S", &e), "#REF!");
        assert_eq!(run("='S'!E1", "O", &e), "S!D1");
    }
}
