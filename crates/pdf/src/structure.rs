//! The logical structure of the document (the tagged-PDF tree), built from the DOCX model in the
//! layout's paragraph order. What a paragraph is (heading level, list item) comes from the
//! layout's [`ParaRole`] and the paragraph style; drawing facts such as borders and shading come
//! from the layout too.
//!
//! What Word writes for tables, headings and languages is taken from its own exports of
//! `fixtures/reference/pdf/probes`: `Table > THead > TR > TH`, `TBody > TR > TH/TD` (the first
//! row and, with the table's `firstColumn` look, the first column are headers), `Table /Alt` from
//! the table description, `H1`..`H6` for numbered headings with the label in the same marked
//! content, `Span /Lang` for text in another language than the document's.

use std::collections::HashMap;
use std::ops::Range;

use open365_docx::{
    Block, BreakKind, Document, Inline, Numbering, Paragraph, RawXml, Run, RunContent, Styles,
    TableChild, WrapType, XmlElement,
};
use open365_layout::ParaRole;
use open365_opc::{Relationships, TargetMode};

use crate::lang::{Languages, primary};
use crate::writer::{Pdf, text_string};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Document,
    P,
    H(u8),
    L,
    Li,
    Lbl,
    LBody,
    Table,
    THead,
    TBody,
    Tr,
    Th,
    Td,
    /// A text box (Word's `Textbox`, role-mapped to `Sect`): the paragraphs of the box.
    Textbox,
    /// A table of contents field's paragraphs (`TOC > TOCI`).
    Toc,
    Toci,
}

impl Kind {
    pub fn name(self) -> String {
        match self {
            Self::Document => "Document".into(),
            Self::P => "P".into(),
            Self::H(n) => format!("H{}", n.clamp(1, 6)),
            Self::L => "L".into(),
            Self::Li => "LI".into(),
            Self::Lbl => "Lbl".into(),
            Self::LBody => "LBody".into(),
            Self::Table => "Table".into(),
            Self::THead => "THead".into(),
            Self::TBody => "TBody".into(),
            Self::Tr => "TR".into(),
            Self::Th => "TH".into(),
            Self::Td => "TD".into(),
            Self::Textbox => "Textbox".into(),
            Self::Toc => "TOC".into(),
            Self::Toci => "TOCI".into(),
        }
    }
}

/// One structure element. `id` is its reserved PDF object number.
#[derive(Debug)]
pub struct Node {
    pub kind: Kind,
    pub id: u32,
    pub kids: Vec<Node>,
    /// Extra entries, written verbatim into the element dictionary.
    pub attrs: String,
}

#[derive(Debug, Clone)]
pub struct ParaMeta {
    /// Structure element that owns the paragraph's marked content.
    pub node: u32,
    /// The `Lbl` element of a list item: it owns the marked content of the list label.
    pub label: Option<u32>,
    /// Marked-content tag name.
    pub tag: String,
    /// Heading level (the title for the outline), also for a numbered heading.
    pub heading: Option<u8>,
    /// The paragraph's space after, in points (Word's link rectangles include it on the last
    /// line).
    pub after: f64,
    /// The paragraph's space before, in points.
    pub before: f64,
    /// Contextual spacing is on, and the style id (spacing is dropped between paragraphs of one
    /// style).
    pub contextual: bool,
    pub style: Option<String>,
    /// First paragraph of its container (the body, a table cell): nothing above it.
    pub first: bool,
    /// The marked content of the paragraph sits in `Span` elements under it (Word's `TOCI` with
    /// a page number link).
    pub spans: bool,
    /// The font of the paragraph mark: Word draws the paragraph's blank in it, whatever the font
    /// of the last run.
    pub mark: Option<MarkFont>,
    /// The paragraph ends a section (`w:sectPr` in its properties): Word draws no blank for
    /// its mark.
    pub sect: bool,
    /// The list label is followed by a tab or a space (not `w:suff` `nothing`): Word draws a
    /// blank after the label, in the first marked content of the text.
    pub label_blank: bool,
    /// The paragraph is in a text frame: Word writes its element after the body and sets each
    /// line in a marked-content sequence of its own (`layout-frame-*`).
    pub framed: bool,
    /// Tab stops of the paragraph in points from the text area's left edge: the custom ones and
    /// the left indent of a hanging paragraph. Empty when only default stops apply.
    pub tabs: Vec<f64>,
}

/// The font a paragraph mark is set in.
#[derive(Debug, Clone)]
pub struct MarkFont {
    pub family: String,
    pub size: f64,
    pub bold: bool,
    pub italic: bool,
}

/// A DrawingML shape (`wps:wsp`) of a paragraph: what Word draws for a floating shape.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeSpec {
    /// `a:prstGeom/@prst`.
    pub geometry: String,
    /// Solid fill; `None` for no fill or a fill that is not a plain RGB colour.
    pub fill: Option<[u8; 3]>,
    /// Outline colour and width in points.
    pub line: Option<([u8; 3], f64)>,
    /// The shape is a text box (the layout reports it with the text boxes).
    pub text_box: bool,
}

fn child<'a>(e: &'a XmlElement, local: &str) -> Option<&'a XmlElement> {
    e.children.iter().find_map(|c| match c {
        RawXml::Element(x) if x.head.local_name() == local => Some(x),
        _ => None,
    })
}

fn find_deep<'a>(e: &'a XmlElement, local: &str) -> Option<&'a XmlElement> {
    for c in &e.children {
        if let RawXml::Element(x) = c {
            if x.head.local_name() == local {
                return Some(x);
            }
            if let Some(f) = find_deep(x, local) {
                return Some(f);
            }
        }
    }
    None
}

fn attr<'a>(e: &'a XmlElement, name: &str) -> Option<&'a str> {
    e.head
        .attrs
        .iter()
        .find(|a| a.local_name() == name)
        .map(|a| a.value.as_str())
}

fn hex_color(v: &str) -> Option<[u8; 3]> {
    if v.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(v, 16).ok()?;
    Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

fn solid(e: &XmlElement) -> Option<[u8; 3]> {
    let fill = child(e, "solidFill")?;
    attr(child(fill, "srgbClr")?, "val").and_then(hex_color)
}

/// The shape of a `w:drawing` (or `mc:AlternateContent`) element, if it holds a `wps:wsp`.
fn shape_of(e: &XmlElement) -> Option<ShapeSpec> {
    let wsp = find_deep(e, "wsp")?;
    let sp = child(wsp, "spPr")?;
    let line = child(sp, "ln");
    let line_fill = line.and_then(|l| {
        let c = solid(l)?;
        let w = attr(l, "w")
            .and_then(|w| w.parse::<f64>().ok())
            .unwrap_or(9525.0);
        Some((c, w / 12700.0))
    });
    Some(ShapeSpec {
        geometry: child(sp, "prstGeom")
            .and_then(|g| attr(g, "prst"))
            .unwrap_or("rect")
            .to_owned(),
        fill: solid(sp),
        line: line_fill,
        text_box: find_deep(wsp, "txbx").is_some(),
    })
}

/// The shapes in a paragraph's runs, in order (pictures are not shapes).
fn shapes_of(content: &[Inline], out: &mut Vec<ShapeSpec>) {
    for inline in content {
        match inline {
            Inline::Run(r) => {
                for c in &r.content {
                    if let RunContent::Raw(RawXml::Element(e)) = c
                        && (e.head.is_w("drawing") || e.head.local_name() == "AlternateContent")
                        && let Some(spec) = shape_of(e)
                    {
                        out.push(spec);
                    }
                }
            }
            Inline::Group(g) => shapes_of(&g.content, out),
            Inline::Raw(_) => {}
        }
    }
}

/// The wrapping of the anchored drawings in a paragraph's runs, in order.
fn anchor_wraps_of(content: &[Inline], out: &mut Vec<WrapType>) {
    for inline in content {
        match inline {
            Inline::Run(r) => {
                for c in &r.content {
                    let RunContent::Raw(RawXml::Element(e)) = c else {
                        continue;
                    };
                    let drawing = if e.head.is_w("drawing") {
                        open365_docx::Drawing::from_element(e)
                    } else {
                        open365_docx::Drawing::from_alternate_content(e)
                    };
                    if let Some(d) = drawing.filter(|d| d.anchored) {
                        out.push(d.wrap);
                    }
                }
            }
            Inline::Group(g) => anchor_wraps_of(&g.content, out),
            Inline::Raw(_) => {}
        }
    }
}

/// Where a hyperlink goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// An external address (`http`, `https`, `mailto`, `ftp`, `tel` only).
    Uri(String),
    /// A bookmark of the document.
    Anchor(String),
}

/// A hyperlink of a paragraph: the text it shows and its target.
#[derive(Debug, Clone)]
pub struct LinkSpec {
    pub text: String,
    pub target: LinkTarget,
    pub kind: LinkKind,
    /// The paragraph mark lies inside the link (a field result that goes on in the next
    /// paragraph): Word draws the paragraph's blank in the link's own `Span`.
    pub includes_mark: bool,
}

/// What made a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    /// A `w:hyperlink`.
    Hyperlink,
    /// A `REF \h` field: Word writes two annotations, one of them a zero-size one at the top
    /// left of the page (evidence: `fields`).
    Ref,
    /// A `PAGEREF \h` field: a `Link` element without an annotation.
    PageRef,
}

/// What the model says about the text of a paragraph: the text itself (cached field results
/// included, field codes left out), the byte range of each hyperlink (parallel to the
/// [`LinkSpec`]s) and the runs whose language differs from the document's.
#[derive(Debug, Default, Clone)]
pub struct ParaText {
    pub text: String,
    pub links: Vec<Range<usize>>,
    pub langs: Vec<(Range<usize>, String)>,
    /// Byte offsets in `text` where a footnote or endnote reference mark stands (the model
    /// holds no text for it; the layout draws the number there).
    pub note_marks: Vec<(usize, bool)>,
    /// The paragraph is part of a table of contents field.
    pub toc: bool,
    /// Language of the paragraph mark when it differs from the document's (the blank Word draws
    /// for the mark gets its own `Span /Lang`).
    pub mark_lang: Option<String>,
    /// The paragraph holds a comment reference: Word puts the paragraph mark's blank in a
    /// `CommentAnchor` element.
    pub comment_ref: bool,
    /// The font of the run each tab character stands in, in order of appearance (the blank
    /// Word draws for a tab has that run's font and size, whatever text is next to it).
    pub tabs: Vec<MarkFont>,
}

#[derive(Debug)]
pub struct DocStructure {
    pub root: Node,
    pub paras: Vec<ParaMeta>,
    /// Hyperlinks by main-story paragraph index, in order of appearance.
    pub links: HashMap<usize, Vec<LinkSpec>>,
    /// The model's text of each main-story paragraph with links.
    pub texts: HashMap<usize, ParaText>,
    /// Bookmark name to the paragraph index it starts in.
    pub bookmarks: HashMap<String, usize>,
    /// Hyperlinks dropped for an unsafe or unreadable target.
    pub dropped_links: usize,
    /// `/ID` strings of table headers and their elements, for the `IDTree`.
    pub ids: Vec<(String, u32)>,
    /// The text boxes of the main story in anchor order (the layout's frame ids): the index of
    /// the paragraph each is anchored in and its content.
    pub boxes: Vec<(usize, std::sync::Arc<open365_docx::TextBox>)>,
    /// The `Textbox` element of each box of `boxes`, after the paragraph the box is anchored in.
    pub box_elems: Vec<u32>,
    /// Paragraphs that carry a section break or sit in a cell covered by a vertical merge: Word writes no `P` for one
    /// that drew nothing.
    pub section_breaks: std::collections::HashSet<u32>,
    /// (story code, paragraph index in the notes story) -> index of the note it belongs to.
    pub note_of: HashMap<(u8, usize), usize>,
    /// The shapes of each main-story paragraph, in order.
    pub shapes: HashMap<usize, Vec<ShapeSpec>>,
    /// The wrapping of the anchored drawings of each main-story paragraph, in order.
    pub wraps: HashMap<usize, Vec<WrapType>>,
}

/// Reads hyperlinks, languages and text of paragraphs.
struct TextReader<'a> {
    rels: Option<&'a Relationships>,
    langs: &'a Languages,
    /// Primary subtag of the document language; text in it needs no `Span`.
    base: Option<&'a str>,
    dropped_links: usize,
    bookmarks: Vec<String>,
    /// Complex fields open at this point of the body (they span paragraphs).
    fields: Vec<OpenField>,
    /// Depth of `w:hyperlink` groups being read.
    in_hyperlink: usize,
    styles: Option<&'a Styles>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldKind {
    Toc,
    Ref,
    PageRef,
    Other,
}

struct OpenField {
    kind: FieldKind,
    instr: String,
    in_result: bool,
    /// Byte offset in the paragraph text where the result starts.
    start: usize,
    /// Bookmark a `REF` or `PAGEREF` field with `\h` points at.
    target: Option<String>,
}

fn field_link(kind: FieldKind, text: String, target: String, includes_mark: bool) -> LinkSpec {
    LinkSpec {
        text,
        target: LinkTarget::Anchor(target),
        kind: if kind == FieldKind::PageRef {
            LinkKind::PageRef
        } else {
            LinkKind::Ref
        },
        includes_mark,
    }
}

/// External targets a PDF viewer may be sent to; anything else (`javascript:`, `file:`, ...) is
/// dropped.
fn safe_uri(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    ["http:", "https:", "mailto:", "ftp:", "tel:"]
        .iter()
        .any(|p| lower.starts_with(p))
}

impl TextReader<'_> {
    fn paragraph(&mut self, para: &Paragraph) -> (ParaText, Vec<LinkSpec>) {
        let mut out = ParaText {
            toc: self
                .fields
                .iter()
                .any(|f| f.kind == FieldKind::Toc && f.in_result),
            ..ParaText::default()
        };
        out.mark_lang = self
            .langs
            .of_paragraph(para)
            .and_then(primary)
            .filter(|l| Some(l.as_str()) != self.base);
        let mut links = Vec::new();
        self.inlines(para, &para.content, &mut out, &mut links);
        // A field whose result goes on in the next paragraph: the mark is inside the link.
        let end = out.text.len();
        for f in &mut self.fields {
            if f.in_result
                && let Some(target) = f.target.clone()
                && end > f.start
                && self.in_hyperlink == 0
            {
                let text = out.text[f.start..end].to_owned();
                out.links.push(f.start..end);
                links.push(field_link(f.kind, text, target, true));
            }
            f.start = 0;
        }
        (out, links)
    }

    fn field_char(&mut self, kind: &str, out: &mut ParaText, links: &mut Vec<LinkSpec>) {
        match kind {
            "begin" => self.fields.push(OpenField {
                kind: FieldKind::Other,
                instr: String::new(),
                in_result: false,
                start: 0,
                target: None,
            }),
            "separate" => {
                let at = out.text.len();
                if let Some(f) = self.fields.last_mut() {
                    let words: Vec<&str> = f.instr.split_whitespace().collect();
                    f.kind = match words.first().map(|w| w.to_ascii_uppercase()).as_deref() {
                        Some("TOC") => FieldKind::Toc,
                        Some("REF") => FieldKind::Ref,
                        Some("PAGEREF") => FieldKind::PageRef,
                        _ => FieldKind::Other,
                    };
                    if matches!(f.kind, FieldKind::Ref | FieldKind::PageRef)
                        && words.iter().any(|w| w.eq_ignore_ascii_case("\\h"))
                    {
                        f.target = words.get(1).map(|w| (*w).to_owned());
                    }
                    f.in_result = true;
                    f.start = at;
                    if f.kind == FieldKind::Toc {
                        out.toc = true;
                    }
                }
            }
            "end" => {
                let Some(f) = self.fields.pop() else { return };
                let end = out.text.len();
                if f.in_result
                    && let Some(target) = f.target
                    && end > f.start
                    && self.in_hyperlink == 0
                {
                    let text = out.text[f.start..end].to_owned();
                    out.links.push(f.start..end);
                    links.push(field_link(f.kind, text, target, false));
                }
            }
            _ => {}
        }
    }

    fn run(&mut self, para: &Paragraph, run: &Run, out: &mut ParaText, links: &mut Vec<LinkSpec>) {
        let start = out.text.len();
        for c in &run.content {
            match c {
                RunContent::Raw(RawXml::Element(e)) if e.head.is_w("fldChar") => {
                    let kind = e.head.attr_w("fldCharType").unwrap_or("").to_owned();
                    self.field_char(&kind, out, links);
                }
                RunContent::Raw(RawXml::Element(e)) if e.head.is_w("commentReference") => {
                    out.comment_ref = true;
                }
                RunContent::Raw(RawXml::Element(e)) if e.head.is_w("instrText") => {
                    if let Some(f) = self.fields.last_mut().filter(|f| !f.in_result) {
                        for child in &e.children {
                            if let RawXml::Text(t) = child {
                                f.instr.push_str(t);
                            }
                        }
                    }
                }
                RunContent::Text { text, .. } => out.text.push_str(text),
                RunContent::Tab => {
                    out.text.push('\t');
                    if let Some(s) = self.styles {
                        let er = s.effective_run_props(para, run);
                        out.tabs.push(MarkFont {
                            family: er.font,
                            size: f64::from(er.size_half_points) / 2.0,
                            bold: er.bold,
                            italic: er.italic,
                        });
                    }
                }
                RunContent::Break(b) => out.text.push(match b.kind() {
                    BreakKind::TextWrapping => '\u{b}',
                    BreakKind::Page => '\u{c}',
                    BreakKind::Column => '\u{e}',
                }),
                RunContent::Raw(RawXml::Element(e))
                    if e.head.is_w("footnoteReference") || e.head.is_w("endnoteReference") =>
                {
                    out.note_marks
                        .push((out.text.len(), e.head.is_w("endnoteReference")));
                }
                RunContent::Raw(_) => {}
            }
        }
        let end = out.text.len();
        if end == start {
            return;
        }
        let Some(lang) = self.langs.of_run(para, run).as_deref().and_then(primary) else {
            return;
        };
        if Some(lang.as_str()) == self.base {
            return;
        }
        match out.langs.last_mut() {
            Some((r, l)) if r.end == start && *l == lang => r.end = end,
            _ => out.langs.push((start..end, lang)),
        }
    }

    fn inlines(
        &mut self,
        para: &Paragraph,
        content: &[Inline],
        out: &mut ParaText,
        links: &mut Vec<LinkSpec>,
    ) {
        for inline in content {
            match inline {
                Inline::Run(r) => self.run(para, r, out, links),
                Inline::Group(g) if g.head.is_w("hyperlink") => {
                    let start = out.text.len();
                    self.in_hyperlink += 1;
                    self.inlines(para, &g.content, out, links);
                    self.in_hyperlink -= 1;
                    let end = out.text.len();
                    let text = out.text[start..end].to_owned();
                    let anchor = g.head.attr_w("anchor").map(str::to_owned);
                    let rel_id = g
                        .head
                        .attrs
                        .iter()
                        .find(|a| {
                            a.local_name() == "id"
                                && a.name.contains(':')
                                && !a.name.starts_with("w:")
                        })
                        .map(|a| a.value.clone());
                    let target = match rel_id {
                        Some(id) => self
                            .rels
                            .and_then(|r| r.get(&id))
                            .filter(|r| r.target_mode == TargetMode::External)
                            .map(|r| match &anchor {
                                Some(a) => format!("{}#{a}", r.target),
                                None => r.target.clone(),
                            })
                            .filter(|u| safe_uri(u))
                            .map(LinkTarget::Uri),
                        None => anchor.map(LinkTarget::Anchor),
                    };
                    match target {
                        Some(target) if !text.trim().is_empty() => {
                            out.links.push(start..end);
                            links.push(LinkSpec {
                                text,
                                target,
                                kind: LinkKind::Hyperlink,
                                includes_mark: false,
                            });
                        }
                        _ => self.dropped_links += 1,
                    }
                }
                Inline::Group(g) => self.inlines(para, &g.content, out, links),
                Inline::Raw(RawXml::Element(e)) if e.head.is_w("bookmarkStart") => {
                    if let Some(name) = e.head.attr_w("name") {
                        self.bookmarks.push(name.to_owned());
                    }
                }
                Inline::Raw(_) => {}
            }
        }
    }
}

struct Walker<'a> {
    pdf: &'a mut Pdf,
    roles: &'a HashMap<usize, ParaRole>,
    styles: Option<&'a Styles>,
    numbering: Option<&'a Numbering>,
    reader: TextReader<'a>,
    next_para: usize,
    paras: Vec<ParaMeta>,
    links: HashMap<usize, Vec<LinkSpec>>,
    texts: HashMap<usize, ParaText>,
    bookmarks: HashMap<String, usize>,
    table_no: usize,
    /// Tables (document-order ids) with a drawn border or fill: Word tags only those as tables.
    tagged_tables: &'a std::collections::HashSet<usize>,
    /// Nesting depth of the table being read (0 outside tables).
    depth: usize,
    /// Nesting depth of layout (untagged) tables being read.
    flat: usize,
    /// Inside a cell that a vertical merge covers.
    vcont: usize,
    ids: Vec<(String, u32)>,
    boxes: Vec<(usize, std::sync::Arc<open365_docx::TextBox>)>,
    box_elems: Vec<u32>,
    section_breaks: std::collections::HashSet<u32>,
    shapes: HashMap<usize, Vec<ShapeSpec>>,
    wraps: HashMap<usize, Vec<WrapType>>,
    /// Elements of paragraphs in text frames, written after the body.
    frame_tail: Vec<Node>,
}

/// The tab stops of a paragraph other than the default ones: custom stops, and the left indent
/// of a hanging paragraph (an implicit stop), in points from the text area's left edge.
fn tab_stops(styles: &Styles, para: &Paragraph) -> Vec<f64> {
    let mut out: Vec<f64> = styles
        .paragraph_flow_props(para)
        .tabs
        .iter()
        .filter(|t| !matches!(t.kind, open365_docx::TabKind::Bar))
        .map(|t| f64::from(t.pos_twips) / 20.0)
        .collect();
    let e = styles.effective_para_props(para);
    if e.first_line_pt < 0.0 {
        out.push(f64::from(e.indent_left_pt));
    }
    out.sort_by(f64::total_cmp);
    out
}

/// The paragraph ends a section (`w:sectPr` in its properties).
fn has_sect_pr(para: &Paragraph) -> bool {
    para.props.as_ref().is_some_and(|p| {
        p.children
            .iter()
            .any(|c| matches!(c, RawXml::Element(e) if e.head.is_w("sectPr")))
    })
}

/// The paragraph is a drop cap (`w:framePr w:dropCap`).
fn has_drop_cap(para: &Paragraph) -> bool {
    para.props.as_ref().is_some_and(|p| {
        p.children.iter().any(|c| {
            matches!(c, RawXml::Element(e) if e.head.is_w("framePr")
                && e.head.attr_w("dropCap").is_some_and(|v| v != "none"))
        })
    })
}

/// The paragraph is in a text frame (`w:framePr`) that is not a drop cap.
fn has_frame(para: &Paragraph) -> bool {
    para.props.as_ref().is_some_and(|p| {
        p.children.iter().any(|c| {
            matches!(c, RawXml::Element(e) if e.head.is_w("framePr")
                && e.head.attr_w("dropCap").is_none_or(|v| v == "none"))
        })
    })
}

fn pstyle(para: &Paragraph) -> Option<&str> {
    para.props.as_ref()?.children.iter().find_map(|c| match c {
        RawXml::Element(e) if e.head.is_w("pStyle") => e.head.attr_w("val"),
        _ => None,
    })
}

/// The level of a heading style (`heading 1`..`heading 9`, through `basedOn`).
fn heading_level(styles: Option<&Styles>, para: &Paragraph) -> Option<u8> {
    let styles = styles?;
    let mut at = pstyle(para).map(str::to_owned);
    for _ in 0..32 {
        let style = styles.style(&at?)?;
        let name = style.name.to_ascii_lowercase();
        if let Some(n) = name
            .strip_prefix("heading ")
            .and_then(|n| n.trim().parse::<u8>().ok())
            .filter(|n| (1..=9).contains(n))
        {
            return Some(n.min(6));
        }
        at = style.based_on.clone();
    }
    None
}

/// The innermost open list of the last list in `out`, `depth` lists deep: each nested list hangs
/// in the `LBody` of the last item of the one around it.
fn deepest(out: &mut [Node], depth: usize) -> &mut Node {
    let mut node = out.last_mut().expect("an open list");
    for _ in 1..depth {
        let li = node.kids.last_mut().expect("a list item");
        let body = li.kids.last_mut().expect("a list body");
        node = body.kids.last_mut().expect("a nested list");
    }
    node
}

/// The text boxes anchored in a paragraph's runs, in order.
fn text_boxes_of(content: &[Inline], out: &mut Vec<std::sync::Arc<open365_docx::TextBox>>) {
    for inline in content {
        match inline {
            Inline::Run(r) => {
                for c in &r.content {
                    let RunContent::Raw(RawXml::Element(e)) = c else {
                        continue;
                    };
                    let drawing = if e.head.is_w("drawing") {
                        open365_docx::Drawing::from_element(e)
                    } else {
                        open365_docx::Drawing::from_alternate_content(e)
                    };
                    if let Some(tb) = drawing.and_then(|d| d.text_box) {
                        out.push(tb);
                    }
                }
            }
            Inline::Group(g) => text_boxes_of(&g.content, out),
            Inline::Raw(_) => {}
        }
    }
}

/// Word's `ListNumbering` value for a number format.
fn list_numbering(fmt: &str) -> &'static str {
    match fmt {
        "decimal" => "Decimal",
        "lowerLetter" => "LowerAlpha",
        "upperLetter" => "UpperAlpha",
        "lowerRoman" => "LowerRoman",
        "upperRoman" => "UpperRoman",
        "bullet" => "Disc",
        _ => "None",
    }
}

impl Walker<'_> {
    /// (list id, level, `ListNumbering`) of a list paragraph (evidence: `layout-lists-formats`,
    /// `layout-lists-multilevel`, `numbering`).
    fn list_info(&self, p: &Paragraph) -> (i64, usize, &'static str) {
        let Some(n) = self.numbering else {
            return (0, 0, "Decimal");
        };
        let Some(at) = n.paragraph_numbering(self.styles, p) else {
            return (0, 0, "Decimal");
        };
        let fmt = n
            .resolve(self.styles, at.num_id)
            .and_then(|r| r.level(at.ilvl))
            .map_or("decimal", |l| l.num_fmt.as_str());
        (at.num_id, at.ilvl, list_numbering(fmt))
    }

    /// The paragraph's list level puts a tab or a space after the label.
    fn label_suffix_blank(&self, p: &Paragraph) -> bool {
        let Some(n) = self.numbering else {
            return false;
        };
        let Some(at) = n.paragraph_numbering(self.styles, p) else {
            return false;
        };
        n.resolve(self.styles, at.num_id)
            .and_then(|r| r.level(at.ilvl))
            .is_some_and(|l| l.suffix != open365_docx::LevelSuffix::Nothing)
    }

    fn node(&mut self, kind: Kind, kids: Vec<Node>) -> Node {
        Node {
            kind,
            id: self.pdf.reserve(),
            kids,
            attrs: String::new(),
        }
    }

    fn blocks(&mut self, blocks: &[Block]) -> Vec<Node> {
        let mut out: Vec<Node> = Vec::new();
        // The open lists of the last list in `out`: (list id, level), outermost first.
        let mut open: Vec<(i64, usize)> = Vec::new();
        for block in blocks {
            if !matches!(block, Block::Paragraph(_)) {
                open.clear();
            }
            match block {
                Block::Paragraph(p) => {
                    let index = self.next_para;
                    self.next_para += 1;
                    let (text, links) = self.reader.paragraph(p);
                    let mut found = Vec::new();
                    text_boxes_of(&p.content, &mut found);
                    let mut box_nodes = Vec::new();
                    for b in found {
                        let node = self.node(Kind::Textbox, Vec::new());
                        self.box_elems.push(node.id);
                        box_nodes.push(node);
                        self.boxes.push((index, b));
                    }
                    let mut shapes = Vec::new();
                    shapes_of(&p.content, &mut shapes);
                    if !shapes.is_empty() {
                        self.shapes.insert(index, shapes);
                    }
                    let mut wraps = Vec::new();
                    anchor_wraps_of(&p.content, &mut wraps);
                    if !wraps.is_empty() {
                        self.wraps.insert(index, wraps);
                    }
                    for name in self.reader.bookmarks.drain(..) {
                        self.bookmarks.entry(name).or_insert(index);
                    }
                    if !links.is_empty() {
                        self.links.insert(index, links);
                    }
                    self.texts.insert(index, text);
                    let role = self.roles.get(&index).copied().unwrap_or_default();
                    let by_style = heading_level(self.styles, p);
                    // A numbered heading is a heading (the label is part of its content), not a
                    // list item.
                    let toc = self.texts[&index].toc;
                    let (kind, labeled, heading) = match (role, by_style) {
                        _ if toc => (Kind::Toci, false, None),
                        (ParaRole::Heading(n), _) => (Kind::H(n), false, Some(n)),
                        (_, Some(n)) => (Kind::H(n), false, Some(n)),
                        (ParaRole::ListItem, None) => (Kind::LBody, true, None),
                        (ParaRole::Plain, None) => (Kind::P, false, None),
                    };
                    let leaf = self.node(kind, Vec::new());
                    if has_sect_pr(p)
                        || self.vcont > 0
                        || !self.texts[&index].text.trim().is_empty()
                    {
                        self.section_breaks.insert(leaf.id);
                    }
                    let lbl = labeled.then(|| self.node(Kind::Lbl, Vec::new()));
                    self.paras.push(ParaMeta {
                        node: leaf.id,
                        label: lbl.as_ref().map(|l| l.id),
                        tag: if kind == Kind::Toci {
                            "Span".to_owned()
                        } else {
                            kind.name()
                        },
                        spans: kind == Kind::Toci && self.links.contains_key(&index),
                        heading,
                        after: self
                            .styles
                            .map_or(0.0, |s| f64::from(s.effective_para_props(p).space_after_pt)),
                        before: self.styles.map_or(0.0, |s| {
                            f64::from(s.effective_para_props(p).space_before_pt)
                        }),
                        contextual: self
                            .styles
                            .is_some_and(|s| s.paragraph_flow_props(p).contextual_spacing),
                        style: pstyle(p).map(str::to_owned),
                        first: out.is_empty(),
                        sect: has_sect_pr(p),
                        label_blank: self.label_suffix_blank(p),
                        framed: self.depth == 0 && has_frame(p),
                        tabs: self.styles.map_or_else(Vec::new, |st| tab_stops(st, p)),
                        mark: self.styles.map(|s| {
                            let m = s.paragraph_mark_props(p).run;
                            MarkFont {
                                family: m.font,
                                size: f64::from(m.size_half_points) / 2.0,
                                bold: m.bold,
                                italic: m.italic,
                            }
                        }),
                    });
                    if let Some(lbl) = lbl {
                        let (num_id, ilvl, fmt) = self.list_info(p);
                        let li = self.node(Kind::Li, vec![lbl, leaf]);
                        if !matches!(out.last(), Some(l) if l.kind == Kind::L) {
                            open.clear();
                        }
                        while open.len() > 1 && open.last().is_some_and(|o| o.1 > ilvl) {
                            open.pop();
                        }
                        if open.first().is_some_and(|o| o.0 != num_id) {
                            open.clear();
                        }
                        let nest = open.last().is_some_and(|o| o.1 < ilvl);
                        let new_list = open.is_empty() || nest;
                        let list = new_list.then(|| {
                            let mut list = self.node(Kind::L, Vec::new());
                            list.attrs = format!("/A << /O /List /ListNumbering /{fmt} >>");
                            list
                        });
                        if new_list {
                            if open.is_empty() {
                                // Hung after the paragraphs before it.
                                let mut list = list.expect("new list");
                                list.kids.push(li);
                                out.push(list);
                            } else {
                                let mut list = list.expect("new list");
                                list.kids.push(li);
                                let item = deepest(&mut out, open.len())
                                    .kids
                                    .last_mut()
                                    .expect("a list item");
                                item.kids.last_mut().expect("a list body").kids.push(list);
                            }
                            open.push((num_id, ilvl));
                        } else {
                            deepest(&mut out, open.len()).kids.push(li);
                        }
                    } else if kind == Kind::Toci {
                        match out.last_mut() {
                            Some(t) if t.kind == Kind::Toc => t.kids.push(leaf),
                            _ => {
                                let t = self.node(Kind::Toc, vec![leaf]);
                                out.push(t);
                            }
                        }
                    } else if kind == Kind::P && self.depth == 0 && has_frame(p) {
                        self.frame_tail.push(leaf);
                    } else if kind == Kind::P && has_drop_cap(p) {
                        // Word wraps the paragraph of a drop cap in a `P` of its own
                        // (`layout-frame-dropcap`).
                        let outer = self.node(Kind::P, vec![leaf]);
                        out.push(outer);
                    } else {
                        out.push(leaf);
                    }
                    // Word writes the text boxes of a paragraph after its `P`.
                    out.extend(box_nodes);
                }
                Block::Table(t) => out.extend(self.table(t)),
                // The paragraphs of a content control follow the layout's order in place.
                Block::Sdt(sdt) => {
                    for content in sdt.content() {
                        out.extend(self.blocks(&content.blocks));
                    }
                }
                Block::Raw(_) => {}
            }
        }
        out
    }

    /// The elements of a table: one `Table` element, or, for a table that draws no border and no
    /// fill (a layout table), the elements of its cells' paragraphs, as Word writes them
    /// (evidence: `p-tshd`, `p-thdr`, `p-tcell`, `p-tmix` and the layout fixtures). A table inside a
    /// tagged table is always a table, with rows directly under it (`p-tnest`).
    fn table(&mut self, t: &open365_docx::Table) -> Vec<Node> {
        let view = t.view();
        self.table_no += 1;
        let no = self.table_no;
        // A table inside any table is a table (`p-tnest`, `layout-table-flow`); only inside a
        // tagged table are its rows directly under it, inside a layout table it is a plain one.
        let inside = self.depth > 0;
        let nested = self.depth > self.flat;
        if !inside && !self.tagged_tables.contains(&(no - 1)) {
            let mut flat = Vec::new();
            self.depth += 1;
            self.flat += 1;
            for row in &view.rows {
                for c in &row.cells {
                    self.vcont += usize::from(c.row_span == 0);
                    flat.extend(self.blocks(&c.cell.blocks));
                    self.vcont -= usize::from(c.row_span == 0);
                }
            }
            self.flat -= 1;
            self.depth -= 1;
            return flat;
        }
        let look = view.props.look.unwrap_or_default();
        let mut head_rows = 0;
        for (r, row) in view.rows.iter().enumerate() {
            // Word ignores the header row flag when the look turns the first row off
            // (evidence: `p-thead`, case I).
            if look.first_row && (r == 0 || (r == head_rows && row.props.is_header)) {
                head_rows += 1;
            } else {
                break;
            }
        }
        // Column headers: (first column, span, id).
        let mut col_heads: Vec<(usize, usize, String)> = Vec::new();
        let mut head_nodes = Vec::new();
        let mut body_nodes = Vec::new();
        for (r, row) in view.rows.iter().enumerate() {
            let in_head = r < head_rows;
            let row_head_id = (!in_head && look.first_column)
                .then(|| row.cells.iter().find(|c| c.col == 0))
                .flatten()
                .map(|_| format!("t{no}r{r}c0"));
            let mut cell_nodes = Vec::new();
            // The cells a vertical merge covers: Word writes them as empty `TD` elements in
            // front of the other cells of the row (`p-vmerge`, `table-merged-cells`).
            let mut covered = Vec::new();
            for c in &row.cells {
                self.depth += 1;
                self.vcont += usize::from(c.row_span == 0);
                let kids = self.blocks(&c.cell.blocks);
                self.vcont -= usize::from(c.row_span == 0);
                self.depth -= 1;
                // A cell covered by a vertical merge: its origin carries the RowSpan.
                if c.row_span == 0 {
                    covered.push(self.node(Kind::Td, Vec::new()));
                    continue;
                }
                let is_row_head = !in_head && look.first_column && c.col == 0;
                let th = in_head || is_row_head;
                let mut entries: Vec<String> = Vec::new();
                let mut attrs = String::new();
                let id = format!("t{no}r{r}c{}", c.col);
                if th {
                    entries.push(format!(
                        "/Scope /{}",
                        if in_head { "Column" } else { "Row" }
                    ));
                    attrs.push_str(&format!("/ID {} ", text_string(&id)));
                } else {
                    let mut headers: Vec<&str> = col_heads
                        .iter()
                        .filter(|(col, span, _)| *col < c.col + c.span && c.col < col + span)
                        .map(|(_, _, id)| id.as_str())
                        .collect();
                    if let Some(h) = &row_head_id {
                        headers.push(h);
                    }
                    if !headers.is_empty() {
                        let list: Vec<String> = headers.iter().map(|h| text_string(h)).collect();
                        entries.push(format!("/Headers [{}]", list.join(" ")));
                    }
                }
                if c.span > 1 {
                    entries.push(format!("/ColSpan {}", c.span));
                }
                if c.row_span > 1 {
                    entries.push(format!("/RowSpan {}", c.row_span));
                }
                if !entries.is_empty() {
                    attrs.push_str(&format!("/A << /O /Table {} >>", entries.join(" ")));
                }
                let mut node = self.node(if th { Kind::Th } else { Kind::Td }, kids);
                node.attrs = attrs.trim_end().to_owned();
                if th {
                    self.ids.push((id.clone(), node.id));
                    if in_head {
                        col_heads.push((c.col, c.span, id));
                    }
                }
                cell_nodes.push(node);
            }
            covered.extend(cell_nodes);
            let tr = self.node(Kind::Tr, covered);
            if in_head {
                head_nodes.push(tr);
            } else {
                body_nodes.push(tr);
            }
        }
        let mut sections = Vec::new();
        if nested {
            // Rows directly under a nested table, its header cells still TH (`p-thead-nest`).
            sections = head_nodes;
            sections.extend(body_nodes);
        } else {
            if !head_nodes.is_empty() {
                sections.push(self.node(Kind::THead, head_nodes));
            }
            if !body_nodes.is_empty() {
                sections.push(self.node(Kind::TBody, body_nodes));
            }
        }
        let mut table = self.node(Kind::Table, sections);
        if let Some(desc) = table_description(t) {
            table.attrs = format!(
                "/Alt {} /A << /O /Table /Summary {} >>",
                text_string(&desc),
                text_string(&desc)
            );
        }
        vec![table]
    }
}

/// The table's description (`w:tblDescription`), which Word writes as the table's `/Alt`.
fn table_description(t: &open365_docx::Table) -> Option<String> {
    t.children.iter().find_map(|c| match c {
        TableChild::Raw(RawXml::Element(pr)) if pr.head.is_w("tblPr") => {
            pr.children.iter().find_map(|c| match c {
                RawXml::Element(d) if d.head.is_w("tblDescription") => d
                    .head
                    .attr_w("val")
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_owned),
                _ => None,
            })
        }
        _ => None,
    })
}

/// Walks the main story in the order the layout numbers paragraphs. `roles` maps a paragraph
/// index to its layout role; a paragraph without one is plain. `base` is the primary language
/// subtag of the document: text in another language gets a `Span` with its own `/Lang`.
pub fn analyse(
    doc: &Document,
    roles: &HashMap<usize, ParaRole>,
    rels: Option<&Relationships>,
    langs: &Languages,
    base: Option<&str>,
    tagged_tables: &std::collections::HashSet<usize>,
    pdf: &mut Pdf,
) -> DocStructure {
    let mut walker = Walker {
        pdf,
        roles,
        styles: doc.styles(),
        numbering: doc.numbering(),
        reader: TextReader {
            rels,
            langs,
            base,
            dropped_links: 0,
            bookmarks: Vec::new(),
            fields: Vec::new(),
            in_hyperlink: 0,
            styles: doc.styles(),
        },
        next_para: 0,
        paras: Vec::new(),
        links: HashMap::new(),
        texts: HashMap::new(),
        bookmarks: HashMap::new(),
        table_no: 0,
        tagged_tables,
        depth: 0,
        flat: 0,
        vcont: 0,
        ids: Vec::new(),
        boxes: Vec::new(),
        box_elems: Vec::new(),
        section_breaks: std::collections::HashSet::new(),
        shapes: HashMap::new(),
        wraps: HashMap::new(),
        frame_tail: Vec::new(),
    };
    let mut kids = walker.blocks(&doc.body().blocks);
    // Word hangs the paragraphs of text frames under the last paragraph of the body.
    match kids.last_mut() {
        Some(last) if last.kind == Kind::P => last.kids.append(&mut walker.frame_tail),
        _ => kids.append(&mut walker.frame_tail),
    }
    let root = walker.node(Kind::Document, kids);
    DocStructure {
        root,
        paras: walker.paras,
        links: walker.links,
        texts: walker.texts,
        bookmarks: walker.bookmarks,
        dropped_links: walker.reader.dropped_links,
        ids: walker.ids,
        boxes: walker.boxes,
        box_elems: walker.box_elems,
        section_breaks: walker.section_breaks,
        note_of: HashMap::new(),
        shapes: walker.shapes,
        wraps: walker.wraps,
    }
}

/// The text and hyperlinks of the paragraphs of another story (a header, footer or notes part),
/// in the order the layout numbers them.
pub fn story_texts(
    blocks: &[Block],
    rels: Option<&Relationships>,
    langs: &Languages,
    base: Option<&str>,
) -> Vec<(ParaText, Vec<LinkSpec>)> {
    fn walk(
        blocks: &[Block],
        reader: &mut TextReader<'_>,
        out: &mut Vec<(ParaText, Vec<LinkSpec>)>,
    ) {
        for block in blocks {
            match block {
                Block::Paragraph(p) => {
                    let r = reader.paragraph(p);
                    reader.bookmarks.clear();
                    out.push(r);
                }
                Block::Table(t) => {
                    for row in t.view().rows {
                        for c in row.cells {
                            walk(&c.cell.blocks, reader, out);
                        }
                    }
                }
                Block::Sdt(sdt) => {
                    for content in sdt.content() {
                        walk(&content.blocks, reader, out);
                    }
                }
                Block::Raw(_) => {}
            }
        }
    }
    let mut reader = TextReader {
        rels,
        langs,
        base,
        dropped_links: 0,
        bookmarks: Vec::new(),
        fields: Vec::new(),
        in_hyperlink: 0,
        styles: None,
    };
    let mut out = Vec::new();
    walk(blocks, &mut reader, &mut out);
    out
}

/// The spacing facts of the paragraphs of another story (a text box), in the order the layout
/// numbers them, for the link rectangles. Nothing here owns marked content.
pub fn story_spacing(styles: Option<&Styles>, blocks: &[Block]) -> Vec<ParaMeta> {
    fn walk(styles: Option<&Styles>, blocks: &[Block], out: &mut Vec<ParaMeta>) {
        for (at, block) in blocks.iter().enumerate() {
            match block {
                Block::Paragraph(p) => out.push(ParaMeta {
                    node: 0,
                    label: None,
                    tag: String::new(),
                    heading: None,
                    after: styles
                        .map_or(0.0, |s| f64::from(s.effective_para_props(p).space_after_pt)),
                    before: styles.map_or(0.0, |s| {
                        f64::from(s.effective_para_props(p).space_before_pt)
                    }),
                    contextual: styles
                        .is_some_and(|s| s.paragraph_flow_props(p).contextual_spacing),
                    style: pstyle(p).map(str::to_owned),
                    first: at == 0,
                    spans: false,
                    mark: None,
                    sect: false,
                    label_blank: false,
                    framed: false,
                    tabs: Vec::new(),
                }),
                Block::Table(t) => {
                    for row in t.view().rows {
                        for c in row.cells {
                            walk(styles, &c.cell.blocks, out);
                        }
                    }
                }
                Block::Sdt(sdt) => {
                    for content in sdt.content() {
                        walk(styles, &content.blocks, out);
                    }
                }
                Block::Raw(_) => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(styles, blocks, &mut out);
    out
}
