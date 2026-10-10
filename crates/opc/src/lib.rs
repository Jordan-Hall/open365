//! Lossless OPC (Open Packaging Conventions) package graph.
//!
//! A [`Package`] keeps every zip entry, in its original order, with its uncompressed
//! bytes. Entries are only re-serialised by their owner: an untouched part is written
//! back byte-identical (ADR-001). Reading treats input as hostile and enforces
//! [`Limits`] against zip bombs and path tricks (threat model T1).

mod name;
mod package;

pub use name::PartName;
pub use package::{Compression, Limits, Package, Part};

/// Errors from reading or writing a package.
#[derive(Debug, thiserror::Error)]
pub enum OpcError {
    #[error("not a readable zip package: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid part name {name:?}: {reason}")]
    InvalidPartName { name: String, reason: &'static str },
    #[error("duplicate part name {0:?} (part names are case-insensitive)")]
    DuplicatePart(String),
    #[error("package exceeds limit: {0}")]
    LimitExceeded(String),
}
