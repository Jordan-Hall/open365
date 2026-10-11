//! Civil dates and times for fields, and Word's `\@` date-time pictures.

use crate::fields_locale::LocaleNames;

/// A proleptic Gregorian date and time of day without a time zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CivilDateTime {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl CivilDateTime {
    /// A date and time; `None` when a component is out of range.
    #[must_use]
    pub fn new(year: i32, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> Option<Self> {
        let valid = (1..=12).contains(&month)
            && day >= 1
            && day <= days_in_month(year, month)
            && hour < 24
            && minute < 60
            && second < 60;
        valid.then_some(Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
        })
    }

    /// Parses `yyyy-mm-ddThh:mm:ss` with an optional fraction and a `Z` or `+hh:mm` suffix.
    /// The result is the UTC instant: an offset is subtracted.
    #[must_use]
    pub fn parse_utc(text: &str) -> Option<Self> {
        let text = text.trim();
        let (date, rest) = text.split_once(['T', ' ']).unwrap_or((text, ""));
        let mut parts = date.splitn(3, '-');
        let year: i32 = parts.next()?.parse().ok()?;
        let month: u8 = parts.next()?.parse().ok()?;
        let day: u8 = parts.next()?.parse().ok()?;
        let (clock, offset) = split_offset(rest);
        let mut hms = clock.splitn(3, ':');
        let hour: u8 = hms
            .next()
            .filter(|s| !s.is_empty())
            .map_or(Some(0), |s| s.parse().ok())?;
        let minute: u8 = hms.next().map_or(Some(0), |s| s.parse().ok())?;
        let second: u8 = hms
            .next()
            .map(|s| s.split('.').next().unwrap_or(s))
            .map_or(Some(0), |s| s.parse().ok())?;
        let base = Self::new(year, month, day, hour, minute, second)?;
        Some(base.add_minutes(-offset))
    }

    /// This time shifted by `minutes` (may be negative).
    #[must_use]
    pub fn add_minutes(self, minutes: i32) -> Self {
        let total = self.days() * 1440
            + i64::from(self.hour) * 60
            + i64::from(self.minute)
            + i64::from(minutes);
        let days = total.div_euclid(1440);
        let rem = total.rem_euclid(1440);
        let (year, month, day) = civil_from_days(days);
        Self {
            year,
            month,
            day,
            hour: u8::try_from(rem / 60).unwrap_or(0),
            minute: u8::try_from(rem % 60).unwrap_or(0),
            second: self.second,
        }
    }

    /// Days since 1970-01-01.
    #[must_use]
    pub fn days(self) -> i64 {
        days_from_civil(self.year, self.month, self.day)
    }

    /// 0 for Sunday .. 6 for Saturday.
    #[must_use]
    pub fn weekday(self) -> usize {
        usize::try_from((self.days() + 4).rem_euclid(7)).unwrap_or(0)
    }
}

fn split_offset(rest: &str) -> (&str, i32) {
    if let Some(stripped) = rest.strip_suffix('Z') {
        return (stripped, 0);
    }
    if let Some(at) = rest.rfind(['+', '-'])
        && at > 0
    {
        let (clock, off) = rest.split_at(at);
        let sign = if off.starts_with('-') { -1 } else { 1 };
        let digits: String = off[1..].chars().filter(char::is_ascii_digit).collect();
        if digits.len() == 4 {
            let h: i32 = digits[..2].parse().unwrap_or(0);
            let m: i32 = digits[2..].parse().unwrap_or(0);
            return (clock, sign * (h * 60 + m));
        }
    }
    (rest, 0)
}

fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(year) => 29,
        _ => 28,
    }
}

/// Days from 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(year: i32, month: u8, day: u8) -> i64 {
    let y = i64::from(year) - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i32, u8, u8) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (
        i32::try_from(y).unwrap_or(0),
        u8::try_from(m).unwrap_or(1),
        u8::try_from(d).unwrap_or(1),
    )
}

/// Formats `dt` with a Word `\@` picture and the given names.
#[must_use]
pub fn format_date_picture(dt: CivilDateTime, picture: &str, names: &LocaleNames) -> String {
    let chars: Vec<char> = picture.chars().collect();
    let mut out = String::new();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        let run = |i: usize, c: char| chars[i..].iter().take_while(|x| **x == c).count();
        match c {
            '\'' => {
                // Quoted literal; `''` is an apostrophe.
                i += 1;
                if chars.get(i) == Some(&'\'') {
                    out.push('\'');
                    i += 1;
                    continue;
                }
                while i < chars.len() && chars[i] != '\'' {
                    out.push(chars[i]);
                    i += 1;
                }
                i += 1;
            }
            // Word ignores a backslash; the next character keeps its meaning.
            '\\' => i += 1,
            'M' => {
                let n = run(i, 'M');
                match n {
                    1 => out.push_str(&dt.month.to_string()),
                    2 => out.push_str(&format!("{:02}", dt.month)),
                    3 => out.push_str(&names.month_abbr[usize::from(dt.month) - 1]),
                    _ => {
                        out.push_str(&names.month_full[usize::from(dt.month) - 1]);
                        // Word appends the month number for each extra M.
                        if n > 4 {
                            out.push_str(&dt.month.to_string());
                        }
                    }
                }
                i += n;
            }
            'd' => {
                let n = run(i, 'd');
                match n {
                    1 => out.push_str(&dt.day.to_string()),
                    2 => out.push_str(&format!("{:02}", dt.day)),
                    3 => out.push_str(&names.day_abbr[dt.weekday()]),
                    _ => {
                        out.push_str(&names.day_full[dt.weekday()]);
                        if n > 4 {
                            out.push_str(&format!("{:02}", dt.day));
                        }
                    }
                }
                i += n;
            }
            'y' => {
                let n = run(i, 'y');
                let yy = dt.year.rem_euclid(100);
                match n {
                    1 | 2 => out.push_str(&format!("{yy:02}")),
                    3 => out.push_str("Error! Unknown character in picture string."),
                    _ => out.push_str(&format!("{:04}", dt.year)),
                }
                i += n;
            }
            'D' => {
                out.push_str(&dt.day.to_string());
                i += 1;
            }
            'h' | 'H' => {
                let n = run(i, c);
                let hour = if c == 'h' {
                    match dt.hour % 12 {
                        0 => 12,
                        h => h,
                    }
                } else {
                    dt.hour
                };
                if n == 1 {
                    out.push_str(&hour.to_string());
                } else {
                    out.push_str(&format!("{hour:02}"));
                }
                i += n;
            }
            'm' => {
                let n = run(i, 'm');
                if n == 1 {
                    out.push_str(&dt.minute.to_string());
                } else {
                    out.push_str(&format!("{:02}", dt.minute));
                }
                i += n;
            }
            's' => {
                let n = run(i, 's');
                if n == 1 {
                    out.push_str(&dt.second.to_string());
                } else {
                    out.push_str(&format!("{:02}", dt.second));
                }
                i += n;
            }
            _ => {
                let rest: String = chars[i..].iter().take(5).collect();
                let lower = rest.to_ascii_lowercase();
                let pm = dt.hour >= 12;
                if lower.starts_with("am/pm") {
                    out.push_str(if pm { &names.pm } else { &names.am });
                    i += 5;
                } else if lower.starts_with("a/p") {
                    out.push_str(if pm { &names.pm } else { &names.am });
                    i += 3;
                } else {
                    out.push(c);
                    i += 1;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weekday_and_roundtrip() {
        let dt = CivilDateTime::new(2024, 3, 3, 14, 5, 9).unwrap();
        assert_eq!(dt.weekday(), 0);
        assert_eq!(dt.add_minutes(24 * 60).day, 4);
        let utc = CivilDateTime::parse_utc("2024-03-03T14:05:09Z").unwrap();
        assert_eq!(utc, dt);
        assert_eq!(dt.add_minutes(-60).hour, 13);
    }
}
