//! Read-only package summary.

use open365_opc::{
    Compression, ContentTypes, IntegrityIssue, Limits, OpcError, Package, Relationships, TargetMode,
};

/// One part of an inspected package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartInfo {
    pub name: String,
    pub size: usize,
    pub compression: Compression,
    pub content_type: Option<String>,
}

/// One relationship of an inspected package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelInfo {
    /// The `.rels` part it was read from.
    pub rels_part: String,
    pub id: String,
    pub rel_type: String,
    pub target: String,
    pub external: bool,
}

/// Summary of a package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    pub parts: Vec<PartInfo>,
    /// Total relationships across all parseable `.rels` parts.
    pub relationship_count: usize,
    pub relationships: Vec<RelInfo>,
    /// `(extension, content type)` defaults.
    pub default_types: Vec<(String, String)>,
    /// `(part name, content type)` overrides, names lower-cased.
    pub override_types: Vec<(String, String)>,
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
    let mut relationships = Vec::new();
    let parts = package
        .parts()
        .map(|part| {
            let name = part.name().as_str().to_owned();
            if name.to_ascii_lowercase().ends_with(".rels")
                && let Ok(rels) = Relationships::parse(part.data())
            {
                for r in rels.iter() {
                    relationships.push(RelInfo {
                        rels_part: name.clone(),
                        id: r.id.clone(),
                        rel_type: r.rel_type.clone(),
                        target: r.target.clone(),
                        external: r.target_mode == TargetMode::External,
                    });
                }
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
    let owned = |v: Vec<(&str, &str)>| {
        v.into_iter()
            .map(|(a, b)| (a.to_owned(), b.to_owned()))
            .collect::<Vec<_>>()
    };
    Inspection {
        parts,
        relationship_count: relationships.len(),
        relationships,
        default_types: types
            .as_ref()
            .map(|t| owned(t.defaults()))
            .unwrap_or_default(),
        override_types: types
            .as_ref()
            .map(|t| owned(t.overrides()))
            .unwrap_or_default(),
        integrity_issues: package.check_integrity(),
    }
}
