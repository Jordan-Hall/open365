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
    /// Major unit; for a logarithmic axis the ratio between ticks.
    pub major: f64,
    pub minor: f64,
    /// Base of a logarithmic axis.
    pub log_base: Option<f64>,
}

impl AxisScale {
    /// Position of `v` along the axis, 0 at `min` and 1 at `max`; unclamped.
    pub fn position(&self, v: f64) -> f64 {
        match self.log_base {
            Some(_) if v > 0.0 && self.min > 0.0 => {
                (v.ln() - self.min.ln()) / (self.max.ln() - self.min.ln())
            }
            _ => (v - self.min) / (self.max - self.min),
        }
    }

    /// Major tick positions from `min` to `max`.
    pub fn ticks(&self) -> Vec<f64> {
        let mut out = Vec::new();
        if let Some(base) = self.log_base {
            let mut v = self.min;
            while v <= self.max * (1.0 + 1e-9) && out.len() < 400 {
                out.push(snap(v, v));
                v *= self.major.max(base);
            }
            return out;
        }
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
            log_base: None,
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
        log_base: None,
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
        log_base: None,
    }
}

/// Largest count of major intervals along a logarithmic value axis `length_pt` long with
/// `font_pt` tick labels: `floor((length - 0.9 font) / (1.2207 font))` like the linear axis but
/// without its cap of 10 (bases 2 and 3 get up to 30). When the axis reaches below 1 the
/// category labels sit inside the plot and the `0.9 font` term drops out. Fitted to Excel's
/// answers on generated log axes (`chart-scale/log-train.json`); only 10 pt was measured.
pub fn log_max_intervals(length_pt: f64, font_pt: f64, labels_in_plot: bool) -> u32 {
    let font = if font_pt > 0.0 { font_pt } else { 10.0 };
    let reserve = if labels_in_plot { 0.0 } else { 0.9 * font };
    (((length_pt - reserve) / (1.2207 * font)).floor()).max(1.0) as u32
}

/// Interval budget of a logarithmic axis whose band of data sits far from 1 (see
/// [`log_scale`]): `floor(length / 0.86 font)`. Fitted to 101 of 105 such axes in
/// `chart-scale/lognarrow-*.json` (10 pt labels only); the misses are line charts and
/// base 2 scatter charts with exponents above 38.
pub fn log_far_intervals(length_pt: f64, font_pt: f64) -> u32 {
    let font = if font_pt > 0.0 { font_pt } else { 10.0 };
    (length_pt / (0.86 * font)).floor().max(1.0) as u32
}

/// How a logarithmic axis treats data in a narrow band of decades far from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogNarrow {
    /// Bar and area charts: the axis always reaches 1.
    Never,
    /// Scatter charts: the axis starts at the data's decade.
    Whole,
    /// Line charts: the axis starts half a band beyond the data's decade.
    Half,
}

/// Automatic scale of a logarithmic axis over data between `lo` and `hi` (both positive).
///
/// 1. The axis always contains 1, the crossing of the category axis: the minimum is
///    `base^floor(log_base lo)` when that is below 1, else 1.
/// 2. The maximum is `base^ceil(log_base hi)`, at least 1.
/// 3. The major unit is `base^k` for the smallest whole `k` that leaves at most
///    `max_intervals` intervals (`ceil(decades / k)`); the minor unit equals the major unit.
pub fn log_scale(
    lo: f64,
    hi: f64,
    base: f64,
    max_intervals: u32,
    narrow: LogNarrow,
    far_intervals: u32,
) -> AxisScale {
    let base = if base > 1.0 { base } else { 10.0 };
    let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
    let (lo, hi) = (
        if lo > 0.0 { lo } else { 1.0 },
        if hi > 0.0 { hi } else { 1.0 },
    );
    let e_lo = (lo.ln() / base.ln() + 1e-12).floor();
    let e_hi = (hi.ln() / base.ln() - 1e-12).ceil();
    let mut e_min = e_lo.min(0.0);
    let mut e_max = e_hi.max(0.0);
    // Data in a narrow band of decades far from 1 do not stretch the axis to 1. The test is
    // the linear rule (`span / far end <= 1/6`) applied to the whole exponents of the data's
    // decades; a line chart then widens the band by half its width on the side of 1, a scatter
    // chart does not, and bar and area charts always reach 1.
    if narrow != LogNarrow::Never {
        let d = (e_hi - e_lo).max(1.0);
        let shift = if narrow == LogNarrow::Half {
            d / 2.0
        } else {
            0.0
        };
        if e_lo >= 1.0 && d / e_hi <= 1.0 / 6.0 + 1e-9 {
            e_min = e_lo - shift;
        } else if e_hi <= -1.0 && d / -e_lo <= 1.0 / 6.0 + 1e-9 {
            e_max = e_hi + shift;
        }
    }
    let mut decades = (e_max - e_min).max(1.0);
    let mut budget = f64::from(max_intervals.max(1));
    if e_min >= 1.0 {
        // A band far above 1 gets the major unit of an axis running down to 1, with a looser
        // budget ([`log_far_intervals`]).
        decades = e_max;
        budget = f64::from(far_intervals.max(1));
    } else if e_max <= -1.0 {
        decades = -e_min;
        budget = f64::from(far_intervals.max(1));
    }
    let mut k = 1.0;
    while (decades / k).ceil() > budget && k < 400.0 {
        k += 1.0;
    }
    let major = base.powf(k);
    AxisScale {
        min: base.powf(e_min),
        max: base.powf(e_max),
        major,
        minor: major,
        log_base: Some(base),
    }
}

/// Unit of a date axis; the order matches Excel's `BaseUnit` / `MajorUnitScale` numbers
/// (0 days, 1 months, 2 years).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateUnit {
    Days,
    Months,
    Years,
}

/// A resolved automatic date axis (Excel 1900 serial numbers).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DateScale {
    /// First serial on the axis.
    pub min: f64,
    /// Last serial on the axis.
    pub max: f64,
    pub base_unit: DateUnit,
    pub major: u32,
    pub major_unit: DateUnit,
    pub minor: u32,
    pub minor_unit: DateUnit,
}

/// Civil date of a 1900-system serial (the 1900 leap-day error only matters before March 1900).
fn civil(serial: i64) -> (i64, i64, i64) {
    // Serial 25569 is 1970-01-01.
    let z = serial - 25569 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Serial of a civil date; inverse of [`civil`].
fn serial_of(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468 + 25569
}

/// Automatic scale of a date category axis over the given serials.
///
/// Observed in Excel (`chart-scale/date-*.json`, 450 axes):
///
/// 1. The base unit comes from the smallest gap between distinct dates: under 28 days it is
///    days, under 360 days months, otherwise years.
/// 2. The axis starts at the first date (days), or the first day of its month or year, and
///    ends at the last date, or the first day of its month or year; the minor unit is one
///    unit of the major unit's kind.
/// 3. The major unit is the smallest count of base units that leaves at most `N` intervals,
///    `N = floor((width - 24) / 14.4)` at 10 pt. With a days base, 1 and 2 days (budget
///    `floor((width - 30) / 14)`) and, on plots 146 to 170 pt tall, 7 days (budget
///    `floor(width / 21)`) come first; then whole months. The 7-day step and its height
///    condition are a fit, not an explanation.
///
/// `width_pt` and `height_pt` are the plot's inside size; only 10 pt labels were measured.
pub fn date_scale(dates: &[f64], width_pt: f64, height_pt: f64, font_pt: f64) -> Option<DateScale> {
    let mut d: Vec<i64> = dates
        .iter()
        .filter(|v| v.is_finite() && **v >= 1.0)
        .map(|v| v.floor() as i64)
        .collect();
    d.sort_unstable();
    d.dedup();
    let (&first, &last) = (d.first()?, d.last()?);
    let gap = d.windows(2).map(|w| w[1] - w[0]).min().unwrap_or(1);
    let base = if gap < 28 {
        DateUnit::Days
    } else if gap < 360 {
        DateUnit::Months
    } else {
        DateUnit::Years
    };
    let k = if font_pt > 0.0 { font_pt / 10.0 } else { 1.0 };
    let (w, h) = (width_pt / k, height_pt / k);
    let (fy, fm, _) = civil(first);
    let (ly, lm, _) = civil(last);
    let months = (ly - fy) * 12 + lm - fm;
    let (min, max, span) = match base {
        DateUnit::Days => (first, last, last - first),
        DateUnit::Months => (serial_of(fy, fm, 1), serial_of(ly, lm, 1), months),
        DateUnit::Years => (serial_of(fy, 1, 1), serial_of(ly, 1, 1), ly - fy),
    };
    let budget = ((w - 24.0) / 14.4).floor().max(1.0) as i64;
    let fewest = |span: i64| {
        (1..=span.max(1))
            .find(|m| (span + m - 1) / m <= budget)
            .unwrap_or(1) as u32
    };
    let (major, major_unit) = if base == DateUnit::Days {
        let day_budget = ((w - 30.0) / 14.0).floor().max(1.0) as i64;
        let week_budget = (w / 21.0).floor().max(1.0) as i64;
        let fits = |step: i64, n: i64| (span + step - 1) / step <= n;
        if fits(1, day_budget) {
            (1, DateUnit::Days)
        } else if fits(2, day_budget) {
            (2, DateUnit::Days)
        } else if (146.0..170.0).contains(&h) && fits(7, week_budget) {
            (7, DateUnit::Days)
        } else {
            let m = (1..=months.max(1))
                .find(|m| (months + m - 1) / m <= budget)
                .unwrap_or(1);
            (m as u32, DateUnit::Months)
        }
    } else {
        (fewest(span), base)
    };
    Some(DateScale {
        min: min as f64,
        max: max as f64,
        base_unit: base,
        major,
        major_unit,
        minor: 1,
        minor_unit: major_unit,
    })
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
    fn civil_dates_round_trip() {
        assert_eq!(civil(42513), (2016, 5, 23));
        assert_eq!(serial_of(2016, 5, 23), 42513);
        assert_eq!(civil(43831), (2020, 1, 1));
    }

    #[test]
    fn ticks_are_clean() {
        let s = auto_scale(0.0, 1.0, 10);
        let t = s.ticks();
        assert_eq!(t.first(), Some(&0.0));
        assert_eq!(t.get(3), Some(&0.6));
    }
}
