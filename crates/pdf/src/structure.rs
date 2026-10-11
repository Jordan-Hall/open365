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
    Block, BreakKind, Document, Inline, Paragraph, RawXml, Run, RunContent, Styles, TableChild,
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
}

/// What the model says about the text of a paragraph: the text itself (cached field results
/// included, field codes left out), the byte range of each hyperlink (parallel to the
/// [`LinkSpec`]s) and the runs whose language differs from the document's.
#[derive(Debug, Default, Clone)]
pub struct ParaText {
    pub text: String,
    pub links: Vec<Range<usize>>,
    pub langs: Vec<(Range<usize>, String)>,
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
}

/// Reads hyperlinks, languages and text of paragraphs.
struct TextReader<'a> {
    rels: Option<&'a Relationships>,
    langs: &'a Languages,
    /// Primary subtag of the document language; text in it needs no `Span`.
    base: Option<&'a str>,
    dropped_links: usize,
    bookmarks: Vec<String>,
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
        let mut out = ParaText::default();
        let mut links = Vec::new();
        self.inlines(para, &para.content, &mut out, &mut links);
        (out, links)
    }

    fn run(&self, para: &Paragraph, run: &Run, out: &mut ParaText) {
        let start = out.text.len();
        for c in &run.content {
            match c {
                RunContent::Text { text, .. } => out.text.push_str(text),
                RunContent::Tab => out.text.push('\t'),
                RunContent::Break(b) => out.text.push(match b.kind() {
                    BreakKind::TextWrapping => '\u{b}',
                    BreakKind::Page => '\u{c}',
                    BreakKind::Column => '\u{e}',
                }),
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
                Inline::Run(r) => self.run(para, r, out),
                Inline::Group(g) if g.head.is_w("hyperlink") => {
                    let start = out.text.len();
                    self.inlines(para, &g.content, out, links);
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
                            links.push(LinkSpec { text, target });
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
    reader: TextReader<'a>,
    next_para: usize,
    paras: Vec<ParaMeta>,
    links: HashMap<usize, Vec<LinkSpec>>,
    texts: HashMap<usize, ParaText>,
    bookmarks: HashMap<String, usize>,
    table_no: usize,
    ids: Vec<(String, u32)>,
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

impl Walker<'_> {
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
        for block in blocks {
            match block {
                Block::Paragraph(p) => {
                    let index = self.next_para;
                    self.next_para += 1;
                    let (text, links) = self.reader.paragraph(p);
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
                    let (kind, labeled, heading) = match (role, by_style) {
                        (ParaRole::Heading(n), _) => (Kind::H(n), false, Some(n)),
                        (_, Some(n)) => (Kind::H(n), false, Some(n)),
                        (ParaRole::ListItem, None) => (Kind::LBody, true, None),
                        (ParaRole::Plain, None) => (Kind::P, false, None),
                    };
                    let leaf = self.node(kind, Vec::new());
                    let lbl = labeled.then(|| self.node(Kind::Lbl, Vec::new()));
                    self.paras.push(ParaMeta {
                        node: leaf.id,
                        label: lbl.as_ref().map(|l| l.id),
                        tag: kind.name(),
                        heading,
                    });
                    if let Some(lbl) = lbl {
                        let li = self.node(Kind::Li, vec![lbl, leaf]);
                        match out.last_mut() {
                            Some(l) if l.kind == Kind::L => l.kids.push(li),
                            _ => {
                                let mut list = self.node(Kind::L, vec![li]);
                                list.attrs = "/A << /O /List /ListNumbering /Decimal >>".into();
                                out.push(list);
                            }
                        }
                    } else {
                        out.push(leaf);
                    }
                }
                Block::Table(t) => out.push(self.table(t)),
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

    fn table(&mut self, t: &open365_docx::Table) -> Node {
        let view = t.view();
        self.table_no += 1;
        let no = self.table_no;
        let look = view.props.look.unwrap_or_default();
        let mut head_rows = 0;
        for (r, row) in view.rows.iter().enumerate() {
            if (r == 0 && look.first_row) || (r == head_rows && row.props.is_header) {
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
            for c in &row.cells {
                let kids = self.blocks(&c.cell.blocks);
                // A cell covered by a vertical merge: its origin carries the RowSpan.
                if c.row_span == 0 {
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
            let tr = self.node(Kind::Tr, cell_nodes);
            if in_head {
                head_nodes.push(tr);
            } else {
                body_nodes.push(tr);
            }
        }
        let mut sections = Vec::new();
        if !head_nodes.is_empty() {
            sections.push(self.node(Kind::THead, head_nodes));
        }
        if !body_nodes.is_empty() {
            sections.push(self.node(Kind::TBody, body_nodes));
        }
        let mut table = self.node(Kind::Table, sections);
        if let Some(desc) = table_description(t) {
            table.attrs = format!(
                "/Alt {} /A << /O /Table /Summary {} >>",
                text_string(&desc),
                text_string(&desc)
            );
        }
        table
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
    pdf: &mut Pdf,
) -> DocStructure {
    let mut walker = Walker {
        pdf,
        roles,
        styles: doc.styles(),
        reader: TextReader {
            rels,
            langs,
            base,
            dropped_links: 0,
            bookmarks: Vec::new(),
        },
        next_para: 0,
        paras: Vec::new(),
        links: HashMap::new(),
        texts: HashMap::new(),
        bookmarks: HashMap::new(),
        table_no: 0,
        ids: Vec::new(),
    };
    let kids = walker.blocks(&doc.body().blocks);
    let root = walker.node(Kind::Document, kids);
    DocStructure {
        root,
        paras: walker.paras,
        links: walker.links,
        texts: walker.texts,
        bookmarks: walker.bookmarks,
        dropped_links: walker.reader.dropped_links,
        ids: walker.ids,
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
    };
    let mut out = Vec::new();
    walk(blocks, &mut reader, &mut out);
    out
}
