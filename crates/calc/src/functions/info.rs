//! Information functions that look at what an argument is: `ISEVEN`, `ISODD`, `ISREF`, `ISFORMULA`,
//! `TYPE` and `ERROR.TYPE`.

use super::{Res, arity, scalar};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value};

/// The parity test behind `ISEVEN` and `ISODD`: the argument is converted like arithmetic does (a boolean
/// or empty text is `#VALUE!`) and truncated toward zero. A multi-cell range is `#VALUE!`, not intersected.
fn parity(ctx: &EvalContext<'_>, args: &[Arg], even: bool) -> Res<Value> {
    arity(args, 1, 1)?;
    if matches!(&args[0], Arg::Range(r) if !r.is_single_cell()) {
        return Err(ErrorKind::Value);
    }
    let n = match scalar(ctx, &args[0])? {
        Value::Bool(_) => return Err(ErrorKind::Value),
        Value::Text(t) if t.is_empty() => return Err(ErrorKind::Value),
        v => v.to_number_in(ctx.date1904)?,
    };
    let is_even = (n.trunc() % 2.0) == 0.0;
    Ok(Value::Bool(is_even == even))
}

pub(super) fn iseven(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    parity(ctx, args, true)
}

pub(super) fn isodd(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    parity(ctx, args, false)
}

/// `ISREF`: true for a reference, including a union and an intersection that is not empty.
pub(super) fn isref(_ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    Ok(Value::Bool(matches!(
        args[0],
        Arg::Range(_) | Arg::Union(_)
    )))
}

/// `ISFORMULA`: whether the first cell of the reference holds a formula. A value that is not a reference
/// is `#VALUE!`, a reference to a missing sheet `#N/A`.
pub(super) fn isformula(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    let range = match &args[0] {
        Arg::Range(r) => r,
        Arg::Union(rs) => rs.first().ok_or(ErrorKind::Value)?,
        Arg::Value(Value::Error(e)) => return Err(*e),
        _ => return Err(ErrorKind::Value),
    };
    if !ctx.source.sheet_exists(&range.sheet) {
        return Err(ErrorKind::NA);
    }
    Ok(Value::Bool(ctx.source.has_formula(
        &range.sheet,
        range.row1,
        range.col1,
    )))
}

/// `TYPE`: 1 number, 2 text, 4 logical, 16 error, 64 array. A reference is intersected like a scalar
/// parameter, so a multi-cell range outside the formula's row or column is the error `#VALUE!` (16).
pub(super) fn type_(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    if matches!(args[0], Arg::Array(_)) {
        return Ok(Value::Number(64.0));
    }
    let code = match ctx.scalar(&args[0]) {
        Value::Empty | Value::Number(_) => 1.0,
        Value::Text(_) => 2.0,
        Value::Bool(_) => 4.0,
        Value::Error(_) => 16.0,
        Value::Array(_) => 64.0,
    };
    Ok(Value::Number(code))
}

/// `ERROR.TYPE`: 1 to 7 for `#NULL!` to `#N/A`, `#N/A` for anything that is not an error.
pub(super) fn error_type(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    match ctx.scalar(&args[0]) {
        Value::Error(e) => {
            let code = ErrorKind::ALL
                .iter()
                .position(|k| *k == e)
                .map_or(7, |i| i + 1);
            Ok(Value::Number(code as f64))
        }
        _ => Err(ErrorKind::NA),
    }
}
