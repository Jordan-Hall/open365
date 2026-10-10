//! Deterministic zip-level and XML-level mutations. Every function is a pure function of
//! its inputs and the [`Rng`], so a `(seed, iteration)` pair reproduces a case.

use std::io::{Cursor, Read, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// splitmix64: tiny, seedable, good enough for mutation choices.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (`n > 0`).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    pub fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in) == 0
    }
}

/// One zip entry in a form that can be edited and re-zipped.
#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub data: Vec<u8>,
    pub stored: bool,
}

/// Reads a well-formed zip into entries. Directory entries are dropped.
pub fn unzip(bytes: &[u8]) -> Vec<Entry> {
    let Ok(mut archive) = ZipArchive::new(Cursor::new(bytes)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for i in 0..archive.len() {
        let Ok(mut f) = archive.by_index(i) else {
            continue;
        };
        if f.is_dir() {
            continue;
        }
        let name = f.name().map_or_else(|_| "?".to_owned(), |n| n.into_owned());
        let stored = f.compression() == CompressionMethod::Stored;
        let mut data = Vec::new();
        if f.read_to_end(&mut data).is_ok() {
            out.push(Entry { name, data, stored });
        }
    }
    out
}

/// Zips entries. Duplicate names are rejected by the writer, so such entries are skipped;
/// use [`zip_with_duplicates`] to make them.
pub fn zip(entries: &[Entry]) -> Vec<u8> {
    let mut w = ZipWriter::new(Cursor::new(Vec::new()));
    for e in entries {
        let method = if e.stored {
            CompressionMethod::Stored
        } else {
            CompressionMethod::Deflated
        };
        if w.start_file(
            e.name.as_str(),
            SimpleFileOptions::default().compression_method(method),
        )
        .is_ok()
        {
            let _ = w.write_all(&e.data);
        }
    }
    w.finish().map(Cursor::into_inner).unwrap_or_default()
}

/// Zips `entries`, then renames the entry named `alias` back to `original` in the raw bytes
/// (same length), producing a zip with two entries of one name.
pub fn zip_with_duplicates(entries: &[Entry], alias: &str, original: &str) -> Vec<u8> {
    let mut bytes = zip(entries);
    if alias.len() != original.len() || alias.is_empty() {
        return bytes;
    }
    let (a, o) = (alias.as_bytes(), original.as_bytes());
    let mut i = 0;
    while i + a.len() <= bytes.len() {
        if &bytes[i..i + a.len()] == a {
            bytes[i..i + a.len()].copy_from_slice(o);
            i += a.len();
        } else {
            i += 1;
        }
    }
    bytes
}

fn flip_bits(data: &mut [u8], rng: &mut Rng) {
    if data.is_empty() {
        return;
    }
    for _ in 0..=rng.below(8) {
        let i = rng.below(data.len());
        data[i] ^= 1 << rng.below(8);
    }
}

fn patch_u32(bytes: &mut [u8], at: usize, value: u32) {
    if let Some(slot) = bytes.get_mut(at..at + 4) {
        slot.copy_from_slice(&value.to_le_bytes());
    }
}

fn find_all(bytes: &[u8], needle: &[u8]) -> Vec<usize> {
    let mut v = Vec::new();
    let mut i = 0;
    while i + needle.len() <= bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            v.push(i);
            i += needle.len();
        } else {
            i += 1;
        }
    }
    v
}

/// A mutated input. `entries` is kept when the case is still described by editable entries
/// (used for minimisation).
#[derive(Debug, Clone)]
pub struct Case {
    pub bytes: Vec<u8>,
    pub entries: Option<Vec<Entry>>,
    pub label: String,
}

/// Mutates raw zip bytes or the entry list (zip level).
pub fn zip_level(seed: &[u8], rng: &mut Rng) -> Case {
    let mut entries = unzip(seed);
    let kind = rng.below(14);
    let label = |s: &str| format!("zip:{s}");
    match kind {
        0 | 1 => {
            let mut bytes = seed.to_vec();
            flip_bits(&mut bytes, rng);
            Case {
                bytes,
                entries: None,
                label: label("bitflip"),
            }
        }
        2 => {
            let mut bytes = seed.to_vec();
            bytes.truncate(rng.below(bytes.len().max(1)));
            Case {
                bytes,
                entries: None,
                label: label("truncate"),
            }
        }
        3 => {
            // Two entries with one name (also exact-case and case-variant).
            if entries.is_empty() {
                return Case {
                    bytes: seed.to_vec(),
                    entries: None,
                    label: label("noop"),
                };
            }
            let i = rng.below(entries.len());
            let original = entries[i].name.clone();
            let mut dup = entries[i].clone();
            if rng.chance(2) {
                dup.name = original.to_ascii_uppercase();
                entries.push(dup);
                let bytes = zip(&entries);
                return Case {
                    bytes,
                    entries: Some(entries),
                    label: label("dup-case"),
                };
            }
            let mut alias = original.clone().into_bytes();
            let last = alias.len() - 1;
            alias[last] = if alias[last] == b'~' { b'!' } else { b'~' };
            dup.name = String::from_utf8_lossy(&alias).into_owned();
            entries.push(dup.clone());
            let bytes = zip_with_duplicates(&entries, &dup.name, &original);
            Case {
                bytes,
                entries: None,
                label: label("dup-name"),
            }
        }
        4 | 5 => {
            // Lie about sizes in the central directory and/or local headers.
            let mut bytes = seed.to_vec();
            let values = [u32::MAX, 0, 1, 0x7FFF_FFFF, 0x1000_0000, rng.next() as u32];
            for (sig, comp_off, size_off) in
                [(&b"PK\x01\x02"[..], 20, 24), (&b"PK\x03\x04"[..], 18, 22)]
            {
                for at in find_all(seed, sig) {
                    if rng.chance(2) {
                        patch_u32(&mut bytes, at + size_off, values[rng.below(values.len())]);
                    }
                    if rng.chance(3) {
                        patch_u32(&mut bytes, at + comp_off, values[rng.below(values.len())]);
                    }
                }
            }
            Case {
                bytes,
                entries: None,
                label: label("sizes"),
            }
        }
        6 => {
            // Zip bomb: one part becomes a long run of one byte.
            if entries.is_empty() {
                return Case {
                    bytes: seed.to_vec(),
                    entries: None,
                    label: label("noop"),
                };
            }
            let i = rng.below(entries.len());
            let len = (1 + rng.below(24)) * 1024 * 1024;
            entries[i].data = vec![if rng.chance(2) { b' ' } else { 0 }; len];
            entries[i].stored = false;
            let bytes = zip(&entries);
            Case {
                bytes,
                entries: Some(entries),
                label: label("bomb"),
            }
        }
        7 => {
            // Hostile names.
            if entries.is_empty() {
                return Case {
                    bytes: seed.to_vec(),
                    entries: None,
                    label: label("noop"),
                };
            }
            let i = rng.below(entries.len());
            let n = entries[i].name.clone();
            entries[i].name = match rng.below(9) {
                0 => format!("../{n}"),
                1 => format!("/{n}"),
                2 => n.replace('/', "\\"),
                3 => format!("{n}\u{1}"),
                4 => String::new(),
                5 => "a/".repeat(5000) + "x.xml",
                6 => format!("{n}/../{n}"),
                7 => format!("./{n}"),
                _ => format!("{n}\u{202e}\u{feff}"),
            };
            let bytes = zip(&entries);
            Case {
                bytes,
                entries: Some(entries),
                label: label("names"),
            }
        }
        8 => {
            // Many entries.
            let count = if rng.chance(8) {
                21_000
            } else {
                100 + rng.below(3000)
            };
            for k in 0..count {
                entries.push(Entry {
                    name: format!("junk/{k}.xml"),
                    data: Vec::new(),
                    stored: true,
                });
            }
            let bytes = zip(&entries);
            Case {
                bytes,
                entries: Some(entries),
                label: label("many"),
            }
        }
        9 => {
            if !entries.is_empty() {
                entries.remove(rng.below(entries.len()));
            }
            let bytes = zip(&entries);
            Case {
                bytes,
                entries: Some(entries),
                label: label("drop-entry"),
            }
        }
        10 => {
            if entries.len() > 1 {
                let (a, b) = (rng.below(entries.len()), rng.below(entries.len()));
                entries.swap(a, b);
            }
            for e in &mut entries {
                if rng.chance(3) {
                    e.stored = !e.stored;
                }
            }
            let bytes = zip(&entries);
            Case {
                bytes,
                entries: Some(entries),
                label: label("reorder"),
            }
        }
        11 => {
            let mut bytes = vec![0u8; rng.below(4096)];
            bytes.extend_from_slice(seed);
            if rng.chance(2) {
                bytes.extend((0..rng.below(4096)).map(|_| rng.next() as u8));
            }
            Case {
                bytes,
                entries: None,
                label: label("junk-around"),
            }
        }
        12 => {
            let mut bytes = seed.to_vec();
            if !bytes.is_empty() {
                let start = rng.below(bytes.len());
                let end = (start + 1 + rng.below(64)).min(bytes.len());
                bytes.drain(start..end);
            }
            Case {
                bytes,
                entries: None,
                label: label("delete-range"),
            }
        }
        _ => {
            // Corrupt entry bytes after unzip (deflate streams stay valid; contents do not).
            if !entries.is_empty() {
                let i = rng.below(entries.len());
                flip_bits(&mut entries[i].data, rng);
            }
            let bytes = zip(&entries);
            Case {
                bytes,
                entries: Some(entries),
                label: label("entry-bitflip"),
            }
        }
    }
}

const DTDS: &[&str] = &[
    "<!DOCTYPE x [<!ENTITY a \"aaaaaaaaaa\"><!ENTITY b \"&a;&a;&a;&a;&a;&a;&a;&a;\"><!ENTITY c \"&b;&b;&b;&b;&b;&b;&b;&b;\">]>",
    "<!DOCTYPE x SYSTEM \"http://127.0.0.1:9/x.dtd\">",
    "<!DOCTYPE x [<!ENTITY e SYSTEM \"file:///etc/passwd\">]>",
    "<!DOCTYPE x [<!ENTITY % p SYSTEM \"http://127.0.0.1:9/p\"> %p;]>",
    "<!DOCTYPE x [<!ENTITY a \"&a;\">]>",
];

fn root_start_end(xml: &[u8]) -> Option<usize> {
    // Index just after the first '>' that closes the root start tag.
    let mut i = 0;
    while i < xml.len() {
        if xml[i] == b'<' && i + 1 < xml.len() && !matches!(xml[i + 1], b'?' | b'!' | b'/') {
            return xml[i..].iter().position(|&b| b == b'>').map(|p| i + p + 1);
        }
        i += 1;
    }
    None
}

/// Mutates one XML (or `.rels`) part and re-zips the package.
pub fn xml_level(seed: &[u8], rng: &mut Rng) -> Case {
    let mut entries = unzip(seed);
    let xml_ids: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.name.ends_with(".xml") || e.name.ends_with(".rels"))
        .map(|(i, _)| i)
        .collect();
    if xml_ids.is_empty() {
        return zip_level(seed, rng);
    }
    let i = xml_ids[rng.below(xml_ids.len())];
    let kind = rng.below(13);
    let name = entries[i].name.clone();
    let d = &mut entries[i].data;
    let label = match kind {
        0 | 1 => {
            flip_bits(d, rng);
            "bitflip"
        }
        2 => {
            d.truncate(rng.below(d.len().max(1)));
            "truncate"
        }
        3 => {
            // DTD after the XML declaration (or at the start).
            let at = d.windows(2).position(|w| w == b"?>").map_or(0, |p| p + 2);
            let dtd = DTDS[rng.below(DTDS.len())];
            d.splice(at..at, dtd.bytes());
            "dtd"
        }
        4 | 5 => {
            // Deep nesting right after the root start tag.
            let depth = [200, 1_000, 10_000, 100_000][rng.below(4)];
            let tag = if rng.chance(2) { "w:p" } else { "a" };
            if let Some(at) = root_start_end(d) {
                let open = format!("<{tag}>").repeat(depth);
                let close = if rng.chance(2) {
                    format!("</{tag}>").repeat(depth)
                } else {
                    String::new()
                };
                let mut ins = open.into_bytes();
                ins.extend_from_slice(close.as_bytes());
                d.splice(at..at, ins);
            }
            "deep-nesting"
        }
        6 => {
            // Replace a run of ASCII digits by an extreme number.
            let digits: Vec<usize> = d
                .iter()
                .enumerate()
                .filter(|(j, b)| b.is_ascii_digit() && (*j == 0 || !d[*j - 1].is_ascii_digit()))
                .map(|(j, _)| j)
                .collect();
            if !digits.is_empty() {
                let at = digits[rng.below(digits.len())];
                let mut end = at;
                while end < d.len() && d[end].is_ascii_digit() {
                    end += 1;
                }
                let values = [
                    "4294967295",
                    "4294967296",
                    "99999999999999999999",
                    "-1",
                    "1048577",
                    "16385",
                    "2147483648",
                    "0",
                    "1e308",
                    "NaN",
                ];
                d.splice(at..end, values[rng.below(values.len())].bytes());
            }
            "numbers"
        }
        7 => {
            // Repeat a random slice (duplicated rows, runs, elements).
            if d.len() > 4 {
                let start = rng.below(d.len());
                let end = (start + 1 + rng.below(512)).min(d.len());
                let slice = d[start..end].to_vec();
                let times = 1 + rng.below(200);
                let mut ins = Vec::new();
                for _ in 0..times {
                    ins.extend_from_slice(&slice);
                }
                d.splice(end..end, ins);
            }
            "repeat-slice"
        }
        8 => {
            if !d.is_empty() {
                let start = rng.below(d.len());
                let end = (start + 1 + rng.below(256)).min(d.len());
                d.drain(start..end);
            }
            "delete-range"
        }
        9 => {
            let refs = [
                "&#0;",
                "&#xD800;",
                "&#x110000;",
                "&bogus;",
                "&#;",
                "&",
                "&amp",
                "&#x1F600;",
                "\u{0}",
                "\u{feff}",
            ];
            if !d.is_empty() {
                let at = rng.below(d.len());
                d.splice(at..at, refs[rng.below(refs.len())].bytes());
            }
            "refs"
        }
        10 => {
            // Giant text/attribute payload.
            let at = d.iter().position(|&b| b == b'>').map_or(0, |p| p + 1);
            let n = (1 + rng.below(4)) * 256 * 1024;
            d.splice(at..at, std::iter::repeat_n(b'A', n));
            "huge-text"
        }
        11 => {
            let mut prefix: Vec<u8> = match rng.below(4) {
                0 => vec![0xEF, 0xBB, 0xBF],
                1 => vec![0xFF, 0xFE],
                2 => vec![0xFE, 0xFF],
                _ => vec![0xC3, 0x28],
            };
            prefix.extend_from_slice(d);
            *d = prefix;
            "bom-encoding"
        }
        _ => {
            // Swap two byte ranges (reorders tags and attributes).
            if d.len() > 8 {
                let a = rng.below(d.len() - 4);
                let b = rng.below(d.len() - 4);
                for k in 0..4 {
                    d.swap(a + k, b + k);
                }
            }
            "swap"
        }
    };
    let bytes = zip(&entries);
    Case {
        bytes,
        entries: Some(entries),
        label: format!("xml:{label}:{name}"),
    }
}

/// Mutates a formula string at the character level.
pub fn formula(seed: &str, rng: &mut Rng) -> String {
    let mut chars: Vec<char> = seed.chars().collect();
    let pieces = [
        "(",
        ")",
        ",",
        ";",
        "\"",
        "'",
        "!",
        ":",
        "$",
        "#REF!",
        "{",
        "}",
        "[",
        "]",
        "^",
        "%",
        "&",
        "<>",
        ">=",
        "1E308",
        "1048577",
        "XFD",
        "AAAA1",
        "R1C1",
        "'Sheet 1'!",
        "\u{0}",
        "\u{202e}",
        "9999999999999999999999",
        "SUM(",
        "IF(",
        "LET(",
        "LAMBDA(",
        ".",
        "-",
        "+",
        "@",
        "#",
    ];
    for _ in 0..=rng.below(4) {
        match rng.below(6) {
            0 if !chars.is_empty() => {
                let i = rng.below(chars.len());
                chars.remove(i);
            }
            1 => {
                let i = rng.below(chars.len() + 1);
                for (k, c) in pieces[rng.below(pieces.len())].chars().enumerate() {
                    chars.insert((i + k).min(chars.len()), c);
                }
            }
            2 if !chars.is_empty() => {
                let i = rng.below(chars.len());
                chars[i] = char::from_u32(rng.next() as u32 % 0x300).unwrap_or('?');
            }
            3 => {
                // Nest deeply.
                let depth = [50, 500, 5_000, 50_000][rng.below(4)];
                let open = ["(", "-", "{", "SUM(", "IF(1,"][rng.below(5)];
                let mut s = String::from("=");
                s.push_str(&open.repeat(depth));
                s.push('1');
                chars = s.chars().collect();
            }
            4 if chars.len() > 1 => {
                let i = rng.below(chars.len());
                let j = rng.below(chars.len());
                chars.swap(i, j);
            }
            _ => {
                let rep: Vec<char> = chars.clone();
                chars.extend(rep);
            }
        }
        if chars.len() > 200_000 {
            chars.truncate(200_000);
        }
    }
    chars.into_iter().collect()
}
