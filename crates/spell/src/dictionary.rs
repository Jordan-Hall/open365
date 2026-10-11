//! A parsed Hunspell dictionary (`.aff` + `.dic`) and the public checking API on it.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::Path;
use std::sync::Arc;

use encoding_rs::Encoding;

use crate::aff::Aff;
use crate::casing::Caser;
use crate::error::SpellError;
use crate::flags::{Flag, FlagSet, parse_flags};

/// A Hunspell-format dictionary: affix rules plus the word list.
///
/// Immutable after construction, so it can be shared between threads behind an [`Arc`].
pub struct Dictionary {
    pub(crate) aff: Aff,
    pub(crate) words: HashMap<Box<str>, Vec<Arc<FlagSet>>>,
    pub(crate) caser: Caser,
    pub(crate) default_try: Vec<char>,
    pub(crate) compound_rule_flags: HashSet<Flag>,
}

impl fmt::Debug for Dictionary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dictionary")
            .field("lang", &self.aff.lang)
            .field("stems", &self.words.len())
            .field("prefix_rules", &self.aff.prefixes.rules.len())
            .field("suffix_rules", &self.aff.suffixes.rules.len())
            .finish()
    }
}

impl Dictionary {
    /// Parses a dictionary from raw `.aff` and `.dic` bytes, honouring the `SET` encoding.
    ///
    /// # Errors
    /// Returns [`SpellError`] for an unsupported encoding or malformed content.
    pub fn parse(aff: &[u8], dic: &[u8]) -> Result<Self, SpellError> {
        let encoding = detect_encoding(aff)?;
        let aff_text = decode(aff, encoding);
        let dic_text = decode(dic, encoding);
        Self::from_strs(&aff_text, &dic_text)
    }

    /// Parses a dictionary from already decoded text.
    ///
    /// # Errors
    /// Returns [`SpellError`] for malformed content.
    pub fn from_strs(aff_text: &str, dic_text: &str) -> Result<Self, SpellError> {
        let aff = Aff::parse(aff_text)?;
        let caser = Caser::for_lang(&aff.lang);
        let mut intern: HashMap<Vec<Flag>, Arc<FlagSet>> = HashMap::new();
        let mut words: HashMap<Box<str>, Vec<Arc<FlagSet>>> = HashMap::new();
        let mut first = true;

        for (index, raw) in dic_text.lines().enumerate() {
            let line = raw.trim_start_matches('\u{feff}').trim_end();
            if line.is_empty() || line.starts_with('\t') || line.starts_with('#') {
                continue;
            }
            if first {
                first = false;
                if line
                    .split_whitespace()
                    .next()
                    .is_some_and(|t| t.parse::<usize>().is_ok())
                {
                    continue;
                }
            }
            let (word, flag_text) = split_dic_line(line);
            let word: String = word.chars().filter(|c| !aff.ignore.contains(c)).collect();
            if word.is_empty() {
                continue;
            }
            let flags = resolve_flags(&aff, flag_text).map_err(|message| SpellError::Dic {
                line: index + 1,
                message,
            })?;
            let set = intern
                .entry(flags.clone())
                .or_insert_with(|| Arc::new(FlagSet::new(flags)))
                .clone();
            words.entry(word.into_boxed_str()).or_default().push(set);
        }

        let default_try = if aff.try_chars.is_empty() {
            frequent_chars(&words)
        } else {
            Vec::new()
        };
        let compound_rule_flags = aff
            .compound_rules
            .iter()
            .flat_map(|rule| rule.iter().map(|item| item.flag))
            .collect();
        Ok(Self {
            aff,
            words,
            caser,
            default_try,
            compound_rule_flags,
        })
    }

    /// Reads a dictionary from `.aff` and `.dic` files.
    ///
    /// # Errors
    /// Returns [`SpellError`] when a file cannot be read or parsed.
    pub fn from_files(aff_path: &Path, dic_path: &Path) -> Result<Self, SpellError> {
        let read = |path: &Path| {
            std::fs::read(path).map_err(|source| SpellError::Io {
                path: path.to_path_buf(),
                source,
            })
        };
        Self::parse(&read(aff_path)?, &read(dic_path)?)
    }

    /// The dictionary language from the `LANG` directive, empty when absent.
    #[must_use]
    pub fn lang(&self) -> &str {
        &self.aff.lang
    }

    /// Extra characters the dictionary treats as part of words (`WORDCHARS`), such as `'` or `.`.
    #[must_use]
    pub fn word_chars(&self) -> &[char] {
        &self.aff.word_chars
    }

    /// Number of distinct dictionary entries (stems).
    #[must_use]
    pub fn len(&self) -> usize {
        self.words.len()
    }

    /// True when the word list is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Characters tried for insert and replace suggestions, most likely first.
    pub(crate) fn try_chars(&self) -> &[char] {
        if self.aff.try_chars.is_empty() {
            &self.default_try
        } else {
            &self.aff.try_chars
        }
    }

    /// Applies `ICONV` and removes `IGNORE` characters.
    pub(crate) fn normalize(&self, word: &str) -> String {
        let converted = apply_conv(&self.aff.iconv, word);
        if self.aff.ignore.is_empty() {
            converted
        } else {
            converted
                .chars()
                .filter(|c| !self.aff.ignore.contains(c))
                .collect()
        }
    }

    /// Applies `OCONV` to a suggestion.
    pub(crate) fn output_convert(&self, word: &str) -> String {
        apply_conv(&self.aff.oconv, word)
    }
}

/// Longest-match replacement used by `ICONV` and `OCONV`.
pub(crate) fn apply_conv(table: &[(String, String)], text: &str) -> String {
    if table.is_empty() {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < text.len() {
        let rest = &text[at..];
        let best = table
            .iter()
            .filter(|(from, _)| !from.is_empty() && rest.starts_with(from.as_str()))
            .max_by_key(|(from, _)| from.len());
        if let Some((from, to)) = best {
            out.push_str(to);
            at += from.len();
        } else if let Some(c) = rest.chars().next() {
            out.push(c);
            at += c.len_utf8();
        } else {
            break;
        }
    }
    out
}

/// Splits a `.dic` line into the word and its flag text, dropping morphological fields.
fn split_dic_line(line: &str) -> (String, &str) {
    // Morphological fields start at a tab, or at a space followed by `xx:`.
    let mut cut = line.len();
    if let Some(tab) = line.find('\t') {
        cut = tab;
    }
    let bytes = line.as_bytes();
    for (i, &b) in bytes.iter().enumerate().take(cut) {
        if b == b' '
            && bytes.get(i + 3) == Some(&b':')
            && bytes.get(i + 1).is_some_and(|c| !c.is_ascii_whitespace())
            && bytes.get(i + 2).is_some_and(|c| !c.is_ascii_whitespace())
        {
            cut = i;
            break;
        }
    }
    let head = line[..cut].trim_end();
    let mut word = String::new();
    let mut chars = head.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => {
                if let Some((_, next)) = chars.next() {
                    word.push(next);
                }
            }
            '/' if !word.is_empty() => return (word, head[i + 1..].trim()),
            other => word.push(other),
        }
    }
    (word, "")
}

fn resolve_flags(aff: &Aff, text: &str) -> Result<Vec<Flag>, String> {
    if text.is_empty() {
        return Ok(Vec::new());
    }
    if !aff.af.is_empty()
        && let Ok(index) = text.parse::<usize>()
    {
        return match index.checked_sub(1).and_then(|i| aff.af.get(i)) {
            Some(flags) => Ok(flags.clone()),
            None => Err(format!("AF alias {index} is not defined")),
        };
    }
    parse_flags(text, aff.flag_mode)
}

/// The most common lower-case characters of the word list, used when `TRY` is absent.
fn frequent_chars(words: &HashMap<Box<str>, Vec<Arc<FlagSet>>>) -> Vec<char> {
    let mut counts: HashMap<char, usize> = HashMap::new();
    for word in words.keys() {
        for c in word
            .chars()
            .filter(|c| c.is_alphabetic() && !c.is_uppercase())
        {
            *counts.entry(c).or_default() += 1;
        }
    }
    let mut ranked: Vec<(char, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked.into_iter().take(48).map(|(c, _)| c).collect()
}

/// Reads the `SET` directive from raw `.aff` bytes and picks the matching decoder.
fn detect_encoding(aff: &[u8]) -> Result<&'static Encoding, SpellError> {
    for line in aff.split(|&b| b == b'\n') {
        let line = line.trim_ascii();
        if let Some(rest) = line.strip_prefix(b"SET")
            && rest.first().is_some_and(u8::is_ascii_whitespace)
        {
            let label = String::from_utf8_lossy(rest).trim().to_owned();
            return encoding_for_label(&label);
        }
    }
    // No SET: modern files are UTF-8; older Hunspell default is ISO-8859-1.
    if std::str::from_utf8(aff).is_ok() {
        Ok(encoding_rs::UTF_8)
    } else {
        Ok(encoding_rs::WINDOWS_1252)
    }
}

fn encoding_for_label(label: &str) -> Result<&'static Encoding, SpellError> {
    let lower = label.to_ascii_lowercase();
    let mapped = if let Some(rest) = lower.strip_prefix("iso8859-") {
        format!("iso-8859-{rest}")
    } else if let Some(rest) = lower.strip_prefix("microsoft-cp") {
        format!("windows-{rest}")
    } else if matches!(lower.as_str(), "tis620-2533" | "tis-620") {
        "windows-874".to_owned()
    } else {
        lower
    };
    Encoding::for_label(mapped.as_bytes())
        .ok_or_else(|| SpellError::UnsupportedEncoding(label.to_owned()))
}

fn decode(bytes: &[u8], encoding: &'static Encoding) -> String {
    encoding.decode(bytes).0.into_owned()
}
