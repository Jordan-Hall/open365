//! Source values and their Excel ordering, plus the date arithmetic grouping needs.

use std::cmp::Ordering;

use open365_xlsx::{CellError, CellValue};

/// One value of a pivot source column or cache record.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Blank,
    /// A number; dates are serial numbers.
    Number(f64),
    Bool(bool),
    /// An error literal such as `#DIV/0!`.
    Error(String),
    Text(String),
}

impl Value {
    pub fn is_blank(&self) -> bool {
        matches!(self, Self::Blank)
    }

    pub fn number(&self) -> Option<f64> {
        match self {
            Self::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// Converts a worksheet cell value.
    pub fn from_cell(value: &CellValue) -> Self {
        match value {
            CellValue::Empty => Self::Blank,
            CellValue::Number(n) => Self::Number(*n),
            CellValue::Bool(b) => Self::Bool(*b),
            CellValue::Error(e) => Self::Error(e.code().to_owned()),
            other => match other.as_text() {
                Some(t) if !t.is_empty() => Self::Text(t.to_owned()),
                _ => Self::Blank,
            },
        }
    }

    /// The error value for an error literal, when it is one Excel knows.
    pub fn error_from_literal(literal: &str) -> Self {
        Self::Error(CellError::parse(literal).code().to_owned())
    }
}

/// Case-insensitive identity of a value: two source cells are the same pivot item when
/// their keys are equal (Excel merges `apple` and `Apple`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ItemKey {
    Blank,
    Number(u64),
    Bool(bool),
    Error(String),
    Text(String),
}

impl ItemKey {
    pub fn of(v: &Value) -> Self {
        match v {
            Value::Blank => Self::Blank,
            Value::Number(n) => Self::Number(if *n == 0.0 { 0.0f64 } else { *n }.to_bits()),
            Value::Bool(b) => Self::Bool(*b),
            Value::Error(e) => Self::Error(e.clone()),
            Value::Text(t) => Self::Text(t.to_lowercase()),
        }
    }
}

/// Position of a value class in Excel's ascending sort: numbers, then text, then
/// logicals, then errors, blanks last.
pub(crate) fn class_rank(v: &Value) -> u8 {
    match v {
        Value::Number(_) => 0,
        Value::Text(_) => 1,
        Value::Bool(_) => 2,
        Value::Error(_) => 3,
        Value::Blank => 4,
    }
}

/// Compares two values the way a pivot field sorts its items ascending. `text` compares
/// strings (locale collation).
pub(crate) fn compare_values(
    a: &Value,
    b: &Value,
    text: &dyn Fn(&str, &str) -> Ordering,
) -> Ordering {
    let (ra, rb) = (class_rank(a), class_rank(b));
    if ra != rb {
        return ra.cmp(&rb);
    }
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (Value::Text(x), Value::Text(y)) => text(x, y),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::Error(x), Value::Error(y)) => x.cmp(y),
        _ => Ordering::Equal,
    }
}

// ---------------------------------------------------------------------------------------------
// Dates. Serial numbers use the 1900 system (with Excel's phantom 29 Feb 1900) or the 1904 system.
// ---------------------------------------------------------------------------------------------

/// Days since 1970-01-01 of a proleptic Gregorian date.
pub(crate) fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Proleptic Gregorian date of a day count since 1970-01-01.
#[allow(dead_code)]
pub(crate) fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

const DAYS_1900_EPOCH: i64 = -25_569; // 1899-12-30 relative to 1970-01-01
const DAYS_1904_EPOCH: i64 = -24_107; // 1904-01-01 relative to 1970-01-01

fn epoch(date1904: bool) -> i64 {
    if date1904 {
        DAYS_1904_EPOCH
    } else {
        DAYS_1900_EPOCH
    }
}

/// Year, month, day of the whole-day part of `serial`.
#[allow(dead_code)]
pub(crate) fn serial_to_ymd(serial: f64, date1904: bool) -> (i64, i64, i64) {
    let day = serial.floor() as i64;
    if !date1904 && day < 61 {
        // Serial 60 is the phantom 29 Feb 1900; 0 is the "zeroth" January 1900.
        return match day {
            ..=0 => (1900, 1, 0),
            1..=31 => (1900, 1, day),
            32..=59 => (1900, 2, day - 31),
            _ => (1900, 2, 29),
        };
    }
    civil_from_days(day + epoch(date1904))
}

/// Serial number of a date.
pub(crate) fn ymd_to_serial(y: i64, m: i64, d: i64, date1904: bool) -> f64 {
    let days = days_from_civil(y, m, d) - epoch(date1904);
    if !date1904 && days < 61 && (y, m) <= (1900, 2) {
        // Dates before 1 Mar 1900 skip the phantom day.
        return days as f64;
    }
    days as f64
}

/// Parses `2024-01-05T13:00:00` (the form of `<d v>` in a cache) into a serial number.
pub(crate) fn iso_to_serial(text: &str, date1904: bool) -> Option<f64> {
    let (date, time) = match text.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (text, None),
    };
    let mut parts = date.splitn(3, '-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    let mut serial = ymd_to_serial(y, m, d, date1904);
    if let Some(t) = time {
        let t = t.trim_end_matches('Z');
        let mut it = t.splitn(3, ':');
        let h: f64 = it.next()?.parse().ok()?;
        let mi: f64 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
        let s: f64 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
        serial += (h * 3600.0 + mi * 60.0 + s) / 86_400.0;
    }
    Some(serial)
}

/// The inverse of [`iso_to_serial`]; used when a refreshed cache is written back.
#[allow(dead_code)]
pub(crate) fn serial_to_iso(serial: f64, date1904: bool) -> String {
    let (y, m, d) = serial_to_ymd(serial, date1904);
    let secs = ((serial - serial.floor()) * 86_400.0).round() as i64;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_round_trips_through_dates() {
        assert_eq!(serial_to_ymd(45292.0, false), (2024, 1, 1));
        assert_eq!(ymd_to_serial(2024, 1, 1, false), 45292.0);
        assert_eq!(serial_to_ymd(61.0, false), (1900, 3, 1));
        assert_eq!(serial_to_ymd(60.0, false), (1900, 2, 29));
        assert_eq!(serial_to_ymd(59.0, false), (1900, 2, 28));
        assert_eq!(serial_to_ymd(1.0, false), (1900, 1, 1));
        assert_eq!(serial_to_ymd(0.0, true), (1904, 1, 1));
        assert_eq!(iso_to_serial("2024-01-05T12:00:00", false), Some(45296.5));
        assert_eq!(serial_to_iso(45296.5, false), "2024-01-05T12:00:00");
    }
}
