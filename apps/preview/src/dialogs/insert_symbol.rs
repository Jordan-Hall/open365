//! Symbol (Insert > Symbol): the subsets the dialog lists and the parsing of a chosen symbol.

/// A subset of the Symbol dialog: name and first and last code point.
pub const SUBSETS: &[(&str, u32, u32)] = &[
    ("Basic Latin", 0x20, 0x7E),
    ("Latin-1 Supplement", 0xA0, 0xFF),
    ("Latin Extended-A", 0x100, 0x17F),
    ("Greek and Coptic", 0x370, 0x3FF),
    ("Cyrillic", 0x400, 0x4FF),
    ("General Punctuation", 0x2000, 0x206F),
    ("Currency Symbols", 0x20A0, 0x20CF),
    ("Letterlike Symbols", 0x2100, 0x214F),
    ("Number Forms", 0x2150, 0x218F),
    ("Arrows", 0x2190, 0x21FF),
    ("Mathematical Operators", 0x2200, 0x22FF),
    ("Box Drawing", 0x2500, 0x257F),
    ("Geometric Shapes", 0x25A0, 0x25FF),
    ("Miscellaneous Symbols", 0x2600, 0x26FF),
];

/// The characters of a subset.
#[must_use]
pub fn subset_chars(name: &str) -> Vec<char> {
    SUBSETS
        .iter()
        .find(|s| s.0.eq_ignore_ascii_case(name))
        .map(|s| (s.1..=s.2).filter_map(char::from_u32).collect())
        .unwrap_or_default()
}

/// The text of a symbol argument: `U+00A9` or `0xA9` (a code point), or the characters themselves.
#[must_use]
pub fn parse_symbol(text: &str) -> Option<String> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let hex = t
        .strip_prefix("U+")
        .or_else(|| t.strip_prefix("u+"))
        .or_else(|| t.strip_prefix("0x"));
    if let Some(h) = hex {
        return u32::from_str_radix(h, 16)
            .ok()
            .and_then(char::from_u32)
            .map(String::from);
    }
    Some(t.to_owned())
}
