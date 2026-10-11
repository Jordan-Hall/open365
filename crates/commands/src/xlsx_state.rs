//! A workbook plus a lazily built calculation engine kept in step with it.

use open365_calc::{CellAddr, Engine, ErrorKind, Value, parse_formula};
use open365_xlsx::{CellError, CellInput, CellValue, SheetKind, Workbook};

use crate::command::{CellEntry, CellRef};
use crate::error::CommandError;

/// A cell as it was before an edit, enough to put it back.
#[derive(Debug, Clone)]
pub(crate) struct CellSnapshot {
    pub value: CellValue,
    /// Formula text without `=`.
    pub formula: Option<String>,
}

impl CellSnapshot {
    /// Errors and ISO dates stored as constants cannot be written back through the
    /// workbook API, so an edit over one is not undoable.
    fn restorable(&self) -> bool {
        self.formula.is_some()
            || matches!(
                self.value,
                CellValue::Empty
                    | CellValue::Number(_)
                    | CellValue::Bool(_)
                    | CellValue::Str(_)
                    | CellValue::SharedString { .. }
                    | CellValue::InlineStr(_)
            )
    }
}

#[derive(Debug)]
pub(crate) struct XlsxState {
    pub workbook: Workbook,
    engine: Option<Engine>,
}

impl XlsxState {
    pub fn new(workbook: Workbook) -> Self {
        Self {
            workbook,
            engine: None,
        }
    }

    /// The cell before an edit; `None` when it cannot be restored.
    pub fn capture(&self, sheet: usize, row: u32, col: u32) -> Option<CellSnapshot> {
        let snap = match self.workbook.cell(sheet, row, col) {
            None => CellSnapshot {
                value: CellValue::Empty,
                formula: None,
            },
            Some(cell) => CellSnapshot {
                value: cell.value.clone(),
                formula: if cell.formula.is_some() {
                    self.workbook.formula_text(sheet, row, col)
                } else {
                    None
                },
            },
        };
        snap.restorable().then_some(snap)
    }

    /// Puts a captured cell back and recalculates.
    pub fn restore(
        &mut self,
        sheet: usize,
        row: u32,
        col: u32,
        snap: &CellSnapshot,
    ) -> Result<Vec<CellRef>, CommandError> {
        match &snap.formula {
            Some(text) => self.set_formula(sheet, row, col, text),
            None => {
                let entry = match &snap.value {
                    CellValue::Number(n) => CellEntry::Number(*n),
                    CellValue::Bool(b) => CellEntry::Bool(*b),
                    CellValue::Str(t) | CellValue::InlineStr(t) => CellEntry::Text(t.to_string()),
                    CellValue::SharedString { text, .. } => CellEntry::Text(text.to_string()),
                    _ => CellEntry::Empty,
                };
                self.set_value(sheet, row, col, &entry)
            }
        }
    }

    pub fn set_value(
        &mut self,
        sheet: usize,
        row: u32,
        col: u32,
        entry: &CellEntry,
    ) -> Result<Vec<CellRef>, CommandError> {
        self.ensure_engine();
        let input = match entry {
            CellEntry::Empty => CellInput::Empty,
            CellEntry::Number(n) => CellInput::Number(*n),
            CellEntry::Text(t) => CellInput::Text(t.clone()),
            CellEntry::Bool(b) => CellInput::Bool(*b),
        };
        self.workbook.set_cell_value(sheet, row, col, input)?;
        let name = self.sheet_name(sheet);
        if let (Some(engine), Some(name)) = (self.engine.as_mut(), name) {
            let value = match entry {
                CellEntry::Empty => Value::Empty,
                CellEntry::Number(n) => Value::Number(*n),
                CellEntry::Text(t) => Value::Text(t.clone()),
                CellEntry::Bool(b) => Value::Bool(*b),
            };
            engine.set_value(&name, row, col, value);
        }
        let mut changed = vec![CellRef { sheet, row, col }];
        self.recalc(&mut changed)?;
        Ok(changed)
    }

    pub fn set_formula(
        &mut self,
        sheet: usize,
        row: u32,
        col: u32,
        formula: &str,
    ) -> Result<Vec<CellRef>, CommandError> {
        parse_formula(formula).map_err(|e| CommandError::Formula(e.to_string()))?;
        self.ensure_engine();
        self.workbook
            .set_cell_formula(sheet, row, col, formula, None)?;
        let name = self.sheet_name(sheet);
        if let (Some(engine), Some(name)) = (self.engine.as_mut(), name) {
            engine
                .set_formula(&name, row, col, formula)
                .map_err(|e| CommandError::Formula(e.to_string()))?;
        }
        let mut changed = vec![CellRef { sheet, row, col }];
        self.recalc(&mut changed)?;
        Ok(changed)
    }

    fn sheet_name(&self, sheet: usize) -> Option<String> {
        self.workbook.sheets().get(sheet).map(|s| s.name.clone())
    }

    /// Builds the engine from the workbook the first time an edit needs it. Cached values
    /// from the file stay in the workbook: the initial recalculation only primes the
    /// engine and its results are discarded.
    fn ensure_engine(&mut self) {
        if self.engine.is_some() {
            return;
        }
        self.engine = Some(build_engine(&self.workbook, &|_, _, _| true));
    }

    /// Recalculates dirty formulas and writes changed results into the workbook as cached
    /// values. Cells that changed are appended to `changed`.
    fn recalc(&mut self, changed: &mut Vec<CellRef>) -> Result<(), CommandError> {
        let Some(engine) = self.engine.as_mut() else {
            return Ok(());
        };
        let recomputed = engine.recalc_dirty();
        let updates: Vec<(CellAddr, Value)> = recomputed
            .into_iter()
            .map(|a| {
                let v = engine.get(&a.sheet, a.row, a.col);
                (a, v)
            })
            .collect();
        for (addr, value) in updates {
            let Some(sheet) = self
                .workbook
                .sheets()
                .iter()
                .position(|s| s.name.eq_ignore_ascii_case(&addr.sheet))
            else {
                continue;
            };
            let Some(text) = self.workbook.formula_text(sheet, addr.row, addr.col) else {
                continue;
            };
            let cached = to_cell_value(&value);
            let same = self
                .workbook
                .cell(sheet, addr.row, addr.col)
                .is_some_and(|c| c.value == cached);
            if same {
                continue;
            }
            self.workbook
                .set_cell_formula(sheet, addr.row, addr.col, &text, Some(cached))?;
            let r = CellRef {
                sheet,
                row: addr.row,
                col: addr.col,
            };
            if !changed.contains(&r) {
                changed.push(r);
            }
        }
        Ok(())
    }
}

/// Builds a calculation engine from the workbook. Cached values stay as the file has them:
/// the initial recalculation only primes the engine. `keep(sheet, row, col)` leaves cells
/// out (a hidden row, say); a sheet is left out when `keep(sheet, 0, 0)` is false.
pub(crate) fn build_engine(wb: &Workbook, keep: &dyn Fn(usize, u32, u32) -> bool) -> Engine {
    let mut engine = Engine::new();
    engine.set_date1904(wb.date1904());
    for (i, info) in wb.sheets().iter().enumerate() {
        if info.kind != SheetKind::Worksheet || !keep(i, 0, 0) {
            continue;
        }
        engine.add_sheet(&info.name);
        let Some(ws) = wb.sheet(i) else { continue };
        for (row, col, cell) in ws.iter_cells() {
            if !keep(i, row, col) {
                continue;
            }
            if cell.formula.is_some() {
                // Seed the cached value so the engine starts from the file's results.
                engine.set_value(&info.name, row, col, to_calc_value(&cell.value));
                // Non-master cells of a legacy array formula have no text (`None`), so
                // they keep their cached values. The engine has no array formula or
                // sheet-scoped name support yet: see docs/architecture/xlsx-model.md,
                // "What calc needs".
                if let Some(text) = ws.formula_text(row, col) {
                    // An unparseable formula keeps its cached value as a constant.
                    let _ = engine.set_formula(&info.name, row, col, &text);
                }
            } else if !matches!(cell.value, CellValue::Empty) {
                engine.set_value(&info.name, row, col, to_calc_value(&cell.value));
            }
        }
    }
    // Only workbook-scoped names: `Engine::define_name` has no sheet scope.
    for name in wb.defined_names() {
        if name.local_sheet_id.is_none() {
            let _ = engine.define_name(&name.name, &name.formula);
        }
    }
    engine.recalc_dirty();
    engine
}

pub(crate) fn to_calc_value(v: &CellValue) -> Value {
    match v {
        CellValue::Empty => Value::Empty,
        CellValue::Number(n) => Value::Number(*n),
        CellValue::Bool(b) => Value::Bool(*b),
        CellValue::Error(e) => Value::Error(match e {
            CellError::Null => ErrorKind::Null,
            CellError::Div0 => ErrorKind::DivZero,
            CellError::Value => ErrorKind::Value,
            CellError::Ref => ErrorKind::Ref,
            CellError::Name => ErrorKind::Name,
            CellError::Num => ErrorKind::Num,
            CellError::NA => ErrorKind::NA,
            CellError::GettingData => ErrorKind::GettingData,
            CellError::Other(code) => ErrorKind::parse(code).unwrap_or(ErrorKind::NA),
        }),
        other => Value::Text(other.as_text().unwrap_or_default().to_owned()),
    }
}

fn to_cell_value(v: &Value) -> CellValue {
    match v.top_left() {
        // A formula that points at a blank cell shows 0.
        Value::Empty => CellValue::Number(0.0),
        Value::Number(n) if n.is_finite() => CellValue::Number(n),
        Value::Number(_) => CellValue::Error(CellError::Num),
        Value::Text(t) => CellValue::Str(t.into()),
        Value::Bool(b) => CellValue::Bool(b),
        Value::Error(e) => CellValue::Error(match e {
            ErrorKind::Null => CellError::Null,
            ErrorKind::DivZero => CellError::Div0,
            ErrorKind::Value => CellError::Value,
            ErrorKind::Ref => CellError::Ref,
            ErrorKind::Name => CellError::Name,
            ErrorKind::Num => CellError::Num,
            ErrorKind::NA => CellError::NA,
            ErrorKind::GettingData => CellError::GettingData,
            other => CellError::Other(other.as_str().into()),
        }),
        Value::Array(_) => CellValue::Error(CellError::Value),
    }
}
