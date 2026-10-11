//! Formula evaluator: turns an [`Ast`] into a [`Value`] against a [`CellSource`].
//!
//! References are kept lazy ([`Arg::Range`]) until a function or operator needs their cells,
//! so whole-column references never allocate per cell. Binary operators lift over arrays
//! element-wise; scalar function parameters take the implicit intersection of a range with the
//! formula's cell, or `#VALUE!` when there is none.

use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::fmt;

use crate::ast::{AreaKind, AreaRef, Ast, BinOp, CellRef, MAX_COL, MAX_ROW, UnaryOp};
use std::sync::{Arc, OnceLock};

use crate::fmtlocale::{FormatLocale, Scope};
use crate::functions::{self, Func, Lifting};
use crate::value::{ErrorKind, Value, compare_values};

mod lambda;

pub use lambda::Lambda;

const MAX_NAME_DEPTH: u32 = 16;
/// Largest range that arithmetic will materialise as an array.
const MAX_ARRAY_CELLS: u64 = 1 << 20;

/// Read access to a workbook. Rows and columns are 1-based.
pub trait CellSource {
    /// The value of a cell; [`Value::Empty`] for blank cells and unknown sheets.
    fn cell(&self, sheet: &str, row: u32, col: u32) -> Value;

    /// The formula of a defined name, if the name exists (workbook scope).
    fn name(&self, name: &str) -> Option<Ast>;

    /// The formula of a defined name as seen from a formula on `sheet`: a name scoped to that sheet hides a
    /// workbook-scoped name of the same spelling.
    fn name_in(&self, _sheet: &str, name: &str) -> Option<Ast> {
        self.name(name)
    }

    /// A table by name (case-insensitive), for structured references.
    fn table(&self, _name: &str) -> Option<crate::tables::PlacedTable> {
        None
    }

    /// Whether a sheet exists. References to missing sheets evaluate to `#REF!`.
    fn sheet_exists(&self, _sheet: &str) -> bool {
        true
    }

    /// Whether the cell holds a formula (for `ISFORMULA`).
    fn has_formula(&self, _sheet: &str, _row: u32, _col: u32) -> bool {
        false
    }

    /// The formula of a cell as text with its leading `=` (for `FORMULATEXT`); `None` for a cell without one.
    fn formula_text(&self, _sheet: &str, _row: u32, _col: u32) -> Option<String> {
        None
    }

    /// Whether the cell holds a formula that calls `SUBTOTAL` or `AGGREGATE` (such cells are skipped by those functions).
    fn is_subtotal(&self, _sheet: &str, _row: u32, _col: u32) -> bool {
        false
    }

    /// Number format, alignment and protection of a cell (for `CELL`).
    fn cell_style(&self, _sheet: &str, _row: u32, _col: u32) -> crate::style::CellStyle {
        crate::style::CellStyle::default()
    }

    /// Width of a column in characters and whether it is the standard width (for `CELL("width")`).
    fn column_width(&self, _sheet: &str, _col: u32) -> (f64, bool) {
        (crate::style::STANDARD_COLUMN_WIDTH, true)
    }

    /// Zero-based position of an existing sheet in the workbook (for `SHEET`).
    fn sheet_position(&self, _sheet: &str) -> Option<usize> {
        Some(0)
    }

    /// Number of sheets in the workbook (for `SHEETS`).
    fn sheet_count(&self) -> usize {
        1
    }

    /// The last used `(row, col)` of a sheet (0 when empty). Whole-column and whole-row
    /// references only scan up to these bounds.
    fn used_bounds(&self, _sheet: &str) -> (u32, u32) {
        (MAX_ROW, MAX_COL)
    }
}

/// Supplies the current time for `TODAY` and `NOW`.
pub trait Clock: fmt::Debug {
    /// Seconds since 1970-01-01 00:00:00 in the user's local time.
    fn now_unix_seconds(&self) -> f64;
}

/// Reads the operating-system clock. It has no time-zone database, so it reports UTC; hosts
/// that need local time inject their own [`Clock`].
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix_seconds(&self) -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64())
    }
}

/// A clock frozen at one instant, for tests and reproducible recalculation.
#[derive(Debug, Clone, Copy)]
pub struct FixedClock(pub f64);

impl Clock for FixedClock {
    fn now_unix_seconds(&self) -> f64 {
        self.0
    }
}

static SYSTEM_CLOCK: SystemClock = SystemClock;

/// Everything the evaluator needs besides the formula itself.
#[derive(Clone, Copy)]
pub struct EvalContext<'a> {
    /// Where cell values and names come from.
    pub source: &'a dyn CellSource,
    /// Sheet that unqualified references resolve against.
    pub sheet: &'a str,
    /// The cell holding the formula, used for implicit intersection.
    pub cell: Option<(u32, u32)>,
    /// Use the 1904 date system.
    pub date1904: bool,
    /// Time source for volatile date functions.
    pub clock: &'a dyn Clock,
    /// Locale for date text, currency and number-format names.
    pub locale: &'a Arc<FormatLocale>,
}

fn default_locale() -> &'static Arc<FormatLocale> {
    static EN_US: OnceLock<Arc<FormatLocale>> = OnceLock::new();
    EN_US.get_or_init(FormatLocale::en_us)
}

impl fmt::Debug for EvalContext<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvalContext")
            .field("sheet", &self.sheet)
            .field("cell", &self.cell)
            .field("date1904", &self.date1904)
            .finish_non_exhaustive()
    }
}

impl<'a> EvalContext<'a> {
    /// A context using the system clock and the 1900 date system.
    pub fn new(source: &'a dyn CellSource, sheet: &'a str) -> Self {
        EvalContext {
            source,
            sheet,
            cell: None,
            date1904: false,
            clock: &SYSTEM_CLOCK,
            locale: default_locale(),
        }
    }

    /// Selects the locale.
    #[must_use]
    pub fn with_locale(mut self, locale: &'a Arc<FormatLocale>) -> Self {
        self.locale = locale;
        self
    }

    /// Sets the formula cell (1-based row, column).
    #[must_use]
    pub fn at_cell(mut self, row: u32, col: u32) -> Self {
        self.cell = Some((row, col));
        self
    }

    /// Selects the date system.
    #[must_use]
    pub fn with_date1904(mut self, date1904: bool) -> Self {
        self.date1904 = date1904;
        self
    }

    /// Injects the clock.
    #[must_use]
    pub fn with_clock(mut self, clock: &'a dyn Clock) -> Self {
        self.clock = clock;
        self
    }

    /// Reduces any argument to one value: a range or array by implicit intersection.
    pub fn scalar(&self, arg: &Arg) -> Value {
        match arg {
            Arg::Missing => Value::Empty,
            Arg::Value(v) => v.clone(),
            // A scalar parameter given an array takes its first element.
            Arg::Array(rows) => rows
                .first()
                .and_then(|r| r.first())
                .cloned()
                .unwrap_or(Value::Error(ErrorKind::Value)),
            Arg::Range(r) => self.implicit_intersection(r),
            Arg::Union(_) | Arg::Lambda(_) => Value::Error(ErrorKind::Value),
            // Functions that do not know reference arrays see the first element.
            Arg::Refs(refs) => match refs.first().and_then(|row| row.first()) {
                Some(Ok(r)) => self.implicit_intersection(r),
                Some(Err(e)) => Value::Error(*e),
                None => Value::Error(ErrorKind::Value),
            },
        }
    }

    fn implicit_intersection(&self, r: &RangeRef) -> Value {
        if r.is_single_cell() {
            return self.source.cell(&r.sheet, r.row1, r.col1);
        }
        if let Some((row, col)) = self.cell {
            if r.col1 == r.col2 && (r.row1..=r.row2).contains(&row) {
                return self.source.cell(&r.sheet, row, r.col1);
            }
            if r.row1 == r.row2 && (r.col1..=r.col2).contains(&col) {
                return self.source.cell(&r.sheet, r.row1, col);
            }
            // A range of several rows and columns gives the cell at the formula's own row and column.
            if (r.row1..=r.row2).contains(&row) && (r.col1..=r.col2).contains(&col) {
                return self.source.cell(&r.sheet, row, col);
            }
        }
        Value::Error(ErrorKind::Value)
    }

    /// Iterates every cell value of an argument in row-major order. Ranges stop at the
    /// sheet's used bounds. A direct value yields itself once.
    pub fn cells<'s>(&'s self, arg: &'s Arg) -> Box<dyn Iterator<Item = Value> + 's> {
        match arg {
            Arg::Missing => Box::new(std::iter::empty()),
            Arg::Value(v) => Box::new(std::iter::once(v.clone())),
            Arg::Array(rows) => Box::new(rows.iter().flatten().cloned()),
            Arg::Range(r) => self.range_cells(r),
            Arg::Union(rs) => Box::new(rs.iter().flat_map(move |r| self.range_cells(r))),
            Arg::Refs(refs) => Box::new(refs.iter().flatten().map(move |e| self.ref_element(e))),
            Arg::Lambda(_) => Box::new(std::iter::once(Value::Error(ErrorKind::Value))),
        }
    }

    /// The value an aggregate function (`SUM`, `MAX`, ...) reads from one element of an array of references:
    /// the cell of a single-cell element, `#VALUE!` for a multi-cell one, the element's own error.
    fn ref_element(&self, element: &Result<RangeRef, ErrorKind>) -> Value {
        match element {
            Ok(r) if r.is_single_cell() => self.source.cell(&r.sheet, r.row1, r.col1),
            Ok(_) => Value::Error(ErrorKind::Value),
            Err(e) => Value::Error(*e),
        }
    }

    fn range_cells<'s>(&'s self, r: &'s RangeRef) -> Box<dyn Iterator<Item = Value> + 's> {
        let (rows, cols) = (self.scan_rows(r), self.scan_cols(r));
        Box::new((0..rows).flat_map(move |dr| {
            (0..cols).map(move |dc| self.source.cell(&r.sheet, r.row1 + dr, r.col1 + dc))
        }))
    }

    /// Ranges with more cells than this are clamped to the sheet's used area when scanned.
    /// Smaller ranges are scanned exactly so blank cells inside them are visible.
    const CLAMP_ABOVE_CELLS: u64 = 1 << 16;

    fn clamps(r: &RangeRef) -> bool {
        u64::from(r.rows()) * u64::from(r.cols()) > Self::CLAMP_ABOVE_CELLS
    }

    /// Number of rows of `r` worth scanning (clamped to the used area for very large ranges
    /// such as whole columns).
    pub fn scan_rows(&self, r: &RangeRef) -> u32 {
        if !Self::clamps(r) {
            return r.rows();
        }
        let (used_row, _) = self.source.used_bounds(&r.sheet);
        let last = r.row2.min(used_row);
        if last < r.row1 { 0 } else { last - r.row1 + 1 }
    }

    /// Number of columns of `r` worth scanning (clamped to the used area for very large
    /// ranges such as whole rows).
    pub fn scan_cols(&self, r: &RangeRef) -> u32 {
        if !Self::clamps(r) {
            return r.cols();
        }
        let (_, used_col) = self.source.used_bounds(&r.sheet);
        let last = r.col2.min(used_col);
        if last < r.col1 { 0 } else { last - r.col1 + 1 }
    }

    /// A random-access view of a rectangular argument, if it is one.
    pub fn grid<'s>(&'s self, arg: &'s Arg) -> Option<Grid<'s>> {
        let data = match arg {
            Arg::Range(r) => GridData::Range(r),
            Arg::Array(rows) => GridData::Rows(rows),
            Arg::Value(v) => GridData::Owned(vec![vec![v.clone()]]),
            // `SUMPRODUCT` sees references as non-numbers; an error element is the element's error.
            Arg::Refs(refs) => GridData::Owned(
                refs.iter()
                    .map(|row| {
                        row.iter()
                            .map(|e| match e {
                                Ok(_) => Value::Empty,
                                Err(e) => Value::Error(*e),
                            })
                            .collect()
                    })
                    .collect(),
            ),
            Arg::Lambda(_) => GridData::Owned(vec![vec![Value::Error(ErrorKind::Value)]]),
            Arg::Missing | Arg::Union(_) => return None,
        };
        Some(Grid { ctx: self, data })
    }
}

/// A rectangular reference with normalised corners (`row1 <= row2`, `col1 <= col2`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeRef {
    /// Sheet name.
    pub sheet: String,
    /// First row (1-based).
    pub row1: u32,
    /// First column (1-based).
    pub col1: u32,
    /// Last row.
    pub row2: u32,
    /// Last column.
    pub col2: u32,
}

impl RangeRef {
    /// Builds a range from any two corners.
    pub fn new(sheet: &str, row_a: u32, col_a: u32, row_b: u32, col_b: u32) -> Self {
        RangeRef {
            sheet: sheet.to_owned(),
            row1: row_a.min(row_b),
            col1: col_a.min(col_b),
            row2: row_a.max(row_b),
            col2: col_a.max(col_b),
        }
    }

    /// Number of rows spanned (not clamped).
    pub fn rows(&self) -> u32 {
        self.row2 - self.row1 + 1
    }

    /// Number of columns spanned (not clamped).
    pub fn cols(&self) -> u32 {
        self.col2 - self.col1 + 1
    }

    /// True for a 1x1 range.
    pub fn is_single_cell(&self) -> bool {
        self.row1 == self.row2 && self.col1 == self.col2
    }

    fn intersection(&self, other: &RangeRef) -> Option<RangeRef> {
        if !self.sheet.eq_ignore_ascii_case(&other.sheet) {
            return None;
        }
        let row1 = self.row1.max(other.row1);
        let row2 = self.row2.min(other.row2);
        let col1 = self.col1.max(other.col1);
        let col2 = self.col2.min(other.col2);
        (row1 <= row2 && col1 <= col2).then(|| RangeRef {
            sheet: self.sheet.clone(),
            row1,
            col1,
            row2,
            col2,
        })
    }
}

/// An evaluated function argument or intermediate result.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    /// An omitted argument (`IF(A1,,2)`).
    Missing,
    /// A scalar produced by an expression or literal. Never an array.
    Value(Value),
    /// An array constant or computed array.
    Array(Vec<Vec<Value>>),
    /// A reference; cells are read lazily.
    Range(RangeRef),
    /// A union of references such as `(A1:A3,C1:C3)`.
    Union(Vec<RangeRef>),
    /// An array of references, rows of elements, what `OFFSET` and `INDIRECT` return when an argument is an
    /// array. An element is a reference or the error `OFFSET`/`INDIRECT` gave for that position.
    Refs(Vec<Vec<Result<RangeRef, ErrorKind>>>),
    /// A function value: `LAMBDA(..)` or a bare function name such as `SUM` passed to `MAP`.
    Lambda(Arc<Lambda>),
}

impl Arg {
    /// Wraps a function result, splitting arrays out of scalar values.
    pub fn from_value(v: Value) -> Arg {
        match v {
            Value::Array(rows) => Arg::Array(rows),
            other => Arg::Value(other),
        }
    }

    /// An error result.
    pub fn error(e: ErrorKind) -> Arg {
        Arg::Value(Value::Error(e))
    }
}

/// Random access to a rectangular argument (0-based indices).
#[derive(Debug)]
pub struct Grid<'a> {
    ctx: &'a EvalContext<'a>,
    data: GridData<'a>,
}

#[derive(Debug)]
enum GridData<'a> {
    Range(&'a RangeRef),
    Rows(&'a [Vec<Value>]),
    Owned(Vec<Vec<Value>>),
}

impl Grid<'_> {
    /// Total rows, including unused rows of a range.
    pub fn rows(&self) -> usize {
        match &self.data {
            GridData::Range(r) => r.rows() as usize,
            GridData::Rows(rows) => rows.len(),
            GridData::Owned(rows) => rows.len(),
        }
    }

    /// Total columns.
    pub fn cols(&self) -> usize {
        match &self.data {
            GridData::Range(r) => r.cols() as usize,
            GridData::Rows(rows) => rows.first().map_or(0, Vec::len),
            GridData::Owned(rows) => rows.first().map_or(0, Vec::len),
        }
    }

    /// Rows worth scanning: for ranges clamped to the used area, else all rows.
    pub fn scan_rows(&self) -> usize {
        match &self.data {
            GridData::Range(r) => self.ctx.scan_rows(r) as usize,
            _ => self.rows(),
        }
    }

    /// Columns worth scanning.
    pub fn scan_cols(&self) -> usize {
        match &self.data {
            GridData::Range(r) => self.ctx.scan_cols(r) as usize,
            _ => self.cols(),
        }
    }

    /// The value at `(row, col)`; blank outside the grid.
    pub fn get(&self, row: usize, col: usize) -> Value {
        match &self.data {
            GridData::Range(r) => {
                if row >= r.rows() as usize || col >= r.cols() as usize {
                    return Value::Empty;
                }
                self.ctx
                    .source
                    .cell(&r.sheet, r.row1 + row as u32, r.col1 + col as u32)
            }
            GridData::Rows(rows) => rows
                .get(row)
                .and_then(|r| r.get(col))
                .cloned()
                .unwrap_or(Value::Empty),
            GridData::Owned(rows) => rows
                .get(row)
                .and_then(|r| r.get(col))
                .cloned()
                .unwrap_or(Value::Empty),
        }
    }
}

/// Evaluates a formula to a final cell value. A blank result becomes `0`; a multi-cell
/// reference result takes the implicit intersection; an array result is returned whole.
pub fn evaluate(ast: &Ast, ctx: &EvalContext<'_>) -> Value {
    evaluate_root(ast, ctx, true)
}

/// Like [`evaluate`]. When `snap_root` is true and the formula's outermost operation is `+` or `-`, a result
/// within binary noise of zero collapses to exactly zero, which is what Excel does for `=0.1+0.2-0.3` (but
/// not for `=(0.1+0.2-0.3)`, nor for the same expression inside a function argument). Pass false for a formula
/// written entirely inside parentheses.
pub fn evaluate_root(ast: &Ast, ctx: &EvalContext<'_>, snap_root: bool) -> Value {
    // Coercion of date text and currency reads the locale of the evaluation in progress.
    let _locale = Scope::enter(ctx.locale);
    let ev = Evaluator {
        ctx,
        name_depth: Cell::new(0),
        under_n: Cell::new(false),
        fn_depth: Cell::new(0),
        locals: RefCell::new(Vec::new()),
        lambda_depth: Cell::new(0),
        shift: Cell::new((0, 0)),
        cse: false,
    };
    let arg = match ast {
        Ast::Binary {
            op: op @ (BinOp::Add | BinOp::Sub),
            lhs,
            rhs,
        } if snap_root => ev.eval_binary(*op, lhs, rhs, false, true),
        _ => ev.eval(ast),
    };
    match arg {
        Arg::Missing => Value::Number(0.0),
        Arg::Value(Value::Empty) => Value::Number(0.0),
        Arg::Value(v) => v,
        Arg::Array(rows) => Value::Array(rows),
        Arg::Range(r) => match ctx.scalar(&Arg::Range(r)) {
            Value::Empty => Value::Number(0.0),
            v => v,
        },
        Arg::Union(_) => Value::Error(ErrorKind::Value),
        Arg::Lambda(_) => Value::Error(ErrorKind::Calc),
        // A cell shows the first element.
        refs @ Arg::Refs(_) => match ctx.scalar(&refs) {
            Value::Empty => Value::Number(0.0),
            v => v,
        },
    }
}

/// Evaluates the formula of a legacy array formula (`{=...}`): operators work over whole ranges, and the result
/// is the full array (a range is read into one; a blank element shows `0`).
pub fn evaluate_array_root(ast: &Ast, ctx: &EvalContext<'_>) -> Value {
    let _locale = Scope::enter(ctx.locale);
    let ev = Evaluator {
        ctx,
        name_depth: Cell::new(0),
        under_n: Cell::new(false),
        fn_depth: Cell::new(0),
        locals: RefCell::new(Vec::new()),
        lambda_depth: Cell::new(0),
        shift: Cell::new((0, 0)),
        cse: true,
    };
    let nonblank = |v: Value| {
        if v == Value::Empty {
            Value::Number(0.0)
        } else {
            v
        }
    };
    match ev.eval_in(ast, true) {
        Arg::Missing => Value::Number(0.0),
        Arg::Value(v) => nonblank(v),
        Arg::Array(rows) => Value::Array(rows),
        Arg::Range(r) if r.is_single_cell() => nonblank(ctx.scalar(&Arg::Range(r))),
        Arg::Range(r) => {
            let arg = Arg::Range(r);
            match ctx.grid(&arg) {
                Some(grid) => Value::Array(
                    (0..grid.scan_rows().max(1))
                        .map(|row| {
                            (0..grid.scan_cols().max(1))
                                .map(|col| nonblank(grid.get(row, col)))
                                .collect()
                        })
                        .collect(),
                ),
                None => Value::Error(ErrorKind::Value),
            }
        }
        Arg::Union(_) => Value::Error(ErrorKind::Value),
        Arg::Lambda(_) => Value::Error(ErrorKind::Calc),
        refs @ Arg::Refs(_) => nonblank(ctx.scalar(&refs)),
    }
}

struct Evaluator<'c, 'a> {
    ctx: &'c EvalContext<'a>,
    name_depth: Cell<u32>,
    /// Set by `N(OFFSET(..))`/`N(INDIRECT(..))`: Excel evaluates the call in array mode, so an array argument
    /// gives an array of references even outside an array context.
    under_n: Cell<bool>,
    /// How many function calls enclose the expression being evaluated. A call nested in another call's
    /// arguments is evaluated in array mode, so `OFFSET`/`INDIRECT` given an array return references per element.
    fn_depth: Cell<u32>,
    /// `LET` names and lambda parameters in scope, innermost last.
    locals: RefCell<Vec<(String, Arg)>>,
    /// How many lambda calls are in progress.
    lambda_depth: Cell<u32>,
    /// While a defined name is evaluated: how far its relative references move, `(rows, columns)`. The name's
    /// text is stored relative to `A1`, so this is the formula cell's offset from `A1`.
    shift: Cell<(i64, i64)>,
    /// Evaluating a legacy array formula (`{=...}`): every function argument is evaluated in array mode, so
    /// `IF(A1:A5>2,1,0)` works on the whole range.
    cse: bool,
}

/// Operand of an arithmetic operator after reference resolution.
enum Operand {
    Scalar(Value),
    Grid(Vec<Vec<Value>>),
}

impl Evaluator<'_, '_> {
    fn eval(&self, ast: &Ast) -> Arg {
        self.eval_in(ast, false)
    }

    /// Evaluates `ast`. `array` is true where Excel evaluates operators over whole ranges (the array
    /// parameters of `INDEX` and `XLOOKUP`); elsewhere an operator applied to a multi-cell range uses the
    /// implicit intersection of the range with the formula's cell, as in a non-array formula.
    fn eval_in(&self, ast: &Ast, array: bool) -> Arg {
        match ast {
            Ast::Number(n) => Arg::Value(Value::Number(*n)),
            Ast::Text(t) => Arg::Value(Value::Text(t.clone())),
            Ast::Bool(b) => Arg::Value(Value::Bool(*b)),
            Ast::Error(e) => Arg::error(*e),
            Ast::Missing => Arg::Missing,
            Ast::Ref(r) => self.eval_ref(r),
            Ast::Area(a) => self.eval_area(a),
            Ast::Name(n) => self.eval_name(n),
            Ast::Structured { table, spec } => self.eval_structured(table, spec),
            Ast::Func { name, args, .. } => self.eval_call(name, args, array),
            Ast::Unary { op, operand } => {
                let v = self.eval_in(operand, array);
                let v = if array { v } else { self.scalarize(operand, v) };
                self.unary(*op, v, array)
            }
            Ast::Invoke { callee, args } => {
                let f = self.eval_value(callee);
                let argv = self.call_args(args);
                self.invoke(&f, argv)
            }
            Ast::Percent(inner) => {
                let v = self.eval_in(inner, array);
                let v = if array { v } else { self.scalarize(inner, v) };
                let d = self.ctx.date1904;
                lift_unary(self.operand(v, array), |x| match x.to_number_in(d) {
                    Ok(n) => Value::Number(n / 100.0),
                    Err(e) => Value::Error(e),
                })
            }
            Ast::Binary { op, lhs, rhs } => self.eval_binary(*op, lhs, rhs, array, false),
            Ast::Array(rows) => self.eval_array(rows),
        }
    }

    fn eval_array(&self, rows: &[Vec<Ast>]) -> Arg {
        let out = rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|item| self.ctx.scalar(&self.eval(item)))
                    .collect()
            })
            .collect();
        Arg::Array(out)
    }

    fn sheet_of<'s>(&'s self, explicit: Option<&'s String>) -> &'s str {
        explicit.map_or(self.ctx.sheet, String::as_str)
    }

    fn eval_ref(&self, r: &CellRef) -> Arg {
        // A `LET` name or lambda parameter such as `x1` hides the cell of that name.
        if r.sheet.is_none() && !r.row_abs && !r.col_abs && !self.locals.borrow().is_empty() {
            let spelled = crate::ast::to_formula_string(&Ast::Ref(r.clone()));
            if let Some(local) = self.local(&spelled) {
                return local;
            }
        }
        let sheet = self.sheet_of(r.sheet.as_ref());
        if !self.ctx.source.sheet_exists(sheet) {
            return Arg::error(ErrorKind::Ref);
        }
        self.cell_range(r)
    }

    #[inline(never)]
    fn cell_range(&self, r: &CellRef) -> Arg {
        let sheet = self.sheet_of(r.sheet.as_ref());
        let (dr, dc) = self.shift.get();
        let row = crate::ast::shift_coord(r.row, r.row_abs, dr, MAX_ROW);
        let col = crate::ast::shift_coord(r.col, r.col_abs, dc, MAX_COL);
        Arg::Range(RangeRef::new(sheet, row, col, row, col))
    }

    fn eval_area(&self, a: &AreaRef) -> Arg {
        let sheet = self.sheet_of(a.sheet.as_ref());
        if !self.ctx.source.sheet_exists(sheet) {
            return Arg::error(ErrorKind::Ref);
        }
        self.area_range(a)
    }

    #[inline(never)]
    fn area_range(&self, a: &AreaRef) -> Arg {
        let sheet = self.sheet_of(a.sheet.as_ref());
        let (dr, dc) = self.shift.get();
        let shifted;
        let a = if (dr, dc) == (0, 0) {
            a
        } else {
            shifted = crate::ast::shift_area(a, dr, dc);
            &shifted
        };
        let (row1, row2, col1, col2) = match a.kind {
            AreaKind::Cells => (a.row1, a.row2, a.col1, a.col2),
            AreaKind::Cols => (1, MAX_ROW, a.col1, a.col2),
            AreaKind::Rows => (a.row1, a.row2, 1, MAX_COL),
        };
        Arg::Range(RangeRef::new(sheet, row1, col1, row2, col2))
    }

    /// A literal reference without the sheet check: `ROWS(Sheet9!A1)` and `ISREF(Sheet9!A1)` still see a
    /// reference to a sheet that does not exist.
    fn unchecked_ref(&self, ast: &Ast) -> Option<Arg> {
        match ast {
            Ast::Ref(r) => Some(self.cell_range(r)),
            Ast::Area(a) => Some(self.area_range(a)),
            Ast::Structured { table, spec } => Some(self.eval_structured(table, spec)),
            _ => None,
        }
    }

    /// `Table[Column]`: the rectangle it selects, `#REF!` for an unknown table or column.
    #[inline(never)]
    fn eval_structured(&self, table: &str, spec: &str) -> Arg {
        let Some(placed) = self.ctx.source.table(table) else {
            return Arg::error(ErrorKind::Ref);
        };
        let row = self.ctx.cell.map(|(row, _)| row);
        match crate::tables::resolve(&placed.def, spec, row) {
            Ok((row1, col1, row2, col2)) => {
                Arg::Range(RangeRef::new(&placed.sheet, row1, col1, row2, col2))
            }
            Err(e) => Arg::error(e),
        }
    }

    #[inline(never)]
    fn eval_name(&self, name: &str) -> Arg {
        if let Some(local) = self.local(name) {
            return local;
        }
        let Some(ast) = self.ctx.source.name_in(self.ctx.sheet, name) else {
            return Arg::error(ErrorKind::Name);
        };
        let depth = self.name_depth.get();
        if depth >= MAX_NAME_DEPTH {
            return Arg::error(ErrorKind::Name);
        }
        self.name_depth.set(depth + 1);
        let before = self.shift.get();
        if depth == 0
            && let Some((row, col)) = self.ctx.cell
        {
            self.shift.set((i64::from(row) - 1, i64::from(col) - 1));
        }
        let result = self.eval(&ast);
        self.shift.set(before);
        self.name_depth.set(depth);
        result
    }

    /// In a legacy array formula `TRANSPOSE(A1:C1)` shapes the whole range, not its intersection.
    #[inline(never)]
    fn whole_ranges_for_transpose(&self, name: &str, evaluated: Vec<Arg>) -> Vec<Arg> {
        if !self.cse || !name.eq_ignore_ascii_case("TRANSPOSE") {
            return evaluated;
        }
        evaluated
            .into_iter()
            .map(|a| match &a {
                Arg::Range(_) => match self.ctx.grid(&a) {
                    Some(g) => Arg::Array(
                        (0..g.scan_rows())
                            .map(|r| (0..g.scan_cols()).map(|c| g.get(r, c)).collect())
                            .collect(),
                    ),
                    None => a.clone(),
                },
                _ => a,
            })
            .collect()
    }

    fn eval_call(&self, name: &str, args: &[Ast], array: bool) -> Arg {
        if let Some(special) = self.eval_special(name, args) {
            return special;
        }
        if let Some(f @ Arg::Lambda(_)) = self.local(name) {
            let argv = self.call_args(args);
            return self.invoke(&f, argv);
        }
        let Some(func) = functions::lookup(name) else {
            return Arg::error(ErrorKind::Name);
        };
        let reference_fn = functions::takes_reference(name);
        // `OFFSET` and `INDIRECT` given an array return an array of references, and `N(OFFSET(..))` hands the
        // array context down to them.
        let under_n = self.under_n.replace(false);
        let nested = self.fn_depth.get() >= 2;
        let ref_fn = functions::ref_lifting(name).filter(|_| array || under_n || nested);
        let feeds_refs = name.eq_ignore_ascii_case("N")
            && matches!(args, [Ast::Func { name: inner, .. }] if functions::ref_lifting(inner).is_some());
        if feeds_refs {
            self.under_n.set(true);
        }
        // Excel answers `ISREF(XLOOKUP(..))` with the reference the lookup returns; Open365 returns values.
        if name.eq_ignore_ascii_case("ISREF")
            && let [Ast::Func { name: inner, .. }] = args
            && inner.eq_ignore_ascii_case("XLOOKUP")
        {
            let found = self.eval(&args[0]);
            return Arg::Value(Value::Bool(!matches!(found, Arg::Value(Value::Error(_)))));
        }
        // A function with scalar parameters is applied to every element of an array argument. Ranges are
        // expanded only where Excel evaluates array parameters (`array`); elsewhere they take the implicit
        // intersection, but computed arrays and array constants always lift.
        let lifting =
            ref_fn.or_else(|| functions::lifting(name).filter(|l| !l.array_only || array));
        let expand = array && lifting.is_some_and(|l| l.expand_ranges);
        self.fn_depth.set(self.fn_depth.get() + 1);
        let evaluated: Vec<Arg> = args
            .iter()
            .enumerate()
            .map(|(i, a)| {
                if reference_fn && let Some(r) = self.unchecked_ref(a) {
                    return r;
                }
                let lifted = expand && lifting.is_some_and(|l| (l.scalar_param)(i));
                // `N(OFFSET(..))` passes the array context on to the reference function.
                let feeds_array_of_refs = (feeds_refs || functions::lifts_over_refs(name))
                    && matches!(a, Ast::Func { name: inner, .. } if functions::ref_lifting(inner).is_some());
                let lifted = lifted
                    || array
                        && (feeds_array_of_refs || ref_fn.is_some_and(|l| (l.scalar_param)(i)));
                let array_param = lifted || self.cse || functions::is_array_param(name, i);
                let arg = self.eval_in(a, array_param);
                // `IFERROR(A1,0)` is a value for `ROW`, `ISREF` and friends, though `ISBLANK` sees through it.
                match (&arg, a) {
                    (Arg::Range(r), Ast::Func { name: inner, .. })
                        if reference_fn
                            && (inner.eq_ignore_ascii_case("IFERROR")
                                || inner.eq_ignore_ascii_case("IFNA")) =>
                    {
                        Arg::Value(self.ctx.scalar(&Arg::Range(r.clone())))
                    }
                    _ => arg,
                }
            })
            .collect();
        self.fn_depth.set(self.fn_depth.get() - 1);
        let evaluated = self.whole_ranges_for_transpose(name, evaluated);
        let evaluated = self.scalarize_names(name, args, evaluated, lifting, array, expand);
        if let Some(l) = lifting {
            let triggers = evaluated.iter().enumerate().any(|(i, a)| {
                l.trigger.unwrap_or(l.scalar_param)(i)
                    && match a {
                        Arg::Array(rows) => {
                            rows.len() > 1
                                || rows.first().is_some_and(|r| r.len() > 1)
                                || ref_fn.is_some()
                        }
                        Arg::Range(r) => expand && !r.is_single_cell(),
                        _ => false,
                    }
            });
            if triggers {
                if ref_fn.is_some() {
                    return self.lift_refs(func, &evaluated, l);
                }
                return self.lift_call(func, &evaluated, l);
            }
        }
        if evaluated.iter().any(|a| matches!(a, Arg::Lambda(_)))
            && let Some(answer) = Self::lambda_answer(name, &evaluated)
        {
            return answer;
        }
        // `TYPE` of a multi-cell range is 64 where Excel evaluates arrays, 16 (the intersection fails) elsewhere.
        if array
            && name.eq_ignore_ascii_case("TYPE")
            && let [Arg::Range(r)] = evaluated.as_slice()
            && !r.is_single_cell()
        {
            return Arg::Value(Value::Number(64.0));
        }
        let result = if evaluated.iter().any(|a| matches!(a, Arg::Refs(_))) {
            self.refs_call(name, func, &evaluated, array)
        } else {
            call_function(self.ctx, func, &evaluated)
        };
        // `ROW(A1:A3)` is an array only where Excel evaluates arrays; in a plain formula it is its first row.
        if !array
            && functions::returns_array_in_array_context(name)
            && let Arg::Array(rows) = &result
        {
            return Arg::from_value(
                rows.first()
                    .and_then(|r| r.first())
                    .cloned()
                    .unwrap_or(Value::Error(ErrorKind::Value)),
            );
        }
        result
    }

    /// Calls a scalar function once per element of its array arguments (scalars broadcast, a single row or
    /// column stretches, anything shorter than the largest operand is `#N/A`). Once an array makes the call lift,
    /// ranges among the other scalar arguments are read as arrays too (`IF({TRUE,FALSE},A1:A2,B1:B2)`).
    fn lift_call(&self, func: &Func, args: &[Arg], lifting: &Lifting) -> Arg {
        let out = self
            .lift_elements(func, args, lifting)
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|arg| match arg {
                        Arg::Value(v) => v,
                        Arg::Array(rows) => rows
                            .first()
                            .and_then(|row| row.first())
                            .cloned()
                            .unwrap_or(Value::Error(ErrorKind::Value)),
                        other => self.ctx.scalar(&other),
                    })
                    .collect()
            })
            .collect();
        Arg::Array(out)
    }

    /// `OFFSET`/`INDIRECT` with an array argument: one reference (or error) per element.
    fn lift_refs(&self, func: &Func, args: &[Arg], lifting: &Lifting) -> Arg {
        Arg::Refs(
            self.lift_elements(func, args, lifting)
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .map(|arg| match arg {
                            Arg::Range(r) => Ok(r),
                            Arg::Value(Value::Error(e)) => Err(e),
                            _ => Err(ErrorKind::Value),
                        })
                        .collect()
                })
                .collect(),
        )
    }

    /// A function called with an array of references (what `OFFSET`/`INDIRECT` return for array arguments).
    /// Excel's answers were measured per function (group `refarr` of the corpus).
    fn refs_call(&self, name: &str, func: &Func, args: &[Arg], array: bool) -> Arg {
        use functions::RefsMode;
        let shape = args.iter().find_map(|a| match a {
            Arg::Refs(refs) => Some((refs.len(), refs.first().map_or(0, Vec::len))),
            _ => None,
        });
        let (rows, cols) = shape.unwrap_or((1, 1));
        let mode = functions::refs_mode(name);
        // `ROWS(OFFSET(..))` at the top of a formula sees the first element, like the other functions there.
        if self.fn_depth.get() == 0
            && (name.eq_ignore_ascii_case("ROWS") || name.eq_ignore_ascii_case("COLUMNS"))
        {
            let converted: Vec<Arg> = args.iter().map(refs_first).collect();
            return call_function(self.ctx, func, &converted);
        }
        // Functions with value parameters see an array of references only where Excel evaluates arrays;
        // inside `SUM(f(..))` they take the first element.
        if matches!(
            mode,
            RefsMode::ValueLifted | RefsMode::Type | RefsMode::FalseArray
        ) && !array
        {
            let converted: Vec<Arg> = args.iter().map(refs_first).collect();
            return call_function(self.ctx, func, &converted);
        }
        match mode {
            RefsMode::Native => call_function(self.ctx, func, args),
            RefsMode::LiftOver => {
                // The criteria are not dereferenced: an array of references matches nothing.
                let is_criteria = |i: usize| {
                    if name.eq_ignore_ascii_case("COUNTIFS") {
                        i % 2 == 1
                    } else {
                        i == 1
                    }
                };
                if args
                    .iter()
                    .enumerate()
                    .any(|(i, a)| is_criteria(i) && matches!(a, Arg::Refs(_)))
                {
                    if array {
                        return Arg::Value(Value::Number(0.0));
                    }
                    let converted: Vec<Arg> = args
                        .iter()
                        .enumerate()
                        .map(|(i, a)| {
                            if is_criteria(i) {
                                refs_first(a)
                            } else {
                                a.clone()
                            }
                        })
                        .collect();
                    return call_function(self.ctx, func, &converted);
                }
                self.lift_over_refs(func, args)
            }
            RefsMode::Union => {
                let mut converted = Vec::with_capacity(args.len());
                for a in args {
                    match a {
                        Arg::Refs(refs) => {
                            let mut ranges = Vec::new();
                            for e in refs.iter().flatten() {
                                match e {
                                    Ok(r) => ranges.push(r.clone()),
                                    Err(e) => return Arg::error(*e),
                                }
                            }
                            converted.push(Arg::Union(ranges));
                        }
                        other => converted.push(other.clone()),
                    }
                }
                call_function(self.ctx, func, &converted)
            }
            RefsMode::Value | RefsMode::ValueLifted => Arg::error(ErrorKind::Value),
            RefsMode::NA => Arg::error(ErrorKind::NA),
            RefsMode::Type => Arg::Value(Value::Number(64.0)),
            RefsMode::NotRef => Arg::Value(Value::Bool(false)),
            RefsMode::FalseArray => Arg::Array(vec![vec![Value::Bool(false); cols]; rows]),
            RefsMode::First => {
                let converted: Vec<Arg> = args.iter().map(refs_first).collect();
                call_function(self.ctx, func, &converted)
            }
        }
    }

    /// `COUNTIF(INDIRECT(..),..)`: a function whose range parameter is given an array of references runs once
    /// per element. An error element is that element's result.
    fn lift_over_refs(&self, func: &Func, args: &[Arg]) -> Arg {
        let shape = args.iter().find_map(|a| match a {
            Arg::Refs(refs) => Some(refs),
            _ => None,
        });
        let Some(shape) = shape else {
            return call_function(self.ctx, func, args);
        };
        let out = shape
            .iter()
            .enumerate()
            .map(|(r, row)| {
                (0..row.len())
                    .map(|c| {
                        let mut call_args = Vec::with_capacity(args.len());
                        for a in args {
                            match a {
                                Arg::Refs(refs) => match refs.get(r).and_then(|x| x.get(c)) {
                                    Some(Ok(range)) => call_args.push(Arg::Range(range.clone())),
                                    Some(Err(e)) => return Value::Error(*e),
                                    None => return Value::Error(ErrorKind::NA),
                                },
                                other => call_args.push(other.clone()),
                            }
                        }
                        let result = call_function(self.ctx, func, &call_args);
                        self.ctx.scalar(&result)
                    })
                    .collect()
            })
            .collect();
        Arg::Array(out)
    }

    /// The result of the function for every element position of its array arguments.
    fn lift_elements(&self, func: &Func, args: &[Arg], lifting: &Lifting) -> Vec<Vec<Arg>> {
        let operands: Vec<Option<Operand>> = args
            .iter()
            .enumerate()
            .map(|(i, a)| {
                if (lifting.scalar_param)(i) && !matches!(a, Arg::Missing) {
                    Some(self.operand(a.clone(), true))
                } else {
                    None
                }
            })
            .collect();
        let rows = operands
            .iter()
            .flatten()
            .map(|o| match o {
                Operand::Grid(g) => g.len(),
                Operand::Scalar(_) => 1,
            })
            .max()
            .unwrap_or(1);
        let cols = operands
            .iter()
            .flatten()
            .map(|o| match o {
                Operand::Grid(g) => g.first().map_or(0, Vec::len),
                Operand::Scalar(_) => 1,
            })
            .max()
            .unwrap_or(1);
        let pick = |o: &Operand, r: usize, c: usize| -> Value {
            match o {
                Operand::Scalar(v) => v.clone(),
                Operand::Grid(g) => {
                    let r = if g.len() == 1 { 0 } else { r };
                    let c = if g.first().map_or(0, Vec::len) == 1 {
                        0
                    } else {
                        c
                    };
                    g.get(r)
                        .and_then(|row| row.get(c))
                        .cloned()
                        .unwrap_or(Value::Error(ErrorKind::NA))
                }
            }
        };
        (0..rows)
            .map(|r| {
                (0..cols)
                    .map(|c| {
                        let call_args: Vec<Arg> = operands
                            .iter()
                            .zip(args)
                            .map(|(o, original)| match o {
                                Some(o) => Arg::Value(pick(o, r, c)),
                                None => original.clone(),
                            })
                            .collect();
                        call_function(self.ctx, func, &call_args)
                    })
                    .collect()
            })
            .collect()
    }

    fn operand(&self, arg: Arg, array: bool) -> Operand {
        match arg {
            Arg::Missing => Operand::Scalar(Value::Empty),
            Arg::Value(v) => Operand::Scalar(v),
            Arg::Array(rows) => {
                if rows.len() == 1 && rows[0].len() == 1 {
                    Operand::Scalar(rows[0][0].clone())
                } else {
                    Operand::Grid(rows)
                }
            }
            Arg::Range(r) => {
                if r.is_single_cell() {
                    return Operand::Scalar(self.ctx.source.cell(&r.sheet, r.row1, r.col1));
                }
                if !array {
                    return Operand::Scalar(self.ctx.scalar(&Arg::Range(r)));
                }
                let cells = u64::from(r.rows()) * u64::from(r.cols());
                if cells > MAX_ARRAY_CELLS {
                    return Operand::Scalar(Value::Error(ErrorKind::Value));
                }
                let grid = (r.row1..=r.row2)
                    .map(|row| {
                        (r.col1..=r.col2)
                            .map(|col| self.ctx.source.cell(&r.sheet, row, col))
                            .collect()
                    })
                    .collect();
                Operand::Grid(grid)
            }
            Arg::Union(_) | Arg::Lambda(_) => Operand::Scalar(Value::Error(ErrorKind::Value)),
            // Arithmetic and comparison on an array of references are `#VALUE!` (measured).
            Arg::Refs(_) => Operand::Scalar(Value::Error(ErrorKind::Value)),
        }
    }

    fn unary(&self, op: UnaryOp, arg: Arg, array: bool) -> Arg {
        match op {
            UnaryOp::Plus => match arg {
                Arg::Range(r) if !array => Arg::from_value(self.ctx.scalar(&Arg::Range(r))),
                other => other,
            },
            UnaryOp::Neg => {
                let d = self.ctx.date1904;
                lift_unary(self.operand(arg, array), |x| match x.to_number_in(d) {
                    Ok(n) => Value::Number(0.0 - n),
                    Err(e) => Value::Error(e),
                })
            }
        }
    }

    fn eval_binary(&self, op: BinOp, lhs: &Ast, rhs: &Ast, array: bool, snap: bool) -> Arg {
        // The operands of `:`, space and `,` are references, never evaluated element-wise.
        let operand_array = array && !matches!(op, BinOp::Range | BinOp::Intersect | BinOp::Union);
        let mut left = self.eval_in(lhs, operand_array);
        let mut right = self.eval_in(rhs, operand_array);
        if !operand_array && !matches!(op, BinOp::Range | BinOp::Intersect | BinOp::Union) {
            left = self.scalarize(lhs, left);
            right = self.scalarize(rhs, right);
        }
        match op {
            BinOp::Range => range_operator(left, right),
            BinOp::Intersect => intersect_operator(left, right),
            BinOp::Union => union_operator(left, right),
            _ => {
                let nc = NumCtx {
                    date1904: self.ctx.date1904,
                    snap,
                };
                lift_binary(
                    self.operand(left, array),
                    self.operand(right, array),
                    op,
                    nc,
                )
            }
        }
    }
}

/// An array of references reduced to its first element, for functions that do not model reference arrays.
fn refs_first(arg: &Arg) -> Arg {
    match arg {
        Arg::Refs(refs) => match refs.first().and_then(|row| row.first()) {
            Some(Ok(r)) => Arg::Range(r.clone()),
            Some(Err(e)) => Arg::error(*e),
            None => Arg::error(ErrorKind::Value),
        },
        other => other.clone(),
    }
}

fn call_function(ctx: &EvalContext<'_>, func: &Func, args: &[Arg]) -> Arg {
    match func {
        Func::Val(f) => Arg::from_value(f(ctx, args)),
        Func::Res(f) => match f(ctx, args) {
            Ok(v) => Arg::from_value(v),
            Err(e) => Arg::error(e),
        },
        Func::Arg(f) => f(ctx, args),
        // Handled by the evaluator before it gets here.
        Func::Lazy => Arg::error(ErrorKind::Name),
    }
}

fn range_operator(left: Arg, right: Arg) -> Arg {
    match (left, right) {
        (Arg::Range(a), Arg::Range(b)) if a.sheet.eq_ignore_ascii_case(&b.sheet) => {
            Arg::Range(RangeRef::new(
                &a.sheet,
                a.row1.min(b.row1),
                a.col1.min(b.col1),
                a.row2.max(b.row2),
                a.col2.max(b.col2),
            ))
        }
        (Arg::Value(Value::Error(e)), _) | (_, Arg::Value(Value::Error(e))) => Arg::error(e),
        _ => Arg::error(ErrorKind::Value),
    }
}

fn intersect_operator(left: Arg, right: Arg) -> Arg {
    match (left, right) {
        (Arg::Range(a), Arg::Range(b)) => match a.intersection(&b) {
            Some(r) => Arg::Range(r),
            None => Arg::error(ErrorKind::Null),
        },
        (Arg::Value(Value::Error(e)), _) | (_, Arg::Value(Value::Error(e))) => Arg::error(e),
        _ => Arg::error(ErrorKind::Value),
    }
}

fn union_operator(left: Arg, right: Arg) -> Arg {
    fn parts(arg: Arg) -> Result<Vec<RangeRef>, ErrorKind> {
        match arg {
            Arg::Range(r) => Ok(vec![r]),
            Arg::Union(rs) => Ok(rs),
            Arg::Value(Value::Error(e)) => Err(e),
            _ => Err(ErrorKind::Value),
        }
    }
    match (parts(left), parts(right)) {
        (Ok(mut a), Ok(b)) => {
            a.extend(b);
            Arg::Union(a)
        }
        (Err(e), _) | (_, Err(e)) => Arg::error(e),
    }
}

fn lift_unary(operand: Operand, f: impl Fn(&Value) -> Value) -> Arg {
    match operand {
        Operand::Scalar(v) => Arg::from_value(f(&v)),
        Operand::Grid(rows) => Arg::Array(
            rows.iter()
                .map(|row| row.iter().map(&f).collect())
                .collect(),
        ),
    }
}

/// How numbers are coerced and combined.
#[derive(Debug, Clone, Copy, Default)]
pub struct NumCtx {
    /// Convert date text in the 1904 date system.
    pub date1904: bool,
    /// Collapse an add/subtract result that is within binary noise of zero to exactly zero.
    pub snap: bool,
}

fn lift_binary(left: Operand, right: Operand, op: BinOp, nc: NumCtx) -> Arg {
    let binary_scalar = |op: BinOp, a: &Value, b: &Value| binary_scalar_with(op, a, b, nc);
    match (left, right) {
        (Operand::Scalar(a), Operand::Scalar(b)) => Arg::from_value(binary_scalar(op, &a, &b)),
        (Operand::Scalar(a), Operand::Grid(g)) => Arg::Array(
            g.iter()
                .map(|row| row.iter().map(|b| binary_scalar(op, &a, b)).collect())
                .collect(),
        ),
        (Operand::Grid(g), Operand::Scalar(b)) => Arg::Array(
            g.iter()
                .map(|row| row.iter().map(|a| binary_scalar(op, a, &b)).collect())
                .collect(),
        ),
        (Operand::Grid(a), Operand::Grid(b)) => {
            let rows = broadcast_len(a.len(), b.len());
            let cols = broadcast_len(a.first().map_or(0, Vec::len), b.first().map_or(0, Vec::len));
            let pick = |g: &[Vec<Value>], r: usize, c: usize| -> Value {
                let r = if g.len() == 1 { 0 } else { r };
                let width = g.first().map_or(0, Vec::len);
                let c = if width == 1 { 0 } else { c };
                g.get(r)
                    .and_then(|row| row.get(c))
                    .cloned()
                    .unwrap_or(Value::Error(ErrorKind::NA))
            };
            Arg::Array(
                (0..rows)
                    .map(|r| {
                        (0..cols)
                            .map(|c| binary_scalar(op, &pick(&a, r, c), &pick(&b, r, c)))
                            .collect()
                    })
                    .collect(),
            )
        }
    }
}

fn broadcast_len(a: usize, b: usize) -> usize {
    if a == 1 {
        b
    } else if b == 1 {
        a
    } else {
        a.max(b)
    }
}

/// Applies an arithmetic, concatenation or comparison operator to two scalars (1900 date system, no
/// zero snapping). The leftmost error wins.
pub fn binary_scalar(op: BinOp, a: &Value, b: &Value) -> Value {
    binary_scalar_with(op, a, b, NumCtx::default())
}

/// Like [`binary_scalar`] with explicit coercion settings.
pub fn binary_scalar_with(op: BinOp, a: &Value, b: &Value, nc: NumCtx) -> Value {
    let arithmetic_op = matches!(
        op,
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Pow
    );
    if !arithmetic_op {
        if let Value::Error(e) = a {
            return Value::Error(*e);
        }
        if let Value::Error(e) = b {
            return Value::Error(*e);
        }
    }
    match op {
        BinOp::Concat => match (a.to_text(), b.to_text()) {
            (Ok(x), Ok(y)) => Value::Text(x + &y),
            (Err(e), _) | (_, Err(e)) => Value::Error(e),
        },
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            let ord = compare_values(a, b);
            let result = match op {
                BinOp::Eq => ord == Ordering::Equal,
                BinOp::Ne => ord != Ordering::Equal,
                BinOp::Lt => ord == Ordering::Less,
                BinOp::Le => ord != Ordering::Greater,
                BinOp::Gt => ord == Ordering::Greater,
                _ => ord != Ordering::Less,
            };
            Value::Bool(result)
        }
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Pow => {
            // Excel coerces the left operand (or fails on it) before it looks at the right one.
            let x = match a.to_number_in(nc.date1904) {
                Ok(x) => x,
                Err(e) => return Value::Error(e),
            };
            let y = match b.to_number_in(nc.date1904) {
                Ok(y) => y,
                Err(e) => return Value::Error(e),
            };
            arithmetic(op, x, y, nc.snap)
        }
        BinOp::Range | BinOp::Intersect | BinOp::Union => Value::Error(ErrorKind::Value),
    }
}

/// Add/subtract results that are within binary noise of zero collapse to exactly zero, as in
/// Excel (`=0.1+0.2-0.3` gives `0`). Excel does this only for the last operation of a formula and in `SUM`.
pub(crate) fn snap_to_zero(result: f64, x: f64, y: f64) -> f64 {
    if result != 0.0 && result.abs() < x.abs().max(y.abs()) * 4.0 * f64::EPSILON {
        0.0
    } else {
        result
    }
}

fn arithmetic(op: BinOp, x: f64, y: f64, snap: bool) -> Value {
    let zero = |result: f64| {
        if snap {
            snap_to_zero(result, x, y)
        } else {
            result
        }
    };
    let result = match op {
        BinOp::Add => zero(x + y),
        BinOp::Sub => zero(x - y),
        BinOp::Mul => x * y,
        BinOp::Div => {
            if y == 0.0 {
                return Value::Error(ErrorKind::DivZero);
            }
            x / y
        }
        _ => match power(x, y) {
            Ok(v) => v,
            Err(e) => return Value::Error(e),
        },
    };
    if result.is_finite() {
        Value::Number(result)
    } else {
        Value::Error(ErrorKind::Num)
    }
}

/// Excel's `^`: `0^0` is `#NUM!`, `0^negative` is `#DIV/0!`, negative base with a fractional
/// exponent is `#NUM!`.
pub fn power(x: f64, y: f64) -> Result<f64, ErrorKind> {
    if x == 0.0 && y == 0.0 {
        return Err(ErrorKind::Num);
    }
    if x == 0.0 && y < 0.0 {
        return Err(ErrorKind::DivZero);
    }
    let r = if y == y.trunc() && y.abs() < 9.2e18 {
        crate::xmath::pow_int(x, y)
    } else if x < 0.0 {
        // A negative base only has a real root when the exponent is 1/odd.
        let inverse = 1.0 / y;
        if (inverse - inverse.round()).abs() < 1e-9 && inverse.round() % 2.0 != 0.0 {
            -fractional_power(-x, y)
        } else {
            return Err(ErrorKind::Num);
        }
    } else {
        fractional_power(x, y)
    };
    if !r.is_finite() || (x < 0.0 && y.abs() > 2_147_483_648.0) {
        return Err(ErrorKind::Num);
    }
    // Results below the smallest normal number are flushed to zero.
    Ok(if r.abs() < f64::MIN_POSITIVE { 0.0 } else { r })
}

/// `x^y` for positive `x` and a fractional `y`. Excel evaluates this as `exp(y * ln x)` with correctly rounded
/// `ln` and `exp` (square roots aside), which differs from a correctly rounded power in the last digit: `27^(1/3)`
/// is 2.9999999999999996.
fn fractional_power(x: f64, y: f64) -> f64 {
    crate::xmath::pow_frac(x, y)
}
