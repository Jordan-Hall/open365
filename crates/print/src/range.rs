//! Page-range text as the Print page's "Pages" box takes it: `1-3, 5, 8-`.
//!
//! Separators are a comma or a semicolon; blanks are ignored. `a-b` is a range, `a-` runs to the
//! last page. Pages print once each, in ascending order, however the text lists them.

use std::fmt;

/// Why the text is not a page range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangeError {
    /// Nothing was typed.
    Empty,
    /// A piece is not a number or a range (the text of the piece).
    Syntax(String),
    /// A range runs backwards (`5-2`).
    Backwards(usize, usize),
    /// Page 0, or a page past the last one.
    OutOfRange { page: usize, total: usize },
}

impl fmt::Display for RangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "Enter the pages to print, for example 1-3, 5."),
            Self::Syntax(s) => write!(f, "\"{s}\" is not a page or a page range."),
            Self::Backwards(a, b) => write!(f, "{a}-{b} counts down; write it as {b}-{a}."),
            Self::OutOfRange { page, total } => {
                write!(f, "Page {page} is outside the document (1 to {total}).")
            }
        }
    }
}

impl std::error::Error for RangeError {}

fn number(s: &str) -> Option<usize> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// The pages `text` names, 1-based, ascending and without repeats.
///
/// # Errors
/// [`RangeError`] when the text is empty, malformed, backwards, or names a page that does not
/// exist among `total` pages.
pub fn parse_page_ranges(text: &str, total: usize) -> Result<Vec<usize>, RangeError> {
    let mut pages = vec![false; total + 1];
    let mut any = false;
    for piece in text.split([',', ';']) {
        let piece: String = piece.chars().filter(|c| !c.is_whitespace()).collect();
        if piece.is_empty() {
            continue;
        }
        any = true;
        let check = |p: usize| {
            if p == 0 || p > total {
                Err(RangeError::OutOfRange { page: p, total })
            } else {
                Ok(p)
            }
        };
        let (a, b) = match piece.split_once('-') {
            None => {
                let p = check(number(&piece).ok_or_else(|| RangeError::Syntax(piece.clone()))?)?;
                (p, p)
            }
            Some((a, b)) => {
                let a = check(number(a).ok_or_else(|| RangeError::Syntax(piece.clone()))?)?;
                let b = if b.is_empty() {
                    total
                } else {
                    check(number(b).ok_or_else(|| RangeError::Syntax(piece.clone()))?)?
                };
                if b < a {
                    return Err(RangeError::Backwards(a, b));
                }
                (a, b)
            }
        };
        pages[a..=b].fill(true);
    }
    if !any {
        return Err(RangeError::Empty);
    }
    Ok((1..=total).filter(|p| pages[*p]).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_ranges_and_open_ends() {
        assert_eq!(parse_page_ranges("1-3, 5", 9), Ok(vec![1, 2, 3, 5]));
        assert_eq!(parse_page_ranges("8-", 10), Ok(vec![8, 9, 10]));
        assert_eq!(parse_page_ranges(" 2 ;1;2 ", 3), Ok(vec![1, 2]));
        assert_eq!(parse_page_ranges("5,1", 5), Ok(vec![1, 5]));
    }

    #[test]
    fn rejects_malformed_ranges() {
        assert_eq!(parse_page_ranges("", 3), Err(RangeError::Empty));
        assert_eq!(parse_page_ranges(" , ", 3), Err(RangeError::Empty));
        assert!(matches!(
            parse_page_ranges("a", 3),
            Err(RangeError::Syntax(_))
        ));
        assert!(matches!(
            parse_page_ranges("1--2", 3),
            Err(RangeError::Syntax(_))
        ));
        assert_eq!(
            parse_page_ranges("3-1", 3),
            Err(RangeError::Backwards(3, 1))
        );
        assert_eq!(
            parse_page_ranges("4", 3),
            Err(RangeError::OutOfRange { page: 4, total: 3 })
        );
        assert_eq!(
            parse_page_ranges("0", 3),
            Err(RangeError::OutOfRange { page: 0, total: 3 })
        );
    }
}
