//! Evaluate Formula: steps through a formula one sub-expression at a time, replacing the next
//! reference, call or operation by its value, as Excel's dialog does.

use open365_calc::ast::UnaryOp;
use open365_calc::{Ast, parse_formula, to_formula_string};
use open365_commands::{EvalValue, evaluate_formula};
use open365_xlsx::Workbook;

/// One Evaluate Formula session for the formula of a cell.
#[derive(Debug, Clone)]
pub struct EvalSession {
    sheet: usize,
    row: u32,
    col: u32,
    expr: Ast,
    /// The formula text before each step, newest last (Step Out / Restart use it).
    history: Vec<Ast>,
}

fn literal(v: &EvalValue) -> Ast {
    match v {
        EvalValue::Empty => Ast::Number(0.0),
        EvalValue::Number(n) if *n < 0.0 => Ast::Unary {
            op: UnaryOp::Neg,
            operand: Box::new(Ast::Number(-n)),
        },
        EvalValue::Number(n) => Ast::Number(*n),
        EvalValue::Text(t) => Ast::Text(t.clone()),
        EvalValue::Bool(b) => Ast::Bool(*b),
        EvalValue::Error(e) => parse_formula(e).unwrap_or(Ast::Text(e.clone())),
        EvalValue::Array(rows) => Ast::Array(
            rows.iter()
                .map(|r| r.iter().map(literal).collect())
                .collect(),
        ),
    }
}

fn is_value(a: &Ast) -> bool {
    match a {
        Ast::Number(_) | Ast::Text(_) | Ast::Bool(_) | Ast::Error(_) | Ast::Missing => true,
        Ast::Array(rows) => rows.iter().flatten().all(is_value),
        Ast::Unary {
            op: UnaryOp::Neg,
            operand,
        } => matches!(**operand, Ast::Number(_)),
        _ => false,
    }
}

impl EvalSession {
    /// Starts on the formula in `(sheet, row, col)`; `None` when the cell holds no formula.
    #[must_use]
    pub fn start(wb: &Workbook, sheet: usize, row: u32, col: u32) -> Option<Self> {
        let text = wb.formula_text(sheet, row, col)?;
        let expr = parse_formula(&text).ok()?;
        Some(Self {
            sheet,
            row,
            col,
            expr,
            history: Vec::new(),
        })
    }

    /// The formula as it stands now, with a leading `=`.
    #[must_use]
    pub fn text(&self) -> String {
        format!("={}", to_formula_string(&self.expr))
    }

    /// True once only a value is left.
    #[must_use]
    pub fn done(&self) -> bool {
        is_value(&self.expr)
    }

    fn eval(&self, wb: &Workbook, node: &Ast) -> Ast {
        let text = format!("={}", to_formula_string(node));
        match evaluate_formula(
            wb,
            self.sheet,
            self.row,
            self.col,
            Some(&text),
            &|_, _, _| true,
        ) {
            Ok(e) => literal(&e.result),
            Err(_) => Ast::Text("#VALUE!".to_owned()),
        }
    }

    /// The tree after one step in `node`, or `None` when `node` is already a value.
    fn reduce(&self, wb: &Workbook, node: &Ast) -> Option<Ast> {
        match node {
            n if is_value(n) => None,
            Ast::Ref(_)
            | Ast::Name(_)
            | Ast::Structured { .. }
            | Ast::Area(_)
            | Ast::DeadRef { .. } => Some(self.eval(wb, node)),
            Ast::Func { name, xlfn, args } => {
                for (i, a) in args.iter().enumerate() {
                    if matches!(a, Ast::Area(_)) {
                        continue;
                    }
                    if let Some(new) = self.reduce(wb, a) {
                        let mut args = args.clone();
                        args[i] = new;
                        return Some(Ast::Func {
                            name: name.clone(),
                            xlfn: *xlfn,
                            args,
                        });
                    }
                }
                Some(self.eval(wb, node))
            }
            Ast::Unary { op, operand } => match self.reduce(wb, operand) {
                Some(new) => Some(Ast::Unary {
                    op: *op,
                    operand: Box::new(new),
                }),
                None => Some(self.eval(wb, node)),
            },
            Ast::Percent(inner) => match self.reduce(wb, inner) {
                Some(new) => Some(Ast::Percent(Box::new(new))),
                None => Some(self.eval(wb, node)),
            },
            Ast::Binary { op, lhs, rhs } => {
                use open365_calc::ast::BinOp;
                if matches!(op, BinOp::Range | BinOp::Intersect | BinOp::Union) {
                    return Some(self.eval(wb, node));
                }
                if let Some(new) = self.reduce(wb, lhs) {
                    return Some(Ast::binary(*op, new, (**rhs).clone()));
                }
                if let Some(new) = self.reduce(wb, rhs) {
                    return Some(Ast::binary(*op, (**lhs).clone(), new));
                }
                Some(self.eval(wb, node))
            }
            _ => Some(self.eval(wb, node)),
        }
    }

    /// Evaluate: replaces the next sub-expression by its value. False once done.
    pub fn step(&mut self, wb: &Workbook) -> bool {
        match self.reduce(wb, &self.expr) {
            Some(new) if new != self.expr => {
                self.history.push(std::mem::replace(&mut self.expr, new));
                true
            }
            _ => false,
        }
    }

    /// Restart: back to the original formula.
    pub fn restart(&mut self) {
        if let Some(first) = self.history.first().cloned() {
            self.expr = first;
            self.history.clear();
        }
    }
}
