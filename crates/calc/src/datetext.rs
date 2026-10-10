//! Date and time text: what `DATEVALUE`, `TIMEVALUE`, `VALUE` and arithmetic accept as a date or a time.
//!
//! Only forms that read the same in every locale are supported: year-first numeric dates
//! (`2024-03-15`, `2024/3/5`), day-first dates with an English month name (`15 Mar 2024`, `15-March-24`) and
//! clock times (`12:30`, `1:30:15.25 PM`, `12:30.5` for minutes and seconds). A date and a time may follow each
//! other in either order. Ambiguous numeric forms (`1/2/2024`) and dates without a year are not recognised.
//!
//! The rules were measured against Excel; see `docs/testing/calc-conformance.md`.

use crate::functions::{serial_to_ymd, ymd_to_serial};

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// The serial number of date and/or time text, or `None` when it is neither. `text` must not have
/// leading or trailing spaces.
pub(crate) fn parse(text: &str, date1904: bool) -> Option<f64> {
    if !text.is_ascii() {
        return None;
    }
    if let Some(t) = parse_time(text) {
        return Some(t);
    }
    if let Some(d) = parse_date(text, date1904) {
        return Some(d);
    }
    // A date and a time separated by spaces, in either order.
    let bytes = text.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b != b' ' || i == 0 || bytes[i - 1] == b' ' {
            continue;
        }
        let mut j = i;
        while j < bytes.len() && bytes[j] == b' ' {
            j += 1;
        }
        if j == bytes.len() {
            break;
        }
        let (left, right) = (&text[..i], &text[j..]);
        if let (Some(d), Some(t)) = (parse_date(left, date1904), parse_time(right)) {
            return Some(d + t);
        }
        if let (Some(t), Some(d)) = (parse_time(left), parse_date(right, date1904)) {
            return Some(d + t);
        }
    }
    None
}

// ---- time ------------------------------------------------------------------------------------

/// Splits a trailing `AM`, `PM`, `A` or `P` (any case, preceded by a space) from a time.
fn split_meridiem(text: &str) -> (&str, Option<bool>) {
    let Some((rest, tail)) = text.rsplit_once(' ') else {
        return (text, None);
    };
    let pm = match tail.to_ascii_lowercase().as_str() {
        "am" | "a" => false,
        "pm" | "p" => true,
        _ => return (text, None),
    };
    (rest.trim_end_matches(' '), Some(pm))
}

/// One clock component: 1 to 4 digits, without a leading zero beyond the second digit.
fn clock_digits(part: &str) -> Option<i64> {
    if part.is_empty() || part.len() > 4 || !part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if part.len() > 2 && part.starts_with('0') {
        return None;
    }
    part.parse().ok()
}

/// Seconds with an optional fraction, rounded half up to the millisecond.
fn clock_seconds(part: &str) -> Option<(i64, f64)> {
    let (whole, frac) = match part.split_once('.') {
        Some((w, f)) => (w, f),
        None => (part, ""),
    };
    let secs = clock_digits(whole)?;
    if !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // Round the decimal fraction to three places by its digits, half up.
    let mut milli: i64 = 0;
    let digits: Vec<i64> = frac.bytes().map(|b| i64::from(b - b'0')).collect();
    for d in digits.iter().take(3) {
        milli = milli * 10 + d;
    }
    for _ in digits.len().min(3)..3 {
        milli *= 10;
    }
    if digits.get(3).is_some_and(|d| *d >= 5) {
        milli += 1;
    }
    Some((secs, secs as f64 + milli as f64 / 1000.0))
}

/// A time of day as a fraction of a day (hours may exceed 23).
fn parse_time(text: &str) -> Option<f64> {
    let (clock, meridiem) = split_meridiem(text);
    let mut parts: Vec<&str> = clock.split(':').map(|p| p.trim_matches(' ')).collect();
    if parts.len() == 1 && meridiem.is_none() {
        // A bare hour needs AM or PM.
        return None;
    }
    // `12:` and `12:30:` are complete times.
    if parts.len() > 1 && parts.last().is_some_and(|p| p.is_empty()) {
        parts.pop();
    }
    if parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    match parts.as_slice() {
        [hour] => finish_time(clock_digits(hour)?, 0, 0, 0.0, meridiem),
        [first, second] if second.contains('.') => {
            // Minutes and seconds.
            let (s_int, s) = clock_seconds(second)?;
            finish_time(0, clock_digits(first)?, s_int, s, meridiem)
        }
        [hour, minute] => finish_time(clock_digits(hour)?, clock_digits(minute)?, 0, 0.0, meridiem),
        [hour, minute, second] => {
            let (s_int, s) = clock_seconds(second)?;
            finish_time(
                clock_digits(hour)?,
                clock_digits(minute)?,
                s_int,
                s,
                meridiem,
            )
        }
        _ => None,
    }
}

/// Validates the components and turns them into a fraction of a day.
fn finish_time(h: i64, m: i64, s_int: i64, s: f64, meridiem: Option<bool>) -> Option<f64> {
    let overflow = i32::from(h >= 24) + i32::from(m >= 60) + i32::from(s_int >= 60);
    let hours = match meridiem {
        Some(pm) => {
            if h > 12 || overflow > 0 {
                return None;
            }
            h % 12 + if pm { 12 } else { 0 }
        }
        None => {
            if overflow > 1 {
                return None;
            }
            h
        }
    };
    Some((hours as f64 * 3600.0 + m as f64 * 60.0 + s) / 86_400.0)
}

// ---- date ------------------------------------------------------------------------------------

/// Digits of a day or month: one or two.
fn small_number(part: &str) -> Option<i64> {
    if part.is_empty() || part.len() > 2 || !part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    part.parse().ok()
}

/// A serial for a real calendar day (the day must exist in the month; 1900-02-29 does).
fn calendar_serial(y: i64, m: i64, d: i64, date1904: bool) -> Option<f64> {
    if !(1..=12).contains(&m) || d < 1 {
        return None;
    }
    let serial = ymd_to_serial(y, m, d, date1904)?;
    (serial_to_ymd(serial, date1904)? == (y, m, d)).then_some(serial)
}

fn parse_date(text: &str, date1904: bool) -> Option<f64> {
    parse_numeric_date(text, date1904).or_else(|| parse_named_date(text, date1904))
}

/// `year-month-day` or `year/month/day` with optional spaces around the separators.
fn parse_numeric_date(text: &str, date1904: bool) -> Option<f64> {
    let bytes = text.as_bytes();
    let mut fields: Vec<&str> = Vec::with_capacity(3);
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'-' || b == b'/' {
            fields.push(&text[start..i]);
            start = i + 1;
        }
    }
    fields.push(&text[start..]);
    let [y, m, d] = fields.as_slice() else {
        return None;
    };
    let (y, m, d) = (
        y.trim_matches(' '),
        m.trim_matches(' '),
        d.trim_matches(' '),
    );
    if y.len() != 4 || !y.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    calendar_serial(
        y.parse().ok()?,
        small_number(m)?,
        small_number(d)?,
        date1904,
    )
}

/// The month of a name or an abbreviation of at least three letters (`Mar`, `Sept`, `January`).
fn month_of(name: &str) -> Option<i64> {
    if name.len() < 3 {
        return None;
    }
    let lower = name.to_ascii_lowercase();
    MONTHS
        .iter()
        .position(|m| m.starts_with(&lower))
        .map(|i| i as i64 + 1)
}

/// `day month-name year`, the separators being spaces, `-` or `/` (or nothing).
fn parse_named_date(text: &str, date1904: bool) -> Option<f64> {
    let bytes = text.as_bytes();
    let d_len = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
    let day = small_number(&text[..d_len])?;
    let rest = text[d_len..].trim_start_matches(' ');
    let rest = rest
        .strip_prefix(['-', '/'])
        .unwrap_or(rest)
        .trim_start_matches(' ');
    let m_len = rest.bytes().take_while(u8::is_ascii_alphabetic).count();
    let month = month_of(&rest[..m_len])?;
    let rest = rest[m_len..].trim_start_matches(' ');
    let rest = rest
        .strip_prefix(['-', '/'])
        .unwrap_or(rest)
        .trim_start_matches(' ');
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let year: i64 = match rest.len() {
        1 | 2 => {
            let y: i64 = rest.parse().ok()?;
            if y < 30 { 2000 + y } else { 1900 + y }
        }
        4 => rest.parse().ok()?,
        _ => return None,
    };
    calendar_serial(year, month, day, date1904)
}

#[cfg(test)]
mod tests {
    use super::parse;

    fn p(s: &str) -> Option<f64> {
        parse(s, false)
    }

    #[test]
    fn times() {
        assert_eq!(p("12:00"), Some(0.5));
        assert_eq!(p("1:30 PM"), Some(0.5625));
        assert_eq!(p("12:30.5"), Some(750.5 / 86_400.0));
        assert_eq!(p("12 AM"), Some(0.0));
        assert_eq!(p("12:"), Some(0.5));
        assert_eq!(p("24:00"), Some(1.0));
        assert_eq!(p("23:60"), Some(1.0));
        assert_eq!(p("24:60"), None);
        assert_eq!(p("12"), None);
        assert_eq!(p("1:30pm"), None);
        assert_eq!(p("13:00 PM"), None);
        assert_eq!(p("10000:00"), None);
    }

    #[test]
    fn dates() {
        assert_eq!(p("2024-03-15"), Some(45366.0));
        assert_eq!(p("2024/3/5"), Some(45356.0));
        assert_eq!(p("2024 - 03 - 15"), Some(45366.0));
        assert_eq!(p("15 Mar 2024"), Some(45366.0));
        assert_eq!(p("15-March-24"), Some(45366.0));
        assert_eq!(p("1900-02-29"), Some(60.0));
        assert_eq!(p("2023-02-29"), None);
        assert_eq!(p("1899-12-31"), None);
        assert_eq!(p("2024-03-15 12:00"), Some(45366.5));
        assert_eq!(p("12:00 2024-03-15"), Some(45366.5));
        assert_eq!(p("2024-03-15T12:00"), None);
    }
}
