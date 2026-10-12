//! A small PDF reader for the tests: objects through the xref table, Flate streams, the text
//! content operators and the ToUnicode CMaps. It reads what our writer produces; it is not a
//! general PDF parser.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use open365_docx::Document;
use open365_layout::{Layout, LayoutOptions, layout_document};
use open365_opc::Limits;
use open365_text::{FontLibrary, FontMatch};

pub mod scan;
pub mod structure;

pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn load_doc(name: &str) -> Document {
    let bytes = std::fs::read(root().join(format!("fixtures/docx/{name}.docx"))).expect("docx");
    Document::open(&bytes, &Limits::default()).expect("open docx")
}

/// The fixture laid out with the bundled fonts; `None` when the document asks for a family the
/// bundle cannot stand in for.
pub fn layout_fixture(name: &str, fonts: &FontLibrary) -> Option<(Document, Layout)> {
    let doc = load_doc(name);
    let layout = layout_document(&doc, fonts, LayoutOptions::default());
    if layout
        .fonts
        .iter()
        .any(|f| f.matched == FontMatch::Fallback)
    {
        return None;
    }
    Some((doc, layout))
}

/// A fixture whose theme font "Calibri Light" is changed to "Calibri", so that the bundled fonts
/// can lay it out. The layout is not Word's, which is fine for tests of the PDF structure.
pub fn layout_with_bundled_theme(name: &str, fonts: &FontLibrary) -> (Document, Layout) {
    use std::io::{Cursor, Write as _};
    let bytes = std::fs::read(root().join(format!("fixtures/docx/{name}.docx"))).expect("docx");
    let mut zin = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..zin.len() {
        let mut f = zin.by_index(i).expect("entry");
        let entry = f.name().expect("name").into_owned();
        let mut data = Vec::new();
        f.read_to_end(&mut data).expect("read");
        if entry.starts_with("word/theme/") {
            data = String::from_utf8_lossy(&data)
                .replace("typeface=\"Calibri Light\"", "typeface=\"Calibri\"")
                .into_bytes();
        }
        out.start_file(&entry, zip::write::SimpleFileOptions::default())
            .expect("start");
        out.write_all(&data).expect("write");
    }
    let patched = out.finish().expect("finish").into_inner();
    let doc = Document::open(&patched, &Limits::default()).expect("open docx");
    let layout = layout_document(&doc, fonts, LayoutOptions::default());
    assert!(
        !layout
            .fonts
            .iter()
            .any(|f| f.matched == FontMatch::Fallback),
        "{name}: still needs a font the bundle lacks"
    );
    (doc, layout)
}

pub fn fixture_names() -> Vec<String> {
    let dir = root().join("fixtures/reference/layout");
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("reference dir")
        .filter_map(|e| {
            let name = e.ok()?.file_name().into_string().ok()?;
            let stem = name.strip_suffix(".json")?;
            (!stem.ends_with(".lists")).then(|| stem.to_owned())
        })
        .collect();
    names.sort();
    names
}

#[derive(Debug, Clone, PartialEq)]
pub enum Obj {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    Name(String),
    Str(Vec<u8>),
    Arr(Vec<Obj>),
    Dict(BTreeMap<String, Obj>),
    Ref(u32),
    Stream(BTreeMap<String, Obj>, Vec<u8>),
}

impl Obj {
    pub fn num(&self) -> f64 {
        match self {
            Obj::Int(i) => *i as f64,
            Obj::Real(r) => *r,
            other => panic!("not a number: {other:?}"),
        }
    }

    pub fn dict(&self) -> &BTreeMap<String, Obj> {
        match self {
            Obj::Dict(d) | Obj::Stream(d, _) => d,
            other => panic!("not a dict: {other:?}"),
        }
    }

    pub fn arr(&self) -> &[Obj] {
        match self {
            Obj::Arr(a) => a,
            other => panic!("not an array: {other:?}"),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Obj::Name(n) => n,
            other => panic!("not a name: {other:?}"),
        }
    }
}

pub struct Reader<'a> {
    pub data: &'a [u8],
    pub offsets: Vec<usize>,
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\n' | b'\r' | b'\t' | 0 | 12)
}

fn is_delim(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn skip_ws(d: &[u8], pos: &mut usize) {
    while *pos < d.len() && (is_ws(d[*pos]) || d[*pos] == b'%') {
        if d[*pos] == b'%' {
            while *pos < d.len() && d[*pos] != b'\n' {
                *pos += 1;
            }
        } else {
            *pos += 1;
        }
    }
}

fn token(d: &[u8], pos: &mut usize) -> String {
    let start = *pos;
    while *pos < d.len() && !is_ws(d[*pos]) && !is_delim(d[*pos]) {
        *pos += 1;
    }
    String::from_utf8_lossy(&d[start..*pos]).into_owned()
}

/// Parses one object. `lookup` resolves a stream `/Length` that is a reference.
pub fn parse_obj(d: &[u8], pos: &mut usize) -> Obj {
    skip_ws(d, pos);
    match d[*pos] {
        b'/' => {
            *pos += 1;
            Obj::Name(token(d, pos))
        }
        b'[' => {
            *pos += 1;
            let mut items = Vec::new();
            loop {
                skip_ws(d, pos);
                if d[*pos] == b']' {
                    *pos += 1;
                    return Obj::Arr(items);
                }
                items.push(parse_obj(d, pos));
            }
        }
        b'<' if d[*pos + 1] == b'<' => {
            *pos += 2;
            let mut map = BTreeMap::new();
            loop {
                skip_ws(d, pos);
                if d[*pos] == b'>' {
                    *pos += 2;
                    break;
                }
                let Obj::Name(key) = parse_obj(d, pos) else {
                    panic!("dict key is not a name at {pos}");
                };
                map.insert(key, parse_obj(d, pos));
            }
            let save = *pos;
            skip_ws(d, pos);
            if d[*pos..].starts_with(b"stream") {
                *pos += 6;
                if d[*pos] == b'\r' {
                    *pos += 1;
                }
                *pos += 1;
                // An indirect /Length (other producers write them) is found by scanning.
                let len = match map.get("Length") {
                    Some(Obj::Int(len)) => *len as usize,
                    _ => d[*pos..]
                        .windows(9)
                        .position(|w| w == b"endstream")
                        .map_or(0, |p| {
                            let mut end = p;
                            while end > 0 && matches!(d[*pos + end - 1], b'\r' | b'\n') {
                                end -= 1;
                            }
                            end
                        }),
                };
                let bytes = d[*pos..*pos + len].to_vec();
                *pos += len;
                skip_ws(d, pos);
                assert!(d[*pos..].starts_with(b"endstream"), "endstream missing");
                *pos += 9;
                return Obj::Stream(map, bytes);
            }
            *pos = save;
            Obj::Dict(map)
        }
        b'<' => {
            *pos += 1;
            let mut hex = String::new();
            while d[*pos] != b'>' {
                if !is_ws(d[*pos]) {
                    hex.push(d[*pos] as char);
                }
                *pos += 1;
            }
            *pos += 1;
            if hex.len() % 2 == 1 {
                hex.push('0');
            }
            Obj::Str(
                (0..hex.len() / 2)
                    .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).expect("hex"))
                    .collect(),
            )
        }
        b'(' => {
            *pos += 1;
            let mut out = Vec::new();
            let mut depth = 1;
            while depth > 0 {
                let b = d[*pos];
                *pos += 1;
                match b {
                    b'\\' => {
                        out.push(d[*pos]);
                        *pos += 1;
                    }
                    b'(' => {
                        depth += 1;
                        out.push(b);
                    }
                    b')' => {
                        depth -= 1;
                        if depth > 0 {
                            out.push(b);
                        }
                    }
                    _ => out.push(b),
                }
            }
            Obj::Str(out)
        }
        _ => {
            let t = token(d, pos);
            match t.as_str() {
                "true" => return Obj::Bool(true),
                "false" => return Obj::Bool(false),
                "null" => return Obj::Null,
                _ => {}
            }
            if let Ok(i) = t.parse::<i64>() {
                // "n g R"?
                let save = *pos;
                skip_ws(d, pos);
                let g = token(d, pos);
                skip_ws(d, pos);
                if g.parse::<u32>().is_ok() && d.get(*pos) == Some(&b'R') {
                    *pos += 1;
                    return Obj::Ref(i as u32);
                }
                *pos = save;
                return Obj::Int(i);
            }
            Obj::Real(
                t.parse::<f64>()
                    .unwrap_or_else(|_| panic!("bad token {t:?}")),
            )
        }
    }
}

impl<'a> Reader<'a> {
    /// Reads the xref table and checks every offset points at its object header.
    pub fn new(data: &'a [u8]) -> Self {
        assert!(data.starts_with(b"%PDF-1."));
        assert!(data.ends_with(b"%%EOF\n"));
        let tail = String::from_utf8_lossy(&data[data.len().saturating_sub(64)..]).into_owned();
        let sx = tail.rfind("startxref").expect("startxref");
        let xref: usize = tail[sx + 9..]
            .split_whitespace()
            .next()
            .expect("xref offset")
            .parse()
            .expect("offset");
        let mut pos = xref;
        assert_eq!(token(data, &mut pos), "xref");
        skip_ws(data, &mut pos);
        assert_eq!(token(data, &mut pos), "0");
        skip_ws(data, &mut pos);
        let count: usize = token(data, &mut pos).parse().expect("count");
        let mut offsets = vec![0usize; count];
        for (i, slot) in offsets.iter_mut().enumerate() {
            skip_ws(data, &mut pos);
            let off: usize = token(data, &mut pos).parse().expect("off");
            skip_ws(data, &mut pos);
            token(data, &mut pos);
            skip_ws(data, &mut pos);
            let kind = token(data, &mut pos);
            if i > 0 {
                assert_eq!(kind, "n");
                let header = format!("{i} 0 obj");
                assert!(
                    data[off..].starts_with(header.as_bytes()),
                    "xref offset of object {i} is wrong"
                );
                *slot = off;
            }
        }
        Self { data, offsets }
    }

    pub fn trailer(&self) -> Obj {
        let text = self.data;
        let at = text
            .windows(7)
            .rposition(|w| w == b"trailer")
            .expect("trailer");
        let mut pos = at + 7;
        parse_obj(text, &mut pos)
    }

    pub fn object_count(&self) -> usize {
        self.offsets.len() - 1
    }

    pub fn get(&self, id: u32) -> Obj {
        let mut pos = self.offsets[id as usize];
        token(self.data, &mut pos);
        skip_ws(self.data, &mut pos);
        token(self.data, &mut pos);
        skip_ws(self.data, &mut pos);
        token(self.data, &mut pos);
        parse_obj(self.data, &mut pos)
    }

    pub fn resolve(&self, o: &Obj) -> Obj {
        match o {
            Obj::Ref(id) => self.get(*id),
            other => other.clone(),
        }
    }

    /// Dictionary entry with references followed.
    pub fn key(&self, o: &Obj, key: &str) -> Option<Obj> {
        o.dict().get(key).map(|v| self.resolve(v))
    }

    pub fn stream(&self, o: &Obj) -> Vec<u8> {
        let Obj::Stream(dict, bytes) = o else {
            panic!("not a stream");
        };
        match dict.get("Filter") {
            Some(Obj::Name(f)) if f == "FlateDecode" => {
                let mut out = Vec::new();
                flate2::read::ZlibDecoder::new(&bytes[..])
                    .read_to_end(&mut out)
                    .expect("inflate");
                out
            }
            None => bytes.clone(),
            other => panic!("filter {other:?}"),
        }
    }

    pub fn pages(&self) -> Vec<(u32, Obj)> {
        let root = self.key(&self.trailer(), "Root").expect("root");
        let pages = self.key(&root, "Pages").expect("pages");
        pages.dict()["Kids"]
            .arr()
            .iter()
            .map(|k| {
                let Obj::Ref(id) = k else { panic!("kid ref") };
                (*id, self.get(*id))
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct PdfGlyph {
    pub font: String,
    pub gid: u32,
    pub x: f64,
    pub y: f64,
}

/// A marked-content sequence that drew text.
#[derive(Debug, Clone)]
pub struct PdfLine {
    pub artifact: bool,
    pub glyphs: Vec<PdfGlyph>,
}

pub struct FontInfo {
    pub widths: BTreeMap<u32, f64>,
    pub to_unicode: BTreeMap<u32, String>,
    /// A simple TrueType font (one byte per code, no ToUnicode): code to glyph id.
    pub simple: Option<BTreeMap<u32, u32>>,
}

/// A simple TrueType font as Word writes blanks, leaders and note marks: widths by code and the
/// glyph each code stands for, read from the embedded font's cmap.
fn simple_font_info(r: &Reader<'_>, font: &Obj) -> FontInfo {
    use skrifa::MetadataProvider as _;
    let first = r.key(font, "FirstChar").expect("FirstChar").num() as u32;
    let widths_obj = r.key(font, "Widths").expect("Widths");
    let widths = widths_obj
        .arr()
        .iter()
        .enumerate()
        .map(|(k, w)| (first + k as u32, r.resolve(w).num()))
        .collect();
    let fd = r.key(font, "FontDescriptor").expect("descriptor");
    let file = r.key(&fd, "FontFile2").expect("FontFile2");
    let data = r.stream(&file);
    let parsed = skrifa::FontRef::new(&data).expect("simple font parses");
    let simple = widths_keys(&widths)
        .into_iter()
        .filter_map(|code| {
            let gid = parsed.charmap().map(char::from_u32(code)?)?;
            Some((code, gid.to_u32()))
        })
        .collect();
    FontInfo {
        widths,
        to_unicode: BTreeMap::new(),
        simple: Some(simple),
    }
}

fn widths_keys(w: &BTreeMap<u32, f64>) -> Vec<u32> {
    w.keys().copied().collect()
}

pub fn font_info(r: &Reader<'_>, font: &Obj) -> FontInfo {
    if matches!(r.key(font, "Subtype"), Some(Obj::Name(n)) if n == "TrueType") {
        return simple_font_info(r, font);
    }
    let desc = r.resolve(&font.dict()["DescendantFonts"].arr()[0]);
    let w = r.key(&desc, "W").expect("W");
    let w = w.arr();
    let mut widths = BTreeMap::new();
    let mut i = 0;
    while i < w.len() {
        let gid = w[i].num() as u32;
        for (k, width) in w[i + 1].arr().iter().enumerate() {
            widths.insert(gid + k as u32, width.num());
        }
        i += 2;
    }
    let cmap = r.stream(&r.key(font, "ToUnicode").expect("ToUnicode"));
    let text = String::from_utf8(cmap).expect("cmap utf8");
    let mut to_unicode = BTreeMap::new();
    let mut in_chars = false;
    for line in text.lines() {
        if line.ends_with("beginbfchar") {
            in_chars = true;
        } else if line == "endbfchar" {
            in_chars = false;
        } else if in_chars {
            let mut parts = line.split_whitespace();
            let src = parts.next().expect("src").trim_matches(['<', '>']);
            let dst = parts.next().expect("dst").trim_matches(['<', '>']);
            let gid = u32::from_str_radix(src, 16).expect("gid");
            let units: Vec<u16> = (0..dst.len() / 4)
                .map(|k| u16::from_str_radix(&dst[4 * k..4 * k + 4], 16).expect("unit"))
                .collect();
            to_unicode.insert(gid, String::from_utf16(&units).expect("utf16"));
        }
    }
    FontInfo {
        widths,
        to_unicode,
        simple: None,
    }
}

/// Interprets a page's content stream: text sequences with the position of every glyph.
pub fn read_text(content: &[u8], fonts: &BTreeMap<String, FontInfo>) -> Vec<PdfLine> {
    let mut lines = Vec::new();
    let mut current: Option<PdfLine> = None;
    let mut pos = 0;
    let mut stack: Vec<Obj> = Vec::new();
    let mut font: Option<String> = None;
    let mut size = 0.0;
    let (mut tx, mut ty) = (0.0, 0.0);
    while {
        skip_ws(content, &mut pos);
        pos < content.len()
    } {
        let c = content[pos];
        let is_operand = c == b'/'
            || c == b'['
            || c == b'<'
            || c == b'('
            || c.is_ascii_digit()
            || c == b'-'
            || c == b'+'
            || c == b'.';
        if is_operand {
            stack.push(parse_obj(content, &mut pos));
            continue;
        }
        let op = token(content, &mut pos);
        match op.as_str() {
            "BDC" | "BMC" => {
                current = Some(PdfLine {
                    artifact: op == "BMC",
                    glyphs: Vec::new(),
                });
            }
            "EMC" => {
                if let Some(l) = current.take()
                    && !l.glyphs.is_empty()
                {
                    lines.push(l);
                }
            }
            "Tf" => {
                let n = stack[stack.len() - 2].name().to_owned();
                size = stack[stack.len() - 1].num();
                font = Some(n);
            }
            "Tm" => {
                let s = &stack[stack.len() - 6..];
                tx = s[4].num();
                ty = s[5].num();
            }
            "TJ" => {
                let name = font.clone().expect("font set");
                let info = &fonts[&name];
                let line = current.as_mut().expect("text outside marked content");
                let Some(Obj::Arr(items)) = stack.last() else {
                    panic!("TJ operand");
                };
                for it in items {
                    match it {
                        Obj::Str(bytes) => {
                            if let Some(map) = &info.simple {
                                for &code in bytes {
                                    let code = u32::from(code);
                                    // The blank at a paragraph mark is not a glyph of the layout.
                                    if code != 32 {
                                        line.glyphs.push(PdfGlyph {
                                            font: name.clone(),
                                            gid: map[&code],
                                            x: tx,
                                            y: ty,
                                        });
                                    }
                                    tx += info.widths[&code] * size / 1000.0;
                                }
                                continue;
                            }
                            for ch in bytes.chunks(2) {
                                let gid = u32::from(ch[0]) << 8 | u32::from(ch[1]);
                                line.glyphs.push(PdfGlyph {
                                    font: name.clone(),
                                    gid,
                                    x: tx,
                                    y: ty,
                                });
                                tx += info.widths[&gid] * size / 1000.0;
                            }
                        }
                        n => tx -= n.num() * size / 1000.0,
                    }
                }
            }
            _ => {}
        }
        stack.clear();
    }
    lines
}

/// All text sequences of the file, per page, plus the font table.
pub fn extract(r: &Reader<'_>) -> (Vec<Vec<PdfLine>>, BTreeMap<String, FontInfo>) {
    let pages = r.pages();
    let first = &pages[0].1;
    let res = r.key(first, "Resources").expect("resources");
    let font_dict = r.key(&res, "Font").expect("fonts");
    let mut by_name = BTreeMap::new();
    for (name, v) in font_dict.dict() {
        by_name.insert(name.clone(), font_info(r, &r.resolve(v)));
    }
    let mut out = Vec::new();
    for (_, page) in &pages {
        let content = r.stream(&r.key(page, "Contents").expect("contents"));
        out.push(read_text(&content, &by_name));
    }
    (out, by_name)
}

/// The resource name the exporter gives the face with this library index: `F<n>`, `n` counting
/// the faces the layout uses in index order.
pub fn resource_names(layout: &Layout) -> BTreeMap<usize, String> {
    let mut used = std::collections::BTreeSet::new();
    for (_, line) in layout.all_lines() {
        for run in &line.runs {
            for g in &run.glyphs {
                used.insert(g.face.index());
            }
        }
    }
    used.into_iter()
        .enumerate()
        .map(|(n, face)| (face, format!("F{n}")))
        .collect()
}

/// The line lies in a table cell whose clip rectangle (the box less its top and bottom margins)
/// is empty: Word draws none of its text.
#[allow(dead_code)]
pub fn clipped_away(page: &open365_layout::Page, line: &open365_layout::Line) -> bool {
    line.story == open365_layout::StoryKind::Main
        && page
            .cells
            .iter()
            .filter(|c| c.height - c.pad_top - c.pad_bottom <= 0.005)
            .any(|c| {
                line.left >= c.x - 0.01
                    && line.left < c.x + c.width
                    && line.top < c.y + c.height
                    && line.top + line.height > c.y
            })
}
