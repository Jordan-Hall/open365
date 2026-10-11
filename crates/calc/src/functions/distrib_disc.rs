//! Discrete distributions: binomial, Poisson, negative binomial and hypergeometric.

use super::distrib::{NUM, nums, nums_bool};
use super::distribdd::{self as dd, Dd};
use super::{Res, arity, num_arg, number, optional};
use crate::eval::{Arg, EvalContext};
use crate::value::Value;

/// Unlike the normal and t functions, the discrete distributions keep subnormal results
/// (`BINOM.DIST(0,500,0.77,FALSE)` is 7.31E-320 in Excel).
fn out(x: f64) -> Res<Value> {
    number(x)
}

fn dd_out(d: Dd) -> Res<Value> {
    number(d.to_f64())
}

fn some<T>(o: Option<T>) -> Res<T> {
    o.ok_or(NUM)
}

/// `ln Gamma(n + 1)`.
fn ln_fact(n: f64) -> Dd {
    dd::ln_gamma(Dd::from(n).add_f(1.0))
}

fn ln_choose(n: f64, k: f64) -> Dd {
    ln_fact(n).sub(ln_fact(k)).sub(ln_fact(n - k))
}

// ---- binomial ------------------------------------------------------------------------------------

fn binom_pmf(k: f64, n: f64, p: f64) -> Dd {
    if p == 0.0 {
        return Dd::from(if k == 0.0 { 1.0 } else { 0.0 });
    }
    if p == 1.0 {
        return Dd::from(if k == n { 1.0 } else { 0.0 });
    }
    let q = Dd::from(1.0).sub_f(p);
    let mut l = ln_choose(n, k).add(dd::ln_p(Dd::from(p)).mul_f(k));
    if n > k {
        l = l.add(dd::ln_p(q).mul_f(n - k));
    }
    dd::exp_sat(l)
}

/// `P(X <= k)` and `P(X > k)` for `X ~ Binomial(n, p)`, `0 <= k <= n`.
fn binom_cdf(k: f64, n: f64, p: f64) -> Option<(Dd, Dd)> {
    if k >= n {
        return Some((Dd::from(1.0), Dd::from(0.0)));
    }
    if p == 0.0 {
        return Some((Dd::from(1.0), Dd::from(0.0)));
    }
    if p == 1.0 {
        return Some((Dd::from(0.0), Dd::from(1.0)));
    }
    let x = Dd::from(1.0).sub_f(p);
    dd::beta_pq(Dd::from(n - k), Dd::from(k + 1.0), x, Dd::from(p))
}

pub(super) fn binom_dist(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let ([k, n, p], cum) = nums_bool::<3>(ctx, args)?;
    let (k, n) = (k.trunc(), n.trunc());
    if k < 0.0 || n < k || !(0.0..=1.0).contains(&p) {
        return Err(NUM);
    }
    if cum {
        dd_out(some(binom_cdf(k, n, p))?.0)
    } else {
        dd_out(binom_pmf(k, n, p))
    }
}

/// Excel refuses a trial count (and every hypergeometric argument) of at least 2^31 - 1, and a negative one
/// before truncation.
const COUNT_LIMIT: f64 = 2_147_483_647.0;

pub(super) fn binom_dist_range(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 4)?;
    let n = num_arg(ctx, &args[0])?;
    let p = num_arg(ctx, &args[1])?;
    let s = num_arg(ctx, &args[2])?;
    let s2 = match optional(args, 3) {
        Some(a) => Some(num_arg(ctx, a)?),
        None => None,
    };
    // Negative counts are refused before truncation (`-0.5` is #NUM!).
    if !(0.0..COUNT_LIMIT).contains(&n) || s < 0.0 || s2.is_some_and(|v| v < 0.0) {
        return Err(NUM);
    }
    let (n, s) = (n.trunc(), s.trunc());
    let s2 = s2.map_or(s, f64::trunc);
    if !(0.0..=1.0).contains(&p) || s > n || s2 < s || s2 > n {
        return Err(NUM);
    }
    if p == 0.0 || p == 1.0 {
        let k = if p == 0.0 { 0.0 } else { n };
        return out(if s <= k && k <= s2 { 1.0 } else { 0.0 });
    }
    if s2 - s <= 20_000.0 {
        // Sum the terms with the ratio recurrence, starting at the end with the larger term so that a tail
        // far from the mean does not underflow at the start.
        let r = Dd::from(p).div(Dd::from(1.0).sub_f(p));
        let (first, last) = (binom_pmf(s, n, p), binom_pmf(s2, n, p));
        let mut sum;
        if last.hi > first.hi {
            let mut term = last;
            sum = term;
            let mut i = s2;
            while i > s {
                term = term.div(r).mul_f(i).div_f(n - i + 1.0);
                sum = sum.add(term);
                i -= 1.0;
            }
        } else {
            let mut term = first;
            sum = term;
            let mut i = s;
            while i < s2 {
                term = term.mul(r).mul_f(n - i).div_f(i + 1.0);
                sum = sum.add(term);
                i += 1.0;
            }
        }
        return dd_out(sum);
    }
    let hi = some(binom_cdf(s2, n, p))?.0;
    let lo = if s > 0.0 {
        some(binom_cdf(s - 1.0, n, p))?.0
    } else {
        Dd::from(0.0)
    };
    dd_out(hi.sub(lo))
}

pub(super) fn binom_inv(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [n, p, alpha] = nums::<3>(ctx, args)?;
    // Probabilities of exactly 0 or 1 are refused (unlike BINOM.DIST); a negative count is refused before truncation.
    let open_unit = |v: f64| v > 0.0 && v < 1.0;
    if !(0.0..COUNT_LIMIT).contains(&n) || !open_unit(p) || !open_unit(alpha) {
        return Err(NUM);
    }
    let n = n.trunc();
    let a = Dd::from(alpha);
    // smallest k with P(X <= k) >= alpha
    let (mut lo, mut hi) = (0.0, n);
    while lo < hi {
        let mid = ((lo + hi) / 2.0).floor();
        let (c, _) = some(binom_cdf(mid, n, p))?;
        if c.lt(a) {
            lo = mid + 1.0;
        } else {
            hi = mid;
        }
    }
    out(lo)
}

// ---- Poisson -------------------------------------------------------------------------------------

pub(super) fn poisson_dist(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let ([k, mean], cum) = nums_bool::<2>(ctx, args)?;
    let k = k.trunc();
    if k < 0.0 || mean < 0.0 {
        return Err(NUM);
    }
    if mean == 0.0 {
        return out(if cum || k == 0.0 { 1.0 } else { 0.0 });
    }
    if cum {
        let (_, q) = some(dd::gamma_pq(Dd::from(k).add_f(1.0), Dd::from(mean)))?;
        return dd_out(q);
    }
    let l = dd::ln_p(Dd::from(mean))
        .mul_f(k)
        .sub_f(mean)
        .sub(ln_fact(k));
    dd_out(dd::exp_sat(l))
}

// ---- negative binomial ---------------------------------------------------------------------------

fn negbinom(f: f64, r: f64, p: f64, cum: bool) -> Res<f64> {
    // A negative failure count is refused before truncation; a success probability of 0 or 1 is #NUM! (not the
    // degenerate distribution) and so is a success count of 1E+16 or more once there is a failure to count
    // (`NEGBINOM.DIST(1,1E+15,0.5,FALSE)` is 0, `NEGBINOM.DIST(1,1E+16,0.5,FALSE)` is #NUM!).
    if f < 0.0 || !(p > 0.0 && p < 1.0) {
        return Err(NUM);
    }
    let (f, r) = (f.trunc(), r.trunc());
    if r < 1.0 || (!cum && f >= 1.0 && r >= 1e16) {
        return Err(NUM);
    }
    if cum {
        let q = Dd::from(1.0).sub_f(p);
        let (lo, _) = some(dd::beta_pq(Dd::from(r), Dd::from(f + 1.0), Dd::from(p), q))?;
        return Ok(lo.to_f64());
    }
    let q = Dd::from(1.0).sub_f(p);
    let l = dd::ln_gamma(Dd::from(f).add(Dd::from(r)))
        .sub(ln_fact(f))
        .sub(dd::ln_gamma(Dd::from(r)))
        .add(dd::ln_p(Dd::from(p)).mul_f(r))
        .add(dd::ln_p(q).mul_f(f));
    Ok(dd::exp_sat(l).to_f64())
}

pub(super) fn negbinom_dist(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let ([f, r, p], cum) = nums_bool::<3>(ctx, args)?;
    out(negbinom(f, r, p, cum)?)
}

pub(super) fn negbinomdist(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [f, r, p] = nums::<3>(ctx, args)?;
    out(negbinom(f, r, p, false)?)
}

// ---- hypergeometric ------------------------------------------------------------------------------

fn hypgeom(s: f64, n: f64, big_s: f64, big_n: f64, cum: bool) -> Res<f64> {
    if [s, n, big_s, big_n]
        .iter()
        .any(|v| *v < 0.0 || *v >= COUNT_LIMIT)
    {
        return Err(NUM);
    }
    let (s, n, big_s, big_n) = (s.trunc(), n.trunc(), big_s.trunc(), big_n.trunc());
    if n > big_n || big_s > big_n {
        return Err(NUM);
    }
    let lo = (n - (big_n - big_s)).max(0.0);
    let hi = n.min(big_s);
    // Outside the support Excel answers with the limit values instead of #NUM!.
    if s < lo {
        return Ok(0.0);
    }
    if s > hi {
        return Ok(if cum { 1.0 } else { 0.0 });
    }
    // ln pmf(i)
    let ln_pmf = |i: f64| -> Dd {
        ln_choose(big_s, i)
            .add(ln_choose(big_n - big_s, n - i))
            .sub(ln_choose(big_n, n))
    };
    let pmf = |i: f64| -> Dd { dd::exp_sat(ln_pmf(i)) };
    if !cum {
        return Ok(pmf(s).to_f64());
    }
    if s - lo > 5_000_000.0 {
        return Err(NUM);
    }
    let mut term = pmf(lo);
    if term.hi == 0.0 && s > lo {
        // pmf(lo) underflows: sum downwards from s, relative to pmf(s), and scale at the end.
        let mut rel = Dd::from(1.0);
        let mut total = rel;
        let mut i = s;
        while i > lo {
            // pmf(i - 1) / pmf(i) = i (N - S - n + i) / ((S - i + 1)(n - i + 1))
            let num = Dd::from(i).mul(dd::sum(big_n - big_s - n, i));
            let den = Dd::from(big_s - i + 1.0).mul_f(n - i + 1.0);
            rel = rel.mul(num).div(den);
            total = total.add(rel);
            if rel.hi < total.hi * 1e-40 && num.hi < den.hi {
                break;
            }
            i -= 1.0;
        }
        return Ok(dd::exp_sat(ln_pmf(s).add(dd::ln_p(total))).to_f64());
    }
    let mut sum = term;
    let mut i = lo;
    while i < s {
        // pmf(i + 1) / pmf(i) = (S - i)(n - i) / ((i + 1)(N - S - n + i + 1))
        let num = Dd::from(big_s - i).mul_f(n - i);
        let den = Dd::from(i + 1.0).mul(dd::sum(big_n - big_s - n, i + 1.0));
        term = term.mul(num).div(den);
        sum = sum.add(term);
        i += 1.0;
    }
    Ok(sum.to_f64())
}

pub(super) fn hypgeom_dist(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let ([s, n, big_s, big_n], cum) = nums_bool::<4>(ctx, args)?;
    out(hypgeom(s, n, big_s, big_n, cum)?)
}

pub(super) fn hypgeomdist(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [s, n, big_s, big_n] = nums::<4>(ctx, args)?;
    out(hypgeom(s, n, big_s, big_n, false)?)
}
