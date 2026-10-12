//! Excel number formats: the text a cell shows for a value and a format code.
//!
//! [`format_value`] reproduces `Range.Text` of Excel 365 (en-US format syntax) for a cell in a column wide enough
//! for the content, which is how the conformance corpus records it. Behaviour measured against Excel, including
//! the differences from the documentation, is listed in `docs/testing/calc-conformance.md`.
//!
//! Display-only parts depend on the cell: `*x` fill repeats `x` to fill the column, and `####` replaces numbers
//! that do not fit. The reference column holds 60 digit widths (the oracle's column width), and glyph widths come
//! from a table measured in Excel's default font ([`glyph_units`]), so those parts match the oracle but are an
//! emulation of what a real column would show.

mod cellcode;
mod date;
mod decimal;
mod hijri;
mod layout;
mod parse;

use std::fmt;

use crate::fmtlocale::FormatLocale;
use crate::value::Value;
use parse::{Kind, NumberFormat, Section};

pub use cellcode::cell_format_code;
pub use parse::NumberFormat as ParsedFormat;

/// Width of the reference column in digit widths (the oracle sets `ColumnWidth = 60`).
pub const REFERENCE_COLUMN_DIGITS: usize = 60;

/// An invalid format code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatError {
    message: String,
}

impl FormatError {
    pub(crate) fn new(message: &str) -> Self {
        FormatError {
            message: message.to_owned(),
        }
    }
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid number format: {}", self.message)
    }
}

impl std::error::Error for FormatError {}

/// Usable width of the reference column in the units of [`glyph_units`].
const COLUMN_UNITS: f64 = 540.5;

/// Width of a glyph in ninths of a digit width (a digit is 9), measured in Excel's default font.
pub fn glyph_units(c: char) -> f64 {
    match c {
        ' ' | '\'' => 3.0,
        '!' | ',' | '-' | '/' | ';' | ']' | 'J' | 'r' => 6.0,
        '#' | '%' | '.' | '?' | '`' | '|' => 6.07,
        '(' | ')' | ':' | 'f' | 't' | '{' | '}' => 5.0,
        'I' | 'i' | 'j' | 'l' => 4.0,
        '&' | 'C' | 'D' | 'U' => 11.0,
        'A' | 'B' | 'P' | 'R' | 'V' => 10.0,
        'G' | 'H' | 'N' | 'O' | 'Q' | 'w' => 12.0,
        'L' | 'T' | 'Z' | 'g' | 'k' | 's' => 8.0,
        'v' | 'x' | 'y' | 'z' => 7.0,
        '0'..='9' => 8.95,
        'M' => 13.0,
        'm' => 14.0,
        'W' => 15.0,
        '@' => 15.0,
        _ => 9.0,
    }
}

/// Output under construction: text, its width in glyph units and the position of the fill character.
pub(crate) struct Out {
    text: String,
    width: f64,
    fill: Option<(usize, char)>,
    /// `*x` fill is ignored (the `TEXT` function).
    no_fill: bool,
    /// `[DBNum1]` to `[DBNum3]` under a CJK language prefix: the numerals that replace the ASCII digits of
    /// numbers and dates (literal text and names keep theirs).
    digits: Option<[char; 10]>,
    /// `[DBNum1]`: an unpadded date part of 10 or more is written in tens (`十五`).
    /// 0 none, 1 `[DBNum1]` (`十五`), 2 `[DBNum2]` (`壱拾伍`, the tens digit is always written).
    tens: u8,
}

/// The numerals of `[DBNum1]` (`n` = 1), `[DBNum2]` (2) and `[DBNum3]` (3) under a language prefix, measured in
/// Excel. Other languages leave the digits alone.
fn dbnum_digits(n: u8, lcid: u32) -> Option<[char; 10]> {
    let table: &[char; 10] = match (lcid & 0xFFFF, n) {
        (0x411, 1) => &[
            '\u{3007}', '\u{4e00}', '\u{4e8c}', '\u{4e09}', '\u{56db}', '\u{4e94}', '\u{516d}',
            '\u{4e03}', '\u{516b}', '\u{4e5d}',
        ],
        (0x411, 2) => &[
            '\u{3007}', '\u{58f1}', '\u{5f10}', '\u{53c2}', '\u{56db}', '\u{4f0d}', '\u{516d}',
            '\u{4e03}', '\u{516b}', '\u{4e5d}',
        ],
        (0x412, 1) => &[
            '\u{ff10}', '\u{4e00}', '\u{4e8c}', '\u{4e09}', '\u{56db}', '\u{4e94}', '\u{f9d1}',
            '\u{4e03}', '\u{516b}', '\u{4e5d}',
        ],
        (0x412, 2) => &[
            '\u{f9b2}', '\u{58f9}', '\u{8cb3}', '\u{f96b}', '\u{56db}', '\u{4f0d}', '\u{f9d1}',
            '\u{4e03}', '\u{516b}', '\u{4e5d}',
        ],
        (0x404 | 0xc04 | 0x1404 | 0x804 | 0x1004, 1) => &[
            '\u{25cb}', '\u{4e00}', '\u{4e8c}', '\u{4e09}', '\u{56db}', '\u{4e94}', '\u{516d}',
            '\u{4e03}', '\u{516b}', '\u{4e5d}',
        ],
        (0x404 | 0xc04 | 0x1404, 2) => &[
            '\u{96f6}', '\u{58f9}', '\u{8cb3}', '\u{53c3}', '\u{8086}', '\u{4f0d}', '\u{9678}',
            '\u{67d2}', '\u{634c}', '\u{7396}',
        ],
        (0x804 | 0x1004, 2) => &[
            '\u{96f6}', '\u{58f9}', '\u{8d30}', '\u{53c1}', '\u{8086}', '\u{4f0d}', '\u{9646}',
            '\u{67d2}', '\u{634c}', '\u{7396}',
        ],
        (0x411 | 0x412 | 0x404 | 0xc04 | 0x1404 | 0x804 | 0x1004, 3) => &[
            '\u{ff10}', '\u{ff11}', '\u{ff12}', '\u{ff13}', '\u{ff14}', '\u{ff15}', '\u{ff16}',
            '\u{ff17}', '\u{ff18}', '\u{ff19}',
        ],
        _ => return None,
    };
    Some(*table)
}

/// The Thai digits, which the Thai date letters use when the code has no `[$-]` tag.
const THAI_DIGITS: [char; 10] = [
    '\u{0e50}', '\u{0e51}', '\u{0e52}', '\u{0e53}', '\u{0e54}', '\u{0e55}', '\u{0e56}', '\u{0e57}',
    '\u{0e58}', '\u{0e59}',
];

/// The digit set of a `[$-xxxxxxxx]` tag's number-shape byte (bits 24 to 31), measured in Excel with the same result under
/// every language (`fixtures/calc/numfmt-nonlatin`). Bytes 0, 1 and the unlisted ones keep the ASCII digits. Tamil and
/// Ethiopic have no zero digit, so 0 stays `0` there.
#[rustfmt::skip]
fn shape_digits(shape: u8) -> Option<[char; 10]> {
    fn run(zero: u32) -> [char; 10] {
        std::array::from_fn(|i| char::from_u32(zero + i as u32).unwrap_or('0'))
    }
    fn no_zero(zero: u32) -> [char; 10] {
        let mut d = run(zero);
        d[0] = '0';
        d
    }
    Some(match shape {
        0x02 => run(0x0660),
        0x03 => run(0x06f0),
        0x04 => run(0x0966),
        0x05 => run(0x09e6),
        0x06 => run(0x0a66),
        0x07 => run(0x0ae6),
        0x08 => run(0x0b66),
        0x09 => no_zero(0x0be6),
        0x0a => run(0x0c66),
        0x0b => run(0x0ce6),
        0x0c => run(0x0d66),
        0x0d => THAI_DIGITS,
        0x0e => run(0x0ed0),
        0x0f => run(0x0f20),
        0x10 => run(0x1040),
        0x11 => no_zero(0x1368),
        0x12 => run(0x17e0),
        0x13 => run(0x1810),
        0x1b => ['\u{3007}', '\u{4e00}', '\u{4e8c}', '\u{4e09}', '\u{56db}', '\u{4e94}', '\u{516d}', '\u{4e03}', '\u{516b}', '\u{4e5d}'],
        0x1c => ['\u{3007}', '\u{58f1}', '\u{5f10}', '\u{53c2}', '\u{56db}', '\u{4f0d}', '\u{516d}', '\u{4e03}', '\u{516b}', '\u{4e5d}'],
        0x1d | 0x20 | 0x23 | 0x26 => run(0xff10),
        0x1e | 0x21 => ['\u{25cb}', '\u{4e00}', '\u{4e8c}', '\u{4e09}', '\u{56db}', '\u{4e94}', '\u{516d}', '\u{4e03}', '\u{516b}', '\u{4e5d}'],
        0x1f => ['\u{96f6}', '\u{58f9}', '\u{8d30}', '\u{53c1}', '\u{8086}', '\u{4f0d}', '\u{9646}', '\u{67d2}', '\u{634c}', '\u{7396}'],
        0x22 => ['\u{96f6}', '\u{58f9}', '\u{8cb3}', '\u{53c3}', '\u{8086}', '\u{4f0d}', '\u{9678}', '\u{67d2}', '\u{634c}', '\u{7396}'],
        0x24 => ['\u{ff10}', '\u{4e00}', '\u{4e8c}', '\u{4e09}', '\u{56db}', '\u{4e94}', '\u{f9d1}', '\u{4e03}', '\u{516b}', '\u{4e5d}'],
        0x25 => ['\u{f9b2}', '\u{58f9}', '\u{8cb3}', '\u{f96b}', '\u{56db}', '\u{4f0d}', '\u{f9d1}', '\u{4e03}', '\u{516b}', '\u{4e5d}'],
        0x27 => ['\u{c601}', '\u{c77c}', '\u{c774}', '\u{c0bc}', '\u{c0ac}', '\u{c624}', '\u{c721}', '\u{ce60}', '\u{d314}', '\u{ad6c}'],
        _ => return None,
    })
}

impl Out {
    fn new(target: Target) -> Self {
        Out {
            text: String::new(),
            width: 0.0,
            fill: None,
            no_fill: target == Target::Function,
            digits: None,
            tens: 0,
        }
    }

    /// Text produced from a number or date part: its digits become the section's numerals (see `digits`).
    pub(crate) fn push(&mut self, s: &str) {
        match self.digits {
            Some(table) if s.chars().any(|c| c.is_ascii_digit()) => {
                let mapped: String = s
                    .chars()
                    .map(|c| match c.to_digit(10) {
                        Some(d) if c.is_ascii_digit() => table[d as usize],
                        _ => c,
                    })
                    .collect();
                self.push_lit(&mapped);
            }
            _ => self.push_lit(s),
        }
    }

    /// Literal text and names: pushed as they are.
    pub(crate) fn push_lit(&mut self, s: &str) {
        self.text.push_str(s);
        self.width += s.chars().map(glyph_units).sum::<f64>();
    }

    /// A two-digit date or time part (`dd`, `mm`, `hh`, `ss`): in tens under `[DBNum1]` and `[DBNum2]` from 10 up.
    pub(crate) fn push_padded(&mut self, n: i64) {
        if self.tens > 0 && n >= 10 {
            self.push_unpadded(n);
        } else {
            self.push(&format!("{n:02}"));
        }
    }

    /// An unpadded date part (`d`, `m`, `h`).
    pub(crate) fn push_unpadded(&mut self, n: i64) {
        match self.digits {
            Some(table) if self.tens > 0 && n >= 10 => {
                let (t, u) = ((n / 10) as usize, (n % 10) as usize);
                let mut s = String::new();
                if t > 1 || self.tens == 2 {
                    s.push(table[t]);
                }
                s.push(if self.tens == 2 {
                    '\u{62fe}'
                } else {
                    '\u{5341}'
                });
                if u > 0 {
                    s.push(table[u]);
                }
                self.push_lit(&s);
            }
            _ => self.push(&n.to_string()),
        }
    }

    /// `_x`: a space. Excel reserves the width of a space whatever `x` is (measured: `_W` and `_(` cost the same).
    pub(crate) fn pad(&mut self, _c: char) {
        self.push(" ");
    }

    /// `*x`: only the first fill of a section counts.
    pub(crate) fn fill(&mut self, c: char) {
        if self.fill.is_none() {
            self.fill = Some((self.text.len(), c));
        }
    }

    fn insert_minus(&mut self) {
        self.text.insert(0, '-');
        self.width += glyph_units('-');
        if let Some((at, _)) = self.fill.as_mut() {
            *at += 1;
        }
    }

    fn finish(self) -> String {
        match self.fill {
            None => self.text,
            Some(_) if self.no_fill => self.text,
            Some((at, c)) => {
                let room = COLUMN_UNITS - self.width;
                let n = if room > 0.0 {
                    (room / glyph_units(c) + 1e-9).floor() as usize
                } else {
                    0
                };
                let mut s = String::with_capacity(self.text.len() + n);
                s.push_str(&self.text[..at]);
                s.extend(std::iter::repeat_n(c, n));
                s.push_str(&self.text[at..]);
                s
            }
        }
    }
}

/// Where the text goes. A cell has a column width (`####` on overflow, `*x` fill); the `TEXT` function has neither.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Cell,
    Function,
}

fn hashes() -> String {
    "#".repeat(REFERENCE_COLUMN_DIGITS)
}

/// The built-in format code for an `numFmtId` (en-US). `None` for ids that depend on the locale (5-8 are the
/// US currency forms, 23-36 and 50-81 are locale-specific date and CJK formats) or are unassigned.
pub fn builtin_format_code(id: u32) -> Option<&'static str> {
    Some(match id {
        0 => "General",
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        5 => "$#,##0_);($#,##0)",
        6 => "$#,##0_);[Red]($#,##0)",
        7 => "$#,##0.00_);($#,##0.00)",
        8 => "$#,##0.00_);[Red]($#,##0.00)",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        12 => "# ?/?",
        13 => "# ??/??",
        14 => "m/d/yyyy",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "m/d/yyyy h:mm",
        37 => "#,##0_);(#,##0)",
        38 => "#,##0_);[Red](#,##0)",
        39 => "#,##0.00_);(#,##0.00)",
        40 => "#,##0.00_);[Red](#,##0.00)",
        41 => "_(* #,##0_);_(* (#,##0);_(* \"-\"_);_(@_)",
        42 => "_($* #,##0_);_($* (#,##0);_($* \"-\"_);_(@_)",
        43 => "_(* #,##0.00_);_(* (#,##0.00);_(* \"-\"??_);_(@_)",
        44 => "_($* #,##0.00_);_($* (#,##0.00);_($* \"-\"??_);_(@_)",
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        47 => "mm:ss.0",
        48 => "##0.0E+0",
        49 => "@",
        _ => return None,
    })
}

/// Checks a format code without applying it.
pub fn validate_format(code: &str) -> Result<(), FormatError> {
    NumberFormat::parse(code).map(|_| ())
}

/// The text Excel shows for `value` with format `code`; an invalid code shows the value as General.
pub fn format_value(value: &Value, code: &str, date1904: bool) -> String {
    try_format_value(value, code, date1904).unwrap_or_else(|_| format_general(value))
}

fn format_general(value: &Value) -> String {
    match value {
        Value::Number(n) => decimal::general_text(*n),
        other => other.to_text().unwrap_or_else(|e| e.to_string()),
    }
}

/// Like [`format_value`], reporting an invalid format code.
pub fn try_format_value(value: &Value, code: &str, date1904: bool) -> Result<String, FormatError> {
    try_format_value_in(value, code, date1904, &FormatLocale::en_us())
}

/// Like [`format_value`] in a locale: month and day names and AM/PM follow it, unless the code carries its own
/// `[$-LCID]` prefix. Separators are not locale-dependent (measured: a `[$-LCID]` prefix leaves them alone).
pub fn format_value_in(value: &Value, code: &str, date1904: bool, locale: &FormatLocale) -> String {
    try_format_value_in(value, code, date1904, locale).unwrap_or_else(|_| format_general(value))
}

/// Like [`try_format_value`] in a locale.
pub fn try_format_value_in(
    value: &Value,
    code: &str,
    date1904: bool,
    locale: &FormatLocale,
) -> Result<String, FormatError> {
    let format = NumberFormat::parse(code)?;
    Ok(format_with_in(&format, value, date1904, locale))
}

/// The built-in format code of a `numFmtId` in a locale (see [`FormatLocale::builtin_format_code`]).
pub fn builtin_format_code_in(id: u32, locale: &FormatLocale) -> Option<String> {
    locale.builtin_format_code(id)
}

/// The text of the `TEXT` function: like a cell display without the column (no `####`, no `*x` fill).
/// `None` where a cell would show `####` (a negative date, a condition no section matches).
pub fn text_function(
    value: &Value,
    code: &str,
    date1904: bool,
) -> Result<Option<String>, FormatError> {
    text_function_in(value, code, date1904, &FormatLocale::en_us())
}

/// Like [`text_function`] in a locale.
pub fn text_function_in(
    value: &Value,
    code: &str,
    date1904: bool,
    locale: &FormatLocale,
) -> Result<Option<String>, FormatError> {
    let format = NumberFormat::parse(code)?;
    Ok(format_target(
        &format,
        value,
        date1904,
        Target::Function,
        locale,
    ))
}

/// `|x|` rounded half away from zero (on its 15 significant digits) to `decimals` places, `decimals` may be
/// negative to round left of the point; used by `FIXED` and `DOLLAR`. Returns the digits (with thousands
/// separators when `grouping`) and whether the rounded value is negative and not zero.
pub fn fixed_digits(x: f64, decimals: i32, grouping: bool) -> (String, bool) {
    let mut dec = decimal::Dec::from_f64(x);
    dec.round_at(i64::from(decimals));
    let int = dec.int_digits();
    let int = if int.is_empty() { "0".to_owned() } else { int };
    let mut out = String::new();
    for (i, c) in int.chars().enumerate() {
        if grouping && i > 0 && (int.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if decimals > 0 {
        out.push('.');
        out.push_str(&dec.frac_digits(decimals as usize));
    }
    (out, x < 0.0 && !dec.is_zero())
}

/// Formats with an already parsed code.
pub fn format_with(format: &NumberFormat, value: &Value, date1904: bool) -> String {
    format_with_in(format, value, date1904, &FormatLocale::en_us())
}

/// Like [`format_with`] in a locale.
pub fn format_with_in(
    format: &NumberFormat,
    value: &Value,
    date1904: bool,
    locale: &FormatLocale,
) -> String {
    format_target(format, value, date1904, Target::Cell, locale).unwrap_or_else(hashes)
}

fn format_target(
    format: &NumberFormat,
    value: &Value,
    date1904: bool,
    target: Target,
    locale: &FormatLocale,
) -> Option<String> {
    Some(match value {
        Value::Number(n) => return format_number(format, *n, date1904, target, locale),
        Value::Text(s) => format_text(format, s, target),
        Value::Bool(b) => format_text(format, if *b { "TRUE" } else { "FALSE" }, target),
        Value::Error(e) => e.as_str().to_owned(),
        Value::Empty => String::new(),
        Value::Array(_) => crate::value::ErrorKind::Value.as_str().to_owned(),
    })
}

/// The index of the text section, if the code has one.
fn text_section(format: &NumberFormat) -> Option<usize> {
    let n = format.sections.len();
    if n == 4 || (n >= 2 && format.sections[n - 1].kind == Kind::Text) {
        Some(n - 1)
    } else {
        None
    }
}

fn format_text(format: &NumberFormat, s: &str, target: Target) -> String {
    let n = format.sections.len();
    let idx = match text_section(format) {
        Some(i) => i,
        None if n == 1 && format.sections[0].kind == Kind::Text => 0,
        None => return s.to_owned(),
    };
    let mut out = Out::new(target);
    for tok in &format.sections[idx].toks {
        match tok {
            parse::Tok::At => out.push(s),
            parse::Tok::Lit(l) => out.push(l),
            parse::Tok::Pad(c) => out.pad(*c),
            parse::Tok::Fill(c) => out.fill(*c),
            _ => {}
        }
    }
    out.finish()
}

/// Picks the section for a number: the section index and whether the minus sign is left to the format.
fn choose_section(format: &NumberFormat, x: f64) -> Option<(usize, bool)> {
    use parse::{Cmp, Cond};
    let secs = &format.sections;
    let m = secs.len() - usize::from(text_section(format).is_some());
    if m <= 1 {
        return Some((0, false));
    }
    let neg_default = Cond {
        op: Cmp::Lt,
        limit: 0.0,
    };
    for i in 0..m {
        let explicit = secs[i].cond;
        let effective = match (m, i, explicit) {
            (_, _, Some(c)) => Some(c),
            (2, 0, None) => Some(Cond {
                op: Cmp::Ge,
                limit: 0.0,
            }),
            (3, 0, None) => Some(Cond {
                op: Cmp::Gt,
                limit: 0.0,
            }),
            (3, 1, None) => Some(neg_default),
            _ => None,
        };
        let hit = effective.is_none_or(|c| c.matches(x));
        if hit {
            let own_sign = x < 0.0
                && (effective.is_some_and(Cond::only_negative)
                    || (i == 1 && m == 2 && secs[0].cond.is_none() && explicit.is_none()));
            return Some((i, own_sign));
        }
    }
    None
}

fn format_number(
    format: &NumberFormat,
    x: f64,
    date1904: bool,
    target: Target,
    locale: &FormatLocale,
) -> Option<String> {
    if !x.is_finite() {
        return Some(crate::value::ErrorKind::Num.as_str().to_owned());
    }
    let secs = &format.sections;
    if secs.len() == 1 && secs[0].kind == Kind::Text {
        return Some(decimal::general_text(x));
    }
    let (idx, own_sign) = choose_section(format, x)?;
    let sec = &secs[idx];
    let mut out = Out::new(target);
    let negative = x < 0.0 && !own_sign;
    let ok = render_section(sec, x.abs(), negative, date1904, locale, &mut out);
    let overflow = target == Target::Cell && sec.kind != Kind::Text && out.width > COLUMN_UNITS;
    if !ok || overflow {
        return None;
    }
    Some(out.finish())
}

fn render_section(
    sec: &Section,
    x: f64,
    negative: bool,
    date1904: bool,
    locale: &FormatLocale,
    out: &mut Out,
) -> bool {
    use parse::Tok;
    out.digits = sec
        .dbnum
        .zip(sec.lcid)
        .and_then(|(n, lcid)| dbnum_digits(n, lcid))
        .or_else(|| shape_digits(sec.shape))
        .or_else(|| sec.thai.then_some(THAI_DIGITS));
    out.tens = match sec.dbnum {
        Some(n @ (1 | 2)) if out.digits.is_some() => n,
        _ => 0,
    };
    match sec.kind {
        Kind::Date => {
            // The 1900 system has no negative dates; the 1904 system shows the magnitude with a minus sign.
            if negative && !date1904 {
                return false;
            }
            let ok = date::render_date(sec, x, date1904, locale, out);
            if ok && negative {
                out.insert_minus();
            }
            ok
        }
        Kind::General | Kind::Text => {
            if negative {
                out.push("-");
            }
            let general = decimal::general_text(x);
            let mut shown = false;
            for tok in &sec.toks {
                match tok {
                    Tok::General => {
                        out.push(&general);
                        shown = true;
                    }
                    Tok::Lit(s) => out.push_lit(s),
                    Tok::Pad(c) => out.pad(*c),
                    Tok::Fill(c) => out.fill(*c),
                    _ => {}
                }
            }
            if !shown && sec.kind == Kind::Text {
                out.push(&general);
            }
            true
        }
        Kind::Number => {
            let has_digits = sec.toks.iter().any(|t| matches!(t, Tok::Digit(_)));
            if !has_digits {
                if negative {
                    out.push("-");
                }
                for tok in &sec.toks {
                    match tok {
                        Tok::Lit(s) => out.push_lit(s),
                        Tok::Pad(c) => out.pad(*c),
                        Tok::Fill(c) => out.fill(*c),
                        Tok::Percent => out.push("%"),
                        Tok::Slash => out.push("/"),
                        Tok::Point => out.push("."),
                        _ => {}
                    }
                }
            } else {
                let nonzero = if layout::is_scientific(sec) {
                    layout::render_sci(sec, x, out)
                } else if layout::is_fraction(sec) {
                    layout::render_fraction(sec, x, out)
                } else {
                    layout::render_plain(sec, x, out)
                };
                // A value that rounds to zero loses its minus sign.
                if negative && nonzero {
                    out.insert_minus();
                }
            }
            true
        }
    }
}

#[cfg(test)]
mod tests;
