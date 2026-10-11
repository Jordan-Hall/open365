//! Math functions added after the first batches: `CEILING.PRECISE`, `FLOOR.PRECISE`, `ISO.CEILING`, `COMBINA`,
//! `MULTINOMIAL` and `RANDARRAY`. Edge rules were measured against Excel (see docs/testing/calc-coverage.md).

use super::arith::{
    count_arg, down_to_multiple, factorial, next_random, strict_num, up_to_multiple,
};
use super::{Res, arity, num_arg, number, optional};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value};

/// `(number, |significance|)` of the three rounding functions; the sign of the significance is ignored.
fn precise_args(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<(f64, f64)> {
    arity(args, 1, 2)?;
    let x = num_arg(ctx, &args[0])?;
    let sig = match optional(args, 1) {
        Some(a) => num_arg(ctx, a)?.abs(),
        None => 1.0,
    };
    Ok((x, sig))
}

/// Rounds toward positive infinity to a multiple of `|significance|`.
pub(super) fn ceiling_precise(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let (x, sig) = precise_args(ctx, args)?;
    if sig == 0.0 || x == 0.0 {
        return Ok(Value::Number(0.0));
    }
    number(if x > 0.0 {
        up_to_multiple(x, sig)
    } else {
        -down_to_multiple(-x, sig)
    })
}

/// Rounds toward negative infinity to a multiple of `|significance|`.
pub(super) fn floor_precise(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let (x, sig) = precise_args(ctx, args)?;
    if sig == 0.0 || x == 0.0 {
        return Ok(Value::Number(0.0));
    }
    number(if x > 0.0 {
        down_to_multiple(x, sig)
    } else {
        -up_to_multiple(-x, sig)
    })
}

/// Combinations with repetition: `COMBIN(n + k - 1, k)`.
pub(super) fn combina(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let n = count_arg(ctx, &args[0])?;
    let k = count_arg(ctx, &args[1])?;
    if n == 0.0 {
        return if k == 0.0 {
            Ok(Value::Number(1.0))
        } else {
            Err(ErrorKind::Num)
        };
    }
    if n >= 2_147_483_647.0 || k >= 2_147_483_647.0 {
        return Err(ErrorKind::Num);
    }
    // Product of (n - 1 + i) / i over the shorter side; Excel does not round the result (46375.99999999999).
    let total = (n + k - 1.0) as u64;
    let steps = k.min(n - 1.0) as u64;
    let mut r = 1.0f64;
    for i in 1..=steps {
        r = r / i as f64 * (total - steps + i) as f64;
        if !r.is_finite() {
            return Err(ErrorKind::Num);
        }
    }
    number(r)
}

pub(super) fn multinomial(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, usize::MAX)?;
    let mut parts = Vec::new();
    for (i, arg) in args.iter().enumerate() {
        match arg {
            // An omitted first argument is #N/A; later ones count as 0.
            Arg::Missing if i == 0 => return Err(ErrorKind::NA),
            Arg::Missing => parts.push(0.0),
            Arg::Value(_) => parts.push(strict_num(ctx, arg)?),
            _ => {
                for v in ctx.cells(arg) {
                    match v {
                        Value::Number(n) => parts.push(n),
                        Value::Error(e) => return Err(e),
                        _ => {}
                    }
                }
            }
        }
    }
    let mut sum = 0.0;
    let mut denominator = 1.0;
    for p in &parts {
        let p = p.trunc();
        if p < 0.0 {
            return Err(ErrorKind::Num);
        }
        sum += p;
        denominator *= factorial(p)?;
    }
    number(factorial(sum)? / denominator)
}

pub(super) fn randarray(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 0, 5)?;
    let get = |i: usize, default: f64| -> Res<f64> {
        match args.get(i) {
            None | Some(Arg::Missing) => Ok(default),
            Some(a) => num_arg(ctx, a),
        }
    };
    let rows = get(0, 1.0)?.trunc();
    let cols = get(1, 1.0)?.trunc();
    let min = get(2, 0.0)?;
    let max = get(3, 1.0)?;
    let integer = match optional(args, 4) {
        Some(a) => super::bool_arg(ctx, a)?,
        None => false,
    };
    if rows < 0.0 || cols < 0.0 || min > max {
        return Err(ErrorKind::Value);
    }
    if integer && (min.fract() != 0.0 || max.fract() != 0.0) {
        return Err(ErrorKind::Value);
    }
    if rows == 0.0 || cols == 0.0 {
        return Err(ErrorKind::Calc);
    }
    if rows > 1_048_576.0 || cols > 16_384.0 * 4.0 || rows * cols > 1.0e7 {
        return Err(ErrorKind::Value);
    }
    let (rows, cols) = (rows as usize, cols as usize);
    let mut out = Vec::with_capacity(rows);
    for _ in 0..rows {
        let mut row = Vec::with_capacity(cols);
        for _ in 0..cols {
            let r = next_random();
            row.push(Value::Number(if integer {
                min + (r * (max - min + 1.0)).floor()
            } else {
                min + r * (max - min)
            }));
        }
        out.push(row);
    }
    Ok(Value::Array(out))
}
