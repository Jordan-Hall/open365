//! CFF subsetting: a standalone CFF table with the outlines of the used glyphs only.
//!
//! Glyph ids stay as they are (an unused glyph keeps its slot with a bare `endchar`), so the PDF
//! can keep using glyph ids as CIDs. Subroutines are kept whole (pruning them needs a charstring
//! interpreter; they are small next to the outlines). A CID-keyed font gets an identity charset so
//! that its CIDs are its glyph ids too. CFF2 and anything unreadable returns `None` and the caller
//! embeds the font whole.

use std::collections::BTreeSet;

fn be16(d: &[u8], o: usize) -> Option<usize> {
    Some(usize::from(u16::from_be_bytes(
        d.get(o..o + 2)?.try_into().ok()?,
    )))
}

/// The items of the INDEX at `at` as byte ranges, and the offset just past it.
fn index(d: &[u8], at: usize) -> Option<(Vec<(usize, usize)>, usize)> {
    let count = be16(d, at)?;
    if count == 0 {
        return Some((Vec::new(), at + 2));
    }
    let off_size = usize::from(*d.get(at + 2)?);
    if !(1..=4).contains(&off_size) {
        return None;
    }
    let read = |i: usize| -> Option<usize> {
        let p = at + 3 + i * off_size;
        let b = d.get(p..p + off_size)?;
        Some(b.iter().fold(0usize, |a, x| a << 8 | usize::from(*x)))
    };
    let base = at + 3 + (count + 1) * off_size - 1;
    let mut items = Vec::with_capacity(count);
    for i in 0..count {
        let (a, b) = (read(i)?, read(i + 1)?);
        if b < a || base + b > d.len() {
            return None;
        }
        items.push((base + a, base + b));
    }
    Some((items, base + read(count)?))
}

fn write_index(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(items.len() as u16).to_be_bytes());
    if items.is_empty() {
        return out;
    }
    let total: usize = items.iter().map(Vec::len).sum::<usize>() + 1;
    let off_size = match total {
        0..=0xFF => 1,
        0x100..=0xFFFF => 2,
        0x1_0000..=0xFF_FFFF => 3,
        _ => 4,
    };
    out.push(off_size as u8);
    let mut off = 1usize;
    let put = |out: &mut Vec<u8>, v: usize| {
        out.extend_from_slice(&(v as u32).to_be_bytes()[4 - off_size..]);
    };
    put(&mut out, off);
    for it in items {
        off += it.len();
        put(&mut out, off);
    }
    for it in items {
        out.extend_from_slice(it);
    }
    out
}

/// One DICT entry: where its bytes are and its integer operands.
struct Entry {
    raw: (usize, usize),
    op: u16,
    args: Vec<i32>,
}

fn parse_dict(d: &[u8]) -> Option<Vec<Entry>> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut start = 0;
    let mut args: Vec<i32> = Vec::new();
    while i < d.len() {
        let b = d[i];
        match b {
            0..=21 => {
                let op = if b == 12 {
                    i += 1;
                    0x0C00 | u16::from(*d.get(i)?)
                } else {
                    u16::from(b)
                };
                i += 1;
                out.push(Entry {
                    raw: (start, i),
                    op,
                    args: std::mem::take(&mut args),
                });
                start = i;
            }
            28 => {
                let v = i16::from_be_bytes(d.get(i + 1..i + 3)?.try_into().ok()?);
                args.push(i32::from(v));
                i += 3;
            }
            29 => {
                args.push(i32::from_be_bytes(d.get(i + 1..i + 5)?.try_into().ok()?));
                i += 5;
            }
            30 => {
                // A real: nibbles up to 0xF. Only operands the font keeps verbatim are reals.
                i += 1;
                loop {
                    let n = *d.get(i)?;
                    i += 1;
                    if n & 0x0F == 0x0F || n >> 4 == 0x0F {
                        break;
                    }
                }
                args.push(0);
            }
            32..=246 => {
                args.push(i32::from(b) - 139);
                i += 1;
            }
            247..=250 => {
                args.push((i32::from(b) - 247) * 256 + i32::from(*d.get(i + 1)?) + 108);
                i += 2;
            }
            251..=254 => {
                args.push(-(i32::from(b) - 251) * 256 - i32::from(*d.get(i + 1)?) - 108);
                i += 2;
            }
            _ => return None,
        }
    }
    Some(out)
}

/// `value` as a five-byte integer operand, so that a DICT's size does not depend on offsets.
fn int5(v: usize) -> [u8; 5] {
    let b = (v as u32).to_be_bytes();
    [29, b[0], b[1], b[2], b[3]]
}

const CHARSET: u16 = 15;
const ENCODING: u16 = 16;
const CHARSTRINGS: u16 = 17;
const PRIVATE: u16 = 18;
const SUBRS: u16 = 19;
const FD_ARRAY: u16 = 0x0C24;
const FD_SELECT: u16 = 0x0C25;
const ROS: u16 = 0x0C1E;

/// Re-encodes a DICT, replacing the entries in `replace` (operator, operands as five-byte
/// integers) and dropping those in `drop`; everything else is copied byte for byte.
fn rewrite_dict(
    d: &[u8],
    entries: &[Entry],
    replace: &[(u16, Vec<usize>)],
    drop: &[u16],
) -> Vec<u8> {
    let mut out = Vec::new();
    let op_bytes = |op: u16| -> Vec<u8> {
        if op >= 0x0C00 {
            vec![12, (op & 0xFF) as u8]
        } else {
            vec![op as u8]
        }
    };
    for e in entries {
        if drop.contains(&e.op) {
            continue;
        }
        if replace.iter().any(|(op, _)| *op == e.op) {
            continue;
        }
        out.extend_from_slice(&d[e.raw.0..e.raw.1]);
    }
    for (op, args) in replace {
        for a in args {
            out.extend_from_slice(&int5(*a));
        }
        out.extend(op_bytes(*op));
    }
    out
}

/// Bytes of the private block (`size` at `off`) with its local subroutines, or `None`.
fn private_block(d: &[u8], size: usize, off: usize) -> Option<&[u8]> {
    let dict = d.get(off..off.checked_add(size)?)?;
    let mut end = off + size;
    if let Some(e) = parse_dict(dict)?.iter().find(|e| e.op == SUBRS) {
        let rel = usize::try_from(*e.args.first()?).ok()?;
        let (_, stop) = index(d, off + rel)?;
        end = end.max(stop);
    }
    d.get(off..end)
}

fn charset_len(d: &[u8], at: usize, n_glyphs: usize) -> Option<usize> {
    let format = *d.get(at)?;
    let mut have = 1; // .notdef
    let mut p = at + 1;
    match format {
        0 => p += 2 * (n_glyphs - 1),
        1 | 2 => {
            while have < n_glyphs {
                let left = if format == 1 {
                    let l = usize::from(*d.get(p + 2)?);
                    p += 3;
                    l
                } else {
                    let l = be16(d, p + 2)?;
                    p += 4;
                    l
                };
                have += left + 1;
            }
        }
        _ => return None,
    }
    Some(p - at)
}

fn fd_select_len(d: &[u8], at: usize, n_glyphs: usize) -> Option<usize> {
    match *d.get(at)? {
        0 => Some(1 + n_glyphs),
        3 => {
            let ranges = be16(d, at + 1)?;
            Some(1 + 2 + ranges * 3 + 2)
        }
        _ => None,
    }
}

/// A CFF with only the outlines of `keep` (and glyph 0); `None` when the font is not a plain CFF.
pub fn subset(cff: &[u8], keep: &BTreeSet<u32>) -> Option<Vec<u8>> {
    if *cff.first()? != 1 {
        return None;
    }
    let hdr = usize::from(*cff.get(2)?);
    let (_, after_names) = index(cff, hdr)?;
    let (tops, after_top) = index(cff, after_names)?;
    let (_, after_strings) = index(cff, after_top)?;
    let (_, after_gsubrs) = index(cff, after_strings)?;
    let top_range = *tops.first()?;
    let top = &cff[top_range.0..top_range.1];
    let top_entries = parse_dict(top)?;
    let find = |op: u16| top_entries.iter().find(|e| e.op == op);
    let (cs_items, _) = index(
        cff,
        usize::try_from(*find(CHARSTRINGS)?.args.first()?).ok()?,
    )?;
    let n_glyphs = cs_items.len();
    let is_cid = find(ROS).is_some();

    // The charstrings: unused glyphs shrink to `endchar`.
    let new_cs: Vec<Vec<u8>> = cs_items
        .iter()
        .enumerate()
        .map(|(g, (a, b))| {
            if g == 0 || keep.contains(&(g as u32)) {
                cff[*a..*b].to_vec()
            } else {
                vec![14]
            }
        })
        .collect();
    let cs_index = write_index(&new_cs);

    // The charset.
    let charset: Vec<u8> = if is_cid {
        if n_glyphs >= 2 {
            let mut c = vec![2];
            c.extend_from_slice(&1u16.to_be_bytes());
            c.extend_from_slice(&((n_glyphs - 2) as u16).to_be_bytes());
            c
        } else {
            vec![0]
        }
    } else {
        match find(CHARSET)
            .and_then(|e| e.args.first())
            .copied()
            .unwrap_or(0)
        {
            0..=2 => Vec::new(),
            off => {
                let off = usize::try_from(off).ok()?;
                cff.get(off..off + charset_len(cff, off, n_glyphs)?)?
                    .to_vec()
            }
        }
    };
    let predefined_charset = !is_cid && charset.is_empty();
    let charset_op = if predefined_charset {
        find(CHARSET)
            .and_then(|e| e.args.first())
            .copied()
            .unwrap_or(0)
    } else {
        0
    } as usize;

    let fd_select: Vec<u8> = match find(FD_SELECT) {
        Some(e) => {
            let off = usize::try_from(*e.args.first()?).ok()?;
            cff.get(off..off + fd_select_len(cff, off, n_glyphs)?)?
                .to_vec()
        }
        None => Vec::new(),
    };

    // Private data: one block, or one per font dict of a CID-keyed font.
    let mut blocks: Vec<Vec<u8>> = Vec::new();
    let mut fd_dicts: Vec<(Vec<u8>, Vec<Entry>)> = Vec::new();
    let mut fd_private: Vec<Option<usize>> = Vec::new(); // block index per font dict
    let mut main_private: Option<usize> = None;
    if let Some(fda) = find(FD_ARRAY) {
        let (fds, _) = index(cff, usize::try_from(*fda.args.first()?).ok()?)?;
        for (a, b) in fds {
            let raw = cff[a..b].to_vec();
            let entries = parse_dict(&raw)?;
            let slot = match entries.iter().find(|e| e.op == PRIVATE) {
                Some(p) => {
                    let (size, off) = (
                        usize::try_from(*p.args.first()?).ok()?,
                        usize::try_from(*p.args.get(1)?).ok()?,
                    );
                    blocks.push(private_block(cff, size, off)?.to_vec());
                    Some(blocks.len() - 1)
                }
                None => None,
            };
            fd_private.push(slot);
            // The private size of each font dict is rewritten with its block below.
            fd_dicts.push((raw, entries));
        }
    } else if let Some(p) = find(PRIVATE) {
        let (size, off) = (
            usize::try_from(*p.args.first()?).ok()?,
            usize::try_from(*p.args.get(1)?).ok()?,
        );
        blocks.push(private_block(cff, size, off)?.to_vec());
        main_private = Some(0);
    }

    // Layout: header, name INDEX, top DICT INDEX, strings, global subrs, then our parts.
    let head_names_strings = cff[..after_names].to_vec();
    let strings_and_gsubrs = &cff[after_top..after_gsubrs];

    let build_top = |charset_off: usize,
                     fd_select_off: usize,
                     cs_off: usize,
                     fd_array_off: usize,
                     private: Option<(usize, usize)>|
     -> Vec<u8> {
        let mut replace: Vec<(u16, Vec<usize>)> = vec![(CHARSTRINGS, vec![cs_off])];
        if is_cid || !predefined_charset {
            replace.push((CHARSET, vec![charset_off]));
        } else {
            replace.push((CHARSET, vec![charset_op]));
        }
        if !fd_select.is_empty() {
            replace.push((FD_SELECT, vec![fd_select_off]));
        }
        if find(FD_ARRAY).is_some() {
            replace.push((FD_ARRAY, vec![fd_array_off]));
        }
        if let Some((size, off)) = private {
            replace.push((PRIVATE, vec![size, off]));
        }
        // A custom encoding is of no use to a PDF that addresses glyphs by id.
        let drop = [ENCODING];
        rewrite_dict(top, &top_entries, &replace, &drop)
    };
    let top_len =
        |private: Option<(usize, usize)>| write_index(&[build_top(0, 0, 0, 0, private)]).len();
    let private_slot = main_private.map(|b| (blocks[b].len(), 0usize));
    let top_index_len = top_len(private_slot);

    let mut pos = head_names_strings.len() + top_index_len + strings_and_gsubrs.len();
    let charset_off = pos;
    if !predefined_charset {
        pos += charset.len();
    }
    let fd_select_off = pos;
    pos += fd_select.len();
    let cs_off = pos;
    pos += cs_index.len();

    // Font dict array (CID) with its private blocks after it.
    let mut fd_array_bytes = Vec::new();
    let fd_array_off = pos;
    let mut block_offsets: Vec<usize> = vec![0; blocks.len()];
    if find(FD_ARRAY).is_some() {
        // First pass for the array size (offsets are five-byte, so sizes are final).
        let trial: Vec<Vec<u8>> = fd_dicts
            .iter()
            .zip(&fd_private)
            .map(|((raw, entries), slot)| {
                let rep: Vec<(u16, Vec<usize>)> = slot
                    .map(|b| vec![(PRIVATE, vec![blocks[b].len(), 0])])
                    .unwrap_or_default();
                rewrite_dict(raw, entries, &rep, &[])
            })
            .collect();
        let array_len = write_index(&trial).len();
        let mut at = fd_array_off + array_len;
        for (i, b) in blocks.iter().enumerate() {
            block_offsets[i] = at;
            at += b.len();
        }
        let real: Vec<Vec<u8>> = fd_dicts
            .iter()
            .zip(&fd_private)
            .map(|((raw, entries), slot)| {
                let rep: Vec<(u16, Vec<usize>)> = slot
                    .map(|b| vec![(PRIVATE, vec![blocks[b].len(), block_offsets[b]])])
                    .unwrap_or_default();
                rewrite_dict(raw, entries, &rep, &[])
            })
            .collect();
        fd_array_bytes = write_index(&real);
    } else if let Some(b) = main_private {
        block_offsets[b] = pos;
    }

    let private = main_private.map(|b| (blocks[b].len(), block_offsets[b]));
    let top_bytes = write_index(&[build_top(
        charset_off,
        fd_select_off,
        cs_off,
        fd_array_off,
        private,
    )]);
    if top_bytes.len() != top_index_len {
        return None;
    }

    let mut out = head_names_strings;
    out.extend_from_slice(&top_bytes);
    out.extend_from_slice(strings_and_gsubrs);
    if !predefined_charset {
        out.extend_from_slice(&charset);
    }
    out.extend_from_slice(&fd_select);
    out.extend_from_slice(&cs_index);
    if !fd_array_bytes.is_empty() {
        out.extend_from_slice(&fd_array_bytes);
        for b in &blocks {
            out.extend_from_slice(b);
        }
    } else if let Some(b) = main_private {
        out.extend_from_slice(&blocks[b]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal name-keyed CFF: four glyphs with charstrings of distinct lengths, a private
    /// dict with local subroutines.
    fn tiny() -> Vec<u8> {
        let name = write_index(&[b"Tiny".to_vec()]);
        let strings = write_index(&[]);
        let gsubrs = write_index(&[]);
        let charstrings = write_index(&[
            vec![14],
            vec![139, 139, 21, 14],
            vec![139, 139, 139, 139, 5, 14],
            vec![139, 139, 139, 139, 139, 139, 8, 14],
        ]);
        let lsubrs = write_index(&[vec![11]]);
        // Private DICT: Subrs offset (relative to the private dict) = its own size (5 + 1).
        let mut private = Vec::new();
        private.extend_from_slice(&int5(6));
        private.push(SUBRS as u8);
        assert_eq!(private.len(), 6);
        let top_dict = |cs: usize, pr: usize| {
            let mut t = Vec::new();
            t.extend_from_slice(&int5(cs));
            t.push(CHARSTRINGS as u8);
            t.extend_from_slice(&int5(private.len()));
            t.extend_from_slice(&int5(pr));
            t.push(PRIVATE as u8);
            t
        };
        let top_len = write_index(&[top_dict(0, 0)]).len();
        let header = [1u8, 0, 4, 1];
        let cs_off = header.len() + name.len() + top_len + strings.len() + gsubrs.len();
        let pr_off = cs_off + charstrings.len();
        let mut out = header.to_vec();
        out.extend_from_slice(&name);
        out.extend_from_slice(&write_index(&[top_dict(cs_off, pr_off)]));
        out.extend_from_slice(&strings);
        out.extend_from_slice(&gsubrs);
        out.extend_from_slice(&charstrings);
        out.extend_from_slice(&private);
        out.extend_from_slice(&lsubrs);
        out
    }

    fn glyph_lengths(cff: &[u8]) -> Vec<usize> {
        let hdr = usize::from(cff[2]);
        let (_, a) = index(cff, hdr).expect("names");
        let (tops, _) = index(cff, a).expect("top");
        let top = parse_dict(&cff[tops[0].0..tops[0].1]).expect("dict");
        let cs = top.iter().find(|e| e.op == CHARSTRINGS).expect("cs");
        let (items, _) = index(cff, cs.args[0] as usize).expect("charstrings");
        items.iter().map(|(a, b)| b - a).collect()
    }

    #[test]
    fn unused_glyphs_shrink_and_ids_stay() {
        let font = tiny();
        assert_eq!(glyph_lengths(&font), vec![1, 4, 6, 8]);
        let keep: BTreeSet<u32> = [2].into_iter().collect();
        let out = subset(&font, &keep).expect("subset");
        assert_eq!(glyph_lengths(&out), vec![1, 1, 6, 1]);
    }

    #[test]
    fn private_dict_and_local_subrs_survive() {
        let font = tiny();
        let out = subset(&font, &BTreeSet::new()).expect("subset");
        let hdr = usize::from(out[2]);
        let (_, a) = index(&out, hdr).expect("names");
        let (tops, _) = index(&out, a).expect("top");
        let top = parse_dict(&out[tops[0].0..tops[0].1]).expect("dict");
        let p = top.iter().find(|e| e.op == PRIVATE).expect("private");
        let (size, off) = (p.args[0] as usize, p.args[1] as usize);
        let block = private_block(&out, size, off).expect("block");
        // The private dict (6 bytes) and an INDEX with one local subroutine.
        assert_eq!(block.len(), 6 + write_index(&[vec![11]]).len());
    }

    #[test]
    fn garbage_is_refused() {
        assert!(subset(&[1, 0, 4, 1, 0xFF, 0xFF], &BTreeSet::new()).is_none());
        assert!(subset(&[2, 0, 5, 0], &BTreeSet::new()).is_none());
    }
}
