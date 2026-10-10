//! Read-only package summary.

use open365_opc::{
    Compression, ContentTypes, IntegrityIssue, Limits, OpcError, Package, Relationships,
};

/// One part of an inspected package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartInfo {
    pub name: String,
    pub size: usize,
    pub compression: Compression,
    pub content_type: Option<String>,
}

/// Summary of a package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    pub parts: Vec<PartInfo>,
    /// Total relationships across all parseable `.rels` parts.
    pub relationship_count: usize,
    pub integrity_issues: Vec<IntegrityIssue>,
}

/// Reads `bytes` with default limits and summarises the package.
pub fn inspect(bytes: &[u8]) -> Result<Inspection, OpcError> {
    let package = Package::read(bytes, &Limits::default())?;
    Ok(inspect_package(&package))
}

pub(crate) fn inspect_package(package: &Package) -> Inspection {
    let types = package
        .part("[Content_Types].xml")
        .and_then(|p| ContentTypes::parse(p.data()).ok());
    let mut relationship_count = 0;
    let parts = package
        .parts()
        .map(|part| {
            let name = part.name().as_str().to_owned();
            if name.to_ascii_lowercase().ends_with(".rels")
                && let Ok(rels) = Relationships::parse(part.data())
            {
                relationship_count += rels.len();
            }
            PartInfo {
                size: part.data().len(),
                compression: part.compression(),
                content_type: types
                    .as_ref()
                    .and_then(|t| t.content_type_of(part.name()))
                    .map(str::to_owned),
                name,
            }
        })
        .collect();
    Inspection {
        parts,
        relationship_count,
        integrity_issues: package.check_integrity(),
    }
}
