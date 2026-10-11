//! The custom user dictionary: persistent additions and session-wide "ignore all" words.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use crate::casing::{CaseKind, Caser};
use crate::error::SpellError;

/// Words the user added (kept across sessions) or chose to ignore (this session only).
///
/// Matching follows Word: an added lower-case word also matches its capitalised and upper-case
/// forms, while an added `Capitalised` or `MixedCase` word matches itself (and its upper-case form
/// when it was added capitalised).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserDictionary {
    added: BTreeSet<String>,
    ignored: HashSet<String>,
}

impl UserDictionary {
    /// An empty user dictionary.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses the persisted form: one word per line, `#` starts a comment line.
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        let mut dictionary = Self::new();
        for line in text.lines() {
            let word = line.trim_start_matches('\u{feff}').trim();
            if !word.starts_with('#') {
                dictionary.add(word);
            }
        }
        dictionary
    }

    /// The persisted form of the added words, sorted, one per line.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for word in &self.added {
            out.push_str(word);
            out.push('\n');
        }
        out
    }

    /// Reads a user dictionary file.
    ///
    /// # Errors
    /// Returns [`SpellError::Io`] when the file cannot be read as UTF-8 text.
    pub fn load(path: &Path) -> Result<Self, SpellError> {
        std::fs::read_to_string(path)
            .map(|text| Self::from_text(&text))
            .map_err(|source| SpellError::Io {
                path: path.to_path_buf(),
                source,
            })
    }

    /// Writes the added words to a file.
    ///
    /// # Errors
    /// Returns [`SpellError::Io`] when the file cannot be written.
    pub fn save(&self, path: &Path) -> Result<(), SpellError> {
        std::fs::write(path, self.to_text()).map_err(|source| SpellError::Io {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Adds a word permanently. Returns false for an empty word, one with whitespace or control
    /// characters, or one that is already present.
    pub fn add(&mut self, word: &str) -> bool {
        valid_entry(word) && self.added.insert(word.to_owned())
    }

    /// Removes an added word. Returns true when it was present.
    pub fn remove(&mut self, word: &str) -> bool {
        self.added.remove(word)
    }

    /// Ignores a word for this session ("Ignore All"). Ignored words are never persisted.
    pub fn ignore(&mut self, word: &str) -> bool {
        valid_entry(word) && self.ignored.insert(word.to_owned())
    }

    /// Forgets every session-wide ignored word.
    pub fn clear_ignored(&mut self) {
        self.ignored.clear();
    }

    /// The added words in sorted order.
    pub fn words(&self) -> impl Iterator<Item = &str> {
        self.added.iter().map(String::as_str)
    }

    /// Number of added words.
    #[must_use]
    pub fn len(&self) -> usize {
        self.added.len()
    }

    /// True when no words were added.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
    }

    /// True when `word` was added or ignored, with Word's capitalisation rules.
    #[must_use]
    pub fn contains(&self, word: &str) -> bool {
        self.contains_with(word, &Caser::default())
    }

    pub(crate) fn contains_with(&self, word: &str, caser: &Caser) -> bool {
        if self.added.is_empty() && self.ignored.is_empty() {
            return false;
        }
        let mut forms = vec![word.to_owned()];
        match caser.kind(word) {
            CaseKind::Title => forms.push(caser.lower(word)),
            CaseKind::Upper => {
                forms.push(caser.title(word));
                forms.push(caser.lower(word));
            }
            _ => {}
        }
        forms
            .iter()
            .any(|form| self.added.contains(form) || self.ignored.contains(form))
    }
}

fn valid_entry(word: &str) -> bool {
    !word.is_empty() && !word.chars().any(|c| c.is_whitespace() || c.is_control())
}
