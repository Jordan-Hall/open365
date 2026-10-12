//! Writer Insert tab operations: page breaks, tables, hyperlinks, bookmarks, pictures,
//! comments, headers and footers, page numbers. Each runs as one undo step (the caller keeps
//! the previous document).

use open365_docx::{
    Attr, AutoFitMode, DocPos, Document, NewComment, RawXml, XmlElement, parse_xml,
};
use serde::{Deserialize, Serialize};

use super::writer_xml::{
    self as x, Main, R_NS, RPR_ORDER, fail, para_mut, ppr_mut, rewrite, rpr_mut, set_ordered, w,
    w_val,
};
use crate::error::CommandError;

/// Where a hyperlink leads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LinkTarget {
    /// A web address, mail address or file.
    Url { url: String },
    /// A bookmark of this document.
    Bookmark { name: String },
}

/// Which header or footer of a section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HfKind {
    Default,
    /// The first page (turns on Different First Page).
    First,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HfAlign {
    Left,
    Center,
    Right,
}

/// What a [`crate::Command::DocxInsert`] does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum WriterInsertOp {
    /// Page Break (Ctrl+Enter): the paragraph splits at `pos` and the break ends the first part.
    PageBreak { pos: DocPos },
    /// Blank Page: a page break, an empty page, a page break.
    BlankPage { pos: DocPos },
    /// Table: `rows` by `cols` before the paragraph at `pos` (which splits when `pos` is inside
    /// it). `text_width_pt` is the width the columns share.
    Table {
        pos: DocPos,
        rows: usize,
        cols: usize,
        text_width_pt: f64,
    },
    /// Link: the text of the range becomes a hyperlink; with a collapsed range `text` is
    /// inserted as the link text.
    Hyperlink {
        start: DocPos,
        end: DocPos,
        target: LinkTarget,
        #[serde(default)]
        tip: Option<String>,
        #[serde(default)]
        text: Option<String>,
    },
    /// Bookmark: marks the range; an existing bookmark of that name moves.
    Bookmark {
        start: DocPos,
        end: DocPos,
        name: String,
    },
    /// Pictures: an inline picture at `pos`. `data_hex` is the file, `ext` its extension.
    Picture {
        pos: DocPos,
        data_hex: String,
        ext: String,
        width_pt: f64,
        height_pt: f64,
        name: String,
        #[serde(default)]
        alt: String,
    },
    /// Comment on the range.
    Comment {
        start: DocPos,
        end: DocPos,
        author: String,
        text: String,
    },
    /// Header or Footer of the section holding paragraph `para`: its text, a page number, or
    /// both. `append` adds a paragraph to the existing content; otherwise the content is
    /// replaced. `remove` deletes the header or footer.
    HeaderFooter {
        para: usize,
        footer: bool,
        kind: HfKind,
        #[serde(default)]
        text: String,
        #[serde(default)]
        page_number: Option<HfAlign>,
        #[serde(default)]
        append: bool,
        #[serde(default)]
        remove: bool,
    },
}

/// Hex text of a byte string (the picture file in a command).
#[must_use]
pub fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn from_hex(text: &str) -> Result<Vec<u8>, CommandError> {
    if !text.len().is_multiple_of(2) {
        return Err(fail("odd picture data"));
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(|_| fail("bad picture data")))
        .collect()
}

/// Runs `op` on `doc`; `doc` is unchanged when it fails.
pub(crate) fn run(doc: &mut Document, op: &WriterInsertOp) -> Result<(), CommandError> {
    match op {
        WriterInsertOp::PageBreak { pos } => page_break(doc, *pos),
        WriterInsertOp::BlankPage { pos } => {
            page_break(doc, *pos)?;
            page_break(doc, DocPos::new(pos.para + 1, 0))
        }
        WriterInsertOp::Table {
            pos,
            rows,
            cols,
            text_width_pt,
        } => {
            doc.insert_table(*pos, *rows, *cols, *text_width_pt, AutoFitMode::Fixed)?;
            Ok(())
        }
        WriterInsertOp::Comment {
            start,
            end,
            author,
            text,
        } => {
            doc.add_comment(*start, *end, &NewComment::new(author, text))?;
            Ok(())
        }
        other => rewrite(doc, |m| apply(m, other)),
    }
}

fn page_break(doc: &mut Document, pos: DocPos) -> Result<(), CommandError> {
    doc.split_paragraph(pos)?;
    rewrite(doc, |m| {
        let p = para_mut(m.root(), pos.para)?;
        let mut br = w(&p.head, "br");
        br.head.set_attr_w("type", "page");
        let mut run = w(&p.head, "r");
        run.children.push(RawXml::Element(br));
        p.children.push(RawXml::Element(run));
        Ok(())
    })
}

fn apply(m: &mut Main, op: &WriterInsertOp) -> Result<(), CommandError> {
    match op {
        WriterInsertOp::Hyperlink {
            start,
            end,
            target,
            tip,
            text,
        } => hyperlink(m, *start, *end, target, tip.as_deref(), text.as_deref()),
        WriterInsertOp::Bookmark { start, end, name } => bookmark(m, *start, *end, name),
        WriterInsertOp::Picture {
            pos,
            data_hex,
            ext,
            width_pt,
            height_pt,
            name,
            alt,
        } => picture(
            m,
            *pos,
            &from_hex(data_hex)?,
            ext,
            *width_pt,
            *height_pt,
            name,
            alt,
        ),
        WriterInsertOp::HeaderFooter {
            para,
            footer,
            kind,
            text,
            page_number,
            append,
            remove,
        } => header_footer(
            m,
            *para,
            *footer,
            *kind,
            text,
            *page_number,
            *append,
            *remove,
        ),
        _ => Err(fail("not a package operation")),
    }
}

// ---- hyperlinks and bookmarks -------------------------------------------------------------

const HYPERLINK_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";
const HYPERLINK_STYLE: &str = "<w:style w:type=\"character\" w:styleId=\"Hyperlink\"><w:name w:val=\"Hyperlink\"/><w:basedOn w:val=\"DefaultParagraphFont\"/><w:uiPriority w:val=\"99\"/><w:unhideWhenUsed/><w:rPr><w:color w:val=\"0563C1\" w:themeColor=\"hyperlink\"/><w:u w:val=\"single\"/></w:rPr></w:style>";

fn r_attr(name: &str, value: &str) -> Attr {
    Attr {
        name: format!("r:{name}"),
        ns: Some(R_NS.into()),
        value: value.to_owned(),
    }
}

fn hyperlink(
    m: &mut Main,
    start: DocPos,
    end: DocPos,
    target: &LinkTarget,
    tip: Option<&str>,
    text: Option<&str>,
) -> Result<(), CommandError> {
    let (s, e) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };
    let mut link = match target {
        LinkTarget::Url { url } => {
            if url.trim().is_empty() {
                return Err(fail("a link needs an address"));
            }
            m.declare_ns("r", R_NS);
            let id = m.add_rel(HYPERLINK_TYPE, url.trim(), true)?;
            let mut l = w(&m.xml.root.head, "hyperlink");
            l.head.attrs.push(r_attr("id", &id));
            l
        }
        LinkTarget::Bookmark { name } => {
            let mut l = w(&m.xml.root.head, "hyperlink");
            l.head.set_attr_w("anchor", name);
            l
        }
    };
    link.head.set_attr_w("history", "1");
    if let Some(t) = tip.filter(|t| !t.is_empty()) {
        link.head.set_attr_w("tooltip", t);
    }
    m.ensure_style("Hyperlink", HYPERLINK_STYLE)?;
    if s == e {
        let text = text
            .filter(|t| !t.is_empty())
            .ok_or_else(|| fail("a link needs text"))?;
        let p = para_mut(m.root(), s.para)?;
        let mut run = w(&p.head, "r");
        let mut props = w(&p.head, "rPr");
        props
            .children
            .push(RawXml::Element(w_val(&p.head, "rStyle", "Hyperlink")));
        run.children.push(RawXml::Element(props));
        run.children
            .push(RawXml::Element(x::text_el(&p.head, "t", text)));
        link.children.push(RawXml::Element(run));
        x::insert_nodes_at(p, s.offset, vec![RawXml::Element(link)]);
        return Ok(());
    }
    for para in s.para..=e.para {
        let p = para_mut(m.root(), para)?;
        let len = x::char_len(p);
        let from = if para == s.para { s.offset.min(len) } else { 0 };
        let to = if para == e.para {
            e.offset.min(len)
        } else {
            len
        };
        if from >= to {
            continue;
        }
        x::range_runs(p, from, to, &mut |r| {
            let like = r.head.clone();
            let rpr = rpr_mut(r);
            set_ordered(rpr, RPR_ORDER, w_val(&like, "rStyle", "Hyperlink"));
        });
        wrap_runs(p, from, to, &link);
    }
    Ok(())
}

/// Wraps the top-level runs inside `from..to` in copies of `link` (one per contiguous stretch).
fn wrap_runs(p: &mut XmlElement, from: usize, to: usize, link: &XmlElement) {
    let mut acc = 0;
    let mut stretch: Vec<usize> = Vec::new();
    let mut stretches: Vec<Vec<usize>> = Vec::new();
    for (i, node) in p.children.iter().enumerate() {
        let len = x::node_len(node);
        let is_run = matches!(node, RawXml::Element(e) if e.head.is_w("r"));
        let inside = is_run && len > 0 && acc >= from && acc + len <= to;
        if inside {
            stretch.push(i);
        } else if !stretch.is_empty() {
            stretches.push(std::mem::take(&mut stretch));
        }
        acc += len;
    }
    if !stretch.is_empty() {
        stretches.push(stretch);
    }
    for run in stretches.into_iter().rev() {
        let (a, b) = (run[0], run[run.len() - 1]);
        let moved: Vec<RawXml> = p.children.drain(a..=b).collect();
        let mut l = link.clone();
        l.children = moved;
        p.children.insert(a, RawXml::Element(l));
    }
}

fn valid_bookmark(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(char::is_alphabetic)
        && name.chars().count() <= 40
        && name.chars().all(|c| c.is_alphanumeric() || c == '_')
}

fn walk_elements(e: &mut XmlElement, f: &mut dyn FnMut(&mut XmlElement)) {
    f(e);
    for c in &mut e.children {
        if let RawXml::Element(x) = c {
            walk_elements(x, f);
        }
    }
}

fn bookmark(m: &mut Main, start: DocPos, end: DocPos, name: &str) -> Result<(), CommandError> {
    if !valid_bookmark(name) {
        return Err(fail(
            "a bookmark name begins with a letter and holds only letters, numbers and underscores",
        ));
    }
    let (s, e) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };
    // An existing bookmark of the name moves: drop its markers.
    let mut stale: Vec<String> = Vec::new();
    let mut max_id = 0i64;
    walk_elements(m.root(), &mut |el| {
        if el.head.is_w("bookmarkStart")
            && let Some(id) = el.head.attr_w("id").and_then(|v| v.parse::<i64>().ok())
        {
            max_id = max_id.max(id);
            if el.head.attr_w("name") == Some(name) {
                stale.push(id.to_string());
            }
        }
    });
    if !stale.is_empty() {
        fn strip(e: &mut XmlElement, stale: &[String]) {
            e.children.retain(|c| match c {
                RawXml::Element(x) => {
                    !((x.head.is_w("bookmarkStart") || x.head.is_w("bookmarkEnd"))
                        && x.head
                            .attr_w("id")
                            .is_some_and(|i| stale.iter().any(|s| s == i)))
                }
                _ => true,
            });
            for c in &mut e.children {
                if let RawXml::Element(x) = c {
                    strip(x, stale);
                }
            }
        }
        strip(m.root(), &stale);
    }
    let id = (max_id + 1).to_string();
    let p = para_mut(m.root(), e.para)?;
    let mut end_el = w(&p.head, "bookmarkEnd");
    end_el.head.set_attr_w("id", &id);
    let len = x::char_len(p);
    x::insert_nodes_at(p, e.offset.min(len), vec![RawXml::Element(end_el)]);
    let p = para_mut(m.root(), s.para)?;
    let mut start_el = w(&p.head, "bookmarkStart");
    start_el.head.set_attr_w("id", &id);
    start_el.head.set_attr_w("name", name);
    let len = x::char_len(p);
    x::insert_nodes_at(p, s.offset.min(len), vec![RawXml::Element(start_el)]);
    Ok(())
}

// ---- pictures -------------------------------------------------------------------------------

const IMAGE_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
const EMU_PER_PT: f64 = 12_700.0;

fn content_type_of(ext: &str) -> Option<&'static str> {
    match ext {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "bmp" => Some("image/bmp"),
        "tif" | "tiff" => Some("image/tiff"),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn picture(
    m: &mut Main,
    pos: DocPos,
    data: &[u8],
    ext: &str,
    width_pt: f64,
    height_pt: f64,
    name: &str,
    alt: &str,
) -> Result<(), CommandError> {
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    let ct =
        content_type_of(&ext).ok_or_else(|| fail(format!("{ext} pictures are not supported")))?;
    if data.is_empty() || !(width_pt > 0.0 && height_pt > 0.0) {
        return Err(fail("the picture is empty"));
    }
    let mut n = 1;
    while m.pkg.part(&format!("word/media/image{n}.{ext}")).is_some() {
        n += 1;
    }
    let part = format!("word/media/image{n}.{ext}");
    m.put_bytes(&part, data.to_vec())?;
    m.content_default(&ext, ct)?;
    m.declare_ns("r", R_NS);
    let rid = m.add_rel(IMAGE_TYPE, &format!("media/image{n}.{ext}"), false)?;
    let mut doc_pr = 0i64;
    walk_elements(m.root(), &mut |el| {
        if el.head.local_name() == "docPr"
            && let Some(id) = el
                .head
                .attrs
                .iter()
                .find(|a| a.name == "id")
                .and_then(|a| a.value.parse::<i64>().ok())
        {
            doc_pr = doc_pr.max(id);
        }
    });
    let (cx, cy) = (
        (width_pt * EMU_PER_PT).round() as i64,
        (height_pt * EMU_PER_PT).round() as i64,
    );
    let name = x::xml_escape(name);
    let alt = x::xml_escape(alt);
    let xml = format!(
        "<w:r xmlns:w=\"{w}\" xmlns:r=\"{r}\" xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/><wp:docPr id=\"{id}\" name=\"{name}\" descr=\"{alt}\"/><wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect=\"1\"/></wp:cNvGraphicFramePr><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><pic:pic><pic:nvPicPr><pic:cNvPr id=\"0\" name=\"{name}\" descr=\"{alt}\"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed=\"{rid}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>",
        w = open365_docx::W_NS,
        r = R_NS,
        id = doc_pr + 1,
    );
    let run = parse_xml(xml.as_bytes())?.root;
    let p = para_mut(m.root(), pos.para)?;
    let len = x::char_len(p);
    x::insert_nodes_at(p, pos.offset.min(len), vec![RawXml::Element(run)]);
    Ok(())
}

// ---- headers and footers --------------------------------------------------------------------

const HEADER_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/header";
const FOOTER_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer";
const HEADER_CT: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml";
const FOOTER_CT: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml";

fn style_xml(id: &str, name: &str) -> String {
    format!(
        "<w:style w:type=\"paragraph\" w:styleId=\"{id}\"><w:name w:val=\"{name}\"/><w:basedOn w:val=\"Normal\"/><w:uiPriority w:val=\"99\"/><w:unhideWhenUsed/><w:pPr><w:tabs><w:tab w:val=\"center\" w:pos=\"4513\"/><w:tab w:val=\"right\" w:pos=\"9026\"/></w:tabs><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr></w:style>"
    )
}

fn hf_paragraph(style: &str, text: &str, page: Option<HfAlign>) -> String {
    let jc = match page {
        Some(HfAlign::Center) => "<w:jc w:val=\"center\"/>",
        Some(HfAlign::Right) => "<w:jc w:val=\"right\"/>",
        _ => "",
    };
    let mut runs = String::new();
    if !text.is_empty() {
        runs.push_str(&format!(
            "<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r>",
            x::xml_escape(text)
        ));
    }
    if page.is_some() {
        runs.push_str("<w:r><w:fldChar w:fldCharType=\"begin\"/></w:r><w:r><w:instrText xml:space=\"preserve\"> PAGE </w:instrText></w:r><w:r><w:fldChar w:fldCharType=\"separate\"/></w:r><w:r><w:rPr><w:noProof/></w:rPr><w:t>1</w:t></w:r><w:r><w:fldChar w:fldCharType=\"end\"/></w:r>");
    }
    format!("<w:p><w:pPr><w:pStyle w:val=\"{style}\"/>{jc}</w:pPr>{runs}</w:p>")
}

/// The `w:sectPr` of the section holding paragraph `para`.
fn section_of(root: &mut XmlElement, para: usize) -> Result<&mut XmlElement, CommandError> {
    let count = x::paragraph_count(root);
    let mut at = None;
    for i in para..count {
        let p = para_mut(root, i)?;
        if x::ppr_has_sect(p) {
            at = Some(i);
            break;
        }
    }
    match at {
        Some(i) => {
            let p = para_mut(root, i)?;
            let props = ppr_mut(p);
            child_mut_sect(props)
        }
        None => {
            let body = root
                .children
                .iter_mut()
                .find_map(|c| match c {
                    RawXml::Element(e) if e.head.is_w("body") => Some(e),
                    _ => None,
                })
                .ok_or_else(|| fail("no body"))?;
            if x::child(body, "sectPr").is_none() {
                let s = w(&body.head, "sectPr");
                body.children.push(RawXml::Element(s));
            }
            x::child_mut(body, "sectPr").ok_or_else(|| fail("no section properties"))
        }
    }
}

fn child_mut_sect(props: &mut XmlElement) -> Result<&mut XmlElement, CommandError> {
    x::child_mut(props, "sectPr").ok_or_else(|| fail("no section properties"))
}

#[allow(clippy::too_many_arguments)]
fn header_footer(
    m: &mut Main,
    para: usize,
    footer: bool,
    kind: HfKind,
    text: &str,
    page: Option<HfAlign>,
    append: bool,
    remove: bool,
) -> Result<(), CommandError> {
    let (local, ty, ct, style, style_name) = if footer {
        (
            "footerReference",
            FOOTER_TYPE,
            FOOTER_CT,
            "Footer",
            "footer",
        )
    } else {
        (
            "headerReference",
            HEADER_TYPE,
            HEADER_CT,
            "Header",
            "header",
        )
    };
    let kind_val = match kind {
        HfKind::Default => "default",
        HfKind::First => "first",
    };
    m.declare_ns("r", R_NS);
    let root_head = m.xml.root.head.clone();
    // The relationship id and part of an existing reference of this kind.
    let (existing, sect_has_title) = {
        let sect = section_of(m.root(), para)?;
        let found = sect.children.iter().find_map(|c| match c {
            RawXml::Element(e) if e.head.is_w(local) && e.head.attr_w("type") == Some(kind_val) => {
                e.head
                    .attrs
                    .iter()
                    .find(|a| a.name == "r:id")
                    .map(|a| a.value.clone())
            }
            _ => None,
        });
        (found, x::child(sect, "titlePg").is_some())
    };
    let rels = m.text_of(&x::rels_name(&m.name)).unwrap_or_default();
    let part_of = |rid: &str| -> Option<String> {
        let tag_at = rels.find(&format!("Id=\"{rid}\""))?;
        let start = rels[..tag_at].rfind("<Relationship ")?;
        let end = rels[tag_at..].find('>')? + tag_at;
        let tag = &rels[start..end];
        let t = tag.find("Target=\"")? + 8;
        let target = &tag[t..t + tag[t..].find('"')?];
        Some(match target.strip_prefix('/') {
            Some(abs) => abs.to_owned(),
            None => format!("word/{target}"),
        })
    };
    if remove {
        let Some(rid) = existing else { return Ok(()) };
        let part = part_of(&rid);
        let sect = section_of(m.root(), para)?;
        sect.children.retain(|c| {
            !matches!(c, RawXml::Element(e) if e.head.is_w(local) && e.head.attrs.iter().any(|a| a.name == "r:id" && a.value == rid))
        });
        if let Some(p) = part {
            m.pkg.remove_part_and_override(&p);
        }
        m.remove_rel(&rid)?;
        return Ok(());
    }
    m.ensure_style(style, &style_xml(style, style_name))?;
    let ns = format!("xmlns:w=\"{}\" xmlns:r=\"{R_NS}\"", open365_docx::W_NS);
    let root_tag = if footer { "w:ftr" } else { "w:hdr" };
    let paragraph = hf_paragraph(style, text, page);
    if let Some(rid) = existing
        && let Some(part) = part_of(&rid)
    {
        let body = if append {
            let old = m.text_of(&part).unwrap_or_default();
            let inner = old
                .find('>')
                .and_then(|_| {
                    let start = old.find(&format!("<{root_tag}"))?;
                    let open_end = old[start..].find('>')? + start + 1;
                    let close = old.rfind(&format!("</{root_tag}>"))?;
                    Some(old[open_end..close].to_owned())
                })
                .unwrap_or_default();
            let inner = if inner.trim() == "<w:p/>" || inner.trim().is_empty() {
                String::new()
            } else {
                inner
            };
            format!("{inner}{paragraph}")
        } else {
            paragraph
        };
        m.put(&part, format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<{root_tag} {ns}>{body}</{root_tag}>"))?;
        return Ok(());
    }
    let prefix = if footer { "footer" } else { "header" };
    let mut n = 1;
    while m.pkg.part(&format!("word/{prefix}{n}.xml")).is_some() {
        n += 1;
    }
    let part = format!("word/{prefix}{n}.xml");
    m.put(&part, format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<{root_tag} {ns}>{paragraph}</{root_tag}>"))?;
    m.content_override(&part, ct)?;
    let rid = m.add_rel(ty, &format!("{prefix}{n}.xml"), false)?;
    let sect = section_of(m.root(), para)?;
    let mut r = w(&root_head, local);
    r.head.set_attr_w("type", kind_val);
    r.head.attrs.push(r_attr("id", &rid));
    // References come first in the section properties.
    sect.children.insert(0, RawXml::Element(r));
    if kind == HfKind::First && !sect_has_title {
        let rank = [
            "pgSz",
            "pgMar",
            "paperSrc",
            "pgBorders",
            "lnNumType",
            "pgNumType",
            "cols",
            "formProt",
            "vAlign",
            "noEndnote",
        ];
        let at = sect
            .children
            .iter()
            .rposition(|c| matches!(c, RawXml::Element(e) if rank.contains(&e.head.local_name())))
            .map_or(sect.children.len(), |i| i + 1);
        sect.children
            .insert(at, RawXml::Element(w(&root_head, "titlePg")));
    }
    Ok(())
}
