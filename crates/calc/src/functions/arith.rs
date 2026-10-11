//! Arithmetic, number-theory and trigonometric functions (everything of the math library that is not an
//! aggregate). Floating point details follow Excel: quotients are snapped to 15 significant digits before
//! rounding (`CEILING(0.3,0.1)` is 0.3), and the trigonometric functions refuse huge arguments.

use std::sync::atomic::{AtomicU64, Ordering};

use super::math::{RoundMode, collect, round_decimal};
use super::{Res, arity, int_arg, num_arg, number, optional};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value, round15};
use crate::xmath;

/// `a / b` with a quotient that is within 15 digits of a whole number treated as that whole number.
fn snapped_quotient(a: f64, b: f64) -> f64 {
    let q = a / b;
    let s = round15(q);
    if s.fract() == 0.0 { s } else { q }
}

fn up_to_multiple(x: f64, sig: f64) -> f64 {
    snapped_quotient(x, sig).ceil() * sig
}

fn down_to_multiple(x: f64, sig: f64) -> f64 {
    snapped_quotient(x, sig).floor() * sig
}

pub(super) fn ceiling(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let x = num_arg(ctx, &args[0])?;
    let sig = num_arg(ctx, &args[1])?;
    if sig == 0.0 || x == 0.0 {
        return Ok(Value::Number(0.0));
    }
    if x > 0.0 && sig < 0.0 {
        return Err(ErrorKind::Num);
    }
    // For a negative number the result moves toward zero with a positive significance and away with a negative one.
    let r = if x > 0.0 {
        up_to_multiple(x, sig)
    } else if sig > 0.0 {
        -down_to_multiple(-x, sig)
    } else {
        -up_to_multiple(-x, -sig)
    };
    number(r)
}

pub(super) fn floor(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let x = num_arg(ctx, &args[0])?;
    let sig = num_arg(ctx, &args[1])?;
    if x == 0.0 {
        return Ok(Value::Number(0.0));
    }
    if sig == 0.0 {
        return Err(ErrorKind::DivZero);
    }
    if x > 0.0 && sig < 0.0 {
        return Err(ErrorKind::Num);
    }
    let r = if x > 0.0 {
        down_to_multiple(x, sig)
    } else if sig > 0.0 {
        -up_to_multiple(-x, sig)
    } else {
        -down_to_multiple(-x, -sig)
    };
    number(r)
}

/// Shared argument handling of `CEILING.MATH` and `FLOOR.MATH`: `(number, |significance|, mode)`.
fn math_args(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<(f64, f64, bool)> {
    arity(args, 1, 3)?;
    let x = num_arg(ctx, &args[0])?;
    let sig = match optional(args, 1) {
        Some(a) => num_arg(ctx, a)?.abs(),
        None => 1.0,
    };
    let mode = match optional(args, 2) {
        Some(a) => num_arg(ctx, a)? != 0.0,
        None => false,
    };
    Ok((x, sig, mode))
}

pub(super) fn ceiling_math(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let (x, sig, away) = math_args(ctx, args)?;
    if sig == 0.0 {
        return Ok(Value::Number(0.0));
    }
    number(if x >= 0.0 {
        up_to_multiple(x, sig)
    } else if away {
        -up_to_multiple(-x, sig)
    } else {
        -down_to_multiple(-x, sig)
    })
}

pub(super) fn floor_math(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let (x, sig, toward_zero) = math_args(ctx, args)?;
    if sig == 0.0 {
        return Ok(Value::Number(0.0));
    }
    number(if x >= 0.0 {
        down_to_multiple(x, sig)
    } else if toward_zero {
        -down_to_multiple(-x, sig)
    } else {
        -up_to_multiple(-x, sig)
    })
}

pub(super) fn mround(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let x = num_arg(ctx, &args[0])?;
    let m = num_arg(ctx, &args[1])?;
    if m == 0.0 {
        return Ok(Value::Number(0.0));
    }
    if x * m < 0.0 {
        return Err(ErrorKind::Num);
    }
    let q = snapped_quotient(x, m);
    number((q.abs() + 0.5).floor().copysign(q) * m)
}

pub(super) fn trunc(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 2)?;
    let x = num_arg(ctx, &args[0])?;
    let places = match optional(args, 1) {
        Some(a) => int_arg(ctx, a)?,
        None => 0,
    };
    number(round_decimal(x, places, RoundMode::Down))
}

pub(super) fn sign(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    let x = num_arg(ctx, &args[0])?;
    Ok(Value::Number(if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }))
}

pub(super) fn exp(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    number(xmath::exp(num_arg(ctx, &args[0])?))
}

pub(super) fn ln(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    let x = num_arg(ctx, &args[0])?;
    if x <= 0.0 {
        return Err(ErrorKind::Num);
    }
    number(xmath::ln(x))
}

pub(super) fn log10(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    let x = num_arg(ctx, &args[0])?;
    if x <= 0.0 {
        return Err(ErrorKind::Num);
    }
    number(xmath::log10(x))
}

pub(super) fn log(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 2)?;
    let x = num_arg(ctx, &args[0])?;
    let Some(base_arg) = args.get(1) else {
        if x <= 0.0 {
            return Err(ErrorKind::Num);
        }
        return number(xmath::log10(x));
    };
    let base = num_arg(ctx, base_arg)?;
    if x <= 0.0 || base <= 0.0 {
        return Err(ErrorKind::Num);
    }
    if base == 1.0 {
        return Err(ErrorKind::DivZero);
    }
    number(xmath::ln(x) / xmath::ln(base))
}

/// Integer argument for the combinatorial functions: truncated, and negative values are `#NUM!`.
fn count_arg(ctx: &EvalContext<'_>, arg: &Arg) -> Res<f64> {
    let n = num_arg(ctx, arg)?.trunc();
    if n < 0.0 { Err(ErrorKind::Num) } else { Ok(n) }
}

/// `n!` for a whole `n` in 0..=170, multiplied from the top down as Excel does (the order changes the last digit for
/// n >= 23). Larger values are `#NUM!`.
fn factorial(n: f64) -> Res<f64> {
    if n > 170.0 {
        return Err(ErrorKind::Num);
    }
    Ok((2..=n as u32).rev().fold(1.0, |acc, i| acc * f64::from(i)))
}

pub(super) fn fact(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    if num_arg(ctx, &args[0])? < 0.0 {
        return Err(ErrorKind::Num);
    }
    number(factorial(count_arg(ctx, &args[0])?)?)
}

pub(super) fn combin(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let n = count_arg(ctx, &args[0])?;
    let k = count_arg(ctx, &args[1])?;
    if k > n {
        return Err(ErrorKind::Num);
    }
    let k = k.min(n - k) as u64;
    let n = n as u64;
    let mut r = 1.0f64;
    for i in 1..=k {
        r = r * (n - k + i) as f64 / i as f64;
    }
    number(r.round())
}

pub(super) fn permut(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let n = count_arg(ctx, &args[0])?;
    let k = count_arg(ctx, &args[1])?;
    if k > n {
        return Err(ErrorKind::Num);
    }
    let (n, k) = (n as u64, k as u64);
    // Multiplied from the bottom up, which is what Excel does (the last digit of large results shows it).
    let mut r = 1.0f64;
    for i in (n - k + 1)..=n {
        r *= i as f64;
    }
    number(r)
}

/// Number parameter of the old Analysis ToolPak functions, which refuse booleans.
pub(super) fn strict_num(ctx: &EvalContext<'_>, arg: &Arg) -> Res<f64> {
    if matches!(ctx.scalar(arg), Value::Bool(_)) {
        return Err(ErrorKind::Value);
    }
    num_arg(ctx, arg)
}

/// Whole numbers for `GCD`/`LCM`: truncated, negative or beyond 2^53 is `#NUM!`. Text anywhere is `#VALUE!`.
fn whole_numbers(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Vec<u64>> {
    let mut nums = Vec::new();
    for arg in args {
        match arg {
            Arg::Missing => return Err(ErrorKind::NA),
            Arg::Value(Value::Bool(_)) => return Err(ErrorKind::Value),
            Arg::Value(_) => nums.push(num_arg(ctx, arg)?),
            _ => {
                for v in ctx.cells(arg) {
                    match v {
                        Value::Number(n) => nums.push(n),
                        Value::Empty => nums.push(0.0),
                        Value::Error(e) => return Err(e),
                        _ => return Err(ErrorKind::Value),
                    }
                }
            }
        }
    }
    arity(args, 1, usize::MAX)?;
    nums.into_iter()
        .map(|x| {
            let t = x.trunc();
            if !(0.0..=9_007_199_254_740_992.0).contains(&t) {
                Err(ErrorKind::Num)
            } else {
                Ok(t as u64)
            }
        })
        .collect()
}

fn gcd2(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

pub(super) fn sumsq(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    number(collect(ctx, args)?.iter().map(|x| x * x).sum())
}

pub(super) fn gcd(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let nums = whole_numbers(ctx, args)?;
    Ok(Value::Number(nums.into_iter().fold(0, gcd2) as f64))
}

pub(super) fn lcm(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let nums = whole_numbers(ctx, args)?;
    let mut acc = 1.0f64;
    for n in nums {
        if n == 0 {
            return Ok(Value::Number(0.0));
        }
        let g = gcd2(acc as u64, n) as f64;
        acc = acc / g * n as f64;
        if acc >= 9_007_199_254_740_992.0 {
            return Err(ErrorKind::Num);
        }
    }
    Ok(Value::Number(acc))
}

pub(super) fn quotient(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let n = strict_num(ctx, &args[0])?;
    let d = strict_num(ctx, &args[1])?;
    if d == 0.0 {
        return Err(ErrorKind::DivZero);
    }
    number((n / d).trunc())
}

fn to_parity(ctx: &EvalContext<'_>, args: &[Arg], even: bool) -> Res<Value> {
    arity(args, 1, 1)?;
    let x = num_arg(ctx, &args[0])?;
    let a = x.abs().ceil();
    let is_even = a % 2.0 == 0.0;
    let r = if is_even == even { a } else { a + 1.0 };
    // EVEN(0) and ODD(0) are 0 and 1; a positive tiny value rounds to the first even/odd number.
    let r = if x == 0.0 && even { 0.0 } else { r };
    number(r.copysign(if x < 0.0 { -1.0 } else { 1.0 }))
}

pub(super) fn even(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    to_parity(ctx, args, true)
}

pub(super) fn odd(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    to_parity(ctx, args, false)
}

// ---- random numbers ------------------------------------------------------------------------------

static RNG: AtomicU64 = AtomicU64::new(0);

/// A uniform value in `[0, 1)` from a splitmix64 generator seeded from the system clock.
fn next_random() -> f64 {
    let mut state = RNG.load(Ordering::Relaxed);
    if state == 0 {
        state = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x9E37_79B9_7F4A_7C15, |d| d.as_nanos() as u64)
            | 1;
    }
    state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    RNG.store(state, Ordering::Relaxed);
    let mut z = state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

pub(super) fn rand(_ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 0, 0)?;
    Ok(Value::Number(next_random()))
}

pub(super) fn randbetween(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let low = strict_num(ctx, &args[0])?.ceil();
    let high = strict_num(ctx, &args[1])?.ceil();
    if low > high {
        return Err(ErrorKind::Num);
    }
    number(low + (next_random() * (high - low + 1.0)).floor())
}

// ---- trigonometry --------------------------------------------------------------------------------

/// Largest magnitude `SIN`, `COS` and `TAN` accept (2^27).
const TRIG_LIMIT: f64 = 134_217_728.0;

fn unary(ctx: &EvalContext<'_>, args: &[Arg], f: impl Fn(f64) -> Res<f64>) -> Res<Value> {
    arity(args, 1, 1)?;
    number(f(num_arg(ctx, &args[0])?)?)
}

/// `SIN`, `COS` and `TAN` reduce by a pi of 64 mantissa bits as Excel does (`SIN(PI())` is `1.22514845490862E-16`,
/// not `1.2246E-16`) and round once: see `xmath::sin_cos_tan`.
fn trig(ctx: &EvalContext<'_>, args: &[Arg], pick: impl Fn((f64, f64, f64)) -> f64) -> Res<Value> {
    unary(ctx, args, |x| {
        if x.abs() >= TRIG_LIMIT {
            return Err(ErrorKind::Num);
        }
        Ok(pick(xmath::sin_cos_tan(x)))
    })
}

pub(super) fn sin(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    trig(ctx, args, |t| t.0)
}

pub(super) fn cos(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    trig(ctx, args, |t| t.1)
}

pub(super) fn tan(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    trig(ctx, args, |t| t.2)
}

pub(super) fn asin(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    unary(ctx, args, |x| {
        if x.abs() > 1.0 {
            Err(ErrorKind::Num)
        } else {
            Ok(xmath::asin(x))
        }
    })
}

pub(super) fn acos(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    unary(ctx, args, |x| {
        if x.abs() > 1.0 {
            Err(ErrorKind::Num)
        } else {
            Ok(xmath::acos(x))
        }
    })
}

pub(super) fn atan(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    unary(ctx, args, |x| Ok(xmath::atan(x)))
}

pub(super) fn atan2(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let x = num_arg(ctx, &args[0])?;
    let y = num_arg(ctx, &args[1])?;
    if x == 0.0 && y == 0.0 {
        return Err(ErrorKind::DivZero);
    }
    number(xmath::atan2(y, x))
}

pub(super) fn degrees(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    unary(ctx, args, |x| Ok(x * (180.0 / std::f64::consts::PI)))
}

pub(super) fn radians(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    unary(ctx, args, |x| Ok(x * (std::f64::consts::PI / 180.0)))
}

/// Overflow of `e^|x|` is `#NUM!` for `SINH`, `COSH` and `TANH` alike.
fn hyperbolic(x: f64, f: impl Fn(f64) -> f64) -> Res<f64> {
    if x.abs() > 709.782_712_893_384 {
        return Err(ErrorKind::Num);
    }
    Ok(f(x))
}

pub(super) fn sinh(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    unary(ctx, args, |x| hyperbolic(x, xmath::sinh))
}

pub(super) fn cosh(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    unary(ctx, args, |x| hyperbolic(x, xmath::cosh))
}

pub(super) fn tanh(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    unary(ctx, args, |x| {
        if x.abs() > 22.0 {
            Ok(x.signum())
        } else {
            hyperbolic(x, xmath::tanh)
        }
    })
}
