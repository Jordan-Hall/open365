//! Correctly rounded elementary functions for the formula engine.
//!
//! Excel's `LN`, `LOG10`, `EXP` and the trigonometric functions are accurate to the last bit (the CRT and
//! Rust's `libm` are not: `LN(1.05)` is 0.04879016416943205 in Excel and `...204` from the C library). This module
//! evaluates every function in double-double arithmetic (about 106 bits) and rounds once, so the result is the
//! correctly rounded double except for the rare argument whose exact value lies within 2^-100 of a rounding
//! boundary. Pure safe Rust, no tables, no dependencies.

/// An unevaluated sum `hi + lo` with `|lo| <= ulp(hi) / 2`.
#[derive(Clone, Copy, Debug)]
pub struct Dd {
    hi: f64,
    lo: f64,
}

const fn dd(hi: f64, lo: f64) -> Dd {
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

impl Dd {
    fn from(x: f64) -> Dd {
        dd(x, 0.0)
    }

    /// The nearest double.
    pub fn to_f64(self) -> f64 {
        self.hi + self.lo
    }

    fn neg(self) -> Dd {
        dd(-self.hi, -self.lo)
    }

    fn add(self, b: Dd) -> Dd {
        let s = two_sum(self.hi, b.hi);
        let t = two_sum(self.lo, b.lo);
        let u = quick_two_sum(s.hi, s.lo + t.hi);
        quick_two_sum(u.hi, u.lo + t.lo)
    }

    fn sub(self, b: Dd) -> Dd {
        self.add(b.neg())
    }

    fn mul(self, b: Dd) -> Dd {
        let p = two_prod(self.hi, b.hi);
        quick_two_sum(p.hi, p.lo + (self.hi * b.lo + self.lo * b.hi))
    }

    fn mul_f(self, b: f64) -> Dd {
        let p = two_prod(self.hi, b);
        quick_two_sum(p.hi, p.lo + self.lo * b)
    }

    fn div(self, b: Dd) -> Dd {
        let q1 = self.hi / b.hi;
        let r = self.sub(b.mul_f(q1));
        let q2 = r.hi / b.hi;
        let r = r.sub(b.mul_f(q2));
        let q3 = r.hi / b.hi;
        quick_two_sum(q1, q2).add(Dd::from(q3))
    }

    fn div_f(self, b: f64) -> Dd {
        self.div(Dd::from(b))
    }

    /// Multiplication by a power of two (exact).
    fn scale(self, p: f64) -> Dd {
        dd(self.hi * p, self.lo * p)
    }
}

const LN2: Dd = dd(std::f64::consts::LN_2, 2.3190468138462996e-17);
const LN10: Dd = dd(std::f64::consts::LN_10, -2.1707562233822494e-16);
const PI: Dd = dd(std::f64::consts::PI, 1.2246467991473532e-16);

/// Pi with the 64 mantissa bits Excel's trigonometric argument reduction uses (`0xC90FDAA22168C235 * 2^-62`).
const PI64: Dd = dd(
    f64::from_bits(0x4009_21FB_5444_2D18),
    565.0 / 4_611_686_018_427_387_904.0, // 565 * 2^-62
);

// ---- exp / ln -------------------------------------------------------------------------------

/// `(expm1 part, k)`: `exp(a) = (1 + s) * 2^k` with `s = expm1(a - k ln 2)`; `a` must be finite and moderate.
fn exp_parts(a: Dd) -> (Dd, i32) {
    let k = (a.hi / LN2.hi).round();
    let r = a.sub(LN2.mul_f(k)).scale(1.0 / 512.0);
    // Taylor series of expm1(r) for |r| <= 7e-4.
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
    // Two steps keep the intermediate scale factors finite.
    let h = k / 2;
    x * 2f64.powi(h) * 2f64.powi(k - h)
}

/// `exp(x) - 1` as a double-double, for `|x| < 700`, relatively accurate for small `x`.
fn expm1_dd(a: Dd) -> Dd {
    let (s, k) = exp_parts(a);
    if k == 0 {
        s
    } else {
        Dd::from(1.0)
            .add(s)
            .scale(2f64.powi(k.clamp(-1000, 1000)))
            .sub(Dd::from(1.0))
    }
}

/// `exp(x)`, correctly rounded. Overflow is infinity and underflow is 0 (subnormals included, rounded once).
pub fn exp(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x > 709.782_712_893_384 {
        return f64::INFINITY;
    }
    if x < -745.2 {
        return 0.0;
    }
    let (s, k) = exp_parts(Dd::from(x));
    let e = Dd::from(1.0).add(s);
    ldexp(e.to_f64(), k)
}

/// `ln(x)` as a double-double for finite `x > 0`.
fn ln_dd(x: f64) -> Dd {
    let mut e = 0i32;
    let mut m = x;
    if m < f64::MIN_POSITIVE {
        m *= 2f64.powi(64);
        e -= 64;
    }
    // m = f * 2^exp with f in [sqrt(1/2), sqrt(2)).
    let bits = m.to_bits();
    let mut exp = ((bits >> 52) & 0x7FF) as i32 - 1023;
    let mut f = f64::from_bits((bits & 0x000F_FFFF_FFFF_FFFF) | 0x3FF0_0000_0000_0000);
    if f > std::f64::consts::SQRT_2 {
        f *= 0.5;
        exp += 1;
    }
    e += exp;
    // ln f = 2 atanh(z), z = (f - 1) / (f + 1).
    let z = Dd::from(f - 1.0).div(two_sum(f, 1.0));
    let z2 = z.mul(z);
    let mut term = z;
    let mut sum = z;
    for k in 1..=26 {
        term = term.mul(z2);
        sum = sum.add(term.div_f(f64::from(2 * k + 1)));
    }
    sum.scale(2.0).add(LN2.mul_f(f64::from(e)))
}

/// Natural logarithm, correctly rounded (the C library result for zero, negative and non-finite `x`).
pub fn ln(x: f64) -> f64 {
    if x == 1.0 || !(x > 0.0 && x.is_finite()) {
        return x.ln();
    }
    ln_dd(x).to_f64()
}

/// Base-10 logarithm, correctly rounded (the C library result for zero, negative and non-finite `x`).
pub fn log10(x: f64) -> f64 {
    if x == 1.0 || !(x > 0.0 && x.is_finite()) {
        return x.log10();
    }
    ln_dd(x).div(LN10).to_f64()
}

/// Excel's `x^n` for an integer `n` (|n| < 2^63): right-to-left binary exponentiation in doubles, one
/// division at the end for a negative `n`. Measured bit for bit against Excel (240 random cases).
pub fn pow_int(x: f64, n: f64) -> f64 {
    let mut m = n.abs() as u64;
    if x == 10.0 {
        // Powers of ten are correctly rounded in Excel (`10^308` is exactly 1E+308); a negative exponent is the
        // reciprocal of the rounded positive power.
        let p = pow10(m);
        return if n < 0.0 { 1.0 / p } else { p };
    }
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
}

/// `10^m`, correctly rounded (infinity from `m > 308`).
fn pow10(m: u64) -> f64 {
    if m > 308 {
        return f64::INFINITY;
    }
    let mut result = Dd::from(1.0);
    let mut base = Dd::from(10.0);
    let mut k = m;
    while k > 0 {
        if k & 1 == 1 {
            result = result.mul(base);
        }
        base = base.mul(base);
        k >>= 1;
    }
    result.to_f64()
}

/// Excel's `x^y` for positive finite `x` and a fractional `y`: `exp(y ln x)` with a correctly rounded `ln`
/// and `exp`, the reciprocal of `exp(-y ln x)` for a negative `y` (measured: 240 of 240 random cases).
pub fn pow_frac(x: f64, y: f64) -> f64 {
    if y == 0.5 {
        x.sqrt()
    } else if y < 0.0 {
        1.0 / exp(-y * ln(x))
    } else {
        exp(y * ln(x))
    }
}

/// `exp(x) - 1`, correctly rounded, for `|x| < 700`.
pub fn exp_m1(x: f64) -> f64 {
    expm1_dd(Dd::from(x)).to_f64()
}

// ---- hyperbolic -----------------------------------------------------------------------------

/// Excel's `SINH`: `(e^x - e^-x) / 2` for `|x| >= 1`, a correctly rounded value below (see the conformance doc
/// for the remaining last-digit differences below 1). `|x|` must be below 710.
pub fn sinh(x: f64) -> f64 {
    if x.abs() < 1.0 {
        let a = Dd::from(x.abs());
        let em1 = expm1_dd(a);
        let r = em1.add(em1.div(em1.add(Dd::from(1.0)))).scale(0.5).to_f64();
        return if x < 0.0 { -r } else { r };
    }
    (exp(x) - exp(-x)) / 2.0
}

/// Excel's `COSH`: `(e^x + e^-x) / 2` (160 of 160 random cases).
pub fn cosh(x: f64) -> f64 {
    (exp(x) + exp(-x)) / 2.0
}

/// Excel's `TANH`: `sinh(x) / cosh(x)` with the two functions above (the large-argument part is exact in 87 of 87
/// random cases, the part below 1 matches 49 of 73).
pub fn tanh(x: f64) -> f64 {
    if x.abs() > 22.0 {
        return x.signum();
    }
    sinh(x) / cosh(x)
}

// ---- sine and cosine ------------------------------------------------------------------------

/// `(sin a, cos a)` for `|a| <= 0.8` by Taylor series.
fn sin_cos_small(a: Dd) -> (Dd, Dd) {
    let a2 = a.mul(a);
    let mut st = a;
    let mut s = a;
    let mut ct = Dd::from(1.0);
    let mut c = ct;
    for k in 1..=17 {
        let n1 = f64::from(2 * k - 1);
        let n2 = f64::from(2 * k);
        let n3 = f64::from(2 * k + 1);
        ct = ct.mul(a2).div_f(n1 * n2).neg();
        c = c.add(ct);
        st = st.mul(a2).div_f(n2 * n3).neg();
        s = s.add(st);
    }
    (s, c)
}

/// `(sin a, cos a)` for `|a| <= pi` (a is reduced further to `|r| <= 0.8` by quarter turns).
fn sin_cos_dd(a: Dd) -> (Dd, Dd) {
    // a = q * pi/2 + r, |r| <= pi/4 (+ rounding); quadrants swap and negate.
    let half_pi = PI.scale(0.5);
    let q = (a.hi / half_pi.hi).round();
    let r = a.sub(half_pi.mul_f(q));
    let (s, c) = sin_cos_small(r);
    match (q as i64).rem_euclid(4) {
        0 => (s, c),
        1 => (c, s.neg()),
        2 => (s.neg(), c.neg()),
        _ => (c.neg(), s),
    }
}

/// Reduces `x` (`|x| < 2^27`) to `(r, quadrant)` with `x = r + quadrant * pi/2` using the 64-bit pi
/// Excel uses, keeping `r` in double-double.
fn reduce64(x: f64) -> (Dd, i64) {
    let half = PI64.scale(0.5);
    let k = (x / half.hi).round();
    (Dd::from(x).sub(half.mul_f(k)), k as i64)
}

/// `sin`, `cos` and `tan` of a reduction of `x` by Excel's 64-bit pi, evaluated without rounding the
/// reduced argument. `x` must satisfy `|x| < 2^27`.
pub fn sin_cos_tan(x: f64) -> (f64, f64, f64) {
    let (r, k) = reduce64(x);
    let (s, c) = sin_cos_small(r);
    let (s, c) = match k.rem_euclid(4) {
        0 => (s, c),
        1 => (c, s.neg()),
        2 => (s.neg(), c.neg()),
        _ => (c.neg(), s),
    };
    (s.to_f64(), c.to_f64(), s.div(c).to_f64())
}

// ---- inverse trigonometric -------------------------------------------------------------------

fn atan_dd(x: Dd) -> Dd {
    let mut y = Dd::from(x.hi.atan());
    for _ in 0..2 {
        // y -= (sin y - x cos y) cos y / (cos^2 y) ... Newton on tan y = x.
        let (s, c) = sin_cos_dd(y);
        let num = s.sub(x.mul(c));
        let den = c.add(x.mul(s));
        y = y.sub(num.div(den));
    }
    y
}

/// Correctly rounded `atan(x)`.
pub fn atan(x: f64) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x.atan();
    }
    atan_dd(Dd::from(x)).to_f64()
}

/// Excel's `ASIN`: `atan(x / sqrt((1 - x)(1 + x)))` with a correctly rounded `atan` (120 of 120 random cases).
pub fn asin(x: f64) -> f64 {
    if x == 0.0 {
        return x;
    }
    if x.abs() == 1.0 {
        return std::f64::consts::FRAC_PI_2.copysign(x);
    }
    atan(x / ((1.0 - x) * (1.0 + x)).sqrt())
}

/// Excel's `ACOS`: `pi/2 - asin(x)` in doubles (120 of 120 random cases).
pub fn acos(x: f64) -> f64 {
    std::f64::consts::FRAC_PI_2 - asin(x)
}

/// Excel's `ATAN2(x, y)` in the C argument order `(y, x)`: `atan(y / x)`, plus or minus pi for `x < 0`.
pub fn atan2(y: f64, x: f64) -> f64 {
    if x == 0.0 || y == 0.0 || !x.is_finite() || !y.is_finite() {
        return y.atan2(x);
    }
    let a = atan(y / x);
    if x > 0.0 {
        a
    } else if y > 0.0 {
        a + std::f64::consts::PI
    } else {
        a - std::f64::consts::PI
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_are_consistent() {
        // exp(ln 2) = 2, exp(ln 10) = 10 to double-double accuracy.
        let e2 = expm1_dd(LN2);
        assert!((e2.hi - 1.0).abs() < 1e-30 && e2.lo.abs() < 1e-30, "{e2:?}");
        let e10 = expm1_dd(LN10);
        assert!((e10.hi - 9.0).abs() < 1e-29, "{e10:?}");
        let (s, c) = sin_cos_dd(PI.scale(0.5));
        assert!(
            (s.hi - 1.0).abs() < 1e-30 && c.hi.abs() < 1e-30,
            "{s:?} {c:?}"
        );
    }

    #[test]
    fn excel_ln_is_correctly_rounded() {
        assert_eq!(ln(1.05), 0.04879016416943205);
        assert_eq!(ln(2.0), std::f64::consts::LN_2);
        assert_eq!(ln(10.0), std::f64::consts::LN_10);
        assert_eq!(ln(1e-300), -690.7755278982137);
        assert_eq!(log10(1000.0), 3.0);
        assert_eq!(log10(1e-300), -300.0);
        assert_eq!(exp(1.0), std::f64::consts::E);
        assert_eq!(exp(0.0), 1.0);
        assert_eq!(exp(-745.0), 5e-324);
    }

    #[test]
    fn inverse_functions_round_trip() {
        assert_eq!(atan(1.0), std::f64::consts::FRAC_PI_4);
        assert_eq!(asin(1.0), std::f64::consts::FRAC_PI_2);
        // Excel computes both from atan, so they are within an ulp of the exact values, not always equal.
        assert!((asin(0.5) - std::f64::consts::FRAC_PI_6).abs() < 2e-16);
        assert!((acos(0.5) - std::f64::consts::FRAC_PI_3).abs() < 3e-16);
        assert_eq!(atan2(1.0, -1.0), 2.356194490192345);
        assert_eq!(atan2(-1.0, -1.0), -2.356194490192345);
        let (s, c, t) = sin_cos_tan(1.0);
        assert_eq!(
            (s, c, t),
            (0.8414709848078965, 0.5403023058681398, 1.5574077246549023)
        );
        assert_eq!(sin_cos_tan(std::f64::consts::PI).0, 1.22514845490862e-16);
        assert_eq!(sinh(1.0), 1.1752011936438014);
        assert_eq!(sinh(0.5), 0.5210953054937474);
        assert_eq!(cosh(1.0), 1.5430806348152437);
        assert_eq!(tanh(1.0), 0.7615941559557649);
        assert_eq!(pow_frac(2.0, 0.5), std::f64::consts::SQRT_2);
        assert_eq!(pow_int(2.0, -3.0), 0.125);
    }
}
