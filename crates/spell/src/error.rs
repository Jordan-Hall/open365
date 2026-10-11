//! Error type for dictionary parsing and loading.

use std::fmt;
use std::path::PathBuf;

/// Failure to parse or locate a dictionary.
#[derive(Debug)]
pub enum SpellError {
    /// Reading a dictionary file failed.
    Io {
        /// The file that could not be read.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// The `.aff` file declares an encoding this crate cannot decode.
    UnsupportedEncoding(String),
    /// The `.aff` file is malformed.
    Aff {
        /// One-based line number.
        line: usize,
        /// What is wrong.
        message: String,
    },
    /// The `.dic` file is malformed.
    Dic {
        /// One-based line number.
        line: usize,
        /// What is wrong.
        message: String,
    },
    /// No dictionary in the library matches the tag or any of its fallbacks.
    NoDictionary {
        /// The requested BCP-47 tag.
        tag: String,
    },
    /// A dictionary was found but failed to load; the message is the original error.
    Load {
        /// The dictionary that failed (`en-GB`, or a file name).
        dictionary: String,
        /// What went wrong.
        message: String,
    },
}

impl fmt::Display for SpellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "cannot read {}: {source}", path.display()),
            Self::UnsupportedEncoding(name) => {
                write!(f, "unsupported dictionary encoding `{name}`")
            }
            Self::Aff { line, message } => write!(f, ".aff line {line}: {message}"),
            Self::Dic { line, message } => write!(f, ".dic line {line}: {message}"),
            Self::NoDictionary { tag } => write!(f, "no dictionary found for `{tag}`"),
            Self::Load {
                dictionary,
                message,
            } => write!(f, "cannot load dictionary `{dictionary}`: {message}"),
        }
    }
}

impl std::error::Error for SpellError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
