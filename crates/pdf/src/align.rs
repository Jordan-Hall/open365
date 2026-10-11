//! Maps positions in the text the DOCX model reads for a paragraph (cached field results, no
//! field codes) onto the text the layout drew (a field may show a value Word computed, such as a
//! page number). A hyperlink or language range found in the model is carried over, so a link
//! whose display text a field replaced is still found where a text search would miss it.

use std::ops::Range;

/// Longest middle part (after the common prefix and suffix) that is aligned exactly; beyond it
/// the middle is treated as one replaced region.
const MAX_CELLS: usize = 4_000_000;

/// For each char boundary `0..=model.chars()` of `model`: how it lands in `shown`.
pub struct Alignment {
    /// Char index in `shown` of each model char, or `None` when the layout shows something else.
    matched: Vec<Option<usize>>,
    /// Number of chars of `shown`.
    shown_len: usize,
    /// Byte offset of each char of `shown`, plus the total length.
    shown_bytes: Vec<usize>,
    model_bytes: Vec<usize>,
}

impl Alignment {
    pub fn new(model: &str, shown: &str) -> Self {
        let m: Vec<char> = model.chars().collect();
        let s: Vec<char> = shown.chars().collect();
        let mut matched: Vec<Option<usize>> = vec![None; m.len()];
        let mut prefix = 0;
        while prefix < m.len() && prefix < s.len() && m[prefix] == s[prefix] {
            matched[prefix] = Some(prefix);
            prefix += 1;
        }
        let mut suffix = 0;
        while suffix < m.len() - prefix
            && suffix < s.len() - prefix
            && m[m.len() - 1 - suffix] == s[s.len() - 1 - suffix]
        {
            matched[m.len() - 1 - suffix] = Some(s.len() - 1 - suffix);
            suffix += 1;
        }
        let (mm, ss) = (&m[prefix..m.len() - suffix], &s[prefix..s.len() - suffix]);
        if !mm.is_empty() && !ss.is_empty() && mm.len().saturating_mul(ss.len()) <= MAX_CELLS {
            // Longest common subsequence of the middle.
            let w = ss.len() + 1;
            let mut table = vec![0u32; (mm.len() + 1) * w];
            for i in (0..mm.len()).rev() {
                for j in (0..ss.len()).rev() {
                    table[i * w + j] = if mm[i] == ss[j] {
                        table[(i + 1) * w + j + 1] + 1
                    } else {
                        table[(i + 1) * w + j].max(table[i * w + j + 1])
                    };
                }
            }
            let (mut i, mut j) = (0, 0);
            while i < mm.len() && j < ss.len() {
                if mm[i] == ss[j] {
                    matched[prefix + i] = Some(prefix + j);
                    i += 1;
                    j += 1;
                } else if table[(i + 1) * w + j] >= table[i * w + j + 1] {
                    i += 1;
                } else {
                    j += 1;
                }
            }
        }
        Self {
            matched,
            shown_len: s.len(),
            shown_bytes: byte_offsets(shown),
            model_bytes: byte_offsets(model),
        }
    }

    fn model_char(&self, byte: usize) -> usize {
        self.model_bytes.partition_point(|b| *b < byte)
    }

    /// The byte range of `shown` that stands for bytes `range` of the model text: the matched
    /// characters, or the whole gap when the layout shows something else in their place.
    pub fn map(&self, range: Range<usize>) -> Option<Range<usize>> {
        let (a, b) = (self.model_char(range.start), self.model_char(range.end));
        if a >= b || b > self.matched.len() {
            return None;
        }
        let prev_end = |at: usize| {
            self.matched[..at]
                .iter()
                .rev()
                .find_map(|m| *m)
                .map_or(0, |j| j + 1)
        };
        let next_start = |at: usize| {
            self.matched[at..]
                .iter()
                .find_map(|m| *m)
                .unwrap_or(self.shown_len)
        };
        let start = self.matched[a].unwrap_or_else(|| prev_end(a));
        let end = self.matched[b - 1].map_or_else(|| next_start(b), |j| j + 1);
        (start < end).then(|| self.shown_bytes[start]..self.shown_bytes[end])
    }
}

fn byte_offsets(s: &str) -> Vec<usize> {
    s.char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(s.len()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_text_maps_to_itself() {
        let a = Alignment::new("See the link now", "See the link now");
        assert_eq!(a.map(4..12), Some(4..12));
    }

    #[test]
    fn a_field_result_that_changed_is_covered_as_a_whole() {
        // The model holds the cached "9", the layout drew the computed "12".
        let a = Alignment::new("Page 9 of doc", "Page 12 of doc");
        assert_eq!(a.map(5..6), Some(5..7));
        // A link around "9 of" still ends where its text ends.
        assert_eq!(a.map(5..10), Some(5..11));
    }

    #[test]
    fn text_before_a_replacement_does_not_shift_later_ranges() {
        let a = Alignment::new("1 x link", "123 x link");
        assert_eq!(a.map(4..8), Some(6..10));
    }

    #[test]
    fn multibyte_text_maps_by_character() {
        let a = Alignment::new("é link", "é link");
        assert_eq!(a.map(3..7), Some(3..7));
    }
}
