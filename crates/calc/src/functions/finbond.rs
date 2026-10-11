//! Securities and day-count financial functions: coupon dates, `PRICE`, `YIELD`, `DURATION`, discount
//! securities, Treasury bills, `ACCRINT`, the French depreciation functions and the small rate helpers.
//!
//! Measured against Excel with the conformance corpus (groups `coupdaybs` ... `tbillyield`); the formulas are
//! the published ones (OpenFormula, Excel documentation) and the operation order is the one that reproduces
//! Excel's last digits.

use super::datetime::{serial_to_ymd, ymd_to_serial};
use super::finance::xpow;
use super::math::collect;
use super::{Res, arity, number, optional};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value};

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn dim(y: i64, m: i64) -> i64 {
    match m {
        2 => {
            if is_leap(y) {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

pub(super) fn ymd(serial: i64, d1904: bool) -> Res<(i64, i64, i64)> {
    serial_to_ymd(serial as f64, d1904).ok_or(ErrorKind::Num)
}

/// A date argument: truncated to a whole day, must be a valid non-negative serial.
pub(super) fn date_arg(ctx: &EvalContext<'_>, arg: &Arg) -> Res<i64> {
    let n = super::datecalc::atp_num(ctx, arg)?.trunc();
    if !(0.0..=2_958_465.0).contains(&n) {
        return Err(ErrorKind::Num);
    }
    Ok(n as i64)
}

pub(super) fn num(ctx: &EvalContext<'_>, arg: &Arg) -> Res<f64> {
    super::datecalc::atp_num(ctx, arg)
}

pub(super) fn opt_basis(ctx: &EvalContext<'_>, args: &[Arg], index: usize) -> Res<u8> {
    match optional(args, index) {
        None => Ok(0),
        Some(a) => {
            let b = num(ctx, a)?.trunc();
            if !(0.0..=4.0).contains(&b) {
                return Err(ErrorKind::Num);
            }
            Ok(b as u8)
        }
    }
}

pub(super) fn frequency(ctx: &EvalContext<'_>, arg: &Arg) -> Res<i64> {
    let f = num(ctx, arg)?.trunc();
    if f == 1.0 || f == 2.0 || f == 4.0 {
        Ok(f as i64)
    } else {
        Err(ErrorKind::Num)
    }
}

// ---- day counts ------------------------------------------------------------------------------

/// Days from `a` to `b` (`a <= b`) under the basis' day count: 30/360 (US for basis 0, European for 4) or actual.
pub(super) fn days_between(basis: u8, a: i64, b: i64, d1904: bool) -> Res<f64> {
    match basis {
        0 | 4 => {
            let (y1, m1, mut d1) = ymd(a, d1904)?;
            let (y2, m2, mut d2) = ymd(b, d1904)?;
            if basis == 4 {
                d1 = d1.min(30);
                d2 = d2.min(30);
            } else {
                let feb_end = |y: i64, m: i64, d: i64| m == 2 && d == dim(y, 2);
                let first = d1;
                if feb_end(y1, m1, d1) {
                    if feb_end(y2, m2, d2) {
                        d2 = 30;
                    }
                    d1 = 30;
                }
                if d2 == 31 && first >= 30 {
                    d2 = 30;
                }
                if d1 == 31 {
                    d1 = 30;
                }
            }
            Ok(((y2 - y1) * 360 + (m2 - m1) * 30 + (d2 - d1)) as f64)
        }
        _ => Ok((b - a) as f64),
    }
}

fn year_days(y: i64) -> i64 {
    if is_leap(y) { 366 } else { 365 }
}

/// The length of the year the basis divides by for the interval `a..b`.
fn year_length(basis: u8, a: i64, b: i64, d1904: bool) -> Res<f64> {
    match basis {
        0 | 2 | 4 => Ok(360.0),
        3 => Ok(365.0),
        _ => {
            let (y1, m1, d1) = ymd(a, d1904)?;
            let (y2, m2, d2) = ymd(b, d1904)?;
            let one_year = y1 == y2 || (y2 == y1 + 1 && (m1 > m2 || (m1 == m2 && d1 >= d2)));
            Ok(if one_year {
                if y1 == y2 {
                    year_days(y1) as f64
                } else if (is_leap(y1) && m1 <= 2)
                    || (is_leap(y2) && (m2 > 2 || (m2 == 2 && d2 == 29)))
                {
                    366.0
                } else {
                    365.0
                }
            } else {
                let total: i64 = (y1..=y2).map(year_days).sum();
                total as f64 / (y2 - y1 + 1) as f64
            })
        }
    }
}

// ---- coupon schedule -------------------------------------------------------------------------

/// The coupon date `k` periods before `maturity` (the end-of-month rule applies when the maturity is one).
pub(super) fn coupon_date(maturity: i64, k: i64, step: i64, d1904: bool) -> Res<i64> {
    let (y, m, d) = ymd(maturity, d1904)?;
    let eom = d == dim(y, m);
    let total = y * 12 + (m - 1) - k * step;
    let (ny, nm) = (total.div_euclid(12), total.rem_euclid(12) + 1);
    let nd = if eom { dim(ny, nm) } else { d.min(dim(ny, nm)) };
    ymd_to_serial(ny, nm, nd, d1904)
        .map(|s| s as i64)
        .ok_or(ErrorKind::Num)
}

/// The date `k * step` months before `anchor` with the day clamped to the month length and no end-of-month rule
/// (`ODDLPRICE`/`ODDLYIELD` step forward from the last interest date this way, measured).
pub(super) fn clamped_date(anchor: i64, k: i64, step: i64, d1904: bool) -> Res<i64> {
    let (y, m, d) = ymd(anchor, d1904)?;
    let total = y * 12 + (m - 1) - k * step;
    let (ny, nm) = (total.div_euclid(12), total.rem_euclid(12) + 1);
    ymd_to_serial(ny, nm, d.min(dim(ny, nm)), d1904)
        .map(|s| s as i64)
        .ok_or(ErrorKind::Num)
}

/// The coupon period around the settlement date.
struct Coupon {
    pcd: i64,
    ncd: i64,
    /// Coupons still to be paid.
    n: i64,
    freq: i64,
    basis: u8,
    d1904: bool,
    settlement: i64,
}

fn coupon(settlement: i64, maturity: i64, freq: i64, basis: u8, d1904: bool) -> Res<Coupon> {
    let step = 12 / freq;
    let (sy, sm, _) = ymd(settlement, d1904)?;
    let (my, mm, _) = ymd(maturity, d1904)?;
    let months = (my - sy) * 12 + (mm - sm);
    let mut k = (months / step - 1).max(0);
    while k > 0 && coupon_date(maturity, k + 1, step, d1904)? > settlement {
        k += 1;
    }
    // Step back while the previous date is still after the settlement, forward while it is not.
    while coupon_date(maturity, k, step, d1904)? <= settlement && k > 0 {
        k -= 1;
    }
    while coupon_date(maturity, k + 1, step, d1904)? > settlement {
        k += 1;
        if k > 100_000 {
            return Err(ErrorKind::Num);
        }
    }
    Ok(Coupon {
        pcd: coupon_date(maturity, k + 1, step, d1904)?,
        ncd: coupon_date(maturity, k, step, d1904)?,
        n: k + 1,
        freq,
        basis,
        d1904,
        settlement,
    })
}

impl Coupon {
    /// Days from the previous coupon date to the settlement.
    fn days_bs(&self) -> Res<f64> {
        days_between(self.basis, self.pcd, self.settlement, self.d1904)
    }

    /// Days in the coupon period.
    fn days(&self) -> f64 {
        match self.basis {
            1 => (self.ncd - self.pcd) as f64,
            3 => 365.0 / self.freq as f64,
            _ => 360.0 / self.freq as f64,
        }
    }

    /// Days from the settlement to the next coupon date.
    fn days_nc(&self) -> Res<f64> {
        match self.basis {
            0 | 4 => Ok(self.days() - self.days_bs()?),
            _ => Ok((self.ncd - self.settlement) as f64),
        }
    }
}

struct CouponArgs {
    settlement: i64,
    maturity: i64,
    freq: i64,
    basis: u8,
}

fn coupon_args(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<CouponArgs> {
    arity(args, 3, 4)?;
    let settlement = date_arg(ctx, &args[0])?;
    let maturity = date_arg(ctx, &args[1])?;
    let freq = frequency(ctx, &args[2])?;
    let basis = opt_basis(ctx, args, 3)?;
    if settlement >= maturity {
        return Err(ErrorKind::Num);
    }
    Ok(CouponArgs {
        settlement,
        maturity,
        freq,
        basis,
    })
}

fn coupon_of(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Coupon> {
    let a = coupon_args(ctx, args)?;
    coupon(a.settlement, a.maturity, a.freq, a.basis, ctx.date1904)
}

pub(super) fn coupdaybs(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    number(coupon_of(ctx, args)?.days_bs()?)
}

pub(super) fn coupdays(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    number(coupon_of(ctx, args)?.days())
}

pub(super) fn coupdaysnc(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    number(coupon_of(ctx, args)?.days_nc()?)
}

pub(super) fn coupncd(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    number(coupon_of(ctx, args)?.ncd as f64)
}

pub(super) fn couppcd(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    number(coupon_of(ctx, args)?.pcd as f64)
}

pub(super) fn coupnum(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    number(coupon_of(ctx, args)?.n as f64)
}

// ---- PRICE, YIELD, DURATION --------------------------------------------------------------------

struct Bond {
    c: Coupon,
    rate: f64,
    redemption: f64,
}

impl Bond {
    /// `(N, A, DSC, E)`
    fn terms(&self) -> Res<(f64, f64, f64, f64)> {
        Ok((
            self.c.n as f64,
            self.c.days_bs()?,
            self.c.days_nc()?,
            self.c.days(),
        ))
    }

    fn price(&self, yld: f64) -> Res<f64> {
        let (n, a, dsc, e) = self.terms()?;
        self.price_with(yld, n, a, dsc, e)
    }

    fn price_with(&self, yld: f64, n: f64, a: f64, dsc: f64, e: f64) -> Res<f64> {
        let freq = self.c.freq as f64;
        let coup = 100.0 * self.rate / freq;
        if n == 1.0 {
            let t1 = dsc / e;
            let denom = 1.0 + t1 * yld / freq;
            return Ok((self.redemption + coup) / denom - a / e * coup);
        }
        let base = 1.0 + yld / freq;
        let mut price = self.redemption / xpow(base, n - 1.0 + dsc / e)?;
        let mut k = 1.0;
        while k <= n {
            price += coup / xpow(base, k - 1.0 + dsc / e)?;
            k += 1.0;
        }
        Ok(price - coup * a / e)
    }
}

fn bond_args(ctx: &EvalContext<'_>, args: &[Arg], n: usize) -> Res<(Coupon, f64, f64, f64)> {
    // price(settlement, maturity, rate, yld|pr, redemption, frequency[, basis])
    arity(args, n, n + 1)?;
    let settlement = date_arg(ctx, &args[0])?;
    let maturity = date_arg(ctx, &args[1])?;
    let rate = num(ctx, &args[2])?;
    let x = num(ctx, &args[3])?;
    let redemption = num(ctx, &args[4])?;
    let freq = frequency(ctx, &args[5])?;
    let basis = opt_basis(ctx, args, 6)?;
    if settlement >= maturity || rate < 0.0 {
        return Err(ErrorKind::Num);
    }
    let c = coupon(settlement, maturity, freq, basis, ctx.date1904)?;
    Ok((c, rate, x, redemption))
}

pub(super) fn price(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let (c, rate, yld, redemption) = bond_args(ctx, args, 6)?;
    if yld < 0.0 || redemption <= 0.0 {
        return Err(ErrorKind::Num);
    }
    number(
        Bond {
            c,
            rate,
            redemption,
        }
        .price(yld)?,
    )
}

pub(super) fn yield_(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let (c, rate, pr, redemption) = bond_args(ctx, args, 6)?;
    if pr <= 0.0 || redemption <= 0.0 {
        return Err(ErrorKind::Num);
    }
    let bond = Bond {
        c,
        rate,
        redemption,
    };
    let (n, a, _, e) = bond.terms()?;
    let dsc = e - a;
    let freq = bond.c.freq as f64;
    let coup = 100.0 * rate / freq;
    if n <= 1.0 {
        let y = ((redemption + coup) / (pr + a / e * coup) - 1.0) * freq * e / dsc;
        return number(y);
    }
    // Newton iteration on the price.
    let mut y = rate;
    for _ in 0..200 {
        let p = |v: f64| bond.price_with(v, n, a, dsc, e);
        let f = p(y)? - pr;
        let h = 1e-7;
        let df = (p(y + h)? - p(y - h)?) / (2.0 * h);
        let next = y - f / df;
        if !next.is_finite() {
            return Err(ErrorKind::Num);
        }
        if (next - y).abs() < 1e-15 {
            return number(next);
        }
        y = next;
    }
    Err(ErrorKind::Num)
}

fn duration_calc(ctx: &EvalContext<'_>, args: &[Arg], modified: bool) -> Res<Value> {
    // (settlement, maturity, coupon, yld, frequency[, basis])
    arity(args, 5, 6)?;
    let settlement = date_arg(ctx, &args[0])?;
    let maturity = date_arg(ctx, &args[1])?;
    let rate = num(ctx, &args[2])?;
    let yld = num(ctx, &args[3])?;
    let freq = frequency(ctx, &args[4])?;
    let basis = opt_basis(ctx, args, 5)?;
    if settlement >= maturity || rate < 0.0 || yld < 0.0 {
        return Err(ErrorKind::Num);
    }
    let c = coupon(settlement, maturity, freq, basis, ctx.date1904)?;
    let bond = Bond {
        c,
        rate,
        redemption: 100.0,
    };
    let (n, _a, dsc, e) = bond.terms()?;
    let f = freq as f64;
    let coup = 100.0 * rate / f;
    let base = 1.0 + yld / f;
    let (mut num_, mut den) = (0.0, 0.0);
    let mut k = 1.0;
    while k <= n {
        let t = k - 1.0 + dsc / e;
        let cf = if k == n { coup + 100.0 } else { coup };
        let pv = cf / xpow(base, t)?;
        num_ += t / f * pv;
        den += pv;
        k += 1.0;
    }
    let d = num_ / den;
    number(if modified { d / base } else { d })
}

pub(super) fn duration(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    duration_calc(ctx, args, false)
}

pub(super) fn mduration(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    duration_calc(ctx, args, true)
}

// ---- discount securities -------------------------------------------------------------------

/// `(DSM, B)` for a security held from `settlement` to `maturity` under `basis`.
fn dsm_and_year(settlement: i64, maturity: i64, basis: u8, d1904: bool) -> Res<(f64, f64)> {
    Ok((
        days_between(basis, settlement, maturity, d1904)?,
        year_length(basis, settlement, maturity, d1904)?,
    ))
}

fn security(ctx: &EvalContext<'_>, args: &[Arg], n: usize) -> Res<(i64, i64, f64, f64, u8)> {
    arity(args, n, n + 1)?;
    let settlement = date_arg(ctx, &args[0])?;
    let maturity = date_arg(ctx, &args[1])?;
    let a = num(ctx, &args[2])?;
    let b = num(ctx, &args[3])?;
    let basis = opt_basis(ctx, args, 4)?;
    if settlement >= maturity {
        return Err(ErrorKind::Num);
    }
    Ok((settlement, maturity, a, b, basis))
}

pub(super) fn disc(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let (s, m, pr, red, basis) = security(ctx, args, 4)?;
    if pr <= 0.0 || red <= 0.0 {
        return Err(ErrorKind::Num);
    }
    let (dsm, b) = dsm_and_year(s, m, basis, ctx.date1904)?;
    number((1.0 - pr / red) * (b / dsm))
}

pub(super) fn pricedisc(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let (s, m, d, red, basis) = security(ctx, args, 4)?;
    if d <= 0.0 || red <= 0.0 {
        return Err(ErrorKind::Num);
    }
    let (dsm, b) = dsm_and_year(s, m, basis, ctx.date1904)?;
    number(red - d * red * dsm / b)
}

pub(super) fn yielddisc(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let (s, m, pr, red, basis) = security(ctx, args, 4)?;
    if pr <= 0.0 || red <= 0.0 {
        return Err(ErrorKind::Num);
    }
    let (dsm, b) = dsm_and_year(s, m, basis, ctx.date1904)?;
    number((red - pr) / pr * (b / dsm))
}

pub(super) fn intrate(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let (s, m, inv, red, basis) = security(ctx, args, 4)?;
    if inv <= 0.0 || red <= 0.0 {
        return Err(ErrorKind::Num);
    }
    let (dsm, b) = dsm_and_year(s, m, basis, ctx.date1904)?;
    number((red - inv) / inv * (b / dsm))
}

pub(super) fn received(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let (s, m, inv, d, basis) = security(ctx, args, 4)?;
    if inv <= 0.0 || d <= 0.0 {
        return Err(ErrorKind::Num);
    }
    let (dsm, b) = dsm_and_year(s, m, basis, ctx.date1904)?;
    number(inv / (1.0 - d * dsm / b))
}

/// `PRICEMAT`/`YIELDMAT`: `(settlement, maturity, issue, rate, x, basis)`.
fn maturity_security(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<(i64, i64, i64, f64, f64, u8)> {
    arity(args, 5, 6)?;
    let s = date_arg(ctx, &args[0])?;
    let m = date_arg(ctx, &args[1])?;
    let i = date_arg(ctx, &args[2])?;
    let rate = num(ctx, &args[3])?;
    let x = num(ctx, &args[4])?;
    let basis = opt_basis(ctx, args, 5)?;
    if s >= m || rate < 0.0 || x < 0.0 {
        return Err(ErrorKind::Num);
    }
    Ok((s, m, i, rate, x, basis))
}

pub(super) fn pricemat(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let (s, m, i, rate, yld, basis) = maturity_security(ctx, args)?;
    let d = ctx.date1904;
    let b = year_length(basis, i, s, d)?;
    let dim_ = days_between(basis, i, m, d)?;
    let a = days_between(basis, i, s, d)?;
    let dsm = days_between(basis, s, m, d)?;
    number((100.0 + dim_ / b * rate * 100.0) / (1.0 + dsm / b * yld) - a / b * rate * 100.0)
}

pub(super) fn yieldmat(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let (s, m, i, rate, pr, basis) = maturity_security(ctx, args)?;
    let d = ctx.date1904;
    let b = year_length(basis, i, s, d)?;
    let dim_ = days_between(basis, i, m, d)?;
    let a = days_between(basis, i, s, d)?;
    let dsm = days_between(basis, s, m, d)?;
    number(((1.0 + dim_ / b * rate) / (pr / 100.0 + a / b * rate) - 1.0) * (b / dsm))
}

// ---- Treasury bills ------------------------------------------------------------------------

fn tbill(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<(f64, f64)> {
    arity(args, 3, 3)?;
    let s = date_arg(ctx, &args[0])?;
    let m = date_arg(ctx, &args[1])?;
    let x = num(ctx, &args[2])?;
    if s >= m || m - s > 365 || x <= 0.0 {
        return Err(ErrorKind::Num);
    }
    Ok(((m - s) as f64, x))
}

pub(super) fn tbillprice(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let (dsm, d) = tbill(ctx, args)?;
    number(100.0 * (1.0 - d * dsm / 360.0))
}

pub(super) fn tbillyield(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let (dsm, pr) = tbill(ctx, args)?;
    number((100.0 - pr) / pr * (360.0 / dsm))
}

pub(super) fn tbilleq(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let (dsm, d) = tbill(ctx, args)?;
    if dsm <= 182.0 {
        return number(365.0 * d / (360.0 - d * dsm));
    }
    let p = 100.0 * (1.0 - d * dsm / 360.0);
    let t = dsm / 365.0;
    let a = (t - 0.5) / 2.0;
    let c = 1.0 - 100.0 / p;
    number((-t + (t * t - 4.0 * a * c).sqrt()) / (2.0 * a))
}

// ---- accrued interest ----------------------------------------------------------------------

pub(super) fn accrintm(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 4, 5)?;
    let i = date_arg(ctx, &args[0])?;
    let s = date_arg(ctx, &args[1])?;
    let rate = num(ctx, &args[2])?;
    let par = match optional(args, 3) {
        Some(a) => num(ctx, a)?,
        None => 1000.0,
    };
    let basis = opt_basis(ctx, args, 4)?;
    if rate <= 0.0 || par <= 0.0 || i >= s {
        return Err(ErrorKind::Num);
    }
    let (a, d) = dsm_and_year(i, s, basis, ctx.date1904)?;
    number(par * rate * (a / d))
}

pub(super) fn accrint(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 6, 8)?;
    let issue = date_arg(ctx, &args[0])?;
    let first = date_arg(ctx, &args[1])?;
    let settlement = date_arg(ctx, &args[2])?;
    let rate = num(ctx, &args[3])?;
    let par = match optional(args, 4) {
        Some(a) => num(ctx, a)?,
        None => 1000.0,
    };
    let freq = frequency(ctx, &args[5])?;
    let basis = opt_basis(ctx, args, 6)?;
    let from_issue = match optional(args, 7) {
        Some(a) => super::bool_arg(ctx, a)?,
        None => true,
    };
    if rate <= 0.0 || par <= 0.0 || issue >= settlement {
        return Err(ErrorKind::Num);
    }
    let d = ctx.date1904;
    let f = freq as f64;
    // Walk the quasi-coupon periods back from the first interest date.
    let step = 12 / freq;
    let start = if from_issue { issue } else { first };
    if !from_issue && settlement <= first {
        return number(0.0);
    }
    let mut sum = 0.0;
    // Period boundaries: first - k * step months.
    let mut k = 0;
    while coupon_date(first, k + 1, step, d)? > start {
        k += 1;
        if k > 100_000 {
            return Err(ErrorKind::Num);
        }
    }
    // Periods from the one holding `start` up to the one holding the settlement.
    let mut j = k + 1;
    loop {
        let lo = coupon_date(first, j, step, d)?;
        let hi = coupon_date(first, j - 1, step, d)?;
        let a_lo = lo.max(start);
        let a_hi = hi.min(settlement);
        if a_hi > a_lo {
            let (days, nl) = match basis {
                0 | 4 => (days_between(basis, a_lo, a_hi, d)?, 360.0 / f),
                1 => ((a_hi - a_lo) as f64, (hi - lo) as f64),
                3 => ((a_hi - a_lo) as f64, 365.0 / f),
                _ => ((a_hi - a_lo) as f64, 360.0 / f),
            };
            sum += days / nl;
        }
        if hi >= settlement {
            break;
        }
        j -= 1;
        if j < -100_000 {
            return Err(ErrorKind::Num);
        }
    }
    number(par * rate / f * sum)
}

// ---- French depreciation -------------------------------------------------------------------

fn amor(ctx: &EvalContext<'_>, args: &[Arg], degressive: bool) -> Res<Value> {
    arity(args, 6, 7)?;
    let cost = num(ctx, &args[0])?;
    let purchased = date_arg(ctx, &args[1])?;
    let first = date_arg(ctx, &args[2])?;
    let salvage = num(ctx, &args[3])?;
    let period = num(ctx, &args[4])?.trunc();
    let rate = num(ctx, &args[5])?;
    let basis = opt_basis(ctx, args, 6)?;
    if cost <= 0.0
        || salvage < 0.0
        || salvage > cost
        || period < 0.0
        || rate <= 0.0
        || purchased > first
    {
        return Err(ErrorKind::Num);
    }
    if basis == 2 {
        return Err(ErrorKind::Num);
    }
    let d = ctx.date1904;
    let (dsm, b) = dsm_and_year(purchased, first, basis, d)?;
    let first_frac = dsm / b;
    if !degressive {
        let one = cost * rate;
        let first_dep = first_frac * one;
        let mut remaining = cost - salvage;
        let full = ((remaining - first_dep) / one).floor();
        if period == 0.0 {
            return number(first_dep.min(remaining));
        }
        remaining -= first_dep;
        let p = period;
        if p <= full {
            return number(one.min(remaining));
        }
        // Periods after the last full one.
        let after = remaining - full * one;
        if p == full + 1.0 {
            return number(after.min(one).max(0.0));
        }
        return number(0.0);
    }
    let life = 1.0 / rate;
    let coeff = if life < 3.0 {
        1.0
    } else if life < 5.0 {
        1.5
    } else if life <= 6.0 {
        2.0
    } else {
        2.5
    };
    let r = rate * coeff;
    let mut value = (first_frac * r * cost).round();
    let mut remaining = cost - value;
    if period == 0.0 {
        return number(value);
    }
    let mut n = 1.0;
    let mut rate_n = r;
    let periods = (life - 1.0).ceil();
    while n <= period {
        if (periods - n) < 2.0 {
            rate_n = 0.5;
        }
        let dep = (rate_n * remaining).round();
        value = dep;
        if remaining - dep < salvage {
            value = if remaining - salvage > 0.0 {
                (remaining - salvage).round()
            } else {
                0.0
            };
        }
        remaining -= value;
        n += 1.0;
    }
    number(value)
}

pub(super) fn amorlinc(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    amor(ctx, args, false)
}

pub(super) fn amordegrc(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    amor(ctx, args, true)
}

// ---- small helpers -------------------------------------------------------------------------

pub(super) fn pduration(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 3)?;
    let rate = num(ctx, &args[0])?;
    let pv = num(ctx, &args[1])?;
    let fv = num(ctx, &args[2])?;
    if rate <= 0.0 || pv <= 0.0 || fv <= 0.0 {
        return Err(ErrorKind::Num);
    }
    number((crate::xmath::ln(fv) - crate::xmath::ln(pv)) / crate::xmath::ln(1.0 + rate))
}

pub(super) fn rri(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 3)?;
    let n = num(ctx, &args[0])?;
    let pv = num(ctx, &args[1])?;
    let fv = num(ctx, &args[2])?;
    if n <= 0.0 || pv == 0.0 {
        return Err(ErrorKind::Num);
    }
    number(xpow(fv / pv, 1.0 / n)? - 1.0)
}

pub(super) fn ispmt(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 4, 4)?;
    let rate = num(ctx, &args[0])?;
    let per = num(ctx, &args[1])?;
    let nper = num(ctx, &args[2])?;
    let pv = num(ctx, &args[3])?;
    if nper == 0.0 {
        return Err(ErrorKind::DivZero);
    }
    number(-pv * rate * (1.0 - per / nper))
}

pub(super) fn fvschedule(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let pv = num(ctx, &args[0])?;
    let rates = collect(ctx, &args[1..])?;
    let mut v = pv;
    for r in rates {
        v *= 1.0 + r;
    }
    number(v)
}
