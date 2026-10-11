//! `ERF`, `ERF.PRECISE`, `ERFC` and `ERFC.PRECISE`, modelled on what Excel returns (measured, not copied).
//!
//! Excel's `ERFC(x)` is `exp(-fl(x*x)) * R(x)` with a correctly rounded `exp` of the *rounded* square and an
//! approximation `R(x) = exp(x^2) erfc(x)` that is a few ulp off the exact value. The rounded square matters: for
//! `x > 4` it moves the result by 2 to 250 ulp, so using it (with the exact `R`) lifts the share of bit-identical
//! results at `x` in `[6, 27]` from 0.5% to 22% and keeps 99.9% within 8 ulp. The approximation itself (segments
//! near 0.5, 2.25, 5.25 and 8, steps near 8.75, 9.05 and 9.65, an error wave of about +-3E-16) is not
//! reproduced; no published coefficient set matches (`docs/testing/calc-coverage.md`, "Follow-up 4").
//! Other measured rules: `ERF(x)` for `|x| < 0.5` is about 0.9 * 2^-53 above the exact value in relative terms
//! (54% bit-identical instead of 40%), `ERFC(x)` for `0 <= x < 0.5` is `1 - ERF(x)` in doubles, `ERFC(x)` for
//! `x < 0` is `2 - ERFC(-x)`, and `ERFC(x)` is 0 from `x = 26.543` on (Cody's XBIG).

use super::distrib::flush;
use super::distribdd::{self as dd, Dd};
use super::{Res, arity, number, scalar};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value};

/// `ERFC` is 0 from here on, although the exact value is still 1E-308 (measured between 26.542 and 26.544).
const ERFC_ZERO: f64 = 26.543;
/// Relative excess of Excel's `ERF` below 0.5 over the exact value, fitted on 15,000 points (0.9 * 2^-53).
const ERF_SMALL_BIAS: f64 = 0.9 * 1.110_223_024_625_156_5e-16;

/// A number argument: unlike most functions, the engineering group refuses a boolean, also from a cell (#VALUE!).
fn limit(ctx: &EvalContext<'_>, arg: &Arg) -> Res<f64> {
    let v = scalar(ctx, arg)?;
    if matches!(v, Value::Bool(_)) {
        return Err(ErrorKind::Value);
    }
    v.to_number_in(ctx.date1904)
}

fn erf_abs(a: f64) -> f64 {
    if a < 0.5 {
        let e = dd::erf_small(Dd::from(a));
        e.add(e.mul_f(ERF_SMALL_BIAS)).to_f64()
    } else {
        Dd::from(1.0).sub(dd::erfc(Dd::from(a))).to_f64()
    }
}

/// `erf(x)` as Excel returns it.
pub(super) fn erf(x: f64) -> f64 {
    let v = erf_abs(x.abs());
    if x < 0.0 { -v } else { v }
}

/// `erfc(x)` as Excel returns it.
pub(super) fn erfc(x: f64) -> f64 {
    if x < 0.0 {
        Dd::from(2.0).sub(dd::erfc(Dd::from(-x))).to_f64()
    } else if x < 0.5 {
        1.0 - erf_abs(x)
    } else if x >= ERFC_ZERO {
        0.0
    } else {
        dd::exp_sat(Dd::from(x * x).neg())
            .mul(dd::erfcx(Dd::from(x)))
            .to_f64()
    }
}

pub(super) fn erf_fn(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 2)?;
    let lower = limit(ctx, &args[0])?;
    let v = match args.get(1) {
        None => erf(lower),
        Some(upper) => erf(limit(ctx, upper)?) - erf(lower),
    };
    number(flush(v))
}

pub(super) fn erf_precise(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    number(flush(erf(limit(ctx, &args[0])?)))
}

pub(super) fn erfc_fn(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    number(flush(erfc(limit(ctx, &args[0])?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules() {
        assert_eq!(erf(0.0), 0.0);
        assert_eq!(erf(-3.0), -erf(3.0));
        assert_eq!(erfc(0.0), 1.0);
        assert_eq!(erfc(26.543), 0.0);
        assert_eq!(erfc(-30.0), 2.0);
        assert!((erf(1.0) - 0.842_700_792_949_714_9).abs() < 1e-15);
        assert!((erfc(10.0) / 2.088_487_583_762_545e-45 - 1.0).abs() < 1e-14);
    }
}
