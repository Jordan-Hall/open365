//! `BESSELI`, `BESSELJ`, `BESSELK` and `BESSELY`.
//!
//! Excel does not return the exact values: its routines are the rational and polynomial approximations of
//! *Numerical Recipes* (2nd ed., `bessj0`, `bessj1`, `bessy0`, `bessy1`, `bessi0`, `bessi1`, `bessk0`, `bessk1`,
//! switching from the rational form to the asymptotic one at `|x| = 8` for `J` and `Y`, at 3.75 for `I`, at 2 for
//! `K`) with the recurrences `bessj`/`bessi` (Miller's downward algorithm, accuracy parameter 40, rescaling at
//! 1E10) and the plain upward recurrences for `K` and `Y`. They are accurate to about 1E-8. Measured against
//! Excel 16.0 (groups `besseli`..`bessely`): `BESSELI`, `BESSELK` and `BESSELY` are bit-identical to this model,
//! and so is `BESSELJ` below 8. `BESSELJ` from 8 on differs: Excel's `J0` asymptotic series has the coefficient
//! `-0.1098628267e-2` where the published one is `-0.1098628627e-2` (used here, found by regression on 100 values),
//! and its `J1` series carries an error of 8E-6 at x = 8 that decays like `x^-6.5`. A least-squares fit of the last three `J1` coefficients to the 35 direct `BESSELJ(x,1)` values removes
//! that error (residual 1E-10) but makes 18 `n >= 2` cases worse, so the published constants stay.
//! The constants are the published approximation constants, not Excel code.

// The coefficients are Excel's truncated approximation constants (0.636619772 is *not* 2/pi): they must stay as is.
#![allow(clippy::approx_constant)]

use super::distribdd::{self as dd, Dd};
use super::{Res, arity, number, scalar};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Kind {
    I,
    J,
    K,
    Y,
}

/// Excel's `EXP` is correctly rounded.
fn exp(x: f64) -> f64 {
    dd::exp_f64(Dd::from(x))
}

const ACC: f64 = 40.0;
const BIGNO: f64 = 1.0e10;
const BIGNI: f64 = 1.0e-10;
/// Longest downward recurrence run; orders beyond it are `#NUM!`.
const MAX_STEPS: u64 = 40_000_000;

fn j0(x: f64) -> f64 {
    let ax = x.abs();
    if ax < 8.0 {
        let y = x * x;
        let a1 = 57_568_490_574.0
            + y * (-13_362_590_354.0
                + y * (651_619_640.7
                    + y * (-11_214_424.18 + y * (77_392.330_17 + y * (-184.905_245_6)))));
        let a2 = 57_568_490_411.0
            + y * (1_029_532_985.0
                + y * (9_494_680.718 + y * (59_272.648_53 + y * (267.853_271_2 + y))));
        a1 / a2
    } else {
        let z = 8.0 / ax;
        let y = z * z;
        let xx = ax - 0.785_398_164;
        let a1 = 1.0
            + y * (-0.109_862_826_7e-2
                + y * (0.273_451_040_7e-4 + y * (-0.207_337_063_9e-5 + y * 0.209_388_721_1e-6)));
        let a2 = -0.156_249_999_5e-1
            + y * (0.143_048_876_5e-3
                + y * (-0.691_114_765_1e-5 + y * (0.762_109_516_1e-6 - y * 0.934_935_152e-7)));
        (0.636_619_772 / ax).sqrt() * (xx.cos() * a1 - z * xx.sin() * a2)
    }
}

fn j1(x: f64) -> f64 {
    let ax = x.abs();
    if ax < 8.0 {
        let y = x * x;
        let a1 = x
            * (72_362_614_232.0
                + y * (-7_895_059_235.0
                    + y * (242_396_853.1
                        + y * (-2_972_611.439 + y * (15_704.482_60 + y * (-30.160_366_06))))));
        let a2 = 144_725_228_442.0
            + y * (2_300_535_178.0
                + y * (18_583_304.74 + y * (99_447.433_94 + y * (376.999_139_7 + y))));
        a1 / a2
    } else {
        let z = 8.0 / ax;
        let y = z * z;
        let xx = ax - 2.356_194_491;
        let a1 = 1.0
            + y * (0.183_105e-2
                + y * (-0.351_639_649_6e-4 + y * (0.245_752_017_4e-5 + y * (-0.240_337_019e-6))));
        let a2 = 0.046_874_999_95
            + y * (-0.200_269_087_3e-3
                + y * (0.844_919_909_6e-5 + y * (-0.882_289_87e-6 + y * 0.105_787_412e-6)));
        let a = (0.636_619_772 / ax).sqrt() * (xx.cos() * a1 - z * xx.sin() * a2);
        if x < 0.0 { -a } else { a }
    }
}

fn y0(x: f64) -> f64 {
    if x < 8.0 {
        let y = x * x;
        let a1 = -2_957_821_389.0
            + y * (7_062_834_065.0
                + y * (-512_359_803.6
                    + y * (10_879_881.29 + y * (-86_327.927_57 + y * 228.462_273_3))));
        let a2 = 40_076_544_269.0
            + y * (745_249_964.8
                + y * (7_189_466.438 + y * (47_447.264_70 + y * (226.103_024_4 + y))));
        (a1 / a2) + 0.636_619_772 * j0(x) * x.ln()
    } else {
        let z = 8.0 / x;
        let y = z * z;
        let xx = x - 0.785_398_164;
        let a1 = 1.0
            + y * (-0.109_862_862_7e-2
                + y * (0.273_451_040_7e-4 + y * (-0.207_337_063_9e-5 + y * 0.209_388_721_1e-6)));
        let a2 = -0.156_249_999_5e-1
            + y * (0.143_048_876_5e-3
                + y * (-0.691_114_765_1e-5 + y * (0.762_109_516_1e-6 + y * (-0.934_945_152e-7))));
        (0.636_619_772 / x).sqrt() * (xx.sin() * a1 + z * xx.cos() * a2)
    }
}

fn y1(x: f64) -> f64 {
    if x < 8.0 {
        let y = x * x;
        let a1 = x
            * (-0.490_060_494_3e13
                + y * (0.127_527_439_0e13
                    + y * (-0.515_343_813_9e11
                        + y * (0.734_926_455_1e9
                            + y * (-0.423_792_272_6e7 + y * 0.851_193_793_5e4)))));
        let a2 = 0.249_958_057_0e14
            + y * (0.424_441_966_4e12
                + y * (0.373_365_036_7e10
                    + y * (0.224_590_400_2e8
                        + y * (0.102_042_605_0e6 + y * (0.354_963_288_5e3 + y)))));
        (a1 / a2) + 0.636_619_772 * (j1(x) * x.ln() - 1.0 / x)
    } else {
        let z = 8.0 / x;
        let y = z * z;
        let xx = x - 2.356_194_491;
        let a1 = 1.0
            + y * (0.183_105e-2
                + y * (-0.351_639_649_6e-4 + y * (0.245_752_017_4e-5 + y * (-0.240_337_019e-6))));
        let a2 = 0.046_874_999_95
            + y * (-0.200_269_087_3e-3
                + y * (0.844_919_909_6e-5 + y * (-0.882_289_87e-6 + y * 0.105_787_412e-6)));
        (0.636_619_772 / x).sqrt() * (xx.sin() * a1 + z * xx.cos() * a2)
    }
}

fn i0(x: f64) -> f64 {
    let ax = x.abs();
    if ax < 3.75 {
        let y = (x / 3.75) * (x / 3.75);
        1.0 + y
            * (3.515_622_9
                + y * (3.089_942_4
                    + y * (1.206_749_2 + y * (0.265_973_2 + y * (0.360_768e-1 + y * 0.458_13e-2)))))
    } else {
        let y = 3.75 / ax;
        (exp(ax) / ax.sqrt())
            * (0.398_942_28
                + y * (0.132_859_2e-1
                    + y * (0.225_319e-2
                        + y * (-0.157_565e-2
                            + y * (0.916_281e-2
                                + y * (-0.205_770_6e-1
                                    + y * (0.263_553_7e-1
                                        + y * (-0.164_763_3e-1 + y * 0.392_377e-2))))))))
    }
}

fn i1(x: f64) -> f64 {
    let ax = x.abs();
    if ax < 3.75 {
        let y = (x / 3.75) * (x / 3.75);
        let a = ax
            * (0.5
                + y * (0.878_905_94
                    + y * (0.514_988_69
                        + y * (0.150_849_34
                            + y * (0.265_873_3e-1 + y * (0.301_532e-2 + y * 0.324_11e-3))))));
        if x < 0.0 { -a } else { a }
    } else {
        let y = 3.75 / ax;
        let t = 0.228_296_7e-1 + y * (-0.289_531_2e-1 + y * (0.178_765_4e-1 - y * 0.420_059e-2));
        let t = 0.398_942_28
            + y * (-0.398_802_4e-1
                + y * (-0.362_018e-2 + y * (0.163_801e-2 + y * (-0.103_155_5e-1 + y * t))));
        let a = t * (exp(ax) / ax.sqrt());
        if x < 0.0 { -a } else { a }
    }
}

fn k0(x: f64) -> f64 {
    if x <= 2.0 {
        let y = x * x / 4.0;
        (-(x / 2.0).ln() * i0(x))
            + (-0.577_215_66
                + y * (0.422_784_20
                    + y * (0.230_697_56
                        + y * (0.348_859_0e-1
                            + y * (0.262_698e-2 + y * (0.107_50e-3 + y * 0.74e-5))))))
    } else {
        let y = 2.0 / x;
        (exp(-x) / x.sqrt())
            * (1.253_314_14
                + y * (-0.783_235_8e-1
                    + y * (0.218_956_8e-1
                        + y * (-0.106_244_6e-1
                            + y * (0.587_872e-2 + y * (-0.251_540e-2 + y * 0.532_08e-3))))))
    }
}

fn k1(x: f64) -> f64 {
    if x <= 2.0 {
        let y = x * x / 4.0;
        ((x / 2.0).ln() * i1(x))
            + (1.0 / x)
                * (1.0
                    + y * (0.154_431_44
                        + y * (-0.672_785_79
                            + y * (-0.181_568_97
                                + y * (-0.191_940_2e-1 + y * (-0.110_404e-2 + y * (-0.4686e-4)))))))
    } else {
        let y = 2.0 / x;
        (exp(-x) / x.sqrt())
            * (1.253_314_14
                + y * (0.234_986_19
                    + y * (-0.365_562_0e-1
                        + y * (0.150_426_8e-1
                            + y * (-0.780_353e-2 + y * (0.325_614e-2 + y * (-0.682_45e-3)))))))
    }
}

fn bessel_i(x: f64, n: u64) -> Res<f64> {
    match n {
        0 => return Ok(i0(x)),
        1 => return Ok(i1(x)),
        _ => {}
    }
    if x == 0.0 {
        return Ok(0.0);
    }
    let tox = 2.0 / x.abs();
    let (mut bip, mut ans, mut bi) = (0.0f64, 0.0f64, 1.0f64);
    let start = 2 * (n + (ACC * n as f64).sqrt() as u64);
    if start > MAX_STEPS {
        return Err(ErrorKind::Num);
    }
    let mut j = start;
    while j > 0 {
        let bim = bip + j as f64 * tox * bi;
        bip = bi;
        bi = bim;
        if bi.abs() > BIGNO {
            ans *= BIGNI;
            bi *= BIGNI;
            bip *= BIGNI;
        }
        if j == n {
            ans = bip;
        }
        j -= 1;
    }
    ans *= i0(x) / bi;
    Ok(if x < 0.0 && n & 1 == 1 { -ans } else { ans })
}

fn bessel_j(x: f64, n: u64) -> Res<f64> {
    match n {
        0 => return Ok(j0(x)),
        1 => return Ok(j1(x)),
        _ => {}
    }
    let ax = x.abs();
    if ax == 0.0 {
        return Ok(0.0);
    }
    let tox = 2.0 / ax;
    let mut ans;
    if ax > n as f64 {
        let (mut bjm, mut bj) = (j0(ax), j1(ax));
        for j in 1..n {
            let bjp = j as f64 * tox * bj - bjm;
            bjm = bj;
            bj = bjp;
        }
        ans = bj;
    } else {
        let m = 2 * ((n + (ACC * n as f64).sqrt() as u64) / 2);
        if m > MAX_STEPS {
            return Err(ErrorKind::Num);
        }
        let mut jsum = false;
        let (mut bjp, mut sum, mut bj) = (0.0f64, 0.0f64, 1.0f64);
        ans = 0.0;
        let mut j = m;
        while j > 0 {
            let bjm = j as f64 * tox * bj - bjp;
            bjp = bj;
            bj = bjm;
            if bj.abs() > BIGNO {
                bj *= BIGNI;
                bjp *= BIGNI;
                ans *= BIGNI;
                sum *= BIGNI;
            }
            if jsum {
                sum += bj;
            }
            jsum = !jsum;
            if j == n {
                ans = bjp;
            }
            j -= 1;
        }
        sum = 2.0 * sum - bj;
        ans /= sum;
    }
    Ok(if x < 0.0 && n & 1 == 1 { -ans } else { ans })
}

fn bessel_y(x: f64, n: u64) -> Res<f64> {
    if x <= 0.0 {
        return Err(ErrorKind::Num);
    }
    match n {
        0 => return Ok(y0(x)),
        1 => return Ok(y1(x)),
        _ => {}
    }
    let tox = 2.0 / x;
    let (mut by, mut bym) = (y1(x), y0(x));
    for j in 1..n {
        let byp = j as f64 * tox * by - bym;
        bym = by;
        by = byp;
        if !by.is_finite() {
            return Err(ErrorKind::Num);
        }
    }
    Ok(by)
}

fn bessel_k(x: f64, n: u64) -> Res<f64> {
    if x <= 0.0 {
        return Err(ErrorKind::Num);
    }
    match n {
        0 => return Ok(k0(x)),
        1 => return Ok(k1(x)),
        _ => {}
    }
    let tox = 2.0 / x;
    let (mut bkm, mut bk) = (k0(x), k1(x));
    for j in 1..n {
        let bkp = bkm + j as f64 * tox * bk;
        bkm = bk;
        bk = bkp;
        if !bk.is_finite() {
            return Err(ErrorKind::Num);
        }
    }
    Ok(bk)
}

/// The function value for a truncated, validated order; a non-finite result is `#NUM!`.
pub(super) fn bessel(kind: Kind, x: f64, n: u64) -> Res<f64> {
    let v = match kind {
        Kind::I => bessel_i(x, n)?,
        Kind::J => bessel_j(x, n)?,
        Kind::K => bessel_k(x, n)?,
        Kind::Y => bessel_y(x, n)?,
    };
    if v.is_finite() {
        Ok(v)
    } else {
        Err(ErrorKind::Num)
    }
}

/// `(x, n)` of a call. Rules measured against Excel 16.0: a boolean argument is `#VALUE!` (also from a cell),
/// the order is truncated, a negative order (before truncation) or one of 2147483647 and above is `#NUM!`.
fn bessel_args(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<(f64, u64)> {
    arity(args, 2, 2)?;
    let get = |arg: &Arg| -> Res<f64> {
        let v = scalar(ctx, arg)?;
        if matches!(v, Value::Bool(_)) {
            return Err(ErrorKind::Value);
        }
        v.to_number_in(ctx.date1904)
    };
    let x = get(&args[0])?;
    let n = get(&args[1])?;
    if !(0.0..2_147_483_647.0).contains(&n) {
        return Err(ErrorKind::Num);
    }
    Ok((x, n.trunc() as u64))
}

fn call(ctx: &EvalContext<'_>, args: &[Arg], kind: Kind) -> Res<Value> {
    let (x, n) = bessel_args(ctx, args)?;
    number(bessel(kind, x, n)?)
}

pub(super) fn besseli(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    call(ctx, args, Kind::I)
}

pub(super) fn besselj(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    call(ctx, args, Kind::J)
}

pub(super) fn besselk(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    call(ctx, args, Kind::K)
}

pub(super) fn bessely(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    call(ctx, args, Kind::Y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approximations_are_close_to_the_exact_values() {
        // Excel's documented example: BESSELJ(1.9, 2) = 0.329925829 (exact 0.3299257277)
        assert!((bessel(Kind::J, 1.9, 2).unwrap() - 0.329_925_829).abs() < 1e-9);
        assert!((bessel(Kind::I, 1.5, 1).unwrap() - 0.981_666_428_6).abs() < 1e-6);
        assert!((bessel(Kind::K, 1.5, 1).unwrap() - 0.277_387_800_5).abs() < 1e-6);
        assert!((bessel(Kind::Y, 2.5, 1).unwrap() - 0.145_918_138).abs() < 1e-6);
    }

    #[test]
    fn domain() {
        assert_eq!(bessel(Kind::Y, 0.0, 0), Err(ErrorKind::Num));
        assert_eq!(bessel(Kind::K, -1.0, 0), Err(ErrorKind::Num));
        assert_eq!(bessel(Kind::Y, 1e-300, 2), Err(ErrorKind::Num));
        assert_eq!(bessel(Kind::I, 1000.0, 0), Err(ErrorKind::Num));
    }
}
