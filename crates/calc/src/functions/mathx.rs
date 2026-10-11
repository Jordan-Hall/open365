//! Reciprocal trigonometric and inverse hyperbolic functions, and the number-system functions
//! (`BASE`, `DECIMAL`, `ARABIC`, `ROMAN`) plus `FACTDOUBLE`, `MULTINOMIAL`, `SERIESSUM`, `SQRTPI`.
//! Formulas were fitted bit for bit against Excel; see "Second math batch" in `docs/testing/calc-conformance.md`.

use super::arith::strict_num;
use super::{Res, arity, int_arg, num_arg, number, optional};
use crate::eval::{Arg, EvalContext, power as eval_power};
use crate::value::{ErrorKind, Value};
use crate::xmath;

const TRIG_LIMIT: f64 = 134_217_728.0;

fn unary(ctx: &EvalContext<'_>, args: &[Arg], f: impl Fn(f64) -> Res<f64>) -> Res<Value> {
    arity(args, 1, 1)?;
    number(f(num_arg(ctx, &args[0])?)?)
}

/// `1 / v`, with `#DIV/0!` for a zero denominator.
fn recip(v: f64) -> Res<f64> {
    if v == 0.0 {
        Err(ErrorKind::DivZero)
    } else {
        Ok(1.0 / v)
    }
}

pub(super) fn asinh(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    unary(ctx, args, |x| {
        let a = x.abs();
        let r = xmath::ln(a + (a * a + 1.0).sqrt());
        Ok(if x < 0.0 { -r } else { r })
    })
}

pub(super) fn acosh(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    unary(ctx, args, |x| {
        if x < 1.0 {
            return Err(ErrorKind::Num);
        }
        Ok(xmath::ln(x + (x * x - 1.0).sqrt()))
    })
}

pub(super) fn atanh(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    unary(ctx, args, |x| {
        if x.abs() >= 1.0 {
            return Err(ErrorKind::Num);
        }
        Ok(xmath::atanh_formula(x))
    })
}

pub(super) fn acot(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    unary(ctx, args, |x| {
        Ok(if x == 0.0 {
            std::f64::consts::FRAC_PI_2
        } else if x > 0.0 {
            xmath::atan(1.0 / x)
        } else {
            xmath::atan(1.0 / x) + std::f64::consts::PI
        })
    })
}

pub(super) fn acoth(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    unary(ctx, args, |x| {
        if x.abs() <= 1.0 {
            return Err(ErrorKind::Num);
        }
        let r = xmath::acoth_magnitude(x.abs());
        Ok(if x < 0.0 { -r } else { r })
    })
}

fn trig3(x: f64) -> Res<(f64, f64, f64)> {
    if x.abs() >= TRIG_LIMIT {
        return Err(ErrorKind::Num);
    }
    Ok(xmath::sin_cos_tan(x))
}

pub(super) fn cot(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    unary(ctx, args, |x| recip(trig3(x)?.2))
}

pub(super) fn csc(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    unary(ctx, args, |x| recip(trig3(x)?.0))
}

pub(super) fn sec(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    unary(ctx, args, |x| recip(trig3(x)?.1))
}

pub(super) fn coth(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    unary(ctx, args, |x| recip(xmath::tanh(x)))
}

pub(super) fn csch(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    unary(ctx, args, |x| {
        if x.abs() > 709.782_712_893_384 {
            return Ok(0.0);
        }
        recip(xmath::sinh(x))
    })
}

pub(super) fn sech(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    unary(ctx, args, |x| {
        if x.abs() > 709.782_712_893_384 {
            return Ok(0.0);
        }
        recip(xmath::cosh(x))
    })
}

pub(super) fn sqrtpi(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    let x = strict_num(ctx, &args[0])?;
    if x < 0.0 {
        return Err(ErrorKind::Num);
    }
    number((x * std::f64::consts::PI).sqrt())
}

// ---- factorials ------------------------------------------------------------------------------

pub(super) fn factdouble(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    let n = strict_num(ctx, &args[0])?.trunc();
    if n < -1.0 {
        return Err(ErrorKind::Num);
    }
    let mut r = 1.0f64;
    let mut i = n;
    while i > 1.0 {
        r *= i;
        i -= 2.0;
    }
    number(r)
}

pub(super) fn seriessum(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 4, 4)?;
    let x = num_arg(ctx, &args[0])?;
    let n = num_arg(ctx, &args[1])?;
    let m = num_arg(ctx, &args[2])?;
    let mut coeffs = Vec::new();
    match &args[3] {
        Arg::Value(Value::Number(c)) => coeffs.push(*c),
        Arg::Value(Value::Error(e)) => return Err(*e),
        Arg::Value(_) | Arg::Missing => return Err(ErrorKind::Value),
        other => {
            for v in ctx.cells(other) {
                match v {
                    Value::Number(c) => coeffs.push(c),
                    Value::Error(e) => return Err(e),
                    _ => return Err(ErrorKind::Value),
                }
            }
        }
    }
    let mut sum = 0.0;
    for (i, c) in coeffs.iter().enumerate() {
        let p = n + (i as f64) * m;
        // Measured: 0 with a non-positive exponent is #NUM!; a negative base with a fractional exponent adds nothing.
        let power = if x == 0.0 {
            if p <= 0.0 {
                return Err(ErrorKind::Num);
            }
            0.0
        } else if x < 0.0 && p.fract() != 0.0 {
            0.0
        } else {
            eval_power(x, p)?
        };
        sum += c * power;
    }
    number(sum)
}

// ---- number systems --------------------------------------------------------------------------

const DIGITS: &[u8; 36] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";

pub(super) fn base(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 3)?;
    let n = num_arg(ctx, &args[0])?.trunc();
    let radix = int_arg(ctx, &args[1])?;
    let min_len = match optional(args, 2) {
        Some(a) => int_arg(ctx, a)?,
        None => 1,
    };
    if !(0.0..9_007_199_254_740_992.0).contains(&n)
        || !(2..=36).contains(&radix)
        || !(0..=255).contains(&min_len)
    {
        return Err(ErrorKind::Num);
    }
    let mut v = n as u64;
    let mut out = Vec::new();
    while v > 0 {
        out.push(DIGITS[(v % radix as u64) as usize]);
        v /= radix as u64;
    }
    while out.len() < min_len as usize {
        out.push(b'0');
    }
    out.reverse();
    Ok(Value::Text(String::from_utf8(out).unwrap_or_default()))
}

pub(super) fn decimal(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let text = super::text_arg(ctx, &args[0])?;
    let radix = int_arg(ctx, &args[1])?;
    if !(2..=36).contains(&radix) {
        return Err(ErrorKind::Num);
    }
    let mut s = text.trim_start_matches(' ');
    if radix == 16
        && let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))
    {
        s = rest;
    }
    // Exact integer arithmetic while it fits, rounded once (Excel's `ZZZZZZZZZZZZ` in base 36 is not the double fold).
    let mut exact: Option<u128> = Some(0);
    let mut approx = 0.0f64;
    for ch in s.chars() {
        let d = ch.to_digit(36).ok_or(ErrorKind::Num)?;
        if i64::from(d) >= radix {
            return Err(ErrorKind::Num);
        }
        exact = exact.and_then(|v| v.checked_mul(radix as u128)?.checked_add(u128::from(d)));
        approx = approx * radix as f64 + f64::from(d);
    }
    number(exact.map_or(approx, |v| v as f64))
}

fn roman_value(c: char) -> Option<f64> {
    Some(match c.to_ascii_uppercase() {
        'M' => 1000.0,
        'D' => 500.0,
        'C' => 100.0,
        'L' => 50.0,
        'X' => 10.0,
        'V' => 5.0,
        'I' => 1.0,
        _ => return None,
    })
}

pub(super) fn arabic(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    let text = match super::scalar(ctx, &args[0])? {
        Value::Text(t) => t,
        Value::Empty => String::new(),
        _ => return Err(ErrorKind::Value),
    };
    let s = text.trim_matches(' ');
    let (negative, body) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    if negative && body.is_empty() {
        return Err(ErrorKind::Num);
    }
    // Right to left: a letter smaller than the largest seen so far is subtracted.
    let mut total = 0.0;
    let mut largest = 0.0f64;
    for c in body.chars().rev() {
        let v = roman_value(c).ok_or(ErrorKind::Value)?;
        if v < largest {
            total -= v;
        } else {
            total += v;
            largest = v;
        }
    }
    number(if negative { -total } else { total })
}

pub(super) fn roman(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 2)?;
    let n = num_arg(ctx, &args[0])?.trunc();
    let mode = match optional(args, 1) {
        Some(a) => match super::scalar(ctx, a)? {
            Value::Bool(true) => 0,
            Value::Bool(false) => 4,
            _ => num_arg(ctx, a)?.trunc() as i64,
        },
        None => 0,
    };
    if !(0.0..=3999.0).contains(&n) || !(0..=4).contains(&mode) {
        return Err(ErrorKind::Value);
    }
    const CHARS: [char; 7] = ['M', 'D', 'C', 'L', 'X', 'V', 'I'];
    const VALUES: [i32; 7] = [1000, 500, 100, 50, 10, 5, 1];
    let mut val = n as i32;
    let mut out = String::new();
    for i in 0..=3usize {
        let mut index = 2 * i;
        let digit = val / VALUES[index];
        if digit % 5 == 4 {
            let index2 = if digit == 4 { index - 1 } else { index - 2 };
            let mut steps = 0;
            while steps < mode && index < 6 {
                steps += 1;
                if VALUES[index2] - VALUES[index + 1] <= val {
                    index += 1;
                } else {
                    steps = mode;
                }
            }
            out.push(CHARS[index]);
            out.push(CHARS[index2]);
            val += VALUES[index];
            val -= VALUES[index2];
        } else {
            let mut d = digit;
            if d > 4 {
                out.push(CHARS[index - 1]);
                d -= 5;
            }
            for _ in 0..d {
                out.push(CHARS[index]);
            }
            val %= VALUES[index];
        }
    }
    Ok(Value::Text(out))
}
