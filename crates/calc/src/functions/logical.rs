//! Logical and information functions.

use super::{Res, arity, bool_arg};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value};

/// Cells of an array-valued argument, if it has more than one cell.
fn multi_cell(ctx: &EvalContext<'_>, arg: &Arg) -> Option<Vec<Vec<Value>>> {
    let grid = ctx.grid(arg)?;
    if matches!(arg, Arg::Value(_)) || (grid.rows() <= 1 && grid.cols() <= 1) {
        return None;
    }
    if grid.rows().saturating_mul(grid.cols()) > 1 << 20 {
        return Some(vec![vec![Value::Error(ErrorKind::Value)]]);
    }
    Some(
        (0..grid.rows())
            .map(|r| (0..grid.cols()).map(|c| grid.get(r, c)).collect())
            .collect(),
    )
}

/// Element of an argument at `(row, col)`, broadcasting scalars and 1-wide dimensions.
fn element(ctx: &EvalContext<'_>, arg: &Arg, row: usize, col: usize) -> Value {
    match ctx.grid(arg) {
        None => Value::Empty,
        Some(g) if g.rows() <= 1 && g.cols() <= 1 => ctx.scalar(arg),
        Some(g) => {
            let r = if g.rows() == 1 { 0 } else { row };
            let c = if g.cols() == 1 { 0 } else { col };
            if r >= g.rows() || c >= g.cols() {
                Value::Error(ErrorKind::NA)
            } else {
                g.get(r, c)
            }
        }
    }
}

fn branch(arg: Option<&Arg>) -> Arg {
    match arg {
        None => Arg::Value(Value::Bool(false)),
        Some(Arg::Missing) => Arg::Value(Value::Number(0.0)),
        Some(a) => a.clone(),
    }
}

pub(super) fn if_(ctx: &EvalContext<'_>, args: &[Arg]) -> Arg {
    if arity(args, 2, 3).is_err() {
        return Arg::error(ErrorKind::Value);
    }
    if let Some(cond) = multi_cell(ctx, &args[0]) {
        let out = cond
            .iter()
            .enumerate()
            .map(|(r, row)| {
                row.iter()
                    .enumerate()
                    .map(|(c, v)| match v.to_bool() {
                        Err(e) => Value::Error(e),
                        Ok(true) => element(ctx, &args[1], r, c),
                        Ok(false) => match args.get(2) {
                            Some(a) => element(ctx, a, r, c),
                            None => Value::Bool(false),
                        },
                    })
                    .collect()
            })
            .collect();
        return Arg::Array(out);
    }
    let cond = ctx.scalar(&args[0]);
    match cond.to_bool() {
        Err(e) => Arg::error(e),
        Ok(true) => branch(Some(&args[1])),
        Ok(false) => branch(args.get(2)),
    }
}

fn error_filter(ctx: &EvalContext<'_>, args: &[Arg], only_na: bool) -> Arg {
    if arity(args, 2, 2).is_err() {
        return Arg::error(ErrorKind::Value);
    }
    let matches = |v: &Value| match v {
        Value::Error(e) => !only_na || *e == ErrorKind::NA,
        _ => false,
    };
    if let Some(values) = multi_cell(ctx, &args[0]) {
        let out = values
            .iter()
            .enumerate()
            .map(|(r, row)| {
                row.iter()
                    .enumerate()
                    .map(|(c, v)| {
                        if matches(v) {
                            element(ctx, &args[1], r, c)
                        } else {
                            v.clone()
                        }
                    })
                    .collect()
            })
            .collect();
        return Arg::Array(out);
    }
    let v = ctx.scalar(&args[0]);
    if matches(&v) {
        branch(Some(&args[1]))
    } else {
        Arg::from_value(v)
    }
}

pub(super) fn iferror(ctx: &EvalContext<'_>, args: &[Arg]) -> Arg {
    error_filter(ctx, args, false)
}

pub(super) fn ifna(ctx: &EvalContext<'_>, args: &[Arg]) -> Arg {
    error_filter(ctx, args, true)
}

/// Booleans seen by `AND`/`OR`/`XOR`: text and blanks inside references are ignored, direct
/// text must be `TRUE`/`FALSE`.
fn logicals(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Vec<bool>> {
    let mut out = Vec::new();
    for arg in args {
        match arg {
            Arg::Missing => out.push(false),
            Arg::Value(v) => out.push(v.to_bool()?),
            _ => {
                for v in ctx.cells(arg) {
                    match v {
                        Value::Bool(b) => out.push(b),
                        Value::Number(n) => out.push(n != 0.0),
                        Value::Error(e) => return Err(e),
                        _ => {}
                    }
                }
            }
        }
    }
    if out.is_empty() {
        Err(ErrorKind::Value)
    } else {
        Ok(out)
    }
}

pub(super) fn and(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    Ok(Value::Bool(logicals(ctx, args)?.iter().all(|b| *b)))
}

pub(super) fn or(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    Ok(Value::Bool(logicals(ctx, args)?.iter().any(|b| *b)))
}

pub(super) fn xor(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let trues = logicals(ctx, args)?.iter().filter(|b| **b).count();
    Ok(Value::Bool(trues % 2 == 1))
}

pub(super) fn not(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    Ok(Value::Bool(!bool_arg(ctx, &args[0])?))
}

fn test(ctx: &EvalContext<'_>, args: &[Arg], predicate: impl Fn(&Value) -> bool) -> Res<Value> {
    arity(args, 1, 1)?;
    Ok(Value::Bool(predicate(&ctx.scalar(&args[0]))))
}

pub(super) fn isblank(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    test(ctx, args, |v| matches!(v, Value::Empty))
}

pub(super) fn isnumber(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    test(ctx, args, |v| matches!(v, Value::Number(_)))
}

pub(super) fn istext(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    test(ctx, args, |v| matches!(v, Value::Text(_)))
}

pub(super) fn iserror(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    test(ctx, args, Value::is_error)
}

pub(super) fn isna(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    test(ctx, args, |v| matches!(v, Value::Error(ErrorKind::NA)))
}
