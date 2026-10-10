//! Helpers shared by the engine and function tests.

use crate::ast::parse_cell_address;
use crate::{Engine, FixedClock, Value};

pub(crate) const SHEET: &str = "Sheet1";
/// Where test formulas are placed (CV1000), away from test data.
const FORMULA_ROW: u32 = 1000;
const FORMULA_COL: u32 = 100;

/// 2023-11-14 22:13:20 UTC.
pub(crate) const CLOCK_SECONDS: f64 = 1_700_000_000.0;

pub(crate) fn n(x: f64) -> Value {
    Value::Number(x)
}

pub(crate) fn t(s: &str) -> Value {
    Value::Text(s.to_owned())
}

pub(crate) fn b(x: bool) -> Value {
    Value::Bool(x)
}

pub(crate) fn err(e: crate::ErrorKind) -> Value {
    Value::Error(e)
}

pub(crate) fn engine_with(cells: &[(&str, Value)]) -> Engine {
    let mut e = Engine::with_clock(Box::new(FixedClock(CLOCK_SECONDS)));
    for (addr, value) in cells {
        let (row, col) = parse_cell_address(addr).unwrap();
        e.set_value(SHEET, row, col, value.clone());
    }
    e
}

/// Evaluates `formula` against an engine and returns the result.
pub(crate) fn eval_in(e: &mut Engine, formula: &str) -> Value {
    e.set_formula(SHEET, FORMULA_ROW, FORMULA_COL, formula)
        .unwrap();
    e.recalc_dirty();
    e.get(SHEET, FORMULA_ROW, FORMULA_COL)
}

/// Evaluates `formula` with the given cells (A1 notation) filled in.
pub(crate) fn eval_with(cells: &[(&str, Value)], formula: &str) -> Value {
    eval_in(&mut engine_with(cells), formula)
}

/// Evaluates `formula` on an empty sheet.
pub(crate) fn eval(formula: &str) -> Value {
    eval_with(&[], formula)
}

/// Like [`eval_with`] but returns the whole array result.
pub(crate) fn eval_full(cells: &[(&str, Value)], formula: &str) -> Value {
    let mut e = engine_with(cells);
    e.set_formula(SHEET, FORMULA_ROW, FORMULA_COL, formula)
        .unwrap();
    e.recalc_dirty();
    e.get_full(SHEET, FORMULA_ROW, FORMULA_COL)
}

/// Asserts a numeric result within a tolerance.
pub(crate) fn assert_close(actual: &Value, expected: f64) {
    match actual {
        Value::Number(x) => assert!(
            (x - expected).abs() < 1e-9,
            "expected about {expected}, got {x}"
        ),
        other => panic!("expected number {expected}, got {other:?}"),
    }
}
