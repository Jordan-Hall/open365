//! The proofing service: dictionary selection by language tag, a shared user dictionary and
//! paragraph checking. Thread-safe and cheap to clone, so a background worker and the UI thread
//! can hold the same service.

use std::collections::HashMap;
use std::fmt;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock};

use crate::bundled::{self, Bundled};
use crate::dictionary::Dictionary;
use crate::error::SpellError;
use crate::library::{DictionaryLibrary, ResolvedDictionary};
use crate::paragraph::{ProofOptions, proof_text};
use crate::suggest::SuggestOptions;
use crate::tokenize::Misspelling;
use crate::user::UserDictionary;

/// Stop remembering per-dictionary verdicts beyond this many distinct words.
const VERDICT_CACHE_LIMIT: usize = 100_000;

/// A stretch of a paragraph in one proofing language (Word: `w:lang` on a run).
///
/// `lang: None` marks text that is not proofed (`w:noProof`, or the language is "no proofing").
/// Adjacent spans with the same language are merged before checking, so a word that crosses a run
/// boundary is still one word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LangSpan {
    /// Byte range in the paragraph text. Ends that fall inside a character are ignored.
    pub byte_range: Range<usize>,
    /// BCP-47 tag (`en-GB`), or `None` for text that is not checked.
    pub lang: Option<String>,
}

impl LangSpan {
    /// A span in language `lang`.
    #[must_use]
    pub fn new(byte_range: Range<usize>, lang: impl Into<String>) -> Self {
        Self {
            byte_range,
            lang: Some(lang.into()),
        }
    }

    /// A span that is not proofed.
    #[must_use]
    pub fn no_proof(byte_range: Range<usize>) -> Self {
        Self {
            byte_range,
            lang: None,
        }
    }
}

/// The result of checking one paragraph.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParagraphReport {
    /// Flagged words in text order, with byte, `char` and UTF-16 ranges.
    pub issues: Vec<Misspelling>,
    /// Language tags that had no dictionary; those spans were not checked.
    pub missing_languages: Vec<String>,
}

/// A loaded dictionary and its memo of verdicts.
struct Loaded {
    dictionary: Dictionary,
    resolved: ResolvedDictionary,
    verdicts: Mutex<HashMap<Box<str>, bool>>,
}

impl Loaded {
    /// The dictionary's verdict, remembered because misspellings are the slow case.
    fn spells(&self, word: &str) -> bool {
        if let Some(&known) = self
            .verdicts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(word)
        {
            return known;
        }
        let verdict = self.dictionary.spell(word);
        let mut memo = self.verdicts.lock().unwrap_or_else(PoisonError::into_inner);
        if memo.len() >= VERDICT_CACHE_LIMIT {
            memo.clear();
        }
        memo.insert(word.into(), verdict);
        verdict
    }
}

#[derive(Clone)]
enum Source {
    Bundled(Bundled),
    Files { aff: PathBuf, dic: PathBuf },
}

type Slot = Arc<OnceLock<Result<Arc<Loaded>, SpellError>>>;

struct Inner {
    library: RwLock<DictionaryLibrary>,
    loaded: Mutex<HashMap<String, Slot>>,
    user: RwLock<UserDictionary>,
    generation: AtomicU64,
    options: RwLock<ProofOptions>,
}

/// Spell checking for a whole application: any number of languages, one user dictionary.
///
/// Dictionaries load lazily on first use (about a hundred milliseconds for a full English one),
/// so call [`Proofing::preload`] from a background thread for the document language. Bundled
/// English dictionaries need no files; every other language comes from directories added with
/// [`Proofing::install_dir`].
#[derive(Clone)]
pub struct Proofing {
    inner: Arc<Inner>,
}

impl fmt::Debug for Proofing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Proofing")
            .field("generation", &self.generation())
            .finish_non_exhaustive()
    }
}

impl Default for Proofing {
    fn default() -> Self {
        Self::new()
    }
}

impl Proofing {
    /// A service with the bundled dictionaries only.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                library: RwLock::new(DictionaryLibrary::default()),
                loaded: Mutex::new(HashMap::new()),
                user: RwLock::new(UserDictionary::new()),
                generation: AtomicU64::new(0),
                options: RwLock::new(ProofOptions::default()),
            }),
        }
    }

    /// Adds a directory of user-installed dictionaries (`xx_YY.aff` + `.dic`, or `xx-YY/index.*`).
    /// Earlier directories win when two provide the same tag.
    ///
    /// # Errors
    /// Returns [`SpellError::Io`] when the directory cannot be listed.
    pub fn install_dir(&self, dir: impl AsRef<Path>) -> Result<(), SpellError> {
        self.inner
            .library
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .add_dir(dir)?;
        self.bump();
        Ok(())
    }

    /// Tags of every dictionary that can be used: bundled and installed.
    #[must_use]
    pub fn available_tags(&self) -> Vec<String> {
        let mut tags: Vec<String> = bundled::bundled_tags()
            .into_iter()
            .map(str::to_owned)
            .collect();
        let library = self
            .inner
            .library
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        for entry in library.dictionaries() {
            if !tags.iter().any(|t| t.eq_ignore_ascii_case(&entry.tag)) {
                tags.push(entry.tag.clone());
            }
        }
        tags.sort();
        tags
    }

    /// Which dictionary would answer for `tag`, without loading it. `None` when there is none.
    #[must_use]
    pub fn resolve(&self, tag: &str) -> Option<ResolvedDictionary> {
        self.locate(tag).map(|(_, _, resolved)| resolved)
    }

    /// Loads the dictionary for `tag` now, so the first paragraph check does not wait for it.
    ///
    /// # Errors
    /// Returns [`SpellError::NoDictionary`] when none matches, or the load error.
    pub fn preload(&self, tag: &str) -> Result<ResolvedDictionary, SpellError> {
        self.dictionary(tag).map(|loaded| loaded.resolved.clone())
    }

    /// Changes whenever the user dictionary, "ignore all" list or installed dictionaries change.
    /// Cached results from an earlier generation may be stale.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.inner.generation.load(Ordering::Acquire)
    }

    fn bump(&self) {
        self.inner.generation.fetch_add(1, Ordering::AcqRel);
    }

    /// The current proofing options.
    #[must_use]
    pub fn options(&self) -> ProofOptions {
        self.inner
            .options
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Replaces the proofing options (the Word "Proofing" settings).
    pub fn set_options(&self, options: ProofOptions) {
        *self
            .inner
            .options
            .write()
            .unwrap_or_else(PoisonError::into_inner) = options;
        self.bump();
    }

    /// "Add to Dictionary": remembers `word` for every language. Returns false if it was invalid
    /// or already present. Persist with [`Proofing::save_user_dictionary`].
    pub fn add_to_dictionary(&self, word: &str) -> bool {
        let added = self.user_mut(|user| user.add(word));
        if added {
            self.bump();
        }
        added
    }

    /// "Ignore All": accepts `word` for this session only.
    pub fn ignore_all(&self, word: &str) -> bool {
        let added = self.user_mut(|user| user.ignore(word));
        if added {
            self.bump();
        }
        added
    }

    /// Removes a word added with [`Proofing::add_to_dictionary`].
    pub fn remove_from_dictionary(&self, word: &str) -> bool {
        let removed = self.user_mut(|user| user.remove(word));
        if removed {
            self.bump();
        }
        removed
    }

    /// Forgets every "Ignore All" word (Word: "Recheck Document").
    pub fn reset_ignored(&self) {
        self.user_mut(UserDictionary::clear_ignored);
        self.bump();
    }

    /// A copy of the user dictionary.
    #[must_use]
    pub fn user_dictionary(&self) -> UserDictionary {
        self.inner
            .user
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Replaces the user dictionary, for example with one read from disk at start-up.
    pub fn set_user_dictionary(&self, user: UserDictionary) {
        self.user_mut(|current| *current = user);
        self.bump();
    }

    /// Reads the user dictionary from a file, keeping the current "Ignore All" words.
    ///
    /// # Errors
    /// Returns [`SpellError::Io`] when the file cannot be read.
    pub fn load_user_dictionary(&self, path: &Path) -> Result<(), SpellError> {
        let loaded = UserDictionary::load(path)?;
        self.user_mut(|user| {
            for word in loaded.words() {
                user.add(word);
            }
        });
        self.bump();
        Ok(())
    }

    /// Writes the user dictionary to a file.
    ///
    /// # Errors
    /// Returns [`SpellError::Io`] when the file cannot be written.
    pub fn save_user_dictionary(&self, path: &Path) -> Result<(), SpellError> {
        self.user_dictionary().save(path)
    }

    fn user_mut<T>(&self, f: impl FnOnce(&mut UserDictionary) -> T) -> T {
        f(&mut self
            .inner
            .user
            .write()
            .unwrap_or_else(PoisonError::into_inner))
    }

    /// True when `word` is correct in language `lang` (user dictionary included). Words in a
    /// language with no dictionary count as correct.
    #[must_use]
    pub fn check_word(&self, word: &str, lang: &str) -> bool {
        let Ok(loaded) = self.dictionary(lang) else {
            return true;
        };
        let user = self
            .inner
            .user
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        user.contains_with(word, &loaded.dictionary.caser) || loaded.spells(word)
    }

    /// Suggestions for a misspelled `word` in language `lang`, best first, at most
    /// [`ProofOptions::max_suggestions`]. Empty when the word is fine or no dictionary exists.
    #[must_use]
    pub fn suggest(&self, word: &str, lang: &str) -> Vec<String> {
        let Ok(loaded) = self.dictionary(lang) else {
            return Vec::new();
        };
        let options = SuggestOptions {
            max_suggestions: self.options().max_suggestions,
        };
        let user = self
            .inner
            .user
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        crate::checker::suggest_with_user(&loaded.dictionary, &user, word, &options)
    }

    /// Checks one paragraph written entirely in `lang`.
    #[must_use]
    pub fn check_paragraph(&self, text: &str, lang: &str) -> ParagraphReport {
        self.check_spans(text, &[LangSpan::new(0..text.len(), lang)])
    }

    /// Checks a paragraph whose runs have different languages. Text outside every span, and
    /// spans with `lang: None`, are not checked.
    #[must_use]
    pub fn check_spans(&self, text: &str, spans: &[LangSpan]) -> ParagraphReport {
        let options = self.options();
        let user = self
            .inner
            .user
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        let mut report = ParagraphReport::default();
        for (range, lang) in merged_spans(text, spans) {
            let Ok(loaded) = self.dictionary(&lang) else {
                if !report.missing_languages.contains(&lang) {
                    report.missing_languages.push(lang);
                }
                continue;
            };
            let slice = &text[range.clone()];
            let mut tokenizer = options.tokenizer.clone();
            tokenizer.extra_word_chars.extend(
                loaded
                    .dictionary
                    .word_chars()
                    .iter()
                    .copied()
                    .filter(|c| !c.is_alphanumeric()),
            );
            let caser = &loaded.dictionary.caser;
            let found = proof_text(slice, &options, &tokenizer, caser, &mut |word| {
                user.contains_with(word, caser) || loaded.spells(word)
            });
            if found.is_empty() {
                continue;
            }
            let base_chars = text[..range.start].chars().count();
            let base_utf16: usize = text[..range.start].chars().map(char::len_utf16).sum();
            for mut issue in found {
                shift(&mut issue.byte_range, range.start);
                shift(&mut issue.char_range, base_chars);
                shift(&mut issue.utf16_range, base_utf16);
                report.issues.push(issue);
            }
        }
        report
    }

    fn dictionary(&self, tag: &str) -> Result<Arc<Loaded>, SpellError> {
        let (key, source, resolved) = self.locate(tag).ok_or_else(|| SpellError::NoDictionary {
            tag: tag.to_owned(),
        })?;
        let slot = Arc::clone(
            self.inner
                .loaded
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entry(key.clone())
                .or_default(),
        );
        // Initialised outside the map lock; concurrent callers for one dictionary wait on the slot.
        match slot.get_or_init(|| Self::load(&source, resolved)) {
            Ok(loaded) => Ok(Arc::clone(loaded)),
            Err(error) => Err(SpellError::Load {
                dictionary: key,
                message: error.to_string(),
            }),
        }
    }

    fn load(source: &Source, resolved: ResolvedDictionary) -> Result<Arc<Loaded>, SpellError> {
        let dictionary = match source {
            Source::Bundled(b) => Dictionary::parse(b.aff, b.dic)?,
            Source::Files { aff, dic } => Dictionary::from_files(aff, dic)?,
        };
        Ok(Arc::new(Loaded {
            dictionary,
            resolved,
            verdicts: Mutex::new(HashMap::new()),
        }))
    }

    /// Picks the dictionary for `tag`: an installed exact match, else a bundled one (nearest
    /// region), else the nearest installed one (`zh-Hant-TW` -> `zh`).
    fn locate(&self, tag: &str) -> Option<(String, Source, ResolvedDictionary)> {
        let key = normalize_tag(tag)?;
        let library = self
            .inner
            .library
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        let installed = library.resolve(&key);
        if let Some((files, resolved)) = &installed
            && resolved.exact
        {
            return Some(from_files(files, resolved, tag));
        }
        if let Some((entry, exact)) = bundled::find(&key) {
            let resolved = ResolvedDictionary {
                requested: tag.to_owned(),
                matched: entry.tag.to_owned(),
                exact,
            };
            return Some((
                format!("bundled:{}", entry.tag),
                Source::Bundled(entry),
                resolved,
            ));
        }
        installed.map(|(files, resolved)| from_files(files, &resolved, tag))
    }
}

fn from_files(
    files: &crate::library::DictionaryFiles,
    resolved: &ResolvedDictionary,
    requested: &str,
) -> (String, Source, ResolvedDictionary) {
    (
        format!("file:{}", files.aff_path.display()),
        Source::Files {
            aff: files.aff_path.clone(),
            dic: files.dic_path.clone(),
        },
        ResolvedDictionary {
            requested: requested.to_owned(),
            ..resolved.clone()
        },
    )
}

/// Lower-case, hyphenated tag; `None` for "no language" markers.
fn normalize_tag(tag: &str) -> Option<String> {
    let key = tag.trim().replace('_', "-").to_ascii_lowercase();
    (!key.is_empty() && !matches!(key.as_str(), "und" | "zxx" | "x-none" | "none")).then_some(key)
}

fn shift(range: &mut Range<usize>, by: usize) {
    range.start += by;
    range.end += by;
}

/// Valid, ordered, non-overlapping spans with a language, adjacent equal languages merged.
fn merged_spans(text: &str, spans: &[LangSpan]) -> Vec<(Range<usize>, String)> {
    let mut sorted: Vec<&LangSpan> = spans.iter().collect();
    sorted.sort_by_key(|s| (s.byte_range.start, s.byte_range.end));
    let mut out: Vec<(Range<usize>, String)> = Vec::new();
    let mut covered = 0;
    for span in sorted {
        let start = span.byte_range.start.max(covered);
        let end = span.byte_range.end.min(text.len());
        if start >= end || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
            continue;
        }
        covered = end;
        let Some(lang) = span.lang.as_deref() else {
            continue;
        };
        match out.last_mut() {
            Some((range, last)) if range.end == start && last == lang => range.end = end,
            _ => out.push((start..end, lang.to_owned())),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_are_clamped_merged_and_ordered() {
        let text = "ab cd ef";
        let spans = [
            LangSpan::new(3..5, "en-GB"),
            LangSpan::new(0..3, "en-GB"),
            LangSpan::no_proof(5..6),
            LangSpan::new(6..99, "de"),
        ];
        assert_eq!(
            merged_spans(text, &spans),
            [(0..5, "en-GB".to_owned()), (6..8, "de".to_owned())]
        );
    }

    #[test]
    fn spans_inside_a_character_are_dropped() {
        let spans = [LangSpan::new(0..1, "en"), LangSpan::new(2..3, "en")];
        assert_eq!(merged_spans("é!", &spans), [(2..3, "en".to_owned())]);
    }

    #[test]
    fn no_language_markers_do_not_resolve() {
        for tag in ["", "und", "zxx", "x-none"] {
            assert!(normalize_tag(tag).is_none(), "{tag}");
        }
        assert_eq!(normalize_tag("EN_gb").as_deref(), Some("en-gb"));
    }
}
