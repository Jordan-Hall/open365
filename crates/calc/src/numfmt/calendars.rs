//! Hebrew (calendar byte 8), Saka (16) and Persian (22) calendars of a `[$-xx0409]` tag, as Excel renders them.
//!
//! Measured on Excel (about 2,400 serials per calendar from 1900 to 9999): all three follow the real calendar. Hebrew
//! matches the standard molad arithmetic for every year below 6000 (the end of Windows' table; later years drift by
//! a few days in Excel and are computed with the same arithmetic here). Persian uses the 33-year leap cycle
//! (leap years 1, 5, 9, 13, 17, 22, 26, 30 of 33), which Excel follows up to year 1535; later years differ by up to
//! three days at year 9378. Saka is the Indian national calendar (Chaitra 1 on 22 March, 21 March in a leap year).

use crate::functions::serial_to_ymd;

pub(super) const HEBREW_LATIN: [&str; 12] = [
    "Tishrei", "Cheshvan", "Kislev", "Tevet", "Shvat", "Adar", "Nisan", "Iyar", "Sivan", "Tamuz",
    "Av", "Elul",
];

pub(super) const HEBREW_LATIN_LEAP: [&str; 13] = [
    "Tishrei", "Cheshvan", "Kislev", "Tevet", "Shvat", "AdarI", "AdarII", "Nisan", "Iyar", "Sivan",
    "Tamuz", "Av", "Elul",
];

pub(super) const HEBREW_NATIVE: [&str; 12] = [
    "\u{5ea}\u{5e9}\u{5e8}\u{5d9}",
    "\u{5d7}\u{5e9}\u{5d5}\u{5df}",
    "\u{5db}\u{5e1}\u{5dc}\u{5d5}",
    "\u{5d8}\u{5d1}\u{5ea}",
    "\u{5e9}\u{5d1}\u{5d8}",
    "\u{5d0}\u{5d3}\u{5e8}",
    "\u{5e0}\u{5d9}\u{5e1}\u{5df}",
    "\u{5d0}\u{5d9}\u{5d9}\u{5e8}",
    "\u{5e1}\u{5d9}\u{5d5}\u{5df}",
    "\u{5ea}\u{5de}\u{5d5}\u{5d6}",
    "\u{5d0}\u{5d1}",
    "\u{5d0}\u{5dc}\u{5d5}\u{5dc}",
];

pub(super) const HEBREW_NATIVE_LEAP: [&str; 13] = [
    "\u{5ea}\u{5e9}\u{5e8}\u{5d9}",
    "\u{5d7}\u{5e9}\u{5d5}\u{5df}",
    "\u{5db}\u{5e1}\u{5dc}\u{5d5}",
    "\u{5d8}\u{5d1}\u{5ea}",
    "\u{5e9}\u{5d1}\u{5d8}",
    "\u{5d0}\u{5d3}\u{5e8} \u{5d0}",
    "\u{5d0}\u{5d3}\u{5e8} \u{5d1}",
    "\u{5e0}\u{5d9}\u{5e1}\u{5df}",
    "\u{5d0}\u{5d9}\u{5d9}\u{5e8}",
    "\u{5e1}\u{5d9}\u{5d5}\u{5df}",
    "\u{5ea}\u{5de}\u{5d5}\u{5d6}",
    "\u{5d0}\u{5d1}",
    "\u{5d0}\u{5dc}\u{5d5}\u{5dc}",
];

pub(super) const HEBREW_DAYS_LATIN: [&str; 7] = [
    "Yom Rishon",
    "Yom Sheni",
    "Yom Shlishi",
    "Yom Revi'i",
    "Yom Chamishi",
    "Yom Shishi",
    "Shabat",
];

pub(super) const HEBREW_ABBR_LATIN: [&str; 7] = [
    "Yom Rishon",
    "Yom Sheni",
    "Yom Shlishi",
    "Yom Revi'i",
    "Yom Chamishi",
    "Yom Shishi",
    "Shabat",
];

pub(super) const HEBREW_DAYS_NATIVE: [&str; 7] = [
    "\u{5d9}\u{5d5}\u{5dd}\u{a0}\u{5e8}\u{5d0}\u{5e9}\u{5d5}\u{5df}",
    "\u{5d9}\u{5d5}\u{5dd}\u{a0}\u{5e9}\u{5e0}\u{5d9}",
    "\u{5d9}\u{5d5}\u{5dd}\u{a0}\u{5e9}\u{5dc}\u{5d9}\u{5e9}\u{5d9}",
    "\u{5d9}\u{5d5}\u{5dd}\u{a0}\u{5e8}\u{5d1}\u{5d9}\u{5e2}\u{5d9}",
    "\u{5d9}\u{5d5}\u{5dd}\u{a0}\u{5d7}\u{5de}\u{5d9}\u{5e9}\u{5d9}",
    "\u{5d9}\u{5d5}\u{5dd}\u{a0}\u{5e9}\u{5d9}\u{5e9}\u{5d9}",
    "\u{5e9}\u{5d1}\u{5ea}",
];

pub(super) const HEBREW_ABBR_NATIVE: [&str; 7] = [
    "\u{5d0}", "\u{5d1}", "\u{5d2}", "\u{5d3}", "\u{5d4}", "\u{5d5}", "\u{5e9}",
];

pub(super) const PERSIAN_LATIN: [&str; 12] = [
    "Farvardin",
    "Ordibehesht",
    "Khord\u{101}d",
    "Tir",
    "Mord\u{101}d",
    "Shahrivar",
    "Mehr",
    "\u{1fb9}ban",
    "\u{1fb9}zar",
    "Day",
    "Bahm\u{101}n",
    "Esfand",
];

pub(super) const PERSIAN_FRENCH: [&str; 12] = [
    "F\u{e0}rv\u{e0}rdine",
    "ordib\u{e9}h\u{e9}cht",
    "khordad",
    "tir",
    "mord\u{e0}d",
    "sh\u{e0}h\u{155}iv\u{e0}r",
    "m\u{e9}hr",
    "aban",
    "az\u{e0}r",
    "d\u{e9}y",
    "b\u{e0}hm\u{e0}n",
    "\u{e9}sf\u{e0}nd",
];

pub(super) const PERSIAN_NATIVE: [&str; 12] = [
    "\u{641}\u{631}\u{648}\u{631}\u{62f}\u{6cc}\u{646}",
    "\u{627}\u{631}\u{62f}\u{6cc}\u{628}\u{647}\u{634}\u{62a}",
    "\u{62e}\u{631}\u{62f}\u{627}\u{62f}",
    "\u{62a}\u{6cc}\u{631}",
    "\u{645}\u{631}\u{62f}\u{627}\u{62f}",
    "\u{634}\u{647}\u{631}\u{6cc}\u{648}\u{631}",
    "\u{645}\u{647}\u{631}",
    "\u{622}\u{628}\u{627}\u{646}",
    "\u{622}\u{630}\u{631}",
    "\u{62f}\u{6cc}",
    "\u{628}\u{647}\u{645}\u{646}",
    "\u{627}\u{633}\u{641}\u{646}\u{62f}",
];

pub(super) const PERSIAN_DAYS_LATIN: [&str; 7] = [
    "Yekshanbeh",
    "Doshanbeh",
    "Seshanbeh",
    "Chah\u{101}rshanbeh",
    "Panjshanbeh",
    "Jom'eh",
    "shanbeh",
];

pub(super) const PERSIAN_DAYS_FRENCH: [&str; 7] = [
    "Y\u{e9}kchanb\u{e9}",
    "Doch\u{e0}nb\u{e9}",
    "S\u{e9}ch\u{e0}nbe",
    "Chaharchanb\u{e9}",
    "P\u{e0}njch\u{e0}nb\u{e9}",
    "Jom\u{e9}",
    "Ch\u{e0}nbe",
];

pub(super) const PERSIAN_DAYS_NATIVE: [&str; 7] = [
    "\u{6cc}\u{6a9}\u{634}\u{646}\u{628}\u{647}",
    "\u{62f}\u{648}\u{634}\u{646}\u{628}\u{647}",
    "\u{633}\u{647} \u{634}\u{646}\u{628}\u{647}",
    "\u{686}\u{647}\u{627}\u{631}\u{634}\u{646}\u{628}\u{647}",
    "\u{67e}\u{646}\u{62c}\u{634}\u{646}\u{628}\u{647}",
    "\u{62c}\u{645}\u{639}\u{647}",
    "\u{634}\u{646}\u{628}\u{647}",
];

pub(super) const SAKA_LATIN: [&str; 12] = [
    "Chaitra",
    "Vaisakha",
    "Jyaistha",
    "Asadha",
    "Sravana",
    "Bhadra",
    "Asvina",
    "Karthika",
    "Agrahayana",
    "Pausa",
    "Magha",
    "Phalguna",
];

pub(super) const SAKA_NATIVE: [&str; 12] = [
    "\u{91a}\u{948}\u{924}\u{94d}\u{930}",
    "\u{935}\u{948}\u{936}\u{93e}\u{916}",
    "\u{91c}\u{94d}\u{92f}\u{947}\u{937}\u{94d}\u{91f}",
    "\u{906}\u{937}\u{93e}\u{95d}",
    "\u{936}\u{94d}\u{930}\u{93e}\u{935}\u{923}",
    "\u{92d}\u{93e}\u{926}\u{94d}\u{930}",
    "\u{906}\u{936}\u{94d}\u{935}\u{940}\u{928}",
    "\u{958}\u{93e}\u{930}\u{94d}\u{924}\u{940}\u{915}",
    "\u{905}\u{917}\u{94d}\u{930}\u{939}\u{93e}\u{92f}\u{928}",
    "\u{92a}\u{94c}\u{938}\u{93e}",
    "\u{92e}\u{93e}\u{918}",
    "\u{92b}\u{932}\u{94d}\u{917}\u{941}\u{923}",
];

pub(super) const SAKA_DAYS_LATIN: [&str; 7] = [
    "Ravivara",
    "Somavara",
    "Mangalvara",
    "Budhavara",
    "Guruvara",
    "Sukravara",
    "Sanivara",
];

pub(super) const SAKA_ABBR_LATIN: [&str; 7] =
    ["Ravi", "Som", "Mangal", "Budh", "Guru", "Sukra", "Sani"];

pub(super) const SAKA_DAYS_NATIVE: [&str; 7] = [
    "\u{930}\u{935}\u{93f}\u{935}\u{93e}\u{930}",
    "\u{938}\u{94b}\u{92e}\u{935}\u{93e}\u{930}",
    "\u{92e}\u{902}\u{917}\u{932}\u{935}\u{93e}\u{930}",
    "\u{92c}\u{941}\u{927}\u{935}\u{93e}\u{930}",
    "\u{917}\u{941}\u{930}\u{941}\u{935}\u{93e}\u{930}",
    "\u{936}\u{941}\u{915}\u{94d}\u{930}\u{935}\u{93e}\u{930}",
    "\u{936}\u{928}\u{93f}\u{935}\u{93e}\u{930}",
];
const CHINESE_MONTHS: [&str; 12] = [
    "\u{4e00}\u{6708}",
    "\u{4e8c}\u{6708}",
    "\u{4e09}\u{6708}",
    "\u{56db}\u{6708}",
    "\u{4e94}\u{6708}",
    "\u{516d}\u{6708}",
    "\u{4e03}\u{6708}",
    "\u{516b}\u{6708}",
    "\u{4e5d}\u{6708}",
    "\u{5341}\u{6708}",
    "\u{5341}\u{4e00}\u{6708}",
    "\u{5341}\u{4e8c}\u{6708}",
];

const CHINESE_DAYS: [&str; 7] = [
    "\u{661f}\u{671f}\u{65e5}",
    "\u{661f}\u{671f}\u{4e00}",
    "\u{661f}\u{671f}\u{4e8c}",
    "\u{661f}\u{671f}\u{4e09}",
    "\u{661f}\u{671f}\u{56db}",
    "\u{661f}\u{671f}\u{4e94}",
    "\u{661f}\u{671f}\u{516d}",
];

const CHINESE_ABBR: [&str; 7] = [
    "\u{9031}\u{65e5}",
    "\u{9031}\u{4e00}",
    "\u{9031}\u{4e8c}",
    "\u{9031}\u{4e09}",
    "\u{9031}\u{56db}",
    "\u{9031}\u{4e94}",
    "\u{9031}\u{516d}",
];

const SYRIAC_MONTHS: [&str; 12] = [
    "\u{643}\u{627}\u{646}\u{648}\u{646}\u{a0}\u{627}\u{644}\u{62b}\u{627}\u{646}\u{64a}",
    "\u{634}\u{628}\u{627}\u{637}",
    "\u{622}\u{630}\u{627}\u{631}",
    "\u{646}\u{64a}\u{633}\u{627}\u{646}",
    "\u{623}\u{64a}\u{627}\u{631}",
    "\u{62d}\u{632}\u{64a}\u{631}\u{627}\u{646}",
    "\u{62a}\u{645}\u{648}\u{632}",
    "\u{622}\u{628}",
    "\u{623}\u{64a}\u{644}\u{648}\u{644}",
    "\u{62a}\u{634}\u{631}\u{64a}\u{646}\u{a0}\u{627}\u{644}\u{623}\u{648}\u{644}",
    "\u{62a}\u{634}\u{631}\u{64a}\u{646}\u{a0}\u{627}\u{644}\u{62b}\u{627}\u{646}\u{64a}",
    "\u{643}\u{627}\u{646}\u{648}\u{646}\u{a0}\u{627}\u{644}\u{623}\u{648}\u{644}",
];

const ARABIC_DAYS: [&str; 7] = [
    "\u{627}\u{644}\u{623}\u{62d}\u{62f}",
    "\u{627}\u{644}\u{625}\u{62b}\u{646}\u{64a}\u{646}",
    "\u{627}\u{644}\u{62b}\u{644}\u{627}\u{62b}\u{627}\u{621}",
    "\u{627}\u{644}\u{623}\u{631}\u{628}\u{639}\u{627}\u{621}",
    "\u{627}\u{644}\u{62e}\u{645}\u{64a}\u{633}",
    "\u{627}\u{644}\u{62c}\u{645}\u{639}\u{629}",
    "\u{627}\u{644}\u{633}\u{628}\u{62a}",
];

const KOREAN_MONTHS: [&str; 12] = [
    "\u{c815}\u{c6d4}",
    "\u{c774}\u{c6d4}",
    "\u{c0bc}\u{c6d4}",
    "\u{c0ac}\u{c6d4}",
    "\u{c624}\u{c6d4}",
    "\u{c720}\u{c6d4}",
    "\u{ce60}\u{c6d4}",
    "\u{d314}\u{c6d4}",
    "\u{ad6c}\u{c6d4}",
    "\u{c2dc}\u{c6d4}",
    "\u{b3d9}\u{c9d3}\u{b2ec}",
    "\u{c123}\u{b2ec}",
];

const KOREAN_DAYS: [&str; 7] = [
    "\u{c77c}\u{c694}\u{c77c}",
    "\u{c6d4}\u{c694}\u{c77c}",
    "\u{d654}\u{c694}\u{c77c}",
    "\u{c218}\u{c694}\u{c77c}",
    "\u{baa9}\u{c694}\u{c77c}",
    "\u{ae08}\u{c694}\u{c77c}",
    "\u{d1a0}\u{c694}\u{c77c}",
];

const KOREAN_ABBR: [&str; 7] = [
    "\u{c77c}", "\u{c6d4}", "\u{d654}", "\u{c218}", "\u{baa9}", "\u{ae08}", "\u{d1a0}",
];

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn is_leap_gregorian(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Real days since 1899-12-31 (`None` for the fictitious 1900-02-29).
fn real_days(serial: i64, date1904: bool) -> Option<i64> {
    let (y, m, d) = serial_to_ymd(serial.max(if date1904 { 0 } else { 1 }) as f64, date1904)?;
    if (y, m, d) == (1900, 2, 29) {
        return None;
    }
    Some(days_from_civil(y, m, d) - days_from_civil(1899, 12, 31))
}

// ----------------------------------------------------------------------------------------------------- Hebrew

pub(super) fn hebrew_leap(y: i64) -> bool {
    (7 * y + 1).rem_euclid(19) < 7
}

fn hebrew_elapsed(y: i64) -> i64 {
    let months = (235 * y - 234).div_euclid(19);
    let parts = 12_084 + 13_753 * months;
    let mut day = months * 29 + parts / 25_920;
    if (3 * (day + 1)).rem_euclid(7) < 3 {
        day += 1;
    }
    day
}

fn hebrew_new_year(y: i64) -> i64 {
    let (a, b, c) = (
        hebrew_elapsed(y - 1),
        hebrew_elapsed(y),
        hebrew_elapsed(y + 1),
    );
    let delay = if c - b == 356 {
        2
    } else if b - a == 382 {
        1
    } else {
        0
    };
    b + delay
}

fn hebrew_month_lengths(y: i64) -> Vec<i64> {
    let len = hebrew_new_year(y + 1) - hebrew_new_year(y);
    let heshvan = if len % 10 == 5 { 30 } else { 29 };
    let kislev = if len % 10 == 3 { 29 } else { 30 };
    let mut v = vec![30, heshvan, kislev, 29, 30];
    if hebrew_leap(y) {
        v.extend([30, 29]);
    } else {
        v.push(29);
    }
    v.extend([30, 29, 30, 29, 30, 29]);
    v
}

/// (year, month, day) with Tishri as month 1; the leap-year Adar I is month 6 and Adar II month 7.
fn hebrew(serial: i64, date1904: bool) -> Option<(i64, i64, i64)> {
    let day = real_days(serial, date1904)? + 2_067_022;
    let mut y = (day as f64 / 365.2468) as i64 + 1;
    while hebrew_new_year(y) > day {
        y -= 1;
    }
    while hebrew_new_year(y + 1) <= day {
        y += 1;
    }
    let mut rest = day - hebrew_new_year(y);
    let mut m = 1;
    for len in hebrew_month_lengths(y) {
        if rest < len {
            break;
        }
        rest -= len;
        m += 1;
    }
    Some((y, m, rest + 1))
}

// ----------------------------------------------------------------------------------------------------- Persian

const PERSIAN_LEAP: [i64; 8] = [1, 5, 9, 13, 17, 22, 26, 30];

fn persian_leap(y: i64) -> bool {
    PERSIAN_LEAP.contains(&y.rem_euclid(33))
}

fn persian_year_start(y: i64) -> i64 {
    let n = y - 1;
    let (full, rest) = (n.div_euclid(33), n.rem_euclid(33));
    365 * n + full * 8 + PERSIAN_LEAP.iter().filter(|&&t| t <= rest).count() as i64
}

fn persian(serial: i64, date1904: bool) -> Option<(i64, i64, i64)> {
    let day = real_days(serial, date1904)? + 466_701; // 1 Farvardin 1 is day 1
    let mut y = day * 33 / 12_053 + 1;
    while persian_year_start(y) >= day {
        y -= 1;
    }
    while persian_year_start(y + 1) < day {
        y += 1;
    }
    let mut rest = day - persian_year_start(y) - 1;
    let mut m = 1;
    loop {
        let len = if m <= 6 {
            31
        } else if m <= 11 || persian_leap(y) {
            30
        } else {
            29
        };
        if rest < len || m == 12 {
            break;
        }
        rest -= len;
        m += 1;
    }
    Some((y, m, rest + 1))
}

// ----------------------------------------------------------------------------------------------------- Saka

fn saka(serial: i64, date1904: bool) -> Option<(i64, i64, i64)> {
    let (g, m, d) = serial_to_ymd(serial.max(if date1904 { 0 } else { 1 }) as f64, date1904)?;
    if (g, m, d) == (1900, 2, 29) {
        return None;
    }
    let day = days_from_civil(g, m, d);
    let start_of = |y: i64| days_from_civil(y, 3, if is_leap_gregorian(y) { 21 } else { 22 });
    let (year, base, leap) = if day >= start_of(g) {
        (g - 78, start_of(g), is_leap_gregorian(g))
    } else {
        (g - 79, start_of(g - 1), is_leap_gregorian(g - 1))
    };
    let mut rest = day - base;
    let mut month = 1;
    for len in [
        if leap { 31 } else { 30 },
        31,
        31,
        31,
        31,
        31,
        30,
        30,
        30,
        30,
        30,
        30,
    ] {
        if rest < len {
            break;
        }
        rest -= len;
        month += 1;
    }
    Some((year, month, rest + 1))
}

/// Converts a serial number into the (year, month, day) of calendar byte 8, 16 or 22.
pub(super) fn convert(calendar: u8, serial: i64, date1904: bool) -> Option<(i64, i64, i64)> {
    match calendar {
        8 => hebrew(serial, date1904),
        16 => saka(serial, date1904),
        22 => persian(serial, date1904),
        _ => None,
    }
}

/// Whether the tag's language gets the script names of the calendar (Arabic and Hebrew for Hebrew, Arabic and Hindi
/// for Saka) or the Latin transliteration.
fn native(calendar: u8, lcid: Option<u32>) -> bool {
    let primary = lcid.map(|l| l & 0x3FF);
    match calendar {
        8 => matches!(primary, Some(0x01 | 0x0D)),
        16 => matches!(primary, Some(0x01 | 0x39)),
        _ => !matches!(primary, Some(0x09 | 0x0C)),
    }
}

/// The `mmmm` name (also used for `mmm`); `None` when the calendar has no table.
pub(super) fn month_name(calendar: u8, lcid: Option<u32>, year: i64, month: i64) -> Option<String> {
    let i = (month - 1).max(0) as usize;
    let nat = native(calendar, lcid);
    let name = match calendar {
        8 => {
            let leap = hebrew_leap(year);
            let list: &[&str] = match (nat, leap) {
                (true, true) => &HEBREW_NATIVE_LEAP,
                (true, false) => &HEBREW_NATIVE,
                (false, true) => &HEBREW_LATIN_LEAP,
                (false, false) => &HEBREW_LATIN,
            };
            list.get(i)?
        }
        16 => (if nat { &SAKA_NATIVE } else { &SAKA_LATIN }).get(i)?,
        22 => {
            let french = lcid.is_some_and(|l| l & 0x3FF == 0x0C);
            (if french {
                &PERSIAN_FRENCH
            } else if nat {
                &PERSIAN_NATIVE
            } else {
                &PERSIAN_LATIN
            })
            .get(i)?
        }
        4 => CHINESE_MONTHS.get(i)?,
        10 => SYRIAC_MONTHS.get(i)?,
        20 => KOREAN_MONTHS.get(i)?,
        _ => return None,
    };
    Some((*name).to_string())
}

/// Weekday name (Sunday first).
pub(super) fn weekday_name(
    calendar: u8,
    lcid: Option<u32>,
    wd: usize,
    abbr: bool,
) -> Option<String> {
    let nat = native(calendar, lcid);
    let name = match (calendar, nat, abbr) {
        (8, true, false) => HEBREW_DAYS_NATIVE[wd],
        (8, true, true) => HEBREW_ABBR_NATIVE[wd],
        (8, false, false) => HEBREW_DAYS_LATIN[wd],
        (8, false, true) => HEBREW_ABBR_LATIN[wd],
        (16, true, _) => SAKA_DAYS_NATIVE[wd],
        (16, false, false) => SAKA_DAYS_LATIN[wd],
        (16, false, true) => SAKA_ABBR_LATIN[wd],
        (22, ..) => {
            if lcid.is_some_and(|l| l & 0x3FF == 0x0C) {
                PERSIAN_DAYS_FRENCH[wd]
            } else if nat {
                // Farsi spells Sunday with Arabic letters and puts a no-break space in Tuesday
                match (wd, lcid.is_some_and(|l| l & 0x3FF == 0x29)) {
                    (0, true) => "\u{64a}\u{643}\u{634}\u{646}\u{628}\u{647}",
                    (2, true) => "\u{633}\u{647}\u{a0}\u{634}\u{646}\u{628}\u{647}",
                    _ => PERSIAN_DAYS_NATIVE[wd],
                }
            } else {
                PERSIAN_DAYS_LATIN[wd]
            }
        }
        (4, _, false) => CHINESE_DAYS[wd],
        (4, _, true) => CHINESE_ABBR[wd],
        (10, ..) => ARABIC_DAYS[wd],
        (20, _, false) => KOREAN_DAYS[wd],
        (20, _, true) => KOREAN_ABBR[wd],
        _ => return None,
    };
    Some(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::convert;

    #[test]
    fn matches_measured_excel_dates() {
        // serial 45366 is 2024-03-15
        assert_eq!(convert(8, 45366, false), Some((5784, 7, 5)));
        assert_eq!(convert(8, 1, false), Some((5660, 5, 1)));
        assert_eq!(convert(22, 45366, false), Some((1402, 12, 25)));
        assert_eq!(convert(22, 1, false), Some((1278, 10, 11)));
        assert_eq!(convert(16, 45366, false), Some((1945, 12, 25)));
        assert_eq!(convert(16, 45500, false), Some((1946, 5, 5)));
        assert_eq!(convert(8, 60, false), None);
    }
}
