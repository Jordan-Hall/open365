use std::fmt;
use std::ops::Range;

use open365_opc::{
    Limits, Package, PartName, Relationships, TargetMode, rels_part_name, resolve_target,
};

use crate::DocxError;
use crate::model::{Body, DocumentRoot, Paragraph};
use crate::ppr::Alignment;
use crate::props::RunPropsPatch;
use crate::ranges::{DocPos, EditLogEntry};
use crate::styles::Styles;
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
    styles: Option<Styles>,
    pub(crate) edit_log: Vec<EditLogEntry>,
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
        let styles = load_styles(&package, &main_part);
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
            styles,
            edit_log: Vec::new(),
        })
    }

    /// The document's styles, when it has a (valid) styles part.
    pub fn styles(&self) -> Option<&Styles> {
        self.styles.as_ref()
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

    pub(crate) fn body_mut(&mut self) -> &mut Body {
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
        self.paragraph_mut(para_idx)?
            .apply_run_props(char_range, &RunPropsPatch::bold(bold))?;
        if !empty {
            self.edited();
        }
        Ok(())
    }

    /// Sets the paragraph style. The default style (usually `Normal`) removes `w:pStyle`.
    /// `Heading1`-`Heading3` and `Title` are added to the styles part when missing; any
    /// other id must already be defined.
    pub fn set_paragraph_style(&mut self, para: usize, style_id: &str) -> Result<(), DocxError> {
        self.check_paragraph(para)?;
        let mut new_id = Some(style_id);
        if let Some(styles) = &mut self.styles {
            if !styles.ensure_builtin(style_id) {
                return Err(DocxError::UnknownStyle(style_id.to_owned()));
            }
            if styles.default_paragraph_style() == Some(style_id) {
                new_id = None;
            }
        } else if style_id.is_empty() {
            new_id = None;
        }
        let paragraph = self.paragraph_mut(para)?;
        if paragraph.style_id() == new_id {
            return Ok(());
        }
        paragraph.set_style_id(new_id);
        self.edited();
        Ok(())
    }

    /// Sets the paragraph alignment (`w:jc`).
    pub fn set_alignment(&mut self, para: usize, alignment: Alignment) -> Result<(), DocxError> {
        self.check_paragraph(para)?;
        let paragraph = self.paragraph_mut(para)?;
        if paragraph.direct_alignment() == Some(alignment) {
            return Ok(());
        }
        paragraph.set_alignment(alignment);
        self.edited();
        Ok(())
    }

    /// Run formatting in effect at the caret `pos`, with styles resolved.
    pub fn char_format_at(&self, pos: DocPos) -> Option<crate::styles::EffectiveRun> {
        let paragraph = self.paragraph(pos.para)?;
        let styles = self.styles.as_ref()?;
        let probe = crate::model::Run {
            head: paragraph.new_run().head,
            props: paragraph.run_props_at(pos.offset),
            content: Vec::new(),
        };
        Some(styles.effective_run_props(paragraph, &probe))
    }

    fn check_paragraph(&self, index: usize) -> Result<(), DocxError> {
        let count = self.paragraph_count();
        if index < count {
            Ok(())
        } else {
            Err(DocxError::ParagraphOutOfRange { index, count })
        }
    }

    pub(crate) fn paragraph_mut(&mut self, index: usize) -> Result<&mut Paragraph, DocxError> {
        let count = self.paragraph_count();
        self.body_mut()
            .paragraph_mut(index)
            .ok_or(DocxError::ParagraphOutOfRange { index, count })
    }

    /// Records a content change; the first one also drops stale parts.
    pub(crate) fn edited(&mut self) {
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
        if let Some(styles) = &mut self.styles
            && styles.is_dirty()
        {
            self.package
                .set_part(styles.part_name().clone(), styles.serialize());
            styles.mark_clean();
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

const STYLES_TYPES: [&str; 1] =
    ["http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles"];
const THEME_TYPES: [&str; 1] =
    ["http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme"];

/// Finds and parses the styles and theme parts through the main part's relationships.
/// A missing or unreadable styles part gives `None`: editing still works without styles.
fn load_styles(package: &Package, main: &PartName) -> Option<Styles> {
    let rels =
        Relationships::parse(package.part(rels_part_name(Some(main)).as_str())?.data()).ok()?;
    let find = |types: &[&str]| -> Option<PartName> {
        let rel = rels.iter().find(|r| {
            r.target_mode == TargetMode::Internal && types.contains(&r.rel_type.as_str())
        })?;
        resolve_target(Some(main), &rel.target).ok()
    };
    let styles_name = find(&STYLES_TYPES)?;
    let styles_part = package.part(styles_name.as_str())?;
    let theme = find(&THEME_TYPES)
        .and_then(|n| package.part(n.as_str()))
        .map(open365_opc::Part::data);
    Styles::parse(styles_name, styles_part.data(), theme).ok()
}
