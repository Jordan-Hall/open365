//! More statistical functions: spread, shape, correlation and regression helpers.
//!
//! The summation orders and groupings are the ones that reproduce Excel's last digits (measured, see
//! `docs/testing/calc-conformance.md`): means are `sum / n` on a left-to-right sum, deviations are formed
//! against the mean, and `CORREL` divides the population covariance by the product of the two population
//! standard deviations.

use std::cmp::Ordering;

use super::distribdd::{self as dd, Dd};
use super::math::collect;
use super::{Res, arity, num_arg, number};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value, parse_number_text_in};

/// Left-to-right sum.
fn ssum(it: impl Iterator<Item = f64>) -> f64 {
    let mut s = 0.0;
    for x in it {
        s += x;
    }
    s
}

pub(super) fn takes_arrays(name: &str, index: usize) -> bool {
    const TWO: &[&str] = &[
        "CORREL",
        "PEARSON",
        "RSQ",
        "SLOPE",
        "INTERCEPT",
        "STEYX",
        "COVAR",
        "COVARIANCE.P",
        "COVARIANCE.S",
    ];
    const ONE: &[&str] = &[
        "AVEDEV", "DEVSQ", "GEOMEAN", "HARMEAN", "KURT", "SKEW", "SKEW.P", "TRIMMEAN", "STDEVA",
        "STDEVPA", "VARA", "VARPA",
    ];
    let is = |list: &[&str]| list.iter().any(|f| f.eq_ignore_ascii_case(name));
    if name.eq_ignore_ascii_case("FORECAST") || name.eq_ignore_ascii_case("FORECAST.LINEAR") {
        return index >= 1;
    }
    is(TWO) || (is(ONE) && (index == 0 || !name.eq_ignore_ascii_case("TRIMMEAN")))
}

// ---- one-array functions -------------------------------------------------------------------

pub(super) fn avedev(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let x = collect(ctx, args)?;
    if x.is_empty() {
        return Err(ErrorKind::Num);
    }
    let n = x.len() as f64;
    let m = ssum(x.iter().copied()) / n;
    number(ssum(x.iter().map(|a| (a - m).abs())) / n)
}

pub(super) fn devsq(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let x = collect(ctx, args)?;
    if x.is_empty() {
        return Err(ErrorKind::Num);
    }
    let m = ssum(x.iter().copied()) / x.len() as f64;
    number(ssum(x.iter().map(|a| (a - m) * (a - m))))
}

pub(super) fn geomean(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let x = collect(ctx, args)?;
    if x.is_empty() || x.iter().any(|a| *a <= 0.0) {
        return Err(ErrorKind::Num);
    }
    let mut s = Dd::from(0.0);
    for a in &x {
        s = s.add(dd::ln(Dd::from(*a)));
    }
    number(dd::exp_f64(s.div_f(x.len() as f64)))
}

pub(super) fn harmean(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let x = collect(ctx, args)?;
    if x.is_empty() {
        return Err(ErrorKind::NA);
    }
    if x.iter().any(|a| *a <= 0.0) {
        return Err(ErrorKind::Num);
    }
    let n = x.len() as f64;
    number(1.0 / (ssum(x.iter().map(|a| 1.0 / a)) / n))
}

/// `(n, deviations, sum of squared deviations)`.
fn moments(x: &[f64]) -> (f64, Vec<f64>, f64) {
    let n = x.len() as f64;
    let m = ssum(x.iter().copied()) / n;
    let d: Vec<f64> = x.iter().map(|a| a - m).collect();
    let s2 = ssum(d.iter().map(|a| a * a));
    (n, d, s2)
}

pub(super) fn skew(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let x = collect(ctx, args)?;
    if x.len() < 3 {
        return Err(ErrorKind::DivZero);
    }
    let (n, d, s2) = moments(&x);
    let s = (s2 / (n - 1.0)).sqrt();
    if s == 0.0 {
        return Err(ErrorKind::DivZero);
    }
    let t = ssum(d.iter().map(|a| (a / s) * (a / s) * (a / s)));
    number(n / ((n - 1.0) * (n - 2.0)) * t)
}

pub(super) fn skew_p(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let x = collect(ctx, args)?;
    if x.len() < 3 {
        return Err(ErrorKind::DivZero);
    }
    let (n, d, s2) = moments(&x);
    let s = (s2 / n).sqrt();
    if s == 0.0 {
        return Err(ErrorKind::DivZero);
    }
    number(ssum(d.iter().map(|a| (a / s) * (a / s) * (a / s))) / n)
}

pub(super) fn kurt(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let x = collect(ctx, args)?;
    if x.len() < 4 {
        return Err(ErrorKind::DivZero);
    }
    let (n, d, s2) = moments(&x);
    let s = (s2 / (n - 1.0)).sqrt();
    if s == 0.0 {
        return Err(ErrorKind::DivZero);
    }
    let t = ssum(d.iter().map(|a| ((a / s) * (a / s)) * ((a / s) * (a / s))));
    number(
        (n * (n + 1.0) / ((n - 1.0) * (n - 2.0) * (n - 3.0))) * t
            - (3.0 * (n - 1.0) * (n - 1.0)) / ((n - 2.0) * (n - 3.0)),
    )
}

pub(super) fn trimmean(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let x = collect(ctx, &args[..1])?;
    let p = num_arg(ctx, &args[1])?;
    if x.is_empty() || !(0.0..1.0).contains(&p) {
        return Err(ErrorKind::Num);
    }
    let n = x.len();
    let k = (n as f64 * p / 2.0).floor() as usize;
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&a, &b| x[a].partial_cmp(&x[b]).unwrap_or(Ordering::Equal));
    let mut drop = vec![false; n];
    for &i in idx.iter().take(k).chain(idx.iter().skip(n - k)) {
        drop[i] = true;
    }
    let kept: Vec<f64> = (0..n).filter(|&i| !drop[i]).map(|i| x[i]).collect();
    number(ssum(kept.iter().copied()) / kept.len() as f64)
}

/// The values of the `...A` functions: text in a reference counts as 0 and a boolean as 1 or 0, an array constant
/// contributes only its numbers, direct arguments are coerced.
fn a_nums(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Vec<f64>> {
    arity(args, 1, usize::MAX)?;
    let mut out = Vec::new();
    for arg in args {
        match arg {
            Arg::Missing => out.push(0.0),
            Arg::Value(v) => out.push(match v {
                Value::Number(n) => *n,
                Value::Bool(b) => f64::from(u8::from(*b)),
                Value::Empty => 0.0,
                Value::Text(t) => parse_number_text_in(t, ctx.date1904).ok_or(ErrorKind::Value)?,
                Value::Error(e) => return Err(*e),
                Value::Array(_) => return Err(ErrorKind::Value),
            }),
            Arg::Array(_) => {
                for v in ctx.cells(arg) {
                    match v {
                        Value::Number(n) => out.push(n),
                        Value::Error(e) => return Err(e),
                        _ => {}
                    }
                }
            }
            _ => {
                for v in ctx.cells(arg) {
                    match v {
                        Value::Number(n) => out.push(n),
                        Value::Bool(b) => out.push(f64::from(u8::from(b))),
                        Value::Text(_) => out.push(0.0),
                        Value::Error(e) => return Err(e),
                        Value::Empty | Value::Array(_) => {}
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Variance as `VARA`/`VARPA` form it: `(Σx² - (Σx)²/n) / (n-1)` and `(nΣx² - (Σx)²) / n²`, with the two-pass form
/// where the subtraction cancels more than a thousandth of `Σx² / divisor` (measured on 100 random data sets).
fn a_variance(ctx: &EvalContext<'_>, args: &[Arg], sample: bool, root: bool) -> Res<Value> {
    let x = a_nums(ctx, args)?;
    let n = x.len() as f64;
    if x.len() < usize::from(sample) + 1 {
        return Err(ErrorKind::DivZero);
    }
    let s1 = ssum(x.iter().copied());
    let s2 = ssum(x.iter().map(|a| a * a));
    if !s2.is_finite() {
        return Err(ErrorKind::Num);
    }
    let divisor = if sample { n - 1.0 } else { n };
    let mut v = if sample {
        (s2 - s1 * s1 / n) / divisor
    } else {
        (n * s2 - s1 * s1) / (n * n)
    };
    if v < s2 * 1e-3 / divisor {
        let m = s1 / n;
        v = ssum(x.iter().map(|a| (a - m) * (a - m))) / divisor;
    }
    number(if root { v.max(0.0).sqrt() } else { v.max(0.0) })
}

pub(super) fn stdeva(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    a_variance(ctx, args, true, true)
}

pub(super) fn stdevpa(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    a_variance(ctx, args, false, true)
}

pub(super) fn vara(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    a_variance(ctx, args, true, false)
}

pub(super) fn varpa(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    a_variance(ctx, args, false, false)
}

// ---- two-array functions -------------------------------------------------------------------

/// The numeric pairs of two equally long arrays; a pair with a non-number on either side is skipped and an
/// error value in either array propagates. Different lengths are `#N/A`.
fn pairs(ctx: &EvalContext<'_>, a: &Arg, b: &Arg) -> Res<Vec<(f64, f64)>> {
    let va: Vec<Value> = ctx.cells(a).collect();
    let vb: Vec<Value> = ctx.cells(b).collect();
    if va.len() != vb.len() {
        return Err(ErrorKind::NA);
    }
    let mut out = Vec::with_capacity(va.len());
    for (p, q) in va.iter().zip(&vb) {
        if let Value::Error(e) = p {
            return Err(*e);
        }
        if let Value::Error(e) = q {
            return Err(*e);
        }
        if let (Value::Number(x), Value::Number(y)) = (p, q) {
            out.push((*x, *y));
        }
    }
    Ok(out)
}

/// Sums of a pair list: `n`, the two means and the sums of squared and cross deviations.
pub(super) struct PairSums {
    pub n: f64,
    pub mx: f64,
    pub my: f64,
    pub sxx: f64,
    pub syy: f64,
    pub sxy: f64,
}

pub(super) fn pair_sums(p: &[(f64, f64)]) -> PairSums {
    let n = p.len() as f64;
    let mx = ssum(p.iter().map(|q| q.0)) / n;
    let my = ssum(p.iter().map(|q| q.1)) / n;
    PairSums {
        n,
        mx,
        my,
        sxx: ssum(p.iter().map(|q| (q.0 - mx) * (q.0 - mx))),
        syy: ssum(p.iter().map(|q| (q.1 - my) * (q.1 - my))),
        sxy: ssum(p.iter().map(|q| (q.0 - mx) * (q.1 - my))),
    }
}

fn two(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<PairSums> {
    arity(args, 2, 2)?;
    let p = pairs(ctx, &args[0], &args[1])?;
    if p.is_empty() {
        return Err(ErrorKind::DivZero);
    }
    Ok(pair_sums(&p))
}

/// Pair sums for the regression functions `f(known_y, known_x)`: `x` is the second argument.
fn two_yx(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<PairSums> {
    arity(args, 2, 2)?;
    let p = pairs(ctx, &args[1], &args[0])?;
    if p.is_empty() {
        return Err(ErrorKind::DivZero);
    }
    Ok(pair_sums(&p))
}

pub(super) fn correl(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let s = two(ctx, args)?;
    if s.sxx == 0.0 || s.syy == 0.0 {
        return Err(ErrorKind::DivZero);
    }
    number((s.sxy / s.n) / ((s.sxx / s.n).sqrt() * (s.syy / s.n).sqrt()))
}

pub(super) fn rsq(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let s = two(ctx, args)?;
    if s.sxx == 0.0 || s.syy == 0.0 {
        return Err(ErrorKind::DivZero);
    }
    let r = s.sxy / (s.sxx * s.syy).sqrt();
    number(r * r)
}

pub(super) fn slope(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let s = two_yx(ctx, args)?;
    if s.sxx == 0.0 {
        return Err(ErrorKind::DivZero);
    }
    number(s.sxy / s.sxx)
}

pub(super) fn intercept(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let s = two_yx(ctx, args)?;
    if s.sxx == 0.0 {
        return Err(ErrorKind::DivZero);
    }
    number(s.my - s.sxy / s.sxx * s.mx)
}

pub(super) fn steyx(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let s = two_yx(ctx, args)?;
    if s.n < 3.0 || s.sxx == 0.0 {
        return Err(ErrorKind::DivZero);
    }
    let v = (s.syy - s.sxy * s.sxy / s.sxx) / (s.n - 2.0);
    number(v.max(0.0).sqrt())
}

pub(super) fn covar(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let s = two(ctx, args)?;
    number(s.sxy / s.n)
}

pub(super) fn covariance_s(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let s = two(ctx, args)?;
    if s.n < 2.0 {
        return Err(ErrorKind::DivZero);
    }
    number(s.sxy / (s.n - 1.0))
}

pub(super) fn forecast(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 3)?;
    let x = num_arg(ctx, &args[0])?;
    let s = two_yx(ctx, &args[1..])?;
    if s.sxx == 0.0 {
        return Err(ErrorKind::DivZero);
    }
    let b = s.sxy / s.sxx;
    number((s.my - b * s.mx) + b * x)
}
