//! Display text for spreadsheet cells.
//!
//! Covers General, fixed decimals with thousands separators, percent, literal prefixes and
//! suffixes (currency symbols), and the common date/time codes. Anything fancier (scientific,
//! fractions, conditions, colours) falls back to General. This is a viewer approximation,
//! not the calculation engine's number-format implementation.

use open365_xlsx::{CellValue, format_number};

/// The text shown in a grid cell.
pub fn display_value(value: &CellValue, code: &str, date1904: bool) -> String {
    match value {
        CellValue::Empty => String::new(),
        CellValue::Number(n) => format_with_code(*n, code, date1904),
        CellValue::Bool(true) => "TRUE".to_owned(),
        CellValue::Bool(false) => "FALSE".to_owned(),
        CellValue::Error(e) => e.code().to_owned(),
        other => other.as_text().unwrap_or_default().to_owned(),
    }
}

fn general(n: f64) -> String {
    let mut out = String::new();
    format_number(n, &mut out);
    out
}

/// Strips quoted literals, escapes and `[...]` blocks, keeping their text for literals.
struct Section {
    /// Code with quotes, brackets and escapes removed (placeholders and date letters only).
    pattern: String,
    prefix: String,
    suffix: String,
}

#[derive(Default)]
struct Literals {
    prefix: String,
    suffix: String,
    seen_placeholder: bool,
}

impl Literals {
    fn push(&mut self, c: char) {
        if self.seen_placeholder {
            self.suffix.push(c);
        } else {
            self.prefix.push(c);
        }
    }
}

fn first_section(code: &str) -> Section {
    let mut pattern = String::new();
    let mut lit = Literals::default();
    let mut chars = code.chars();
    while let Some(c) = chars.next() {
        match c {
            ';' => break,
            '"' => {
                for q in chars.by_ref() {
                    if q == '"' {
                        break;
                    }
                    lit.push(q);
                }
            }
            '\\' => {
                if let Some(n) = chars.next() {
                    lit.push(n);
                }
            }
            '_' => {
                // `_x` reserves the width of x; show a space.
                let _ = chars.next();
                lit.push(' ');
            }
            '*' => {
                let _ = chars.next();
            }
            '[' => {
                let mut inner = String::new();
                for q in chars.by_ref() {
                    if q == ']' {
                        break;
                    }
                    inner.push(q);
                }
                // `[$$-409]` carries a currency symbol; `[h]`/`[mm]` are elapsed time.
                if let Some(rest) = inner.strip_prefix('$') {
                    for ch in rest.split('-').next().unwrap_or_default().chars() {
                        lit.push(ch);
                    }
                }
            }
            '0' | '#' | '?' | 'y' | 'Y' | 'm' | 'M' | 'd' | 'D' | 'h' | 'H' | 's' | 'S' => {
                lit.seen_placeholder = true;
                pattern.push(c);
            }
            '.' | ',' | '%' | 'E' | 'e' | '/' | ':' | 'A' | 'a' | 'P' | 'p' => pattern.push(c),
            ' ' | '-' | '+' => {
                if lit.seen_placeholder {
                    pattern.push(c);
                } else {
                    lit.push(c);
                }
            }
            other => lit.push(other),
        }
    }
    Section {
        pattern,
        prefix: lit.prefix,
        suffix: lit.suffix,
    }
}

pub fn format_with_code(n: f64, code: &str, date1904: bool) -> String {
    let code = code.trim();
    if code.is_empty() || code.eq_ignore_ascii_case("general") {
        return general(n);
    }
    let section = first_section(code);
    let p = section.pattern.as_str();
    let is_date = p
        .chars()
        .any(|c| matches!(c, 'y' | 'Y' | 'd' | 'D' | 'h' | 'H' | 's' | 'S' | 'm' | 'M'));
    if is_date {
        return format_date(n, p, date1904).unwrap_or_else(|| general(n));
    }
    if p.contains(['E', 'e']) || p.contains('/') || !p.contains(['0', '#', '?']) {
        return general(n);
    }
    let percent = p.contains('%');
    let value = if percent { n * 100.0 } else { n };
    let numeric: String = p.chars().take_while(|c| *c != '%').collect();
    let decimals = numeric.split_once('.').map_or(0, |(_, frac)| {
        frac.chars()
            .filter(|c| matches!(c, '0' | '#' | '?'))
            .count()
    });
    let thousands = numeric
        .split('.')
        .next()
        .is_some_and(|int| int.contains(','));
    let mut text = format!("{:.*}", decimals, value.abs());
    if thousands {
        text = group_thousands(&text);
    }
    let negative = value < 0.0 && text.chars().any(|c| matches!(c, '1'..='9'));
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    out.push_str(&section.prefix);
    out.push_str(&text);
    if percent {
        out.push('%');
    }
    out.push_str(&section.suffix);
    out
}

fn group_thousands(text: &str) -> String {
    let (int, frac) = text
        .split_once('.')
        .map_or((text, None), |(i, f)| (i, Some(f)));
    let mut out = String::new();
    let len = int.len();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if let Some(f) = frac {
        out.push('.');
        out.push_str(f);
    }
    out
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// Days since 1970-01-01 to (year, month 1..=12, day).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn format_date(serial: f64, pattern: &str, date1904: bool) -> Option<String> {
    if !serial.is_finite() || !(0.0..=2_958_465.0).contains(&serial) {
        return None;
    }
    let day_number = serial.floor();
    let frac = serial - day_number;
    let mut secs = (frac * 86_400.0).round() as i64;
    let mut day_number = day_number as i64;
    if secs >= 86_400 {
        secs -= 86_400;
        day_number += 1;
    }
    // Days since 1970-01-01.
    let days = if date1904 {
        day_number - 24_107
    } else if day_number < 61 {
        // Serial 60 is Excel's non-existent 1900-02-29; treat the early range as 1900-01-xx.
        day_number - 25_568
    } else {
        day_number - 25_569
    };
    let (year, month, day) = civil_from_days(days);
    let weekday = (days + 4).rem_euclid(7) as usize;
    let (hour, minute, second) = (secs / 3600, (secs / 60) % 60, secs % 60);
    let twelve_hour = pattern.to_ascii_uppercase().contains("AM/PM")
        || pattern.to_ascii_uppercase().contains("A/P");

    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let lower = c.to_ascii_lowercase();
        let mut run = 1;
        while i + run < chars.len() && chars[i + run].to_ascii_lowercase() == lower {
            run += 1;
        }
        match lower {
            'y' => {
                if run <= 2 {
                    out.push_str(&format!("{:02}", year.rem_euclid(100)));
                } else {
                    out.push_str(&format!("{year:04}"));
                }
            }
            'm' => {
                // Minutes when next to hours or seconds.
                let before_h = chars[..i]
                    .iter()
                    .rev()
                    .find(|c| c.is_ascii_alphabetic())
                    .is_some_and(|c| matches!(c.to_ascii_lowercase(), 'h'));
                let after_s = chars[i + run..]
                    .iter()
                    .find(|c| c.is_ascii_alphabetic())
                    .is_some_and(|c| matches!(c.to_ascii_lowercase(), 's'));
                if before_h || after_s {
                    out.push_str(&if run >= 2 {
                        format!("{minute:02}")
                    } else {
                        minute.to_string()
                    });
                } else {
                    match run {
                        1 => out.push_str(&month.to_string()),
                        2 => out.push_str(&format!("{month:02}")),
                        3 => out.push_str(&MONTHS[(month - 1) as usize][..3]),
                        _ => out.push_str(MONTHS[(month - 1) as usize]),
                    }
                }
            }
            'd' => match run {
                1 => out.push_str(&day.to_string()),
                2 => out.push_str(&format!("{day:02}")),
                3 => out.push_str(&DAYS[weekday][..3]),
                _ => out.push_str(DAYS[weekday]),
            },
            'h' => {
                let h = if twelve_hour {
                    match hour % 12 {
                        0 => 12,
                        other => other,
                    }
                } else {
                    hour
                };
                out.push_str(&if run >= 2 {
                    format!("{h:02}")
                } else {
                    h.to_string()
                });
            }
            's' => out.push_str(&if run >= 2 {
                format!("{second:02}")
            } else {
                second.to_string()
            }),
            'a' | 'p' => {
                // AM/PM or A/P
                let rest: String = chars[i..].iter().collect::<String>().to_ascii_uppercase();
                if rest.starts_with("AM/PM") {
                    out.push_str(if hour < 12 { "AM" } else { "PM" });
                    i += 5;
                    continue;
                }
                if rest.starts_with("A/P") {
                    out.push_str(if hour < 12 { "A" } else { "P" });
                    i += 3;
                    continue;
                }
                out.push(c);
            }
            _ => {
                for _ in 0..run {
                    out.push(c);
                }
            }
        }
        i += run;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn general_and_fixed() {
        assert_eq!(format_with_code(3.0, "General", false), "3");
        assert_eq!(format_with_code(3.14259, "0.00", false), "3.14");
        assert_eq!(
            format_with_code(1234567.891, "#,##0.00", false),
            "1,234,567.89"
        );
        assert_eq!(format_with_code(0.256, "0%", false), "26%");
        assert_eq!(
            format_with_code(1234.5, "\"$\"#,##0.00", false),
            "$1,234.50"
        );
        assert_eq!(format_with_code(-2.5, "0.0", false), "-2.5");
    }

    #[test]
    fn dates() {
        // 2024-03-15 is serial 45366 in the 1900 system.
        assert_eq!(format_with_code(45366.0, "yyyy-mm-dd", false), "2024-03-15");
        assert_eq!(format_with_code(45366.0, "d/m/yy", false), "15/3/24");
        assert_eq!(
            format_with_code(45366.5, "yyyy-mm-dd hh:mm", false),
            "2024-03-15 12:00"
        );
        assert_eq!(
            format_with_code(45366.0, "mmm d, yyyy", false),
            "Mar 15, 2024"
        );
        assert_eq!(
            format_with_code(45366.0 - 1462.0, "yyyy-mm-dd", true),
            "2024-03-15"
        );
    }

    #[test]
    fn unsupported_falls_back_to_general() {
        assert_eq!(format_with_code(1500.0, "0.00E+00", false), "1500");
    }
}
