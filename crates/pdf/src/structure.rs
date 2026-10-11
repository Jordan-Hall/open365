//! The logical structure of the document (the tagged-PDF tree), built from the DOCX model in the
//! layout's paragraph order. What a paragraph is (heading level, list item) comes from the
//! layout's [`ParaRole`]; drawing facts such as borders and shading come from the layout too.

use std::collections::HashMap;

use open365_docx::{Block, Document};
use open365_layout::ParaRole;

use crate::writer::Pdf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Document,
    P,
    H(u8),
    L,
    Li,
    LBody,
    Table,
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
            Self::LBody => "LBody".into(),
            Self::Table => "Table".into(),
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
    /// Extra attributes, written verbatim into the element dictionary.
    pub attrs: String,
}

#[derive(Debug, Clone)]
pub struct ParaMeta {
    /// Structure element that owns the paragraph's marked content.
    pub node: u32,
    /// Marked-content tag name.
    pub tag: String,
}

#[derive(Debug)]
pub struct DocStructure {
    pub root: Node,
    pub paras: Vec<ParaMeta>,
}

struct Walker<'a> {
    pdf: &'a mut Pdf,
    roles: &'a HashMap<usize, ParaRole>,
    next_para: usize,
    paras: Vec<ParaMeta>,
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
                Block::Paragraph(_) => {
                    let index = self.next_para;
                    self.next_para += 1;
                    let role = self.roles.get(&index).copied().unwrap_or_default();
                    let labeled = role == ParaRole::ListItem;
                    let kind = match role {
                        ParaRole::ListItem => Kind::LBody,
                        ParaRole::Heading(n) => Kind::H(n),
                        ParaRole::Plain => Kind::P,
                    };
                    let leaf = self.node(kind, Vec::new());
                    self.paras.push(ParaMeta {
                        node: leaf.id,
                        tag: kind.name(),
                    });
                    if labeled {
                        let li = self.node(Kind::Li, vec![leaf]);
                        match out.last_mut() {
                            Some(l) if l.kind == Kind::L => l.kids.push(li),
                            _ => {
                                let list = self.node(Kind::L, vec![li]);
                                out.push(list);
                            }
                        }
                    } else {
                        out.push(leaf);
                    }
                }
                Block::Table(t) => out.push(self.table(t)),
                Block::Raw(_) => {}
            }
        }
        out
    }

    fn table(&mut self, t: &open365_docx::Table) -> Node {
        let view = t.view();
        let mut row_nodes = Vec::new();
        for row in &view.rows {
            let mut cell_nodes = Vec::new();
            for c in &row.cells {
                let kids = self.blocks(&c.cell.blocks);
                let kind = if row.props.is_header {
                    Kind::Th
                } else {
                    Kind::Td
                };
                let mut node = self.node(kind, kids);
                let mut attrs = String::new();
                if row.props.is_header {
                    attrs.push_str("/Scope /Column ");
                }
                if c.span > 1 {
                    attrs.push_str(&format!("/ColSpan {} ", c.span));
                }
                if c.row_span > 1 {
                    attrs.push_str(&format!("/RowSpan {} ", c.row_span));
                }
                if !attrs.is_empty() {
                    node.attrs = format!("/A << /O /Table {attrs}>>");
                }
                cell_nodes.push(node);
            }
            row_nodes.push(self.node(Kind::Tr, cell_nodes));
        }
        self.node(Kind::Table, row_nodes)
    }
}

/// Walks the main story in the order the layout numbers paragraphs. `roles` maps a paragraph
/// index to its layout role; a paragraph without one is plain.
pub fn analyse(doc: &Document, roles: &HashMap<usize, ParaRole>, pdf: &mut Pdf) -> DocStructure {
    let mut walker = Walker {
        pdf,
        roles,
        next_para: 0,
        paras: Vec::new(),
    };
    let kids = walker.blocks(&doc.body().blocks);
    let root = walker.node(Kind::Document, kids);
    DocStructure {
        root,
        paras: walker.paras,
    }
}
