//! Date functions and the serial-number date systems.
//!
//! The 1900 system counts days from 1900-01-00 (serial 1 is 1900-01-01) and, like Excel,
//! treats 1900 as a leap year: serial 60 is the non-existent 1900-02-29, and serials from 61
//! on match the real calendar. The 1904 system counts days from 1904-01-01 (serial 0).

use super::{Res, arity, int_arg, num_arg, number};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value};

/// Largest valid serial: 9999-12-31 in the 1900 system.
const MAX_SERIAL_1900: i64 = 2_958_465;
/// Largest valid serial: 9999-12-31 in the 1904 system.
const MAX_SERIAL_1904: i64 = 2_957_003;
/// Serial of 1970-01-01 in each system.
const UNIX_EPOCH_1900: f64 = 25_569.0;
const UNIX_EPOCH_1904: f64 = 24_107.0;

/// Days since 1970-01-01 of a proleptic Gregorian date (month 1..=12).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Inverse of [`days_from_civil`].
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// Serial number of a date. Month and day may overflow and roll over, as in `DATE`.
/// Returns `None` outside the supported range (1900-01-00 to 9999-12-31).
pub fn ymd_to_serial(year: i64, month: i64, day: i64, date1904: bool) -> Option<f64> {
    let months = year.checked_mul(12)?.checked_add(month - 1)?;
    let (y, m) = (months.div_euclid(12), months.rem_euclid(12) + 1);
    if !(0..=10_000).contains(&y) || (!date1904 && y < 1900) {
        return None;
    }
    let first = days_from_civil(y, m, 1);
    let serial = if date1904 {
        first - days_from_civil(1904, 1, 1) + day - 1
    } else {
        let mut s = first - days_from_civil(1899, 12, 30);
        if s < 61 {
            // Excel counts the fictitious 1900-02-29, so dates before March 1900 sit one lower.
            s -= 1;
        }
        s + day - 1
    };
    let max = if date1904 {
        MAX_SERIAL_1904
    } else {
        MAX_SERIAL_1900
    };
    (0..=max).contains(&serial).then_some(serial as f64)
}

/// Calendar date of a serial (fractional part ignored). Serial 0 in the 1900 system is
/// `(1900, 1, 0)` and serial 60 is the fictitious `(1900, 2, 29)`.
pub fn serial_to_ymd(serial: f64, date1904: bool) -> Option<(i64, i64, i64)> {
    if !serial.is_finite() || serial < 0.0 {
        return None;
    }
    let s = serial.floor() as i64;
    if date1904 {
        return (s <= MAX_SERIAL_1904).then(|| civil_from_days(days_from_civil(1904, 1, 1) + s));
    }
    if s > MAX_SERIAL_1900 {
        return None;
    }
    Some(match s {
        0 => (1900, 1, 0),
        1..=59 => civil_from_days(days_from_civil(1899, 12, 31) + s),
        60 => (1900, 2, 29),
        _ => civil_from_days(days_from_civil(1899, 12, 30) + s),
    })
}

pub(super) fn date(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 3)?;
    let mut year = int_arg(ctx, &args[0])?;
    let month = int_arg(ctx, &args[1])?;
    let day = int_arg(ctx, &args[2])?;
    if !(0..10_000).contains(&year) {
        return Err(ErrorKind::Num);
    }
    if year < 1900 {
        year += 1900;
    }
    ymd_to_serial(year, month, day, ctx.date1904)
        .map(Value::Number)
        .ok_or(ErrorKind::Num)
}

fn part(ctx: &EvalContext<'_>, args: &[Arg], pick: fn((i64, i64, i64)) -> i64) -> Res<Value> {
    arity(args, 1, 1)?;
    let serial = num_arg(ctx, &args[0])?;
    let ymd = serial_to_ymd(serial, ctx.date1904).ok_or(ErrorKind::Num)?;
    Ok(Value::Number(pick(ymd) as f64))
}

pub(super) fn year(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    part(ctx, args, |(y, _, _)| y)
}

pub(super) fn month(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    part(ctx, args, |(_, m, _)| m)
}

pub(super) fn day(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    part(ctx, args, |(_, _, d)| d)
}

fn now_serial(ctx: &EvalContext<'_>) -> f64 {
    let epoch = if ctx.date1904 {
        UNIX_EPOCH_1904
    } else {
        UNIX_EPOCH_1900
    };
    ctx.clock.now_unix_seconds() / 86_400.0 + epoch
}

pub(super) fn today(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 0, 0)?;
    number(now_serial(ctx).floor())
}

pub(super) fn now(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 0, 0)?;
    number(now_serial(ctx))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(y: i64, m: i64, d: i64) -> Option<f64> {
        ymd_to_serial(y, m, d, false)
    }

    #[test]
    fn serial_anchor_points_1900() {
        assert_eq!(s(1900, 1, 1), Some(1.0));
        assert_eq!(s(1900, 2, 28), Some(59.0));
        assert_eq!(s(1900, 2, 29), Some(60.0), "Excel's fake leap day");
        assert_eq!(s(1900, 3, 1), Some(61.0));
        assert_eq!(s(1900, 3, 0), Some(60.0));
        assert_eq!(s(1900, 1, 0), Some(0.0));
        assert_eq!(s(1970, 1, 1), Some(25_569.0));
        assert_eq!(s(2000, 1, 1), Some(36_526.0));
        assert_eq!(s(2024, 2, 29), Some(45_351.0));
        assert_eq!(s(9999, 12, 31), Some(2_958_465.0));
        assert_eq!(s(10_000, 1, 1), None);
        assert_eq!(s(1899, 12, 31), None);
    }

    #[test]
    fn serial_to_date_1900() {
        let f = |n: f64| serial_to_ymd(n, false);
        assert_eq!(f(0.0), Some((1900, 1, 0)));
        assert_eq!(f(1.0), Some((1900, 1, 1)));
        assert_eq!(f(59.0), Some((1900, 2, 28)));
        assert_eq!(f(60.0), Some((1900, 2, 29)));
        assert_eq!(f(61.0), Some((1900, 3, 1)));
        assert_eq!(f(25_569.9), Some((1970, 1, 1)));
        assert_eq!(f(45_351.0), Some((2024, 2, 29)));
        assert_eq!(f(2_958_465.0), Some((9999, 12, 31)));
        assert_eq!(f(2_958_466.0), None);
        assert_eq!(f(-1.0), None);
    }

    #[test]
    fn round_trip_every_serial_after_the_fake_leap_day() {
        for serial in (61..120_000).step_by(7) {
            let (y, m, d) = serial_to_ymd(f64::from(serial), false).unwrap();
            assert_eq!(s(y, m, d), Some(f64::from(serial)), "{y}-{m}-{d}");
        }
        for serial in 1..60 {
            let (y, m, d) = serial_to_ymd(f64::from(serial), false).unwrap();
            assert_eq!(s(y, m, d), Some(f64::from(serial)));
        }
    }

    #[test]
    fn date_system_1904() {
        let t = |y, m, d| ymd_to_serial(y, m, d, true);
        assert_eq!(t(1904, 1, 1), Some(0.0));
        assert_eq!(t(1904, 1, 2), Some(1.0));
        assert_eq!(t(1970, 1, 1), Some(24_107.0));
        assert_eq!(t(1900, 1, 1), None);
        assert_eq!(serial_to_ymd(0.0, true), Some((1904, 1, 1)));
        assert_eq!(serial_to_ymd(24_107.0, true), Some((1970, 1, 1)));
        // The two systems differ by 1462 days for the same date.
        assert_eq!(s(2024, 5, 17).unwrap() - t(2024, 5, 17).unwrap(), 1462.0);
    }

    #[test]
    fn month_and_day_overflow_roll_over() {
        assert_eq!(s(2024, 13, 1), s(2025, 1, 1));
        assert_eq!(s(2024, 0, 1), s(2023, 12, 1));
        assert_eq!(s(2024, 1, 32), s(2024, 2, 1));
        assert_eq!(s(2024, 3, 0), s(2024, 2, 29));
        assert_eq!(s(2024, -1, 1), s(2023, 11, 1));
    }
}
