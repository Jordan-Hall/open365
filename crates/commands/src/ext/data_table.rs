//! What-If Analysis > Data Table: the grid of results of one formula for a list of inputs.
//!
//! The selected `range` holds the input values in its top row and/or left column; the formulas
//! sit where Excel expects them (see [`run`]). The results are written as values (Excel keeps
//! live `{=TABLE(...)}` cells; this engine has no such function, so the grid is not recalculated
//! when the inputs change; re-run the command instead). The input cells are put back afterwards.

use open365_xlsx::CellValue;

use crate::command::{CellEntry, CellRef};
use crate::error::CommandError;
use crate::find::cell_display_text;
use crate::format::RangeRef;
use crate::xlsx_state::XlsxState;

fn bad(msg: &str) -> CommandError {
    CommandError::Format(msg.to_owned())
}

fn entry(x: &XlsxState, sheet: usize, r: u32, c: u32) -> CellEntry {
    match x.workbook.cell(sheet, r, c).map(|cell| &cell.value) {
        Some(CellValue::Number(n)) => CellEntry::Number(*n),
        Some(CellValue::Bool(b)) => CellEntry::Bool(*b),
        Some(CellValue::Empty) | None => CellEntry::Empty,
        _ => CellEntry::Text(cell_display_text(&x.workbook, sheet, r, c)),
    }
}

/// Fills the interior of `range` (top row and left column are the inputs).
///
/// - `col_input` only: the left column holds values for that cell; formulas are in the top row.
/// - `row_input` only: the top row holds values for that cell; formulas are in the left column.
/// - both: the top-left cell is the formula; the top row feeds `row_input`, the left column
///   feeds `col_input`.
pub(crate) fn run(
    x: &mut XlsxState,
    sheet: usize,
    range: RangeRef,
    row_input: Option<(u32, u32)>,
    col_input: Option<(u32, u32)>,
) -> Result<Vec<CellRef>, CommandError> {
    if row_input.is_none() && col_input.is_none() {
        return Err(bad("Enter a row input cell or a column input cell."));
    }
    if range.last_row <= range.first_row || range.last_col <= range.first_col {
        return Err(bad("Select the input values and the formula first."));
    }
    let (r0, c0) = (range.first_row, range.first_col);
    let mut saved = Vec::new();
    for cell in [row_input, col_input].into_iter().flatten() {
        let snap = x
            .capture(sheet, cell.0, cell.1)
            .ok_or_else(|| bad("Input cell cannot be restored."))?;
        saved.push((cell, snap));
    }
    let mut results: Vec<(u32, u32, CellEntry)> = Vec::new();
    let outcome = (|| -> Result<(), CommandError> {
        for r in r0 + 1..=range.last_row {
            for c in c0 + 1..=range.last_col {
                let formula_cell = match (row_input, col_input) {
                    (Some(_), Some(_)) => (r0, c0),
                    (None, Some(_)) => (r0, c),
                    _ => (r, c0),
                };
                if let Some((ir, ic)) = row_input {
                    let v = entry(x, sheet, r0, c);
                    x.set_value(sheet, ir, ic, &v)?;
                }
                if let Some((ir, ic)) = col_input {
                    let v = entry(x, sheet, r, c0);
                    x.set_value(sheet, ir, ic, &v)?;
                }
                results.push((r, c, entry(x, sheet, formula_cell.0, formula_cell.1)));
            }
        }
        Ok(())
    })();
    let mut changed = Vec::new();
    for ((ir, ic), snap) in &saved {
        for cell in x.restore(sheet, *ir, *ic, snap)? {
            if !changed.contains(&cell) {
                changed.push(cell);
            }
        }
    }
    outcome?;
    for (r, c, v) in results {
        for cell in x.set_value(sheet, r, c, &v)? {
            if !changed.contains(&cell) {
                changed.push(cell);
            }
        }
    }
    Ok(changed)
}
