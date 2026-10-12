//! Formulas tab on the sheet model: AutoSum, Show Formulas, formula auditing arrows, error
//! checking and the requests for the function, name and evaluate dialogs.
//!
//! The view state (Show Formulas, the audit arrows, the pending dialog request) is
//! `SheetModel::ext_formulas`; the grid reads `show_formulas` and `arrows`, the shell takes
//! dialog requests with `SheetModel::take_request`.

use std::collections::BTreeSet;

use open365_calc::ast::Ast;
use open365_calc::parse_formula;
use open365_commands::objects_ops::NameRequest;
use open365_commands::{Command, EvalValue, FormulaEngine, ObjectsOp};
use open365_xlsx::CellValue;

use crate::dialogs::formulas_function::{Category, formula_for, is_function};
use crate::sheet::{Rect4, SheetModel, a1};

/// Which way an audit arrow traces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrowKind {
    Precedent,
    Dependent,
}

/// A trace arrow from the cells `from` to the cell `to` (data flows from `from` to `to`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arrow {
    pub from: Rect4,
    pub to: (u32, u32),
    /// The source lies on another sheet: Excel draws a dashed arrow to a sheet icon.
    pub off_sheet: bool,
    pub kind: ArrowKind,
}

/// A dialog the shell should open for a Formulas command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Insert Function, optionally on a category.
    InsertFunction {
        category: Option<Category>,
    },
    /// Function Arguments for one function.
    FunctionArguments {
        name: String,
    },
    NameManager,
    /// Evaluate Formula on the active cell.
    EvaluateFormula,
}

/// What the Formulas tab keeps on the sheet model (none of it is in the file or the undo history).
#[derive(Debug, Default)]
pub struct FormulasState {
    pub show_formulas: bool,
    pub arrows: Vec<Arrow>,
    request: Option<Request>,
    recent: Vec<String>,
}

/// Cell and area references of `ast`: `(sheet qualifier, rect)`.
fn collect(ast: &Ast, out: &mut Vec<(Option<String>, Rect4)>) {
    match ast {
        Ast::Ref(r) => out.push((r.sheet.clone(), (r.row, r.col, r.row, r.col))),
        Ast::Area(a) => {
            let rows = (a.row1.min(a.row2), a.row1.max(a.row2));
            let cols = (a.col1.min(a.col2), a.col1.max(a.col2));
            out.push((a.sheet.clone(), (rows.0, cols.0, rows.1, cols.1)));
        }
        Ast::Func { args, .. } => args.iter().for_each(|a| collect(a, out)),
        Ast::Invoke { callee, args } => {
            collect(callee, out);
            args.iter().for_each(|a| collect(a, out));
        }
        Ast::Unary { operand, .. } => collect(operand, out),
        Ast::Percent(inner) => collect(inner, out),
        Ast::Binary { lhs, rhs, .. } => {
            collect(lhs, out);
            collect(rhs, out);
        }
        Ast::Array(rows) => rows.iter().flatten().for_each(|a| collect(a, out)),
        _ => {}
    }
}

fn inside(r: Rect4, cell: (u32, u32)) -> bool {
    (r.0..=r.2).contains(&cell.0) && (r.1..=r.3).contains(&cell.1)
}

fn range_text(r: Rect4) -> String {
    if (r.0, r.1) == (r.2, r.3) {
        a1(r.0, r.1)
    } else {
        format!("{}:{}", a1(r.0, r.1), a1(r.2, r.3))
    }
}

impl SheetModel {
    fn is_number_cell(&self, row: u32, col: u32) -> bool {
        self.target
            .xlsx()
            .and_then(|wb| wb.cell(self.sheet, row, col))
            .is_some_and(|c| matches!(c.value, CellValue::Number(_)))
    }

    /// The references of the formula in a cell: `(rect, on another sheet)`.
    fn formula_refs(&self, row: u32, col: u32) -> Vec<(Rect4, bool)> {
        let Some(wb) = self.target.xlsx() else {
            return Vec::new();
        };
        let Some(text) = wb.formula_text(self.sheet, row, col) else {
            return Vec::new();
        };
        let Ok(ast) = parse_formula(&text) else {
            return Vec::new();
        };
        let own = wb.sheets().get(self.sheet).map(|s| s.name.to_lowercase());
        let mut refs = Vec::new();
        collect(&ast, &mut refs);
        refs.into_iter()
            .map(|(q, rect)| {
                let off = q.is_some_and(|q| Some(q.trim_matches('\'').to_lowercase()) != own);
                (rect, off)
            })
            .collect()
    }

    /// The contiguous numbers directly above (else left of) `(row, col)`, as Excel's AutoSum
    /// guesses its range.
    fn autosum_range(&self, row: u32, col: u32) -> Option<Rect4> {
        let mut top = row;
        while top > 1 && self.is_number_cell(top - 1, col) {
            top -= 1;
        }
        if top < row {
            return Some((top, col, row - 1, col));
        }
        let mut left = col;
        while left > 1 && self.is_number_cell(row, left - 1) {
            left -= 1;
        }
        (left < col).then_some((row, left, row, col - 1))
    }

    /// AutoSum and its menu: `function` is `SUM`, `AVERAGE`, `COUNT`, `MAX` or `MIN`.
    ///
    /// One cell selected: the formula goes in that cell over the numbers above it, else to its
    /// left. A range of several rows gets one formula under each column, a single row one at its
    /// right. One undo step.
    ///
    /// # Errors
    /// The command layer's message when the cell cannot be written.
    pub fn auto_sum(&mut self, function: &str) -> Result<(), String> {
        let f = function.to_ascii_uppercase();
        let (r1, c1, r2, c2) = self.range();
        if !self.is_range() {
            let text = match self.autosum_range(r1, c1) {
                Some(r) => format!("={f}({})", range_text(r)),
                None => format!("={f}()"),
            };
            if parse_formula(&text).is_err() {
                // Excel starts editing `=SUM(` here; there is no half-typed formula to keep.
                self.status = format!("Select the cells for {f}.");
                return Ok(());
            }
            return self.enter(r1, c1, &text);
        }
        let (row0, col0, cells): (u32, u32, Vec<String>) = if r2 > r1 {
            let cells = (c1..=c2)
                .map(|c| format!("={f}({})", range_text((r1, c, r2, c))))
                .collect();
            (r2 + 1, c1, cells)
        } else {
            (
                r1,
                c2 + 1,
                vec![format!("={f}({})", range_text((r1, c1, r1, c2)))],
            )
        };
        self.apply(Command::XlsxPaste {
            sheet: self.sheet,
            row: row0,
            col: col0,
            rows: vec![cells],
        })?;
        self.select_cell((row0, col0));
        Ok(())
    }

    /// The dialog request a command left, if any (taken: a second call gives `None`).
    pub fn take_request(&mut self) -> Option<Request> {
        self.ext_formulas.request.take()
    }

    /// Recently used functions, newest first (at most ten).
    #[must_use]
    pub fn recent_functions(&self) -> &[String] {
        &self.ext_formulas.recent
    }

    /// Toggles Show Formulas (Ctrl+`); returns the new state.
    pub fn toggle_show_formulas(&mut self) -> bool {
        self.ext_formulas.show_formulas = !self.ext_formulas.show_formulas;
        self.ext_formulas.show_formulas
    }

    /// What a cell displays: like [`SheetModel::shown`], but a formula cell shows its formula
    /// while Show Formulas is on.
    #[must_use]
    pub fn shown_for_view(&self, row: u32, col: u32) -> Option<(String, bool)> {
        let src = self.source(row, col);
        if self.ext_formulas.show_formulas && src.starts_with('=') {
            return Some((src, false));
        }
        self.shown(row, col)
    }

    /// Trace Precedents on the active cell; each further call adds the next level.
    pub fn trace_precedents(&mut self) {
        self.trace(ArrowKind::Precedent);
    }

    /// Trace Dependents on the active cell; each further call adds the next level.
    pub fn trace_dependents(&mut self) {
        self.trace(ArrowKind::Dependent);
    }

    fn trace(&mut self, kind: ArrowKind) {
        let existing: Vec<Arrow> = self
            .ext_formulas
            .arrows
            .iter()
            .filter(|a| a.kind == kind)
            .cloned()
            .collect();
        let (ar, ac) = self.active;
        let started = match kind {
            ArrowKind::Precedent => existing.iter().any(|a| a.to == self.active),
            ArrowKind::Dependent => existing.iter().any(|a| a.from == (ar, ac, ar, ac)),
        };
        let anchors: Vec<(u32, u32)> = if !started {
            vec![self.active]
        } else if kind == ArrowKind::Precedent {
            let done: BTreeSet<(u32, u32)> = existing.iter().map(|a| a.to).collect();
            existing
                .iter()
                .filter(|a| !a.off_sheet)
                .flat_map(|a| self.formula_cells_in(a.from))
                .filter(|c| !done.contains(c))
                .collect()
        } else {
            let done: BTreeSet<(u32, u32)> =
                existing.iter().map(|a| (a.from.0, a.from.1)).collect();
            existing
                .iter()
                .map(|a| a.to)
                .filter(|c| !done.contains(c))
                .collect()
        };
        let mut added = Vec::new();
        for cell in anchors {
            if kind == ArrowKind::Precedent {
                for (rect, off) in self.formula_refs(cell.0, cell.1) {
                    added.push(Arrow {
                        from: rect,
                        to: cell,
                        off_sheet: off,
                        kind,
                    });
                }
            } else {
                for dep in self.dependents_of(cell) {
                    added.push(Arrow {
                        from: (cell.0, cell.1, cell.0, cell.1),
                        to: dep,
                        off_sheet: false,
                        kind,
                    });
                }
            }
        }
        added.retain(|a| !existing.contains(a));
        if added.is_empty() {
            self.status = match (kind, existing.is_empty()) {
                (ArrowKind::Precedent, true) => {
                    "The active cell contains no formula with precedents.".to_owned()
                }
                (ArrowKind::Dependent, true) => {
                    "The active cell has no dependents on this sheet.".to_owned()
                }
                _ => "There are no more cells to trace.".to_owned(),
            };
            return;
        }
        self.status.clear();
        self.ext_formulas.arrows.extend(added);
    }

    fn formula_cells_in(&self, r: Rect4) -> Vec<(u32, u32)> {
        let Some(wb) = self.target.xlsx() else {
            return Vec::new();
        };
        wb.iter_cells(self.sheet)
            .filter(|(row, col, c)| c.formula.is_some() && inside(r, (*row, *col)))
            .map(|(row, col, _)| (row, col))
            .collect()
    }

    /// The formula cells of this sheet whose formulas read `cell`.
    fn dependents_of(&self, cell: (u32, u32)) -> Vec<(u32, u32)> {
        let Some(wb) = self.target.xlsx() else {
            return Vec::new();
        };
        let mut out: Vec<(u32, u32)> = wb
            .iter_cells(self.sheet)
            .filter(|(_, _, c)| c.formula.is_some())
            .map(|(r, c, _)| (r, c))
            .filter(|&(r, c)| {
                (r, c) != cell
                    && self
                        .formula_refs(r, c)
                        .iter()
                        .any(|(rect, off)| !off && inside(*rect, cell))
            })
            .collect();
        out.sort_unstable();
        out
    }

    /// Remove Arrows.
    pub fn remove_arrows(&mut self) {
        self.ext_formulas.arrows.clear();
    }

    /// Error Checking: selects the next cell (row by row, from the active cell, wrapping)
    /// whose value is an error and says what it is. `None` when the sheet has none.
    pub fn check_errors(&mut self) -> Option<(u32, u32)> {
        let mut errors: Vec<(u32, u32)> = self
            .target
            .xlsx()?
            .iter_cells(self.sheet)
            .filter(|(_, _, c)| matches!(c.value, CellValue::Error(_)))
            .map(|(r, c, _)| (r, c))
            .collect();
        errors.sort_unstable();
        let found = errors
            .iter()
            .find(|&&c| c > self.active)
            .or_else(|| errors.first())
            .copied();
        if let Some(cell) = found {
            self.select_cell(cell);
            self.status = format!(
                "Error in cell {}: {}",
                a1(cell.0, cell.1),
                self.display(cell.0, cell.1)
            );
        } else {
            self.status = "The error check is complete for the entire sheet.".to_owned();
        }
        found
    }

    /// Opens Insert Function (or a category list) through the shell.
    pub fn request_insert_function(&mut self, category: Option<Category>) {
        self.ext_formulas.request = Some(Request::InsertFunction { category });
    }

    /// A function picked from a Function Library list: Function Arguments for it.
    pub fn request_function(&mut self, name: &str) -> bool {
        if !is_function(name) {
            self.status = format!("{name} is not a known function.");
            return false;
        }
        self.ext_formulas.request = Some(Request::FunctionArguments {
            name: name.to_ascii_uppercase(),
        });
        true
    }

    /// Name Manager through the shell.
    pub fn request_name_manager(&mut self) {
        self.ext_formulas.request = Some(Request::NameManager);
    }

    /// Evaluate Formula on the active cell, or the message Excel gives for a constant.
    pub fn request_evaluate(&mut self) -> bool {
        if self.source(self.active.0, self.active.1).starts_with('=') {
            self.ext_formulas.request = Some(Request::EvaluateFormula);
            true
        } else {
            self.status = "The cell currently being evaluated contains a constant.".to_owned();
            false
        }
    }

    /// OK in Function Arguments: writes `name(args)` to the active cell and remembers the function.
    ///
    /// # Errors
    /// The message when the formula does not parse or cannot be written.
    pub fn apply_function(&mut self, name: &str, args: &[&str]) -> Result<(), String> {
        let text = formula_for(name, args);
        if let Err(e) = parse_formula(&text) {
            self.status = format!("There's a problem with this formula: {e:?}");
            return Err(self.status.clone());
        }
        let (r, c) = self.active;
        self.enter(r, c, &text)?;
        let up = name.to_ascii_uppercase();
        let recent = &mut self.ext_formulas.recent;
        recent.retain(|n| *n != up);
        recent.insert(0, up);
        recent.truncate(10);
        Ok(())
    }

    /// Starts Evaluate Formula on the active cell.
    #[must_use]
    pub fn evaluate_session(&self) -> Option<crate::dialogs::formulas_evaluate::EvalSession> {
        let (r, c) = self.active;
        crate::dialogs::formulas_evaluate::EvalSession::start(self.target.xlsx()?, self.sheet, r, c)
    }

    /// Evaluate: one step of `session`; false once only a value is left.
    pub fn evaluate_step(
        &self,
        session: &mut crate::dialogs::formulas_evaluate::EvalSession,
    ) -> bool {
        self.target.xlsx().is_some_and(|wb| session.step(wb))
    }

    /// Evaluate: Step In on the next cell with a formula; false when there is none.
    pub fn evaluate_step_in(
        &self,
        session: &mut crate::dialogs::formulas_evaluate::EvalSession,
    ) -> bool {
        self.target.xlsx().is_some_and(|wb| session.step_in(wb))
    }

    /// Evaluate: Step Out of a formula Step In opened.
    pub fn evaluate_step_out(
        &self,
        session: &mut crate::dialogs::formulas_evaluate::EvalSession,
    ) -> bool {
        self.target.xlsx().is_some_and(|wb| session.step_out(wb))
    }

    /// Evaluate: whether Step In has a formula to open.
    #[must_use]
    pub fn evaluate_can_step_in(
        &self,
        session: &crate::dialogs::formulas_evaluate::EvalSession,
    ) -> bool {
        self.target.xlsx().is_some_and(|wb| session.can_step_in(wb))
    }

    /// Evaluate: the cell the session is in, as `Sheet1!$C$3`.
    #[must_use]
    pub fn evaluate_reference(
        &self,
        session: &crate::dialogs::formulas_evaluate::EvalSession,
    ) -> String {
        self.target
            .xlsx()
            .map(|wb| session.reference(wb))
            .unwrap_or_default()
    }

    /// The Name Manager rows.
    #[must_use]
    pub fn name_rows(&self) -> Vec<crate::dialogs::formulas_names::NameRow> {
        self.target
            .xlsx()
            .map(crate::dialogs::formulas_names::rows)
            .unwrap_or_default()
    }

    // ---- Name Manager --------------------------------------------------------------------

    /// The names of the sheets, in tab order (the Scope list of New Name).
    #[must_use]
    pub fn sheet_names(&self) -> Vec<String> {
        self.target
            .xlsx()
            .map(|wb| wb.sheets().iter().map(|i| i.name.clone()).collect())
            .unwrap_or_default()
    }

    /// The index of the active sheet.
    #[must_use]
    pub fn active_sheet_index(&self) -> usize {
        self.sheet
    }

    fn name_op(&mut self, op: ObjectsOp) -> Result<(), String> {
        self.apply(Command::SheetObjects {
            sheet: self.sheet,
            op,
        })
    }

    /// Name Manager > New: one undo step.
    ///
    /// # Errors
    /// The command layer's message (illegal name, name taken).
    pub fn name_define(&mut self, name: NameRequest) -> Result<(), String> {
        self.name_op(ObjectsOp::DefineName(name))
    }

    /// Name Manager > Edit: renames or retargets `name` (scope `scope`).
    ///
    /// # Errors
    /// The command layer's message.
    pub fn name_edit(
        &mut self,
        name: &str,
        scope: Option<usize>,
        to: NameRequest,
    ) -> Result<(), String> {
        self.name_op(ObjectsOp::EditName {
            name: name.to_owned(),
            scope,
            to,
        })
    }

    /// Name Manager > Delete.
    ///
    /// # Errors
    /// The command layer's message.
    pub fn name_delete(&mut self, name: &str, scope: Option<usize>) -> Result<(), String> {
        self.name_op(ObjectsOp::DeleteName {
            name: name.to_owned(),
            scope,
        })
    }

    // ---- Function Arguments -------------------------------------------------------------

    /// What each formula in `formulas` (with or without a leading `=`) would give in the active
    /// cell, as Excel's Function Arguments dialog shows it: text in quotes when `quote`, arrays
    /// as `{1,2;3,4}`. `None` for an empty text or one that does not evaluate. Nothing is
    /// written to the workbook.
    #[must_use]
    pub fn evaluate_previews(&self, formulas: &[String], quote: bool) -> Vec<Option<String>> {
        let Some(wb) = self.target.xlsx() else {
            return vec![None; formulas.len()];
        };
        let keep = |_: usize, _: u32, _: u32| true;
        let mut engine = FormulaEngine::build(wb, &keep);
        formulas
            .iter()
            .map(|f| {
                let t = f.trim();
                if t.is_empty() {
                    return None;
                }
                // Stacked on its own so a range gives its whole array, as the dialog shows,
                // and not the single value implicit intersection would pick.
                let text = format!("=VSTACK({})", t.trim_start_matches('='));
                engine
                    .evaluate(
                        wb,
                        self.sheet,
                        self.active.0,
                        self.active.1,
                        Some(&text),
                        &keep,
                    )
                    .ok()
                    .map(|e| show_value(&e.result, quote))
            })
            .collect()
    }
}

/// A number the way Excel shows it in a dialog: up to 15 significant digits.
fn show_number(n: f64) -> String {
    if (n - n.round()).abs() < 1e-9 && n.abs() < 1e15 {
        return format!("{}", n.round() as i64);
    }
    let s = format!("{n:.14e}");
    let value: f64 = s.parse().unwrap_or(n);
    let text = format!("{value}");
    if text.len() > 20 { s } else { text }
}

fn show_value(v: &EvalValue, quote: bool) -> String {
    match v {
        EvalValue::Empty => String::new(),
        EvalValue::Number(n) => show_number(*n),
        EvalValue::Text(t) if quote => format!("\"{t}\""),
        EvalValue::Text(t) => t.clone(),
        EvalValue::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_owned(),
        EvalValue::Error(e) => e.clone(),
        EvalValue::Array(rows) if rows.len() == 1 && rows[0].len() == 1 => {
            show_value(&rows[0][0], quote)
        }
        EvalValue::Array(rows) => {
            let body: Vec<String> = rows
                .iter()
                .map(|r| {
                    r.iter()
                        .map(|c| show_value(c, true))
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .collect();
            format!("{{{}}}", body.join(";"))
        }
    }
}
