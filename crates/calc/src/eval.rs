//! Formula evaluator: turns an [`Ast`] into a [`Value`] against a [`CellSource`].
//!
//! References are kept lazy ([`Arg::Range`]) until a function or operator needs their cells,
//! so whole-column references never allocate per cell. Binary operators lift over arrays
//! element-wise; scalar function parameters take the implicit intersection of a range with the
//! formula's cell, or `#VALUE!` when there is none.

use std::cell::Cell;
use std::cmp::Ordering;
use std::fmt;

use crate::ast::{AreaKind, AreaRef, Ast, BinOp, CellRef, MAX_COL, MAX_ROW, UnaryOp};
use crate::functions::{self, Func};
use crate::value::{ErrorKind, Value, compare_values};

const MAX_NAME_DEPTH: u32 = 16;
/// Largest range that arithmetic will materialise as an array.
const MAX_ARRAY_CELLS: u64 = 1 << 20;

/// Read access to a workbook. Rows and columns are 1-based.
pub trait CellSource {
    /// The value of a cell; [`Value::Empty`] for blank cells and unknown sheets.
    fn cell(&self, sheet: &str, row: u32, col: u32) -> Value;

    /// The formula of a defined name, if the name exists.
    fn name(&self, name: &str) -> Option<Ast>;

    /// Whether a sheet exists. References to missing sheets evaluate to `#REF!`.
    fn sheet_exists(&self, _sheet: &str) -> bool {
        true
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
        }
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
            Arg::Array(rows) => {
                if rows.len() == 1 && rows[0].len() == 1 {
                    rows[0][0].clone()
                } else {
                    Value::Error(ErrorKind::Value)
                }
            }
            Arg::Range(r) => self.implicit_intersection(r),
            Arg::Union(_) => Value::Error(ErrorKind::Value),
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
    let ev = Evaluator {
        ctx,
        name_depth: Cell::new(0),
    };
    let arg = ev.eval(ast);
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
    }
}

struct Evaluator<'c, 'a> {
    ctx: &'c EvalContext<'a>,
    name_depth: Cell<u32>,
}

/// Operand of an arithmetic operator after reference resolution.
enum Operand {
    Scalar(Value),
    Grid(Vec<Vec<Value>>),
}

impl Evaluator<'_, '_> {
    fn eval(&self, ast: &Ast) -> Arg {
        match ast {
            Ast::Number(n) => Arg::Value(Value::Number(*n)),
            Ast::Text(t) => Arg::Value(Value::Text(t.clone())),
            Ast::Bool(b) => Arg::Value(Value::Bool(*b)),
            Ast::Error(e) => Arg::error(*e),
            Ast::Missing => Arg::Missing,
            Ast::Ref(r) => self.eval_ref(r),
            Ast::Area(a) => self.eval_area(a),
            Ast::Name(n) => self.eval_name(n),
            Ast::Func { name, args, .. } => self.eval_call(name, args),
            Ast::Unary { op, operand } => {
                let v = self.eval(operand);
                self.unary(*op, v)
            }
            Ast::Percent(inner) => {
                let v = self.eval(inner);
                lift_unary(self.operand(v), |x| match x.to_number() {
                    Ok(n) => Value::Number(n / 100.0),
                    Err(e) => Value::Error(e),
                })
            }
            Ast::Binary { op, lhs, rhs } => self.eval_binary(*op, lhs, rhs),
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
        let sheet = self.sheet_of(r.sheet.as_ref());
        if !self.ctx.source.sheet_exists(sheet) {
            return Arg::error(ErrorKind::Ref);
        }
        Arg::Range(RangeRef::new(sheet, r.row, r.col, r.row, r.col))
    }

    fn eval_area(&self, a: &AreaRef) -> Arg {
        let sheet = self.sheet_of(a.sheet.as_ref());
        if !self.ctx.source.sheet_exists(sheet) {
            return Arg::error(ErrorKind::Ref);
        }
        let (row1, row2, col1, col2) = match a.kind {
            AreaKind::Cells => (a.row1, a.row2, a.col1, a.col2),
            AreaKind::Cols => (1, MAX_ROW, a.col1, a.col2),
            AreaKind::Rows => (a.row1, a.row2, 1, MAX_COL),
        };
        Arg::Range(RangeRef::new(sheet, row1, col1, row2, col2))
    }

    fn eval_name(&self, name: &str) -> Arg {
        let Some(ast) = self.ctx.source.name(name) else {
            return Arg::error(ErrorKind::Name);
        };
        let depth = self.name_depth.get();
        if depth >= MAX_NAME_DEPTH {
            return Arg::error(ErrorKind::Name);
        }
        self.name_depth.set(depth + 1);
        let result = self.eval(&ast);
        self.name_depth.set(depth);
        result
    }

    fn eval_call(&self, name: &str, args: &[Ast]) -> Arg {
        let Some(func) = functions::lookup(name) else {
            return Arg::error(ErrorKind::Name);
        };
        let evaluated: Vec<Arg> = args.iter().map(|a| self.eval(a)).collect();
        match func {
            Func::Val(f) => Arg::from_value(f(self.ctx, &evaluated)),
            Func::Res(f) => match f(self.ctx, &evaluated) {
                Ok(v) => Arg::from_value(v),
                Err(e) => Arg::error(e),
            },
            Func::Arg(f) => f(self.ctx, &evaluated),
        }
    }

    fn operand(&self, arg: Arg) -> Operand {
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
            Arg::Union(_) => Operand::Scalar(Value::Error(ErrorKind::Value)),
        }
    }

    fn unary(&self, op: UnaryOp, arg: Arg) -> Arg {
        match op {
            UnaryOp::Plus => match arg {
                Arg::Range(r) => Arg::from_value(self.ctx.scalar(&Arg::Range(r))),
                other => other,
            },
            UnaryOp::Neg => lift_unary(self.operand(arg), |x| match x.to_number() {
                Ok(n) => Value::Number(0.0 - n),
                Err(e) => Value::Error(e),
            }),
        }
    }

    fn eval_binary(&self, op: BinOp, lhs: &Ast, rhs: &Ast) -> Arg {
        let left = self.eval(lhs);
        let right = self.eval(rhs);
        match op {
            BinOp::Range => range_operator(left, right),
            BinOp::Intersect => intersect_operator(left, right),
            BinOp::Union => union_operator(left, right),
            _ => lift_binary(self.operand(left), self.operand(right), op),
        }
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

fn lift_binary(left: Operand, right: Operand, op: BinOp) -> Arg {
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

/// Applies an arithmetic, concatenation or comparison operator to two scalars. The leftmost
/// error wins.
pub fn binary_scalar(op: BinOp, a: &Value, b: &Value) -> Value {
    if let Value::Error(e) = a {
        return Value::Error(*e);
    }
    if let Value::Error(e) = b {
        return Value::Error(*e);
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
            let (x, y) = match (a.to_number(), b.to_number()) {
                (Ok(x), Ok(y)) => (x, y),
                (Err(e), _) | (_, Err(e)) => return Value::Error(e),
            };
            arithmetic(op, x, y)
        }
        BinOp::Range | BinOp::Intersect | BinOp::Union => Value::Error(ErrorKind::Value),
    }
}

/// Add/subtract results that are within binary noise of zero collapse to exactly zero, as in
/// Excel (`0.1+0.2-0.3` shows `0`).
fn snap_to_zero(result: f64, x: f64, y: f64) -> f64 {
    if result != 0.0 && result.abs() < x.abs().max(y.abs()) * 4.0 * f64::EPSILON {
        0.0
    } else {
        result
    }
}

fn arithmetic(op: BinOp, x: f64, y: f64) -> Value {
    let result = match op {
        BinOp::Add => snap_to_zero(x + y, x, y),
        BinOp::Sub => snap_to_zero(x - y, x, y),
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
    let r = x.powf(y);
    if r.is_finite() {
        Ok(r)
    } else {
        Err(ErrorKind::Num)
    }
}
