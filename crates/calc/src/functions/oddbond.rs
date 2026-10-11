//! `ODDFPRICE`, `ODDFYIELD`, `ODDLPRICE` and `ODDLYIELD`: securities with an odd first or last coupon period.
//!
//! The published formulas, generalised over the quasi-coupon periods the odd period spans (`NC` of them, counted
//! from the first coupon date backwards to the issue date, or from the last interest date forwards to maturity),
//! with the day counts of the basis (`finbond::days_between`). Measured against Excel with the conformance groups
//! `oddfprice`, `oddfyield`, `oddlprice` and `oddlyield`.

use super::finbond::{coupon_date, date_arg, days_between, frequency, num, opt_basis};
use super::{Res, arity, number};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value};

/// Day-count context of one call.
struct Dc {
    basis: u8,
    freq: i64,
    d1904: bool,
}

impl Dc {
    fn days(&self, a: i64, b: i64) -> Res<f64> {
        if b < a {
            return Ok(0.0);
        }
        days_between(self.basis, a, b, self.d1904)
    }

    /// Normal length of a quasi-coupon period.
    fn normal(&self, a: i64, b: i64) -> f64 {
        match self.basis {
            1 => (b - a) as f64,
            3 => 365.0 / self.freq as f64,
            _ => 360.0 / self.freq as f64,
        }
    }
}

/// Quasi-coupon dates `anchor + k * step months` (end-of-month rule from the anchor), `k = 0..=count`.
fn quasi(anchor: i64, step: i64, k: i64, d1904: bool) -> Res<i64> {
    coupon_date(anchor, k, step, d1904)
}

/// Dates are `Res<Vec<i64>>` from the first coupon back to (and including) the first date on or before `issue`.
fn backwards(first: i64, issue: i64, step: i64, d1904: bool) -> Res<Vec<i64>> {
    let mut v = vec![first];
    let mut k = 1;
    while *v.last().unwrap_or(&first) > issue {
        v.push(quasi(first, step, k, d1904)?);
        k += 1;
        if k > 100_000 {
            return Err(ErrorKind::Num);
        }
    }
    Ok(v)
}

/// Dates from the last interest date forwards to the first date on or after `maturity`.
fn forwards(last: i64, maturity: i64, step: i64, d1904: bool) -> Res<Vec<i64>> {
    // follow the schedule that ends at the last interest date: step forward from it
    let mut v = vec![last];
    let mut k = 1;
    while *v.last().unwrap_or(&last) < maturity {
        v.push(quasi(last, -step, k, d1904)?);
        k += 1;
        if k > 100_000 {
            return Err(ErrorKind::Num);
        }
    }
    Ok(v)
}

struct First {
    redemption: f64,
    c: f64,
    base: f64,
    n: f64,
    tau: f64,
    c1: f64,
    accrued: f64,
}

impl First {
    fn price(&self, y: f64) -> f64 {
        let b = 1.0 + y / self.freq_f();
        let mut p = self.redemption / b.powf(self.n - 1.0 + self.tau) + self.c1 / b.powf(self.tau);
        let mut k = 2.0;
        while k <= self.n {
            p += self.c / b.powf(k - 1.0 + self.tau);
            k += 1.0;
        }
        p - self.c * self.accrued
    }

    fn freq_f(&self) -> f64 {
        self.base
    }
}

struct Args {
    settlement: i64,
    maturity: i64,
    other: i64,
    first: i64,
    rate: f64,
    x: f64,
    redemption: f64,
    freq: i64,
    basis: u8,
}

fn first_args(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Args> {
    arity(args, 8, 9)?;
    let settlement = date_arg(ctx, &args[0])?;
    let maturity = date_arg(ctx, &args[1])?;
    let issue = date_arg(ctx, &args[2])?;
    let first = date_arg(ctx, &args[3])?;
    let rate = num(ctx, &args[4])?;
    let x = num(ctx, &args[5])?;
    let redemption = num(ctx, &args[6])?;
    let freq = frequency(ctx, &args[7])?;
    let basis = opt_basis(ctx, args, 8)?;
    if !(maturity > first && first > settlement && settlement > issue)
        || rate < 0.0
        || x < 0.0
        || redemption <= 0.0
    {
        return Err(ErrorKind::Num);
    }
    Ok(Args {
        settlement,
        maturity,
        other: issue,
        first,
        rate,
        x,
        redemption,
        freq,
        basis,
    })
}

fn last_args(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Args> {
    arity(args, 7, 8)?;
    let settlement = date_arg(ctx, &args[0])?;
    let maturity = date_arg(ctx, &args[1])?;
    let last = date_arg(ctx, &args[2])?;
    let rate = num(ctx, &args[3])?;
    let x = num(ctx, &args[4])?;
    let redemption = num(ctx, &args[5])?;
    let freq = frequency(ctx, &args[6])?;
    let basis = opt_basis(ctx, args, 7)?;
    if !(maturity > settlement && settlement > last) || rate < 0.0 || x < 0.0 || redemption <= 0.0 {
        return Err(ErrorKind::Num);
    }
    Ok(Args {
        settlement,
        maturity,
        other: last,
        first: last,
        rate,
        x,
        redemption,
        freq,
        basis,
    })
}

fn first_model(a: &Args, d1904: bool) -> Res<First> {
    let dc = Dc {
        basis: a.basis,
        freq: a.freq,
        d1904,
    };
    let step = 12 / a.freq;
    let f = a.freq as f64;
    let c = 100.0 * a.rate / f;
    let q = backwards(a.first, a.other, step, d1904)?;
    // period i (1-based) is [q[i], q[i-1]]
    let mut c1 = 0.0;
    let mut accrued = 0.0;
    let mut tau = 0.0;
    let mut found = false;
    for i in 1..q.len() {
        let (lo, hi) = (q[i], q[i - 1]);
        let nl = dc.normal(lo, hi);
        let start = lo.max(a.other);
        c1 += dc.days(start, hi)? / nl;
        if a.settlement > start {
            accrued += dc.days(start, a.settlement.min(hi))? / nl;
        }
        if !found && lo <= a.settlement && a.settlement < hi {
            found = true;
            tau = (i - 1) as f64 + dc.days(a.settlement, hi)? / nl;
        }
    }
    c1 *= c;
    // coupons from the first coupon date to maturity
    let mut k = 0;
    loop {
        let d = quasi(a.maturity, step, k, d1904)?;
        if d < a.first {
            break;
        }
        k += 1;
        if k > 100_000 {
            return Err(ErrorKind::Num);
        }
    }
    // the loop counted the dates maturity - j*step >= first for j = 0..k-1: that is k dates
    let n = k as f64;
    Ok(First {
        redemption: a.redemption,
        c,
        base: f,
        n,
        tau,
        c1,
        accrued,
    })
}

pub(super) fn oddfprice(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let a = first_args(ctx, args)?;
    let m = first_model(&a, ctx.date1904)?;
    number(m.price(a.x))
}

pub(super) fn oddfyield(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let a = first_args(ctx, args)?;
    let m = first_model(&a, ctx.date1904)?;
    // price falls as the yield rises: bracket and bisect, then polish with secant steps
    let f = a.freq as f64;
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    // a price above the zero-yield price needs a negative yield
    let mut guard = 0;
    while m.price(lo) < a.x {
        hi = lo;
        lo = -f + (lo + f) / 2.0;
        guard += 1;
        if guard > 60 {
            return Err(ErrorKind::Num);
        }
    }
    while m.price(hi) > a.x {
        hi *= 2.0;
        if hi > 1e6 {
            return Err(ErrorKind::Num);
        }
    }
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if m.price(mid) > a.x {
            lo = mid;
        } else {
            hi = mid;
        }
        if hi - lo <= f64::EPSILON * hi.abs() {
            break;
        }
    }
    number(0.5 * (lo + hi))
}

struct Last {
    redemption: f64,
    c: f64,
    freq: f64,
    dc_sum: f64,
    dsm_sum: f64,
    accrued: f64,
}

fn last_model(a: &Args, d1904: bool) -> Res<Last> {
    let dc = Dc {
        basis: a.basis,
        freq: a.freq,
        d1904,
    };
    let step = 12 / a.freq;
    let f = a.freq as f64;
    let c = 100.0 * a.rate / f;
    let p = forwards(a.other, a.maturity, step, d1904)?;
    let (mut dc_sum, mut dsm_sum, mut accrued) = (0.0, 0.0, 0.0);
    for i in 1..p.len() {
        let (lo, hi) = (p[i - 1], p[i]);
        let nl = dc.normal(lo, hi);
        let end = hi.min(a.maturity);
        dc_sum += dc.days(lo, end)? / nl;
        if a.settlement < end {
            dsm_sum += dc.days(lo.max(a.settlement), end)? / nl;
        }
        if a.settlement > lo {
            accrued += dc.days(lo, a.settlement.min(end))? / nl;
        }
    }
    Ok(Last {
        redemption: a.redemption,
        c,
        freq: f,
        dc_sum,
        dsm_sum,
        accrued,
    })
}

pub(super) fn oddlprice(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let a = last_args(ctx, args)?;
    let m = last_model(&a, ctx.date1904)?;
    let top = m.redemption + m.c * m.dc_sum;
    let denom = 1.0 + m.dsm_sum * a.x / m.freq;
    number(top / denom - m.c * m.accrued)
}

pub(super) fn oddlyield(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let a = last_args(ctx, args)?;
    if a.x <= 0.0 {
        return Err(ErrorKind::Num);
    }
    let m = last_model(&a, ctx.date1904)?;
    let top = m.redemption + m.c * m.dc_sum;
    let pa = a.x + m.c * m.accrued;
    number(((top - pa) / pa) * (m.freq / m.dsm_sum))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_examples() {
        use crate::functions::datetime::ymd_to_serial;
        let d = |y, m, dd| ymd_to_serial(y, m, dd, false).unwrap() as i64;
        // Microsoft's ODDFPRICE example: 113.597717
        let a = Args {
            settlement: d(2008, 11, 11),
            maturity: d(2021, 3, 1),
            other: d(2008, 10, 15),
            first: d(2009, 3, 1),
            rate: 0.0785,
            x: 0.0625,
            redemption: 100.0,
            freq: 2,
            basis: 1,
        };
        let m = first_model(&a, false).unwrap();
        assert!(
            (m.price(a.x) - 113.597_717).abs() < 1e-6,
            "{}",
            m.price(a.x)
        );
    }
}
