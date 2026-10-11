//! Numbers in fields: the default number format, `\#` pictures, number-to-words and the
//! `\*` numbering formats (Roman, alphabetic, hex, ordinal, CardText, OrdText, DollarText).
//!
//! Word keeps numbers as 14-digit decimals and rounds half away from zero; this module does
//! the same by working on the decimal digits of the shortest round-trip representation.

/// Message Word writes when a number does not fit a `\*` format.
pub(crate) const NOT_REPRESENTABLE: &str =
    "Error! Number cannot be represented in specified format.";

/// Decimal digits of `value`, at most 14 significant: `(integer digits, fraction digits)`
/// of the magnitude, without sign. The integer part is `"0"` for values below one.
fn digits_of(value: f64) -> (String, String) {
    let value = value.abs();
    if !value.is_finite() {
        return ("0".to_owned(), String::new());
    }
    // 14 significant digits, then expand the exponent form by hand.
    let text = format!("{value:.13e}");
    let Some((mantissa, exp)) = text.split_once('e') else {
        return ("0".to_owned(), String::new());
    };
    let exp: i32 = exp.parse().unwrap_or(0);
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let point = exp + 1; // digits before the decimal point
    let (int, frac) = if point <= 0 {
        let zeros = usize::try_from(-point).unwrap_or(0);
        ("0".to_owned(), format!("{}{digits}", "0".repeat(zeros)))
    } else {
        let point = usize::try_from(point).unwrap_or(0);
        if digits.len() <= point {
            (
                format!("{digits}{}", "0".repeat(point - digits.len())),
                String::new(),
            )
        } else {
            (digits[..point].to_owned(), digits[point..].to_owned())
        }
    };
    let int = int.trim_start_matches('0');
    let int = if int.is_empty() { "0" } else { int };
    (int.to_owned(), frac.trim_end_matches('0').to_owned())
}

/// Rounds the digit strings half away from zero to `decimals` fraction digits and pads the
/// fraction to exactly that many digits.
fn round_digits(int: &str, frac: &str, decimals: usize) -> (String, String) {
    if frac.len() <= decimals {
        return (int.to_owned(), format!("{frac:0<decimals$}"));
    }
    let round_up = frac.as_bytes().get(decimals).is_some_and(|d| *d >= b'5');
    let mut digits: Vec<u8> = int.bytes().chain(frac.bytes().take(decimals)).collect();
    if round_up {
        let mut i = digits.len();
        loop {
            if i == 0 {
                digits.insert(0, b'1');
                break;
            }
            i -= 1;
            if digits[i] == b'9' {
                digits[i] = b'0';
            } else {
                digits[i] += 1;
                break;
            }
        }
    }
    let split = digits.len() - decimals;
    let text: String = digits.iter().map(|b| char::from(*b)).collect();
    let int = text[..split].trim_start_matches('0');
    let int = if int.is_empty() { "0" } else { int };
    (int.to_owned(), text[split..].to_owned())
}

/// `value` rounded half away from zero to `decimals` places (negative: tens, hundreds...).
pub(crate) fn round_decimal(value: f64, decimals: i32) -> f64 {
    if !value.is_finite() {
        return value;
    }
    let negative = value < 0.0;
    let (int, frac) = digits_of(value);
    let text = if decimals >= 0 {
        let (i, f) = round_digits(&int, &frac, usize::try_from(decimals).unwrap_or(0));
        format!("{i}.{f}")
    } else {
        // Round to 10^-decimals: shift the digit string.
        let places = usize::try_from(-decimals).unwrap_or(0);
        if int.len() <= places {
            let half = int.len() == places && int.as_bytes().first().is_some_and(|d| *d >= b'5');
            if half {
                format!("1{}", "0".repeat(places))
            } else {
                "0".to_owned()
            }
        } else {
            let keep = int.len() - places;
            let head = &int[..keep];
            let up = int.as_bytes().get(keep).is_some_and(|d| *d >= b'5');
            let (rounded, _) = round_digits(head, if up { "5" } else { "0" }, 0);
            format!("{rounded}{}", "0".repeat(places))
        }
    };
    let magnitude: f64 = text.parse().unwrap_or(0.0);
    if negative { -magnitude } else { magnitude }
}

/// The default result of `=`: rounded to `scale` decimals, trailing zeros removed. A value
/// that rounds to zero but is not zero reads `0.0`.
pub(crate) fn default_number_text(value: f64, scale: u32) -> String {
    let negative = value < 0.0;
    let (int, frac) = digits_of(value);
    let (int, frac) = round_digits(&int, &frac, usize::try_from(scale).unwrap_or(0));
    let frac = frac.trim_end_matches('0');
    let zero = int == "0" && frac.is_empty();
    if zero {
        return if value == 0.0 || scale == 0 {
            "0".to_owned()
        } else {
            "0.0".to_owned()
        };
    }
    let sign = if negative { "-" } else { "" };
    if frac.is_empty() {
        format!("{sign}{int}")
    } else {
        format!("{sign}{int}.{frac}")
    }
}

// ---------------------------------------------------------------------------
// `\#` pictures
// ---------------------------------------------------------------------------

/// Characters Word uses in numeric results; the defaults are the English ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberSymbols {
    pub decimal: char,
    pub group: char,
}

impl Default for NumberSymbols {
    fn default() -> Self {
        Self {
            decimal: '.',
            group: ',',
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    /// `0`
    Zero,
    /// `#`
    Hash,
    /// `x`
    Trunc,
    Point,
    Group,
    /// `-`
    Minus,
    /// `+`
    Plus,
    Lit(String),
}

fn split_sections(picture: &str) -> Vec<String> {
    let mut sections = vec![String::new()];
    let mut quoted = false;
    for c in picture.chars() {
        match c {
            '\'' => {
                quoted = !quoted;
                if let Some(last) = sections.last_mut() {
                    last.push(c);
                }
            }
            ';' if !quoted => sections.push(String::new()),
            _ => {
                if let Some(last) = sections.last_mut() {
                    last.push(c);
                }
            }
        }
    }
    sections
}

fn parse_section(section: &str) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut chars = section.chars().peekable();
    while let Some(c) = chars.next() {
        pieces.push(match c {
            '\\' => continue,
            '0' => Piece::Zero,
            '#' => Piece::Hash,
            'x' => Piece::Trunc,
            '.' => Piece::Point,
            ',' => Piece::Group,
            '-' => Piece::Minus,
            '+' => Piece::Plus,
            '\'' => {
                let mut lit = String::new();
                for q in chars.by_ref() {
                    if q == '\'' {
                        break;
                    }
                    lit.push(q);
                }
                Piece::Lit(lit)
            }
            other => Piece::Lit(other.to_string()),
        });
    }
    pieces
}

/// Formats `value` with a Word numeric picture (`\# "..."`).
///
/// Digits fill the digit placeholders from the right across the whole picture, extra digits
/// stand in front of the first placeholder, `#` shows a space where there is no digit, a `,`
/// in the integer part groups all digits in threes, `-` and `+` stand for the sign and the
/// sections are positive;negative;zero.
#[must_use]
pub fn format_picture(value: f64, picture: &str, symbols: &NumberSymbols) -> String {
    let sections = split_sections(picture);
    let negative = value < 0.0;
    let zero = value == 0.0;
    // Section choice: positive, negative, zero. A missing section falls back to the first.
    let (section, own_sign) = if zero && sections.len() >= 3 {
        (&sections[2], true)
    } else if negative && sections.len() >= 2 {
        (&sections[1], true)
    } else {
        (&sections[0], false)
    };
    let pieces: Vec<Piece> = parse_section(section);

    let point = pieces.iter().position(|p| *p == Piece::Point);
    let is_slot = |p: &Piece| matches!(p, Piece::Zero | Piece::Hash | Piece::Trunc);
    let frac_slots = point.map_or(0, |at| {
        pieces[at + 1..].iter().filter(|p| is_slot(p)).count()
    });
    let (int, frac) = digits_of(value);
    let (int, frac) = round_digits(&int, &frac, frac_slots);
    let all_zero = int == "0" && frac.bytes().all(|b| b == b'0');

    let int_end = point.unwrap_or(pieces.len());
    let int_pieces = &pieces[..int_end];
    let int_slots = int_pieces.iter().filter(|p| is_slot(p)).count();
    // `x` as the first placeholder drops the digits that do not fit.
    let truncating = int_pieces.iter().find(|p| is_slot(p)) == Some(&Piece::Trunc);
    let group = int_pieces.contains(&Piece::Group);
    let has_sign_piece = pieces
        .iter()
        .any(|p| matches!(p, Piece::Minus | Piece::Plus));
    // Integer digits; a pure fraction has none.
    let mut int_digits: Vec<char> = if int == "0" {
        Vec::new()
    } else {
        int.chars().collect()
    };
    let mut truncated = false;
    if truncating && int_digits.len() > int_slots {
        int_digits.drain(..int_digits.len() - int_slots);
        truncated = true;
    }
    let digit_count = int_digits.len();
    let surplus = digit_count.saturating_sub(int_slots);
    let auto_sign = negative && !own_sign && !has_sign_piece;

    let mut out = String::new();
    let mut slot = 0usize;
    let mut sign_pending = auto_sign && !truncated;
    let first_slot_at = int_pieces.iter().position(is_slot);
    // Emits one integer digit, followed by a group separator when one is due.
    let push_digit = |out: &mut String, index: usize| {
        out.push(int_digits[index]);
        let from_right = digit_count - 1 - index;
        if group && from_right > 0 && from_right.is_multiple_of(3) {
            out.push(symbols.group);
        }
    };
    for (idx, piece) in int_pieces.iter().enumerate() {
        match piece {
            Piece::Zero | Piece::Hash | Piece::Trunc => {
                if sign_pending && Some(idx) == first_slot_at && !truncating {
                    out.push('-');
                    sign_pending = false;
                }
                if slot == 0 {
                    for i in 0..surplus {
                        push_digit(&mut out, i);
                    }
                }
                let index = (digit_count + slot).checked_sub(int_slots);
                if sign_pending
                    && truncating
                    && (index.is_some_and(|i| i < digit_count) || *piece == Piece::Zero)
                {
                    out.push('-');
                    sign_pending = false;
                }
                match index {
                    Some(i) if i < digit_count => push_digit(&mut out, i),
                    _ if *piece == Piece::Zero => out.push('0'),
                    _ => out.push(' '),
                }
                slot += 1;
            }
            Piece::Group => {}
            Piece::Minus => out.push(if negative { '-' } else { ' ' }),
            Piece::Plus => out.push(if negative {
                '-'
            } else if all_zero {
                ' '
            } else {
                '+'
            }),
            Piece::Lit(text) => out.push_str(text),
            Piece::Point => {}
        }
    }
    if sign_pending {
        // A picture without digit placeholders still carries the sign.
        out.insert(0, '-');
    }

    if let Some(at) = point {
        out.push(symbols.decimal);
        let mut fi = 0usize;
        let mut first = true;
        let last_nonzero = frac.bytes().rposition(|b| b != b'0');
        for piece in &pieces[at + 1..] {
            match piece {
                Piece::Zero | Piece::Hash | Piece::Trunc => {
                    let digit = frac.as_bytes().get(fi).copied().map_or('0', char::from);
                    let trailing = last_nonzero.is_none_or(|n| fi > n);
                    if *piece == Piece::Zero {
                        out.push(digit);
                    } else if trailing && !(first && all_zero) {
                        out.push(' ');
                    } else {
                        out.push(digit);
                    }
                    fi += 1;
                    first = false;
                }
                Piece::Lit(text) => out.push_str(text),
                Piece::Minus => out.push(if negative { '-' } else { ' ' }),
                Piece::Plus => out.push(if negative {
                    '-'
                } else if all_zero {
                    ' '
                } else {
                    '+'
                }),
                Piece::Point | Piece::Group => {}
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// `\*` numbering formats
// ---------------------------------------------------------------------------

/// `value` as an integer for a numbering format: rounded half away from zero.
fn rounded_int(value: f64) -> Option<i64> {
    let r = round_decimal(value, 0);
    if r.abs() > 1.0e15 {
        return None;
    }
    // Truncation is exact: `r` is integral and below 1e15.
    #[allow(clippy::cast_possible_truncation)]
    Some(r as i64)
}

const ONES: [&str; 20] = [
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
];
const TENS: [&str; 10] = [
    "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
];
const ORD_ONES: [&str; 20] = [
    "zeroth",
    "first",
    "second",
    "third",
    "fourth",
    "fifth",
    "sixth",
    "seventh",
    "eighth",
    "ninth",
    "tenth",
    "eleventh",
    "twelfth",
    "thirteenth",
    "fourteenth",
    "fifteenth",
    "sixteenth",
    "seventeenth",
    "eighteenth",
    "nineteenth",
];
const ORD_TENS: [&str; 10] = [
    "",
    "",
    "twentieth",
    "thirtieth",
    "fortieth",
    "fiftieth",
    "sixtieth",
    "seventieth",
    "eightieth",
    "ninetieth",
];

/// English words for 0..=999999. `ordinal` turns the last word into its ordinal form.
fn words(n: u32, ordinal: bool) -> String {
    let mut parts: Vec<String> = Vec::new();
    let thousands = n / 1000;
    let rest = n % 1000;
    if thousands > 0 {
        parts.push(format!("{} thousand", below_thousand(thousands, false)));
    }
    if rest > 0 || thousands == 0 {
        parts.push(below_thousand(rest, false));
    }
    let mut text = parts.join(" ");
    if ordinal {
        text = ordinalise(&text);
    }
    text
}

fn below_thousand(n: u32, _ordinal: bool) -> String {
    let mut parts: Vec<String> = Vec::new();
    let hundreds = n / 100;
    let rest = n % 100;
    if hundreds > 0 {
        parts.push(format!(
            "{} hundred",
            ONES[usize::try_from(hundreds).unwrap_or(0)]
        ));
    }
    if rest > 0 || hundreds == 0 {
        if rest < 20 {
            parts.push(ONES[usize::try_from(rest).unwrap_or(0)].to_owned());
        } else {
            let tens = TENS[usize::try_from(rest / 10).unwrap_or(0)];
            let unit = rest % 10;
            if unit == 0 {
                parts.push(tens.to_owned());
            } else {
                parts.push(format!(
                    "{tens}-{}",
                    ONES[usize::try_from(unit).unwrap_or(0)]
                ));
            }
        }
    }
    parts.join(" ")
}

/// Replaces the final word of a cardinal phrase by its ordinal.
fn ordinalise(text: &str) -> String {
    let (head, last) = match text.rfind([' ', '-']) {
        Some(at) => (&text[..=at], &text[at + 1..]),
        None => ("", text),
    };
    let ord = ONES
        .iter()
        .position(|w| *w == last)
        .map(|i| ORD_ONES[i].to_owned())
        .or_else(|| {
            TENS.iter()
                .position(|w| !w.is_empty() && *w == last)
                .map(|i| ORD_TENS[i].to_owned())
        })
        .unwrap_or_else(|| format!("{last}th"));
    format!("{head}{ord}")
}

/// `\* CardText`: 0..=999999 after rounding; Word refuses 0.5..1 and negative numbers.
pub(crate) fn card_text(value: f64) -> Option<String> {
    let n = text_range(value)?;
    Some(words(n, false))
}

/// `\* OrdText`.
pub(crate) fn ord_text(value: f64) -> Option<String> {
    let n = text_range(value)?;
    Some(words(n, true))
}

fn text_range(value: f64) -> Option<u32> {
    if (0.5..1.0).contains(&value.abs()) && value.abs() >= 0.5 {
        return None;
    }
    let n = rounded_int(value)?;
    if value < 0.0 && n != 0 {
        return None;
    }
    if n.unsigned_abs() > 999_999 {
        return None;
    }
    u32::try_from(n.unsigned_abs()).ok()
}

/// `\* DollarText`: words for the whole part plus `nn/100` cents.
pub(crate) fn dollar_text(value: f64) -> Option<String> {
    if value < 0.0 {
        return None;
    }
    let (int, frac) = digits_of(value);
    let whole: u64 = int.parse().ok()?;
    if whole > 999_999 {
        return None;
    }
    let (_, cents) = round_digits("0", &frac, 2);
    let cents = if cents == "100" {
        "00".to_owned()
    } else {
        cents
    };
    let whole_text = words(u32::try_from(whole).ok()?, false);
    Some(format!("{whole_text} and {cents:0>2}/100"))
}

/// `\* Ordinal`: 1st, 2nd, 3rd, 4th...
pub(crate) fn ordinal_number(value: f64) -> Option<String> {
    if (0.5..1.0).contains(&value.abs()) {
        return None;
    }
    let n = rounded_int(value)?;
    if n.unsigned_abs() > 2_147_483_647 {
        return None;
    }
    let abs = n.unsigned_abs();
    let suffix = match (abs % 100, abs % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    };
    Some(format!("{n}{suffix}"))
}

/// `\* Roman` / `\* roman`: 0 gives a space, 1..=99999 are supported; thousands beyond 3999
/// repeat `M`.
pub(crate) fn roman(value: f64, lower: bool) -> Option<String> {
    if (0.5..1.0).contains(&value.abs()) {
        return None;
    }
    let n = rounded_int(value)?;
    if !(0..=32_767).contains(&n) {
        return None;
    }
    if n == 0 {
        return Some(" ".to_owned());
    }
    const TABLE: [(i64, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut left = n;
    let mut out = String::new();
    for (v, s) in TABLE {
        while left >= v {
            out.push_str(s);
            left -= v;
        }
    }
    Some(if lower { out.to_lowercase() } else { out })
}

/// `\* ALPHABETIC` / `\* alphabetic`: A..Z, AA..ZZ, AAA... (at most 30 letters).
pub(crate) fn alphabetic(value: f64, lower: bool) -> Option<String> {
    if (0.5..1.0).contains(&value.abs()) {
        return None;
    }
    let n = rounded_int(value)?;
    if !(0..=780).contains(&n) {
        return None;
    }
    if n == 0 {
        return Some(" ".to_owned());
    }
    let index = u8::try_from((n - 1) % 26).ok()?;
    let count = usize::try_from((n - 1) / 26 + 1).ok()?;
    let letter = char::from(if lower { b'a' } else { b'A' } + index);
    Some(std::iter::repeat_n(letter, count).collect())
}

/// `\* Hex`: 0..=65535 in upper case.
pub(crate) fn hex(value: f64) -> Option<String> {
    if (0.5..1.0).contains(&value.abs()) {
        return None;
    }
    let n = rounded_int(value)?;
    if !(0..=65_535).contains(&n) {
        return None;
    }
    Some(format!("{n:X}"))
}

/// `\* Arabic`: the rounded integer.
pub(crate) fn arabic(value: f64) -> Option<String> {
    if (0.5..1.0).contains(&value.abs()) {
        return None;
    }
    Some(rounded_int(value)?.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_text_trims_and_rounds() {
        assert_eq!(default_number_text(10.0 / 3.0, 2), "3.33");
        assert_eq!(default_number_text(3.0, 2), "3");
        assert_eq!(default_number_text(0.003, 2), "0.0");
        assert_eq!(default_number_text(-2.5, 1), "-2.5");
        assert_eq!(default_number_text(2.25, 1), "2.3");
    }

    #[test]
    fn pictures() {
        let s = NumberSymbols::default();
        assert_eq!(format_picture(1234.5678, "#,##0.00", &s), "1,234.57");
        assert_eq!(format_picture(0.0, "#,##0", &s), "   0");
        assert_eq!(format_picture(-1.0, "0.00", &s), "-1.00");
        assert_eq!(
            format_picture(-1.0, "$#,##0.00;($#,##0.00)", &s),
            "($   1.00)"
        );
        assert_eq!(format_picture(5.0, "000", &s), "005");
    }

    #[test]
    fn words_and_romans() {
        assert_eq!(card_text(21.0).as_deref(), Some("twenty-one"));
        assert_eq!(ord_text(22.0).as_deref(), Some("twenty-second"));
        assert_eq!(ord_text(100.0).as_deref(), Some("one hundredth"));
        assert_eq!(
            dollar_text(1234.56).as_deref(),
            Some("one thousand two hundred thirty-four and 56/100")
        );
        assert_eq!(roman(1234.0, false).as_deref(), Some("MCCXXXIV"));
        assert_eq!(alphabetic(40.0, false).as_deref(), Some("NN"));
        assert_eq!(ordinal_number(22.0).as_deref(), Some("22nd"));
        assert_eq!(ordinal_number(11.0).as_deref(), Some("11th"));
    }

    #[test]
    fn round_half_away() {
        assert!((round_decimal(2.345, 2) - 2.35).abs() < 1e-12);
        assert!((round_decimal(-2.5, 0) + 3.0).abs() < 1e-12);
        assert!((round_decimal(1234.567, -2) - 1200.0).abs() < 1e-12);
    }
}
