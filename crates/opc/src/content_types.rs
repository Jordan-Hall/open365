//! Read-only parsing of `[Content_Types].xml` and a package integrity check.

use std::collections::HashMap;

use crate::main_part::{CONTENT_TYPES_PART, MainPartKind, OFFICE_DOCUMENT_RELS};
use crate::rels::{
    Relationships, TargetMode, attr, rels_part_name, rels_source, resolve_target, visit_elements,
};
use crate::{OpcError, Package, PartName};

/// Parsed `[Content_Types].xml`: extension defaults plus per-part overrides.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContentTypes {
    /// Lower-cased extension (no dot) to content type.
    defaults: HashMap<String, String>,
    /// Lower-cased part name (no leading `/`) to content type.
    overrides: HashMap<String, String>,
}

impl ContentTypes {
    /// Parses `[Content_Types].xml`. Rejects DOCTYPEs and malformed entries.
    pub fn parse(xml: &[u8]) -> Result<Self, OpcError> {
        let malformed = |msg: &str| OpcError::MalformedPart(msg.to_owned());
        let mut types = Self::default();
        let mut has_root = false;
        visit_elements(xml, |element, first| {
            let name = element.local_name();
            if first {
                has_root = true;
                return if name.as_ref() == "Types" {
                    Ok(())
                } else {
                    Err(malformed("root element is not <Types>"))
                };
            }
            let content_type =
                || attr(element, "ContentType")?.ok_or_else(|| malformed("missing ContentType"));
            match name.as_ref() {
                "Default" => {
                    let ext = attr(element, "Extension")?
                        .ok_or_else(|| malformed("<Default> missing Extension"))?;
                    types
                        .defaults
                        .insert(ext.to_ascii_lowercase(), content_type()?);
                }
                "Override" => {
                    let part = attr(element, "PartName")?
                        .and_then(|p| p.strip_prefix('/').map(str::to_ascii_lowercase))
                        .ok_or_else(|| malformed("<Override> PartName missing or not absolute"))?;
                    types.overrides.insert(part, content_type()?);
                }
                _ => {}
            }
            Ok(())
        })?;
        if !has_root {
            return Err(malformed("root element is not <Types>"));
        }
        Ok(types)
    }

    /// Extension defaults as sorted `(extension, content type)` pairs.
    pub fn defaults(&self) -> Vec<(&str, &str)> {
        let mut v: Vec<_> = self
            .defaults
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        v.sort_unstable();
        v
    }

    /// Part overrides as sorted `(lower-cased part name, content type)` pairs.
    pub fn overrides(&self) -> Vec<(&str, &str)> {
        let mut v: Vec<_> = self
            .overrides
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        v.sort_unstable();
        v
    }

    /// The content type of `part`: an override wins over the extension default.
    pub fn content_type_of(&self, part: &PartName) -> Option<&str> {
        if let Some(ct) = self.overrides.get(&part.key()) {
            return Some(ct);
        }
        let file = part.as_str().rsplit('/').next()?;
        let (_, ext) = file.rsplit_once('.')?;
        self.defaults
            .get(&ext.to_ascii_lowercase())
            .map(String::as_str)
    }
}

/// A problem found by [`Package::check_integrity`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IntegrityIssue {
    /// The package has no `[Content_Types].xml`.
    MissingContentTypes,
    /// `[Content_Types].xml` could not be parsed.
    MalformedContentTypes { error: String },
    /// A `.rels` part could not be parsed or is not shaped `dir/_rels/name.rels`.
    MalformedRels { part: PartName, error: String },
    /// An internal relationship points at a part that does not exist.
    DanglingRelationship {
        /// `None` for package-level relationships.
        source: Option<PartName>,
        id: String,
        target: String,
    },
    /// An internal relationship target is not a valid part path (or escapes the root).
    InvalidTarget {
        source: Option<PartName>,
        id: String,
        target: String,
    },
    /// A part has neither an override nor a default content type.
    MissingContentType { part: PartName },
    /// An `<Override>` names a part that is not in the package.
    DanglingOverride {
        /// Lower-cased part name, no leading `/`.
        part: String,
    },
    /// The part the package-level `officeDocument` relationship points at has a content
    /// type that is none of the [`MainPartKind`] types (or none at all).
    UnrecognisedMainPart {
        part: PartName,
        content_type: Option<String>,
    },
}

impl Package {
    /// The main part (the target of the package-level `officeDocument` relationship) and
    /// the kind its content type declares. `None` when the package has no readable
    /// `officeDocument` relationship, or the type is not a [`MainPartKind`].
    pub fn main_part_kind(&self) -> Option<(PartName, MainPartKind)> {
        let types = ContentTypes::parse(self.part(CONTENT_TYPES_PART)?.data()).ok()?;
        let (name, content_type) = self.main_part_type(&types)?;
        let kind = MainPartKind::from_content_type(content_type.as_deref()?)?;
        Some((name, kind))
    }

    fn main_part_type(&self, types: &ContentTypes) -> Option<(PartName, Option<String>)> {
        let rels = Relationships::parse(self.part(rels_part_name(None).as_str())?.data()).ok()?;
        rels.iter()
            .filter(|r| r.target_mode == TargetMode::Internal)
            .filter(|r| OFFICE_DOCUMENT_RELS.contains(&r.rel_type.as_str()))
            .filter_map(|r| resolve_target(None, &r.target).ok())
            .find(|t| self.part(t.as_str()).is_some())
            .map(|t| {
                let ct = types.content_type_of(&t).map(str::to_owned);
                (t, ct)
            })
    }

    /// Inspects the package without modifying it. External relationship targets are
    /// never fetched or resolved.
    pub fn check_integrity(&self) -> Vec<IntegrityIssue> {
        let mut issues = Vec::new();

        let content_types = match self.part("[Content_Types].xml") {
            None => {
                issues.push(IntegrityIssue::MissingContentTypes);
                None
            }
            Some(part) => match ContentTypes::parse(part.data()) {
                Ok(types) => Some(types),
                Err(error) => {
                    issues.push(IntegrityIssue::MalformedContentTypes {
                        error: error.to_string(),
                    });
                    None
                }
            },
        };

        if let Some(types) = &content_types {
            for (part, _) in types.overrides() {
                if self.part(part).is_none() {
                    issues.push(IntegrityIssue::DanglingOverride {
                        part: part.to_owned(),
                    });
                }
            }
            if let Some((part, content_type)) = self.main_part_type(types)
                && content_type
                    .as_deref()
                    .and_then(MainPartKind::from_content_type)
                    .is_none()
            {
                issues.push(IntegrityIssue::UnrecognisedMainPart { part, content_type });
            }
        }

        for part in self.parts() {
            let name = part.name();
            if let Some(types) = &content_types
                && !name.as_str().eq_ignore_ascii_case("[Content_Types].xml")
                && types.content_type_of(name).is_none()
            {
                issues.push(IntegrityIssue::MissingContentType { part: name.clone() });
            }

            let is_rels = name.as_str().len() >= 5
                && name.as_str().as_bytes()[name.as_str().len() - 5..]
                    .eq_ignore_ascii_case(b".rels");
            if !is_rels {
                continue;
            }
            let Some(source) = rels_source(name) else {
                issues.push(IntegrityIssue::MalformedRels {
                    part: name.clone(),
                    error: "not located at dir/_rels/name.rels".to_owned(),
                });
                continue;
            };
            let rels = match Relationships::parse(part.data()) {
                Ok(rels) => rels,
                Err(error) => {
                    issues.push(IntegrityIssue::MalformedRels {
                        part: name.clone(),
                        error: error.to_string(),
                    });
                    continue;
                }
            };
            for rel in rels
                .iter()
                .filter(|r| r.target_mode == TargetMode::Internal)
            {
                match resolve_target(source.as_ref(), &rel.target) {
                    Ok(target) if self.part(target.as_str()).is_some() => {}
                    Ok(_) => issues.push(IntegrityIssue::DanglingRelationship {
                        source: source.clone(),
                        id: rel.id.clone(),
                        target: rel.target.clone(),
                    }),
                    Err(_) => issues.push(IntegrityIssue::InvalidTarget {
                        source: source.clone(),
                        id: rel.id.clone(),
                        target: rel.target.clone(),
                    }),
                }
            }
        }
        issues
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TYPES: &[u8] = br#"<?xml version="1.0"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="XML" ContentType="application/xml"/>
  <Default Extension="png" ContentType="image/png"/>
  <Override PartName="/word/document.xml" ContentType="application/doc+xml"/>
</Types>"#;

    fn name(s: &str) -> PartName {
        PartName::new(s).unwrap()
    }

    #[test]
    fn override_wins_over_default() {
        let types = ContentTypes::parse(TYPES).unwrap();
        assert_eq!(
            types.content_type_of(&name("word/document.xml")),
            Some("application/doc+xml")
        );
        assert_eq!(
            types.content_type_of(&name("WORD/Document.xml")),
            Some("application/doc+xml")
        );
        assert_eq!(
            types.content_type_of(&name("word/styles.xml")),
            Some("application/xml")
        );
        assert_eq!(
            types.content_type_of(&name("word/media/a.PNG")),
            Some("image/png")
        );
        assert_eq!(types.content_type_of(&name("word/data.bin")), None);
        assert_eq!(types.content_type_of(&name("noext")), None);
    }

    #[test]
    fn rejects_doctype_and_malformed() {
        assert!(matches!(
            ContentTypes::parse(b"<!DOCTYPE Types><Types/>"),
            Err(OpcError::DoctypeForbidden)
        ));
        assert!(ContentTypes::parse(b"<Other/>").is_err());
        assert!(
            ContentTypes::parse(br#"<Types><Override PartName="rel" ContentType="x"/></Types>"#)
                .is_err()
        );
    }

    fn package(rels_target: &str, extra: Option<&str>) -> Package {
        let mut package = Package::default();
        package.set_part(name("[Content_Types].xml"), TYPES.to_vec());
        let rels = format!(
            r#"<Relationships>
<Relationship Id="rId1" Type="t" Target="word/document.xml"/>
<Relationship Id="rId2" Type="t" Target="https://example.com/missing" TargetMode="External"/>
<Relationship Id="rId3" Type="t" Target="{rels_target}"/>
</Relationships>"#
        );
        package.set_part(name("_rels/.rels"), rels.into_bytes());
        package.set_part(name("word/document.xml"), b"<d/>".to_vec());
        package.set_part(name("word/media/a.png"), b"x".to_vec());
        if let Some(extra) = extra {
            package.set_part(name(extra), b"x".to_vec());
        }
        package
    }

    #[test]
    fn consistent_package_has_no_issues() {
        assert_eq!(
            package("word/media/a.png", None).check_integrity(),
            Vec::new()
        );
    }

    #[test]
    fn reports_dangling_and_missing_content_type() {
        let issues = package("word/media/gone.png", Some("word/data.bin")).check_integrity();
        assert_eq!(
            issues,
            vec![
                IntegrityIssue::DanglingRelationship {
                    source: None,
                    id: "rId3".to_owned(),
                    target: "word/media/gone.png".to_owned(),
                },
                IntegrityIssue::MissingContentType {
                    part: name("word/data.bin")
                },
            ]
        );
    }

    #[test]
    fn reports_malformed_rels_missing_types_and_bad_target() {
        let mut package = package("../../x", None);
        package.set_part(name("word/_rels/document.xml.rels"), b"<nope".to_vec());
        let issues = package.check_integrity();
        assert!(issues.contains(&IntegrityIssue::InvalidTarget {
            source: None,
            id: "rId3".to_owned(),
            target: "../../x".to_owned(),
        }));
        assert!(
            issues
                .iter()
                .any(|i| matches!(i, IntegrityIssue::MalformedRels { .. }))
        );
        package.remove_part("[Content_Types].xml");
        assert!(
            package
                .check_integrity()
                .contains(&IntegrityIssue::MissingContentTypes)
        );
    }

    fn office_package(kind: MainPartKind, vba: bool) -> Package {
        let dir = if kind.is_word() { "word" } else { "xl" };
        let main = if kind.is_word() {
            "document.xml"
        } else {
            "workbook.xml"
        };
        let mut types = format!(
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="bin" ContentType="{}"/><Override PartName="/{dir}/{main}" ContentType="{}"/><Override PartName="/{dir}/vbaData.xml" ContentType="{}"/></Types>"#,
            crate::VBA_PROJECT_CONTENT_TYPE,
            kind.content_type(),
            crate::VBA_DATA_CONTENT_TYPE,
        );
        if !vba {
            types = types.replace(crate::VBA_DATA_CONTENT_TYPE, "application/xml");
        }
        let mut package = Package::default();
        package.set_part(name("[Content_Types].xml"), types.into_bytes());
        package.set_part(
            name("_rels/.rels"),
            format!(
                r#"<Relationships><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="{dir}/{main}"/></Relationships>"#
            )
            .into_bytes(),
        );
        package.set_part(name(&format!("{dir}/{main}")), b"<x/>".to_vec());
        package.set_part(name(&format!("{dir}/vbaData.xml")), b"<v/>".to_vec());
        package.set_part(name(&format!("{dir}/vbaProject.bin")), b"x".to_vec());
        package
    }

    #[test]
    fn every_main_part_kind_is_recognised_without_issues() {
        for kind in MainPartKind::ALL {
            for vba in [false, true] {
                let package = office_package(kind, vba);
                assert_eq!(package.check_integrity(), Vec::new(), "{kind:?}");
                assert_eq!(package.main_part_kind().map(|(_, k)| k), Some(kind));
            }
        }
    }

    #[test]
    fn unknown_main_type_and_dangling_override_are_reported() {
        let mut package = office_package(MainPartKind::Docx, true);
        package.remove_part("word/vbaData.xml");
        assert_eq!(
            package.check_integrity(),
            vec![IntegrityIssue::DanglingOverride {
                part: "word/vbadata.xml".to_owned()
            }]
        );
        package.remove_part_and_override("word/vbaData.xml");
        assert_eq!(package.check_integrity(), Vec::new());

        let types = package.part("[Content_Types].xml").unwrap().data().to_vec();
        let types = String::from_utf8(types)
            .unwrap()
            .replace(MainPartKind::Docx.content_type(), "application/xml");
        package.set_part(name("[Content_Types].xml"), types.into_bytes());
        assert!(matches!(
            package.check_integrity().as_slice(),
            [IntegrityIssue::UnrecognisedMainPart { .. }]
        ));
        assert_eq!(package.main_part_kind(), None);
    }
}
