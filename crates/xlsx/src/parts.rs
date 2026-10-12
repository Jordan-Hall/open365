//! Package bookkeeping the workbook model needs when a part is added or removed:
//! relationship parts and `[Content_Types].xml` are edited as trees and written back.

use std::fmt;

use open365_opc::{Package, PartName, Relationships, TargetMode, rels_part_name, resolve_target};

use crate::XlsxError;
use crate::xml::{self, Attr, ParsedXml, RawXml, XmlElement};

const CONTENT_TYPES: &str = "[Content_Types].xml";

/// Something the first edit changed in the package beyond the cells themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HygieneAction {
    /// A part was removed because the edit made it stale.
    RemovedPart { name: String, reason: &'static str },
    /// A relationship was removed. `source` is `None` for the package-level `_rels/.rels`.
    RemovedRelationship {
        source: Option<String>,
        id: String,
        rel_type: String,
    },
    /// An `Override` entry in `[Content_Types].xml` was removed with its part.
    RemovedContentTypeOverride { part: String },
    /// A part was created because the edit needed it.
    AddedPart { name: String, reason: &'static str },
    /// `<calcPr fullCalcOnLoad="1"/>` was set so the application recalculates on load.
    SetFullCalcOnLoad,
    /// The workbook part's content type was changed in `[Content_Types].xml`.
    ChangedContentType {
        part: String,
        from: String,
        to: String,
    },
}

impl fmt::Display for HygieneAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RemovedPart { name, reason } => write!(f, "removed part {name}: {reason}"),
            Self::RemovedRelationship {
                source,
                id,
                rel_type,
            } => write!(
                f,
                "removed relationship {id} ({rel_type}) of {}",
                source.as_deref().unwrap_or("the package")
            ),
            Self::RemovedContentTypeOverride { part } => {
                write!(f, "removed content type override for /{part}")
            }
            Self::AddedPart { name, reason } => write!(f, "added part {name}: {reason}"),
            Self::SetFullCalcOnLoad => write!(f, "set calcPr fullCalcOnLoad=\"1\""),
            Self::ChangedContentType { part, from, to } => {
                write!(f, "changed content type of {part} from {from} to {to}")
            }
        }
    }
}

fn load_tree(package: &Package, name: &str) -> Result<Option<ParsedXml>, XlsxError> {
    package.part(name).map(|p| xml::parse(p.data())).transpose()
}

/// Removes relationships by id from the rels part of `source` and writes it back.
pub(crate) fn remove_relationships(
    package: &mut Package,
    source: Option<&PartName>,
    ids: &[String],
) -> Result<(), XlsxError> {
    if ids.is_empty() {
        return Ok(());
    }
    let rels_name = rels_part_name(source);
    let Some(mut tree) = load_tree(package, rels_name.as_str())? else {
        return Ok(());
    };
    tree.root.children.retain(|child| match child {
        RawXml::Element(e) if e.head.local_name() == "Relationship" => !e
            .head
            .attr("Id")
            .is_some_and(|id| ids.iter().any(|x| x == id)),
        _ => true,
    });
    package.set_part(rels_name, tree.to_bytes());
    Ok(())
}

/// Adds an internal relationship and returns its new id.
pub(crate) fn add_relationship(
    package: &mut Package,
    source: &PartName,
    rel_type: &str,
    target: &str,
) -> Result<String, XlsxError> {
    let rels_name = rels_part_name(Some(source));
    let mut tree = match load_tree(package, rels_name.as_str())? {
        Some(t) => t,
        None => xml::parse(
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#,
        )?,
    };
    let mut next = 1u32;
    for e in tree.root.elements() {
        if let Some(n) = e
            .head
            .attr("Id")
            .and_then(|id| id.strip_prefix("rId"))
            .and_then(|n| n.parse::<u32>().ok())
        {
            next = next.max(n + 1);
        }
    }
    let id = format!("rId{next}");
    let mut rel = tree.root.sibling("Relationship");
    rel.head.attrs = vec![
        Attr::plain("Id", &id),
        Attr::plain("Type", rel_type),
        Attr::plain("Target", target),
    ];
    tree.root.children.push(RawXml::Element(rel));
    package.set_part(rels_name, tree.to_bytes());
    Ok(id)
}

/// Removes the `Override` for `part` (a name without the leading `/`). True when one existed.
pub(crate) fn remove_override(package: &mut Package, part: &str) -> Result<bool, XlsxError> {
    let Some(mut tree) = load_tree(package, CONTENT_TYPES)? else {
        return Ok(false);
    };
    let want = format!("/{part}").to_ascii_lowercase();
    let before = tree.root.children.len();
    tree.root.children.retain(|child| match child {
        RawXml::Element(e) if e.head.local_name() == "Override" => e
            .head
            .attr("PartName")
            .is_none_or(|n| n.to_ascii_lowercase() != want),
        _ => true,
    });
    if tree.root.children.len() == before {
        return Ok(false);
    }
    let name = PartName::new(CONTENT_TYPES)?;
    package.set_part(name, tree.to_bytes());
    Ok(true)
}

/// Sets the `ContentType` of the `Override` for `part` (name without the leading `/`).
/// True when an override was found.
pub(crate) fn set_override_type(
    package: &mut Package,
    part: &str,
    content_type: &str,
) -> Result<bool, XlsxError> {
    let Some(mut tree) = load_tree(package, CONTENT_TYPES)? else {
        return Err(XlsxError::MissingPart(CONTENT_TYPES.to_owned()));
    };
    let want = format!("/{part}").to_ascii_lowercase();
    let mut found = false;
    for child in &mut tree.root.children {
        let RawXml::Element(e) = child else { continue };
        let is_target = e.head.local_name() == "Override"
            && e.head
                .attr("PartName")
                .is_some_and(|n| n.to_ascii_lowercase() == want);
        if !is_target {
            continue;
        }
        for a in &mut e.head.attrs {
            if a.name == "ContentType" {
                content_type.clone_into(&mut a.value);
                found = true;
            }
        }
    }
    if found {
        let name = PartName::new(CONTENT_TYPES)?;
        package.set_part(name, tree.to_bytes());
    }
    Ok(found)
}

/// Adds an `Override` for `part` (name without the leading `/`).
pub(crate) fn add_override(
    package: &mut Package,
    part: &str,
    content_type: &str,
) -> Result<(), XlsxError> {
    let Some(mut tree) = load_tree(package, CONTENT_TYPES)? else {
        return Err(XlsxError::MissingPart(CONTENT_TYPES.to_owned()));
    };
    let mut el: XmlElement = tree.root.sibling("Override");
    el.head.attrs = vec![
        Attr::plain("PartName", &format!("/{part}")),
        Attr::plain("ContentType", content_type),
    ];
    tree.root.children.push(RawXml::Element(el));
    let name = PartName::new(CONTENT_TYPES)?;
    package.set_part(name, tree.to_bytes());
    Ok(())
}

/// Removes a part together with its relationship entry and content-type override.
fn remove_part_fully(
    package: &mut Package,
    source: Option<&PartName>,
    rel_id: &str,
    rel_type: &str,
    target: Option<&PartName>,
    reason: &'static str,
    log: &mut Vec<HygieneAction>,
) -> Result<(), XlsxError> {
    log.push(HygieneAction::RemovedRelationship {
        source: source.map(ToString::to_string),
        id: rel_id.to_owned(),
        rel_type: rel_type.to_owned(),
    });
    remove_relationships(package, source, &[rel_id.to_owned()])?;
    if let Some(target) = target {
        if package.remove_part(target.as_str()).is_some() {
            log.push(HygieneAction::RemovedPart {
                name: target.to_string(),
                reason,
            });
        }
        if remove_override(package, target.as_str())? {
            log.push(HygieneAction::RemovedContentTypeOverride {
                part: target.to_string(),
            });
        }
    }
    Ok(())
}

/// Removes what the first edit makes stale: the calculation chain (it lists formula
/// cells in an order that no longer holds), the thumbnail and digital signatures.
/// Each part goes together with its relationship and content-type override.
pub(crate) fn invalidate_stale_parts(
    package: &mut Package,
    workbook: &PartName,
    log: &mut Vec<HygieneAction>,
) -> Result<(), XlsxError> {
    // calcChain: from the workbook's own relationships.
    let wb_rels_name = rels_part_name(Some(workbook));
    if let Some(part) = package.part(wb_rels_name.as_str())
        && let Ok(rels) = Relationships::parse(part.data())
    {
        let doomed: Vec<_> = rels
            .iter()
            .filter(|r| r.target_mode == TargetMode::Internal && r.rel_type.ends_with("/calcChain"))
            .cloned()
            .collect();
        for rel in doomed {
            let target = resolve_target(Some(workbook), &rel.target).ok();
            remove_part_fully(
                package,
                Some(workbook),
                &rel.id,
                &rel.rel_type,
                target.as_ref(),
                "cell calculation order is stale after an edit",
                log,
            )?;
        }
    }

    // Thumbnail and signatures: from the package relationships.
    let root_rels = rels_part_name(None);
    if let Some(part) = package.part(root_rels.as_str())
        && let Ok(rels) = Relationships::parse(part.data())
    {
        for rel in rels.iter().cloned().collect::<Vec<_>>() {
            let thumb = rel.rel_type.ends_with("/metadata/thumbnail");
            let sig = rel.rel_type.ends_with("/digital-signature/origin");
            if !thumb && !sig {
                continue;
            }
            let target = (rel.target_mode == TargetMode::Internal)
                .then(|| resolve_target(None, &rel.target).ok())
                .flatten();
            remove_part_fully(
                package,
                None,
                &rel.id,
                &rel.rel_type,
                target.as_ref(),
                if thumb {
                    "thumbnail shows the pre-edit workbook"
                } else {
                    "digital signature no longer matches the edited package"
                },
                log,
            )?;
        }
    }
    let signature_parts: Vec<String> = package
        .parts()
        .map(|p| p.name().to_string())
        .filter(|n| n.to_ascii_lowercase().starts_with("_xmlsignatures/"))
        .collect();
    for name in signature_parts {
        if package.remove_part(&name).is_some() {
            log.push(HygieneAction::RemovedPart {
                name: name.clone(),
                reason: "digital signature no longer matches the edited package",
            });
        }
        if remove_override(package, &name)? {
            log.push(HygieneAction::RemovedContentTypeOverride { part: name });
        }
    }
    Ok(())
}

/// Adds a `Default` extension entry to `[Content_Types].xml` unless the extension has one.
pub(crate) fn add_default(
    package: &mut Package,
    extension: &str,
    content_type: &str,
) -> Result<(), XlsxError> {
    let Some(mut tree) = load_tree(package, CONTENT_TYPES)? else {
        return Err(XlsxError::MissingPart(CONTENT_TYPES.to_owned()));
    };
    let exists = tree.root.elements().any(|e| {
        e.head.local_name() == "Default"
            && e.head
                .attr("Extension")
                .is_some_and(|x| x.eq_ignore_ascii_case(extension))
    });
    if exists {
        return Ok(());
    }
    let mut el: XmlElement = tree.root.sibling("Default");
    el.head.attrs = vec![
        Attr::plain("Extension", extension),
        Attr::plain("ContentType", content_type),
    ];
    // Excel lists the defaults before the overrides.
    let at = tree
        .root
        .children
        .iter()
        .position(|c| {
            c.as_element()
                .is_some_and(|e| e.head.local_name() == "Override")
        })
        .unwrap_or(tree.root.children.len());
    tree.root.children.insert(at, RawXml::Element(el));
    let name = PartName::new(CONTENT_TYPES)?;
    package.set_part(name, tree.to_bytes());
    Ok(())
}

/// The content type of `part` as `[Content_Types].xml` gives it (override, then extension default).
pub(crate) fn content_type_of(package: &Package, part: &str) -> Option<String> {
    let types = package.part(CONTENT_TYPES)?;
    let ct = open365_opc::ContentTypes::parse(types.data()).ok()?;
    let name = PartName::new(part).ok()?;
    ct.content_type_of(&name).map(str::to_owned)
}

/// Adds a relationship of either mode and returns its new id (`rId` plus the next free number).
pub(crate) fn add_relationship_mode(
    package: &mut Package,
    source: Option<&PartName>,
    rel_type: &str,
    target: &str,
    external: bool,
) -> Result<String, XlsxError> {
    let rels_name = rels_part_name(source);
    let mut tree = match load_tree(package, rels_name.as_str())? {
        Some(t) => t,
        None => xml::parse(
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#,
        )?,
    };
    let mut next = 1u32;
    for e in tree.root.elements() {
        if let Some(n) = e
            .head
            .attr("Id")
            .and_then(|id| id.strip_prefix("rId"))
            .and_then(|n| n.parse::<u32>().ok())
        {
            next = next.max(n + 1);
        }
    }
    let id = format!("rId{next}");
    let mut rel = tree.root.sibling("Relationship");
    rel.head.attrs = vec![
        Attr::plain("Id", &id),
        Attr::plain("Type", rel_type),
        Attr::plain("Target", target),
    ];
    if external {
        rel.head.attrs.push(Attr::plain("TargetMode", "External"));
    }
    tree.root.children.push(RawXml::Element(rel));
    package.set_part(rels_name, tree.to_bytes());
    Ok(id)
}
