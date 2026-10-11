//! PDF 1.7 export of a laid-out document (layer 4, ADR-004: the print path).
//!
//! [`export_pdf`] takes the DOCX model, its [`Layout`] and the [`FontLibrary`] the layout used,
//! and writes a tagged PDF: subset fonts with a ToUnicode CMap, glyphs placed at the layout's
//! exact positions, underline and strike, table shading and borders, pictures, page sizes from
//! the sections, and a structure tree (P, H1 to H6, L/LI/LBody, Table/TR/TH/TD, Figure with Alt).
//! See `docs/architecture/pdf-export.md` for what is and is not covered.
//!
//! Pictures are untrusted input: see [`image`] for the limits.

mod align;
mod cff;
mod font;
mod icc;
mod image;
mod lang;
mod meta;
mod structure;
mod writer;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

use open365_docx::Document;
use open365_layout::{
    BorderLine, BorderStyle, ClusterKind, GlyphRun, ImagePlacement, ImageSource, Layout, Line,
    Page, ParaRole, PlacedGlyph, StoryKind,
};
use open365_opc::{PartName, Relationships, TargetMode, rels_part_name, resolve_target};
use open365_text::FontLibrary;

use font::FaceDraw;
use lang::Languages;
use meta::{write_info, write_xmp};
use structure::{DocStructure, LinkSpec, LinkTarget, Node, ParaText};
use writer::{Pdf, digest, num, text_string};

pub use image::{MAX_BYTES as MAX_IMAGE_BYTES, MAX_PIXELS as MAX_IMAGE_PIXELS};
pub use meta::PdfMetadata;

/// The archival profile to write, when one is wanted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Conformance {
    /// A plain PDF 1.7 file, as Word writes by default.
    #[default]
    None,
    /// PDF/A-1b (ISO 19005-1, level B): PDF 1.4, no transparency, an sRGB output intent.
    PdfA1b,
    /// PDF/A-2b (ISO 19005-2, level B): PDF 1.7 with an sRGB output intent.
    PdfA2b,
}

impl Conformance {
    /// The `pdfaid` part and conformance level, for the XMP packet.
    pub(crate) fn pdfa(self) -> Option<(u8, &'static str)> {
        match self {
            Self::None => None,
            Self::PdfA1b => Some((1, "B")),
            Self::PdfA2b => Some((2, "B")),
        }
    }

    fn version(self) -> &'static str {
        match self {
            Self::PdfA1b => "1.4",
            _ => "1.7",
        }
    }
}

/// What the export writes beyond the page content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PdfOptions {
    /// Archival profile; [`Conformance::None`] by default.
    pub conformance: Conformance,
    /// Bookmarks (the outline) from the headings, as Word's "Create bookmarks using Headings".
    pub bookmarks: bool,
    /// Declare PDF/UA-1 (`pdfuaid:part` in the XMP) and write what the profile needs besides the
    /// tags: a title (the first heading when the caller gave none) shown in the title bar.
    pub ua: bool,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            conformance: Conformance::None,
            bookmarks: true,
            ua: false,
        }
    }
}

/// The finished file plus what could not be represented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfOutput {
    pub bytes: Vec<u8>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PdfError {
    /// The layout has no pages (for example because the library has no faces).
    NoPages,
    /// A glyph refers to a face the library does not hold.
    MissingFace(usize),
    /// The font file could not be read or subset.
    Font(String),
    /// The font's licence bits (OS/2 fsType) forbid embedding.
    FontEmbeddingForbidden(String),
    /// The caller's cancellation flag was set; no output was produced.
    Cancelled,
}

impl fmt::Display for PdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoPages => write!(f, "the layout has no pages"),
            Self::MissingFace(i) => write!(f, "layout uses face {i}, which the library lacks"),
            Self::Font(m) => write!(f, "font error: {m}"),
            Self::FontEmbeddingForbidden(n) => {
                write!(f, "the licence of font {n} forbids embedding")
            }
            Self::Cancelled => write!(f, "the export was cancelled"),
        }
    }
}

impl std::error::Error for PdfError {}

fn pdf_color(c: [u8; 3]) -> String {
    format!(
        "{} {} {}",
        num(f64::from(c[0]) / 255.0, 4),
        num(f64::from(c[1]) / 255.0, 4),
        num(f64::from(c[2]) / 255.0, 4)
    )
}

/// What owns a marked-content sequence.
#[derive(Clone)]
enum Owner {
    Struct(u32, String),
    Artifact,
}

/// A child of a structure element, in reading order.
#[derive(Clone, Copy)]
enum Kid {
    /// Marked content: page index and MCID.
    Mcr(usize, u32),
    /// Another structure element (a link inside a paragraph).
    Elem(u32),
}

struct FigureRec {
    id: u32,
    page: usize,
    mcid: u32,
    alt: Option<String>,
}

/// A hyperlink's `Link` structure element and the annotations it owns.
struct LinkElem {
    id: u32,
    parent: u32,
    /// (page index, annotation object) per line the link covers.
    annots: Vec<(usize, u32)>,
}

/// A hyperlink's place in its paragraph: byte range in the paragraph text and index in
/// [`DocStructure::links`].
type Span = (usize, usize, usize);

/// A link element to write: id, parent element, (page index, annotation) pairs.
type LinkOut = (u32, u32, Vec<(usize, u32)>);

/// A `Span` with a `/Lang`: id, parent element, language.
struct LangElem {
    id: u32,
    parent: u32,
    lang: String,
}

/// A language range of a paragraph in the layout's text: byte range and primary subtag.
type LangSpan = (usize, usize, String);

/// A heading for the outline: level, title, page index, x and top of its first line.
struct Heading {
    level: u8,
    title: String,
    page: usize,
    x: f64,
    top: f64,
}

struct Export<'a> {
    layout: &'a Layout,
    doc: &'a Document,
    st: DocStructure,
    faces: BTreeMap<usize, FaceDraw>,
    page_ids: Vec<u32>,
    /// Page size as written to the MediaBox.
    page_dims: Vec<(f64, f64)>,
    /// struct element id -> its children in drawing order.
    kids: HashMap<u32, Vec<Kid>>,
    /// parent struct element id -> figures.
    figures: HashMap<u32, Vec<FigureRec>>,
    /// (story, paragraph) -> extra element for note lines.
    extras: Vec<u32>,
    extra_keys: HashMap<(u8, usize), u32>,
    images: HashMap<String, Option<(String, u32)>>,
    /// Main-story lines already drawn: (paragraph, first offset).
    drawn: std::collections::HashSet<(usize, usize)>,
    /// Glyph (face, id) to the text the font's ToUnicode CMap gives it.
    to_unicode: BTreeMap<(usize, u32), String>,
    /// Main-story paragraph -> (page index, x, top) of its first line.
    para_first: HashMap<usize, (usize, f64, f64)>,
    link_spans: HashMap<usize, Vec<Span>>,
    lang_spans: HashMap<usize, Vec<LangSpan>>,
    lang_elems: HashMap<(usize, usize), LangElem>,
    link_elems: HashMap<(usize, usize), LinkElem>,
    /// Counter for link elements of annotations that have no paragraph element (header, footer).
    free_links: usize,
    /// Section of the page being drawn (the header and footer parts differ per section).
    current_section: usize,
    /// Link element of each annotation's `StructParent`, in key order.
    struct_parents: Vec<u32>,
    page_annots: Vec<Vec<u32>>,
    /// PDF/A-1 has no transparency: pictures with alpha are composited onto white.
    flatten: bool,
    /// PDF/A and PDF/UA forbid references to the .notdef glyph: it is left out (the cursor
    /// is corrected by the next glyph's position).
    skip_notdef: bool,
    ua: bool,
    warnings: Vec<String>,
    pdf: Pdf,
}

struct PageBuf {
    index: usize,
    height: f64,
    content: String,
    owners: Vec<u32>,
}

impl PageBuf {
    fn y(&self, y: f64) -> f64 {
        self.height - y
    }
}

/// A hyperlink piece, a list label or plain text: what a run of glyphs on a line belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Seg {
    Text,
    Label,
    Link(usize),
    /// Text in another language than the document's: index in the paragraph's language spans.
    Lang(usize),
}

/// Glyphs `from..to` of run `run`.
struct Piece {
    run: usize,
    from: usize,
    to: usize,
}

/// The size Word's PDF gives a page. Word lays the page out on a 600 dpi device grid, and for A4
/// its MediaBox is 595.2 x 841.92 pt (4960 x 7016 device pixels), not the 595.3 x 841.9 pt of
/// the section (11906 x 16838 twips). Evidence: all 51 A4 pages of the reference exports in
/// `fixtures/reference/pdf`. Other sizes (US Letter, 612 x 792) are exact on that grid.
fn page_dims(w: f64, h: f64) -> (f64, f64) {
    let snap = |v: f64| {
        if (v - 595.3).abs() < 0.06 {
            595.2
        } else if (v - 841.9).abs() < 0.06 {
            841.92
        } else {
            v
        }
    };
    (snap(w), snap(h))
}

/// Writes `layout` of `doc` as a tagged PDF 1.7 file with bookmarks and hyperlinks (see
/// [`PdfOptions`] for the choices).
///
/// `fonts` must be the library the layout was made with.
pub fn export_pdf(
    doc: &Document,
    layout: &Layout,
    fonts: &FontLibrary,
    meta: &PdfMetadata,
) -> Result<PdfOutput, PdfError> {
    export_pdf_with(
        doc,
        layout,
        fonts,
        meta,
        &PdfOptions::default(),
        &AtomicBool::new(false),
    )
}

/// As [`export_pdf`], but gives up with [`PdfError::Cancelled`] soon after `cancel` is set. The
/// flag is looked at before each font is embedded and before each page is drawn.
pub fn export_pdf_cancellable(
    doc: &Document,
    layout: &Layout,
    fonts: &FontLibrary,
    meta: &PdfMetadata,
    cancel: &AtomicBool,
) -> Result<PdfOutput, PdfError> {
    export_pdf_with(doc, layout, fonts, meta, &PdfOptions::default(), cancel)
}

/// As [`export_pdf_cancellable`], with explicit [`PdfOptions`] (PDF/A, bookmarks).
pub fn export_pdf_with(
    doc: &Document,
    layout: &Layout,
    fonts: &FontLibrary,
    meta: &PdfMetadata,
    options: &PdfOptions,
    cancel: &AtomicBool,
) -> Result<PdfOutput, PdfError> {
    let check = || {
        if cancel.load(Ordering::Relaxed) {
            Err(PdfError::Cancelled)
        } else {
            Ok(())
        }
    };
    check()?;
    if layout.pages.is_empty() {
        return Err(PdfError::NoPages);
    }
    let mut pdf = Pdf::with_version(options.conformance.version());
    let mut warnings: Vec<String> = Vec::new();

    // Fonts: the glyphs each face draws.
    let mut used: BTreeMap<usize, BTreeSet<u32>> = BTreeMap::new();
    for (_, line) in layout.all_lines() {
        for run in &line.runs {
            for g in &run.glyphs {
                used.entry(g.face.index()).or_default().insert(g.glyph_id);
            }
        }
    }
    let (learned, to_unicode) = cluster_text(layout, fonts, &used);
    let no_extra = BTreeMap::new();
    let mut face_draw = BTreeMap::new();
    let mut font_objects = Vec::new();
    for (n, (index, gids)) in used.iter().enumerate() {
        check()?;
        let face = fonts
            .try_face(open365_text::FaceId::from_index(*index))
            .ok_or(PdfError::MissingFace(*index))?;
        let (id, draw) = font::embed(
            &mut pdf,
            face,
            gids,
            learned.get(index).unwrap_or(&no_extra),
            format!("F{n}"),
            &mut warnings,
            options.conformance == Conformance::PdfA1b,
        )?;
        font_objects.push((draw.resource.clone(), id));
        face_draw.insert(*index, draw);
    }

    let mut roles: HashMap<usize, ParaRole> = HashMap::new();
    for (_, line) in layout.lines() {
        if line.story == StoryKind::Main {
            roles.entry(line.paragraph).or_insert(line.role);
        }
    }
    let main_rels = relationships(doc, doc.main_part());
    let langs = Languages::read(doc);
    // The document language: the caller's, else the document's own default (Word writes the
    // primary subtag). Text in another language gets a `Span` with its own `/Lang`.
    let catalog_lang: Option<String> = meta
        .language
        .clone()
        .filter(|l| !l.is_empty())
        .or_else(|| langs.default_tag().and_then(lang::primary));
    let base_lang = catalog_lang.as_deref().and_then(lang::primary);
    let mut st = structure::analyse(
        doc,
        &roles,
        main_rels.as_ref(),
        &langs,
        base_lang.as_deref(),
        &mut pdf,
    );
    add_story_texts(doc, layout, &langs, base_lang.as_deref(), &mut st);
    if st.dropped_links > 0 {
        warnings.push(format!(
            "{} hyperlink(s) without a usable target were left out",
            st.dropped_links
        ));
    }
    let resources_id = pdf.reserve();
    let pages_id = pdf.reserve();
    let page_ids: Vec<u32> = layout.pages.iter().map(|_| pdf.reserve()).collect();
    let dims: Vec<(f64, f64)> = layout
        .pages
        .iter()
        .map(|p| page_dims(p.section.page_width, p.section.page_height))
        .collect();
    let (para_first, link_spans, lang_spans, headings) = index_main_story(layout, &st);
    let mut ex = Export {
        layout,
        doc,
        st,
        faces: face_draw,
        page_ids,
        page_dims: dims,
        kids: HashMap::new(),
        figures: HashMap::new(),
        extras: Vec::new(),
        extra_keys: HashMap::new(),
        images: HashMap::new(),
        drawn: std::collections::HashSet::new(),
        to_unicode,
        para_first,
        link_spans,
        lang_spans,
        lang_elems: HashMap::new(),
        free_links: 0,
        current_section: 0,
        link_elems: HashMap::new(),
        struct_parents: Vec::new(),
        page_annots: vec![Vec::new(); layout.pages.len()],
        flatten: options.conformance == Conformance::PdfA1b,
        skip_notdef: options.conformance != Conformance::None || options.ua,
        ua: options.ua,
        warnings,
        pdf,
    };

    let mut page_owners: Vec<Vec<u32>> = Vec::new();
    for (i, page) in layout.pages.iter().enumerate() {
        check()?;
        let (content, owners) = ex.render_page(i, page);
        let content_id = ex.pdf.add_stream("", content.as_bytes(), true);
        let (w, h) = ex.page_dims[i];
        let annots = if ex.page_annots[i].is_empty() {
            String::new()
        } else {
            let refs: String = ex.page_annots[i]
                .iter()
                .map(|a| format!("{a} 0 R "))
                .collect();
            format!(" /Annots [{refs}]")
        };
        ex.pdf.set(
            ex.page_ids[i],
            format!(
                "<< /Type /Page /Parent {pages_id} 0 R /MediaBox [0 0 {} {}] /Resources {resources_id} 0 R /Contents {content_id} 0 R /StructParents {i} /Tabs /S{annots} >>",
                num(w, 3),
                num(h, 3)
            ),
        );
        page_owners.push(owners);
    }

    let mut xobjects = String::new();
    let mut seen = BTreeSet::new();
    for (name, id) in ex.images.values().flatten() {
        if seen.insert(*id) {
            xobjects.push_str(&format!("/{name} {id} 0 R "));
        }
    }
    let fonts_dict: String = font_objects
        .iter()
        .map(|(n, id)| format!("/{n} {id} 0 R "))
        .collect();
    ex.pdf.set(
        resources_id,
        format!(
            "<< /ProcSet [/PDF /Text /ImageB /ImageC] /Font << {fonts_dict}>> /XObject << {xobjects}>> >>"
        ),
    );
    let kids: String = ex.page_ids.iter().map(|id| format!("{id} 0 R ")).collect();
    ex.pdf.set(
        pages_id,
        format!(
            "<< /Type /Pages /Kids [{kids}] /Count {} >>",
            ex.page_ids.len()
        ),
    );

    check()?;
    let struct_root = ex.write_structure(&page_owners);
    let outlines = if options.bookmarks {
        ex.write_outlines(&headings)
    } else {
        None
    };
    let mut meta = meta.clone();
    if options.ua && meta.title.as_deref().is_none_or(str::is_empty) {
        // PDF/UA needs a title in the title bar: the first heading, else a plain name.
        let title = headings.first().map_or("Document", |h| h.title.as_str());
        ex.warnings.push(format!(
            "PDF/UA needs a document title; \"{title}\" was used (pass one with --title)"
        ));
        meta.title = Some(title.to_owned());
    }
    let info = write_info(&mut ex.pdf, &meta);
    let xmp = write_xmp(&mut ex.pdf, &meta, options.conformance, options.ua);
    let catalog = ex.pdf.reserve();
    let mut cat = format!(
        "<< /Type /Catalog /Pages {pages_id} 0 R /MarkInfo << /Marked true >> /StructTreeRoot {struct_root} 0 R /Metadata {xmp} 0 R"
    );
    if let Some(o) = outlines {
        cat.push_str(&format!(" /Outlines {o} 0 R"));
    }
    if let Some(lang) = catalog_lang.as_deref() {
        cat.push_str(&format!(" /Lang {}", text_string(lang)));
    }
    if meta.title.as_deref().is_some_and(|t| !t.is_empty()) || options.ua {
        cat.push_str(" /ViewerPreferences << /DisplayDocTitle true >>");
    }
    if options.conformance != Conformance::None {
        let profile = icc::srgb_profile();
        let icc_id = ex.pdf.add_stream("/N 3", &profile, true);
        cat.push_str(&format!(
            " /OutputIntents [<< /Type /OutputIntent /S /GTS_PDFA1 /OutputConditionIdentifier (sRGB IEC61966-2.1) /RegistryName (http://www.color.org) /Info (sRGB IEC61966-2.1) /DestOutputProfile {icc_id} 0 R >>]"
        ));
    }
    cat.push_str(" >>");
    ex.pdf.set(catalog, cat);

    let warnings = std::mem::take(&mut ex.warnings);
    let identity = digest(
        format!(
            "{:?}{:?}{}{}",
            meta,
            options,
            ex.page_ids.len(),
            ex.layout.lines().count()
        )
        .as_bytes(),
    );
    let bytes = ex
        .pdf
        .finish(catalog, &format!("/Info {info} 0 R"), identity);
    Ok(PdfOutput { bytes, warnings })
}

type MainIndex = (
    HashMap<usize, (usize, f64, f64)>,
    HashMap<usize, Vec<Span>>,
    HashMap<usize, Vec<LangSpan>>,
    Vec<Heading>,
);

/// The key of a paragraph in the maps of links and languages: the paragraph index for the main
/// story, a composite for the other stories (headers and footers per section).
fn story_key(story: StoryKind, section: usize, paragraph: usize) -> usize {
    if story == StoryKind::Main {
        return paragraph;
    }
    let section = match story {
        StoryKind::Footnotes | StoryKind::Endnotes => 0,
        _ => section.min(0xFFFF),
    };
    (usize::from(story.word_code()) << 48) | (section << 32) | (paragraph & 0xFFFF_FFFF)
}

fn line_key(line: &Line, section: usize) -> usize {
    story_key(line.story, section, line.paragraph)
}

/// The first line of every main-story paragraph, where each hyperlink and each foreign-language
/// run sits in the paragraph text the layout drew (found by aligning the model's text with it, so
/// a field that shows another value does not lose them), and the headings for the outline.
fn index_main_story(layout: &Layout, st: &DocStructure) -> MainIndex {
    let mut first = HashMap::new();
    let mut spans: HashMap<usize, Vec<Span>> = HashMap::new();
    let mut langs: HashMap<usize, Vec<LangSpan>> = HashMap::new();
    let mut seen = std::collections::HashSet::new();
    let mut headings = Vec::new();
    for (pi, page) in layout.pages.iter().enumerate() {
        for line in page
            .header
            .iter()
            .chain(&page.lines)
            .chain(&page.notes)
            .chain(&page.footer)
        {
            if line.story == StoryKind::Main && !first.contains_key(&line.paragraph) {
                first.insert(line.paragraph, (pi, line.left, line.top));
                if let Some(level) = st.paras.get(line.paragraph).and_then(|p| p.heading) {
                    let label = line.label.as_deref().unwrap_or("");
                    let raw = format!("{label} {}", line.text);
                    let title: String = raw
                        .chars()
                        .map(|c| if c.is_control() { ' ' } else { c })
                        .collect();
                    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !title.is_empty() {
                        headings.push(Heading {
                            level,
                            title,
                            page: pi,
                            x: line.left,
                            top: line.top,
                        });
                    }
                }
            }
            let key = line_key(line, page.section_index);
            if !seen.insert(key) {
                continue;
            }
            let Some(pt) = st.texts.get(&key) else {
                continue;
            };
            if pt.links.is_empty() && pt.langs.is_empty() {
                continue;
            }
            let al = align::Alignment::new(&pt.text, &line.text);
            let found: Vec<Span> = pt
                .links
                .iter()
                .enumerate()
                .filter_map(|(k, r)| al.map(r.clone()).map(|m| (m.start, m.end, k)))
                .collect();
            if !found.is_empty() {
                spans.insert(key, found);
            }
            let found: Vec<LangSpan> = pt
                .langs
                .iter()
                .filter_map(|(r, l)| al.map(r.clone()).map(|m| (m.start, m.end, l.clone())))
                .collect();
            if !found.is_empty() {
                langs.insert(key, found);
            }
        }
    }
    (first, spans, langs, headings)
}

/// Reads the text and hyperlinks of the header, footer and note paragraphs the layout drew, so
/// that links there are found too. The paragraphs of a story are numbered as the layout numbers
/// them: in file order, tables included; notes by their top-level paragraphs.
fn add_story_texts(
    doc: &Document,
    layout: &Layout,
    langs: &Languages,
    base: Option<&str>,
    st: &mut DocStructure,
) {
    let put = |story: StoryKind,
               section: usize,
               stories: Vec<(ParaText, Vec<LinkSpec>)>,
               st: &mut DocStructure| {
        for (i, (mut text, links)) in stories.into_iter().enumerate() {
            let key = story_key(story, section, i);
            if links.is_empty() && text.langs.is_empty() {
                continue;
            }
            if base.is_some() {
                text.langs.retain(|(_, l)| Some(l.as_str()) != base);
            }
            if !links.is_empty() {
                st.links.insert(key, links);
            }
            st.texts.insert(key, text);
        }
    };
    // Headers and footers: the part each (story, section) shows.
    let sections = doc.sections();
    let mut wanted = std::collections::BTreeMap::new();
    for page in &layout.pages {
        for line in page.header.iter().chain(&page.footer) {
            wanted.insert((line.story.word_code(), page.section_index), line.story);
        }
    }
    for ((_, section), story) in wanted {
        let (footer, kind) = match story {
            StoryKind::PrimaryHeader => (false, open365_docx::HeaderFooterType::Default),
            StoryKind::FirstHeader => (false, open365_docx::HeaderFooterType::First),
            StoryKind::EvenHeader => (false, open365_docx::HeaderFooterType::Even),
            StoryKind::PrimaryFooter => (true, open365_docx::HeaderFooterType::Default),
            StoryKind::FirstFooter => (true, open365_docx::HeaderFooterType::First),
            StoryKind::EvenFooter => (true, open365_docx::HeaderFooterType::Even),
            _ => continue,
        };
        // A kind a section does not name is inherited from the one before.
        let Some(rel) = sections
            .iter()
            .take(section + 1)
            .rev()
            .flat_map(|s| s.header_footer_refs.iter())
            .find(|r| r.footer == footer && r.kind == kind)
            .map(|r| r.rel_id.clone())
        else {
            continue;
        };
        let (Some(blocks), Some(part)) = (
            doc.header_footer_blocks(&rel),
            related(doc, doc.main_part(), |r| r.id == rel),
        ) else {
            continue;
        };
        let rels = relationships(doc, &part);
        let texts = structure::story_texts(&blocks, rels.as_ref(), langs, base);
        put(story, section, texts, st);
    }
    // Footnotes and endnotes.
    for (story, notes, suffix) in [
        (StoryKind::Footnotes, doc.footnotes(), "/footnotes"),
        (StoryKind::Endnotes, doc.endnotes(), "/endnotes"),
    ] {
        let rels = related(doc, doc.main_part(), |r| r.rel_type.ends_with(suffix))
            .and_then(|part| relationships(doc, &part));
        let mut all = Vec::new();
        for note in notes
            .iter()
            .filter(|n| n.kind == open365_docx::NoteKind::Normal)
        {
            for block in &note.blocks {
                if matches!(block, open365_docx::Block::Paragraph(_)) {
                    all.extend(structure::story_texts(
                        std::slice::from_ref(block),
                        rels.as_ref(),
                        langs,
                        base,
                    ));
                }
            }
        }
        put(story, 0, all, st);
    }
}

/// True when `text` is one character that `shown` only draws in another case: a character set in
/// capitals or small capitals, which Word's PDF copies as the capital.
fn differs_by_case(text: &str, shown: char) -> bool {
    let mut it = text.chars();
    match (it.next(), it.next()) {
        (Some(c), None) => c != shown && c.to_uppercase().eq(shown.to_uppercase()),
        _ => false,
    }
}

/// The text each glyph stands for, from the layout's clusters. A glyph whose cluster is several
/// characters (a ligature) or that the font's cmap does not describe gets an entry of its own
/// for the ToUnicode CMap; the second map is what the CMap will say for every glyph, to find the
/// glyphs that need `ActualText`.
#[allow(clippy::type_complexity)]
fn cluster_text(
    layout: &Layout,
    fonts: &FontLibrary,
    used: &BTreeMap<usize, BTreeSet<u32>>,
) -> (
    BTreeMap<usize, BTreeMap<u32, String>>,
    BTreeMap<(usize, u32), String>,
) {
    let rev: BTreeMap<usize, BTreeMap<u32, char>> = used
        .keys()
        .filter_map(|i| {
            let face = fonts.try_face(open365_text::FaceId::from_index(*i))?;
            Some((*i, font::reverse_map(face)))
        })
        .collect();
    let mut extra: BTreeMap<usize, BTreeMap<u32, String>> = BTreeMap::new();
    for (_, line) in layout.all_lines() {
        for g in line.runs.iter().flat_map(|r| &r.glyphs) {
            let text = line.cluster_text(g);
            if text.is_empty() || text.chars().all(|c| c.is_control()) {
                continue;
            }
            let face = g.face.index();
            let described = rev.get(&face).and_then(|m| m.get(&g.glyph_id));
            // Capitals and small capitals: the glyph shown is the capital, and Word's PDF copies
            // the capital (no ActualText), so the reverse cmap stands.
            let shown_case = described.is_some_and(|c| differs_by_case(text, *c));
            if !shown_case && (described.is_none() || text.chars().nth(1).is_some()) {
                extra
                    .entry(face)
                    .or_default()
                    .entry(g.glyph_id)
                    .or_insert_with(|| text.to_owned());
            }
        }
    }
    let mut table: BTreeMap<(usize, u32), String> = BTreeMap::new();
    for (face, gids) in used {
        for gid in gids {
            let text = extra
                .get(face)
                .and_then(|m| m.get(gid))
                .cloned()
                .or_else(|| {
                    rev.get(face)
                        .and_then(|m| m.get(gid))
                        .map(|c| font::expand(*c))
                });
            if let Some(t) = text {
                table.insert((*face, *gid), t);
            }
        }
    }
    (extra, table)
}

impl Export<'_> {
    fn render_page(&mut self, index: usize, page: &Page) -> (String, Vec<u32>) {
        self.current_section = page.section_index;
        let mut buf = PageBuf {
            index,
            height: self.page_dims[index].1,
            content: String::new(),
            owners: Vec::new(),
        };
        self.draw_cells(&mut buf, page);
        for img in page.images.iter().filter(|i| !i.anchored || i.behind_text) {
            self.draw_image(&mut buf, img, true);
        }
        self.draw_borders(&mut buf, page);
        self.draw_rules(&mut buf, page);
        for line in page
            .header
            .iter()
            .chain(&page.lines)
            .chain(page.frames.iter().flat_map(|f| f.lines.iter()))
            .chain(&page.notes)
            .chain(&page.footer)
        {
            self.draw_line(&mut buf, line);
        }
        for img in page.images.iter().filter(|i| i.anchored && !i.behind_text) {
            self.draw_image(&mut buf, img, true);
        }
        // Pictures of headers, footers and notes are not part of the reading order.
        for img in page
            .header_images
            .iter()
            .chain(&page.note_images)
            .chain(&page.footer_images)
        {
            self.draw_image(&mut buf, img, false);
        }
        (buf.content, buf.owners)
    }

    /// Opens a marked-content sequence.
    fn begin(&mut self, buf: &mut PageBuf, owner: &Owner) {
        match owner {
            Owner::Artifact => buf.content.push_str("/Artifact BMC\n"),
            Owner::Struct(id, tag) => {
                let mcid = buf.owners.len() as u32;
                buf.owners.push(*id);
                self.kids
                    .entry(*id)
                    .or_default()
                    .push(Kid::Mcr(buf.index, mcid));
                buf.content
                    .push_str(&format!("/{tag} << /MCID {mcid} >> BDC\n"));
            }
        }
    }

    fn owner_of(&mut self, line: &Line) -> Owner {
        match line.story {
            StoryKind::Main => match self.st.paras.get(line.paragraph) {
                // A repeated table header row shows the same text again: not content.
                _ if !self.drawn.insert((line.paragraph, line.start)) => Owner::Artifact,
                Some(p) => Owner::Struct(p.node, p.tag.clone()),
                None => Owner::Artifact,
            },
            StoryKind::Footnotes | StoryKind::Endnotes => {
                let key = (line.story.word_code(), line.paragraph);
                let id = match self.extra_keys.get(&key) {
                    Some(id) => *id,
                    None => {
                        let id = self.pdf.reserve();
                        self.extras.push(id);
                        self.extra_keys.insert(key, id);
                        id
                    }
                };
                Owner::Struct(id, "P".to_owned())
            }
            _ => Owner::Artifact,
        }
    }

    /// Splits the glyphs of a line into what each belongs to: list label, hyperlink or text.
    fn segments(
        &self,
        line: &Line,
        key: usize,
        links: bool,
        label_apart: bool,
    ) -> Vec<(Seg, Vec<Piece>)> {
        let spans = if links {
            self.link_spans.get(&key)
        } else {
            None
        };
        let langs = if links {
            self.lang_spans.get(&key)
        } else {
            None
        };
        let mut out: Vec<(Seg, Vec<Piece>)> = Vec::new();
        let mut prev = Seg::Text;
        for (ri, run) in line.runs.iter().enumerate() {
            for (gi, g) in run.glyphs.iter().enumerate() {
                let c = g.cluster;
                let key = match c.kind {
                    // A heading's number is part of the heading's own marked content.
                    ClusterKind::Label if label_apart => Seg::Label,
                    ClusterKind::Label => Seg::Text,
                    ClusterKind::Other => Seg::Text,
                    // The later glyphs of a cluster belong with the first.
                    ClusterKind::Text if c.text_start == c.text_end => match prev {
                        Seg::Label => Seg::Text,
                        other => other,
                    },
                    ClusterKind::Text => {
                        let at = c.text_start as usize;
                        spans
                            .and_then(|s| s.iter().find(|(a, b, _)| *a <= at && at < *b))
                            .map(|(_, _, k)| Seg::Link(*k))
                            .or_else(|| {
                                langs.and_then(|l| {
                                    l.iter()
                                        .position(|(a, b, _)| *a <= at && at < *b)
                                        .map(Seg::Lang)
                                })
                            })
                            .unwrap_or(Seg::Text)
                    }
                };
                prev = key;
                match out.last_mut() {
                    Some((seg, pieces)) if *seg == key => match pieces.last_mut() {
                        Some(p) if p.run == ri && p.to == gi => p.to = gi + 1,
                        _ => pieces.push(Piece {
                            run: ri,
                            from: gi,
                            to: gi + 1,
                        }),
                    },
                    _ => out.push((
                        key,
                        vec![Piece {
                            run: ri,
                            from: gi,
                            to: gi + 1,
                        }],
                    )),
                }
            }
        }
        out
    }

    fn draw_line(&mut self, buf: &mut PageBuf, line: &Line) {
        if line.runs.iter().all(|r| r.glyphs.is_empty()) {
            return;
        }
        let owner = self.owner_of(line);
        let structured = matches!(owner, Owner::Struct(..));
        let main = line.story == StoryKind::Main && structured;
        let key = line_key(line, self.section_of(line));
        let label_owner = if main {
            self.st
                .paras
                .get(line.paragraph)
                .and_then(|p| p.label)
                .map(|id| Owner::Struct(id, "Lbl".to_owned()))
        } else {
            None
        };
        // Links and languages are looked for in the main story, the notes and (links only, they
        // are artifacts) the header and footer.
        let tagged = structured || self.link_spans.contains_key(&key);
        for (seg, pieces) in self.segments(line, key, tagged, label_owner.is_some()) {
            let seg_owner = match (seg, &owner) {
                (Seg::Label, _) => label_owner.clone().unwrap_or_else(|| owner.clone()),
                (Seg::Link(k), Owner::Struct(parent, _)) => self.link_owner(key, k, *parent),
                (Seg::Lang(i), Owner::Struct(parent, _)) => self.lang_owner(key, i, *parent),
                _ => owner.clone(),
            };
            self.begin(buf, &seg_owner);
            let glyphs: Vec<&PlacedGlyph> = pieces
                .iter()
                .flat_map(|p| line.runs[p.run].glyphs[p.from..p.to].iter())
                .collect();
            let actual = self.actual_text(line, &glyphs);
            if let Some(t) = &actual {
                buf.content
                    .push_str(&format!("/Span << /ActualText {} >> BDC\n", text_string(t)));
            }
            for p in &pieces {
                self.draw_run(buf, line, &line.runs[p.run], p.from..p.to);
            }
            if actual.is_some() {
                buf.content.push_str("EMC\n");
            }
            buf.content.push_str("EMC\n");
            if let Seg::Link(k) = seg {
                let x0 = glyphs.iter().map(|g| g.x).fold(f64::INFINITY, f64::min);
                let x1 = glyphs
                    .iter()
                    .map(|g| g.x + g.advance)
                    .fold(f64::NEG_INFINITY, f64::max);
                if x0.is_finite() && x1.is_finite() {
                    self.add_link(buf, line, key, k, (x0, x1));
                }
            }
        }
    }

    /// The `Link` element of hyperlink `k` of `paragraph`, made on first use and hung under the
    /// paragraph's element at that point of the reading order.
    fn link_owner(&mut self, paragraph: usize, k: usize, parent: u32) -> Owner {
        let id = match self.link_elems.get(&(paragraph, k)) {
            Some(l) => l.id,
            None => {
                let id = self.pdf.reserve();
                self.kids.entry(parent).or_default().push(Kid::Elem(id));
                self.link_elems.insert(
                    (paragraph, k),
                    LinkElem {
                        id,
                        parent,
                        annots: Vec::new(),
                    },
                );
                id
            }
        };
        Owner::Struct(id, "Link".to_owned())
    }

    /// The `Span` element carrying the `/Lang` of language span `i` of a paragraph, made on first
    /// use and hung under the paragraph's element.
    fn lang_owner(&mut self, key: usize, i: usize, parent: u32) -> Owner {
        let lang = self
            .lang_spans
            .get(&key)
            .and_then(|l| l.get(i))
            .map(|(_, _, l)| l.clone())
            .unwrap_or_default();
        let id = match self.lang_elems.get(&(key, i)) {
            Some(l) => l.id,
            None => {
                let id = self.pdf.reserve();
                self.kids.entry(parent).or_default().push(Kid::Elem(id));
                self.lang_elems
                    .insert((key, i), LangElem { id, parent, lang });
                id
            }
        };
        Owner::Struct(id, "Span".to_owned())
    }

    /// The section a line's story belongs to (headers and footers differ per section).
    fn section_of(&self, line: &Line) -> usize {
        match line.story {
            StoryKind::Main | StoryKind::Footnotes | StoryKind::Endnotes => 0,
            _ => self.current_section,
        }
    }

    /// A `Link` annotation over `x` on `line`, to the hyperlink's target.
    fn add_link(&mut self, buf: &mut PageBuf, line: &Line, key: usize, k: usize, x: (f64, f64)) {
        let Some(spec) = self.st.links.get(&key).and_then(|l| l.get(k)).cloned() else {
            return;
        };
        let action = match &spec.target {
            LinkTarget::Uri(u) => format!("/A << /S /URI /URI ({}) >>", uri_string(u)),
            LinkTarget::Anchor(name) => {
                let dest = self
                    .st
                    .bookmarks
                    .get(name)
                    .and_then(|p| self.para_first.get(p))
                    .map(|(page, left, top)| {
                        format!(
                            "/Dest [{} 0 R /XYZ {} {} 0]",
                            self.page_ids[*page],
                            num(*left, 3),
                            num(self.page_dims[*page].1 - top, 3)
                        )
                    });
                match dest {
                    Some(d) => d,
                    None => {
                        self.warnings.push(format!(
                            "hyperlink to unknown bookmark {name} left inactive"
                        ));
                        return;
                    }
                }
            }
        };
        let annot = self.pdf.reserve();
        let mut parent_key = self.link_elems.get_mut(&(key, k)).map(|l| {
            l.annots.push((buf.index, annot));
            l.id
        });
        if parent_key.is_none() {
            // A link in an artifact (header, footer): its annotation still gets a `Link` element
            // under the document element, so that no link annotation is left untagged.
            let id = self.pdf.reserve();
            let root = self.st.root.id;
            self.kids.entry(root).or_default().push(Kid::Elem(id));
            self.free_links += 1;
            self.link_elems.insert(
                (usize::MAX - self.free_links, 0),
                LinkElem {
                    id,
                    parent: root,
                    annots: vec![(buf.index, annot)],
                },
            );
            parent_key = Some(id);
        }
        let struct_parent = parent_key.map(|id| {
            self.struct_parents.push(id);
            self.layout.pages.len() + self.struct_parents.len() - 1
        });
        let sp = struct_parent.map_or(String::new(), |k| format!(" /StructParent {k}"));
        self.pdf.set(
            annot,
            format!(
                "<< /Type /Annot /Subtype /Link /Rect [{} {} {} {}] /Border [0 0 0] /F 4 {action} /Contents {}{sp} >>",
                num(x.0, 3),
                num(buf.y(line.top + line.height), 3),
                num(x.1, 3),
                num(buf.y(line.top), 3),
                text_string(&spec.text)
            ),
        );
        self.page_annots[buf.index].push(annot);
    }

    /// The text of the glyphs when the ToUnicode CMap cannot say it glyph by glyph: a glyph that
    /// stands for other characters than the CMap gives it (a character the font draws with the
    /// glyph of another, an accent merged into its base). `None` when the CMap is enough.
    fn actual_text(&self, line: &Line, glyphs: &[&PlacedGlyph]) -> Option<String> {
        let private = |c: char| ('\u{e000}'..='\u{f8ff}').contains(&c);
        let mut text = String::new();
        let mut differs = false;
        for g in glyphs {
            if g.cluster.kind != ClusterKind::Text {
                continue;
            }
            let t = line.cluster_text(g);
            if t.is_empty() {
                continue;
            }
            let mapped = self.to_unicode.get(&(g.face.index(), g.glyph_id));
            let shown_case = mapped.is_some_and(|u| {
                let mut it = u.chars();
                matches!((it.next(), it.next()), (Some(c), None) if differs_by_case(t, c))
            });
            text.push_str(if shown_case {
                mapped.map_or(t, String::as_str)
            } else {
                t
            });
            if !shown_case
                && !t.chars().any(private)
                && mapped.is_none_or(|u| u != t)
                && !t.chars().all(|c| c.is_control())
            {
                differs = true;
            }
        }
        differs.then_some(text)
    }

    /// Draws glyphs `range` of `run`.
    fn draw_run(
        &mut self,
        buf: &mut PageBuf,
        line: &Line,
        run: &GlyphRun,
        range: std::ops::Range<usize>,
    ) {
        let glyphs = &run.glyphs[range];
        let (Some(first), Some(last)) = (glyphs.first(), glyphs.last()) else {
            return;
        };
        if run.size.is_nan() || run.size <= 0.0 {
            return;
        }
        let color = run.color.map_or_else(|| "0 0 0".to_owned(), pdf_color);
        buf.content.push_str(&format!("{color} rg\n"));
        let mut start = 0;
        while start < glyphs.len() {
            let g0 = glyphs[start];
            if self.skip_notdef && g0.glyph_id == 0 {
                start += 1;
                continue;
            }
            let mut end = start + 1;
            while end < glyphs.len()
                && glyphs[end].face == g0.face
                && !(self.skip_notdef && glyphs[end].glyph_id == 0)
                && (glyphs[end].y_offset - g0.y_offset).abs() < 1e-6
            {
                end += 1;
            }
            let Some(fd) = self.faces.get(&g0.face.index()) else {
                start = end;
                continue;
            };
            let size = run.size;
            let x0 = (g0.x * 1e4).round() / 1e4;
            let y = buf.y(line.baseline) + g0.y_offset;
            let mut cursor = x0;
            let mut parts = String::new();
            let mut in_string = false;
            for (k, g) in glyphs[start..end].iter().enumerate() {
                if k > 0 {
                    let adj = (-(g.x - cursor) * 1000.0 / size * 1000.0).round() / 1000.0;
                    if adj != 0.0 {
                        if in_string {
                            parts.push('>');
                            in_string = false;
                        }
                        parts.push_str(&num(adj, 3));
                        cursor += -adj * size / 1000.0;
                    }
                }
                if !in_string {
                    parts.push('<');
                    in_string = true;
                }
                parts.push_str(&format!("{:04X}", g.glyph_id & 0xFFFF));
                let w = fd.widths.get(&g.glyph_id).copied().unwrap_or(0) as f64;
                cursor += w * size / 1000.0;
            }
            if in_string {
                parts.push('>');
            }
            buf.content.push_str(&format!(
                "BT /{} {} Tf 1 0 0 1 {} {} Tm [{parts}] TJ ET\n",
                fd.resource,
                num(size, 3),
                num(x0, 4),
                num(y, 4)
            ));
            start = end;
        }
        if (run.underline || run.strike)
            && let Some(fd) = self.faces.get(&first.face.index())
        {
            let x0 = first.x;
            let x1 = last.x + last.advance;
            let base = buf.y(line.baseline) + first.y_offset;
            let mut stroke = |pos: f64, thick: f64| {
                let yy = base + pos * run.size;
                buf.content.push_str(&format!(
                    "{} RG {} w {} {} m {} {} l S\n",
                    color,
                    num(thick * run.size, 3),
                    num(x0, 4),
                    num(yy, 4),
                    num(x1, 4),
                    num(yy, 4)
                ));
            };
            if run.underline {
                stroke(fd.underline_pos, fd.underline_thickness);
            }
            if run.strike {
                stroke(fd.strike_pos, fd.strike_thickness);
            }
        }
    }

    fn draw_cells(&mut self, buf: &mut PageBuf, page: &Page) {
        let mut shaded = false;
        for cell in &page.cells {
            let Some(fill) = cell.fill else {
                continue;
            };
            if !shaded {
                buf.content.push_str("/Artifact BMC\n");
                shaded = true;
            }
            buf.content.push_str(&format!(
                "{} rg {} {} {} {} re f\n",
                pdf_color(fill),
                num(cell.x, 4),
                num(buf.y(cell.y + cell.height), 4),
                num(cell.width, 4),
                num(cell.height, 4)
            ));
        }
        if shaded {
            buf.content.push_str("EMC\n");
        }
    }

    fn draw_borders(&mut self, buf: &mut PageBuf, page: &Page) {
        let mut open = false;
        for cell in &page.cells {
            let b = &cell.borders;
            let width = |l: &Option<BorderLine>| l.as_ref().map_or(0.0, |l| l.width);
            let (wl, wr, wt, wb) = (
                width(&b.left),
                width(&b.right),
                width(&b.top),
                width(&b.bottom),
            );
            let (x, y, w, h) = (cell.x, cell.y, cell.width, cell.height);
            // Rectangle of each strip in page coordinates (y down), corners included.
            let strips = [
                (&b.top, true, (x - wl, y - wt, w + wl + wr, wt)),
                (&b.bottom, true, (x - wl, y + h, w + wl + wr, wb)),
                (&b.left, false, (x - wl, y - wt, wl, h + wt + wb)),
                (&b.right, false, (x + w, y - wt, wr, h + wt + wb)),
            ];
            for (line, horizontal, rect) in strips {
                let Some(line) = line else { continue };
                if !open {
                    buf.content.push_str("/Artifact BMC\n");
                    open = true;
                }
                draw_border_strip(&mut buf.content, buf.height, line, horizontal, rect);
            }
        }
        if open {
            buf.content.push_str("EMC\n");
        }
    }

    /// Paragraph border rules (`Page::rules`), as artifacts.
    fn draw_rules(&mut self, buf: &mut PageBuf, page: &Page) {
        if page.rules.is_empty() {
            return;
        }
        buf.content.push_str(
            "/Artifact BMC
",
        );
        for r in &page.rules {
            let line = BorderLine {
                width: r.width.min(r.height),
                color: r.color,
                style: r.style,
            };
            let rect = (r.x, r.y, r.width, r.height);
            draw_border_strip(
                &mut buf.content,
                buf.height,
                &line,
                r.width >= r.height,
                rect,
            );
        }
        buf.content.push_str(
            "EMC
",
        );
    }

    /// Draws a picture; `tagged` pictures of the main story become Figure elements.
    fn draw_image(&mut self, buf: &mut PageBuf, img: &ImagePlacement, tagged: bool) {
        let Some(embed) = img.embed.as_deref() else {
            return;
        };
        let key = format!("{:?}|{embed}", img.source);
        if !self.images.contains_key(&key) {
            let entry = match image_part(self.doc, &img.source, embed) {
                None => {
                    self.warnings
                        .push(format!("picture {embed}: image part not found"));
                    None
                }
                Some((part_name, bytes)) => {
                    match image::embed(&mut self.pdf, bytes, self.flatten) {
                        Ok(id) => Some((format!("Im{}", self.images.len()), id)),
                        Err(e) => {
                            self.warnings
                                .push(format!("picture {embed} ({part_name}): {e}"));
                            None
                        }
                    }
                }
            };
            self.images.insert(key.clone(), entry);
        }
        let Some(Some((name, _))) = self.images.get(&key) else {
            return;
        };
        let name = name.clone();
        let parent = if tagged {
            self.st.paras.get(img.paragraph).map(|p| p.node)
        } else {
            None
        };
        let mut alt = img.description.clone().filter(|d| !d.trim().is_empty());
        if alt.is_none() && self.ua && tagged {
            // PDF/UA needs a description on every figure.
            alt = Some("Picture without description".to_owned());
        }
        match parent {
            Some(parent) => {
                let id = self.pdf.reserve();
                let mcid = buf.owners.len() as u32;
                buf.owners.push(id);
                if alt.is_none() {
                    self.warnings.push(format!(
                        "picture {embed} on page {} has no alternative text",
                        buf.index + 1
                    ));
                }
                self.figures.entry(parent).or_default().push(FigureRec {
                    id,
                    page: buf.index,
                    mcid,
                    alt,
                });
                buf.content
                    .push_str(&format!("/Figure << /MCID {mcid} >> BDC\n"));
            }
            None => buf.content.push_str("/Artifact BMC\n"),
        }
        buf.content.push_str(&format!(
            "q {} 0 0 {} {} {} cm /{name} Do Q\nEMC\n",
            num(img.width, 4),
            num(img.height, 4),
            num(img.x, 4),
            num(buf.y(img.y + img.height), 4)
        ));
    }

    fn write_structure(&mut self, page_owners: &[Vec<u32>]) -> u32 {
        let root_id = self.st.root.id;
        let tree_id = self.pdf.reserve();
        let root = std::mem::replace(
            &mut self.st.root,
            Node {
                kind: structure::Kind::Document,
                id: 0,
                kids: Vec::new(),
                attrs: String::new(),
            },
        );
        self.emit_node(&root, tree_id);
        // Extra elements (note lines) hang off the document element.
        let extras = self.extras.clone();
        for id in &extras {
            let k = self.kid_list(*id);
            self.pdf.set(
                *id,
                format!("<< /Type /StructElem /S /P /P {root_id} 0 R /K [{k}] >>"),
            );
        }
        // Hyperlinks: the marked content of the link text, then the annotations.
        let mut links: Vec<LinkOut> = self
            .link_elems
            .values()
            .map(|l| (l.id, l.parent, l.annots.clone()))
            .collect();
        links.sort_unstable_by_key(|l| l.0);
        for (id, parent, annots) in links {
            let mut k = self.kid_list(id);
            for (page, annot) in annots {
                k.push_str(&format!(
                    "<< /Type /OBJR /Obj {annot} 0 R /Pg {} 0 R >> ",
                    self.page_ids[page]
                ));
            }
            self.pdf.set(
                id,
                format!("<< /Type /StructElem /S /Link /P {parent} 0 R /K [{k}] >>"),
            );
        }
        let mut spans: Vec<(u32, u32, String)> = self
            .lang_elems
            .values()
            .map(|l| (l.id, l.parent, l.lang.clone()))
            .collect();
        spans.sort_unstable_by_key(|l| l.0);
        for (id, parent, lang) in spans {
            let k = self.kid_list(id);
            self.pdf.set(
                id,
                format!(
                    "<< /Type /StructElem /S /Span /P {parent} 0 R /Lang {} /K [{k}] >>",
                    text_string(&lang)
                ),
            );
        }
        let mut nums = String::new();
        for (i, owners) in page_owners.iter().enumerate() {
            let refs: String = owners.iter().map(|id| format!("{id} 0 R ")).collect();
            nums.push_str(&format!("{i} [{refs}] "));
        }
        for (j, id) in self.struct_parents.iter().enumerate() {
            nums.push_str(&format!("{} {id} 0 R ", page_owners.len() + j));
        }
        let parent_tree = self.pdf.add(format!("<< /Nums [{nums}] >>"));
        // Table header cells carry an /ID that the cells' /Headers name: the IDTree maps them.
        let mut ids = std::mem::take(&mut self.st.ids);
        ids.sort();
        let id_tree = if ids.is_empty() {
            String::new()
        } else {
            let names: String = ids
                .iter()
                .map(|(name, obj)| format!("{} {obj} 0 R ", text_string(name)))
                .collect();
            let tree = self.pdf.add(format!("<< /Names [{names}] >>"));
            format!(" /IDTree {tree} 0 R")
        };
        self.pdf.set(
            tree_id,
            format!(
                "<< /Type /StructTreeRoot /K {root_id} 0 R /ParentTree {parent_tree} 0 R{id_tree} /ParentTreeNextKey {} >>",
                page_owners.len() + self.struct_parents.len()
            ),
        );
        tree_id
    }

    /// The outline (Word's "bookmarks using headings"): a tree by heading level, each entry a
    /// destination at the top of its heading. A level that skips (H1 then H3) nests under the
    /// nearest shallower heading.
    fn write_outlines(&mut self, headings: &[Heading]) -> Option<u32> {
        if headings.is_empty() {
            return None;
        }
        let ids: Vec<u32> = headings.iter().map(|_| self.pdf.reserve()).collect();
        let root = self.pdf.reserve();
        let parent = outline_parents(&headings.iter().map(|h| h.level).collect::<Vec<_>>());
        let children = |p: Option<usize>| -> Vec<usize> {
            (0..headings.len()).filter(|i| parent[*i] == p).collect()
        };
        let descendants = |p: usize| -> usize {
            let mut n = 0;
            let mut frontier = vec![p];
            while let Some(x) = frontier.pop() {
                for c in children(Some(x)) {
                    n += 1;
                    frontier.push(c);
                }
            }
            n
        };
        for (i, h) in headings.iter().enumerate() {
            let siblings = children(parent[i]);
            let pos = siblings.iter().position(|s| *s == i).unwrap_or(0);
            let mut d = format!(
                "<< /Title {} /Parent {} 0 R /Dest [{} 0 R /XYZ {} {} 0]",
                text_string(&h.title),
                parent[i].map_or(root, |p| ids[p]),
                self.page_ids[h.page],
                num(h.x, 3),
                num(self.page_dims[h.page].1 - h.top, 3)
            );
            if pos > 0 {
                d.push_str(&format!(" /Prev {} 0 R", ids[siblings[pos - 1]]));
            }
            if let Some(next) = siblings.get(pos + 1) {
                d.push_str(&format!(" /Next {} 0 R", ids[*next]));
            }
            let kids = children(Some(i));
            if let (Some(first), Some(last)) = (kids.first(), kids.last()) {
                d.push_str(&format!(
                    " /First {} 0 R /Last {} 0 R /Count {}",
                    ids[*first],
                    ids[*last],
                    descendants(i)
                ));
            }
            d.push_str(" >>");
            self.pdf.set(ids[i], d);
        }
        let top = children(None);
        let (first, last) = (top.first()?, top.last()?);
        self.pdf.set(
            root,
            format!(
                "<< /Type /Outlines /First {} 0 R /Last {} 0 R /Count {} >>",
                ids[*first],
                ids[*last],
                headings.len()
            ),
        );
        Some(root)
    }

    fn emit_node(&mut self, node: &Node, parent: u32) {
        let mut k = String::new();
        for kid in &node.kids {
            k.push_str(&format!("{} 0 R ", kid.id));
        }
        k.push_str(&self.kid_list(node.id));
        if let Some(figs) = self.figures.remove(&node.id) {
            for f in figs {
                k.push_str(&format!("{} 0 R ", f.id));
                let alt = f
                    .alt
                    .as_deref()
                    .map_or(String::new(), |a| format!("/Alt {} ", text_string(a)));
                self.pdf.set(
                    f.id,
                    format!(
                        "<< /Type /StructElem /S /Figure /P {} 0 R {alt}/K [<< /Type /MCR /Pg {} 0 R /MCID {} >>] >>",
                        node.id, self.page_ids[f.page], f.mcid
                    ),
                );
            }
        }
        if node.kind == structure::Kind::Document {
            for id in &self.extras {
                k.push_str(&format!("{id} 0 R "));
            }
        }
        self.pdf.set(
            node.id,
            format!(
                "<< /Type /StructElem /S /{} /P {parent} 0 R {}/K [{k}] >>",
                node.kind.name(),
                if node.attrs.is_empty() {
                    String::new()
                } else {
                    format!("{} ", node.attrs)
                }
            ),
        );
        for kid in &node.kids {
            self.emit_node(kid, node.id);
        }
    }

    /// The children of a leaf element in reading order: its marked content and its links.
    fn kid_list(&self, id: u32) -> String {
        self.kids
            .get(&id)
            .into_iter()
            .flatten()
            .map(|kid| match kid {
                Kid::Mcr(page, mcid) => format!(
                    "<< /Type /MCR /Pg {} 0 R /MCID {mcid} >> ",
                    self.page_ids[*page]
                ),
                Kid::Elem(e) => format!("{e} 0 R "),
            })
            .collect()
    }
}

/// For each heading level in document order, the index of the heading it nests under (`None` at
/// the top). A level that skips (1 then 3) nests under the nearest shallower heading, and a
/// deeper heading after a shallower one never becomes its sibling.
fn outline_parents(levels: &[u8]) -> Vec<Option<usize>> {
    let mut parent = Vec::with_capacity(levels.len());
    let mut stack: Vec<usize> = Vec::new();
    for (i, level) in levels.iter().enumerate() {
        while stack.last().is_some_and(|j| levels[*j] >= *level) {
            stack.pop();
        }
        parent.push(stack.last().copied());
        stack.push(i);
    }
    parent
}

/// A URI as a PDF literal string: ASCII only, so anything else (and the string delimiters) is
/// written percent-encoded.
fn uri_string(url: &str) -> String {
    let mut out = String::new();
    for b in url.trim().bytes() {
        if b.is_ascii_graphic() && !matches!(b, b'(' | b')' | b'\\') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The part and bytes a picture's `embed` id names, through the relationships of the part the
/// layout says it belongs to.
fn image_part<'a>(
    doc: &'a Document,
    source: &ImageSource,
    embed: &str,
) -> Option<(String, &'a [u8])> {
    let owner = match source {
        ImageSource::Main => return doc.image_part(embed).map(|p| (p.name, p.bytes)),
        ImageSource::HeaderFooter(rel) => related(doc, doc.main_part(), |r| r.id == *rel)?,
        ImageSource::Footnotes => {
            related(doc, doc.main_part(), |r| r.rel_type.ends_with("/footnotes"))?
        }
        ImageSource::Endnotes => {
            related(doc, doc.main_part(), |r| r.rel_type.ends_with("/endnotes"))?
        }
    };
    let rel = relationships(doc, &owner)?.get(embed).cloned()?;
    if rel.target_mode != TargetMode::Internal {
        return None;
    }
    let name = resolve_target(Some(&owner), &rel.target).ok()?;
    let part = doc.package().part(name.as_str())?;
    Some((name.as_str().to_owned(), part.data()))
}

fn relationships(doc: &Document, part: &PartName) -> Option<Relationships> {
    let rels = doc.package().part(rels_part_name(Some(part)).as_str())?;
    Relationships::parse(rels.data()).ok()
}

fn related(
    doc: &Document,
    from: &PartName,
    pick: impl Fn(&open365_opc::Relationship) -> bool,
) -> Option<PartName> {
    let rels = relationships(doc, from)?;
    let rel = rels
        .iter()
        .find(|r| r.target_mode == TargetMode::Internal && pick(r))?;
    resolve_target(Some(from), &rel.target).ok()
}

fn draw_border_strip(
    out: &mut String,
    page_h: f64,
    b: &BorderLine,
    horizontal: bool,
    rect: (f64, f64, f64, f64),
) {
    let color = pdf_color(b.color);
    let (x, y, w, h) = rect;
    let fill = |out: &mut String, x: f64, y: f64, w: f64, h: f64| {
        out.push_str(&format!(
            "{color} rg {} {} {} {} re f\n",
            num(x, 4),
            num(page_h - y - h, 4),
            num(w, 4),
            num(h, 4)
        ));
    };
    match b.style {
        BorderStyle::Double => {
            let third = if horizontal { h / 3.0 } else { w / 3.0 };
            if horizontal {
                fill(out, x, y, w, third);
                fill(out, x, y + 2.0 * third, w, third);
            } else {
                fill(out, x, y, third, h);
                fill(out, x + 2.0 * third, y, third, h);
            }
        }
        BorderStyle::Dotted | BorderStyle::Dashed => {
            let thick = if horizontal { h } else { w };
            let on = if b.style == BorderStyle::Dotted {
                thick
            } else {
                thick * 3.0
            };
            let (x1, y1, x2, y2) = if horizontal {
                (x, y + h / 2.0, x + w, y + h / 2.0)
            } else {
                (x + w / 2.0, y, x + w / 2.0, y + h)
            };
            out.push_str(&format!(
                "q {color} RG {} w [{} {}] 0 d {} {} m {} {} l S Q\n",
                num(thick, 4),
                num(on, 3),
                num(on, 3),
                num(x1, 4),
                num(page_h - y1, 4),
                num(x2, 4),
                num(page_h - y2, 4)
            ));
        }
        BorderStyle::Solid => fill(out, x, y, w, h),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outline_nesting_follows_heading_levels() {
        // H1 H2 H2 H3 H1 H3 H2
        assert_eq!(
            outline_parents(&[1, 2, 2, 3, 1, 3, 2]),
            [None, Some(0), Some(0), Some(2), None, Some(4), Some(4)]
        );
        // The first heading is H2: it is a top-level entry; an H1 after it is too.
        assert_eq!(outline_parents(&[2, 1, 2]), [None, None, Some(1)]);
        assert!(outline_parents(&[]).is_empty());
    }

    #[test]
    fn a4_pages_take_words_600_dpi_size() {
        assert_eq!(page_dims(595.3, 841.9), (595.2, 841.92));
        assert_eq!(page_dims(841.9, 595.3), (841.92, 595.2));
        assert_eq!(page_dims(612.0, 792.0), (612.0, 792.0));
    }

    #[test]
    fn uris_are_written_as_ascii() {
        assert_eq!(
            uri_string("https://a.example/p q"),
            "https://a.example/p%20q"
        );
        assert_eq!(
            uri_string("https://a.example/(x)"),
            "https://a.example/%28x%29"
        );
        assert_eq!(
            uri_string("https://a.example/é"),
            "https://a.example/%C3%A9"
        );
    }
}
