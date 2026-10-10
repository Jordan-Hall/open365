//! Logical and information functions.

use super::{Res, arity, bool_arg};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value};

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
    // The condition is a scalar parameter: a range takes the implicit intersection.
    match ctx.scalar(&args[0]).to_bool() {
        Err(e) => Arg::error(e),
        Ok(true) => branch(Some(&args[1])),
        Ok(false) => branch(args.get(2)),
    }
}

fn error_filter(ctx: &EvalContext<'_>, args: &[Arg], only_na: bool) -> Arg {
    if arity(args, 2, 2).is_err() {
        return Arg::error(ErrorKind::Value);
    }
    // The tested value is a scalar parameter: a range takes the implicit intersection.
    let v = ctx.scalar(&args[0]);
    let caught = match &v {
        Value::Error(e) => !only_na || *e == ErrorKind::NA,
        _ => false,
    };
    if caught {
        branch(Some(&args[1]))
    } else if matches!(args[0], Arg::Range(_)) && !v.is_error() {
        // A reference passes through, so `ISBLANK(IFERROR(A1,0))` still sees the cell.
        args[0].clone()
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
            Arg::Value(Value::Error(e)) => return Err(*e),
            // Direct text that is not TRUE/FALSE is ignored, like text in a reference.
            Arg::Value(v @ Value::Text(_)) => {
                if let Ok(b) = v.to_bool() {
                    out.push(b);
                }
            }
            Arg::Value(Value::Empty) => {}
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

pub(super) fn true_fn(_ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 0, 0)?;
    Ok(Value::Bool(true))
}

pub(super) fn false_fn(_ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 0, 0)?;
    Ok(Value::Bool(false))
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
