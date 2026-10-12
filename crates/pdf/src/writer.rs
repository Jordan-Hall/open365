//! Minimal PDF object writer: numbered objects, Flate streams, a classic xref table.

use std::io::Write as _;

use flate2::Compression;
use flate2::write::ZlibEncoder;

/// Collects indirect objects and serialises them.
#[derive(Debug)]
pub struct Pdf {
    objects: Vec<Option<Vec<u8>>>,
    /// The header version: `1.7`, or `1.4` for PDF/A-1.
    version: &'static str,
}

impl Default for Pdf {
    fn default() -> Self {
        Self::with_version("1.7")
    }
}

impl Pdf {
    pub fn with_version(version: &'static str) -> Self {
        Self {
            objects: Vec::new(),
            version,
        }
    }

    /// Reserves an object number to be filled with [`Pdf::set`].
    pub fn reserve(&mut self) -> u32 {
        self.objects.push(None);
        self.objects.len() as u32
    }

    pub fn set(&mut self, id: u32, body: impl Into<Vec<u8>>) {
        self.objects[id as usize - 1] = Some(body.into());
    }

    /// The number of objects.
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// The body of object `id`, when it is set.
    pub fn body(&self, id: u32) -> Option<&[u8]> {
        self.objects.get(id as usize - 1)?.as_deref()
    }

    pub fn add(&mut self, body: impl Into<Vec<u8>>) -> u32 {
        let id = self.reserve();
        self.set(id, body);
        id
    }

    /// Adds a stream object. `dict` holds the extra dictionary entries (no brackets).
    pub fn add_stream(&mut self, dict: &str, data: &[u8], compress: bool) -> u32 {
        let id = self.reserve();
        self.set_stream(id, dict, data, compress);
        id
    }

    pub fn set_stream(&mut self, id: u32, dict: &str, data: &[u8], compress: bool) {
        let (payload, filter) = if compress {
            (deflate(data), " /Filter /FlateDecode")
        } else {
            (data.to_vec(), "")
        };
        let mut out =
            format!("<< {dict}{filter} /Length {} >>\nstream\n", payload.len()).into_bytes();
        out.extend_from_slice(&payload);
        out.extend_from_slice(b"\nendstream");
        self.set(id, out);
    }

    /// Serialises the file. `extra_trailer` is appended to the trailer dictionary.
    pub fn finish(self, root: u32, extra_trailer: &str, id: [u8; 16]) -> Vec<u8> {
        let mut out: Vec<u8> = format!("%PDF-{}\n", self.version).into_bytes();
        out.extend_from_slice(b"%\xE2\xE3\xCF\xD3\n");
        let mut offsets = Vec::with_capacity(self.objects.len());
        for (i, obj) in self.objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(obj.as_deref().unwrap_or(b"null"));
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes(),
        );
        for o in &offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        let hex: String = id.iter().map(|b| format!("{b:02x}")).collect();
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root {root} 0 R {extra_trailer} /ID [<{hex}> <{hex}>] >>\nstartxref\n{xref}\n%%EOF\n",
                offsets.len() + 1
            )
            .as_bytes(),
        );
        out
    }
}

pub fn deflate(data: &[u8]) -> Vec<u8> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    // Writing to a Vec cannot fail.
    let _ = enc.write_all(data);
    enc.finish().unwrap_or_default()
}

/// Formats a number with at most `places` decimals and no trailing zeros.
pub fn num(v: f64, places: usize) -> String {
    let v = if v.is_finite() { v } else { 0.0 };
    let mut s = format!("{v:.places$}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" { "0".to_owned() } else { s }
}

/// A PDF text string: a literal for ASCII, UTF-16BE with a byte order mark otherwise.
pub fn text_string(s: &str) -> String {
    if s.is_ascii() && !s.bytes().any(|b| b < 0x20) {
        let mut out = String::from("(");
        for c in s.chars() {
            if matches!(c, '(' | ')' | '\\') {
                out.push('\\');
            }
            out.push(c);
        }
        out.push(')');
        return out;
    }
    let mut out = String::from("<FEFF");
    for u in s.encode_utf16() {
        out.push_str(&format!("{u:04X}"));
    }
    out.push('>');
    out
}

/// Escapes text for XML.
pub fn xml_escape(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .fold(String::new(), |mut out, c| {
            match c {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                c => out.push(c),
            }
            out
        })
}

/// A 128-bit FNV-style digest, deterministic, for the file identifier.
pub fn digest(data: &[u8]) -> [u8; 16] {
    let mut a: u64 = 0xcbf2_9ce4_8422_2325;
    let mut b: u64 = 0x8422_2325_cbf2_9ce4;
    for &x in data {
        a = (a ^ u64::from(x)).wrapping_mul(0x0000_0100_0000_01b3);
        b = (b.rotate_left(5) ^ u64::from(x)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&a.to_be_bytes());
    out[8..].copy_from_slice(&b.to_be_bytes());
    out
}
