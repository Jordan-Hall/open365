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

use open365_docx::{Border, Document, Shading};
use open365_layout::{CellBox, GlyphRun, Layout, Line, Page, StoryKind, list_labels};
use open365_text::FontLibrary;

use font::FaceDraw;
use structure::{DocStructure, Edge, Node};
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
    let learned = learn_ligatures(doc, layout, fonts, &used);
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

    let st = structure::analyse(doc, list_labels(doc), &mut pdf);
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

/// Glyphs that no character maps to (ligatures such as "ft" or "tti") are not described by the
/// font's cmap. The layout does not keep clusters, so the text they stand for is recovered by
/// lining each paragraph's glyphs up with the paragraph's text.
fn learn_ligatures(
    doc: &Document,
    layout: &Layout,
    fonts: &FontLibrary,
    used: &BTreeMap<usize, BTreeSet<u32>>,
) -> BTreeMap<usize, BTreeMap<u32, String>> {
    let rev: BTreeMap<usize, BTreeMap<u32, char>> = used
        .keys()
        .filter_map(|i| {
            let face = fonts.try_face(open365_text::FaceId::from_index(*i))?;
            Some((*i, font::reverse_map(face)))
        })
        .collect();
    let mut glyphs: BTreeMap<usize, Vec<(usize, u32)>> = BTreeMap::new();
    for (_, line) in layout.lines() {
        for run in line.runs.iter().filter(|r| !r.superscript) {
            glyphs
                .entry(line.paragraph)
                .or_default()
                .extend(run.glyphs.iter().map(|g| (g.face.index(), g.glyph_id)));
        }
    }
    let paragraphs = doc.body().paragraphs();
    let mut out: BTreeMap<usize, BTreeMap<u32, String>> = BTreeMap::new();
    for (index, g) in &glyphs {
        let Some(p) = paragraphs.get(*index) else {
            continue;
        };
        let text: Vec<char> = p
            .text()
            .chars()
            .filter(|c| !c.is_whitespace() && !c.is_control())
            .collect();
        // A list label comes first among the glyphs; try a few starting points.
        let found = (0..g.len().min(8)).find_map(|skip| align(&text, &g[skip..], &rev));
        for ((face, gid), s) in found.into_iter().flatten() {
            out.entry(face).or_default().entry(gid).or_insert(s);
        }
    }
    out
}

type GlyphKey = (usize, u32);

fn align(
    text: &[char],
    glyphs: &[GlyphKey],
    rev: &BTreeMap<usize, BTreeMap<u32, char>>,
) -> Option<Vec<(GlyphKey, String)>> {
    let char_of = |k: &GlyphKey| rev.get(&k.0).and_then(|m| m.get(&k.1)).copied();
    let (mut i, mut j) = (0, 0);
    let mut found = Vec::new();
    while j < text.len() {
        let key = *glyphs.get(i)?;
        match char_of(&key) {
            Some(c) if c.is_whitespace() => i += 1,
            Some(c) => {
                let s: Vec<char> = font::expand(c).chars().collect();
                if !text[j..].starts_with(&s) {
                    return None;
                }
                i += 1;
                j += s.len();
            }
            None => {
                // The next glyph that has a character tells where the ligature ends.
                let next: Vec<char> = glyphs[i + 1..]
                    .iter()
                    .filter_map(char_of)
                    .find(|c| !c.is_whitespace())
                    .map(|c| font::expand(c).chars().collect())
                    .unwrap_or_default();
                let k = (2..=4).find(|k| {
                    j + k <= text.len()
                        && (j + k == text.len()
                            || next.is_empty()
                            || text[j + k..].starts_with(&next))
                })?;
                found.push((key, text[j..j + k].iter().collect()));
                i += 1;
                j += k;
            }
        }
    }
    Some(found)
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

fn shading_fill(s: &Shading) -> Option<[u8; 3]> {
    let pattern = s.pattern.as_str();
    match pattern {
        "nil" | "none" => None,
        "solid" => s.color.or(s.fill),
        p => {
            if let Some(pct) = p.strip_prefix("pct").and_then(|n| n.parse::<f64>().ok()) {
                let t = (pct / 100.0).clamp(0.0, 1.0);
                let fg = s.color.unwrap_or([0, 0, 0]);
                let bg = s.fill.unwrap_or([255, 255, 255]);
                let mix =
                    |i: usize| (f64::from(fg[i]) * t + f64::from(bg[i]) * (1.0 - t)).round() as u8;
                Some([mix(0), mix(1), mix(2)])
            } else {
                s.fill
            }
        }
    }
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
            self.draw_image(&mut buf, img);
        }
        self.draw_borders(&mut buf, page);
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
            self.draw_image(&mut buf, img);
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
        for run in &line.runs {
            self.draw_run(buf, line, run);
        }
        buf.content.push_str("EMC\n");
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
            let Some((ti, r, ci)) = self.cell_of(page, cell) else {
                continue;
            };
            let Some(sh) = self.st.tables[ti].rows[r][ci]
                .shading
                .as_ref()
                .and_then(shading_fill)
            else {
                continue;
            };
            if !shaded {
                buf.content.push_str("/Artifact BMC\n");
                shaded = true;
            }
            buf.content.push_str(&format!(
                "{} rg {} {} {} {} re f\n",
                pdf_color(sh),
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
            let Some((ti, r, ci)) = self.cell_of(page, cell) else {
                continue;
            };
            let t = &self.st.tables[ti];
            let edges = [
                (Edge::Top, t.border(r, ci, Edge::Top)),
                (Edge::Bottom, t.border(r, ci, Edge::Bottom)),
                (Edge::Left, t.border(r, ci, Edge::Left)),
                (Edge::Right, t.border(r, ci, Edge::Right)),
            ];
            let width_of = |e: Edge| {
                edges
                    .iter()
                    .find(|(k, _)| std::mem::discriminant(k) == std::mem::discriminant(&e))
                    .and_then(|(_, b)| b.as_ref())
                    .map_or(0.0, |b: &Border| b.width)
            };
            let (wl, wr, wt, wb) = (
                width_of(Edge::Left),
                width_of(Edge::Right),
                width_of(Edge::Top),
                width_of(Edge::Bottom),
            );
            for (edge, border) in &edges {
                let Some(b) = border else { continue };
                if b.width <= 0.0 {
                    continue;
                }
                if !open {
                    buf.content.push_str("/Artifact BMC\n");
                    open = true;
                }
                let (x, y, w, h) = (cell.x, cell.y, cell.width, cell.height);
                // Rectangle of the strip in page coordinates (y down), corners included.
                let rect = match edge {
                    Edge::Top => (x - wl, y - b.width, w + wl + wr, b.width),
                    Edge::Bottom => (x - wl, y + h, w + wl + wr, b.width),
                    Edge::Left => (x - b.width, y - wt, b.width, h + wt + wb),
                    Edge::Right => (x + w, y - wt, b.width, h + wt + wb),
                };
                draw_border_strip(&mut buf.content, buf.height, b, *edge, rect);
            }
        }
        if open {
            buf.content.push_str("EMC\n");
        }
    }

    /// The table cell a page rectangle belongs to: found through the text inside it.
    fn cell_of(&self, page: &Page, cell: &CellBox) -> Option<(usize, usize, usize)> {
        let mut best: Option<(usize, usize, usize, usize)> = None;
        for line in &page.lines {
            if line.top + 0.5 < cell.y
                || line.top > cell.y + cell.height + 0.5
                || line.left + 0.5 < cell.x
                || line.left > cell.x + cell.width + 0.5
            {
                continue;
            }
            if let Some((t, r, c, depth)) = self.st.paras.get(line.paragraph).and_then(|p| p.cell)
                && best.is_none_or(|b| depth < b.3)
            {
                best = Some((t, r, c, depth));
            }
        }
        best.map(|(t, r, c, _)| (t, r, c))
    }

    fn draw_image(&mut self, buf: &mut PageBuf, img: &open365_layout::ImagePlacement) {
        let Some(embed) = img.embed.as_deref() else {
            return;
        };
        if !self.images.contains_key(embed) {
            let entry = match self.doc.image_part(embed) {
                None => {
                    self.warnings
                        .push(format!("picture {embed}: image part not found"));
                    None
                }
                Some(part) => match image::embed(&mut self.pdf, part.bytes) {
                    Ok(id) => Some((format!("Im{}", self.images.len()), id)),
                    Err(e) => {
                        self.warnings
                            .push(format!("picture {embed} ({}): {e}", part.name));
                        None
                    }
                },
            };
            self.images.insert(embed.to_owned(), entry);
        }
        let Some(Some((name, _))) = self.images.get(embed) else {
            return;
        };
        let name = name.clone();
        let parent = self.st.paras.get(img.paragraph).map(|p| p.node);
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

fn draw_border_strip(
    out: &mut String,
    page_h: f64,
    b: &Border,
    edge: Edge,
    rect: (f64, f64, f64, f64),
) {
    let color = pdf_color(b.color.unwrap_or([0, 0, 0]));
    let (x, y, w, h) = rect;
    let horizontal = matches!(edge, Edge::Top | Edge::Bottom);
    let fill = |out: &mut String, x: f64, y: f64, w: f64, h: f64| {
        out.push_str(&format!(
            "{color} rg {} {} {} {} re f\n",
            num(x, 4),
            num(page_h - y - h, 4),
            num(w, 4),
            num(h, 4)
        ));
    };
    match b.style.as_str() {
        "double" | "triple" | "thinThickSmallGap" | "thickThinSmallGap" => {
            let third = if horizontal { h / 3.0 } else { w / 3.0 };
            if horizontal {
                fill(out, x, y, w, third);
                fill(out, x, y + 2.0 * third, w, third);
            } else {
                fill(out, x, y, third, h);
                fill(out, x + 2.0 * third, y, third, h);
            }
        }
        "dotted" | "dashed" | "dotDash" | "dotDotDash" | "dashSmallGap" => {
            let thick = if horizontal { h } else { w };
            let on = if b.style == "dotted" {
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
        _ => fill(out, x, y, w, h),
    }
}
