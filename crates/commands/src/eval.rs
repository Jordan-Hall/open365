//! Read-only formula evaluation against a workbook (no command, nothing is saved).
//!
//! The formula is installed in a scratch calculation engine built from the workbook's cells,
//! so the open workbook and its file are never touched. Read-only, so the effect class is
//! `Read`.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use open365_calc::{Engine, Value};
use open365_xlsx::Workbook;
use serde::Serialize;

use crate::error::CommandError;
use crate::xlsx_state::{build_engine, build_engine_cancellable};

/// Longest formula accepted, in characters.
pub const MAX_FORMULA_CHARS: usize = 8192;
/// Most rows and columns of an array result that are returned.
pub const MAX_RESULT_DIM: usize = 100;
/// Longest text result returned, in bytes.
pub const MAX_RESULT_TEXT: usize = 32 * 1024;

/// A formula result in a form fit for output.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum EvalValue {
    Empty,
    Number(f64),
    Text(String),
    Bool(bool),
    /// An error such as `#DIV/0!`.
    Error(String),
    Array(Vec<Vec<EvalValue>>),
}

/// The outcome of [`evaluate_formula`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Evaluated {
    /// The formula that was evaluated, without a leading `=`.
    pub formula: String,
    pub result: EvalValue,
    /// True when the text or array result was cut to the output limits.
    pub truncated: bool,
}

fn convert(value: &Value, truncated: &mut bool) -> EvalValue {
    match value {
        Value::Empty => EvalValue::Empty,
        Value::Number(n) => EvalValue::Number(*n),
        Value::Bool(b) => EvalValue::Bool(*b),
        Value::Error(e) => EvalValue::Error(e.to_string()),
        Value::Text(t) => {
            let mut end = t.len().min(MAX_RESULT_TEXT);
            while !t.is_char_boundary(end) {
                end -= 1;
            }
            if end < t.len() {
                *truncated = true;
            }
            EvalValue::Text(t[..end].to_owned())
        }
        Value::Array(rows) => {
            if rows.len() > MAX_RESULT_DIM || rows.iter().any(|r| r.len() > MAX_RESULT_DIM) {
                *truncated = true;
            }
            EvalValue::Array(
                rows.iter()
                    .take(MAX_RESULT_DIM)
                    .map(|r| {
                        r.iter()
                            .take(MAX_RESULT_DIM)
                            .map(|v| convert(v, truncated))
                            .collect()
                    })
                    .collect(),
            )
        }
    }
}

/// Evaluates `formula` as if it were entered in cell (`row`, `col`; 1-based) of `sheet`, so
/// relative references resolve from there. With `formula` `None` the cell's own formula is
/// recalculated. `keep(sheet, row, col)` decides which cells the evaluation may see (a caller
/// that hides content passes a filter); a sheet is excluded when `keep(sheet, 0, 0)` is false.
///
/// # Errors
/// [`CommandError::Formula`] when the formula is too long, does not parse, the cell has no
/// formula and none was given, or the sheet is not a visible worksheet.
pub fn evaluate_formula(
    wb: &Workbook,
    sheet: usize,
    row: u32,
    col: u32,
    formula: Option<&str>,
    keep: &dyn Fn(usize, u32, u32) -> bool,
) -> Result<Evaluated, CommandError> {
    FormulaEngine::build(wb, keep).evaluate(wb, sheet, row, col, formula, keep)
}

/// A calculation engine built once from a workbook and reused for many evaluations. Each
/// evaluation puts the formula into a cell, reads the result and puts the cell back, so the
/// engine stays equal to the workbook it was built from. Build a new one when the workbook
/// changes. It is not `Send` (the engine holds a clock); keep it on the thread that built it.
#[derive(Debug)]
pub struct FormulaEngine {
    engine: Engine,
}

impl FormulaEngine {
    pub(crate) fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn build(wb: &Workbook, keep: &dyn Fn(usize, u32, u32) -> bool) -> Self {
        Self {
            engine: build_engine(wb, keep),
        }
    }

    /// As [`FormulaEngine::build`], but gives up with [`CommandError::Cancelled`] soon after
    /// `cancel` is set. Nothing partial is returned.
    ///
    /// # Errors
    /// [`CommandError::Cancelled`].
    pub fn build_cancellable(
        wb: &Workbook,
        keep: &dyn Fn(usize, u32, u32) -> bool,
        cancel: &Arc<AtomicBool>,
    ) -> Result<Self, CommandError> {
        build_engine_cancellable(wb, keep, Some(cancel))
            .map(|engine| Self { engine })
            .ok_or(CommandError::Cancelled)
    }

    /// As [`evaluate_formula`], on this engine. `wb` and `keep` must be the ones the engine
    /// was built from.
    ///
    /// # Errors
    /// As [`evaluate_formula`].
    pub fn evaluate(
        &mut self,
        wb: &Workbook,
        sheet: usize,
        row: u32,
        col: u32,
        formula: Option<&str>,
        keep: &dyn Fn(usize, u32, u32) -> bool,
    ) -> Result<Evaluated, CommandError> {
        evaluate_on(&mut self.engine, wb, sheet, row, col, formula, keep)
    }

    /// As [`FormulaEngine::evaluate`] with the cell as `(sheet, row, col)`, but stops soon after
    /// `cancel` is set and returns [`CommandError::Cancelled`]. The engine stays usable: the probe cell is put back and
    /// the cells the recalculation did not reach stay marked dirty, so the next evaluation
    /// finishes them.
    ///
    /// # Errors
    /// As [`FormulaEngine::evaluate`], plus [`CommandError::Cancelled`].
    pub fn evaluate_cancellable(
        &mut self,
        wb: &Workbook,
        (sheet, row, col): (usize, u32, u32),
        formula: Option<&str>,
        keep: &dyn Fn(usize, u32, u32) -> bool,
        cancel: &Arc<AtomicBool>,
    ) -> Result<Evaluated, CommandError> {
        self.engine.set_cancel(Some(Arc::clone(cancel)));
        let out = evaluate_on(&mut self.engine, wb, sheet, row, col, formula, keep);
        self.engine.set_cancel(None);
        out
    }
}

fn evaluate_on(
    engine: &mut Engine,
    wb: &Workbook,
    sheet: usize,
    row: u32,
    col: u32,
    formula: Option<&str>,
    keep: &dyn Fn(usize, u32, u32) -> bool,
) -> Result<Evaluated, CommandError> {
    let info = wb
        .sheets()
        .get(sheet)
        .ok_or_else(|| CommandError::Formula("no such sheet".to_owned()))?;
    if !keep(sheet, 0, 0) || wb.sheet(sheet).is_none() {
        return Err(CommandError::Formula("no such sheet".to_owned()));
    }
    let text = match formula {
        Some(f) => f.to_owned(),
        None => Some(())
            .filter(|()| keep(sheet, row, col))
            .and_then(|()| wb.formula_text(sheet, row, col))
            .ok_or_else(|| {
                CommandError::Formula("the cell has no formula; give a formula".to_owned())
            })?,
    };
    let text = text.trim_start_matches('=').to_owned();
    if text.chars().count() > MAX_FORMULA_CHARS {
        return Err(CommandError::Formula(format!(
            "formula is longer than {MAX_FORMULA_CHARS} characters"
        )));
    }
    let prior_formula = engine
        .get_formula(&info.name, row, col)
        .map(|f| f.trim_start_matches('=').to_owned());
    let prior_value = engine.get_full(&info.name, row, col);
    let set = engine.set_formula(&info.name, row, col, &text);
    let result = if set.is_ok() {
        engine.recalc_dirty();
        Some(engine.get_full(&info.name, row, col))
    } else {
        None
    };
    // Cancelled mid-recalculation: the result may be stale. Put the cell back below but do
    // not recalculate; the cells not reached are still dirty and the next call settles them.
    let cancelled = engine.cancel_requested();
    // Put the cell back, so the next evaluation starts from the workbook's own state.
    match prior_formula {
        Some(f) => {
            let _ = engine.set_formula(&info.name, row, col, &f);
        }
        None if prior_value == Value::Empty => engine.clear(&info.name, row, col),
        None => engine.set_value(&info.name, row, col, prior_value),
    }
    if cancelled {
        return Err(CommandError::Cancelled);
    }
    engine.recalc_dirty();
    let value = match (set, result) {
        (Err(e), _) => return Err(CommandError::Formula(e.to_string())),
        (Ok(()), Some(v)) => v,
        (Ok(()), None) => Value::Empty,
    };
    let mut truncated = false;
    let result = convert(&value, &mut truncated);
    Ok(Evaluated {
        formula: text,
        result,
        truncated,
    })
}
