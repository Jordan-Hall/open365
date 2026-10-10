//! Date and time sections.

use super::Out;
use super::parse::{Section, Tok};
use crate::fmtlocale::FormatLocale;
use crate::functions::serial_to_ymd;

const MAX_SERIAL_1900: f64 = 2_958_465.0;
const MAX_SERIAL_1904: f64 = 2_957_003.0;

/// Renders a date/time section. Returns false when the value cannot be shown (Excel prints `####`).
pub(super) fn render_date(
    sec: &Section,
    v: f64,
    date1904: bool,
    default_locale: &FormatLocale,
    out: &mut Out,
) -> bool {
    // A `[$-LCID]` prefix selects the names of that language.
    let by_lcid = sec.lcid.and_then(FormatLocale::for_lcid);
    let loc: &FormatLocale = by_lcid.as_deref().unwrap_or(default_locale);
    if !v.is_finite() || v < 0.0 {
        return false;
    }
    let sub = sec
        .toks
        .iter()
        .find_map(|t| match t {
            Tok::SubSec(n) => Some(*n),
            _ => None,
        })
        .unwrap_or(0);
    let calendar = sec.toks.iter().any(|t| {
        matches!(
            t,
            Tok::Year(_) | Tok::Month(_) | Tok::Day(_) | Tok::BuddhistYear(_)
        )
    });
    // Everything except the elapsed minute and second counters stops at 9999-12-31.
    let limited = sec.toks.iter().any(|t| {
        matches!(
            t,
            Tok::Year(_)
                | Tok::Month(_)
                | Tok::Day(_)
                | Tok::BuddhistYear(_)
                | Tok::Hour(_)
                | Tok::Minute(_)
                | Tok::Second(_)
                | Tok::AmPm { .. }
                | Tok::Elapsed('h', _)
        )
    });
    let per_sec = 10f64.powi(sub as i32);
    let ticks_per_day = 86_400.0 * per_sec;
    let mut days = v.floor();
    let mut t = ((v - days) * ticks_per_day + 0.5).floor();
    if t >= ticks_per_day {
        days += 1.0;
        t -= ticks_per_day;
    }
    let max = if date1904 {
        MAX_SERIAL_1904
    } else {
        MAX_SERIAL_1900
    };
    if limited && days > max {
        return false;
    }
    let ymd = if calendar {
        match serial_to_ymd(days, date1904) {
            Some(x) => x,
            None => return false,
        }
    } else {
        (0, 1, 1)
    };
    let weekday = if date1904 {
        (days as i64 + 5).rem_euclid(7)
    } else {
        (days as i64 - 1).rem_euclid(7)
    } as usize;
    let secs_total = (t / per_sec).floor();
    let sub_ticks = t - secs_total * per_sec;
    let hour24 = (secs_total / 3600.0).floor();
    let minute = ((secs_total - hour24 * 3600.0) / 60.0).floor();
    let second = secs_total - hour24 * 3600.0 - minute * 60.0;
    let elapsed_secs = days * 86_400.0 + secs_total;
    for tok in &sec.toks {
        match tok {
            Tok::Lit(s) => out.push(s),
            Tok::Pad(c) => out.pad(*c),
            Tok::Fill(c) => out.fill(*c),
            Tok::Year(n) => {
                if *n <= 2 {
                    out.push(&format!("{:02}", ymd.0.rem_euclid(100)));
                } else {
                    out.push(&format!("{:04}", ymd.0));
                }
            }
            Tok::Month(n) => {
                let m = (ymd.1.clamp(1, 12) - 1) as usize;
                match n {
                    1 => out.push(&ymd.1.to_string()),
                    2 => out.push(&format!("{:02}", ymd.1)),
                    3 => out.push(&loc.month_abbr[m]),
                    5 => out.push(&loc.month_full[m].chars().take(1).collect::<String>()),
                    _ => out.push(&loc.month_full[m]),
                }
            }
            Tok::Day(n) => match n {
                1 => out.push(&ymd.2.to_string()),
                2 => out.push(&format!("{:02}", ymd.2)),
                3 => out.push(&loc.day_abbr[weekday]),
                _ => out.push(&loc.day_full[weekday]),
            },
            Tok::Hour(n) => {
                let mut h = hour24 as i64;
                if sec.has_ampm {
                    h %= 12;
                    if h == 0 {
                        h = 12;
                    }
                }
                if *n >= 2 {
                    out.push(&format!("{h:02}"));
                } else {
                    out.push(&h.to_string());
                }
            }
            Tok::Minute(n) => {
                if *n >= 2 {
                    out.push(&format!("{:02}", minute as i64));
                } else {
                    out.push(&(minute as i64).to_string());
                }
            }
            Tok::Second(n) => {
                if *n >= 2 {
                    out.push(&format!("{:02}", second as i64));
                } else {
                    out.push(&(second as i64).to_string());
                }
            }
            Tok::SubSec(n) => {
                out.push(".");
                out.push(&format!("{:0width$}", sub_ticks as i64, width = *n));
            }
            Tok::AmPm { short, upper } => {
                let pm = hour24 >= 12.0;
                // `AM/PM` is always upper case; `a/p` keeps the case written in the format code.
                let full = if pm { &loc.pm } else { &loc.am };
                if !*short {
                    out.push(full);
                } else {
                    let s: String = full.chars().take(1).collect();
                    if *upper {
                        out.push(&s);
                    } else {
                        out.push(&s.to_lowercase());
                    }
                }
            }
            Tok::BuddhistYear(n) => {
                let y = ymd.0 + 543;
                if *n <= 2 {
                    out.push(&format!("{:02}", y.rem_euclid(100)));
                } else {
                    out.push(&format!("{y:04}"));
                }
            }
            Tok::Elapsed(kind, n) => {
                let total = match kind {
                    'h' => (elapsed_secs / 3600.0).floor(),
                    'm' => (elapsed_secs / 60.0).floor(),
                    _ => elapsed_secs,
                };
                out.push(&format!("{:0width$}", total as i64, width = *n));
            }
            _ => {}
        }
    }
    true
}
