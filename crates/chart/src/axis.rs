//! Automatic value-axis scaling, matched to Excel.
//!
//! The rules were derived from, and are checked against, Excel's own answers for hundreds of
//! generated data ranges (`fixtures/reference/xlsx/chart-axes/`, recorded by
//! `tools/office/chart-oracle.ps1`, replayed by `tests/axis_corpus.rs`). Only observed
//! behaviour is encoded; nothing comes from Microsoft code.
//!
//! Summary of the rules (data minimum `lo`, maximum `hi`, `span = hi - lo`):
//!
//! 1. The axis starts at zero unless the data sit in a narrow band far from zero: for positive
//!    data the minimum is nonzero when `span / hi < 1/6`, and then it starts `span / 2` below
//!    the data minimum. Negative data mirror this at the top. Data of both signs get a
//!    minimum and maximum 5 % of the span beyond the data.
//! 2. The side that stays at zero gets headroom of 5 % of the plotted range; a nonzero
//!    side gets 5 % of the span.
//! 3. The major unit is the smallest of 1, 2 and 5 times a power of ten for which the padded
//!    range holds at most `max_intervals` major units; `max_intervals` follows the plot size
//!    ([`max_intervals`]).
//! 4. The minimum rounds down and the maximum rounds up to a multiple of the major unit.
//! 5. The minor unit is a fifth of the major unit.

/// A resolved value axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxisScale {
    pub min: f64,
    pub max: f64,
    pub major: f64,
    pub minor: f64,
}

impl AxisScale {
    /// Major tick positions from `min` to `max`.
    pub fn ticks(&self) -> Vec<f64> {
        let mut out = Vec::new();
        if self.major.is_nan()
            || self.major <= 0.0
            || !self.min.is_finite()
            || !self.max.is_finite()
        {
            return out;
        }
        let n = ((self.max - self.min) / self.major + 1e-9).floor() as usize;
        for i in 0..=n.min(10_000) {
            let v = self.min + self.major * i as f64;
            // Snap float noise: 0.30000000000000004 prints as 0.3.
            out.push(snap(v, self.major));
        }
        out
    }
}

/// Rounds `v` to the precision of `unit` so repeated additions do not leave float noise.
fn snap(v: f64, unit: f64) -> f64 {
    let digits = (-unit.log10().floor()).clamp(0.0, 15.0) as i32 + 2;
    let p = 10f64.powi(digits);
    if v.abs() * p > 1e15 {
        return v;
    }
    (v * p).round() / p
}

/// Largest count of major units Excel allows along a plot axis of `length_pt` points with tick
/// labels of `font_pt` points. Fitted to Excel's answers (`chart-axes/sweep.json` and the
/// generated corpus):
///
/// * a vertical axis: `floor((length - 0.9 font) / (1.2207 font))`, one line of label text per
///   interval;
/// * a horizontal axis: `floor((length + 2.1 font) / (3.75 font))`, 37.5 pt per interval at
///   10 pt;
/// * never more than 10 and never less than 1.
///
/// `length_pt` is the plot's inside length measured as if the category labels sat outside the
/// plot (Excel moves them into the plot when the data are all negative, and the budget does
/// not follow).
pub fn max_intervals(length_pt: f64, horizontal: bool, font_pt: f64) -> u32 {
    let font = if font_pt > 0.0 { font_pt } else { 10.0 };
    let n = if horizontal {
        ((length_pt + 2.1 * font) / (3.75 * font)).floor()
    } else {
        ((length_pt - 0.9 * font) / (1.2207 * font)).floor()
    };
    n.clamp(1.0, 10.0) as u32
}

fn nice_major(range: f64, max_intervals: u32) -> f64 {
    let n = f64::from(max_intervals.max(1));
    let start = range.log10().floor() as i32 - 3;
    for e in start..start + 40 {
        for m in [1.0, 2.0, 5.0] {
            let unit = m * 10f64.powi(e);
            if range / unit <= n * (1.0 + 1e-12) {
                return unit;
            }
        }
    }
    range
}

fn floor_to(v: f64, unit: f64) -> f64 {
    (v / unit + 1e-9).floor() * unit
}

fn ceil_to(v: f64, unit: f64) -> f64 {
    (v / unit - 1e-9).ceil() * unit
}

/// Smallest and largest plotted value of a set of series (one `Vec` per series, aligned by
/// category). For stacked charts the stack sums count as well as the single values, which is
/// why a stacked chart over `100..116` still starts at zero in Excel.
pub fn data_extent(series: &[Vec<Option<f64>>], stacked: bool) -> Option<(f64, f64)> {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for v in series.iter().flatten().flatten() {
        lo = lo.min(*v);
        hi = hi.max(*v);
    }
    if stacked {
        let n = series.iter().map(Vec::len).max().unwrap_or(0);
        for i in 0..n {
            let (mut pos, mut neg) = (0.0, 0.0);
            for v in series.iter().filter_map(|s| s.get(i).copied().flatten()) {
                if v >= 0.0 {
                    pos += v;
                } else {
                    neg += v;
                }
            }
            lo = lo.min(neg).min(pos);
            hi = hi.max(pos).max(neg);
        }
    }
    (lo <= hi).then_some((lo, hi))
}

/// Automatic scale for data between `lo` and `hi` (the smallest and largest plotted value;
/// for stacked charts the stack sums and the individual values).
pub fn auto_scale(lo: f64, hi: f64, max_intervals: u32) -> AxisScale {
    let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
    if !lo.is_finite() || !hi.is_finite() || (lo == 0.0 && hi == 0.0) {
        return AxisScale {
            min: 0.0,
            max: 1.0,
            major: 0.1,
            minor: 0.02,
        };
    }
    let span = hi - lo;
    let (low, high) = if lo >= 0.0 {
        let low = if lo > 0.0 && span > 0.0 && span / hi < 1.0 / 6.0 {
            lo - span / 2.0
        } else {
            0.0
        };
        (low, hi + 0.05 * (hi - low))
    } else if hi <= 0.0 {
        let high = if hi < 0.0 && span > 0.0 && span / -lo < 1.0 / 6.0 {
            hi + span / 2.0
        } else {
            0.0
        };
        (lo - 0.05 * (high - lo), high)
    } else {
        (lo - 0.05 * span, hi + 0.05 * span)
    };
    let major = nice_major(high - low, max_intervals);
    let min = if low == 0.0 {
        0.0
    } else {
        floor_to(low, major)
    };
    let max = if high == 0.0 {
        0.0
    } else {
        ceil_to(high, major)
    };
    AxisScale {
        min,
        max,
        major,
        minor: major / 5.0,
    }
}

/// The scale a value axis ends up with: explicit `c:min`, `c:max`, `c:majorUnit` and
/// `c:minorUnit` win over the automatic ones. With only one end fixed, the automatic scale is
/// computed for the data and the fixed end replaces its side (unverified against Excel).
pub fn resolve_scale(
    fixed_min: Option<f64>,
    fixed_max: Option<f64>,
    fixed_major: Option<f64>,
    fixed_minor: Option<f64>,
    data: Option<(f64, f64)>,
    max_intervals: u32,
) -> AxisScale {
    let auto = match data {
        Some((lo, hi)) => auto_scale(lo, hi, max_intervals),
        None => auto_scale(0.0, 0.0, max_intervals),
    };
    let min = fixed_min.unwrap_or(auto.min);
    let max = fixed_max.unwrap_or(auto.max);
    let (min, max) = if max > min {
        (min, max)
    } else {
        (min, min + 1.0)
    };
    let major = fixed_major.filter(|m| *m > 0.0).unwrap_or_else(|| {
        if fixed_min.is_none() && fixed_max.is_none() {
            auto.major
        } else {
            nice_major(max - min, max_intervals)
        }
    });
    AxisScale {
        min,
        max,
        major,
        minor: fixed_minor.filter(|m| *m > 0.0).unwrap_or(major / 5.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_positive_data_start_at_zero() {
        let s = auto_scale(1.0, 4.0, 10);
        assert_eq!((s.min, s.max, s.major), (0.0, 4.5, 0.5));
    }

    #[test]
    fn narrow_band_gets_a_nonzero_minimum() {
        let s = auto_scale(99.0, 101.0, 10);
        assert_eq!((s.min, s.max, s.major), (98.0, 101.5, 0.5));
    }

    #[test]
    fn ticks_are_clean() {
        let s = auto_scale(0.0, 1.0, 10);
        let t = s.ticks();
        assert_eq!(t.first(), Some(&0.0));
        assert_eq!(t.get(3), Some(&0.6));
    }
}
