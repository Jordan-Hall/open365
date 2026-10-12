//! The Hijri calendar of a `B2` prefix or a calendar byte 6 tag, as Excel renders it.
//!
//! Measured on Excel (1,990 serials over 1900 to 9666): the tabular civil calendar with the Kuwaiti leap-year pattern
//! (leap years 2, 5, 7, 10, 13, 15, 18, 21, 24, 26, 29 of a 30-year cycle; odd months 30 days, even months 29 days,
//! month 12 has 30 days in a leap year). 1 Muharram 1 is day 466,583 after 1899-12-31 counting real days (the
//! non-existent 1900-02-29 is not counted). Serial 60 gives `#VALUE!`, serial 0 is shown as serial 1.

const LEAP: [i64; 11] = [2, 5, 7, 10, 13, 15, 18, 21, 24, 26, 29];
/// Days between "day 1 of Hijri year 1" in the internal count and real day 0 (1899-12-31).
const EPOCH: i64 = 466_582;

fn is_leap(year: i64) -> bool {
    LEAP.contains(&year.rem_euclid(30))
}

/// Days before 1 Muharram of `year` (the first day of year 1 is day 0).
fn year_start(year: i64) -> i64 {
    let n = year - 1;
    let cycles = n.div_euclid(30);
    let rest = n.rem_euclid(30);
    n * 354 + cycles * 11 + LEAP.iter().filter(|&&t| t <= rest).count() as i64
}

fn month_len(year: i64, month: i64) -> i64 {
    if month % 2 == 1 || (month == 12 && is_leap(year)) {
        30
    } else {
        29
    }
}

/// (year, month, day) of a serial number of the 1900 system; `None` for serial 60 (which Excel refuses).
pub(super) fn from_serial(
    serial: i64,
    date1904: bool,
    um_al_qura: bool,
) -> Option<(i64, i64, i64)> {
    let s = if date1904 {
        serial + 1462
    } else {
        serial.max(1)
    };
    if s == 60 {
        return None;
    }
    if um_al_qura && let Some(x) = umalqura(s) {
        return Some(x);
    }
    let real = if s >= 61 { s - 1 } else { s };
    let days = real + EPOCH - 1; // 0 is 1 Muharram of year 1
    let mut year = days * 30 / 10631 + 1;
    while year_start(year) > days {
        year -= 1;
    }
    while year_start(year + 1) <= days {
        year += 1;
    }
    let mut rest = days - year_start(year);
    let mut month = 1;
    while rest >= month_len(year, month) {
        rest -= month_len(year, month);
        month += 1;
    }
    Some((year, month, rest + 1))
}

/// Umm al-Qura (calendar byte 23): measured month starts from 1318-01-01 (serial 121) to the end of 1450 (serial
/// 47251); one word per year, bit `m - 1` set when month `m` has 30 days. Outside that range Excel shows the
/// tabular calendar above (measured at 2,226 serials up to year 9666).
const UQ_FIRST_SERIAL: i64 = 121;
const UQ_END_SERIAL: i64 = 47_252;
const UQ_FIRST_YEAR: i64 = 1318;
const UQ_YEARS: [u16; 133] = [
    0x2EA, 0x6E9, 0xED2, 0xEA4, 0xD4A, 0xA96, 0x536, 0xAB5, 0xDAA, 0xBA4, 0xB49, 0xA93, 0x52B,
    0xA57, 0x4B6, 0xAB5, 0x5AA, 0xD55, 0xD2A, 0xA56, 0x4AE, 0x95D, 0x2EC, 0x6D5, 0x6AA, 0x555,
    0x4AB, 0x95B, 0x2BA, 0x575, 0xBB2, 0x764, 0x749, 0x655, 0x2AB, 0x55B, 0xADA, 0x6D4, 0xEC9,
    0xD92, 0xD25, 0xA4D, 0x2AD, 0x56D, 0xB6A, 0xB52, 0xAA5, 0xA4B, 0x497, 0x937, 0x2B6, 0x575,
    0xD6A, 0xD52, 0xA96, 0x92D, 0x25D, 0x4DD, 0xADA, 0x5D4, 0xDA9, 0xD52, 0xAAA, 0x4D6, 0x9B6,
    0x374, 0x769, 0x752, 0x6A5, 0x54B, 0xAAB, 0x55A, 0xAD5, 0xDD2, 0xDA4, 0xD49, 0xA95, 0x52D,
    0xA5D, 0x55A, 0xAD5, 0x6AA, 0x695, 0x52B, 0xA57, 0x4AE, 0x976, 0x56C, 0xB55, 0xAAA, 0xA55,
    0x4AD, 0x95D, 0x2DA, 0x5D9, 0xDB2, 0xBA4, 0xB4A, 0xA55, 0x2B5, 0x575, 0xB6A, 0xBD2, 0xBC4,
    0xB89, 0xA95, 0x52D, 0x5AD, 0xB6A, 0x6D4, 0xDC9, 0xD92, 0xAA6, 0x956, 0x2AE, 0x56D, 0x36A,
    0xB55, 0xAAA, 0x94D, 0x49D, 0x95D, 0x2BA, 0x5B5, 0x5AA, 0xD55, 0xA9A, 0x92E, 0x26E, 0x55D,
    0xADA, 0x6D4, 0x6A5,
];

fn umalqura(serial: i64) -> Option<(i64, i64, i64)> {
    if !(UQ_FIRST_SERIAL..UQ_END_SERIAL).contains(&serial) {
        return None;
    }
    let mut rest = serial - UQ_FIRST_SERIAL;
    for (i, word) in UQ_YEARS.iter().enumerate() {
        for month in 1..=12i64 {
            let len = if word >> (month - 1) & 1 == 1 { 30 } else { 29 };
            if rest < len {
                return Some((UQ_FIRST_YEAR + i as i64, month, rest + 1));
            }
            rest -= len;
        }
    }
    None
}

/// Which name tables a `[$-LCID]` language uses.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Names {
    Arabic,
    English,
    French,
    Turkish,
    Indonesian,
    Persian,
    /// The language has its own weekday names but Excel prints its Gregorian month names (or the table is unknown).
    Locale,
}

pub(super) fn names_for(lcid: Option<u32>) -> Names {
    let Some(l) = lcid else { return Names::Arabic };
    match l & 0x3FF {
        0x01 | 0x3C | 0x45 | 0x5F | 0x73 | 0x7B | 0x7D | 0x7F => Names::Arabic,
        0x0C => Names::French,
        0x1F => Names::Turkish,
        0x21 => Names::Indonesian,
        0x29 => Names::Persian,
        0x3F | 0x1C | 0x20 | 0x28 | 0x2C | 0x3E | 0x40 | 0x42 | 0x43 | 0x44 | 0x63 | 0x65
        | 0x67 | 0x6A => Names::Locale,
        _ => Names::English,
    }
}

/// Arabic month names (`mmm` is always this table; `mmmm` for Arabic languages), with a no-break space.
pub(super) fn arabic_month(m: usize, plain_space: bool) -> String {
    const T: [&str; 12] = [
        "\u{645}\u{62d}\u{631}\u{645}",
        "\u{635}\u{641}\u{631}",
        "\u{631}\u{628}\u{64a}\u{639}#\u{627}\u{644}\u{623}\u{648}\u{644}",
        "\u{631}\u{628}\u{64a}\u{639}#\u{627}\u{644}\u{62b}\u{627}\u{646}\u{64a}",
        "\u{62c}\u{645}\u{627}\u{62f}\u{649}#\u{627}\u{644}\u{623}\u{648}\u{644}\u{649}",
        "\u{62c}\u{645}\u{627}\u{62f}\u{649}#\u{627}\u{644}\u{62b}\u{627}\u{646}\u{64a}\u{629}",
        "\u{631}\u{62c}\u{628}",
        "\u{634}\u{639}\u{628}\u{627}\u{646}",
        "\u{631}\u{645}\u{636}\u{627}\u{646}",
        "\u{634}\u{648}\u{627}\u{644}",
        "\u{630}\u{648}#\u{627}\u{644}\u{642}\u{639}\u{62f}\u{629}",
        "\u{630}\u{648}#\u{627}\u{644}\u{62d}\u{62c}\u{629}",
    ];
    T[m].replace('#', if plain_space { " " } else { "\u{a0}" })
}

const ENGLISH: [&str; 12] = [
    "Muharram",
    "Safar",
    "Rabi' Awwal",
    "Rabi' Thani",
    "Jamada El Oula",
    "Jamada El Thaniah",
    "Rajab",
    "Sha'ban",
    "Ramadan",
    "Shawwal",
    "Thoul Ki'dah",
    "Thoul Hijjah",
];
const FRENCH: [&str; 12] = [
    "Mouharam",
    "Safar",
    "Rabi' Aoual",
    "Rabi' Thani",
    "Jamada El Oula",
    "Jamada El Thaniah",
    "Rajab",
    "Chaaban",
    "Ramadan",
    "Chaoual",
    "Zoul Ki'dah",
    "Zoul Hijjah",
];
const TURKISH: [&str; 12] = [
    "Muharrem",
    "Safer",
    "Rebiyülevvel",
    "Rebiyülahir",
    "Cemaziyelevvel",
    "Cemaziyelahir",
    "Recep",
    "Şaban",
    "Ramazan",
    "Şevval",
    "Zilkade",
    "Zilhicce",
];
const INDONESIAN: [&str; 12] = [
    "Muharam",
    "Safar",
    "Rabiulawal",
    "Rabiulakhir",
    "Jumadilawal",
    "Jumadilakhir",
    "Rajab",
    "Syakban",
    "Ramadan",
    "Syawal",
    "Zulkaidah",
    "Zulhijah",
];
const PERSIAN: [&str; 12] = [
    "محرم",
    "صفر",
    "ربیع الاول",
    "ربیع الثانی",
    "جمادی الاول",
    "جمادی الثانی",
    "رجب",
    "شعبان",
    "رمضان",
    "شوال",
    "ذی القعده",
    "ذی الحجه",
];

/// The `mmmm` name, or `None` when the language prints its Gregorian month name.
pub(super) fn month_full(names: Names, lcid: Option<u32>, m: usize) -> Option<String> {
    Some(match names {
        Names::Arabic => arabic_month(m, lcid.is_some_and(|l| l & 0xFFFF == 0x0C01)),
        Names::English => ENGLISH[m].to_string(),
        Names::French => FRENCH[m].to_string(),
        Names::Turkish => TURKISH[m].to_string(),
        Names::Indonesian => INDONESIAN[m].to_string(),
        Names::Persian => PERSIAN[m].to_string(),
        Names::Locale => return None,
    })
}

/// Weekday names for the Arabic and English tables (Sunday first); the other languages use their own.
pub(super) fn weekday(names: Names, wd: usize) -> Option<&'static str> {
    const AR: [&str; 7] = [
        "\u{627}\u{644}\u{623}\u{62d}\u{62f}",
        "\u{627}\u{644}\u{625}\u{62b}\u{646}\u{64a}\u{646}",
        "\u{627}\u{644}\u{62b}\u{644}\u{627}\u{62b}\u{627}\u{621}",
        "\u{627}\u{644}\u{623}\u{631}\u{628}\u{639}\u{627}\u{621}",
        "\u{627}\u{644}\u{62e}\u{645}\u{64a}\u{633}",
        "\u{627}\u{644}\u{62c}\u{645}\u{639}\u{629}",
        "\u{627}\u{644}\u{633}\u{628}\u{62a}",
    ];
    const EN: [&str; 7] = [
        "AlAhad",
        "AlEthnien",
        "AthThulatha",
        "AlArbia'a",
        "AlKhamis",
        "AlJumaa",
        "AsSabt",
    ];
    match names {
        Names::Arabic => Some(AR[wd]),
        Names::English => Some(EN[wd]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::from_serial;

    #[test]
    fn matches_measured_excel_dates() {
        assert_eq!(from_serial(1, false, false), Some((1317, 8, 29)));
        assert_eq!(from_serial(0, false, false), Some((1317, 8, 29)));
        assert_eq!(from_serial(60, false, false), None);
        assert_eq!(from_serial(61, false, false), Some((1317, 10, 29)));
        assert_eq!(from_serial(45366, false, false), Some((1445, 9, 6)));
        assert_eq!(from_serial(2_951_244, false, false), Some((9645, 11, 18)));
        assert_eq!(from_serial(2_958_465, false, false), Some((9666, 4, 3)));
        // Umm al-Qura: 2024-03-15 is 5 Ramadan 1445, 1318-01-01 is serial 121, 1450-12 has 30 days (serial 47251 is its last day)
        assert_eq!(from_serial(45366, false, true), Some((1445, 9, 5)));
        assert_eq!(from_serial(121, false, true), Some((1318, 1, 1)));
        assert_eq!(from_serial(47_251, false, true), Some((1450, 12, 29)));
        assert_eq!(
            from_serial(47_252, false, true),
            from_serial(47_252, false, false)
        );
    }
}
