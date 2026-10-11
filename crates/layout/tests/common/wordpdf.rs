//! Glyph positions out of Word's own "Save as PDF" (`fixtures/reference/pdf/<name>.pdf`).
//!
//! A deliberately small reader for the files Word writes: plain indirect objects, Flate content
//! streams, Type0/Identity-H fonts with a `/W` array and a `ToUnicode` map (plus simple TrueType
//! fonts with `/Widths`), and text positioned by `Tm` and `TJ`. Every glyph's origin is the
//! text-line origin plus the widths and `TJ` adjustments before it, which is how Word places
//! justified, tabbed and kerned text on its 600 dpi grid. Not a validator and not general.
#![allow(dead_code)]

use std::collections::HashMap;
use std::io::Read as _;

/// One drawn glyph. `x` is the glyph origin, `y` the baseline, both from the page top-left.
#[derive(Debug, Clone)]
pub struct WordGlyph {
    pub ch: char,
    pub x: f64,
    pub y: f64,
    pub size: f64,
}

pub struct WordPage {
    pub width: f64,
    pub height: f64,
    pub glyphs: Vec<WordGlyph>,
}

struct Obj {
    dict: String,
    stream: Option<Vec<u8>>,
}

struct Font {
    /// Bytes per character code: 2 for Identity-H, 1 for simple fonts.
    width_bytes: usize,
    widths: HashMap<u32, f64>,
    default_width: f64,
    unicode: HashMap<u32, String>,
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from >= hay.len() {
        return None;
    }
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

fn inflate(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let _ = flate2::read::ZlibDecoder::new(bytes).read_to_end(&mut out);
    out
}

fn parse_objects(data: &[u8]) -> HashMap<u32, Obj> {
    let mut objs = HashMap::new();
    let mut i = 0;
    while let Some(p) = find(data, b" 0 obj", i) {
        i = p + 6;
        let mut s = p;
        while s > 0 && data[s - 1].is_ascii_digit() {
            s -= 1;
        }
        if s == p || (s > 0 && !matches!(data[s - 1], b'\n' | b'\r')) {
            continue;
        }
        let Ok(num) = std::str::from_utf8(&data[s..p])
            .unwrap_or("x")
            .parse::<u32>()
        else {
            continue;
        };
        let end = find(data, b"endobj", i).unwrap_or(data.len());
        let body = &data[i..end];
        let (dict_end, stream) = match find(body, b"stream", 0) {
            Some(sp) if body[..sp].windows(2).any(|w| w == b">>") => {
                let mut st = sp + 6;
                if body.get(st) == Some(&b'\r') {
                    st += 1;
                }
                if body.get(st) == Some(&b'\n') {
                    st += 1;
                }
                let e = find(body, b"endstream", st).unwrap_or(body.len());
                (sp, Some(body[st..e].to_vec()))
            }
            _ => (body.len(), None),
        };
        let dict: String = body[..dict_end].iter().map(|&b| b as char).collect();
        let stream = stream.map(|s| {
            if dict.contains("FlateDecode") {
                inflate(&s)
            } else {
                s
            }
        });
        objs.insert(num, Obj { dict, stream });
        i = end;
    }
    objs
}

/// The number after `key` in `dict`, e.g. `/DW 1000`.
fn num_after(dict: &str, key: &str) -> Option<f64> {
    let at = dict.find(key)? + key.len();
    let rest = dict[at..].trim_start();
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// The object number of `key N 0 R`.
fn ref_after(dict: &str, key: &str) -> Option<u32> {
    let at = dict.find(key)? + key.len();
    let rest = dict[at..].trim_start().trim_start_matches('[').trim_start();
    let end = rest.find(|c: char| !c.is_ascii_digit())?;
    let n: u32 = rest[..end].parse().ok()?;
    rest[end..].trim_start().starts_with("0 R").then_some(n)
}

/// All `/Name N 0 R` pairs inside the `/Font << ... >>` of a resource dictionary.
fn font_refs(dict: &str) -> Vec<(String, u32)> {
    let Some(at) = dict.find("/Font") else {
        return Vec::new();
    };
    let rest = &dict[at + 5..];
    let Some(open) = rest.find("<<") else {
        return Vec::new();
    };
    let rest = &rest[open + 2..];
    let close = rest.find(">>").unwrap_or(rest.len());
    let spaced = rest[..close].replace('/', " /");
    let toks: Vec<&str> = spaced.split_whitespace().collect();
    let mut out = Vec::new();
    let mut k = 0;
    while k + 2 < toks.len() {
        if toks[k].starts_with('/') && toks[k + 2] == "0" && toks.get(k + 3) == Some(&"R") {
            if let Ok(n) = toks[k + 1].parse() {
                out.push((toks[k][1..].to_owned(), n));
            }
            k += 4;
        } else {
            k += 1;
        }
    }
    out
}

fn hex_u32(s: &str) -> u32 {
    u32::from_str_radix(s, 16).unwrap_or(0)
}

fn hex_text(s: &str) -> String {
    let units: Vec<u16> = (0..s.len() / 4)
        .map(|i| hex_u32(&s[i * 4..i * 4 + 4]) as u16)
        .collect();
    String::from_utf16_lossy(&units)
}

fn angle_groups(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(a) = rest.find('<') {
        let Some(b) = rest[a..].find('>') else { break };
        out.push(&rest[a + 1..a + b]);
        rest = &rest[a + b + 1..];
    }
    out
}

fn parse_cmap(cm: &str) -> HashMap<u32, String> {
    let mut map = HashMap::new();
    let mut rest = cm;
    while let Some(a) = rest.find("beginbfchar") {
        let body = &rest[a + 11..];
        let end = body.find("endbfchar").unwrap_or(body.len());
        for pair in angle_groups(&body[..end]).chunks(2) {
            if let [code, uni] = pair {
                map.insert(hex_u32(code), hex_text(uni));
            }
        }
        rest = &body[end..];
    }
    let mut rest = cm;
    while let Some(a) = rest.find("beginbfrange") {
        let body = &rest[a + 12..];
        let end = body.find("endbfrange").unwrap_or(body.len());
        for line in body[..end].lines() {
            let g = angle_groups(line);
            if g.len() == 3 && !line.contains('[') {
                let (lo, hi, d) = (hex_u32(g[0]), hex_u32(g[1]), hex_u32(g[2]));
                for c in lo..=hi {
                    if let Some(ch) = char::from_u32(d + c - lo) {
                        map.insert(c, ch.to_string());
                    }
                }
            } else if g.len() > 3 && line.contains('[') {
                let lo = hex_u32(g[0]);
                for (k, u) in g[2..].iter().enumerate() {
                    map.insert(lo + k as u32, hex_text(u));
                }
            }
        }
        rest = &body[end..];
    }
    map
}

/// Widths of `c [w w w] c [w]` and `c1 c2 w` entries.
fn parse_w(text: &str, widths: &mut HashMap<u32, f64>) {
    let t = text.replace('[', " [ ").replace(']', " ] ");
    let toks: Vec<&str> = t.split_whitespace().collect();
    let mut k = 0;
    while k < toks.len() {
        let Ok(first) = toks[k].parse::<u32>() else {
            k += 1;
            continue;
        };
        if toks.get(k + 1) == Some(&"[") {
            let mut c = first;
            k += 2;
            while k < toks.len() && toks[k] != "]" {
                if let Ok(w) = toks[k].parse::<f64>() {
                    widths.insert(c, w);
                    c += 1;
                }
                k += 1;
            }
            k += 1;
        } else if let (Some(Ok(last)), Some(Ok(w))) = (
            toks.get(k + 1).map(|s| s.parse::<u32>()),
            toks.get(k + 2).map(|s| s.parse::<f64>()),
        ) {
            for c in first..=last {
                widths.insert(c, w);
            }
            k += 3;
        } else {
            k += 1;
        }
    }
}

fn obj_text(objs: &HashMap<u32, Obj>, n: u32) -> String {
    objs.get(&n).map_or(String::new(), |o| o.dict.clone())
}

fn load_font(objs: &HashMap<u32, Obj>, n: u32) -> Font {
    let d = obj_text(objs, n);
    let mut font = Font {
        width_bytes: 1,
        widths: HashMap::new(),
        default_width: 1000.0,
        unicode: HashMap::new(),
    };
    if let Some(tu) = ref_after(&d, "/ToUnicode")
        && let Some(s) = objs.get(&tu).and_then(|o| o.stream.as_ref())
    {
        font.unicode = parse_cmap(&String::from_utf8_lossy(s));
    }
    if d.contains("/Type0") {
        font.width_bytes = 2;
        if let Some(desc) = ref_after(&d, "/DescendantFonts") {
            let mut dd = obj_text(objs, desc);
            if !dd.contains("/DW")
                && !dd.contains("/Subtype")
                && let Some(inner) = ref_after(&dd, "[")
            {
                dd = obj_text(objs, inner);
            }
            font.default_width = num_after(&dd, "/DW").unwrap_or(1000.0);
            let w_text = match ref_after(&dd, "/W ") {
                Some(w) => obj_text(objs, w),
                None => dd
                    .find("/W")
                    .map_or(String::new(), |a| dd[a + 2..].to_owned()),
            };
            parse_w(&w_text, &mut font.widths);
        }
    } else {
        let first = num_after(&d, "/FirstChar").unwrap_or(0.0) as u32;
        let list = match ref_after(&d, "/Widths") {
            Some(w) => obj_text(objs, w),
            None => d
                .find("/Widths")
                .map_or(String::new(), |a| d[a + 7..].to_owned()),
        };
        let inner = list.replace(['[', ']'], " ");
        for (k, w) in inner
            .split_whitespace()
            .filter_map(|s| s.parse::<f64>().ok())
            .enumerate()
        {
            font.widths.insert(first + k as u32, w);
        }
    }
    font
}

#[derive(Debug, Clone)]
enum Tok {
    Num(f64),
    Name(String),
    Op(String),
    Hex(Vec<u8>),
    Lit(Vec<u8>),
    Open,
    Close,
}

fn tokenize(s: &[u8]) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        match c {
            b' ' | b'\n' | b'\r' | b'\t' | 0 => i += 1,
            b'[' => {
                out.push(Tok::Open);
                i += 1;
            }
            b']' => {
                out.push(Tok::Close);
                i += 1;
            }
            b'<' if s.get(i + 1) == Some(&b'<') => i += 2,
            b'>' if s.get(i + 1) == Some(&b'>') => i += 2,
            b'<' => {
                let e = s[i..]
                    .iter()
                    .position(|&b| b == b'>')
                    .map_or(s.len(), |p| i + p);
                let hex: Vec<u8> = s[i + 1..e]
                    .iter()
                    .copied()
                    .filter(u8::is_ascii_hexdigit)
                    .collect();
                let bytes = hex
                    .chunks(2)
                    .map(|p| {
                        let t: String = p.iter().map(|&b| b as char).collect();
                        u8::from_str_radix(&format!("{t:0<2}"), 16).unwrap_or(0)
                    })
                    .collect();
                out.push(Tok::Hex(bytes));
                i = e + 1;
            }
            b'(' => {
                let mut depth = 1;
                let mut j = i + 1;
                let mut bytes = Vec::new();
                while j < s.len() && depth > 0 {
                    match s[j] {
                        b'\\' => {
                            j += 1;
                            if let Some(&n) = s.get(j) {
                                bytes.push(match n {
                                    b'n' => b'\n',
                                    b'r' => b'\r',
                                    b't' => b'\t',
                                    other => other,
                                });
                            }
                        }
                        b'(' => {
                            depth += 1;
                            bytes.push(b'(');
                        }
                        b')' => {
                            depth -= 1;
                            if depth > 0 {
                                bytes.push(b')');
                            }
                        }
                        other => bytes.push(other),
                    }
                    j += 1;
                }
                out.push(Tok::Lit(bytes));
                i = j;
            }
            b'/' => {
                let e = s[i + 1..]
                    .iter()
                    .position(|b| b" \n\r\t[]<>()/".contains(b))
                    .map_or(s.len(), |p| i + 1 + p);
                out.push(Tok::Name(
                    String::from_utf8_lossy(&s[i + 1..e]).into_owned(),
                ));
                i = e;
            }
            _ => {
                let e = s[i..]
                    .iter()
                    .position(|b| b" \n\r\t[]<>()/".contains(b))
                    .map_or(s.len(), |p| i + p);
                let w = String::from_utf8_lossy(&s[i..e]).into_owned();
                match w.parse::<f64>() {
                    Ok(n) => out.push(Tok::Num(n)),
                    Err(_) => out.push(Tok::Op(w)),
                }
                i = e.max(i + 1);
            }
        }
    }
    out
}

fn page_content(objs: &HashMap<u32, Obj>, page: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let Some(at) = page.find("/Contents") else {
        return out;
    };
    let rest = page[at + 9..].trim_start();
    let list: Vec<u32> = if let Some(r) = rest.strip_prefix('[') {
        let r = &r[..r.find(']').unwrap_or(r.len())];
        r.split_whitespace()
            .collect::<Vec<_>>()
            .chunks(3)
            .filter_map(|c| c.first()?.parse().ok())
            .collect()
    } else {
        rest.split_whitespace()
            .next()
            .and_then(|n| n.parse().ok())
            .into_iter()
            .collect()
    };
    for n in list {
        if let Some(s) = objs.get(&n).and_then(|o| o.stream.as_ref()) {
            out.extend_from_slice(s);
            out.push(b'\n');
        }
    }
    out
}

fn page_list(objs: &HashMap<u32, Obj>) -> Vec<u32> {
    let pages = objs.iter().find_map(|(n, o)| {
        (o.dict.contains("/Type/Pages") || o.dict.contains("/Type /Pages")).then_some(*n)
    });
    let Some(p) = pages else { return Vec::new() };
    let d = obj_text(objs, p);
    let Some(k) = d.find("/Kids") else {
        return Vec::new();
    };
    let rest = &d[k + 5..];
    let rest = &rest[rest.find('[').map_or(0, |a| a + 1)..];
    let rest = &rest[..rest.find(']').unwrap_or(rest.len())];
    let toks: Vec<&str> = rest.split_whitespace().collect();
    toks.chunks(3)
        .filter_map(|c| c.first()?.parse().ok())
        .collect()
}

/// The glyphs of every page of the Word PDF in `data`.
pub fn read_pages(data: &[u8]) -> Vec<WordPage> {
    let objs = parse_objects(data);
    let mut fonts: HashMap<u32, Font> = HashMap::new();
    let mut pages = Vec::new();
    for pn in page_list(&objs) {
        let pd = obj_text(&objs, pn);
        let (mut width, mut height) = (612.0, 792.0);
        if let Some(a) = pd.find("/MediaBox") {
            let inner = pd[a + 9..].replace(['[', ']'], " ");
            let v: Vec<f64> = inner
                .split_whitespace()
                .take(4)
                .filter_map(|s| s.parse().ok())
                .collect();
            if v.len() == 4 {
                width = v[2];
                height = v[3];
            }
        }
        let refs = font_refs(&pd);
        for (_, n) in &refs {
            fonts.entry(*n).or_insert_with(|| load_font(&objs, *n));
        }
        let content = page_content(&objs, &pd);
        let toks = tokenize(&content);
        let mut glyphs = Vec::new();
        let mut stack: Vec<Tok> = Vec::new();
        let (mut tx, mut ty) = (0.0, 0.0);
        let mut size = 0.0;
        let mut font: Option<&Font> = None;
        let mut leading = 0.0;
        for t in toks {
            let Tok::Op(op) = &t else {
                stack.push(t);
                continue;
            };
            let nums: Vec<f64> = stack
                .iter()
                .filter_map(|t| if let Tok::Num(n) = t { Some(*n) } else { None })
                .collect();
            match op.as_str() {
                "Tf" => {
                    if let Some(Tok::Name(name)) =
                        stack.iter().rev().find(|t| matches!(t, Tok::Name(_)))
                    {
                        font = refs
                            .iter()
                            .find(|(n, _)| n == name)
                            .and_then(|(_, o)| fonts.get(o));
                    }
                    size = nums.last().copied().unwrap_or(size);
                }
                "Tm" if nums.len() >= 6 => {
                    tx = nums[nums.len() - 2];
                    ty = nums[nums.len() - 1];
                }
                "Td" | "TD" if nums.len() >= 2 => {
                    tx += nums[nums.len() - 2];
                    ty += nums[nums.len() - 1];
                    if op == "TD" {
                        leading = -nums[nums.len() - 1];
                    }
                }
                "TL" => leading = nums.last().copied().unwrap_or(leading),
                "T*" => ty -= leading,
                "BT" => {
                    tx = 0.0;
                    ty = 0.0;
                }
                "Tj" | "TJ" | "'" | "\"" => {
                    if let Some(f) = font {
                        for item in &stack {
                            match item {
                                Tok::Num(n) => tx -= n / 1000.0 * size,
                                Tok::Hex(b) | Tok::Lit(b) => {
                                    for code in b.chunks(f.width_bytes) {
                                        let c =
                                            code.iter().fold(0u32, |a, &b| a << 8 | u32::from(b));
                                        let w =
                                            f.widths.get(&c).copied().unwrap_or(f.default_width);
                                        let text = f
                                            .unicode
                                            .get(&c)
                                            .cloned()
                                            .or_else(|| {
                                                (f.width_bytes == 1)
                                                    .then(|| char::from(code[0]).to_string())
                                            })
                                            .unwrap_or_else(|| "\u{fffd}".to_owned());
                                        for ch in text.chars() {
                                            glyphs.push(WordGlyph {
                                                ch,
                                                x: tx,
                                                y: height - ty,
                                                size,
                                            });
                                        }
                                        tx += w / 1000.0 * size;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                _ => {}
            }
            stack.clear();
        }
        pages.push(WordPage {
            width,
            height,
            glyphs,
        });
    }
    pages
}
