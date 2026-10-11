//! Double-double arithmetic (about 106 bits) for the statistical and distribution functions.
//!
//! Excel carries several of its statistical results (`GEOMEAN`, the gamma function family, the incomplete gamma
//! and beta integrals) in more precision than a double, so the last digit of the result is that of the exact
//! value rounded once. These helpers evaluate in double-double and round at the end.
#![allow(
    clippy::approx_constant,
    clippy::excessive_precision,
    clippy::manual_range_contains,
    clippy::needless_range_loop
)]

/// An unevaluated sum `hi + lo` with `|lo| <= ulp(hi) / 2`.
#[derive(Clone, Copy, Debug)]
pub(super) struct Dd {
    pub hi: f64,
    pub lo: f64,
}

pub(super) const fn dd(hi: f64, lo: f64) -> Dd {
    Dd { hi, lo }
}

fn two_sum(a: f64, b: f64) -> Dd {
    let s = a + b;
    let bb = s - a;
    dd(s, (a - (s - bb)) + (b - bb))
}

fn quick_two_sum(a: f64, b: f64) -> Dd {
    let s = a + b;
    dd(s, b - (s - a))
}

fn two_prod(a: f64, b: f64) -> Dd {
    let p = a * b;
    dd(p, a.mul_add(b, -p))
}

pub(super) const LN2: Dd = dd(std::f64::consts::LN_2, 2.319_046_813_846_299_6e-17);

impl Dd {
    pub fn from(x: f64) -> Dd {
        dd(x, 0.0)
    }

    /// The nearest double.
    pub fn to_f64(self) -> f64 {
        self.hi + self.lo
    }

    pub fn neg(self) -> Dd {
        dd(-self.hi, -self.lo)
    }

    pub fn add(self, b: Dd) -> Dd {
        let s = two_sum(self.hi, b.hi);
        let t = two_sum(self.lo, b.lo);
        let u = quick_two_sum(s.hi, s.lo + t.hi);
        quick_two_sum(u.hi, u.lo + t.lo)
    }

    pub fn sub(self, b: Dd) -> Dd {
        self.add(b.neg())
    }

    pub fn sub_f(self, b: f64) -> Dd {
        self.sub(Dd::from(b))
    }

    pub fn mul(self, b: Dd) -> Dd {
        let p = two_prod(self.hi, b.hi);
        quick_two_sum(p.hi, p.lo + (self.hi * b.lo + self.lo * b.hi))
    }

    pub fn mul_f(self, b: f64) -> Dd {
        let p = two_prod(self.hi, b);
        quick_two_sum(p.hi, p.lo + self.lo * b)
    }

    pub fn div(self, b: Dd) -> Dd {
        let q1 = self.hi / b.hi;
        let r = self.sub(b.mul_f(q1));
        let q2 = r.hi / b.hi;
        let r = r.sub(b.mul_f(q2));
        let q3 = r.hi / b.hi;
        quick_two_sum(q1, q2).add(Dd::from(q3))
    }

    pub fn div_f(self, b: f64) -> Dd {
        self.div(Dd::from(b))
    }

    pub fn scale(self, p: f64) -> Dd {
        dd(self.hi * p, self.lo * p)
    }
}

/// `exp(a) = (1 + s) * 2^k`, with `s = expm1(a - k ln 2)`; `a` finite and moderate.
fn exp_parts(a: Dd) -> (Dd, i32) {
    let k = (a.hi / std::f64::consts::LN_2).round();
    let r = a.sub(LN2.mul_f(k)).scale(1.0 / 512.0);
    let mut term = r;
    let mut sum = r;
    for i in 2..=13 {
        term = term.mul(r).div_f(f64::from(i));
        sum = sum.add(term);
    }
    for _ in 0..9 {
        sum = sum.scale(2.0).add(sum.mul(sum));
    }
    (sum, k as i32)
}

fn ldexp(x: f64, k: i32) -> f64 {
    let h = k / 2;
    x * 2f64.powi(h) * 2f64.powi(k - h)
}

/// `exp(a)` as a double-double for `|a| < 700`.
pub(super) fn exp(a: Dd) -> Dd {
    let (s, k) = exp_parts(a);
    Dd::from(1.0).add(s).scale(2f64.powi(k))
}

/// `exp(a)` rounded once; infinity on overflow, zero on underflow.
pub(super) fn exp_f64(a: Dd) -> f64 {
    if a.hi > 709.78 {
        return f64::INFINITY;
    }
    if a.hi < -745.2 {
        return 0.0;
    }
    let (s, k) = exp_parts(a);
    ldexp(Dd::from(1.0).add(s).to_f64(), k)
}

/// `ln(x)` for finite `x > 0`: a Newton step on the double result.
pub(super) fn ln(x: Dd) -> Dd {
    let y0 = x.hi.ln();
    if x.hi == 1.0 && x.lo == 0.0 {
        return Dd::from(0.0);
    }
    // y1 = y0 + x * exp(-y0) - 1
    let e = exp(Dd::from(-y0));
    Dd::from(y0).add(x.mul(e).sub_f(1.0))
}

// ---- more arithmetic and constants ---------------------------------------------------------------

pub(super) const PI: Dd = dd(3.141592653589793, 1.2246467991473532e-16);
pub(super) const SQRT_PI: Dd = dd(1.772453850905516, -7.666586499825799e-17);
pub(super) const SQRT2: Dd = dd(1.4142135623730951, -9.667293313452913e-17);
pub(super) const LN_SQRT_2PI: Dd = dd(0.9189385332046728, -3.8782941580672414e-17);
pub(super) const FRAC_2_SQRT_PI: Dd = dd(1.1283791670955126, 1.533545961316588e-17);
pub(super) const LN_PI: Dd = dd(1.1447298858494002, 1.0265951162707826e-17);

/// `B(2k) / (2k (2k - 1))` for `k = 1..=20`: the Stirling series coefficients.
const STIRLING: [Dd; 20] = [
    dd(0.08333333333333333, 4.625929269271485e-18),
    dd(-0.002777777777777778, 1.0601087908747154e-19),
    dd(0.0007936507936507937, 6.883823317368282e-22),
    dd(-0.0005952380952380953, 5.36938218754726e-20),
    dd(0.0008417508417508417, 3.6870174889237694e-20),
    dd(-0.0019175269175269176, 1.0675702776872475e-19),
    dd(0.00641025641025641, 2.2240044563805217e-19),
    dd(-0.029550653594771242, 4.861760957508855e-19),
    dd(0.17964437236883057, -6.401600482710946e-19),
    dd(-1.3924322169059011, 1.5837056989230303e-17),
    dd(13.402864044168393, -6.154114101993966e-16),
    dd(-156.84828462600203, 9.391823141715389e-15),
    dd(2193.1033333333335, -1.3339255626002948e-13),
    dd(-36108.77125372499, 5.897583353514365e-13),
    dd(691472.268851313, 2.5585296305158e-11),
    dd(-15238221.539407415, -8.76774522490625e-10),
    dd(382900751.39141417, -2.4082684757733585e-08),
    dd(-10882266035.784391, 3.141830930219749e-07),
    dd(347320283765.00226, -6.048528997747748e-06),
    dd(-12369602142269.275, 0.0009363732896507286),
];

impl Dd {
    pub fn abs(self) -> Dd {
        if self.hi < 0.0 { self.neg() } else { self }
    }

    pub fn add_f(self, b: f64) -> Dd {
        self.add(Dd::from(b))
    }

    pub fn lt(self, b: Dd) -> bool {
        self.hi < b.hi || (self.hi == b.hi && self.lo < b.lo)
    }

    pub fn recip(self) -> Dd {
        Dd::from(1.0).div(self)
    }

    pub fn square(self) -> Dd {
        self.mul(self)
    }

    /// Exact scaling by `2^k`.
    pub fn ldexp(self, k: i32) -> Dd {
        let h = k / 2;
        self.scale(2f64.powi(h)).scale(2f64.powi(k - h))
    }
}

/// The exact product of two doubles.
pub(super) fn prod(a: f64, b: f64) -> Dd {
    two_prod(a, b)
}

/// The exact sum of two doubles.
pub(super) fn sum(a: f64, b: f64) -> Dd {
    two_sum(a, b)
}

/// `ln(x)` for any finite `x > 0` (two Newton steps; subnormal and huge arguments are scaled first).
pub(super) fn ln_p(x: Dd) -> Dd {
    if x.hi == 1.0 && x.lo == 0.0 {
        return Dd::from(0.0);
    }
    if !(1e-250..=1e250).contains(&x.hi) {
        let k = x.hi.log2().floor() as i32;
        return ln_p(x.ldexp(-k)).add(LN2.mul_f(f64::from(k)));
    }
    let mut y = Dd::from(x.hi.ln());
    for _ in 0..2 {
        y = y.add(x.mul(exp(y.neg())).sub_f(1.0));
    }
    y
}

/// `exp(a)` as a double-double, saturating: below `-700` only a double is kept.
pub(super) fn exp_sat(a: Dd) -> Dd {
    if a.hi < -700.0 {
        Dd::from(exp_f64(a))
    } else if a.hi > 700.0 {
        Dd::from(f64::INFINITY)
    } else {
        exp(a)
    }
}

/// `ln Gamma(x)` for `x > 0` (Stirling series after shifting the argument above 40).
pub(super) fn ln_gamma(x: Dd) -> Dd {
    if (x.hi == 1.0 || x.hi == 2.0) && x.lo == 0.0 {
        return Dd::from(0.0);
    }
    let mut p = Dd::from(1.0);
    let mut y = x;
    while y.hi < 40.0 {
        p = p.mul(y);
        y = y.add_f(1.0);
    }
    let ly = ln_p(y);
    let mut s = y.sub_f(0.5).mul(ly).sub(y).add(LN_SQRT_2PI);
    let inv = y.recip();
    let inv2 = inv.square();
    let mut pw = inv;
    for c in &STIRLING {
        let t = c.mul(pw);
        s = s.add(t);
        if t.hi.abs() <= s.hi.abs() * 1e-36 {
            break;
        }
        pw = pw.mul(inv2);
    }
    if p.hi == 1.0 && p.lo == 0.0 {
        s
    } else {
        s.sub(ln_p(p))
    }
}

/// `sin(pi * x)` for a finite double `x`, exact at the integers.
pub(super) fn sin_pi(x: f64) -> Dd {
    let n = x.round();
    let f = x - n; // exact
    if f == 0.0 {
        return Dd::from(0.0);
    }
    let z = PI.mul_f(f);
    let z2 = z.square();
    let mut term = z;
    let mut s = z;
    let mut k = 1.0;
    loop {
        term = term.mul(z2).div_f((k + 1.0) * (k + 2.0)).neg();
        k += 2.0;
        s = s.add(term);
        if term.hi.abs() <= s.hi.abs() * 1e-36 {
            break;
        }
    }
    if n.rem_euclid(2.0) == 1.0 { s.neg() } else { s }
}

/// `erf(x)` for `0 <= x < 3`: `2/sqrt(pi) exp(-x^2) sum 2^n x^(2n+1) / (2n+1)!!`, all terms positive.
fn erf_series(x: Dd) -> Dd {
    if x.hi == 0.0 {
        return Dd::from(0.0);
    }
    let x2 = x.square();
    let mut term = x;
    let mut s = x;
    let mut n = 1.0;
    loop {
        term = term.mul(x2).scale(2.0).div_f(2.0 * n + 1.0);
        s = s.add(term);
        n += 1.0;
        if term.hi <= s.hi * 1e-36 {
            break;
        }
    }
    FRAC_2_SQRT_PI.mul(exp(x2.neg())).mul(s)
}

/// `erf(x)` for `0 <= x < 3` in double-double.
pub(super) fn erf_small(x: Dd) -> Dd {
    erf_series(x)
}

/// The continued-fraction denominator `x + (1/2)/(x + 1/(x + (3/2)/(x + ...)))` for `x >= 3`.
fn erfc_denominator(x: Dd) -> Dd {
    let n = if x.hi < 4.0 {
        170
    } else if x.hi < 8.0 {
        80
    } else {
        40
    };
    let mut t = x;
    for k in (1..=n).rev() {
        t = x.add(Dd::from(f64::from(k) / 2.0).div(t));
    }
    t
}

/// `erfc(x)` for `x >= 0`, with a relative accuracy near 1e-30.
pub(super) fn erfc(x: Dd) -> Dd {
    if x.hi < 3.0 {
        return Dd::from(1.0).sub(erf_series(x));
    }
    if x.hi > 27.3 {
        return Dd::from(0.0);
    }
    // erfc(x) = exp(-x^2) / (sqrt(pi) (x + (1/2)/(x + 1/(x + (3/2)/(x + ...)))))
    exp_sat(x.square().neg()).div(SQRT_PI.mul(erfc_denominator(x)))
}

/// The scaled `exp(x^2) erfc(x)` for `x >= 0`, free of the overflow of `exp(x^2)`.
pub(super) fn erfcx(x: Dd) -> Dd {
    if x.hi < 3.0 {
        return Dd::from(1.0).sub(erf_series(x)).mul(exp(x.square()));
    }
    SQRT_PI.mul(erfc_denominator(x)).recip()
}

/// `(P(Z <= z), P(Z > z))` for the standard normal, both with full relative accuracy.
pub(super) fn norm_cdf(z: Dd) -> (Dd, Dd) {
    let tail = erfc(z.abs().div(SQRT2)).scale(0.5);
    let rest = Dd::from(1.0).sub(tail);
    if z.hi < 0.0 {
        (tail, rest)
    } else {
        (rest, tail)
    }
}

/// The standard normal density.
pub(super) fn norm_pdf(z: Dd) -> Dd {
    let h = z.square().scale(0.5);
    exp_sat(h.neg()).div(SQRT2.mul(SQRT_PI))
}

const EPS: f64 = 1e-34;
const MAX_ITER: u32 = 400_000;

/// Regularized incomplete gamma `(P(a, x), Q(a, x))` for `a > 0`, `x >= 0`.
pub(super) fn gamma_pq(a: Dd, x: Dd) -> Option<(Dd, Dd)> {
    if !(a.hi.is_finite() && x.hi.is_finite()) || x.hi < 0.0 || a.hi <= 0.0 {
        return None;
    }
    if x.hi == 0.0 {
        return Some((Dd::from(0.0), Dd::from(1.0)));
    }
    let l = a.mul(ln_p(x)).sub(x).sub(ln_gamma(a));
    if x.lt(a.add_f(1.0)) {
        let mut t = a.recip();
        let mut s = t;
        let mut i = 0;
        loop {
            i += 1;
            if i > MAX_ITER {
                return None;
            }
            t = t.mul(x).div(a.add_f(f64::from(i)));
            s = s.add(t);
            if t.hi <= s.hi * EPS {
                break;
            }
        }
        let p = exp_sat(l).mul(s);
        Some((p, Dd::from(1.0).sub(p)))
    } else {
        let tiny = 1e-290;
        let mut b = x.add_f(1.0).sub(a);
        let mut c = Dd::from(1.0 / tiny);
        let mut d = b.recip();
        let mut h = d;
        let mut i = 0;
        loop {
            i += 1;
            if i > MAX_ITER {
                return None;
            }
            let fi = f64::from(i);
            let an = a.sub_f(fi).mul_f(fi);
            b = b.add_f(2.0);
            d = an.mul(d).add(b);
            if d.hi.abs() < tiny {
                d = Dd::from(tiny);
            }
            c = b.add(an.div(c));
            if c.hi.abs() < tiny {
                c = Dd::from(tiny);
            }
            d = d.recip();
            let del = d.mul(c);
            h = h.mul(del);
            if del.sub_f(1.0).hi.abs() < EPS {
                break;
            }
        }
        let q = exp_sat(l).mul(h);
        Some((Dd::from(1.0).sub(q), q))
    }
}

/// The continued fraction of the incomplete beta function (modified Lentz).
fn beta_cf(a: Dd, b: Dd, x: Dd) -> Option<Dd> {
    let tiny = 1e-290;
    let qab = a.add(b);
    let qap = a.add_f(1.0);
    let qam = a.sub_f(1.0);
    let one = Dd::from(1.0);
    let guard = |v: Dd| if v.hi.abs() < tiny { Dd::from(tiny) } else { v };
    let mut c = one;
    let mut d = guard(one.sub(qab.mul(x).div(qap))).recip();
    let mut h = d;
    let mut m = 0.0;
    loop {
        m += 1.0;
        if m > f64::from(MAX_ITER) {
            return None;
        }
        let m2 = 2.0 * m;
        let aa = b
            .sub_f(m)
            .mul_f(m)
            .mul(x)
            .div(qam.add_f(m2).mul(a.add_f(m2)));
        d = guard(one.add(aa.mul(d))).recip();
        c = guard(one.add(aa.div(c)));
        h = h.mul(d).mul(c);
        let aa = a
            .add_f(m)
            .mul(qab.add_f(m))
            .mul(x)
            .div(a.add_f(m2).mul(qap.add_f(m2)))
            .neg();
        d = guard(one.add(aa.mul(d))).recip();
        c = guard(one.add(aa.div(c)));
        let del = d.mul(c);
        h = h.mul(del);
        if del.sub_f(1.0).hi.abs() < EPS {
            break;
        }
    }
    Some(h)
}

/// Regularized incomplete beta `(I_x(a, b), 1 - I_x(a, b))` for `a, b > 0`, with `y = 1 - x` supplied
/// accurately by the caller (`x` and `y` in `[0, 1]`).
pub(super) fn beta_pq(a: Dd, b: Dd, x: Dd, y: Dd) -> Option<(Dd, Dd)> {
    if !(a.hi.is_finite() && b.hi.is_finite() && x.hi.is_finite() && y.hi.is_finite())
        || a.hi <= 0.0
        || b.hi <= 0.0
        || x.hi < 0.0
        || y.hi < 0.0
    {
        return None;
    }
    if x.hi <= 0.0 {
        return Some((Dd::from(0.0), Dd::from(1.0)));
    }
    if y.hi <= 0.0 {
        return Some((Dd::from(1.0), Dd::from(0.0)));
    }
    let l = a
        .mul(ln_p(x))
        .add(b.mul(ln_p(y)))
        .add(ln_gamma(a.add(b)))
        .sub(ln_gamma(a))
        .sub(ln_gamma(b));
    if x.lt(a.add_f(1.0).div(a.add(b).add_f(2.0))) {
        let p = exp_sat(l).div(a).mul(beta_cf(a, b, x)?);
        Some((p, Dd::from(1.0).sub(p)))
    } else {
        let q = exp_sat(l).div(b).mul(beta_cf(b, a, y)?);
        Some((Dd::from(1.0).sub(q), q))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Relative difference between a double-double and a reference given as `(hi, lo)`.
    fn rel(a: Dd, want: (f64, f64)) -> f64 {
        let d = a.sub(dd(want.0, want.1));
        (d.hi / want.0).abs()
    }

    #[test]
    fn erfc_matches_reference() {
        let cases = [
            (0.5, (0.4795001221869535, -1.900077467916287e-17)),
            (3.0, (2.209049699858544e-05, 1.5563377960343457e-22)),
            (3.5, (7.430983723414128e-07, -3.117067749063089e-23)),
            (5.0, (1.537459794428035e-12, -8.569418222079096e-29)),
            (10.0, (2.088487583762545e-45, -1.2006565763501381e-61)),
        ];
        for (x, want) in cases {
            assert!(rel(erfc(Dd::from(x)), want) < 1e-29, "erfc({x})");
        }
    }

    #[test]
    fn ln_gamma_matches_reference() {
        let cases = [
            (0.3, (1.0957979948180756, -4.412082839547077e-17)),
            (1.5, (-0.12078223763524522, -4.1797047492946264e-18)),
            (7.7, (7.926541356269005, -2.217616680615796e-17)),
            (50.5, (146.51925549072064, -1.1368682497816444e-14)),
            (1000.1, (5905.91115373108, 2.097470544441104e-13)),
        ];
        for (x, want) in cases {
            assert!(rel(ln_gamma(Dd::from(x)), want) < 1e-29, "lgamma({x})");
        }
    }

    #[test]
    fn incomplete_gamma_and_beta_match_reference() {
        let (p, q) = gamma_pq(Dd::from(2.5), Dd::from(3.0)).unwrap();
        assert!(rel(p, (0.6937810815867216, 1.7220885921402372e-17)) < 1e-29);
        assert!(rel(q, (0.3062189184132784, -1.7220885921402372e-17)) < 1e-29);
        let (p, q) = gamma_pq(Dd::from(40.0), Dd::from(60.0)).unwrap();
        assert!(rel(p, (0.9974518076963866, 3.8942608953214374e-17)) < 1e-29);
        assert!(rel(q, (0.0025481923036133223, 8.866925626378361e-20)) < 1e-29);
        let (p, _) = beta_pq(
            Dd::from(2.0),
            Dd::from(3.0),
            Dd::from(0.375),
            Dd::from(0.625),
        )
        .unwrap();
        assert!(rel(p, (0.481201171875, 0.0)) < 1e-29);
        let (p, q) = beta_pq(
            Dd::from(0.5),
            Dd::from(7.0),
            Dd::from(0.875),
            Dd::from(0.125),
        )
        .unwrap();
        assert!(rel(p, (0.9999998941506186, 2.452982861161787e-17)) < 1e-29);
        assert!(rel(q, (1.0584938134650723e-07, -5.922039548449696e-24)) < 1e-29);
        let (p, q) = beta_pq(
            Dd::from(40.0),
            Dd::from(60.0),
            Dd::from(0.375),
            Dd::from(0.625),
        )
        .unwrap();
        assert!(rel(p, (0.3087821967608015, 7.815417315219801e-18)) < 1e-29);
        assert!(rel(q, (0.6912178032391985, 4.769573391603803e-17)) < 1e-29);
    }
}
