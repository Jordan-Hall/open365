//! How Word reads the text of a table cell for `= SUM(ABOVE)` and friends.
//!
//! Measured with the `tables` oracle set (`fixtures/docx/fields-tables.docx`). A cell is read as a
//! list of numbers: a comma followed by at least three digits is a thousands separator, any other
//! comma separates two numbers (`1,5` is 6, `12,34` is 46); a second `.` starts a new number
//! (`1.5.5` is 2). `(5)` is -5, `-(5)` is 5, `5%` and `50 %` are percentages, words and symbols such as
//! `$` are ignored (`2 apples` is 2). A leading `+`, a trailing `-`, an exponent without a sign
//! (`1e2`) or letters glued to digits (`0x10`) make the cell not a number at all, while `1e+2`
//! is 100. A cell that is more than a bare number (brackets, words, symbols, an exponent) is
//! "special": it only counts while no bare number was counted before it.

/// The numbers of one cell.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CellNumbers {
    pub values: Vec<f64>,
    /// More than a bare number (brackets, words, symbols, exponent).
    pub special: bool,
    /// A thousands separator was read.
    pub grouped: bool,
    /// The cell had brackets.
    pub parens: bool,
}

/// What a cell holds.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CellRead {
    Blank,
    NoNumber,
    Numbers(CellNumbers),
}

/// How the result of a formula over table cells is written when no `\#` picture is given.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TableStyle {
    pub grouped: bool,
    pub parens: bool,
}

impl TableStyle {
    /// Writes `text` (a plain number such as `-1003.5`) with thousands separators and brackets
    /// for a negative value, as far as the cells had them.
    pub(crate) fn apply(self, text: &str) -> String {
        let negative = text.starts_with('-');
        let body = text.trim_start_matches('-');
        let (int, frac) = match body.split_once('.') {
            Some((i, f)) => (i, Some(f)),
            None => (body, None),
        };
        let mut grouped = String::new();
        if self.grouped && int.chars().all(|c| c.is_ascii_digit()) {
            let len = int.len();
            for (i, c) in int.chars().enumerate() {
                if i > 0 && (len - i) % 3 == 0 {
                    grouped.push(',');
                }
                grouped.push(c);
            }
        } else {
            grouped.push_str(int);
        }
        if let Some(f) = frac {
            grouped.push('.');
            grouped.push_str(f);
        }
        match (negative, self.parens) {
            (true, true) => format!("({grouped})"),
            (true, false) => format!("-{grouped}"),
            _ => grouped,
        }
    }
}

fn three_digits(chars: &[char], at: usize) -> bool {
    chars
        .get(at..at + 3)
        .is_some_and(|s| s.iter().all(char::is_ascii_digit))
}

/// Reads the text of one cell.
pub(crate) fn read_cell(text: &str) -> CellRead {
    let chars: Vec<char> = text.chars().collect();
    if chars.iter().all(|c| c.is_whitespace()) {
        return CellRead::Blank;
    }
    let mut cell = CellNumbers {
        values: Vec::new(),
        special: false,
        grouped: false,
        parens: false,
    };
    let mut negative = false;
    let mut after_number = false;
    let mut gap_after_number = false;
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        match c {
            c if c.is_whitespace() => {
                gap_after_number |= after_number;
                after_number = false;
                i += 1;
            }
            '(' => {
                cell.parens = true;
                cell.special = true;
                negative = !negative;
                i += 1;
            }
            ')' => {
                cell.special = true;
                i += 1;
            }
            '-' => {
                if after_number {
                    return CellRead::NoNumber;
                }
                negative = !negative;
                i += 1;
            }
            '+' => return CellRead::NoNumber,
            ',' => {
                after_number = false;
                i += 1;
            }
            '%' => {
                // A percent sign apart from its number.
                if let Some(last) = cell.values.last_mut() {
                    *last /= 100.0;
                }
                cell.special = true;
                i += 1;
            }
            c if c.is_ascii_digit()
                || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit)) =>
            {
                // `1 000`: two numbers apart by a space are more than a bare number.
                if gap_after_number {
                    cell.special = true;
                }
                let mut digits = String::new();
                let mut seen_dot = false;
                while i < chars.len() {
                    let d = chars[i];
                    if d.is_ascii_digit() {
                        digits.push(d);
                        i += 1;
                    } else if d == ',' && three_digits(&chars, i + 1) {
                        cell.grouped = true;
                        i += 1;
                    } else if d == '.' && !seen_dot {
                        seen_dot = true;
                        digits.push(d);
                        i += 1;
                    } else {
                        break;
                    }
                }
                let mut value: f64 = digits.parse().unwrap_or(0.0);
                match chars.get(i) {
                    Some('%') => {
                        value /= 100.0;
                        i += 1;
                    }
                    Some('e' | 'E') => {
                        let sign = chars.get(i + 1).copied();
                        let exp_digits = chars.get(i + 2).is_some_and(char::is_ascii_digit);
                        if matches!(sign, Some('+' | '-')) && exp_digits {
                            let mut j = i + 2;
                            let mut exp = String::new();
                            while j < chars.len() && chars[j].is_ascii_digit() {
                                exp.push(chars[j]);
                                j += 1;
                            }
                            let exp: i32 = exp.parse().unwrap_or(0);
                            let exp = if sign == Some('-') { -exp } else { exp };
                            value *= 10f64.powi(exp);
                            cell.special = true;
                            i = j;
                        } else {
                            return CellRead::NoNumber;
                        }
                    }
                    Some(l) if l.is_alphabetic() => return CellRead::NoNumber,
                    _ => {}
                }
                cell.values.push(if negative { -value } else { value });
                negative = false;
                after_number = true;
                gap_after_number = false;
            }
            c if c.is_alphabetic() => {
                while i < chars.len() && chars[i].is_alphanumeric() {
                    i += 1;
                }
                cell.special = true;
                after_number = false;
            }
            _ => {
                cell.special = true;
                after_number = false;
                i += 1;
            }
        }
    }
    if cell.values.is_empty() {
        CellRead::NoNumber
    } else {
        CellRead::Numbers(cell)
    }
}

/// The numbers a direction (`ABOVE`, `LEFT`, ...) collects from `cells`, nearest first. Leading
/// cells without a number are passed over; after the first number, a blank or text cell ends the
/// walk, and so does a "special" cell once a bare number was counted.
pub(crate) fn walk(cells: &[&str]) -> (Vec<f64>, TableStyle) {
    let mut out = Vec::new();
    let mut style = TableStyle::default();
    let mut found = false;
    let mut bare = false;
    for text in cells {
        match read_cell(text) {
            CellRead::Blank | CellRead::NoNumber => {
                if found {
                    break;
                }
            }
            CellRead::Numbers(cell) => {
                if cell.special && bare {
                    break;
                }
                found = true;
                bare |= !cell.special;
                style.grouped |= cell.grouped;
                style.parens |= cell.parens;
                out.extend(cell.values);
            }
        }
    }
    (out, style)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sum(cells: &[&str]) -> (f64, TableStyle) {
        let (values, style) = walk(cells);
        (values.iter().sum(), style)
    }

    /// Values measured with Word 16 (`fixtures/reference/docx/fields-tables`).
    #[test]
    fn cells_follow_word() {
        assert_eq!(sum(&["-2", "5", "1,000"]).0, 1003.0);
        assert!(sum(&["1,000"]).1.grouped);
        assert_eq!(sum(&["1", "12,34"]).0, 47.0);
        assert_eq!(sum(&["1e2"]).0, 0.0);
        assert_eq!(sum(&["2", "1e2"]).0, 2.0);
        assert_eq!(sum(&["1e2", "3"]).0, 3.0);
        assert_eq!(sum(&["2", "(5)"]).0, 2.0);
        assert_eq!(sum(&["(5)", "2"]).0, -3.0);
        assert!(sum(&["(5)", "2"]).1.parens);
        assert_eq!(sum(&["1e+2"]).0, 100.0);
        assert_eq!(sum(&["-(5)"]).0, 5.0);
        assert_eq!(sum(&["+5"]).0, 0.0);
        assert_eq!(sum(&["2 apples"]).0, 2.0);
        assert_eq!(sum(&["3", "2 apples"]).0, 3.0);
        assert_eq!(sum(&["1.5.5"]).0, 2.0);
        assert_eq!(sum(&["50 %", "5"]).0, 5.5);
    }

    #[test]
    fn style_writes_separators_and_brackets() {
        let both = TableStyle {
            grouped: true,
            parens: true,
        };
        assert_eq!(both.apply("-1005"), "(1,005)");
        assert_eq!(both.apply("1234567.5"), "1,234,567.5");
        assert_eq!(TableStyle::default().apply("-12"), "-12");
    }
}
