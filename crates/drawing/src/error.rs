//! Parse errors.

use std::fmt;

/// Why a fragment could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// Malformed XML.
    Xml(String),
    /// The fragment carried a DOCTYPE.
    Doctype,
    /// Nesting beyond the limit.
    TooDeep,
    /// A geometry definition is invalid (bad formula, wrong argument count).
    Geometry(String),
    /// The root element is not a drawing object this crate reads.
    UnsupportedRoot(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Xml(m) => write!(f, "malformed XML: {m}"),
            Self::Doctype => f.write_str("DOCTYPE is not allowed"),
            Self::TooDeep => f.write_str("element nesting too deep"),
            Self::Geometry(m) => write!(f, "invalid geometry: {m}"),
            Self::UnsupportedRoot(n) => write!(f, "unsupported drawing root element `{n}`"),
        }
    }
}

impl std::error::Error for ParseError {}
