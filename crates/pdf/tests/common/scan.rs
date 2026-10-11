//! A general-enough PDF content reader for the tests: it finds indirect objects by scanning (so
//! classic tables, xref streams and hybrid files all work), unpacks object streams, walks the page
//! tree and interprets the text operators with the full text and graphics matrices. It reads both
//! Word's PDF and ours, so the glyph positions of the two can be compared. It is not a validator.
#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap};
use std::io::Read as _;

use super::{Obj, parse_obj};

pub struct Scan {
    objs: HashMap<u32, Obj>,
}

fn is_digit(b: u8) -> bool {
    b.is_ascii_digit()
}

fn inflate(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    // A damaged tail still yields what was decoded.
    let _ = flate2::read::ZlibDecoder::new(bytes).read_to_end(&mut out);
    out
}

impl Scan {
    pub fn parse(data: &[u8]) -> Self {
        let mut objs: HashMap<u32, Obj> = HashMap::new();
        let mut i = 0;
        while i + 6 < data.len() {
            // "<n> <g> obj" at the start of a line.
            if data[i] == b'o'
                && data[i..].starts_with(b"obj")
                && i >= 4
                && data[i - 1] == b' '
                && !data[i..].starts_with(b"object")
            {
                let mut j = i - 1;
                let gen_end = j;
                while j > 0 && is_digit(data[j - 1]) {
                    j -= 1;
                }
                let gen_ok = j < gen_end && j > 0 && data[j - 1] == b' ';
                if gen_ok {
                    let mut k = j - 1;
                    let num_end = k;
                    while k > 0 && is_digit(data[k - 1]) {
                        k -= 1;
                    }
                    let line_start = k == 0 || matches!(data[k - 1], b'\n' | b'\r');
                    if k < num_end
                        && line_start
                        && let Ok(n) = std::str::from_utf8(&data[k..num_end])
                            .unwrap_or("x")
                            .parse::<u32>()
                    {
                        let mut pos = i + 3;
                        let obj = std::panic::catch_unwind(|| {
                            let mut p = pos;
                            let o = parse_obj(data, &mut p);
                            (o, p)
                        });
                        if let Ok((o, p)) = obj {
                            objs.insert(n, o);
                            pos = p;
                        }
                        i = pos.max(i + 3);
                        continue;
                    }
                }
            }
            i += 1;
        }
        let mut scan = Self { objs };
        scan.unpack_object_streams();
        scan
    }

    fn unpack_object_streams(&mut self) {
        let streams: Vec<u32> = self
            .objs
            .iter()
            .filter_map(|(id, o)| match o {
                Obj::Stream(d, _) if matches!(d.get("Type"), Some(Obj::Name(n)) if n == "ObjStm") => {
                    Some(*id)
                }
                _ => None,
            })
            .collect();
        for id in streams {
            let o = self.objs[&id].clone();
            let data = self.stream(&o);
            let d = o.dict();
            let n = self.resolve(&d["N"]).num() as usize;
            let first = self.resolve(&d["First"]).num() as usize;
            let mut pos = 0;
            let mut pairs = Vec::new();
            for _ in 0..n {
                let a = parse_obj(&data, &mut pos).num() as u32;
                let b = parse_obj(&data, &mut pos).num() as usize;
                pairs.push((a, b));
            }
            for (num, off) in pairs {
                let mut p = first + off;
                if p < data.len() {
                    let obj = parse_obj(&data, &mut p);
                    self.objs.entry(num).or_insert(obj);
                }
            }
        }
    }

    pub fn get(&self, id: u32) -> Obj {
        self.objs.get(&id).cloned().unwrap_or(Obj::Null)
    }

    pub fn resolve(&self, o: &Obj) -> Obj {
        let mut cur = o.clone();
        for _ in 0..16 {
            match cur {
                Obj::Ref(id) => cur = self.get(id),
                other => return other,
            }
        }
        Obj::Null
    }

    pub fn key(&self, o: &Obj, key: &str) -> Option<Obj> {
        match o {
            Obj::Dict(d) | Obj::Stream(d, _) => d.get(key).map(|v| self.resolve(v)),
            _ => None,
        }
    }

    pub fn stream(&self, o: &Obj) -> Vec<u8> {
        let Obj::Stream(dict, bytes) = o else {
            return Vec::new();
        };
        let filters: Vec<String> = match dict.get("Filter").map(|f| self.resolve(f)) {
            Some(Obj::Name(n)) => vec![n],
            Some(Obj::Arr(a)) => a
                .iter()
                .filter_map(|x| match self.resolve(x) {
                    Obj::Name(n) => Some(n),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let mut data = bytes.clone();
        for f in filters {
            if f == "FlateDecode" || f == "Fl" {
                data = inflate(&data);
            }
        }
        data
    }

    pub fn catalog(&self) -> Option<Obj> {
        self.objs
            .values()
            .find(|o| matches!(self.key(o, "Type"), Some(Obj::Name(n)) if n == "Catalog"))
            .cloned()
    }

    /// Page objects in order, each with the inheritable entries (Resources, MediaBox) resolved.
    pub fn pages(&self) -> Vec<Obj> {
        let Some(cat) = self.catalog() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if let Some(root) = self.key(&cat, "Pages") {
            self.walk(&root, &BTreeMap::new(), &mut out);
        }
        out
    }

    fn walk(&self, node: &Obj, inherited: &BTreeMap<String, Obj>, out: &mut Vec<Obj>) {
        let mut inh = inherited.clone();
        for k in ["Resources", "MediaBox"] {
            if let Some(v) = self.key(node, k) {
                inh.insert(k.to_owned(), v);
            }
        }
        match self.key(node, "Kids") {
            Some(Obj::Arr(kids)) => {
                for kid in kids {
                    self.walk(&self.resolve(&kid), &inh, out);
                }
            }
            _ => {
                let mut d = node.dict().clone();
                for (k, v) in inh {
                    d.entry(k).or_insert(v);
                }
                out.push(Obj::Dict(d));
            }
        }
    }

    /// Page size in points from the MediaBox.
    pub fn page_size(&self, page: &Obj) -> (f64, f64) {
        match self.key(page, "MediaBox") {
            Some(Obj::Arr(a)) if a.len() == 4 => {
                let n: Vec<f64> = a.iter().map(|x| self.resolve(x).num()).collect();
                (n[2] - n[0], n[3] - n[1])
            }
            _ => (0.0, 0.0),
        }
    }
}

/// One glyph as drawn: the text it stands for and the device-space origin (user space, y up).
#[derive(Debug, Clone)]
pub struct Glyph {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub size: f64,
}

pub struct PageGlyphs {
    pub width: f64,
    pub height: f64,
    pub glyphs: Vec<Glyph>,
}

struct FontData {
    two_byte: bool,
    widths: HashMap<u32, f64>,
    default_width: f64,
    first_char: u32,
    simple_widths: Vec<f64>,
    missing: f64,
    unicode: HashMap<u32, String>,
}

impl FontData {
    fn width(&self, code: u32) -> f64 {
        if self.two_byte {
            self.widths
                .get(&code)
                .copied()
                .unwrap_or(self.default_width)
        } else {
            code.checked_sub(self.first_char)
                .and_then(|i| self.simple_widths.get(i as usize))
                .copied()
                .unwrap_or(self.missing)
        }
    }
}

fn hex_units(h: &str) -> String {
    let units: Vec<u16> = (0..h.len() / 4)
        .filter_map(|k| u16::from_str_radix(&h[4 * k..4 * k + 4], 16).ok())
        .collect();
    String::from_utf16_lossy(&units)
}

fn parse_to_unicode(cmap: &str) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    let mut rest = cmap;
    while let Some(i) = rest.find("beginbf") {
        let kind_range = rest[i..].starts_with("beginbfrange");
        let body_start = i + if kind_range { 12 } else { 11 };
        let end_tag = if kind_range {
            "endbfrange"
        } else {
            "endbfchar"
        };
        let Some(e) = rest[body_start..].find(end_tag) else {
            break;
        };
        let body = &rest[body_start..body_start + e];
        // Tokens: <hex> or [ or ].
        let mut toks: Vec<String> = Vec::new();
        let b = body.as_bytes();
        let mut p = 0;
        while p < b.len() {
            match b[p] {
                b'<' => {
                    let q = body[p..].find('>').map_or(b.len(), |x| p + x);
                    toks.push(body[p + 1..q].trim().to_owned());
                    p = q + 1;
                }
                b'[' => {
                    toks.push("[".into());
                    p += 1;
                }
                b']' => {
                    toks.push("]".into());
                    p += 1;
                }
                _ => p += 1,
            }
        }
        let mut t = 0;
        while t < toks.len() {
            if kind_range {
                let (Some(lo), Some(hi)) = (toks.get(t), toks.get(t + 1)) else {
                    break;
                };
                let lo = u32::from_str_radix(lo, 16).unwrap_or(0);
                let hi = u32::from_str_radix(hi, 16).unwrap_or(0);
                if toks.get(t + 2).map(String::as_str) == Some("[") {
                    t += 3;
                    let mut code = lo;
                    while t < toks.len() && toks[t] != "]" {
                        out.insert(code, hex_units(&toks[t]));
                        code += 1;
                        t += 1;
                    }
                    t += 1;
                } else if let Some(dst) = toks.get(t + 2) {
                    let base = hex_units(dst);
                    for code in lo..=hi.min(lo + 0xFFFF) {
                        let mut chars: Vec<char> = base.chars().collect();
                        if let Some(last) = chars.last_mut()
                            && let Some(c) = char::from_u32(*last as u32 + (code - lo))
                        {
                            *last = c;
                        }
                        out.insert(code, chars.into_iter().collect());
                    }
                    t += 3;
                } else {
                    break;
                }
            } else {
                let (Some(src), Some(dst)) = (toks.get(t), toks.get(t + 1)) else {
                    break;
                };
                out.insert(u32::from_str_radix(src, 16).unwrap_or(0), hex_units(dst));
                t += 2;
            }
        }
        rest = &rest[body_start + e..];
    }
    out
}

fn load_font(s: &Scan, font: &Obj) -> FontData {
    let subtype = match s.key(font, "Subtype") {
        Some(Obj::Name(n)) => n,
        _ => String::new(),
    };
    let unicode = s
        .key(font, "ToUnicode")
        .map(|t| parse_to_unicode(&String::from_utf8_lossy(&s.stream(&t))))
        .unwrap_or_default();
    if subtype == "Type0" {
        let desc = match s.key(font, "DescendantFonts") {
            Some(Obj::Arr(a)) => a.first().map(|d| s.resolve(d)).unwrap_or(Obj::Null),
            _ => Obj::Null,
        };
        let default_width = s.key(&desc, "DW").map_or(1000.0, |o| o.num());
        let mut widths = HashMap::new();
        if let Some(Obj::Arr(w)) = s.key(&desc, "W") {
            let w: Vec<Obj> = w.iter().map(|x| s.resolve(x)).collect();
            let mut i = 0;
            while i + 1 < w.len() {
                let first = w[i].num() as u32;
                match &w[i + 1] {
                    Obj::Arr(list) => {
                        for (k, width) in list.iter().enumerate() {
                            widths.insert(first + k as u32, s.resolve(width).num());
                        }
                        i += 2;
                    }
                    last => {
                        let (last, width) = (last.num() as u32, w.get(i + 2).map_or(0.0, Obj::num));
                        for c in first..=last.min(first + 0xFFFF) {
                            widths.insert(c, width);
                        }
                        i += 3;
                    }
                }
            }
        }
        FontData {
            two_byte: true,
            widths,
            default_width,
            first_char: 0,
            simple_widths: Vec::new(),
            missing: 0.0,
            unicode,
        }
    } else {
        let first_char = s.key(font, "FirstChar").map_or(0.0, |o| o.num()) as u32;
        let simple_widths = match s.key(font, "Widths") {
            Some(Obj::Arr(a)) => a.iter().map(|x| s.resolve(x).num()).collect(),
            _ => Vec::new(),
        };
        let missing = s
            .key(font, "FontDescriptor")
            .and_then(|d| s.key(&d, "MissingWidth"))
            .map_or(0.0, |o| o.num());
        FontData {
            two_byte: false,
            widths: HashMap::new(),
            default_width: 0.0,
            first_char,
            simple_widths,
            missing,
            unicode,
        }
    }
}

type Matrix = [f64; 6];

const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `a` applied first, then `b`.
fn mul(a: &Matrix, b: &Matrix) -> Matrix {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

fn translate(x: f64, y: f64) -> Matrix {
    [1.0, 0.0, 0.0, 1.0, x, y]
}

struct TextState {
    font: Option<String>,
    size: f64,
    tc: f64,
    tw: f64,
    th: f64,
    tl: f64,
    rise: f64,
}

fn num_of(o: &Obj) -> f64 {
    match o {
        Obj::Int(i) => *i as f64,
        Obj::Real(r) => *r,
        _ => 0.0,
    }
}

/// Every glyph the page's content streams draw, in drawing order.
pub fn page_glyphs(s: &Scan, page: &Obj) -> PageGlyphs {
    let (width, height) = s.page_size(page);
    let mut fonts: HashMap<String, FontData> = HashMap::new();
    if let Some(res) = s.key(page, "Resources")
        && let Some(fd) = s.key(&res, "Font")
    {
        for (name, v) in fd.dict() {
            fonts.insert(name.clone(), load_font(s, &s.resolve(v)));
        }
    }
    let content: Vec<u8> = match s.key(page, "Contents") {
        Some(Obj::Arr(parts)) => parts
            .iter()
            .flat_map(|p| {
                let mut v = s.stream(&s.resolve(p));
                v.push(b'\n');
                v
            })
            .collect(),
        Some(o) => s.stream(&o),
        None => Vec::new(),
    };
    let mut glyphs = Vec::new();
    let mut ctm = IDENTITY;
    let mut ctm_stack: Vec<Matrix> = Vec::new();
    let (mut tm, mut tlm) = (IDENTITY, IDENTITY);
    let mut st = TextState {
        font: None,
        size: 0.0,
        tc: 0.0,
        tw: 0.0,
        th: 1.0,
        tl: 0.0,
        rise: 0.0,
    };
    let mut stack: Vec<Obj> = Vec::new();
    let mut pos = 0;
    let skip_ws = |pos: &mut usize| {
        while *pos < content.len() && matches!(content[*pos], b' ' | b'\n' | b'\r' | b'\t' | 0 | 12)
        {
            *pos += 1;
        }
    };
    loop {
        skip_ws(&mut pos);
        if pos >= content.len() {
            break;
        }
        let c = content[pos];
        if c == b'%' {
            while pos < content.len() && content[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }
        let operand =
            matches!(c, b'/' | b'[' | b'<' | b'(' | b'-' | b'+' | b'.') || c.is_ascii_digit();
        if operand {
            let mut p = pos;
            let parsed = std::panic::catch_unwind(|| {
                let mut q = p;
                let o = parse_obj(&content, &mut q);
                (o, q)
            });
            match parsed {
                Ok((o, q)) => {
                    stack.push(o);
                    pos = q.max(p + 1);
                }
                Err(_) => {
                    p += 1;
                    pos = p;
                }
            }
            continue;
        }
        let start = pos;
        while pos < content.len()
            && !matches!(
                content[pos],
                b' ' | b'\n' | b'\r' | b'\t' | 0 | 12 | b'/' | b'[' | b'<' | b'(' | b'%'
            )
        {
            pos += 1;
        }
        let op = String::from_utf8_lossy(&content[start..pos]).into_owned();
        let n = |k: usize| stack.get(stack.len().wrapping_sub(k)).map_or(0.0, num_of);
        let mut show: Vec<Obj> = Vec::new();
        match op.as_str() {
            "q" => ctm_stack.push(ctm),
            "Q" => ctm = ctm_stack.pop().unwrap_or(IDENTITY),
            "cm" => ctm = mul(&[n(6), n(5), n(4), n(3), n(2), n(1)], &ctm),
            "BT" => {
                tm = IDENTITY;
                tlm = IDENTITY;
            }
            "Tf" => {
                if let Some(Obj::Name(name)) = stack.get(stack.len().wrapping_sub(2)) {
                    st.font = Some(name.clone());
                }
                st.size = n(1);
            }
            "Tc" => st.tc = n(1),
            "Tw" => st.tw = n(1),
            "Tz" => st.th = n(1) / 100.0,
            "TL" => st.tl = n(1),
            "Ts" => st.rise = n(1),
            "Tm" => {
                tm = [n(6), n(5), n(4), n(3), n(2), n(1)];
                tlm = tm;
            }
            "Td" => {
                tlm = mul(&translate(n(2), n(1)), &tlm);
                tm = tlm;
            }
            "TD" => {
                st.tl = -n(1);
                tlm = mul(&translate(n(2), n(1)), &tlm);
                tm = tlm;
            }
            "T*" => {
                tlm = mul(&translate(0.0, -st.tl), &tlm);
                tm = tlm;
            }
            "Tj" => show = stack.last().cloned().into_iter().collect(),
            "'" | "\"" => {
                tlm = mul(&translate(0.0, -st.tl), &tlm);
                tm = tlm;
                show = stack.last().cloned().into_iter().collect();
            }
            "TJ" => {
                if let Some(Obj::Arr(a)) = stack.last() {
                    show = a.clone();
                }
            }
            _ => {}
        }
        if !show.is_empty()
            && let Some(font) = st.font.as_ref().and_then(|f| fonts.get(f))
        {
            for item in &show {
                match item {
                    Obj::Str(bytes) => {
                        let step = if font.two_byte { 2 } else { 1 };
                        for ch in bytes.chunks(step) {
                            let code = ch.iter().fold(0u32, |a, b| a << 8 | u32::from(*b));
                            let origin = mul(&translate(0.0, st.rise), &mul(&tm, &ctm));
                            let text = font
                                .unicode
                                .get(&code)
                                .cloned()
                                .unwrap_or_else(|| "\u{fffd}".into());
                            glyphs.push(Glyph {
                                text,
                                x: origin[4],
                                y: origin[5],
                                size: st.size * tm[2].hypot(tm[3]) * ctm[2].hypot(ctm[3]),
                            });
                            let w = font.width(code) / 1000.0 * st.size;
                            let space = if !font.two_byte && code == 32 {
                                st.tw
                            } else {
                                0.0
                            };
                            tm = mul(&translate((w + st.tc + space) * st.th, 0.0), &tm);
                        }
                    }
                    other => {
                        tm = mul(
                            &translate(-num_of(other) / 1000.0 * st.size * st.th, 0.0),
                            &tm,
                        );
                    }
                }
            }
        }
        stack.clear();
    }
    PageGlyphs {
        width,
        height,
        glyphs,
    }
}

/// All pages of a PDF with their glyphs.
pub fn glyphs_of(data: &[u8]) -> Vec<PageGlyphs> {
    let s = Scan::parse(data);
    s.pages().iter().map(|p| page_glyphs(&s, p)).collect()
}
