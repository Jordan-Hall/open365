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

mod date;
mod decimal;
mod layout;
mod parse;

use std::fmt;

use crate::value::Value;
use parse::{Kind, NumberFormat, Section};

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
}

impl Out {
    fn new() -> Self {
        Out {
            text: String::new(),
            width: 0.0,
            fill: None,
        }
    }

    pub(crate) fn push(&mut self, s: &str) {
        self.text.push_str(s);
        self.width += s.chars().map(glyph_units).sum::<f64>();
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
    let format = NumberFormat::parse(code)?;
    Ok(format_with(&format, value, date1904))
}

/// Formats with an already parsed code.
pub fn format_with(format: &NumberFormat, value: &Value, date1904: bool) -> String {
    match value {
        Value::Number(n) => format_number(format, *n, date1904),
        Value::Text(s) => format_text(format, s),
        Value::Bool(b) => format_text(format, if *b { "TRUE" } else { "FALSE" }),
        Value::Error(e) => e.as_str().to_owned(),
        Value::Empty => String::new(),
        Value::Array(_) => crate::value::ErrorKind::Value.as_str().to_owned(),
    }
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

fn format_text(format: &NumberFormat, s: &str) -> String {
    let n = format.sections.len();
    let idx = match text_section(format) {
        Some(i) => i,
        None if n == 1 && format.sections[0].kind == Kind::Text => 0,
        None => return s.to_owned(),
    };
    let mut out = Out::new();
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

fn format_number(format: &NumberFormat, x: f64, date1904: bool) -> String {
    if !x.is_finite() {
        return crate::value::ErrorKind::Num.as_str().to_owned();
    }
    let secs = &format.sections;
    if secs.len() == 1 && secs[0].kind == Kind::Text {
        return decimal::general_text(x);
    }
    let Some((idx, own_sign)) = choose_section(format, x) else {
        return hashes();
    };
    let sec = &secs[idx];
    let mut out = Out::new();
    let negative = x < 0.0 && !own_sign;
    let ok = render_section(sec, x.abs(), negative, date1904, &mut out);
    if !ok || (sec.kind != Kind::Text && out.width > COLUMN_UNITS) {
        return hashes();
    }
    out.finish()
}

fn render_section(sec: &Section, x: f64, negative: bool, date1904: bool, out: &mut Out) -> bool {
    use parse::Tok;
    match sec.kind {
        Kind::Date => {
            // The 1900 system has no negative dates; the 1904 system shows the magnitude with a minus sign.
            if negative && !date1904 {
                return false;
            }
            let ok = date::render_date(sec, x, date1904, out);
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
                    Tok::Lit(s) => out.push(s),
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
                        Tok::Lit(s) => out.push(s),
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
