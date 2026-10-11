//! Locale-dependent built-in number formats (ECMA-376 Part 1, 18.8.30).
//!
//! A cell format that names a built-in `numFmtId` has no `numFmt` element: the format code is implied, and
//! Excel resolves it from the Windows locale of the machine that opens the file. This module resolves an id
//! for any Windows LCID, with the rules below.
//!
//! Evidence, kept apart:
//!
//! * **Measured** (Excel 16.0, `Range.Text`, `NumberFormat`, `NumberFormatLocal` of a workbook that references
//!   ids 0..100 with no `numFmts` part; `fixtures/reference/xlsx/numfmt-locale/measured-en-GB.json`, recorded
//!   by `tools/office/make-numfmt-locale-refs.ps1 -Measure`): under en-GB, 14 is the system short date, 15 and 16
//!   take a two-digit day, 20 to 22 take a two-digit hour, 5 to 8 and 37 to 44 follow the currency's negative
//!   pattern (minus sign, no parentheses), 27 to 31, 36, 50 to 58, 71 and 72 are the short date, 32 to 35 the
//!   time with seconds, and 59 to 81 repeat 1 to 22 and 45 to 47.
//! * **Public specification** (ECMA-376 Part 1, 18.8.30 as republished by Microsoft): the zh-TW, zh-CN, ja-JP,
//!   ko-KR and th-TH tables for ids 27 to 36, 50 to 58 and 59 to 81.
//! * **Derived** (labelled in `docs/testing/xlsx-numfmt-locale.md`): the same rules applied to the Windows locale
//!   data (NLS, see `builtin_numfmt_nls.rs`) of every other locale. Excel can only be measured in the machine's
//!   own locale, and this project never changes regional settings.
//!
//! Format codes are in the invariant (en-US) syntax that a file stores: `,` groups, `.` is the decimal point,
//! whatever the locale shows.

use crate::builtin_numfmt_nls::NLS;

/// One row of Windows locale data (generated).
pub(crate) struct Nls {
    pub lcid: u32,
    /// `DateTimeFormat.ShortDatePattern`.
    pub short_date: &'static str,
    /// `DateTimeFormat.ShortTimePattern`.
    pub short_time: &'static str,
    /// `DateTimeFormat.LongTimePattern`.
    pub time: &'static str,
    /// `NumberFormat.CurrencySymbol`.
    pub currency: &'static str,
    /// `NumberFormat.CurrencyPositivePattern`.
    pub currency_pos: u8,
    /// `NumberFormat.CurrencyNegativePattern`.
    pub currency_neg: u8,
}

const EN_US: u32 = 0x0409;

/// The LCIDs that have their own data row, in ascending order.
pub fn builtin_number_format_lcids() -> impl Iterator<Item = u32> {
    NLS.iter().map(|row| row.lcid)
}

/// The LCID whose data row [`builtin_number_format_in`] uses for `lcid`: the LCID itself, else the default
/// sub-locale of its language, else any locale of the language, else en-US.
pub fn resolve_number_format_lcid(lcid: u32) -> u32 {
    row_for(lcid).lcid
}

fn row_for(lcid: u32) -> &'static Nls {
    let lcid = lcid & 0xFFFF;
    let find = |wanted: u32| NLS.iter().find(|row| row.lcid == wanted);
    let primary = lcid & 0x3FF;
    find(lcid)
        .or_else(|| find(primary | 0x400))
        .or_else(|| NLS.iter().find(|row| row.lcid & 0x3FF == primary))
        .or_else(|| find(EN_US))
        .unwrap_or(&NLS[0])
}

/// Converts a Windows (.NET) date or time pattern to an Excel format code: `M` becomes `m`, `H` becomes `h`,
/// `tt` becomes `AM/PM`, quoted text uses double quotes, and years are two or four digits.
pub fn nls_pattern_to_excel(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let run = chars[i..].iter().take_while(|&&x| x == c).count();
        match c {
            '\'' | '"' => {
                // Quoted literal up to the matching quote.
                let mut j = i + 1;
                let mut text = String::new();
                while j < chars.len() && chars[j] != c {
                    text.push(chars[j]);
                    j += 1;
                }
                out.push('"');
                out.push_str(&text);
                out.push('"');
                i = j + 1;
                continue;
            }
            '\\' => {
                if let Some(next) = chars.get(i + 1) {
                    out.push('\\');
                    out.push(*next);
                }
                i += 2;
                continue;
            }
            'y' => out.push_str(if run <= 2 { "yy" } else { "yyyy" }),
            'M' => out.push_str(&"m".repeat(run.min(4))),
            'd' => out.push_str(&"d".repeat(run.min(4))),
            'H' | 'h' => out.push_str(&"h".repeat(run.min(2))),
            'm' => out.push_str(&"m".repeat(run.min(2))),
            's' => out.push_str(&"s".repeat(run.min(2))),
            't' => out.push_str(if run >= 2 { "AM/PM" } else { "A/P" }),
            'g' => out.push_str(&"g".repeat(run.min(3))),
            'f' | 'F' | 'K' | 'z' => {}
            _ => out.push_str(&c.to_string().repeat(run)),
        }
        i += run;
    }
    out
}

/// The East Asian or Thai table a locale selects.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Table {
    ZhTw,
    ZhCn,
    JaJp,
    KoKr,
    ThTh,
    None,
}

fn table_for(lcid: u32) -> Table {
    match lcid & 0xFFFF {
        0x0404 | 0x0C04 | 0x1404 | 0x7C04 => Table::ZhTw,
        0x0004 | 0x0804 | 0x1004 => Table::ZhCn,
        l if l & 0x3FF == 0x11 => Table::JaJp,
        l if l & 0x3FF == 0x12 => Table::KoKr,
        l if l & 0x3FF == 0x1E => Table::ThTh,
        _ => Table::None,
    }
}

/// Ids 27 to 36 and 50 to 58 of the zh-TW table, indexed by `id - 27` for 27..=36 then `id - 50 + 10` for 50..=58.
const ZH_TW: [&str; 19] = [
    "[$-404]e/m/d",
    "[$-404]e\"年\"m\"月\"d\"日\"",
    "[$-404]e\"年\"m\"月\"d\"日\"",
    "m/d/yy",
    "yyyy\"年\"m\"月\"d\"日\"",
    "hh\"時\"mm\"分\"",
    "hh\"時\"mm\"分\"ss\"秒\"",
    "上午/下午hh\"時\"mm\"分\"",
    "上午/下午hh\"時\"mm\"分\"ss\"秒\"",
    "[$-404]e/m/d",
    "[$-404]e/m/d",
    "[$-404]e\"年\"m\"月\"d\"日\"",
    "上午/下午hh\"時\"mm\"分\"",
    "上午/下午hh\"時\"mm\"分\"ss\"秒\"",
    "[$-404]e\"年\"m\"月\"d\"日\"",
    "上午/下午hh\"時\"mm\"分\"",
    "上午/下午hh\"時\"mm\"分\"ss\"秒\"",
    "[$-404]e/m/d",
    "[$-404]e\"年\"m\"月\"d\"日\"",
];

const ZH_CN: [&str; 19] = [
    "yyyy\"年\"m\"月\"",
    "m\"月\"d\"日\"",
    "m\"月\"d\"日\"",
    "m-d-yy",
    "yyyy\"年\"m\"月\"d\"日\"",
    "h\"时\"mm\"分\"",
    "h\"时\"mm\"分\"ss\"秒\"",
    "上午/下午h\"时\"mm\"分\"",
    "上午/下午h\"时\"mm\"分\"ss\"秒\"",
    "yyyy\"年\"m\"月\"",
    "yyyy\"年\"m\"月\"",
    "m\"月\"d\"日\"",
    "yyyy\"年\"m\"月\"",
    "m\"月\"d\"日\"",
    "m\"月\"d\"日\"",
    "上午/下午h\"时\"mm\"分\"",
    "上午/下午h\"时\"mm\"分\"ss\"秒\"",
    "yyyy\"年\"m\"月\"",
    "m\"月\"d\"日\"",
];

const JA_JP: [&str; 19] = [
    "[$-411]ge.m.d",
    "[$-411]ggge\"年\"m\"月\"d\"日\"",
    "[$-411]ggge\"年\"m\"月\"d\"日\"",
    "m/d/yy",
    "yyyy\"年\"m\"月\"d\"日\"",
    "h\"時\"mm\"分\"",
    "h\"時\"mm\"分\"ss\"秒\"",
    "yyyy\"年\"m\"月\"",
    "m\"月\"d\"日\"",
    "[$-411]ge.m.d",
    "[$-411]ge.m.d",
    "[$-411]ggge\"年\"m\"月\"d\"日\"",
    "yyyy\"年\"m\"月\"",
    "m\"月\"d\"日\"",
    "[$-411]ggge\"年\"m\"月\"d\"日\"",
    "yyyy\"年\"m\"月\"",
    "m\"月\"d\"日\"",
    "[$-411]ge.m.d",
    "[$-411]ggge\"年\"m\"月\"d\"日\"",
];

const KO_KR: [&str; 19] = [
    "yyyy\"年\" mm\"月\" dd\"日\"",
    "mm-dd",
    "mm-dd",
    "mm-dd-yy",
    "yyyy\"년\" mm\"월\" dd\"일\"",
    "h\"시\" mm\"분\"",
    "h\"시\" mm\"분\" ss\"초\"",
    "yyyy-mm-dd",
    "yyyy-mm-dd",
    "yyyy\"年\" mm\"月\" dd\"日\"",
    "yyyy\"年\" mm\"月\" dd\"日\"",
    "mm-dd",
    "yyyy-mm-dd",
    "yyyy-mm-dd",
    "mm-dd",
    "yyyy-mm-dd",
    "yyyy-mm-dd",
    "yyyy\"年\" mm\"月\" dd\"日\"",
    "mm-dd",
];

/// th-TH ids 59 to 62, 67 to 70 and 71 to 81 (the table of 18.8.30). `None` entries (63 to 66) fall through.
fn thai(id: u32) -> Option<&'static str> {
    Some(match id {
        59 => "t0",
        60 => "t0.00",
        61 => "t#,##0",
        62 => "t#,##0.00",
        67 => "t0%",
        68 => "t0.00%",
        69 => "t# ?/?",
        70 => "t# ??/??",
        71 => "ว/ด/ปปปป",
        72 => "ว-ดดด-ปป",
        73 => "ว-ดดด",
        74 => "ดดด-ปป",
        75 => "ช:นน",
        76 => "ช:นน:ทท",
        77 => "ว/ด/ปปปป ช:นน",
        78 => "นน:ทท",
        79 => "[ช]:นน:ทท",
        80 => "นน:ทท.0",
        81 => "d/m/bb",
        _ => return None,
    })
}

fn east_asian(table: Table, id: u32) -> Option<&'static str> {
    let index = match id {
        27..=36 => (id - 27) as usize,
        50..=58 => (id - 50) as usize + 10,
        _ => return None,
    };
    let list = match table {
        Table::ZhTw => &ZH_TW,
        Table::ZhCn => &ZH_CN,
        Table::JaJp => &JA_JP,
        Table::KoKr => &KO_KR,
        Table::ThTh | Table::None => return None,
    };
    Some(list[index])
}

/// What a locale contributes to the built-in codes.
struct Params {
    short_date: String,
    long_time: String,
    day: &'static str,
    hour: &'static str,
    symbol: String,
    pos: u8,
    neg: u8,
}

impl Params {
    fn new(row: &Nls) -> Self {
        let short_date = nls_pattern_to_excel(row.short_date);
        let day = if short_date.replace("\"", "").contains("dd") {
            "dd"
        } else {
            "d"
        };
        let hour = if nls_pattern_to_excel(row.short_time).contains("hh") {
            "hh"
        } else {
            "h"
        };
        Self {
            short_date,
            long_time: nls_pattern_to_excel(row.time),
            day,
            hour,
            symbol: currency_literal(row.currency),
            pos: row.currency_pos,
            neg: row.currency_neg,
        }
    }

    /// The negative currency pattern wraps the number in parentheses.
    fn parens(&self) -> bool {
        matches!(self.neg, 0 | 4 | 14 | 15)
    }
}

/// How Excel writes a currency symbol in a code: `$` quoted (as in the specification's en-US codes), a lone
/// non-ASCII symbol bare (measured: `£` under en-GB), anything else quoted.
fn currency_literal(symbol: &str) -> String {
    let mut chars = symbol.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if !c.is_ascii() => symbol.to_owned(),
        _ => format!("\"{}\"", symbol.replace('"', "")),
    }
}

const POSITIVE: [&str; 4] = ["$n", "n$", "$ n", "n $"];
const NEGATIVE: [&str; 16] = [
    "($n)", "-$n", "$-n", "$n-", "(n$)", "-n$", "n-$", "n$-", "-n $", "-$ n", "n $-", "$ n-",
    "$ -n", "n- $", "($ n)", "(n $)",
];

fn render_currency(pattern: &str, symbol: &str, number: &str) -> String {
    let mut out = String::new();
    for c in pattern.chars() {
        match c {
            '$' => out.push_str(symbol),
            'n' => out.push_str(number),
            ' ' => out.push_str("\\ "),
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            other => out.push(other),
        }
    }
    out
}

/// Ids 5 to 8.
fn currency(p: &Params, decimals: bool, red: bool) -> String {
    let number = if decimals { "#,##0.00" } else { "#,##0" };
    let positive = render_currency(POSITIVE[usize::from(p.pos.min(3))], &p.symbol, number);
    let negative = render_currency(NEGATIVE[usize::from(p.neg.min(15))], &p.symbol, number);
    let pad = if p.parens() { "_)" } else { "" };
    let colour = if red { "[Red]" } else { "" };
    format!("{positive}{pad};{colour}{negative}")
}

/// Ids 37 to 40.
fn grouped(p: &Params, decimals: bool, red: bool) -> String {
    let number = if decimals { "#,##0.00" } else { "#,##0" };
    let colour = if red { "[Red]" } else { "" };
    if p.parens() {
        format!("{number}_);{colour}({number})")
    } else {
        format!("{number};{colour}-{number}")
    }
}

/// Ids 41 to 44 (accounting).
fn accounting(p: &Params, decimals: bool, with_symbol: bool) -> String {
    let number = if decimals { "#,##0.00" } else { "#,##0" };
    let zero = if decimals { "\"-\"??" } else { "\"-\"" };
    let (open, close) = if p.parens() {
        ("_(", "_)")
    } else {
        ("_-", "_-")
    };
    let spaced = matches!(p.pos, 2 | 3);
    let (before, after) = if !with_symbol {
        (String::new(), String::new())
    } else if matches!(p.pos, 0 | 2) {
        let space = if spaced { "\\ " } else { "" };
        (format!("{}{space}", p.symbol), String::new())
    } else {
        let space = if spaced { "\\ " } else { "" };
        (String::new(), format!("{space}{}", p.symbol))
    };
    let negative = if p.parens() {
        format!("{open}{before}* \\({number}\\){after}")
    } else {
        format!("-{before}* {number}{after}")
    };
    // The negative section of a parenthesis locale ends without padding: the closing `\)` is the padding.
    let negative = if p.parens() {
        negative
    } else {
        format!("{negative}{close}")
    };
    format!(
        "{open}{before}* {number}{after}{close};{negative};{open}{before}* {zero}{after}{close};{open}@{close}"
    )
}

/// The format code that Excel resolves for built-in `numFmtId` `id` under the Windows locale `lcid`, in the
/// invariant syntax a file stores. `None` for ids that are not built in (23 to 26, 82 and up); Excel shows those
/// as General.
pub fn builtin_number_format_in(id: u32, lcid: u32) -> Option<String> {
    let row = row_for(lcid);
    let table = table_for(row.lcid);
    if table == Table::ThTh
        && let Some(code) = thai(id)
    {
        return Some(code.to_owned());
    }
    if let Some(code) = east_asian(table, id) {
        return Some(code.to_owned());
    }
    let p = Params::new(row);
    let static_code = |code: &str| Some(code.to_owned());
    match id {
        0 => static_code("General"),
        1 | 59 => static_code("0"),
        2 | 60 => static_code("0.00"),
        3 | 61 => static_code("#,##0"),
        4 | 62 => static_code("#,##0.00"),
        5 | 63 => Some(currency(&p, false, false)),
        6 | 64 => Some(currency(&p, false, true)),
        7 | 65 => Some(currency(&p, true, false)),
        8 | 66 => Some(currency(&p, true, true)),
        9 | 67 => static_code("0%"),
        10 | 68 => static_code("0.00%"),
        11 => static_code("0.00E+00"),
        12 | 69 => static_code("# ?/?"),
        13 | 70 => static_code("# ??/??"),
        14 | 27..=31 | 36 | 50..=58 | 71 | 72 => Some(p.short_date),
        15 | 73 => Some(format!("{}-mmm-yy", p.day)),
        16 | 74 => Some(format!("{}-mmm", p.day)),
        17 | 75 => static_code("mmm-yy"),
        18 => static_code("h:mm AM/PM"),
        19 => static_code("h:mm:ss AM/PM"),
        20 | 76 => Some(format!("{}:mm", p.hour)),
        21 | 77 => Some(format!("{}:mm:ss", p.hour)),
        22 | 78 => Some(format!("{} {}:mm", p.short_date, p.hour)),
        32..=35 => Some(p.long_time),
        37 => Some(grouped(&p, false, false)),
        38 => Some(grouped(&p, false, true)),
        39 => Some(grouped(&p, true, false)),
        40 => Some(grouped(&p, true, true)),
        41 => Some(accounting(&p, false, false)),
        42 => Some(accounting(&p, false, true)),
        43 => Some(accounting(&p, true, false)),
        44 => Some(accounting(&p, true, true)),
        45 | 79 => static_code("mm:ss"),
        46 | 80 => static_code("[h]:mm:ss"),
        47 | 81 => static_code("mm:ss.0"),
        48 => static_code("##0.0E+0"),
        49 => static_code("@"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_conversion() {
        assert_eq!(nls_pattern_to_excel("M/d/yyyy"), "m/d/yyyy");
        assert_eq!(nls_pattern_to_excel("dd.MM.yy"), "dd.mm.yy");
        assert_eq!(nls_pattern_to_excel("h:mm:ss tt"), "h:mm:ss AM/PM");
        assert_eq!(nls_pattern_to_excel("HH:mm"), "hh:mm");
        assert_eq!(
            nls_pattern_to_excel("d.M.yyyy '\u{433}.'"),
            "d.m.yyyy \"\u{433}.\""
        );
    }

    #[test]
    fn unknown_lcids_fall_back_to_the_language_then_en_us() {
        assert_eq!(resolve_number_format_lcid(0x0409), 0x0409);
        // No row for this sub-locale: the language's default sub-locale (en-US) answers.
        assert_eq!(resolve_number_format_lcid(0x7009), EN_US);
        assert_eq!(resolve_number_format_lcid(0xFFFF), EN_US);
    }
}
