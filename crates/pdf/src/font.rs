//! Font embedding: glyf subsetting for TrueType, whole-file embedding for CFF/OpenType,
//! widths and the ToUnicode CMap.

use std::collections::{BTreeMap, BTreeSet};

use open365_text::FontFace;
use skrifa::raw::TableProvider;
use skrifa::{FontRef, MetadataProvider};

use crate::PdfError;
use crate::writer::{Pdf, digest, num};

/// Per-face data the content writer needs while drawing.
#[derive(Debug, Clone)]
pub struct FaceDraw {
    pub resource: String,
    /// Integer widths in 1/1000 em as written to /W.
    pub widths: BTreeMap<u32, i64>,
    pub underline_pos: f64,
    pub underline_thickness: f64,
    pub strike_pos: f64,
    pub strike_thickness: f64,
}

struct Sfnt<'a> {
    tables: Vec<([u8; 4], &'a [u8])>,
}

fn be16(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(o..o + 2)?.try_into().ok()?))
}

fn be32(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

impl<'a> Sfnt<'a> {
    fn parse(data: &'a [u8], index: u32) -> Option<Self> {
        let mut base = 0usize;
        if data.get(..4)? == b"ttcf" {
            let count = be32(data, 8)?;
            if index >= count {
                return None;
            }
            base = be32(data, 12 + 4 * index as usize)? as usize;
        }
        let n = usize::from(be16(data, base + 4)?);
        let mut tables = Vec::with_capacity(n);
        for i in 0..n {
            let rec = base + 12 + 16 * i;
            let tag: [u8; 4] = data.get(rec..rec + 4)?.try_into().ok()?;
            let off = be32(data, rec + 8)? as usize;
            let len = be32(data, rec + 12)? as usize;
            tables.push((tag, data.get(off..off.checked_add(len)?)?));
        }
        Some(Self { tables })
    }

    fn get(&self, tag: &[u8; 4]) -> Option<&'a [u8]> {
        self.tables.iter().find(|(t, _)| t == tag).map(|(_, d)| *d)
    }
}

fn checksum(data: &[u8]) -> u32 {
    let mut sum = 0u32;
    for chunk in data.chunks(4) {
        let mut w = [0u8; 4];
        w[..chunk.len()].copy_from_slice(chunk);
        sum = sum.wrapping_add(u32::from_be_bytes(w));
    }
    sum
}

/// Serialises a standalone sfnt from `tables`, sorted by tag as the spec requires.
fn build_sfnt(mut tables: Vec<([u8; 4], Vec<u8>)>, cff: bool) -> Vec<u8> {
    tables.sort_by_key(|(t, _)| *t);
    let n = tables.len() as u16;
    let mut pow = 1u16;
    let mut log = 0u16;
    while pow * 2 <= n {
        pow *= 2;
        log += 1;
    }
    let mut out = Vec::new();
    out.extend_from_slice(if cff { b"OTTO" } else { &[0, 1, 0, 0] });
    out.extend_from_slice(&n.to_be_bytes());
    out.extend_from_slice(&(pow * 16).to_be_bytes());
    out.extend_from_slice(&log.to_be_bytes());
    out.extend_from_slice(&(n * 16 - pow * 16).to_be_bytes());
    let mut offset = 12 + 16 * tables.len();
    let mut head_pos = None;
    for (tag, data) in &tables {
        out.extend_from_slice(tag);
        out.extend_from_slice(&checksum(data).to_be_bytes());
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        if tag == b"head" {
            head_pos = Some(offset);
        }
        offset += data.len().div_ceil(4) * 4;
    }
    for (_, data) in &tables {
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(4) * 4, 0);
    }
    if let Some(h) = head_pos {
        // checkSumAdjustment was zeroed by the caller.
        let adj = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
        if let Some(slot) = out.get_mut(h + 8..h + 12) {
            slot.copy_from_slice(&adj.to_be_bytes());
        }
    }
    out
}

/// Glyph ids a composite glyph pulls in; `None` for malformed data.
fn components(glyph: &[u8]) -> Option<Vec<u16>> {
    if glyph.is_empty() {
        return Some(Vec::new());
    }
    let contours = i16::from_be_bytes(glyph.get(0..2)?.try_into().ok()?);
    if contours >= 0 {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    let mut pos = 10;
    for _ in 0..1024 {
        let flags = be16(glyph, pos)?;
        out.push(be16(glyph, pos + 2)?);
        pos += 4 + if flags & 0x1 != 0 { 4 } else { 2 };
        if flags & 0x8 != 0 {
            pos += 2;
        } else if flags & 0x40 != 0 {
            pos += 4;
        } else if flags & 0x80 != 0 {
            pos += 8;
        }
        if flags & 0x20 == 0 {
            return Some(out);
        }
    }
    None
}

fn glyph_slice<'a>(loca: &[u8], glyf: &'a [u8], long: bool, gid: usize) -> Option<&'a [u8]> {
    let (a, b) = if long {
        (
            be32(loca, gid * 4)? as usize,
            be32(loca, gid * 4 + 4)? as usize,
        )
    } else {
        (
            usize::from(be16(loca, gid * 2)?) * 2,
            usize::from(be16(loca, gid * 2 + 2)?) * 2,
        )
    };
    if b < a { None } else { glyf.get(a..b) }
}

/// Rebuilds the font with outlines only for `keep` and the glyphs its composites need. Glyph
/// ids stay the same, so the PDF can use them as CIDs with an identity CIDToGIDMap.
fn subset_truetype(sfnt: &Sfnt<'_>, keep: &BTreeSet<u32>) -> Option<Vec<u8>> {
    let head = sfnt.get(b"head")?;
    let loca = sfnt.get(b"loca")?;
    let glyf = sfnt.get(b"glyf")?;
    let maxp = sfnt.get(b"maxp")?;
    let num_glyphs = usize::from(be16(maxp, 4)?);
    let long = be16(head, 50)? != 0;
    let mut closure: BTreeSet<usize> = BTreeSet::new();
    let mut stack: Vec<usize> = keep.iter().map(|g| *g as usize).collect();
    stack.push(0);
    while let Some(g) = stack.pop() {
        if g >= num_glyphs || !closure.insert(g) {
            continue;
        }
        for c in components(glyph_slice(loca, glyf, long, g)?)? {
            stack.push(usize::from(c));
        }
    }
    let mut new_glyf = Vec::new();
    let mut new_loca = Vec::with_capacity((num_glyphs + 1) * 4);
    for g in 0..num_glyphs {
        new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());
        if closure.contains(&g) {
            new_glyf.extend_from_slice(glyph_slice(loca, glyf, long, g)?);
            new_glyf.resize(new_glyf.len().div_ceil(4) * 4, 0);
        }
    }
    new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());
    let mut tables: Vec<([u8; 4], Vec<u8>)> = Vec::new();
    for (tag, data) in &sfnt.tables {
        match tag {
            b"head" => {
                let mut h = data.to_vec();
                h.get_mut(8..12)?.fill(0);
                h.get_mut(50..52)?.copy_from_slice(&1u16.to_be_bytes());
                tables.push((*tag, h));
            }
            b"loca" => tables.push((*tag, new_loca.clone())),
            b"glyf" => tables.push((*tag, new_glyf.clone())),
            b"hhea" | b"maxp" | b"hmtx" | b"cvt " | b"fpgm" | b"prep" => {
                tables.push((*tag, data.to_vec()));
            }
            _ => {}
        }
    }
    Some(build_sfnt(tables, false))
}

fn whole_font(sfnt: &Sfnt<'_>, cff: bool) -> Vec<u8> {
    let tables = sfnt
        .tables
        .iter()
        .map(|(t, d)| {
            let mut data = d.to_vec();
            if t == b"head" && data.len() >= 12 {
                data[8..12].fill(0);
            }
            (*t, data)
        })
        .collect();
    build_sfnt(tables, cff)
}

/// Private-use code points that Symbol-style fonts use for common bullets.
fn normalise_unicode(c: char) -> char {
    match c as u32 {
        0xF0B7 => '\u{2022}',
        0xF0A7 => '\u{25AA}',
        0xF0D8 => '\u{27A2}',
        0xF0FC => '\u{2713}',
        0xF076 => '\u{2756}',
        _ => c,
    }
}

fn is_private(c: char) -> bool {
    (0xE000..=0xF8FF).contains(&(c as u32))
}

/// Glyph id to character; a non-private, lower code point wins when several map to a glyph.
pub fn reverse_cmap(font: &FontRef<'_>) -> BTreeMap<u32, char> {
    let mut out: BTreeMap<u32, char> = BTreeMap::new();
    for (cp, gid) in font.charmap().mappings() {
        let Some(c) = char::from_u32(cp) else {
            continue;
        };
        let gid = gid.to_u32();
        let better = match out.get(&gid) {
            None => true,
            Some(old) => (is_private(*old), *old as u32) > (is_private(c), c as u32),
        };
        if better {
            out.insert(gid, c);
        }
    }
    out.into_iter()
        .map(|(g, c)| (g, normalise_unicode(c)))
        .collect()
}

/// The text a character stands for in running text: ligature presentation forms are spelled out.
pub fn expand(c: char) -> String {
    match c as u32 {
        0xFB00 => "ff".into(),
        0xFB01 => "fi".into(),
        0xFB02 => "fl".into(),
        0xFB03 => "ffi".into(),
        0xFB04 => "ffl".into(),
        0xFB05 | 0xFB06 => "st".into(),
        _ => c.to_string(),
    }
}

/// Glyph id to character for `face`.
pub fn reverse_map(face: &FontFace) -> BTreeMap<u32, char> {
    FontRef::from_index(face.bytes(), face.index())
        .map(|f| reverse_cmap(&f))
        .unwrap_or_default()
}

fn to_unicode_cmap(
    map: &BTreeMap<u32, char>,
    extra: &BTreeMap<u32, String>,
    used: &BTreeSet<u32>,
) -> String {
    let entries: Vec<(u32, String)> = used
        .iter()
        .filter_map(|g| {
            extra
                .get(g)
                .cloned()
                .or_else(|| map.get(g).map(|c| expand(*c)))
                .map(|t| (*g, t))
        })
        .collect();
    let mut s = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    for chunk in entries.chunks(100) {
        s.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (g, c) in chunk {
            let hex: String = c.encode_utf16().map(|u| format!("{u:04X}")).collect();
            s.push_str(&format!("<{:04X}> <{hex}>\n", g & 0xFFFF));
        }
        s.push_str("endbfchar\n");
    }
    s.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    s
}

fn ps_name(face: &FontFace, tag: &str) -> String {
    let mut base: String = face
        .family()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    if base.is_empty() {
        base.push_str("Font");
    }
    if face.is_bold() {
        base.push_str("-Bold");
    }
    if face.is_italic() {
        base.push_str("-Italic");
    }
    format!("{tag}+{base}")
}

/// Embeds `face` for the glyph ids `used` and returns the Type0 font object and the drawing
/// data for the content stream.
pub fn embed(
    pdf: &mut Pdf,
    face: &FontFace,
    used: &BTreeSet<u32>,
    extra: &BTreeMap<u32, String>,
    resource: String,
    warnings: &mut Vec<String>,
    cid_set: bool,
) -> Result<(u32, FaceDraw), PdfError> {
    let name = face.family().to_owned();
    let font = FontRef::from_index(face.bytes(), face.index())
        .map_err(|e| PdfError::Font(format!("{name}: {e}")))?;
    let sfnt = Sfnt::parse(face.bytes(), face.index())
        .ok_or_else(|| PdfError::Font(format!("{name}: unreadable table directory")))?;
    let os2 = font.os2().ok();
    let fs_type = os2.as_ref().map_or(0, |t| t.fs_type());
    if fs_type & 0x000E == 0x0002 {
        return Err(PdfError::FontEmbeddingForbidden(name));
    }
    let upem = f64::from(face.units_per_em());
    let scale = 1000.0 / upem;

    let metrics = font.glyph_metrics(
        skrifa::instance::Size::unscaled(),
        skrifa::instance::LocationRef::default(),
    );
    let mut widths = BTreeMap::new();
    let mut gids = used.clone();
    gids.insert(0);
    for &g in &gids {
        let adv = metrics
            .advance_width(skrifa::GlyphId::new(g))
            .map_or(0.0, f64::from);
        widths.insert(g, (adv * scale).round() as i64);
    }

    let tag: String = {
        let mut bytes = name.as_bytes().to_vec();
        for g in &gids {
            bytes.extend_from_slice(&g.to_be_bytes());
        }
        digest(&bytes)[..6]
            .iter()
            .map(|b| char::from(b'A' + b % 26))
            .collect()
    };
    let ps = ps_name(face, &tag);

    let is_cff = sfnt.get(b"CFF ").is_some() && sfnt.get(b"glyf").is_none();
    let subset_cff = if is_cff && fs_type & 0x0100 == 0 {
        sfnt.get(b"CFF ").and_then(|t| crate::cff::subset(t, &gids))
    } else {
        None
    };
    let (file_id, descendant_subtype) = if let Some(data) = subset_cff {
        // A bare CFF, valid from PDF 1.3 on (an OpenType wrapper needs 1.6).
        let id = pdf.add_stream("/Subtype /CIDFontType0C", &data, true);
        (id, "CIDFontType0")
    } else if is_cff {
        warnings.push(format!("{name}: CFF font embedded whole (no subsetting)"));
        let data = whole_font(&sfnt, true);
        let id = pdf.add_stream(
            &format!("/Subtype /OpenType /Length1 {}", data.len()),
            &data,
            true,
        );
        (id, "CIDFontType0")
    } else {
        let no_subset = fs_type & 0x0100 != 0;
        let data = if no_subset {
            Some(whole_font(&sfnt, false))
        } else {
            subset_truetype(&sfnt, &gids)
        }
        .ok_or_else(|| PdfError::Font(format!("{name}: cannot subset glyf table")))?;
        let id = pdf.add_stream(&format!("/Length1 {}", data.len()), &data, true);
        (id, "CIDFontType2")
    };

    let head = font
        .head()
        .map_err(|e| PdfError::Font(format!("{name}: {e}")))?;
    let hhea = font.hhea().ok();
    let post = font.post().ok();
    let ascent = hhea
        .as_ref()
        .map_or(upem * 0.8, |h| f64::from(h.ascender().to_i16()));
    let descent = hhea
        .as_ref()
        .map_or(-upem * 0.2, |h| f64::from(h.descender().to_i16()));
    let cap = os2
        .as_ref()
        .and_then(|t| t.s_cap_height())
        .map_or(ascent, f64::from);
    let angle = post.as_ref().map_or(0.0, |p| p.italic_angle().to_f64());
    let flags = 32 | if face.is_italic() { 64 } else { 0 };
    let bbox = [
        f64::from(head.x_min()) * scale,
        f64::from(head.y_min()) * scale,
        f64::from(head.x_max()) * scale,
        f64::from(head.y_max()) * scale,
    ];
    let file_key = if is_cff { "FontFile3" } else { "FontFile2" };
    // CIDSet (PDF/A-1 needs it, and only there is it exact): one bit per CID in the subset, CID 0
    // the most significant bit of the first byte.
    let cid_set_entry = if cid_set {
        let max_gid = gids.iter().next_back().copied().unwrap_or(0) as usize;
        let mut bits = vec![0u8; max_gid / 8 + 1];
        for &g in &gids {
            bits[g as usize / 8] |= 0x80 >> (g % 8);
        }
        format!(" /CIDSet {} 0 R", pdf.add_stream("", &bits, true))
    } else {
        String::new()
    };
    let desc = pdf.add(format!(
        "<< /Type /FontDescriptor /FontName /{ps} /Flags {flags} /FontBBox [{} {} {} {}] /ItalicAngle {} /Ascent {} /Descent {} /CapHeight {} /StemV {} /{file_key} {file_id} 0 R{cid_set_entry} >>",
        num(bbox[0], 0),
        num(bbox[1], 0),
        num(bbox[2], 0),
        num(bbox[3], 0),
        num(angle, 2),
        num(ascent * scale, 0),
        num(descent * scale, 0),
        num(cap * scale, 0),
        if face.is_bold() { 140 } else { 80 },
    ));

    let mut w = String::from("[");
    for (g, width) in &widths {
        w.push_str(&format!("{g} [{width}] "));
    }
    w.push(']');
    let map_line = if is_cff {
        ""
    } else {
        " /CIDToGIDMap /Identity"
    };
    let cid = pdf.add(format!(
        "<< /Type /Font /Subtype /{descendant_subtype} /BaseFont /{ps} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {desc} 0 R /DW 1000 /W {w}{map_line} >>"
    ));

    let cmap = to_unicode_cmap(&reverse_cmap(&font), extra, &gids);
    let tu = pdf.add_stream("", cmap.as_bytes(), true);
    let type0 = pdf.add(format!(
        "<< /Type /Font /Subtype /Type0 /BaseFont /{ps} /Encoding /Identity-H /DescendantFonts [{cid} 0 R] /ToUnicode {tu} 0 R >>"
    ));

    let (u_pos, u_thick) = post.as_ref().map_or((-0.1 * upem, 0.05 * upem), |p| {
        (
            f64::from(p.underline_position().to_i16()),
            f64::from(p.underline_thickness().to_i16()),
        )
    });
    let (s_pos, s_thick) = os2.as_ref().map_or((0.3 * upem, 0.05 * upem), |t| {
        (
            f64::from(t.y_strikeout_position()),
            f64::from(t.y_strikeout_size()),
        )
    });
    Ok((
        type0,
        FaceDraw {
            resource,
            widths,
            underline_pos: u_pos / upem,
            underline_thickness: (u_thick / upem).max(0.02),
            strike_pos: s_pos / upem,
            strike_thickness: (s_thick / upem).max(0.02),
        },
    ))
}
