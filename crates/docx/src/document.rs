use std::fmt;
use std::ops::Range;

use open365_opc::{
    Limits, Package, PartName, Relationships, TargetMode, rels_part_name, resolve_target,
};

use crate::DocxError;
use crate::model::{Body, DocumentRoot, Paragraph};
use crate::xml::{self, RawXml};

const OFFICE_DOCUMENT_TYPES: [&str; 2] = [
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument",
    "http://purl.oclc.org/ooxml/officeDocument/relationships/officeDocument",
];

/// Content the parser could not represent. A non-empty log blocks [`Document::save`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DroppedItem {
    /// Where it was found (part name and a short path).
    pub location: String,
    pub description: String,
}

/// Something removed from the package because the first edit made it stale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HygieneAction {
    /// A part was removed.
    RemovedPart { name: String, reason: &'static str },
    /// A package-level relationship was removed.
    RemovedRelationship { id: String, rel_type: String },
}

impl fmt::Display for HygieneAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RemovedPart { name, reason } => write!(f, "removed part {name}: {reason}"),
            Self::RemovedRelationship { id, rel_type } => {
                write!(f, "removed relationship {id} ({rel_type})")
            }
        }
    }
}

/// A WordprocessingML document: the OPC package plus a typed model of the main story.
/// Every other part is untouched and written back byte-identical.
#[derive(Debug)]
pub struct Document {
    package: Package,
    main_part: PartName,
    prolog: String,
    root: DocumentRoot,
    epilogue: String,
    dirty: bool,
    hygiene_done: bool,
    dropped: Vec<DroppedItem>,
    hygiene: Vec<HygieneAction>,
}

impl Document {
    /// Opens a DOCX. The main story is found through the package-level
    /// `officeDocument` relationship, not by name.
    pub fn open(bytes: &[u8], limits: &Limits) -> Result<Self, DocxError> {
        let package = Package::read(bytes, limits)?;
        let rels_part = package
            .part(rels_part_name(None).as_str())
            .ok_or(DocxError::NoMainDocument)?;
        let rels = Relationships::parse(rels_part.data())?;
        let rel = rels
            .iter()
            .find(|r| {
                r.target_mode == TargetMode::Internal
                    && OFFICE_DOCUMENT_TYPES.contains(&r.rel_type.as_str())
            })
            .ok_or(DocxError::NoMainDocument)?;
        let target = resolve_target(None, &rel.target)?;
        let part = package
            .part(target.as_str())
            .ok_or_else(|| DocxError::MissingMainPart(target.to_string()))?;
        let main_part = part.name().clone();
        let parsed = xml::parse(part.data())?;
        let root = DocumentRoot::from_xml(parsed.root).map_err(DocxError::NotWordprocessing)?;
        Ok(Self {
            package,
            main_part,
            prolog: parsed.prolog,
            root,
            epilogue: parsed.epilogue,
            dirty: false,
            hygiene_done: false,
            dropped: Vec::new(),
            hygiene: Vec::new(),
        })
    }

    /// Name of the main document part.
    pub fn main_part(&self) -> &PartName {
        &self.main_part
    }

    pub fn package(&self) -> &Package {
        &self.package
    }

    /// The typed main story.
    pub fn body(&self) -> &Body {
        // `from_xml` guarantees a body.
        match self.root.body() {
            Some(body) => body,
            None => unreachable!("DocumentRoot::from_xml requires a w:body"),
        }
    }

    fn body_mut(&mut self) -> &mut Body {
        match self.root.body_mut() {
            Some(body) => body,
            None => unreachable!("DocumentRoot::from_xml requires a w:body"),
        }
    }

    /// Content that could not be represented. Empty means nothing was lost.
    pub fn dropped(&self) -> &[DroppedItem] {
        &self.dropped
    }

    /// Parts and relationships removed by the first edit (stale thumbnail, signatures).
    pub fn hygiene_log(&self) -> &[HygieneAction] {
        &self.hygiene
    }

    /// True when the main story changed since opening or the last save.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Number of body-level paragraphs in document order. Paragraphs inside tables are
    /// counted (cells in row order, nested tables included); paragraphs inside raw blocks
    /// such as content controls are not.
    pub fn paragraph_count(&self) -> usize {
        self.body().paragraphs().len()
    }

    pub fn paragraph(&self, index: usize) -> Option<&Paragraph> {
        self.body().paragraphs().get(index).copied()
    }

    /// Text of paragraph `index`: `w:t` text, tabs as `\t`, breaks as `\n`.
    pub fn paragraph_text(&self, index: usize) -> Option<String> {
        self.paragraph(index).map(Paragraph::text)
    }

    /// Inserts `text` at character offset `char_offset` (Unicode scalar values) of
    /// paragraph `para_idx`.
    pub fn insert_text(
        &mut self,
        para_idx: usize,
        char_offset: usize,
        text: &str,
    ) -> Result<(), DocxError> {
        self.check_paragraph(para_idx)?;
        let changed = {
            let paragraph = self.paragraph_mut(para_idx)?;
            let before = paragraph.char_len();
            paragraph.insert_text(char_offset, text)?;
            before != paragraph.char_len()
        };
        if changed {
            self.edited();
        }
        Ok(())
    }

    /// Sets bold on `char_range` of paragraph `para_idx`, splitting runs at the
    /// boundaries and cloning their properties.
    pub fn set_bold(
        &mut self,
        para_idx: usize,
        char_range: Range<usize>,
        bold: bool,
    ) -> Result<(), DocxError> {
        self.check_paragraph(para_idx)?;
        let empty = char_range.start == char_range.end;
        self.paragraph_mut(para_idx)?.set_bold(char_range, bold)?;
        if !empty {
            self.edited();
        }
        Ok(())
    }

    fn check_paragraph(&self, index: usize) -> Result<(), DocxError> {
        let count = self.paragraph_count();
        if index < count {
            Ok(())
        } else {
            Err(DocxError::ParagraphOutOfRange { index, count })
        }
    }

    fn paragraph_mut(&mut self, index: usize) -> Result<&mut Paragraph, DocxError> {
        let count = self.paragraph_count();
        self.body_mut()
            .paragraph_mut(index)
            .ok_or(DocxError::ParagraphOutOfRange { index, count })
    }

    /// Records a content change; the first one also drops stale parts.
    fn edited(&mut self) {
        self.dirty = true;
        if !self.hygiene_done {
            self.hygiene_done = true;
            invalidate_stale_parts(&mut self.package, &mut self.hygiene);
        }
    }

    /// The main part serialised from the model (original prolog and epilogue kept).
    /// Does not touch the package.
    pub fn serialize_main(&self) -> Vec<u8> {
        let mut out = String::new();
        out.push_str(&self.prolog);
        self.root.write(&mut out);
        out.push_str(&self.epilogue);
        out.into_bytes()
    }

    /// Writes the package. Only a changed main part is re-serialised; every other part is
    /// written back byte-identical. Refuses while the dropped-content log is non-empty.
    pub fn save(&mut self) -> Result<Vec<u8>, DocxError> {
        if !self.dropped.is_empty() {
            return Err(DocxError::DroppedContent(self.dropped.len()));
        }
        if self.dirty {
            let bytes = self.serialize_main();
            self.package.set_part(self.main_part.clone(), bytes);
            self.dirty = false;
        }
        Ok(self.package.write()?)
    }
}

/// Removes the thumbnail (it shows the old first page) and digital signatures (they no
/// longer match). Other parts are left alone.
fn invalidate_stale_parts(package: &mut Package, log: &mut Vec<HygieneAction>) {
    const THUMBNAIL: &str = "/metadata/thumbnail";
    const SIGNATURE_ORIGIN: &str = "/digital-signature/origin";
    let rels_name = rels_part_name(None);
    let Some(rels_part) = package.part(rels_name.as_str()) else {
        return;
    };
    let Ok(rels) = Relationships::parse(rels_part.data()) else {
        return;
    };

    let mut removed_ids: Vec<String> = Vec::new();
    for rel in rels.iter() {
        let (is_thumbnail, is_signature) = (
            rel.rel_type.ends_with(THUMBNAIL),
            rel.rel_type.ends_with(SIGNATURE_ORIGIN),
        );
        if !is_thumbnail && !is_signature {
            continue;
        }
        removed_ids.push(rel.id.clone());
        log.push(HygieneAction::RemovedRelationship {
            id: rel.id.clone(),
            rel_type: rel.rel_type.clone(),
        });
        if rel.target_mode == TargetMode::Internal
            && let Ok(target) = resolve_target(None, &rel.target)
            && package.remove_part(target.as_str()).is_some()
        {
            log.push(HygieneAction::RemovedPart {
                name: target.to_string(),
                reason: if is_thumbnail {
                    "thumbnail shows the pre-edit first page"
                } else {
                    "digital signature no longer matches the edited package"
                },
            });
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
                name,
                reason: "digital signature no longer matches the edited package",
            });
        }
    }

    if removed_ids.is_empty() {
        return;
    }
    let Some(part) = package.part(rels_name.as_str()) else {
        return;
    };
    let Ok(mut parsed) = xml::parse(part.data()) else {
        return;
    };
    parsed.root.children.retain(|child| match child {
        RawXml::Element(e) if e.head.local_name() == "Relationship" => !e
            .head
            .attrs
            .iter()
            .any(|a| a.name == "Id" && removed_ids.contains(&a.value)),
        _ => true,
    });
    package.set_part(rels_name, parsed.to_bytes());
}
