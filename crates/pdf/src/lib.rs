//! PDF 1.7 export of a laid-out document (layer 4, ADR-004: the print path).
//!
//! [`export_pdf`] takes the DOCX model, its [`Layout`] and the [`FontLibrary`] the layout used,
//! and writes a tagged PDF: subset fonts with a ToUnicode CMap, glyphs placed at the layout's
//! exact positions, underline and strike, table shading and borders, pictures, page sizes from
//! the sections, and a structure tree (P, H1 to H6, L/LI/LBody, Table/TR/TH/TD, Figure with Alt).
//! See `docs/architecture/pdf-export.md` for what is and is not covered.
//!
//! Pictures are untrusted input: see [`image`] for the limits.

mod font;
mod image;
mod structure;
mod writer;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;

use open365_docx::Document;
use open365_layout::{
    BorderLine, BorderStyle, ClusterKind, GlyphRun, ImagePlacement, ImageSource, Layout, Line,
    Page, ParaRole, StoryKind,
};
use open365_opc::{PartName, Relationships, TargetMode, rels_part_name, resolve_target};
use open365_text::FontLibrary;

use font::FaceDraw;
use structure::{DocStructure, Node};
use writer::{Pdf, digest, num, text_string, xml_escape};

pub use image::{MAX_BYTES as MAX_IMAGE_BYTES, MAX_PIXELS as MAX_IMAGE_PIXELS};

/// Document information supplied by the caller. Nothing is filled in by default: no author, no
/// dates, so an export does not leak the machine or the user.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PdfMetadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    /// The producing application; defaults to `Open365`.
    pub creator: Option<String>,
    /// A PDF date string such as `D:20260101120000Z`, written as is.
    pub created: Option<String>,
    /// BCP 47 language tag of the document (`/Lang`).
    pub language: Option<String>,
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

struct FigureRec {
    id: u32,
    page: usize,
    mcid: u32,
    alt: Option<String>,
}

struct Export<'a> {
    layout: &'a Layout,
    doc: &'a Document,
    st: DocStructure,
    faces: BTreeMap<usize, FaceDraw>,
    page_ids: Vec<u32>,
    /// struct element id -> (page index, mcid) pairs.
    mcrs: HashMap<u32, Vec<(usize, u32)>>,
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

/// Writes `layout` of `doc` as a tagged PDF 1.7 file.
///
/// `fonts` must be the library the layout was made with.
pub fn export_pdf(
    doc: &Document,
    layout: &Layout,
    fonts: &FontLibrary,
    meta: &PdfMetadata,
) -> Result<PdfOutput, PdfError> {
    if layout.pages.is_empty() {
        return Err(PdfError::NoPages);
    }
    let mut pdf = Pdf::default();
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
    let st = structure::analyse(doc, &roles, &mut pdf);
    let resources_id = pdf.reserve();
    let pages_id = pdf.reserve();
    let page_ids: Vec<u32> = layout.pages.iter().map(|_| pdf.reserve()).collect();
    let mut ex = Export {
        layout,
        doc,
        st,
        faces: face_draw,
        page_ids,
        mcrs: HashMap::new(),
        figures: HashMap::new(),
        extras: Vec::new(),
        extra_keys: HashMap::new(),
        images: HashMap::new(),
        drawn: std::collections::HashSet::new(),
        to_unicode,
        warnings,
        pdf,
    };

    let mut page_owners: Vec<Vec<u32>> = Vec::new();
    for (i, page) in layout.pages.iter().enumerate() {
        let (content, owners) = ex.render_page(i, page);
        let content_id = ex.pdf.add_stream("", content.as_bytes(), true);
        let (w, h) = (page.section.page_width, page.section.page_height);
        ex.pdf.set(
            ex.page_ids[i],
            format!(
                "<< /Type /Page /Parent {pages_id} 0 R /MediaBox [0 0 {} {}] /Resources {resources_id} 0 R /Contents {content_id} 0 R /StructParents {i} /Tabs /S >>",
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

    let struct_root = ex.write_structure(&page_owners);
    let info = write_info(&mut ex.pdf, meta);
    let xmp = write_xmp(&mut ex.pdf, meta);
    let catalog = ex.pdf.reserve();
    let mut cat = format!(
        "<< /Type /Catalog /Pages {pages_id} 0 R /MarkInfo << /Marked true >> /StructTreeRoot {struct_root} 0 R /Metadata {xmp} 0 R"
    );
    if let Some(lang) = meta.language.as_deref().filter(|l| !l.is_empty()) {
        cat.push_str(&format!(" /Lang {}", text_string(lang)));
    }
    if meta.title.as_deref().is_some_and(|t| !t.is_empty()) {
        cat.push_str(" /ViewerPreferences << /DisplayDocTitle true >>");
    }
    cat.push_str(" >>");
    ex.pdf.set(catalog, cat);

    let warnings = std::mem::take(&mut ex.warnings);
    let identity = digest(
        format!(
            "{:?}{}{}",
            meta,
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
            if described.is_none() || text.chars().nth(1).is_some() {
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

fn write_info(pdf: &mut Pdf, meta: &PdfMetadata) -> u32 {
    let mut d = String::from("<< ");
    for (key, value) in [
        ("Title", &meta.title),
        ("Author", &meta.author),
        ("Subject", &meta.subject),
        ("Keywords", &meta.keywords),
        ("CreationDate", &meta.created),
    ] {
        if let Some(v) = value.as_deref().filter(|v| !v.is_empty()) {
            d.push_str(&format!("/{key} {} ", text_string(v)));
        }
    }
    let creator = meta.creator.as_deref().unwrap_or("Open365");
    d.push_str(&format!(
        "/Creator {} /Producer (Open365) >>",
        text_string(creator)
    ));
    pdf.add(d)
}

fn write_xmp(pdf: &mut Pdf, meta: &PdfMetadata) -> u32 {
    let mut x = String::from(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\"><dc:format>application/pdf</dc:format>",
    );
    if let Some(t) = meta.title.as_deref().filter(|t| !t.is_empty()) {
        x.push_str(&format!(
            "<dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:title>",
            xml_escape(t)
        ));
    }
    if let Some(a) = meta.author.as_deref().filter(|a| !a.is_empty()) {
        x.push_str(&format!(
            "<dc:creator><rdf:Seq><rdf:li>{}</rdf:li></rdf:Seq></dc:creator>",
            xml_escape(a)
        ));
    }
    x.push_str("<pdf:Producer>Open365</pdf:Producer></rdf:Description></rdf:RDF></x:xmpmeta>\n<?xpacket end=\"w\"?>");
    pdf.add_stream("/Type /Metadata /Subtype /XML", x.as_bytes(), false)
}

impl Export<'_> {
    fn render_page(&mut self, index: usize, page: &Page) -> (String, Vec<u32>) {
        let mut buf = PageBuf {
            index,
            height: page.section.page_height,
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

    /// Opens a marked-content sequence; returns whether one was opened.
    fn begin(&mut self, buf: &mut PageBuf, owner: &Owner) {
        match owner {
            Owner::Artifact => buf.content.push_str("/Artifact BMC\n"),
            Owner::Struct(id, tag) => {
                let mcid = buf.owners.len() as u32;
                buf.owners.push(*id);
                self.mcrs.entry(*id).or_default().push((buf.index, mcid));
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

    fn draw_line(&mut self, buf: &mut PageBuf, line: &Line) {
        if line.runs.iter().all(|r| r.glyphs.is_empty()) {
            return;
        }
        let owner = self.owner_of(line);
        self.begin(buf, &owner);
        let actual = self.actual_text(line);
        if let Some(t) = &actual {
            buf.content
                .push_str(&format!("/Span << /ActualText {} >> BDC\n", text_string(t)));
        }
        for run in &line.runs {
            self.draw_run(buf, line, run);
        }
        if actual.is_some() {
            buf.content.push_str("EMC\n");
        }
        buf.content.push_str("EMC\n");
    }

    /// The text of the line when the ToUnicode CMap cannot say it glyph by glyph: a glyph that
    /// stands for other characters than the CMap gives it (a character the font draws with the
    /// glyph of another, an accent merged into its base). `None` when the CMap is enough.
    fn actual_text(&self, line: &Line) -> Option<String> {
        let private = |c: char| ('\u{e000}'..='\u{f8ff}').contains(&c);
        let mut text = String::new();
        let mut differs = false;
        for g in line.runs.iter().flat_map(|r| &r.glyphs) {
            if g.cluster.kind != ClusterKind::Text {
                continue;
            }
            let t = line.cluster_text(g);
            if t.is_empty() {
                continue;
            }
            text.push_str(t);
            if !t.chars().any(private)
                && self
                    .to_unicode
                    .get(&(g.face.index(), g.glyph_id))
                    .is_none_or(|u| u != t)
                && !t.chars().all(|c| c.is_control())
            {
                differs = true;
            }
        }
        differs.then_some(text)
    }

    fn draw_run(&mut self, buf: &mut PageBuf, line: &Line, run: &GlyphRun) {
        let (Some(first), Some(last)) = (run.glyphs.first(), run.glyphs.last()) else {
            return;
        };
        if run.size.is_nan() || run.size <= 0.0 {
            return;
        }
        let color = run.color.map_or_else(|| "0 0 0".to_owned(), pdf_color);
        buf.content.push_str(&format!("{color} rg\n"));
        let mut start = 0;
        while start < run.glyphs.len() {
            let g0 = run.glyphs[start];
            let mut end = start + 1;
            while end < run.glyphs.len()
                && run.glyphs[end].face == g0.face
                && (run.glyphs[end].y_offset - g0.y_offset).abs() < 1e-6
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
            for (k, g) in run.glyphs[start..end].iter().enumerate() {
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
                Some((part_name, bytes)) => match image::embed(&mut self.pdf, bytes) {
                    Ok(id) => Some((format!("Im{}", self.images.len()), id)),
                    Err(e) => {
                        self.warnings
                            .push(format!("picture {embed} ({part_name}): {e}"));
                        None
                    }
                },
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
        let alt = img.description.clone().filter(|d| !d.trim().is_empty());
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
            let k = self.mcr_list(*id);
            self.pdf.set(
                *id,
                format!("<< /Type /StructElem /S /P /P {root_id} 0 R /K [{k}] >>"),
            );
        }
        let mut nums = String::new();
        for (i, owners) in page_owners.iter().enumerate() {
            let refs: String = owners.iter().map(|id| format!("{id} 0 R ")).collect();
            nums.push_str(&format!("{i} [{refs}] "));
        }
        let parent_tree = self.pdf.add(format!("<< /Nums [{nums}] >>"));
        self.pdf.set(
            tree_id,
            format!(
                "<< /Type /StructTreeRoot /K {root_id} 0 R /ParentTree {parent_tree} 0 R /ParentTreeNextKey {} >>",
                page_owners.len()
            ),
        );
        tree_id
    }

    fn emit_node(&mut self, node: &Node, parent: u32) {
        let mut k = String::new();
        for kid in &node.kids {
            k.push_str(&format!("{} 0 R ", kid.id));
        }
        k.push_str(&self.mcr_list(node.id));
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

    fn mcr_list(&self, id: u32) -> String {
        self.mcrs
            .get(&id)
            .into_iter()
            .flatten()
            .map(|(page, mcid)| {
                format!(
                    "<< /Type /MCR /Pg {} 0 R /MCID {mcid} >> ",
                    self.page_ids[*page]
                )
            })
            .collect()
    }
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
