//! The short code `CELL("format")` reports for a number format (`F2`, `,0`, `P1`, `D4`, `G`, ...).
//!
//! Excel documents the table but not the rules behind it; these were measured on Excel 16 with
//! `tools/office/calc-oracle.ps1` (corpus group `cellfmt`):
//!
//! * the first section decides: no digit placeholder, text and fraction formats are `G`; an empty first
//!   section is `H` when the next two are empty too (`;;;`) and `G` otherwise; the system date codes
//!   `[$-F800]` and `[$-F400]` are `G`;
//! * a currency symbol written unquoted or escaped makes it `C<decimals>`, otherwise `%` gives `P`, an exponent
//!   `S` (decimals of the mantissa), a thousands separator `,` and anything else `F`;
//! * `-` is appended when the first or second of at least two sections has a colour, `()` when the first
//!   section has an unquoted opening parenthesis;
//! * dates look only at the order of the first components: day-month-year `D1`, day-month `D2`, month-year
//!   `D3`, month-day-year `D4`, month-day `D5`, and a leading hour-minute(-second) `D9`, `D8`, `D7` (AM/PM),
//!   `D6`; any other order is `G`. A `[$-LCID]` prefix counts as a leading day.

use super::parse::{Kind, NumberFormat, Section, Tok, fraction_shape, is_color, split_sections};

/// Quote-aware scan of one raw section. `currency` is the locale's currency symbol.
struct RawScan {
    color: bool,
    open_paren: bool,
    currency: bool,
}

fn scan_raw(section: &str, currency: &str) -> RawScan {
    let mut out = RawScan {
        color: false,
        open_paren: false,
        currency: false,
    };
    let chars: Vec<char> = section.chars().collect();
    let cur: Vec<char> = currency.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '"' => {
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    i += 1;
                }
            }
            '[' => {
                let start = i + 1;
                while i < chars.len() && chars[i] != ']' {
                    i += 1;
                }
                let body: String = chars[start..i.min(chars.len())].iter().collect();
                if is_color(&body) {
                    out.color = true;
                }
            }
            '_' | '*' => i += 1,
            '\\' => {
                i += 1;
                if let Some(&c) = chars.get(i) {
                    out.note_literal(c, &chars[i..], &cur);
                }
            }
            c => out.note_literal(c, &chars[i..], &cur),
        }
        i += 1;
    }
    out
}

impl RawScan {
    fn note_literal(&mut self, c: char, rest: &[char], currency: &[char]) {
        if c == '(' {
            self.open_paren = true;
        }
        if !currency.is_empty() && rest.starts_with(currency) {
            self.currency = true;
        }
    }
}

fn date_code(section: &Section) -> &'static str {
    #[derive(Clone, Copy, PartialEq)]
    enum C {
        D,
        M,
        Y,
        H,
        Mi,
        S,
        Other,
    }
    let mut seq: Vec<C> = section
        .toks
        .iter()
        .filter_map(|t| match t {
            Tok::Day(_) => Some(C::D),
            Tok::Month(_) => Some(C::M),
            Tok::Year(_) | Tok::BuddhistYear(_) => Some(C::Y),
            Tok::Hour(_) => Some(C::H),
            Tok::Minute(_) => Some(C::Mi),
            Tok::Second(_) => Some(C::S),
            Tok::Elapsed(..) | Tok::Era => Some(C::Other),
            _ => None,
        })
        .collect();
    if section.lcid.is_some() {
        seq.insert(0, C::D);
    }
    let at = |i: usize| seq.get(i).copied();
    match (at(0), at(1), at(2)) {
        (Some(C::H), Some(C::Mi), Some(C::S)) => {
            if section.has_ampm {
                "D6"
            } else {
                "D8"
            }
        }
        (Some(C::H), Some(C::Mi), _) => {
            if section.has_ampm {
                "D7"
            } else {
                "D9"
            }
        }
        (Some(C::D), Some(C::M), Some(C::Y)) => "D1",
        (Some(C::M), Some(C::D), Some(C::Y)) => "D4",
        (Some(C::D), Some(C::M), _) => "D2",
        (Some(C::M), Some(C::Y), _) => "D3",
        (Some(C::M), Some(C::D), _) => "D5",
        _ => "G",
    }
}

/// The `CELL("format")` code for a number format code; `currency` is the locale's currency symbol.
pub fn cell_format_code(code: &str, currency: &str) -> String {
    let Ok(raw) = split_sections(code) else {
        return "G".to_owned();
    };
    let Ok(parsed) = NumberFormat::parse(code) else {
        return "G".to_owned();
    };
    let lower = code.to_ascii_lowercase();
    if lower.contains("[$-f800]") || lower.contains("[$-f400]") {
        return "G".to_owned();
    }
    if raw.first().is_none_or(|s| s.trim().is_empty()) {
        let hidden = raw.iter().skip(1).take(2).all(|s| s.trim().is_empty());
        return if hidden { "H" } else { "G" }.to_owned();
    }
    let Some(first) = parsed.sections.first() else {
        return "G".to_owned();
    };
    let first_raw = scan_raw(raw[0], currency);
    let coloured = raw.len() >= 2
        && (first_raw.color || raw.get(1).is_some_and(|s| scan_raw(s, currency).color));
    let mut out = match first.kind {
        Kind::General | Kind::Text => "G".to_owned(),
        Kind::Date => date_code(first).to_owned(),
        Kind::Number => number_code(first, &first_raw),
    };
    if coloured {
        out.push('-');
    }
    if first_raw.open_paren && first.kind != Kind::Text {
        out.push_str("()");
    }
    out
}

fn number_code(section: &Section, raw: &RawScan) -> String {
    let toks = &section.toks;
    let has_digit = toks.iter().any(|t| matches!(t, Tok::Digit(_)));
    if !has_digit || fraction_shape(toks) {
        return "G".to_owned();
    }
    let exp = toks.iter().position(|t| matches!(t, Tok::Exp { .. }));
    let mantissa = &toks[..exp.unwrap_or(toks.len())];
    // Excel reports at most 15 decimals.
    let decimals = mantissa
        .iter()
        .position(|t| matches!(t, Tok::Point))
        .map_or(0, |p| {
            mantissa[p + 1..]
                .iter()
                .filter(|t| matches!(t, Tok::Digit(_)))
                .count()
                .min(15)
        });
    let letter = if raw.currency {
        "C"
    } else if toks.iter().any(|t| matches!(t, Tok::Percent)) {
        "P"
    } else if exp.is_some() {
        "S"
    } else if section.grouping {
        ","
    } else {
        "F"
    };
    format!("{letter}{decimals}")
}
