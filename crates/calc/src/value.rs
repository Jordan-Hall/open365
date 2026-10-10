//! Cell values, Excel error codes, type coercion and the Excel ordering of values.
//!
//! Coercion and ordering follow the behaviour documented in the public Excel
//! function reference (arithmetic coerces text and booleans to numbers, comparison
//! orders numbers before text before booleans, text comparison ignores case).

use std::cmp::Ordering;
use std::fmt;

/// An Excel error value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// `#NULL!`
    Null,
    /// `#DIV/0!`
    DivZero,
    /// `#VALUE!`
    Value,
    /// `#REF!`
    Ref,
    /// `#NAME?`
    Name,
    /// `#NUM!`
    Num,
    /// `#N/A`
    NA,
}

impl ErrorKind {
    /// Every error kind, in Excel's error-number order.
    pub const ALL: [ErrorKind; 7] = [
        ErrorKind::Null,
        ErrorKind::DivZero,
        ErrorKind::Value,
        ErrorKind::Ref,
        ErrorKind::Name,
        ErrorKind::Num,
        ErrorKind::NA,
    ];

    /// The text Excel displays for this error.
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorKind::Null => "#NULL!",
            ErrorKind::DivZero => "#DIV/0!",
            ErrorKind::Value => "#VALUE!",
            ErrorKind::Ref => "#REF!",
            ErrorKind::Name => "#NAME?",
            ErrorKind::Num => "#NUM!",
            ErrorKind::NA => "#N/A",
        }
    }

    /// Parses an error literal such as `#N/A` (case-insensitive).
    pub fn parse(text: &str) -> Option<ErrorKind> {
        ErrorKind::ALL
            .iter()
            .copied()
            .find(|k| k.as_str().eq_ignore_ascii_case(text))
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::error::Error for ErrorKind {}

/// A cell value or intermediate evaluation result.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A blank cell.
    Empty,
    /// A number (dates are numbers too).
    Number(f64),
    /// A text string.
    Text(String),
    /// A boolean.
    Bool(bool),
    /// An error value.
    Error(ErrorKind),
    /// A rectangular array of values, stored as rows.
    Array(Vec<Vec<Value>>),
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Number(n)
    }
}

impl From<i32> for Value {
    fn from(n: i32) -> Self {
        Value::Number(f64::from(n))
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Text(s.to_owned())
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Text(s)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

impl From<ErrorKind> for Value {
    fn from(e: ErrorKind) -> Self {
        Value::Error(e)
    }
}

impl Value {
    /// True for [`Value::Error`].
    pub fn is_error(&self) -> bool {
        matches!(self, Value::Error(_))
    }

    /// The error kind if this is an error value.
    pub fn as_error(&self) -> Option<ErrorKind> {
        match self {
            Value::Error(e) => Some(*e),
            _ => None,
        }
    }

    /// For arrays the top-left element, otherwise the value itself.
    pub fn top_left(&self) -> Value {
        match self {
            Value::Array(rows) => rows
                .first()
                .and_then(|r| r.first())
                .cloned()
                .unwrap_or(Value::Error(ErrorKind::Value)),
            v => v.clone(),
        }
    }

    /// Coerces to a number the way arithmetic does: blank is 0, booleans are 0/1 and text is
    /// parsed. Errors propagate; non-numeric text and arrays give `#VALUE!`.
    pub fn to_number(&self) -> Result<f64, ErrorKind> {
        self.to_number_in(false)
    }

    /// Like [`Value::to_number`], with date text such as `2024-01-15` converted in the given date system.
    pub fn to_number_in(&self, date1904: bool) -> Result<f64, ErrorKind> {
        match self {
            Value::Empty => Ok(0.0),
            Value::Number(n) => Ok(*n),
            Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            Value::Text(t) => parse_number_text_in(t, date1904).ok_or(ErrorKind::Value),
            Value::Error(e) => Err(*e),
            Value::Array(_) => Err(ErrorKind::Value),
        }
    }

    /// Coerces to text the way concatenation does. Errors propagate.
    pub fn to_text(&self) -> Result<String, ErrorKind> {
        match self {
            Value::Empty => Ok(String::new()),
            Value::Number(n) => Ok(number_to_text(*n)),
            Value::Bool(b) => Ok(if *b { "TRUE" } else { "FALSE" }.to_owned()),
            Value::Text(t) => Ok(t.clone()),
            Value::Error(e) => Err(*e),
            Value::Array(_) => Err(ErrorKind::Value),
        }
    }

    /// Coerces to a boolean the way `IF`/`AND` do: numbers are non-zero tests, the text
    /// `TRUE`/`FALSE` (any case) converts, anything else is `#VALUE!`.
    pub fn to_bool(&self) -> Result<bool, ErrorKind> {
        match self {
            Value::Empty => Ok(false),
            Value::Number(n) => Ok(*n != 0.0),
            Value::Bool(b) => Ok(*b),
            Value::Text(t) if t.eq_ignore_ascii_case("TRUE") => Ok(true),
            Value::Text(t) if t.eq_ignore_ascii_case("FALSE") => Ok(false),
            Value::Text(_) | Value::Array(_) => Err(ErrorKind::Value),
            Value::Error(e) => Err(*e),
        }
    }
}

/// Largest magnitude Excel accepts for a number typed or parsed from text (values from `1E308` up are rejected).
const NUMBER_LIMIT: f64 = 1e308;

/// Parses a decimal number literal the way Excel's formula parser does: only the first 15 significant
/// digits count (the rest are dropped, not rounded), values of `1E308` or more are rejected and values below
/// the smallest normal number become zero. `raw` has no sign.
pub fn excel_literal(raw: &str) -> Option<f64> {
    let (mantissa, exponent) = match raw.find(['e', 'E']) {
        Some(i) => (&raw[..i], raw[i + 1..].parse::<i32>().ok()?),
        None => (raw, 0),
    };
    let (int_part, frac_part) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits: String = format!("{int_part}{frac_part}");
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let leading = digits.bytes().take_while(|&b| b == b'0').count();
    let significant = &digits[leading..];
    if significant.is_empty() {
        return Some(0.0);
    }
    let kept = &significant[..significant.len().min(15)];
    // value = 0.kept x 10^point, where point counts the integer digits left of the decimal point.
    let point = int_part.len() as i64 - leading as i64 + i64::from(exponent);
    if point > 400 {
        return None;
    }
    if point < -400 {
        return Some(0.0);
    }
    let n: f64 = format!("0.{kept}e{point}").parse().ok()?;
    if !n.is_finite() || n >= NUMBER_LIMIT {
        return None;
    }
    if n < f64::MIN_POSITIVE {
        // Excel turns literals just below the smallest normal number into 0 and rejects smaller ones.
        return if point > -308 { Some(0.0) } else { None };
    }
    Some(n)
}

/// Parses text as Excel would for arithmetic coercion: spaces around the number, sign (also `(12)` for
/// -12), optional `$`, digits with thousands separators, decimal point, exponent, a percent sign on either
/// end, and ISO dates (`2024-01-15`) or times (`12:30`, `1:30 PM`).
pub fn parse_number_text(text: &str) -> Option<f64> {
    parse_number_text_in(text, false)
}

/// Like [`parse_number_text`], converting date text in the given date system.
pub fn parse_number_text_in(text: &str, date1904: bool) -> Option<f64> {
    let s = text.trim_matches(' ');
    if s.is_empty() {
        return None;
    }
    parse_decimal_text(s).or_else(|| parse_date_time_text(s, date1904))
}

fn parse_decimal_text(s: &str) -> Option<f64> {
    if let Some(inner) = s.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
        return parse_signed_text(inner.trim_matches(' '), false).map(|n| -n);
    }
    parse_signed_text(s, true)
}

fn parse_signed_text(s: &str, allow_sign: bool) -> Option<f64> {
    let mut rest = s;
    let mut neg = false;
    if allow_sign && let Some(c) = rest.chars().next().filter(|c| matches!(c, '+' | '-')) {
        neg = c == '-';
        rest = rest[1..].trim_start_matches(' ');
    }
    rest = rest.strip_prefix('$').unwrap_or(rest);
    let mut percent = false;
    if let Some(r) = rest.strip_prefix('%') {
        percent = true;
        rest = r.trim_start_matches(' ');
    }
    if let Some(r) = rest.strip_suffix('%') {
        if percent {
            return None;
        }
        percent = true;
        rest = r.trim_end_matches(' ');
    }
    let (mantissa, exponent) = match rest.find(['e', 'E']) {
        Some(i) => (&rest[..i], Some(&rest[i + 1..])),
        None => (rest, None),
    };
    let (int_part, frac_part) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if int_part.is_empty() && frac_part.is_empty() {
        return None;
    }
    if !frac_part.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let int_digits = if int_part.contains(',') {
        let groups: Vec<&str> = int_part.split(',').collect();
        let valid = !groups[0].is_empty()
            && groups[0].len() <= 3
            && groups[1..].iter().all(|g| g.len() >= 3)
            && groups.iter().all(|g| g.bytes().all(|c| c.is_ascii_digit()));
        if !valid {
            return None;
        }
        groups.concat()
    } else {
        if !int_part.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        int_part.to_owned()
    };
    let mut clean = format!("{int_digits}.{frac_part}");
    if let Some(exp) = exponent {
        let digits = exp.strip_prefix(['+', '-']).unwrap_or(exp);
        if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        clean.push('e');
        clean.push_str(exp);
    }
    let mut n = excel_literal(&clean)?;
    if percent {
        n /= 100.0;
    }
    Some(if neg { -n } else { n })
}

/// `hh:mm[:ss[.fff]] [AM|PM]` as a fraction of a day.
fn parse_time_text(s: &str) -> Option<f64> {
    let (clock, meridiem) = match s.rsplit_once(' ') {
        Some((c, m)) if m.eq_ignore_ascii_case("AM") || m.eq_ignore_ascii_case("PM") => {
            (c.trim_end_matches(' '), Some(m.eq_ignore_ascii_case("PM")))
        }
        _ => (s, None),
    };
    let mut parts = clock.split(':');
    let mut hours: f64 = parts.next()?.parse::<u32>().ok()?.into();
    let minutes: f64 = parts.next()?.parse::<u32>().ok()?.into();
    let seconds: f64 = match parts.next() {
        None => 0.0,
        Some(sec) if !sec.is_empty() && sec.bytes().all(|b| b.is_ascii_digit() || b == b'.') => {
            sec.parse().ok()?
        }
        Some(_) => return None,
    };
    if parts.next().is_some() {
        return None;
    }
    if let Some(pm) = meridiem {
        if !(1.0..=12.0).contains(&hours) {
            return None;
        }
        hours = (hours % 12.0) + if pm { 12.0 } else { 0.0 };
    }
    Some((hours * 3600.0 + minutes * 60.0 + seconds) / 86_400.0)
}

fn parse_date_time_text(s: &str, date1904: bool) -> Option<f64> {
    if s.contains(':') && !s.contains('-') {
        return parse_time_text(s);
    }
    let (date, time) = match s.split_once(' ') {
        Some((d, t)) => (d, Some(t.trim_start_matches(' '))),
        None => (s, None),
    };
    let mut parts = date.split('-');
    let (y, m, d) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some()
        || y.len() != 4
        || !(1..=2).contains(&m.len())
        || !(1..=2).contains(&d.len())
    {
        return None;
    }
    let (y, m, d): (i64, i64, i64) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
    if !(1..=12).contains(&m) || d < 1 {
        return None;
    }
    let serial = crate::functions::ymd_to_serial(y, m, d, date1904)?;
    // ymd_to_serial rolls days over; text dates must be real calendar days.
    if crate::functions::serial_to_ymd(serial, date1904)? != (y, m, d) {
        return None;
    }
    let fraction = match time {
        None | Some("") => 0.0,
        Some(t) => parse_time_text(t)?,
    };
    Some(serial + fraction)
}

/// Longest positional (non-scientific) rendering, in characters, before General switches to
/// scientific notation. Measured against Excel: `1E19` and `1.23456789012345E-4` stay positional, `1E20`
/// and `1.23456789012345E-5` do not.
const MAX_POSITIONAL_LEN: usize = 20;

/// Rounds to 15 significant digits and returns `(digits without trailing zeros, decimal exponent)` where the
/// value is `d.ddd x 10^exp`.
fn digits15(n: f64) -> (String, i32) {
    let sci = format!("{:.14e}", n.abs());
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let all_digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let digits = all_digits.trim_end_matches('0');
    (if digits.is_empty() { "0" } else { digits }.to_owned(), exp)
}

/// Formats a number the way Excel's General format does when converting to text: 15 significant
/// digits, no trailing zeros, positional notation up to 20 characters and `1.5E-05` style scientific
/// notation beyond that.
pub fn number_to_text(n: f64) -> String {
    if n == 0.0 {
        return "0".to_owned();
    }
    if !n.is_finite() {
        return ErrorKind::Num.as_str().to_owned();
    }
    let (digits, exp) = digits15(n);
    let sign = if n < 0.0 { "-" } else { "" };
    let positional_len = if exp >= 0 {
        let int_len = exp as usize + 1;
        if digits.len() > int_len {
            digits.len() + 1
        } else {
            int_len
        }
    } else {
        digits.len() + (-exp) as usize + 1
    };
    if positional_len > MAX_POSITIONAL_LEN {
        let (first, rest) = digits.split_at(1);
        let frac = if rest.is_empty() {
            String::new()
        } else {
            format!(".{rest}")
        };
        let esign = if exp < 0 { '-' } else { '+' };
        return format!("{sign}{first}{frac}E{esign}{:02}", exp.abs());
    }
    if exp >= 0 {
        let int_len = exp as usize + 1;
        if digits.len() <= int_len {
            format!("{sign}{digits}{}", "0".repeat(int_len - digits.len()))
        } else {
            let (i, f) = digits.split_at(int_len);
            format!("{sign}{i}.{f}")
        }
    } else {
        format!("{sign}0.{}{digits}", "0".repeat((-exp - 1) as usize))
    }
}

/// `x` rounded to 15 significant digits.
pub(crate) fn round15(x: f64) -> f64 {
    format!("{x:.14e}").parse().unwrap_or(x)
}

/// Compares two numbers the way Excel's comparison operators do: values that are equal after rounding
/// to 15 significant digits are equal. (Lookups such as `MATCH` use exact equality instead.)
pub fn compare_numbers(a: f64, b: f64) -> Ordering {
    if a == b {
        return Ordering::Equal;
    }
    let scale = a.abs().max(b.abs());
    // Two doubles further apart than 2e-14 relative cannot round to the same 15-digit value.
    if (a - b).abs() > scale * 2e-14 {
        return a.partial_cmp(&b).unwrap_or(Ordering::Equal);
    }
    let (ra, rb) = (round15(a), round15(b));
    if ra == rb {
        Ordering::Equal
    } else {
        a.partial_cmp(&b).unwrap_or(Ordering::Equal)
    }
}

pub use crate::collate::compare_text;

fn rank(v: &Value) -> u8 {
    match v {
        Value::Number(_) | Value::Empty => 0,
        Value::Text(_) => 1,
        Value::Bool(_) => 2,
        Value::Error(_) | Value::Array(_) => 3,
    }
}

/// Excel comparison ordering: numbers < text < booleans, text compared case-insensitively.
/// A blank compares as 0, empty text or FALSE depending on the other operand. Errors and
/// arrays are not comparable and sort last.
pub fn compare_values(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::Empty, Value::Empty) => Ordering::Equal,
        (Value::Empty, Value::Number(y)) => compare_numbers(0.0, *y),
        (Value::Number(x), Value::Empty) => compare_numbers(*x, 0.0),
        (Value::Empty, Value::Text(y)) => compare_text("", y),
        (Value::Text(x), Value::Empty) => compare_text(x, ""),
        (Value::Empty, Value::Bool(y)) => false.cmp(y),
        (Value::Bool(x), Value::Empty) => x.cmp(&false),
        (Value::Number(x), Value::Number(y)) => compare_numbers(*x, *y),
        (Value::Text(x), Value::Text(y)) => compare_text(x, y),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        _ => rank(a).cmp(&rank(b)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_matches_excel() {
        let shown: Vec<String> = ErrorKind::ALL.iter().map(ToString::to_string).collect();
        assert_eq!(
            shown,
            [
                "#NULL!", "#DIV/0!", "#VALUE!", "#REF!", "#NAME?", "#NUM!", "#N/A"
            ]
        );
    }

    #[test]
    fn error_parse_is_case_insensitive() {
        assert_eq!(ErrorKind::parse("#n/a"), Some(ErrorKind::NA));
        assert_eq!(ErrorKind::parse("#DIV/0!"), Some(ErrorKind::DivZero));
        assert_eq!(ErrorKind::parse("#bogus"), None);
    }

    #[test]
    fn coerce_to_number() {
        assert_eq!(Value::Empty.to_number(), Ok(0.0));
        assert_eq!(Value::Bool(true).to_number(), Ok(1.0));
        assert_eq!(Value::Bool(false).to_number(), Ok(0.0));
        assert_eq!(Value::from("  12.5 ").to_number(), Ok(12.5));
        assert_eq!(Value::from("1,234.5").to_number(), Ok(1234.5));
        assert_eq!(Value::from("-$3").to_number(), Ok(-3.0));
        assert_eq!(Value::from("50%").to_number(), Ok(0.5));
        assert_eq!(Value::from("1e3").to_number(), Ok(1000.0));
        assert_eq!(Value::from(".5").to_number(), Ok(0.5));
        assert_eq!(Value::from("abc").to_number(), Err(ErrorKind::Value));
        assert_eq!(Value::from("").to_number(), Err(ErrorKind::Value));
        assert_eq!(Value::from("1,23").to_number(), Err(ErrorKind::Value));
        assert_eq!(Value::from("inf").to_number(), Err(ErrorKind::Value));
        assert_eq!(Value::from("1e").to_number(), Err(ErrorKind::Value));
        assert_eq!(Value::Error(ErrorKind::NA).to_number(), Err(ErrorKind::NA));
    }

    #[test]
    fn coerce_to_text() {
        assert_eq!(Value::Empty.to_text(), Ok(String::new()));
        assert_eq!(Value::Bool(true).to_text(), Ok("TRUE".to_owned()));
        assert_eq!(Value::Number(2.0).to_text(), Ok("2".to_owned()));
        assert_eq!(Value::Error(ErrorKind::Ref).to_text(), Err(ErrorKind::Ref));
    }

    #[test]
    fn coerce_to_bool() {
        assert_eq!(Value::Number(0.0).to_bool(), Ok(false));
        assert_eq!(Value::Number(-2.0).to_bool(), Ok(true));
        assert_eq!(Value::from("true").to_bool(), Ok(true));
        assert_eq!(Value::from("x").to_bool(), Err(ErrorKind::Value));
        assert_eq!(Value::Empty.to_bool(), Ok(false));
    }

    #[test]
    fn general_number_format() {
        assert_eq!(number_to_text(0.1 + 0.2), "0.3");
        assert_eq!(number_to_text(1.0 / 3.0), "0.333333333333333");
        assert_eq!(number_to_text(100.0), "100");
        assert_eq!(number_to_text(-2.5), "-2.5");
        assert_eq!(number_to_text(123_456_789_012_345.0), "123456789012345");
        assert_eq!(number_to_text(1e15), "1000000000000000");
        assert_eq!(number_to_text(1e19), "10000000000000000000");
        assert_eq!(number_to_text(1e20), "1E+20");
        assert_eq!(number_to_text(1_234_567_890_123_456.0), "1234567890123460");
        assert_eq!(number_to_text(1.5e20), "1.5E+20");
        assert_eq!(number_to_text(0.0001), "0.0001");
        assert_eq!(number_to_text(1e-10), "0.0000000001");
        assert_eq!(number_to_text(1.23456789012345e-5), "1.23456789012345E-05");
        assert_eq!(number_to_text(1.23456789012345e-4), "0.000123456789012345");
        assert_eq!(number_to_text(-0.0), "0");
        assert_eq!(number_to_text(1e300), "1E+300");
        assert_eq!(number_to_text(0.5), "0.5");
        assert_eq!(number_to_text(1234.5678), "1234.5678");
    }

    #[test]
    fn ordering_numbers_text_bools() {
        let n = Value::Number(1e9);
        let t = Value::from("a");
        let b = Value::Bool(false);
        assert_eq!(compare_values(&n, &t), Ordering::Less);
        assert_eq!(compare_values(&t, &b), Ordering::Less);
        assert_eq!(compare_values(&n, &b), Ordering::Less);
        assert_eq!(compare_values(&b, &n), Ordering::Greater);
    }

    #[test]
    fn ordering_text_ignores_case() {
        assert_eq!(
            compare_values(&Value::from("ABC"), &Value::from("abc")),
            Ordering::Equal
        );
        assert_eq!(
            compare_values(&Value::from("a"), &Value::from("B")),
            Ordering::Less
        );
    }

    #[test]
    fn ordering_blank() {
        assert_eq!(
            compare_values(&Value::Empty, &Value::Number(0.0)),
            Ordering::Equal
        );
        assert_eq!(
            compare_values(&Value::Empty, &Value::from("")),
            Ordering::Equal
        );
        assert_eq!(
            compare_values(&Value::Empty, &Value::Bool(false)),
            Ordering::Equal
        );
        assert_eq!(
            compare_values(&Value::Empty, &Value::Number(1.0)),
            Ordering::Less
        );
    }

    #[test]
    fn number_comparison_equates_15_digits() {
        assert_eq!(compare_numbers(1.0, 1.0 + 4.9e-15), Ordering::Equal);
        assert_eq!(compare_numbers(1.0, 1.0 + 5.1e-15), Ordering::Less);
        assert_eq!(compare_numbers(1e15, 1e15 + 0.125), Ordering::Equal);
    }

    #[test]
    fn number_comparison_tolerates_binary_noise() {
        assert_eq!(compare_numbers(0.1 + 0.2, 0.3), Ordering::Equal);
        assert_eq!(compare_numbers(1.0, 1.0001), Ordering::Less);
    }

    #[test]
    fn top_left_of_array() {
        let a = Value::Array(vec![vec![Value::Number(1.0), Value::Number(2.0)]]);
        assert_eq!(a.top_left(), Value::Number(1.0));
        assert_eq!(Value::Bool(true).top_left(), Value::Bool(true));
    }
}
