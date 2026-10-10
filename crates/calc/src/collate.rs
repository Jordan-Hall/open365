//! Text collation for comparison operators, lookups and criteria.
//!
//! Excel compares text case-insensitively with the system's linguistic sort, not by code point. The
//! behaviour reproduced here was measured against Excel (en-US/en-GB sort order):
//!
//! * hyphen and apostrophe are ignored at the first level (`"a-b"` sorts with `"ab"`, then after it);
//! * ASCII punctuation and space sort before digits, digits before letters, in ASCII order among themselves;
//! * Latin letters compare by their unaccented base letter; an accent only breaks a tie
//!   (`"e" < "é" < "f"`); `ß` and `æ` compare as `ss` and `ae`;
//! * other scripts follow Latin in code point order, then symbols and emoji, then CJK.
//!
//! This approximates the Unicode collation algorithm, which is not implemented.

use std::cmp::Ordering;

/// Lower-case Latin letters with diacritics (and a few ligatures) mapped to their base letters. Sorted by
/// character so it can be binary searched.
#[rustfmt::skip]
const LATIN_FOLD: &[(char, &str)] = &[
    ('ß', "ss"), ('à', "a"), ('á', "a"), ('â', "a"), ('ã', "a"), ('ä', "a"),
    ('å', "a"), ('æ', "ae"), ('ç', "c"), ('è', "e"), ('é', "e"), ('ê', "e"),
    ('ë', "e"), ('ì', "i"), ('í', "i"), ('î', "i"), ('ï', "i"), ('ð', "d"),
    ('ñ', "n"), ('ò', "o"), ('ó', "o"), ('ô', "o"), ('õ', "o"), ('ö', "o"),
    ('ø', "o"), ('ù', "u"), ('ú', "u"), ('û', "u"), ('ü', "u"), ('ý', "y"),
    ('þ', "th"), ('ÿ', "y"), ('ā', "a"), ('ă', "a"), ('ą', "a"), ('ć', "c"),
    ('ĉ', "c"), ('ċ', "c"), ('č', "c"), ('ď', "d"), ('đ', "d"), ('ē', "e"),
    ('ĕ', "e"), ('ė', "e"), ('ę', "e"), ('ě', "e"), ('ĝ', "g"), ('ğ', "g"),
    ('ġ', "g"), ('ģ', "g"), ('ĥ', "h"), ('ħ', "h"), ('ĩ', "i"), ('ī', "i"),
    ('ĭ', "i"), ('į', "i"), ('ı', "i"), ('ĵ', "j"), ('ķ', "k"), ('ĸ', "k"),
    ('ĺ', "l"), ('ļ', "l"), ('ľ', "l"), ('ł', "l"), ('ń', "n"), ('ņ', "n"),
    ('ň', "n"), ('ŋ', "n"), ('ō', "o"), ('ŏ', "o"), ('ő', "o"), ('œ', "oe"),
    ('ŕ', "r"), ('ŗ', "r"), ('ř', "r"), ('ś', "s"), ('ŝ', "s"), ('ş', "s"),
    ('š', "s"), ('ţ', "t"), ('ť', "t"), ('ŧ', "t"), ('ũ', "u"), ('ū', "u"),
    ('ŭ', "u"), ('ů', "u"), ('ű', "u"), ('ų', "u"), ('ŵ', "w"), ('ŷ', "y"),
    ('ź', "z"), ('ż', "z"), ('ž', "z"), ('ƒ', "f"), ('ơ', "o"), ('ư', "u"),
    ('ǎ', "a"), ('ǐ', "i"), ('ǒ', "o"), ('ǔ', "u"), ('ǖ', "u"), ('ǘ', "u"),
    ('ǚ', "u"), ('ǜ', "u"), ('ǟ', "a"), ('ǡ', "a"), ('ǧ', "g"), ('ǩ', "k"),
    ('ǫ', "o"), ('ǭ', "o"), ('ǰ', "j"), ('ǵ', "g"), ('ǹ', "n"), ('ǻ', "a"),
    ('ȁ', "a"), ('ȃ', "a"), ('ȅ', "e"), ('ȇ', "e"), ('ȉ', "i"), ('ȋ', "i"),
    ('ȍ', "o"), ('ȏ', "o"), ('ȑ', "r"), ('ȓ', "r"), ('ȕ', "u"), ('ȗ', "u"),
    ('ș', "s"), ('ț', "t"), ('ȟ', "h"), ('ȧ', "a"), ('ȩ', "e"), ('ȫ', "o"),
    ('ȭ', "o"), ('ȯ', "o"), ('ȱ', "o"), ('ȳ', "y"),
];

/// Characters that carry no weight at the first level.
fn ignorable(c: char) -> bool {
    matches!(c, '-' | '\'' | '\u{ad}')
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0x20000..=0x3FFFF)
}

/// First-level weight of one lower-case character.
fn primary(c: char) -> u32 {
    match c {
        'a'..='z' => 0x200 + (c as u32 - 'a' as u32),
        '0'..='9' => 0x100 + (c as u32 - '0' as u32),
        c if c.is_ascii() => c as u32,
        c if c.is_whitespace() => 0x21,
        c if is_cjk(c) => 0x40_0000 + c as u32,
        c if c.is_alphabetic() => 0x1_0000 + c as u32,
        c => 0x20_0000 + c as u32,
    }
}

/// Weights of a string: `(first level, accent flag)` per character, plus the positions of ignorable
/// characters.
/// Per-character `(first level weight, accent flag)` and the positions of ignorable characters.
type Weights = (Vec<(u32, u8)>, Vec<(usize, char)>);

fn weights(s: &str) -> Weights {
    let mut out = Vec::with_capacity(s.len());
    let mut skipped = Vec::new();
    for (i, c) in s.chars().enumerate() {
        let c = c.to_lowercase().next().unwrap_or(c);
        if ignorable(c) {
            skipped.push((i, c));
            continue;
        }
        match LATIN_FOLD.binary_search_by_key(&c, |(k, _)| *k) {
            Ok(idx) => {
                let base = LATIN_FOLD[idx].1;
                let accent = u8::from(base.len() == 1);
                out.extend(base.chars().map(|b| (primary(b), accent)));
            }
            Err(_) => out.push((primary(c), 0)),
        }
    }
    (out, skipped)
}

/// Compares two strings the way Excel's comparison operators do (case-insensitive, linguistic order).
pub fn compare_text(a: &str, b: &str) -> Ordering {
    let plain = |s: &str| s.is_ascii() && !s.contains(['-', '\'']);
    if plain(a) && plain(b) {
        return a
            .bytes()
            .map(|c| primary(char::from(c.to_ascii_lowercase())))
            .cmp(
                b.bytes()
                    .map(|c| primary(char::from(c.to_ascii_lowercase()))),
            );
    }
    let (wa, sa) = weights(a);
    let (wb, sb) = weights(b);
    wa.iter()
        .map(|w| w.0)
        .cmp(wb.iter().map(|w| w.0))
        .then_with(|| wa.iter().map(|w| w.1).cmp(wb.iter().map(|w| w.1)))
        .then_with(|| sa.len().cmp(&sb.len()))
        .then_with(|| sa.cmp(&sb))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lt(a: &str, b: &str) -> bool {
        compare_text(a, b) == Ordering::Less
    }

    #[test]
    fn measured_against_excel() {
        assert!(lt("a", "b"));
        assert_eq!(compare_text("ABC", "abc"), Ordering::Equal);
        assert!(lt("e", "é"));
        assert!(lt("é", "f"));
        assert!(!lt("é", "e"));
        assert!(lt("_", "1"));
        assert!(lt("~", "a"));
        assert!(lt("9", "a"));
        assert!(lt("-", "_"));
        assert!(!lt("~", "|"));
        assert!(lt("a b", "ab"));
        assert!(lt("a_b", "aa"));
        assert!(lt("ab", "a-c"));
        assert!(!lt("a-c", "ab"));
        assert!(!lt("a-b", "ab"));
        assert!(lt("ab", "a-b"));
        assert!(lt("ß", "st"));
        assert_eq!(compare_text("ss", "ß"), Ordering::Equal);
        assert!(lt("a", "α"));
        assert!(lt("α", "中"));
        assert!(lt("😀", "中"));
        assert!(!lt("😀", "a"));
    }
}
