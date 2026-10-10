//! List labels: counts the paragraphs of a document in order and computes the label of every
//! numbered one. The definitions (abstract numbers, instances, levels, style links and the
//! resolution of a paragraph's `w:numPr`) come from [`open365_docx::Numbering`].
//!
//! Rules (checked against Word with the `layout-lists-*` fixtures and
//! `fixtures/reference/layout/*.lists.json`):
//!
//! * Counters belong to the abstract numbering, so two `w:num` that share one continue the same
//!   sequence unless the second carries a `w:startOverride` for the level.
//! * A paragraph at level L adds one to its level's counter and restarts the deeper levels
//!   (honouring `w:lvlRestart`). A level that has not been used shows its start value.
//! * Letters repeat after `z` (`aa`, `bb`), roman numerals stop at 3999.

use std::collections::{HashMap, HashSet};

use open365_docx::{
    Document, IndentOverride, LEVELS, LevelJustification, LevelRun, LevelSuffix, NumLevel,
    Paragraph, ResolvedNum, Styles,
};

pub(crate) type Suffix = LevelSuffix;
pub(crate) type LabelAlign = LevelJustification;
pub(crate) type LabelRun = LevelRun;

/// A paragraph's computed list label.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ListLabel {
    pub text: String,
    pub suffix: Suffix,
    pub align: LabelAlign,
    /// Indents of the numbering level; direct paragraph formatting still wins.
    pub indent: IndentOverride,
    pub run: LabelRun,
    /// True for a bullet: the text is drawn as is.
    pub bullet: bool,
}

/// Counters while the paragraphs are walked in document order.
#[derive(Debug, Default)]
pub(crate) struct NumberingState {
    counters: HashMap<i64, [Option<i64>; LEVELS]>,
    starts: HashMap<i64, [Option<i64>; LEVELS]>,
    seen_nums: HashSet<i64>,
}

/// Labels of a document's numbered paragraphs.
pub(crate) struct Labeler<'a> {
    numbering: &'a open365_docx::Numbering,
    styles: Option<&'a Styles>,
}

impl<'a> Labeler<'a> {
    /// `None` when the document has no numbering part.
    pub(crate) fn new(doc: &'a Document) -> Option<Self> {
        Some(Self {
            numbering: doc.numbering()?,
            styles: doc.styles(),
        })
    }

    /// Label of `para`, advancing the counters; `None` for an unnumbered paragraph.
    pub(crate) fn label(&self, state: &mut NumberingState, para: &Paragraph) -> Option<ListLabel> {
        let at = self.numbering.paragraph_numbering(self.styles, para)?;
        let (num_id, ilvl) = (at.num_id, at.ilvl);
        let resolved: ResolvedNum<'_> = self.numbering.resolve(self.styles, num_id)?;
        let level: &NumLevel = resolved.level(ilvl)?;
        let abstract_id = resolved.abstract_id;

        let counters = state.counters.entry(abstract_id).or_default();
        let starts = state.starts.entry(abstract_id).or_default();
        if state.seen_nums.insert(num_id) {
            // The first paragraph of an instance applies its start overrides.
            for o in &resolved.num.overrides {
                if let Some(s) = o.start_override {
                    counters[o.ilvl..].fill(None);
                    starts[o.ilvl] = Some(s);
                }
            }
        }
        let start_of = |l: usize, starts: &[Option<i64>; LEVELS]| {
            starts[l].unwrap_or_else(|| resolved.level(l).map_or(1, |lv| lv.start))
        };
        counters[ilvl] = Some(match counters[ilvl] {
            Some(v) => v + 1,
            None => start_of(ilvl, starts),
        });
        for d in ilvl + 1..LEVELS {
            let resets = match resolved.level(d).and_then(|l| l.restart) {
                None => true,
                Some(0) => false,
                Some(k) => ilvl < k,
            };
            if resets {
                counters[d] = None;
                // A restarted level starts from the level's own start from now on.
                starts[d] = None;
            }
        }

        let bullet = level.is_bullet();
        let text = if bullet {
            level.text.clone()
        } else {
            let value = |l: usize| counters[l].unwrap_or_else(|| start_of(l, starts));
            let mut out = String::new();
            let mut chars = level.text.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '%'
                    && let Some(d) = chars.peek().and_then(|d| d.to_digit(10))
                    && (1..=LEVELS as u32).contains(&d)
                {
                    chars.next();
                    let l = d as usize - 1;
                    let fmt = if level.is_legal && l != ilvl {
                        "decimal"
                    } else {
                        resolved
                            .level(l)
                            .map_or("decimal", |lv| lv.num_fmt.as_str())
                    };
                    out.push_str(&format_number(value(l), fmt));
                } else {
                    out.push(c);
                }
            }
            out
        };
        Some(ListLabel {
            text,
            suffix: level.suffix,
            align: level.justification,
            indent: level.indent,
            run: level.run.clone(),
            bullet,
        })
    }
}

fn roman(mut n: i64, upper: bool) -> Option<String> {
    if !(1..4000).contains(&n) {
        return None;
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
    let mut out = String::new();
    for (v, s) in TABLE {
        while n >= v {
            out.push_str(s);
            n -= v;
        }
    }
    Some(if upper { out } else { out.to_lowercase() })
}

fn letters(n: i64, upper: bool) -> Option<String> {
    if n < 1 {
        return None;
    }
    let base = if upper { b'A' } else { b'a' };
    let c = char::from(base + ((n - 1) % 26) as u8);
    Some(std::iter::repeat_n(c, ((n - 1) / 26 + 1) as usize).collect())
}

fn ordinal_suffix(n: i64) -> &'static str {
    match (n % 100, n % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    }
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
const ORDINAL_ONES: [&str; 20] = [
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

/// English words for 0..=99, cardinal or ordinal.
fn words(n: i64, ordinal: bool) -> Option<String> {
    if !(0..100).contains(&n) {
        return None;
    }
    let n = n as usize;
    let s = if n < 20 {
        (if ordinal { ORDINAL_ONES[n] } else { ONES[n] }).to_owned()
    } else {
        let (t, o) = (n / 10, n % 10);
        match (o, ordinal) {
            (0, false) => TENS[t].to_owned(),
            (0, true) => format!("{}ieth", TENS[t].trim_end_matches('y')),
            (_, false) => format!("{}-{}", TENS[t], ONES[o]),
            (_, true) => format!("{}-{}", TENS[t], ORDINAL_ONES[o]),
        }
    };
    let mut chars = s.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
}

/// One counter value in a `w:numFmt`.
pub(crate) fn format_number(n: i64, fmt: &str) -> String {
    let formatted = match fmt {
        "upperRoman" => roman(n, true),
        "lowerRoman" => roman(n, false),
        "upperLetter" => letters(n, true),
        "lowerLetter" => letters(n, false),
        "decimalZero" => Some(format!("{n:02}")),
        "ordinal" => Some(format!("{n}{}", ordinal_suffix(n))),
        "cardinalText" => words(n, false),
        "ordinalText" => words(n, true),
        "none" => Some(String::new()),
        _ => None,
    };
    formatted.unwrap_or_else(|| n.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_numbers_like_word() {
        assert_eq!(format_number(27, "lowerLetter"), "aa");
        assert_eq!(format_number(28, "lowerLetter"), "bb");
        assert_eq!(format_number(38, "upperRoman"), "XXXVIII");
        assert_eq!(format_number(4, "lowerRoman"), "iv");
        assert_eq!(format_number(3, "decimalZero"), "03");
        assert_eq!(format_number(12, "ordinal"), "12th");
        assert_eq!(format_number(22, "ordinal"), "22nd");
        assert_eq!(format_number(7, "unknownFormat"), "7");
    }
}
