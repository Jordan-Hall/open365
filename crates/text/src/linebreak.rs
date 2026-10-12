/// Kind of a line-break opportunity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakKind {
    /// A line may end here (UAX #14 allowed break).
    Allowed,
    /// A line must end here (UAX #14 mandatory break after a newline).
    Mandatory,
}

/// A position in the text (byte offset) before which a line may or must end: the text
/// before `offset` belongs to the line that ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BreakOpportunity {
    pub offset: usize,
    pub kind: BreakKind,
}

/// Line-break opportunities of `text` per Unicode UAX #14. The end of the text is always
/// reported (as `Mandatory`), matching the crate's convention.
pub fn break_opportunities(text: &str) -> Vec<BreakOpportunity> {
    unicode_linebreak::linebreaks(text)
        .filter(|&(offset, kind)| {
            !(kind == unicode_linebreak::BreakOpportunity::Allowed
                && inside_slash_word(text, offset))
        })
        .map(|(offset, kind)| BreakOpportunity {
            offset,
            kind: match kind {
                unicode_linebreak::BreakOpportunity::Mandatory => BreakKind::Mandatory,
                unicode_linebreak::BreakOpportunity::Allowed => BreakKind::Allowed,
            },
        })
        .collect()
}

/// Word keeps `student/athlete` together: no line ends after a slash that stands between two
/// letters (measured on the template corpus, `f5b49e76`).
fn inside_slash_word(text: &str, offset: usize) -> bool {
    let before = &text[..offset];
    let mut back = before.chars().rev();
    if back.next() != Some('/') {
        return false;
    }
    back.next().is_some_and(char::is_alphabetic)
        && text[offset..]
            .chars()
            .next()
            .is_some_and(char::is_alphabetic)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breaks_after_spaces_and_at_end() {
        let b = break_opportunities("ab cd");
        assert_eq!(
            b,
            vec![
                BreakOpportunity {
                    offset: 3,
                    kind: BreakKind::Allowed
                },
                BreakOpportunity {
                    offset: 5,
                    kind: BreakKind::Mandatory
                },
            ]
        );
    }

    #[test]
    fn newline_is_mandatory() {
        let b = break_opportunities("a\nb");
        assert_eq!(
            b[0],
            BreakOpportunity {
                offset: 2,
                kind: BreakKind::Mandatory
            }
        );
    }
}

#[cfg(test)]
mod slash_tests {
    use super::*;

    #[test]
    fn no_break_after_a_slash_between_letters() {
        let b = break_opportunities("student/athlete of");
        assert!(b.iter().all(|o| o.offset != "student/".len()));
        // Still allowed after a space, and after a slash that is not between letters.
        assert!(b.iter().any(|o| o.offset == "student/athlete ".len()));
        let c = break_opportunities("a/ b 1/2x");
        assert!(c.iter().any(|o| o.offset == "a/ ".len()));
    }
}
