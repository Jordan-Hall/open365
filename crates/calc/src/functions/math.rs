//! Aggregation and arithmetic functions.
//!
//! Excel treats direct arguments and range arguments differently: a direct text argument is
//! coerced (`SUM("5")` is 5, `SUM("x")` is `#VALUE!`), while text and booleans inside a
//! reference or array are ignored. Errors always propagate.

use super::{Res, arity, int_arg, num_arg, number};
use crate::eval::{Arg, EvalContext, power as eval_power};
use crate::value::{ErrorKind, Value, parse_number_text};

/// Collects the numbers an aggregate function sees.
fn collect(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Vec<f64>> {
    let mut out = Vec::new();
    for arg in args {
        match arg {
            Arg::Missing => out.push(0.0),
            Arg::Value(v) => out.push(match v {
                Value::Number(n) => *n,
                Value::Bool(b) => f64::from(u8::from(*b)),
                Value::Empty => 0.0,
                Value::Text(t) => parse_number_text(t).ok_or(ErrorKind::Value)?,
                Value::Error(e) => return Err(*e),
                Value::Array(_) => return Err(ErrorKind::Value),
            }),
            _ => {
                for v in ctx.cells(arg) {
                    match v {
                        Value::Number(n) => out.push(n),
                        Value::Error(e) => return Err(e),
                        _ => {}
                    }
                }
            }
        }
    }
    Ok(out)
}

pub(super) fn sum(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let total: f64 = collect(ctx, args)?.iter().sum();
    number(total)
}

pub(super) fn average(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let nums = collect(ctx, args)?;
    if nums.is_empty() {
        return Err(ErrorKind::DivZero);
    }
    number(nums.iter().sum::<f64>() / nums.len() as f64)
}

pub(super) fn min(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let nums = collect(ctx, args)?;
    Ok(Value::Number(
        nums.into_iter().reduce(f64::min).unwrap_or(0.0),
    ))
}

pub(super) fn max(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let nums = collect(ctx, args)?;
    Ok(Value::Number(
        nums.into_iter().reduce(f64::max).unwrap_or(0.0),
    ))
}

pub(super) fn product(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let nums = collect(ctx, args)?;
    if nums.is_empty() {
        return Ok(Value::Number(0.0));
    }
    number(nums.iter().product())
}

pub(super) fn count(ctx: &EvalContext<'_>, args: &[Arg]) -> Value {
    let mut n = 0u32;
    for arg in args {
        match arg {
            Arg::Missing => n += 1,
            Arg::Value(v) => {
                let counted = match v {
                    Value::Number(_) | Value::Bool(_) => true,
                    Value::Text(t) => parse_number_text(t).is_some(),
                    _ => false,
                };
                n += u32::from(counted);
            }
            _ => {
                n += ctx
                    .cells(arg)
                    .filter(|v| matches!(v, Value::Number(_)))
                    .count() as u32;
            }
        }
    }
    Value::Number(f64::from(n))
}

pub(super) fn counta(ctx: &EvalContext<'_>, args: &[Arg]) -> Value {
    let mut n = 0u32;
    for arg in args {
        match arg {
            Arg::Missing => n += 1,
            Arg::Value(v) => n += u32::from(*v != Value::Empty),
            _ => n += ctx.cells(arg).filter(|v| *v != Value::Empty).count() as u32,
        }
    }
    Value::Number(f64::from(n))
}

pub(super) fn countblank(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    let blank = match &args[0] {
        Arg::Range(r) => {
            let total = u64::from(r.rows()) * u64::from(r.cols());
            let (mut non_empty, mut empty_text) = (0u64, 0u64);
            for v in ctx.cells(&args[0]) {
                match v {
                    Value::Empty => {}
                    Value::Text(t) if t.is_empty() => {
                        non_empty += 1;
                        empty_text += 1;
                    }
                    _ => non_empty += 1,
                }
            }
            total - non_empty + empty_text
        }
        Arg::Array(_) | Arg::Union(_) => ctx
            .cells(&args[0])
            .filter(|v| matches!(v, Value::Empty) || matches!(v, Value::Text(t) if t.is_empty()))
            .count() as u64,
        _ => return Err(ErrorKind::Value),
    };
    Ok(Value::Number(blank as f64))
}

/// Rounding direction on the 15-significant-digit decimal form of a number, which is how
/// Excel avoids binary artefacts such as `ROUND(2.675, 2) = 2.67`.
#[derive(Clone, Copy)]
pub(super) enum RoundMode {
    /// Half away from zero.
    Nearest,
    /// Away from zero.
    Up,
    /// Toward zero.
    Down,
}

pub(crate) fn round_decimal(x: f64, places: i64, mode: RoundMode) -> f64 {
    if x == 0.0 || !x.is_finite() || places > 300 {
        return x;
    }
    if places < -300 {
        return 0.0;
    }
    let sci = format!("{:.14e}", x.abs());
    let Some((mantissa, exp)) = sci.split_once('e') else {
        return x;
    };
    let exp10: i64 = exp.parse().unwrap_or(0);
    let digits: Vec<u8> = mantissa
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();
    let keep = exp10 + 1 + places;
    if keep >= digits.len() as i64 {
        return x;
    }
    let (kept, first_discarded, any_discarded) = if keep >= 0 {
        let k = keep as usize;
        (&digits[..k], digits[k], digits[k..].iter().any(|&d| d != 0))
    } else {
        (&digits[..0], 0, true)
    };
    let mut kept_int = kept.iter().fold(0u64, |acc, &d| acc * 10 + u64::from(d));
    let increment = match mode {
        RoundMode::Nearest => first_discarded >= 5,
        RoundMode::Up => any_discarded,
        RoundMode::Down => false,
    };
    if increment {
        kept_int += 1;
    }
    let scale = 10f64.powi(places.unsigned_abs().min(300) as i32);
    let magnitude = if places >= 0 {
        kept_int as f64 / scale
    } else {
        kept_int as f64 * scale
    };
    if magnitude == 0.0 {
        0.0
    } else if x < 0.0 {
        -magnitude
    } else {
        magnitude
    }
}

fn rounding(ctx: &EvalContext<'_>, args: &[Arg], mode: RoundMode) -> Res<Value> {
    arity(args, 2, 2)?;
    let x = num_arg(ctx, &args[0])?;
    let places = int_arg(ctx, &args[1])?;
    number(round_decimal(x, places, mode))
}

pub(super) fn round(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    rounding(ctx, args, RoundMode::Nearest)
}

pub(super) fn roundup(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    rounding(ctx, args, RoundMode::Up)
}

pub(super) fn rounddown(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    rounding(ctx, args, RoundMode::Down)
}

pub(super) fn int(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    number(num_arg(ctx, &args[0])?.floor())
}

pub(super) fn modulo(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let n = num_arg(ctx, &args[0])?;
    let d = num_arg(ctx, &args[1])?;
    if d == 0.0 {
        return Err(ErrorKind::DivZero);
    }
    let mut r = n % d;
    if r != 0.0 && (r < 0.0) != (d < 0.0) {
        r += d;
    }
    number(r)
}

pub(super) fn abs(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    number(num_arg(ctx, &args[0])?.abs())
}

pub(super) fn sqrt(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    let x = num_arg(ctx, &args[0])?;
    if x < 0.0 {
        return Err(ErrorKind::Num);
    }
    number(x.sqrt())
}

pub(super) fn power(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let x = num_arg(ctx, &args[0])?;
    let y = num_arg(ctx, &args[1])?;
    number(eval_power(x, y)?)
}

pub(super) fn pi(_ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 0, 0)?;
    Ok(Value::Number(std::f64::consts::PI))
}
