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
    /// Export only these pages, 1-based and inclusive (Word's "Page(s) From .. To"); the end is
    /// clamped to the last page. Page numbers drawn in the text stay as laid out. Links and
    /// outline entries that lead to a page left out are dropped (links with a warning).
    pub pages: Option<(usize, usize)>,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            conformance: Conformance::None,
            bookmarks: true,
            ua: false,
            pages: None,
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

/// Word's `RoleMap`: custom element names it writes, with the standard type each stands for
/// (from its exports; the names it never emits in a plain document are kept so that a reader sees
/// the same map).
const ROLE_MAP: &str = "<< /Footnote /Note /Endnote /Note /Textbox /Sect /Header /Sect /Footer /Sect /InlineShape /Sect /Annotation /Sect /Artifact /Sect /Workbook /Document /Worksheet /Part /Macrosheet /Part /Chartsheet /Part /Dialogsheet /Part /Slide /Sect /Chart /Sect /Diagram /Figure /Title /H1 /CommentAnchor /Span /Notes /Sect >>";

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

/// The "language" of a [`LangSpan`] that marks a note reference mark: written as Word writes it,
/// a `Link` holding a `Span` (no `/Lang`) and the `Footnote` or `Endnote` element of the note
/// body. Not a valid language tag, so it cannot clash; it goes on with `F` or `E`, the note's
/// number among the notes of its kind, `:` and its number among all notes of the document.
const NOTE_REF: &str = "<note-ref>";

/// The note a [`NOTE_REF`] language stands for: (story code, note number, sequence number).
fn note_ref(lang: &str) -> Option<(u8, usize, usize)> {
    let rest = lang.strip_prefix(NOTE_REF)?;
    let code = if rest.starts_with('E') { 3 } else { 2 };
    let (no, seq) = rest.get(1..)?.split_once(':')?;
    Some((code, no.parse().ok()?, seq.parse().ok()?))
}

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
    /// (story code, note index) -> `Note` element (the entries of `extras`).
    note_elems: HashMap<(u8, usize), u32>,
    /// `Note` element -> the `P` elements of its paragraphs, in drawing order.
    note_paras: HashMap<u32, Vec<u32>>,
    /// (story code, note number) -> the `Link` element around the note's reference mark and the
    /// note's number among all notes.
    note_link: HashMap<(u8, usize), (u32, usize)>,
    /// (story code, note number) -> (page, x, top) of the note's first line.
    note_first: HashMap<(u8, usize), (usize, f64, f64)>,
    /// Layout paragraph number of the first endnote paragraph.
    end_base: usize,
    /// (story code, frame, paragraph) -> story offset of the paragraph's last and first line.
    last_line: HashMap<(u8, usize, usize), usize>,
    first_line: HashMap<(u8, usize, usize), usize>,
    images: HashMap<String, Option<(String, u32)>>,
    /// Main-story paragraph -> page of its first line (empty lines count); the content on a later
    /// page is a continuation `Span` under the paragraph, as in Word's PDF.
    para_page: HashMap<usize, usize>,
    /// (paragraph, page) -> the continuation `Span`.
    cont_spans: HashMap<(usize, usize), u32>,
    /// Every continuation `Span` made: (paragraph, element). A link or language run ends the open
    /// one (Word's order is `Span, Link, Span` under the `P`).
    cont_all: Vec<(usize, u32)>,
    /// (box, paragraph in the box) -> the `P` of a text box's paragraph.
    box_p: HashMap<(usize, usize), u32>,
    /// `Textbox` element -> its paragraphs in drawing order.
    box_kids: HashMap<u32, Vec<u32>>,
    /// Glyph (face, id) to the text the font's ToUnicode CMap gives it.
    to_unicode: BTreeMap<(usize, u32), String>,
    /// Main-story paragraph -> (page index, x, top) of its first line.
    para_first: HashMap<usize, (usize, f64, f64)>,
    link_spans: HashMap<usize, Vec<Span>>,
    lang_spans: HashMap<usize, Vec<LangSpan>>,
    lang_elems: HashMap<(usize, usize), LangElem>,
    link_elems: HashMap<(usize, usize), LinkElem>,
    /// `Link` element -> the `Span` inside it that holds the link text.
    link_span: HashMap<u32, u32>,
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

/// Word widens a link rectangle by 3 pixels of its 96 dpi device grid on each side.
const LINK_PAD: f64 = 2.25;

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
    if let Some((from, to)) = options.pages {
        let last = layout.pages.len();
        let (from, to) = (from.max(1), to.min(last));
        if from > to {
            return Err(PdfError::NoPages);
        }
        if (from, to) != (1, last) {
            let mut part = layout.clone();
            part.pages = layout.pages[from - 1..to].to_vec();
            let whole = PdfOptions {
                pages: None,
                ..*options
            };
            return export_pdf_with(doc, &part, fonts, meta, &whole, cancel);
        }
    }
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
    let tagged_tables: std::collections::HashSet<usize> = layout
        .pages
        .iter()
        .flat_map(|p| &p.cells)
        .filter(|c| {
            let b = &c.borders;
            c.fill.is_some()
                || b.top.is_some()
                || b.bottom.is_some()
                || b.left.is_some()
                || b.right.is_some()
        })
        .map(|c| c.table)
        .collect();
    let main_rels = relationships(doc, doc.main_part());
    let langs = Languages::read(doc);
    // The catalog language: the caller's, else `en`. Word does not take it from the document: its
    // exports of documents whose default language is de-DE, fr-FR or en-GB all say `en` (the
    // application's language), and every run in another language than that gets a `Span` with its
    // own `/Lang`, a whole German document included (evidence: `probes/p-lang-settings`,
    // `probe-lang*`). The caller (a desktop or CLI that knows the user's language) passes it in
    // `PdfMetadata::language`.
    let catalog_lang: Option<String> = Some(
        meta.language
            .clone()
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| "en".to_owned()),
    );
    let base_lang = catalog_lang.as_deref().and_then(lang::primary);
    let mut st = structure::analyse(
        doc,
        &roles,
        main_rels.as_ref(),
        &langs,
        base_lang.as_deref(),
        &tagged_tables,
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
    // The layout numbers the paragraphs of the endnotes after those of the body.
    let end_base = layout
        .all_lines()
        .filter(|(_, l)| l.story == StoryKind::Endnotes)
        .map(|(_, l)| l.paragraph)
        .min()
        .unwrap_or(0);
    let (para_first, link_spans, lang_spans, headings) = index_main_story(layout, &st, end_base);
    let mut note_first: HashMap<(u8, usize), (usize, f64, f64)> = HashMap::new();
    for (pi, page) in layout.pages.iter().enumerate() {
        for line in page.notes.iter().chain(&page.lines) {
            let code = line.story.word_code();
            if !matches!(line.story, StoryKind::Footnotes | StoryKind::Endnotes) {
                continue;
            }
            let para = note_paragraph(line, end_base);
            if let Some(no) = st.note_of.get(&(code, para)) {
                note_first
                    .entry((code, *no))
                    .or_insert((pi, line.left, line.top));
            }
        }
    }
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
        note_elems: HashMap::new(),
        note_paras: HashMap::new(),
        note_link: HashMap::new(),
        note_first,
        end_base,
        first_line: {
            let mut m: HashMap<(u8, usize, usize), usize> = HashMap::new();
            for (_, line) in layout.all_lines() {
                let key = (
                    line.story.word_code(),
                    line.frame.unwrap_or(0),
                    note_paragraph(line, end_base),
                );
                let e = m.entry(key).or_insert(line.start);
                *e = (*e).min(line.start);
            }
            m
        },
        last_line: {
            let mut m: HashMap<(u8, usize, usize), usize> = HashMap::new();
            for (_, line) in layout.all_lines() {
                let key = (
                    line.story.word_code(),
                    line.frame.unwrap_or(0),
                    note_paragraph(line, end_base),
                );
                let e = m.entry(key).or_insert(line.start);
                *e = (*e).max(line.start);
            }
            m
        },
        images: HashMap::new(),
        para_page: HashMap::new(),
        cont_spans: HashMap::new(),
        cont_all: Vec::new(),
        box_p: HashMap::new(),
        box_kids: HashMap::new(),
        to_unicode,
        link_span: HashMap::new(),
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

/// The index of a note line's paragraph in its notes story (the layout numbers the endnotes'
/// paragraphs after the body's).
fn note_paragraph(line: &Line, end_base: usize) -> usize {
    if line.story == StoryKind::Endnotes {
        line.paragraph.saturating_sub(end_base)
    } else {
        line.paragraph
    }
}

fn line_key(line: &Line, section: usize, end_base: usize) -> usize {
    // A text box is a story of its own: the section slot holds the box's number.
    let section = if line.story == StoryKind::TextBox {
        line.frame.unwrap_or(0)
    } else {
        section
    };
    story_key(line.story, section, note_paragraph(line, end_base))
}

/// The first line of every main-story paragraph, where each hyperlink and each foreign-language
/// run sits in the paragraph text the layout drew (found by aligning the model's text with it, so
/// a field that shows another value does not lose them), and the headings for the outline.
fn index_main_story(layout: &Layout, st: &DocStructure, end_base: usize) -> MainIndex {
    let mut first = HashMap::new();
    let mut spans: HashMap<usize, Vec<Span>> = HashMap::new();
    let mut langs: HashMap<usize, Vec<LangSpan>> = HashMap::new();
    let mut seen = std::collections::HashSet::new();
    let mut headings = Vec::new();
    // The notes in the order of their reference marks: the number among the notes of the kind
    // and among all notes.
    let mut mark_note: HashMap<(usize, usize), (bool, usize, usize)> = HashMap::new();
    let (mut counts, mut seq) = ([0usize; 2], 0usize);
    let mut text_keys: Vec<usize> = st.texts.keys().copied().collect();
    text_keys.sort_unstable();
    for key in text_keys {
        for (at, end) in &st.texts[&key].note_marks {
            seq += 1;
            let n = &mut counts[usize::from(*end)];
            mark_note.insert((key, *at), (*end, *n, seq));
            *n += 1;
        }
    }
    for (pi, page) in layout.pages.iter().enumerate() {
        for line in page
            .header
            .iter()
            .chain(&page.lines)
            .chain(page.frames.iter().flat_map(|f| f.lines.iter()))
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
            let key = line_key(line, page.section_index, end_base);
            if !seen.insert(key) {
                continue;
            }
            let Some(pt) = st.texts.get(&key) else {
                continue;
            };
            if pt.links.is_empty() && pt.langs.is_empty() && pt.note_marks.is_empty() {
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
            let mut found: Vec<LangSpan> = pt
                .langs
                .iter()
                .filter_map(|(r, l)| al.map(r.clone()).map(|m| (m.start, m.end, l.clone())))
                .collect();
            // A reference mark is a short run of digits or letters the model has no text for.
            for (at, _) in &pt.note_marks {
                let Some((end, no, seq)) = mark_note.get(&(key, *at)).copied() else {
                    continue;
                };
                if let Some(g) = al.gap_at(*at) {
                    // Without the paragraph mark or other control characters at its end.
                    let shown = line.text[g.clone()].trim_end_matches(char::is_control);
                    if !shown.is_empty()
                        && shown.len() <= 8
                        && shown.chars().all(char::is_alphanumeric)
                    {
                        let tag = format!("{NOTE_REF}{}{no}:{seq}", if end { 'E' } else { 'F' });
                        found.push((g.start, g.start + shown.len(), tag));
                    }
                }
            }
            found.sort_unstable_by_key(|f| f.0);
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
    // Text boxes: one story each, numbered as the layout numbers its frames. Only their links
    // are carried over; the text stays an artifact.
    let rels = relationships(doc, doc.main_part());
    for (id, (anchor, tb)) in st.boxes.clone().iter().enumerate() {
        let used = layout
            .pages
            .iter()
            .flat_map(|p| &p.frames)
            .any(|f| f.id == id && f.paragraph == *anchor);
        if !used {
            continue;
        }
        let texts = structure::story_texts(&tb.blocks, rels.as_ref(), langs, base);
        let texts = texts
            .into_iter()
            .map(|(mut t, l)| {
                t.langs.clear();
                (t, l)
            })
            .collect();
        put(StoryKind::TextBox, id, texts, st);
    }
    // Footnotes and endnotes.
    for (story, notes, suffix) in [
        (StoryKind::Footnotes, doc.footnotes(), "/footnotes"),
        (StoryKind::Endnotes, doc.endnotes(), "/endnotes"),
    ] {
        let rels = related(doc, doc.main_part(), |r| r.rel_type.ends_with(suffix))
            .and_then(|part| relationships(doc, &part));
        let mut all = Vec::new();
        for (note_no, note) in notes
            .iter()
            .filter(|n| n.kind == open365_docx::NoteKind::Normal)
            .enumerate()
        {
            for block in &note.blocks {
                if matches!(block, open365_docx::Block::Paragraph(_)) {
                    st.note_of.insert((story.word_code(), all.len()), note_no);
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

    fn owner_of(&mut self, line: &Line, page: usize) -> Owner {
        match line.story {
            StoryKind::Main => match self.st.paras.get(line.paragraph) {
                // A repeated table header row shows the same text again: Word tags it as a
                // continuation `Span` of the original paragraph (`layout-table-pages`).
                Some(p) => {
                    let first = *self.para_page.entry(line.paragraph).or_insert(page);
                    if first == page {
                        Owner::Struct(p.node, p.tag.clone())
                    } else {
                        let node = p.node;
                        let id = match self.cont_spans.get(&(line.paragraph, page)) {
                            Some(id) => *id,
                            None => {
                                let id = self.pdf.reserve();
                                self.kids.entry(node).or_default().push(Kid::Elem(id));
                                self.cont_spans.insert((line.paragraph, page), id);
                                self.cont_all.push((line.paragraph, id));
                                id
                            }
                        };
                        Owner::Struct(id, "Span".to_owned())
                    }
                }
                None => Owner::Artifact,
            },
            StoryKind::TextBox => {
                let frame = line.frame.unwrap_or(0);
                let Some(&tb) = self.st.box_elems.get(frame) else {
                    return Owner::Artifact;
                };
                let id = match self.box_p.get(&(frame, line.paragraph)) {
                    Some(id) => *id,
                    None => {
                        let id = self.pdf.reserve();
                        self.box_p.insert((frame, line.paragraph), id);
                        self.box_kids.entry(tb).or_default().push(id);
                        id
                    }
                };
                Owner::Struct(id, "P".to_owned())
            }
            StoryKind::Footnotes | StoryKind::Endnotes => {
                let key = (line.story.word_code(), note_paragraph(line, self.end_base));
                let id = match self.extra_keys.get(&key) {
                    Some(id) => *id,
                    None => {
                        let id = self.pdf.reserve();
                        // The paragraphs of one note share a `Note` element.
                        let note = self.st.note_of.get(&key).copied().unwrap_or(key.1);
                        let nkey = (key.0, note);
                        let note_id = match self.note_elems.get(&nkey) {
                            Some(n) => *n,
                            None => {
                                let n = self.pdf.reserve();
                                self.extras.push(n);
                                self.note_elems.insert(nkey, n);
                                n
                            }
                        };
                        self.note_paras.entry(note_id).or_default().push(id);
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
        // Word writes a tab leader where it lies on the line, so that it copies out between the
        // text on either side; the layout lists the leaders after the text of the line.
        let mut order: Vec<(usize, usize, &PlacedGlyph)> = line
            .runs
            .iter()
            .enumerate()
            .flat_map(|(ri, run)| {
                run.glyphs
                    .iter()
                    .enumerate()
                    .map(move |(gi, g)| (ri, gi, g))
            })
            .collect();
        if order.iter().any(|(_, _, g)| line.leader_char(g).is_some()) {
            order.sort_by(|a, b| a.2.x.total_cmp(&b.2.x));
        }
        {
            for (ri, gi, g) in order {
                let c = g.cluster;
                let key = match c.kind {
                    // A heading's number is part of the heading's own marked content.
                    ClusterKind::Label if label_apart => Seg::Label,
                    ClusterKind::Label => Seg::Text,
                    // A tab leader is text of the paragraph, as in Word's PDF (one string of the
                    // leader character in the paragraph's span, no artifact, no ActualText).
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
            if line.story == StoryKind::Main {
                self.para_page.entry(line.paragraph).or_insert(buf.index);
            }
            return;
        }
        let owner = self.owner_of(line, buf.index);
        let structured = matches!(owner, Owner::Struct(..));
        let main = line.story == StoryKind::Main && structured;
        let key = line_key(line, self.section_of(line), self.end_base);
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
            // Links and language runs hang under the paragraph, also on a continuation page.
            let parent_of = |this: &Self, id: u32| -> u32 {
                if line.story == StoryKind::Main {
                    this.st.paras.get(line.paragraph).map_or(id, |p| p.node)
                } else {
                    id
                }
            };
            if matches!(seg, Seg::Link(_) | Seg::Lang(_)) && line.story == StoryKind::Main {
                self.cont_spans.remove(&(line.paragraph, buf.index));
            }
            let seg_owner = match (seg, &owner) {
                (Seg::Label, _) => label_owner.clone().unwrap_or_else(|| owner.clone()),
                (Seg::Link(k), Owner::Struct(parent, _)) => {
                    let parent = parent_of(self, *parent);
                    self.link_owner(key, k, parent)
                }
                (Seg::Lang(i), Owner::Struct(parent, _)) => {
                    let parent = parent_of(self, *parent);
                    self.lang_owner(key, i, parent)
                }
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
            if let Seg::Lang(i) = seg {
                let x0 = glyphs.iter().map(|g| g.x).fold(f64::INFINITY, f64::min);
                let x1 = glyphs
                    .iter()
                    .map(|g| g.x + g.advance)
                    .fold(f64::NEG_INFINITY, f64::max);
                if x0.is_finite() && x1.is_finite() {
                    self.add_note_link(buf, line, key, i, (x0, x1));
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
        // Word puts the text of a link in a `Span` inside the `Link`.
        let span = match self.link_span.get(&id) {
            Some(s) => *s,
            None => {
                let s = self.pdf.reserve();
                self.kids.entry(id).or_default().push(Kid::Elem(s));
                self.link_span.insert(id, s);
                s
            }
        };
        Owner::Struct(span, "Span".to_owned())
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
                let mut parent = parent;
                if let Some((code, no, seq)) = note_ref(&lang) {
                    // Word: the mark is a `Span` in a `Link` that also holds the note's body.
                    let link = self.pdf.reserve();
                    self.kids.entry(parent).or_default().push(Kid::Elem(link));
                    self.link_elems.insert(
                        (key, usize::MAX / 2 + i),
                        LinkElem {
                            id: link,
                            parent,
                            annots: Vec::new(),
                        },
                    );
                    self.note_link.insert((code, no), (link, seq));
                    parent = link;
                }
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

    /// The rectangle of a link annotation as Word makes it: the glyph extent widened by 3 px
    /// (2.25 pt) on each side, from the top of the line box to its bottom, which on the last line
    /// of a paragraph also holds the paragraph's space after (evidence: every link of the
    /// reference exports; `tests/word_structure.rs`). Returns `[x0, y0, x1, y1]` in PDF space.
    fn link_rect(&self, buf: &PageBuf, line: &Line, x: (f64, f64), pad_right: f64) -> [f64; 4] {
        let key = |l: &Line| {
            (
                l.story.word_code(),
                l.frame.unwrap_or(0),
                note_paragraph(l, self.end_base),
            )
        };
        let (mut above, mut below) = (0.0, 0.0);
        if line.story == StoryKind::Main
            && let Some(p) = self.st.paras.get(line.paragraph)
        {
            let prev = line
                .paragraph
                .checked_sub(1)
                .and_then(|i| self.st.paras.get(i))
                .filter(|_| !p.first);
            let next = self.st.paras.get(line.paragraph + 1);
            // Contextual spacing: a paragraph that has it drops its own space towards a
            // neighbour of the same style (`p-rect`: the other paragraph keeps its space).
            let same = |a: &structure::ParaMeta, b: &structure::ParaMeta| {
                a.contextual && a.style == b.style
            };
            if self.first_line.get(&key(line)) == Some(&line.start) {
                // Space above the paragraph that is not already below the one before it.
                above = match prev {
                    Some(q) => {
                        let before = if same(p, q) { 0.0 } else { p.before };
                        let after = if same(q, p) { 0.0 } else { q.after };
                        (before - after).max(0.0)
                    }
                    None => p.before,
                };
            }
            if self.last_line.get(&key(line)) == Some(&line.start) {
                below = match next {
                    Some(n) if same(p, n) => 0.0,
                    _ => p.after,
                };
            }
        }
        [
            x.0 - LINK_PAD,
            buf.y(line.top + line.height + below),
            x.1 + pad_right,
            buf.y(line.top - above),
        ]
    }

    /// The `Link` annotation over a note's reference mark, to the first line of the note.
    fn add_note_link(
        &mut self,
        buf: &mut PageBuf,
        line: &Line,
        key: usize,
        i: usize,
        x: (f64, f64),
    ) {
        let Some((code, no, _)) = self
            .lang_spans
            .get(&key)
            .and_then(|l| l.get(i))
            .and_then(|l| note_ref(&l.2))
        else {
            return;
        };
        let Some(&(page, left, top)) = self.note_first.get(&(code, no)) else {
            return;
        };
        let Some(&(link, _)) = self.note_link.get(&(code, no)) else {
            return;
        };
        let r = self.link_rect(buf, line, x, LINK_PAD);
        let annot = self.pdf.reserve();
        if let Some(l) = self.link_elems.get_mut(&(key, usize::MAX / 2 + i)) {
            l.annots.push((buf.index, annot));
        }
        self.struct_parents.push(link);
        let sp = self.layout.pages.len() + self.struct_parents.len() - 1;
        self.pdf.set(
            annot,
            format!(
                "<< /Type /Annot /Subtype /Link /Rect [{} {} {} {}] /Border [0 0 0] /F 4 /Dest [{} 0 R /XYZ {} {} 0] /StructParent {sp} >>",
                num(r[0], 3),
                num(r[1], 3),
                num(r[2], 3),
                num(r[3], 3),
                self.page_ids[page],
                // Word's destination: 3 pt left of the note, on whole points.
                num(left - 3.0, 0),
                num((self.page_dims[page].1 - top).round(), 0)
            ),
        );
        self.page_annots[buf.index].push(annot);
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
        if parent_key.is_none() && (self.ua || line.story == StoryKind::TextBox) {
            // A link in an artifact (header, footer): Word leaves its annotation untagged; PDF/UA
            // needs every link annotation in the tree, so there it gets a `Link` element under
            // the document element.
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
        let r = self.link_rect(buf, line, x, LINK_PAD);
        self.pdf.set(
            annot,
            format!(
                "<< /Type /Annot /Subtype /Link /Rect [{} {} {} {}] /Border [0 0 0] /F 4 {action} /Contents {}{sp} >>",
                num(r[0], 3),
                num(r[1], 3),
                num(r[2], 3),
                num(r[3], 3),
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
        // The note bodies: Word's `Footnote` or `Endnote` element inside the `Link` around the
        // reference mark (role-mapped to `Note`), holding the paragraphs of the note.
        let extras = self.extras.clone();
        let by_elem: HashMap<u32, (u8, usize)> =
            self.note_elems.iter().map(|(k, v)| (*v, *k)).collect();
        let mut loose = self.note_link.values().map(|l| l.1).max().unwrap_or(0);
        for id in &extras {
            let paras = self.note_paras.get(id).cloned().unwrap_or_default();
            for p in &paras {
                let k = self.kid_list(*p);
                self.pdf.set(
                    *p,
                    format!("<< /Type /StructElem /S /P /P {id} 0 R /K [{k}] >>"),
                );
            }
            let key = by_elem.get(id).copied().unwrap_or((2, 0));
            let (parent, seq) = match self.note_link.get(&key) {
                Some(l) => *l,
                None => {
                    loose += 1;
                    (root_id, loose)
                }
            };
            let name = format!("Note {seq}");
            let role = if key.0 == 3 { "Endnote" } else { "Footnote" };
            let kids: String = paras.iter().map(|p| format!("{p} 0 R ")).collect();
            self.pdf.set(
                *id,
                format!(
                    "<< /Type /StructElem /S /{role} /ID {} /P {parent} 0 R /K [{kids}] >>",
                    text_string(&name)
                ),
            );
            self.st.ids.push((name, *id));
        }
        // Hyperlinks: the marked content of the link text, then the annotations.
        let mut links: Vec<LinkOut> = self
            .link_elems
            .values()
            .map(|l| (l.id, l.parent, l.annots.clone()))
            .collect();
        links.sort_unstable_by_key(|l| l.0);
        let note_links: HashMap<u32, (u8, usize)> =
            self.note_link.iter().map(|(k, v)| (v.0, *k)).collect();
        for (id, parent, annots) in links {
            let objrs: String = annots
                .iter()
                .map(|(page, annot)| {
                    format!(
                        "<< /Type /OBJR /Obj {annot} 0 R /Pg {} 0 R >> ",
                        self.page_ids[*page]
                    )
                })
                .collect();
            let k = match note_links.get(&id) {
                // Word's order: the annotation, the mark, the note.
                Some(key) => {
                    let note = self
                        .note_elems
                        .get(key)
                        .map_or(String::new(), |n| format!("{n} 0 R "));
                    format!("{objrs}{}{note}", self.kid_list(id))
                }
                None => format!("{objrs}{}", self.kid_list(id)),
            };
            if let Some(span) = self.link_span.get(&id).copied() {
                let sk = self.kid_list(span);
                self.pdf.set(
                    span,
                    format!("<< /Type /StructElem /S /Span /P {id} 0 R /K [{sk}] >>"),
                );
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
            let dict = if lang.starts_with(NOTE_REF) {
                format!("<< /Type /StructElem /S /Span /P {parent} 0 R /K [{k}] >>")
            } else {
                format!(
                    "<< /Type /StructElem /S /Span /P {parent} 0 R /Lang {} /K [{k}] >>",
                    text_string(&lang)
                )
            };
            self.pdf.set(id, dict);
        }
        // Content of a paragraph on a later page than its first line: a `Span` under the `P`.
        for (para, id) in std::mem::take(&mut self.cont_all) {
            let parent = self.st.paras.get(para).map_or(root_id, |p| p.node);
            let k = self.kid_list(id);
            self.pdf.set(
                id,
                format!("<< /Type /StructElem /S /Span /P {parent} 0 R /K [{k}] >>"),
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
                "<< /Type /StructTreeRoot /K {root_id} 0 R /RoleMap {ROLE_MAP} /ParentTree {parent_tree} 0 R{id_tree} /ParentTreeNextKey {} >>",
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
        // The marked content of a list item's body comes before the lists nested in it.
        let mut k = if node.kind == structure::Kind::LBody {
            self.kid_list(node.id)
        } else {
            String::new()
        };
        for kid in &node.kids {
            if self.is_empty_leaf(kid) {
                continue;
            }
            k.push_str(&format!("{} 0 R ", kid.id));
            // Word writes a picture's `Figure` after the paragraph that holds it, not inside it.
            if let Some(figs) = self.figures.remove(&kid.id) {
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
        }
        if node.kind != structure::Kind::LBody {
            k.push_str(&self.kid_list(node.id));
        }
        if node.kind == structure::Kind::Textbox {
            for p in self.box_kids.get(&node.id).cloned().unwrap_or_default() {
                k.push_str(&format!("{p} 0 R "));
                let pk = self.kid_list(p);
                self.pdf.set(
                    p,
                    format!("<< /Type /StructElem /S /P /P {} 0 R /K [{pk}] >>", node.id),
                );
            }
        }
        if node.kind == structure::Kind::Document {
            // The notes whose mark was found hang under its `Link`.
            let placed: Vec<u32> = self
                .note_elems
                .iter()
                .filter(|(key, _)| self.note_link.contains_key(*key))
                .map(|(_, id)| *id)
                .collect();
            for id in self.extras.iter().filter(|id| !placed.contains(id)) {
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
            if !self.is_empty_leaf(kid) {
                self.emit_node(kid, node.id);
            }
        }
    }

    /// An element Word leaves out: a text box that drew nothing (not on a page), a paragraph
    /// that ends a section and drew nothing (`layout-sect-empty`).
    fn is_empty_leaf(&self, node: &Node) -> bool {
        match node.kind {
            structure::Kind::Textbox => !self.box_kids.contains_key(&node.id),
            structure::Kind::P | structure::Kind::H(_) => {
                self.st.section_breaks.contains(&node.id)
                    && self.kids.get(&node.id).is_none_or(Vec::is_empty)
                    && !self.figures.contains_key(&node.id)
            }
            _ => false,
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
