//! Aggregation and arithmetic functions.
//!
//! Excel treats direct arguments and range arguments differently: a direct text argument is
//! coerced (`SUM("5")` is 5, `SUM("x")` is `#VALUE!`), while text and booleans inside a
//! reference or array are ignored. Errors always propagate.

use super::{Res, arity, int_arg, num_arg, number};
use crate::eval::{Arg, EvalContext, power as eval_power, snap_to_zero};
use crate::value::{ErrorKind, Value, parse_number_text_in};

/// Collects the numbers an aggregate function sees.
fn collect(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Vec<f64>> {
    collect_with(ctx, args, true)
}

/// `missing_is_zero` is false for `PRODUCT`, which skips omitted arguments.
fn collect_with(ctx: &EvalContext<'_>, args: &[Arg], missing_is_zero: bool) -> Res<Vec<f64>> {
    // Excel refuses a call without arguments, such as `=SUM()`.
    arity(args, 1, usize::MAX)?;
    let mut out = Vec::new();
    for arg in args {
        match arg {
            Arg::Missing => {
                if missing_is_zero {
                    out.push(0.0);
                }
            }
            Arg::Value(v) => out.push(match v {
                Value::Number(n) => *n,
                Value::Bool(b) => f64::from(u8::from(*b)),
                Value::Empty => 0.0,
                Value::Text(t) => parse_number_text_in(t, ctx.date1904).ok_or(ErrorKind::Value)?,
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

/// Adds left to right. Like Excel, the last addition collapses a result within binary noise of zero to
/// exactly zero (`SUM(0.1,0.2,-0.3)` is 0).
fn total(nums: &[f64]) -> f64 {
    match nums.split_last() {
        None => 0.0,
        Some((last, head)) => {
            let before: f64 = head.iter().fold(0.0, |acc, x| acc + x);
            snap_to_zero(before + last, before, *last)
        }
    }
}

pub(super) fn sum(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    number(total(&collect(ctx, args)?))
}

pub(super) fn average(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let nums = collect(ctx, args)?;
    if nums.is_empty() {
        return Err(ErrorKind::DivZero);
    }
    number(total(&nums) / nums.len() as f64)
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
    let nums = collect_with(ctx, args, false)?;
    if nums.is_empty() {
        return Ok(Value::Number(0.0));
    }
    number(nums.iter().product())
}

pub(super) fn count(ctx: &EvalContext<'_>, args: &[Arg]) -> Value {
    if args.is_empty() {
        return Value::Error(ErrorKind::Value);
    }
    let mut n = 0u32;
    for arg in args {
        match arg {
            Arg::Missing => n += 1,
            Arg::Value(v) => {
                let counted = match v {
                    Value::Number(_) | Value::Bool(_) => true,
                    Value::Text(t) => parse_number_text_in(t, ctx.date1904).is_some(),
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
    if args.is_empty() {
        return Value::Error(ErrorKind::Value);
    }
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
    // ROUND works on the shortest digits that identify the double, cut (not rounded) to 15 significant
    // digits (so ROUND(0.49999999999999994,0) is 0); ROUNDUP/ROUNDDOWN on the number rounded to 15 digits
    // (so ROUNDDOWN(0.7+0.1,1) is 0.8).
    let sci = if matches!(mode, RoundMode::Nearest) {
        format!("{:e}", x.abs())
    } else {
        format!("{:.14e}", x.abs())
    };
    let Some((mantissa, exp)) = sci.split_once('e') else {
        return x;
    };
    let exp10: i64 = exp.parse().unwrap_or(0);
    let mut digits: Vec<u8> = mantissa
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();
    digits.truncate(15);
    let keep = exp10 + 1 + places;
    let (kept, first_discarded, any_discarded) = if keep >= digits.len() as i64 {
        (&digits[..], 0, false)
    } else if keep >= 0 {
        let k = keep as usize;
        (&digits[..k], digits[k], digits[k..].iter().any(|&d| d != 0))
    } else {
        (&digits[..0], 0, true)
    };
    let mut kept_int = kept.iter().fold(0u64, |acc, &d| acc * 10 + u64::from(d));
    // Digits dropped by the 15-digit cut sit below the kept position when `keep >= 15`; nothing to add then.
    let increment = match mode {
        RoundMode::Nearest => first_discarded >= 5,
        RoundMode::Up => any_discarded,
        RoundMode::Down => false,
    };
    if increment {
        kept_int += 1;
    }
    // value = kept_int x 10^(exp10 + 1 - kept.len()) when nothing was dropped, else x 10^(-places).
    let scale_exp = if keep >= digits.len() as i64 {
        exp10 + 1 - kept.len() as i64
    } else {
        -places
    };
    let magnitude = format!("{kept_int}e{scale_exp}")
        .parse::<f64>()
        .unwrap_or(x.abs());
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
    if matches!(mode, RoundMode::Up) && places < -308 {
        return Err(ErrorKind::Num);
    }
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
    let x = num_arg(ctx, &args[0])?;
    // Excel works on the 15-digit value: INT(0.57*100) is 57, not 56. Beyond 1E15 there is nothing to snap.
    let snapped = if x.abs() < 1e15 {
        format!("{x:.14e}").parse::<f64>().unwrap_or(x)
    } else {
        x
    };
    number(snapped.floor())
}

/// Quotients at or above this make `MOD` return `#NUM!`.
const MOD_QUOTIENT_LIMIT: f64 = 1e12;

pub(super) fn modulo(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let n = num_arg(ctx, &args[0])?;
    let d = num_arg(ctx, &args[1])?;
    if d == 0.0 {
        return Err(ErrorKind::DivZero);
    }
    // Excel gives #NUM! when the quotient is too large for the remainder to be meaningful.
    if (n / d).abs() >= MOD_QUOTIENT_LIMIT {
        return Err(ErrorKind::Num);
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
