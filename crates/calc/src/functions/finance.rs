//! Financial functions, measured against Excel (see `docs/testing/calc-conformance.md`).
//!
//! Excel raises `1 + rate` to an integer power by right-to-left binary exponentiation and to a fractional power
//! with `exp(n * ln x)`. The annuity functions share that power.

use super::math::collect;
use super::{Res, arity, num_arg, number, optional};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value};

/// `x^n` the way the financial functions compute it.
pub(super) fn xpow(x: f64, n: f64) -> Res<f64> {
    if x == 0.0 {
        if n == 0.0 {
            return Err(ErrorKind::Num);
        }
        if n < 0.0 {
            return Err(ErrorKind::DivZero);
        }
    }
    let r = if n == n.trunc() && n.abs() < 2_147_483_648.0 {
        let mut m = n.abs() as u64;
        let mut result = 1.0;
        let mut base = x;
        while m > 0 {
            if m & 1 == 1 {
                result *= base;
            }
            base *= base;
            m >>= 1;
        }
        if n < 0.0 { 1.0 / result } else { result }
    } else if x < 0.0 {
        return Err(ErrorKind::Num);
    } else if n == 0.5 {
        x.sqrt()
    } else {
        (n * x.ln()).exp()
    };
    if r.is_finite() {
        Ok(r)
    } else {
        Err(ErrorKind::Num)
    }
}

/// The optional argument `index` as a number, `default` when absent or omitted.
fn opt_num(ctx: &EvalContext<'_>, args: &[Arg], index: usize, default: f64) -> Res<f64> {
    match optional(args, index) {
        Some(a) => num_arg(ctx, a),
        None => Ok(default),
    }
}

/// A payment-timing argument: any non-zero value means the start of the period.
fn timing(t: f64) -> bool {
    t != 0.0
}

/// Future value of an annuity, Excel's `FV`.
pub(super) fn fv_calc(r: f64, n: f64, pmt: f64, pv: f64, t: bool) -> Res<f64> {
    if r == 0.0 {
        return Ok(-(pv + pmt * n));
    }
    let q = xpow(1.0 + r, n)?;
    let a = (q - 1.0) / r;
    let a = if t { a * (1.0 + r) } else { a };
    Ok(-(pv * q + pmt * a))
}

pub(super) fn pv_calc(r: f64, n: f64, pmt: f64, fv: f64, t: bool) -> Res<f64> {
    if r == 0.0 {
        return Ok(-(fv + pmt * n));
    }
    let q = xpow(1.0 + r, n)?;
    if q == 0.0 {
        return Err(ErrorKind::DivZero);
    }
    let a = (q - 1.0) / r;
    let a = if t { a * (1.0 + r) } else { a };
    Ok(-(fv + pmt * a) / q)
}

pub(super) fn pmt_calc(r: f64, n: f64, pv: f64, fv: f64, t: bool) -> Res<f64> {
    if n == 0.0 {
        return Err(ErrorKind::Num);
    }
    if r == 0.0 {
        return Ok(-(pv + fv) / n);
    }
    if xpow(1.0 + r, n).is_ok_and(|q| q != 0.0) {
        let e = (n * r.ln_1p()).exp_m1();
        let k = if t { 1.0 + r } else { 1.0 };
        Ok(-r * (pv + (pv + fv) / e) / k)
    } else {
        Err(ErrorKind::Num)
    }
}

pub(super) fn pv(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 5)?;
    let r = num_arg(ctx, &args[0])?;
    let n = num_arg(ctx, &args[1])?;
    let pmt = num_arg(ctx, &args[2])?;
    let fv = opt_num(ctx, args, 3, 0.0)?;
    let t = opt_num(ctx, args, 4, 0.0)?;
    number(pv_calc(r, n, pmt, fv, timing(t))?)
}

pub(super) fn fv(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 5)?;
    let r = num_arg(ctx, &args[0])?;
    let n = num_arg(ctx, &args[1])?;
    let pmt = num_arg(ctx, &args[2])?;
    let pv = opt_num(ctx, args, 3, 0.0)?;
    let t = opt_num(ctx, args, 4, 0.0)?;
    number(fv_calc(r, n, pmt, pv, timing(t))?)
}

pub(super) fn pmt(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 5)?;
    let r = num_arg(ctx, &args[0])?;
    let n = num_arg(ctx, &args[1])?;
    let pv = num_arg(ctx, &args[2])?;
    let fv = opt_num(ctx, args, 3, 0.0)?;
    let t = opt_num(ctx, args, 4, 0.0)?;
    number(pmt_calc(r, n, pv, fv, timing(t))?)
}

pub(super) fn nper(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 5)?;
    let r = num_arg(ctx, &args[0])?;
    let pmt = num_arg(ctx, &args[1])?;
    let pv = num_arg(ctx, &args[2])?;
    let fv = opt_num(ctx, args, 3, 0.0)?;
    let t = timing(opt_num(ctx, args, 4, 0.0)?);
    if r == 0.0 {
        if pmt == 0.0 {
            return Err(ErrorKind::DivZero);
        }
        return number(-(pv + fv) / pmt);
    }
    let k = if t { pmt * (1.0 + r) } else { pmt };
    let num = k - fv * r;
    let den = k + pv * r;
    let l = (1.0 + r).ln();
    if l == 0.0 {
        return Err(ErrorKind::DivZero);
    }
    if l.is_infinite() {
        return Err(ErrorKind::Num);
    }
    number((num / den).ln() / l)
}

// ---- IPMT, PPMT, CUMIPMT, CUMPRINC ---------------------------------------------------------

/// Interest part of payment `per`.
fn ipmt_calc(r: f64, per: f64, nper: f64, pv: f64, fv: f64, t: bool) -> Res<f64> {
    if per < 1.0 || per > nper.ceil() {
        return Err(ErrorKind::Num);
    }
    let pmt = pmt_calc(r, nper, pv, fv, t)?;
    if t {
        if per == 1.0 {
            return Ok(0.0);
        }
        Ok((fv_calc(r, per - 2.0, pmt, pv, true)? - pmt) * r)
    } else {
        Ok(fv_calc(r, per - 1.0, pmt, pv, false)? * r)
    }
}

pub(super) fn ipmt(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 4, 6)?;
    let r = num_arg(ctx, &args[0])?;
    let per = num_arg(ctx, &args[1])?;
    let nper = num_arg(ctx, &args[2])?;
    let pv = num_arg(ctx, &args[3])?;
    let fv = opt_num(ctx, args, 4, 0.0)?;
    let t = opt_num(ctx, args, 5, 0.0)?;
    number(ipmt_calc(r, per, nper, pv, fv, timing(t))?)
}

pub(super) fn ppmt(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 4, 6)?;
    let r = num_arg(ctx, &args[0])?;
    let per = num_arg(ctx, &args[1])?;
    let nper = num_arg(ctx, &args[2])?;
    let pv = num_arg(ctx, &args[3])?;
    let fv = opt_num(ctx, args, 4, 0.0)?;
    let t = timing(opt_num(ctx, args, 5, 0.0)?);
    let i = ipmt_calc(r, per, nper, pv, fv, t)?;
    number(pmt_calc(r, nper, pv, fv, t)? - i)
}

fn cumulative(ctx: &EvalContext<'_>, args: &[Arg], principal: bool) -> Res<Value> {
    arity(args, 6, 6)?;
    let r = num_arg(ctx, &args[0])?;
    let nper = num_arg(ctx, &args[1])?;
    let pv = num_arg(ctx, &args[2])?;
    let start = num_arg(ctx, &args[3])?;
    let end = num_arg(ctx, &args[4])?;
    let t = num_arg(ctx, &args[5])?;
    if r <= 0.0 || nper <= 0.0 || pv <= 0.0 || start < 1.0 || end < start || end > nper {
        return Err(ErrorKind::Num);
    }
    if t != 0.0 && t != 1.0 {
        return Err(ErrorKind::Num);
    }
    let t = t == 1.0;
    let mut sum = 0.0;
    let mut per = start.ceil();
    while per <= end.trunc() {
        let i = ipmt_calc(r, per, nper, pv, 0.0, t)?;
        sum += if principal {
            pmt_calc(r, nper, pv, 0.0, t)? - i
        } else {
            i
        };
        per += 1.0;
    }
    number(sum)
}

pub(super) fn cumipmt(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    cumulative(ctx, args, false)
}

pub(super) fn cumprinc(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    cumulative(ctx, args, true)
}

// ---- RATE ---------------------------------------------------------------------------------

const RATE_ITERATIONS: usize = 20;
const RATE_EPSILON: f64 = 1e-7;

pub(super) fn rate(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 6)?;
    let n = num_arg(ctx, &args[0])?;
    let pmt = num_arg(ctx, &args[1])?;
    let pv = num_arg(ctx, &args[2])?;
    let fv = opt_num(ctx, args, 3, 0.0)?;
    let t = if timing(opt_num(ctx, args, 4, 0.0)?) {
        1.0
    } else {
        0.0
    };
    let mut x = opt_num(ctx, args, 5, 0.1)?;
    for _ in 0..RATE_ITERATIONS {
        if x <= -1.0 {
            return Err(ErrorKind::Num);
        }
        let (f, df) = if x == 0.0 {
            (
                pv + pmt * n + fv,
                pv * n + pmt * n * (n - 1.0) / 2.0 + pmt * t * n,
            )
        } else {
            let u = 1.0 + x;
            let q = xpow(u, n)?;
            let q1 = q / u;
            let a = (q - 1.0) / x;
            (
                fv + pmt * (1.0 + x * t) * a + pv * q,
                pv * n * q1 + pmt * (1.0 + x * t) * (q1 * n / x - a / x) + pmt * t * a,
            )
        };
        let next = x - f / df;
        if !next.is_finite() {
            return Err(ErrorKind::Num);
        }
        if (next - x).abs() < RATE_EPSILON {
            return number(next);
        }
        x = next;
    }
    Err(ErrorKind::Num)
}

// ---- NPV, XNPV, IRR, XIRR, MIRR ----------------------------------------------------------

pub(super) fn npv(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 255)?;
    let r = num_arg(ctx, &args[0])?;
    let values = collect(ctx, &args[1..])?;
    let u = 1.0 + r;
    // Excel sums v_i / u^i (u^i is a running product) and divides the total by u once.
    let mut sum = 0.0;
    let mut p = 1.0;
    for v in &values {
        if u == 0.0 {
            return Err(ErrorKind::DivZero);
        }
        sum += v / p;
        p *= u;
    }
    number(sum / u)
}

/// The numbers of one array or range argument; text, booleans and blanks inside a reference are skipped.
fn numbers_of(ctx: &EvalContext<'_>, arg: &Arg) -> Res<Vec<f64>> {
    let mut out = Vec::new();
    match arg {
        Arg::Value(Value::Number(n)) => out.push(*n),
        Arg::Value(Value::Error(e)) => return Err(*e),
        Arg::Value(_) | Arg::Missing => return Err(ErrorKind::Value),
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
    Ok(out)
}

/// Values and dates of `XNPV`/`XIRR`: every cell must be a number and the two lists equally long.
fn dated_values(ctx: &EvalContext<'_>, vals: &Arg, dates: &Arg) -> Res<(Vec<f64>, Vec<f64>)> {
    let cells = |arg: &Arg| -> Res<Vec<f64>> {
        let mut out = Vec::new();
        for v in ctx.cells(arg) {
            match v {
                Value::Error(e) => return Err(e),
                other => out.push(other.to_number_in(ctx.date1904)?),
            }
        }
        Ok(out)
    };
    let v = cells(vals)?;
    let d = cells(dates)?;
    if v.len() != d.len() || v.is_empty() {
        return Err(ErrorKind::Num);
    }
    let base = d[0].trunc();
    if d.iter().any(|x| x.trunc() < base || *x < 0.0) {
        return Err(ErrorKind::Num);
    }
    Ok((v, d.iter().map(|x| x.trunc() - base).collect()))
}

pub(super) fn xnpv(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 3)?;
    let r = num_arg(ctx, &args[0])?;
    let (v, d) = dated_values(ctx, &args[1], &args[2])?;
    if r <= 0.0 {
        return Err(ErrorKind::Num);
    }
    let mut sum = 0.0;
    for (x, days) in v.iter().zip(&d) {
        sum += x / xpow(1.0 + r, days / 365.0)?;
    }
    number(sum)
}

fn has_both_signs(v: &[f64]) -> bool {
    v.iter().any(|x| *x > 0.0) && v.iter().any(|x| *x < 0.0)
}

const IRR_ITERATIONS: usize = 20;
const IRR_EPSILON: f64 = 1e-12;

pub(super) fn irr(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 2)?;
    let v = numbers_of(ctx, &args[0])?;
    let mut x = opt_num(ctx, args, 1, 0.1)?;
    if !has_both_signs(&v) {
        return Err(ErrorKind::Num);
    }
    for _ in 0..IRR_ITERATIONS {
        if x <= -1.0 {
            return Err(ErrorKind::Num);
        }
        let u = 1.0 + x;
        let (mut f, mut df) = (0.0, 0.0);
        for (i, c) in v.iter().enumerate() {
            let p = xpow(u, i as f64)?;
            f += c / p;
            df -= (i as f64) * c / (p * u);
        }
        let next = x - f / df;
        if !next.is_finite() {
            return Err(ErrorKind::Num);
        }
        if (next - x).abs() < IRR_EPSILON {
            return number(next);
        }
        x = next;
    }
    Err(ErrorKind::Num)
}

const XIRR_ITERATIONS: usize = 100;
const XIRR_EPSILON: f64 = 1e-12;

pub(super) fn xirr(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 3)?;
    let (v, d) = dated_values(ctx, &args[0], &args[1])?;
    let mut x = opt_num(ctx, args, 2, 0.1)?;
    if !has_both_signs(&v) {
        return Err(ErrorKind::Num);
    }
    for _ in 0..XIRR_ITERATIONS {
        if x <= -1.0 {
            return Err(ErrorKind::Num);
        }
        let u = 1.0 + x;
        let (mut f, mut df) = (0.0, 0.0);
        for (c, days) in v.iter().zip(&d) {
            let e = days / 365.0;
            let p = xpow(u, e)?;
            f += c / p;
            df -= e * c / (p * u);
        }
        let next = x - f / df;
        if !next.is_finite() {
            return Err(ErrorKind::Num);
        }
        if (next - x).abs() < XIRR_EPSILON {
            return number(next);
        }
        x = next;
    }
    Err(ErrorKind::Num)
}

pub(super) fn mirr(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 3)?;
    let v = numbers_of(ctx, &args[0])?;
    let fin = num_arg(ctx, &args[1])?;
    let rein = num_arg(ctx, &args[2])?;
    if v.len() < 2 || !has_both_signs(&v) {
        return Err(ErrorKind::DivZero);
    }
    if rein == -1.0 {
        return Err(ErrorKind::DivZero);
    }
    let n = v.len() as f64;
    let (mut pos, mut neg) = (0.0, 0.0);
    for (i, c) in v.iter().enumerate() {
        let k = (i + 1) as f64;
        if *c > 0.0 {
            pos += c / xpow(1.0 + rein, k)?;
        } else if *c < 0.0 {
            neg += c / xpow(1.0 + fin, k)?;
        }
    }
    let ratio = -pos * xpow(1.0 + rein, n)? / (neg * (1.0 + fin));
    number(xpow(ratio, 1.0 / (n - 1.0))? - 1.0)
}

// ---- depreciation -------------------------------------------------------------------------

pub(super) fn sln(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 3)?;
    let cost = num_arg(ctx, &args[0])?;
    let salvage = num_arg(ctx, &args[1])?;
    let life = num_arg(ctx, &args[2])?;
    if life == 0.0 {
        return Err(ErrorKind::DivZero);
    }
    number((cost - salvage) / life)
}

pub(super) fn syd(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 4, 4)?;
    let cost = num_arg(ctx, &args[0])?;
    let salvage = num_arg(ctx, &args[1])?;
    let life = num_arg(ctx, &args[2])?;
    let per = num_arg(ctx, &args[3])?;
    if salvage < 0.0 || life <= 0.0 || per <= 0.0 || per > life {
        return Err(ErrorKind::Num);
    }
    let denominator = life * (life + 1.0);
    if denominator.is_infinite() {
        return Err(ErrorKind::DivZero);
    }
    number((cost - salvage) * (life - per + 1.0) * 2.0 / denominator)
}

pub(super) fn db(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 4, 5)?;
    let cost = num_arg(ctx, &args[0])?;
    let salvage = num_arg(ctx, &args[1])?;
    let life = num_arg(ctx, &args[2])?;
    let period = num_arg(ctx, &args[3])?;
    let month = opt_num(ctx, args, 4, 12.0)?.trunc();
    if cost < 0.0 || salvage < 0.0 || life <= 0.0 || period <= 0.0 || !(1.0..=12.0).contains(&month)
    {
        return Err(ErrorKind::Num);
    }
    let extra = life < 1.0 && period >= 1.0;
    let period = period.floor().max(1.0);
    if period > life + if month < 12.0 { 1.0 } else { 0.0 } {
        return Err(ErrorKind::Num);
    }
    if cost == 0.0 {
        return number(0.0);
    }
    let rate = ((1.0 - xpow(salvage / cost, 1.0 / life)?) * 1000.0 + 0.5).floor() / 1000.0;
    // A life below one year adds one more iteration for periods of 1 or more; periods past the life take the partial-year form.
    let last = period + if extra { 1.0 } else { 0.0 };
    let mut book = cost;
    let mut value = 0.0;
    let mut p = 1.0;
    while p <= last {
        value = if p == 1.0 {
            cost * rate * (month / 12.0)
        } else if p > life {
            book * rate / 12.0 * (12.0 - month)
        } else {
            book * rate
        };
        book -= value;
        p += 1.0;
    }
    number(value)
}

/// One period of double-declining-balance depreciation. A period below 1 counts as the first period. The
/// declining balance before the period is `cost * (1 - rate)^(period - 1)` (binary power, see `xpow`), the
/// depreciation is that balance times `rate` and is cut so the book value never drops below `salvage`.
fn ddb_calc(cost: f64, salvage: f64, life: f64, period: f64, factor: f64) -> Res<f64> {
    let period = period.max(1.0);
    let rate = factor / life;
    let value = if rate >= 1.0 {
        let old = if period == 1.0 { cost } else { 0.0 };
        if salvage > 0.0 { old - salvage } else { old }
    } else {
        let old = cost * xpow(1.0 - rate, period - 1.0)?;
        let d = old * rate;
        if old - d < salvage { old - salvage } else { d }
    };
    Ok(value.max(0.0))
}

pub(super) fn ddb(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 4, 5)?;
    let cost = num_arg(ctx, &args[0])?;
    let salvage = num_arg(ctx, &args[1])?;
    let life = num_arg(ctx, &args[2])?;
    let period = num_arg(ctx, &args[3])?;
    let factor = opt_num(ctx, args, 4, 2.0)?;
    if cost < 0.0 || salvage < 0.0 || life <= 0.0 || period <= 0.0 || period > life || factor <= 0.0
    {
        return Err(ErrorKind::Num);
    }
    number(ddb_calc(cost, salvage, life, period, factor)?)
}

pub(super) fn vdb(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 5, 7)?;
    let cost = num_arg(ctx, &args[0])?;
    let salvage = num_arg(ctx, &args[1])?;
    let life = num_arg(ctx, &args[2])?;
    let start = num_arg(ctx, &args[3])?;
    let end = num_arg(ctx, &args[4])?;
    let factor = opt_num(ctx, args, 5, 2.0)?;
    let no_switch = match optional(args, 6) {
        Some(a) => super::bool_arg(ctx, a)?,
        None => false,
    };
    if cost < 0.0
        || salvage < 0.0
        || life <= 0.0
        || start < 0.0
        || end < start
        || end > life
        || factor <= 0.0
    {
        return Err(ErrorKind::Num);
    }
    let mut book = cost;
    let mut total = 0.0;
    let mut k = 0.0;
    while k < end.ceil() {
        let mut d = (book * factor / life).min(book - salvage).max(0.0);
        if !no_switch {
            let sl = (book - salvage) / (life - k);
            if sl > d {
                d = sl;
            }
        }
        let overlap = (end.min(k + 1.0) - start.max(k)).max(0.0);
        total += d * overlap;
        book -= d;
        k += 1.0;
    }
    number(total)
}

// ---- rates and fractions ------------------------------------------------------------------

pub(super) fn effect(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let nom = super::datecalc::atp_num(ctx, &args[0])?;
    let n = super::datecalc::atp_num(ctx, &args[1])?.trunc();
    if nom <= 0.0 || n < 1.0 {
        return Err(ErrorKind::Num);
    }
    number(xpow(1.0 + nom / n, n)? - 1.0)
}

pub(super) fn nominal(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let eff = super::datecalc::atp_num(ctx, &args[0])?;
    let n = super::datecalc::atp_num(ctx, &args[1])?.trunc();
    if eff <= 0.0 || n < 1.0 {
        return Err(ErrorKind::Num);
    }
    number(n * (xpow(1.0 + eff, 1.0 / n)? - 1.0))
}

pub(super) fn dollarde(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let d = super::datecalc::atp_num(ctx, &args[0])?;
    let f = super::datecalc::atp_num(ctx, &args[1])?.trunc() as i64;
    if f < 0 {
        return Err(ErrorKind::Num);
    }
    if f == 0 {
        return Err(ErrorKind::DivZero);
    }
    let f = f as f64;
    let whole = d.trunc();
    let frac = d - whole;
    let digits = f.log10().ceil();
    number(whole + frac * 10f64.powf(digits) / f)
}

pub(super) fn dollarfr(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let d = super::datecalc::atp_num(ctx, &args[0])?;
    let f = super::datecalc::atp_num(ctx, &args[1])?.trunc() as i64;
    if f < 0 {
        return Err(ErrorKind::Num);
    }
    if f == 0 {
        return Err(ErrorKind::DivZero);
    }
    let f = f as f64;
    let whole = d.trunc();
    let frac = d - whole;
    let digits = f.log10().ceil();
    number(whole + frac * f / 10f64.powf(digits))
}
