//! Character properties for the PCRE2-compatible regular expression engine behind `REGEXTEST`, `REGEXEXTRACT`
//! and `REGEXREPLACE` (UTF and UCP mode, as Excel runs it): general categories, the derived classes
//! (`\w`, `\d`, `\s`, `\h`, `\v`, POSIX names) and simple case folding.
//!
//! Script properties (`\p{Greek}`) cover the common scripts with range tables that follow the Unicode script
//! blocks closely but are not the full Scripts.txt data; see `fixtures/calc/known-differences.toml`.

use unicode_general_category::get_general_category;

/// A character property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Prop {
    Any,
    /// A general category: one letter (`L`, minor 0) or two (`Lu`).
    Cat(u8, u8),
    /// `L&`: Lu, Ll or Lt.
    Lc,
    /// Letters and numbers.
    Xan,
    /// Space: Z, `\h` or `\v`.
    Space,
    /// Word: letters, numbers, `_`, Mn and Pc.
    Word,
    Digit,
    HSpace,
    VSpace,
    Punct,
    Graph,
    Print,
    Xdigit,
    Ascii,
    Script(&'static [(u32, u32)]),
}

fn category(c: char) -> (u8, u8) {
    let a = get_general_category(c).abbreviation().as_bytes();
    (a[0], a[1])
}

pub(super) fn is_hspace(c: char) -> bool {
    matches!(
        c,
        '\t' | ' ' | '\u{a0}' | '\u{1680}' | '\u{180e}' | '\u{2000}'
            ..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}'
    )
}

pub(super) fn is_vspace(c: char) -> bool {
    matches!(c, '\n'..='\r' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

pub(super) fn is_word(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_alphanumeric() || c == '_';
    }
    let (a, b) = category(c);
    matches!(a, b'L' | b'N') || (a == b'M' && b == b'n') || (a == b'P' && b == b'c')
}

fn is_space(c: char) -> bool {
    is_hspace(c) || is_vspace(c) || category(c).0 == b'Z'
}

fn in_ranges(table: &[(u32, u32)], c: char) -> bool {
    let u = c as u32;
    table.iter().any(|&(lo, hi)| (lo..=hi).contains(&u))
}

impl Prop {
    pub(super) fn matches(self, c: char) -> bool {
        match self {
            Prop::Any => true,
            Prop::Cat(a, b) => {
                let (x, y) = category(c);
                x == a && (b == 0 || y == b)
            }
            Prop::Lc => matches!(category(c), (b'L', b'u' | b'l' | b't')),
            Prop::Xan => matches!(category(c).0, b'L' | b'N'),
            Prop::Space => is_space(c),
            Prop::Word => is_word(c),
            Prop::Digit => category(c) == (b'N', b'd'),
            Prop::HSpace => is_hspace(c),
            Prop::VSpace => is_vspace(c),
            Prop::Punct => {
                let (a, _) = category(c);
                a == b'P' || (a == b'S' && (c as u32) < 256)
            }
            Prop::Graph => {
                let (a, b) = category(c);
                !(a == b'Z' || (a == b'C' && matches!(b, b'c' | b's' | b'n')))
            }
            Prop::Print => {
                let (a, b) = category(c);
                !((a == b'Z' && b != b's') || (a == b'C' && matches!(b, b'c' | b's' | b'n')))
            }
            Prop::Xdigit => c.is_ascii_hexdigit(),
            Prop::Ascii => c.is_ascii(),
            Prop::Script(t) => in_ranges(t, c),
        }
    }
}

const LATIN: &[(u32, u32)] = &[
    (0x41, 0x5a),
    (0x61, 0x7a),
    (0xaa, 0xaa),
    (0xba, 0xba),
    (0xc0, 0xd6),
    (0xd8, 0xf6),
    (0xf8, 0x2b8),
    (0x2e0, 0x2e4),
    (0x1d00, 0x1d25),
    (0x1d2c, 0x1d5c),
    (0x1d62, 0x1d65),
    (0x1d6b, 0x1d77),
    (0x1d79, 0x1dbe),
    (0x1e00, 0x1eff),
    (0x2071, 0x2071),
    (0x207f, 0x207f),
    (0x2090, 0x209c),
    (0x212a, 0x212b),
    (0x2132, 0x2132),
    (0x214e, 0x214e),
    (0x2160, 0x2188),
    (0x2c60, 0x2c7f),
    (0xa722, 0xa787),
    (0xa78b, 0xa7ff),
    (0xab30, 0xab5a),
    (0xab5c, 0xab64),
    (0xfb00, 0xfb06),
    (0xff21, 0xff3a),
    (0xff41, 0xff5a),
];
const GREEK: &[(u32, u32)] = &[
    (0x370, 0x373),
    (0x375, 0x377),
    (0x37a, 0x37d),
    (0x37f, 0x37f),
    (0x384, 0x384),
    (0x386, 0x386),
    (0x388, 0x3e1),
    (0x3f0, 0x3ff),
    (0x1d26, 0x1d2a),
    (0x1f00, 0x1fff),
    (0x2126, 0x2126),
];
const CYRILLIC: &[(u32, u32)] = &[
    (0x400, 0x484),
    (0x487, 0x52f),
    (0x1c80, 0x1c88),
    (0x1d2b, 0x1d2b),
    (0x1d78, 0x1d78),
    (0x2de0, 0x2dff),
    (0xa640, 0xa69f),
    (0xfe2e, 0xfe2f),
];
const HAN: &[(u32, u32)] = &[
    (0x2e80, 0x2e99),
    (0x2e9b, 0x2ef3),
    (0x2f00, 0x2fd5),
    (0x3005, 0x3005),
    (0x3007, 0x3007),
    (0x3021, 0x3029),
    (0x3038, 0x303b),
    (0x3400, 0x4dbf),
    (0x4e00, 0x9fff),
    (0xf900, 0xfa6d),
    (0xfa70, 0xfad9),
    (0x20000, 0x2a6df),
    (0x2a700, 0x2ebef),
    (0x30000, 0x323af),
];
const HIRAGANA: &[(u32, u32)] = &[
    (0x3041, 0x3096),
    (0x309d, 0x309f),
    (0x1b001, 0x1b11f),
    (0x1f200, 0x1f200),
];
const KATAKANA: &[(u32, u32)] = &[
    (0x30a1, 0x30fa),
    (0x30fd, 0x30ff),
    (0x31f0, 0x31ff),
    (0x32d0, 0x32fe),
    (0x3300, 0x3357),
    (0xff66, 0xff6f),
    (0xff71, 0xff9d),
];
const HANGUL: &[(u32, u32)] = &[
    (0x1100, 0x11ff),
    (0x302e, 0x302f),
    (0x3131, 0x318e),
    (0x3200, 0x321e),
    (0x3260, 0x327e),
    (0xa960, 0xa97c),
    (0xac00, 0xd7a3),
    (0xd7b0, 0xd7fb),
    (0xffa0, 0xffdc),
];
const ARABIC: &[(u32, u32)] = &[
    (0x600, 0x604),
    (0x606, 0x60b),
    (0x60d, 0x61a),
    (0x61c, 0x61e),
    (0x620, 0x63f),
    (0x641, 0x64a),
    (0x656, 0x66f),
    (0x671, 0x6dc),
    (0x6de, 0x6ff),
    (0x750, 0x77f),
    (0x8a0, 0x8ff),
    (0xfb50, 0xfdff),
    (0xfe70, 0xfefc),
];
const HEBREW: &[(u32, u32)] = &[
    (0x591, 0x5c7),
    (0x5d0, 0x5ea),
    (0x5ef, 0x5f4),
    (0xfb1d, 0xfb4f),
];
const THAI: &[(u32, u32)] = &[(0xe01, 0xe3a), (0xe40, 0xe5b)];
const DEVANAGARI: &[(u32, u32)] = &[
    (0x900, 0x950),
    (0x955, 0x963),
    (0x966, 0x97f),
    (0xa8e0, 0xa8ff),
];
const ARMENIAN: &[(u32, u32)] = &[
    (0x531, 0x556),
    (0x559, 0x58a),
    (0x58d, 0x58f),
    (0xfb13, 0xfb17),
];
const GEORGIAN: &[(u32, u32)] = &[(0x10a0, 0x10ff), (0x1c90, 0x1cbf), (0x2d00, 0x2d2d)];

/// The property named in `\p{...}`, matched without case and ignoring spaces, hyphens and underscores.
/// Only the short category names, the PCRE2 extras and the supported scripts exist (`\p{Letter}` does not).
pub(super) fn prop_by_name(name: &str) -> Option<Prop> {
    let key: String = name
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '_'))
        .collect::<String>()
        .to_ascii_lowercase();
    let script = |t| Some(Prop::Script(t));
    match key.as_str() {
        "any" => return Some(Prop::Any),
        "l&" | "lc" => return Some(Prop::Lc),
        "xan" => return Some(Prop::Xan),
        "xsp" | "xps" => return Some(Prop::Space),
        "xwd" => return Some(Prop::Word),
        "latin" => return script(LATIN),
        "greek" => return script(GREEK),
        "cyrillic" => return script(CYRILLIC),
        "han" => return script(HAN),
        "hiragana" => return script(HIRAGANA),
        "katakana" => return script(KATAKANA),
        "hangul" => return script(HANGUL),
        "arabic" => return script(ARABIC),
        "hebrew" => return script(HEBREW),
        "thai" => return script(THAI),
        "devanagari" => return script(DEVANAGARI),
        "armenian" => return script(ARMENIAN),
        "georgian" => return script(GEORGIAN),
        _ => {}
    }
    const MAJORS: &[u8] = b"lmnpszc";
    const MINORS: &[&str] = &[
        "lu", "ll", "lt", "lm", "lo", "mn", "mc", "me", "nd", "nl", "no", "pc", "pd", "ps", "pe",
        "pi", "pf", "po", "sm", "sc", "sk", "so", "zs", "zl", "zp", "cc", "cf", "cs", "co", "cn",
    ];
    let b = key.as_bytes();
    if b.len() == 1 && MAJORS.contains(&b[0]) {
        return Some(Prop::Cat(b[0].to_ascii_uppercase(), 0));
    }
    if MINORS.contains(&key.as_str()) {
        return Some(Prop::Cat(b[0].to_ascii_uppercase(), b[1]));
    }
    None
}

/// A POSIX class name inside `[[:name:]]`.
pub(super) fn posix_class(name: &str) -> Option<Prop> {
    Some(match name {
        "alpha" => Prop::Cat(b'L', 0),
        "lower" => Prop::Cat(b'L', b'l'),
        "upper" => Prop::Cat(b'L', b'u'),
        "alnum" => Prop::Xan,
        "digit" => Prop::Digit,
        "xdigit" => Prop::Xdigit,
        "space" => Prop::Space,
        "blank" => Prop::HSpace,
        "cntrl" => Prop::Cat(b'C', b'c'),
        "punct" => Prop::Punct,
        "graph" => Prop::Graph,
        "print" => Prop::Print,
        "word" => Prop::Word,
        "ascii" => Prop::Ascii,
        _ => return None,
    })
}

fn single<I: Iterator<Item = char>>(mut it: I) -> Option<char> {
    match (it.next(), it.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}

/// Simple case folding, one character to one character (no `ß` to `ss`; Turkish dotted and dotless i stay).
pub(super) fn fold(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_lowercase();
    }
    if c == '\u{130}' || c == '\u{131}' {
        return c;
    }
    let up = single(c.to_uppercase()).unwrap_or(c);
    single(up.to_lowercase()).unwrap_or(up)
}

/// The characters whose membership decides a caseless class test.
pub(super) fn case_variants(c: char) -> [char; 4] {
    [
        c,
        single(c.to_lowercase()).unwrap_or(c),
        single(c.to_uppercase()).unwrap_or(c),
        fold(c),
    ]
}

/// True when `c` has a different case form, so a caseless literal needs the folded comparison.
pub(super) fn has_case(c: char) -> bool {
    case_variants(c).iter().any(|&x| x != c)
}

/// Spacing, enclosing and zero-width-joiner characters that stay with the preceding one in `\X`.
pub(super) fn extends_grapheme(c: char) -> bool {
    matches!(category(c), (b'M', _)) || matches!(c, '\u{200d}' | '\u{fe0f}' | '\u{fe0e}')
}
