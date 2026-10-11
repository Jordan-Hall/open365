//! Hunspell-compatible spell checking and suggestions for Word and Excel proofing.
//!
//! A clean-room engine written from the public Hunspell format documentation: affix rules with
//! continuation classes and cross products, `COMPOUNDFLAG`/`COMPOUNDRULE` compounding,
//! `ICONV`/`OCONV`, `IGNORE`, `BREAK`, `NEEDAFFIX`, `KEEPCASE`, `FORBIDDENWORD`, `NOSUGGEST`,
//! `REP`/`MAP`/`KEY`/`TRY` suggestions and legacy single-byte encodings. No dictionary is bundled
//! and nothing is fetched from the network; dictionaries are read from a directory chosen by the
//! caller. See `docs/architecture/spell-check.md` and `docs/research/spell-dictionaries.md`.
//!
//! ```
//! use open365_spell::{Checker, TokenizerOptions};
//!
//! let checker = Checker::from_strs("SET UTF-8\nTRY esianrtolcdugmphbyfvkwz\n", "3\nthe\nquick\nfox\n")?;
//! assert!(checker.check("the"));
//! assert!(!checker.check("teh"));
//! assert_eq!(checker.suggest("teh"), ["the"]);
//! let found = checker.find_misspellings("the quik fox", &TokenizerOptions::default());
//! assert_eq!(found[0].word, "quik");
//! assert_eq!(found[0].utf16_range, 4..8);
//! # Ok::<(), open365_spell::SpellError>(())
//! ```

mod aff;
mod bundled;
mod casing;
mod check;
mod checker;
mod dictionary;
mod error;
mod flags;
mod library;
mod paragraph;
mod proof;
mod session;
mod suggest;
mod tokenize;
mod user;

pub use bundled::bundled_tags;
pub use checker::Checker;
pub use dictionary::Dictionary;
pub use error::SpellError;
pub use library::{DictionaryFiles, DictionaryLibrary, ResolvedDictionary};
pub use paragraph::ProofOptions;
pub use proof::{LangSpan, ParagraphReport, Proofing};
pub use session::{ProofRequest, ProofResponse, ProofSession, ProofWorker};
pub use suggest::SuggestOptions;
pub use tokenize::{IssueKind, Misspelling, TokenizerOptions, Word, find_words};
pub use user::{MAX_FILE_BYTES, MAX_WORD_CHARS, MAX_WORDS, UserDictionary};
