//! A small reader for the PDF files Word exports: indirect objects found by scanning, the page
//! tree walked in order, FlateDecode content streams interpreted for path construction and
//! painting operators only. Text, images and clipping are skipped.

use std::collections::HashMap;
use std::io::Read as _;

use super::geom::Cmd;

struct Object {
    /// The dictionary text before the stream (or the whole body), one char per byte.
    dict: String,
    stream: Option<Vec<u8>>,
}

pub struct Pdf {
    objs: HashMap<u32, Object>,
}

fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from >= hay.len() {
        return None;
    }
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

fn number_after(s: &str, key: &str) -> Option<usize> {
    let i = s.find(key)? + key.len();
    let rest = s[i..].trim_start();
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    // `/Length 12 0 R` is an indirect length: not usable here.
    let after = rest[digits.len()..].trim_start();
    if after.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Object numbers referenced after `key`: `/Key 5 0 R` or `/Key [ 5 0 R 6 0 R ]`.
fn refs_after(dict: &str, key: &str) -> Vec<u32> {
    let mut from = 0;
    while let Some(p) = dict[from..].find(key) {
        let i = from + p + key.len();
        from = i;
        let rest = dict[i..].trim_start();
        let (body, array) = match rest.strip_prefix('[') {
            Some(r) => (r.split(']').next().unwrap_or(""), true),
            None => (rest, false),
        };
        let toks: Vec<&str> = body.split_whitespace().collect();
        let mut out = Vec::new();
        let mut k = 0;
        while k + 2 < toks.len() && toks[k + 2].starts_with('R') {
            let Ok(n) = toks[k].parse::<u32>() else { break };
            out.push(n);
            k += 3;
            if !array {
                break;
            }
        }
        if !out.is_empty() {
            return out;
        }
    }
    Vec::new()
}

fn inflate(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    // A damaged tail still yields what was decoded.
    let _ = flate2::read::ZlibDecoder::new(bytes).read_to_end(&mut out);
    out
}

impl Pdf {
    pub fn parse(data: &[u8]) -> Self {
        let mut objs = HashMap::new();
        let mut i = 0;
        while let Some(p) = find(data, b" obj", i) {
            i = p + 4;
            // "<num> <gen> obj" at the start of a line.
            let mut j = p;
            while j > 0 && data[j - 1].is_ascii_digit() {
                j -= 1;
            }
            if j == p || j == 0 || data[j - 1] != b' ' {
                continue;
            }
            let mut k = j - 1;
            while k > 0 && data[k - 1].is_ascii_digit() {
                k -= 1;
            }
            if k == j - 1 || !(k == 0 || matches!(data[k - 1], b'\n' | b'\r')) {
                continue;
            }
            let Ok(num) = latin1(&data[k..j - 1]).parse::<u32>() else {
                continue;
            };
            let body = p + 4;
            let end_obj = find(data, b"endobj", body).unwrap_or(data.len());
            // `stream` right after the dictionary (`>>` then optional white space).
            let mut stream_at = None;
            let mut s = body;
            while let Some(q) = find(data, b"stream", s) {
                if q >= end_obj {
                    break;
                }
                let prev = data[body..q]
                    .iter()
                    .rev()
                    .find(|c| !c.is_ascii_whitespace());
                if prev == Some(&b'>') {
                    stream_at = Some(q);
                    break;
                }
                s = q + 6;
            }
            let obj = match stream_at {
                None => Object {
                    dict: latin1(&data[body..end_obj]),
                    stream: None,
                },
                Some(q) => {
                    let dict = latin1(&data[body..q]);
                    let mut st = q + 6;
                    if data.get(st) == Some(&b'\r') {
                        st += 1;
                    }
                    if data.get(st) == Some(&b'\n') {
                        st += 1;
                    }
                    let en = match number_after(&dict, "/Length") {
                        Some(n) if st + n <= data.len() => st + n,
                        _ => find(data, b"endstream", st).unwrap_or(data.len()),
                    };
                    i = en;
                    let mut bytes = data[st..en].to_vec();
                    if dict.contains("FlateDecode") {
                        bytes = inflate(&bytes);
                    }
                    Object {
                        dict,
                        stream: Some(bytes),
                    }
                }
            };
            objs.insert(num, obj);
        }
        Self { objs }
    }

    fn dict_compact(&self, id: u32) -> String {
        self.objs
            .get(&id)
            .map(|o| o.dict.split_whitespace().collect::<String>())
            .unwrap_or_default()
    }

    /// The page objects in document order.
    pub fn page_ids(&self) -> Vec<u32> {
        let root = self
            .objs
            .iter()
            .find(|(id, _)| self.dict_compact(**id).contains("/Type/Catalog"))
            .and_then(|(_, o)| refs_after(&o.dict, "/Pages").first().copied());
        let mut out = Vec::new();
        if let Some(r) = root {
            self.walk(r, &mut out, 0);
        }
        out
    }

    fn walk(&self, id: u32, out: &mut Vec<u32>, depth: usize) {
        let Some(o) = self.objs.get(&id) else { return };
        if depth > 16 {
            return;
        }
        if o.dict.contains("/Kids") {
            for k in refs_after(&o.dict, "/Kids") {
                self.walk(k, out, depth + 1);
            }
        } else {
            out.push(id);
        }
    }

    /// The decoded, concatenated content stream of a page.
    pub fn page_content(&self, page: u32) -> Vec<u8> {
        let Some(o) = self.objs.get(&page) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for c in refs_after(&o.dict, "/Contents") {
            if let Some(Object {
                stream: Some(s), ..
            }) = self.objs.get(&c)
            {
                out.extend_from_slice(s);
                out.push(b'\n');
            }
        }
        out
    }

    /// The painted paths of every page, in user space (points, y up).
    pub fn page_paths(&self) -> Vec<Vec<Painted>> {
        self.page_ids()
            .into_iter()
            .map(|p| paint_ops(&self.page_content(p)))
            .collect()
    }
}

/// One painted path: the paint operator (`S`, `f`, `B`, ...) and its subpaths.
#[derive(Debug, Clone, PartialEq)]
pub struct Painted {
    pub paint: String,
    pub subpaths: Vec<Vec<Cmd>>,
}

impl Painted {
    pub fn stroked(&self) -> bool {
        matches!(self.paint.as_str(), "S" | "s" | "B" | "B*" | "b" | "b*")
    }
}

#[derive(Debug, Clone)]
enum Tok {
    Num(f64),
    Other,
    Op(String),
}

fn tokens(src: &[u8]) -> Vec<Tok> {
    let mut out = Vec::new();
    let n = src.len();
    let mut i = 0;
    let is_delim = |c: u8| c.is_ascii_whitespace() || b"()<>[]{}/%".contains(&c);
    while i < n {
        let c = src[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c == b'%' {
            while i < n && src[i] != b'\n' && src[i] != b'\r' {
                i += 1;
            }
        } else if c == b'(' {
            let mut depth = 0;
            while i < n {
                match src[i] {
                    b'\\' => i += 1,
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            i += 1;
            out.push(Tok::Other);
        } else if c == b'<' && src.get(i + 1) == Some(&b'<') {
            let mut depth = 0;
            while i + 1 < n {
                if src[i] == b'<' && src[i + 1] == b'<' {
                    depth += 1;
                    i += 2;
                } else if src[i] == b'>' && src[i + 1] == b'>' {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            out.push(Tok::Other);
        } else if c == b'<' {
            while i < n && src[i] != b'>' {
                i += 1;
            }
            i += 1;
            out.push(Tok::Other);
        } else if c == b'[' {
            let mut depth = 0;
            while i < n {
                match src[i] {
                    b'[' => depth += 1,
                    b']' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    b'(' => {
                        // Skip a string so a bracket inside it does not count.
                        let mut d = 0;
                        while i < n {
                            match src[i] {
                                b'\\' => i += 1,
                                b'(' => d += 1,
                                b')' => {
                                    d -= 1;
                                    if d == 0 {
                                        break;
                                    }
                                }
                                _ => {}
                            }
                            i += 1;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            i += 1;
            out.push(Tok::Other);
        } else if c == b'/' {
            i += 1;
            while i < n && !is_delim(src[i]) {
                i += 1;
            }
            out.push(Tok::Other);
        } else if matches!(c, b'>' | b']' | b'{' | b'}' | b')') {
            i += 1;
        } else {
            let s = i;
            while i < n && !is_delim(src[i]) {
                i += 1;
            }
            let w = latin1(&src[s..i]);
            match w.parse::<f64>() {
                Ok(v)
                    if w.bytes()
                        .next()
                        .is_some_and(|b| b.is_ascii_digit() || b"+-.".contains(&b)) =>
                {
                    out.push(Tok::Num(v));
                }
                _ => out.push(Tok::Op(w)),
            }
        }
    }
    out
}

type Matrix = [f64; 6];

fn mul(a: Matrix, b: Matrix) -> Matrix {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

fn apply(m: &Matrix, x: f64, y: f64) -> (f64, f64) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

/// Interprets a content stream and returns the paths it paints.
pub fn paint_ops(content: &[u8]) -> Vec<Painted> {
    let mut out = Vec::new();
    let mut ctm: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    let mut stack: Vec<Matrix> = Vec::new();
    let mut subs: Vec<Vec<Cmd>> = Vec::new();
    let mut cur = (0.0, 0.0);
    let mut args: Vec<f64> = Vec::new();
    for t in tokens(content) {
        let op = match t {
            Tok::Num(v) => {
                args.push(v);
                continue;
            }
            Tok::Other => {
                args.clear();
                continue;
            }
            Tok::Op(op) => op,
        };
        let a = std::mem::take(&mut args);
        let pt = |m: &Matrix, i: usize| apply(m, a[i], a[i + 1]);
        match (op.as_str(), a.len()) {
            ("q", _) => stack.push(ctm),
            ("Q", _) => {
                if let Some(m) = stack.pop() {
                    ctm = m;
                }
            }
            ("cm", 6) => ctm = mul([a[0], a[1], a[2], a[3], a[4], a[5]], ctm),
            ("m", 2) => {
                cur = pt(&ctm, 0);
                subs.push(vec![Cmd::M(cur.0, cur.1)]);
            }
            ("l", 2) => {
                cur = pt(&ctm, 0);
                if let Some(s) = subs.last_mut() {
                    s.push(Cmd::L(cur.0, cur.1));
                }
            }
            ("c", 6) | ("v", 4) | ("y", 4) => {
                let (c1, c2, e) = match op.as_str() {
                    "c" => (pt(&ctm, 0), pt(&ctm, 2), pt(&ctm, 4)),
                    "v" => (cur, pt(&ctm, 0), pt(&ctm, 2)),
                    _ => (pt(&ctm, 0), pt(&ctm, 2), pt(&ctm, 2)),
                };
                cur = e;
                if let Some(s) = subs.last_mut() {
                    s.push(Cmd::C(c1.0, c1.1, c2.0, c2.1, e.0, e.1));
                }
            }
            ("h", _) => {
                if let Some(s) = subs.last_mut() {
                    s.push(Cmd::Z);
                }
            }
            ("re", 4) => {
                let (x, y, w, h) = (a[0], a[1], a[2], a[3]);
                let p = [(x, y), (x + w, y), (x + w, y + h), (x, y + h)]
                    .map(|(px, py)| apply(&ctm, px, py));
                subs.push(vec![
                    Cmd::M(p[0].0, p[0].1),
                    Cmd::L(p[1].0, p[1].1),
                    Cmd::L(p[2].0, p[2].1),
                    Cmd::L(p[3].0, p[3].1),
                    Cmd::Z,
                ]);
                cur = p[0];
            }
            ("S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*", _) => {
                if matches!(op.as_str(), "s" | "b" | "b*")
                    && let Some(s) = subs.last_mut()
                {
                    s.push(Cmd::Z);
                }
                out.push(Painted {
                    paint: op,
                    subpaths: std::mem::take(&mut subs),
                });
            }
            ("n", _) => subs.clear(),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paints_and_discards() {
        let p = paint_ops(
            b"q 0 0 612 792 re W* n Q /P <</MCID 1>> BDC 0.75 w 72 696 m 72 709.25 82.745 720 96 720 c 96 600 l h S EMC 10 10 5 5 re f",
        );
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].paint, "S");
        assert_eq!(
            p[0].subpaths,
            vec![vec![
                Cmd::M(72.0, 696.0),
                Cmd::C(72.0, 709.25, 82.745, 720.0, 96.0, 720.0),
                Cmd::L(96.0, 600.0),
                Cmd::Z
            ]]
        );
        assert_eq!(p[1].paint, "f");
    }

    #[test]
    fn cm_scales_points() {
        let p = paint_ops(b"q 2 0 0 -2 0 100 cm 1 1 m 2 2 l S Q");
        assert_eq!(p[0].subpaths[0][0], Cmd::M(2.0, 98.0));
        assert_eq!(p[0].subpaths[0][1], Cmd::L(4.0, 96.0));
    }
}
