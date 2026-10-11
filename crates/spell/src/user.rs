//! The custom user dictionary: persistent additions and session-wide "ignore all" words.

use std::collections::{BTreeSet, HashSet};
use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::casing::{CaseKind, Caser};
use crate::error::SpellError;

/// Largest user dictionary file that is read.
pub const MAX_FILE_BYTES: u64 = 1 << 20;
/// Most words kept from a user dictionary file.
pub const MAX_WORDS: usize = 50_000;
/// Longest word accepted, in characters.
pub const MAX_WORD_CHARS: usize = 128;

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
    ///
    /// Input is untrusted: at most [`MAX_WORDS`] words are kept and words longer than
    /// [`MAX_WORD_CHARS`] characters are skipped.
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        let mut dictionary = Self::new();
        for line in text.lines() {
            if dictionary.added.len() >= MAX_WORDS {
                break;
            }
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
    /// Returns [`SpellError::Io`] when the file cannot be read, is larger than
    /// [`MAX_FILE_BYTES`] or is not valid UTF-8 (a corrupt file). Nothing is loaded in that case;
    /// the next [`UserDictionary::save`] replaces the bad file.
    pub fn load(path: &Path) -> Result<Self, SpellError> {
        let io = |source| SpellError::Io {
            path: path.to_path_buf(),
            source,
        };
        let file = std::fs::File::open(path).map_err(io)?;
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(io)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "user dictionary is larger than the 1 MiB limit",
            )));
        }
        let text = String::from_utf8(bytes).map_err(|_| {
            io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "user dictionary is not valid UTF-8",
            ))
        })?;
        Ok(Self::from_text(&text))
    }

    /// Like [`UserDictionary::load`], but a file that does not exist yet is an empty dictionary
    /// (the first start-up, before anything was added).
    ///
    /// # Errors
    /// The same as [`UserDictionary::load`], except for a missing file.
    pub fn load_or_default(path: &Path) -> Result<Self, SpellError> {
        match Self::load(path) {
            Err(SpellError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
                Ok(Self::new())
            }
            other => other,
        }
    }

    /// Writes the added words to a file atomically: the text goes to a temporary file in the same
    /// directory, is flushed to disk and then renamed over `path`, so a crash leaves either the
    /// old or the new dictionary. Missing parent directories are created.
    ///
    /// # Errors
    /// Returns [`SpellError::Io`] when the file cannot be written.
    pub fn save(&self, path: &Path) -> Result<(), SpellError> {
        let io = |source| SpellError::Io {
            path: path.to_path_buf(),
            source,
        };
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(format!(".{}.tmp", std::process::id()));
        let temp = path.with_file_name(name);
        let written = (|| {
            let mut file = std::fs::File::create(&temp)?;
            file.write_all(self.to_text().as_bytes())?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temp, path)
        })();
        if let Err(e) = written {
            std::fs::remove_file(&temp).ok();
            return Err(io(e));
        }
        Ok(())
    }

    /// The per-user location of the custom dictionary: `%APPDATA%\Open365\Spelling\custom.dic` on
    /// Windows, `~/Library/Application Support/Open365/Spelling/custom.dic` on macOS and
    /// `$XDG_CONFIG_HOME` (default `~/.config`)`/open365/spelling/custom.dic` elsewhere. `None`
    /// when the platform variables are not set.
    #[must_use]
    pub fn default_path() -> Option<PathBuf> {
        let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
        Self::path_from(
            var("APPDATA").as_deref(),
            var("XDG_CONFIG_HOME").as_deref(),
            var("HOME").as_deref(),
        )
    }

    fn path_from(
        appdata: Option<&OsStr>,
        xdg_config: Option<&OsStr>,
        home: Option<&OsStr>,
    ) -> Option<PathBuf> {
        let file = "custom.dic";
        if cfg!(windows) {
            Some(
                PathBuf::from(appdata?)
                    .join("Open365")
                    .join("Spelling")
                    .join(file),
            )
        } else if cfg!(target_os = "macos") {
            let base = PathBuf::from(home?)
                .join("Library")
                .join("Application Support");
            Some(base.join("Open365").join("Spelling").join(file))
        } else {
            let base = match xdg_config {
                Some(dir) if Path::new(dir).is_absolute() => PathBuf::from(dir),
                _ => PathBuf::from(home?).join(".config"),
            };
            Some(base.join("open365").join("spelling").join(file))
        }
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
    !word.is_empty()
        && word.chars().count() <= MAX_WORD_CHARS
        && !word.chars().any(|c| c.is_whitespace() || c.is_control())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("open365-spell-user-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn save_is_atomic_and_round_trips() {
        let path = temp("nested/atomic.dic");
        let mut dict = UserDictionary::new();
        dict.add("Open365");
        dict.add("zork");
        dict.ignore("session");
        dict.save(&path).unwrap();
        dict.remove("zork");
        dict.save(&path).unwrap();
        let loaded = UserDictionary::load(&path).unwrap();
        assert_eq!(loaded.words().collect::<Vec<_>>(), ["Open365"]);
        let leftovers = std::fs::read_dir(path.parent().unwrap()).unwrap().count();
        assert_eq!(leftovers, 1, "no temporary file is left behind");
    }

    #[test]
    fn missing_is_empty_and_corrupt_or_oversized_files_are_rejected() {
        assert!(
            UserDictionary::load_or_default(&temp("absent.dic"))
                .unwrap()
                .is_empty()
        );
        let bad = temp("bad.dic");
        std::fs::write(&bad, [0x66, 0xff, 0xfe, 0x0a]).unwrap();
        assert!(UserDictionary::load(&bad).is_err());
        let big = temp("big.dic");
        std::fs::write(&big, vec![b'a'; (MAX_FILE_BYTES + 1) as usize]).unwrap();
        assert!(UserDictionary::load(&big).is_err());
        // A rejected file is replaced by the next save.
        let mut dict = UserDictionary::new();
        dict.add("fixed");
        dict.save(&bad).unwrap();
        assert_eq!(UserDictionary::load(&bad).unwrap().len(), 1);
    }

    #[test]
    fn untrusted_text_is_bounded() {
        let text: String = (0..MAX_WORDS + 10).map(|i| format!("w{i}\n")).collect();
        assert_eq!(UserDictionary::from_text(&text).len(), MAX_WORDS);
        assert!(UserDictionary::from_text(&"x".repeat(MAX_WORD_CHARS + 1)).is_empty());
        let mut dict = UserDictionary::from_text("\u{feff}# comment\n alpha \nbeta gamma\n\n");
        assert_eq!(dict.words().collect::<Vec<_>>(), ["alpha"]);
        assert!(dict.remove("alpha"));
        assert!(!dict.remove("alpha"));
    }

    #[test]
    fn default_path_is_per_user() {
        let os = |s: &'static str| Some(OsStr::new(s));
        if cfg!(windows) {
            let path = UserDictionary::path_from(os("C:/Users/a/AppData/Roaming"), None, None);
            assert!(path.unwrap().ends_with("Open365/Spelling/custom.dic"));
            assert!(UserDictionary::path_from(None, None, os("/h")).is_none());
        } else if !cfg!(target_os = "macos") {
            let path = UserDictionary::path_from(None, None, os("/home/a"));
            assert_eq!(
                path.unwrap(),
                PathBuf::from("/home/a/.config/open365/spelling/custom.dic")
            );
            assert!(UserDictionary::path_from(None, None, None).is_none());
        }
    }
}
