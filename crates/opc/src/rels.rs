//! Read-only parsing of OPC relationship parts (`*.rels`).
//!
//! These types are for inspection only. Relationship and content-type parts are never
//! re-serialised here: parts stay byte-identical unless their owner edits them (ADR-001).

use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;

use crate::{OpcError, PartName};

/// Whether a relationship target is a package part or an external URI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetMode {
    Internal,
    External,
}

/// One `<Relationship>` element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relationship {
    pub id: String,
    pub rel_type: String,
    pub target: String,
    pub target_mode: TargetMode,
}

/// The relationships of one source part (or of the package itself).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Relationships {
    items: Vec<Relationship>,
}

/// Calls `f` for every element in document order (`first` is true for the root). Any
/// DOCTYPE is rejected outright (threat model T1): we never process DTDs or entity
/// declarations. Nothing is allocated per element; `f` decodes only the attributes it
/// needs with [`attr`].
pub(crate) fn visit_elements(
    xml: &[u8],
    mut f: impl FnMut(&BytesStart<'_>, bool) -> Result<(), OpcError>,
) -> Result<(), OpcError> {
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut first = true;
    loop {
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|e| OpcError::Xml(e.to_string()))?;
        match event {
            Event::DocType(_) => return Err(OpcError::DoctypeForbidden),
            Event::Start(e) | Event::Empty(e) => {
                f(&e, first)?;
                first = false;
            }
            Event::Eof => return Ok(()),
            _ => {}
        }
        buf.clear();
    }
}

/// The unescaped value of the attribute whose local name is `key`. Malformed attribute
/// syntax anywhere on the element is an error.
pub(crate) fn attr(e: &BytesStart<'_>, key: &str) -> Result<Option<String>, OpcError> {
    let mut found = None;
    for attr in e.attributes() {
        let attr = attr.map_err(|e| OpcError::Xml(e.to_string()))?;
        if found.is_none() && attr.key.local_name().as_ref() == key {
            found = Some(
                attr.normalized_value(XmlVersion::Explicit1_0)
                    .map_err(|e| OpcError::Xml(e.to_string()))?
                    .into_owned(),
            );
        }
    }
    Ok(found)
}

impl Relationships {
    /// Parses a relationships part. Rejects DOCTYPEs and malformed entries.
    pub fn parse(xml: &[u8]) -> Result<Self, OpcError> {
        let malformed = |msg: &str| OpcError::MalformedPart(msg.to_owned());
        let mut items = Vec::new();
        let mut has_root = false;
        visit_elements(xml, |element, first| {
            let name = element.local_name();
            if first {
                has_root = true;
                return if name.as_ref() == "Relationships" {
                    Ok(())
                } else {
                    Err(malformed("root element is not <Relationships>"))
                };
            }
            if name.as_ref() != "Relationship" {
                return Ok(());
            }
            let get = |key: &str| {
                attr(element, key)?
                    .ok_or_else(|| malformed(&format!("<Relationship> missing {key}")))
            };
            let target_mode = match attr(element, "TargetMode")?.as_deref() {
                None | Some("Internal") => TargetMode::Internal,
                Some("External") => TargetMode::External,
                Some(_) => return Err(malformed("unknown TargetMode")),
            };
            items.push(Relationship {
                id: get("Id")?,
                rel_type: get("Type")?,
                target: get("Target")?,
                target_mode,
            });
            Ok(())
        })?;
        if !has_root {
            return Err(malformed("root element is not <Relationships>"));
        }
        Ok(Self { items })
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Relationship> {
        self.items.iter()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Looks up a relationship by its `Id` (case-sensitive per the spec).
    pub fn get(&self, id: &str) -> Option<&Relationship> {
        self.items.iter().find(|r| r.id == id)
    }
}

impl<'a> IntoIterator for &'a Relationships {
    type Item = &'a Relationship;
    type IntoIter = std::slice::Iter<'a, Relationship>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Name of the relationships part for `source` (`None` is the package itself):
/// `dir/name.ext` maps to `dir/_rels/name.ext.rels`; the package maps to `_rels/.rels`.
pub fn rels_part_name(source: Option<&PartName>) -> PartName {
    let name = match source {
        None => "_rels/.rels".to_owned(),
        Some(src) => match src.as_str().rsplit_once('/') {
            Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
            None => format!("_rels/{}.rels", src.as_str()),
        },
    };
    // A valid source name yields a valid rels name (same segments plus fixed ones).
    PartName::new(&name).expect("derived rels part name is valid")
}

/// The source part a rels part belongs to; `Ok(None)` for the package-level
/// `_rels/.rels`. `None` when `rels` is not shaped like `dir/_rels/name.ext.rels`.
pub(crate) fn rels_source(rels: &PartName) -> Option<Option<PartName>> {
    let lower = rels.as_str().to_ascii_lowercase();
    if lower == "_rels/.rels" {
        return Some(None);
    }
    let (dir, file) = rels.as_str().rsplit_once('/')?;
    let (parent, rels_dir) = match dir.rsplit_once('/') {
        Some((parent, last)) => (Some(parent), last),
        None => (None, dir),
    };
    if !rels_dir.eq_ignore_ascii_case("_rels") {
        return None;
    }
    let stem = file.get(..file.len().checked_sub(5)?)?;
    if !file[stem.len()..].eq_ignore_ascii_case(".rels") || stem.is_empty() {
        return None;
    }
    let source = match parent {
        Some(parent) => format!("{parent}/{stem}"),
        None => stem.to_owned(),
    };
    PartName::new(&source).ok().map(Some)
}

/// Resolves an internal relationship `target` to a part name. Relative targets are
/// resolved against the source part's directory; a leading `/` is package-absolute.
/// `.` and `..` segments are collapsed; escaping the package root is an error.
///
/// Percent-decoding and URI fragments are not handled yet: targets are treated as
/// literal part paths.
pub fn resolve_target(source: Option<&PartName>, target: &str) -> Result<PartName, OpcError> {
    let escapes = || OpcError::TargetEscapesRoot(target.to_owned());
    let mut segments: Vec<&str> = Vec::new();
    if !target.starts_with('/')
        && let Some(src) = source
        && let Some((dir, _)) = src.as_str().rsplit_once('/')
    {
        segments.extend(dir.split('/'));
    }
    for segment in target.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop().ok_or_else(escapes)?;
            }
            other => segments.push(other),
        }
    }
    PartName::new(&segments.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://x/image" Target="media/image1.png"/>
  <Relationship Id="rId2" Type="http://x/hyperlink" Target="https://example.com/?a=1&amp;b=2" TargetMode="External"/>
</Relationships>"#;

    fn name(s: &str) -> PartName {
        PartName::new(s).unwrap()
    }

    #[test]
    fn parses_internal_and_external() {
        let rels = Relationships::parse(SAMPLE).unwrap();
        assert_eq!(rels.len(), 2);
        assert!(!rels.is_empty());
        let r1 = rels.get("rId1").unwrap();
        assert_eq!(r1.target_mode, TargetMode::Internal);
        assert_eq!(r1.rel_type, "http://x/image");
        let r2 = rels.get("rId2").unwrap();
        assert_eq!(r2.target_mode, TargetMode::External);
        assert_eq!(r2.target, "https://example.com/?a=1&b=2");
        assert!(rels.get("nope").is_none());
        assert_eq!((&rels).into_iter().count(), 2);
    }

    #[test]
    fn rejects_doctype() {
        let xml = br#"<!DOCTYPE r [<!ENTITY x SYSTEM "file:///etc/passwd">]><Relationships/>"#;
        assert!(matches!(
            Relationships::parse(xml),
            Err(OpcError::DoctypeForbidden)
        ));
    }

    #[test]
    fn rejects_missing_attribute_wrong_root_and_bad_xml() {
        let missing = br#"<Relationships><Relationship Id="a" Type="t"/></Relationships>"#;
        assert!(matches!(
            Relationships::parse(missing),
            Err(OpcError::MalformedPart(_))
        ));
        assert!(Relationships::parse(b"<Other/>").is_err());
        assert!(Relationships::parse(b"<Relationships><a></Relationships>").is_err());
    }

    #[test]
    fn rels_part_names() {
        assert_eq!(rels_part_name(None).as_str(), "_rels/.rels");
        assert_eq!(
            rels_part_name(Some(&name("word/document.xml"))).as_str(),
            "word/_rels/document.xml.rels"
        );
        assert_eq!(
            rels_part_name(Some(&name("a.xml"))).as_str(),
            "_rels/a.xml.rels"
        );
    }

    #[test]
    fn rels_source_inverts_rels_part_name() {
        assert_eq!(rels_source(&name("_rels/.rels")), Some(None));
        let doc = name("word/document.xml");
        assert_eq!(rels_source(&rels_part_name(Some(&doc))), Some(Some(doc)));
        let top = name("a.xml");
        assert_eq!(rels_source(&rels_part_name(Some(&top))), Some(Some(top)));
        assert_eq!(rels_source(&name("word/stray.rels")), None);
    }

    #[test]
    fn resolves_targets() {
        let doc = name("word/document.xml");
        assert_eq!(
            resolve_target(Some(&doc), "media/image1.png")
                .unwrap()
                .as_str(),
            "word/media/image1.png"
        );
        assert_eq!(
            resolve_target(Some(&doc), "../xl/workbook.xml")
                .unwrap()
                .as_str(),
            "xl/workbook.xml"
        );
        let deep = name("word/charts/chart1.xml");
        assert_eq!(
            resolve_target(Some(&deep), "../media/image1.png")
                .unwrap()
                .as_str(),
            "word/media/image1.png"
        );
        assert_eq!(
            resolve_target(Some(&doc), "/xl/workbook.xml")
                .unwrap()
                .as_str(),
            "xl/workbook.xml"
        );
        assert_eq!(
            resolve_target(None, "word/document.xml").unwrap().as_str(),
            "word/document.xml"
        );
    }

    #[test]
    fn rejects_escaping_target() {
        let doc = name("word/document.xml");
        assert!(matches!(
            resolve_target(Some(&doc), "../../x"),
            Err(OpcError::TargetEscapesRoot(_))
        ));
        assert!(matches!(
            resolve_target(None, "../x"),
            Err(OpcError::TargetEscapesRoot(_))
        ));
    }
}
