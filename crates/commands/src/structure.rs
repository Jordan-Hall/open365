//! Insert and delete rows and columns: the bridge between the workbook model and the formula engine's
//! reference rewriting.

use std::cell::Cell;

use open365_calc::parse_formula;
use open365_calc::refshift::{
    Axis, EditKind, StructuralEdit, depends_on_edit_band, shift_anchored_text, shift_formula_text,
    shift_name_text,
};
use open365_xlsx::{EditAxis, FormulaShifter, StructEdit, Workbook};

use crate::command::Command;
use crate::error::CommandError;

/// Which structural edit a command is, as `(sheet, edit)`.
pub(crate) fn edit_of(command: &Command) -> Option<(usize, StructEdit)> {
    let (sheet, axis, delete, at, count) = match command {
        Command::InsertRows { sheet, at, count } => (*sheet, EditAxis::Rows, false, *at, *count),
        Command::DeleteRows { sheet, at, count } => (*sheet, EditAxis::Rows, true, *at, *count),
        Command::InsertColumns { sheet, at, count } => (*sheet, EditAxis::Cols, false, *at, *count),
        Command::DeleteColumns { sheet, at, count } => (*sheet, EditAxis::Cols, true, *at, *count),
        _ => return None,
    };
    Some((
        sheet,
        StructEdit {
            axis,
            delete,
            at,
            count,
        },
    ))
}

/// Rewrites formulas with the calc crate's rules.
struct CalcShifter {
    edit: StructuralEdit,
    /// Formulas that did not parse and were left as they are.
    skipped: Cell<usize>,
}

impl FormulaShifter for CalcShifter {
    fn shift(&self, text: &str, home: &str) -> Option<String> {
        match shift_formula_text(text, Some(home), &self.edit) {
            Ok(new) => new,
            Err(_) => {
                self.skipped.set(self.skipped.get() + 1);
                None
            }
        }
    }

    fn shift_name(&self, text: &str) -> Option<String> {
        shift_name_text(text, &self.edit).ok().flatten()
    }

    fn shift_anchored(&self, text: &str, home: &str, delta: i64) -> Option<String> {
        shift_anchored_text(text, home, &self.edit, delta)
            .ok()
            .flatten()
    }
}

/// What the edit did, for the command outcome.
pub(crate) struct Applied {
    /// Formula cells whose result may have changed, at their new positions.
    pub recalc: Vec<(usize, u32, u32)>,
    pub notes: Vec<String>,
}

/// Applies the edit to the workbook model. On an error the workbook is left as it was.
pub(crate) fn apply(
    workbook: &mut Workbook,
    sheet: usize,
    edit: StructEdit,
) -> Result<Applied, CommandError> {
    let name = workbook
        .sheets()
        .get(sheet)
        .map(|s| s.name.clone())
        .ok_or_else(|| {
            CommandError::Xlsx(open365_xlsx::XlsxError::SheetOutOfRange {
                index: sheet,
                count: workbook.sheets().len(),
            })
        })?;
    let shifter = CalcShifter {
        edit: StructuralEdit {
            sheet: name,
            axis: match edit.axis {
                EditAxis::Rows => Axis::Row,
                EditAxis::Cols => Axis::Col,
            },
            kind: if edit.delete {
                EditKind::Delete
            } else {
                EditKind::Insert
            },
            at: edit.at,
            count: edit.count,
        },
        skipped: Cell::new(0),
    };
    let backup = workbook.clone();
    match workbook.edit_structure(sheet, edit, &shifter) {
        Ok(mut recalc) => {
            // Formulas whose references did not move but whose answer may: whole-column ranges, names, tables.
            let known: std::collections::HashSet<(usize, u32, u32)> =
                recalc.iter().copied().collect();
            for (i, info) in workbook.sheets().iter().enumerate() {
                let Some(ws) = workbook.sheet(i) else {
                    continue;
                };
                for (row, col, cell) in ws.iter_cells() {
                    if cell.formula.is_none() || known.contains(&(i, row, col)) {
                        continue;
                    }
                    let Some(text) = ws.formula_text(row, col) else {
                        continue;
                    };
                    if parse_formula(&text)
                        .is_ok_and(|ast| depends_on_edit_band(&ast, &info.name, &shifter.edit))
                    {
                        recalc.push((i, row, col));
                    }
                }
            }
            let mut notes = Vec::new();
            if shifter.skipped.get() > 0 {
                notes.push(format!(
                    "{} formula(s) could not be parsed and their references were not adjusted",
                    shifter.skipped.get()
                ));
            }
            Ok(Applied { recalc, notes })
        }
        Err(e) => {
            *workbook = backup;
            Err(e.into())
        }
    }
}
