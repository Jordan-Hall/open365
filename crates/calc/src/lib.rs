//! Open365 formula engine (ADR-009): Excel-compatible formula parser, evaluator and dependency graph.
//!
//! Layers, bottom up: [`value`] (cell values, errors, coercion), [`ast`] and [`parser`]
//! (A1 formula grammar), [`eval`] (evaluator over a [`CellSource`]), [`functions`] (the worksheet
//! function registry), [`graph`] (dependency graph with a range index) and [`Engine`], a small
//! in-memory workbook that ties them together. All coordinates are 1-based.

pub mod ast;
mod collate;
mod datetext;
pub mod eval;
mod fmtlocale;
pub mod functions;
pub mod graph;
pub mod numfmt;
pub mod parser;
pub mod value;
pub mod xmath;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub use ast::{Ast, to_formula_string};
pub use eval::{CellSource, Clock, EvalContext, FixedClock, SystemClock, evaluate, evaluate_root};
pub use fmtlocale::{DateOrder, FormatLocale, cldr_to_excel};
pub use parser::{ParseError, ParseErrorKind, parse_formula};
pub use value::{ErrorKind, Value};

use graph::{CellId, Dep, Graph, extract_deps};

/// Most extra passes over `OFFSET`/`INDIRECT` formulas in one recalculation (the longest chain of them).
const MAX_DYNAMIC_ROUNDS: usize = 64;

/// A cell address with its sheet name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CellAddr {
    /// Sheet name.
    pub sheet: String,
    /// 1-based row.
    pub row: u32,
    /// 1-based column.
    pub col: u32,
}

#[derive(Debug)]
struct Formula {
    ast: Ast,
    names: Vec<String>,
    /// The outermost add/subtract may snap a near-zero result to zero (false for `=(A1-B1)`).
    snap_root: bool,
    /// The formula contains `SUBTOTAL` or `AGGREGATE`; other subtotals ignore this cell.
    has_subtotal: bool,
}

/// True when the whole formula text sits inside one pair of parentheses, as in `=(A1-B1)`.
fn fully_parenthesized(formula: &str) -> bool {
    let text = formula.trim().trim_start_matches('=').trim();
    if !text.starts_with('(') {
        return false;
    }
    let (mut depth, mut in_string) = (0usize, false);
    for (i, c) in text.char_indices() {
        match c {
            '"' => in_string = !in_string,
            '(' if !in_string => depth += 1,
            ')' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return i + 1 == text.len();
                }
            }
            _ => {}
        }
    }
    false
}

#[derive(Debug)]
struct CellData {
    value: Value,
    formula: Option<Formula>,
}

#[derive(Debug)]
struct SheetData {
    name: String,
    /// False for a placeholder created because a formula mentions an unknown sheet.
    exists: bool,
    cells: HashMap<(u32, u32), CellData>,
    max_row: u32,
    max_col: u32,
}

/// An in-memory workbook: constant values and formulas per sheet, a dependency graph and
/// cached formula results. Formulas are evaluated by [`Engine::recalc_dirty`].
///
/// Cells that are part of a reference cycle hold `0` (as Excel does) and are reported by
/// [`Engine::is_circular`]. Formulas returning arrays do not spill: [`Engine::get`] returns
/// the top-left element and [`Engine::get_full`] the whole array.
#[derive(Debug)]
pub struct Engine {
    sheets: Vec<SheetData>,
    graph: Graph,
    names: HashMap<String, Ast>,
    name_users: HashMap<String, HashSet<CellId>>,
    date1904: bool,
    locale: Arc<FormatLocale>,
    clock: Box<dyn Clock>,
    dirty: HashSet<CellId>,
    circular: HashSet<CellId>,
    /// Formulas that call `OFFSET` or `INDIRECT`.
    dynamic: HashSet<CellId>,
    /// `ISFORMULA` watchers: `(formula cell, sheet, row1, col1, row2, col2)` of the cell it asks about. They are
    /// recalculated when that cell gains or loses a formula but are not graph dependencies.
    watchers: Vec<(CellId, u32, u32, u32, u32, u32)>,
    all_dirty: bool,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// An empty workbook using the system clock and the 1900 date system.
    pub fn new() -> Self {
        Self::with_clock(Box::new(SystemClock))
    }

    /// An empty workbook with an injected clock (for `TODAY`/`NOW`).
    pub fn with_clock(clock: Box<dyn Clock>) -> Self {
        Engine {
            sheets: Vec::new(),
            graph: Graph::new(),
            names: HashMap::new(),
            name_users: HashMap::new(),
            date1904: false,
            locale: FormatLocale::en_us(),
            clock,
            dirty: HashSet::new(),
            circular: HashSet::new(),
            dynamic: HashSet::new(),
            watchers: Vec::new(),
            all_dirty: false,
        }
    }

    /// Replaces the clock. Volatile formulas pick it up on the next recalculation.
    pub fn set_clock(&mut self, clock: Box<dyn Clock>) {
        self.clock = clock;
    }

    /// Whether the workbook uses the 1904 date system.
    pub fn date1904(&self) -> bool {
        self.date1904
    }

    /// Selects the date system and schedules a full recalculation.
    pub fn set_date1904(&mut self, date1904: bool) {
        if self.date1904 != date1904 {
            self.date1904 = date1904;
            self.all_dirty = true;
        }
    }

    /// The locale of the workbook (default en-US): number-format names, date text and currency.
    pub fn locale(&self) -> &FormatLocale {
        &self.locale
    }

    /// Selects the locale and schedules a full recalculation.
    pub fn set_locale(&mut self, locale: &open365_locale::Locale) {
        self.set_format_locale(Arc::new(FormatLocale::new(locale)));
    }

    /// Like [`Engine::set_locale`] with locale data built earlier (building it takes milliseconds).
    pub fn set_format_locale(&mut self, locale: Arc<FormatLocale>) {
        self.locale = locale;
        self.all_dirty = true;
    }

    /// Adds an (empty) sheet; existing sheets are left alone.
    pub fn add_sheet(&mut self, name: &str) {
        self.ensure_sheet(name);
    }

    /// Names of all existing sheets in creation order.
    pub fn sheet_names(&self) -> Vec<&str> {
        self.sheets
            .iter()
            .filter(|s| s.exists)
            .map(|s| s.name.as_str())
            .collect()
    }

    fn find_sheet(&self, name: &str) -> Option<usize> {
        self.sheets
            .iter()
            .position(|s| s.name.eq_ignore_ascii_case(name))
    }

    fn push_sheet(&mut self, name: &str, exists: bool) -> usize {
        self.sheets.push(SheetData {
            name: name.to_owned(),
            exists,
            cells: HashMap::new(),
            max_row: 0,
            max_col: 0,
        });
        self.sheets.len() - 1
    }

    /// Finds or creates a real sheet.
    fn ensure_sheet(&mut self, name: &str) -> u32 {
        let index = match self.find_sheet(name) {
            Some(i) => {
                if !self.sheets[i].exists {
                    self.sheets[i].exists = true;
                    // Formulas that referred to the missing sheet evaluated to #REF!.
                    self.all_dirty = true;
                }
                i
            }
            None => self.push_sheet(name, true),
        };
        index as u32
    }

    /// Finds a sheet or creates a placeholder so dependencies can be recorded against it.
    fn intern_sheet(&mut self, name: &str) -> u32 {
        match self.find_sheet(name) {
            Some(i) => i as u32,
            None => self.push_sheet(name, false) as u32,
        }
    }

    fn id(&mut self, sheet: &str, row: u32, col: u32) -> CellId {
        CellId {
            sheet: self.ensure_sheet(sheet),
            row,
            col,
        }
    }

    fn addr(&self, id: CellId) -> CellAddr {
        CellAddr {
            sheet: self.sheets[id.sheet as usize].name.clone(),
            row: id.row,
            col: id.col,
        }
    }

    fn forget_names(&mut self, id: CellId) {
        let used = self.sheets[id.sheet as usize]
            .cells
            .get_mut(&(id.row, id.col))
            .and_then(|c| c.formula.take())
            .map(|f| f.names)
            .unwrap_or_default();
        for name in used {
            if let Some(users) = self.name_users.get_mut(&name.to_ascii_lowercase()) {
                users.remove(&id);
            }
        }
    }

    fn drop_formula(&mut self, id: CellId) {
        self.forget_names(id);
        self.graph.remove(id);
        self.dirty.remove(&id);
        self.circular.remove(&id);
        self.dynamic.remove(&id);
        self.watchers.retain(|w| w.0 != id);
    }

    fn mark_dependents(&mut self, id: CellId) {
        let mut roots = vec![id];
        roots.extend(
            self.watchers
                .iter()
                .filter(|&&(watcher, sheet, r1, c1, r2, c2)| {
                    watcher != id
                        && sheet == id.sheet
                        && (r1..=r2).contains(&id.row)
                        && (c1..=c2).contains(&id.col)
                })
                .map(|w| w.0),
        );
        let mut dependents = self.graph.dirty_from(&roots);
        dependents.extend(roots.into_iter().skip(1));
        self.dirty.extend(dependents);
    }

    fn touch(&mut self, id: CellId) {
        let sheet = &mut self.sheets[id.sheet as usize];
        sheet.max_row = sheet.max_row.max(id.row);
        sheet.max_col = sheet.max_col.max(id.col);
    }

    /// Stores a constant (an empty value clears the cell) and marks its dependents dirty.
    pub fn set_value(&mut self, sheet: &str, row: u32, col: u32, value: Value) {
        let id = self.id(sheet, row, col);
        self.drop_formula(id);
        let cells = &mut self.sheets[id.sheet as usize].cells;
        if value == Value::Empty {
            cells.remove(&(row, col));
        } else {
            cells.insert(
                (row, col),
                CellData {
                    value,
                    formula: None,
                },
            );
            self.touch(id);
        }
        self.mark_dependents(id);
    }

    /// Clears a cell.
    pub fn clear(&mut self, sheet: &str, row: u32, col: u32) {
        self.set_value(sheet, row, col, Value::Empty);
    }

    /// Parses and stores a formula (a leading `=` is optional). The result is computed by
    /// the next [`Engine::recalc_dirty`]. On a parse error the cell is left unchanged.
    pub fn set_formula(
        &mut self,
        sheet: &str,
        row: u32,
        col: u32,
        formula: &str,
    ) -> Result<(), ParseError> {
        let ast = parse_formula(formula)?;
        let id = self.id(sheet, row, col);
        self.install_formula(id, ast, !fully_parenthesized(formula));
        Ok(())
    }

    fn install_formula(&mut self, id: CellId, ast: Ast, snap_root: bool) {
        let previous = self.sheets[id.sheet as usize]
            .cells
            .get(&(id.row, id.col))
            .map(|c| c.value.clone());
        self.drop_formula(id);
        let sheet_name = self.sheets[id.sheet as usize].name.clone();
        let deps = {
            let names = &self.names;
            extract_deps(&ast, &sheet_name, &|n| {
                names.get(&n.to_ascii_lowercase()).cloned()
            })
        };
        let graph_deps: Vec<Dep> = deps
            .refs
            .iter()
            .map(|r| {
                let sheet = self.intern_sheet(&r.sheet);
                Dep::from_corners(sheet, r.row1, r.col1, r.row2, r.col2)
            })
            .collect();
        self.graph.set_formula(id, graph_deps, deps.volatile);
        if deps.dynamic {
            self.dynamic.insert(id);
        }
        for w in &deps.watched {
            let sheet = self.intern_sheet(&w.sheet);
            self.watchers
                .push((id, sheet, w.row1, w.col1, w.row2, w.col2));
        }
        for name in &deps.names {
            self.name_users
                .entry(name.to_ascii_lowercase())
                .or_default()
                .insert(id);
        }
        self.sheets[id.sheet as usize].cells.insert(
            (id.row, id.col),
            CellData {
                value: previous.unwrap_or(Value::Empty),
                formula: Some(Formula {
                    ast,
                    names: deps.names,
                    snap_root,
                    has_subtotal: deps.subtotal,
                }),
            },
        );
        self.touch(id);
        self.dirty.insert(id);
        self.mark_dependents(id);
    }

    /// Defines (or redefines) a workbook-level name as a formula, e.g. `Sheet1!$A$1:$A$10`.
    pub fn define_name(&mut self, name: &str, formula: &str) -> Result<(), ParseError> {
        let ast = parse_formula(formula)?;
        self.names.insert(name.to_ascii_lowercase(), ast);
        self.refresh_name_users(name);
        Ok(())
    }

    /// Removes a defined name; formulas using it evaluate to `#NAME?`.
    pub fn remove_name(&mut self, name: &str) {
        self.names.remove(&name.to_ascii_lowercase());
        self.refresh_name_users(name);
    }

    fn refresh_name_users(&mut self, name: &str) {
        let users: Vec<CellId> = self
            .name_users
            .get(&name.to_ascii_lowercase())
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default();
        for id in users {
            let formula = self.sheets[id.sheet as usize]
                .cells
                .get(&(id.row, id.col))
                .and_then(|c| c.formula.as_ref())
                .map(|f| (f.ast.clone(), f.snap_root));
            if let Some((ast, snap_root)) = formula {
                self.install_formula(id, ast, snap_root);
            }
        }
    }

    fn cell_data(&self, sheet: &str, row: u32, col: u32) -> Option<&CellData> {
        let index = self.find_sheet(sheet)?;
        self.sheets[index].cells.get(&(row, col))
    }

    /// The cell's value; for array results the top-left element.
    pub fn get(&self, sheet: &str, row: u32, col: u32) -> Value {
        self.cell_data(sheet, row, col)
            .map_or(Value::Empty, |c| c.value.top_left())
    }

    /// The cell's value including whole arrays.
    pub fn get_full(&self, sheet: &str, row: u32, col: u32) -> Value {
        self.cell_data(sheet, row, col)
            .map_or(Value::Empty, |c| c.value.clone())
    }

    /// The canonical formula text (no leading `=`), if the cell holds a formula.
    pub fn get_formula(&self, sheet: &str, row: u32, col: u32) -> Option<String> {
        let formula = self.cell_data(sheet, row, col)?.formula.as_ref()?;
        Some(to_formula_string(&formula.ast))
    }

    /// Whether the cell is part of a reference cycle after the last recalculation.
    pub fn is_circular(&self, sheet: &str, row: u32, col: u32) -> bool {
        self.find_sheet(sheet).is_some_and(|i| {
            self.circular.contains(&CellId {
                sheet: i as u32,
                row,
                col,
            })
        })
    }

    /// All cells currently in a reference cycle.
    pub fn circular_cells(&self) -> Vec<CellAddr> {
        let mut cells: Vec<CellId> = self.circular.iter().copied().collect();
        cells.sort_unstable();
        cells.into_iter().map(|c| self.addr(c)).collect()
    }

    /// Formula cells waiting for recalculation.
    pub fn dirty_count(&self) -> usize {
        self.dirty.len()
    }

    /// The dependency graph, for inspection.
    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    /// Recalculates every dirty formula (plus volatile ones) in dependency order and returns
    /// the cells that were recomputed.
    pub fn recalc_dirty(&mut self) -> Vec<CellAddr> {
        if self.all_dirty {
            self.all_dirty = false;
            self.dirty.extend(self.graph.formula_cells());
        }
        let volatile: Vec<CellId> = self.graph.volatile_cells().collect();
        if !volatile.is_empty() {
            let downstream = self.graph.dirty_from(&volatile);
            self.dirty.extend(volatile);
            self.dirty.extend(downstream);
        }
        // One reading of the clock per pass, so `NOW()-NOW()` is 0 as in Excel.
        let now = FixedClock(self.clock.now_unix_seconds());
        let dirty = std::mem::take(&mut self.dirty);
        let plan = self.graph.plan(&dirty);
        let mut seen: HashSet<CellId> = HashSet::new();
        let mut recomputed = Vec::with_capacity(plan.order.len());
        self.run_plan(plan, &now, &mut seen, &mut recomputed);
        // `OFFSET` and `INDIRECT` read cells the dependency graph does not know, so the first pass may have
        // evaluated them before the cells they point at. Evaluate them (and what depends on them) again until
        // nothing changes; each round settles one more link of a chain of such formulas.
        if !self.dynamic.is_empty() {
            let dynamic: Vec<CellId> = self.dynamic.iter().copied().collect();
            for _ in 0..MAX_DYNAMIC_ROUNDS {
                let mut again = self.graph.dirty_from(&dynamic);
                again.extend(dynamic.iter().copied());
                let plan = self.graph.plan(&again);
                if !self.run_plan(plan, &now, &mut seen, &mut recomputed) {
                    break;
                }
            }
        }
        recomputed
    }

    /// Evaluates the cells of a plan in order; true when any value changed.
    fn run_plan(
        &mut self,
        plan: graph::RecalcPlan,
        now: &FixedClock,
        seen: &mut HashSet<CellId>,
        recomputed: &mut Vec<CellAddr>,
    ) -> bool {
        let mut changed = false;
        for id in plan.order {
            let value = if plan.cyclic.contains(&id) {
                self.circular.insert(id);
                Value::Number(0.0)
            } else {
                self.circular.remove(&id);
                self.eval_cell(id, now)
            };
            if let Some(cell) = self.sheets[id.sheet as usize]
                .cells
                .get_mut(&(id.row, id.col))
            {
                changed |= cell.value != value;
                cell.value = value;
            }
            if seen.insert(id) {
                recomputed.push(self.addr(id));
            }
        }
        changed
    }

    /// Recalculates every formula in the workbook.
    pub fn recalc_all(&mut self) -> Vec<CellAddr> {
        self.all_dirty = true;
        self.recalc_dirty()
    }

    fn eval_cell(&self, id: CellId, clock: &dyn Clock) -> Value {
        let sheet = &self.sheets[id.sheet as usize];
        let Some(formula) = sheet
            .cells
            .get(&(id.row, id.col))
            .and_then(|c| c.formula.as_ref())
        else {
            return Value::Empty;
        };
        let ctx = EvalContext {
            source: self,
            sheet: &sheet.name,
            cell: Some((id.row, id.col)),
            date1904: self.date1904,
            clock,
            locale: &self.locale,
        };
        evaluate_root(&formula.ast, &ctx, formula.snap_root)
    }
}

impl CellSource for Engine {
    fn cell(&self, sheet: &str, row: u32, col: u32) -> Value {
        self.cell_data(sheet, row, col)
            .map_or(Value::Empty, |c| c.value.top_left())
    }

    fn name(&self, name: &str) -> Option<Ast> {
        self.names.get(&name.to_ascii_lowercase()).cloned()
    }

    fn sheet_exists(&self, sheet: &str) -> bool {
        self.find_sheet(sheet)
            .is_some_and(|i| self.sheets[i].exists)
    }

    fn has_formula(&self, sheet: &str, row: u32, col: u32) -> bool {
        self.cell_data(sheet, row, col)
            .is_some_and(|c| c.formula.is_some())
    }

    fn is_subtotal(&self, sheet: &str, row: u32, col: u32) -> bool {
        self.cell_data(sheet, row, col)
            .and_then(|c| c.formula.as_ref())
            .is_some_and(|f| f.has_subtotal)
    }

    fn used_bounds(&self, sheet: &str) -> (u32, u32) {
        self.find_sheet(sheet)
            .map_or((0, 0), |i| (self.sheets[i].max_row, self.sheets[i].max_col))
    }
}

#[cfg(test)]
mod engine_tests;
#[cfg(test)]
mod function_tests;
#[cfg(test)]
mod test_support;
