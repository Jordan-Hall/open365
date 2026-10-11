//! Statistical distributions: normal, lognormal, t, chi-square, F, beta, gamma and their legacy names.
//!
//! The probabilities are evaluated in double-double precision (`distribdd`) and rounded once, so they agree with
//! the exact value to the last bit wherever Excel does; the places where Excel's own result is a few ulps off the
//! exact one are listed in `fixtures/calc/known-differences.toml`. Parameters are coerced the way Excel does
//! (text numbers and booleans are numbers, a blank is 0, integer parameters are truncated) and every argument is
//! coerced before the domain checks, so an error value in a later argument wins over `#NUM!`.
#![allow(
    clippy::approx_constant,
    clippy::excessive_precision,
    clippy::manual_range_contains,
    clippy::needless_range_loop
)]

use super::distribdd::{self as dd, Dd};
use super::{Res, arity, bool_arg, num_arg, number};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value};

pub(super) const NUM: ErrorKind = ErrorKind::Num;

// ---- argument helpers ----------------------------------------------------------------------------

/// `N` numeric arguments (the argument count must be exactly `N`).
pub(super) fn nums<const N: usize>(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<[f64; N]> {
    arity(args, N, N)?;
    let mut out = [0.0; N];
    for (slot, a) in out.iter_mut().zip(args) {
        *slot = num_arg(ctx, a)?;
    }
    Ok(out)
}

/// `N` numbers followed by a boolean.
pub(super) fn nums_bool<const N: usize>(
    ctx: &EvalContext<'_>,
    args: &[Arg],
) -> Res<([f64; N], bool)> {
    arity(args, N + 1, N + 1)?;
    let mut out = [0.0; N];
    for (slot, a) in out.iter_mut().zip(args) {
        *slot = num_arg(ctx, a)?;
    }
    Ok((out, bool_arg(ctx, &args[N])?))
}

/// The continuous distributions (not the discrete ones) return 0 for a result below the smallest normal double (2.2250738585072014E-308) is 0
/// (measured: `NORM.S.DIST(-37.51)` is 3.16E-308 and `NORM.S.DIST(-37.52)` is 0).
pub(super) fn flush(x: f64) -> f64 {
    if x != 0.0 && x.abs() < f64::MIN_POSITIVE {
        0.0
    } else {
        x
    }
}

pub(super) fn out(x: f64) -> Res<Value> {
    number(flush(x))
}

fn dd_out(d: Dd) -> Res<Value> {
    number(flush(d.to_f64()))
}

fn some<T>(o: Option<T>) -> Res<T> {
    o.ok_or(NUM)
}

fn truncated(x: f64) -> f64 {
    x.trunc()
}

// ---- root finding --------------------------------------------------------------------------------

fn next_up(x: f64) -> f64 {
    x.next_up()
}

fn bisect(lo: f64, hi: f64) -> f64 {
    if lo == 0.0 {
        if hi > 1e-300 { hi * 0.015625 } else { 0.5 * hi }
    } else if lo > 0.0 && hi / lo > 4.0 {
        lo.sqrt() * hi.sqrt()
    } else {
        0.5 * lo + 0.5 * hi
    }
}

/// The double nearest to the root of the increasing function `g` on `[lo, hi]` (`g(lo) <= 0 <= g(hi)`).
/// `g` is evaluated in double-double precision; false position with Illinois damping, bisection as the guard.
pub(super) fn solve(mut lo: f64, mut hi: f64, g: &dyn Fn(f64) -> Option<Dd>) -> Option<f64> {
    let mut glo = g(lo)?;
    let mut ghi = g(hi)?;
    if glo.hi >= 0.0 {
        return Some(lo);
    }
    if ghi.hi <= 0.0 {
        return Some(hi);
    }
    let mut side = 0;
    let mut glo_f = glo.hi;
    let mut ghi_f = ghi.hi;
    for iter in 0..600 {
        if next_up(lo) >= hi {
            break;
        }
        let mut x = if iter % 4 == 3 {
            bisect(lo, hi)
        } else {
            lo + (hi - lo) * (glo_f / (glo_f - ghi_f))
        };
        if !(x > lo && x < hi) {
            x = bisect(lo, hi);
            if !(x > lo && x < hi) {
                break;
            }
        }
        let gx = g(x)?;
        if gx.hi == 0.0 {
            return Some(x);
        }
        if gx.hi < 0.0 {
            lo = x;
            glo = gx;
            glo_f = gx.hi;
            if side == -1 {
                ghi_f *= 0.5;
            }
            side = -1;
        } else {
            hi = x;
            ghi = gx;
            ghi_f = gx.hi;
            if side == 1 {
                glo_f *= 0.5;
            }
            side = 1;
        }
    }
    // lo and hi are adjacent doubles: take the nearer one to the interpolated root.
    let w = glo.neg().div(ghi.sub(glo));
    let r = Dd::from(lo).add(Dd::from(hi).sub(Dd::from(lo)).mul(w));
    let mid = Dd::from(lo).add(Dd::from(hi)).scale(0.5);
    Some(if r.lt(mid) { lo } else { hi })
}

/// Root of the increasing `g` on `[0, inf)` with `g(0) < 0`: brackets by growing `start` by 16.
pub(super) fn solve_pos(start: f64, g: &dyn Fn(f64) -> Option<Dd>) -> Option<f64> {
    let mut lo = 0.0;
    let mut hi = start;
    while g(hi)?.hi < 0.0 {
        lo = hi;
        hi *= 16.0;
        if !hi.is_finite() {
            return None;
        }
    }
    solve(lo, hi, g)
}

// ---- normal --------------------------------------------------------------------------------------

/// Starting value for the standard normal quantile of the lower-tail probability `q` (about 1e-9 relative).
fn acklam(q: f64) -> f64 {
    const A: [f64; 6] = [
        -3.969683028665376e+01,
        2.209460984245205e+02,
        -2.759285104469687e+02,
        1.383577518672690e+02,
        -3.066479806614716e+01,
        2.506628277459239e+00,
    ];
    const B: [f64; 5] = [
        -5.447609879822406e+01,
        1.615858368580409e+02,
        -1.556989798598866e+02,
        6.680131188771972e+01,
        -1.328068155288572e+01,
    ];
    const C: [f64; 6] = [
        -7.784894002430293e-03,
        -3.223964580411365e-01,
        -2.400758277161838e+00,
        -2.549732539343734e+00,
        4.374664141464968e+00,
        2.938163982698783e+00,
    ];
    const D: [f64; 4] = [
        7.784695709041462e-03,
        3.224671290700398e-01,
        2.445134137142996e+00,
        3.754408661907416e+00,
    ];
    if q < 0.02425 {
        let t = (-2.0 * q.ln()).sqrt();
        (((((C[0] * t + C[1]) * t + C[2]) * t + C[3]) * t + C[4]) * t + C[5])
            / ((((D[0] * t + D[1]) * t + D[2]) * t + D[3]) * t + 1.0)
    } else {
        let r = q - 0.5;
        let s = r * r;
        (((((A[0] * s + A[1]) * s + A[2]) * s + A[3]) * s + A[4]) * s + A[5]) * r
            / (((((B[0] * s + B[1]) * s + B[2]) * s + B[3]) * s + B[4]) * s + 1.0)
    }
}

/// Standard normal quantile of `p` in `(0, 1)`, to double-double precision.
pub(super) fn norm_s_inv_dd(p: f64) -> Dd {
    if p == 0.5 {
        return Dd::from(0.0);
    }
    let lower = p < 0.5;
    let q = if lower {
        Dd::from(p)
    } else {
        Dd::from(1.0).sub_f(p)
    };
    let mut x = Dd::from(acklam(q.hi));
    for _ in 0..5 {
        let (cdf, _) = dd::norm_cdf(x);
        let f = cdf.sub(q);
        let d = dd::norm_pdf(x);
        if d.hi == 0.0 {
            break;
        }
        let u = f.div(d);
        let step = u.div(Dd::from(1.0).add(x.mul(u).scale(0.5)));
        x = x.sub(step);
        if step.hi.abs() <= x.hi.abs() * 1e-30 {
            break;
        }
    }
    if lower { x } else { x.neg() }
}

pub(super) fn norm_s_inv_f(p: f64) -> Res<f64> {
    if !(p > 0.0 && p < 1.0) {
        return Err(NUM);
    }
    if let Some(z) = super::distrib_normq::tail_quantile(p) {
        return Ok(z);
    }
    Ok(norm_s_inv_dd(p).to_f64())
}

fn norm_s_dist_f(z: f64, cum: bool) -> Res<f64> {
    if cum {
        Ok(dd::norm_cdf(Dd::from(z)).0.to_f64())
    } else {
        Ok(dd::norm_pdf(Dd::from(z)).to_f64())
    }
}

pub(super) fn norm_s_dist(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let ([z], cum) = nums_bool::<1>(ctx, args)?;
    out(norm_s_dist_f(z, cum)?)
}

pub(super) fn normsdist(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [z] = nums::<1>(ctx, args)?;
    out(norm_s_dist_f(z, true)?)
}

pub(super) fn norm_s_inv(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [p] = nums::<1>(ctx, args)?;
    out(norm_s_inv_f(p)?)
}

pub(super) fn phi(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [z] = nums::<1>(ctx, args)?;
    out(norm_s_dist_f(z, false)?)
}

pub(super) fn gauss(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [z] = nums::<1>(ctx, args)?;
    // Phi(z) - 1/2 without losing the small differences near zero.
    dd_out(dd::norm_cdf(Dd::from(z)).0.sub_f(0.5))
}

pub(super) fn norm_dist(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let ([x, m, s], cum) = nums_bool::<3>(ctx, args)?;
    if s <= 0.0 {
        return Err(NUM);
    }
    let z = (x - m) / s;
    if cum {
        out(norm_s_dist_f(z, true)?)
    } else {
        dd_out(dd::norm_pdf(Dd::from(z)).div_f(s))
    }
}

pub(super) fn normdist(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    norm_dist(ctx, args)
}

pub(super) fn norm_inv(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [p, m, s] = nums::<3>(ctx, args)?;
    if !(p > 0.0 && p < 1.0) || s <= 0.0 {
        return Err(NUM);
    }
    out(m + s * norm_s_inv_f(p)?)
}

pub(super) fn standardize(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [x, m, s] = nums::<3>(ctx, args)?;
    if s <= 0.0 {
        return Err(NUM);
    }
    out((x - m) / s)
}

fn lognorm_z(x: f64, m: f64, s: f64) -> Res<Dd> {
    if x <= 0.0 || s <= 0.0 {
        return Err(NUM);
    }
    Ok(dd::ln_p(Dd::from(x)).sub_f(m).div_f(s))
}

pub(super) fn lognorm_dist(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let ([x, m, s], cum) = nums_bool::<3>(ctx, args)?;
    let z = lognorm_z(x, m, s)?;
    if cum {
        dd_out(dd::norm_cdf(z).0)
    } else {
        dd_out(dd::norm_pdf(z).div_f(x).div_f(s))
    }
}

pub(super) fn lognormdist(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [x, m, s] = nums::<3>(ctx, args)?;
    let z = lognorm_z(x, m, s)?;
    dd_out(dd::norm_cdf(z).0)
}

pub(super) fn lognorm_inv(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [p, m, s] = nums::<3>(ctx, args)?;
    if !(p > 0.0 && p < 1.0) || s <= 0.0 {
        return Err(NUM);
    }
    let z = norm_s_inv_dd(p);
    dd_out(dd::exp_sat(z.mul_f(s).add_f(m)))
}

// ---- gamma family --------------------------------------------------------------------------------

/// `Gamma(x)` for any real `x` except the non-positive integers.
pub(super) fn gamma_fn(x: f64) -> Res<f64> {
    if x == 0.0 || (x < 0.0 && x == x.floor()) || x > 171.7 {
        return Err(NUM);
    }
    if x > 0.0 {
        let l = dd::ln_gamma(Dd::from(x));
        let g = dd::exp_f64(l);
        return if g.is_finite() { Ok(g) } else { Err(NUM) };
    }
    // reflection: Gamma(x) = pi / (sin(pi x) Gamma(1 - x))
    let s = dd::sin_pi(x);
    let l = dd::ln_gamma(Dd::from(1.0).sub_f(x));
    let m = dd::exp_f64(dd::LN_PI.sub(l).sub(dd::ln_p(s.abs())));
    if !m.is_finite() {
        return Err(NUM);
    }
    Ok(if s.hi < 0.0 { -m } else { m })
}

pub(super) fn gamma(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [x] = nums::<1>(ctx, args)?;
    out(gamma_fn(x)?)
}

pub(super) fn gammaln(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [x] = nums::<1>(ctx, args)?;
    if x <= 0.0 {
        return Err(NUM);
    }
    dd_out(dd::ln_gamma(Dd::from(x)))
}

fn gamma_cdf(a: f64, x: f64) -> Option<(Dd, Dd)> {
    match dd::gamma_pq(Dd::from(a), Dd::from(x)) {
        Some(v) => Some(v),
        // Far in the upper tail (`GAMMA.DIST(1E+300,2,1.5,TRUE)`): Excel returns the limit, 1.
        None if x > a => Some((Dd::from(1.0), Dd::from(0.0))),
        None => None,
    }
}

/// `ln` of the gamma density (shape `a`, scale 1) at `x > 0`.
fn gamma_ln_pdf(a: f64, x: f64) -> Dd {
    Dd::from(a)
        .sub_f(1.0)
        .mul(dd::ln_p(Dd::from(x)))
        .sub(Dd::from(x))
        .sub(dd::ln_gamma(Dd::from(a)))
}

fn gamma_dist_f(x: f64, a: f64, b: f64, cum: bool) -> Res<f64> {
    if x < 0.0 || a <= 0.0 || b <= 0.0 {
        return Err(NUM);
    }
    let t = x / b;
    if cum {
        return Ok(some(gamma_cdf(a, t))?.0.to_f64());
    }
    if t == 0.0 {
        return if a < 1.0 {
            Err(NUM)
        } else if a == 1.0 {
            Ok(1.0 / b)
        } else {
            Ok(0.0)
        };
    }
    Ok(dd::exp_sat(gamma_ln_pdf(a, t)).div_f(b).to_f64())
}

pub(super) fn gamma_dist(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let ([x, a, b], cum) = nums_bool::<3>(ctx, args)?;
    // Unlike CHISQ.DIST (df = 2), GAMMA.DIST's density at 0 is #NUM! for a shape of 1 or less.
    if !cum && x == 0.0 && a <= 1.0 && b > 0.0 {
        return Err(NUM);
    }
    out(gamma_dist_f(x, a, b, cum)?)
}

pub(super) fn gamma_inv(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [p, a, b] = nums::<3>(ctx, args)?;
    if !(p >= 0.0 && p < 1.0) || a <= 0.0 || b <= 0.0 {
        return Err(NUM);
    }
    if p == 0.0 {
        return out(0.0);
    }
    let x = gamma_quantile(p, a, false)?;
    out(x * b)
}

/// Quantile of the gamma distribution (scale 1) for the left (`right == false`) or right tail probability `p`.
fn gamma_quantile(p: f64, a: f64, right: bool) -> Res<f64> {
    let pd = Dd::from(p);
    let g = |x: f64| -> Option<Dd> {
        let (lo, hi) = gamma_cdf(a, x)?;
        Some(if right { pd.sub(hi) } else { lo.sub(pd) })
    };
    some(solve_pos(a.max(1.0), &g))
}

/// Largest degrees of freedom Excel accepts (after truncation): `T.DIST(1.5,10000000000,TRUE)` is a number,
/// `T.DIST(1.5,10000000001,TRUE)` is #NUM!. `T.DIST(...,FALSE)` and `CHISQ.INV.RT` have no upper limit.
const MAX_DF: f64 = 10_000_000_000.0;

// ---- chi-square ----------------------------------------------------------------------------------

fn chi_df(df: f64) -> Res<f64> {
    let df = truncated(df);
    if !(1.0..=MAX_DF).contains(&df) {
        return Err(NUM);
    }
    Ok(df)
}

pub(super) fn chisq_dist(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let ([x, df], cum) = nums_bool::<2>(ctx, args)?;
    let df = chi_df(df)?;
    if x < 0.0 {
        return Err(NUM);
    }
    out(gamma_dist_f(x, df / 2.0, 2.0, cum)?)
}

pub(super) fn chisq_dist_rt(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [x, df] = nums::<2>(ctx, args)?;
    let df = chi_df(df)?;
    if x < 0.0 {
        return Err(NUM);
    }
    dd_out(some(gamma_cdf(df / 2.0, x / 2.0))?.1)
}

pub(super) fn chisq_inv(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [p, df] = nums::<2>(ctx, args)?;
    let df = chi_df(df)?;
    if !(p >= 0.0 && p < 1.0) {
        return Err(NUM);
    }
    if p == 0.0 {
        return out(0.0);
    }
    out(gamma_quantile(p, df / 2.0, false)? * 2.0)
}

pub(super) fn chisq_inv_rt(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [p, df] = nums::<2>(ctx, args)?;
    let df = chi_df(df)?;
    if !(p > 0.0 && p <= 1.0) {
        return Err(NUM);
    }
    if p == 1.0 {
        return out(0.0);
    }
    out(gamma_quantile(p, df / 2.0, true)? * 2.0)
}

// ---- beta ----------------------------------------------------------------------------------------

fn beta_cdf(a: f64, b: f64, x: Dd, y: Dd) -> Option<(Dd, Dd)> {
    dd::beta_pq(Dd::from(a), Dd::from(b), x, y)
}

/// `ln` of the beta function.
fn ln_beta(a: Dd, b: Dd) -> Dd {
    dd::ln_gamma(a)
        .add(dd::ln_gamma(b))
        .sub(dd::ln_gamma(a.add(b)))
}

/// Beta density at `x` in `(0, 1)` with `y = 1 - x`.
fn beta_pdf(a: f64, b: f64, x: Dd, y: Dd) -> Dd {
    let (ad, bd) = (Dd::from(a), Dd::from(b));
    let l = ad
        .sub_f(1.0)
        .mul(dd::ln_p(x))
        .add(bd.sub_f(1.0).mul(dd::ln_p(y)))
        .sub(ln_beta(ad, bd));
    dd::exp_sat(l)
}

fn beta_dist_f(x: f64, a: f64, b: f64, cum: bool, lo: f64, hi: f64) -> Res<f64> {
    if a <= 0.0 || b <= 0.0 || x < lo || x > hi || lo == hi {
        return Err(NUM);
    }
    let (xs, ys, width) = if lo == 0.0 && hi == 1.0 {
        (Dd::from(x), Dd::from(1.0).sub_f(x), 1.0)
    } else {
        let w = dd::sum(hi, -lo);
        (dd::sum(x, -lo).div(w), dd::sum(hi, -x).div(w), hi - lo)
    };
    if cum {
        return Ok(some(beta_cdf(a, b, xs, ys))?.0.to_f64());
    }
    if xs.hi <= 0.0 || ys.hi <= 0.0 {
        // Measured: at an end point the density is #NUM! when the exponent there (a at the lower end, b at the
        // upper end) is at most 1, and 0 above 1 (Excel does not return the finite limit for exponent 1).
        let a_end = if xs.hi <= 0.0 { a } else { b };
        return if a_end <= 1.0 { Err(NUM) } else { Ok(0.0) };
    }
    Ok(beta_pdf(a, b, xs, ys).div_f(width).to_f64())
}

fn opt_num(ctx: &EvalContext<'_>, args: &[Arg], i: usize, default: f64) -> Res<f64> {
    match args.get(i) {
        None | Some(Arg::Missing) => Ok(default),
        Some(a) => num_arg(ctx, a),
    }
}

pub(super) fn beta_dist(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 4, 6)?;
    let x = num_arg(ctx, &args[0])?;
    let a = num_arg(ctx, &args[1])?;
    let b = num_arg(ctx, &args[2])?;
    let cum = bool_arg(ctx, &args[3])?;
    let lo = opt_num(ctx, args, 4, 0.0)?;
    let hi = opt_num(ctx, args, 5, 1.0)?;
    out(beta_dist_f(x, a, b, cum, lo, hi)?)
}

pub(super) fn betadist(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 5)?;
    let x = num_arg(ctx, &args[0])?;
    let a = num_arg(ctx, &args[1])?;
    let b = num_arg(ctx, &args[2])?;
    let lo = opt_num(ctx, args, 3, 0.0)?;
    let hi = opt_num(ctx, args, 4, 1.0)?;
    out(beta_dist_f(x, a, b, true, lo, hi)?)
}

pub(super) fn beta_inv(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 5)?;
    let p = num_arg(ctx, &args[0])?;
    let a = num_arg(ctx, &args[1])?;
    let b = num_arg(ctx, &args[2])?;
    let lo = opt_num(ctx, args, 3, 0.0)?;
    let hi = opt_num(ctx, args, 4, 1.0)?;
    if !(p > 0.0 && p < 1.0) || a <= 0.0 || b <= 0.0 || lo >= hi {
        return Err(NUM);
    }
    let x = beta_quantile(p, a, b)?;
    out(lo + x * (hi - lo))
}

/// Quantile of the standard beta distribution for the left-tail probability `p` in `(0, 1]`.
fn beta_quantile(p: f64, a: f64, b: f64) -> Res<f64> {
    if p == 1.0 {
        return Ok(1.0);
    }
    let pd = Dd::from(p);
    let g = |x: f64| -> Option<Dd> {
        let (lo, _) = beta_cdf(a, b, Dd::from(x), Dd::from(1.0).sub_f(x))?;
        Some(lo.sub(pd))
    };
    some(solve(0.0, 1.0, &g))
}

// ---- t -------------------------------------------------------------------------------------------

/// `(P(T <= t), P(T > t), P(|T| > |t|))` of Student's t with `df` degrees of freedom.
fn t_tails(t: f64, df: f64) -> Option<(Dd, Dd, Dd)> {
    let nu = Dd::from(df);
    let t2 = dd::prod(t, t);
    if !t2.hi.is_finite() {
        let z = Dd::from(0.0);
        let one = Dd::from(1.0);
        return Some(if t > 0.0 { (one, z, z) } else { (z, one, z) });
    }
    let den = nu.add(t2);
    let x = nu.div(den);
    let y = t2.div(den);
    let (two, _) = dd::beta_pq(nu.scale(0.5), Dd::from(0.5), x, y)?;
    let half = two.scale(0.5);
    let rest = Dd::from(1.0).sub(half);
    Some(if t > 0.0 {
        (rest, half, two)
    } else {
        (half, rest, two)
    })
}

fn t_pdf(t: f64, df: f64) -> Dd {
    let nu = Dd::from(df);
    let a = dd::ln_gamma(nu.add_f(1.0).scale(0.5)).sub(dd::ln_gamma(nu.scale(0.5)));
    let b = dd::ln_p(nu.mul(dd::PI)).scale(0.5);
    let c = nu
        .add_f(1.0)
        .scale(0.5)
        .mul(dd::ln_p(Dd::from(1.0).add(dd::prod(t, t).div(nu))));
    dd::exp_sat(a.sub(b).sub(c))
}

/// Excel squares `t`; a square that overflows (|t| > 1.34E154) is #NUM! for every t function.
fn t_square_fits(t: f64) -> Res<()> {
    if (t * t).is_infinite() {
        Err(NUM)
    } else {
        Ok(())
    }
}

fn t_df(df: f64) -> Res<f64> {
    let df = truncated(df);
    if !(1.0..=MAX_DF).contains(&df) {
        return Err(NUM);
    }
    Ok(df)
}

pub(super) fn t_dist(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let ([x, df], cum) = nums_bool::<2>(ctx, args)?;
    if cum {
        let df = t_df(df)?;
        t_square_fits(x)?;
        dd_out(some(t_tails(x, df))?.0)
    } else {
        // The density has no upper limit on the degrees of freedom; 0 (after truncation) is #DIV/0!.
        let df = truncated(df);
        if df < 0.0 {
            return Err(NUM);
        }
        if df == 0.0 {
            return Err(ErrorKind::DivZero);
        }
        t_square_fits(x)?;
        dd_out(t_pdf(x, df))
    }
}

pub(super) fn t_dist_rt(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [x, df] = nums::<2>(ctx, args)?;
    let df = t_df(df)?;
    t_square_fits(x)?;
    dd_out(some(t_tails(x, df))?.1)
}

pub(super) fn t_dist_2t(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [x, df] = nums::<2>(ctx, args)?;
    let df = t_df(df)?;
    if x < 0.0 {
        return Err(NUM);
    }
    t_square_fits(x)?;
    dd_out(some(t_tails(x, df))?.2)
}

pub(super) fn tdist(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [x, df, tails] = nums::<3>(ctx, args)?;
    let df = t_df(df)?;
    let tails = truncated(tails);
    if x < 0.0 || !(tails == 1.0 || tails == 2.0) {
        return Err(NUM);
    }
    t_square_fits(x)?;
    let (_, rt, two) = some(t_tails(x, df))?;
    dd_out(if tails == 1.0 { rt } else { two })
}

/// Positive `s` with two-tail probability `p` for Student's t.
fn t_quantile_2t(p: f64, df: f64) -> Res<f64> {
    if p == 1.0 {
        return Ok(0.0);
    }
    let pd = Dd::from(p);
    let g = |s: f64| -> Option<Dd> { Some(pd.sub(t_tails(s, df)?.2)) };
    some(solve_pos(1.0, &g))
}

pub(super) fn t_inv(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [p, df] = nums::<2>(ctx, args)?;
    let df = t_df(df)?;
    if !(p > 0.0 && p < 1.0) {
        return Err(NUM);
    }
    if p == 0.5 {
        return out(0.0);
    }
    let q = if p < 0.5 { p } else { 1.0 - p };
    // `1 - p` is exact for p in [0.5, 1) in a double.
    let s = t_quantile_2t(2.0 * q, df)?;
    out(if p < 0.5 { -s } else { s })
}

pub(super) fn t_inv_2t_f(p: f64, df: f64) -> Res<f64> {
    let df = t_df(df)?;
    if !(p > 0.0 && p <= 1.0) {
        return Err(NUM);
    }
    t_quantile_2t(p, df)
}

pub(super) fn t_inv_2t(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [p, df] = nums::<2>(ctx, args)?;
    let df = t_df(df)?;
    if !(p > 0.0 && p <= 1.0) {
        return Err(NUM);
    }
    out(t_quantile_2t(p, df)?)
}

// ---- F -------------------------------------------------------------------------------------------

fn f_df(df: f64) -> Res<f64> {
    let df = truncated(df);
    if !(1.0..=MAX_DF).contains(&df) {
        return Err(NUM);
    }
    Ok(df)
}

/// `(P(F <= x), P(F > x))` for `x >= 0`.
fn f_tails(x: f64, d1: f64, d2: f64) -> Option<(Dd, Dd)> {
    let n = dd::prod(d1, x);
    let den = n.add_f(d2);
    let xs = n.div(den);
    let ys = Dd::from(d2).div(den);
    beta_cdf_dd(Dd::from(d1).scale(0.5), Dd::from(d2).scale(0.5), xs, ys)
}

fn beta_cdf_dd(a: Dd, b: Dd, x: Dd, y: Dd) -> Option<(Dd, Dd)> {
    dd::beta_pq(a, b, x, y)
}

fn f_pdf(x: f64, d1: f64, d2: f64) -> Dd {
    // f(x) = (d1/d2)^(d1/2) x^(d1/2 - 1) (1 + d1 x/d2)^(-(d1+d2)/2) / B(d1/2, d2/2)
    let (a, b) = (Dd::from(d1).scale(0.5), Dd::from(d2).scale(0.5));
    let r = Dd::from(d1).div_f(d2);
    let l = a
        .mul(dd::ln_p(r))
        .add(a.sub_f(1.0).mul(dd::ln_p(Dd::from(x))))
        .sub(a.add(b).mul(dd::ln_p(Dd::from(1.0).add(r.mul_f(x)))))
        .sub(ln_beta(a, b));
    dd::exp_sat(l)
}

pub(super) fn f_dist(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let ([x, d1, d2], cum) = nums_bool::<3>(ctx, args)?;
    let (d1, d2) = (f_df(d1)?, f_df(d2)?);
    if x < 0.0 {
        return Err(NUM);
    }
    if cum {
        dd_out(some(f_tails(x, d1, d2))?.0)
    } else if x == 0.0 {
        out(if d1 < 2.0 {
            return Err(NUM);
        } else if d1 == 2.0 {
            1.0
        } else {
            0.0
        })
    } else {
        dd_out(f_pdf(x, d1, d2))
    }
}

pub(super) fn f_dist_rt(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [x, d1, d2] = nums::<3>(ctx, args)?;
    let (d1, d2) = (f_df(d1)?, f_df(d2)?);
    if x < 0.0 {
        return Err(NUM);
    }
    dd_out(some(f_tails(x, d1, d2))?.1)
}

fn f_quantile(p: f64, d1: f64, d2: f64, right: bool) -> Res<f64> {
    let pd = Dd::from(p);
    let g = |x: f64| -> Option<Dd> {
        let (lo, hi) = f_tails(x, d1, d2)?;
        Some(if right { pd.sub(hi) } else { lo.sub(pd) })
    };
    some(solve_pos(1.0, &g))
}

pub(super) fn f_inv(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [p, d1, d2] = nums::<3>(ctx, args)?;
    let (d1, d2) = (f_df(d1)?, f_df(d2)?);
    if !(p >= 0.0 && p < 1.0) {
        return Err(NUM);
    }
    if p == 0.0 {
        return out(0.0);
    }
    out(f_quantile(p, d1, d2, false)?)
}

pub(super) fn f_inv_rt(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [p, d1, d2] = nums::<3>(ctx, args)?;
    let (d1, d2) = (f_df(d1)?, f_df(d2)?);
    if !(p > 0.0 && p <= 1.0) {
        return Err(NUM);
    }
    if p == 1.0 {
        return out(0.0);
    }
    out(f_quantile(p, d1, d2, true)?)
}

// ---- Weibull, exponential ------------------------------------------------------------------------

pub(super) fn weibull_dist(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let ([x, a, b], cum) = nums_bool::<3>(ctx, args)?;
    if x < 0.0 || a <= 0.0 || b <= 0.0 {
        return Err(NUM);
    }
    // (x / b)^a in double-double: exp(a ln(x / b)).
    if x == 0.0 {
        // Measured: the density at 0 is 0 for every shape (`WEIBULL.DIST(0,1,1,FALSE)` is 0, not 1).
        return out(0.0);
    }
    let u = Dd::from(x).div_f(b);
    let w = dd::exp_sat(Dd::from(a).mul(dd::ln_p(u)));
    if cum {
        // 1 - exp(-w)
        let e = dd::exp_sat(w.neg());
        return dd_out(Dd::from(1.0).sub(e));
    }
    let v = dd::exp_sat(Dd::from(a).sub_f(1.0).mul(dd::ln_p(u)));
    dd_out(v.mul(dd::exp_sat(w.neg())).mul_f(a).div_f(b))
}

pub(super) fn expon_dist(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let ([x, l], cum) = nums_bool::<2>(ctx, args)?;
    if x < 0.0 || l <= 0.0 {
        return Err(NUM);
    }
    let t = dd::prod(x, l);
    let e = dd::exp_sat(t.neg());
    if cum {
        dd_out(Dd::from(1.0).sub(e))
    } else {
        dd_out(e.mul_f(l))
    }
}
