//! The user-facing proofing object: a dictionary plus the user's own words.

use std::path::Path;
use std::sync::Arc;

use crate::casing::CaseKind;
use crate::dictionary::Dictionary;
use crate::error::SpellError;
use crate::library::{DictionaryLibrary, ResolvedDictionary};
use crate::suggest::{SuggestOptions, bounded_distance};
use crate::tokenize::{Misspelling, TokenizerOptions, find_words};
use crate::user::UserDictionary;

/// Spell checker for one language: a shared [`Dictionary`] plus a per-user [`UserDictionary`].
///
/// Cloning is cheap; clones share the dictionary and copy the user words.
#[derive(Clone, Debug)]
pub struct Checker {
    dictionary: Arc<Dictionary>,
    user: UserDictionary,
    resolved: Option<ResolvedDictionary>,
}

impl Checker {
    /// Wraps an already loaded dictionary.
    #[must_use]
    pub fn new(dictionary: impl Into<Arc<Dictionary>>) -> Self {
        Self {
            dictionary: dictionary.into(),
            user: UserDictionary::new(),
            resolved: None,
        }
    }

    /// Builds a checker from `.aff` and `.dic` text.
    ///
    /// # Errors
    /// Returns [`SpellError`] for malformed dictionary content.
    pub fn from_strs(aff: &str, dic: &str) -> Result<Self, SpellError> {
        Ok(Self::new(Dictionary::from_strs(aff, dic)?))
    }

    /// Loads the dictionary for a BCP-47 `tag` from `dir`, falling back to less specific
    /// dictionaries (`en-GB` -> `en`). Nothing is downloaded.
    ///
    /// # Errors
    /// Returns [`SpellError::NoDictionary`] when no dictionary matches, or a read/parse error.
    pub fn open(dir: impl AsRef<Path>, tag: &str) -> Result<Self, SpellError> {
        Self::from_library(&DictionaryLibrary::open(dir)?, tag)
    }

    /// Loads the dictionary for `tag` from an already scanned library.
    ///
    /// # Errors
    /// Returns [`SpellError::NoDictionary`] when no dictionary matches, or a read/parse error.
    pub fn from_library(library: &DictionaryLibrary, tag: &str) -> Result<Self, SpellError> {
        let (dictionary, resolved) = library.load(tag)?;
        Ok(Self {
            dictionary: Arc::new(dictionary),
            user: UserDictionary::new(),
            resolved: Some(resolved),
        })
    }

    /// The underlying dictionary.
    #[must_use]
    pub fn dictionary(&self) -> &Dictionary {
        &self.dictionary
    }

    /// Which dictionary file answered, when loaded through a library.
    #[must_use]
    pub fn resolved(&self) -> Option<&ResolvedDictionary> {
        self.resolved.as_ref()
    }

    /// True when `word` is spelled correctly, is in the user dictionary or is ignored.
    #[must_use]
    pub fn check(&self, word: &str) -> bool {
        self.user.contains_with(word, &self.dictionary.caser) || self.dictionary.spell(word)
    }

    /// Suggestions for a misspelled word, best first (at most ten).
    #[must_use]
    pub fn suggest(&self, word: &str) -> Vec<String> {
        self.suggest_with(word, &SuggestOptions::default())
    }

    /// Suggestions with explicit limits. User dictionary words one edit away come first.
    #[must_use]
    pub fn suggest_with(&self, word: &str, options: &SuggestOptions) -> Vec<String> {
        if self.check(word) {
            return Vec::new();
        }
        let caser = &self.dictionary.caser;
        let typed: Vec<char> = caser.lower(word).chars().collect();
        let kind = caser.kind(word);
        let mut out: Vec<String> = self
            .user
            .words()
            .filter(|w| {
                let candidate: Vec<char> = caser.lower(w).chars().collect();
                bounded_distance(&typed, &candidate, 1).is_some()
            })
            .map(|w| match kind {
                CaseKind::Upper => caser.upper(w),
                CaseKind::Title if w.chars().next().is_some_and(char::is_lowercase) => {
                    caser.capitalize_first(w)
                }
                _ => w.to_owned(),
            })
            .collect();
        for suggestion in self.dictionary.suggest(word, options) {
            if !out.contains(&suggestion) {
                out.push(suggestion);
            }
        }
        out.truncate(options.max_suggestions);
        out
    }

    /// Finds the misspelled words of a paragraph, with byte, `char` and UTF-16 offsets.
    ///
    /// URLs, email addresses, numbers, ALL-CAPS words and words with digits are skipped by
    /// default; see [`TokenizerOptions`].
    #[must_use]
    pub fn find_misspellings(&self, text: &str, options: &TokenizerOptions) -> Vec<Misspelling> {
        let mut options = options.clone();
        options.extra_word_chars.extend(
            self.dictionary
                .word_chars()
                .iter()
                .copied()
                .filter(|c| !c.is_alphanumeric()),
        );
        find_words(text, &options)
            .iter()
            .filter(|word| !self.check(word.text))
            .map(Misspelling::from)
            .collect()
    }

    /// The user's own words.
    #[must_use]
    pub fn user_dictionary(&self) -> &UserDictionary {
        &self.user
    }

    /// Mutable access to the user's own words.
    pub fn user_dictionary_mut(&mut self) -> &mut UserDictionary {
        &mut self.user
    }

    /// Replaces the user dictionary, for example with one loaded from disk.
    pub fn set_user_dictionary(&mut self, user: UserDictionary) {
        self.user = user;
    }

    /// Adds a word to the user dictionary ("Add to Dictionary").
    pub fn add_word(&mut self, word: &str) -> bool {
        self.user.add(word)
    }

    /// Ignores a word for the rest of the session ("Ignore All").
    pub fn ignore_word(&mut self, word: &str) -> bool {
        self.user.ignore(word)
    }
}
