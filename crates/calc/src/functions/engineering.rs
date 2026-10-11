//! Engineering functions: base conversions, bit operations, `DELTA`, `GESTEP`.
//! Measured against Excel; see docs/testing/calc-conformance.md.

use super::datecalc::{atp_num, atp_value};
use super::{Res, arity, num_arg, optional};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value, number_to_text};

/// Text of the number argument of a base-conversion function.
fn digits_text(ctx: &EvalContext<'_>, arg: &Arg) -> Res<String> {
    match atp_value(ctx, arg)? {
        Value::Number(n) => Ok(number_to_text(n)),
        Value::Text(t) => Ok(t),
        Value::Empty => Ok(String::new()),
        Value::Error(e) => Err(e),
        _ => Err(ErrorKind::Value),
    }
}

/// Parses a 10 digit two's complement number in `radix` (bits per digit 1, 3 or 4).
fn parse_radix(text: &str, radix: u32) -> Res<i64> {
    if text.chars().count() > 10 {
        return Err(ErrorKind::Num);
    }
    let mut v: i64 = 0;
    for c in text.chars() {
        let d = c.to_digit(radix).ok_or(ErrorKind::Num)?;
        v = v * i64::from(radix) + i64::from(d);
    }
    let bits = match radix {
        2 => 10,
        8 => 30,
        _ => 40,
    };
    if text.chars().count() == 10 && v >= 1i64 << (bits - 1) {
        v -= 1i64 << bits;
    }
    Ok(v)
}

fn to_radix(n: i64, radix: u32, places: Option<i64>) -> Res<Value> {
    let bits = match radix {
        2 => 10,
        8 => 30,
        _ => 40,
    };
    if n < -(1i64 << (bits - 1)) || n >= 1i64 << (bits - 1) {
        return Err(ErrorKind::Num);
    }
    let neg = n < 0;
    let mut u = if neg {
        (n + (1i64 << bits)) as u64
    } else {
        n as u64
    };
    let mut out = Vec::new();
    while u > 0 {
        out.push(std::char::from_digit((u % u64::from(radix)) as u32, radix).unwrap_or('0'));
        u /= u64::from(radix);
    }
    if out.is_empty() {
        out.push('0');
    }
    let mut s: String = out.iter().rev().collect();
    if neg {
        let width = 10;
        while s.len() < width {
            s.insert(0, '0');
        }
        // A negative number always has all 10 digits.
        if let Some(p) = places
            && !(1..=10).contains(&p)
        {
            return Err(ErrorKind::Num);
        }
    } else if let Some(p) = places {
        if !(1..=10).contains(&p) || (s.len() as i64) > p {
            return Err(ErrorKind::Num);
        }
        while (s.len() as i64) < p {
            s.insert(0, '0');
        }
    }
    Ok(Value::Text(s.to_uppercase()))
}

fn places_arg(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Option<i64>> {
    match optional(args, 1) {
        None => Ok(None),
        Some(a) => Ok(Some(atp_num(ctx, a)?.trunc() as i64)),
    }
}

fn radix_to_radix(ctx: &EvalContext<'_>, args: &[Arg], from: u32, to: u32) -> Res<Value> {
    arity(args, 1, 2)?;
    let text = digits_text(ctx, &args[0])?;
    let places = places_arg(ctx, args)?;
    let n = parse_radix(&text, from)?;
    to_radix(n, to, places)
}

macro_rules! conv {
    ($name:ident, $from:expr, $to:expr) => {
        pub(super) fn $name(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
            radix_to_radix(ctx, args, $from, $to)
        }
    };
}
conv!(bin2oct, 2, 8);
conv!(bin2hex, 2, 16);
conv!(oct2bin, 8, 2);
conv!(oct2hex, 8, 16);
conv!(hex2bin, 16, 2);
conv!(hex2oct, 16, 8);

macro_rules! to_dec {
    ($name:ident, $from:expr) => {
        pub(super) fn $name(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
            arity(args, 1, 1)?;
            let text = digits_text(ctx, &args[0])?;
            Ok(Value::Number(parse_radix(&text, $from)? as f64))
        }
    };
}
to_dec!(bin2dec, 2);
to_dec!(oct2dec, 8);
to_dec!(hex2dec, 16);

fn dec_to(ctx: &EvalContext<'_>, args: &[Arg], to: u32) -> Res<Value> {
    arity(args, 1, 2)?;
    // The places argument is checked before the number is looked at.
    let places = places_arg(ctx, args)?;
    if let Some(p) = places
        && !(1..=10).contains(&p)
    {
        return Err(ErrorKind::Num);
    }
    let n = atp_num(ctx, &args[0])?.trunc();
    if n.abs() > 1e12 {
        return Err(ErrorKind::Num);
    }
    to_radix(n as i64, to, places)
}

pub(super) fn dec2bin(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    dec_to(ctx, args, 2)
}
pub(super) fn dec2oct(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    dec_to(ctx, args, 8)
}
pub(super) fn dec2hex(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    dec_to(ctx, args, 16)
}

const BIT_LIMIT: f64 = 281_474_976_710_656.0; // 2^48

/// A bit-operation operand: booleans count as 0/1 and an omitted argument is 0.
fn bit_number(ctx: &EvalContext<'_>, arg: &Arg) -> Res<f64> {
    match arg {
        Arg::Missing => Ok(0.0),
        Arg::Range(r) if !r.is_single_cell() => Err(ErrorKind::Value),
        _ => num_arg(ctx, arg),
    }
}

fn bit_check(n: f64) -> Res<u64> {
    if n < 0.0 || n != n.trunc() || n >= BIT_LIMIT {
        return Err(ErrorKind::Num);
    }
    Ok(n as u64)
}

fn bitop(ctx: &EvalContext<'_>, args: &[Arg], op: fn(u64, u64) -> u64) -> Res<Value> {
    arity(args, 2, 2)?;
    let a = bit_number(ctx, &args[0])?;
    let b = bit_number(ctx, &args[1])?;
    Ok(Value::Number(op(bit_check(a)?, bit_check(b)?) as f64))
}

pub(super) fn bitand(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    bitop(ctx, args, |a, b| a & b)
}
pub(super) fn bitor(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    bitop(ctx, args, |a, b| a | b)
}
pub(super) fn bitxor(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    bitop(ctx, args, |a, b| a ^ b)
}

/// A left shift by 52 or 53 gives 0 (`BITLSHIFT(1,52)`), a smaller one is `#NUM!` from 2^48 up, and a zero
/// operand is 0 for any shift.
fn shift(ctx: &EvalContext<'_>, args: &[Arg], left: bool) -> Res<Value> {
    arity(args, 2, 2)?;
    let a = bit_number(ctx, &args[0])?;
    let s = bit_number(ctx, &args[1])?.trunc();
    let a = bit_check(a)?;
    if a == 0 {
        return Ok(Value::Number(0.0));
    }
    if s.abs() > 53.0 {
        return Err(ErrorKind::Num);
    }
    let s = s as i64;
    let left = if s < 0 { !left } else { left };
    let s = s.unsigned_abs() as u32;
    let r = if left {
        if s >= 52 {
            0.0
        } else {
            (u128::from(a) << s) as f64
        }
    } else {
        (a >> s) as f64
    };
    if r >= BIT_LIMIT {
        return Err(ErrorKind::Num);
    }
    Ok(Value::Number(r))
}

pub(super) fn bitlshift(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    shift(ctx, args, true)
}
pub(super) fn bitrshift(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    shift(ctx, args, false)
}

fn opt_num(ctx: &EvalContext<'_>, args: &[Arg], i: usize) -> Res<f64> {
    match optional(args, i) {
        None => Ok(0.0),
        Some(a) => atp_num(ctx, a),
    }
}

pub(super) fn delta(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 2)?;
    let a = atp_num(ctx, &args[0])?;
    let b = opt_num(ctx, args, 1)?;
    Ok(Value::Number(if a == b { 1.0 } else { 0.0 }))
}

pub(super) fn gestep(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 2)?;
    let a = atp_num(ctx, &args[0])?;
    let b = opt_num(ctx, args, 1)?;
    Ok(Value::Number(if a >= b { 1.0 } else { 0.0 }))
}
