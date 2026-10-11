//! Locating dictionaries on disk by BCP-47 tag, with language fallback.

use std::fs;
use std::path::{Path, PathBuf};

use crate::dictionary::Dictionary;
use crate::error::SpellError;

/// One dictionary found on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DictionaryFiles {
    /// The BCP-47 tag derived from the file or folder name, such as `en-GB`.
    pub tag: String,
    /// Path of the `.aff` file.
    pub aff_path: PathBuf,
    /// Path of the `.dic` file.
    pub dic_path: PathBuf,
    key: String,
}

/// Which dictionary answered a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedDictionary {
    /// The tag that was asked for.
    pub requested: String,
    /// The tag of the dictionary that was loaded.
    pub matched: String,
    /// False when a less specific or sibling dictionary was used (`en-GB` answered by `en`).
    pub exact: bool,
}

/// A set of dictionaries available in one or more directories.
///
/// Two layouts are understood: flat `en_GB.aff` + `en_GB.dic` pairs (LibreOffice, Hunspell) and
/// one folder per tag holding `index.aff` + `index.dic` (the `dictionaries` npm layout). Tags may
/// be written `en-GB`, `en_GB` or `en_gb`. Dictionaries are only read from disk, never fetched.
#[derive(Clone, Debug, Default)]
pub struct DictionaryLibrary {
    entries: Vec<DictionaryFiles>,
}

impl DictionaryLibrary {
    /// Scans `dir` for dictionaries.
    ///
    /// # Errors
    /// Returns [`SpellError::Io`] when the directory cannot be listed.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, SpellError> {
        let mut library = Self::default();
        library.add_dir(dir)?;
        Ok(library)
    }

    /// Scans another directory. Earlier directories win when two provide the same tag.
    ///
    /// # Errors
    /// Returns [`SpellError::Io`] when the directory cannot be listed.
    pub fn add_dir(&mut self, dir: impl AsRef<Path>) -> Result<(), SpellError> {
        let dir = dir.as_ref();
        let io = |source| SpellError::Io {
            path: dir.to_path_buf(),
            source,
        };
        let mut found: Vec<DictionaryFiles> = Vec::new();
        for item in fs::read_dir(dir).map_err(io)? {
            let path = item.map_err(io)?.path();
            if path.is_dir() {
                let (aff, dic) = (path.join("index.aff"), path.join("index.dic"));
                if let (true, Some(name)) = (aff.is_file() && dic.is_file(), path.file_name()) {
                    found.extend(entry_for(&name.to_string_lossy(), aff, dic));
                }
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("dic"))
            {
                let aff = path.with_extension("aff");
                if let (true, Some(stem)) = (aff.is_file(), path.file_stem()) {
                    found.extend(entry_for(&stem.to_string_lossy(), aff, path.clone()));
                }
            }
        }
        found.sort_by(|a, b| a.key.cmp(&b.key).then(a.aff_path.cmp(&b.aff_path)));
        for entry in found {
            if !self.entries.iter().any(|e| e.key == entry.key) {
                self.entries.push(entry);
            }
        }
        Ok(())
    }

    /// The dictionaries found, in tag order within each directory.
    #[must_use]
    pub fn dictionaries(&self) -> &[DictionaryFiles] {
        &self.entries
    }

    /// Finds the dictionary for `tag`: the exact tag, then each shorter prefix (`zh-Hant-TW` ->
    /// `zh-Hant` -> `zh`), and at each step a sibling variant (`de-DE` -> `de-DE-frami`, `en` ->
    /// `en-US`).
    #[must_use]
    pub fn resolve(&self, tag: &str) -> Option<(&DictionaryFiles, ResolvedDictionary)> {
        let wanted = tag_key(tag);
        if wanted.is_empty() || wanted == "und" {
            return None;
        }
        let parts: Vec<&str> = wanted.split('-').collect();
        for len in (1..=parts.len()).rev() {
            let prefix = parts[..len].join("-");
            let hit = self
                .entries
                .iter()
                .find(|e| e.key == prefix)
                .or_else(|| self.best_variant(&prefix));
            if let Some(entry) = hit {
                let exact = entry.key == wanted;
                return Some((
                    entry,
                    ResolvedDictionary {
                        requested: tag.to_owned(),
                        matched: entry.tag.clone(),
                        exact,
                    },
                ));
            }
        }
        None
    }

    /// Among `prefix-*` dictionaries, prefers the language's home region, then the shortest tag.
    fn best_variant(&self, prefix: &str) -> Option<&DictionaryFiles> {
        let lead = format!("{prefix}-");
        let home = default_region(prefix).map(|region| format!("{prefix}-{region}"));
        self.entries
            .iter()
            .filter(|e| e.key.starts_with(&lead))
            .min_by_key(|e| {
                let preferred = home
                    .as_deref()
                    .is_some_and(|h| e.key == h || e.key.starts_with(&format!("{h}-")));
                (!preferred, e.key.len(), e.key.clone())
            })
    }

    /// Loads the dictionary for `tag`.
    ///
    /// # Errors
    /// Returns [`SpellError::NoDictionary`] when nothing matches, or the parse error.
    pub fn load(&self, tag: &str) -> Result<(Dictionary, ResolvedDictionary), SpellError> {
        let (files, resolved) = self.resolve(tag).ok_or_else(|| SpellError::NoDictionary {
            tag: tag.to_owned(),
        })?;
        Ok((
            Dictionary::from_files(&files.aff_path, &files.dic_path)?,
            resolved,
        ))
    }
}

/// Lower-cased, hyphen separated form used for matching.
fn tag_key(tag: &str) -> String {
    tag.trim().replace('_', "-").to_ascii_lowercase()
}

/// Builds an entry if `name` looks like a language tag.
fn entry_for(name: &str, aff_path: PathBuf, dic_path: PathBuf) -> Option<DictionaryFiles> {
    let key = tag_key(name);
    let valid = !key.is_empty()
        && key.split('-').all(|part| {
            (1..=8).contains(&part.len()) && part.chars().all(|c| c.is_ascii_alphanumeric())
        })
        && key.split('-').next().is_some_and(|lang| {
            (2..=3).contains(&lang.len()) && lang.chars().all(|c| c.is_ascii_alphabetic())
        });
    valid.then(|| DictionaryFiles {
        tag: display_tag(&key),
        aff_path,
        dic_path,
        key,
    })
}

/// BCP-47 capitalisation: `en-gb` -> `en-GB`, `sr-latn` -> `sr-Latn`.
fn display_tag(key: &str) -> String {
    key.split('-')
        .enumerate()
        .map(|(i, part)| match (i, part.len()) {
            (0, _) => part.to_owned(),
            (_, 4) if part.chars().all(|c| c.is_ascii_alphabetic()) => {
                let mut chars = part.chars();
                chars
                    .next()
                    .map(|c| c.to_ascii_uppercase().to_string() + chars.as_str())
                    .unwrap_or_default()
            }
            (_, 2) if part.chars().all(|c| c.is_ascii_alphabetic()) => part.to_ascii_uppercase(),
            _ => part.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("-")
}

/// The region whose dictionary stands in for a bare language tag.
fn default_region(language: &str) -> Option<&'static str> {
    Some(match language {
        "en" => "us",
        "pt" => "br",
        "es" => "es",
        "zh" => "cn",
        "de" => "de",
        "fr" => "fr",
        "it" => "it",
        "nl" => "nl",
        "ru" => "ru",
        "pl" => "pl",
        "sv" => "se",
        "da" => "dk",
        "cs" => "cz",
        "el" => "gr",
        "uk" => "ua",
        "ja" => "jp",
        "ko" => "kr",
        "ar" => "sa",
        "he" => "il",
        "hi" => "in",
        "tr" => "tr",
        _ => return None,
    })
}
