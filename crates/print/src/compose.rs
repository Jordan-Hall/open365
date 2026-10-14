//! Page surgery on a finished PDF: cut the pages a job needs, repeat them for copies and place
//! them on sheets, and read a coarse "ink map" of a page for the live preview.
//!
//! The input is the PDF our own pipeline wrote (Writer: `open365-pdf`; Sheets: the sheet
//! printer), but it is parsed as untrusted bytes: stream sizes are bounded and nothing here
//! panics on a malformed file.
//!
//! [`compose`] writes every output sheet as a new page that draws the source pages as Form
//! XObjects, so a page that is printed three times is stored once. Links, the outline and the
//! structure tree are dropped: paper has no use for them.

use std::collections::HashMap;
use std::fmt;

use lopdf::content::Content;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

/// The most a page's content may decode to.
const MAX_CONTENT: usize = 64 * 1024 * 1024;
/// The most ink items one page reports.
const MAX_INK: usize = 4_000;

/// Why a PDF could not be read or composed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PdfError {
    /// The bytes are not a PDF this code can read (the parser's message).
    Parse(String),
    /// The PDF has no pages, or the job selects none.
    NoPages,
    /// The job names a source page the PDF does not have.
    BadPage(usize),
    /// The page tree is not usable (no catalog or page root).
    Structure(&'static str),
    /// Writing the result failed.
    Write(String),
}

impl fmt::Display for PdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(m) => write!(f, "The pages could not be read: {m}"),
            Self::NoPages => write!(f, "There are no pages to print."),
            Self::BadPage(i) => write!(f, "Page {} does not exist.", i + 1),
            Self::Structure(m) => write!(f, "The page tree is damaged: {m}"),
            Self::Write(m) => write!(f, "The print file could not be written: {m}"),
        }
    }
}

impl std::error::Error for PdfError {}

/// A PDF whose pages can be inspected and recomposed.
#[derive(Debug, Clone)]
pub struct PdfSource {
    doc: Document,
    pages: Vec<ObjectId>,
    boxes: Vec<[f64; 4]>,
}

impl PdfSource {
    /// Reads `bytes`.
    ///
    /// # Errors
    /// [`PdfError`] when the bytes are not a PDF or it has no pages.
    pub fn load(bytes: &[u8]) -> Result<Self, PdfError> {
        let doc = Document::load_mem(bytes).map_err(|e| PdfError::Parse(e.to_string()))?;
        let pages: Vec<ObjectId> = doc.get_pages().values().copied().collect();
        if pages.is_empty() {
            return Err(PdfError::NoPages);
        }
        let boxes = pages
            .iter()
            .map(|id| media_box(&doc, *id).unwrap_or([0.0, 0.0, 595.3, 841.9]))
            .collect();
        Ok(Self { doc, pages, boxes })
    }

    #[must_use]
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Width and height of page `i` (0-based) in points.
    #[must_use]
    pub fn page_size(&self, i: usize) -> Option<(f64, f64)> {
        self.boxes.get(i).map(|b| (b[2] - b[0], b[3] - b[1]))
    }

    /// A coarse picture of page `i`: where the text, rules, fills and pictures are.
    #[must_use]
    pub fn ink(&self, i: usize) -> Option<Ink> {
        let id = *self.pages.get(i)?;
        let b = self.boxes[i];
        let content = self.doc.get_page_content_with_limit(id, MAX_CONTENT).ok()?;
        let ops = Content::decode(&content).ok()?.operations;
        let fonts: HashMap<Vec<u8>, bool> = self
            .doc
            .get_page_fonts(id)
            .map(|f| {
                f.into_iter()
                    .map(|(k, d)| {
                        let wide = d
                            .get(b"Subtype")
                            .and_then(Object::as_name)
                            .is_ok_and(|n| n == b"Type0");
                        (k, wide)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut ink = Ink {
            width: b[2] - b[0],
            height: b[3] - b[1],
            items: Vec::new(),
        };
        let mut walker = Walker::new(fonts, (b[0], b[1]));
        walker.run(&ops, &mut ink.items);
        ink.items = merge_text(std::mem::take(&mut ink.items));
        ink.items.truncate(MAX_INK);
        Some(ink)
    }
}

/// The ink of one page, in points from the page's lower-left corner (y up).
#[derive(Debug, Clone, PartialEq)]
pub struct Ink {
    pub width: f64,
    pub height: f64,
    pub items: Vec<InkItem>,
}

/// One mark on a page. `gray` is 0 (black) to 255 (white).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InkItem {
    /// A line of text: its baseline-left origin, extent and height.
    Text {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        gray: u8,
    },
    /// A filled rectangle.
    Rect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        gray: u8,
    },
    /// A stroked line.
    Line {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        width: f64,
        gray: u8,
    },
    /// A picture or other drawn object.
    Picture { x: f64, y: f64, w: f64, h: f64 },
}

type Matrix = [f64; 6];
const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `m` then `n` (PDF `cm` order: the new matrix is applied first).
fn mul(m: &Matrix, n: &Matrix) -> Matrix {
    [
        m[0] * n[0] + m[1] * n[2],
        m[0] * n[1] + m[1] * n[3],
        m[2] * n[0] + m[3] * n[2],
        m[2] * n[1] + m[3] * n[3],
        m[4] * n[0] + m[5] * n[2] + n[4],
        m[4] * n[1] + m[5] * n[3] + n[5],
    ]
}

fn apply(m: &Matrix, x: f64, y: f64) -> (f64, f64) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

fn num(o: &Object) -> Option<f64> {
    match o {
        Object::Integer(i) => Some(*i as f64),
        Object::Real(r) => Some(f64::from(*r)),
        _ => None,
    }
}

fn nums(operands: &[Object]) -> Vec<f64> {
    operands.iter().filter_map(num).collect()
}

fn gray_of(v: &[f64]) -> u8 {
    let g = match v.len() {
        1 => v[0],
        3 => 0.3 * v[0] + 0.59 * v[1] + 0.11 * v[2],
        4 => (1.0 - v[3]) * (1.0 - (0.3 * v[0] + 0.59 * v[1] + 0.11 * v[2]).min(1.0)),
        _ => 0.0,
    };
    (g.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[derive(Clone)]
struct GState {
    ctm: Matrix,
    fill: u8,
    stroke: u8,
    line_width: f64,
}

struct Walker {
    fonts: HashMap<Vec<u8>, bool>,
    origin: (f64, f64),
    gs: GState,
    stack: Vec<GState>,
    tm: Matrix,
    tlm: Matrix,
    size: f64,
    wide: bool,
    leading: f64,
    path: Vec<(f64, f64, f64, f64)>,
    segs: Vec<(f64, f64, f64, f64)>,
    cur: (f64, f64),
}

impl Walker {
    fn new(fonts: HashMap<Vec<u8>, bool>, origin: (f64, f64)) -> Self {
        Self {
            fonts,
            origin,
            gs: GState {
                ctm: IDENTITY,
                fill: 0,
                stroke: 0,
                line_width: 1.0,
            },
            stack: Vec::new(),
            tm: IDENTITY,
            tlm: IDENTITY,
            size: 10.0,
            wide: false,
            leading: 0.0,
            path: Vec::new(),
            segs: Vec::new(),
            cur: (0.0, 0.0),
        }
    }

    fn to_page(&self, x: f64, y: f64) -> (f64, f64) {
        let (px, py) = apply(&self.gs.ctm, x, y);
        (px - self.origin.0, py - self.origin.1)
    }

    fn rect_out(&self, x: f64, y: f64, w: f64, h: f64) -> (f64, f64, f64, f64) {
        let a = self.to_page(x, y);
        let b = self.to_page(x + w, y + h);
        (
            a.0.min(b.0),
            a.1.min(b.1),
            (a.0 - b.0).abs(),
            (a.1 - b.1).abs(),
        )
    }

    fn move_text(&mut self, tx: f64, ty: f64) {
        self.tlm = mul(&[1.0, 0.0, 0.0, 1.0, tx, ty], &self.tlm);
        self.tm = self.tlm;
    }

    fn show(&mut self, glyph_bytes: usize, adjust: f64, out: &mut Vec<InkItem>) {
        let glyphs = if self.wide {
            glyph_bytes / 2
        } else {
            glyph_bytes
        };
        let adv = glyphs as f64 * self.size * 0.5 - adjust / 1000.0 * self.size;
        if glyphs > 0 && adv > 0.0 {
            let full = mul(&self.tm, &self.gs.ctm);
            let (x, y) = apply(&full, 0.0, 0.0);
            let sx = full[0].hypot(full[1]);
            let sy = full[2].hypot(full[3]);
            out.push(InkItem::Text {
                x: x - self.origin.0,
                y: y - self.origin.1,
                w: adv * sx,
                h: self.size * sy * 0.72,
                gray: self.gs.fill,
            });
        }
        self.tm = mul(&[1.0, 0.0, 0.0, 1.0, adv, 0.0], &self.tm);
    }

    fn run(&mut self, ops: &[lopdf::content::Operation], out: &mut Vec<InkItem>) {
        for op in ops {
            if out.len() >= MAX_INK * 4 {
                break;
            }
            let v = nums(&op.operands);
            match op.operator.as_str() {
                "q" => self.stack.push(self.gs.clone()),
                "Q" => {
                    if let Some(g) = self.stack.pop() {
                        self.gs = g;
                    }
                }
                "cm" if v.len() == 6 => {
                    let m = [v[0], v[1], v[2], v[3], v[4], v[5]];
                    self.gs.ctm = mul(&m, &self.gs.ctm);
                }
                "w" if !v.is_empty() => self.gs.line_width = v[0],
                "g" | "rg" | "k" => self.gs.fill = gray_of(&v),
                "G" | "RG" | "K" => self.gs.stroke = gray_of(&v),
                "BT" => {
                    self.tm = IDENTITY;
                    self.tlm = IDENTITY;
                }
                "Tf" => {
                    if let (Some(Object::Name(n)), Some(s)) =
                        (op.operands.first(), op.operands.get(1).and_then(num))
                    {
                        self.wide = self.fonts.get(n).copied().unwrap_or(false);
                        self.size = s.abs();
                    }
                }
                "TL" if !v.is_empty() => self.leading = v[0],
                "Tm" if v.len() == 6 => {
                    self.tm = [v[0], v[1], v[2], v[3], v[4], v[5]];
                    self.tlm = self.tm;
                }
                "Td" if v.len() == 2 => self.move_text(v[0], v[1]),
                "TD" if v.len() == 2 => {
                    self.leading = -v[1];
                    self.move_text(v[0], v[1]);
                }
                "T*" => self.move_text(0.0, -self.leading),
                "Tj" => {
                    if let Some(Object::String(s, _)) = op.operands.first() {
                        self.show(s.len(), 0.0, out);
                    }
                }
                "'" | "\"" => {
                    self.move_text(0.0, -self.leading);
                    if let Some(Object::String(s, _)) = op.operands.last() {
                        self.show(s.len(), 0.0, out);
                    }
                }
                "TJ" => {
                    if let Some(Object::Array(items)) = op.operands.first() {
                        let mut bytes = 0usize;
                        let mut adjust = 0.0;
                        for it in items {
                            match it {
                                Object::String(s, _) => bytes += s.len(),
                                other => adjust += num(other).unwrap_or(0.0),
                            }
                        }
                        self.show(bytes, adjust, out);
                    }
                }
                "re" if v.len() == 4 => {
                    let r = self.rect_out(v[0], v[1], v[2], v[3]);
                    self.path.push(r);
                }
                "m" if v.len() == 2 => self.cur = (v[0], v[1]),
                "l" if v.len() == 2 => {
                    let a = self.to_page(self.cur.0, self.cur.1);
                    let b = self.to_page(v[0], v[1]);
                    self.segs.push((a.0, a.1, b.0, b.1));
                    self.cur = (v[0], v[1]);
                }
                "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" => {
                    for &(x, y, w, h) in &self.path {
                        out.push(InkItem::Rect {
                            x,
                            y,
                            w,
                            h,
                            gray: self.gs.fill,
                        });
                    }
                    self.stroke(
                        op.operator.starts_with('B') || op.operator.starts_with('b'),
                        out,
                    );
                }
                "S" | "s" => self.stroke(true, out),
                "n" => {
                    self.path.clear();
                    self.segs.clear();
                }
                "Do" => {
                    let (x, y, w, h) = self.rect_out(0.0, 0.0, 1.0, 1.0);
                    out.push(InkItem::Picture { x, y, w, h });
                }
                _ => {}
            }
        }
    }

    fn stroke(&mut self, stroke: bool, out: &mut Vec<InkItem>) {
        if stroke {
            let width = self.gs.line_width * self.gs.ctm[0].hypot(self.gs.ctm[1]);
            for &(x1, y1, x2, y2) in &self.segs {
                out.push(InkItem::Line {
                    x1,
                    y1,
                    x2,
                    y2,
                    width,
                    gray: self.gs.stroke,
                });
            }
            for &(x, y, w, h) in &self.path {
                for (a, b, c, d) in [
                    (x, y, x + w, y),
                    (x + w, y, x + w, y + h),
                    (x + w, y + h, x, y + h),
                    (x, y + h, x, y),
                ] {
                    out.push(InkItem::Line {
                        x1: a,
                        y1: b,
                        x2: c,
                        y2: d,
                        width,
                        gray: self.gs.stroke,
                    });
                }
            }
        }
        self.path.clear();
        self.segs.clear();
    }
}

/// Joins text pieces on one baseline that nearly touch, so a line is one bar.
fn merge_text(items: Vec<InkItem>) -> Vec<InkItem> {
    let mut rest = Vec::with_capacity(items.len());
    let mut text: Vec<(f64, f64, f64, f64, u8)> = Vec::new();
    for it in items {
        match it {
            InkItem::Text { x, y, w, h, gray } => text.push((x, y, w, h, gray)),
            other => rest.push(other),
        }
    }
    text.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.total_cmp(&b.0)));
    let mut merged: Vec<(f64, f64, f64, f64, u8)> = Vec::new();
    for t in text {
        if let Some(last) = merged.last_mut()
            && (last.1 - t.1).abs() < 0.5
            && t.0 - (last.0 + last.2) < last.3.max(t.3) * 1.5
        {
            let right = (last.0 + last.2).max(t.0 + t.2);
            last.2 = right - last.0;
            last.3 = last.3.max(t.3);
            continue;
        }
        merged.push(t);
    }
    // Fills first, then rules, pictures, text on top.
    rest.extend(
        merged
            .into_iter()
            .map(|(x, y, w, h, gray)| InkItem::Text { x, y, w, h, gray }),
    );
    rest.sort_by_key(|i| match i {
        InkItem::Rect { .. } => 0,
        InkItem::Picture { .. } => 1,
        InkItem::Line { .. } => 2,
        InkItem::Text { .. } => 3,
    });
    rest
}

fn resolve<'a>(doc: &'a Document, o: &'a Object) -> &'a Object {
    match o {
        Object::Reference(r) => doc.get_object(*r).unwrap_or(o),
        _ => o,
    }
}

/// The value of an inheritable page key.
fn inherited<'a>(doc: &'a Document, mut id: ObjectId, key: &[u8]) -> Option<&'a Object> {
    for _ in 0..32 {
        let d = doc.get_dictionary(id).ok()?;
        if let Ok(o) = d.get(key) {
            return Some(resolve(doc, o));
        }
        id = d.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

fn media_box(doc: &Document, id: ObjectId) -> Option<[f64; 4]> {
    let arr = inherited(doc, id, b"MediaBox")?.as_array().ok()?;
    let mut v = arr.iter().filter_map(|o| num(resolve(doc, o)));
    let b = [v.next()?, v.next()?, v.next()?, v.next()?];
    let (x0, x1) = (b[0].min(b[2]), b[0].max(b[2]));
    let (y0, y1) = (b[1].min(b[3]), b[1].max(b[3]));
    (x1 - x0 > 1.0 && y1 - y0 > 1.0).then_some([x0, y0, x1, y1])
}

/// Where one source page lands on a sheet: scaled by `scale`, its lower-left corner at
/// (`dx`, `dy`) from the sheet's lower-left corner (points, y up).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub source: usize,
    pub scale: f64,
    pub dx: f64,
    pub dy: f64,
}

fn normal_n(per_sheet: u8) -> u8 {
    if matches!(per_sheet, 1 | 2 | 4 | 6 | 8 | 16) {
        per_sheet
    } else {
        1
    }
}

impl PdfSource {
    /// The placements of the pages `chunk` (0-based source pages) on one sheet of `sheet` points,
    /// `per_sheet` to a sheet, in reading order. A page larger than its cell is scaled down to
    /// fit; a smaller one keeps its size; either is centred in its cell.
    #[must_use]
    pub fn placements(&self, chunk: &[usize], sheet: (f64, f64), per_sheet: u8) -> Vec<Placement> {
        let (cols, rows) = grid(normal_n(per_sheet));
        let (cw, ch) = (sheet.0 / cols as f64, sheet.1 / rows as f64);
        chunk
            .iter()
            .take(cols * rows)
            .enumerate()
            .filter_map(|(slot, &i)| {
                let (pw, ph) = self.page_size(i)?;
                let scale = (cw / pw).min(ch / ph).min(1.0);
                let (col, row) = (slot % cols, slot / cols);
                Some(Placement {
                    source: i,
                    scale,
                    dx: col as f64 * cw + (cw - pw * scale) / 2.0,
                    dy: sheet.1 - (row as f64 + 1.0) * ch + (ch - ph * scale) / 2.0,
                })
            })
            .collect()
    }
}

/// Columns and rows of `n` pages on one sheet, in reading order.
#[must_use]
pub fn grid(n: u8) -> (usize, usize) {
    match n {
        2 => (2, 1),
        4 => (2, 2),
        6 => (2, 3),
        8 => (2, 4),
        16 => (4, 4),
        _ => (1, 1),
    }
}

fn fmt(v: f64) -> String {
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Writes the job: `sequence` (0-based source pages, in print order) placed `per_sheet` to a
/// sheet of `sheet` points, as a PDF.
///
/// A page that is larger than its cell is scaled down to fit and centred; a smaller page keeps
/// its size and is centred, so a sheet laid out for the paper passes through unchanged.
///
/// # Errors
/// [`PdfError`] when the sequence is empty or names a page the source lacks, or the result
/// cannot be written.
pub fn compose(
    src: &PdfSource,
    sequence: &[usize],
    sheet: (f64, f64),
    per_sheet: u8,
) -> Result<Vec<u8>, PdfError> {
    if sequence.is_empty() {
        return Err(PdfError::NoPages);
    }
    if let Some(bad) = sequence.iter().find(|i| **i >= src.pages.len()) {
        return Err(PdfError::BadPage(*bad));
    }
    let mut doc = src.doc.clone();
    let root = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"Pages").ok())
        .and_then(|o| o.as_reference().ok())
        .ok_or(PdfError::Structure("no page root"))?;

    let mut forms: HashMap<usize, ObjectId> = HashMap::new();
    for &i in sequence {
        if forms.contains_key(&i) {
            continue;
        }
        let pid = src.pages[i];
        let b = src.boxes[i];
        let content = doc
            .get_page_content_with_limit(pid, MAX_CONTENT)
            .map_err(|e| PdfError::Parse(e.to_string()))?;
        let resources = match inherited(&doc, pid, b"Resources") {
            Some(Object::Dictionary(d)) => d.clone(),
            _ => Dictionary::new(),
        };
        let mut dict = Dictionary::new();
        dict.set("Type", Object::Name(b"XObject".to_vec()));
        dict.set("Subtype", Object::Name(b"Form".to_vec()));
        dict.set(
            "BBox",
            Object::Array(b.iter().map(|v| Object::Real(*v as f32)).collect()),
        );
        dict.set("Resources", Object::Dictionary(resources));
        let mut form = Stream::new(dict, content);
        let _ = form.compress();
        forms.insert(i, doc.add_object(Object::Stream(form)));
    }

    let n = normal_n(per_sheet);
    let mut kids = Vec::new();
    for chunk in sequence.chunks(usize::from(n)) {
        let mut ops = String::new();
        let mut xobjects = Dictionary::new();
        for (slot, p) in src.placements(chunk, sheet, n).iter().enumerate() {
            let b = src.boxes[p.source];
            let name = format!("P{slot}");
            ops.push_str(&format!(
                "q {} 0 0 {} {} {} cm /{name} Do Q
",
                fmt(p.scale),
                fmt(p.scale),
                fmt(p.dx - b[0] * p.scale),
                fmt(p.dy - b[1] * p.scale)
            ));
            xobjects.set(
                name.as_bytes().to_vec(),
                Object::Reference(forms[&p.source]),
            );
        }
        let contents = doc.add_object(Object::Stream(Stream::new(
            Dictionary::new(),
            ops.into_bytes(),
        )));
        let mut res = Dictionary::new();
        res.set("XObject", Object::Dictionary(xobjects));
        let mut page = Dictionary::new();
        page.set("Type", Object::Name(b"Page".to_vec()));
        page.set("Parent", Object::Reference(root));
        page.set(
            "MediaBox",
            Object::Array(vec![
                Object::Integer(0),
                Object::Integer(0),
                Object::Real(sheet.0 as f32),
                Object::Real(sheet.1 as f32),
            ]),
        );
        page.set("Resources", Object::Dictionary(res));
        page.set("Contents", Object::Reference(contents));
        kids.push(Object::Reference(doc.add_object(Object::Dictionary(page))));
    }

    let count = kids.len() as i64;
    let pages = doc
        .get_dictionary_mut(root)
        .map_err(|_| PdfError::Structure("page root is not a dictionary"))?;
    pages.set("Kids", Object::Array(kids));
    pages.set("Count", Object::Integer(count));
    for key in ["MediaBox", "Rotate", "Resources"] {
        pages.remove(key.as_bytes());
    }
    if let Ok(cat) = doc.catalog_mut() {
        for key in [
            "Outlines",
            "StructTreeRoot",
            "Names",
            "Dests",
            "OpenAction",
            "AcroForm",
            "PageLabels",
            "MarkInfo",
            "AA",
            "Threads",
        ] {
            cat.remove(key.as_bytes());
        }
    }
    doc.prune_objects();
    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|e| PdfError::Write(e.to_string()))?;
    Ok(out)
}

/// The page sizes of a composed job, for tests and the preview.
#[must_use]
pub fn sheet_sizes(bytes: &[u8]) -> Vec<(f64, f64)> {
    PdfSource::load(bytes)
        .map(|s| (0..s.page_count()).filter_map(|i| s.page_size(i)).collect())
        .unwrap_or_default()
}
