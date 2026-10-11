//! A read-only view of the main story with block-level content controls (`w:sdt`) unwrapped,
//! for layout. The typed model keeps a content control as raw XML (round trip), so its
//! paragraphs are not addressable; layout needs them in place, with the two story characters
//! Word counts for each control (its start and its end, `layout-sdt`).

use std::sync::Arc;

use crate::model::{Block, Cell, Inline, Paragraph, Row, RowChild, Table, TableChild};
use crate::xml::{ElementHead, RawXml, W_NS, XmlElement};

/// Blocks with the block-level content controls unwrapped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlatBlocks {
    pub blocks: Vec<Block>,
    /// For each paragraph of the source in `Body::paragraphs` order (the addressable ones), its
    /// index among the paragraphs of `blocks` (tables included, document order).
    pub addressable: Vec<usize>,
}

/// Name of the synthetic inline element that stands for a control's start character.
pub const SDT_START: &str = "sdtStart";
/// Name of the synthetic inline element that stands for a control's end character.
pub const SDT_END: &str = "sdtEnd";

fn marker(local: &str) -> Inline {
    Inline::Raw(RawXml::Element(XmlElement {
        head: ElementHead {
            name: format!("w:{local}"),
            ns: Some(Arc::from(W_NS)),
            attrs: Vec::new(),
        },
        children: Vec::new(),
    }))
}

/// True when `inline` is a synthetic control-start (or control-end) marker made by
/// [`flatten_content_controls`].
pub fn is_sdt_marker(inline: &Inline) -> bool {
    matches!(inline, Inline::Raw(RawXml::Element(e)) if e.head.is_w(SDT_START) || e.head.is_w(SDT_END))
}

fn has_sdt(blocks: &[Block]) -> bool {
    blocks.iter().any(|b| match b {
        Block::Raw(_) => b.sdt_content().is_some(),
        Block::Table(t) => t.children.iter().any(|c| match c {
            TableChild::Row(r) => r.children.iter().any(|rc| match rc {
                RowChild::Cell(c) => has_sdt(&c.blocks),
                RowChild::Raw(raw) => is_sdt(raw),
            }),
            TableChild::Raw(_) => false,
        }),
        Block::Paragraph(_) => false,
    })
}

fn is_sdt(raw: &RawXml) -> bool {
    matches!(raw, RawXml::Element(e) if e.head.is_w("sdt"))
}

struct Flattener {
    /// Paragraphs emitted so far.
    emitted: usize,
    addressable: Vec<usize>,
}

impl Flattener {
    fn blocks(&mut self, blocks: &[Block], in_sdt: bool, out: &mut Vec<Block>) {
        for block in blocks {
            match block {
                Block::Paragraph(p) => {
                    if !in_sdt {
                        self.addressable.push(self.emitted);
                    }
                    self.emitted += 1;
                    out.push(Block::Paragraph(p.clone()));
                }
                Block::Table(t) => {
                    let t = self.table(t, in_sdt);
                    out.push(Block::Table(t));
                }
                Block::Raw(_) => match block.sdt_content() {
                    Some(content) => {
                        let mut inner = Vec::new();
                        self.blocks(&content, true, &mut inner);
                        let first = inner.iter().position(|b| matches!(b, Block::Paragraph(_)));
                        let last = inner.iter().rposition(|b| matches!(b, Block::Paragraph(_)));
                        if let Some(Block::Paragraph(p)) = first.and_then(|i| inner.get_mut(i)) {
                            add_marker(p, SDT_START, true);
                        }
                        if let Some(Block::Paragraph(p)) = last.and_then(|i| inner.get_mut(i)) {
                            add_marker(p, SDT_END, false);
                        }
                        out.extend(inner);
                    }
                    None => out.push(block.clone()),
                },
            }
        }
    }

    fn table(&mut self, t: &Table, in_sdt: bool) -> Table {
        let children = t
            .children
            .iter()
            .map(|c| match c {
                TableChild::Row(r) => TableChild::Row(self.row(r, in_sdt)),
                TableChild::Raw(raw) => TableChild::Raw(raw.clone()),
            })
            .collect();
        Table {
            head: t.head.clone(),
            children,
        }
    }

    fn row(&mut self, r: &Row, in_sdt: bool) -> Row {
        let mut children = Vec::new();
        for c in &r.children {
            match c {
                RowChild::Cell(cell) => children.push(RowChild::Cell(self.cell(cell, in_sdt))),
                RowChild::Raw(raw) if is_sdt(raw) => {
                    // A row-level control wraps cells.
                    let wrapped = Block::Raw(raw.clone());
                    for child in wrapped.sdt_content_raw().unwrap_or_default() {
                        match child {
                            RawXml::Element(e) if e.head.is_w("tc") => {
                                let cell = Block::Raw(RawXml::Element(e)).into_cell();
                                if let Some(cell) = cell {
                                    children.push(RowChild::Cell(self.cell(&cell, true)));
                                }
                            }
                            other => children.push(RowChild::Raw(other)),
                        }
                    }
                }
                RowChild::Raw(raw) => children.push(RowChild::Raw(raw.clone())),
            }
        }
        Row {
            head: r.head.clone(),
            children,
        }
    }

    fn cell(&mut self, c: &Cell, in_sdt: bool) -> Cell {
        let mut blocks = Vec::new();
        self.blocks(&c.blocks, in_sdt, &mut blocks);
        Cell {
            head: c.head.clone(),
            blocks,
        }
    }
}

fn add_marker(p: &mut Paragraph, local: &str, front: bool) {
    if front {
        p.content.insert(0, marker(local));
    } else {
        p.content.push(marker(local));
    }
}

/// `blocks` with every block-level content control (also in table cells and rows) replaced by
/// its content, the control's first and last paragraphs carrying a start and an end marker
/// (see [`is_sdt_marker`]). `None` when there is no control to unwrap.
pub fn flatten_content_controls(blocks: &[Block]) -> Option<FlatBlocks> {
    if !has_sdt(blocks) {
        return None;
    }
    let mut f = Flattener {
        emitted: 0,
        addressable: Vec::new(),
    };
    let mut out = Vec::new();
    f.blocks(blocks, false, &mut out);
    Some(FlatBlocks {
        blocks: out,
        addressable: f.addressable,
    })
}
