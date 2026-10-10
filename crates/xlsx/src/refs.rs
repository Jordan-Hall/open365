//! A1-style cell references and a small formula tokenizer that shifts relative
//! references (used to expand shared formulas).

/// Last worksheet row (1-based).
pub const MAX_ROW: u32 = 1_048_576;
/// Last worksheet column (`XFD`, 1-based).
pub const MAX_COL: u32 = 16_384;

/// `1` gives `A`, `27` gives `AA`, `16384` gives `XFD`. `0` gives an empty string.
pub fn col_letters(col: u32) -> String {
    let mut n = col;
    let mut out = Vec::new();
    while n > 0 {
        let rem = (n - 1) % 26;
        out.push(b'A' + u8::try_from(rem).unwrap_or(0));
        n = (n - 1) / 26;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// Column number for `A`..`XFD` (case-insensitive); `None` when out of range or not letters.
pub fn col_number(letters: &str) -> Option<u32> {
    if letters.is_empty() || letters.len() > 3 {
        return None;
    }
    let mut n: u32 = 0;
    for b in letters.bytes() {
        if !b.is_ascii_alphabetic() {
            return None;
        }
        n = n * 26 + u32::from(b.to_ascii_uppercase() - b'A' + 1);
    }
    (n <= MAX_COL).then_some(n)
}

/// Parses `B3` into `(row, col)`. No `$`, no sheet name; rejects anything outside the
/// 1048576 x 16384 grid.
pub fn parse_cell_ref(s: &str) -> Option<(u32, u32)> {
    let split = s.find(|c: char| !c.is_ascii_alphabetic())?;
    let (letters, digits) = s.split_at(split);
    let col = col_number(letters)?;
    if digits.is_empty() || digits.len() > 7 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let row: u32 = digits.parse().ok()?;
    (1..=MAX_ROW).contains(&row).then_some((row, col))
}

/// `(row, col)` as `B3`.
pub fn cell_ref_string(row: u32, col: u32) -> String {
    format!("{}{row}", col_letters(col))
}

/// Parses `A1` or `A1:C9` into `(row1, col1, row2, col2)` with the corners ordered.
pub fn parse_range(s: &str) -> Option<(u32, u32, u32, u32)> {
    let (a, b) = match s.split_once(':') {
        Some((a, b)) => (parse_cell_ref(a)?, parse_cell_ref(b)?),
        None => {
            let a = parse_cell_ref(s)?;
            (a, a)
        }
    };
    Some((a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1)))
}

#[derive(Debug)]
enum Tok {
    Word(String),
    Colon,
    Bang,
    LParen,
    /// Anything copied through unchanged.
    Other(String),
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
        || matches!(c, '_' | '.' | '$' | '\\')
        || (!c.is_ascii() && !c.is_whitespace())
}

fn tokenize(text: &str) -> Vec<Tok> {
    let chars: Vec<char> = text.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' | '\'' => {
                // String literal or quoted sheet name; a doubled quote is an escape.
                let mut j = i + 1;
                while j < chars.len() {
                    if chars[j] == c {
                        if chars.get(j + 1) == Some(&c) {
                            j += 2;
                            continue;
                        }
                        j += 1;
                        break;
                    }
                    j += 1;
                }
                toks.push(Tok::Other(chars[i..j.min(chars.len())].iter().collect()));
                i = j;
            }
            '[' => {
                // Structured reference or external workbook index; nested, `'` escapes.
                let mut depth = 0usize;
                let mut j = i;
                while j < chars.len() {
                    match chars[j] {
                        '\'' => j += 1,
                        '[' => depth += 1,
                        ']' => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                j += 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                let end = j.min(chars.len());
                toks.push(Tok::Other(chars[i..end].iter().collect()));
                i = end;
            }
            '#' => {
                // Error literal: #REF!, #N/A, #DIV/0!, #NAME?, ...
                let mut j = i + 1;
                while j < chars.len()
                    && (chars[j].is_alphanumeric() || matches!(chars[j], '!' | '?' | '/' | '_'))
                {
                    let stop = chars[j] == '!' || chars[j] == '?';
                    j += 1;
                    if stop {
                        break;
                    }
                }
                toks.push(Tok::Other(chars[i..j].iter().collect()));
                i = j;
            }
            ':' => {
                toks.push(Tok::Colon);
                i += 1;
            }
            '!' => {
                toks.push(Tok::Bang);
                i += 1;
            }
            '(' => {
                toks.push(Tok::LParen);
                i += 1;
            }
            c if is_word_char(c) => {
                let mut j = i;
                while j < chars.len() && is_word_char(chars[j]) {
                    j += 1;
                }
                toks.push(Tok::Word(chars[i..j].iter().collect()));
                i = j;
            }
            c => {
                toks.push(Tok::Other(c.to_string()));
                i += 1;
            }
        }
    }
    toks
}

/// `$?` plus the rest.
fn strip_dollar(s: &str) -> (bool, &str) {
    match s.strip_prefix('$') {
        Some(rest) => (true, rest),
        None => (false, s),
    }
}

struct CellParts {
    col: u32,
    col_abs: bool,
    row: u32,
    row_abs: bool,
}

fn parse_word_cell(word: &str) -> Option<CellParts> {
    let (col_abs, rest) = strip_dollar(word);
    let split = rest.find(|c: char| !c.is_ascii_alphabetic())?;
    let (letters, rest) = rest.split_at(split);
    let col = col_number(letters)?;
    let (row_abs, digits) = strip_dollar(rest);
    if digits.is_empty() || digits.len() > 7 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let row: u32 = digits.parse().ok()?;
    (1..=MAX_ROW).contains(&row).then_some(CellParts {
        col,
        col_abs,
        row,
        row_abs,
    })
}

/// `$?LETTERS` naming a valid column.
fn parse_word_col(word: &str) -> Option<(bool, u32)> {
    let (abs, letters) = strip_dollar(word);
    col_number(letters).map(|c| (abs, c))
}

/// `$?DIGITS` naming a valid row.
fn parse_word_row(word: &str) -> Option<(bool, u32)> {
    let (abs, digits) = strip_dollar(word);
    if digits.is_empty() || digits.len() > 7 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let row: u32 = digits.parse().ok()?;
    (1..=MAX_ROW).contains(&row).then_some((abs, row))
}

fn shifted(value: u32, delta: i64, max: u32) -> Option<u32> {
    let v = i64::from(value) + delta;
    u32::try_from(v).ok().filter(|v| (1..=max).contains(v))
}

const REF_ERROR: &str = "#REF!";

fn shift_word(
    word: &str,
    prev: Option<&Tok>,
    prev2: Option<&Tok>,
    next: Option<&Tok>,
    next2: Option<&Tok>,
    dr: i64,
    dc: i64,
) -> Option<String> {
    if matches!(next, Some(Tok::LParen | Tok::Bang)) {
        return None;
    }
    if let Some(cell) = parse_word_cell(word) {
        let col = if cell.col_abs {
            Some(cell.col)
        } else {
            shifted(cell.col, dc, MAX_COL)
        };
        let row = if cell.row_abs {
            Some(cell.row)
        } else {
            shifted(cell.row, dr, MAX_ROW)
        };
        return Some(match (col, row) {
            (Some(col), Some(row)) => format!(
                "{}{}{}{row}",
                if cell.col_abs { "$" } else { "" },
                col_letters(col),
                if cell.row_abs { "$" } else { "" },
            ),
            _ => REF_ERROR.to_owned(),
        });
    }
    let neighbour = |side: Option<&Tok>, beyond: Option<&Tok>| -> Option<String> {
        match (side, beyond) {
            (Some(Tok::Colon), Some(Tok::Word(w))) => Some(w.clone()),
            _ => None,
        }
    };
    let other = neighbour(next, next2).or_else(|| neighbour(prev, prev2));
    let other = other.as_deref();
    if let (Some((abs, col)), Some(_)) = (parse_word_col(word), other.and_then(parse_word_col)) {
        if abs {
            return None;
        }
        return Some(match shifted(col, dc, MAX_COL) {
            Some(col) => col_letters(col),
            None => REF_ERROR.to_owned(),
        });
    }
    if let (Some((abs, row)), Some(_)) = (parse_word_row(word), other.and_then(parse_word_row)) {
        if abs {
            return None;
        }
        return Some(match shifted(row, dr, MAX_ROW) {
            Some(row) => row.to_string(),
            None => REF_ERROR.to_owned(),
        });
    }
    None
}

/// Shifts every relative reference in formula `text` by `dr` rows and `dc` columns, the
/// way a shared formula is read by a dependent cell. `$`-absolute parts stay put;
/// references inside string literals, quoted sheet names, structured-reference brackets
/// and function names (`LOG10(`) are not touched. A reference shifted off the grid
/// becomes `#REF!`.
pub fn shift_formula(text: &str, dr: i64, dc: i64) -> String {
    if dr == 0 && dc == 0 {
        return text.to_owned();
    }
    let toks = tokenize(text);
    let mut out = String::with_capacity(text.len() + 8);
    for (i, tok) in toks.iter().enumerate() {
        match tok {
            Tok::Word(w) => {
                let prev = i.checked_sub(1).and_then(|k| toks.get(k));
                let prev2 = i.checked_sub(2).and_then(|k| toks.get(k));
                let next = toks.get(i + 1);
                let next2 = toks.get(i + 2);
                match shift_word(w, prev, prev2, next, next2, dr, dc) {
                    Some(s) => out.push_str(&s),
                    None => out.push_str(w),
                }
            }
            Tok::Colon => out.push(':'),
            Tok::Bang => out.push('!'),
            Tok::LParen => out.push('('),
            Tok::Other(s) => out.push_str(s),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_round_trip() {
        for n in [1, 26, 27, 52, 702, 703, 16_384] {
            assert_eq!(col_number(&col_letters(n)), Some(n));
        }
        assert_eq!(col_letters(16_384), "XFD");
        assert_eq!(col_number("XFE"), None);
        assert_eq!(col_number("AAAA"), None);
    }

    #[test]
    fn cell_refs_are_bounds_checked() {
        assert_eq!(parse_cell_ref("A1"), Some((1, 1)));
        assert_eq!(parse_cell_ref("XFD1048576"), Some((1_048_576, 16_384)));
        assert_eq!(parse_cell_ref("XFD1048577"), None);
        assert_eq!(parse_cell_ref("XFE1"), None);
        assert_eq!(parse_cell_ref("A0"), None);
        assert_eq!(parse_cell_ref("A"), None);
        assert_eq!(parse_cell_ref("1A"), None);
        assert_eq!(parse_cell_ref("A1B"), None);
        assert_eq!(parse_range("C9:A1"), Some((1, 1, 9, 3)));
    }

    #[test]
    fn shifts_relative_parts_only() {
        assert_eq!(shift_formula("A1*2+1", 1, 0), "A2*2+1");
        assert_eq!(shift_formula("$A1+A$1+$A$1+A1", 2, 3), "$A3+D$1+$A$1+D3");
        assert_eq!(shift_formula("SUM(A1:J1)", 4, 0), "SUM(A5:J5)");
        assert_eq!(shift_formula("SUM(A:A,B1:B$9)", 0, 1), "SUM(B:B,C1:C$9)");
        assert_eq!(shift_formula("SUM(1:1)", 2, 0), "SUM(3:3)");
        assert_eq!(shift_formula("SUM($1:$1)", 2, 0), "SUM($1:$1)");
    }

    #[test]
    fn leaves_strings_names_functions_and_sheet_names_alone() {
        assert_eq!(
            shift_formula("LOG10(A1)&\"A1 \"\"B2\"\"\"", 1, 1),
            "LOG10(B2)&\"A1 \"\"B2\"\"\""
        );
        assert_eq!(
            shift_formula("Sheet1!A1+'A1 b'!B2", 1, 0),
            "Sheet1!A2+'A1 b'!B3"
        );
        assert_eq!(shift_formula("'it''s'!A1", 0, 1), "'it''s'!B1");
        assert_eq!(
            shift_formula("Table1[[#This Row],[A1]]+A1", 1, 0),
            "Table1[[#This Row],[A1]]+A2"
        );
        assert_eq!(shift_formula("TaxRate*A1", 1, 0), "TaxRate*A2");
        assert_eq!(
            shift_formula("IF(A1=#REF!,#N/A,A1)", 1, 0),
            "IF(A2=#REF!,#N/A,A2)"
        );
        assert_eq!(shift_formula("1.5E3+A1", 1, 0), "1.5E3+A2");
        assert_eq!(
            shift_formula("[1]Sheet1!A1+_xlfn.IFERROR(B1,0)", 1, 0),
            "[1]Sheet1!A2+_xlfn.IFERROR(B2,0)"
        );
    }

    #[test]
    fn shifting_off_the_grid_gives_ref_error() {
        assert_eq!(shift_formula("A1+B2", -1, 0), "#REF!+B1");
        assert_eq!(shift_formula("XFD1", 0, 1), "#REF!");
    }
}
