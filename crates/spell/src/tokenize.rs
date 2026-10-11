//! Paragraph tokenizer: finds checkable words with byte, `char` and UTF-16 offsets.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use crate::casing::is_all_caps;

/// Which words the tokenizer reports. The defaults match Word's proofing options.
#[derive(Clone, Debug)]
pub struct TokenizerOptions {
    /// Skip words written entirely in capitals, such as `NASA` (Word: "Ignore words in UPPERCASE").
    pub ignore_uppercase: bool,
    /// Skip words that contain digits, such as `mp3` (Word: "Ignore words with numbers").
    pub ignore_words_with_digits: bool,
    /// Skip URLs, email addresses and file paths (Word: "Ignore Internet and file addresses").
    pub ignore_internet_addresses: bool,
    /// Skip words shorter than this many characters.
    pub min_word_chars: usize,
    /// Characters that may join two word parts, for example from a dictionary's `WORDCHARS`.
    pub extra_word_chars: Vec<char>,
}

impl Default for TokenizerOptions {
    fn default() -> Self {
        Self {
            ignore_uppercase: true,
            ignore_words_with_digits: true,
            ignore_internet_addresses: true,
            min_word_chars: 1,
            extra_word_chars: Vec::new(),
        }
    }
}

/// A word found in a paragraph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Word<'a> {
    /// The word text.
    pub text: &'a str,
    /// Offsets in UTF-8 bytes.
    pub byte_range: Range<usize>,
    /// Offsets in Unicode scalar values (`char`s).
    pub char_range: Range<usize>,
    /// Offsets in UTF-16 code units, as used by Word's document model.
    pub utf16_range: Range<usize>,
}

/// A misspelled word with its position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Misspelling {
    /// The misspelled text as written.
    pub word: String,
    /// Offsets in UTF-8 bytes.
    pub byte_range: Range<usize>,
    /// Offsets in Unicode scalar values (`char`s).
    pub char_range: Range<usize>,
    /// Offsets in UTF-16 code units.
    pub utf16_range: Range<usize>,
}

impl From<&Word<'_>> for Misspelling {
    fn from(word: &Word<'_>) -> Self {
        Self {
            word: word.text.to_owned(),
            byte_range: word.byte_range.clone(),
            char_range: word.char_range.clone(),
            utf16_range: word.utf16_range.clone(),
        }
    }
}

/// Finds the words of `text` that should be spell-checked under `options`.
#[must_use]
pub fn find_words<'a>(text: &'a str, options: &TokenizerOptions) -> Vec<Word<'a>> {
    let segments = split_segments(text, options);
    let ignored = if options.ignore_internet_addresses {
        internet_address_ranges(text)
    } else {
        Vec::new()
    };
    let mut offsets = Offsets::new(text);
    let mut words = Vec::new();
    let mut i = 0;
    while i < segments.len() {
        let (start, first) = segments[i];
        if !is_word_segment(first) {
            i += 1;
            continue;
        }
        let mut end = start + first.len();
        i += 1;
        // Join `well-known`, `rock'n'roll` and dictionary `WORDCHARS` between word parts.
        while i + 1 < segments.len()
            && is_joiner(segments[i].1, options)
            && is_word_segment(segments[i + 1].1)
        {
            end = segments[i + 1].0 + segments[i + 1].1.len();
            i += 2;
        }
        let token = &text[start..end];
        if !is_reportable(token, options) {
            continue;
        }
        if ignored.iter().any(|r| r.start <= start && end <= r.end) {
            continue;
        }
        let (char_start, utf16_start) = offsets.at(start);
        let (char_end, utf16_end) = offsets.at(end);
        words.push(Word {
            text: token,
            byte_range: start..end,
            char_range: char_start..char_end,
            utf16_range: utf16_start..utf16_end,
        });
    }
    words
}

/// UAX #29 keeps `a.b` and `a:b` together; split them unless the dictionary uses the character.
fn split_segments<'a>(text: &'a str, options: &TokenizerOptions) -> Vec<(usize, &'a str)> {
    let splits = |c: char| matches!(c, '.' | ':') && !options.extra_word_chars.contains(&c);
    let mut out = Vec::new();
    for (start, segment) in text.split_word_bound_indices() {
        if !segment.contains(splits) {
            out.push((start, segment));
            continue;
        }
        let mut piece_start = 0;
        for (at, c) in segment.char_indices() {
            if splits(c) {
                if at > piece_start {
                    out.push((start + piece_start, &segment[piece_start..at]));
                }
                out.push((start + at, &segment[at..at + c.len_utf8()]));
                piece_start = at + c.len_utf8();
            }
        }
        if piece_start < segment.len() {
            out.push((start + piece_start, &segment[piece_start..]));
        }
    }
    out
}

fn is_word_segment(segment: &str) -> bool {
    segment.chars().next().is_some_and(char::is_alphanumeric)
}

fn is_joiner(segment: &str, options: &TokenizerOptions) -> bool {
    let mut chars = segment.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => {
            matches!(c, '-' | '\u{2010}' | '\u{2011}' | '\'' | '\u{2019}')
                || options.extra_word_chars.contains(&c)
        }
        _ => false,
    }
}

fn is_reportable(token: &str, options: &TokenizerOptions) -> bool {
    if !token.chars().any(char::is_alphabetic) {
        return false;
    }
    if options.ignore_words_with_digits && token.chars().any(char::is_numeric) {
        return false;
    }
    if options.ignore_uppercase && is_all_caps(token) {
        return false;
    }
    if token
        .chars()
        .filter(|c| c.is_alphabetic())
        .all(is_ideographic)
    {
        return false;
    }
    token.chars().count() >= options.min_word_chars
}

/// Han, Hiragana and Katakana have no word-level spelling to check.
fn is_ideographic(c: char) -> bool {
    matches!(u32::from(c),
        0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF
        | 0xFF66..=0xFF9F | 0x20000..=0x2FA1F)
}

/// Converts monotonically increasing byte offsets to `char` and UTF-16 offsets.
struct Offsets<'a> {
    text: &'a str,
    byte: usize,
    chars: usize,
    utf16: usize,
}

impl<'a> Offsets<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            byte: 0,
            chars: 0,
            utf16: 0,
        }
    }

    fn at(&mut self, byte: usize) -> (usize, usize) {
        if byte > self.byte {
            for c in self.text[self.byte..byte].chars() {
                self.chars += 1;
                self.utf16 += c.len_utf16();
            }
            self.byte = byte;
        }
        (self.chars, self.utf16)
    }
}

/// Byte ranges of URLs, email addresses and file paths.
fn internet_address_ranges(text: &str) -> Vec<Range<usize>> {
    const LEADING: &[char] = &[
        '(', '[', '{', '<', '"', '\'', '\u{201C}', '\u{2018}', '\u{AB}',
    ];
    const TRAILING: &[char] = &[
        ')', ']', '}', '>', '.', ',', ';', ':', '!', '?', '"', '\'', '\u{201D}', '\u{2019}',
        '\u{BB}', '\u{2026}',
    ];
    let mut ranges = Vec::new();
    let mut chunk_start = None;
    let mut flush = |start: usize, end: usize| {
        let chunk = &text[start..end];
        let trimmed_start = chunk.trim_start_matches(LEADING);
        let lead = chunk.len() - trimmed_start.len();
        let trimmed = trimmed_start.trim_end_matches(TRAILING);
        if looks_like_address(trimmed) {
            ranges.push(start + lead..start + lead + trimmed.len());
        }
    };
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            if let Some(start) = chunk_start.take() {
                flush(start, i);
            }
        } else if chunk_start.is_none() {
            chunk_start = Some(i);
        }
    }
    if let Some(start) = chunk_start {
        flush(start, text.len());
    }
    ranges
}

fn looks_like_address(chunk: &str) -> bool {
    let lower = chunk.to_lowercase();
    if let Some(at) = lower.find("://")
        && at > 0
        && lower[..at]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return true;
    }
    if lower.starts_with("www.") || lower.starts_with("mailto:") || lower.starts_with("file:") {
        return true;
    }
    if is_email(chunk) {
        return true;
    }
    let bytes = chunk.as_bytes();
    // `C:\dir\file.txt`, `C:/dir`, `\\server\share`, `/usr/lib/x`, `~/notes`.
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
    {
        return true;
    }
    if chunk.starts_with("\\\\") && chunk.len() > 2 {
        return true;
    }
    (chunk.starts_with('/') && chunk[1..].contains('/') && chunk.len() > 3)
        || chunk.starts_with("~/")
}

fn is_email(chunk: &str) -> bool {
    let Some((local, domain)) = chunk.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && local
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-'))
        && domain.contains('.')
        && !domain.contains('@')
        && domain.split('.').all(|label| {
            !label.is_empty() && label.chars().all(|c| c.is_alphanumeric() || c == '-')
        })
}
