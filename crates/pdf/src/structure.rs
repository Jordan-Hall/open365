//! The logical structure of the document (the tagged-PDF tree) and the table facts the
//! content writer needs, both derived from the DOCX model in the layout's paragraph order.

use open365_docx::{
    Block, Border, BorderSet, CellPosition, CellProps, Document, Shading, Styles, TableProps,
};

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
    /// (table, row, cell index in row) and the table nesting depth.
    pub cell: Option<(usize, usize, usize, usize)>,
}

#[derive(Debug)]
pub struct CellMeta {
    pub col: usize,
    pub span: usize,
    pub row_span: usize,
    pub own: BorderSet,
    pub styled: BorderSet,
    pub shading: Option<Shading>,
}

#[derive(Debug, Default)]
pub struct TableMeta {
    pub borders: BorderSet,
    pub columns: usize,
    pub rows: Vec<Vec<CellMeta>>,
}

#[derive(Debug)]
pub struct DocStructure {
    pub root: Node,
    pub paras: Vec<ParaMeta>,
    pub tables: Vec<TableMeta>,
}

/// Which edge of a cell.
#[derive(Debug, Clone, Copy)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

impl TableMeta {
    /// The border drawn on `edge` of cell `c` of row `r`: the cell's own, then its table
    /// style's, then the table's (the same order the layout measures borders in).
    pub fn border(&self, r: usize, c: usize, edge: Edge) -> Option<Border> {
        let cell = self.rows.get(r)?.get(c)?;
        let nrows = self.rows.len();
        let last_col = cell.col + cell.span >= self.columns;
        let last_row = r + cell.row_span.max(1) >= nrows;
        let (own, sty, tb) = (&cell.own, &cell.styled, &self.borders);
        let pick = |a: &Option<Border>, b: &Option<Border>, t: &Option<Border>| {
            a.as_ref().or(b.as_ref()).or(t.as_ref()).cloned()
        };
        let found = match edge {
            Edge::Top => pick(
                &own.top,
                &sty.top,
                if r == 0 { &tb.top } else { &tb.inside_h },
            ),
            Edge::Bottom => pick(
                &own.bottom,
                &sty.bottom,
                if last_row { &tb.bottom } else { &tb.inside_h },
            ),
            Edge::Left => pick(
                &own.left,
                &sty.left,
                if cell.col == 0 {
                    &tb.left
                } else {
                    &tb.inside_v
                },
            ),
            Edge::Right => pick(
                &own.right,
                &sty.right,
                if last_col { &tb.right } else { &tb.inside_v },
            ),
        };
        found.filter(Border::is_visible)
    }
}

fn heading_level(styles: Option<&Styles>, style_id: Option<&str>) -> Option<u8> {
    let styles = styles?;
    let mut id = style_id?.to_owned();
    for _ in 0..8 {
        let info = styles.style(&id)?;
        let name = info.name.to_lowercase();
        if let Some(rest) = name.strip_prefix("heading ")
            && let Ok(n) = rest.trim().parse::<u8>()
            && n >= 1
        {
            return Some(n.min(6));
        }
        id = info.based_on.clone()?;
    }
    None
}

struct Walker<'a> {
    pdf: &'a mut Pdf,
    styles: Option<&'a Styles>,
    labels: Vec<Option<String>>,
    next_para: usize,
    paras: Vec<ParaMeta>,
    tables: Vec<TableMeta>,
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

    fn blocks(
        &mut self,
        blocks: &[Block],
        cell: Option<(usize, usize, usize, usize)>,
    ) -> Vec<Node> {
        let mut out: Vec<Node> = Vec::new();
        for block in blocks {
            match block {
                Block::Paragraph(p) => {
                    let index = self.next_para;
                    self.next_para += 1;
                    let labeled = self.labels.get(index).is_some_and(Option::is_some);
                    let kind = if labeled {
                        Kind::LBody
                    } else {
                        heading_level(self.styles, p.style_id()).map_or(Kind::P, Kind::H)
                    };
                    let leaf = self.node(kind, Vec::new());
                    self.paras.push(ParaMeta {
                        node: leaf.id,
                        tag: kind.name(),
                        cell,
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
                Block::Table(t) => {
                    let depth = cell.map_or(0, |c| c.3 + 1);
                    out.push(self.table(t, depth));
                }
                Block::Raw(_) => {}
            }
        }
        out
    }

    fn table(&mut self, t: &open365_docx::Table, depth: usize) -> Node {
        let ti = self.tables.len();
        self.tables.push(TableMeta::default());
        let view = t.view();
        let props: TableProps = match self.styles {
            Some(s) => s.table_props(&view.props),
            None => view.props.clone(),
        };
        let columns = view
            .rows
            .iter()
            .map(|r| {
                r.cells
                    .last()
                    .map_or(r.props.grid_before, |c| c.col + c.span)
            })
            .max()
            .unwrap_or(0)
            .max(view.grid.len());
        let nrows = view.rows.len();
        let mut meta_rows = Vec::new();
        let mut row_nodes = Vec::new();
        for (r, row) in view.rows.iter().enumerate() {
            let mut meta_cells = Vec::new();
            let mut cell_nodes = Vec::new();
            for (ci, c) in row.cells.iter().enumerate() {
                let fmt: CellProps = self
                    .styles
                    .map(|s| {
                        s.table_cell_format(
                            &props,
                            CellPosition {
                                row: r,
                                column: c.col,
                                rows: nrows,
                                columns,
                            },
                        )
                        .cell
                    })
                    .unwrap_or_default();
                meta_cells.push(CellMeta {
                    col: c.col,
                    span: c.span,
                    row_span: c.row_span,
                    own: c.props.borders.clone(),
                    styled: fmt.borders.clone(),
                    shading: c.props.shading.clone().or(fmt.shading.clone()),
                });
                let kids = self.blocks(&c.cell.blocks, Some((ti, r, ci, depth)));
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
            meta_rows.push(meta_cells);
            row_nodes.push(self.node(Kind::Tr, cell_nodes));
        }
        self.tables[ti] = TableMeta {
            borders: props.borders.clone(),
            columns,
            rows: meta_rows,
        };
        self.node(Kind::Table, row_nodes)
    }
}

/// Walks the main story in the order the layout numbers paragraphs.
pub fn analyse(doc: &Document, labels: Vec<Option<String>>, pdf: &mut Pdf) -> DocStructure {
    let mut walker = Walker {
        pdf,
        styles: doc.styles(),
        labels,
        next_para: 0,
        paras: Vec::new(),
        tables: Vec::new(),
    };
    let kids = walker.blocks(&doc.body().blocks, None);
    let root = walker.node(Kind::Document, kids);
    DocStructure {
        root,
        paras: walker.paras,
        tables: walker.tables,
    }
}
