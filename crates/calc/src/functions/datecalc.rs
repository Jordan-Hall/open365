//! Date and time functions built on serial numbers: `TIME`, `HOUR`, `MINUTE`, `SECOND`, `WEEKDAY`,
//! `WEEKNUM`, `ISOWEEKNUM`, `EDATE`, `EOMONTH`, `DATEDIF`, `DAYS`, `DAYS360`, `NETWORKDAYS(.INTL)`,
//! `WORKDAY(.INTL)`, `YEARFRAC`, `DATEVALUE` and `TIMEVALUE`.
//!
//! Serial 1 of the 1900 system is a Sunday (Excel's day 1 is 1900-01-01 and it treats 1900 as a leap
//! year, so the weekday of every serial is `(serial - 1) mod 7`). The 1904 system is offset by 1462 days.

use std::collections::BTreeSet;

use super::datetime::{
    MAX_SERIAL_1900, MAX_SERIAL_1904, days_from_civil, serial_to_ymd, ymd_to_serial,
};
use super::{Res, arity, num_arg, optional, scalar, text_arg};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value};

fn max_serial(date1904: bool) -> i64 {
    if date1904 {
        MAX_SERIAL_1904
    } else {
        MAX_SERIAL_1900
    }
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Excel believes 1900 was a leap year.
fn is_leap_excel(y: i64) -> bool {
    is_leap(y) || y == 1900
}

fn days_in_month(y: i64, m: i64) -> i64 {
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

fn ymd(serial: i64, date1904: bool) -> Res<(i64, i64, i64)> {
    serial_to_ymd(serial as f64, date1904).ok_or(ErrorKind::Num)
}

fn check_serial(ctx: &EvalContext<'_>, n: f64) -> Res<i64> {
    if !(0.0..=max_serial(ctx.date1904) as f64).contains(&n) {
        return Err(ErrorKind::Num);
    }
    Ok(n as i64)
}

/// A date argument whose time of day is rounded to the nearest second first, so `x.9999999` already is the next
/// day (`DAY`, `WEEKDAY`, `WEEKNUM`, `DAYS360`).
fn rounded_serial_arg(ctx: &EvalContext<'_>, arg: &Arg) -> Res<i64> {
    rounded_serial(ctx, num_arg(ctx, arg)?)
}

fn rounded_serial(ctx: &EvalContext<'_>, raw: f64) -> Res<i64> {
    if raw < 0.0 || raw > (max_serial(ctx.date1904) + 1) as f64 {
        return Err(ErrorKind::Num);
    }
    check_serial(ctx, (raw * 86_400.0).round().div_euclid(86_400.0))
}

/// An integer argument truncated toward zero.
fn trunc_arg(ctx: &EvalContext<'_>, arg: &Arg) -> Res<i64> {
    let n = num_arg(ctx, arg)?.trunc();
    if n.abs() > 9.0e15 {
        return Err(ErrorKind::Num);
    }
    Ok(n as i64)
}

/// A scalar argument of the Analysis ToolPak style functions: an omitted argument is `#N/A`, a boolean or a
/// multi-cell range is `#VALUE!` (no implicit intersection).
pub(super) fn atp_value(ctx: &EvalContext<'_>, arg: &Arg) -> Res<Value> {
    match arg {
        Arg::Missing => Err(ErrorKind::NA),
        Arg::Range(r) if !r.is_single_cell() => Err(ErrorKind::Value),
        Arg::Union(_) => Err(ErrorKind::Value),
        _ => match scalar(ctx, arg)? {
            Value::Bool(_) => Err(ErrorKind::Value),
            v => Ok(v),
        },
    }
}

pub(super) fn atp_num(ctx: &EvalContext<'_>, arg: &Arg) -> Res<f64> {
    atp_value(ctx, arg)?.to_number_in(ctx.date1904)
}

fn atp_int(ctx: &EvalContext<'_>, arg: &Arg) -> Res<i64> {
    let n = atp_num(ctx, arg)?.trunc();
    if n.abs() > 9.0e15 {
        return Err(ErrorKind::Num);
    }
    Ok(n as i64)
}

/// An optional integer argument of an Analysis ToolPak style function; omitted means `default`.
fn atp_opt_int(ctx: &EvalContext<'_>, args: &[Arg], index: usize, default: i64) -> Res<i64> {
    match optional(args, index) {
        None => Ok(default),
        Some(a) => atp_int(ctx, a),
    }
}

/// Day of the week with Sunday as 0.
fn sunday0(serial: i64, date1904: bool) -> i64 {
    let s = if date1904 { serial + 1462 } else { serial };
    (s - 1).rem_euclid(7)
}

fn in_range(serial: i64, date1904: bool) -> Res<f64> {
    if (0..=max_serial(date1904)).contains(&serial) {
        Ok(serial as f64)
    } else {
        Err(ErrorKind::Num)
    }
}

// ---- time of day ---------------------------------------------------------------------------

pub(super) fn time(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 3)?;
    let h = trunc_arg(ctx, &args[0])?;
    let m = trunc_arg(ctx, &args[1])?;
    let s = trunc_arg(ctx, &args[2])?;
    if [h, m, s].iter().any(|v| v.abs() > 32767) {
        return Err(ErrorKind::Num);
    }
    let total = h * 3600 + m * 60 + s;
    if total < 0 {
        return Err(ErrorKind::Num);
    }
    let days = total as f64 / 86_400.0;
    Ok(Value::Number(days - days.floor()))
}

/// Splits a time-bearing serial into (hour, minute, second). The time of day is rounded to the nearest second
/// and may carry into the next day, which must still be a valid date.
fn clock_parts(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<(i64, i64, i64)> {
    arity(args, 1, 1)?;
    let raw = num_arg(ctx, &args[0])?;
    if raw < 0.0 {
        return Err(ErrorKind::Num);
    }
    let day = raw.floor();
    let mut secs = ((raw - day) * 86_400.0).round() as i64;
    let mut day = day;
    if secs >= 86_400 {
        secs -= 86_400;
        day += 1.0;
    }
    if day > max_serial(ctx.date1904) as f64 {
        return Err(ErrorKind::Num);
    }
    Ok((secs / 3600, secs / 60 % 60, secs % 60))
}

pub(super) fn hour(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    Ok(Value::Number(clock_parts(ctx, args)?.0 as f64))
}

pub(super) fn minute(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    Ok(Value::Number(clock_parts(ctx, args)?.1 as f64))
}

pub(super) fn second(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    Ok(Value::Number(clock_parts(ctx, args)?.2 as f64))
}

// ---- weekdays and week numbers ---------------------------------------------------------------

pub(super) fn weekday(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 2)?;
    let raw = num_arg(ctx, &args[0])?;
    let kind = match args.get(1) {
        None => 1,
        Some(a) => trunc_arg(ctx, a)?,
    };
    let serial = rounded_serial(ctx, raw)?;
    let sun = sunday0(serial, ctx.date1904);
    let n = match kind {
        1 => sun + 1,
        2 => (sun + 6) % 7 + 1,
        3 => (sun + 6) % 7,
        11..=17 => (sun - (kind - 10) % 7).rem_euclid(7) + 1,
        _ => return Err(ErrorKind::Num),
    };
    Ok(Value::Number(n as f64))
}

/// Serial of January 1 of a year (the year of a valid serial, so at least 1900).
fn jan1_serial(year: i64, date1904: bool) -> i64 {
    let first = days_from_civil(year, 1, 1);
    if date1904 {
        first - days_from_civil(1904, 1, 1)
    } else {
        first - days_from_civil(1899, 12, 30) - i64::from(year == 1900)
    }
}

/// ISO 8601 week number of a serial.
fn iso_week(serial: i64, date1904: bool) -> Res<i64> {
    let monday0 = (sunday0(serial, date1904) + 6) % 7;
    let thursday = serial - monday0 + 3;
    let (year, _, _) = serial_to_ymd(thursday.max(0) as f64, date1904).ok_or(ErrorKind::Num)?;
    let year = if thursday < 0 { year - 1 } else { year };
    Ok((thursday - jan1_serial(year, date1904)).div_euclid(7) + 1)
}

pub(super) fn weeknum(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 2)?;
    let raw = atp_num(ctx, &args[0])?;
    let kind = atp_opt_int(ctx, args, 1, 1)?;
    let serial = rounded_serial(ctx, raw)?;
    let start = match kind {
        1 => 0,
        2 => 1,
        11..=17 => (kind - 10) % 7,
        21 => return Ok(Value::Number(iso_week(serial, ctx.date1904)? as f64)),
        _ => return Err(ErrorKind::Num),
    };
    let (year, _, _) = ymd(serial, ctx.date1904)?;
    let jan1 = jan1_serial(year, ctx.date1904);
    let offset = (sunday0(jan1, ctx.date1904) - start).rem_euclid(7);
    Ok(Value::Number(
        ((serial - jan1 + offset).div_euclid(7) + 1) as f64,
    ))
}

pub(super) fn isoweeknum(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    let serial = rounded_serial_arg(ctx, &args[0])?;
    Ok(Value::Number(iso_week(serial, ctx.date1904)? as f64))
}

// ---- month arithmetic ------------------------------------------------------------------------

/// Serial of a calendar date computed in the 1900 system and moved to the active one, so that results before
/// 1904-01-01 are negative serials in the 1904 system (down to 1900-01-01).
fn extended_serial(year: i64, month: i64, day: i64, date1904: bool) -> Option<i64> {
    let serial = ymd_to_serial(year, month, day, false)? as i64;
    if date1904 {
        let shifted = serial - 1462;
        (serial >= 1 && shifted <= MAX_SERIAL_1904).then_some(shifted)
    } else {
        Some(serial)
    }
}

/// Moves a serial by whole months; `end_of_month` picks the last day of the target month.
fn shift_months(ctx: &EvalContext<'_>, args: &[Arg], end_of_month: bool) -> Res<Value> {
    arity(args, 2, 2)?;
    let start = atp_num(ctx, &args[0])?.floor();
    let months = atp_int(ctx, &args[1])?;
    let serial = check_serial(ctx, start)?;
    let (y, m, d) = ymd(serial, ctx.date1904)?;
    let total = y * 12 + (m - 1) + months;
    let (ny, nm) = (total.div_euclid(12), total.rem_euclid(12) + 1);
    let day = if end_of_month {
        days_in_month(ny, nm)
    } else {
        d.min(days_in_month(ny, nm))
    };
    let out = extended_serial(ny, nm, day, ctx.date1904).ok_or(ErrorKind::Num)?;
    Ok(Value::Number(out as f64))
}

pub(super) fn edate(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    shift_months(ctx, args, false)
}

pub(super) fn eomonth(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    shift_months(ctx, args, true)
}

// ---- differences -----------------------------------------------------------------------------

pub(super) fn datedif(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 3)?;
    let start = num_arg(ctx, &args[0])?.floor();
    let end = num_arg(ctx, &args[1])?.floor();
    let unit = text_arg(ctx, &args[2])?.to_ascii_uppercase();
    let start = check_serial(ctx, start)?;
    let end = check_serial(ctx, end)?;
    if start > end {
        return Err(ErrorKind::Num);
    }
    let d1904 = ctx.date1904;
    let (y1, m1, d1) = ymd(start, d1904)?;
    let (y2, m2, d2) = ymd(end, d1904)?;
    let months = (y2 - y1) * 12 + (m2 - m1) - i64::from(d2 < d1);
    // The days of the end date beyond the last whole month ("MD"); Excel counts 29 days in February 1900.
    let md = if d2 >= d1 {
        d2 - d1
    } else {
        let (py, pm) = if m2 == 1 { (y2 - 1, 12) } else { (y2, m2 - 1) };
        let in_month = if pm == 2 && is_leap_excel(py) {
            29
        } else {
            days_in_month(py, pm)
        };
        d2 - d1 + in_month
    };
    let n = match unit.as_str() {
        "D" => end - start,
        "M" => months,
        "Y" => months / 12,
        "YM" => months % 12,
        "MD" => md,
        // The whole months past the last full year, counted from the start date inside the start's own
        // year, plus the leftover days: this is where Excel's leap-day oddities come from.
        "YD" => {
            let through = ymd_to_serial(y1, m1 + months % 12, d1, d1904).ok_or(ErrorKind::Num)?;
            through as i64 - start + md
        }
        _ => return Err(ErrorKind::Num),
    };
    Ok(Value::Number(n as f64))
}

pub(super) fn days(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let end = num_arg(ctx, &args[0])?.floor();
    let start = num_arg(ctx, &args[1])?.floor();
    let end = check_serial(ctx, end)?;
    let start = check_serial(ctx, start)?;
    Ok(Value::Number((end - start) as f64))
}

/// Day count conventions on (year, month, day) triples.
#[derive(Clone, Copy, PartialEq)]
enum Count360 {
    /// `DAYS360` without the European flag: the end of February counts as day 30 for the start date only.
    DaysUs,
    /// `YEARFRAC` basis 0: NASD rules with the real February.
    YearfracUs,
    /// European: day 31 becomes 30.
    Eu,
}

fn days360_between(a: (i64, i64, i64), b: (i64, i64, i64), mode: Count360) -> i64 {
    let (y1, m1, mut d1) = a;
    let (y2, m2, mut d2) = b;
    match mode {
        Count360::Eu => {
            d1 = d1.min(30);
            d2 = d2.min(30);
        }
        Count360::DaysUs => {
            if m1 == 2 && d1 == if is_leap_excel(y1) { 29 } else { 28 } {
                d1 = 30;
            }
            if d2 == 31 && d1 >= 30 {
                d2 = 30;
            }
            if d1 == 31 {
                d1 = 30;
            }
        }
        Count360::YearfracUs => {
            let feb_end = |y: i64, m: i64, d: i64| m == 2 && d == days_in_month(y, 2);
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
    }
    (y2 - y1) * 360 + (m2 - m1) * 30 + (d2 - d1)
}

pub(super) fn days360(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 3)?;
    let start = num_arg(ctx, &args[0])?;
    let end = num_arg(ctx, &args[1])?;
    let european = match args.get(2) {
        None => false,
        Some(a) => super::bool_arg(ctx, a)?,
    };
    let a = ymd(rounded_serial(ctx, start)?, ctx.date1904)?;
    let b = ymd(rounded_serial(ctx, end)?, ctx.date1904)?;
    let mode = if european {
        Count360::Eu
    } else {
        Count360::DaysUs
    };
    Ok(Value::Number(days360_between(a, b, mode) as f64))
}

fn year_days(year: i64) -> i64 {
    if is_leap(year) { 366 } else { 365 }
}

pub(super) fn yearfrac(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 3)?;
    let start = atp_num(ctx, &args[0])?;
    let end = atp_num(ctx, &args[1])?;
    let basis = atp_opt_int(ctx, args, 2, 0)?;
    let mut start = rounded_serial(ctx, start)?;
    let mut end = rounded_serial(ctx, end)?;
    if !(0..=4).contains(&basis) {
        return Err(ErrorKind::Num);
    }
    if start > end {
        std::mem::swap(&mut start, &mut end);
    }
    let d1904 = ctx.date1904;
    let a = ymd(start, d1904)?;
    let b = ymd(end, d1904)?;
    let actual = (end - start) as f64;
    let frac = match basis {
        0 => days360_between(a, b, Count360::YearfracUs) as f64 / 360.0,
        4 => days360_between(a, b, Count360::Eu) as f64 / 360.0,
        2 => actual / 360.0,
        3 => actual / 365.0,
        _ => {
            let (y1, m1, d1) = a;
            let (y2, m2, d2) = b;
            let one_year = y1 == y2 || (y2 == y1 + 1 && (m1 > m2 || (m1 == m2 && d1 >= d2)));
            let length = if one_year {
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
            };
            actual / length
        }
    };
    Ok(Value::Number(frac))
}

// ---- working days ---------------------------------------------------------------------------

/// Weekend days as a mask indexed by Sunday-based weekday.
type Weekend = [bool; 7];

const SAT_SUN: Weekend = [true, false, false, false, false, false, true];

/// The weekend parameter: a code 1-7 or 11-17, or a seven character string starting on Monday. An error value
/// in that position is not propagated: `NETWORKDAYS.INTL` turns it into `#VALUE!`, `WORKDAY.INTL` into `#NUM!`.
fn weekend_mask(ctx: &EvalContext<'_>, arg: Option<&Arg>, on_error: ErrorKind) -> Res<Weekend> {
    let Some(arg) = arg else {
        return Ok(SAT_SUN);
    };
    match ctx.scalar(arg) {
        Value::Error(_) => Err(on_error),
        Value::Text(t) => {
            let bytes = t.as_bytes();
            if bytes.len() != 7 || !bytes.iter().all(|b| matches!(b, b'0' | b'1')) {
                return Err(ErrorKind::Value);
            }
            let mut mask = [false; 7];
            for (i, b) in bytes.iter().enumerate() {
                mask[(i + 1) % 7] = *b == b'1';
            }
            Ok(mask)
        }
        v => code_mask(v.to_number_in(ctx.date1904)?.trunc()),
    }
}

fn code_mask(code: f64) -> Res<Weekend> {
    let mut mask = [false; 7];
    let code = code as i64;
    match code {
        1..=7 => {
            // 1 = Sat/Sun, 2 = Sun/Mon, ... 7 = Fri/Sat.
            let first = (code + 5) % 7;
            mask[first as usize] = true;
            mask[((first + 1) % 7) as usize] = true;
        }
        11..=17 => mask[(code - 11) as usize] = true,
        _ => return Err(ErrorKind::Num),
    }
    Ok(mask)
}

/// The holiday list as serials.
fn holidays(ctx: &EvalContext<'_>, arg: Option<&Arg>) -> Res<BTreeSet<i64>> {
    let mut out = BTreeSet::new();
    let Some(arg) = arg else {
        return Ok(out);
    };
    for v in ctx.cells(arg) {
        match v {
            Value::Empty => {}
            Value::Error(e) => return Err(e),
            Value::Bool(_) | Value::Array(_) => return Err(ErrorKind::Value),
            v => {
                let n = v.to_number_in(ctx.date1904)?.floor();
                out.insert(check_serial(ctx, n)?);
            }
        }
    }
    Ok(out)
}

fn is_workday(serial: i64, mask: &Weekend, date1904: bool) -> bool {
    !mask[sunday0(serial, date1904) as usize]
}

/// Number of working days in `lo..=hi`, ignoring holidays.
fn count_workdays(lo: i64, hi: i64, mask: &Weekend, date1904: bool) -> i64 {
    if hi < lo {
        return 0;
    }
    let per_week = mask.iter().filter(|w| !**w).count() as i64;
    let len = hi - lo + 1;
    let mut count = len / 7 * per_week;
    for d in (hi - len % 7 + 1)..=hi {
        if is_workday(d, mask, date1904) {
            count += 1;
        }
    }
    count
}

fn networkdays_impl(
    ctx: &EvalContext<'_>,
    args: &[Arg],
    mask: Res<Weekend>,
    hol_at: usize,
) -> Res<Value> {
    let a = atp_num(ctx, &args[0])?.floor();
    let b = atp_num(ctx, &args[1])?.floor();
    let mask = mask?;
    let hol = holidays(ctx, optional(args, hol_at))?;
    let (a, b) = (check_serial(ctx, a)?, check_serial(ctx, b)?);
    let d1904 = ctx.date1904;
    let (lo, hi, sign) = if a <= b { (a, b, 1) } else { (b, a, -1) };
    let mut n = count_workdays(lo, hi, &mask, d1904);
    n -= hol
        .range(lo..=hi)
        .filter(|h| is_workday(**h, &mask, d1904))
        .count() as i64;
    Ok(Value::Number((sign * n) as f64))
}

pub(super) fn networkdays(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 3)?;
    networkdays_impl(ctx, args, Ok(SAT_SUN), 2)
}

pub(super) fn networkdays_intl(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 4)?;
    networkdays_impl(
        ctx,
        args,
        weekend_mask(ctx, optional(args, 2), ErrorKind::Value),
        3,
    )
}

/// The serial `n` working days away from `start` (n != 0), holidays excluded. The mask has a working day.
fn shift_workdays(start: i64, n: i64, mask: &Weekend, hol: &BTreeSet<i64>, date1904: bool) -> i64 {
    let per_week = mask.iter().filter(|w| !**w).count() as i64;
    let step = n.signum();
    let hol: Vec<i64> = hol
        .iter()
        .copied()
        .filter(|h| is_workday(*h, mask, date1904))
        .collect();
    let mut need = n.abs();
    let mut pos = start;
    loop {
        // Jump whole weeks, then walk the remainder.
        let weeks = (need - 1) / per_week;
        let mut cur = pos + step * 7 * weeks;
        let mut left = need - weeks * per_week;
        while left > 0 {
            cur += step;
            if is_workday(cur, mask, date1904) {
                left -= 1;
            }
        }
        // Holidays between pos (exclusive) and cur (inclusive) cost extra working days.
        let (lo, hi) = if step > 0 {
            (pos + 1, cur)
        } else {
            (cur, pos - 1)
        };
        let skipped = hol.iter().filter(|h| (lo..=hi).contains(h)).count() as i64;
        if skipped == 0 {
            return cur;
        }
        pos = cur;
        need = skipped;
    }
}

fn workday_impl(
    ctx: &EvalContext<'_>,
    args: &[Arg],
    mask: Res<Weekend>,
    hol_at: usize,
) -> Res<Value> {
    let start = atp_num(ctx, &args[0])?.floor();
    // The day count rounds down, so -0.9 is one day back.
    let n = atp_num(ctx, &args[1])?.floor();
    let mask = mask?;
    let hol = holidays(ctx, optional(args, hol_at))?;
    let start = check_serial(ctx, start)?;
    if n.abs() > 10_000_000.0 {
        return Err(ErrorKind::Num);
    }
    let n = n as i64;
    if n == 0 {
        return Ok(Value::Number(start as f64));
    }
    if mask.iter().all(|w| *w) {
        return Err(ErrorKind::Value);
    }
    let out = shift_workdays(start, n, &mask, &hol, ctx.date1904);
    Ok(Value::Number(in_range(out, ctx.date1904)?))
}

pub(super) fn workday(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 3)?;
    workday_impl(ctx, args, Ok(SAT_SUN), 2)
}

pub(super) fn workday_intl(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 4)?;
    workday_impl(
        ctx,
        args,
        weekend_mask(ctx, optional(args, 2), ErrorKind::Num),
        3,
    )
}

// ---- text to date and time -------------------------------------------------------------------

fn text_serial(ctx: &EvalContext<'_>, arg: &Arg) -> Res<f64> {
    match scalar(ctx, arg)? {
        Value::Text(t) => {
            crate::value::parse_datetime_text(&t, ctx.date1904).ok_or(ErrorKind::Value)
        }
        _ => Err(ErrorKind::Value),
    }
}

pub(super) fn datevalue(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    let serial = text_serial(ctx, &args[0])?;
    Ok(Value::Number(serial.floor()))
}

pub(super) fn timevalue(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    let serial = text_serial(ctx, &args[0])?;
    Ok(Value::Number(serial - serial.floor()))
}
