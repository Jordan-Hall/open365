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
    /// Index into `LANGS`.
    Table(usize),
}

pub(super) fn names_for(lcid: Option<u32>) -> Names {
    let Some(l) = lcid else { return Names::Arabic };
    match l & 0x3FF {
        0x01 | 0x3C | 0x45 | 0x5F | 0x73 | 0x7B | 0x7D | 0x7F => Names::Arabic,
        0x0C => Names::French,
        0x1F => Names::Turkish,
        0x21 => Names::Indonesian,
        0x29 => Names::Persian,
        p => match LANGS.iter().position(|l| l.0 == p) {
            Some(i) => Names::Table(i),
            None => Names::English,
        },
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
        Names::Table(i) => LANGS[i].1[m].to_string(),
    })
}

/// Measured Hijri month names (`mmmm`), weekday names (`dddd`) and weekday abbreviations (`ddd`, Sunday first) of the
/// languages that have their own tables: primary language id, months, weekdays, abbreviations.
type Lang = (
    u32,
    [&'static str; 12],
    [&'static str; 7],
    [&'static str; 7],
);
const LANGS: [Lang; 14] = [
    (
        0x1C,
        [
            "Muharrem",
            "Safer",
            "Rabiulevvel",
            "Rabiulahir",
            "Xhemadilevvel",
            "Xhemadilahir",
            "Rexhep",
            "Shaban",
            "Ramazan",
            "Sheval",
            "Dhulkade",
            "Dhulhixhxhe",
        ],
        [
            "e diel",
            "e h\u{eb}n\u{eb}",
            "e mart\u{eb}",
            "e m\u{eb}rkur\u{eb}",
            "e enjte",
            "e premte",
            "e shtun\u{eb}",
        ],
        ["Die", "H\u{eb}n", "Mar", "M\u{eb}r", "Enj", "Pre", "Sht"],
    ),
    (
        0x20,
        [
            "\u{645}\u{62d}\u{631}\u{651}\u{645}",
            "\u{635}\u{641}\u{631}",
            "\u{631}\u{628}\u{6cc}\u{639} \u{627}\u{644}\u{623}\u{648}\u{651}\u{644}",
            "\u{631}\u{628}\u{6cc}\u{639} \u{627}\u{644}\u{62b}\u{627}\u{646}\u{6cc}",
            "\u{62c}\u{645}\u{627}\u{62f}\u{649} \u{627}\u{644}\u{623}\u{648}\u{644}",
            "\u{62c}\u{645}\u{627}\u{62f}\u{649} \u{627}\u{644}\u{62b}\u{627}\u{646}\u{6cc}",
            "\u{631}\u{62c}\u{628}",
            "\u{634}\u{639}\u{628}\u{627}\u{646}",
            "\u{631}\u{645}\u{636}\u{627}\u{646}",
            "\u{634}\u{648}\u{651}\u{627}\u{644}",
            "\u{630}\u{648} \u{627}\u{644}\u{642}\u{639}\u{62f}",
            "\u{630}\u{648} \u{627}\u{644}\u{62d}\u{62c}\u{651}",
        ],
        [
            "\u{627}\u{62a}\u{648}\u{627}\u{631}",
            "\u{67e}\u{64a}\u{631}",
            "\u{645}\u{646}\u{6af}\u{644}",
            "\u{628}\u{62f}\u{6be}",
            "\u{62c}\u{645}\u{639}\u{631}\u{627}\u{62a}",
            "\u{62c}\u{645}\u{639}\u{647}",
            "\u{647}\u{641}\u{62a}\u{647}",
        ],
        [
            "\u{627}\u{62a}\u{648}\u{627}\u{631}",
            "\u{67e}\u{64a}\u{631}",
            "\u{645}\u{646}\u{6af}\u{644}",
            "\u{628}\u{62f}\u{6be}",
            "\u{62c}\u{645}\u{639}\u{631}\u{627}\u{62a}",
            "\u{62c}\u{645}\u{639}\u{647}",
            "\u{647}\u{641}\u{62a}\u{647}",
        ],
    ),
    (
        0x28,
        [
            "\u{41c}\u{443}\u{4b3}\u{430}\u{440}\u{440}\u{430}\u{43c}",
            "\u{421}\u{430}\u{444}\u{430}\u{440}",
            "\u{420}\u{430}\u{431}\u{435}\u{44a}\u{443}\u{43b}\u{430}\u{432}\u{432}\u{430}\u{43b}",
            "\u{420}\u{430}\u{431}\u{435}\u{44a}\u{443}\u{441}\u{441}\u{43e}\u{43d}\u{4e3}",
            "\u{4b6}\u{438}\u{43c}\u{43e}\u{434}\u{438}\u{44e}\u{43b}\u{430}\u{432}\u{432}\u{430}\u{43b}",
            "\u{4b6}\u{438}\u{43c}\u{43e}\u{434}\u{438}\u{44e}\u{441}\u{441}\u{43e}\u{43d}\u{4e3}",
            "\u{420}\u{430}\u{4b7}\u{430}\u{431}",
            "\u{428}\u{430}\u{44a}\u{431}\u{43e}\u{43d}",
            "\u{420}\u{430}\u{43c}\u{430}\u{437}\u{43e}\u{43d}",
            "\u{428}\u{430}\u{432}\u{432}\u{43e}\u{43b}",
            "\u{417}\u{443}\u{43b}\u{49b}\u{430}\u{44a}\u{434}\u{430}",
            "\u{417}\u{443}\u{43b}\u{4b3}\u{438}\u{4b7}\u{4b7}\u{430}",
        ],
        [
            "\u{42f}\u{448}",
            "\u{414}\u{443}\u{448}\u{430}\u{43d}\u{431}\u{435}",
            "\u{421}\u{435}\u{448}\u{430}\u{43d}\u{431}\u{435}",
            "\u{427}\u{43e}\u{440}\u{448}\u{430}\u{43d}\u{431}\u{435}",
            "\u{41f}\u{430}\u{43d}\u{4b7}\u{448}\u{430}\u{43d}\u{431}\u{435}",
            "\u{4b6}\u{443}\u{43c}\u{44a}\u{430}",
            "\u{428}\u{430}\u{43d}\u{431}\u{435}",
        ],
        [
            "\u{42f}\u{448}",
            "\u{414}\u{448}",
            "\u{421}\u{448}",
            "\u{427}\u{448}",
            "\u{41f}\u{448}",
            "\u{4b6}\u{43c}",
            "\u{428}\u{43d}",
        ],
    ),
    (
        0x2C,
        [
            "M\u{259}h\u{259}rr\u{259}m",
            "S\u{259}f\u{259}r",
            "R\u{259}bi\u{fc}l\u{259}vv\u{259}l",
            "R\u{259}bi\u{fc}lax\u{131}r",
            "C\u{259}madiy\u{259}l\u{259}vv\u{259}l",
            "C\u{259}madiy\u{259}lax\u{131}r",
            "R\u{259}c\u{259}b",
            "\u{15e}aban",
            "Ramazan",
            "\u{15e}\u{259}vval",
            "Zilq\u{259}d\u{259}",
            "Zilhicc\u{259}",
        ],
        [
            "Bazar",
            "Bazar\u{a0}ert\u{259}si",
            "\u{c7}\u{259}r\u{15f}\u{259}nb\u{259}\u{a0}ax\u{15f}am\u{131}",
            "\u{c7}\u{259}r\u{15f}\u{259}nb\u{259}",
            "C\u{fc}m\u{259}\u{a0}ax\u{15f}am\u{131}",
            "C\u{fc}m\u{259}",
            "\u{15e}\u{259}nb\u{259}",
        ],
        ["B", "Be", "\u{c7}a", "\u{c7}", "Ca", "C", "\u{15e}"],
    ),
    (
        0x3E,
        [
            "Muharram",
            "Safar",
            "Rabiul Awal",
            "Rabiul Akhir",
            "Jamadil Awal",
            "Jamadil Akhir",
            "Rejab",
            "Syaaban",
            "Ramadhan",
            "Syawal",
            "Zulkaedah",
            "Zulhijjah",
        ],
        [
            "Ahad", "Isnin", "Selasa", "Rabu", "Khamis", "Jumaat", "Sabtu",
        ],
        ["Ahd", "Isn", "Sel", "Rab", "Kha", "Jum", "Sab"],
    ),
    (
        0x40,
        [
            "\u{4af}\u{447}\u{442}\u{4af}\u{43d} \u{430}\u{439}\u{44b}",
            "\u{431}\u{438}\u{440}\u{434}\u{438}\u{43d} \u{430}\u{439}\u{44b}",
            "\u{436}\u{430}\u{43b}\u{433}\u{430}\u{43d} \u{43a}\u{443}\u{440}\u{430}\u{43d}",
            "\u{447}\u{44b}\u{43d} \u{43a}\u{443}\u{440}\u{430}\u{43d}",
            "\u{431}\u{443}\u{433}\u{443}",
            "\u{43a}\u{443}\u{43b}\u{436}\u{430}",
            "\u{442}\u{435}\u{43a}\u{435}",
            "\u{431}\u{430}\u{448} \u{43e}\u{43e}\u{43d}\u{430}",
            "\u{430}\u{44f}\u{43a} \u{43e}\u{43e}\u{43d}\u{430}",
            "\u{442}\u{43e}\u{433}\u{443}\u{437}\u{434}\u{443}\u{43d} \u{430}\u{439}\u{44b}",
            "\u{436}\u{435}\u{442}\u{438}\u{43d}\u{438}\u{43d} \u{430}\u{439}\u{44b}",
            "\u{431}\u{435}\u{448}\u{442}\u{438}\u{43d} \u{430}\u{439}\u{44b}",
        ],
        [
            "\u{416}\u{435}\u{43a}\u{448}\u{435}\u{43c}\u{431}\u{438}",
            "\u{414}\u{4af}\u{439}\u{448}\u{4e9}\u{43c}\u{431}\u{4af}",
            "\u{428}\u{435}\u{439}\u{448}\u{435}\u{43c}\u{431}\u{438}",
            "\u{428}\u{430}\u{440}\u{448}\u{435}\u{43c}\u{431}\u{438}",
            "\u{411}\u{435}\u{439}\u{448}\u{435}\u{43c}\u{431}\u{438}",
            "\u{416}\u{443}\u{43c}\u{430}",
            "\u{418}\u{448}\u{435}\u{43c}\u{431}\u{438}",
        ],
        [
            "\u{416}\u{448}",
            "\u{414}\u{448}",
            "\u{428}\u{448}",
            "\u{428}\u{440}",
            "\u{411}\u{448}",
            "\u{416}\u{43c}",
            "\u{418}\u{448}",
        ],
    ),
    (
        0x42,
        [
            "A\u{15f}yr",
            "Sapar",
            "Rebi\u{fc}l-ewwel",
            "Rebi\u{fc}l-ahyr",
            "Jemazi\u{fd}el-ewwel",
            "Jemazi\u{fd}el-ahyr",
            "Rejep",
            "Meret",
            "Remezan",
            "Ba\u{fd}ram",
            "Zilkade",
            "Gurban",
        ],
        [
            "Du\u{15f}enbe",
            "Si\u{15f}enbe",
            "\u{c7}ar\u{15f}enbe",
            "Pen\u{15f}enbe",
            "Anna",
            "\u{15e}enbe",
            "\u{dd}ek\u{15f}enbe",
        ],
        ["Db", "Sb", "\u{c7}b", "Pb", "An", "\u{15e}b", "\u{dd}b"],
    ),
    (
        0x43,
        [
            "Muharram",
            "Safar",
            "Robi\u{2019} ul-avval",
            "Robi\u{2019} us-soniy",
            "Jumad ul-avval",
            "Jumad us-soniy",
            "Rajab",
            "Sha\u{2019}bon",
            "Ramazon",
            "Shavvol",
            "Zulqa\u{2019}da",
            "Zulhijja",
        ],
        [
            "yakshanba",
            "dushanba",
            "seshanba",
            "chorshanba",
            "payshanba",
            "juma",
            "shanba",
        ],
        ["Du", "Se", "Ch", "Pa", "Ju", "Sh", "Ya"],
    ),
    (
        0x44,
        [
            "\u{41c}\u{4e9}\u{445}\u{4d9}\u{440}\u{440}\u{4d9}\u{43c}",
            "\u{421}\u{4d9}\u{444}\u{4d9}\u{440}",
            "\u{420}\u{430}\u{431}\u{438}\u{433}\u{44b}\u{43b}\u{44c}-\u{4d9}\u{4af}\u{432}\u{4d9}\u{43b}",
            "\u{420}\u{430}\u{431}\u{438}\u{433}\u{44b}\u{43b}\u{44c}-\u{430}\u{445}\u{438}\u{440}",
            "\u{496}\u{4e9}\u{43c}\u{4d9}\u{434}\u{438}\u{4d9}\u{43b}-\u{4d9}\u{4af}\u{432}\u{4d9}\u{43b}",
            "\u{496}\u{4e9}\u{43c}\u{4d9}\u{434}\u{438}\u{4d9}\u{43b}-\u{430}\u{445}\u{438}\u{440}",
            "\u{420}\u{4d9}\u{497}\u{4d9}\u{43f}",
            "\u{428}\u{4d9}\u{433}\u{44a}\u{431}\u{430}\u{43d}",
            "\u{420}\u{430}\u{43c}\u{430}\u{437}\u{430}\u{43d}",
            "\u{428}\u{4d9}\u{4af}\u{432}\u{4d9}\u{43b}",
            "\u{417}\u{4e9}\u{43b}\u{43a}\u{430}\u{433}\u{44b}\u{439}\u{434}\u{4d9}",
            "\u{417}\u{4e9}\u{43b}\u{445}\u{438}\u{497}\u{497}\u{4d9}",
        ],
        [
            "\u{42f}\u{43a}\u{448}\u{4d9}\u{43c}\u{431}\u{435}",
            "\u{414}\u{4af}\u{448}\u{4d9}\u{43c}\u{431}\u{435}",
            "\u{421}\u{438}\u{448}\u{4d9}\u{43c}\u{431}\u{435}",
            "\u{427}\u{4d9}\u{440}\u{448}\u{4d9}\u{43c}\u{431}\u{435}",
            "\u{41f}\u{4d9}\u{43d}\u{497}\u{435}\u{448}\u{4d9}\u{43c}\u{431}\u{435}",
            "\u{496}\u{43e}\u{43c}\u{433}\u{430}",
            "\u{428}\u{438}\u{43c}\u{431}\u{4d9}",
        ],
        [
            "\u{42f}\u{43a}\u{448}.",
            "\u{414}\u{4af}\u{448}.",
            "\u{421}\u{438}\u{448}.",
            "\u{427}\u{4d9}\u{440}\u{448}.",
            "\u{41f}\u{4d9}\u{43d}\u{497}.",
            "\u{496}\u{43e}\u{43c}.",
            "\u{428}\u{438}\u{43c}.",
        ],
    ),
    (
        0x63,
        [
            "\u{645}\u{62d}\u{631}\u{645}",
            "\u{635}\u{641}\u{631}",
            "\u{631}\u{628}\u{64a}\u{639} \u{627}\u{644}\u{623}\u{648}\u{651}\u{644}",
            "\u{631}\u{628}\u{64a}\u{639} \u{627}\u{644}\u{62b}\u{627}\u{646}\u{64a}",
            "\u{62c}\u{645}\u{627}\u{62f}\u{649} \u{627}\u{644}\u{623}\u{648}\u{644}",
            "\u{62c}\u{645}\u{627}\u{62f}\u{649} \u{627}\u{644}\u{62b}\u{627}\u{646}\u{649}",
            "\u{631}\u{62c}\u{628}",
            "\u{634}\u{639}\u{628}\u{627}\u{646}",
            "\u{631}\u{645}\u{636}\u{627}\u{646}",
            "\u{634}\u{648}\u{651}\u{627}\u{644}",
            "\u{630}\u{648} \u{627}\u{644}\u{642}\u{639}\u{62f}\u{629}",
            "\u{630}\u{648} \u{627}\u{644}\u{62d}\u{62c}\u{651}\u{629}",
        ],
        [
            "\u{6cc}\u{6a9}\u{634}\u{646}\u{628}\u{647}",
            "\u{62f}\u{648}\u{634}\u{646}\u{628}\u{647}",
            "\u{633}\u{647}\u{200c}\u{634}\u{646}\u{628}\u{647}",
            "\u{686}\u{627}\u{631}\u{634}\u{646}\u{628}\u{647}",
            "\u{67e}\u{646}\u{62c}\u{634}\u{646}\u{628}\u{647}",
            "\u{62c}\u{645}\u{639}\u{647}",
            "\u{634}\u{646}\u{628}\u{647}",
        ],
        [
            "\u{6cc}\u{6a9}\u{634}\u{646}\u{628}\u{647}",
            "\u{62f}\u{648}\u{634}\u{646}\u{628}\u{647}",
            "\u{633}\u{647}\u{200c}\u{634}\u{646}\u{628}\u{647}",
            "\u{686}\u{627}\u{631}\u{634}\u{646}\u{628}\u{647}",
            "\u{67e}\u{646}\u{62c}\u{634}\u{646}\u{628}\u{647}",
            "\u{62c}\u{645}\u{639}\u{647}",
            "\u{634}\u{646}\u{628}\u{647}",
        ],
    ),
    (
        0x65,
        [
            "\u{789}\u{7aa}\u{799}\u{7a6}\u{787}\u{7b0}\u{783}\u{7a6}\u{789}\u{7b0}",
            "\u{79e}\u{7a6}\u{78a}\u{7a6}\u{783}\u{7aa}",
            "\u{783}\u{7a6}\u{784}\u{7a9}\u{7a2}\u{7aa}\u{78d}\u{7b0}\u{787}\u{7a6}\u{787}\u{7b0}\u{788}\u{7a6}\u{78d}\u{7b0}",
            "\u{783}\u{7a6}\u{784}\u{7a9}\u{7a2}\u{7aa}\u{78d}\u{7b0}\u{787}\u{7a7}\u{79a}\u{7a8}\u{783}\u{7aa}",
            "\u{796}\u{7aa}\u{789}\u{7a7}\u{78b}\u{7a6}\u{78d}\u{7b0}\u{787}\u{7ab}\u{78d}\u{7a7}",
            "\u{796}\u{7aa}\u{789}\u{7a7}\u{78b}\u{7a6}\u{78d}\u{7b0}\u{787}\u{7a7}\u{79a}\u{7a8}\u{783}\u{7a7}",
            "\u{783}\u{7a6}\u{796}\u{7a6}\u{784}\u{7b0}",
            "\u{79d}\u{7a6}\u{7a2}\u{7b0}\u{784}\u{7a7}\u{782}\u{7b0}",
            "\u{783}\u{7a6}\u{789}\u{7a6}\u{79f}\u{7a7}\u{782}\u{7b0}",
            "\u{79d}\u{7a6}\u{787}\u{7b0}\u{788}\u{7a7}\u{78d}\u{7b0}",
            "\u{79b}\u{7aa}\u{78d}\u{7b0}\u{7a4}\u{7a6}\u{7a2}\u{7a8}\u{78b}\u{7a7}",
            "\u{79b}\u{7aa}\u{78d}\u{7b0}\u{799}\u{7a8}\u{787}\u{7b0}\u{796}\u{7a7}",
        ],
        [
            "\u{787}\u{7a7}\u{78b}\u{7a9}\u{787}\u{7b0}\u{78c}\u{7a6}",
            "\u{780}\u{7af}\u{789}\u{7a6}",
            "\u{787}\u{7a6}\u{782}\u{7b0}\u{78e}\u{7a7}\u{783}\u{7a6}",
            "\u{784}\u{7aa}\u{78b}\u{7a6}",
            "\u{784}\u{7aa}\u{783}\u{7a7}\u{790}\u{7b0}\u{78a}\u{7a6}\u{78c}\u{7a8}",
            "\u{780}\u{7aa}\u{786}\u{7aa}\u{783}\u{7aa}",
            "\u{780}\u{7ae}\u{782}\u{7a8}\u{780}\u{7a8}\u{783}\u{7aa}",
        ],
        [
            "\u{787}\u{7a7}\u{78b}\u{7a9}\u{787}\u{7b0}\u{78c}\u{7a6}",
            "\u{780}\u{7af}\u{789}\u{7a6}",
            "\u{787}\u{7a6}\u{782}\u{7b0}\u{78e}\u{7a7}\u{783}\u{7a6}",
            "\u{784}\u{7aa}\u{78b}\u{7a6}",
            "\u{784}\u{7aa}\u{783}\u{7a7}\u{790}\u{7b0}\u{78a}\u{7a6}\u{78c}\u{7a8}",
            "\u{780}\u{7aa}\u{786}\u{7aa}\u{783}\u{7aa}",
            "\u{780}\u{7ae}\u{782}\u{7a8}\u{780}\u{7a8}\u{783}\u{7aa}",
        ],
    ),
    (
        0x67,
        [
            "haaraana",
            "dewo haaraana",
            "gammu",
            "rakki gammu",
            "rakkaati gammu",
            "afuru",
            "yummum yaawa",
            "yaawa",
            "koorka",
            "humturaandu",
            "safturaandu",
            "taaski",
        ],
        [
            "Alati", "Tenen", "Talaata", "Alarba", "Alkamisa", "Juma", "Asewe",
        ],
        ["Ala", "Ten", "Tal", "Ala", "Alk", "Jum", "Ase"],
    ),
    (
        0x6A,
        [
            "O\u{1e63}u Muharram",
            "O\u{1e63}u Safar",
            "O\u{1e63}u R Awwal",
            "O\u{1e63}u R Aakhir",
            "O\u{1e63}u J Awwal",
            "O\u{1e63}u J Aakhira",
            "O\u{1e63}u Rajab",
            "O\u{1e63}u Sha'baan",
            "O\u{1e63}u Ramadhan",
            "O\u{1e63}u Shawwal",
            "O\u{1e63}u Dhul Qa'dah",
            "O\u{1e63}u Dhul Hijjah",
        ],
        [
            "\u{1ecc}j\u{1ecd}\u{301} \u{c0}\u{ec}k\u{fa}",
            "\u{1ecc}j\u{1ecd}\u{301} Aj\u{e9}",
            "\u{1ecc}j\u{1ecd}\u{301} \u{cc}s\u{1eb9}\u{301}gun",
            "\u{1ecc}j\u{1ecd}\u{301}r\u{fa}",
            "\u{1ecc}j\u{1ecd}\u{301}b\u{1ecd}\u{300}",
            "\u{1ecc}j\u{1ecd}\u{301} \u{1eb8}t\u{ec}",
            "\u{1ecc}j\u{1ecd}\u{301} \u{c0}b\u{e1}m\u{1eb9}\u{301}ta",
        ],
        [
            "\u{c0}\u{ec}k",
            "Aj",
            "\u{cc}\u{1e63}g",
            "\u{1ecc}jr",
            "\u{1ecc}jb",
            "\u{1eb8}t",
            "\u{c0}b\u{e1}",
        ],
    ),
    (
        0x3F,
        [
            "\u{49a}\u{430}\u{4a3}\u{442}\u{430}\u{440}",
            "\u{410}\u{49b}\u{43f}\u{430}\u{43d}",
            "\u{41d}\u{430}\u{443}\u{440}\u{44b}\u{437}",
            "\u{421}\u{4d9}\u{443}\u{456}\u{440}",
            "\u{41c}\u{430}\u{43c}\u{44b}\u{440}",
            "\u{41c}\u{430}\u{443}\u{441}\u{44b}\u{43c}",
            "\u{428}\u{456}\u{43b}\u{434}\u{435}",
            "\u{422}\u{430}\u{43c}\u{44b}\u{437}",
            "\u{49a}\u{44b}\u{440}\u{43a}\u{4af}\u{439}\u{435}\u{43a}",
            "\u{49a}\u{430}\u{437}\u{430}\u{43d}",
            "\u{49a}\u{430}\u{440}\u{430}\u{448}\u{430}",
            "\u{416}\u{435}\u{43b}\u{442}\u{43e}\u{49b}\u{441}\u{430}\u{43d}",
        ],
        [
            "\u{416}\u{435}\u{43a}\u{441}\u{435}\u{43d}\u{431}\u{456}",
            "\u{414}\u{4af}\u{439}\u{441}\u{435}\u{43d}\u{431}\u{456}",
            "\u{421}\u{435}\u{439}\u{441}\u{435}\u{43d}\u{431}\u{456}",
            "\u{421}\u{4d9}\u{440}\u{441}\u{435}\u{43d}\u{431}\u{456}",
            "\u{411}\u{435}\u{439}\u{441}\u{435}\u{43d}\u{431}\u{456}",
            "\u{416}\u{4b1}\u{43c}\u{430}",
            "\u{421}\u{435}\u{43d}\u{431}\u{456}",
        ],
        [
            "\u{416}\u{435}\u{43a}",
            "\u{414}\u{4af}\u{439}",
            "\u{421}\u{435}\u{439}",
            "\u{421}\u{4d9}\u{440}",
            "\u{411}\u{435}\u{439}",
            "\u{416}\u{4b1}\u{43c}",
            "\u{421}\u{435}\u{43d}",
        ],
    ),
];

/// Weekday names for the Arabic, English and table languages (Sunday first); the other languages use their own.
pub(super) fn weekday(names: Names, wd: usize, abbr: bool) -> Option<String> {
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
        Names::Arabic => Some(AR[wd].to_string()),
        Names::English => Some(EN[wd].to_string()),
        Names::Table(i) => Some(if abbr { LANGS[i].3[wd] } else { LANGS[i].2[wd] }.to_string()),
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
