//! Standard normal quantile in the tails, as Excel computes it.
//!
//! Excel's `NORM.S.INV` is Wichura's algorithm AS 241 (PPND16; M. J. Wichura, "The percentage points of the
//! normal distribution", Applied Statistics 37 (1988) 477-484) in both tails. The measured boundaries are
//! `p < 0.075` and `p >= 0.9375` (not the symmetric 0.425 of the paper): 99.9% of 5,800 sampled tail points
//! (`r > 5`) and 96% of 3,500 (`r <= 5`, the rest lie in `0.925 < p < 0.9375`) are bit-identical. In the centre
//! Excel's results differ from AS 241 by a few ulp in a pattern that looks like a correction step with its own
//! normal CDF, which is not reproduced; the engine keeps its exact value there.

// The coefficients are quoted from the paper with all their published digits.
#![allow(clippy::excessive_precision)]

use crate::xmath;

const C: [f64; 8] = [
    1.42343711074968357734,
    4.63033784615654529590,
    5.76949722146069140550,
    3.64784832476320460504,
    1.27045825245236838258,
    2.41780725177450611770E-1,
    2.27238449892691845833E-2,
    7.74545014278341407640E-4,
];
const D: [f64; 8] = [
    1.0,
    2.05319162663775882187,
    1.67638483018380384940,
    6.89767334985100004550E-1,
    1.48103976427480074590E-1,
    1.51986665636164571966E-2,
    5.47593808499534494600E-4,
    1.05075007164441684324E-9,
];
const E: [f64; 8] = [
    6.65790464350110377720,
    5.46378491116411436990,
    1.78482653991729133580,
    2.96560571828504891230E-1,
    2.65321895265761230930E-2,
    1.24266094738807843860E-3,
    2.71155556874348757815E-5,
    2.01033439929228813265E-7,
];
const F: [f64; 8] = [
    1.0,
    5.99832206555887937690E-1,
    1.36929880922735805310E-1,
    1.48753612908506148525E-2,
    7.86869131145613259100E-4,
    1.84631831751005468180E-5,
    1.42151175831644588870E-7,
    2.04426310338993978564E-15,
];

/// Lowest probability of the centre region and first probability past it (measured against Excel).
const CENTRE_LOW: f64 = 0.075;
const CENTRE_HIGH: f64 = 0.9375;

fn horner(c: &[f64; 8], r: f64) -> f64 {
    c.iter().rev().fold(0.0, |acc, v| acc * r + v)
}

/// AS 241 quantile for `p` in `(0, 0.075)` or `[0.9375, 1)`; `None` in the centre, where Excel's algorithm is unknown.
pub(super) fn tail_quantile(p: f64) -> Option<f64> {
    if !(p > 0.0 && p < 1.0) || (CENTRE_LOW..CENTRE_HIGH).contains(&p) {
        return None;
    }
    let lower = p < 0.5;
    let t = if lower { p } else { 1.0 - p };
    let mut r = (-xmath::ln(t)).sqrt();
    let v = if r <= 5.0 {
        r -= 1.6;
        horner(&C, r) / horner(&D, r)
    } else {
        r -= 5.0;
        horner(&E, r) / horner(&F, r)
    };
    Some(if lower { -v } else { v })
}

#[cfg(test)]
mod tests {
    use super::tail_quantile;

    #[test]
    fn tails_match_excel() {
        // Values recorded from Excel (NORM.S.INV).
        assert_eq!(tail_quantile(1e-6), Some(-4.753424308822899));
        assert_eq!(tail_quantile(0.01), Some(-2.3263478740408408));
        assert_eq!(tail_quantile(0.99), Some(2.3263478740408408));
        assert_eq!(tail_quantile(0.5), None);
    }
}
