use std::fmt;
use std::ops::Range;

use open365_opc::{
    ContentTypes, Limits, Package, PartName, Relationships, TargetMode, rels_part_name,
    resolve_target,
};

use crate::DocxError;
use crate::model::{Body, DocumentRoot, Paragraph};
use crate::numbering::Numbering;
use crate::ppr::Alignment;
use crate::props::RunPropsPatch;
use crate::ranges::{DocPos, EditLogEntry};
use crate::section::Section;
use crate::settings::Settings;
use crate::styles::Styles;
use crate::xml::{self, RawXml};

const OFFICE_DOCUMENT_TYPES: [&str; 2] = [
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument",
    "http://purl.oclc.org/ooxml/officeDocument/relationships/officeDocument",
];

const CT_DOCUMENT: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";
const CT_TEMPLATE: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml";
const CT_MACRO_DOCUMENT: &str = "application/vnd.ms-word.document.macroEnabled.main+xml";
const CT_MACRO_TEMPLATE: &str = "application/vnd.ms-word.template.macroEnabledTemplate.main+xml";
const CT_VBA_PROJECT: &str = "application/vnd.ms-office.vbaProject";
const CONTENT_TYPES_PART: &str = "[Content_Types].xml";
const SETTINGS_TYPES: [&str; 1] =
    ["http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings"];

/// What the main part's content type says the package is (ECMA-376 Part 2 section 10).
/// Macros are never executed whatever the kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentKind {
    /// `.docx`
    Document,
    /// `.dotx`
    Template,
    /// `.docm`
    MacroEnabledDocument,
    /// `.dotm`
    MacroEnabledTemplate,
}

impl DocumentKind {
    /// The main-part content type for this kind.
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Document => CT_DOCUMENT,
            Self::Template => CT_TEMPLATE,
            Self::MacroEnabledDocument => CT_MACRO_DOCUMENT,
            Self::MacroEnabledTemplate => CT_MACRO_TEMPLATE,
        }
    }

    /// The kind a main-part content type denotes, or `None` for any other type.
    pub fn from_content_type(content_type: &str) -> Option<Self> {
        Some(match content_type {
            CT_DOCUMENT => Self::Document,
            CT_TEMPLATE => Self::Template,
            CT_MACRO_DOCUMENT => Self::MacroEnabledDocument,
            CT_MACRO_TEMPLATE => Self::MacroEnabledTemplate,
            _ => return None,
        })
    }

    pub fn is_template(self) -> bool {
        matches!(self, Self::Template | Self::MacroEnabledTemplate)
    }

    pub fn is_macro_enabled(self) -> bool {
        matches!(
            self,
            Self::MacroEnabledDocument | Self::MacroEnabledTemplate
        )
    }
}

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
    /// The main part's content type was changed in `[Content_Types].xml`.
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
            Self::RemovedRelationship { id, rel_type } => {
                write!(f, "removed relationship {id} ({rel_type})")
            }
            Self::ChangedContentType { part, from, to } => {
                write!(f, "changed content type of {part} from {from} to {to}")
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
    numbering: Option<Numbering>,
    settings: Settings,
    kind: DocumentKind,
    pub(crate) edit_log: Vec<EditLogEntry>,
    pub(crate) tracking: Option<crate::review::TrackChanges>,
    pub(crate) next_annotation_id: Option<u64>,
    pub(crate) comments: crate::comments::CommentParts,
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
        let numbering = load_numbering(&package, &main_part);
        let settings = load_settings(&package, &main_part);
        let kind = detect_kind(&package, &main_part);
        let comments = crate::comments::CommentParts::load(&package, &main_part);
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
            numbering,
            settings,
            kind,
            edit_log: Vec::new(),
            tracking: None,
            next_annotation_id: None,
            comments,
        })
    }

    /// The document's styles, when it has a (valid) styles part.
    pub fn styles(&self) -> Option<&Styles> {
        self.styles.as_ref()
    }

    /// The document's numbering definitions, when it has a numbering part.
    pub fn numbering(&self) -> Option<&Numbering> {
        self.numbering.as_ref()
    }

    /// Layout-relevant settings from `word/settings.xml` (defaults when there is none).
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// The sections in document order. Each section ends at the paragraph whose `w:pPr`
    /// carries its `w:sectPr` (see [`Section::end_paragraph`]); the last one is the body's
    /// final `w:sectPr`. A document with no `w:sectPr` has one section with all defaults.
    /// Paragraphs inside content controls are not scanned (they are not addressable).
    pub fn sections(&self) -> Vec<Section> {
        let mut out = Vec::new();
        for (i, p) in self.body().paragraphs().into_iter().enumerate() {
            let sect = p.props.as_ref().and_then(|ppr| {
                ppr.children
                    .iter()
                    .filter_map(RawXml::as_element)
                    .find(|e| e.head.is_w("sectPr"))
            });
            if let Some(sect) = sect {
                out.push(Section::from_sect_pr(sect, out.len(), Some(i)));
            }
        }
        let last = self.body().blocks.iter().rev().find_map(|b| match b {
            crate::model::Block::Raw(RawXml::Element(e)) if e.head.is_w("sectPr") => Some(e),
            _ => None,
        });
        match last {
            Some(sect) => out.push(Section::from_sect_pr(sect, out.len(), None)),
            None => out.push(Section {
                index: out.len(),
                ..Section::default()
            }),
        }
        out
    }

    /// The kind of package the main part's content type declares.
    pub fn kind(&self) -> DocumentKind {
        self.kind
    }

    /// True when the package carries a VBA project. It is kept byte-identical and never run.
    pub fn has_vba_project(&self) -> bool {
        has_vba(&self.package, &self.main_part)
    }

    /// Turns a template into a document for editing: the main part's content type becomes
    /// the plain document type, or the macro-enabled document type when a VBA project is
    /// present. The change is logged in [`Document::hygiene_log`]. Returns `Ok(false)`
    /// without touching anything when this is not a template.
    pub fn instantiate_template(&mut self) -> Result<bool, DocxError> {
        if !self.kind.is_template() {
            return Ok(false);
        }
        let to = if self.has_vba_project() {
            DocumentKind::MacroEnabledDocument
        } else {
            DocumentKind::Document
        };
        let from = self.kind.content_type();
        let part = self
            .package
            .part(CONTENT_TYPES_PART)
            .ok_or_else(|| DocxError::Xml("package has no [Content_Types].xml".to_owned()))?;
        let mut parsed = xml::parse(part.data())?;
        let target = format!("/{}", self.main_part.as_str().trim_start_matches('/'));
        let mut found = false;
        for child in &mut parsed.root.children {
            let RawXml::Element(e) = child else { continue };
            if e.head.local_name() != "Override" {
                continue;
            }
            let is_main = e
                .head
                .attrs
                .iter()
                .any(|a| a.name == "PartName" && a.value.eq_ignore_ascii_case(&target));
            if !is_main {
                continue;
            }
            for a in &mut e.head.attrs {
                if a.name == "ContentType" {
                    to.content_type().clone_into(&mut a.value);
                    found = true;
                }
            }
        }
        if !found {
            return Err(DocxError::Xml(format!(
                "no content type override for the main part {target}"
            )));
        }
        let name = PartName::new(CONTENT_TYPES_PART)
            .map_err(|e| DocxError::Xml(format!("content types part name: {e}")))?;
        self.package.set_part(name, parsed.to_bytes());
        self.hygiene.push(HygieneAction::ChangedContentType {
            part: target,
            from: from.to_owned(),
            to: to.content_type().to_owned(),
        });
        self.kind = to;
        Ok(true)
    }

    /// The image part the main part's relationship `rel_id` (`a:blip/@r:embed`) points at.
    /// `None` for an unknown id, an external target or a missing part.
    pub fn image_part(&self, rel_id: &str) -> Option<crate::drawing::ImagePart<'_>> {
        let rels_part = self
            .package
            .part(rels_part_name(Some(&self.main_part)).as_str())?;
        let rels = Relationships::parse(rels_part.data()).ok()?;
        let rel = rels.get(rel_id)?;
        if rel.target_mode != TargetMode::Internal {
            return None;
        }
        let name = resolve_target(Some(&self.main_part), &rel.target).ok()?;
        let part = self.package.part(name.as_str())?;
        let content_type = self
            .package
            .part(CONTENT_TYPES_PART)
            .and_then(|p| ContentTypes::parse(p.data()).ok())
            .and_then(|ct| ct.content_type_of(&name).map(str::to_owned))
            .unwrap_or_default();
        Some(crate::drawing::ImagePart {
            name: name.as_str().to_owned(),
            content_type,
            bytes: part.data(),
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
        self.dirty || self.comments.is_dirty()
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
        if let Some(track) = self.tracking.clone() {
            return self.tracked_insert_text(para_idx, char_offset, text, &track);
        }
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
        self.comments.flush(&mut self.package, &self.main_part)?;
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
            && package.remove_part_and_override(target.as_str()).is_some()
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
        if package.remove_part_and_override(&name).is_some() {
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

fn detect_kind(package: &Package, main: &PartName) -> DocumentKind {
    package
        .part(CONTENT_TYPES_PART)
        .and_then(|p| ContentTypes::parse(p.data()).ok())
        .and_then(|ct| {
            ct.content_type_of(main)
                .and_then(DocumentKind::from_content_type)
        })
        .unwrap_or(DocumentKind::Document)
}

fn has_vba(package: &Package, main: &PartName) -> bool {
    let by_rel = package
        .part(rels_part_name(Some(main)).as_str())
        .and_then(|p| Relationships::parse(p.data()).ok())
        .is_some_and(|rels| {
            rels.iter()
                .any(|r| r.rel_type.to_ascii_lowercase().ends_with("/vbaproject"))
        });
    by_rel
        || package
            .part(CONTENT_TYPES_PART)
            .and_then(|p| ContentTypes::parse(p.data()).ok())
            .is_some_and(|ct| {
                ct.overrides()
                    .iter()
                    .any(|(_, content_type)| *content_type == CT_VBA_PROJECT)
            })
}

/// Parses the settings part found through the main part's relationships.
fn load_settings(package: &Package, main: &PartName) -> Settings {
    let load = || -> Option<Settings> {
        let rels =
            Relationships::parse(package.part(rels_part_name(Some(main)).as_str())?.data()).ok()?;
        let rel = rels.iter().find(|r| {
            r.target_mode == TargetMode::Internal && SETTINGS_TYPES.contains(&r.rel_type.as_str())
        })?;
        let name = resolve_target(Some(main), &rel.target).ok()?;
        Settings::parse(package.part(name.as_str())?.data()).ok()
    };
    load().unwrap_or_default()
}

const STYLES_TYPES: [&str; 1] =
    ["http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles"];
const THEME_TYPES: [&str; 1] =
    ["http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme"];

const NUMBERING_TYPES: [&str; 1] =
    ["http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering"];

/// Finds and parses the numbering part through the main part's relationships.
fn load_numbering(package: &Package, main: &PartName) -> Option<Numbering> {
    let rels =
        Relationships::parse(package.part(rels_part_name(Some(main)).as_str())?.data()).ok()?;
    let rel = rels.iter().find(|r| {
        r.target_mode == TargetMode::Internal && NUMBERING_TYPES.contains(&r.rel_type.as_str())
    })?;
    let name = resolve_target(Some(main), &rel.target).ok()?;
    Numbering::parse(package.part(name.as_str())?.data())
}

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
