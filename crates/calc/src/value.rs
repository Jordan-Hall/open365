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
        match self {
            Value::Empty => Ok(0.0),
            Value::Number(n) => Ok(*n),
            Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            Value::Text(t) => parse_number_text(t).ok_or(ErrorKind::Value),
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

/// Parses text as Excel would for arithmetic coercion: optional surrounding spaces, sign,
/// optional `$`, digits with optional thousands separators, decimal point, exponent and a
/// trailing percent sign.
pub fn parse_number_text(text: &str) -> Option<f64> {
    let s = text.trim();
    if s.is_empty() {
        return None;
    }
    let mut chars = s.chars().peekable();
    let mut neg = false;
    if let Some(&c) = chars.peek()
        && (c == '+' || c == '-')
    {
        neg = c == '-';
        chars.next();
    }
    if chars.peek() == Some(&'$') {
        chars.next();
    }
    let rest: String = chars.collect();
    let (body, percent) = match rest.strip_suffix('%') {
        Some(b) => (b.trim_end(), true),
        None => (rest.as_str(), false),
    };
    let (mantissa, exponent) = match body.find(['e', 'E']) {
        Some(i) => (&body[..i], Some(&body[i + 1..])),
        None => (body, None),
    };
    let (int_part, frac_part) = match mantissa.split_once('.') {
        Some((i, f)) => (i, f),
        None => (mantissa, ""),
    };
    if int_part.is_empty() && frac_part.is_empty() {
        return None;
    }
    if !frac_part.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let int_digits: String = if int_part.contains(',') {
        let groups: Vec<&str> = int_part.split(',').collect();
        let valid = !groups[0].is_empty()
            && groups[0].len() <= 3
            && groups[1..].iter().all(|g| g.len() == 3)
            && groups.iter().all(|g| g.chars().all(|c| c.is_ascii_digit()));
        if !valid {
            return None;
        }
        groups.concat()
    } else {
        if !int_part.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        int_part.to_owned()
    };
    let mut clean = String::new();
    clean.push_str(if int_digits.is_empty() {
        "0"
    } else {
        &int_digits
    });
    if !frac_part.is_empty() {
        clean.push('.');
        clean.push_str(frac_part);
    }
    if let Some(exp) = exponent {
        let digits = exp.strip_prefix(['+', '-']).unwrap_or(exp);
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        clean.push('e');
        clean.push_str(exp);
    }
    let mut n: f64 = clean.parse().ok()?;
    if !n.is_finite() {
        return None;
    }
    if percent {
        n /= 100.0;
    }
    Some(if neg { -n } else { n })
}

/// Formats a number the way Excel's General format does when converting to text:
/// 15 significant digits, no trailing zeros, scientific notation for very large or very
/// small magnitudes (`1E+15`, `1E-10`).
pub fn number_to_text(n: f64) -> String {
    if n == 0.0 {
        return "0".to_owned();
    }
    if !n.is_finite() {
        return ErrorKind::Num.as_str().to_owned();
    }
    let sci = format!("{:.14e}", n.abs());
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let all_digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let digits = all_digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let sign = if n < 0.0 { "-" } else { "" };
    if !(-9..15).contains(&exp) {
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

/// Compares two numbers the way Excel's comparison operators do: values that agree to about
/// 15 significant digits are equal.
pub fn compare_numbers(a: f64, b: f64) -> Ordering {
    if a == b {
        return Ordering::Equal;
    }
    let scale = a.abs().max(b.abs());
    if (a - b).abs() <= scale * 4.0 * f64::EPSILON {
        return Ordering::Equal;
    }
    a.partial_cmp(&b).unwrap_or(Ordering::Equal)
}

/// Case-insensitive text comparison.
pub fn compare_text(a: &str, b: &str) -> Ordering {
    a.chars()
        .flat_map(char::to_lowercase)
        .cmp(b.chars().flat_map(char::to_lowercase))
}

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
        assert_eq!(number_to_text(1e15), "1E+15");
        assert_eq!(
            number_to_text(1_234_567_890_123_456.0),
            "1.23456789012346E+15"
        );
        assert_eq!(number_to_text(1.5e20), "1.5E+20");
        assert_eq!(number_to_text(0.0001), "0.0001");
        assert_eq!(number_to_text(1e-10), "1E-10");
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
