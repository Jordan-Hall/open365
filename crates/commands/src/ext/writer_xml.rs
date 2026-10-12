//! Tree edits of a Word document's main part, for features the document model keeps as raw
//! XML or cannot yet change itself (run formatting beyond bold, lists, hyperlinks, pictures,
//! headers). The document is saved, its package patched and the result opened again; the caller
//! keeps the previous bytes for undo.
//!
//! The tree helpers mirror how `open365-docx` counts: a `w:t` with text only counts its
//! characters, `w:tab` and `w:br` count one, hyperlinks, insertions and the other inline groups
//! count their runs, anything else counts none. Paragraph indices run in document order through
//! table cells and block content controls.

use open365_docx::{Attr, Document, ElementHead, ParsedXml, RawXml, XmlElement, parse_xml};
use open365_opc::{Limits, Package, PartName};

use crate::error::CommandError;

pub(crate) const R_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

pub(crate) fn fail(message: impl Into<String>) -> CommandError {
    CommandError::Format(message.into())
}

fn opc(e: impl std::fmt::Display) -> CommandError {
    CommandError::Format(e.to_string())
}

/// The package and the parsed main part while an edit runs.
#[derive(Debug)]
pub(crate) struct Main {
    pub pkg: Package,
    pub name: PartName,
    pub xml: ParsedXml,
}

/// Saves `doc`, lets `edit` change the main part and the package, and opens the result in place
/// of `doc`. Track-change recording carries over. `doc` is unchanged when `edit` fails.
pub(crate) fn rewrite(
    doc: &mut Document,
    edit: impl FnOnce(&mut Main) -> Result<(), CommandError>,
) -> Result<(), CommandError> {
    let track = doc.track_changes().cloned();
    let name = doc.main_part().clone();
    let bytes = doc.save()?;
    let limits = Limits::default();
    let pkg = Package::read(&bytes, &limits).map_err(opc)?;
    let part = pkg
        .part(name.as_str())
        .ok_or_else(|| fail("the document has no main part"))?;
    let xml = parse_xml(part.data())?;
    let mut main = Main { pkg, name, xml };
    edit(&mut main)?;
    let data = main.xml.to_bytes();
    main.pkg.set_part(main.name.clone(), data);
    let out = main.pkg.write().map_err(opc)?;
    let mut fresh = Document::open(&out, &limits)?;
    fresh.set_track_changes(track);
    *doc = fresh;
    Ok(())
}

// ---- elements -------------------------------------------------------------------------------

/// An empty `w:` element named like `like`'s prefix.
pub(crate) fn w(like: &ElementHead, local: &str) -> XmlElement {
    like.sibling(local).into_empty()
}

/// `w:local w:val="val"`.
pub(crate) fn w_val(like: &ElementHead, local: &str, val: &str) -> XmlElement {
    let mut e = w(like, local);
    e.head.set_attr_w("val", val);
    e
}

pub(crate) fn text_el(like: &ElementHead, local: &str, text: &str) -> XmlElement {
    let mut e = w(like, local);
    e.head.attrs.push(Attr {
        name: "xml:space".to_owned(),
        ns: Some(XML_NS.into()),
        value: "preserve".to_owned(),
    });
    if !text.is_empty() {
        e.children.push(RawXml::Text(text.to_owned()));
    }
    e
}

pub(crate) fn child<'a>(e: &'a XmlElement, local: &str) -> Option<&'a XmlElement> {
    e.children.iter().find_map(|c| match c {
        RawXml::Element(x) if x.head.is_w(local) => Some(x),
        _ => None,
    })
}

pub(crate) fn child_mut<'a>(e: &'a mut XmlElement, local: &str) -> Option<&'a mut XmlElement> {
    e.children.iter_mut().find_map(|c| match c {
        RawXml::Element(x) if x.head.is_w(local) => Some(x),
        _ => None,
    })
}

pub(crate) fn remove_child(e: &mut XmlElement, local: &str) -> bool {
    let before = e.children.len();
    e.children
        .retain(|c| !matches!(c, RawXml::Element(x) if x.head.is_w(local)));
    e.children.len() != before
}

/// Child order of `w:rPr` (§17.3.2).
pub(crate) const RPR_ORDER: &[&str] = &[
    "rStyle",
    "rFonts",
    "b",
    "bCs",
    "i",
    "iCs",
    "caps",
    "smallCaps",
    "strike",
    "dstrike",
    "outline",
    "shadow",
    "emboss",
    "imprint",
    "noProof",
    "snapToGrid",
    "vanish",
    "webHidden",
    "color",
    "spacing",
    "w",
    "kern",
    "position",
    "sz",
    "szCs",
    "highlight",
    "u",
    "effect",
    "bdr",
    "shd",
    "fitText",
    "vertAlign",
    "rtl",
    "cs",
    "em",
    "lang",
    "eastAsianLayout",
    "specVanish",
    "oMath",
];

/// Child order of `w:pPr` (§17.3.1).
pub(crate) const PPR_ORDER: &[&str] = &[
    "pStyle",
    "keepNext",
    "keepLines",
    "pageBreakBefore",
    "framePr",
    "widowControl",
    "numPr",
    "suppressLineNumbers",
    "pBdr",
    "shd",
    "tabs",
    "suppressAutoHyphens",
    "kinsoku",
    "wordWrap",
    "overflowPunct",
    "topLinePunct",
    "autoSpaceDE",
    "autoSpaceDN",
    "bidi",
    "adjustRightInd",
    "snapToGrid",
    "spacing",
    "ind",
    "contextualSpacing",
    "mirrorIndents",
    "suppressOverlap",
    "jc",
    "textDirection",
    "textAlignment",
    "textboxTightWrap",
    "outlineLvl",
    "divId",
    "cnfStyle",
    "rPr",
    "sectPr",
    "pPrChange",
];

/// Puts `new` into `parent`, replacing a child of the same name, at the place the schema order
/// `order` gives it.
pub(crate) fn set_ordered(parent: &mut XmlElement, order: &[&str], new: XmlElement) {
    let local = new.head.local_name().to_owned();
    remove_child(parent, &local);
    let rank = |name: &str| order.iter().position(|o| *o == name).unwrap_or(order.len());
    let mine = rank(&local);
    let at = parent
        .children
        .iter()
        .position(|c| matches!(c, RawXml::Element(x) if rank(x.head.local_name()) > mine))
        .unwrap_or(parent.children.len());
    parent.children.insert(at, RawXml::Element(new));
}

/// The paragraph's `w:pPr`, created first when missing.
pub(crate) fn ppr_mut(p: &mut XmlElement) -> &mut XmlElement {
    let has = matches!(p.children.first(), Some(RawXml::Element(e)) if e.head.is_w("pPr"));
    if !has {
        let new = w(&p.head, "pPr");
        p.children.insert(0, RawXml::Element(new));
    }
    match p.children.first_mut() {
        Some(RawXml::Element(e)) => e,
        _ => unreachable!("pPr was just inserted"),
    }
}

pub(crate) fn ppr(p: &XmlElement) -> Option<&XmlElement> {
    match p.children.first() {
        Some(RawXml::Element(e)) if e.head.is_w("pPr") => Some(e),
        _ => None,
    }
}

/// True when the paragraph's properties end a section (`w:sectPr`).
pub(crate) fn ppr_has_sect(p: &XmlElement) -> bool {
    ppr(p).is_some_and(|pp| child(pp, "sectPr").is_some())
}

/// The run's `w:rPr`, created first when missing.
pub(crate) fn rpr_mut(r: &mut XmlElement) -> &mut XmlElement {
    let has = matches!(r.children.first(), Some(RawXml::Element(e)) if e.head.is_w("rPr"));
    if !has {
        let new = w(&r.head, "rPr");
        r.children.insert(0, RawXml::Element(new));
    }
    match r.children.first_mut() {
        Some(RawXml::Element(e)) => e,
        _ => unreachable!("rPr was just inserted"),
    }
}

/// Drops an empty `w:rPr` or `w:pPr` left behind by removals.
pub(crate) fn drop_empty_props(r: &mut XmlElement) {
    if matches!(r.children.first(), Some(RawXml::Element(e)) if (e.head.is_w("rPr") || e.head.is_w("pPr")) && e.children.is_empty())
    {
        r.children.remove(0);
    }
}

// ---- paragraphs -----------------------------------------------------------------------------

/// Child indices from the document element down to a container of blocks.
pub(crate) type Path = Vec<usize>;

fn is_element(c: &RawXml, local: &str) -> bool {
    matches!(c, RawXml::Element(e) if e.head.is_w(local))
}

fn walk(
    children: &[RawXml],
    base: &mut Path,
    seen: &mut usize,
    target: usize,
) -> Option<(Path, usize)> {
    for (i, c) in children.iter().enumerate() {
        let RawXml::Element(e) = c else { continue };
        if e.head.is_w("p") {
            if *seen == target {
                return Some((base.clone(), i));
            }
            *seen += 1;
        } else if e.head.is_w("tbl") {
            for (ri, row) in e.children.iter().enumerate() {
                let RawXml::Element(row) = row else { continue };
                if !row.head.is_w("tr") {
                    continue;
                }
                for (ci, cell) in row.children.iter().enumerate() {
                    let RawXml::Element(cell) = cell else {
                        continue;
                    };
                    if !cell.head.is_w("tc") {
                        continue;
                    }
                    base.extend([i, ri, ci]);
                    let found = walk(&cell.children, base, seen, target);
                    base.truncate(base.len() - 3);
                    if found.is_some() {
                        return found;
                    }
                }
            }
        } else if e.head.is_w("sdt") {
            for (ci, content) in e.children.iter().enumerate() {
                let RawXml::Element(content) = content else {
                    continue;
                };
                if !content.head.is_w("sdtContent") {
                    continue;
                }
                base.extend([i, ci]);
                let found = walk(&content.children, base, seen, target);
                base.truncate(base.len() - 2);
                if found.is_some() {
                    return found;
                }
            }
        }
    }
    None
}

/// The container holding paragraph `para` and its index in that container.
pub(crate) fn locate(root: &XmlElement, para: usize) -> Option<(Path, usize)> {
    let body = root.children.iter().position(|c| is_element(c, "body"))?;
    let RawXml::Element(b) = &root.children[body] else {
        return None;
    };
    let mut base = vec![body];
    walk(&b.children, &mut base, &mut 0, para)
}

/// Number of paragraphs in document order.
pub(crate) fn paragraph_count(root: &XmlElement) -> usize {
    fn count(children: &[RawXml]) -> usize {
        let mut n = 0;
        for c in children {
            let RawXml::Element(e) = c else { continue };
            if e.head.is_w("p") {
                n += 1;
            } else if e.head.is_w("tbl") {
                for row in e
                    .children
                    .iter()
                    .filter_map(RawXml::as_element)
                    .filter(|r| r.head.is_w("tr"))
                {
                    for cell in row
                        .children
                        .iter()
                        .filter_map(RawXml::as_element)
                        .filter(|c| c.head.is_w("tc"))
                    {
                        n += count(&cell.children);
                    }
                }
            } else if e.head.is_w("sdt") {
                for content in e
                    .children
                    .iter()
                    .filter_map(RawXml::as_element)
                    .filter(|c| c.head.is_w("sdtContent"))
                {
                    n += count(&content.children);
                }
            }
        }
        n
    }
    root.children
        .iter()
        .filter_map(RawXml::as_element)
        .find(|e| e.head.is_w("body"))
        .map_or(0, |b| count(&b.children))
}

pub(crate) fn container_mut<'a>(
    root: &'a mut XmlElement,
    path: &[usize],
) -> Option<&'a mut Vec<RawXml>> {
    let mut node = root;
    for &i in path {
        node = match node.children.get_mut(i)? {
            RawXml::Element(e) => e,
            _ => return None,
        };
    }
    Some(&mut node.children)
}

/// Paragraph `para` for editing.
pub(crate) fn para_mut(
    root: &mut XmlElement,
    para: usize,
) -> Result<&mut XmlElement, CommandError> {
    let (path, i) = locate(root, para).ok_or_else(|| fail(format!("no paragraph {para}")))?;
    let blocks = container_mut(root, &path).ok_or_else(|| fail("container not found"))?;
    match blocks.get_mut(i) {
        Some(RawXml::Element(e)) => Ok(e),
        _ => Err(fail("paragraph not found")),
    }
}

// ---- characters -----------------------------------------------------------------------------

const GROUPS: &[&str] = &[
    "hyperlink",
    "ins",
    "del",
    "moveTo",
    "smartTag",
    "fldSimple",
    "sdt",
    "sdtContent",
];

fn is_group(e: &XmlElement) -> bool {
    GROUPS.iter().any(|g| e.head.is_w(g))
}

fn content_len(n: &RawXml) -> usize {
    match n {
        RawXml::Element(e)
            if e.head.is_w("t") && e.children.iter().all(|c| matches!(c, RawXml::Text(_))) =>
        {
            e.children
                .iter()
                .map(|c| match c {
                    RawXml::Text(t) => t.chars().count(),
                    _ => 0,
                })
                .sum()
        }
        RawXml::Element(e) if e.head.is_w("t") => 0,
        RawXml::Element(e)
            if e.head.is_w("tab") && e.head.attrs.is_empty() && e.children.is_empty() =>
        {
            1
        }
        RawXml::Element(e) if e.head.is_w("br") && e.children.is_empty() => 1,
        _ => 0,
    }
}

/// Characters a paragraph child contributes.
pub(crate) fn node_len(n: &RawXml) -> usize {
    match n {
        RawXml::Element(e) if e.head.is_w("r") => e.children.iter().map(content_len).sum(),
        RawXml::Element(e) if is_group(e) => e.children.iter().map(node_len).sum(),
        _ => 0,
    }
}

pub(crate) fn char_len(p: &XmlElement) -> usize {
    p.children.iter().map(node_len).sum()
}

/// The paragraph's text as the document model reads it (tab `\t`, break `\n`).
pub(crate) fn para_text(p: &XmlElement) -> String {
    fn push(n: &RawXml, out: &mut String) {
        match n {
            RawXml::Element(e) if e.head.is_w("r") => {
                for c in &e.children {
                    match c {
                        RawXml::Element(t) if t.head.is_w("t") && content_len(c) > 0 => {
                            for x in &t.children {
                                if let RawXml::Text(s) = x {
                                    out.push_str(s);
                                }
                            }
                        }
                        RawXml::Element(_) if content_len(c) == 1 => {
                            let tab = matches!(c, RawXml::Element(x) if x.head.is_w("tab"));
                            out.push(if tab { '\t' } else { '\n' });
                        }
                        _ => {}
                    }
                }
            }
            RawXml::Element(e) if is_group(e) => {
                for c in &e.children {
                    push(c, out);
                }
            }
            _ => {}
        }
    }
    let mut out = String::new();
    for c in &p.children {
        push(c, &mut out);
    }
    out
}

fn split_text_el(like: &XmlElement, text: String) -> XmlElement {
    let mut head = like.head.clone();
    head.attrs.retain(|a| !a.is_xml_space());
    head.attrs.push(Attr {
        name: "xml:space".to_owned(),
        ns: Some(XML_NS.into()),
        value: "preserve".to_owned(),
    });
    XmlElement {
        head,
        children: if text.is_empty() {
            Vec::new()
        } else {
            vec![RawXml::Text(text)]
        },
    }
}

/// Splits run `r` after `k` characters (`0 < k < len`); `r` keeps the left part.
fn split_run(r: &mut XmlElement, k: usize) -> XmlElement {
    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut acc = 0;
    let old = std::mem::take(&mut r.children);
    for (idx, node) in old.into_iter().enumerate() {
        if idx == 0 && is_element(&node, "rPr") {
            right.push(node.clone());
            left.push(node);
            continue;
        }
        let len = content_len(&node);
        if acc >= k {
            right.push(node);
        } else if len == 0 || acc + len <= k {
            left.push(node);
            acc += len;
        } else if let RawXml::Element(t) = &node {
            let text: String = t
                .children
                .iter()
                .filter_map(|c| {
                    if let RawXml::Text(s) = c {
                        Some(s.as_str())
                    } else {
                        None
                    }
                })
                .collect();
            let head: String = text.chars().take(k - acc).collect();
            let tail: String = text.chars().skip(k - acc).collect();
            left.push(RawXml::Element(split_text_el(t, head)));
            right.push(RawXml::Element(split_text_el(t, tail)));
            acc = k;
        }
    }
    r.children = left;
    XmlElement {
        head: r.head.clone(),
        children: right,
    }
}

/// Makes `offset` a boundary between two runs. Returns whether a run was split.
fn split_at(children: &mut Vec<RawXml>, offset: usize, acc: &mut usize) -> bool {
    let mut i = 0;
    while i < children.len() {
        let len = node_len(&children[i]);
        let start = *acc;
        match &mut children[i] {
            RawXml::Element(e) if e.head.is_w("r") => {
                if len > 0 && offset > start && offset < start + len {
                    let right = split_run(e, offset - start);
                    children.insert(i + 1, RawXml::Element(right));
                    return true;
                }
                *acc = start + len;
            }
            RawXml::Element(e) if is_group(e) => {
                if len > 0
                    && offset > start
                    && offset < start + len
                    && split_at(&mut e.children, offset, acc)
                {
                    return true;
                }
                *acc = start + len;
            }
            _ => {}
        }
        i += 1;
    }
    false
}

fn runs_in(
    children: &mut [RawXml],
    s: usize,
    e: usize,
    acc: &mut usize,
    f: &mut dyn FnMut(&mut XmlElement),
) {
    for node in children {
        let len = node_len(node);
        let RawXml::Element(el) = node else { continue };
        if el.head.is_w("r") {
            let (st, en) = (*acc, *acc + len);
            let inside = if len > 0 {
                st >= s && en <= e
            } else {
                st > s && st < e
            };
            if inside {
                f(el);
            }
            *acc = en;
        } else if is_group(el) {
            let before = *acc;
            runs_in(&mut el.children, s, e, acc, f);
            *acc = before + len;
        }
    }
}

/// Calls `f` on every run inside characters `s..e` of paragraph `p`, splitting runs at the
/// ends first.
pub(crate) fn range_runs(
    p: &mut XmlElement,
    s: usize,
    e: usize,
    f: &mut dyn FnMut(&mut XmlElement),
) {
    if s >= e {
        return;
    }
    for boundary in [s, e] {
        split_at(&mut p.children, boundary, &mut 0);
    }
    runs_in(&mut p.children, s, e, &mut 0, f);
}

fn insert_at(
    children: &mut Vec<RawXml>,
    offset: usize,
    acc: &mut usize,
    nodes: &mut Option<Vec<RawXml>>,
) -> bool {
    let mut i = 0;
    while i < children.len() {
        let len = node_len(&children[i]);
        let start = *acc;
        let visible = matches!(&children[i], RawXml::Element(e) if e.head.is_w("r") || is_group(e));
        if visible && start >= offset {
            if let Some(n) = nodes.take() {
                for (k, node) in n.into_iter().enumerate() {
                    children.insert(i + k, node);
                }
            }
            return true;
        }
        if let RawXml::Element(e) = &mut children[i]
            && is_group(e)
            && len > 0
            && offset > start
            && offset < start + len
            && insert_at(&mut e.children, offset, acc, nodes)
        {
            return true;
        }
        *acc = start + len;
        i += 1;
    }
    false
}

/// Inserts `nodes` at character `offset` of paragraph `p` (after the runs before it).
pub(crate) fn insert_nodes_at(p: &mut XmlElement, offset: usize, nodes: Vec<RawXml>) {
    split_at(&mut p.children, offset, &mut 0);
    let mut pending = Some(nodes);
    if !insert_at(&mut p.children, offset, &mut 0, &mut pending)
        && let Some(n) = pending
    {
        p.children.extend(n);
    }
}

/// The characters of `p` in `s..e` in document order, after splitting runs at both ends.
pub(crate) fn range_text(p: &mut XmlElement, s: usize, e: usize) -> Vec<char> {
    let mut out = Vec::new();
    range_runs(p, s, e, &mut |run| {
        for c in &run.children {
            match c {
                RawXml::Element(t) if t.head.is_w("t") && content_len(c) > 0 => {
                    for x in &t.children {
                        if let RawXml::Text(s) = x {
                            out.extend(s.chars());
                        }
                    }
                }
                RawXml::Element(_) if content_len(c) == 1 => {
                    let tab = matches!(c, RawXml::Element(x) if x.head.is_w("tab"));
                    out.push(if tab { '\t' } else { '\n' });
                }
                _ => {}
            }
        }
    });
    out
}

/// Writes `new` (one character per character of the range, tabs and breaks included) back over
/// characters `s..e` of `p`; only the text of `w:t` elements changes.
pub(crate) fn write_range_text(p: &mut XmlElement, s: usize, e: usize, new: &[char]) {
    let mut at = 0;
    range_runs(p, s, e, &mut |run| {
        for c in &mut run.children {
            let n = content_len(c);
            if n == 0 {
                continue;
            }
            if let RawXml::Element(t) = c
                && t.head.is_w("t")
            {
                let replacement: String = new.iter().skip(at).take(n).collect();
                t.children = if replacement.is_empty() {
                    Vec::new()
                } else {
                    vec![RawXml::Text(replacement)]
                };
            }
            at += n;
        }
    });
}

// ---- package helpers ------------------------------------------------------------------------

pub(crate) fn rels_name(main: &PartName) -> String {
    let s = main.as_str();
    match s.rfind('/') {
        Some(i) => format!("{}/_rels/{}.rels", &s[..i], &s[i + 1..]),
        None => format!("_rels/{s}.rels"),
    }
}

fn attr_of(tag: &str, name: &str) -> Option<String> {
    let key = format!("{name}=\"");
    let at = tag.find(&key)? + key.len();
    let end = tag[at..].find('"')?;
    Some(tag[at..at + end].to_owned())
}

pub(crate) fn xml_escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

impl Main {
    pub(crate) fn root(&mut self) -> &mut XmlElement {
        &mut self.xml.root
    }

    /// Text of a part, empty when missing.
    pub(crate) fn text_of(&self, part: &str) -> Option<String> {
        self.pkg
            .part(part)
            .map(|p| String::from_utf8_lossy(p.data()).into_owned())
    }

    pub(crate) fn put(&mut self, part: &str, text: String) -> Result<(), CommandError> {
        self.pkg
            .set_part(PartName::new(part).map_err(opc)?, text.into_bytes());
        Ok(())
    }

    pub(crate) fn put_bytes(&mut self, part: &str, data: Vec<u8>) -> Result<(), CommandError> {
        self.pkg.set_part(PartName::new(part).map_err(opc)?, data);
        Ok(())
    }

    /// Makes sure the `[Content_Types].xml` carries an `Override` for `part`.
    pub(crate) fn content_override(
        &mut self,
        part: &str,
        content_type: &str,
    ) -> Result<(), CommandError> {
        let mut text = self
            .text_of("[Content_Types].xml")
            .ok_or_else(|| fail("no content types"))?;
        let marker = format!("PartName=\"/{part}\"");
        if text.contains(&marker) {
            return Ok(());
        }
        let at = text
            .rfind("</Types>")
            .ok_or_else(|| fail("bad content types"))?;
        text.insert_str(
            at,
            &format!("<Override PartName=\"/{part}\" ContentType=\"{content_type}\"/>"),
        );
        self.put("[Content_Types].xml", text)
    }

    /// Makes sure the content types carry a `Default` for the extension.
    pub(crate) fn content_default(
        &mut self,
        ext: &str,
        content_type: &str,
    ) -> Result<(), CommandError> {
        let mut text = self
            .text_of("[Content_Types].xml")
            .ok_or_else(|| fail("no content types"))?;
        let lower = text.to_ascii_lowercase();
        if lower.contains(&format!("extension=\"{}\"", ext.to_ascii_lowercase())) {
            return Ok(());
        }
        let at = text
            .find("<Override")
            .or_else(|| text.rfind("</Types>"))
            .ok_or_else(|| fail("bad content types"))?;
        text.insert_str(
            at,
            &format!("<Default Extension=\"{ext}\" ContentType=\"{content_type}\"/>"),
        );
        self.put("[Content_Types].xml", text)
    }

    /// Adds a relationship of the main part and returns its id. `external` sets
    /// `TargetMode="External"`. An identical relationship is reused.
    pub(crate) fn add_rel(
        &mut self,
        rel_type: &str,
        target: &str,
        external: bool,
    ) -> Result<String, CommandError> {
        let name = rels_name(&self.name);
        let mut text = self.text_of(&name).unwrap_or_else(|| {
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"></Relationships>".to_owned()
        });
        let mut max = 0u32;
        let mut rest = text.as_str();
        while let Some(i) = rest.find("<Relationship ") {
            let tag = &rest[i..rest
                .len()
                .min(i + rest[i..].find('>').map_or(rest.len() - i, |e| e + 1))];
            if let Some(id) = attr_of(tag, "Id")
                && let Some(n) = id.strip_prefix("rId").and_then(|n| n.parse::<u32>().ok())
            {
                max = max.max(n);
            }
            if attr_of(tag, "Type").as_deref() == Some(rel_type)
                && attr_of(tag, "Target").as_deref() == Some(&xml_escape(target))
                && (attr_of(tag, "TargetMode").as_deref() == Some("External")) == external
                && let Some(id) = attr_of(tag, "Id")
            {
                return Ok(id);
            }
            rest = &rest[i + 14..];
        }
        let id = format!("rId{}", max + 1);
        let mode = if external {
            " TargetMode=\"External\""
        } else {
            ""
        };
        let at = text
            .rfind("</Relationships>")
            .ok_or_else(|| fail("bad relationships part"))?;
        text.insert_str(
            at,
            &format!(
                "<Relationship Id=\"{id}\" Type=\"{rel_type}\" Target=\"{}\"{mode}/>",
                xml_escape(target)
            ),
        );
        self.put(&name, text)?;
        Ok(id)
    }

    /// Removes the relationship `id` of the main part.
    pub(crate) fn remove_rel(&mut self, id: &str) -> Result<(), CommandError> {
        let name = rels_name(&self.name);
        let Some(mut text) = self.text_of(&name) else {
            return Ok(());
        };
        if let Some(at) = text.find(&format!("Id=\"{id}\""))
            && let Some(start) = text[..at].rfind("<Relationship ")
            && let Some(end) = text[at..].find('>')
        {
            text.replace_range(start..=at + end, "");
            self.put(&name, text)?;
        }
        Ok(())
    }

    /// Declares `xmlns:prefix` on the document element when it is missing.
    pub(crate) fn declare_ns(&mut self, prefix: &str, uri: &str) {
        let name = format!("xmlns:{prefix}");
        let root = self.root();
        if !root.head.attrs.iter().any(|a| a.name == name) {
            root.head.attrs.push(Attr {
                name,
                ns: None,
                value: uri.to_owned(),
            });
        }
    }

    /// Appends `<w:style>` text before `</w:styles>` unless a style with `id` exists.
    pub(crate) fn ensure_style(&mut self, id: &str, xml: &str) -> Result<(), CommandError> {
        let part = "word/styles.xml";
        let Some(mut text) = self.text_of(part) else {
            return Ok(());
        };
        if text.contains(&format!("w:styleId=\"{id}\"")) {
            return Ok(());
        }
        let Some(at) = text.rfind("</w:styles>") else {
            return Ok(());
        };
        text.insert_str(at, xml);
        self.put(part, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(xml: &str) -> XmlElement {
        let src = format!("<w:p xmlns:w=\"{}\">{xml}</w:p>", open365_docx::W_NS);
        parse_xml(src.as_bytes()).expect("xml").root
    }

    fn out(e: &XmlElement) -> String {
        let mut s = String::new();
        e.write(&mut s);
        s
    }

    #[test]
    fn counts_like_the_model_and_splits_inside_a_text() {
        let mut para = p("<w:r><w:t>hello</w:t><w:tab/><w:t>x</w:t></w:r>");
        assert_eq!(char_len(&para), 7);
        assert_eq!(para_text(&para), "hello\tx");
        let mut n = 0;
        range_runs(&mut para, 2, 5, &mut |_| n += 1);
        assert_eq!(n, 1);
        assert_eq!(para.children.len(), 3);
        assert_eq!(char_len(&para), 7);
        assert!(out(&para).contains("xml:space=\"preserve\""));
    }

    #[test]
    fn text_is_rewritten_in_place() {
        let mut para = p("<w:r><w:t>ab</w:t></w:r><w:r><w:t>cd</w:t></w:r>");
        let chars = range_text(&mut para, 1, 3);
        assert_eq!(chars, vec!['b', 'c']);
        write_range_text(&mut para, 1, 3, &['B', 'C']);
        assert_eq!(para_text(&para), "aBCd");
    }

    #[test]
    fn nodes_go_in_at_a_character_offset() {
        let mut para = p("<w:r><w:t>abcd</w:t></w:r>");
        let marker = RawXml::Element(w(&para.head, "bookmarkStart"));
        insert_nodes_at(&mut para, 2, vec![marker]);
        let text = out(&para);
        let at = text.find("bookmarkStart").expect("marker");
        assert!(text[..at].contains("ab") && text[at..].contains("cd"));
    }
}
