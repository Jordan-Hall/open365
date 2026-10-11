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
    // The Thai calendar (byte 7) and the untagged Thai letters use Thai names; the US Gregorian calendar (byte 2) English ones.
    let names_lcid = match (sec.calendar, sec.thai) {
        (7, _) | (_, true) => Some(0x041E),
        (2, _) => Some(0x0409),
        _ => sec.lcid,
    };
    let by_lcid = names_lcid.and_then(FormatLocale::for_lcid);
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
            Tok::Year(_)
                | Tok::Month(_)
                | Tok::Day(_)
                | Tok::BuddhistYear(_)
                | Tok::Era(_)
                | Tok::EraYear(_)
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
            Tok::Lit(s) => out.push_lit(s),
            Tok::Pad(c) => out.pad(*c),
            Tok::Fill(c) => out.fill(*c),
            Tok::Year(n) => {
                let y = if sec.calendar == 7 {
                    ymd.0 + 543
                } else {
                    ymd.0
                };
                if *n <= 2 {
                    out.push(&format!("{:02}", y.rem_euclid(100)));
                } else {
                    out.push(&format!("{y:04}"));
                }
            }
            Tok::Month(n) => {
                let m = (ymd.1.clamp(1, 12) - 1) as usize;
                match n {
                    1 => out.push_unpadded(ymd.1),
                    2 => out.push_padded(ymd.1),
                    3 => out.push_lit(&loc.month_abbr[m]),
                    5 => out.push_lit(&loc.month_full[m].chars().take(1).collect::<String>()),
                    _ => out.push_lit(&loc.month_full[m]),
                }
            }
            Tok::Day(n) => match n {
                1 => out.push_unpadded(ymd.2),
                2 => out.push_padded(ymd.2),
                3 => out.push_lit(&loc.day_abbr[weekday]),
                _ => out.push_lit(&loc.day_full[weekday]),
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
                    out.push_padded(h);
                } else {
                    out.push_unpadded(h);
                }
            }
            Tok::Minute(n) => {
                if *n >= 2 {
                    out.push_padded(minute as i64);
                } else {
                    out.push_unpadded(minute as i64);
                }
            }
            Tok::Second(n) => {
                if *n >= 2 {
                    out.push_padded(second as i64);
                } else {
                    out.push_unpadded(second as i64);
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
                    out.push_lit(full);
                } else {
                    let s: String = full.chars().take(1).collect();
                    if *upper {
                        out.push_lit(&s);
                    } else {
                        out.push_lit(&s.to_lowercase());
                    }
                }
            }
            Tok::BuddhistYear(n) => {
                // With a `[$-LCID]` prefix `b` is the Gregorian year (measured for every language tried).
                let y = if sec.lcid.is_some() && sec.calendar != 7 {
                    ymd.0
                } else {
                    ymd.0 + 543
                };
                if *n <= 2 {
                    out.push(&format!("{:02}", y.rem_euclid(100)));
                } else {
                    out.push(&format!("{y:04}"));
                }
            }
            Tok::Era(n) if sec.lcid.is_some_and(|l| l & 0xFFFF == 0x411) => {
                let (kanji, initial, _) = japanese_era(ymd);
                match n {
                    1 => out.push_lit(initial),
                    2 => out.push_lit(&kanji.chars().take(1).collect::<String>()),
                    _ => out.push_lit(kanji),
                }
            }
            Tok::EraYear(n) => {
                if sec.lcid.is_some_and(|l| l & 0xFFFF == 0x411) {
                    let (_, _, year) = japanese_era(ymd);
                    if *n >= 2 {
                        out.push(&format!("{year:02}"));
                    } else {
                        out.push(&year.to_string());
                    }
                } else {
                    out.push(&format!("{:04}", ymd.0));
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

/// The Japanese era of a date: the name, its Latin initial and the year within the era (the first year is 1).
/// Dates before the Meiji era (which starts on 8 September 1868) cannot occur: the serial numbers start in 1900.
fn japanese_era(ymd: (i64, i64, i64)) -> (&'static str, &'static str, i64) {
    const ERAS: [((i64, i64, i64), &str, &str); 5] = [
        ((2019, 5, 1), "\u{4ee4}\u{548c}", "R"),
        ((1989, 1, 8), "\u{5e73}\u{6210}", "H"),
        ((1926, 12, 25), "\u{662d}\u{548c}", "S"),
        ((1912, 7, 30), "\u{5927}\u{6b63}", "T"),
        ((1868, 9, 8), "\u{660e}\u{6cbb}", "M"),
    ];
    for (start, kanji, initial) in ERAS {
        if ymd >= start {
            return (kanji, initial, ymd.0 - start.0 + 1);
        }
    }
    ("\u{660e}\u{6cbb}", "M", ymd.0 - 1867)
}
