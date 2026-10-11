//! Database functions (`DSUM`, `DAVERAGE`, ...): records are the rows below the header of the database range,
//! criteria rows are alternatives (OR) and criteria columns must all hold (AND).
//!
//! Measured: plain text criteria match the start of the cell; a text header that names no field matches
//! nothing; criteria cells holding formulas (evaluated per record by Excel) are not supported.

use super::criteria::Criteria;
use super::math::total;
use super::{Res, arity, number};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value, parse_number_text};

type Rows = Vec<Vec<Value>>;

fn grid_of(ctx: &EvalContext<'_>, arg: &Arg) -> Res<Rows> {
    match arg {
        Arg::Range(r) => {
            let (nr, nc) = (ctx.scan_rows(r), ctx.scan_cols(r));
            Ok((0..nr)
                .map(|dr| {
                    (0..nc)
                        .map(|dc| ctx.source.cell(&r.sheet, r.row1 + dr, r.col1 + dc))
                        .collect()
                })
                .collect())
        }
        Arg::Value(Value::Error(e)) => Err(*e),
        _ => Err(ErrorKind::Value),
    }
}

fn header_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Text(x), Value::Text(y)) => x.to_lowercase() == y.to_lowercase(),
        (Value::Number(x), Value::Number(y)) => x == y,
        _ => false,
    }
}

/// A criterion cell as a matcher. Plain text matches the start of the cell (`App` finds `Apple`);
/// an explicit operator, `=` or a number is exact.
fn matcher(cell: &Value) -> Option<Criteria> {
    match cell {
        Value::Empty => None,
        Value::Text(t) if t.is_empty() => None,
        Value::Text(t) => {
            let exact = t.starts_with(['=', '<', '>']) || parse_number_text(t).is_some();
            if exact {
                Some(Criteria::parse(t))
            } else {
                Some(Criteria::parse(&format!("{t}*")))
            }
        }
        v => Some(Criteria::from_value(v)),
    }
}

fn matching_rows(db: &Rows, crit: &Rows) -> Vec<usize> {
    let headers = &db[0];
    let crit_headers = &crit[0];
    let columns: Vec<Option<usize>> = crit_headers
        .iter()
        .map(|h| headers.iter().position(|d| header_eq(d, h)))
        .collect();
    let mut out = Vec::new();
    'record: for (i, record) in db.iter().enumerate().skip(1) {
        for row in &crit[1..] {
            let mut ok = true;
            for (j, cell) in row.iter().enumerate() {
                let Some(m) = matcher(cell) else { continue };
                match columns.get(j).copied().flatten() {
                    Some(col) => {
                        if !m.matches(&record[col]) {
                            ok = false;
                            break;
                        }
                    }
                    // A text header that names no field matches nothing; a numeric header is ignored.
                    None => {
                        if matches!(crit_headers[j], Value::Text(_)) {
                            ok = false;
                            break;
                        }
                    }
                }
            }
            if ok {
                out.push(i);
                continue 'record;
            }
        }
    }
    out
}

/// Sample variance `(sum x^2 - (sum x)^2 / n) / (n - 1)`, population variance `(n sum x^2 - (sum x)^2) / n^2`:
/// what the database functions show in the last digits (a few sample cases with cancellation differ).
fn spread(nums: &[f64], sample: bool, root: bool) -> Res<Value> {
    let n = nums.len() as f64;
    if nums.len() < usize::from(sample) + 1 {
        return Err(ErrorKind::DivZero);
    }
    let (mut s, mut s2) = (0.0, 0.0);
    for x in nums {
        s += x;
        s2 += x * x;
    }
    let v = if sample {
        (s2 - s * s / n) / (n - 1.0)
    } else {
        (n * s2 - s * s) / (n * n)
    }
    .max(0.0);
    number(if root { v.sqrt() } else { v })
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Sum,
    Average,
    Count,
    CountA,
    Get,
    Max,
    Min,
    Product,
    StdevS,
    StdevP,
    VarS,
    VarP,
}

fn run(ctx: &EvalContext<'_>, args: &[Arg], kind: Kind) -> Res<Value> {
    arity(args, 3, 3)?;
    let db = grid_of(ctx, &args[0])?;
    let crit = grid_of(ctx, &args[2])?;
    if db.len() < 2 || crit.len() < 2 {
        return Err(ErrorKind::Value);
    }
    let width = db[0].len();
    let optional_field = matches!(kind, Kind::Count | Kind::CountA);
    let field: Option<usize> = match &args[1] {
        Arg::Missing if optional_field => None,
        a => match ctx.scalar(a) {
            Value::Error(e) => return Err(e),
            Value::Empty if optional_field => None,
            v @ (Value::Number(_) | Value::Bool(true)) => {
                let k = match v {
                    Value::Number(n) => n.trunc(),
                    _ => 1.0,
                };
                if k < 1.0 || k > width as f64 {
                    return Err(ErrorKind::Value);
                }
                Some(k as usize - 1)
            }
            Value::Text(t) => Some(
                db[0]
                    .iter()
                    .position(
                        |h| matches!(h, Value::Text(x) if x.to_lowercase() == t.to_lowercase()),
                    )
                    .ok_or(ErrorKind::Value)?,
            ),
            _ => return Err(ErrorKind::Value),
        },
    };
    let rows = matching_rows(&db, &crit);
    let Some(col) = field else {
        return Ok(Value::Number(rows.len() as f64));
    };
    let cells: Vec<&Value> = rows.iter().map(|&i| &db[i][col]).collect();
    if kind == Kind::Get {
        // Records whose field is blank do not count.
        let cells: Vec<&Value> = cells
            .into_iter()
            .filter(|v| !matches!(v, Value::Empty))
            .collect();
        return match cells.len() {
            0 => Err(ErrorKind::Value),
            1 => Ok(cells[0].clone()),
            _ => Err(ErrorKind::Num),
        };
    }
    if kind == Kind::CountA {
        return Ok(Value::Number(
            cells.iter().filter(|v| !matches!(v, Value::Empty)).count() as f64,
        ));
    }
    if kind != Kind::Count
        && let Some(Value::Error(e)) = cells.iter().find(|v| matches!(v, Value::Error(_)))
    {
        return Err(*e);
    }
    let nums: Vec<f64> = cells
        .iter()
        .filter_map(|v| match v {
            Value::Number(n) => Some(*n),
            _ => None,
        })
        .collect();
    match kind {
        Kind::Count => Ok(Value::Number(nums.len() as f64)),
        Kind::Sum => number(total(&nums)),
        Kind::Average => {
            if nums.is_empty() {
                Err(ErrorKind::DivZero)
            } else {
                number(total(&nums) / nums.len() as f64)
            }
        }
        Kind::Max => Ok(Value::Number(
            nums.into_iter().reduce(f64::max).unwrap_or(0.0),
        )),
        Kind::Min => Ok(Value::Number(
            nums.into_iter().reduce(f64::min).unwrap_or(0.0),
        )),
        Kind::Product => {
            if nums.is_empty() {
                Ok(Value::Number(0.0))
            } else {
                number(nums.iter().product())
            }
        }
        Kind::StdevS => spread(&nums, true, true),
        Kind::StdevP => spread(&nums, false, true),
        Kind::VarS => spread(&nums, true, false),
        Kind::VarP => spread(&nums, false, false),
        Kind::Get | Kind::CountA => Err(ErrorKind::Value),
    }
}

macro_rules! dfn {
    ($name:ident, $kind:expr) => {
        pub(super) fn $name(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
            run(ctx, args, $kind)
        }
    };
}
dfn!(dsum, Kind::Sum);
dfn!(daverage, Kind::Average);
dfn!(dcount, Kind::Count);
dfn!(dcounta, Kind::CountA);
dfn!(dget, Kind::Get);
dfn!(dmax, Kind::Max);
dfn!(dmin, Kind::Min);
dfn!(dproduct, Kind::Product);
dfn!(dstdev, Kind::StdevS);
dfn!(dstdevp, Kind::StdevP);
dfn!(dvar, Kind::VarS);
dfn!(dvarp, Kind::VarP);
