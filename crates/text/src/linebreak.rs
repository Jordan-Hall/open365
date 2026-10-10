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
        .map(|(offset, kind)| BreakOpportunity {
            offset,
            kind: match kind {
                unicode_linebreak::BreakOpportunity::Mandatory => BreakKind::Mandatory,
                unicode_linebreak::BreakOpportunity::Allowed => BreakKind::Allowed,
            },
        })
        .collect()
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
