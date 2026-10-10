//! Logical and information functions.

use super::{Res, arity, bool_arg};
use crate::ast::BinOp;
use crate::eval::{Arg, EvalContext, binary_scalar};
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

pub(super) fn na(_ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 0, 0)?;
    Err(ErrorKind::NA)
}

pub(super) fn islogical(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    test(ctx, args, |v| matches!(v, Value::Bool(_)))
}

pub(super) fn isnontext(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    test(ctx, args, |v| !matches!(v, Value::Text(_)))
}

/// `ISERR`: any error except `#N/A`.
pub(super) fn iserr(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    test(
        ctx,
        args,
        |v| matches!(v, Value::Error(e) if *e != ErrorKind::NA),
    )
}

/// `IFS(test1, value1, ...)`: the value after the first true test, `#N/A` when none is. A test that is not
/// a boolean is `#VALUE!`; an error test is that error. The chosen value is returned as it is (references
/// stay references).
pub(super) fn ifs(ctx: &EvalContext<'_>, args: &[Arg]) -> Arg {
    if args.len() < 2 || !args.len().is_multiple_of(2) {
        return Arg::error(ErrorKind::Value);
    }
    for pair in args.chunks(2) {
        match ctx.scalar(&pair[0]).to_bool() {
            Err(e) => return Arg::error(e),
            Ok(true) => return branch(Some(&pair[1])),
            Ok(false) => {}
        }
    }
    Arg::error(ErrorKind::NA)
}

/// `SWITCH(expression, value1, result1, ..., [default])`. Values compare as `=` does (text without case,
/// 15 digits); a blank is the number 0. An error expression is returned at once; an error in a value that
/// is reached is returned too, while later pairs are never looked at.
pub(super) fn switch(ctx: &EvalContext<'_>, args: &[Arg]) -> Arg {
    if args.len() < 3 {
        return Arg::error(ErrorKind::Value);
    }
    let blank_as_zero = |v: Value| match v {
        Value::Empty => Value::Number(0.0),
        v => v,
    };
    let expression = blank_as_zero(ctx.scalar(&args[0]));
    if let Value::Error(e) = expression {
        return Arg::error(e);
    }
    let pairs = (args.len() - 1) / 2;
    for i in 0..pairs {
        let candidate = blank_as_zero(ctx.scalar(&args[1 + 2 * i]));
        match binary_scalar(BinOp::Eq, &expression, &candidate) {
            Value::Bool(true) => return branch(Some(&args[2 + 2 * i])),
            Value::Error(e) => return Arg::error(e),
            _ => {}
        }
    }
    if (args.len() - 1) % 2 == 1 {
        branch(args.last())
    } else {
        Arg::error(ErrorKind::NA)
    }
}
