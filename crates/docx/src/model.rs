//! Typed model of the main story (ECMA-376 Part 1 §17.2-§17.4). Everything that is not
//! modelled stays as [`RawXml`] in its original position, so an unedited tree serialises
//! back to an infoset-equal document.

use crate::props::RunProps;
use crate::xml::{
    Attr, ElementHead, RawXml, XmlElement, is_w_ns, sibling_name, write_end, write_head,
};

/// `w:document`. Children other than the first `w:body` stay raw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentRoot {
    pub head: ElementHead,
    pub children: Vec<DocChild>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocChild {
    Body(Body),
    Raw(RawXml),
}

/// `w:body`. Section properties (`w:sectPr`) are the final `Block::Raw`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Body {
    pub head: ElementHead,
    pub blocks: Vec<Block>,
}

/// Block-level content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Paragraph(Paragraph),
    Table(Table),
    /// Block-level `w:sdt`, `w:customXml`, bookmarks, section properties, `mc:AlternateContent`...
    Raw(RawXml),
}

/// `w:p`. The paragraph properties are typed only as far as raw XML; `props` holds the
/// `w:pPr` element when it is the very first child.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paragraph {
    pub head: ElementHead,
    pub props: Option<XmlElement>,
    pub content: Vec<Inline>,
}

/// Paragraph-level content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Run(Run),
    /// Hyperlinks, tracked insertions/deletions, smart tags and simple fields.
    Group(Group),
    /// Bookmarks, complex-field markers outside runs, `w:sdt`, math, comments ranges,
    /// `mc:AlternateContent`, whitespace and anything unknown.
    Raw(RawXml),
}

/// A run-level container whose children are again inline content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub head: ElementHead,
    pub content: Vec<Inline>,
}

/// `w:r`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub head: ElementHead,
    pub props: RunProps,
    pub content: Vec<RunContent>,
}

/// Run content. Only `w:t`, `w:tab` and `w:br` count as characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunContent {
    /// `w:t`. `attrs` holds attributes other than `xml:space`.
    Text {
        text: String,
        preserve_space: bool,
        attrs: Vec<Attr>,
    },
    Tab,
    Break(Break),
    /// `w:delText`, `w:instrText`, `w:fldChar`, drawings, symbols, footnote references...
    Raw(RawXml),
}

/// `w:br`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Break {
    pub attrs: Vec<Attr>,
}

/// Kinds of `w:br` (`w:type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakKind {
    TextWrapping,
    Page,
    Column,
}

impl Break {
    /// A break of the given kind. `w:type` is omitted for line breaks (the default).
    pub fn new(kind: BreakKind, owner: &ElementHead) -> Self {
        let mut head = owner.sibling("br");
        match kind {
            BreakKind::TextWrapping => {}
            BreakKind::Page => head.set_attr_w("type", "page"),
            BreakKind::Column => head.set_attr_w("type", "column"),
        }
        Self { attrs: head.attrs }
    }

    /// Kind of break; unknown values read as `TextWrapping`.
    pub fn kind(&self) -> BreakKind {
        let value = self
            .attrs
            .iter()
            .find(|a| is_w_ns(a.ns.as_deref()) && a.local_name() == "type")
            .map(|a| a.value.as_str());
        match value {
            Some("page") => BreakKind::Page,
            Some("column") => BreakKind::Column,
            _ => BreakKind::TextWrapping,
        }
    }
}

/// `w:tbl`. Children other than rows (`w:tblPr`, `w:tblGrid`, ...) stay raw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub head: ElementHead,
    pub children: Vec<TableChild>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableChild {
    Row(Row),
    Raw(RawXml),
}

/// `w:tr`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub head: ElementHead,
    pub children: Vec<RowChild>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowChild {
    Cell(Cell),
    Raw(RawXml),
}

/// `w:tc`. `w:tcPr` is the first `Block::Raw`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub head: ElementHead,
    pub blocks: Vec<Block>,
}

// ---------------------------------------------------------------------------
// Conversion from the generic tree
// ---------------------------------------------------------------------------

impl DocumentRoot {
    /// `None` when the root is not `w:document` or has no `w:body`.
    pub(crate) fn from_xml(root: XmlElement) -> Result<Self, String> {
        if !root.head.is_w("document") {
            return Err(format!(
                "root element is <{}>, not w:document",
                root.head.name
            ));
        }
        let mut seen_body = false;
        let children = root
            .children
            .into_iter()
            .map(|child| match child {
                RawXml::Element(e) if !seen_body && e.head.is_w("body") => {
                    seen_body = true;
                    DocChild::Body(Body::from_xml(e))
                }
                other => DocChild::Raw(other),
            })
            .collect();
        if !seen_body {
            return Err("w:document has no w:body".to_owned());
        }
        Ok(Self {
            head: root.head,
            children,
        })
    }

    pub fn body(&self) -> Option<&Body> {
        self.children.iter().find_map(|c| match c {
            DocChild::Body(b) => Some(b),
            DocChild::Raw(_) => None,
        })
    }

    pub(crate) fn body_mut(&mut self) -> Option<&mut Body> {
        self.children.iter_mut().find_map(|c| match c {
            DocChild::Body(b) => Some(b),
            DocChild::Raw(_) => None,
        })
    }

    pub(crate) fn write(&self, out: &mut String) {
        if self.children.is_empty() {
            write_head(&self.head, true, out);
            return;
        }
        write_head(&self.head, false, out);
        for child in &self.children {
            match child {
                DocChild::Body(b) => b.write(out),
                DocChild::Raw(r) => r.write(out),
            }
        }
        write_end(&self.head, out);
    }
}

impl Body {
    fn from_xml(e: XmlElement) -> Self {
        Self {
            head: e.head,
            blocks: blocks_from(e.children),
        }
    }

    fn write(&self, out: &mut String) {
        write_blocks(&self.head, &self.blocks, out);
    }
}

pub(crate) fn blocks_from(children: Vec<RawXml>) -> Vec<Block> {
    children
        .into_iter()
        .map(|child| match child {
            RawXml::Element(e) if e.head.is_w("p") => Block::Paragraph(Paragraph::from_xml(e)),
            RawXml::Element(e) if e.head.is_w("tbl") => Block::Table(Table::from_xml(e)),
            other => Block::Raw(other),
        })
        .collect()
}

fn write_blocks(head: &ElementHead, blocks: &[Block], out: &mut String) {
    if blocks.is_empty() {
        write_head(head, true, out);
        return;
    }
    write_head(head, false, out);
    for block in blocks {
        match block {
            Block::Paragraph(p) => p.write(out),
            Block::Table(t) => t.write(out),
            Block::Raw(r) => r.write(out),
        }
    }
    write_end(head, out);
}

impl Table {
    fn from_xml(e: XmlElement) -> Self {
        let children = e
            .children
            .into_iter()
            .map(|child| match child {
                RawXml::Element(r) if r.head.is_w("tr") => TableChild::Row(Row::from_xml(r)),
                other => TableChild::Raw(other),
            })
            .collect();
        Self {
            head: e.head,
            children,
        }
    }

    pub fn rows(&self) -> impl Iterator<Item = &Row> {
        self.children.iter().filter_map(|c| match c {
            TableChild::Row(r) => Some(r),
            TableChild::Raw(_) => None,
        })
    }

    fn write(&self, out: &mut String) {
        if self.children.is_empty() {
            write_head(&self.head, true, out);
            return;
        }
        write_head(&self.head, false, out);
        for child in &self.children {
            match child {
                TableChild::Row(r) => r.write(out),
                TableChild::Raw(r) => r.write(out),
            }
        }
        write_end(&self.head, out);
    }
}

impl Row {
    fn from_xml(e: XmlElement) -> Self {
        let children = e
            .children
            .into_iter()
            .map(|child| match child {
                RawXml::Element(c) if c.head.is_w("tc") => RowChild::Cell(Cell::from_xml(c)),
                other => RowChild::Raw(other),
            })
            .collect();
        Self {
            head: e.head,
            children,
        }
    }

    pub fn cells(&self) -> impl Iterator<Item = &Cell> {
        self.children.iter().filter_map(|c| match c {
            RowChild::Cell(c) => Some(c),
            RowChild::Raw(_) => None,
        })
    }

    fn write(&self, out: &mut String) {
        if self.children.is_empty() {
            write_head(&self.head, true, out);
            return;
        }
        write_head(&self.head, false, out);
        for child in &self.children {
            match child {
                RowChild::Cell(c) => c.write(out),
                RowChild::Raw(r) => r.write(out),
            }
        }
        write_end(&self.head, out);
    }
}

impl Cell {
    fn from_xml(e: XmlElement) -> Self {
        Self {
            head: e.head,
            blocks: blocks_from(e.children),
        }
    }

    fn write(&self, out: &mut String) {
        write_blocks(&self.head, &self.blocks, out);
    }
}

impl Paragraph {
    fn from_xml(e: XmlElement) -> Self {
        let mut children = e.children.into_iter().peekable();
        let props = match children.peek() {
            Some(RawXml::Element(first)) if first.head.is_w("pPr") => match children.next() {
                Some(RawXml::Element(el)) => Some(el),
                _ => None,
            },
            _ => None,
        };
        Self {
            head: e.head,
            props,
            content: inlines_from(children),
        }
    }

    fn write(&self, out: &mut String) {
        if self.props.is_none() && self.content.is_empty() {
            write_head(&self.head, true, out);
            return;
        }
        write_head(&self.head, false, out);
        if let Some(props) = &self.props {
            props.write(out);
        }
        write_inlines(&self.content, out);
        write_end(&self.head, out);
    }
}

/// Elements whose children are inline content.
const GROUPS: &[&str] = &[
    "hyperlink",
    "ins",
    "del",
    "moveFrom",
    "moveTo",
    "smartTag",
    "fldSimple",
];

fn inlines_from(children: impl Iterator<Item = RawXml>) -> Vec<Inline> {
    children
        .map(|child| match child {
            RawXml::Element(e) if e.head.is_w("r") => Inline::Run(Run::from_xml(e)),
            RawXml::Element(e) if GROUPS.iter().any(|g| e.head.is_w(g)) => Inline::Group(Group {
                head: e.head,
                content: inlines_from(e.children.into_iter()),
            }),
            other => Inline::Raw(other),
        })
        .collect()
}

fn write_inlines(content: &[Inline], out: &mut String) {
    for inline in content {
        match inline {
            Inline::Run(r) => r.write(out),
            Inline::Group(g) => {
                if g.content.is_empty() {
                    write_head(&g.head, true, out);
                } else {
                    write_head(&g.head, false, out);
                    write_inlines(&g.content, out);
                    write_end(&g.head, out);
                }
            }
            Inline::Raw(r) => r.write(out),
        }
    }
}

impl Run {
    fn from_xml(e: XmlElement) -> Self {
        let mut children = e.children.into_iter().peekable();
        let props = match children.peek() {
            Some(RawXml::Element(first)) if first.head.is_w("rPr") => match children.next() {
                Some(RawXml::Element(el)) => RunProps::from_element(el),
                _ => RunProps::default(),
            },
            _ => RunProps::default(),
        };
        Self {
            head: e.head,
            props,
            content: children.map(RunContent::from_xml).collect(),
        }
    }

    pub(crate) fn write(&self, out: &mut String) {
        if !self.props.is_present() && self.content.is_empty() {
            write_head(&self.head, true, out);
            return;
        }
        write_head(&self.head, false, out);
        self.props.write(out);
        for item in &self.content {
            item.write(&self.head, out);
        }
        write_end(&self.head, out);
    }

    /// Number of characters this run contributes to paragraph text.
    pub fn char_len(&self) -> usize {
        self.content.iter().map(RunContent::char_len).sum()
    }

    /// Text of the run: `w:t` content, tabs as `\t`, breaks as `\n`.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for item in &self.content {
            item.push_text(&mut out);
        }
        out
    }

    /// Sets or clears bold, creating `w:rPr` when needed.
    pub fn set_bold(&mut self, value: Option<bool>) {
        let owner = self.head.clone();
        self.props.set_bold(value, &owner);
    }

    /// A run with the same element name and properties but no content.
    pub(crate) fn empty_clone(&self) -> Self {
        Self {
            head: self.head.clone(),
            props: self.props.clone(),
            content: Vec::new(),
        }
    }
}

impl RunContent {
    fn from_xml(node: RawXml) -> Self {
        let RawXml::Element(e) = node else {
            return Self::Raw(node);
        };
        if e.head.is_w("t") {
            let text_only = e.children.iter().all(|c| matches!(c, RawXml::Text(_)));
            let mut preserve_space = false;
            let mut attrs = Vec::new();
            for attr in &e.head.attrs {
                if attr.is_xml_space() && attr.value == "preserve" {
                    preserve_space = true;
                } else {
                    attrs.push(attr.clone());
                }
            }
            if text_only {
                let text = e
                    .children
                    .iter()
                    .filter_map(|c| match c {
                        RawXml::Text(t) => Some(t.as_str()),
                        _ => None,
                    })
                    .collect();
                return Self::Text {
                    text,
                    preserve_space,
                    attrs,
                };
            }
        } else if e.head.is_w("tab") && e.head.attrs.is_empty() && e.children.is_empty() {
            return Self::Tab;
        } else if e.head.is_w("br") && e.children.is_empty() {
            return Self::Break(Break {
                attrs: e.head.attrs,
            });
        }
        Self::Raw(RawXml::Element(e))
    }

    fn write(&self, run: &ElementHead, out: &mut String) {
        match self {
            Self::Text {
                text,
                preserve_space,
                attrs,
            } => {
                let mut head = run.sibling("t");
                head.attrs.clone_from(attrs);
                if *preserve_space {
                    head.attrs.retain(|a| !a.is_xml_space());
                    head.attrs.push(Attr {
                        name: "xml:space".to_owned(),
                        ns: Some("http://www.w3.org/XML/1998/namespace".into()),
                        value: "preserve".to_owned(),
                    });
                }
                if text.is_empty() {
                    write_head(&head, true, out);
                } else {
                    write_head(&head, false, out);
                    crate::xml::escape_text(text, out);
                    write_end(&head, out);
                }
            }
            Self::Tab => write_head(&run.sibling("tab"), true, out),
            Self::Break(b) => {
                let mut head = run.sibling("br");
                head.attrs.clone_from(&b.attrs);
                write_head(&head, true, out);
            }
            Self::Raw(r) => r.write(out),
        }
    }

    /// Characters contributed to paragraph text (Unicode scalar values).
    pub fn char_len(&self) -> usize {
        match self {
            Self::Text { text, .. } => text.chars().count(),
            Self::Tab | Self::Break(_) => 1,
            Self::Raw(_) => 0,
        }
    }

    fn push_text(&self, out: &mut String) {
        match self {
            Self::Text { text, .. } => out.push_str(text),
            Self::Tab => out.push('\t'),
            Self::Break(_) => out.push('\n'),
            Self::Raw(_) => {}
        }
    }
}

// Group text helpers and name derivation used by the edit layer.

impl Inline {
    /// Characters this inline contributes to paragraph text.
    pub fn char_len(&self) -> usize {
        match self {
            Self::Run(r) => r.char_len(),
            Self::Group(g) => g.content.iter().map(Inline::char_len).sum(),
            Self::Raw(_) => 0,
        }
    }
}

impl Paragraph {
    /// Concatenated text of all runs, including those in hyperlinks and insertions.
    pub fn text(&self) -> String {
        fn collect(content: &[Inline], out: &mut String) {
            for inline in content {
                match inline {
                    Inline::Run(r) => {
                        for item in &r.content {
                            item.push_text(out);
                        }
                    }
                    Inline::Group(g) => collect(&g.content, out),
                    Inline::Raw(_) => {}
                }
            }
        }
        let mut out = String::new();
        collect(&self.content, &mut out);
        out
    }

    /// Number of characters in [`Paragraph::text`].
    pub fn char_len(&self) -> usize {
        self.content.iter().map(Inline::char_len).sum()
    }

    /// A new empty run named like the paragraph's runs (`w:r`).
    pub(crate) fn new_run(&self) -> Run {
        Run {
            head: ElementHead {
                name: sibling_name(&self.head.name, "r"),
                ns: self.head.ns.clone(),
                attrs: Vec::new(),
            },
            props: RunProps::default(),
            content: Vec::new(),
        }
    }
}

impl Body {
    /// Paragraphs in document order, including those in table cells (recursively).
    /// Paragraphs inside raw blocks (content controls, `mc:AlternateContent`) are not
    /// included.
    pub fn paragraphs(&self) -> Vec<&Paragraph> {
        fn walk<'a>(blocks: &'a [Block], out: &mut Vec<&'a Paragraph>) {
            for block in blocks {
                match block {
                    Block::Paragraph(p) => out.push(p),
                    Block::Table(t) => {
                        for row in t.rows() {
                            for cell in row.cells() {
                                walk(&cell.blocks, out);
                            }
                        }
                    }
                    Block::Raw(_) => {}
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.blocks, &mut out);
        out
    }

    pub(crate) fn paragraph_mut(&mut self, index: usize) -> Option<&mut Paragraph> {
        fn walk<'a>(
            blocks: &'a mut [Block],
            seen: &mut usize,
            index: usize,
        ) -> Option<&'a mut Paragraph> {
            for block in blocks {
                match block {
                    Block::Paragraph(p) => {
                        if *seen == index {
                            return Some(p);
                        }
                        *seen += 1;
                    }
                    Block::Table(t) => {
                        for child in &mut t.children {
                            let TableChild::Row(row) = child else {
                                continue;
                            };
                            for rc in &mut row.children {
                                let RowChild::Cell(cell) = rc else { continue };
                                if let Some(p) = walk(&mut cell.blocks, seen, index) {
                                    return Some(p);
                                }
                            }
                        }
                    }
                    Block::Raw(_) => {}
                }
            }
            None
        }
        let mut seen = 0;
        walk(&mut self.blocks, &mut seen, index)
    }
}
